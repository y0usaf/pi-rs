// Regenerates tests/jsdiff-parity/oracle.json from the vendored jsdiff 8.0.4
// (`ref/pi/node_modules/diff`) — the same library Pi's coding agent uses for
// edit diffs (`edit-diff.ts`) and intra-line diff highlighting (`diff.ts`).
// Run via scripts/jsdiff-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import * as Diff from "../../ref/pi/node_modules/diff/libesm/index.js";

type DiffCase = { name: string; old: string; new: string };
type PatchCase = DiffCase & { oldName: string; newName: string; context: number; headers: string };
type Cases = { lines: DiffCase[]; words: DiffCase[]; patch: PatchCase[] };

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Cases;

function headerOptions(headers: string) {
	switch (headers) {
		case "include":
			return Diff.INCLUDE_HEADERS;
		case "file":
			return Diff.FILE_HEADERS_ONLY;
		case "omit":
			return Diff.OMIT_HEADERS;
		default:
			throw new Error(`unknown headers option: ${headers}`);
	}
}

const oracle = {
	lines: cases.lines.map((c) => ({ name: c.name, changes: Diff.diffLines(c.old, c.new) })),
	words: cases.words.map((c) => ({ name: c.name, changes: Diff.diffWords(c.old, c.new) })),
	patch: cases.patch.map((c) => ({
		name: c.name,
		patch: Diff.createTwoFilesPatch(c.oldName, c.newName, c.old, c.new, undefined, undefined, {
			context: c.context,
			headerOptions: headerOptions(c.headers),
		}),
	})),
};

console.log(JSON.stringify(oracle, null, "\t"));
