import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const root = process.argv[2];
const { renderLatex } = await import(pathToFileURL(`${root}/src/latex.ts`).href);
const latexSource = readFileSync(`${root}/src/latex.ts`, "utf8");
const testSource = readFileSync(`${root}/test/latex.test.ts`, "utf8");

const inputs = new Set();
for (const m of testSource.matchAll(/String\.raw`([^`]*)`/g)) inputs.add(m[1]);
for (const m of testSource.matchAll(/\[\s*("(?:[^"\\]|\\.)*")\s*,/g)) inputs.add(JSON.parse(m[1]));
for (const m of testSource.matchAll(/renderLatex\(("(?:[^"\\]|\\.)*")/g)) inputs.add(JSON.parse(m[1]));

const commands = new Set();
for (const m of latexSource.matchAll(/^\t([A-Za-z]+): "/gm)) commands.add(m[1]);
for (const m of latexSource.matchAll(/^\t"([A-Za-z]+)",$/gm)) commands.add(m[1]);
for (const m of latexSource.matchAll(/command === "([^"]+)"/g)) commands.add(m[1]);
for (const m of latexSource.matchAll(/environment === "([^"]+)"/g)) commands.add(`env:${m[1]}`);
const envs = ["equation", "equation*", "displaymath", "aligned", "align", "align*", "alignedat", "alignat", "alignat*",
	"gather", "gathered", "multline", "multline*", "split", "cases", "cases*", "array", "matrix", "smallmatrix", "pmatrix",
	"bmatrix", "Bmatrix", "vmatrix", "Vmatrix", "foo"];
for (const c of commands) {
	if (c.startsWith("env:")) continue;
	for (const t of [`\\${c}`, `\\${c}{x}`, `\\${c}{ab}{c}`, `a\\${c} b`, `\\${c}_{i=0}^{n} x`, `\\${c}\\limits_0^1 y`,
		`\\${c}\\nolimits_a f`, `\\${c} x`, `\\${c}{x+1}{y-1}`, `\\${c}[3]{x}`, `\\${c}*{op}_i`, `\\${c}{=}`, `\\${c}x^2`,
		`\\${c}{}`, `2\\${c}(x)`]) inputs.add(t);
}
for (const s of ["!", ",", ":", ";", " ", ">", "{", "}", "$", "%", "#", "_", "&", "|", "\\", "\n", "\r\n", "[", "("]) {
	inputs.add(`a\\${s}b`);
	inputs.add(`\\sin\\${s}x`);
}
for (const e of envs) {
	for (const body of ["a & b \\\\ c & d", "x^2 &= 1 \\\\[2pt] y &= \\frac{1}{2},", "{cc} 1 & 2 \\\\ 3 & 4", "a, & x>0 \\\\ b, & \\text{otherwise}",
		"1 & 2 & 3", "x", "", "a \\\\ b \\\\ c \\\\ d", "{rl} a &= b & c &= d \\\\ e &= f", "\\frac{a}{b} & c \\\\ d & e^{2}",
		"x & \\text{if } x \\ge 0 \\\\ -x & \\text{when } x<0 \\\\ 0 & \\text{for } y \\\\ 1 & y"]) {
		inputs.add(`\\begin{${e}}${body}\\end{${e}}`);
		inputs.add(`f(x) = \\begin{${e}}${body}\\end{${e}}.`);
		inputs.add(`A \\begin{${e}}${body}\\end{${e}} B \\begin{${e}}${body}\\end{${e}}`);
	}
	inputs.add(`\\begin{${e}}x`);
}

let seed = 0x2545f491;
const rand = (n) => {
	seed ^= seed << 13; seed >>>= 0;
	seed ^= seed >>> 17;
	seed ^= seed << 5; seed >>>= 0;
	return seed % n;
};
const cmdList = [...commands].filter((c) => !c.startsWith("env:"));
const atoms = ["x", "y", "2", "10", "a", "Z", "R", " ", "  ", "+", "-", "=", "<", ">", "(", ")", "[", "]", ",", ".", "/",
	"*", "'", "|", "&", "~", "\n", "{", "}", "{x}", "{ab}", "{1}", "^", "_", "^2", "_i", "^{n+1}", "_{i=0}", "^{-1}", "_{ij}",
	"^{\\frac{1}{2}}", "_{x_1}", "^{a^b}", "\\frac{1}{2}", "\\frac{x+1}{y}", "\\tfrac12", "\\sqrt{x}", "\\sqrt[3]{x}",
	"\\sum_{i=1}^{n}", "\\int_0^\\infty", "\\lim_{x\\to 0}", "\\left(", "\\right)", "\\left.", "\\right|", "\\text{ if }",
	"\\mathbb{R}", "\\operatorname{rank}", "\\not=", "\\not\\in", "\\,", "\\!", "\\quad", "\\\\",
	"\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}", "\\begin{cases}1&x>0\\\\0&x\\le0\\end{cases}",
	"\\begin{aligned}a&=b\\\\c&=d\\end{aligned}", "日本", "é", "\t", "%", "$", "#", "\\{", "\\}", "\\|"];
for (let i = 0; i < 4000; i++) {
	const n = 1 + rand(9);
	let s = "";
	for (let j = 0; j < n; j++) s += rand(3) === 0 ? `\\${cmdList[rand(cmdList.length)]}` : atoms[rand(atoms.length)];
	inputs.add(s);
}

for (const a of ["😀", "日本", "é", "👍🏽", "e\u0301"]) {
	for (const t of [`x^{${a}}`, `x_{${a}}y`, `\\hat{${a}}`, `\\frac{${a}}{2}`, `${a}^2`, `\\mathbb{${a}}`, `\\not{${a}}`,
		`\\sqrt{${a}}`, `\\sum_{${a}}^{${a}${a}} x`, `\\begin{pmatrix}${a}&1\\\\2&${a}${a}\\end{pmatrix}`,
		`\\begin{cases}${a}&x\\\\1&${a}\\end{cases}`, `\\frac{${a}${a}}{\\frac{1}{${a}}}`]) inputs.add(t);
}

const cases = [...inputs]
	.map((s) => ({ s, i: renderLatex(s) ?? null, d: renderLatex(s, { display: true }) ?? null }))
	.filter((c) => (c.i ?? "").isWellFormed() && (c.d ?? "").isWellFormed());
process.stderr.write(`${cases.length} of ${inputs.size} inputs kept (others split surrogate pairs)\n`);
process.stdout.write(JSON.stringify({ cases }));
