import { createInterface } from "node:readline";
import { ExtensionRunner, discoverAndLoadExtensions } from "./extension-runtime/index.mjs";

let runner;
let uiCounter = 0;
let appRequestCounter = 0;
let commandActionPromises = null;
let commandActionChain = null;
let appRequestsReady = false;
let resolvedKeybindings = {};
let resolvedShortcuts = new Map();
let shortcutExecutionChain = Promise.resolve();
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

const DEFAULT_UI_VIEWPORT = { width: 80, height: 24 };
const MAX_WIDGET_LINES = 10;
const IDENTITY_SELECT_LIST_THEME = {
	selectedPrefix: (text) => text,
	selectedText: (text) => text,
	description: (text) => text,
	scrollInfo: (text) => text,
	noMatch: (text) => text,
};
const IDENTITY_EDITOR_THEME = {
	borderColor: (text) => text,
	selectList: IDENTITY_SELECT_LIST_THEME,
};
const IDENTITY_THEME = {
	fg(_name, text) {
		return text;
	},
	bg(_name, text) {
		return text;
	},
	bold(text) {
		return text;
	},
	italic(text) {
		return text;
	},
	underline(text) {
		return text;
	},
	strikethrough(text) {
		return text;
	},
};

let headerComponentState = null;
let footerComponentState = null;
const widgetComponentStates = new Map();
let editorComponentState = null;

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

function fireAndTrackAsyncAction(event, action) {
	const previous = commandActionChain ?? Promise.resolve();
	const tracked = previous
		.then(action)
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

async function requestUiViewport() {
	if (!appRequestsReady) {
		return DEFAULT_UI_VIEWPORT;
	}
	try {
		const viewport = await requestHost("get_ui_viewport");
		return {
			width: typeof viewport?.width === "number" ? viewport.width : DEFAULT_UI_VIEWPORT.width,
			height: typeof viewport?.height === "number" ? viewport.height : DEFAULT_UI_VIEWPORT.height,
		};
	} catch {
		return DEFAULT_UI_VIEWPORT;
	}
}

async function requestFooterDataSnapshot() {
	if (!appRequestsReady) {
		return {
			cwd: "",
			gitBranch: undefined,
			availableProviderCount: 0,
			extensionStatuses: {},
		};
	}
	try {
		const snapshot = await requestHost("get_footer_data");
		return {
			cwd: typeof snapshot?.cwd === "string" ? snapshot.cwd : "",
			gitBranch: typeof snapshot?.gitBranch === "string" ? snapshot.gitBranch : undefined,
			availableProviderCount:
				typeof snapshot?.availableProviderCount === "number" ? snapshot.availableProviderCount : 0,
			extensionStatuses:
				snapshot?.extensionStatuses && typeof snapshot.extensionStatuses === "object"
					? { ...snapshot.extensionStatuses }
					: {},
		};
	} catch {
		return {
			cwd: "",
			gitBranch: undefined,
			availableProviderCount: 0,
			extensionStatuses: {},
		};
	}
}

function createFooterDataProxy(snapshot) {
	return {
		getGitBranch() {
			return snapshot.gitBranch;
		},
		getExtensionStatuses() {
			return { ...snapshot.extensionStatuses };
		},
		onBranchChange() {
			return () => {};
		},
	};
}

function normalizeRenderedLines(lines, maxLines) {
	if (!Array.isArray(lines)) {
		return [];
	}

	const normalized = lines.map((line) => (typeof line === "string" ? line : String(line ?? "")));
	if (typeof maxLines === "number" && normalized.length > maxLines) {
		return [...normalized.slice(0, maxLines), "... (widget truncated)"];
	}
	return normalized;
}

function getEditorComponentText(component) {
	if (!component || typeof component !== "object") {
		return "";
	}
	if (typeof component.getExpandedText === "function") {
		return component.getExpandedText();
	}
	if (typeof component.getText === "function") {
		return component.getText();
	}
	return "";
}

async function renderComponentLines(component, viewport, maxLines) {
	if (!component || typeof component.render !== "function") {
		return [];
	}
	if (typeof component.setViewportSize === "function") {
		component.setViewportSize(viewport.width, viewport.height);
	}
	return normalizeRenderedLines(component.render(viewport.width), maxLines);
}

function disposeComponentState(state) {
	try {
		state?.component?.dispose?.();
	} catch {
		// Ignore disposal errors from extension-owned components.
	}
}

function emitUiRequest(method, payload = {}) {
	send({
		type: "extension_ui_request",
		id: nextUiId(),
		method,
		...payload,
	});
}

function createFakeTui(onRenderRequested, viewport) {
	return {
		requestRender() {
			onRenderRequested?.();
		},
		terminal: {
			rows: viewport.height,
			columns: viewport.width,
		},
	};
}

function matchesResolvedKeybinding(data, keybinding) {
	const normalized = typeof keybinding === "string" ? keybinding.toLowerCase() : "";
	if (!normalized) {
		return false;
	}

	if (normalized === "escape") {
		return data === "\x1b";
	}
	if (normalized === "enter") {
		return data === "\r" || data === "\n";
	}
	if (normalized === "tab") {
		return data === "\t";
	}
	if (normalized === "shift+tab") {
		return data === "\x1b[Z";
	}
	if (normalized.startsWith("ctrl+") && normalized.length === 6) {
		const character = normalized.slice(5);
		if (character >= "a" && character <= "z") {
			return data === String.fromCharCode(character.charCodeAt(0) - 96);
		}
	}
	if (normalized.startsWith("alt+") && normalized.length === 5) {
		return data === `\x1b${normalized.slice(4)}`;
	}
	if (normalized.length === 1) {
		return data === normalized;
	}
	return false;
}

function createKeybindingsProxy() {
	return {
		matches(data, action) {
			const configured = resolvedKeybindings?.[action];
			const keys = Array.isArray(configured) ? configured : configured ? [configured] : [];
			return keys.some((keybinding) => matchesResolvedKeybinding(data, keybinding));
		},
	};
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
			emitUiRequest("notify", { message, notifyType: type });
		},
		onTerminalInput() {
			return () => {};
		},
		setStatus(key, text) {
			emitUiRequest("setStatus", {
				statusKey: key,
				statusText: text,
			});
		},
		setWorkingMessage() {},
		setHiddenThinkingLabel() {},
		setWidget(key, content, options) {
			disposeComponentState(widgetComponentStates.get(key));
			widgetComponentStates.delete(key);
			const placement = options?.placement;

			if (content === undefined) {
				emitUiRequest("setWidget", {
					widgetKey: key,
					widgetPlacement: placement,
				});
				return;
			}

			if (Array.isArray(content)) {
				emitUiRequest("setWidget", {
					widgetKey: key,
					widgetLines: normalizeRenderedLines(content, MAX_WIDGET_LINES),
					widgetPlacement: placement,
				});
				return;
			}

			fireAndTrackAsyncAction("set_widget", async () => {
				const viewport = await requestUiViewport();
				const state = { component: undefined, placement };
				const fakeTui = createFakeTui(() => {
					void (async () => {
						if (widgetComponentStates.get(key) !== state) {
							return;
						}
						const nextViewport = await requestUiViewport();
						const widgetLines = await renderComponentLines(state.component, nextViewport, MAX_WIDGET_LINES);
						emitUiRequest("setWidget", {
							widgetKey: key,
							widgetLines,
							widgetPlacement: placement,
						});
					})();
				}, viewport);
				state.component = await content(fakeTui, IDENTITY_THEME);
				widgetComponentStates.set(key, state);
				const widgetLines = await renderComponentLines(state.component, viewport, MAX_WIDGET_LINES);
				emitUiRequest("setWidget", {
					widgetKey: key,
					widgetLines,
					widgetPlacement: placement,
				});
			});
		},
		setFooter(factory) {
			disposeComponentState(footerComponentState);
			footerComponentState = null;
			if (!factory) {
				emitUiRequest("setFooter");
				return;
			}

			fireAndTrackAsyncAction("set_footer", async () => {
				const viewport = await requestUiViewport();
				const footerData = createFooterDataProxy(await requestFooterDataSnapshot());
				const state = { component: undefined };
				const fakeTui = createFakeTui(() => {
					void (async () => {
						if (footerComponentState !== state) {
							return;
						}
						const nextViewport = await requestUiViewport();
						const footerLines = await renderComponentLines(state.component, nextViewport);
						emitUiRequest("setFooter", { footerLines });
					})();
				}, viewport);
				state.component = await factory(fakeTui, IDENTITY_THEME, footerData);
				footerComponentState = state;
				const footerLines = await renderComponentLines(state.component, viewport);
				emitUiRequest("setFooter", { footerLines });
			});
		},
		setHeader(factory) {
			disposeComponentState(headerComponentState);
			headerComponentState = null;
			if (!factory) {
				emitUiRequest("setHeader");
				return;
			}

			fireAndTrackAsyncAction("set_header", async () => {
				const viewport = await requestUiViewport();
				const state = { component: undefined };
				const fakeTui = createFakeTui(() => {
					void (async () => {
						if (headerComponentState !== state) {
							return;
						}
						const nextViewport = await requestUiViewport();
						const headerLines = await renderComponentLines(state.component, nextViewport);
						emitUiRequest("setHeader", { headerLines });
					})();
				}, viewport);
				state.component = await factory(fakeTui, IDENTITY_THEME);
				headerComponentState = state;
				const headerLines = await renderComponentLines(state.component, viewport);
				emitUiRequest("setHeader", { headerLines });
			});
		},
		setTitle(title) {
			emitUiRequest("setTitle", { title });
		},
		async custom() {
			return undefined;
		},
		pasteToEditor(text) {
			this.setEditorText(text);
		},
		setEditorText(text) {
			emitUiRequest("set_editor_text", { text });
			if (editorComponentState?.component && typeof editorComponentState.component.setText === "function") {
				fireAndTrackAsyncAction("set_editor_text", async () => {
					editorComponentState.component.setText(text);
					const viewport = await requestUiViewport();
					const editorLines = await renderComponentLines(editorComponentState.component, viewport);
					emitUiRequest("setEditorComponent", { editorLines });
				});
			}
		},
		getEditorText() {
			return getEditorComponentText(editorComponentState?.component);
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
		setEditorComponent(factory) {
			const previousEditor = editorComponentState;
			if (!factory) {
				const currentText = getEditorComponentText(previousEditor?.component);
				disposeComponentState(previousEditor);
				editorComponentState = null;
				if (currentText) {
					emitUiRequest("set_editor_text", { text: currentText });
				}
				emitUiRequest("setEditorComponent");
				return;
			}

			fireAndTrackAsyncAction("set_editor_component", async () => {
				disposeComponentState(previousEditor);
				const viewport = await requestUiViewport();
				let currentText = getEditorComponentText(previousEditor?.component);
				if (!currentText && appRequestsReady) {
					try {
						const editorText = await requestHost("get_editor_text");
						if (typeof editorText === "string") {
							currentText = editorText;
						}
					} catch {
						// Ignore host lookup errors and fall back to an empty editor.
					}
				}
				const state = { component: undefined, submittedText: undefined };
				const fakeTui = createFakeTui(() => {
					void (async () => {
						if (editorComponentState !== state) {
							return;
						}
						const nextViewport = await requestUiViewport();
						const editorLines = await renderComponentLines(state.component, nextViewport);
						emitUiRequest("setEditorComponent", { editorLines });
					})();
				}, viewport);
				state.component = await factory(fakeTui, IDENTITY_EDITOR_THEME, createKeybindingsProxy());
				if (typeof state.component.setText === "function") {
					state.component.setText(currentText);
				}
				state.component.onEscape = () => {
					fireAndTrackHostRequest("ui_editor_action", { action: "interrupt" }, "ui_editor_action");
				};
				state.component.onCtrlD = () => {
					fireAndTrackHostRequest("ui_editor_action", { action: "exit" }, "ui_editor_action");
				};
				state.component.onSubmit = (value) => {
					state.submittedText = value;
					fireAndTrackHostRequest("ui_editor_submit", { text: value }, "ui_editor_submit");
				};
				editorComponentState = state;
				const editorLines = await renderComponentLines(state.component, viewport);
				emitUiRequest("setEditorComponent", { editorLines });
			});
		},
		get theme() {
			return IDENTITY_THEME;
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

function cloneJsonValue(value) {
	if (value === undefined) {
		return undefined;
	}
	return JSON.parse(JSON.stringify(value));
}

function serializeProviderConfig(config) {
	if (!config || typeof config !== "object" || Array.isArray(config)) {
		return { ok: false, error: "Provider config must be an object" };
	}
	if (typeof config.streamSimple === "function") {
		return {
			ok: false,
			error: "streamSimple is not supported in the Rust RPC extension bridge yet",
		};
	}
	if (config.oauth !== undefined) {
		return {
			ok: false,
			error: "OAuth provider registration is not supported in the Rust RPC extension bridge yet",
		};
	}

	const normalized = {};
	if (typeof config.baseUrl === "string") {
		normalized.baseUrl = config.baseUrl;
	}
	if (typeof config.apiKey === "string") {
		normalized.apiKey = config.apiKey;
	}
	if (typeof config.api === "string") {
		normalized.api = config.api;
	}
	if (config.headers && typeof config.headers === "object" && !Array.isArray(config.headers)) {
		normalized.headers = cloneJsonValue(config.headers);
	}
	if (typeof config.authHeader === "boolean") {
		normalized.authHeader = config.authHeader;
	}
	if (Array.isArray(config.models)) {
		normalized.models = cloneJsonValue(config.models);
	}

	return { ok: true, config: normalized };
}

function createProviderRegisterMutation(name, config, diagnostics) {
	if (typeof name !== "string" || name.trim().length === 0) {
		diagnostics?.push({
			level: "warning",
			message: "Extension provider registration skipped: provider name must be a non-empty string",
		});
		return undefined;
	}

	const serialized = serializeProviderConfig(config);
	if (!serialized.ok) {
		diagnostics?.push({
			level: "warning",
			message: `Extension provider registration skipped for ${name}: ${serialized.error}`,
		});
		return undefined;
	}

	return {
		action: "register",
		name,
		config: serialized.config,
	};
}

function bindRunner(loaded, cwd, providerMutations, diagnostics) {
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
			compact(options) {
				const promise = requestHost("compact", {
					customInstructions: options?.customInstructions,
				})
					.then((result) => {
						options?.onComplete?.(result);
						return result;
					})
					.catch((error) => {
						const resolvedError = error instanceof Error ? error : new Error(String(error));
						options?.onError?.(resolvedError);
						emitRuntimeError("compact", resolvedError.message);
						return undefined;
					});
				trackCommandAction(promise);
			},
			getSystemPrompt() {
				return runtimeState.systemPrompt;
			},
		},
		{
			registerProvider(name, config) {
				const mutation = createProviderRegisterMutation(
					name,
					config,
					appRequestsReady ? undefined : diagnostics,
				);
				if (!mutation) {
					if (appRequestsReady) {
						emitRuntimeError(
							"register_provider",
							`Provider registration skipped for ${String(name)}`,
						);
					}
					return;
				}
				if (!appRequestsReady) {
					providerMutations.push(mutation);
					return;
				}
				fireAndTrackHostRequest(
					"register_provider",
					{ name: mutation.name, config: mutation.config },
					"register_provider",
				);
			},
			unregisterProvider(name) {
				if (typeof name !== "string" || name.trim().length === 0) {
					if (appRequestsReady) {
						emitRuntimeError(
							"unregister_provider",
							"Provider name must be a non-empty string",
						);
					} else {
						diagnostics?.push({
							level: "warning",
							message: "Extension provider unregistration skipped: provider name must be a non-empty string",
						});
					}
					return;
				}
				const mutation = { action: "unregister", name };
				if (!appRequestsReady) {
					providerMutations.push(mutation);
					return;
				}
				fireAndTrackHostRequest(
					"unregister_provider",
					{ name: mutation.name },
					"unregister_provider",
				);
			},
		},
	);
	runner.bindCommandContext({
		async waitForIdle() {
			await requestHost("wait_for_idle");
		},
		async newSession(options) {
			const before = runner?.hasHandlers("session_before_switch")
				? await runner.emit({ type: "session_before_switch", reason: "new" })
				: undefined;
			if (before?.cancel) {
				return { cancelled: true };
			}
			const result = (await requestHost("new_session", { options })) ?? { cancelled: false };
			if (!result?.cancelled && runner?.hasHandlers("session_shutdown")) {
				await runner.emit({ type: "session_shutdown" });
			}
			return result;
		},
		async fork(entryId) {
			const before = runner?.hasHandlers("session_before_fork")
				? await runner.emit({ type: "session_before_fork", entryId })
				: undefined;
			if (before?.cancel) {
				return { cancelled: true };
			}
			const result = (await requestHost("fork", { entryId })) ?? { cancelled: false };
			if (!result?.cancelled && runner?.hasHandlers("session_shutdown")) {
				await runner.emit({ type: "session_shutdown" });
			}
			return result;
		},
		async navigateTree(targetId, options) {
			const result = await requestHost("navigate_tree", { targetId, options });
			return result ?? { cancelled: false };
		},
		async switchSession(sessionPath) {
			const before = runner?.hasHandlers("session_before_switch")
				? await runner.emit({
						type: "session_before_switch",
						reason: "resume",
						targetSessionFile: sessionPath,
					})
				: undefined;
			if (before?.cancel) {
				return { cancelled: true };
			}
			const result = (await requestHost("switch_session", { sessionPath })) ?? {
				cancelled: false,
			};
			if (!result?.cancelled && runner?.hasHandlers("session_shutdown")) {
				await runner.emit({ type: "session_shutdown" });
			}
			return result;
		},
		async reload() {
			await requestHost("reload");
			if (runner?.hasHandlers("session_shutdown")) {
				await runner.emit({ type: "session_shutdown" });
			}
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

function extensionShortcuts() {
	return Array.from(resolvedShortcuts.values()).map((shortcut) => ({
		shortcut: shortcut.shortcut,
		description: shortcut.description,
		extensionPath: shortcut.extensionPath,
	}));
}

async function runTrackedHostAction(handler) {
	const previousActionPromises = commandActionPromises;
	const previousActionChain = commandActionChain;
	commandActionPromises = [];
	commandActionChain = Promise.resolve();
	try {
		await handler();
		await Promise.all(commandActionPromises);
	} finally {
		commandActionPromises = previousActionPromises;
		commandActionChain = previousActionChain;
	}
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
	resolvedKeybindings = message.keybindings ?? {};
	resolvedShortcuts = new Map();
	const loaded = message.noExtensions
		? { extensions: [], errors: [], runtime: { flagValues: new Map() } }
		: await discoverAndLoadExtensions(
				message.extensions ?? [],
				message.cwd,
				message.agentDir ?? undefined,
			);
	const diagnostics = [...loadDiagnostics(loaded.errors)];
	const providerMutations = [];
	if (!loaded.extensions || loaded.extensions.length === 0) {
		reply(message.id, {
			extensionCount: 0,
			commands: [],
			tools: [],
			shortcuts: [],
			skillPaths: [],
			promptPaths: [],
			themePaths: [],
			providerMutations,
			diagnostics,
		});
		return;
	}

	diagnostics.push(...applyExtensionFlagValues(message.flagValues, loaded));
	bindRunner(loaded, message.cwd, providerMutations, diagnostics);
	// Mirror TypeScript runtime behavior before session_start runs so load-time
	// extension tools appear in getAllTools()/getActiveTools() immediately.
	updateRuntimeToolStateFromExtensionTools(extensionTools());
	resolvedShortcuts = runner.getShortcuts(resolvedKeybindings);
	diagnostics.push(...runner.getShortcutDiagnostics().map(({ level, message: text }) => ({ level, message: text })));

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
	const shortcuts = extensionShortcuts();
	runtimeState.commands = [...commands, ...(message.state?.commands ?? [])];
	reply(message.id, {
		extensionCount: loaded.extensions.length,
		commands,
		tools,
		shortcuts,
		skillPaths: resources.skillPaths,
		promptPaths: resources.promptPaths,
		themePaths: resources.themePaths,
		providerMutations,
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
	try {
		await runTrackedHostAction(async () => {
			await command.handler(message.args ?? "", runner.createCommandContext());
		});
		reply(message.id, { handled: true });
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleExecuteShortcut(message) {
	if (!runner) {
		reply(message.id, { handled: false });
		return;
	}
	const shortcutKey = typeof message.shortcut === "string" ? message.shortcut.toLowerCase() : "";
	const shortcut = resolvedShortcuts.get(shortcutKey);
	if (!shortcut) {
		reply(message.id, { handled: false });
		return;
	}

	const currentExecution = shortcutExecutionChain.then(() =>
		runTrackedHostAction(async () => {
			await shortcut.handler(runner.createContext());
		}),
	);
	shortcutExecutionChain = currentExecution.catch(() => undefined);

	try {
		await currentExecution;
		reply(message.id, { handled: true });
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
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

async function handleBeforeFork(message) {
	if (!runner || !runner.hasHandlers("session_before_fork")) {
		reply(message.id, null);
		return;
	}
	try {
		const result = await runner.emit({
			type: "session_before_fork",
			entryId: message.entryId,
		});
		reply(message.id, result ?? null);
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

async function handleBeforeAgentStart(message) {
	if (!runner || !runner.hasHandlers("before_agent_start")) {
		reply(message.id, null);
		return;
	}
	try {
		const result = await runner.emitBeforeAgentStart(
			message.prompt ?? "",
			message.images ?? [],
			message.systemPrompt ?? runtimeState.systemPrompt ?? "",
		);
		reply(message.id, result ? {
			messages: result.messages ?? [],
			systemPrompt: result.systemPrompt,
		} : null);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleBeforeCompact(message) {
	if (!runner || !runner.hasHandlers("session_before_compact")) {
		reply(message.id, null);
		return;
	}
	try {
		const controller = new AbortController();
		const result = await runner.emit({
			type: "session_before_compact",
			preparation: message.preparation,
			branchEntries: message.branchEntries ?? [],
			customInstructions: message.customInstructions,
			signal: controller.signal,
		});
		reply(message.id, result ?? null);
	} catch (error) {
		replyError(message.id, error instanceof Error ? error.message : String(error));
	}
}

async function handleBeforeTree(message) {
	if (!runner || !runner.hasHandlers("session_before_tree")) {
		reply(message.id, null);
		return;
	}
	try {
		const controller = new AbortController();
		const result = await runner.emit({
			type: "session_before_tree",
			preparation: message.preparation,
			signal: controller.signal,
		});
		reply(message.id, result ?? null);
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

async function handleEditorComponentInput(message) {
	if (!editorComponentState?.component) {
		reply(message.id, { lines: [], text: "" });
		return;
	}

	const viewport = {
		width: typeof message.width === "number" ? message.width : DEFAULT_UI_VIEWPORT.width,
		height: typeof message.height === "number" ? message.height : DEFAULT_UI_VIEWPORT.height,
	};
	if (typeof editorComponentState.component.setViewportSize === "function") {
		editorComponentState.component.setViewportSize(viewport.width, viewport.height);
	}
	if (typeof editorComponentState.component.handleInput === "function") {
		editorComponentState.submittedText = undefined;
		editorComponentState.component.handleInput(typeof message.data === "string" ? message.data : "");
		if (editorComponentState.submittedText !== undefined && typeof editorComponentState.component.setText === "function") {
			editorComponentState.component.setText("");
			editorComponentState.submittedText = undefined;
		}
	}

	const lines = await renderComponentLines(editorComponentState.component, viewport);
	reply(message.id, {
		lines,
		text: getEditorComponentText(editorComponentState.component),
	});
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
			case "execute_shortcut":
				await handleExecuteShortcut(message);
				break;
			case "before_switch":
				await handleBeforeSwitch(message);
				break;
			case "before_fork":
				await handleBeforeFork(message);
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
			case "before_agent_start":
				await handleBeforeAgentStart(message);
				break;
			case "before_compact":
				await handleBeforeCompact(message);
				break;
			case "before_tree":
				await handleBeforeTree(message);
				break;
			case "before_provider_request":
				await handleBeforeProviderRequest(message);
				break;
			case "editor_component_input":
				await handleEditorComponentInput(message);
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
