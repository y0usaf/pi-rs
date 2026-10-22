// Golden fixtures for utils: node parity/gen-utils-fixtures.mjs <ref/packages/tui> > tests/fixtures/utils.json
import { pathToFileURL } from "node:url";
const u = await import(pathToFileURL(process.argv[2] + "/src/utils.ts").href);
const E = "\x1b";
const pieces = [
	"a", "b", "hello", "world", " ", "  ", "\t", "-", "foo/bar", "x.y",
	"\u65e5\u672c\u8a9e", "\u3053\u3093\u306b\u3061\u306f", "\uff0c", "\u3002", "\ud55c\uae00",
	"\ud83d\udc4d", "\ud83d\udc4d\ud83c\udffd", "\ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67", "\ud83c\uddfa\ud83c\uddf8", "\ud83c\uddfa", "\u2764\ufe0f", "1\ufe0f\u20e3",
	"e\u0301", "\u0e01\u0e33", "\u0e19\u0e49\u0e33", "\u0915\u094d\u0937", "\u0928\u093f", "\uff76\uff9e", "\u200b", "\u00ad",
	E + "[31m", E + "[0m", E + "[1;4m", E + "[24m", E + "[38;5;240m", E + "[48;2;1;2;3m", E + "[39;49m", E + "[2K", E + "[22m",
	E + "]8;;https://x.dev" + E + "\\", E + "]8;;" + E + "\\", E + "]8;id=1;http://a\x07", E + "]8;;\x07",
	E + "_pi:c\x07", E, "\x07",
];
let seed = 12345;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return seed % n;
};
const fixed = [
	"", "hello world", "a\tb", "The quick brown fox jumps over the lazy dog",
	E + "[31mred text that wraps around" + E + "[0m plain",
	E + "[4munderlined long text here" + E + "[24m",
	E + "]8;;https://example.com" + E + "\\link text that is long" + E + "]8;;" + E + "\\",
	"\u65e5\u672c\u8a9e\u306e\u30c6\u30ad\u30b9\u30c8\u3067\u3059\u3002\u6298\u308a\u8fd4\u3057",
	"line1\nline2\r\nline3\rline4", E + "[1mbold\nstill bold" + E + "[0m",
	"averyveryverylongwordwithoutanyspaces", "   leading and trailing   ",
];
const corpus = [...fixed];
for (let k = 0; k < 400; k++) {
	let s = "";
	const n = 1 + rnd(12);
	for (let j = 0; j < n; j++) s += pieces[rnd(pieces.length)];
	corpus.push(s);
}
const cases = [];
for (const s of corpus) {
	const c = { s, vw: u.visibleWidth(s), strip: u.stripTerminalSequences(s), norm: u.normalizeTerminalOutput(s), bg: u.getActiveBackgroundAnsi(s) };
	c.wrap = {};
	c.trunc = {};
	c.slice = [];
	c.seg = [];
	c.cell = [];
	for (const w of [1, 2, 3, 5, 8, 13, 40]) {
		c.wrap[w] = u.wrapTextWithAnsi(s, w);
		c.trunc[w] = [u.truncateToWidth(s, w), u.truncateToWidth(s, w, "\u2026", true), u.truncateToWidth(s, w, "", false)];
	}
	for (const [a, l, strict] of [[0, 3, false], [1, 4, true], [2, 5, false], [3, 2, true], [0, 50, false]]) {
		c.slice.push([a, l, strict, u.sliceWithWidth(s, a, l, strict)]);
	}
	for (const [b, a, l, strict] of [[2, 4, 3, false], [3, 3, 4, true], [1, 5, 0, false], [4, 6, 5, true]]) {
		c.seg.push([b, a, l, strict, u.extractSegments(s, b, a, l, strict)]);
	}
	for (const col of [0, 1, 2, 3, 5]) {
		c.cell.push([col, u.getGraphemeCellRange(s, col) ?? null, u.getOsc8LinkAtColumn(s, col) ?? null]);
	}
	cases.push(c);
}
process.stdout.write(JSON.stringify(cases));
