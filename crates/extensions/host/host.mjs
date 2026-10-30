import { AsyncLocalStorage } from "node:async_hooks";
import fs from "node:fs";
import path from "node:path";
import { MessageChannel, Worker, receiveMessageOnPort } from "node:worker_threads";

const protocol = process.stdout.write.bind(process.stdout);
process.stdout.write = (chunk, encoding, callback) => process.stderr.write(chunk, encoding, callback);

const answers = new Map();
let nextCallId = 1;
const current = new AsyncLocalStorage();
const inflight = new Map();

function send(message) {
	protocol(JSON.stringify(message) + "\n");
}

function askStream(call, args, update) {
	const callId = nextCallId++;
	const answer = new Promise((resolve, reject) => answers.set(callId, { resolve, reject, update }));
	send({ ...args, call, callId, request: current.getStore()?.id });
	return { callId, answer };
}

function ask(call, args) {
	return askStream(call, args).answer;
}

function effect(call) {
	send(call);
}

const READER = `
const { parentPort, workerData } = require("node:worker_threads");
const { openSync, readSync } = require("node:fs");
const { StringDecoder } = require("node:string_decoder");
const blocked = new Int32Array(workerData.blocked);
const decoder = new StringDecoder("utf8");
const chunk = Buffer.alloc(65536);
function answerOf(line) {
	try {
		const message = JSON.parse(line);
		return "update" in message ? undefined : message.answer;
	} catch {
		return undefined;
	}
}
const input = openSync("/proc/self/fd/0", "r");
let pending = "";
try {
	for (let read; (read = readSync(input, chunk)) > 0; ) {
		pending += decoder.write(chunk.subarray(0, read));
		for (let end; (end = pending.indexOf("\\n")) >= 0; ) {
			const line = pending.slice(0, end);
			pending = pending.slice(end + 1);
			const id = Atomics.load(blocked, 0);
			if (id !== 0 && answerOf(line) === id) {
				workerData.port.postMessage(line);
				Atomics.store(blocked, 0, 0);
				Atomics.notify(blocked, 0);
			} else {
				parentPort.postMessage(line);
			}
		}
	}
} finally {
	Atomics.store(blocked, 0, 0);
	Atomics.notify(blocked, 0);
}
parentPort.postMessage(null);
`;

const blocked = new Int32Array(new SharedArrayBuffer(4));
const { port1: answered, port2: answering } = new MessageChannel();

function askNow(call, args) {
	const callId = nextCallId++;
	Atomics.store(blocked, 0, callId);
	send({ ...args, call, callId, request: current.getStore()?.id });
	while (Atomics.load(blocked, 0) === callId) Atomics.wait(blocked, 0, callId);
	const received = receiveMessageOnPort(answered);
	if (!received) throw new Error(`pi-rs did not answer ${call}`);
	const reply = JSON.parse(received.message);
	if (reply.ok !== true) throw new Error(reply.error ?? "the call failed");
	return reply.value;
}

/** A member of upstream's API pi-rs does not back yet: reported to pi-rs, and thrown to the caller. */
function unported(member) {
	effect({ call: "unsupported", member });
	return new Error(`pi-rs does not implement ${member} yet`);
}

globalThis.__piHost = { effect, unported };

const dist = new URL("./node_modules/@earendil-works/pi-coding-agent/dist/", import.meta.url);
const { loadExtensions } = await import(new URL("core/extensions/loader.js", dist));
const { ExtensionRunner } = await import(new URL("core/extensions/runner.js", dist));
const { createEventBus } = await import(new URL("core/event-bus.js", dist));
const { SessionManager } = await import(new URL("core/session-manager.js", dist));
const { convertToLlm } = await import(new URL("core/messages.js", dist));
const { createSourceInfo, createSyntheticSourceInfo } = await import(new URL("core/source-info.js", dist));
const { normalizeBuildSystemPromptOptions } = await import(new URL("core/system-prompt.js", dist));
const { killAll } = await import(new URL("core/exec.js", dist));
const { initTheme } = await import(new URL("modes/interactive/theme/theme.js", dist));
const { flowOf } = await import(new URL("./node_modules/@earendil-works/pi-ai/dist/providers/all.js", import.meta.url));
const usage = await import(new URL("core/compaction/compaction.js", dist));
const { getLatestCompactionEntry } = await import(new URL("core/session-manager.js", dist));
const { createVirtualModel, findLatestResponse, isVirtualModel } = await import(new URL("core/virtual-models.js", dist));

function exit() {
	killAll();
	process.exit(0);
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

const state = {
	mode: "print",
	hasUI: false,
	cwd: process.cwd(),
	trusted: false,
	streaming: false,
	pending: 0,
	session: undefined,
	models: { current: undefined, list: [], configured: [] },
	tools: { active: [], all: [], known: [] },
	prompt: undefined,
};

let runner;
const providers = new Map();
const registrants = new Map();
const virtualModels = new Map();

function absorb(request) {
	Object.assign(state, request.updates);
	if (typeof request.mode === "string") state.mode = request.mode;
	if (typeof request.hasUI === "boolean") state.hasUI = request.hasUI;
	if (typeof request.cwd === "string") state.cwd = request.cwd;
	if (typeof request.streaming === "boolean") state.streaming = request.streaming;
	if (typeof request.pending === "number") state.pending = request.pending;
	if (request.updates?.session) session.refresh();
}

const WRITES = /^(append|branch|reset|new|set|create)/;

/** `ctx.sessionManager`: upstream's own SessionManager over the entries pi-rs last sent, refusing writes. */
const session = (() => {
	const view = SessionManager.inMemory(state.cwd);
	const load = view._loadEntries.bind(view);
	for (const name of Object.getOwnPropertyNames(SessionManager.prototype)) {
		if (WRITES.test(name)) {
			view[name] = () => {
				throw unported(`sessionManager.${name}`);
			};
		}
	}
	view.getSessionFile = () => state.session?.file ?? undefined;
	view.getSessionDir = () => state.session?.dir ?? "";
	view.isPersisted = () => typeof state.session?.file === "string";
	view.getSessionName = () => state.session?.name ?? undefined;
	view.refresh = () => {
		const entries = state.session?.header ? [state.session.header, ...(state.session.entries ?? [])] : [];
		if (entries.length > 0) load(entries);
	};
	return view;
})();

const REGISTRY_UNPORTED = [
	"stream",
	"streamSimple",
	"classify",
	"generateImages",
	"getError",
	"getProvider",
	"getProviderDisplayName",
	"getModelOfType",
	"findOfType",
	"getModelsOfType",
	"getAvailableOfType",
	"getApiKeyAndHeaders",
	"getApiKeyForProvider",
	"registerProvider",
	"unregisterProvider",
	"registerVirtualModel",
	"unregisterVirtualModel",
];

/** `ctx.modelRegistry`: the catalog and logins pi-rs last sent, with requests pi-rs runs. */
const modelRegistry = (() => {
	const list = () => state.models?.list ?? [];
	const configured = () => new Set(state.models?.configured ?? []);
	const registry = {
		getAll: () => list().map((model) => ({ ...model })),
		getAvailable: () => {
			const ids = configured();
			return list().filter((model) => ids.has(model.provider));
		},
		find: (provider, modelId) => list().find((model) => model.provider === provider && model.id === modelId),
		hasConfiguredAuth: (model) => configured().has(model?.provider),
		getProviderAuthStatus: (provider) => ({ configured: configured().has(provider) }),
		getProviderAuth: async (provider) => (await ask("providerAuth", { provider })) ?? undefined,
		refresh: async (options) => {
			const answer = await ask("refreshModels", {
				allowNetwork: options?.allowNetwork,
				force: options?.force,
				providers: options?.providers ? [...options.providers] : undefined,
			});
			state.models = { ...state.models, list: answer.list, configured: answer.configured };
			const errors = Object.entries(answer.errors ?? {}).map(([id, message]) => [id, new Error(message)]);
			return { aborted: answer.aborted, errors: new Map(errors) };
		},
		complete: (model, context, options) => {
			const forwarded = new Set(["maxTokens", "reasoningEffort", "signal"]);
			const dropped = Object.keys(options ?? {}).filter((key) => !forwarded.has(key));
			if (dropped.length > 0) throw unported(`modelRegistry.complete options ${dropped.join(", ")}`);
			return ask("complete", {
				provider: model?.provider,
				modelId: model?.id,
				context,
				options: { maxTokens: options?.maxTokens, reasoningEffort: options?.reasoningEffort },
			});
		},
	};
	for (const member of REGISTRY_UNPORTED) {
		registry[member] = () => {
			throw unported(`modelRegistry.${member}`);
		};
	}
	return registry;
})();

function plain(value) {
	return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

function toolInfos() {
	return (runner?.getAllRegisteredTools() ?? []).map(({ definition, sourceInfo }) => ({
		name: definition.name,
		label: definition.label ?? definition.name,
		description: definition.description ?? "",
		parameters: plain(definition.parameters),
		promptSnippet: definition.promptSnippet,
		promptGuidelines: definition.promptGuidelines ?? [],
		executionMode: definition.executionMode,
		exposure: definition.exposure,
		defaultActive: definition.defaultActive,
		path: sourceInfo?.path ?? "",
	}));
}

function providerInfos() {
	return [...providers.values()].map(({ provider, path: extensionPath }) => {
		const oauth = provider.auth?.oauth ?? provider.oauth;
		return {
			id: provider.id,
			name: provider.name ?? provider.id,
			oauth: oauth ? { name: oauth.name ?? provider.name ?? provider.id, flow: flowOf(oauth) ?? null } : null,
			streams: typeof provider.streamSimple === "function",
			path: extensionPath,
		};
	});
}

function activated(definition) {
	const exposure = definition.exposure ?? "direct";
	if (exposure !== "direct" && exposure !== "model-only") return false;
	return definition.defaultActive !== false;
}

const DISCOVERY_GLOBALS = /\b(searchTools|describeNamespace|describeTool|ALL_TOOLS)\b/;

const REACHES = {
	"builtin:codemode": (input) =>
		typeof input?.code !== "string" ? [] : DISCOVERY_GLOBALS.test(input.code) ? "all" : (input.code.match(/\w+/g) ?? []),
	"builtin:tool-search": () => "all",
};

function bundledSource(path) {
	return { path, source: "builtin", scope: "temporary", origin: "top-level" };
}

function viewInfo(view) {
	const source = view.source;
	const info = {
		name: view.name,
		description: view.description,
		parameters: view.parameters,
		promptGuidelines: view.promptGuidelines ?? undefined,
		exposure: view.exposure,
	};
	if (source.kind === "bundled" && REACHES[source.path]) info.reachesTools = REACHES[source.path];
	if (source.kind === "builtin") info.sourceInfo = createSyntheticSourceInfo(`builtin:${view.name}`, { source: "builtin" });
	else if (source.kind === "bundled") info.sourceInfo = bundledSource(source.path);
	else info.sourceInfo = bundledSource("builtin:mcp");
	return info;
}

function registeredInfo({ definition, sourceInfo }) {
	return {
		name: definition.name,
		description: definition.description,
		parameters: definition.parameters,
		promptGuidelines: definition.promptGuidelines,
		exposure: definition.exposure ?? "direct",
		...(definition.namespace ? { namespace: definition.namespace } : {}),
		...(definition.annotations ? { annotations: { ...definition.annotations } } : {}),
		...(definition.reachesTools ? { reachesTools: definition.reachesTools } : {}),
		sourceInfo,
	};
}

function pendingTools() {
	const known = new Set(state.tools.known ?? []);
	return (runner?.getAllRegisteredTools() ?? []).filter(({ definition }) => !known.has(definition.name));
}

function allTools() {
	const registered = new Map((runner?.getAllRegisteredTools() ?? []).map((tool) => [tool.definition.name, tool]));
	const all = (state.tools.all ?? []).map((view) =>
		view.source.kind === "extension" && registered.has(view.name)
			? registeredInfo(registered.get(view.name))
			: viewInfo(view),
	);
	for (const tool of pendingTools()) {
		const info = registeredInfo(tool);
		const index = all.findIndex((existing) => existing.name === info.name);
		if (index >= 0) all[index] = info;
		else all.push(info);
	}
	return all;
}

function activeTools() {
	const active = [...(state.tools.active ?? [])];
	for (const { definition } of pendingTools()) {
		if (activated(definition) && !active.includes(definition.name)) active.push(definition.name);
	}
	return active;
}

function setActiveTools(names) {
	Object.assign(state, askNow("activeTools", { names: [...names] }));
}

function applyCliSources(extensions, cli, cwd) {
	const roots = (cli ?? []).map((entry) => path.resolve(cwd, entry));
	for (const extension of extensions) {
		const resolved = path.resolve(extension.path);
		if (!roots.some((root) => resolved === root || resolved.startsWith(`${root}${path.sep}`))) continue;
		extension.sourceInfo = createSourceInfo(extension.path, { source: "cli", scope: "temporary", origin: "top-level" });
		for (const command of extension.commands.values()) command.sourceInfo = extension.sourceInfo;
		for (const tool of extension.tools.values()) tool.sourceInfo = extension.sourceInfo;
	}
}

function applyFlags(result, flags) {
	const types = new Map();
	for (const extension of result.extensions) {
		for (const [name, flag] of extension.flags) types.set(name, flag.type);
	}
	const errors = [];
	const unknown = [];
	for (const [name, value] of flags ?? []) {
		const type = types.get(name);
		if (!type) {
			unknown.push(name);
			continue;
		}
		if (type === "boolean") {
			result.runtime.flagValues.set(name, true);
			continue;
		}
		if (typeof value === "string") {
			result.runtime.flagValues.set(name, value);
			continue;
		}
		errors.push(`Extension flag "--${name}" requires a value`);
	}
	if (unknown.length > 0) {
		errors.push(`Unknown option${unknown.length === 1 ? "" : "s"}: ${unknown.map((name) => `--${name}`).join(", ")}`);
	}
	return errors;
}

function registerProvider(provider, extensionPath = "") {
	if (typeof provider?.id !== "string" || provider.id.length === 0) {
		throw new Error(`Provider registered by extension "${extensionPath}" must have a non-empty string id.`);
	}
	providers.set(provider.id, { provider, path: extensionPath });
	effect({ call: "providers", providers: providerInfos() });
}

const thrower = (member) => () => {
	throw unported(member);
};
const rejecter = (member) => async () => {
	throw unported(member);
};

const actions = {
	sendMessage: thrower("pi.sendMessage"),
	sendUserMessage: (content, options) => {
		const parts = typeof content === "string" ? [{ type: "text", text: content }] : content;
		effect({ call: "sendUserMessage", content: parts, deliverAs: options?.deliverAs ?? null });
	},
	appendEntry: thrower("pi.appendEntry"),
	setSessionName: thrower("pi.setSessionName"),
	getSessionName: () => state.session?.name ?? undefined,
	setLabel: thrower("pi.setLabel"),
	getActiveTools: activeTools,
	getAllTools: allTools,
	getSettings: thrower("pi.getSettings"),
	setActiveTools,
	refreshTools: () => effect({ call: "tools", tools: toolInfos() }),
	getCommands: thrower("pi.getCommands"),
	setModel: rejecter("pi.setModel"),
	getThinkingLevel: thrower("pi.getThinkingLevel"),
	setThinkingLevel: thrower("pi.setThinkingLevel"),
};

function physicalModel(provider, id) {
	return (state.models?.list ?? []).find(
		(model) => model.provider === provider && model.id === id && !isVirtualModel(model),
	);
}

function limitsModel() {
	const selected = state.models?.current;
	if (!selected || !isVirtualModel(selected)) return selected;
	const latest = findLatestResponse(session.buildSessionProjection().messages);
	return (latest && physicalModel(latest.provider, latest.model)) ?? selected;
}

function contextUsage() {
	const model = limitsModel();
	if (!model) return undefined;
	const contextWindow = model.contextWindow ?? 0;
	if (contextWindow <= 0) return undefined;
	const projection = session.buildSessionProjection();
	const branch = session.getBranch();
	const latestCompaction = getLatestCompactionEntry(branch);
	if (latestCompaction) {
		const projectedAssistants = new Set(
			projection.entries.flatMap((entry) =>
				entry.messages.some(
					(message) =>
						message.role === "assistant" &&
						message.stopReason !== "aborted" &&
						message.stopReason !== "error" &&
						usage.calculateContextTokens(message.usage) > 0,
				)
					? [entry.sourceEntry.id]
					: [],
			),
		);
		const compactionIndex = branch.findIndex((entry) => entry.id === latestCompaction.id);
		const hasPostCompactionUsage = branch
			.slice(compactionIndex + 1)
			.some((entry) => projectedAssistants.has(entry.id));
		if (!hasPostCompactionUsage) return { tokens: null, contextWindow, percent: null };
	}
	const estimate = usage.estimateProjectedContextTokens(projection, branch);
	return { tokens: estimate.tokens, contextWindow, percent: (estimate.tokens / contextWindow) * 100 };
}

async function compact(customInstructions) {
	const answer = await ask("compact", { customInstructions });
	absorb({ updates: { session: answer.session } });
	const entry = getLatestCompactionEntry(session.getBranch());
	if (!entry) throw new Error("the compaction left no compaction entry");
	const projected = session.buildSessionProjection().messages;
	return {
		summary: entry.summary,
		firstKeptEntryId: entry.firstKeptEntryId,
		tokensBefore: entry.tokensBefore,
		estimatedTokensAfter: projected.reduce((sum, message) => sum + usage.estimateTokens(message), 0),
		usage: entry.usage,
		details: entry.details,
	};
}

const contextActions = {
	getModel: () => state.models?.current,
	getScopedModels: thrower("ctx.scopedModels"),
	isIdle: () => !state.streaming,
	isProjectTrusted: () => state.trusted,
	getSignal: () => current.getStore()?.signal,
	abort: () => effect({ call: "abort" }),
	hasPendingMessages: () => state.pending > 0,
	shutdown: () => effect({ call: "shutdown" }),
	getContextUsage: contextUsage,
	compact: (options) => {
		void (async () => {
			try {
				const result = await compact(options?.customInstructions);
				options?.onComplete?.(result);
			} catch (error) {
				options?.onError?.(error instanceof Error ? error : new Error(String(error)));
			}
		})();
	},
	getSystemPrompt: () => state.prompt?.text ?? "",
	getSystemPromptOptions: () => {
		if (!state.prompt) return undefined;
		state.prompt.normalized ??= normalizeBuildSystemPromptOptions(state.prompt.options);
		return state.prompt.normalized;
	},
	executeTool: rejecter("ctx.executeTool"),
	getCallableTools: thrower("ctx.tools"),
};

const providerActions = {
	registerProvider: (name, config) => registerProvider({ ...config, id: name }, registrants.get(name)),
	registerNativeProvider: (provider) => registerProvider(provider, registrants.get(provider?.id)),
	unregisterProvider: (name) => {
		if (providers.delete(name)) effect({ call: "providers", providers: providerInfos() });
	},
	registerVirtualModel: (definition) => {
		const { provider, id } = definition;
		if (!provider.trim() || !id.trim()) throw new Error("Virtual model provider and id must not be empty.");
		const existing = (state.models?.list ?? []).find((model) => model.provider === provider && model.id === id);
		if (existing && !isVirtualModel(existing)) {
			throw new Error(`Virtual model ${provider}/${id} conflicts with a physical model.`);
		}
		const models = virtualModels.get(provider) ?? new Map();
		models.set(id, { model: createVirtualModel(definition), route: (request) => definition.route(request) });
		virtualModels.set(provider, models);
		reportVirtualModels();
	},
	unregisterVirtualModel: (provider, id) => {
		const models = virtualModels.get(provider);
		if (!models?.delete(id)) return;
		if (models.size === 0) virtualModels.delete(provider);
		reportVirtualModels();
	},
};

function reportVirtualModels() {
	const models = [...virtualModels.values()].flatMap((byId) => [...byId.values()].map((entry) => entry.model));
	effect({ call: "virtualModels", models });
}

async function route(request) {
	absorb(request);
	const { provider, id, reason, thinkingLevel, messages, failed } = request.route;
	const virtual = virtualModels.get(provider)?.get(id);
	if (!virtual) throw new Error(`Virtual model ${provider}/${id} is not registered.`);
	const stored = request.route.state;
	const asked = { reason, thinkingLevel, signal: current.getStore()?.signal };
	if (reason !== "direct") asked.state = stored;
	const latest = findLatestResponse(messages);
	const previousModel = latest && physicalModel(latest.provider, latest.model);
	const failedModel = failed && physicalModel(failed.provider, failed.model);
	const routed = await virtual.route({
		...asked,
		model: virtual.model,
		previous: previousModel && { model: previousModel, thinkingLevel: latest?.thinkingLevel },
		failed: failedModel && failed && { model: failedModel, thinkingLevel: failed.thinkingLevel, message: failed },
		messages,
	});
	const changed = reason !== "direct" && routed.state !== undefined && routed.state !== stored;
	return {
		provider: routed.model.provider,
		id: routed.model.id,
		thinkingLevel: routed.thinkingLevel,
		state: changed ? { value: routed.state } : null,
	};
}

function unset(payload, path) {
	let parent = payload;
	for (const key of path.slice(0, -1)) parent = parent?.[key];
	if (parent && typeof parent === "object") parent[path.at(-1)] = undefined;
}

const commandActions = {
	waitForIdle: rejecter("ctx.waitForIdle"),
	newSession: rejecter("ctx.newSession"),
	fork: rejecter("ctx.fork"),
	navigateTree: rejecter("ctx.navigateTree"),
	switchSession: rejecter("ctx.switchSession"),
	reload: rejecter("ctx.reload"),
};

/** The UI pi-rs draws: `notify`; each other member of upstream's UI context is reported and thrown. */
function uiOf(noOp) {
	const ui = {};
	for (const [name, value] of Object.entries(noOp)) {
		ui[name] = typeof value === "function" ? thrower(`ctx.ui.${name}`) : value;
	}
	ui.notify = (message, type) => effect({ call: "notify", text: String(message), level: type ?? "info" });
	return ui;
}

/** What an extension registered that upstream acts on in this mode and pi-rs does not yet; renderers and shortcuts only act in the TUI. */
function unconsumed(extension, servers) {
	const used = [];
	if (servers.some((server) => server.extensionPath === extension.path)) used.push("registerMcpServer");
	if (state.mode !== "tui") return used;
	if (extension.shortcuts.size > 0) used.push("registerShortcut");
	if (extension.messageRenderers.size > 0) used.push("registerMessageRenderer");
	if (extension.toolRenderers?.length > 0) used.push("registerToolRenderer");
	if (extension.entryRenderers?.size > 0) used.push("registerEntryRenderer");
	if (extension.markdownTransformer) used.push("registerMarkdownTransformer");
	return used;
}

function summary(extension, servers) {
	return {
		path: extension.path,
		resolvedPath: extension.resolvedPath,
		events: [...extension.handlers.keys()],
		commands: [...extension.commands.values()].map((command) => ({
			name: command.name,
			description: command.description ?? "",
		})),
		unsupported: unconsumed(extension, servers),
	};
}

async function load(request) {
	absorb(request);
	state.trusted = request.trusted === true;
	if (request.agentDir) process.env.PI_CODING_AGENT_DIR = request.agentDir;
	initTheme(request.theme, false);
	runner?.invalidate();
	providers.clear();
	const hadVirtualModels = virtualModels.size > 0;
	virtualModels.clear();
	const result = await loadExtensions(resolvePaths(request), state.cwd, createEventBus());
	applyCliSources(result.extensions, request.cli, request.cwd ?? state.cwd);
	registrants.clear();
	for (const { name, extensionPath } of result.runtime.pendingProviderRegistrations) registrants.set(name, extensionPath);
	for (const { provider, extensionPath } of result.runtime.pendingNativeProviderRegistrations) {
		registrants.set(provider?.id, extensionPath);
	}
	runner = new ExtensionRunner(result.extensions, result.runtime, state.cwd, session, modelRegistry);
	runner.onError((error) =>
		effect({ call: "error", path: error.extensionPath, event: error.event ?? "", error: error.error }),
	);
	runner.bindCore(actions, contextActions, providerActions);
	if (hadVirtualModels && virtualModels.size === 0) reportVirtualModels();
	runner.setUIContext(undefined, state.mode);
	if (state.hasUI) runner.setUIContext(uiOf(runner.getUIContext()), state.mode);
	runner.bindCommandContext(commandActions);
	return {
		extensions: result.extensions.map((extension) => summary(extension, result.runtime.mcpServers.list())),
		errors: result.errors,
		tools: toolInfos(),
		providers: providerInfos(),
		flagErrors: applyFlags(result, request.flags),
	};
}

async function emitEvent(request) {
	absorb(request);
	if (request.name === "before_provider_request") {
		const { payload, unset: declared } = request.event;
		for (const path of declared) unset(payload, path);
		return { payload: await runner.emitBeforeProviderRequest(payload) };
	}
	const event = { type: request.name, ...request.event };
	if (request.name === "message_end") {
		const message = await runner.emitMessageEnd(event);
		return message ? { message } : null;
	}
	if (request.name === "input") {
		return await runner.emitInput(event.text, event.images, event.source ?? "interactive", event.streamingBehavior);
	}
	if (request.name === "before_agent_start") {
		const trigger = { messageType: "user", prompt: event.prompt, images: event.images };
		return await runner.emitBeforeAgentStart(trigger, event.systemPromptOptions);
	}
	if (request.name === "turn_end") return await turnEnd(event.turnIndex);
	if (request.name === "resources_discover") return await runner.emitResourcesDiscover(event.cwd, event.reason);
	await runner.emit(event);
	return null;
}

function boundaryContext() {
	const projection = session.buildSessionProjection();
	const llmMessages = convertToLlm(projection.messages);
	const finalRole = llmMessages.at(-1)?.role;
	const runnable = llmMessages.some((message) => message.role !== "system") && finalRole !== "assistant";
	return {
		contextEntries: projection.entries,
		contextMessages: projection.messages,
		llmMessages,
		pendingMessages: [],
		canContinue: runnable || state.pending > 0,
	};
}

async function turnEnd(turnIndex) {
	if (!runner.hasHandlers("turn_end")) return null;
	const branch = session.getBranch();
	const at = branch.findLastIndex((entry) => entry.type === "message" && entry.message.role === "assistant");
	if (at < 0) {
		runner.emitError({
			extensionPath: "<boundary>",
			event: "turn_end",
			error: "turn_end could not resolve the persisted assistant entry ID",
		});
		return null;
	}
	const results = [];
	for (const entry of branch.slice(at + 1)) {
		if (entry.type !== "message" || entry.message.role !== "toolResult") break;
		results.push(entry);
	}
	const message = branch[at].message;
	const outcome = message.stopReason === "aborted" ? "aborted" : message.stopReason === "error" ? "error" : "completed";
	const boundary = await runner.emitBoundary(
		{
			type: "turn_end",
			turnIndex,
			message,
			toolResults: results.map((entry) => entry.message),
			messageEntryId: branch[at].id,
			toolResultEntryIds: results.map((entry) => entry.id),
			outcome,
		},
		boundaryContext,
	);
	return { entries: boundary.entries, continue: boundary.continue };
}

async function runCommand(request) {
	absorb(request);
	const command = runner.getCommand(request.name);
	if (!command) return null;
	try {
		await command.handler(request.args ?? "", runner.createCommandContext());
	} catch (error) {
		runner.emitError({
			extensionPath: `command:${request.name}`,
			event: "command",
			error: error instanceof Error ? error.message : String(error),
		});
	}
	return null;
}

async function runTool(request) {
	absorb(request);
	const definition = runner.getToolDefinition(request.name);
	if (!definition) throw new Error(`Tool ${request.name} not found`);
	const signal = current.getStore()?.signal;
	const onUpdate = (partial) => send({ id: request.id, update: partial ?? null });
	const result = await definition.execute(
		request.toolCallId,
		request.params,
		signal,
		onUpdate,
		runner.createToolContext(request.toolCallId, signal),
	);
	return result ?? null;
}

/** One request to a provider an extension registered, each event its stream yields sent as it comes. */
async function runStream(request) {
	const provider = providers.get(request.provider)?.provider;
	if (typeof provider?.streamSimple !== "function") {
		throw new Error(`No extension registered a provider ${request.provider} that streams`);
	}
	const options = { ...request.options, signal: current.getStore()?.signal };
	if (request.hooks) {
		options.onPayload = (payload) =>
			runner.hasHandlers("before_provider_request") ? runner.emitBeforeProviderRequest(payload) : payload;
		options.onResponse = async (response) => {
			if (!runner.hasHandlers("after_provider_response")) return;
			await runner.emit({ type: "after_provider_response", status: response.status, headers: response.headers });
		};
		options.onProviderStreamEvent = async (data, model) => {
			if (!runner.hasHandlers("provider_stream_event")) return;
			await runner.emit({ data, type: "provider_stream_event", provider: model.provider, api: model.api, model: model.id });
		};
	}
	for await (const event of provider.streamSimple(request.model, request.context, options)) {
		send({ id: request.id, update: event });
	}
	return null;
}

async function handle(request) {
	switch (request.op) {
		case "load":
			return await load(request);
		case "event":
			return await emitEvent(request);
		case "command":
			return await runCommand(request);
		case "tool":
			return await runTool(request);
		case "stream":
			return await runStream(request);
		case "route":
			return await route(request);
		case "shutdown":
			exit();
		default:
			throw new Error(`unknown op: ${request.op}`);
	}
}

function receive(line) {
	let request;
	try {
		request = JSON.parse(line);
	} catch {
		return;
	}
	if (request.answer !== undefined) {
		const waiting = answers.get(request.answer);
		if (!waiting) return;
		if ("update" in request) {
			waiting.update?.(request.update);
			return;
		}
		answers.delete(request.answer);
		if (request.ok === true) waiting.resolve(request.value);
		else waiting.reject(new Error(request.error ?? "the call failed"));
		return;
	}
	if (request.cancel !== undefined) {
		inflight.get(request.cancel)?.abort();
		return;
	}
	const controller = new AbortController();
	inflight.set(request.id, controller);
	current.run({ id: request.id, signal: controller.signal }, async () => {
		try {
			const value = await handle(request);
			send({ id: request.id, ok: true, value: value ?? null });
		} catch (error) {
			send({ id: request.id, ok: false, error: error instanceof Error ? error.message : String(error) });
		} finally {
			inflight.delete(request.id);
		}
	});
}

const reader = new Worker(READER, {
	eval: true,
	workerData: { blocked: blocked.buffer, port: answering },
	transferList: [answering],
});
reader.on("message", (line) => (line === null ? exit() : receive(line)));
reader.on("error", (error) => {
	process.stderr.write(`the extension host stopped reading pi-rs: ${error?.stack ?? error}\n`);
	killAll();
	process.exit(1);
});
