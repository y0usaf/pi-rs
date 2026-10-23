// Compares the JSON lines of the TS and Rust tool drivers. Each side's work directory is
// replaced by DIR, and temp file names by TMP, before comparing. For cases marked `unordered`
// (fd and rg walk directories in parallel), the lines before the first blank line are sorted.
// Usage: node compare-tools.mjs TS.jsonl RS.jsonl CASES.json
import { readFileSync } from "node:fs";

const unordered = new Set(
	JSON.parse(readFileSync(process.argv[4], "utf8"))
		.filter((c) => c.unordered)
		.map((c) => c.name),
);
const load = (path) =>
	new Map(
		readFileSync(path, "utf8")
			.split("\n")
			.filter(Boolean)
			.map((line) => {
				const row = JSON.parse(line);
				if (unordered.has(row.name) && row.result.content?.[0]?.type === "text") {
					const [head, ...rest] = row.result.content[0].text.split("\n\n");
					row.result.content[0].text = [head.split("\n").sort().join("\n"), ...rest].join("\n\n");
				}
				const text = JSON.stringify(row.result)
					.replaceAll(row.dir, "DIR")
					.replace(/\/(tmp|var\/folders\/[^"]*?)\/pi-bash-[0-9a-f]{16}\.log/g, "TMP");
				return [row.name, text];
			}),
	);
const [tsPath, rsPath] = process.argv.slice(2);
const ts = load(tsPath);
const rs = load(rsPath);
let failed = 0;
for (const [name, expected] of ts) {
	const actual = rs.get(name);
	if (actual === expected) continue;
	failed++;
	console.log(`--- ${name}\n  ts: ${expected.slice(0, 1500)}\n  rs: ${(actual ?? "(missing)").slice(0, 1500)}`);
}
console.log(`${ts.size - failed}/${ts.size} tool cases match`);
process.exit(failed ? 1 : 0);
