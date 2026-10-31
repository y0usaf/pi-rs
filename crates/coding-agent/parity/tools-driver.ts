import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const pi = process.env.PI_TS;
if (!pi) throw new Error("PI_TS must point at the upstream checkout");
const { createToolDefinition } = await import(pathToFileURL(join(pi, "packages/coding-agent/src/core/tools/index.ts")).href);
const { validateToolArguments } = await import(pathToFileURL(join(pi, "packages/ai/src/utils/validation.ts")).href);

const [casesPath, root] = process.argv.slice(2);
const cases = JSON.parse(readFileSync(casesPath, "utf8"));
for (const c of cases) {
	const dir = join(root, c.name);
	mkdirSync(dir, { recursive: true });
	for (const [name, text] of Object.entries(c.files ?? {})) {
		mkdirSync(dirname(join(dir, name)), { recursive: true });
		writeFileSync(join(dir, name), text);
	}
	for (const [name, spec] of Object.entries(c.gen ?? {})) {
		const lines = [];
		for (let i = 0; i < spec.lines; i++) lines.push(String(i).padStart(spec.width, "0").replaceAll("0", "x"));
		writeFileSync(join(dir, name), lines.join("\n"));
	}
	for (const [name, data] of Object.entries(c.b64 ?? {})) writeFileSync(join(dir, name), Buffer.from(data, "base64"));
	for (const d of c.mkdir ?? []) mkdirSync(join(dir, d), { recursive: true });

	const def = createToolDefinition(c.tool, dir);
	let result;
	try {
		let args = structuredClone(c.args);
		if (def.prepareArguments) args = def.prepareArguments(args);
		const valid = validateToolArguments(def, { type: "toolCall", id: "", name: c.tool, arguments: args });
		const out = await def.execute("call", valid, undefined, undefined, undefined);
		result = {
			content: out.content.map((b) =>
				b.type === "text" ? { type: "text", text: b.text } : { type: "image", mimeType: b.mimeType, bytes: b.data.length },
			),
			details: out.details ?? null,
			isError: out.isError ?? false,
		};
	} catch (error) {
		result = { threw: error instanceof Error ? error.message : String(error) };
	}
	if (c.show) {
		result.files = Object.fromEntries(c.show.map((p) => {
			let text = "";
			try { text = readFileSync(join(dir, p), "utf8"); } catch {}
			return [p, text];
		}));
	}
	console.log(JSON.stringify({ name: c.name, dir, result }));
}
