import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
const [tui, out] = process.argv.slice(2);
const dir = join(tui, "test", ".parity-tmp");
mkdirSync(dir, { recursive: true });
const shimSrc = `
import * as k from "../../src/keys.ts";
export * from "../../src/keys.ts";
const env = () => Object.fromEntries(["WT_SESSION", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"].map((n) => [n, process.env[n] ?? null]));
globalThis.__keyCalls ??= [];
const rec = (fn, name) => (...args) => {
	const r = fn(...args);
	globalThis.__keyCalls.push({ fn: name, args, kitty: k.isKittyProtocolActive(), env: env(), r: r ?? null });
	return r;
};
export const matchesKey = rec(k.matchesKey, "matchesKey");
export const parseKey = rec(k.parseKey, "parseKey");
export const decodeKittyPrintable = rec(k.decodeKittyPrintable, "decodeKittyPrintable");
export const decodePrintableKey = rec(k.decodePrintableKey, "decodePrintableKey");
export const isKeyRelease = rec(k.isKeyRelease, "isKeyRelease");
export const isKeyRepeat = rec(k.isKeyRepeat, "isKeyRepeat");
`;
writeFileSync(join(dir, "keys-shim.ts"), shimSrc);
const test = readFileSync(join(tui, "test", "keys.test.ts"), "utf8").replace('"../src/keys.ts"', '"./keys-shim.ts"');
writeFileSync(join(dir, "keys.test.ts"), test);
process.on("exit", () => {
	mkdirSync(dirname(out), { recursive: true });
	writeFileSync(out, JSON.stringify(globalThis.__keyCalls ?? []));
	console.error(`recorded ${(globalThis.__keyCalls ?? []).length} calls`);
});
await import(pathToFileURL(join(dir, "keys.test.ts")).href);
