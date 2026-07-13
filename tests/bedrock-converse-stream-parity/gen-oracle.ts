import { readFileSync } from "node:fs";
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
