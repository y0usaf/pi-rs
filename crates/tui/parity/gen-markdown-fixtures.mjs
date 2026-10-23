// Golden fixtures for Markdown: node parity/gen-markdown-fixtures.mjs <ref/packages/tui> > tests/fixtures/markdown.json
// Needs marked@18.0.11, marked-cjk-friendly@0.1.2 and get-east-asian-width@1.6.0 resolvable from the ref tree.
import { pathToFileURL } from "node:url";
const src = (f) => pathToFileURL(`${process.argv[2]}/src/${f}`).href;
const { Markdown } = await import(src("components/markdown.ts"));
const { setCapabilities } = await import(src("terminal-image.ts"));

const E = "\x1b";
// chalk-like styles: reopen after an inner close, close/reopen around newlines.
const sty = (open, close) => (s) => {
	let t = s.split(`${E}[${close}m`).join(`${E}[${close}m${E}[${open}m`);
	t = t.split("\n").join(`${E}[${close}m\n${E}[${open}m`);
	return `${E}[${open}m${t}${E}[${close}m`;
};
const bold = sty(1, 22), dim = sty(2, 22), italic = sty(3, 23), underline = sty(4, 24), strike = sty(9, 29);
const cyan = sty(36, 39), blue = sty(34, 39), yellow = sty(33, 39), green = sty(32, 39), gray = sty(90, 39);
const theme = {
	heading: (s) => bold(cyan(s)), link: blue, linkUrl: dim, code: yellow, codeBlock: green, codeBlockBorder: dim,
	quote: italic, quoteBorder: dim, hr: dim, listBullet: cyan, bold, italic, strikethrough: strike, underline,
};
const themeIndent = { ...theme, codeBlockIndent: "> " };

const corpus = [
	"source", "1. alpha\n1. beta\n1. gamma", "  4. forth\n  3. third\n\n10) ten\n7) seven\n\n+ plus\n* star\n- minus\n+",
	"- [ ] beep\n- [x] boop", "- alpha beta gamma delta epsilon", "1. alpha beta gamma delta epsilon", "10. alpha beta gamma delta epsilon",
	"- parent\n  - alpha beta gamma delta epsilon", "1. parent\n   - alpha beta gamma delta epsilon", "- > alpha beta gamma delta epsilon zeta",
	"- ```ts\n  alpha beta gamma delta epsilon zeta\n  ```", "- Item 1\n  - Nested 1.1\n  - Nested 1.2\n- Item 2",
	"- a\n\n- b\n\n- c", "1. one\n2. two\n   1. nested\n   2. nested2\n3. three", "5. five\n6. six", "- \n- x", "-",
	String.raw`A map $\mathbb{C}^3 \to \mathbb{C}^3$, $xy$, $x-y$, $-x$, $\frac{1}{2}$, $\rightarrow$, and \(s \to \infty\).`,
	String.raw`Map \(\mathbb{C}^3`, "\\[\nx^2", String.raw`Map $\mathbb{C}^3`, String.raw`"\"`, "Cost $5 and $10", "$HOME and $PATH_VAR",
	"$$\n\\widetilde Y_{sf}\n=\n(1-w_{sf})\\mu_{sf}^{\\mathrm{MAR}}\n$$\n\nInline \\(A_{sf}\\)", "$$x^2$$", "\\[a+b\\]\n\ntext", "$$\n| a | b |\n|---|---|\n| 1 | 2 |",
	"This is thinking with `inline code` and more text after", "This is thinking with **bold text** and more after", "---", "***\n___",
	'# Hello', '## Two\n### Three\n#### Four\nbody', "Setext\n===\n\nOther\n---", "> This is a quote", "> Quote with **bold** and `code`",
	"> line one\n> line two\n>\n> - list in quote\n\nafter", "> > nested quote\n> back", "> ```\n> code\n> ```",
	'### Why `sourceInfo` should not be optional', '# Title with `code` inside', '## Heading with **bold** and more',
	"Use ~~strikethrough~~ here", "Use ~strikethrough~ literally", "~~ not ~~ and ~~x~~y", "中文**强调**。和*斜体*，", "**「括弧」**の中",
	"Contact user@example.com for help", "Visit https://example.com for more", "[click here](https://example.com)", "[Email me](mailto:test@example.com)",
	"<https://a.b/c> and www.example.com.", "[ref link][r]\n\n[r]: https://ref.example 'Title'", "![img](http://x/y.png) after",
	"This is text with <thinking>hidden content</thinking> that should be visible", "<div>\nblock html\n</div>\n\npara", "<!-- comment -->",
	"```html\n<div>Some HTML</div>\n```", "```ts\nconst x = 1;\n``", "```ts\nconst x = 1;\n```", "```\nunclosed\ncode", "~~~py\nprint(1)\n~~~\nafter",
	"    indented code\n    more", "line one  \nline two\\\nline three", "a\nb\nc", "Escapes: \\* \\_ \\# \\\\ \\` \\[x\\]", "&amp; &lt;tag&gt; &copy; &#65;",
	"*em* _em_ **strong** __strong__ ***both*** *nested **deep** em*", "a*b*c a_b_c", "``code `with` ticks``", "` a `",
	"| a | b |\n|---|---|\n| 1 | **2** |", "| Link | Plain |\n| --- | --- |\n| [**one two three four five six**](https://example.com) | normal text |",
	"| Left | Center | Right |\n|:-----|:------:|------:|\n| a | b | c |\n| longer cell content here | x | yy |",
	"| h |\n|---|\n| averyveryverylongwordwithoutanybreaksinsideofitreally |", "a | b\n--|--\n1 | 2", "| x | y |\n|---|---|",
	"Text before\n| a | b |\n|---|---|\n| 1 | 2 |\nText after", "| 日本 | 語 |\n|---|---|\n| テキスト | 👍🏽 |",
	"tab\there\n\n-\titem", "trailing spaces   \n\n\n\nmany blanks", "\n\n  leading blank lines", "   ", "",
	'# H\n\npara\n\n- l1\n- l2\n\n> q\n\n```\nc\n```\n\n---\n\n| a |\n|---|\n| b |\n\nend',
	"Paragraph\n- list right after", "1. a\n\n   continuation para\n2. b", "- a\n  b lazy\nc lazy", "* [ ] task\n* [X] done\n* not task",
];

let seed = 4242;
const rnd = (n) => { seed = (seed * 1103515245 + 12345) % 2147483648; return Math.floor(seed / 65536) % n; };
const frags = ["word", " ", "  ", "\n", "\n\n", "**", "*", "_", "`", "~~", "~", "$", "$$", "\\(", "\\)", "\\[", "\\]", "x^2", "\\alpha",
	'# ', '## ', "- ", "1. ", "> ", "```", "|", "---", "[", "](", ")", "http://a.co", "<b>", "</b>", "\\", "日本語", "👍🏽", "\t", "    ",
	"averyveryverylongwordwithoutbreaks", "a_b", "&amp;", "[ ] ", "-|-", "。", "!", "<", ">", "=", "+", "~~~"];
for (let k = 0; k < 400; k++) {
	let s = "";
	const n = 1 + rnd(14);
	for (let i = 0; i < n; i++) s += frags[rnd(frags.length)];
	corpus.push(s);
}

const widths = [1, 3, 8, 20, 40, 80];
const variants = [
	{ px: 0, py: 0 }, { px: 1, py: 1, style: "gray" }, { px: 2, py: 0, style: "bg" }, { px: 0, py: 0, hyperlinks: true },
	{ px: 0, py: 0, opts: { preserveOrderedListMarkers: true, preserveBackslashEscapes: true } }, { px: 1, py: 0, opts: { renderLatex: false }, indent: true },
];
const styles = {
	gray: { color: gray, italic: true },
	bg: { color: yellow, bgColor: (s) => `${E}[48;5;236m${s}${E}[49m`, bold: true, underline: true, strikethrough: true },
};
const out = { cases: [] };
for (const [i, t] of corpus.entries()) {
	const r = [];
	const vs = i < 140 ? variants : [variants[i % variants.length]];
	for (const v of vs) {
		setCapabilities({ images: null, trueColor: false, hyperlinks: !!v.hyperlinks });
		const m = new Markdown(t, v.px, v.py, v.indent ? themeIndent : theme, v.style ? styles[v.style] : undefined, v.opts);
		r.push({ v: variants.indexOf(v), out: widths.map((w) => m.render(w)) });
	}
	out.cases.push({ t, r });
}
console.log(JSON.stringify(out));
