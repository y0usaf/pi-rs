// Golden fixtures for keys: node parity/gen-keys-fixtures.mjs <ref/packages/tui> > tests/fixtures/keys.json
import { pathToFileURL } from "node:url";
delete process.env.WT_SESSION;
const k = await import(pathToFileURL(process.argv[2] + "/src/keys.ts").href);
const E = "\x1b";
const inputs = new Set();
for (let c = 0; c < 128; c++) {
	inputs.add(String.fromCharCode(c));
	inputs.add(E + String.fromCharCode(c));
}
for (const s of ["A", "B", "C", "D", "H", "F", "E", "Z", "a", "b", "c", "d", "e", "1~", "2~", "3~", "4~", "5~", "6~", "7~", "8~",
	"2$", "3$", "5$", "6$", "7$", "8$", "2^", "3^", "5^", "6^", "7^", "8^", "11~", "12~", "13~", "14~", "15~", "17~", "18~",
	"19~", "20~", "21~", "23~", "24~", "[A", "[B", "[C", "[D", "[E", "[5~", "[6~", "200~"]) inputs.add(E + "[" + s);
for (const s of ["A", "B", "C", "D", "H", "F", "E", "M", "P", "Q", "R", "S", "a", "b", "c", "d", "e"]) inputs.add(E + "O" + s);
for (const m of [1, 2, 3, 4, 5, 6, 7, 8, 9, 13, 17, 65, 129, 193]) {
	for (const f of ["A", "B", "C", "D", "H", "F"]) {
		inputs.add(`${E}[1;${m}${f}`);
		inputs.add(`${E}[1;${m}:3${f}`);
		inputs.add(`${E}[1;${m}:2${f}`);
	}
	for (const n of [2, 3, 5, 6, 7, 8, 9]) {
		inputs.add(`${E}[${n};${m}~`);
		inputs.add(`${E}[${n};${m}:3~`);
	}
	for (const cp of [9, 13, 27, 32, 127, 97, 99, 65, 122, 48, 57, 61, 43, 45, 47, 91, 93, 92, 59, 1089, 1057, 57399, 57409, 57414, 57417, 57426, 57441, 233]) {
		inputs.add(`${E}[${cp};${m}u`);
		inputs.add(`${E}[${cp};${m}:3u`);
		inputs.add(`${E}[27;${m};${cp}~`);
	}
}
for (const s of ["61:43;2u", "61:43;6u", "1089::99;5u", "1089::99;1u", "1057:1089:99;5u", "97:65;2u", "47::91;5u", "107::118;5u",
	"49:33;2u", "39:34;2u", "99u", "99;1u", "97;1:1u", "13u", "57414u", "27u", "9;2u", "1u", "128512u", "55296u",
	"97::;5u", "97:;5u", ":97u", "97;;5u", "0u", "31u", "99;3u", "99;9u", "99;65u"]) inputs.add(E + "[" + s);
for (const s of ["", "ab", "\u00e9", "\u65e5", "\x1b\r", "\x1b ", "\x1b\x7f", "\x1b\b", "\x1b\x1b", "\x1b\x1c", "\x1b\x1d", "\x1b\x1f",
	"\x1bp", "\x1bn", E + "[200~x:3u", "abc:3u"]) inputs.add(s);
const ids = [];
const bases = ["escape", "esc", "enter", "return", "tab", "space", "backspace", "delete", "insert", "clear", "home", "end",
	"pageUp", "pageDown", "up", "down", "left", "right", "f1", "f5", "f12", "a", "c", "h", "k", "z", "0", "5", "9",
	"-", "=", "[", "]", "\\", "/", ";", "'", "+", "!", "_", "|", "~", "{", "<", "?"];
for (const b of bases) for (const m of ["", "ctrl+", "shift+", "alt+", "super+", "ctrl+shift+", "shift+ctrl+", "ctrl+alt+", "alt+shift+", "ctrl+super+", "ctrl+shift+alt+"]) ids.push(m + b);
ids.push("ctrl+ctrl+a", "Ctrl+A", "ctrl+", "a+b", "ctrl++", "+", "bogus");
const out = { ids, modes: {} };
for (const kitty of [false, true]) {
	k.setKittyProtocolActive(kitty);
	const rows = [];
	for (const d of inputs) {
		const matched = [];
		ids.forEach((id, i) => {
			if (k.matchesKey(d, id)) matched.push(i);
		});
		rows.push({ d, parse: k.parseKey(d) ?? null, kp: k.decodeKittyPrintable(d) ?? null, pk: k.decodePrintableKey(d) ?? null,
			rel: k.isKeyRelease(d), rep: k.isKeyRepeat(d), m: matched });
	}
	out.modes[kitty ? "kitty" : "legacy"] = rows;
}
process.stdout.write(JSON.stringify(out));
