import { pathToFileURL } from "node:url";
const src = (f) => pathToFileURL(`${process.argv[2]}/src/${f}`).href;
const { Input } = await import(src("components/input.ts"));
const wordSeg = new Intl.Segmenter(undefined, { granularity: "word" });
let seed = 31337;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return Math.floor(seed / 65536) % n;
};
const E = "\x1b";
const keys = [
	"a", "b", " ", "x", "hello", "日", "本", "👍🏽", "é", "-", ".", "/", "\t",
	E + "[D", E + "[C", E + "[H", E + "[F", "\x01", "\x05", "\x02", "\x06", E + "b", E + "f", E + "[1;5D", E + "[1;5C",
	"\x7f", E + "[3~", "\x04", "\x17", E + "\x7f", E + "d", "\x15", "\x0b", "\x19", E + "y", "\x1f",
	E + "[97u", E + "[65;2u", E + "[200~pasted\ntext\twith tab" + E + "[201~", E + "[200~part", "ial" + E + "[201~z",
	"\r", "\n", E, "\x03", "\x00", "\x1b[27;5;106~",
];
const cases = [];
for (let k = 0; k < 400; k++) {
	const opts = {};
	if (rnd(3) === 0) opts.prompt = ["", "$ ", "»» ", "prompt> "][rnd(4)];
	if (rnd(3) === 0) opts.placeholder = ["Type here", "日本語のプレースホルダー", "p"][rnd(3)];
	if (opts.placeholder && rnd(2)) opts.placeholderStyle = (s) => `${E}[2m${s}${E}[22m`;
	const input = new Input(opts);
	input.focused = rnd(4) !== 0;
	const events = [];
	input.onSubmit = (v) => events.push(["submit", v]);
	input.onEscape = () => events.push(["escape"]);
	const steps = [];
	const n = 1 + rnd(25);
	for (let s = 0; s < n; s++) {
		const d = keys[rnd(keys.length)];
		const segs = [...wordSeg.segment(input.getValue())].map((x) => [x.index, x.segment, !!x.isWordLike]);
		input.handleInput(d);
		const w = 1 + rnd(30);
		steps.push({ segs, d, w, r: input.render(w), v: input.getValue(), c: input.cursor, ev: events.splice(0) });
	}
	cases.push({ opts: { prompt: opts.prompt ?? null, placeholder: opts.placeholder ?? null, styled: !!opts.placeholderStyle }, focused: input.focused, steps });
}
process.stdout.write(JSON.stringify(cases));
