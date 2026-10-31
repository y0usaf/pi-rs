import { pathToFileURL } from 'node:url';
const src = (f) => pathToFileURL(`${process.argv[2]}/src/${f}`).href;
const { Editor } = await import(src('components/editor.ts'));
const wordSeg = new Intl.Segmenter(undefined, { granularity: 'word' });
let seed = 4242;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return Math.floor(seed / 65536) % n;
};
const E = '\x1b';
const COMMANDS = ['help', 'model', 'mode', 'clear', 'compact', 'settings'];
const WORDS = ['apple', 'apricot', 'banana', '日本語', 'cherry', 'src/main.rs', 'src/lib.rs'];
const provider = {
	triggerCharacters: ['$'],
	async getSuggestions(lines, cl, cc, { force }) {
		const before = (lines[cl] || '').slice(0, cc);
		const slash = /^\/(\w*)$/.exec(before);
		if (slash) {
			const items = COMMANDS.filter((c) => c.startsWith(slash[1])).map((c) => ({ value: c, label: c, description: `run ${c}` }));
			return items.length ? { items, prefix: before } : null;
		}
		const tok = /(?:^|\s)([@\x23$]?)(\S*)$/.exec(before);
		if (!tok || (!tok[1] && !force)) return null;
		const items = WORDS.filter((w) => w.startsWith(tok[2])).map((w) => ({ value: tok[1] + w, label: w }));
		return items.length ? { items, prefix: tok[1] + tok[2] } : null;
	},
	applyCompletion(lines, cl, cc, item, prefix) {
		const line = lines[cl] || '';
		const start = Math.max(0, cc - prefix.length);
		const value = prefix.startsWith('/') ? `/${item.value} ` : item.value;
		const out = [...lines];
		out[cl] = line.slice(0, start) + value + line.slice(cc);
		return { lines: out, cursorLine: cl, cursorCol: start + value.length };
	},
	shouldTriggerFileCompletion(lines, cl, cc) {
		const b = (lines[cl] || '').slice(0, cc).trim();
		return !(b.startsWith('/') && !b.includes(' '));
	},
};
const big = Array.from({ length: 12 }, (_, i) => `line ${i}`).join('\n');
const keys = [
	'a', 'b', ' ', 'x', 'hello', '日', '本', '👍🏽', 'é', '-', '.', '/', '@', '\x23', '$', '\\', '\t', 's', 'r', 'c', 'ap',
	'the quick brown fox jumps over', '長い日本語の文章です', 'word ',
	E + '[D', E + '[C', E + '[A', E + '[B', E + '[H', E + '[F', E + '[5~', E + '[6~', '\x01', '\x05', '\x02', '\x06',
	E + 'b', E + 'f', E + '[1;5D', E + '[1;5C',
	'\x7f', E + '[3~', '\x04', '\x17', E + '\x7f', E + 'd', '\x15', '\x0b', '\x19', E + 'y', '\x1f',
	'\x1d', E + '\x1d', E + '[97u', E + '[65;2u', E + '[13;2u', '\r', '\r', '\n', E + '\r', E, '\x03',
	E + '[200~pasted\ntext\twith tab' + E + '[201~', E + '[200~' + big + E + '[201~', E + '[200~/tmp/x' + E + '[201~',
	E + '[200~part', 'ial' + E + '[201~z',
];
const tick = () => new Promise((r) => setTimeout(r, 0));
const cases = [];
for (let k = 0; k < 300; k++) {
	const rows = [10, 24, 40][rnd(3)];
	const tui = { terminal: { rows }, requestRender() {} };
	const theme = { borderColor: (s) => `<B>${s}</B>`, selectList: {
		selectedPrefix: (s) => `<P>${s}`, selectedText: (s) => `${E}[1m${s}${E}[22m`, description: (s) => `${E}[2m${s}${E}[22m`,
		scrollInfo: (s) => `${E}[90m${s}${E}[39m`, noMatch: (s) => `!${s}` } };
	const opts = { paddingX: rnd(3), autocompleteMaxVisible: 3 + rnd(4) };
	const ed = new Editor(tui, theme, opts);
	ed.focused = rnd(4) !== 0;
	const useProvider = rnd(4) !== 0;
	if (useProvider) ed.setAutocompleteProvider(provider);
	const history = rnd(2) ? ['first prompt', 'second\nmultiline prompt'] : [];
	for (const h of history) ed.addToHistory(h);
	const events = [];
	ed.onSubmit = (v) => { events.push(['submit', v]); ed.addToHistory(v); };
	ed.onChange = (v) => events.push(['change', v]);
	const steps = [];
	const n = 1 + rnd(30);
	let w = 8 + rnd(30);
	for (let s = 0; s < n; s++) {
		const d = keys[rnd(keys.length)];
		if (rnd(6) === 0) w = 8 + rnd(30);
		const segs = ed.getLines().map((l) => [...wordSeg.segment(l)].map((x) => [x.index, x.segment, !!x.isWordLike]));
		ed.handleInput(d);
		await tick();
		if (ed.autocompleteDebounceTimer) await new Promise((r) => setTimeout(r, 30));
		await tick();
		const r = ed.render(w);
		const c = ed.getCursor();
		steps.push({ d, w, segs, r, lines: ed.getLines(), cl: c.line, cc: c.col, ev: events.splice(0), ac: ed.isShowingAutocomplete() });
	}
	cases.push({ rows, opts, focused: ed.focused, provider: useProvider, history, steps });
}
process.stdout.write(JSON.stringify(cases));
