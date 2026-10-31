import fs from "node:fs";
import { pathToFileURL } from "node:url";
const dir = `${process.argv[2]}/src/components`;
const tmp = `${dir}/.markdown-tokens-tmp.ts`;
fs.writeFileSync(tmp, fs.readFileSync(`${dir}/markdown.ts`, "utf8") + "\nexport { markdownParser };\n");
const { markdownParser } = await import(pathToFileURL(tmp).href);
fs.unlinkSync(tmp);
const cases = JSON.parse(fs.readFileSync(new URL("../tests/fixtures/markdown.json", import.meta.url), "utf8")).cases;
const proj = (t) => ({
	type: t.type, raw: t.raw, text: typeof t.text === "string" ? t.text : "",
	depth: t.type === "heading" ? t.depth : 0, lang: t.type === "code" ? (t.lang ?? null) : null,
	href: t.type === "link" || t.type === "image" ? t.href : "", title: t.type === "link" || t.type === "image" ? (t.title || null) : null,
	ordered: !!t.ordered, start: typeof t.start === "number" ? t.start : null, loose: !!t.loose,
	task: !!t.task, checked: typeof t.checked === "boolean" ? t.checked : null, pending: !!t.pending,
	tokens: (t.tokens ?? []).map(proj), items: (t.items ?? []).map(proj),
	header: (t.header ?? []).map((c) => ({ text: c.text, tokens: c.tokens.map(proj) })),
	rows: (t.rows ?? []).map((r) => r.map((c) => ({ text: c.text, tokens: c.tokens.map(proj) }))),
});
console.log(JSON.stringify(cases.map((c) => markdownParser.lexer(c.t.replace(/\t/g, "   ")).map(proj))));
