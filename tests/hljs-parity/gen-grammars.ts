// Serializes compiled highlight.js 10.7.3 grammars (the vendored library
// Pi's coding agent uses for syntax highlighting) into the data catalog
// embedded by pi-rs's Rust engine (crates/pi-rs-host/data/hljs-grammars.json).
//
// The grammars are compiled by the vendored library itself (compileLanguage
// runs here, at generation time); the Rust side interprets the compiled
// mode graph 1:1 (grammar-as-data, per the models.json precedent). Runtime
// callbacks that survive compilation are mapped to named builtins the Rust
// engine implements; an unknown callback is a hard error so the boundary
// stays explicit.
//
// Scope: the languages reachable from Pi's coding agent presentation layer —
// theme.ts getLanguageFromPath extensions plus the fenced-code tags Pi's own
// tests pin (`diff`, `html`) — closed over subLanguage references. Everything
// outside the set falls back to Pi's unvalidated-language path (recorded in
// PLAN.md 2b.3).
//
// Run via scripts/hljs-grammars. Do not edit the output by hand.
import hljs from "../../ref/pi/node_modules/highlight.js/lib/index.js";

// theme.ts getLanguageFromPath target languages (values of extToLang),
// plus fence tags exercised by Pi's own syntax-highlight tests.
const WANTED = [
	"typescript",
	"javascript",
	"python",
	"ruby",
	"rust",
	"go",
	"java",
	"kotlin",
	"swift",
	"c",
	"cpp",
	"csharp",
	"php",
	"bash",
	"fish",
	"powershell",
	"sql",
	"html",
	"css",
	"scss",
	"sass",
	"less",
	"json",
	"yaml",
	"toml",
	"xml",
	"markdown",
	"dockerfile",
	"makefile",
	"cmake",
	"lua",
	"perl",
	"r",
	"scala",
	"clojure",
	"elixir",
	"erlang",
	"haskell",
	"ocaml",
	"vim",
	"graphql",
	"protobuf",
	"hcl",
	"diff",
];

type Json = unknown;

function source(re: unknown): string | null {
	if (!re) return null;
	if (typeof re === "string") return re;
	return (re as RegExp).source;
}

// core.js countMatchGroups, applied to the rule's raw source.
function countMatchGroups(re: string): number {
	return (new RegExp(re.toString() + "|").exec("") as RegExpExecArray).length - 1;
}

function callbackName(fn: (...args: unknown[]) => unknown, language: string, kind: string): string {
	const src = fn.toString();
	if (src.includes("_beginMatch")) {
		return kind === "on:begin" ? "end-same-as-begin:begin" : "end-same-as-begin:end";
	}
	if (src.includes("index !== 0")) return "shebang";
	if (src.includes("afterMatchIndex")) return "is-truly-opening-tag";
	throw new Error(`unknown ${kind} callback in language '${language}': ${src}`);
}

function serializeLanguage(name: string) {
	// Force compilation; the library compiles the registered definition in
	// place (compileLanguage via _highlight).
	hljs.highlight("", { language: name, ignoreIllegals: true });
	const lang = (hljs as any).getLanguage(name);
	if (!lang || !lang.isCompiled) throw new Error(`language '${name}' did not compile`);

	const ids = new Map<object, number>();
	const modes: Json[] = [];

	function visit(mode: any): number {
		const known = ids.get(mode);
		if (known !== undefined) return known;
		const id = modes.length;
		ids.set(mode, id);
		modes.push(null);

		const keywords = mode.keywords
			? Object.fromEntries(
					Object.keys(mode.keywords).map((word) => {
						const [className, relevance] = mode.keywords[word];
						return [word, [className, relevance]];
					}),
				)
			: null;

		const rules = (mode.matcher?.rules ?? []).map(([re, opts]: [unknown, any]) => ({
			type: opts.type,
			re: source(re),
			groups: countMatchGroups(source(re) ?? ""),
			mode: opts.rule ? visit(opts.rule) : null,
		}));

		modes[id] = {
			className: mode.className ?? null,
			keywords,
			keywordPattern: mode.keywordPatternRe ? mode.keywordPatternRe.source : null,
			beginRe: mode.beginRe ? mode.beginRe.source : null,
			endRe: mode.endRe ? mode.endRe.source : null,
			skip: !!mode.skip,
			excludeBegin: !!mode.excludeBegin,
			excludeEnd: !!mode.excludeEnd,
			returnBegin: !!mode.returnBegin,
			returnEnd: !!mode.returnEnd,
			endsWithParent: !!mode.endsWithParent,
			endsParent: !!mode.endsParent,
			endSameAsBegin: !!mode.endSameAsBegin,
			subLanguage: mode.subLanguage ?? null,
			relevance: typeof mode.relevance === "number" ? mode.relevance : 1,
			beforeBegin: mode.__beforeBegin ? "skip-if-preceding-dot" : null,
			onBegin: mode["on:begin"] ? callbackName(mode["on:begin"], name, "on:begin") : null,
			onEnd: mode["on:end"] ? callbackName(mode["on:end"], name, "on:end") : null,
			starts: mode.starts ? visit(mode.starts) : null,
			rules,
		};
		return id;
	}

	const root = visit(lang);
	return {
		name,
		caseInsensitive: !!lang.case_insensitive,
		aliases: lang.aliases ?? [],
		classNameAliases: Object.fromEntries(
			Object.keys(lang.classNameAliases ?? {}).map((k) => [k, lang.classNameAliases[k]]),
		),
		supersetOf: lang.supersetOf ?? null,
		disableAutodetect: !!lang.disableAutodetect,
		root,
		modes,
	};
}

// Resolve wanted names through the registry (aliases like html→xml,
// toml→ini) and drop the ones the vendored library does not ship — Pi's
// supportsLanguage is false for those too, so both sides take the
// mdCodeBlock fallback.
const canonical = new Set<string>();
for (const wanted of WANTED) {
	const lang = (hljs as any).getLanguage(wanted);
	if (!lang) continue;
	// Find the registered key for this definition.
	const key = hljs.listLanguages().find((name) => (hljs as any).getLanguage(name) === lang);
	if (!key) throw new Error(`no registry key for '${wanted}'`);
	canonical.add(key);
}

// Serialize in registration order (relevance ties in highlightAuto resolve
// by original ordering), closing over subLanguage references.
const serialized = new Map<string, ReturnType<typeof serializeLanguage>>();
let changed = true;
while (changed) {
	changed = false;
	for (const name of hljs.listLanguages()) {
		if (!canonical.has(name) || serialized.has(name)) continue;
		const entry = serializeLanguage(name);
		serialized.set(name, entry);
		changed = true;
		for (const mode of entry.modes as any[]) {
			const sub = mode.subLanguage;
			const subNames = typeof sub === "string" ? [sub] : Array.isArray(sub) ? sub : [];
			for (const subName of subNames) {
				const subLang = (hljs as any).getLanguage(subName);
				if (!subLang) continue;
				const key = hljs.listLanguages().find((n) => (hljs as any).getLanguage(n) === subLang);
				if (key && !canonical.has(key)) canonical.add(key);
			}
		}
	}
}

const languages = hljs
	.listLanguages()
	.filter((name) => serialized.has(name))
	.map((name) => serialized.get(name));

process.stdout.write(JSON.stringify({ hljs: "10.7.3", languages }));
