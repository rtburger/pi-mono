import { createInterface } from "node:readline";
import { ExtensionRunner, discoverAndLoadExtensions } from "./extension-runtime/index.mjs";

let runner;
let uiCounter = 0;
let appRequestCounter = 0;
let commandActionPromises = null;
let commandActionChain = null;
let appRequestsReady = false;
const pendingUiRequests = new Map();
const pendingAppRequests = new Map();
const runtimeState = {
	model: undefined,
	thinkingLevel: "off",
	isIdle: true,
	hasPendingMessages: false,
	systemPrompt: "",
	sessionName: undefined,
	activeTools: [],
	allTools: [],
	commands: [],
	contextUsage: undefined,
};

function send(value) {
	process.stdout.write(`${JSON.stringify(value)}\n`);
}

function reply(id, data) {
	send({ type: "response", id, success: true, data });
}

function replyError(id, error) {
	send({ type: "response", id, success: false, error });
}

function emitRuntimeError(event, error) {
	send({ type: "extension_error", extensionPath: "<runtime>", event, error });
}

function emitUnsupported(event, error) {
	emitRuntimeError(event, error);
}

function nextUiId() {
	uiCounter += 1;
	return `ui-${uiCounter}`;
}

function nextAppRequestId() {
	appRequestCounter += 1;
	return `app-${appRequestCounter}`;
}

function requestHost(method, payload = {}) {
	const id = nextAppRequestId();
	return new Promise((resolve, reject) => {
		pendingAppRequests.set(id, { resolve, reject });
		send({ type: "app_request", id, method, ...payload });
	});
}

function trackCommandAction(promise) {
	if (Array.isArray(commandActionPromises)) {
		commandActionPromises.push(promise);
	}
	return promise;
}

function fireAndTrackHostRequest(method, payload, event, onSuccess) {
	const previous = commandActionChain ?? Promise.resolve();
	const tracked = previous
		.then(() => requestHost(method, payload))
		.then((data) => {
			onSuccess?.(data);
			return data;
		})
		.catch((error) => {
			emitRuntimeError(event, error instanceof Error ? error.message : String(error));
			return undefined;
		});
	commandActionChain = tracked;
	trackCommandAction(tracked);
}

function resolveAppResponse(message) {
	const pending = pendingAppRequests.get(message.id);
	if (!pending) {
		return;
	}
	pendingAppRequests.delete(message.id);
	if (message.success) {
		pending.resolve(message.data);
		return;
	}
	pending.reject(new Error(typeof message.error === "string" ? message.error : "Host request failed"));
}

function applyRuntimeState(next) {
	if (!next || typeof next !== "object") {
		return;
	}
	if ("model" in next) {
		runtimeState.model = next.model;
	}
	if ("thinkingLevel" in next && typeof next.thinkingLevel === "string") {
		runtimeState.thinkingLevel = next.thinkingLevel;
	}
	if ("isIdle" in next) {
		runtimeState.isIdle = Boolean(next.isIdle);
	}
	if ("hasPendingMessages" in next) {
		runtimeState.hasPendingMessages = Boolean(next.hasPendingMessages);
	}
	if ("systemPrompt" in next && typeof next.systemPrompt === "string") {
		runtimeState.systemPrompt = next.systemPrompt;
	}
	if ("sessionName" in next) {
		runtimeState.sessionName = typeof next.sessionName === "string" ? next.sessionName : undefined;
	}
	if (Array.isArray(next.activeTools)) {
		runtimeState.activeTools = [...next.activeTools];
	}
	if (Array.isArray(next.allTools)) {
		runtimeState.allTools = [...next.allTools];
	}
	if (Array.isArray(next.commands)) {
		runtimeState.commands = [...next.commands];
	}
	if ("contextUsage" in next) {
		runtimeState.contextUsage = next.contextUsage;
	}
}

function createDialogPromise(opts, defaultValue, request, parseResponse) {
	if (opts?.signal?.aborted) {
		return Promise.resolve(defaultValue);
	}

	const id = nextUiId();
	return new Promise((resolve) => {
		let timeoutId;
		const cleanup = () => {
			if (timeoutId) {
				clearTimeout(timeoutId);
			}
			opts?.signal?.removeEventListener("abort", onAbort);
			pendingUiRequests.delete(id);
		};
		const onAbort = () => {
			cleanup();
			resolve(defaultValue);
		};

		opts?.signal?.addEventListener("abort", onAbort, { once: true });
		if (opts?.timeout) {
			timeoutId = setTimeout(() => {
				cleanup();
				resolve(defaultValue);
			}, opts.timeout);
		}

		pendingUiRequests.set(id, {
			resolve(response) {
				cleanup();
				resolve(parseResponse(response));
			},
		});
		send({ type: "extension_ui_request", id, ...request });
	});
}

function createUiContext() {
	return {
		select(title, options, opts) {
			return createDialogPromise(
				opts,
				undefined,
				{ method: "select", title, options, timeout: opts?.timeout },
				(response) => {
					if (response.cancelled) {
						return undefined;
					}
					return typeof response.value === "string" ? response.value : undefined;
				},
			);
		},
		confirm(title, message, opts) {
			return createDialogPromise(
				opts,
				false,
				{ method: "confirm", title, message, timeout: opts?.timeout },
				(response) => {
					if (response.cancelled) {
						return false;
					}
					return Boolean(response.confirmed);
				},
			);
		},
		input(title, placeholder, opts) {
			return createDialogPromise(
				opts,
				undefined,
				{ method: "input", title, placeholder, timeout: opts?.timeout },
				(response) => {
					if (response.cancelled) {
						return undefined;
					}
					return typeof response.value === "string" ? response.value : undefined;
				},
			);
		},
		notify(message, type) {
			send({
				type: "extension_ui_request",
				id: nextUiId(),
				method: "notify",
				message,
				notifyType: type,
			});
		},
		onTerminalInput() {
			return () => {};
		},
		setStatus(key, text) {
			send({
				type: "extension_ui_request",
				id: nextUiId(),
				method: "setStatus",
				statusKey: key,
				statusText: text,
			});
		},
		setWorkingMessage() {},
		setHiddenThinkingLabel() {},
		setWidget(key, content, options) {
			if (content === undefined || Array.isArray(content)) {
				send({
					type: "extension_ui_request",
					id: nextUiId(),
					method: "setWidget",
					widgetKey: key,
					widgetLines: content,
					widgetPlacement: options?.placement,
				});
			}
		},
		setFooter() {},
		setHeader() {},
		setTitle(title) {
			send({ type: "extension_ui_request", id: nextUiId(), method: "setTitle", title });
		},
		async custom() {
			return undefined;
		},
		pasteToEditor(text) {
			this.setEditorText(text);
		},
		setEditorText(text) {
			send({ type: "extension_ui_request", id: nextUiId(), method: "set_editor_text", text });
		},
		getEditorText() {
			return "";
		},
		editor(title, prefill) {
			return createDialogPromise(
				undefined,
				undefined,
				{ method: "editor", title, prefill },
				(response) => {
					if (response.cancelled) {
						return undefined;
					}
					return typeof response.value === "string" ? response.value : undefined;
				},
			);
		},
		setEditorComponent() {},
		get theme() {
			return { fg(_name, text) { return text; }, bg(_name, text) { return text; } };
		},
		getAllThemes() {
			return [];
		},
		getTheme() {
			return undefined;
		},
		setTheme() {
			return { success: false, error: "Theme switching not supported in Rust RPC mode" };
		},
		getToolsExpanded() {
			return false;
		},
		setToolsExpanded() {},
	};
}

function loadDiagnostics(errors) {
	return (errors ?? []).map(({ path, error }) => ({
		level: "warning",
		message: path ? `${error} (${path})` : error,
	}));
}

function applyExtensionFlagValues(flagValues, loaded) {
	const diagnostics = [];
	const registeredFlags = new Map();
	for (const extension of loaded.extensions) {
		for (const [name, flag] of extension.flags) {
			if (!registeredFlags.has(name)) {
				registeredFlags.set(name, { type: flag.type });
			}
		}
	}

	const unknownFlags = [];
	for (const [name, value] of Object.entries(flagValues ?? {})) {
		const flag = registeredFlags.get(name);
		if (!flag) {
			unknownFlags.push(name);
			continue;
		}
		if (flag.type === "boolean") {
			loaded.runtime.flagValues.set(name, true);
			continue;
		}
		if (typeof value === "string") {
			loaded.runtime.flagValues.set(name, value);
			continue;
		}
		diagnostics.push({
			level: "error",
			message: `Extension flag "--${name}" requires a value`,
		});
	}

	if (unknownFlags.length > 0) {
		diagnostics.push({
			level: "error",
			message: `Unknown option${unknownFlags.length === 1 ? "" : "s"}: ${unknownFlags
				.map((name) => `--${name}`)
				.join(", ")}`,
		});
	}

	return diagnostics;
}

function bindRunner(loaded, cwd) {
	const dummySessionManager = {};
	const dummyModelRegistry = {
		registerProvider() {},
		unregisterProvider() {},
	};

	runner = new ExtensionRunner(
		loaded.extensions,
		loaded.runtime,
		cwd,
		dummySessionManager,
		dummyModelRegistry,
	);
	const unsupported = (event) => {
		emitUnsupported(event, `${event} is not supported in the Rust RPC extension bridge yet`);
	};

	runner.bindCore(
		{
			sendMessage(message, options) {
				fireAndTrackHostRequest("send_message", { message, options }, "send_message");
			},
			sendUserMessage(content, options) {
				fireAndTrackHostRequest(
					"send_user_message",
					{ content, options },
					"send_user_message",
				);
			},
			appendEntry(customType, data) {
				fireAndTrackHostRequest("append_entry", { customType, data }, "append_entry");
			},
			setSessionName(name) {
				fireAndTrackHostRequest(
					"set_session_name",
					{ name },
					"set_session_name",
					() => {
						runtimeState.sessionName = name;
					},
				);
			},
			getSessionName() {
				return runtimeState.sessionName;
			},
			setLabel(entryId, label) {
				fireAndTrackHostRequest("set_label", { entryId, label }, "set_label");
			},
			getActiveTools() {
				return runtimeState.activeTools;
			},
			getAllTools() {
				return runtimeState.allTools;
			},
			setActiveTools(toolNames) {
				const knownToolNames = new Set(runtimeState.allTools.map((tool) => tool?.name).filter(Boolean));
				runtimeState.activeTools = toolNames.filter((name) =>
					knownToolNames.size === 0 ? true : knownToolNames.has(name),
				);
				fireAndTrackHostRequest(
					"set_active_tools",
					{ toolNames },
					"set_active_tools",
					(data) => {
						if (Array.isArray(data?.activeTools)) {
							runtimeState.activeTools = [...data.activeTools];
						}
					},
				);
			},
			refreshTools() {
				const tools = extensionTools();
				updateRuntimeToolStateFromExtensionTools(tools);
				if (!appRequestsReady) {
					return;
				}
				fireAndTrackHostRequest(
					"refresh_tools",
					{ tools },
					"refresh_tools",
					(data) => {
						if (Array.isArray(data?.activeTools)) {
							runtimeState.activeTools = [...data.activeTools];
						}
						if (Array.isArray(data?.allTools)) {
							runtimeState.allTools = [...data.allTools];
						}
					},
				);
			},
			getCommands() {
				return runtimeState.commands;
			},
			async setModel(model) {
				const success = await requestHost("set_model", { model });
				if (success) {
					runtimeState.model = model;
				}
				return Boolean(success);
			},
			getThinkingLevel() {
				return runtimeState.thinkingLevel;
			},
			setThinkingLevel(level) {
				fireAndTrackHostRequest(
					"set_thinking_level",
					{ level },
					"set_thinking_level",
					() => {
						runtimeState.thinkingLevel = level;
					},
				);
			},
		},
		{
			getModel() {
				return runtimeState.model;
			},
			isIdle() {
				return runtimeState.isIdle;
			},
			getSignal() {
				return undefined;
			},
			abort() {
				emitUnsupported("abort");
			},
			hasPendingMessages() {
				return runtimeState.hasPendingMessages;
			},
			shutdown() {
				send({ type: "shutdown_requested" });
			},
			getContextUsage() {
				return runtimeState.contextUsage;
			},
			compact() {
				emitUnsupported("compact");
			},
			getSystemPrompt() {
				return runtimeState.systemPrompt;
			},
		},
		{
			registerProvider() {
				emitUnsupported("register_provider");
			},
			unregisterProvider() {
				emitUnsupported("unregister_provider");
			},
		},
	);
	runner.bindCommandContext({
		async waitForIdle() {
			await requestHost("wait_for_idle");
		},
		async newSession(options) {
			const result = await requestHost("new_session", { options });
			return result ?? { cancelled: false };
		},
		async fork(entryId) {
			const result = await requestHost("fork", { entryId });
			return result ?? { cancelled: false };
		},
		async navigateTree(targetId, options) {
			const result = await requestHost("navigate_tree", { targetId, options });
			return result ?? { cancelled: false };
		},
		async switchSession(sessionPath) {
			const result = await requestHost("switch_session", { sessionPath });
			return result ?? { cancelled: false };
		},
		async reload() {
			await requestHost("reload");
		},
	});
	runner.setUIContext(createUiContext());
	runner.onError((error) => {
		send({ type: "extension_error", ...error });
	});
}

function extensionCommands() {
	if (!runner) {
		return [];
	}
	return runner.getRegisteredCommands().map((command) => ({
		name: command.invocationName,
		description: command.description,
		sourceInfo: command.sourceInfo,
	}));
}

function extensionTools() {
	if (!runner) {
		return [];
	}
	return runner.getAllRegisteredTools().map((tool) => ({
		name: tool.definition.name,
		description: tool.definition.description,
		parameters: tool.definition.parameters,
		sourceInfo: tool.sourceInfo,
		promptSnippet: tool.definition.promptSnippet,
		promptGuidelines: tool.definition.promptGuidelines,
	}));
}

function updateRuntimeToolStateFromExtensionTools(nextExtensionTools) {
	const previousNames = new Set(runtimeState.allTools.map((tool) => tool?.name).filter(Boolean));
	const builtInTools = runtimeState.allTools.filter((tool) => tool?.sourceInfo?.source === "builtin");
	const mergedToolsByName = new Map();
	for (const tool of builtInTools) {
		if (tool?.name) {
			mergedToolsByName.set(tool.name, tool);
		}
	}
	for (const tool of nextExtensionTools) {
		if (tool?.name) {
			mergedToolsByName.set(tool.name, tool);
		}
	}

	const nextAllTools = Array.from(mergedToolsByName.values());
	const nextToolNames = new Set(nextAllTools.map((tool) => tool?.name).filter(Boolean));
	const nextActiveTools = runtimeState.activeTools.filter((name) => nextToolNames.has(name));
	const seenActiveTools = new Set(nextActiveTools);
	for (const tool of nextAllTools) {
		const toolName = tool?.name;
		if (!toolName) {
			continue;
		}
		if (!previousNames.has(toolName) && !seenActiveTools.has(toolName)) {
			nextActiveTools.push(toolName);
			seenActiveTools.add(toolName);
		}
	}

	runtimeState.allTools = nextAllTools;
	runtimeState.activeTools = nextActiveTools;
}

async function handleInit(message) {
	applyRuntimeState(message.state);
	const loaded = message.noExtensions
		? { extensions: [], errors: [], runtime: { flagValues: new Map() } }
		: await discoverAndLoadExtensions(message.extensions ?? [], message.cwd, message.agentDir);
	const diagnostics = [...loadDiagnostics(loaded.errors)];
	if (!loaded.extensions || loaded.extensions.length === 0) {
		reply(message.id, {
			extensionCount: 0,
			commands: [],
			tools: [],
			skillPaths: [],
			promptPaths: [],
			themePaths: [],
			diagnostics,
		});
		return;
	}

	diagnostics.push(...applyExtensionFlagValues(message.flagValues, loaded));
	bindRunner(loaded, message.cwd);

	const sessionStartEvent = {
		type: "session_start",
		reason: message.sessionStartReason ?? "startup",
	};
	if (message.previousSessionFile) {
		sessionStartEvent.previousSessionFile = message.previousSessionFile;
	}
	await runner.emit(sessionStartEvent);
	const resources = runner.hasHandlers("resources_discover")
		? await runner.emitResourcesDiscover(
				message.cwd,
				message.sessionStartReason === "reload" ? "reload" : "startup",
			)
		: { skillPaths: [], promptPaths: [], themePaths: [] };
	const commands = extensionCommands();
	const tools = extensionTools();
	runtimeState.commands = [...commands, ...(message.state?.commands ?? [])];
	reply(message.id, {
		extensionCount: loaded.extensions.length,
		commands,
		tools,
		skillPaths: resources.skillPaths,
		promptPaths: resources.promptPaths,
		themePaths: resources.themePaths,
		diagnostics,
	});
	appRequestsReady = true;
}

async function handleExecuteCommand(message) {
	if (!runner) {
		reply(message.id, { handled: false });
		return;
	}
	const command = runner.getCommand(message.name);
	if (!command) {
		reply(message.id, { handled: false });
		return;
	}
	const previousActionPromises = commandActionPromises;
	const previousActionChain = commandActionChain;
	commandActionPromises = [];
	commandActionChain = Promise.resolve();
	try {
		await command.handler(message.args ?? "", runner.createCommandContext());
		await Promise.all(commandActionPromises);
		reply(message.id, { handled: true });
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	} finally {
		commandActionPromises = previousActionPromises;
		commandActionChain = previousActionChain;
	}
}

async function handleBeforeSwitch(message) {
	if (!runner) {
		reply(message.id, { cancelled: false });
		return;
	}
	try {
		const result = await runner.emit({
			type: "session_before_switch",
			reason: message.reason ?? "new",
			targetSessionFile: message.targetSessionFile,
		});
		reply(message.id, { cancelled: Boolean(result?.cancel) });
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleToolCall(message) {
	if (!runner || !runner.hasHandlers("tool_call")) {
		reply(message.id, null);
		return;
	}
	try {
		const result = await runner.emitToolCall({
			type: "tool_call",
			toolName: message.toolName,
			toolCallId: message.toolCallId,
			input: message.input ?? {},
		});
		reply(message.id, result ?? null);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleExecuteTool(message) {
	if (!runner) {
		replyError(message.id, "Extension runtime unavailable");
		return;
	}
	const tool = runner.getAllRegisteredTools().find((entry) => entry.definition.name === message.toolName);
	if (!tool) {
		replyError(message.id, `Unknown extension tool: ${message.toolName}`);
		return;
	}
	try {
		const result = await tool.definition.execute(
			message.toolCallId,
			message.args ?? {},
			undefined,
			undefined,
			runner.createContext(),
		);
		reply(message.id, result);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleToolResult(message) {
	if (!runner || !runner.hasHandlers("tool_result")) {
		reply(message.id, null);
		return;
	}
	try {
		const result = await runner.emitToolResult({
			type: "tool_result",
			toolName: message.toolName,
			toolCallId: message.toolCallId,
			input: message.input ?? {},
			content: message.content ?? [],
			details: message.details,
			isError: Boolean(message.isError),
		});
		reply(message.id, result ?? null);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleInput(message) {
	if (!runner || !runner.hasHandlers("input")) {
		reply(message.id, { action: "continue" });
		return;
	}
	try {
		const result = await runner.emitInput(message.text ?? "", message.images, message.source ?? "rpc");
		reply(message.id, result);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleBeforeProviderRequest(message) {
	if (!runner || !runner.hasHandlers("before_provider_request")) {
		reply(message.id, message.payload ?? null);
		return;
	}
	try {
		const payload = await runner.emitBeforeProviderRequest(message.payload);
		reply(message.id, payload ?? null);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleUiResponse(message) {
	const pending = pendingUiRequests.get(message.response?.id);
	if (pending) {
		pendingUiRequests.delete(message.response.id);
		pending.resolve(message.response);
	}
}

async function handleEvent(message) {
	if (!runner) {
		return;
	}
	if (message.event?.type === "turn_start") {
		runtimeState.isIdle = false;
	}
	if (message.event?.type === "agent_end") {
		runtimeState.isIdle = true;
	}
	await runner.emit(message.event);
}

async function handleShutdown(message) {
	if (runner) {
		await runner.emit({ type: "session_shutdown" });
	}
	reply(message.id, { ok: true });
	process.exit(0);
}

async function handleMessage(rawLine) {
	let message;
	try {
		message = JSON.parse(rawLine);
	} catch (error) {
		send({
			type: "extension_error",
			extensionPath: "<runtime>",
			event: "parse",
			error: error instanceof Error ? error.message : String(error),
		});
		return;
	}

	if (!message || typeof message !== "object") {
		return;
	}

	applyRuntimeState(message.state);

	try {
		switch (message.type) {
			case "app_response":
				resolveAppResponse(message);
				break;
			case "init":
				await handleInit(message);
				break;
			case "update_state":
				reply(message.id, { ok: true });
				break;
			case "execute_command":
				await handleExecuteCommand(message);
				break;
			case "before_switch":
				await handleBeforeSwitch(message);
				break;
			case "tool_call":
				await handleToolCall(message);
				break;
			case "execute_tool":
				await handleExecuteTool(message);
				break;
			case "tool_result":
				await handleToolResult(message);
				break;
			case "input":
				await handleInput(message);
				break;
			case "before_provider_request":
				await handleBeforeProviderRequest(message);
				break;
			case "ui_response":
				await handleUiResponse(message);
				break;
			case "event":
				await handleEvent(message);
				break;
			case "shutdown":
				await handleShutdown(message);
				break;
			default:
				replyError(message.id, `Unknown sidecar message: ${message.type}`);
		}
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

const rl = createInterface({ input: process.stdin, crlfDelay: Number.POSITIVE_INFINITY });
rl.on("line", (line) => {
	const trimmed = line.trim();
	if (!trimmed) {
		return;
	}
	void handleMessage(trimmed);
});
rl.on("close", () => {
	process.exit(0);
});
