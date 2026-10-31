import { pathToFileURL } from "node:url";
const { findWordBackward, findWordForward } = await import(pathToFileURL(`${process.argv[2]}/src/word-navigation.ts`).href);
const { getWordSegmenter } = await import(pathToFileURL(`${process.argv[2]}/src/utils.ts`).href);
const texts = [
	"hello world", "  leading", "trailing   ", "foo.bar(baz)", "a-b_c/d", "path/to/file.ts:12", "can't won't", "x == y && z",
	"日本語のテキストです", "한국어 텍스트", "mixed 日本 text", "emoji 👍🏽 here", "café naïve", "1.5e10 3,000", "snake_case camelCase",
	"https:\/\/example.com/a?b=c", "   ", "", "...", "--flag=value", "tab\tsep", "line1\nline2", "ไทยภาษา", "a.b.c.d", "e\u0301clair",
	"Привет мир", "x[0] = y{1};", "@user #tag $var", "a  b   c", "αβγ.δεζ",
];
const seg = getWordSegmenter();
const out = texts.map((t) => {
	const b = [], f = [];
	for (let i = 0; i <= t.length; i++) {
		if (i > 0 && i < t.length && t.charCodeAt(i) >= 0xdc00 && t.charCodeAt(i) <= 0xdfff) {
			b.push(null);
			f.push(null);
			continue;
		}
		b.push(findWordBackward(t, i));
		f.push(findWordForward(t, i));
	}
	const segs = [...seg.segment(t)].map((s) => [s.index, s.segment, !!s.isWordLike]);
	return { t, b, f, segs };
});
process.stdout.write(JSON.stringify(out));
