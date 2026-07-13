import { readFileSync } from "node:fs";
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
