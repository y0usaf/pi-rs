// Regenerates tests/agent-parity/oracle.json by driving Pi's real Agent /
// agent-loop.ts (ref/pi @ c5582102) with scripted streams, scripted tools,
// scripted hooks, and event-count triggers described in cases.json. For each
// case the oracle records the full subscriber event sequence (deep-copied at
// dispatch, timestamps scrubbed to 0), every stream-call request snapshot
// (model id, reasoning, systemPrompt, converted messages), per-phase
// prompt/continue outcomes, and the final agent state. The Lua driver
// (driver.lua) replays the same cases through pi.agent.new; both sides
// synthesize identical stream events from each turn spec.
// Run via scripts/agent-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import { Agent } from "../../ref/pi/packages/agent/src/agent.ts";
import { AssistantMessageEventStream } from "../../ref/pi/packages/ai/src/utils/event-stream.ts";

type Json = any;

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Json;

const EMPTY_USAGE = {
	input: 0,
	output: 0,
	cacheRead: 0,
	cacheWrite: 0,
	totalTokens: 0,
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const tick = () => new Promise((resolve) => setImmediate(resolve));

function deepCopy<T>(value: T): T {
	return JSON.parse(JSON.stringify(value));
}

/** Scrub every `timestamp` field to 0 so wall-clock values never enter the oracle. */
function scrub(value: Json): Json {
	if (Array.isArray(value)) return value.map(scrub);
	if (value !== null && typeof value === "object") {
		const out: Record<string, Json> = {};
		for (const [key, item] of Object.entries(value)) {
			out[key] = key === "timestamp" && typeof item === "number" ? 0 : scrub(item);
		}
		return out;
	}
	return value;
}

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
 * Synthesize the scripted stream event list for a turn spec. Mirrored 1:1 by
 * driver.lua synthesize(); the recorded message_update events pin any drift.
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

function makeStreamFn(caseSpec: Json, recorder: { requests: Json[] }) {
	let turnIndex = 0;
	return (model: Json, context: Json, options: Json) => {
		const turn = caseSpec.turns[Math.min(turnIndex, caseSpec.turns.length - 1)];
		turnIndex += 1;
		recorder.requests.push(
			scrub(
				deepCopy({
					model: model.id,
					reasoning: options?.reasoning ?? "none",
					systemPrompt: context.systemPrompt ?? "",
					messages: context.messages,
				}),
			),
		);
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

function scriptedHook<T>(scripts: Json[] | undefined, apply: (entry: Json) => T): ((...args: Json[]) => Promise<T | undefined>) | undefined {
	if (!scripts) return undefined;
	let index = 0;
	return async () => {
		const entry = scripts[Math.min(index, scripts.length - 1)];
		index += 1;
		if (!entry || entry.skip) return undefined;
		if (entry.throw) throw new Error(entry.throw);
		return apply(entry);
	};
}

async function runCase(caseSpec: Json, models: Json): Promise<Json> {
	const options = caseSpec.options ?? {};
	const model = models[options.model ?? "default"];
	const recorder = { events: [] as Json[], requests: [] as Json[] };
	const hooks = caseSpec.hooks ?? {};
	const agent = new Agent({
		initialState: {
			systemPrompt: options.systemPrompt ?? "",
			model,
			thinkingLevel: options.thinkingLevel,
			tools: (caseSpec.tools ?? []).map(buildTool),
			messages: deepCopy(options.initialMessages ?? []),
		},
		streamFn: makeStreamFn(caseSpec, recorder) as Json,
		toolExecution: options.toolExecution,
		steeringMode: options.steeringMode,
		followUpMode: options.followUpMode,
		beforeToolCall: scriptedHook(hooks.beforeToolCall, (entry) => ({
			block: entry.block,
			reason: entry.reason,
		})) as Json,
		afterToolCall: scriptedHook(hooks.afterToolCall, (entry) => ({
			content: entry.content,
			details: entry.details,
			isError: entry.isError,
			terminate: entry.terminate,
		})) as Json,
		prepareNextTurn: scriptedHook(hooks.prepareNextTurn, (entry) => ({
			model: entry.model ? models[entry.model] : undefined,
			thinkingLevel: entry.thinkingLevel,
		})) as Json,
	});

	const counts: Record<string, number> = {};
	const fired = new Set<number>();
	agent.subscribe((event: Json) => {
		recorder.events.push(scrub(deepCopy(event)));
		counts[event.type] = (counts[event.type] ?? 0) + 1;
		(caseSpec.triggers ?? []).forEach((trigger: Json, index: number) => {
			if (fired.has(index)) return;
			if (trigger.on.event !== event.type || counts[event.type] !== trigger.on.count) return;
			fired.add(index);
			if (trigger.action === "steer") agent.steer(deepCopy(trigger.message));
			else if (trigger.action === "followUp") agent.followUp(deepCopy(trigger.message));
			else if (trigger.action === "abort") agent.abort();
			else throw new Error(`unknown trigger action ${trigger.action}`);
		});
	});

	const phases: Json[] = [];
	for (const phase of caseSpec.phases) {
		for (const message of phase.steer ?? []) agent.steer(deepCopy(message));
		for (const message of phase.followUp ?? []) agent.followUp(deepCopy(message));
		try {
			if (phase.continue) await agent.continue();
			else await agent.prompt(deepCopy(phase.prompt));
			phases.push({ ok: true });
		} catch (error) {
			phases.push({ ok: false, error: error instanceof Error ? error.message : String(error) });
		}
	}

	const state: Json = { messages: scrub(deepCopy(agent.state.messages)) };
	if (agent.state.errorMessage !== undefined) state.errorMessage = agent.state.errorMessage;
	return {
		name: caseSpec.name,
		events: recorder.events,
		requests: recorder.requests,
		phases,
		state,
	};
}

async function main(): Promise<void> {
	const results: Json[] = [];
	for (const caseSpec of spec.cases) {
		results.push(await runCase(caseSpec, spec.models));
	}
	process.stdout.write(`${JSON.stringify({ cases: results }, null, "\t")}\n`);
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
