// Generates tests/file-processor-parity/oracle.json by driving Pi's real
// `processFileArguments` (cli/file-processor.ts) text path and
// `buildInitialMessage` (cli/initial-message.ts), plus the pi-rs-side
// `@file` argument parsing contract (cli/args.ts fileArgs vs messages).
//
// The oracle records Pi's exact `ProcessedFiles.text` for text files
// (file-missing and empty-file behaviors included) and `buildInitialMessage`
// composition (stdin + fileText + first message), so the port has a
// differential rather than repeating the spec's string templates by hand.
//
// Run via scripts/file-processor-oracle. Offline normal checks consume the
// checked oracle; opt-in regeneration drives the pinned Pi source.
import { writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { processFileArguments } from "../../ref/pi/packages/coding-agent/src/cli/file-processor.ts";
import { buildInitialMessage } from "../../ref/pi/packages/coding-agent/src/cli/initial-message.ts";

// The fixture directory is resolved to the absolute repo path and the emitted
// text substitutes that absolute prefix with a stable sentinel so the checked
// oracle is portable (does not encode this machine's repo path) while the
// Rust test performs the identical substitution before comparing byte-for-byte.
const FIX_SENTINEL = "@FIXDIR@";
const fixDir = resolve(dirname(new URL(import.meta.url).pathname), "fixtures");

const out: any = { oracle: "Pi v0.79.0 c5582102", cases: [] };

const pathOf = (name: string) => join(fixDir, name);

function norm(text: string): string {
	return text.split(fixDir).join(FIX_SENTINEL);
}

// --- Text-file processing cases -------------------------------------------
async function runFile(name: string, files: string[]) {
	let text = "";
	let imagesLen = 0;
	let images: any[] = [];
	let threw: string | null = null;
	try {
		const result = await processFileArguments(files);
		text = norm(result.text);
		imagesLen = result.images.length;
		images = result.images.map((im) => ({ mimeType: im.mimeType, data: im.data }));
	} catch (e) {
		threw = e instanceof Error ? e.message : String(e);
	}
	out.cases.push({
		kind: "files",
		name,
		files: files.map((f) => f.split(fixDir).join(FIX_SENTINEL)),
		text,
		imagesLen,
		images,
		threw,
	});
}

async function main() {
	await runFile("single-text", [pathOf("note.txt")]);
	await runFile("multi-text", [pathOf("note.txt"), pathOf("multi.txt")]);
	await runFile("empty-file-skipped", [pathOf("empty.txt")]);
	await runFile("mix-empty-and-text", [pathOf("empty.txt"), pathOf("note.txt")]);
	// The missing-file case is NOT run in-process: Pi's processFileArguments
	// prints `Error: File not found: <abs>` to stderr (chalk.red) and calls
	// process.exit(1), which cannot be captured here. It is pinned separately
	// by the Rust test as a stderr+exit contract (spec file-processor.ts).

	// --- Image-file processing cases ------------------------------------------
	// These drive Pi's real image sniff (`mime.ts`) + base64 attachment path
	// with `autoResizeImages: false` so the recorded base64 is deterministic
	// (auto-resize would yield variable bytes). The Rust test reproduces the
	// same options so the comparison is byte-for-byte.
	async function runImage(name: string, file: string) {
		const result = await processFileArguments([pathOf(file)], { autoResizeImages: false });
		out.cases.push({
			kind: "files",
			name,
			files: [pathOf(file).split(fixDir).join(FIX_SENTINEL)],
			text: norm(result.text),
			imagesLen: result.images.length,
			images: result.images.map((im) => ({ mimeType: im.mimeType, data: im.data })),
			threw: null,
		});
	}
	await runImage("static-png", "tiny.png");
	// acTL within the 4100-byte sniff window -> animated -> not an image.
	await runImage("apng-within-window", "apng-win.png");
	// acTL beyond the 4100-byte sniff window -> Pi reads only the first 4100
	// bytes so it never sees acTL and classifies this as a static png.
	await runImage("apng-beyond-window", "apng-beyond.png");

	// --- buildInitialMessage composition --------------------------------------
	const noFiles = { messages: [], fileArgs: [] } as any;
	out.cases.push({
		kind: "init",
		name: "stdin-only",
		input: { parsed: { ...noFiles }, stdinContent: "from stdin" },
		result: buildInitialMessage({ parsed: { ...noFiles }, stdinContent: "from stdin" }),
	});
	out.cases.push({
		kind: "init",
		name: "message-only",
		input: { parsed: { messages: ["hi"] }, fileText: undefined, stdinContent: undefined },
		result: buildInitialMessage({ parsed: { messages: ["hi"] }, fileText: undefined, stdinContent: undefined }),
	});
	out.cases.push({
		kind: "init",
		name: "stdin-plus-message",
		input: { parsed: { messages: ["hi"] }, fileText: "FILE\n", stdinContent: "STDIN\n" },
		result: buildInitialMessage({ parsed: { messages: ["hi"] }, fileText: "FILE\n", stdinContent: "STDIN\n" }),
	});

	writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
	console.log(`wrote ${process.argv[2]}`);
}
main().catch((e) => {
	console.error(e);
	process.exit(1);
});
