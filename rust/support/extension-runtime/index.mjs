import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import * as fs from "node:fs";
import { createRequire } from "node:module";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { createJiti } from "@mariozechner/jiti";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, "../../..");
const require = createRequire(import.meta.url);

const WORKSPACE_ALIASES = {
	"@mariozechner/pi-agent-core": path.join(REPO_ROOT, "packages/agent/src/index.ts"),
	"@mariozechner/pi-ai": path.join(REPO_ROOT, "packages/ai/src/index.ts"),
	"@mariozechner/pi-ai/oauth": path.join(REPO_ROOT, "packages/ai/src/oauth.ts"),
	"@mariozechner/pi-coding-agent": path.join(REPO_ROOT, "packages/coding-agent/src/index.ts"),
	"@mariozechner/pi-tui": path.join(REPO_ROOT, "packages/tui/src/index.ts"),
};

const SOURCE_INFO_DEFAULTS = {
	scope: "temporary",
	origin: "top-level",
};

const UNICODE_SPACES = /[\u00A0\u2000-\u200A\u202F\u205F\u3000]/g;

function normalizeUnicodeSpaces(value) {
	return value.replace(UNICODE_SPACES, " ");
}

function expandPath(value) {
	const normalized = normalizeUnicodeSpaces(value);
	if (normalized.startsWith("~/")) {
		return path.join(os.homedir(), normalized.slice(2));
	}
	if (normalized === "~") {
		return os.homedir();
	}
	if (normalized.startsWith("~")) {
		return path.join(os.homedir(), normalized.slice(1));
	}
	return normalized;
}

function resolvePath(value, cwd) {
	const expanded = expandPath(value);
	if (path.isAbsolute(expanded)) {
		return expanded;
	}
	return path.resolve(cwd, expanded);
}

function getAgentDir() {
	const envDir = process.env.PI_CODING_AGENT_DIR;
	if (envDir) {
		return expandPath(envDir);
	}
	return path.join(os.homedir(), ".pi", "agent");
}

function createEventBus() {
	const emitter = new EventEmitter();
	return {
		emit(channel, data) {
			emitter.emit(channel, data);
		},
		on(channel, handler) {
			const safeHandler = async (data) => {
				try {
					await handler(data);
				} catch (error) {
					console.error(`Event handler error (${channel}):`, error);
				}
			};
			emitter.on(channel, safeHandler);
			return () => emitter.off(channel, safeHandler);
		},
		clear() {
			emitter.removeAllListeners();
		},
	};
}

function createSyntheticSourceInfo(filePath, options) {
	return {
		path: filePath,
		source: options.source,
		scope: options.scope ?? SOURCE_INFO_DEFAULTS.scope,
		origin: options.origin ?? SOURCE_INFO_DEFAULTS.origin,
		baseDir: options.baseDir,
	};
}

async function execCommand(command, args, cwd, options) {
	return new Promise((resolve) => {
		const proc = spawn(command, args, {
			cwd,
			shell: false,
			stdio: ["ignore", "pipe", "pipe"],
		});

		let stdout = "";
		let stderr = "";
		let killed = false;
		let timeoutId;

		const killProcess = () => {
			if (killed) {
				return;
			}
			killed = true;
			proc.kill("SIGTERM");
			setTimeout(() => {
				if (!proc.killed) {
					proc.kill("SIGKILL");
				}
			}, 5000);
		};

		if (options?.signal) {
			if (options.signal.aborted) {
				killProcess();
			} else {
				options.signal.addEventListener("abort", killProcess, { once: true });
			}
		}

		if (options?.timeout && options.timeout > 0) {
			timeoutId = setTimeout(() => {
				killProcess();
			}, options.timeout);
		}

		proc.stdout?.on("data", (data) => {
			stdout += data.toString();
		});

		proc.stderr?.on("data", (data) => {
			stderr += data.toString();
		});

		const finish = (code) => {
			if (timeoutId) {
				clearTimeout(timeoutId);
			}
			if (options?.signal) {
				options.signal.removeEventListener("abort", killProcess);
			}
			resolve({
				stdout,
				stderr,
				code: code ?? 0,
				killed,
			});
		};

		proc.once("error", () => finish(1));
		proc.once("close", finish);
	});
}

function createExtensionRuntime() {
	const notInitialized = () => {
		throw new Error("Extension runtime not initialized. Action methods cannot be called during extension loading.");
	};

	const runtime = {
		sendMessage: notInitialized,
		sendUserMessage: notInitialized,
		appendEntry: notInitialized,
		setSessionName: notInitialized,
		getSessionName: notInitialized,
		setLabel: notInitialized,
		getActiveTools: notInitialized,
		getAllTools: notInitialized,
		setActiveTools: notInitialized,
		refreshTools: () => {},
		getCommands: notInitialized,
		setModel: () => Promise.reject(new Error("Extension runtime not initialized")),
		getThinkingLevel: notInitialized,
		setThinkingLevel: notInitialized,
		flagValues: new Map(),
		pendingProviderRegistrations: [],
		registerProvider(name, config, extensionPath = "<unknown>") {
			runtime.pendingProviderRegistrations.push({ name, config, extensionPath });
		},
		unregisterProvider(name) {
			runtime.pendingProviderRegistrations = runtime.pendingProviderRegistrations.filter(
				(registration) => registration.name !== name,
			);
		},
	};

	return runtime;
}

function createExtension(extensionPath, resolvedPath) {
	const source =
		extensionPath.startsWith("<") && extensionPath.endsWith(">")
			? extensionPath.slice(1, -1).split(":")[0] || "temporary"
			: "local";
	const baseDir = extensionPath.startsWith("<") ? undefined : path.dirname(resolvedPath);

	return {
		path: extensionPath,
		resolvedPath,
		sourceInfo: createSyntheticSourceInfo(extensionPath, { source, baseDir }),
		handlers: new Map(),
		tools: new Map(),
		messageRenderers: new Map(),
		commands: new Map(),
		flags: new Map(),
		shortcuts: new Map(),
	};
}

let aliases;
function getAliases() {
	if (aliases) {
		return aliases;
	}

	const resolveWorkspaceEntry = (workspacePath, specifier) => {
		if (fs.existsSync(workspacePath)) {
			return workspacePath;
		}
		return fileURLToPath(import.meta.resolve(specifier));
	};

	aliases = {
		"@mariozechner/pi-agent-core": resolveWorkspaceEntry(
			WORKSPACE_ALIASES["@mariozechner/pi-agent-core"],
			"@mariozechner/pi-agent-core",
		),
		"@mariozechner/pi-ai": resolveWorkspaceEntry(WORKSPACE_ALIASES["@mariozechner/pi-ai"], "@mariozechner/pi-ai"),
		"@mariozechner/pi-ai/oauth": resolveWorkspaceEntry(
			WORKSPACE_ALIASES["@mariozechner/pi-ai/oauth"],
			"@mariozechner/pi-ai/oauth",
		),
		"@mariozechner/pi-coding-agent": resolveWorkspaceEntry(
			WORKSPACE_ALIASES["@mariozechner/pi-coding-agent"],
			"@mariozechner/pi-coding-agent",
		),
		"@mariozechner/pi-tui": resolveWorkspaceEntry(
			WORKSPACE_ALIASES["@mariozechner/pi-tui"],
			"@mariozechner/pi-tui",
		),
	};

	const typeboxEntry = require.resolve("@sinclair/typebox");
	aliases["@sinclair/typebox"] = typeboxEntry.replace(/[\\/]build[\\/]cjs[\\/]index\.js$/, "");

	return aliases;
}

function createExtensionAPI(extension, runtime, cwd, eventBus) {
	return {
		on(event, handler) {
			const handlers = extension.handlers.get(event) ?? [];
			handlers.push(handler);
			extension.handlers.set(event, handlers);
		},
		registerTool(tool) {
			extension.tools.set(tool.name, {
				definition: tool,
				sourceInfo: extension.sourceInfo,
			});
			runtime.refreshTools();
		},
		registerCommand(name, options) {
			extension.commands.set(name, {
				name,
				sourceInfo: extension.sourceInfo,
				...options,
			});
		},
		registerShortcut(shortcut, options) {
			extension.shortcuts.set(shortcut, {
				shortcut,
				extensionPath: extension.path,
				...options,
			});
		},
		registerFlag(name, options) {
			extension.flags.set(name, {
				name,
				extensionPath: extension.path,
				...options,
			});
			if (options.default !== undefined && !runtime.flagValues.has(name)) {
				runtime.flagValues.set(name, options.default);
			}
		},
		registerMessageRenderer(customType, renderer) {
			extension.messageRenderers.set(customType, renderer);
		},
		getFlag(name) {
			if (!extension.flags.has(name)) {
				return undefined;
			}
			return runtime.flagValues.get(name);
		},
		sendMessage(message, options) {
			runtime.sendMessage(message, options);
		},
		sendUserMessage(content, options) {
			runtime.sendUserMessage(content, options);
		},
		appendEntry(customType, data) {
			runtime.appendEntry(customType, data);
		},
		setSessionName(name) {
			runtime.setSessionName(name);
		},
		getSessionName() {
			return runtime.getSessionName();
		},
		setLabel(entryId, label) {
			runtime.setLabel(entryId, label);
		},
		exec(command, args, options) {
			return execCommand(command, args, options?.cwd ?? cwd, options);
		},
		getActiveTools() {
			return runtime.getActiveTools();
		},
		getAllTools() {
			return runtime.getAllTools();
		},
		setActiveTools(toolNames) {
			runtime.setActiveTools(toolNames);
		},
		getCommands() {
			return runtime.getCommands();
		},
		setModel(model) {
			return runtime.setModel(model);
		},
		getThinkingLevel() {
			return runtime.getThinkingLevel();
		},
		setThinkingLevel(level) {
			runtime.setThinkingLevel(level);
		},
		registerProvider(name, config) {
			runtime.registerProvider(name, config, extension.path);
		},
		unregisterProvider(name) {
			runtime.unregisterProvider(name, extension.path);
		},
		events: eventBus,
	};
}

async function loadExtensionModule(extensionPath) {
	const jiti = createJiti(import.meta.url, {
		moduleCache: false,
		alias: getAliases(),
	});

	const moduleValue = await jiti.import(extensionPath, { default: true });
	return typeof moduleValue === "function" ? moduleValue : undefined;
}

async function loadExtension(extensionPath, cwd, eventBus, runtime) {
	const resolvedPath = resolvePath(extensionPath, cwd);

	try {
		const factory = await loadExtensionModule(resolvedPath);
		if (!factory) {
			return {
				extension: null,
				error: `Extension does not export a valid factory function: ${extensionPath}`,
			};
		}

		const extension = createExtension(extensionPath, resolvedPath);
		const api = createExtensionAPI(extension, runtime, cwd, eventBus);
		await factory(api);
		return { extension, error: null };
	} catch (error) {
		return {
			extension: null,
			error: `Failed to load extension: ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}

export async function loadExtensionFromFactory(
	factory,
	cwd,
	eventBus = createEventBus(),
	runtime = createExtensionRuntime(),
	extensionPath = "<inline>",
) {
	const extension = createExtension(extensionPath, extensionPath);
	const api = createExtensionAPI(extension, runtime, cwd, eventBus);
	await factory(api);
	return extension;
}

export async function loadExtensions(paths, cwd, eventBus = createEventBus()) {
	const extensions = [];
	const errors = [];
	const runtime = createExtensionRuntime();

	for (const extensionPath of paths) {
		const { extension, error } = await loadExtension(extensionPath, cwd, eventBus, runtime);
		if (error) {
			errors.push({ path: extensionPath, error });
			continue;
		}
		if (extension) {
			extensions.push(extension);
		}
	}

	return { extensions, errors, runtime };
}

function readPiManifest(packageJsonPath) {
	try {
		const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf-8"));
		if (packageJson.pi && typeof packageJson.pi === "object") {
			return packageJson.pi;
		}
		return null;
	} catch {
		return null;
	}
}

function isExtensionFile(name) {
	return name.endsWith(".ts") || name.endsWith(".js") || name.endsWith(".mts") || name.endsWith(".mjs");
}

function resolveExtensionEntries(directory) {
	const packageJsonPath = path.join(directory, "package.json");
	if (fs.existsSync(packageJsonPath)) {
		const manifest = readPiManifest(packageJsonPath);
		if (manifest?.extensions?.length) {
			const entries = [];
			for (const extensionPath of manifest.extensions) {
				const resolvedPath = path.resolve(directory, extensionPath);
				if (fs.existsSync(resolvedPath)) {
					entries.push(resolvedPath);
				}
			}
			if (entries.length > 0) {
				return entries;
			}
		}
	}

	for (const fileName of ["index.ts", "index.js", "index.mts", "index.mjs"]) {
		const candidate = path.join(directory, fileName);
		if (fs.existsSync(candidate)) {
			return [candidate];
		}
	}

	return null;
}

function discoverExtensionsInDir(directory) {
	if (!fs.existsSync(directory)) {
		return [];
	}

	const discovered = [];
	try {
		for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
			const entryPath = path.join(directory, entry.name);
			if ((entry.isFile() || entry.isSymbolicLink()) && isExtensionFile(entry.name)) {
				discovered.push(entryPath);
				continue;
			}
			if (entry.isDirectory() || entry.isSymbolicLink()) {
				const entries = resolveExtensionEntries(entryPath);
				if (entries) {
					discovered.push(...entries);
				}
			}
		}
	} catch {
		return [];
	}

	return discovered;
}

export async function discoverAndLoadExtensions(configuredPaths, cwd, agentDir = getAgentDir(), eventBus) {
	const allPaths = [];
	const seen = new Set();

	const addPaths = (paths) => {
		for (const candidatePath of paths) {
			const resolved = path.resolve(candidatePath);
			if (!seen.has(resolved)) {
				seen.add(resolved);
				allPaths.push(candidatePath);
			}
		}
	};

	addPaths(discoverExtensionsInDir(path.join(cwd, ".pi", "extensions")));
	addPaths(discoverExtensionsInDir(path.join(agentDir, "extensions")));

	for (const configuredPath of configuredPaths) {
		const resolved = resolvePath(configuredPath, cwd);
		if (fs.existsSync(resolved) && fs.statSync(resolved).isDirectory()) {
			const entries = resolveExtensionEntries(resolved);
			if (entries) {
				addPaths(entries);
				continue;
			}
			addPaths(discoverExtensionsInDir(resolved));
			continue;
		}
		addPaths([resolved]);
	}

	return loadExtensions(allPaths, cwd, eventBus);
}

const noOpUIContext = {
	select: async () => undefined,
	confirm: async () => false,
	input: async () => undefined,
	notify: () => {},
	onTerminalInput: () => () => {},
	setStatus: () => {},
	setWorkingMessage: () => {},
	setHiddenThinkingLabel: () => {},
	setWidget: () => {},
	setFooter: () => {},
	setHeader: () => {},
	setTitle: () => {},
	custom: async () => undefined,
	pasteToEditor: () => {},
	setEditorText: () => {},
	getEditorText: () => "",
	editor: async () => undefined,
	setEditorComponent: () => {},
	get theme() {
		return {
			fg(_name, text) {
				return text;
			},
			bg(_name, text) {
				return text;
			},
		};
	},
	getAllThemes: () => [],
	getTheme: () => undefined,
	setTheme: () => ({ success: false, error: "UI not available" }),
	getToolsExpanded: () => false,
	setToolsExpanded: () => {},
};

export class ExtensionRunner {
	constructor(extensions, runtime, cwd, sessionManager, modelRegistry) {
		this.extensions = extensions;
		this.runtime = runtime;
		this.uiContext = noOpUIContext;
		this.cwd = cwd;
		this.sessionManager = sessionManager;
		this.modelRegistry = modelRegistry;
		this.errorListeners = new Set();
		this.getModel = () => undefined;
		this.isIdleFn = () => true;
		this.getSignalFn = () => undefined;
		this.waitForIdleFn = async () => {};
		this.abortFn = () => {};
		this.hasPendingMessagesFn = () => false;
		this.getContextUsageFn = () => undefined;
		this.compactFn = () => {};
		this.getSystemPromptFn = () => "";
		this.newSessionHandler = async () => ({ cancelled: false });
		this.forkHandler = async () => ({ cancelled: false });
		this.navigateTreeHandler = async () => ({ cancelled: false });
		this.switchSessionHandler = async () => ({ cancelled: false });
		this.reloadHandler = async () => {};
		this.shutdownHandler = () => {};
	}

	bindCore(actions, contextActions, providerActions = {}) {
		this.runtime.sendMessage = actions.sendMessage;
		this.runtime.sendUserMessage = actions.sendUserMessage;
		this.runtime.appendEntry = actions.appendEntry;
		this.runtime.setSessionName = actions.setSessionName;
		this.runtime.getSessionName = actions.getSessionName;
		this.runtime.setLabel = actions.setLabel;
		this.runtime.getActiveTools = actions.getActiveTools;
		this.runtime.getAllTools = actions.getAllTools;
		this.runtime.setActiveTools = actions.setActiveTools;
		this.runtime.refreshTools = actions.refreshTools;
		this.runtime.getCommands = actions.getCommands;
		this.runtime.setModel = actions.setModel;
		this.runtime.getThinkingLevel = actions.getThinkingLevel;
		this.runtime.setThinkingLevel = actions.setThinkingLevel;

		this.getModel = contextActions.getModel;
		this.isIdleFn = contextActions.isIdle;
		this.getSignalFn = contextActions.getSignal;
		this.abortFn = contextActions.abort;
		this.hasPendingMessagesFn = contextActions.hasPendingMessages;
		this.shutdownHandler = contextActions.shutdown;
		this.getContextUsageFn = contextActions.getContextUsage;
		this.compactFn = contextActions.compact;
		this.getSystemPromptFn = contextActions.getSystemPrompt;

		for (const registration of this.runtime.pendingProviderRegistrations) {
			try {
				if (providerActions.registerProvider) {
					providerActions.registerProvider(registration.name, registration.config);
				} else {
					this.modelRegistry.registerProvider(registration.name, registration.config);
				}
			} catch (error) {
				this.emitError({
					extensionPath: registration.extensionPath,
					event: "register_provider",
					error: error instanceof Error ? error.message : String(error),
					stack: error instanceof Error ? error.stack : undefined,
				});
			}
		}
		this.runtime.pendingProviderRegistrations = [];

		this.runtime.registerProvider = (name, config) => {
			if (providerActions.registerProvider) {
				providerActions.registerProvider(name, config);
				return;
			}
			this.modelRegistry.registerProvider(name, config);
		};
		this.runtime.unregisterProvider = (name) => {
			if (providerActions.unregisterProvider) {
				providerActions.unregisterProvider(name);
				return;
			}
			this.modelRegistry.unregisterProvider(name);
		};
	}

	bindCommandContext(actions) {
		if (!actions) {
			this.waitForIdleFn = async () => {};
			this.newSessionHandler = async () => ({ cancelled: false });
			this.forkHandler = async () => ({ cancelled: false });
			this.navigateTreeHandler = async () => ({ cancelled: false });
			this.switchSessionHandler = async () => ({ cancelled: false });
			this.reloadHandler = async () => {};
			return;
		}

		this.waitForIdleFn = actions.waitForIdle;
		this.newSessionHandler = actions.newSession;
		this.forkHandler = actions.fork;
		this.navigateTreeHandler = actions.navigateTree;
		this.switchSessionHandler = actions.switchSession;
		this.reloadHandler = actions.reload;
	}

	setUIContext(uiContext) {
		this.uiContext = uiContext ?? noOpUIContext;
	}

	onError(listener) {
		this.errorListeners.add(listener);
		return () => this.errorListeners.delete(listener);
	}

	emitError(error) {
		for (const listener of this.errorListeners) {
			listener(error);
		}
	}

	hasHandlers(eventType) {
		return this.extensions.some((extension) => {
			const handlers = extension.handlers.get(eventType);
			return Array.isArray(handlers) && handlers.length > 0;
		});
	}

	getAllRegisteredTools() {
		const toolsByName = new Map();
		for (const extension of this.extensions) {
			for (const tool of extension.tools.values()) {
				if (!toolsByName.has(tool.definition.name)) {
					toolsByName.set(tool.definition.name, tool);
				}
			}
		}
		return Array.from(toolsByName.values());
	}

	resolveRegisteredCommands() {
		const commands = [];
		const counts = new Map();

		for (const extension of this.extensions) {
			for (const command of extension.commands.values()) {
				commands.push(command);
				counts.set(command.name, (counts.get(command.name) ?? 0) + 1);
			}
		}

		const seen = new Map();
		const takenInvocationNames = new Set();

		return commands.map((command) => {
			const occurrence = (seen.get(command.name) ?? 0) + 1;
			seen.set(command.name, occurrence);

			let invocationName = (counts.get(command.name) ?? 0) > 1 ? `${command.name}:${occurrence}` : command.name;
			if (takenInvocationNames.has(invocationName)) {
				let suffix = occurrence;
				do {
					suffix += 1;
					invocationName = `${command.name}:${suffix}`;
				} while (takenInvocationNames.has(invocationName));
			}

			takenInvocationNames.add(invocationName);
			return {
				...command,
				invocationName,
			};
		});
	}

	getRegisteredCommands() {
		return this.resolveRegisteredCommands();
	}

	getCommand(name) {
		return this.resolveRegisteredCommands().find((command) => command.invocationName === name);
	}

	createContext() {
		const getModel = this.getModel;
		return {
			ui: this.uiContext,
			hasUI: this.uiContext !== noOpUIContext,
			cwd: this.cwd,
			sessionManager: this.sessionManager,
			modelRegistry: this.modelRegistry,
			get model() {
				return getModel();
			},
			isIdle: () => this.isIdleFn(),
			signal: this.getSignalFn(),
			abort: () => this.abortFn(),
			hasPendingMessages: () => this.hasPendingMessagesFn(),
			shutdown: () => this.shutdownHandler(),
			getContextUsage: () => this.getContextUsageFn(),
			compact: (options) => this.compactFn(options),
			getSystemPrompt: () => this.getSystemPromptFn(),
		};
	}

	createCommandContext() {
		return {
			...this.createContext(),
			waitForIdle: () => this.waitForIdleFn(),
			newSession: (options) => this.newSessionHandler(options),
			fork: (entryId) => this.forkHandler(entryId),
			navigateTree: (targetId, options) => this.navigateTreeHandler(targetId, options),
			switchSession: (sessionPath) => this.switchSessionHandler(sessionPath),
			reload: () => this.reloadHandler(),
		};
	}

	isSessionBeforeEvent(event) {
		return ["session_before_switch", "session_before_fork", "session_before_compact", "session_before_tree"].includes(
			event.type,
		);
	}

	async emit(event) {
		const context = this.createContext();
		let result;

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get(event.type);
			if (!handlers?.length) {
				continue;
			}

			for (const handler of handlers) {
				try {
					const handlerResult = await handler(event, context);
					if (this.isSessionBeforeEvent(event) && handlerResult) {
						result = handlerResult;
						if (handlerResult.cancel) {
							return result;
						}
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: event.type,
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		return result;
	}

	async emitToolCall(event) {
		const context = this.createContext();
		let result;

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get("tool_call");
			if (!handlers?.length) {
				continue;
			}

			for (const handler of handlers) {
				try {
					const handlerResult = await handler(event, context);
					if (handlerResult) {
						result = handlerResult;
						if (handlerResult.block) {
							return result;
						}
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: "tool_call",
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		return result;
	}

	async emitToolResult(event) {
		const context = this.createContext();
		const currentEvent = { ...event };
		let modified = false;

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get("tool_result");
			if (!handlers?.length) {
				continue;
			}

			for (const handler of handlers) {
				try {
					const handlerResult = await handler(currentEvent, context);
					if (!handlerResult) {
						continue;
					}
					if (handlerResult.content !== undefined) {
						currentEvent.content = handlerResult.content;
						modified = true;
					}
					if (handlerResult.details !== undefined) {
						currentEvent.details = handlerResult.details;
						modified = true;
					}
					if (handlerResult.isError !== undefined) {
						currentEvent.isError = handlerResult.isError;
						modified = true;
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: "tool_result",
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		if (!modified) {
			return undefined;
		}

		return {
			content: currentEvent.content,
			details: currentEvent.details,
			isError: currentEvent.isError,
		};
	}

	async emitBeforeProviderRequest(payload) {
		const context = this.createContext();
		let currentPayload = payload;

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get("before_provider_request");
			if (!handlers?.length) {
				continue;
			}

			for (const handler of handlers) {
				try {
					const handlerResult = await handler({ type: "before_provider_request", payload: currentPayload }, context);
					if (handlerResult !== undefined) {
						currentPayload = handlerResult;
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: "before_provider_request",
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		return currentPayload;
	}

	async emitInput(text, images, source) {
		const context = this.createContext();
		let currentText = text;
		let currentImages = images;

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get("input") ?? [];
			for (const handler of handlers) {
				try {
					const result = await handler({ type: "input", text: currentText, images: currentImages, source }, context);
					if (result?.action === "handled") {
						return result;
					}
					if (result?.action === "transform") {
						currentText = result.text;
						currentImages = result.images ?? currentImages;
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: "input",
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		return currentText !== text || currentImages !== images
			? { action: "transform", text: currentText, images: currentImages }
			: { action: "continue" };
	}

	async emitResourcesDiscover(cwd, reason) {
		const context = this.createContext();
		const skillPaths = [];
		const promptPaths = [];
		const themePaths = [];

		for (const extension of this.extensions) {
			const handlers = extension.handlers.get("resources_discover");
			if (!handlers?.length) {
				continue;
			}

			for (const handler of handlers) {
				try {
					const result = await handler({ type: "resources_discover", cwd, reason }, context);
					if (result?.skillPaths?.length) {
						skillPaths.push(...result.skillPaths.map((entryPath) => ({ path: entryPath, extensionPath: extension.path })));
					}
					if (result?.promptPaths?.length) {
						promptPaths.push(...result.promptPaths.map((entryPath) => ({ path: entryPath, extensionPath: extension.path })));
					}
					if (result?.themePaths?.length) {
						themePaths.push(...result.themePaths.map((entryPath) => ({ path: entryPath, extensionPath: extension.path })));
					}
				} catch (error) {
					this.emitError({
						extensionPath: extension.path,
						event: "resources_discover",
						error: error instanceof Error ? error.message : String(error),
						stack: error instanceof Error ? error.stack : undefined,
					});
				}
			}
		}

		return { skillPaths, promptPaths, themePaths };
	}
}
