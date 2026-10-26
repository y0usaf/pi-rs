import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { register } from "node:module";
import { pathToFileURL } from "node:url";

import { drain } from "./common.mjs";

register("./register.mjs", import.meta.url);

const protocol = process.stdout.write.bind(process.stdout);
process.stdout.write = (chunk, encoding, callback) => process.stderr.write(chunk, encoding, callback);

const UNSUPPORTED = [
	"registerTool",
	"registerShortcut",
	"registerFlag",
	"getFlag",
	"registerMessageRenderer",
	"registerMarkdownTransformer",
	"registerEntryRenderer",
	"registerToolRenderer",
	"sendMessage",
	"appendEntry",
	"setSessionName",
	"getSessionName",
	"setLabel",
	"exec",
	"getSettings",
	"setActiveTools",
	"getCommands",
	"setModel",
	"getThinkingLevel",
	"setThinkingLevel",
	"registerProvider",
	"unregisterProvider",
	"registerMcpServer",
	"unregisterMcpServer",
	"getMcpServers",
	"registerVirtualModel",
	"unregisterVirtualModel",
	"route",
];

function send(message) {
	protocol(JSON.stringify(message) + "\n");
}

// Calls an extension makes into pi-rs while a handler is waiting: sent on the same channel as the
// answer, numbered so the reply names the call it satisfies.
const answers = new Map();
let nextCallId = 1;

function ask(call, args) {
	const callId = nextCallId++;
	send({ call, callId, ...args });
	return new Promise((resolve, reject) => answers.set(callId, { resolve, reject }));
}

function stripBom(text) {
	return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
}

function manifestOf(dir) {
	try {
		const pkg = JSON.parse(stripBom(fs.readFileSync(path.join(dir, "package.json"), "utf8")));
		const pi = pkg.pi;
		if (typeof pi !== "object" || pi === null) return null;
		const extensions = Array.isArray(pi.extensions) && pi.extensions.every((entry) => typeof entry === "string");
		return extensions ? { extensions: pi.extensions } : {};
	} catch {
		return null;
	}
}

function entriesOf(dir) {
	const manifest = manifestOf(dir);
	if (manifest?.extensions?.length) {
		const entries = manifest.extensions
			.map((entry) => path.resolve(dir, entry))
			.filter((entry) => fs.existsSync(entry));
		if (entries.length > 0) return entries;
	}
	for (const name of ["index.ts", "index.js"]) {
		const candidate = path.join(dir, name);
		if (fs.existsSync(candidate)) return [candidate];
	}
	return null;
}

function discover(dir) {
	let listing;
	try {
		listing = fs.readdirSync(dir, { withFileTypes: true });
	} catch {
		return [];
	}
	const found = [];
	for (const entry of listing) {
		const full = path.join(dir, entry.name);
		if ((entry.isFile() || entry.isSymbolicLink()) && (entry.name.endsWith(".ts") || entry.name.endsWith(".js"))) {
			found.push(full);
			continue;
		}
		if (entry.isDirectory() || entry.isSymbolicLink()) {
			const inner = entriesOf(full);
			if (inner) found.push(...inner);
		}
	}
	return found;
}

function resolvePaths(request) {
	const all = [];
	const seen = new Set();
	const add = (list) => {
		for (const entry of list) {
			const resolved = path.resolve(entry);
			if (seen.has(resolved)) continue;
			seen.add(resolved);
			all.push(entry);
		}
	};
	for (const dir of request.dirs ?? []) add(discover(dir));
	for (const source of request.paths ?? []) {
		const resolved = path.resolve(request.cwd, source);
		let stats;
		try {
			stats = fs.statSync(resolved);
		} catch {
			add([resolved]);
			continue;
		}
		if (stats.isDirectory()) {
			const entries = entriesOf(resolved);
			if (entries) {
				add(entries);
				continue;
			}
			add(discover(resolved));
			continue;
		}
		add([resolved]);
	}
	return all;
}

function create(extensionPath) {
	return {
		path: extensionPath,
		resolvedPath: extensionPath,
		events: [],
		commands: [],
		unsupported: [],
		eventHandlers: new Map(),
		commandHandlers: new Map(),
	};
}

// Calls an extension makes into pi-rs while a request is being served. They travel back with
// the answer, so the host applies them in order once the extension has returned.
const calls = [];
const errors = [];

const UNIMPLEMENTED_REGISTRY = [
	"stream",
	"streamSimple",
	"classify",
	"generateImages",
	"refresh",
	"getError",
	"getProvider",
	"getProviderAuth",
	"getProviderAuthStatus",
	"getProviderDisplayName",
	"getModelOfType",
	"findOfType",
	"getModelsOfType",
	"getAvailableOfType",
	"getApiKeyAndHeaders",
	"getApiKeyForProvider",
];

/** `ctx.modelRegistry`: the models `find` searches, and a `complete` pi-rs runs. */
function registryOf() {
	const models = runtime.models ?? {};
	const list = models.list ?? [];
	const configured = new Set(models.configured ?? []);
	const registry = {
		getAll: () => list.map((model) => ({ ...model })),
		getAvailable: () => list.filter((model) => configured.has(model.provider)),
		find: (provider, modelId) => list.find((model) => model.provider === provider && model.id === modelId),
		hasConfiguredAuth: (model) => configured.has(model?.provider),
		complete: (model, context, options) =>
			ask("complete", {
				provider: model?.provider,
				modelId: model?.id,
				context,
				options: {
					maxTokens: options?.maxTokens,
					reasoningEffort: options?.reasoningEffort,
				},
			}),
	};
	for (const member of UNIMPLEMENTED_REGISTRY) {
		registry[member] = () => {
			calls.push({ call: "unsupported", member: `modelRegistry.${member}` });
			return Promise.resolve(undefined);
		};
	}
	return registry;
}

function sessionManagerOf(session) {
	const entries = session?.entries ?? [];
	const entry = (id) => entries.find((candidate) => candidate.id === id);
	return {
		getCwd: () => session?.header?.cwd ?? runtime.cwd,
		getSessionDir: () => session?.dir ?? "",
		getSessionId: () => session?.header?.id ?? "",
		getSessionFile: () => session?.file ?? undefined,
		getLeafId: () => entries.at(-1)?.id ?? null,
		getLeafEntry: () => entries.at(-1),
		getEntry: entry,
		getLabel: () => undefined,
		getBranch(fromId) {
			const end = fromId === undefined ? entries.length : entries.findIndex((candidate) => candidate.id === fromId) + 1;
			return entries.slice(0, end);
		},
		getHeader: () => session?.header ?? null,
		getEntries: () => [...entries],
		getTree() {
			let node;
			for (const leaf of [...entries].reverse()) node = { entry: leaf, children: node ? [node] : [] };
			return node ? [node] : [];
		},
		getSessionName: () => session?.name ?? undefined,
	};
}

function contextOf() {
	const call = (value) => calls.push(value);
	const signal = new AbortController();
	return {
		ui: {
			notify(message, type) {
				call({ call: "notify", text: String(message), level: type ?? "info" });
			},
			setStatus() {
				calls.push({ call: "unsupported", member: "ui.setStatus" });
			},
			select() {
				calls.push({ call: "unsupported", member: "ui.select" });
				return Promise.resolve(undefined);
			},
			confirm() {
				calls.push({ call: "unsupported", member: "ui.confirm" });
				return Promise.resolve(false);
			},
			input() {
				calls.push({ call: "unsupported", member: "ui.input" });
				return Promise.resolve(undefined);
			},
			custom() {
				calls.push({ call: "unsupported", member: "ui.custom" });
				return Promise.resolve(undefined);
			},
			onTerminalInput() {
				return () => {};
			},
			setWidget() {
				calls.push({ call: "unsupported", member: "ui.setWidget" });
			},
			setTheme() {},
			getTheme() {
				return undefined;
			},
		},
		mode: runtime.mode,
		hasUI: runtime.hasUI,
		cwd: runtime.cwd,
		sessionManager: sessionManagerOf(runtime.session),
		modelRegistry: registryOf(),
		model: runtime.models?.current,
		scopedModels: [],
		isIdle: () => !runtime.streaming,
		isProjectTrusted: () => runtime.trusted,
		signal: signal.signal,
		abort() {
			call({ call: "abort" });
		},
		hasPendingMessages: () => runtime.pending > 0,
		shutdown() {
			call({ call: "shutdown" });
		},
		getContextUsage: () => undefined,
		compact() {},
		getSystemPrompt: () => "",
	};
}

function apiFor(extension, runtime) {
	const note = (member) => {
		if (!extension.unsupported.includes(member)) extension.unsupported.push(member);
	};
	const api = {
		on(name, handler) {
			extension.events.push(name);
			const handlers = extension.eventHandlers.get(name) ?? [];
			handlers.push(handler);
			extension.eventHandlers.set(name, handlers);
			return () => {
				const registered = extension.eventHandlers.get(name);
				if (!registered) return;
				const index = registered.indexOf(handler);
				if (index >= 0) registered.splice(index, 1);
			};
		},
		registerCommand(name, options) {
			extension.commands.push({ name, description: options?.description ?? "" });
			extension.commandHandlers.set(name, options?.handler);
		},
		sendUserMessage(content, options) {
			const parts = typeof content === "string" ? [{ type: "text", text: content }] : content;
			calls.push({
				call: "sendUserMessage",
				content: parts,
				deliverAs: options?.deliverAs ?? null,
			});
			return Promise.resolve();
		},
		getActiveTools: () => [...runtime.tools.active],
		getAllTools: () => runtime.tools.all.map((tool) => ({ ...tool })),
		events: {
			on() {
				return () => {};
			},
			emit() {},
		},
	};
	for (const member of UNSUPPORTED) api[member] = () => note(member);
	return api;
}

function summary(extension) {
	return {
		path: extension.path,
		resolvedPath: extension.resolvedPath,
		events: extension.events,
		commands: extension.commands,
		unsupported: extension.unsupported,
	};
}

const loaded = new Map();
let runtime = {
	mode: "print",
	hasUI: false,
	cwd: process.cwd(),
	trusted: false,
	streaming: false,
	pending: 0,
	session: undefined,
	models: { current: undefined, list: [], configured: [] },
	tools: { active: [], all: [] },
};

async function load(request) {
	Object.assign(runtime, request.updates);
	const loadErrors = [];
	globalThis.__piExtensionAgentDir = request.agentDir;
	if (typeof request.mode === "string") runtime.mode = request.mode;
	runtime.hasUI = request.hasUI === true;
	runtime.cwd = request.cwd ?? runtime.cwd;
	runtime.trusted = request.trusted === true;
	const extensions = [];
	const found = [];
	for (const extensionPath of resolvePaths(request)) {
		try {
			if (loaded.has(extensionPath)) {
				extensions.push(summary(loaded.get(extensionPath)));
				found.push(loaded.get(extensionPath));
				continue;
			}
			const module = await import(pathToFileURL(extensionPath).href);
			const factory = module.default;
			if (typeof factory !== "function") {
				loadErrors.push({
					path: extensionPath,
					error: `Extension does not export a valid factory function: ${extensionPath}`,
				});
				continue;
			}
			const extension = create(extensionPath);
			drain();
			await factory(apiFor(extension, runtime));
			for (const name of drain()) {
				if (!extension.unsupported.includes(name)) extension.unsupported.push(name);
			}
			loaded.set(extensionPath, extension);
			extensions.push(summary(extension));
			found.push(extension);
		} catch (error) {
			loadErrors.push({
				path: extensionPath,
				error: `Failed to load extension: ${error instanceof Error ? error.message : String(error)}`,
			});
		}
	}
	return { value: { extensions, errors: loadErrors } };
}

function handlersOf(name) {
	const found = [];
	for (const extension of loaded.values()) {
		for (const handler of extension.eventHandlers.get(name) ?? []) {
			found.push({ extension, handler });
		}
	}
	return found;
}

function sameRole(a, b) {
	return (a?.role ?? "") === (b?.role ?? "");
}

function failed(extension, name, error) {
	errors.push({
		path: extension.path,
		event: name,
		error: error instanceof Error ? error.message : String(error),
	});
}

async function emitMessageEnd(request) {
	let message = request.event.message;
	let replaced = false;
	for (const { extension, handler } of handlersOf("message_end")) {
		const ctx = contextOf();
		try {
			const result = await handler({ ...request.event, message }, ctx);
			if (!result?.message) continue;
			if (!sameRole(result.message, message)) {
				failed(
					extension,
					"message_end",
					new Error("message_end handlers must return a message with the same role"),
				);
				continue;
			}
			message = result.message;
			replaced = true;
		} catch (error) {
			failed(extension, "message_end", error);
		}
	}
	return { value: replaced ? { message } : null };
}

async function emitInput(request) {
	let text = request.event.text;
	let images = request.event.images;
	for (const { extension, handler } of handlersOf("input")) {
		const ctx = contextOf();
		try {
			const result = await handler({ ...request.event, text, images }, ctx);
			if (result?.action === "handled") return { value: { action: "handled" } };
			if (result?.action === "transform") {
				text = result.text ?? text;
				images = result.images ?? images;
			}
		} catch (error) {
			failed(extension, "input", error);
		}
	}
	const changed = text !== request.event.text || images !== request.event.images;
	return { value: changed ? { action: "transform", text, images } : { action: "continue" } };
}

async function emitEvent(request) {
	const name = request.name;
	runtime.streaming = request.streaming === true;
	runtime.pending = request.pending ?? 0;
	Object.assign(runtime, request.updates);
	if (name === "message_end") return await emitMessageEnd(request);
	if (name === "input") return await emitInput(request);
	for (const { extension, handler } of handlersOf(name)) {
		const ctx = contextOf();
		try {
			await handler(request.event, ctx);
		} catch (error) {
			failed(extension, name, error);
		}
	}
	return { value: null };
}

async function runCommand(request) {
	const name = request.name;
	Object.assign(runtime, request.updates);
	for (const extension of loaded.values()) {
		const handler = extension.commandHandlers.get(name);
		if (!handler) continue;
		const ctx = contextOf();
		try {
			await handler(request.args ?? "", ctx);
		} catch (error) {
			errors.push({
				path: `command:${name}`,
				event: "command",
				error: error instanceof Error ? error.message : String(error),
			});
		}
		return { value: null };
	}
	return { value: null };
}

async function handle(request) {
	switch (request.op) {
		case "load":
			return await load(request);
		case "event":
			return await emitEvent(request);
		case "command":
			return await runCommand(request);
		case "shutdown":
			process.exit(0);
		default:
			throw new Error(`unknown op: ${request.op}`);
	}
}

let queue = Promise.resolve();
const input = readline.createInterface({ input: process.stdin, terminal: false });
input.on("line", (line) => {
	let request;
	try {
		request = JSON.parse(line);
	} catch {
		return;
	}
	if (request.answer !== undefined) {
		const waiting = answers.get(request.answer);
		if (!waiting) return;
		answers.delete(request.answer);
		if (request.ok === true) waiting.resolve(request.value);
		else waiting.reject(new Error(request.error ?? "the call failed"));
		return;
	}
	queue = queue.then(async () => {
		try {
			const answer = await handle(request);
			const value = answer.value;
			send({ id: request.id, ok: true, value, calls, errors });
		} catch (error) {
			send({
				id: request.id,
				ok: false,
				error: error instanceof Error ? error.message : String(error),
				calls,
				errors,
			});
		}
		calls.length = 0;
		errors.length = 0;
	});
});
input.on("close", () => process.exit(0));
