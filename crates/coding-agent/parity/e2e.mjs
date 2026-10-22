// Runs each case of e2e.json through upstream `pi` and `pi-rs` against a scripted
// OpenAI-compatible server, then compares the requests (minus volatile fields) and the output.
// Usage: node e2e.mjs CASES OUTDIR PI_RS_BIN   (PI_TS names the upstream checkout)
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join } from "node:path";

const [casesPath, root, rsBin] = process.argv.slice(2);
const pi = process.env.PI_TS;
const cases = JSON.parse(readFileSync(casesPath, "utf8"));

function frame(choices, extra = {}) {
	return `data: ${JSON.stringify({ id: "chatcmpl-1", object: "chat.completion.chunk", created: 0, model: "mock-1", choices, ...extra })}\n\n`;
}
const chunk = (delta, finish) => frame([{ index: 0, delta, finish_reason: finish ?? null }]);

/** A server answering request n with script[n]; resolves to { port, log, close }. */
function serve(script) {
	return new Promise((resolve) => {
		const log = [];
		const server = createServer(async (req, res) => {
			const parts = [];
			for await (const part of req) parts.push(part);
			log.push(JSON.parse(Buffer.concat(parts).toString("utf8")));
			const step = script[log.length - 1] ?? { text: "(script exhausted)" };
			if (step.status) {
				res.writeHead(step.status, { "content-type": "application/json" });
				res.end(JSON.stringify({ error: { message: step.error ?? "failure" } }));
				return;
			}
			res.writeHead(200, { "content-type": "text/event-stream" });
			res.write(chunk({ role: "assistant", content: "" }));
			if (step.text !== undefined) {
				res.write(chunk({ content: step.text }));
				res.write(chunk({}, step.finish ?? "stop"));
			} else {
				const calls = Array.isArray(step.tool) ? step.tool : [step.tool];
				calls.forEach((call, index) =>
					res.write(
						chunk({
							tool_calls: [
								{ index, id: `call_${log.length}_${index}`, type: "function", function: { name: call.name, arguments: typeof call.arguments === "string" ? call.arguments : JSON.stringify(call.arguments) } },
							],
						}),
					),
				);
				res.write(chunk({}, "tool_calls"));
			}
			res.write(frame([], { usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } }));
			res.write("data: [DONE]\n\n");
			res.end();
		});
		server.listen(0, "127.0.0.1", () => resolve({ port: server.address().port, log, close: () => server.close() }));
	});
}

function setup(dir, c, port) {
	const home = join(dir, "home");
	const agent = join(home, ".pi/agent");
	const proj = join(home, "proj");
	mkdirSync(agent, { recursive: true });
	mkdirSync(proj, { recursive: true });
	for (const [name, text] of Object.entries(c.files ?? {})) {
		const path = join(dir, name);
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, text);
	}
	const model = { id: "mock-1", name: "Mock One", contextWindow: 100000, maxTokens: 4096, ...(c.model ?? {}) };
	const models = { providers: { mock: { baseUrl: `http://127.0.0.1:${port}/v1`, api: "openai-completions", apiKey: "sk-mock", models: [model] } } };
	writeFileSync(join(agent, "models.json"), JSON.stringify(models, null, 2));
	if (c.settings) writeFileSync(join(agent, "settings.json"), JSON.stringify(c.settings));
	return { home, agent, proj };
}

function scrub(text, dir) {
	return text.replaceAll(dir, "<dir>");
}

/** The request without what legitimately differs between two runs. */
function normalize(body, dir) {
	const copy = JSON.parse(scrub(JSON.stringify(body), dir));
	delete copy.prompt_cache_key;
	delete copy.metadata;
	// Known pi-ai gap (crates/ai/parity/known.json): Rust sends `strict: false` on
	// openai-completions tools where TS leaves it out for unknown providers.
	for (const tool of copy.tools ?? []) delete tool.function?.strict;
	return copy;
}

async function runOne(kind, c) {
	const dir = join(root, c.name, kind);
	const server = await serve(c.script);
	const { home, agent, proj } = setup(dir, c, server.port);
	const env = { ...process.env, HOME: home, PI_CODING_AGENT_DIR: agent, PI_OFFLINE: "1", PI_PACKAGE_DIR: "/pkg" };
	delete env.PI_SESSION_ID;
	const args = ["-p", "--model", "mock/mock-1", ...(c.args ?? [])];
	const cmd = kind === "ts" ? process.execPath : rsBin;
	const argv =
		kind === "ts"
			? ["--import", join(pi, "packages/coding-agent/src/experimental/source-resolver.ts"), join(pi, "packages/coding-agent/src/cli.ts"), ...args]
			: args;
	const result = await new Promise((resolve) => {
		const child = spawn(cmd, argv, { cwd: proj, env });
		let stdout = "";
		let stderr = "";
		child.stdout.setEncoding("utf8").on("data", (d) => (stdout += d));
		child.stderr.setEncoding("utf8").on("data", (d) => (stderr += d));
		const timer = setTimeout(() => child.kill("SIGKILL"), 60000);
		child.on("close", (status) => {
			clearTimeout(timer);
			resolve({ status, stdout, stderr });
		});
		child.stdin.end(c.stdin ?? "");
	});
	server.close();
	return {
		code: result.status,
		stdout: scrub(result.stdout ?? "", dir),
		stderr: scrub(result.stderr ?? "", dir),
		requests: server.log.map((b) => normalize(b, dir)),
	};
}

/**
 * Known differences that come from layers below this crate, which it does not change. Each is
 * applied to pi-rs's side so the rest of the case still compares, and reported when used.
 */
const KNOWN = {
	harnessToolErrors:
		"pi-durable's harness reports invalid arguments and unknown tools itself, wrapped as `<harness>\\n[error] ...\\n</harness>`, and names an unknown tool `Tool X is not available` (crates/durable/src/harness/tool.rs, harness_error); upstream's agent loop sends the bare message and `Tool X not found`",
};

function applyKnown(run, used) {
	for (const request of run.requests) {
		for (const message of request.messages ?? []) {
			if (message.role !== "tool" || typeof message.content !== "string") continue;
			const m = /^<harness>\n\[error\] ([\s\S]*)\n<\/harness>$/.exec(message.content);
			if (!m) continue;
			used.add("harnessToolErrors");
			message.content = m[1].replace(/^Tool (.+) is not available$/, "Tool $1 not found");
		}
	}
}

let bad = 0;
const used = new Set();
for (const c of cases) {
	const ts = await runOne("ts", c);
	const rs = await runOne("rs", c);
	applyKnown(rs, used);
	const problems = [];
	for (const key of ["code", "stdout", "stderr"]) {
		if (c.known?.[key]) {
			if (JSON.stringify(ts[key]) !== JSON.stringify(rs[key])) used.add(`${c.name}.${key}: ${c.known[key]}`);
			continue;
		}
		if (JSON.stringify(ts[key]) !== JSON.stringify(rs[key])) problems.push(`${key}:\n    ts: ${JSON.stringify(ts[key])}\n    rs: ${JSON.stringify(rs[key])}`);
	}
	if (ts.requests.length !== rs.requests.length) problems.push(`requests: ts sent ${ts.requests.length}, rs ${rs.requests.length}`);
	ts.requests.forEach((tq, i) => {
		const rq = rs.requests[i];
		if (!rq) return;
		const keys = new Set([...Object.keys(tq), ...Object.keys(rq)]);
		for (const key of keys) {
			const a = JSON.stringify(tq[key], null, 1);
			const b = JSON.stringify(rq[key], null, 1);
			if (a === b) continue;
			const al = (a ?? "undefined").split("\n");
			const bl = (b ?? "undefined").split("\n");
			const lines = [];
			for (let j = 0; j < Math.max(al.length, bl.length) && lines.length < 12; j++) {
				if (al[j] !== bl[j]) lines.push(`      ts: ${al[j]}\n      rs: ${bl[j]}`);
			}
			problems.push(`request ${i} ${key}:\n${lines.join("\n")}`);
		}
	});
	if (problems.length > 0) {
		bad++;
		console.log(`--- ${c.name}\n  ${problems.join("\n  ")}`);
	}
	if (process.env.E2E_KEEP) writeFileSync(join(root, `${c.name}.json`), JSON.stringify({ ts, rs }, null, 1));
}
for (const key of used) console.log(`known difference: ${KNOWN[key] ?? key}`);
console.log(bad === 0 ? `e2e: ${cases.length} cases match` : `e2e: ${bad} of ${cases.length} cases differ`);
process.exit(bad === 0 ? 0 : 1);
