import { pathToFileURL } from "node:url";
const src = (f) => pathToFileURL(`${process.argv[2]}/src/${f}`).href;
const { TuiMainScreen } = await import(src("tui-main-screen.ts"));
const { CURSOR_MARKER, renderFakeCursor } = await import(src("tui.ts"));
const { setCapabilities } = await import(src("terminal-image.ts"));
setCapabilities({ images: null, trueColor: true, hyperlinks: true });
delete process.env.TERMUX_VERSION;

class RecTerminal {
	constructor() {
		this.cols = 40;
		this.rowsN = 10;
		this.out = "";
	}
	start() {}
	stop() {}
	async drainInput() {}
	write(d) {
		this.out += d;
	}
	get columns() {
		return this.cols;
	}
	get rows() {
		return this.rowsN;
	}
	get kittyProtocolActive() {
		return false;
	}
	moveBy() {}
	hideCursor() {
		this.out += "\x1b[?25l";
	}
	showCursor() {
		this.out += "\x1b[?25h";
	}
	clearLine() {}
	clearFromCursor() {}
	clearScreen() {}
	setTitle() {}
	setProgress() {}
	setProgramStatus() {}
}

class Lines {
	constructor() {
		this.lines = [];
	}
	render() {
		return this.lines;
	}
	invalidate() {}
}

let seed = 4242;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return Math.floor(seed / 65536) % n;
};
const words = ["alpha", "beta", "\x1b[31mred\x1b[0m", "日本語", "x", "\x1b[4munder", "👍", "tab\there", "", "long line of words here"];
const mkLine = (w) => {
	let s = "";
	const n = rnd(5);
	for (let i = 0; i < n; i++) s += (i ? " " : "") + words[rnd(words.length)];
	return s.length > 0 && rnd(10) === 0 ? s + CURSOR_MARKER : s;
};
const truncate = (s, w) => {
	let out = "";
	for (const ch of s) {
		const t = out + ch;
		if (globalThis.vw(t) > w) break;
		out = t;
	}
	return out;
};
const { visibleWidth } = await import(src("utils.ts"));
globalThis.vw = visibleWidth;

const scenarios = [];
for (let k = 0; k < 500; k++) {
	const term = new RecTerminal();
	term.cols = 10 + rnd(40);
	term.rowsN = 3 + rnd(12);
	const hw = rnd(3) === 0;
	const tui = new TuiMainScreen(term, hw);
	tui.setClearOnShrink(rnd(4) === 0);
	const comp = new Lines();
	tui.addChild(comp);
	tui.start();
	term.out = "";
	const frames = [];

	const overlays = [];
	const anchors = ["center", "top-left", "top-right", "bottom-left", "bottom-right", "top-center", "bottom-center", "left-center", "right-center"];
	const size = () => (rnd(3) === 0 ? `${rnd(101)}%` : rnd(60));
	const ovOpts = () => {
		const o = {};
		if (rnd(2)) o.width = size();
		if (rnd(4) === 0) o.minWidth = rnd(30);
		if (rnd(3) === 0) o.maxHeight = size();
		if (rnd(2)) o.anchor = anchors[rnd(anchors.length)];
		if (rnd(4) === 0) o.offsetX = rnd(9) - 4;
		if (rnd(4) === 0) o.offsetY = rnd(9) - 4;
		if (rnd(5) === 0) o.row = size();
		if (rnd(5) === 0) o.col = size();
		if (rnd(4) === 0) o.margin = rnd(2) ? rnd(4) : { top: rnd(3), right: rnd(3), bottom: rnd(3), left: rnd(3) - 1 };
		if (rnd(6) === 0) o.nonCapturing = true;
		return o;
	};
	let lines = [];
	const nFrames = 2 + rnd(8);
	for (let f = 0; f < nFrames; f++) {
		const op = rnd(10);
		if (op === 0) term.cols = Math.max(5, term.cols + rnd(11) - 5);
		else if (op === 1) term.rowsN = Math.max(2, term.rowsN + rnd(7) - 3);
		else if (op <= 3) lines = lines.concat(Array.from({ length: 1 + rnd(term.rowsN * 2) }, () => ""));
		if (op === 4) lines = lines.slice(0, rnd(lines.length + 1));
		if (lines.length === 0 || rnd(2) === 0) lines.push("");
		const edits = rnd(4);
		for (let e = 0; e < edits && lines.length; e++) lines[rnd(lines.length)] = "";
		lines = lines.map((l, i) => (l === "" && rnd(3) ? mkLine() : l));
		if (rnd(8) === 0) {
			const at = rnd(lines.length);
			lines[at] = `pre ${renderFakeCursor("c")} post`;
		}
		lines = lines.map((l) => truncate(l, term.cols));
		comp.lines = lines.slice();
		const ops = [];

		if (rnd(5) === 0) {
			const c = new Lines();
			c.lines = Array.from({ length: 1 + rnd(6) }, () => mkLine());
			const opts = ovOpts();
			const handle = tui.showOverlay(c, opts);
			overlays.push({ c, handle });
			ops.push({ op: "show", lines: c.lines, opts });
		} else if (overlays.length && rnd(4) === 0) {
			const i = rnd(overlays.length);
			const h = overlays[i].handle;
			const kind = rnd(3);
			if (kind === 0) {
				h.hide();
				overlays.splice(i, 1);
			} else if (kind === 1) h.setHidden(!h.isHidden());
			else h.focus();
			ops.push({ op: ["hide", "toggle", "focus"][kind], index: i });
		}
		const force = rnd(15) === 0;
		tui.renderNow(force);
		frames.push({ cols: term.cols, rows: term.rowsN, lines: comp.lines, ops, force, out: term.out });
		term.out = "";
	}
	tui.stop();
	scenarios.push({ hw, clearOnShrink: tui.getClearOnShrink(), frames, stopOut: term.out, fullRedraws: tui.fullRedraws });
}
process.stdout.write(JSON.stringify(scenarios));
