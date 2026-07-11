// Regenerates tests/anthropic-parity/oracle.json by running Pi's real
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
