// Golden fixtures for StdinBuffer: random chunkings of terminal input with a fake clock.
// node parity/gen-stdin-fixtures.mjs <ref/packages/tui> > tests/fixtures/stdin.json
import { pathToFileURL } from "node:url";
let now = 0;
let timers = [];
let nextId = 1;
globalThis.setTimeout = (fn, ms) => {
	const t = { id: nextId++, at: now + ms, fn };
	timers.push(t);
	return t;
};
globalThis.clearTimeout = (t) => {
	timers = timers.filter((x) => x !== t);
};
const { StdinBuffer } = await import(pathToFileURL(process.argv[2] + "/src/stdin-buffer.ts").href);
const E = "\x1b";
const atoms = [
	"a", "b", "Z", " ", "1", "\r", "\n", "\t", "\x7f", "\x03", "\u00e9", "\u4e16",

	E, E + "[A", E + "[1;5C", E + "OP", E + "a", E + "]", E + E, E + "[<35;20;5m", E + "[<0;1;1M", E + "[M !!",

	E + "[97u", E + "[97;1u", E + "[99;5u", E + "[27;1:3u", E + E + "[27;1:3u", E + "[3~", E + "[200~hello\nworld" + E + "[201~",
	E + "[200~" + E + "[A paste" + E + "[201~", E + "]11;rgb:0000/0000/0000" + E + "\\", E + "]10;rgb:ffff/ffff/ffff\x07",

	E + "]4;12;rgb:1/2/3\x07", E + "P>|kitty(0.40)" + E + "\\", E + "_Gi=31;OK" + E + "\\", E + "]7501;?1\x07",

	E + "]8;;http:\/\/x\x07", E + "]11;rgb:00",
	"\x18", "\x1a", E + "_Gi=1,p=2;", E + "[?62;4c", E + "[I", E + "[O",
];
let seed = 99;
const rnd = (n) => {
	seed = (seed * 1103515245 + 12345) % 2147483648;
	return Math.floor(seed / 65536) % n;
};
const scripts = [];
for (let k = 0; k < 600; k++) {
	let text = "";
	const n = 1 + rnd(6);
	for (let j = 0; j < n; j++) text += atoms[rnd(atoms.length)];
	const steps = [];
	let i = 0;
	while (i < text.length) {
		let len = 1 + rnd(8);
		const lastUnit = text.charCodeAt(i + len - 1);
		if (i + len < text.length && lastUnit >= 0xd800 && lastUnit < 0xdc00) len++;
		steps.push({ feed: text.slice(i, i + len), wait: rnd(4) === 0 ? rnd(70) : 0 });
		i += len;
	}
	steps.push({ feed: null, wait: 100 });
	scripts.push(steps);
}
const out = [];
for (const steps of scripts) {
	now = 0;
	timers = [];
	const buf = new StdinBuffer({ timeout: 50, escapeTimeout: 10 });
	const events = [];
	buf.on("data", (d) => events.push(["data", d, now]));
	buf.on("paste", (d) => events.push(["paste", d, now]));
	for (const s of steps) {
		if (s.feed !== null) buf.process(s.feed);
		const until = now + s.wait;
		for (;;) {
			timers.sort((a, b) => a.at - b.at || a.id - b.id);
			const t = timers[0];
			if (!t || t.at > until) break;
			timers.shift();
			now = t.at;
			t.fn();
		}
		now = until;
	}
	out.push({ steps, events, rest: buf.getBuffer() });
}
process.stdout.write(JSON.stringify(out));
