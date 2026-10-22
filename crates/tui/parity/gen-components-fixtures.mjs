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

const { SelectList } = await import(src("components/select-list.ts"));
const { fuzzyMatch, fuzzyFilter } = await import(src("fuzzy.ts"));
const th = {
	selectedPrefix: (s) => `<P>${s}`,
	selectedText: (s) => `${E}[1m${s}${E}[22m`,
	description: (s) => `${E}[2m${s}${E}[22m`,
	scrollInfo: (s) => `${E}[90m${s}${E}[39m`,
	noMatch: (s) => `!${s}`,
};
const names = ["model", "settings", "session", "new", "resume", "Model-Picker", "a/b/c.ts", "src/utils.rs", "x1", "abc12", "12abc", "日本", "export_html", "tree", "fork", "compact"];
const descs = [undefined, "", "Pick a model", "multi\nline\r\ndescription that is quite long indeed and keeps going", "短い説明", "\x1b[31mred\x1b[0m desc"];
out.fuzzy = [];
const queries = ["", "m", "mo", "mdl", "set", "s/n", "ses ion", "abc12", "12abc", "a1", "1a", "zz", "src/u", "日", "Tree", " ", "x 1", "e"];
for (const q of queries) {
	out.fuzzy.push({ q, m: names.map((n) => fuzzyMatch(q, n)), f: fuzzyFilter(names, q, (x) => x) });
}
out.select = [];
const keys = ["\x1b[A", "\x1b[B", "\r", "\x1b", "x", "\x1b[B", "\x1b[B"];
for (let k = 0; k < 120; k++) {
	const n = rnd(12);
	const items = Array.from({ length: n }, () => {
		const v = names[rnd(names.length)];
		return { value: v, label: rnd(3) ? "" : v.toUpperCase() + (rnd(2) ? " label" : ""), description: descs[rnd(descs.length)] };
	});
	const maxVisible = 1 + rnd(7);
	const layout = {};
	if (rnd(3) === 0) layout.minPrimaryColumnWidth = rnd(40);
	if (rnd(3) === 0) layout.maxPrimaryColumnWidth = rnd(40);
	const list = new SelectList(items, maxVisible, th, layout);
	const events = [];
	list.onSelect = (i) => events.push(["select", i.value]);
	list.onCancel = () => events.push(["cancel"]);
	list.onSelectionChange = (i) => events.push(["change", i.value]);
	const steps = [];
	for (let s = 0; s < 6; s++) {
		const op = rnd(4);
		let input = null, filter = null;
		if (op === 0) {
			filter = ["", "s", "m", "mo", "zz", "a"][rnd(6)];
			list.setFilter(filter);
		} else {
			input = keys[rnd(keys.length)];
			list.handleInput(input);
		}
		steps.push({ input, filter, r: [20, 41, 60, 100].map((w) => list.render(w)), ev: events.splice(0) });
	}
	out.select.push({ items, maxVisible, layout, steps });
}
process.stdout.write(JSON.stringify(out));
