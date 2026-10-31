import { mkdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
const { CombinedAutocompleteProvider } = await import(pathToFileURL(`${process.argv[2]}/src/autocomplete.ts`).href);
const outer = process.argv[3];
rmSync(outer, { recursive: true, force: true });
const root = join(outer, "work");
mkdirSync(join(outer, "sibling"), { recursive: true });
writeFileSync(join(outer, "top.txt"), "x");
const files = ["src/main.rs", "src/lib.rs", "src/utils/mod.rs", "src/utils/width.rs", "README.md", "readme-old.txt", "Cargo.toml",
	"docs/guide.md", "docs/API.md", "my file.txt", "日本語/ファイル.md", ".hidden/x", "app/[slug]/page.tsx", "_under.md", "1-first.md", "b.md", "B2.md"];
for (const f of files) {
	mkdirSync(join(root, f, ".."), { recursive: true });
	writeFileSync(join(root, f), "x");
}
mkdirSync(join(root, "emptydir"), { recursive: true });
symlinkSync("docs", join(root, "docs-link"));
const home = join(root, "home");
mkdirSync(join(home, "Dev"), { recursive: true });
writeFileSync(join(home, "notes.txt"), "x");
process.env.HOME = home;
const commands = [
	{ name: "model", description: "Select model", argumentHint: "<name>", getArgumentCompletions: (p) => ["gpt", "claude", "gemini"].filter((x) => x.startsWith(p)).map((x) => ({ value: x, label: x })) },
	{ name: "settings", description: "Open settings" },
	{ name: "skill:review", description: "Review code" },
	{ name: "skill:model-tuning" },
	{ value: "export", label: "export", description: "Export session" },
	{ name: "new", argumentHint: "<title>" },
];
const p = new CombinedAutocompleteProvider(commands, root, "fd");
const texts = ["", "/", "/m", "/mo", "/sk", "/rev", "/zz", "/model ", "/model g", "/model x", "/settings x", "  /se", "see src/", "src/m", "src/utils/", "./", "./src/", "docs/", "Docs/", "~/", "~/D", "~", "@", "@src", "@main", "@src/ut", "@\"my", "\"my f", "@doc", "(src/", "`src/ma", "app/[slug]/pa", "x=src/", "日本語/", "look at ", "text，src/", "readme", "@READ", "b", "@docs-link/", "emptydir/", "\"src/", "say \"hi\" src/", "../"];
const out = [];
const ac = new AbortController();
for (const t of texts) {
	for (const force of [false, true]) {
		const s = await p.getSuggestions([t], 0, t.length, { signal: ac.signal, force });
		const applied = s ? s.items.slice(0, 3).map((it) => p.applyCompletion([t + "\x22tail"], 0, t.length, it, s.prefix)) : [];
		out.push({ t, force, s: s ?? null, applied, trig: p.shouldTriggerFileCompletion([t], 0, t.length) });
	}
}
process.stdout.write(JSON.stringify(out));
