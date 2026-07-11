// Regenerates tests/session-parity/oracle.json by driving Pi's real
// AgentSession + SessionManager (ref/pi @ c5582102) with scripted streams,
// scripted tools, and event-count triggers described in cases.json. For each
// case the oracle records the session JSONL file Pi persists — whether the
// file exists (SessionManager._persist defers file creation until the first
// assistant message) and every entry, with uuids normalized to U1..Un in
// first-appearance order, ISO entry timestamps scrubbed to "TS", numeric
// message timestamps scrubbed to 0, and the case cwd substituted with {CWD}.
// The Lua replay (coding-agent.lua `session-parity`) runs the same cases
// through pi-rs's product persistence policy (utils/agent-session.lua over
// pi.session.*); crates/pi-rs-app/tests/session_parity.rs compares parsed
// entries (order-sensitive per line sequence, key-order-insensitive per
// entry: Lua tables do not preserve JS insertion order).
// Run via scripts/session-oracle. Do not edit the oracle by hand.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// tools-manager.ts computes TOOLS_DIR at import time, so the env pin must
// land before any coding-agent module loads (tool-oracle precedent).
process.env.PI_CODING_AGENT_DIR = mkdtempSync(join(tmpdir(), "pi-rs-session-oracle-agentdir-"));

type Json = any;

const tick = () => new Promise((resolve) => setImmediate(resolve));
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function deepCopy<T>(value: T): T {
	return JSON.parse(JSON.stringify(value));
}

const EMPTY_USAGE = {
	input: 0,
	output: 0,
	cacheRead: 0,
	cacheWrite: 0,
	totalTokens: 0,
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

function baseMessage(model: Json, content: Json[], stopReason: string): Json {
	return {
		role: "assistant",
		content,
		api: model.api,
		provider: model.provider,
		model: model.id,
		usage: deepCopy(EMPTY_USAGE),
		stopReason,
		timestamp: 0,
	};
}

/**
 * Scripted stream synthesis — mirrored 1:1 by the Lua `session-parity`
 * command (itself a copy of tests/agent-parity machinery, which pins the
 * shared event shapes against Pi's loop).
 */
function synthesize(turn: Json, model: Json): { events: Json[]; final: Json } {
	const blocks: Json[] = turn.blocks ?? [];
	const snapshot = (count: number, current?: Json): Json[] => {
		const content = blocks.slice(0, count).map(deepCopy);
		if (current !== undefined) content.push(current);
		return content;
	};
	const events: Json[] = [{ type: "start", partial: baseMessage(model, [], "stop") }];
	blocks.forEach((block, index) => {
		const ci = index;
		if (block.type === "text") {
			events.push({
				type: "text_start",
				contentIndex: ci,
				partial: baseMessage(model, snapshot(index, { type: "text", text: "" }), "stop"),
			});
			events.push({
				type: "text_delta",
				contentIndex: ci,
				delta: block.text,
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
			events.push({
				type: "text_end",
				contentIndex: ci,
				content: block.text,
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
		} else if (block.type === "thinking") {
			events.push({
				type: "thinking_start",
				contentIndex: ci,
				partial: baseMessage(model, snapshot(index, { type: "thinking", thinking: "" }), "stop"),
			});
			events.push({
				type: "thinking_delta",
				contentIndex: ci,
				delta: block.thinking,
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
			events.push({
				type: "thinking_end",
				contentIndex: ci,
				content: block.thinking,
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
		} else if (block.type === "toolCall") {
			events.push({
				type: "toolcall_start",
				contentIndex: ci,
				partial: baseMessage(
					model,
					snapshot(index, { type: "toolCall", id: block.id, name: block.name, arguments: {} }),
					"stop",
				),
			});
			events.push({
				type: "toolcall_delta",
				contentIndex: ci,
				delta: JSON.stringify(block.arguments),
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
			events.push({
				type: "toolcall_end",
				contentIndex: ci,
				toolCall: deepCopy(block),
				partial: baseMessage(model, snapshot(index + 1), "stop"),
			});
		} else {
			throw new Error(`unknown block type ${block.type}`);
		}
	});
	const final = baseMessage(model, snapshot(blocks.length), turn.stopReason ?? "stop");
	if (turn.errorMessage !== undefined) final.errorMessage = turn.errorMessage;
	const terminal =
		turn.stopReason === "error" || turn.stopReason === "aborted"
			? { type: "error", reason: turn.stopReason, error: final }
			: { type: "done", reason: turn.stopReason ?? "stop", message: final };
	events.push(terminal);
	return { events, final };
}

function buildTool(toolSpec: Json) {
	let count = 0;
	const invocations: Json[] = toolSpec.invocations ?? [];
	return {
		label: toolSpec.name,
		name: toolSpec.name,
		description: `scripted ${toolSpec.name}`,
		parameters: toolSpec.parameters,
		executionMode: toolSpec.executionMode,
		execute: async (_id: string, _args: unknown, signal?: AbortSignal, onUpdate?: (partial: Json) => void) => {
			const inv = invocations.length > 0 ? invocations[Math.min(count, invocations.length - 1)] : {};
			count += 1;
			const check = () => {
				if (inv.abortCheck && signal?.aborted) throw new Error(`${toolSpec.name} aborted`);
			};
			check();
			for (const update of inv.updates ?? []) {
				if (update.sleepMs) await sleep(update.sleepMs);
				check();
				onUpdate?.(deepCopy(update.partial));
			}
			if (inv.sleepMs) await sleep(inv.sleepMs);
			check();
			if (inv.throw) throw new Error(inv.throw);
			return deepCopy(inv.result ?? { content: [{ type: "text", text: `${toolSpec.name} ok` }], details: {} });
		},
	};
}

const ISO_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;

/** Scrub timestamps everywhere and cwd occurrences in strings. */
function scrubValues(value: Json, cwd: string): Json {
	if (Array.isArray(value)) return value.map((item) => scrubValues(item, cwd));
	if (value !== null && typeof value === "object") {
		const out: Record<string, Json> = {};
		for (const [key, item] of Object.entries(value)) {
			if (key === "timestamp" && typeof item === "number") out[key] = 0;
			else if (key === "timestamp" && typeof item === "string" && ISO_TIMESTAMP.test(item)) out[key] = "TS";
			else out[key] = scrubValues(item, cwd);
		}
		return out;
	}
	if (typeof value === "string" && value.includes(cwd)) {
		return value.split(cwd).join("{CWD}");
	}
	return value;
}

/**
 * Normalize the session file: uuids (entry `id`/`parentId` and the header
 * id) map to U1..Un in first-appearance order; timestamps and cwd scrubbed.
 */
function normalizeEntries(lines: string[], cwd: string): Json[] {
	const idMap = new Map<string, string>();
	const mapId = (id: string): string => {
		if (!idMap.has(id)) idMap.set(id, `U${idMap.size + 1}`);
		return idMap.get(id)!;
	};
	return lines.map((line) => {
		const entry = JSON.parse(line);
		if (typeof entry.id === "string") entry.id = mapId(entry.id);
		if (typeof entry.parentId === "string") entry.parentId = mapId(entry.parentId);
		return scrubValues(entry, cwd);
	});
}

async function main() {
	// Dynamic imports so the PI_CODING_AGENT_DIR pin above lands first.
	const { Agent } = await import("../../ref/pi/packages/agent/src/agent.ts");
	const { AssistantMessageEventStream } = await import("../../ref/pi/packages/ai/src/utils/event-stream.ts");
	const { AgentSession } = await import("../../ref/pi/packages/coding-agent/src/core/agent-session.ts");
	const { AuthStorage } = await import("../../ref/pi/packages/coding-agent/src/core/auth-storage.ts");
	const { ModelRegistry } = await import("../../ref/pi/packages/coding-agent/src/core/model-registry.ts");
	const { SessionManager } = await import("../../ref/pi/packages/coding-agent/src/core/session-manager.ts");
	const { SettingsManager } = await import("../../ref/pi/packages/coding-agent/src/core/settings-manager.ts");
	const { createTestResourceLoader } = await import("../../ref/pi/packages/coding-agent/test/utilities.ts");

	function makeStreamFn(caseSpec: Json) {
		let turnIndex = 0;
		return (model: Json, _context: Json, options: Json) => {
			const turn = caseSpec.turns[Math.min(turnIndex, caseSpec.turns.length - 1)];
			turnIndex += 1;
			if (turn.throw) throw new Error(turn.throw);
			const stream = new AssistantMessageEventStream();
			const signal: AbortSignal | undefined = options?.signal;
			void (async () => {
				const { events } = synthesize(turn, model);
				let lastContent: Json[] = [];
				for (const event of events) {
					await tick();
					if (signal?.aborted) {
						const aborted = baseMessage(model, lastContent, "aborted");
						aborted.errorMessage = "Request was aborted";
						stream.push({ type: "error", reason: "aborted", error: aborted });
						return;
					}
					stream.push(event);
					const partial = event.partial ?? event.message ?? event.error;
					if (partial?.content) lastContent = deepCopy(partial.content);
				}
			})();
			return stream;
		};
	}

	const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Json;
	const results: Json[] = [];

	for (const caseSpec of spec.cases) {
		const options = caseSpec.options ?? {};
		const models = spec.models;
		const model = models[options.model ?? "default"];

		const tempDir = mkdtempSync(join(tmpdir(), "pi-rs-session-oracle-"));
		const cwd = join(tempDir, "work");
		mkdirSync(cwd, { recursive: true });
		const sessionDir = join(tempDir, "sessions");

		const sessionManager = SessionManager.create(cwd, sessionDir);
		const settingsManager = SettingsManager.create(cwd, tempDir);
		const authStorage = AuthStorage.create(join(tempDir, "auth.json"));
		for (const m of Object.values(models) as Json[]) {
			authStorage.setRuntimeApiKey(m.provider, "oracle-key");
		}
		const modelRegistry = ModelRegistry.create(authStorage, join(tempDir, "models.json"));

		const tools = (caseSpec.tools ?? []).map(buildTool);
		const agent = new Agent({
			getApiKey: () => "oracle-key",
			initialState: {
				systemPrompt: options.systemPrompt ?? "",
				model,
				thinkingLevel: options.thinkingLevel,
				tools,
				messages: [],
			},
			streamFn: makeStreamFn(caseSpec) as Json,
		});

		const session = new AgentSession({
			agent,
			sessionManager,
			settingsManager,
			cwd,
			modelRegistry,
			resourceLoader: createTestResourceLoader(),
			baseToolsOverride: Object.fromEntries(tools.map((tool: Json) => [tool.name, tool])),
		});

		// sdk.ts createAgentSession — the new-session initial appends (the
		// sdk factory itself builds real provider transports, so the oracle
		// replays its persistence slice over the same managers).
		if (model) sessionManager.appendModelChange(model.provider, model.id);
		sessionManager.appendThinkingLevelChange(options.thinkingLevel ?? "off");

		const counts: Record<string, number> = {};
		const fired = new Set<number>();
		session.subscribe((event: Json) => {
			counts[event.type] = (counts[event.type] ?? 0) + 1;
			(caseSpec.triggers ?? []).forEach((trigger: Json, index: number) => {
				if (fired.has(index)) return;
				if (trigger.on.event !== event.type || counts[event.type] !== trigger.on.count) return;
				fired.add(index);
				if (trigger.action === "abort") void session.abort();
				else if (trigger.action === "steer") void session.prompt(trigger.text, { streamingBehavior: "steer" });
				else if (trigger.action === "followUp")
					void session.prompt(trigger.text, { streamingBehavior: "followUp" });
				else throw new Error(`unknown trigger action ${trigger.action}`);
			});
		});

		for (const op of caseSpec.ops ?? []) {
			if (op.op === "prompt") await session.prompt(op.text);
			else if (op.op === "setName") session.setSessionName(op.name);
			else if (op.op === "setModel") await session.setModel(models[op.model]);
			else throw new Error(`unknown op ${op.op}`);
		}

		const sessionFile = sessionManager.getSessionFile();
		const exists = sessionFile !== undefined && existsSync(sessionFile);
		const entries = exists
			? normalizeEntries(
					readFileSync(sessionFile!, "utf8")
						.split("\n")
						.filter((line) => line.trim().length > 0),
					cwd,
				)
			: [];
		results.push({ name: caseSpec.name, exists, entries });

		session.dispose?.();
		rmSync(tempDir, { recursive: true, force: true });
	}

	console.log(JSON.stringify({ cases: results }, null, "\t"));
}

void main().catch((error) => {
	console.error(error);
	process.exit(1);
});
