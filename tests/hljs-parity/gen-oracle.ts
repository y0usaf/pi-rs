// Regenerates tests/hljs-parity/oracle.json from the vendored
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
