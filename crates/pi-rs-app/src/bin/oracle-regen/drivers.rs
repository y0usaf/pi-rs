//! Embedded oracle-regeneration drivers (Rust port of the deleted
//! tests/*-parity/gen-oracle.ts files; PLAN A.3). Each driver runs the
//! hash-pinned vendored Pi code (ref/pi) against scripted inputs and
//! prints the canonical oracle JSON. The oracle-regen binary
//! materializes the requested driver into its parity directory, runs it
//! with the pinned node/bun runtime, and writes oracle.json.

/// Registry of supported parity dirs: name -> (driver source, input file, runner).
// Driver spec: (parity name, driver source, input file, runner, env keys to unset).
pub type ParitySpec<'a> = (&'a str, &'a str, Option<&'a str>, &'a str, &'a [&'a str]);
pub const PARITIES: &[ParitySpec] = &[
    ("agent", AGENT_DRIVER, Some("cases.json"), "tsx", &[]),
    ("anthropic", ANTHROPIC_DRIVER, Some("cases.json"), "tsx", &[]),
    ("azure-openai-responses", AZURE_OPENAI_RESPONSES_DRIVER, Some("cases.json"), "tsx", &[]),
    ("bedrock-converse-stream", BEDROCK_CONVERSE_STREAM_DRIVER, Some("cases.json"), "tsx", &["AWS_BEARER_TOKEN_BEDROCK", "AWS_PROFILE", "AWS_REGION", "AWS_DEFAULT_REGION"]),
    ("compaction", COMPACTION_DRIVER, Some("cases.json"), "tsx", &[]),
    ("export-html", EXPORT_HTML_DRIVER, Some("case.json"), "tsx", &[]),
    ("extension-context", EXTENSION_CONTEXT_DRIVER, None, "bun", &[]),
    ("extension-event", EXTENSION_EVENT_DRIVER, None, "bun", &[]),
    ("extension-runtime", EXTENSION_RUNTIME_DRIVER, None, "bun", &[]),
    ("google-generative-ai", GOOGLE_GENERATIVE_AI_DRIVER, Some("cases.json"), "tsx", &[]),
    ("google-vertex", GOOGLE_VERTEX_DRIVER, Some("cases.json"), "tsx", &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
    ("hljs", HLJS_DRIVER, Some("cases.json"), "tsx", &[]),
    ("image", IMAGE_DRIVER, Some("cases.json"), "tsx", &[]),
    ("jsdiff", JSDIFF_DRIVER, Some("cases.json"), "tsx", &[]),
    ("mistral-conversations", MISTRAL_CONVERSATIONS_DRIVER, Some("cases.json"), "tsx", &["MISTRAL_API_KEY"]),
    ("openai-codex-responses", OPENAI_CODEX_RESPONSES_DRIVER, Some("cases.json"), "tsx", &[]),
    ("openai-codex-websocket", OPENAI_CODEX_WEBSOCKET_DRIVER, Some("cases.json"), "tsx", &[]),
    ("openai-completions", OPENAI_COMPLETIONS_DRIVER, Some("cases.json"), "tsx", &["OPENAI_API_KEY", "OPENROUTER_API_KEY", "DEEPSEEK_API_KEY", "MOONSHOT_API_KEY"]),
    ("openai-responses", OPENAI_RESPONSES_DRIVER, Some("cases.json"), "tsx", &[]),
    ("retry", RETRY_DRIVER, Some("cases.json"), "tsx", &[]),
    ("session", SESSION_DRIVER, Some("cases.json"), "tsx", &[]),
    ("system-prompt", SYSTEM_PROMPT_DRIVER, Some("cases.json"), "tsx", &[]),
    ("tool", TOOL_DRIVER, Some("cases.json"), "tsx", &[]),
];

/// agent-parity driver (port of tests/agent-parity/gen-oracle.ts).
pub const AGENT_DRIVER: &str = r#"// Regenerates tests/agent-parity/oracle.json by driving Pi's real Agent /
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
"#;

/// anthropic-parity driver (port of tests/anthropic-parity/gen-oracle.ts).
pub const ANTHROPIC_DRIVER: &str = r#"// Regenerates tests/anthropic-parity/oracle.json by running Pi's real
// `streamAnthropic`/`streamSimpleAnthropic` (ref/pi @ c5582102, vendored
// @anthropic-ai/sdk 0.91.1) against a scripted local HTTP stub. For each
// case the oracle records every captured HTTP request (method, path,
// meaningful headers, body), the emitted event sequence (without their
// partial/message snapshots), and the final message from `result()`.
// Run via scripts/anthropic-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { Socket } from "node:net";
import {
	streamAnthropic,
	streamSimpleAnthropic,
	type AnthropicOptions,
} from "../../ref/pi/packages/ai/src/providers/anthropic.ts";
import type { Context, Model, SimpleStreamOptions } from "../../ref/pi/packages/ai/src/types.ts";

type SseEvent = { event: string; data: unknown };
type ScriptedResponse = {
	status: number;
	sse?: string;
	events?: SseEvent[];
	json?: unknown;
	text?: string;
	headers?: Record<string, string>;
	hang?: boolean;
};
type Case = {
	name: string;
	model: string;
	simple?: boolean;
	context: Context;
	options: Record<string, unknown>;
	responses: ScriptedResponse[];
	abortAfterEvents?: number;
};
type Cases = {
	models: Record<string, Model<"anthropic-messages">>;
	sse: Record<string, SseEvent[]>;
	cases: Case[];
};

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Cases;

type CapturedRequest = {
	method: string;
	path: string;
	headers: Record<string, string>;
	body: unknown;
};

// host/content-length/connection/accept-encoding vary by client;
// accept-language and sec-fetch-mode are undici fetch artifacts;
// x-stainless-* is SDK telemetry. None are provider-meaningful.
const DROPPED_HEADERS = new Set([
	"host",
	"content-length",
	"connection",
	"accept-encoding",
	"accept-language",
	"sec-fetch-mode",
]);

/** Keep only wire-meaningful headers; SDK/HTTP-client telemetry is noise. */
function filterHeaders(raw: Record<string, string | string[] | undefined>): Record<string, string> {
	const entries: Array<[string, string]> = [];
	for (const [key, value] of Object.entries(raw)) {
		const name = key.toLowerCase();
		if (DROPPED_HEADERS.has(name) || name.startsWith("x-stainless-")) continue;
		const text = Array.isArray(value) ? value.join(", ") : (value ?? "");
		if (name === "user-agent" && !text.startsWith("claude-cli/")) continue;
		entries.push([name, text]);
	}
	entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
	return Object.fromEntries(entries);
}

function sseBody(events: SseEvent[]): string {
	return events.map((e) => `event: ${e.event}\ndata: ${JSON.stringify(e.data)}\n\n`).join("");
}

function responseBody(response: ScriptedResponse, shared: Record<string, SseEvent[]>): { body: string; contentType: string } {
	const events = response.sse ? shared[response.sse]! : response.events;
	if (events) return { body: sseBody(events), contentType: "text/event-stream" };
	if (response.json !== undefined) return { body: JSON.stringify(response.json), contentType: "application/json" };
	return { body: response.text ?? "", contentType: "text/plain" };
}

function serveCase(c: Case): Promise<{ server: Server; url: string; requests: CapturedRequest[]; sockets: Set<Socket> }> {
	const requests: CapturedRequest[] = [];
	const sockets = new Set<Socket>();
	let index = 0;
	const server = createServer((req, res) => {
		const chunks: Buffer[] = [];
		req.on("data", (chunk) => chunks.push(chunk));
		req.on("end", () => {
			const bodyText = Buffer.concat(chunks).toString("utf8");
			requests.push({
				method: req.method ?? "",
				path: req.url ?? "",
				headers: filterHeaders(req.headers),
				body: bodyText.length > 0 ? JSON.parse(bodyText) : null,
			});
			const scripted = c.responses[index] ?? c.responses[c.responses.length - 1];
			index += 1;
			if (!scripted) {
				res.writeHead(500).end("no scripted response");
				return;
			}
			const { body, contentType } = responseBody(scripted, spec.sse);
			res.writeHead(scripted.status, { "content-type": contentType, ...(scripted.headers ?? {}) });
			if (scripted.hang) {
				res.write(body);
				// Hold the connection open; the driver aborts.
			} else {
				res.end(body);
			}
		});
	});
	server.on("connection", (socket) => {
		sockets.add(socket);
		socket.on("close", () => sockets.delete(socket));
	});
	return new Promise((resolve) => {
		server.listen(0, "127.0.0.1", () => {
			const address = server.address() as { port: number };
			resolve({ server, url: `http://127.0.0.1:${address.port}`, requests, sockets });
		});
	});
}

/** Event JSON minus the `partial`/`message`/`error` snapshots. */
function summarize(event: Record<string, unknown>): Record<string, unknown> {
	const { partial: _p, message: _m, error: _e, ...rest } = event;
	return rest;
}

function normalizeMessage(message: Record<string, unknown>): Record<string, unknown> {
	return { ...message, timestamp: 0 };
}

async function runCase(c: Case): Promise<Record<string, unknown>> {
	const { server, url, requests, sockets } = await serveCase(c);
	const model = { ...spec.models[c.model]!, baseUrl: url };
	const controller = new AbortController();
	const events: Array<Record<string, unknown>> = [];
	let result: Record<string, unknown> | undefined;
	let syncError: string | undefined;
	try {
		const { reasoning, thinkingBudgets, ...anthropicOptions } = c.options as Record<string, unknown> & {
			reasoning?: SimpleStreamOptions["reasoning"];
			thinkingBudgets?: SimpleStreamOptions["thinkingBudgets"];
		};
		const stream = c.simple
			? streamSimpleAnthropic(model, c.context, {
					...(anthropicOptions as SimpleStreamOptions),
					reasoning,
					thinkingBudgets,
					signal: controller.signal,
				})
			: streamAnthropic(model, c.context, {
					...(anthropicOptions as AnthropicOptions),
					signal: controller.signal,
				});
		for await (const event of stream) {
			events.push(summarize(event as unknown as Record<string, unknown>));
			if (c.abortAfterEvents !== undefined && events.length === c.abortAfterEvents) {
				controller.abort();
			}
		}
		result = normalizeMessage((await stream.result()) as unknown as Record<string, unknown>);
	} catch (error) {
		syncError = error instanceof Error ? error.message : String(error);
	} finally {
		for (const socket of sockets) socket.destroy();
		server.close();
	}
	return {
		name: c.name,
		requests,
		...(syncError !== undefined ? { syncError } : { events, result }),
	};
}

async function main() {
	const oracle: Array<Record<string, unknown>> = [];
	for (const c of spec.cases) {
		oracle.push(await runCase(c));
	}
	console.log(JSON.stringify({ cases: oracle }, null, "\t"));
}

main().catch((error) => {
	console.error(error);
	process.exitCode = 1;
});
"#;

/// azure-openai-responses-parity driver (port of tests/azure-openai-responses-parity/gen-oracle.ts).
pub const AZURE_OPENAI_RESPONSES_DRIVER: &str = r#"// Pi-derived Azure OpenAI Responses oracle. Run via scripts/azure-openai-responses-oracle.
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import {
  streamAzureOpenAIResponses, streamSimpleAzureOpenAIResponses, type AzureOpenAIResponsesOptions,
} from "../../ref/pi/packages/ai/src/providers/azure-openai-responses.ts";
import type { Context, Model, SimpleStreamOptions } from "../../ref/pi/packages/ai/src/types.ts";

type Response = { status: number; sse?: string; events?: unknown[]; json?: unknown; text?: string };
type Case = { name: string; model: string; simple?: boolean; noServerBase?: boolean; env?: Record<string,string>; context: Context; options: Record<string, unknown>; responses: Response[] };
type Spec = { models: Record<string, Model<"azure-openai-responses">>; sse: Record<string, unknown[]>; cases: Case[] };
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Spec;
const DROP = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode"]);
function headers(raw: Record<string, string | string[] | undefined>) {
  return Object.fromEntries(Object.entries(raw).filter(([k]) => !DROP.has(k) && !k.startsWith("x-stainless-") && k !== "user-agent").map(([k,v]) => [k, Array.isArray(v) ? v.join(", ") : (v ?? "")]).sort(([a],[b]) => a.localeCompare(b)));
}
function body(r: Response) {
  const events = r.sse ? spec.sse[r.sse]! : r.events;
  if (events) return { contentType: "text/event-stream", text: events.map(e => `data: ${JSON.stringify(e)}\n\n`).join("") };
  if (r.json !== undefined) return { contentType: "application/json", text: JSON.stringify(r.json) };
  return { contentType: "text/plain", text: r.text ?? "" };
}
function serve(c: Case): Promise<{ server: Server; url: string; requests: unknown[] }> {
  const requests: unknown[] = []; let index = 0;
  const server = createServer((req, res) => { const chunks: Buffer[] = [];
    req.on("data", c => chunks.push(c)); req.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8");
      requests.push({ method: req.method ?? "", path: req.url ?? "", headers: headers(req.headers), body: text ? JSON.parse(text) : null });
      const scripted = c.responses[index++] ?? c.responses.at(-1); if (!scripted) { res.destroy(); return; }
      const value = body(scripted); res.writeHead(scripted.status, { "content-type": value.contentType }); res.end(value.text);
    });
  });
  return new Promise(resolve => server.listen(0, "127.0.0.1", () => resolve({ server, url: `http://127.0.0.1:${(server.address() as {port:number}).port}`, requests })));
}
function summarize(event: Record<string, unknown>) { const { partial: _p, message: _m, error: _e, ...rest } = event; return rest; }
async function run(c: Case) {
  const envKeys = ["AZURE_OPENAI_API_VERSION", "AZURE_OPENAI_BASE_URL", "AZURE_OPENAI_RESOURCE_NAME", "AZURE_OPENAI_DEPLOYMENT_NAME_MAP"];
  const oldEnv = new Map<string, string | undefined>();
  for (const key of envKeys) { oldEnv.set(key, process.env[key]); delete process.env[key]; }
  for (const [key, value] of Object.entries(c.env ?? {})) process.env[key] = value;
  const { server, url, requests } = await serve(c); const model = { ...spec.models[c.model]! };
  if (c.simple && !c.noServerBase) model.baseUrl = url;
  const options = { ...c.options, ...(!c.simple && !c.noServerBase ? { azureBaseUrl: url } : {}) };
  const events: unknown[] = []; let result: unknown; let syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleAzureOpenAIResponses(model, c.context, options as SimpleStreamOptions) : streamAzureOpenAIResponses(model, c.context, options as AzureOpenAIResponsesOptions);
    for await (const event of stream) events.push(summarize(event as unknown as Record<string, unknown>));
    result = { ...(await stream.result()) as unknown as Record<string, unknown>, timestamp: 0 };
  } catch (e) { syncError = e instanceof Error ? e.message : String(e); }
  server.close(); for (const [key, value] of oldEnv) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
  return { name: c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}
async function main() { const cases = []; for (const c of spec.cases) cases.push(await run(c)); console.log(JSON.stringify({ cases }, null, "\t")); }
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// bedrock-converse-stream-parity driver (port of tests/bedrock-converse-stream-parity/gen-oracle.ts).
pub const BEDROCK_CONVERSE_STREAM_DRIVER: &str = r#"import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { join } from "node:path";
import { streamBedrock, streamSimpleBedrock } from "../../ref/pi/packages/ai/src/providers/amazon-bedrock.ts";

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8"));
const require = createRequire(import.meta.url);
const { EventStreamCodec } = require(join(process.cwd(), "ref/pi/node_modules/@smithy/core/event-streams"));
const { fromUtf8, toUtf8 } = require(join(process.cwd(), "ref/pi/node_modules/@smithy/util-utf8"));
const codec = new EventStreamCodec(toUtf8, fromUtf8);
const drop = new Set(["host","content-length","connection","accept","accept-encoding","user-agent","amz-sdk-invocation-id","amz-sdk-request","x-amz-user-agent"]);

function frame(event: any): Uint8Array {
  return codec.encode({
    headers: {
      ":event-type": { type:"string", value:event.type },
      ":message-type": { type:"string", value:"event" },
      ":content-type": { type:"string", value:"application/json" },
    },
    body: fromUtf8(JSON.stringify(event.value)),
  });
}

async function run(c: any) {
  const requests: any[] = []; let index = 0;
  const server = createServer((req, res) => { const chunks: Buffer[] = [];
    req.on("data", chunk => chunks.push(chunk)); req.on("end", () => {
      const text = Buffer.concat(chunks).toString();
      const headers = Object.fromEntries(Object.entries(req.headers).filter(([name]) => !drop.has(name)).map(([name,value]) => [name, Array.isArray(value) ? value.join(", ") : value ?? ""]).sort(([a],[b]) => a.localeCompare(b)));
      requests.push({ method:req.method, path:req.url, headers, body:text ? JSON.parse(text) : null });
      const scripted = c.responses[index++] ?? c.responses.at(-1); if (!scripted) { res.destroy(); return; }
      if (scripted.events) {
        const body = Buffer.concat(scripted.events.map((event: any) => Buffer.from(frame(event))));
        res.writeHead(scripted.status, { "content-type":"application/vnd.amazon.eventstream", "x-amzn-requestid":"fixture-request" }); res.end(body);
      } else {
        const body = scripted.json !== undefined ? JSON.stringify(scripted.json) : scripted.text ?? "";
        res.writeHead(scripted.status, { "content-type":scripted.json !== undefined ? "application/json" : "text/plain" }); res.end(body);
      }
    });
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const model = { ...spec.models[c.model], baseUrl:`http://127.0.0.1:${(server.address() as any).port}` };
  const oldForce = process.env.AWS_BEDROCK_FORCE_HTTP1; process.env.AWS_BEDROCK_FORCE_HTTP1 = "1";
  const oldBearer = process.env.AWS_BEARER_TOKEN_BEDROCK; delete process.env.AWS_BEARER_TOKEN_BEDROCK;
  if (c.simple && c.options.bearerToken) process.env.AWS_BEARER_TOKEN_BEDROCK = c.options.bearerToken;
  const events: any[] = []; let result: any, syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleBedrock(model, c.context, c.options) : streamBedrock(model, c.context, c.options);
    for await (const event of stream) { const { partial, message, error, ...summary } = event as any; events.push(summary); }
    result = { ...await stream.result(), timestamp:0 };
  } catch (error) { syncError = error instanceof Error ? error.message : String(error); }
  finally {
    server.close();
    if (oldForce === undefined) delete process.env.AWS_BEDROCK_FORCE_HTTP1; else process.env.AWS_BEDROCK_FORCE_HTTP1 = oldForce;
    if (oldBearer === undefined) delete process.env.AWS_BEARER_TOKEN_BEDROCK; else process.env.AWS_BEARER_TOKEN_BEDROCK = oldBearer;
  }
  return { name:c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}

async function main() { const cases = []; for (const c of spec.cases) cases.push(await run(c)); console.log(JSON.stringify({ cases }, null, "\t")); }
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// compaction-parity driver (port of tests/compaction-parity/gen-oracle.ts).
pub const COMPACTION_DRIVER: &str = r#"// Regenerates tests/compaction-parity/oracle.json by driving Pi's real
// compaction pipeline (ref/pi @ c5582102 core/compaction/compaction.ts and
// pi-ai utils/overflow.ts) over the scripted session entries in cases.json.
// Modes:
//   prepare  — prepareCompaction(entries, settings): the cut point,
//              messages to summarize, split-turn prefix, previous-summary
//              merge, file ops, and tokensBefore.
//   compact  — prepareCompaction + compact() with an injected scripted
//              streamFn recording every summarization request (system
//              prompt, prompt text, maxTokens, reasoning) and the final
//              summary/details, or the thrown error.
//   tokens   — estimateContextTokens / calculateContextTokens.
//   should   — shouldCompact.
//   overflow — isContextOverflow (pi-ai overflow patterns).
// Undefined/null fields are omitted so the Lua replay (which drops nils)
// compares as parsed JSON. Date.now is pinned; the Lua replay passes the
// same now_ms. Run via scripts/compaction-oracle. Do not edit the oracle
// by hand.
import { readFileSync } from "node:fs";
import type { AssistantMessage } from "../../ref/pi/packages/ai/src/types.ts";
import { isContextOverflow } from "../../ref/pi/packages/ai/src/utils/overflow.ts";
import {
	calculateContextTokens,
	compact,
	estimateContextTokens,
	prepareCompaction,
	shouldCompact,
} from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import type { SessionEntry } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";

type Json = any;

const NOW_MS = 1750000000000;
const RealDate = Date;
class FixedDate extends RealDate {
	constructor(...args: ConstructorParameters<typeof RealDate>) {
		if (args.length === 0) {
			super(NOW_MS);
		} else {
			super(...args);
		}
	}
	static now(): number {
		return NOW_MS;
	}
}
(globalThis as { Date: unknown }).Date = FixedDate;

const casesPath = process.argv[2] ?? "tests/compaction-parity/cases.json";
const spec = JSON.parse(readFileSync(casesPath, "utf8")) as Json;

/** Drop undefined/null recursively so both sides compare field-wise. */
function stripNull(value: Json): Json {
	if (Array.isArray(value)) return value.map(stripNull);
	if (value && typeof value === "object") {
		const out: Json = {};
		for (const [key, item] of Object.entries(value)) {
			if (item === undefined || item === null) continue;
			out[key] = stripNull(item);
		}
		return out;
	}
	return value;
}

function fileOpsToJson(fileOps: { read: Set<string>; written: Set<string>; edited: Set<string> }): Json {
	return {
		read: [...fileOps.read].sort(),
		written: [...fileOps.written].sort(),
		edited: [...fileOps.edited].sort(),
	};
}

function preparationToJson(preparation: Json): Json {
	return {
		firstKeptEntryId: preparation.firstKeptEntryId,
		isSplitTurn: preparation.isSplitTurn,
		tokensBefore: preparation.tokensBefore,
		previousSummary: preparation.previousSummary,
		messagesToSummarize: preparation.messagesToSummarize,
		turnPrefixMessages: preparation.turnPrefixMessages,
		fileOps: fileOpsToJson(preparation.fileOps),
	};
}

function scriptedStreamFn(caseSpec: Json, requests: Json[]): Json {
	return async (model: Json, context: Json, options: Json) => {
		const index = requests.length;
		requests.push({
			systemPrompt: context.systemPrompt,
			messages: context.messages,
			maxTokens: options.maxTokens,
			reasoning: options.reasoning,
			apiKey: options.apiKey,
		});
		const scripted = (caseSpec.responses ?? [])[index] ?? { text: "" };
		const message: AssistantMessage = {
			role: "assistant",
			content: scripted.errorMessage ? [] : [{ type: "text", text: scripted.text ?? "" }],
			api: model.api,
			provider: model.provider,
			model: model.id,
			usage: {
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 0,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			stopReason: scripted.errorMessage ? "error" : "stop",
			errorMessage: scripted.errorMessage,
			timestamp: 0,
		};
		return { result: async () => message } as Json;
	};
}

async function runCase(caseSpec: Json): Promise<Json> {
	const mode = caseSpec.mode ?? "prepare";
	const model = spec.models[caseSpec.model ?? "default"];
	const settings = {
		enabled: true,
		reserveTokens: 16384,
		keepRecentTokens: 20000,
		...(caseSpec.settings ?? {}),
	};

	if (mode === "tokens") {
		const out: Json = {};
		if (caseSpec.messages) out.estimate = estimateContextTokens(caseSpec.messages);
		if (caseSpec.usage) out.contextTokens = calculateContextTokens(caseSpec.usage);
		return out;
	}
	if (mode === "should") {
		return {
			shouldCompact: shouldCompact(caseSpec.contextTokens, caseSpec.contextWindow, settings),
		};
	}
	if (mode === "overflow") {
		return {
			overflow: isContextOverflow(caseSpec.message, caseSpec.contextWindow),
		};
	}

	const entries = caseSpec.entries as SessionEntry[];
	const preparation = prepareCompaction(entries, settings);
	if (!preparation) {
		return { prepared: false };
	}
	const result: Json = { prepared: true, preparation: preparationToJson(preparation) };
	if (mode === "compact") {
		const requests: Json[] = [];
		try {
			const compactResult = await compact(
				preparation,
				model,
				caseSpec.apiKey ?? "oracle-key",
				undefined,
				caseSpec.customInstructions,
				undefined,
				caseSpec.thinkingLevel,
				scriptedStreamFn(caseSpec, requests),
			);
			result.result = compactResult;
		} catch (error) {
			result.error = error instanceof Error ? error.message : String(error);
		}
		result.requests = requests;
	}
	return result;
}

async function main() {
	const cases: Json[] = [];
	for (const caseSpec of spec.cases) {
		cases.push({ name: caseSpec.name, ...stripNull(await runCase(caseSpec)) });
	}
	process.stdout.write(`${JSON.stringify({ cases }, null, "\t")}\n`);
}

main().catch((error) => {
	console.error(error);
	process.exitCode = 1;
});
"#;

/// export-html-parity driver (port of tests/export-html-parity/gen-oracle.ts).
pub const EXPORT_HTML_DRIVER: &str = r#"import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AgentState } from "../../ref/pi/packages/agent/src/types.ts";
import { exportSessionToHtml } from "../../ref/pi/packages/coding-agent/src/core/export-html/index.ts";
import { SessionManager } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";
import { initTheme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";

async function main() {
  const casePath = process.argv[2];
  if (!casePath) throw new Error("usage: gen-oracle.ts <case.json>");
  const fixture = JSON.parse(readFileSync(casePath, "utf8")) as {
    session: string;
    systemPrompt: string;
    tools: Array<{ name: string; description: string; parameters: unknown }>;
  };
  const dir = mkdtempSync(join(tmpdir(), "pi-export-html-parity-"));
  try {
    const sessionPath = join(dir, "session.jsonl");
    const outputPath = join(dir, "session.html");
    writeFileSync(sessionPath, fixture.session);
    initTheme("dark", false);
    const manager = SessionManager.open(sessionPath);
    const state = { systemPrompt: fixture.systemPrompt, tools: fixture.tools } as unknown as AgentState;
    await exportSessionToHtml(manager, state, { outputPath, themeName: "dark" });
    const html = readFileSync(outputPath, "utf8");
    if (process.env.EXPORT_HTML_DEBUG) writeFileSync(process.env.EXPORT_HTML_DEBUG, html);
    const encoded = html.split('<script id="session-data" type="application/json">')[1]?.split("</script>")[0];
    if (!encoded) throw new Error("session payload marker missing");
    const payload = Buffer.from(encoded, "base64").toString("utf8");
    const sha256 = (value: string) => createHash("sha256").update(value).digest("hex");
    process.stdout.write(`${JSON.stringify({ payload, htmlSha256: sha256(html) }, null, 2)}\n`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
"#;

/// extension-context-parity driver (port of tests/extension-context-parity/gen-oracle.ts).
pub const EXTENSION_CONTEXT_DRIVER: &str = r#"// PLAN 9.2: derive contexts, restrictions, and lifecycle action order from Pi's real runner.
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

const staleMessage = "This extension ctx is stale after session replacement or reload. Do not use a captured pi or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().";

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function modeMatrix(runner: any): Promise<Array<{ mode: string; hasUI: boolean }>> {
  const modes: Array<{ mode: string; hasUI: boolean }> = [];
  for (const mode of ["print", "json"] as const) {
    runner.setUIContext(undefined, mode);
    const ctx = runner.createContext();
    modes.push({ mode: ctx.mode, hasUI: ctx.hasUI });
  }
  runner.setUIContext({ notify() {} } as any, "rpc");
  const rpc = runner.createContext();
  modes.push({ mode: rpc.mode, hasUI: rpc.hasUI });
  runner.setUIContext({ notify() {} } as any, "tui");
  return modes;
}

async function actionMatrix(runner: any): Promise<any> {
  const trace: string[] = [];
  runner.bindCommandContext({
    waitForIdle: async () => { trace.push("wait"); },
    newSession: async (options: any) => {
      trace.push(`new:${options?.parentSession ?? ""}`);
      return { cancelled: options?.parentSession === "cancel" };
    },
    fork: async (entryId: string, options: any) => {
      trace.push(`fork:${entryId}:${options?.position ?? "before"}`);
      return { cancelled: entryId === "cancel" };
    },
    navigateTree: async (targetId: string, options: any) => {
      trace.push(`tree:${targetId}:${options?.summarize === true}:${options?.label ?? ""}`);
      return { cancelled: targetId === "cancel" };
    },
    switchSession: async (path: string) => {
      trace.push(`switch:${path}`);
      return { cancelled: path === "cancel" };
    },
    reload: async () => { trace.push("reload"); },
  });
  const base = runner.createContext();
  const ctx = runner.createCommandContext();
  await ctx.waitForIdle();
  const outcomes = {
    newSession: await ctx.newSession({ parentSession: "parent.jsonl" }),
    newCancelled: await ctx.newSession({ parentSession: "cancel" }),
    fork: await ctx.fork("entry-1", { position: "at" }),
    forkCancelled: await ctx.fork("cancel"),
    tree: await ctx.navigateTree("entry-2", { summarize: true, label: "kept" }),
    treeCancelled: await ctx.navigateTree("cancel"),
    switchSession: await ctx.switchSession("other.jsonl"),
    switchCancelled: await ctx.switchSession("cancel"),
  };
  await ctx.reload();
  return {
    restrictions: {
      baseNewSession: typeof (base as any).newSession,
      baseFork: typeof (base as any).fork,
      baseTree: typeof (base as any).navigateTree,
      baseSwitch: typeof (base as any).switchSession,
      baseReload: typeof (base as any).reload,
      commandNewSession: typeof ctx.newSession,
      commandFork: typeof ctx.fork,
      commandTree: typeof ctx.navigateTree,
      commandSwitch: typeof ctx.switchSession,
      commandReload: typeof ctx.reload,
    },
    trace,
    outcomes,
  };
}

async function replacementOrder(): Promise<any> {
  const oldHarness = await createHarness();
  const freshHarness = await createHarness();
  try {
    const noUi = { select: async () => undefined, confirm: async () => false, input: async () => undefined, notify: () => {} } as any;
    await oldHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
    await freshHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
    const oldRunner = oldHarness.session.extensionRunner;
    const freshRunner = freshHarness.session.extensionRunner;
    const trace: string[] = [];
    oldRunner.bindCommandContext({
      waitForIdle: async () => {},
      newSession: async (options: any) => {
        trace.push("shutdown");
        oldRunner.invalidate();
        trace.push("rebind");
        if (options?.withSession) await options.withSession(freshRunner.createCommandContext());
        trace.push("action-return");
        return { cancelled: false };
      },
      fork: async () => ({ cancelled: false }),
      navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => {},
    });
    const old = oldRunner.createCommandContext();
    const result = await old.newSession({
      withSession: async (fresh: any) => {
        trace.push("withSession");
        trace.push(`fresh:${fresh.mode}:${fresh.isIdle()}`);
        try { old.isIdle(); } catch (error) { trace.push(`old-stale:${errorText(error) === staleMessage}`); }
      },
    });
    let stale = "";
    try { old.isIdle(); } catch (error) { stale = errorText(error); }
    return { trace, result, stale };
  } finally {
    oldHarness.cleanup();
    freshHarness.cleanup();
  }
}

async function reloadOrder(): Promise<any> {
  const harness = await createHarness();
  try {
    await harness.session.bindExtensions({ mode: "print" });
    const runner = harness.session.extensionRunner;
    const trace: string[] = [];
    runner.bindCommandContext({
      waitForIdle: async () => {}, newSession: async () => ({ cancelled: false }),
      fork: async () => ({ cancelled: false }), navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => { trace.push("shutdown"); runner.invalidate(); trace.push("reloaded"); },
    });
    const ctx = runner.createCommandContext();
    await ctx.reload();
    let stale = "";
    try { ctx.getSystemPrompt(); } catch (error) { stale = errorText(error); }
    return { trace, stale };
  } finally {
    harness.cleanup();
  }
}

async function main(): Promise<void> {
  const harness = await createHarness({ systemPrompt: "context oracle prompt" });
  try {
    let shutdowns = 0;
    const noUi = { select: async () => undefined, confirm: async () => false, input: async () => undefined, notify: () => {} } as any;
    await harness.session.bindExtensions({ mode: "tui", uiContext: noUi, shutdownHandler: () => { shutdowns++; } });
    const startupModel = harness.getModel();
    harness.sessionManager.appendModelChange(startupModel.provider, startupModel.id);
    harness.sessionManager.appendThinkingLevelChange("off");
    const runner = harness.session.extensionRunner;
    const modes = await modeMatrix(runner);
    const ctx = runner.createCommandContext();
    const model = ctx.model!;
    const found = ctx.modelRegistry.find(model.provider, model.id);
    ctx.shutdown();
    const snapshot = {
      mode: ctx.mode, hasUI: ctx.hasUI, cwd: "{CWD}", trusted: ctx.isProjectTrusted(),
      idle: ctx.isIdle(), pending: ctx.hasPendingMessages(), hasSignal: ctx.signal !== undefined,
      model: { provider: model.provider, id: model.id },
      session: { persisted: ctx.sessionManager.isPersisted(), cwd: "{CWD}", entries: ctx.sessionManager.getEntries().length, branch: ctx.sessionManager.getBranch().length },
      registryFound: found ? { provider: found.provider, id: found.id } : null,
      systemPromptHasCwd: ctx.getSystemPrompt().includes(`Current working directory: ${ctx.cwd}`),
      systemPromptOptionsCwd: ctx.getSystemPromptOptions().cwd === ctx.cwd,
      usage: ctx.getContextUsage() ?? null, waitForIdle: typeof ctx.waitForIdle === "function", shutdowns,
    };
    runner.invalidate();
    let stale = "";
    try { ctx.isIdle(); } catch (error) { stale = errorText(error); }
    const actionsHarness = await createHarness();
    let actions: any;
    try {
      await actionsHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
      actions = await actionMatrix(actionsHarness.session.extensionRunner);
    } finally { actionsHarness.cleanup(); }
    process.stdout.write(`${JSON.stringify({ snapshot, stale, modes, actions, replacement: await replacementOrder(), reload: await reloadOrder() }, null, "\t")}\n`);
  } finally { harness.cleanup(); }
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
"#;

/// extension-event-parity driver (port of tests/extension-event-parity/gen-oracle.ts).
pub const EXTENSION_EVENT_DRIVER: &str = r#"// PLAN 9.3: Pi-generated oracle for the complete ExtensionRunner fold contract.
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { DefaultResourceLoader } from "../../ref/pi/packages/coding-agent/src/core/resource-loader.ts";
import { emitProjectTrustEvent } from "../../ref/pi/packages/coding-agent/src/core/extensions/runner.ts";
import { fauxAssistantMessage } from "../../ref/pi/packages/ai/src/providers/faux.ts";
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

const root = mkdtempSync(join(tmpdir(), "pi-rs-extension-events-"));
const first = join(root, "01-first.ts");
const second = join(root, "02-second.ts");
writeFileSync(first, `
export default function (pi: any) {
  (globalThis as any).__eventTrace = [];
  const log = (event: any) => (globalThis as any).__eventTrace.push("first:" + event.type);
  for (const type of ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"]) pi.on(type as any, async (event: any) => { log(event); if (type === "agent_end") throw new Error("first agent error"); });
  pi.on("resources_discover", async (event: any) => { log(event); return {skillPaths:["skills-a"]}; });
  pi.on("session_before_switch", async (event: any) => { log(event); return {cancel:false,owner:"first"}; });
  pi.on("session_before_fork", async (event: any) => { log(event); return {cancel:false}; });
  pi.on("session_before_compact", async (event: any) => { log(event); return {compaction:{summary:"hook summary",firstKeptEntryId:"keep",tokensBefore:9,details:{owner:"first"}}}; });
  pi.on("session_before_tree", async (event: any) => { log(event); return {customInstructions:"first instructions",label:"first-label"}; });
  pi.on("context", async (event: any) => { log(event); return {messages:[...event.messages,{role:"user",content:"first",timestamp:0}]}; });
  pi.on("before_provider_request", async (event: any) => { log(event); return {...event.payload,first:true}; });
  pi.on("before_agent_start", async (event: any) => { log(event); return {message:{customType:"first",content:"notice",display:true},systemPrompt:event.systemPrompt+"|first"}; });
  pi.on("message_end", async (event: any) => { log(event); if (event.message.role !== "assistant" || event.message.content?.some?.((part:any) => part.type === "toolCall")) return; return {message:{...event.message,content:[{type:"text",text:"first replacement"}]}}; });
  pi.registerTool({name:"event_tool",label:"Event Tool",description:"event seam",parameters:{type:"object",properties:{value:{type:"string"}},required:["value"]},async execute(_id:string,input:any){return {content:[{type:"text",text:"tool:"+input.value}],details:{base:true}};}});\n  pi.on("tool_call", async (event: any) => { log(event); if (event.input.command) event.input.command += " --first"; else event.input.first=true; return {owner:"first"}; });
  pi.on("tool_result", async (event: any) => { log(event); return {content:[{type:"text",text:"first result"}],details:{first:true}}; });
  pi.on("user_bash", async (event: any) => { log(event); return undefined; });
  pi.on("input", async (event: any) => { log(event); return {action:"transform",text:event.text+"|first"}; });
  pi.on("project_trust", async (event: any) => { log(event); return {trusted:"undecided"}; });
  pi.registerCommand("event-trace", {handler: async () => (globalThis as any).__eventTrace});
}`);
writeFileSync(second, `
export default function (pi: any) {
  const log = (event: any) => (globalThis as any).__eventTrace.push("second:" + event.type);
  for (const type of ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"]) pi.on(type as any, async (event: any) => log(event));
  pi.on("resources_discover", async (event: any) => { log(event); return {promptPaths:["prompts-b"],themePaths:["themes-b"]}; });
  pi.on("session_before_switch", async (event: any) => { log(event); return {cancel:true,owner:"second"}; });
  pi.on("session_before_switch", async () => { (globalThis as any).__eventTrace.push("second:after-cancel"); });
  pi.on("session_before_fork", async (event: any) => { log(event); return {cancel:true}; });
  pi.on("session_before_compact", async (event: any) => { log(event); return {cancel:true}; });
  pi.on("session_before_tree", async (event: any) => { log(event); return {summary:{summary:"tree summary",details:{second:true}},replaceInstructions:true,label:"second-label"}; });
  pi.on("context", async (event: any) => { log(event); return {messages:[...event.messages,{role:"user",content:"second",timestamp:0}]}; });
  pi.on("before_provider_request", async (event: any) => { log(event); return {...event.payload,second:event.payload.first}; });
  pi.on("before_agent_start", async (event: any) => { log(event); return {message:{customType:"second",content:event.systemPrompt,display:false},systemPrompt:event.systemPrompt+"|second"}; });
  pi.on("message_end", async (event: any) => { log(event); return {message:{role:"user",content:"invalid",timestamp:0}}; });
  pi.on("tool_call", async (event: any) => { log(event); return event.toolName === "bash" ? {block:true,reason:event.input.command} : undefined; });
  pi.on("tool_result", async (event: any) => { log(event); return {isError:true,details:{second:event.details.first}}; });
  pi.on("user_bash", async (event: any) => { log(event); return {result:{output:"handled bash",exitCode:7,cancelled:false,truncated:false}}; });
  pi.on("input", async (event: any) => { log(event); return event.text.includes("handle") ? {action:"handled"} : {action:"transform",text:event.text+"|second",images:event.images}; });
  pi.on("project_trust", async (event: any) => { log(event); return {trusted:"yes",remember:true}; });
}`);

const stable = (path: string) => basename(path).replace(/\.[^.]+$/, "");
const clean = (value: any): any => {
  if (Array.isArray(value)) return value.map(clean);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k,v]) => [k, clean(v)]));
  if (typeof value === "string") return value.replaceAll(first, stable(first)).replaceAll(second, stable(second));
  return value;
};

async function main() {
  const loader = new DefaultResourceLoader({cwd:root, agentDir:root, additionalExtensionPaths:[first,second], noSkills:true, noPromptTemplates:true, noThemes:true, noContextFiles:true});
  await loader.reload();
  const loaded = loader.getExtensions();
  const harness = await createHarness({resourceLoader:loader});
  try {
    const runner = harness.session.extensionRunner;
    const errors:any[] = [];
    runner.onError((error:any) => errors.push({extensionPath:stable(error.extensionPath),event:error.event,error:error.error}));
    const genericTypes = ["session_start","session_compact","session_shutdown","session_tree","after_provider_response","agent_start","agent_end","turn_start","turn_end","message_start","message_update","tool_execution_start","tool_execution_update","tool_execution_end","model_select","thinking_level_select"];
    for (const type of genericTypes) await runner.emit({type,status:201,headers:{x:"y"},messages:[],turnIndex:2,timestamp:123,message:{role:"assistant",content:[],timestamp:0},toolResults:[],toolCallId:"call",toolName:"bash",args:{command:"x"},partialResult:{content:[]},result:{content:[]},isError:false,model:harness.model,previousModel:undefined,source:"set",level:"low",previousLevel:"off",newLeafId:"leaf",oldLeafId:"old",fromExtension:false,compactionEntry:{id:"compact"}} as any);
    const beforeSwitch = await runner.emit({type:"session_before_switch",reason:"resume",targetSessionFile:"target.jsonl"});
    const beforeFork = await runner.emit({type:"session_before_fork",entryId:"entry",position:"before"});
    const beforeCompact = await runner.emit({type:"session_before_compact",preparation:{firstKeptEntryId:"keep"},branchEntries:[],signal:new AbortController().signal} as any);
    const beforeTree = await runner.emit({type:"session_before_tree",preparation:{targetId:"target"},signal:new AbortController().signal} as any);
    const context = await runner.emitContext([{role:"user",content:"base",timestamp:0}] as any);
    const payload = await runner.emitBeforeProviderRequest({base:true});
    const beforeAgent = await runner.emitBeforeAgentStart("prompt",undefined,"system",{cwd:root});
    const message = await runner.emitMessageEnd({type:"message_end",message:{role:"assistant",content:[{type:"text",text:"base"}],api:"x",provider:"x",model:"x",usage:{},stopReason:"stop",timestamp:0}} as any);
    const toolInput:any = {command:"echo"};
    const toolCall = await runner.emitToolCall({type:"tool_call",toolCallId:"call",toolName:"bash",input:toolInput});
    const toolResult = await runner.emitToolResult({type:"tool_result",toolCallId:"call",toolName:"bash",input:toolInput,content:[{type:"text",text:"base result"}],details:{base:true},isError:false} as any);
    const userBash = await runner.emitUserBash({type:"user_bash",command:"echo hi",excludeFromContext:false,cwd:root});
    const input = await runner.emitInput("go",undefined,"interactive");
    const handledInput = await runner.emitInput("handle",undefined,"interactive","steer");
    const trust = await emitProjectTrustEvent(loaded,{type:"project_trust",cwd:root},{cwd:root,mode:"tui",hasUI:false,ui:runner.getUIContext()});
    const resources = await runner.emitResourcesDiscover(root,"startup");
    const trace = [...await runner.getCommand("event-trace")!.handler("",runner.createCommandContext())];
    const traceBeforeProduct = trace.length;
    const foldErrorCount = errors.length;
    harness.setResponses([
      fauxAssistantMessage({type:"toolCall",id:"event-call",name:"event_tool",arguments:{value:"x"}}, {timestamp:0}),
      fauxAssistantMessage("done", {timestamp:0}),
    ]);
    await harness.session.prompt("go");
    const fullTrace = await runner.getCommand("event-trace")!.handler("",runner.createCommandContext());
    const significant = new Set(["session_start","resources_discover","input","before_agent_start","agent_start","turn_start","message_start","message_end","after_provider_response","tool_execution_start","tool_call","tool_result","tool_execution_end","turn_end","agent_end"]);
    const productTrace = fullTrace.slice(traceBeforeProduct).filter((entry:string) => significant.has(entry.slice(entry.indexOf(":")+1)));
    process.stdout.write(JSON.stringify(clean({beforeSwitch,beforeFork,beforeCompact,beforeTree,context,payload,beforeAgent,message,toolInput,toolCall,toolResult,userBash,input,handledInput,trust:trust.result,resources,errors,foldErrorCount,trace,productTrace}),null,"\t")+"\n");
  } finally { harness.cleanup(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
"#;

/// extension-runtime-parity driver (port of tests/extension-runtime-parity/gen-oracle.ts).
pub const EXTENSION_RUNTIME_DRIVER: &str = r#"// PLAN 9.1: generate the loader/runtime oracle from Pi's real resource loader,
// ExtensionRunner, AgentSession, and faux provider request path.
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, sep } from "node:path";

import { fauxAssistantMessage } from "../../ref/pi/packages/ai/src/providers/faux.ts";
import { DefaultResourceLoader } from "../../ref/pi/packages/coding-agent/src/core/resource-loader.ts";
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

type Json = any;

const root = mkdtempSync(join(tmpdir(), "pi-rs-extension-runtime-oracle-"));
const cwd = join(root, "project");
const agentDir = join(root, "agent");
mkdirSync(cwd, { recursive: true });
mkdirSync(agentDir, { recursive: true });

const sources: Array<[string, string]> = [
  ["01-first.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace = ["first:start"];
      await Promise.resolve();
      (globalThis as any).__extensionTrace.push("first:end");
      pi.registerTool({ name: "shared", label: "Shared First", description: "first wins", parameters: {type:"object",properties:{},required:[]}, async execute() { return {content:[{type:"text",text:"first"}],details:{owner:"first"}}; } });
      pi.registerTool({ name: "hello", label: "Hello", description: "A simple greeting tool", parameters: {type:"object",properties:{name:{type:"string",description:"Name to greet"}},required:["name"]}, async execute(_id: string, params: any) { return {content:[{type:"text",text:\`Hello, \${params.name}!\`}],details:{greeted:params.name}}; } });
      pi.registerCommand("dup", { description: "first dup", handler: async () => "first-command" });
      pi.registerCommand("trace", { description: "trace", handler: async () => (globalThis as any).__extensionTrace });
      pi.registerFlag("plan", { description: "Plan mode", type: "boolean", default: false });
      pi.registerFlag("profile", { description: "Profile name", type: "string", default: "safe" });
      pi.registerCommand("flag-values", { handler: async () => ({plan:pi.getFlag("plan"),profile:pi.getFlag("profile"),missing:pi.getFlag("missing")}) });
      pi.registerCommand("catalog", { description: "catalog", getArgumentCompletions: (prefix: string) => ["extension", "prompt", "skill"].filter((source) => source.startsWith(prefix)).map((source) => ({value:source,label:source})), handler: async () => pi.getCommands() });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:first"); return {tag:"first"}; });
    }
  `],
  ["02-bad.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace.push("bad:start");
      pi.registerTool({ name: "ghost", label: "Ghost", description: "must roll back", parameters: {}, async execute() { return {}; } });
      pi.registerCommand("ghost", { handler: async () => "ghost" });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:ghost"); });
      await Promise.resolve();
      throw new Error("broken init");
    }
  `],
  ["03-second.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace.push("second:start");
      await new Promise((resolve) => setTimeout(resolve, 1));
      (globalThis as any).__extensionTrace.push("second:end");
      pi.registerTool({ name: "shared", label: "Shared Second", description: "loses", parameters: {type:"object",properties:{},required:[]}, async execute() { return {content:[{type:"text",text:"second"}],details:{owner:"second"}}; } });
      pi.registerCommand("dup", { description: "second dup", handler: async () => "second-command" });
      pi.registerFlag("plan", { description: "Conflicting plan", type: "boolean", default: true });
      pi.registerFlag("second-only", { type: "string" });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:second"); return {tag:"second"}; });
    }
  `],
  ["04-block.ts", `
    export default function (pi: any) {
      (globalThis as any).__extensionTrace.push("block:loaded");
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:block"); return {block:true,reason:"blocked"}; });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:after-block"); return {tag:"after"}; });
    }
  `],
];

const paths: string[] = [];
for (const [name, source] of sources) {
  const path = join(root, name);
  writeFileSync(path, source);
  paths.push(path);
}

function stablePath(path: string): string {
  return basename(path).replace(/\.[^.]+$/, "");
}

async function main(): Promise<void> {
  const loader = new DefaultResourceLoader({
    cwd,
    agentDir,
    additionalExtensionPaths: paths,
    noSkills: true,
    noPromptTemplates: true,
    noThemes: true,
    noContextFiles: true,
  });
  await loader.reload();
  const loaded = loader.getExtensions();
  const harness = await createHarness({ resourceLoader: loader });
  try {
    const runner = harness.session.extensionRunner;
    const capturedRequests: Json[] = [];
    harness.setResponses([
      (context: Json) => {
        capturedRequests.push({
          toolNames: (context.tools ?? []).map((tool: Json) => tool.name),
          extensionTools: (context.tools ?? [])
            .filter((tool: Json) => tool.name === "hello" || tool.name === "shared")
            .map((tool: Json) => ({name: tool.name, description: tool.description, parameters: tool.parameters})),
        });
        return fauxAssistantMessage("done", { timestamp: 0 });
      },
    ]);
    await harness.session.prompt("hello");

    const hello = runner.getToolDefinition("hello")!;
    const helloResult = await hello.execute("call-1", {name:"Ada"}, new AbortController().signal, undefined, runner.createContext());
    const commandResults = [];
    for (const name of ["dup:1", "dup:2"]) {
      const command = runner.getCommand(name)!;
      commandResults.push({name, result: await command.handler("", runner.createCommandContext())});
    }
    const hookResult = await runner.emitToolCall({type:"tool_call", toolCallId:"call-2", toolName:"bash", input:{command:"sudo true"}});
    const trace = await runner.getCommand("trace")!.handler("", runner.createCommandContext());
    const flagValues = await runner.getCommand("flag-values")!.handler("", runner.createCommandContext());
    const catalogCommand = runner.getCommand("catalog")!;
    const commandCatalog = (await catalogCommand.handler("", runner.createCommandContext())).map((command: Json) => ({
      name: command.name,
      description: command.description ?? null,
      source: command.source,
      sourceInfo: {...command.sourceInfo, path: stablePath(command.sourceInfo.path), source: stablePath(command.sourceInfo.source)},
    }));
    const argumentCompletions = await catalogCommand.getArgumentCompletions!("pr");

    const output = {
      loaded: loaded.extensions.map((extension: Json) => stablePath(extension.path)),
      errors: loaded.errors.map((error: Json) => ({path: stablePath(error.path), error: error.error.replaceAll(`${root}${sep}`, "").replaceAll(".ts", "")})),
      tools: runner.getAllRegisteredTools().map((tool: Json) => ({name: tool.definition.name, source: stablePath(tool.sourceInfo.path)})),
      commands: runner.getRegisteredCommands().map((command: Json) => ({name: command.name, invocationName: command.invocationName, source: stablePath(command.sourceInfo.path), description: command.description ?? null})),
      flags: Array.from(runner.getFlags().values()).map((flag: Json) => ({name: flag.name, source: stablePath(flag.extensionPath), description: flag.description ?? null, type: flag.type, default: flag.default ?? null})),
      commandResults,
      commandCatalog,
      argumentCompletions,
      flagValues,
      helloResult,
      hookResult,
      trace,
      capturedRequests,
    };
    process.stdout.write(`${JSON.stringify(output, null, "\t")}\n`);
  } finally {
    harness.cleanup();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
"#;

/// google-generative-ai-parity driver (port of tests/google-generative-ai-parity/gen-oracle.ts).
pub const GOOGLE_GENERATIVE_AI_DRIVER: &str = r#"import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { streamGoogle, streamSimpleGoogle } from "../../ref/pi/packages/ai/src/providers/google.ts";
const spec=JSON.parse(readFileSync(process.argv[2]!,"utf8"));
const drop=new Set(["host","content-length","connection","accept-encoding","accept-language","sec-fetch-mode","user-agent"]);
async function run(c:any){const requests:any[]=[];let i=0;const server=createServer((req,res)=>{const bs:Buffer[]=[];req.on("data",x=>bs.push(x));req.on("end",()=>{const text=Buffer.concat(bs).toString();requests.push({method:req.method,path:req.url,headers:Object.fromEntries(Object.entries(req.headers).filter(([k])=>!drop.has(k)).map(([k,v])=>[k,Array.isArray(v)?v.join(", "):v??""]).sort(([a],[b])=>a.localeCompare(b))),body:text?JSON.parse(text):null});const r=c.responses[i++]??c.responses.at(-1);if(!r){res.destroy();return}const body=r.chunks?r.chunks.map((x:any)=>`data: ${JSON.stringify(x)}\n\n`).join(""):r.json!==undefined?JSON.stringify(r.json):r.text??"";res.writeHead(r.status,{"content-type":r.chunks?"text/event-stream":r.json!==undefined?"application/json":"text/plain"});res.end(body)})});await new Promise<void>(ok=>server.listen(0,"127.0.0.1",ok));const url=`http://127.0.0.1:${(server.address() as any).port}`;const model={...spec.models[c.model],baseUrl:url};const events:any[]=[];let result:any,syncError:string|undefined;try{const s=c.simple?streamSimpleGoogle(model,c.context,c.options):streamGoogle(model,c.context,c.options);for await(const e of s){const{partial,message,error,...rest}=e as any;events.push(rest)}result={...await s.result(),timestamp:0}}catch(e){syncError=e instanceof Error?e.message:String(e)}server.close();return{name:c.name,requests,...(syncError?{syncError}:{events,result})}}
async function main(){const cases=[];for(const c of spec.cases)cases.push(await run(c));console.log(JSON.stringify({cases},null,"\t"));}
main().catch(error=>{console.error(error);process.exitCode=1});
"#;

/// google-vertex-parity driver (port of tests/google-vertex-parity/gen-oracle.ts).
pub const GOOGLE_VERTEX_DRIVER: &str = r#"import { createHash, createHmac, createPublicKey, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { streamGoogleVertex, streamSimpleGoogleVertex } from "../../ref/pi/packages/ai/src/providers/google-vertex.ts";

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8"));
const fixtureDir = dirname(process.argv[2]!);
const drop = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode", "user-agent"]);
const { Gaxios } = createRequire(import.meta.url)(join(process.cwd(), "ref/pi/node_modules/gaxios"));

function awsHmac(key: Buffer | string, text: string): Buffer {
	return createHmac("sha256", key).update(text).digest();
}

function normalizeAwsSubject(encoded: string): any {
	const value = JSON.parse(decodeURIComponent(encoded));
	const headers = Object.fromEntries(value.headers.map(({ key, value }: any) => [key, value]));
	const authorization = headers.authorization as string;
	const match = authorization.match(/^AWS4-HMAC-SHA256 Credential=([^/]+)\/(\d{8})\/([^/]+)\/([^/]+)\/aws4_request, SignedHeaders=([^,]+), Signature=([0-9a-f]+)$/);
	if (!match) throw new Error(`invalid AWS authorization: ${authorization}`);
	const [, accessKey, date, region, service, signedHeaders, signature] = match;
	const secret = accessKey === "env-access" ? "env-secret" : "metadata-secret";
	const signed = signedHeaders!.split(";");
	const canonicalHeaders = signed.map(name => `${name}:${headers[name]}\n`).join("");
	const parsed = new URL(value.url);
	const emptyHash = createHash("sha256").update("").digest("hex");
	const canonical = `${value.method}\n${parsed.pathname}\n${parsed.search.slice(1)}\n${canonicalHeaders}\n${signedHeaders}\n${emptyHash}`;
	const stringToSign = `AWS4-HMAC-SHA256\n${headers["x-amz-date"]}\n${date}/${region}/${service}/aws4_request\n${createHash("sha256").update(canonical).digest("hex")}`;
	const kDate = awsHmac(`AWS4${secret}`, date);
	const kRegion = awsHmac(kDate, region);
	const kService = awsHmac(kRegion, service);
	const kSigning = awsHmac(kService, "aws4_request");
	const expected = createHmac("sha256", kSigning).update(stringToSign).digest("hex");
	value.url = `{AWS_ORIGIN}${parsed.pathname}${parsed.search}`;
	value.headers = value.headers.map((header: any) => {
		if (header.key === "host") return { ...header, value: "{AWS_HOST}" };
		if (header.key === "x-amz-date") return { ...header, value: "{AWS_DATE}" };
		if (header.key === "authorization") return { key: header.key, value: { accessKey, date: "{AWS_DATE}", region, service, signedHeaders, signatureValid: signature === expected } };
		return header;
	});
	return value;
}

function normalizedBody(path: string | undefined, headers: Record<string, string>, text: string): any {
	if (!text) return null;
	if (path === "/oauth-token") {
		const form = new URLSearchParams(text);
		const [encodedHeader, encodedPayload, signature] = form.get("assertion")!.split(".");
		const payload = JSON.parse(Buffer.from(encodedPayload!, "base64url").toString());
		payload.exp -= payload.iat;
		payload.iat = 0;
		return {
			grant_type: form.get("grant_type"),
			assertion: {
				header: JSON.parse(Buffer.from(encodedHeader!, "base64url").toString()),
				payload,
				signatureBytes: Buffer.from(signature!, "base64url").length,
				signatureValid: verify(
					"RSA-SHA256",
					Buffer.from(`${encodedHeader}.${encodedPayload}`),
					createPublicKey(readFileSync(join(fixtureDir, "service-account-key.pem"))),
					Buffer.from(signature!, "base64url"),
				),
			},
		};
	}
	if (path === "/sts") {
		const form = new URLSearchParams(text);
		const entries = Object.fromEntries(form);
		if (entries.subject_token_type === "urn:ietf:params:aws:token-type:aws4_request") entries.subject_token = normalizeAwsSubject(entries.subject_token);
		return entries;
	}
	return headers["content-type"]?.startsWith("application/json") ? JSON.parse(text) : text;
}

async function run(c: any) {
	const requests: any[] = [];
	let i = 0;
	const server = createServer((req, res) => {
		const bs: Buffer[] = [];
		req.on("data", x => bs.push(x));
		req.on("end", () => {
			const text = Buffer.concat(bs).toString();
			const headers = Object.fromEntries(Object.entries(req.headers)
				.filter(([k]) => !drop.has(k))
				.map(([k, v]) => [k, Array.isArray(v) ? v.join(", ") : v ?? ""])
				.sort(([a], [b]) => a.localeCompare(b)));
			requests.push({ method: req.method, path: req.url, headers, body: normalizedBody(req.url, headers, text) });
			if (req.url === "/token" || req.url === "/oauth-token") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ access_token: "adc-token", token_type: "Bearer", expires_in: 3600 }));
				return;
			}
			if (req.url === "/subject-text") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("url-subject-token");
				return;
			}
			if (req.url === "/subject-json") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ token: "url-json-token" }));
				return;
			}
			if (req.url === "/aws-imds-token") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("imds-token");
				return;
			}
			if (req.url === "/aws-region") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("us-east-2b");
				return;
			}
			if (req.url === "/aws-creds") {
				res.writeHead(200, { "content-type": "text/plain" });
				res.end("fixture-role");
				return;
			}
			if (req.url === "/aws-creds/fixture-role") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ AccessKeyId: "metadata-access", SecretAccessKey: "metadata-secret", Token: "metadata-session" }));
				return;
			}
			if (req.url === "/sts") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ access_token: "sts-token", issued_token_type: "urn:ietf:params:oauth:token-type:access_token", token_type: "Bearer", expires_in: 3600 }));
				return;
			}
			if (req.url === "/impersonate") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ accessToken: "adc-token", expireTime: "2099-01-01T00:00:00Z" }));
				return;
			}
			if (req.url === "/project/123") {
				res.writeHead(200, { "content-type": "application/json" });
				res.end(JSON.stringify({ projectId: "p" }));
				return;
			}
			const r = c.responses[i++] ?? c.responses.at(-1);
			if (!r) { res.destroy(); return; }
			const body = r.chunks ? r.chunks.map((x: any) => `data: ${JSON.stringify(x)}\n\n`).join("") : r.json !== undefined ? JSON.stringify(r.json) : r.text ?? "";
			res.writeHead(r.status, { "content-type": r.chunks ? "text/event-stream" : r.json !== undefined ? "application/json" : "text/plain" });
			res.end(body);
		});
	});

	await new Promise<void>(ok => server.listen(0, "127.0.0.1", ok));
	const url = `http://127.0.0.1:${(server.address() as any).port}`;
	const model = { ...spec.models[c.model], ...(!c.noServerBase ? { baseUrl: url + (c.baseSuffix ?? "") } : {}) };
	let dir: string | undefined;
	const oldCredentials = process.env.GOOGLE_APPLICATION_CREDENTIALS;
	const oldAllowExecutables = process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES;
	const oldAws = Object.fromEntries(["AWS_REGION", "AWS_DEFAULT_REGION", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"].map(name => [name, process.env[name]]));
	const originalAdapter = Gaxios.prototype._defaultAdapter;
	if (c.adc) {
		dir = mkdtempSync(join(tmpdir(), "pi-vertex-oracle-"));
		const credentialPath = join(dir, "adc.json");
		let credentials: any;
		if (c.adc === "authorized-user") {
			credentials = { type: "external_account_authorized_user", client_id: "client", client_secret: "secret", refresh_token: "refresh", token_url: `${url}/token` };
		} else if (c.adc === "service-account") {
			credentials = { type: "service_account", project_id: "p", client_email: "test@p.iam.gserviceaccount.com", private_key: readFileSync(join(fixtureDir, "service-account-key.pem"), "utf8") };
			Gaxios.prototype._defaultAdapter = function(config: any) {
				if (config.url.toString() === "https://oauth2.googleapis.com/token") {
					config = { ...config, url: new URL(`${url}/oauth-token`) };
				}
				return originalAdapter.call(this, config);
			};
		} else if (c.adc === "workload-certificate") {
			const certificateConfigPath = join(dir, "certificate-config.json");
			writeFileSync(certificateConfigPath, JSON.stringify({
				cert_configs: { workload: {
					cert_path: join(fixtureDir, "certificate.pem"),
					key_path: join(fixtureDir, "certificate-key.pem"),
				} },
			}));
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:oauth:token-type:mtls",
				token_url: `${url}/sts`,
				credential_source: { certificate: {
					certificate_config_location: certificateConfigPath,
					trust_chain_path: join(fixtureDir, "certificate-chain.pem"),
				} },
				cloud_resource_manager_url: `${url}/project/`,
			};
			// The oracle's loopback is HTTP; remove only the mTLS agent at this
			// deterministic transport seam. Production google-auth still creates and
			// installs the certificate-backed agent before every request.
			Gaxios.prototype._defaultAdapter = function(config: any) {
				if (config.url.toString().startsWith(url)) config = { ...config, agent: undefined };
				return originalAdapter.call(this, config);
			};
		} else if (c.adc === "workload-aws-env" || c.adc === "workload-aws-metadata") {
			const source: any = {
				environment_id: "aws1",
				regional_cred_verification_url: `${url}/aws-verify?Action=GetCallerIdentity&Version=2011-06-15`,
			};
			if (c.adc === "workload-aws-env") {
				process.env.AWS_REGION = "us-west-1";
				process.env.AWS_ACCESS_KEY_ID = "env-access";
				process.env.AWS_SECRET_ACCESS_KEY = "env-secret";
				process.env.AWS_SESSION_TOKEN = "env-session";
			} else {
				for (const name of Object.keys(oldAws)) delete process.env[name];
				Object.assign(source, { region_url: `${url}/aws-region`, url: `${url}/aws-creds`, imdsv2_session_token_url: `${url}/aws-imds-token` });
			}
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:aws:token-type:aws4_request",
				token_url: `${url}/sts`,
				credential_source: source,
				cloud_resource_manager_url: `${url}/project/`,
			};
		} else if (c.adc === "workload-executable" || c.adc === "workload-executable-cached") {
			const executablePath = join(dir, "subject token.mjs");
			const outputFile = join(dir, "executable-output.json");
			writeFileSync(executablePath, `const token = [process.env.GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE, process.env.GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE, process.env.GOOGLE_EXTERNAL_ACCOUNT_INTERACTIVE, process.env.GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE ?? ""].join("|"); process.stdout.write(JSON.stringify({version:1,success:true,token_type:"urn:ietf:params:oauth:token-type:jwt",id_token:token}));`);
			const executable: any = { command: `${process.execPath} "${executablePath}"`, timeout_millis: 5000 };
			if (c.adc === "workload-executable-cached") {
				writeFileSync(outputFile, JSON.stringify({ version: 1, success: true, token_type: "urn:ietf:params:oauth:token-type:jwt", id_token: "cached-executable-token", expiration_time: Math.round(Date.now() / 1000) + 3600 }));
				executable.output_file = outputFile;
				executable.command = "must-not-run";
			}
			process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES = "1";
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:oauth:token-type:jwt",
				token_url: `${url}/sts`,
				credential_source: { executable },
				cloud_resource_manager_url: `${url}/project/`,
			};
		} else {
			const usesJson = c.adc === "workload-json-impersonated" || c.adc === "workload-url-json";
			const usesUrl = c.adc === "workload-url-text" || c.adc === "workload-url-json";
			const credentialSource = usesUrl
				? { url: `${url}/subject-${usesJson ? "json" : "text"}`, headers: { "x-subject-header": "present" } }
				: { file: join(dir, "subject-token") };
			if (!usesUrl) {
				writeFileSync(credentialSource.file!, usesJson ? JSON.stringify({ token: "subject-token" }) : "subject-token");
			}
			if (usesJson) {
				Object.assign(credentialSource, { format: { type: "json", subject_token_field_name: "token" } });
			}
			credentials = {
				type: "external_account",
				audience: "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/provider",
				subject_token_type: "urn:ietf:params:oauth:token-type:jwt",
				token_url: `${url}/sts`,
				credential_source: credentialSource,
				cloud_resource_manager_url: `${url}/project/`,
				...(c.adc === "workload-json-impersonated" ? { service_account_impersonation_url: `${url}/impersonate`, service_account_impersonation: { token_lifetime_seconds: 1800 } } : {}),
			};
		}

		writeFileSync(credentialPath, JSON.stringify(credentials));
		process.env.GOOGLE_APPLICATION_CREDENTIALS = credentialPath;
	}
	const events: any[] = [];
	let result: any, syncError: string | undefined;
	try {
		const s = c.simple ? streamSimpleGoogleVertex(model, c.context, c.options) : streamGoogleVertex(model, c.context, c.options);
		for await (const e of s) { const { partial, message, error, ...rest } = e as any; events.push(rest); }
		result = { ...await s.result(), timestamp: 0 };
	} catch (e) {
		syncError = e instanceof Error ? e.message : String(e);
	} finally {
		server.close();
		Gaxios.prototype._defaultAdapter = originalAdapter;
		if (oldCredentials === undefined) delete process.env.GOOGLE_APPLICATION_CREDENTIALS;
		else process.env.GOOGLE_APPLICATION_CREDENTIALS = oldCredentials;
		if (oldAllowExecutables === undefined) delete process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES;
		else process.env.GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES = oldAllowExecutables;
		for (const [name, value] of Object.entries(oldAws)) {
			if (value === undefined) delete process.env[name];
			else process.env[name] = value;
		}
		if (dir) rmSync(dir, { recursive: true, force: true });
	}
	return { name: c.name, requests, ...(syncError ? { syncError } : { events, result }) };
}

async function main() {
	const cases = [];
	for (const c of spec.cases) cases.push(await run(c));
	console.log(JSON.stringify({ cases }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// hljs-parity driver (port of tests/hljs-parity/gen-oracle.ts).
pub const HLJS_DRIVER: &str = r#"// Regenerates tests/hljs-parity/oracle.json from the vendored
// highlight.js 10.7.3 (`ref/pi/node_modules/highlight.js`) — the library
// Pi's coding agent uses through utils/syntax-highlight.ts. Run via
// scripts/hljs-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import hljs from "../../ref/pi/node_modules/highlight.js/lib/index.js";

type Case = { name: string; language?: string; subset?: string[]; code: string };

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Case[];

const oracle = cases.map((c) => {
	const result = c.language
		? hljs.highlight(c.code, { language: c.language, ignoreIllegals: true })
		: hljs.highlightAuto(c.code, c.subset);
	return {
		name: c.name,
		value: result.value,
		relevance: result.relevance,
		illegal: !!result.illegal,
		detectedLanguage: c.language ? result.language : result.language,
	};
});

console.log(JSON.stringify(oracle, null, "\t"));
"#;

/// image-parity driver (port of tests/image-parity/gen-oracle.ts).
pub const IMAGE_DRIVER: &str = r#"// Regenerates tests/image-parity/oracle.json from Pi's real image
// machinery: `utils/image-resize-core.ts` `resizeImageInProcess` and
// `utils/image-convert.ts` `convertToPng`, both running the vendored
// `@silvia-odwyer/photon-node` 0.3.4 WASM build. Case *inputs* are
// synthesized deterministically here (photon from raw pixels, plus EXIF
// segment splicing) and recorded in the oracle alongside the expected
// outputs, so pi-rs's replay consumes exact bytes. Run via
// scripts/image-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import { convertToPng } from "../../ref/pi/packages/coding-agent/src/utils/image-convert.ts";
import { resizeImageInProcess } from "../../ref/pi/packages/coding-agent/src/utils/image-resize-core.ts";
import { loadPhoton } from "../../ref/pi/packages/coding-agent/src/utils/photon.ts";

type Pattern = "gradient" | "noise" | "flat";

interface CaseSpec {
	name: string;
	width: number;
	height: number;
	pattern: Pattern;
	format: "png" | "jpeg" | "webp";
	/** Splice an EXIF APP1 orientation segment (JPEG only). */
	exifOrientation?: number;
	mimeType: string;
	kind: "resize" | "convert";
	options?: { maxWidth?: number; maxHeight?: number; maxBytes?: number; jpegQuality?: number };
	/** convert cases: corrupt the base64 payload instead. */
	garbage?: boolean;
}

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as CaseSpec[];

// Deterministic LCG so inputs are reproducible across regenerations.
function makeRng(seed: number): () => number {
	let state = seed >>> 0;
	return () => {
		state = (state * 1664525 + 1013904223) >>> 0;
		return state / 0x100000000;
	};
}

function makePixels(width: number, height: number, pattern: Pattern): Uint8Array {
	const pixels = new Uint8Array(width * height * 4);
	const rng = makeRng(width * 7919 + height * 104729 + pattern.length);
	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const i = (y * width + x) * 4;
			if (pattern === "gradient") {
				pixels[i] = Math.floor((x * 255) / Math.max(1, width - 1));
				pixels[i + 1] = Math.floor((y * 255) / Math.max(1, height - 1));
				pixels[i + 2] = (x + y) % 256;
			} else if (pattern === "noise") {
				pixels[i] = Math.floor(rng() * 256);
				pixels[i + 1] = Math.floor(rng() * 256);
				pixels[i + 2] = Math.floor(rng() * 256);
			} else {
				pixels[i] = 40;
				pixels[i + 1] = 90;
				pixels[i + 2] = 160;
			}
			pixels[i + 3] = 255;
		}
	}
	return pixels;
}

/** Minimal little-endian EXIF APP1 segment carrying only tag 0x0112. */
function exifApp1(orientation: number): Uint8Array {
	const payload = new Uint8Array(32);
	payload.set([0x45, 0x78, 0x69, 0x66, 0x00, 0x00], 0); // "Exif\0\0"
	payload.set([0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00], 6); // II TIFF, IFD @8
	payload.set([0x01, 0x00], 14); // entry count 1
	payload.set([0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, orientation, 0x00, 0x00, 0x00], 16);
	payload.set([0x00, 0x00, 0x00, 0x00], 28); // next IFD
	const segment = new Uint8Array(4 + payload.length);
	segment[0] = 0xff;
	segment[1] = 0xe1;
	const length = payload.length + 2;
	segment[2] = (length >> 8) & 0xff;
	segment[3] = length & 0xff;
	segment.set(payload, 4);
	return segment;
}

function spliceExif(jpeg: Uint8Array, orientation: number): Uint8Array {
	const app1 = exifApp1(orientation);
	const out = new Uint8Array(jpeg.length + app1.length);
	out.set(jpeg.subarray(0, 2), 0); // SOI
	out.set(app1, 2);
	out.set(jpeg.subarray(2), 2 + app1.length);
	return out;
}

async function synthesize(spec: CaseSpec): Promise<Uint8Array> {
	const photon = await loadPhoton();
	if (!photon) throw new Error("photon-node failed to load");
	const image = new photon.PhotonImage(makePixels(spec.width, spec.height, spec.pattern), spec.width, spec.height);
	try {
		let bytes: Uint8Array;
		if (spec.format === "png") bytes = image.get_bytes();
		else if (spec.format === "jpeg") bytes = image.get_bytes_jpeg(90);
		else bytes = image.get_bytes_webp();
		if (spec.exifOrientation !== undefined) {
			if (spec.format !== "jpeg") throw new Error(`${spec.name}: EXIF splicing supports JPEG only`);
			bytes = spliceExif(bytes, spec.exifOrientation);
		}
		return bytes;
	} finally {
		image.free();
	}
}

async function main(): Promise<void> {
	const oracle: unknown[] = [];
	for (const spec of cases) {
		const inputBytes = await synthesize(spec);
		let input = Buffer.from(inputBytes).toString("base64");
		if (spec.garbage) input = `not-an-image-${input.slice(0, 24)}`;
		if (spec.kind === "resize") {
			const result = await resizeImageInProcess(
				new Uint8Array(Buffer.from(input, "base64")),
				spec.mimeType,
				spec.options,
			);
			oracle.push({ name: spec.name, kind: spec.kind, mimeType: spec.mimeType, options: spec.options ?? null, input, expected: result });
		} else {
			const result = await convertToPng(input, spec.mimeType);
			oracle.push({ name: spec.name, kind: spec.kind, mimeType: spec.mimeType, input, expected: result });
		}
	}
	console.log(JSON.stringify(oracle, null, "\t"));
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
"#;

/// jsdiff-parity driver (port of tests/jsdiff-parity/gen-oracle.ts).
pub const JSDIFF_DRIVER: &str = r#"// Regenerates tests/jsdiff-parity/oracle.json from the vendored jsdiff 8.0.4
// (`ref/pi/node_modules/diff`) — the same library Pi's coding agent uses for
// edit diffs (`edit-diff.ts`) and intra-line diff highlighting (`diff.ts`).
// Run via scripts/jsdiff-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import * as Diff from "../../ref/pi/node_modules/diff/libesm/index.js";

type DiffCase = { name: string; old: string; new: string };
type PatchCase = DiffCase & { oldName: string; newName: string; context: number; headers: string };
type Cases = { lines: DiffCase[]; words: DiffCase[]; patch: PatchCase[] };

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Cases;

function headerOptions(headers: string) {
	switch (headers) {
		case "include":
			return Diff.INCLUDE_HEADERS;
		case "file":
			return Diff.FILE_HEADERS_ONLY;
		case "omit":
			return Diff.OMIT_HEADERS;
		default:
			throw new Error(`unknown headers option: ${headers}`);
	}
}

const oracle = {
	lines: cases.lines.map((c) => ({ name: c.name, changes: Diff.diffLines(c.old, c.new) })),
	words: cases.words.map((c) => ({ name: c.name, changes: Diff.diffWords(c.old, c.new) })),
	patch: cases.patch.map((c) => ({
		name: c.name,
		patch: Diff.createTwoFilesPatch(c.oldName, c.newName, c.old, c.new, undefined, undefined, {
			context: c.context,
			headerOptions: headerOptions(c.headers),
		}),
	})),
};

console.log(JSON.stringify(oracle, null, "\t"));
"#;

/// mistral-conversations-parity driver (port of tests/mistral-conversations-parity/gen-oracle.ts).
pub const MISTRAL_CONVERSATIONS_DRIVER: &str = r#"import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { streamMistral, streamSimpleMistral } from "../../ref/pi/packages/ai/src/providers/mistral.ts";

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8"));
const drop = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode", "user-agent"]);

async function run(c: any) {
  const requests: any[] = [];
  let responseIndex = 0;
  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", chunk => chunks.push(chunk));
    req.on("end", () => {
      const text = Buffer.concat(chunks).toString();
      const headers = Object.fromEntries(Object.entries(req.headers)
        .filter(([name]) => !drop.has(name))
        .map(([name, value]) => [name, Array.isArray(value) ? value.join(", ") : value ?? ""])
        .sort(([a], [b]) => a.localeCompare(b)));
      requests.push({ method: req.method, path: req.url, headers, body: text ? JSON.parse(text) : null });
      const scripted = c.responses[responseIndex++] ?? c.responses.at(-1);
      if (!scripted) { res.destroy(); return; }
      const body = scripted.events
        ? scripted.events.map((event: unknown) => `data: ${JSON.stringify(event)}\n\n`).join("") + "data: [DONE]\n\n"
        : scripted.json !== undefined ? JSON.stringify(scripted.json) : scripted.text ?? "";
      res.writeHead(scripted.status, { "content-type": scripted.events ? "text/event-stream" : scripted.json !== undefined ? "application/json" : "text/plain" });
      res.end(body);
    });
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const model = { ...spec.models[c.model], baseUrl: `http://127.0.0.1:${(server.address() as any).port}` };
  const events: any[] = [];
  let result: any;
  let syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleMistral(model, c.context, c.options) : streamMistral(model, c.context, c.options);
    for await (const event of stream) {
      const { partial, message, error, ...summary } = event as any;
      events.push(summary);
    }
    result = { ...await stream.result(), timestamp: 0 };
  } catch (error) {
    syncError = error instanceof Error ? error.message : String(error);
  } finally {
    server.close();
  }
  return { name: c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}

async function main() {
  const cases = [];
  for (const c of spec.cases) cases.push(await run(c));
  console.log(JSON.stringify({ cases }, null, "\t"));
}

main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// openai-codex-responses-parity driver (port of tests/openai-codex-responses-parity/gen-oracle.ts).
pub const OPENAI_CODEX_RESPONSES_DRIVER: &str = r#"// Pi-derived OpenAI Codex Responses SSE oracle. Run via scripts/openai-codex-responses-oracle.
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import {
  streamOpenAICodexResponses, streamSimpleOpenAICodexResponses, type OpenAICodexResponsesOptions,
} from "../../ref/pi/packages/ai/src/providers/openai-codex-responses.ts";
import type { Context, Model, SimpleStreamOptions } from "../../ref/pi/packages/ai/src/types.ts";

type Response = { status: number; headers?: Record<string, string>; sse?: string; events?: unknown[]; json?: unknown; text?: string };
type Case = { name: string; model: string; simple?: boolean; context: Context; options: Record<string, unknown>; responses: Response[] };
type Spec = { models: Record<string, Model<"openai-codex-responses">>; sse: Record<string, unknown[]>; cases: Case[] };
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Spec;
const DROP = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode", "user-agent"]);
function headers(raw: Record<string, string | string[] | undefined>) {
  return Object.fromEntries(Object.entries(raw).filter(([k]) => !DROP.has(k)).map(([k,v]) => [k, Array.isArray(v) ? v.join(", ") : (v ?? "")]).sort(([a],[b]) => a.localeCompare(b)));
}
function body(r: Response) {
  const events = r.sse ? spec.sse[r.sse]! : r.events;
  if (events) return { contentType: "text/event-stream", text: events.map(e => `data: ${JSON.stringify(e)}\n\n`).join("") };
  if (r.json !== undefined) return { contentType: "application/json", text: JSON.stringify(r.json) };
  return { contentType: "text/plain", text: r.text ?? "" };
}
function serve(c: Case): Promise<{ server: Server; url: string; requests: unknown[] }> {
  const requests: unknown[] = []; let index = 0;
  const server = createServer((req, res) => { const chunks: Buffer[] = [];
    req.on("data", c => chunks.push(c)); req.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8");
      requests.push({ method: req.method ?? "", path: req.url ?? "", headers: headers(req.headers), body: text ? JSON.parse(text) : null });
      const scripted = c.responses[index++] ?? c.responses.at(-1)!; const value = body(scripted);
      res.writeHead(scripted.status, { "content-type": value.contentType, ...(scripted.headers ?? {}) }); res.end(value.text);
    });
  });
  return new Promise(resolve => server.listen(0, "127.0.0.1", () => resolve({ server, url: `http://127.0.0.1:${(server.address() as {port:number}).port}`, requests })));
}
function summarize(event: Record<string, unknown>) { const { partial: _p, message: _m, error: _e, ...rest } = event; return rest; }
async function run(c: Case) {
  const { server, url, requests } = await serve(c); const model = { ...spec.models[c.model]!, baseUrl: url };
  const events: unknown[] = []; let result: unknown; let syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleOpenAICodexResponses(model, c.context, c.options as SimpleStreamOptions) : streamOpenAICodexResponses(model, c.context, c.options as OpenAICodexResponsesOptions);
    for await (const event of stream) events.push(summarize(event as unknown as Record<string, unknown>));
    result = { ...(await stream.result()) as unknown as Record<string, unknown>, timestamp: 0 };
  } catch (e) { syncError = e instanceof Error ? e.message : String(e); }
  server.close();
  return { name: c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}
async function main() {
  const cases = []; for (const c of spec.cases) cases.push(await run(c));
  console.log(JSON.stringify({ cases }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// openai-codex-websocket-parity driver (port of tests/openai-codex-websocket-parity/gen-oracle.ts).
pub const OPENAI_CODEX_WEBSOCKET_DRIVER: &str = r#"import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { WebSocketServer } from "../../ref/pi/node_modules/ws/wrapper.mjs";
import {
  closeOpenAICodexWebSocketSessions,
  getOpenAICodexWebSocketDebugStats,
  resetOpenAICodexWebSocketDebugStats,
  streamOpenAICodexResponses,
} from "../../ref/pi/packages/ai/src/providers/openai-codex-responses.ts";
import type { Context, Model, Transport } from "../../ref/pi/packages/ai/src/types.ts";

type Turn = { context: Context; events: unknown[] };
type Scenario = { name: string; transport: Transport; sessionId: string; failBeforeStart?: boolean; timeoutBeforeStart?: boolean; timeoutMs?: number; turns: Turn[] };
type Spec = { model: Model<"openai-codex-responses">; token: string; scenarios: Scenario[] };
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Spec;
const KEEP = new Set(["authorization", "chatgpt-account-id", "openai-beta", "originator", "session-id", "x-client-request-id"]);
function selected(headers: Record<string, string | string[] | undefined>) {
  return Object.fromEntries(Object.entries(headers).filter(([key]) => KEEP.has(key)).map(([key, value]) => [key, Array.isArray(value) ? value.join(", ") : value ?? ""]).sort(([a], [b]) => a.localeCompare(b)));
}
function summarize(event: Record<string, unknown>) { const { partial: _p, message: _m, error: _e, ...rest } = event; return rest; }
function scrubResult(value: any) {
  value.timestamp = 0;
  for (const diagnostic of value.diagnostics ?? []) {
    diagnostic.timestamp = 0;
    if (diagnostic.error) delete diagnostic.error.stack;
  }
  return value;
}
async function run(scenario: Scenario) {
  resetOpenAICodexWebSocketDebugStats(scenario.sessionId);
  closeOpenAICodexWebSocketSessions(scenario.sessionId);
  const wsRequests: unknown[] = []; const httpRequests: unknown[] = []; let wsTurn = 0;
  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", chunk => chunks.push(chunk));
    req.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8");
      httpRequests.push({ method: req.method, path: req.url, headers: selected(req.headers), body: text ? JSON.parse(text) : null });
      const events = scenario.turns[0]!.events.map(event => `data: ${JSON.stringify(event)}\n\n`).join("");
      res.writeHead(200, { "content-type": "text/event-stream" }); res.end(events);
    });
  });
  const wss = new WebSocketServer({ server });
  wss.on("connection", (socket, request) => {
    if (scenario.failBeforeStart) { socket.terminate(); return; }
    socket.on("message", data => {
      const turn = scenario.turns[wsTurn++]!;
      wsRequests.push({ path: request.url, headers: selected(request.headers), body: JSON.parse(data.toString()) });
      if (!scenario.timeoutBeforeStart) for (const event of turn.events) socket.send(JSON.stringify(event));
    });
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address() as { port: number };
  const model = { ...spec.model, baseUrl: `http://127.0.0.1:${address.port}` };
  const turns = [];
  for (const turn of scenario.turns) {
    const stream = streamOpenAICodexResponses(model, turn.context, {
      apiKey: spec.token, transport: scenario.transport, sessionId: scenario.sessionId, timeoutMs: scenario.timeoutMs,
    });
    const events = [];
    for await (const event of stream) events.push(summarize(event as any));
    turns.push({ events, result: scrubResult(await stream.result()) });
  }
  const stats = getOpenAICodexWebSocketDebugStats(scenario.sessionId);
  closeOpenAICodexWebSocketSessions(scenario.sessionId);
  await new Promise<void>(resolve => wss.close(() => server.close(() => resolve())));
  return { name: scenario.name, wsRequests, httpRequests, turns, stats };
}
async function main() {
  const scenarios = [];
  for (const scenario of spec.scenarios) scenarios.push(await run(scenario));
  console.log(JSON.stringify({ scenarios }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// openai-completions-parity driver (port of tests/openai-completions-parity/gen-oracle.ts).
pub const OPENAI_COMPLETIONS_DRIVER: &str = r#"import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { streamOpenAICompletions, streamSimpleOpenAICompletions } from "../../ref/pi/packages/ai/src/providers/openai-completions.ts";

const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8"));
const drop = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode"]);

async function run(c: any) {
  const requests: any[] = [];
  let index = 0;
  const server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", chunk => chunks.push(chunk));
    req.on("end", () => {
      const text = Buffer.concat(chunks).toString();
      const headers = Object.fromEntries(Object.entries(req.headers)
        .filter(([name, value]) => !drop.has(name) && !name.startsWith("x-stainless-") && !(name === "user-agent" && !String(value).startsWith("claude-cli/")))
        .map(([name, value]) => [name, Array.isArray(value) ? value.join(", ") : value ?? ""])
        .sort(([a], [b]) => a.localeCompare(b)));
      requests.push({ method:req.method, path:req.url, headers, body:text ? JSON.parse(text) : null });
      const scripted = c.responses[index++] ?? c.responses.at(-1);
      if (!scripted) { res.destroy(); return; }
      const body = scripted.events ? scripted.events.map((event: unknown) => `data: ${JSON.stringify(event)}\n\n`).join("") + "data: [DONE]\n\n" : scripted.json !== undefined ? JSON.stringify(scripted.json) : scripted.text ?? "";
      res.writeHead(scripted.status, { "content-type":scripted.events ? "text/event-stream" : scripted.json !== undefined ? "application/json" : "text/plain" });
      res.end(body);
    });
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const model = { ...spec.models[c.model], baseUrl:`http://127.0.0.1:${(server.address() as any).port}` };
  const events: any[] = [];
  let result: any, syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleOpenAICompletions(model, c.context, c.options) : streamOpenAICompletions(model, c.context, c.options);
    for await (const event of stream) { const { partial, message, error, ...summary } = event as any; events.push(summary); }
    result = { ...await stream.result(), timestamp:0 };
  } catch (error) { syncError = error instanceof Error ? error.message : String(error); }
  finally { server.close(); }
  return { name:c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}

async function main() {
  const cases = [];
  for (const c of spec.cases) cases.push(await run(c));
  console.log(JSON.stringify({ cases }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// openai-responses-parity driver (port of tests/openai-responses-parity/gen-oracle.ts).
pub const OPENAI_RESPONSES_DRIVER: &str = r#"// Pi-derived OpenAI Responses oracle. Run via scripts/openai-responses-oracle.
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import {
  streamOpenAIResponses, streamSimpleOpenAIResponses, type OpenAIResponsesOptions,
} from "../../ref/pi/packages/ai/src/providers/openai-responses.ts";
import type { Context, Model, SimpleStreamOptions } from "../../ref/pi/packages/ai/src/types.ts";

type Response = { status: number; sse?: string; events?: unknown[]; json?: unknown; text?: string };
type Case = { name: string; model: string; simple?: boolean; context: Context; options: Record<string, unknown>; responses: Response[] };
type Spec = { models: Record<string, Model<"openai-responses">>; sse: Record<string, unknown[]>; cases: Case[] };
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Spec;
const DROP = new Set(["host", "content-length", "connection", "accept-encoding", "accept-language", "sec-fetch-mode"]);
function headers(raw: Record<string, string | string[] | undefined>) {
  return Object.fromEntries(Object.entries(raw).filter(([k, v]) => !DROP.has(k) && !k.startsWith("x-stainless-") && !(k === "user-agent" && !String(v).startsWith("claude-cli/"))).map(([k,v]) => [k, Array.isArray(v) ? v.join(", ") : (v ?? "")]).sort(([a],[b]) => a.localeCompare(b)));
}
function body(r: Response) {
  const events = r.sse ? spec.sse[r.sse]! : r.events;
  if (events) return { contentType: "text/event-stream", text: events.map(e => `data: ${JSON.stringify(e)}\n\n`).join("") };
  if (r.json !== undefined) return { contentType: "application/json", text: JSON.stringify(r.json) };
  return { contentType: "text/plain", text: r.text ?? "" };
}
function serve(c: Case): Promise<{ server: Server; url: string; requests: unknown[] }> {
  const requests: unknown[] = []; let index = 0;
  const server = createServer((req, res) => { const chunks: Buffer[] = [];
    req.on("data", c => chunks.push(c)); req.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8");
      requests.push({ method: req.method ?? "", path: req.url ?? "", headers: headers(req.headers), body: text ? JSON.parse(text) : null });
      const scripted = c.responses[index++] ?? c.responses.at(-1)!; const value = body(scripted);
      res.writeHead(scripted.status, { "content-type": value.contentType }); res.end(value.text);
    });
  });
  return new Promise(resolve => server.listen(0, "127.0.0.1", () => resolve({ server, url: `http://127.0.0.1:${(server.address() as {port:number}).port}`, requests })));
}
function summarize(event: Record<string, unknown>) { const { partial: _p, message: _m, error: _e, ...rest } = event; return rest; }
async function run(c: Case) {
  const { server, url, requests } = await serve(c); const model = { ...spec.models[c.model]!, baseUrl: url };
  const events: unknown[] = []; let result: unknown; let syncError: string | undefined;
  try {
    const stream = c.simple ? streamSimpleOpenAIResponses(model, c.context, c.options as SimpleStreamOptions) : streamOpenAIResponses(model, c.context, c.options as OpenAIResponsesOptions);
    for await (const event of stream) events.push(summarize(event as unknown as Record<string, unknown>));
    result = { ...(await stream.result()) as unknown as Record<string, unknown>, timestamp: 0 };
  } catch (e) { syncError = e instanceof Error ? e.message : String(e); }
  server.close();
  return { name: c.name, requests, ...(syncError === undefined ? { events, result } : { syncError }) };
}
async function main() {
  const cases = []; for (const c of spec.cases) cases.push(await run(c));
  console.log(JSON.stringify({ cases }, null, "\t"));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
"#;

/// retry-parity driver (port of tests/retry-parity/gen-oracle.ts).
pub const RETRY_DRIVER: &str = r#"// PLAN 7.10: generate the retry policy oracle from Pi's real AgentSession.
// Classification invokes the private policy only to characterize observable
// retry decisions; run cases drive public prompt/subscribe/abortRetry and record
// stable event, attempt, context-removal, and final-state fields.
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.PI_CODING_AGENT_DIR = mkdtempSync(join(tmpdir(), "pi-rs-retry-oracle-agentdir-"));
let fauxAssistantMessage: any;
let createHarness: any;

type Json = any;
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Json;

function stableMessage(message: Json): Json {
  const content = Array.isArray(message?.content) ? message.content : [];
  return {
    role: message?.role,
    text: content.filter((block: Json) => block.type === "text").map((block: Json) => block.text ?? "").join(""),
    ...(message?.stopReason !== undefined ? { stopReason: message.stopReason } : {}),
    ...(message?.errorMessage !== undefined ? { errorMessage: message.errorMessage } : {}),
  };
}

function stableEvent(event: Json): Json | undefined {
  if (event.type === "agent_end") return { type: event.type, willRetry: event.willRetry ?? false };
  if (event.type === "auto_retry_start") {
    return {
      type: event.type, attempt: event.attempt, maxAttempts: event.maxAttempts,
      delayMs: event.delayMs, errorMessage: event.errorMessage,
    };
  }
  if (event.type === "auto_retry_end") {
    return {
      type: event.type, success: event.success, attempt: event.attempt,
      ...(event.finalError !== undefined ? { finalError: event.finalError } : {}),
    };
  }
  if (event.type === "message_end" && event.message?.role === "assistant") {
    return { type: event.type, ...stableMessage(event.message) };
  }
  return undefined;
}

async function runCase(caseSpec: Json): Promise<Json> {
  const modelDefinition = {
    id: spec.model.id, name: spec.model.name, reasoning: false,
    input: ["text"], cost: spec.model.cost,
    contextWindow: spec.model.contextWindow, maxTokens: spec.model.maxTokens,
  };
  const harness = await createHarness({
    models: [modelDefinition],
    settings: caseSpec.mode === "run" ? { retry: caseSpec.settings } : undefined,
  });
  try {
    if (caseSpec.mode === "classify") {
      const message = fauxAssistantMessage("", {
        stopReason: caseSpec.message.stopReason,
        errorMessage: caseSpec.message.errorMessage,
      });
      const retryable = (harness.session as any)._isRetryableError(message);
      return { retryable };
    }

    const contexts: Json[] = [];
    const responses: any[] = caseSpec.turns.map((turn: Json) => (context: Json) => {
      contexts.push(context.messages.map(stableMessage));
      return fauxAssistantMessage(turn.text ?? "", {
        stopReason: turn.stopReason ?? "stop",
        errorMessage: turn.errorMessage,
        timestamp: 0,
      });
    });
    harness.setResponses(responses);
    if (caseSpec.cancelAttempt !== undefined) {
      let cancelled = false;
      harness.session.subscribe((event) => {
        if (event.type === "auto_retry_start" && event.attempt === caseSpec.cancelAttempt && !cancelled) {
          cancelled = true;
          setImmediate(() => harness.session.abortRetry());
        }
      });
    }
    if (caseSpec.queueOnRetry) {
      let queued = false;
      harness.session.subscribe((event) => {
        if (event.type === "agent_end" && event.willRetry && !queued) {
          queued = true;
          void harness.session.followUp(caseSpec.queueOnRetry);
        }
      });
    }
    await harness.session.prompt(caseSpec.prompt ?? "test");
    return {
      events: harness.events.map(stableEvent).filter((event) => event !== undefined),
      callCount: harness.faux.state.callCount,
      contexts,
      messages: harness.session.messages.map(stableMessage),
    };
  } finally {
    harness.cleanup();
  }
}

async function main(): Promise<void> {
  ({ fauxAssistantMessage } = await import("../../ref/pi/packages/ai/src/providers/faux.ts"));
  ({ createHarness } = await import("../../ref/pi/packages/coding-agent/test/suite/harness.ts"));
  const cases = [];
  for (const caseSpec of spec.cases) cases.push({ name: caseSpec.name, ...(await runCase(caseSpec)) });
  process.stdout.write(`${JSON.stringify({ cases }, null, "\t")}\n`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
"#;

/// session-parity driver (port of tests/session-parity/gen-oracle.ts).
pub const SESSION_DRIVER: &str = r#"// Regenerates tests/session-parity/oracle.json by driving Pi's real
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
"#;

/// system-prompt-parity driver (port of tests/system-prompt-parity/gen-oracle.ts).
pub const SYSTEM_PROMPT_DRIVER: &str = r#"// Regenerates tests/system-prompt-parity/oracle.json from Pi's real
// buildSystemPrompt / loadProjectContextFiles / tool definitions
// (ref/pi/packages/coding-agent). The private agent-session.ts
// normalization + _rebuildSystemPrompt composition is copied here (the
// established harness pattern for private wiring bodies). Run via
// scripts/system-prompt-oracle. Do not edit the oracle by hand.
//
// Determinism pins: TZ=UTC, PI_PACKAGE_DIR=/pi-rs-pkg (config.ts doc
// paths), a per-case fixed Date, and temp fixture roots substituted with
// {ROOT} in the recorded output.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { buildSystemPrompt } from "../../ref/pi/packages/coding-agent/src/core/system-prompt.ts";
import { createAllToolDefinitions } from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";

// Both pins are read lazily (config.ts getPackageDir at call time, TZ at
// the first Date use), so setting them after the hoisted imports is safe.
process.env.TZ = "UTC";
process.env.PI_PACKAGE_DIR = "/pi-rs-pkg";

interface SkillCase {
	name: string;
	description: string;
	filePath: string;
	disableModelInvocation?: boolean;
}

interface SessionCase {
	name: string;
	toolNames: string[];
	customPrompt?: string;
	appendSystemPrompt?: string[];
	skills?: SkillCase[];
	tree: Record<string, string>;
	cwd: string;
	agentDir: string;
	nowMs: number;
}

interface RawCase {
	name: string;
	cwd: string;
	selectedTools?: string[];
	toolSnippets?: Record<string, string>;
	promptGuidelines?: string[];
	customPrompt?: string;
	appendSystemPrompt?: string;
	contextFiles?: Array<{ path: string; content: string }>;
	skills?: SkillCase[];
	nowMs: number;
}

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as {
	session: SessionCase[];
	raw: RawCase[];
};

const RealDate = Date;
function withNow<T>(nowMs: number, fn: () => T): T {
	class FakeDate extends RealDate {
		constructor(...args: unknown[]) {
			if (args.length === 0) {
				super(nowMs);
			} else {
				// biome-ignore lint/suspicious/noExplicitAny: harness
				super(...(args as [any]));
			}
		}
	}
	(FakeDate as unknown as { now: () => number }).now = () => nowMs;
	(globalThis as { Date: unknown }).Date = FakeDate;
	try {
		return fn();
	} finally {
		(globalThis as { Date: unknown }).Date = RealDate;
	}
}

// Copied from resource-loader.ts loadContextFileFromDir /
// loadProjectContextFiles (importing the module transitively hits the
// vendored jiti's missing "./static" export under tsx; the function is
// self-contained), minus the chalk warning on unreadable files.
function loadContextFileFromDir(dir: string): { path: string; content: string } | null {
	const candidates = ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];
	for (const filename of candidates) {
		const filePath = join(dir, filename);
		if (existsSync(filePath)) {
			try {
				return {
					path: filePath,
					content: readFileSync(filePath, "utf-8"),
				};
			} catch {
				// keep scanning
			}
		}
	}
	return null;
}

function loadProjectContextFiles(options: {
	cwd: string;
	agentDir: string;
}): Array<{ path: string; content: string }> {
	const resolvedCwd = resolve(options.cwd);
	const resolvedAgentDir = resolve(options.agentDir);

	const contextFiles: Array<{ path: string; content: string }> = [];
	const seenPaths = new Set<string>();

	const globalContext = loadContextFileFromDir(resolvedAgentDir);
	if (globalContext) {
		contextFiles.push(globalContext);
		seenPaths.add(globalContext.path);
	}

	const ancestorContextFiles: Array<{ path: string; content: string }> = [];

	let currentDir = resolvedCwd;
	const root = resolve("/");

	while (true) {
		const contextFile = loadContextFileFromDir(currentDir);
		if (contextFile && !seenPaths.has(contextFile.path)) {
			ancestorContextFiles.unshift(contextFile);
			seenPaths.add(contextFile.path);
		}

		if (currentDir === root) break;

		const parentDir = resolve(currentDir, "..");
		if (parentDir === currentDir) break;
		currentDir = parentDir;
	}

	contextFiles.push(...ancestorContextFiles);

	return contextFiles;
}

// Copied from agent-session.ts _normalizePromptSnippet.
function normalizePromptSnippet(text: string | undefined): string | undefined {
	if (!text) return undefined;
	const oneLine = text
		.replace(/[\r\n]+/g, " ")
		.replace(/\s+/g, " ")
		.trim();
	return oneLine.length > 0 ? oneLine : undefined;
}

// Copied from agent-session.ts _normalizePromptGuidelines.
function normalizePromptGuidelines(guidelines: string[] | undefined): string[] {
	if (!guidelines || guidelines.length === 0) {
		return [];
	}
	const unique = new Set<string>();
	for (const guideline of guidelines) {
		const normalized = guideline.trim();
		if (normalized.length > 0) {
			unique.add(normalized);
		}
	}
	return Array.from(unique);
}

const oracle = {
	session: cases.session.map((c) => {
		const root = mkdtempSync(join(tmpdir(), "pi-rs-sysprompt-"));
		try {
			for (const [rel, content] of Object.entries(c.tree ?? {})) {
				const path = join(root, rel);
				mkdirSync(dirname(path), { recursive: true });
				writeFileSync(path, content);
			}
			const cwd = resolve(root, c.cwd);
			const agentDir = resolve(root, c.agentDir);
			const contextFiles = loadProjectContextFiles({ cwd, agentDir });
			// agent-session.ts _rebuildSystemPrompt over the base tool
			// definitions (the registered-definition registry).
			const defs = createAllToolDefinitions(cwd) as Record<
				string,
				{ promptSnippet?: string; promptGuidelines?: string[] }
			>;
			const validToolNames = (c.toolNames ?? []).filter((name) => name in defs);
			const toolSnippets: Record<string, string> = {};
			const promptGuidelines: string[] = [];
			for (const name of validToolNames) {
				const snippet = normalizePromptSnippet(defs[name].promptSnippet);
				if (snippet) toolSnippets[name] = snippet;
				promptGuidelines.push(...normalizePromptGuidelines(defs[name].promptGuidelines));
			}
			const appendList = c.appendSystemPrompt ?? [];
			const prompt = withNow(c.nowMs, () =>
				buildSystemPrompt({
					cwd,
					skills: (c.skills ?? []) as never,
					contextFiles,
					customPrompt: c.customPrompt || undefined,
					appendSystemPrompt: appendList.length > 0 ? appendList.join("\n\n") : undefined,
					selectedTools: validToolNames,
					toolSnippets,
					promptGuidelines,
				}),
			);
			return {
				name: c.name,
				contextFiles: contextFiles.map((file) => ({
					path: file.path.split(root).join("{ROOT}"),
					content: file.content,
				})),
				prompt: prompt.split(root).join("{ROOT}"),
			};
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	}),
	raw: cases.raw.map((c) => ({
		name: c.name,
		prompt: withNow(c.nowMs, () =>
			buildSystemPrompt({
				cwd: c.cwd,
				selectedTools: c.selectedTools ?? undefined,
				toolSnippets: c.toolSnippets ?? undefined,
				promptGuidelines: c.promptGuidelines ?? undefined,
				customPrompt: c.customPrompt ?? undefined,
				appendSystemPrompt: c.appendSystemPrompt ?? undefined,
				contextFiles: c.contextFiles ?? undefined,
				skills: (c.skills ?? undefined) as never,
			}),
		),
	})),
};

console.log(JSON.stringify(oracle, null, "\t"));
"#;

/// tool-parity driver (port of tests/tool-parity/gen-oracle.ts).
pub const TOOL_DRIVER: &str = r#"// Regenerates tests/tool-parity/oracle.json from Pi's real core/tools
// implementations (ref/pi/packages/coding-agent): each case builds a
// fixture tree in a temp root, runs the tool's prepareArguments +
// execute exactly the way the agent loop invokes it (toolCallId, args,
// signal, onUpdate, ctx), and records the result/error plus filesystem
// effects. Run via scripts/tool-oracle. Do not edit the oracle by hand.
//
// Determinism pins: PI_CODING_AGENT_DIR points at an empty temp dir so
// ensureTool resolves the system rg/fd from PATH (the nix shell provides
// them); temp roots are substituted with {ROOT} and the bash tool's
// persisted full-output path with {FULL_OUTPUT} in the recorded output.
// Grep/find cases are restricted to deterministic outputs (single
// matching file) because rg/fd traverse directories in parallel; the
// multi-file ordering behavior stays covered by pi-rs's behavioral tests.
// The read image cases cover both auto-resize outcomes: a small PNG
// within all limits (image-resize-core: wasResized=false, original
// bytes untouched) and an oversized PNG that pi resizes through Photon
// (pi-rs through the pi.image photon-slice port — byte parity pinned by
// tests/image-parity).
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";

// The env pin must land before the tools modules load (tools-manager.ts
// computes TOOLS_DIR at import time), so the ref/pi imports are dynamic
// inside main() — the tsx cjs transform rejects top-level await.
process.env.PI_CODING_AGENT_DIR = mkdtempSync(join(tmpdir(), "pi-rs-tool-oracle-agentdir-"));

type Generated = { gen: "lines"; count: number; prefix?: string; suffix?: string } | {
	gen: "repeat";
	unit: string;
	count: number;
};

interface Case {
	name: string;
	tool: "read" | "bash" | "edit" | "write" | "grep" | "find" | "ls";
	tree: Record<string, string | Generated>;
	binary?: Record<string, string>;
	args: Record<string, unknown>;
	abort?: "pre";
	abortAfterMs?: number;
	model?: { id: string; input: string[] };
	recordFs?: boolean;
	recordFullOutput?: boolean;
}

function generate(spec: Generated): string {
	if (spec.gen === "repeat") return spec.unit.repeat(spec.count);
	const lines: string[] = [];
	for (let i = 1; i <= spec.count; i++) {
		lines.push(`${spec.prefix ?? ""}${i}${spec.suffix ?? ""}`);
	}
	return lines.join("\n");
}

function materialize(root: string, c: Case): void {
	for (const [rel, value] of Object.entries(c.tree ?? {})) {
		const path = join(root, rel);
		if (rel.endsWith("/")) {
			mkdirSync(path, { recursive: true });
			continue;
		}
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, typeof value === "string" ? value : generate(value));
	}
	for (const [rel, base64] of Object.entries(c.binary ?? {})) {
		const path = join(root, rel);
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, Buffer.from(base64, "base64"));
	}
}

function walkFiles(root: string): Record<string, string> {
	const out: Record<string, string> = {};
	const visit = (dir: string): void => {
		for (const entry of readdirSync(dir).sort()) {
			const path = join(dir, entry);
			if (statSync(path).isDirectory()) visit(path);
			else out[relative(root, path).split(sep).join("/")] = readFileSync(path, "utf-8");
		}
	};
	visit(root);
	return out;
}

async function main(): Promise<void> {
	const { createReadToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/read.ts"
	);
	const { createBashToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/bash.ts"
	);
	const { createEditToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/edit.ts"
	);
	const { createWriteToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/write.ts"
	);
	const { createGrepToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/grep.ts"
	);
	const { createFindToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/find.ts"
	);
	const { createLsToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/ls.ts"
	);

	function createDefinition(tool: Case["tool"], cwd: string) {
		switch (tool) {
			case "read":
				return createReadToolDefinition(cwd);
			case "bash":
				return createBashToolDefinition(cwd);
			case "edit":
				return createEditToolDefinition(cwd);
			case "write":
				return createWriteToolDefinition(cwd);
			case "grep":
				return createGrepToolDefinition(cwd);
			case "find":
				return createFindToolDefinition(cwd);
			case "ls":
				return createLsToolDefinition(cwd);
		}
	}

	const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as { cases: Case[] };

	const results: unknown[] = [];
	for (const c of cases.cases) {
	const root = mkdtempSync(join(tmpdir(), "pi-rs-tool-parity-"));
	try {
		materialize(root, c);
		const definition = createDefinition(c.tool, root) as {
			prepareArguments?: (args: unknown) => unknown;
			execute: (
				id: string,
				args: unknown,
				signal?: AbortSignal,
				onUpdate?: unknown,
				ctx?: unknown,
			) => Promise<{ content: unknown; details?: unknown }>;
		};
		const controller = new AbortController();
		if (c.abort === "pre") controller.abort();
		let abortTimer: NodeJS.Timeout | undefined;
		if (typeof c.abortAfterMs === "number") {
			abortTimer = setTimeout(() => controller.abort(), c.abortAfterMs);
		}
		let ok = true;
		let payload: unknown;
		try {
			let args: unknown = c.args;
			if (definition.prepareArguments) args = definition.prepareArguments(args);
			payload = await definition.execute(
				"parity-call",
				args,
				controller.signal,
				undefined,
				c.model ? { model: c.model } : undefined,
			);
		} catch (error) {
			ok = false;
			payload = error instanceof Error ? error.message : String(error);
		} finally {
			if (abortTimer) clearTimeout(abortTimer);
		}

		let fullOutput: string | undefined;
		let fullOutputPath: string | undefined;
		if (
			ok &&
			payload &&
			typeof payload === "object" &&
			(payload as { details?: { fullOutputPath?: string } }).details?.fullOutputPath
		) {
			fullOutputPath = (payload as { details: { fullOutputPath: string } }).details.fullOutputPath;
			if (c.recordFullOutput) fullOutput = readFileSync(fullOutputPath, "utf-8");
		}

		const substitute = (text: string): string => {
			let out = text.split(root).join("{ROOT}");
			if (fullOutputPath) out = out.split(fullOutputPath).join("{FULL_OUTPUT}");
			return out;
		};

		const entry: Record<string, unknown> = { name: c.name, ok };
		if (ok) {
			entry.result = JSON.parse(substitute(JSON.stringify(payload)));
		} else {
			entry.error = substitute(payload as string);
		}
		if (c.recordFs) entry.files = walkFiles(root);
		if (fullOutput !== undefined) entry.fullOutput = fullOutput;
		if (fullOutputPath) rmSync(fullOutputPath, { force: true });
		results.push(entry);
	} finally {
		rmSync(root, { recursive: true, force: true });
	}
	}

	console.log(JSON.stringify({ cases: results }, null, "\t"));
	process.exit(0);
}

void main();
"#;
