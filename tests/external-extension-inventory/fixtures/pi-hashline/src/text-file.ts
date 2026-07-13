import { open, stat, readFile } from "node:fs/promises";
import { TextDecoder } from "node:util";
import { type FileSnapshot, getFileSnapshot, sameFileSnapshot } from "./snapshot";

export type LineEnding = "\n" | "\r\n";
export type LineTerminator = LineEnding | "\r" | "";

export type TextLineRecord = {
  text: string;
  ending: LineTerminator;
};

export type LoadedTextFile = {
  bom: string;
  rawText: string;
  text: string;
  lineEnding: LineEnding;
};

export type LoadedTextFileWithSnapshot = LoadedTextFile & {
  snapshot: FileSnapshot;
};

export function detectLineEnding(text: string): LineEnding {
  const crlf = text.indexOf("\r\n");
  const lf = text.indexOf("\n");
  if (crlf === -1 || lf === -1) return "\n";
  return crlf <= lf ? "\r\n" : "\n";
}

export function normalizeToLF(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

export function restoreLineEnding(text: string, lineEnding: LineEnding): string {
  return lineEnding === "\r\n" ? text.replace(/\n/g, "\r\n") : text;
}

export function splitTextLineRecords(text: string): TextLineRecord[] {
  if (text.length === 0) return [];

  const records: TextLineRecord[] = [];
  let lineStart = 0;
  let index = 0;

  while (index < text.length) {
    const char = text[index];
    if (char !== "\r" && char !== "\n") {
      index++;
      continue;
    }

    const ending: LineTerminator = char === "\r" && text[index + 1] === "\n" ? "\r\n" : char;
    records.push({ text: text.slice(lineStart, index), ending });
    index += ending.length;
    lineStart = index;
  }

  if (lineStart < text.length) {
    records.push({ text: text.slice(lineStart), ending: "" });
  }

  return records;
}

export function joinTextLineRecords(records: TextLineRecord[]): string {
  return records.map((record) => `${record.text}${record.ending}`).join("");
}

export function stripBom(text: string): { bom: string; text: string } {
  return text.startsWith("\uFEFF")
    ? { bom: "\uFEFF", text: text.slice(1) }
    : { bom: "", text };
}

export function detectSupportedImageMime(buffer: Uint8Array): string | undefined {
  if (buffer.length >= 8 &&
    buffer[0] === 0x89 &&
    buffer[1] === 0x50 &&
    buffer[2] === 0x4e &&
    buffer[3] === 0x47 &&
    buffer[4] === 0x0d &&
    buffer[5] === 0x0a &&
    buffer[6] === 0x1a &&
    buffer[7] === 0x0a
  ) {
    return "image/png";
  }

  if (buffer.length >= 3 && buffer[0] === 0xff && buffer[1] === 0xd8 && buffer[2] === 0xff) {
    return "image/jpeg";
  }

  if (buffer.length >= 6) {
    const header = Buffer.from(buffer.subarray(0, 6)).toString("ascii");
    if (header === "GIF87a" || header === "GIF89a") return "image/gif";
  }

  if (buffer.length >= 12) {
    const riff = Buffer.from(buffer.subarray(0, 4)).toString("ascii");
    const webp = Buffer.from(buffer.subarray(8, 12)).toString("ascii");
    if (riff === "RIFF" && webp === "WEBP") return "image/webp";
  }

  return undefined;
}

export async function isSupportedImageFile(path: string): Promise<boolean> {
  const fileStat = await stat(path);
  if (!fileStat.isFile()) return false;

  const fileHandle = await open(path, "r");
  try {
    const buffer = Buffer.alloc(12);
    const { bytesRead } = await fileHandle.read(buffer, 0, buffer.length, 0);
    return detectSupportedImageMime(buffer.subarray(0, bytesRead)) !== undefined;
  } finally {
    await fileHandle.close();
  }
}

function hasNullByte(buffer: Buffer): boolean {
  return buffer.includes(0);
}

function decodeUtf8(buffer: Buffer): string {
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(buffer);
  } catch (error: unknown) {
    if (error instanceof TypeError) {
      throw new Error("File is not valid UTF-8 text.");
    }
    throw error;
  }
}

export async function loadTextFile(path: string): Promise<LoadedTextFile> {
  const fileStat = await stat(path);
  if (fileStat.isDirectory()) {
    throw new Error("Path is a directory. Use ls to inspect directories.");
  }
  if (!fileStat.isFile()) {
    throw new Error("Path is not a regular file.");
  }

  const buffer = await readFile(path);
  if (hasNullByte(buffer)) {
    throw new Error("File appears to be binary (null bytes detected). Hashline tools only support UTF-8 text files.");
  }

  const decoded = decodeUtf8(buffer);
  const { bom, text: rawText } = stripBom(decoded);
  const lineEnding = detectLineEnding(rawText);
  return { bom, rawText, text: normalizeToLF(rawText), lineEnding };
}

export async function loadTextFileWithSnapshot(path: string): Promise<LoadedTextFileWithSnapshot> {
  for (let attempt = 0; attempt < 2; attempt++) {
    const before = await getFileSnapshot(path);
    const file = await loadTextFile(path);
    const after = await getFileSnapshot(path);
    if (sameFileSnapshot(before, after)) {
      return { ...file, snapshot: after };
    }
  }

  throw new Error("[E_CONCURRENT_MODIFICATION] File changed while being read. Re-read and retry with fresh anchors.");
}
