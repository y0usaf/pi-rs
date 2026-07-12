// Pi-derived OpenAI Responses oracle. Run via scripts/openai-responses-oracle.
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
