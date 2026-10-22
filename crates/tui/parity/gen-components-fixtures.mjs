// Golden fixtures for components: node parity/gen-components-fixtures.mjs <ref/packages/tui> > tests/fixtures/components.json
import { pathToFileURL } from "node:url";
const src = (f) => pathToFileURL(`${process.argv[2]}/src/${f}`).href;
const { Text } = await import(src("components/text.ts"));
const { TruncatedText } = await import(src("components/truncated-text.ts"));
const { Spacer } = await import(src("components/spacer.ts"));
const { Loader } = await import(src("components/loader.ts"));

let seed = 777;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return Math.floor(seed / 65536) % n;
};
const E = "\x1b";
const pieces = ["hello", "world", " ", "  ", "\t", "\n", "日本語テキスト", "👍🏽", E + "[31m", E + "[0m", E + "[4m", "foo-bar/baz", "averyveryverylongwordwithoutbreaks", "é", "...", E + "[44m"];
const randText = () => {
	let s = "";
	const n = rnd(10);
	for (let i = 0; i < n; i++) s += pieces[rnd(pieces.length)];
	return s;
};
const bg = (s) => `${E}[48;5;236m${s}${E}[49m`;
const widths = [1, 2, 3, 5, 8, 13, 21, 40, 80];
const out = { text: [], truncated: [], spacer: [], loader: [] };
for (let k = 0; k < 300; k++) {
	const t = randText();
	const px = rnd(4), py = rnd(3), useBg = rnd(3) === 0;
	const c = new Text(t, px, py, useBg ? bg : undefined);
	out.text.push({ t, px, py, bg: useBg, r: widths.map((w) => c.render(w)) });
	const tt = new TruncatedText(t, px, py);
	out.truncated.push({ t, px, py, r: widths.map((w) => tt.render(w)) });
}
for (const n of [0, 1, 3]) out.spacer.push({ n, r: new Spacer(n).render(10) });
const ui = { requestRender() {} };
const spin = (s) => `${E}[36m${s}${E}[39m`;
const dim = (s) => `${E}[2m${s}${E}[22m`;
for (const [msg, ind] of [["Loading...", undefined], ["Working on it, a long message that wraps", undefined], ["Custom", { frames: ["a", "b"] }], ["None", { frames: [] }]]) {
	const l = new Loader(ui, spin, dim, msg, ind);
	const frames = [];
	for (let i = 0; i < 12; i++) {
		frames.push(widths.map((w) => l.render(w)));
		l.currentFrame = (l.currentFrame + 1) % Math.max(1, l.frames.length);
		l.updateDisplay();
	}
	l.stop();
	out.loader.push({ msg, ind: ind ?? null, frames });
}
process.stdout.write(JSON.stringify(out));
