// Regenerates tests/image-parity/oracle.json from Pi's real image
// machinery: `utils/image-resize-core.ts` `resizeImageInProcess` and
// `utils/image-convert.ts` `convertToPng`, both running the vendored
// `@silvia-odwyer/photon-node` 0.3.4 WASM build. Case *inputs* are
// synthesized deterministically here (photon from raw pixels, plus EXIF
// segment splicing) and recorded in the oracle alongside the expected
// outputs, so pi-rs's replay consumes exact bytes. Run via
// scripts/image-oracle. Do not edit the oracle by hand.
import { readFileSync } from "node:fs";
import { convertToPng } from "../../ref/pi/packages/coding-agent/src/utils/image-convert.ts";
import { resizeImageInProcess } from "../../ref/pi/packages/coding-agent/src/utils/image-resize-core.ts";
import { loadPhoton } from "../../ref/pi/packages/coding-agent/src/utils/photon.ts";

type Pattern = "gradient" | "noise" | "flat";

interface CaseSpec {
	name: string;
	width: number;
	height: number;
	pattern: Pattern;
	format: "png" | "jpeg" | "webp";
	/** Splice an EXIF APP1 orientation segment (JPEG only). */
	exifOrientation?: number;
	mimeType: string;
	kind: "resize" | "convert";
	options?: { maxWidth?: number; maxHeight?: number; maxBytes?: number; jpegQuality?: number };
	/** convert cases: corrupt the base64 payload instead. */
	garbage?: boolean;
}

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as CaseSpec[];

// Deterministic LCG so inputs are reproducible across regenerations.
function makeRng(seed: number): () => number {
	let state = seed >>> 0;
	return () => {
		state = (state * 1664525 + 1013904223) >>> 0;
		return state / 0x100000000;
	};
}

function makePixels(width: number, height: number, pattern: Pattern): Uint8Array {
	const pixels = new Uint8Array(width * height * 4);
	const rng = makeRng(width * 7919 + height * 104729 + pattern.length);
	for (let y = 0; y < height; y++) {
		for (let x = 0; x < width; x++) {
			const i = (y * width + x) * 4;
			if (pattern === "gradient") {
				pixels[i] = Math.floor((x * 255) / Math.max(1, width - 1));
				pixels[i + 1] = Math.floor((y * 255) / Math.max(1, height - 1));
				pixels[i + 2] = (x + y) % 256;
			} else if (pattern === "noise") {
				pixels[i] = Math.floor(rng() * 256);
				pixels[i + 1] = Math.floor(rng() * 256);
				pixels[i + 2] = Math.floor(rng() * 256);
			} else {
				pixels[i] = 40;
				pixels[i + 1] = 90;
				pixels[i + 2] = 160;
			}
			pixels[i + 3] = 255;
		}
	}
	return pixels;
}

/** Minimal little-endian EXIF APP1 segment carrying only tag 0x0112. */
function exifApp1(orientation: number): Uint8Array {
	const payload = new Uint8Array(32);
	payload.set([0x45, 0x78, 0x69, 0x66, 0x00, 0x00], 0); // "Exif\0\0"
	payload.set([0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00], 6); // II TIFF, IFD @8
	payload.set([0x01, 0x00], 14); // entry count 1
	payload.set([0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, orientation, 0x00, 0x00, 0x00], 16);
	payload.set([0x00, 0x00, 0x00, 0x00], 28); // next IFD
	const segment = new Uint8Array(4 + payload.length);
	segment[0] = 0xff;
	segment[1] = 0xe1;
	const length = payload.length + 2;
	segment[2] = (length >> 8) & 0xff;
	segment[3] = length & 0xff;
	segment.set(payload, 4);
	return segment;
}

function spliceExif(jpeg: Uint8Array, orientation: number): Uint8Array {
	const app1 = exifApp1(orientation);
	const out = new Uint8Array(jpeg.length + app1.length);
	out.set(jpeg.subarray(0, 2), 0); // SOI
	out.set(app1, 2);
	out.set(jpeg.subarray(2), 2 + app1.length);
	return out;
}

async function synthesize(spec: CaseSpec): Promise<Uint8Array> {
	const photon = await loadPhoton();
	if (!photon) throw new Error("photon-node failed to load");
	const image = new photon.PhotonImage(makePixels(spec.width, spec.height, spec.pattern), spec.width, spec.height);
	try {
		let bytes: Uint8Array;
		if (spec.format === "png") bytes = image.get_bytes();
		else if (spec.format === "jpeg") bytes = image.get_bytes_jpeg(90);
		else bytes = image.get_bytes_webp();
		if (spec.exifOrientation !== undefined) {
			if (spec.format !== "jpeg") throw new Error(`${spec.name}: EXIF splicing supports JPEG only`);
			bytes = spliceExif(bytes, spec.exifOrientation);
		}
		return bytes;
	} finally {
		image.free();
	}
}

async function main(): Promise<void> {
	const oracle: unknown[] = [];
	for (const spec of cases) {
		const inputBytes = await synthesize(spec);
		let input = Buffer.from(inputBytes).toString("base64");
		if (spec.garbage) input = `not-an-image-${input.slice(0, 24)}`;
		if (spec.kind === "resize") {
			const result = await resizeImageInProcess(
				new Uint8Array(Buffer.from(input, "base64")),
				spec.mimeType,
				spec.options,
			);
			oracle.push({ name: spec.name, kind: spec.kind, mimeType: spec.mimeType, options: spec.options ?? null, input, expected: result });
		} else {
			const result = await convertToPng(input, spec.mimeType);
			oracle.push({ name: spec.name, kind: spec.kind, mimeType: spec.mimeType, input, expected: result });
		}
	}
	console.log(JSON.stringify(oracle, null, "\t"));
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
