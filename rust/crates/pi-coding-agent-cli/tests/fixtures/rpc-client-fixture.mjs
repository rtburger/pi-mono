import process from "node:process";

const expectedProvider = process.env.RPC_FIXTURE_EXPECT_PROVIDER;
const expectedModel = process.env.RPC_FIXTURE_EXPECT_MODEL;
const expectedArgs = (process.env.RPC_FIXTURE_EXPECT_ARGS ?? "")
	.split("\u0000")
	.filter(Boolean);
const mode = process.env.RPC_FIXTURE_MODE ?? "default";
const extensionMode = process.env.RPC_FIXTURE_EXTENSION_MODE ?? "off";

if (mode === "exit-immediately") {
	process.stderr.write("fixture exiting immediately\n");
	process.exit(3);
}

const args = process.argv.slice(2);
const assertArg = (flag, expected) => {
	const index = args.indexOf(flag);
	if (index === -1 || args[index + 1] !== expected) {
		process.stderr.write(`expected ${flag}=${expected}, got: ${args.join(" ")}\n`);
		process.exit(4);
	}
};

if (!args.includes("--mode") || args[args.indexOf("--mode") + 1] !== "rpc") {
	process.stderr.write(`expected --mode rpc, got: ${args.join(" ")}\n`);
	process.exit(4);
}
if (expectedProvider) {
	assertArg("--provider", expectedProvider);
}
if (expectedModel) {
	assertArg("--model", expectedModel);
}
for (const arg of expectedArgs) {
	if (!args.includes(arg)) {
		process.stderr.write(`missing expected arg ${arg}, got: ${args.join(" ")}\n`);
		process.exit(4);
	}
}

const model = {
	id: expectedModel ?? "fixture-model",
	name: "Fixture Model",
	api: "faux",
	provider: expectedProvider ?? "fixture-provider",
	baseUrl: "http://localhost:0",
	reasoning: true,
	input: ["text", "image"],
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
	contextWindow: 128000,
	maxTokens: 16384,
	compat: null,
};
const alternateModel = {
	...model,
	id: `${model.id}-alt`,
	name: "Fixture Model Alt",
	reasoning: false,
};

const sourceInfo = {
	path: "/tmp/fixture.prompt.md",
	source: "local",
	scope: "top-level",
	origin: "top-level",
	baseDir: "/tmp",
};

let state = {
	model,
	thinkingLevel: "off",
	isStreaming: false,
	isCompacting: false,
	steeringMode: "all",
	followUpMode: "all",
	sessionFile: "/tmp/fixture-session.jsonl",
	sessionId: "fixture-session-id",
	sessionName: undefined,
	autoCompactionEnabled: true,
	messageCount: 0,
	pendingMessageCount: 0,
};
let messages = [];
let lastAssistantText = null;
let pendingExtensionRequestId = null;
let promptCounter = 0;

process.stderr.write("fixture stderr ready\n");
process.stdin.setEncoding("utf8");

const write = (value) => {
	process.stdout.write(`${JSON.stringify(value)}\n`);
};

const assistantMessage = (text, timestamp) => ({
	role: "assistant",
	content: [{ type: "text", text, text_signature: null }],
	api: model.api,
	provider: model.provider,
	model: model.id,
	response_id: null,
	usage: {
		input: 1,
		output: 1,
		cache_read: 0,
		cache_write: 0,
		total_tokens: 2,
		cost: { input: 0, output: 0, cache_read: 0, cache_write: 0, total: 0 },
	},
	stop_reason: "stop",
	error_message: null,
	timestamp: timestamp,
});

const userMessage = (text, timestamp) => ({
	role: "user",
	content: [{ type: "text", text }],
	timestamp: timestamp,
});

const updateMessageState = (text) => {
	promptCounter += 1;
	const user = userMessage(text, promptCounter * 10);
	const assistant = assistantMessage(`fixture reply ${promptCounter}`, promptCounter * 10 + 1);
	messages = [user, assistant];
	state.messageCount = messages.length;
	lastAssistantText = assistant.content[0].text;
	return { user, assistant };
};

const sendPromptEvents = (text) => {
	state.isStreaming = true;
	const { assistant } = updateMessageState(text);
	write({ type: "agent_start" });
	write({
		type: "message_update",
		message: assistant,
		assistantMessageEvent: {
			type: "text_delta",
			content_index: 0,
			delta: assistant.content[0].text,
			partial: assistant,
		},
	});

	if (extensionMode === "prompt") {
		pendingExtensionRequestId = `ext-${promptCounter}`;
		write({
			type: "extension_ui_request",
			id: pendingExtensionRequestId,
			method: "input",
			title: "Fixture Input",
			placeholder: "/demo",
		});
		return;
	}

	state.isStreaming = false;
	write({ type: "agent_end", messages });
};

const finishExtensionPrompt = (value) => {
	pendingExtensionRequestId = null;
	state.isStreaming = false;
	lastAssistantText = `extension:${value ?? "cancelled"}`;
	messages = [
		userMessage("extension prompt", 1000),
		assistantMessage(lastAssistantText, 1001),
	];
	state.messageCount = messages.length;
	write({
		type: "message_end",
		message: messages[1],
	});
	write({ type: "agent_end", messages });
};

let buffer = "";
process.stdin.on("data", (chunk) => {
	buffer += chunk;
	while (true) {
		const newlineIndex = buffer.indexOf("\n");
		if (newlineIndex === -1) {
			return;
		}

		const line = buffer.slice(0, newlineIndex).replace(/\r$/, "");
		buffer = buffer.slice(newlineIndex + 1);
		if (line.length === 0) {
			continue;
		}

		const command = JSON.parse(line);
		handleCommand(command);
	}
});

process.stdin.on("end", () => {
	process.exit(0);
});

const handleCommand = (command) => {
	switch (command.type) {
		case "get_state":
			write({ id: command.id, type: "response", command: command.type, success: true, data: state });
			break;
		case "get_available_models":
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: { models: [model, alternateModel] },
			});
			break;
		case "set_thinking_level":
			state.thinkingLevel = command.level;
			write({ id: command.id, type: "response", command: command.type, success: true });
			break;
		case "set_session_name":
			state.sessionName = command.name;
			write({ id: command.id, type: "response", command: command.type, success: true });
			break;
		case "get_last_assistant_text":
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: { text: lastAssistantText },
			});
			break;
		case "get_messages":
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: { messages },
			});
			break;
		case "get_commands":
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: {
					commands: [
						{
							name: "fixture-command",
							description: "Fixture command",
							source: "prompt",
							sourceInfo,
						},
					],
				},
			});
			break;
		case "new_session":
			messages = [];
			state.messageCount = 0;
			lastAssistantText = null;
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: { cancelled: false },
			});
			break;
		case "bash":
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: true,
				data: {
					output: `bash:${command.command}`,
					exitCode: 0,
					cancelled: false,
					truncated: false,
					fullOutputPath: null,
				},
			});
			break;
		case "prompt":
			write({ id: command.id, type: "response", command: command.type, success: true });
			setTimeout(() => sendPromptEvents(command.message), 10);
			break;
		case "extension_ui_response":
			if (pendingExtensionRequestId === command.id) {
				finishExtensionPrompt(command.value ?? (command.cancelled ? null : String(command.confirmed)));
			}
			break;
		default:
			write({
				id: command.id,
				type: "response",
				command: command.type,
				success: false,
				error: `unsupported command: ${command.type}`,
			});
	}
};
