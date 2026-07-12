import { readFileSync } from "node:fs";
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
