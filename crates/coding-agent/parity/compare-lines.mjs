// Compares two JSON-lines files line by line, showing a line diff of the first differing
// string field. Usage: node compare-lines.mjs LABEL TS.jsonl RS.jsonl
import { readFileSync } from "node:fs";

const [label, tsPath, rsPath] = process.argv.slice(2);
const read = (p) => readFileSync(p, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
const ts = read(tsPath);
const rs = read(rsPath);
let bad = ts.length === rs.length ? 0 : 1;
for (let i = 0; i < Math.max(ts.length, rs.length); i++) {
	const a = JSON.stringify(ts[i]);
	const b = JSON.stringify(rs[i]);
	if (a === b) continue;
	bad++;
	console.log(`--- ${label} case ${i} differs`);
	for (const key of Object.keys(ts[i] ?? {})) {
		const x = JSON.stringify(ts[i][key], null, 1);
		const y = JSON.stringify(rs[i]?.[key], null, 1);
		if (x === y) continue;
		const xl = (typeof ts[i][key] === "string" ? ts[i][key] : x).split("\n");
		const yl = (typeof rs[i]?.[key] === "string" ? rs[i][key] : (y ?? "")).split("\n");
		for (let j = 0; j < Math.max(xl.length, yl.length); j++) {
			if (xl[j] !== yl[j]) console.log(`  ${key} line ${j + 1}:\n    ts: ${xl[j]}\n    rs: ${yl[j]}`);
		}
	}
}
console.log(bad === 0 ? `${label}: ${ts.length} cases match` : `${label}: ${bad} mismatches`);
process.exit(bad === 0 ? 0 : 1);
