import { formatSize } from "@earendil-works/pi-coding-agent";
import { EXISTING_CODE_MARKER } from "./constants.js";
import type { MorphSettings } from "./types.js";

export function normalizeCodeEditInput(codeEdit: string): string {
  const trimmed = codeEdit.trim();
  const lines = trimmed.split("\n");
  if (lines.length < 3) return codeEdit;

  const firstLine = lines[0] ?? "";
  const lastLine = lines[lines.length - 1] ?? "";
  if (/^```[\w-]*$/.test(firstLine) && /^```$/.test(lastLine)) {
    return lines.slice(1, -1).join("\n");
  }

  return codeEdit;
}

export function stripOuterCodeFence(text: string): string {
  const trimmed = text.trim();
  const lines = trimmed.split("\n");
  if (lines.length < 3) return text;

  const firstLine = lines[0] ?? "";
  const lastLine = lines[lines.length - 1] ?? "";
  if (/^```[\w-]*$/.test(firstLine) && /^```$/.test(lastLine)) {
    return lines.slice(1, -1).join("\n");
  }

  return text;
}

export function detectLineEnding(text: string): "\n" | "\r\n" {
  const crlf = (text.match(/\r\n/g) ?? []).length;
  const lf = (text.match(/(?<!\r)\n/g) ?? []).length;
  return crlf > lf ? "\r\n" : "\n";
}

export function normalizeLineEndings(text: string, eol: "\n" | "\r\n"): string {
  const lf = text.replace(/\r\n/g, "\n");
  return eol === "\n" ? lf : lf.replace(/\n/g, "\r\n");
}

export function byteLength(text: string): number {
  return Buffer.byteLength(text, "utf8");
}

export function summarizeChange(original: string, merged: string): { text: string; changed: boolean; oldLines: number; newLines: number } {
  if (original === merged) {
    const lineCount = original.split("\n").length;
    return { text: "No changes detected.", changed: false, oldLines: lineCount, newLines: lineCount };
  }

  const oldLines = original.split("\n");
  const newLines = merged.split("\n");
  let prefix = 0;
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) prefix++;

  let oldSuffix = oldLines.length - 1;
  let newSuffix = newLines.length - 1;
  while (oldSuffix >= prefix && newSuffix >= prefix && oldLines[oldSuffix] === newLines[newSuffix]) {
    oldSuffix--;
    newSuffix--;
  }

  const removed = Math.max(0, oldSuffix - prefix + 1);
  const added = Math.max(0, newSuffix - prefix + 1);
  const startLine = prefix + 1;
  const oldPreview = oldLines.slice(prefix, Math.min(oldSuffix + 1, prefix + 12));
  const newPreview = newLines.slice(prefix, Math.min(newSuffix + 1, prefix + 12));

  const parts = [
    `Changed around line ${startLine}: +${added} -${removed} lines in changed window`,
    "",
    "```diff",
    ...oldPreview.map((line) => `-${line}`),
    ...(removed > oldPreview.length ? ["-... (removed preview truncated)"] : []),
    ...newPreview.map((line) => `+${line}`),
    ...(added > newPreview.length ? ["+... (added preview truncated)"] : []),
    "```",
  ];

  return { text: parts.join("\n"), changed: true, oldLines: oldLines.length, newLines: newLines.length };
}

export function validateMergedOutput(original: string, merged: string, codeEdit: string, settings: MorphSettings): void {
  const hasMarkers = codeEdit.includes(EXISTING_CODE_MARKER);
  const originalHadMarker = original.includes(EXISTING_CODE_MARKER);

  if (hasMarkers && !originalHadMarker && merged.includes(EXISTING_CODE_MARKER)) {
    throw new Error(
      `Morph output still contains ${JSON.stringify(EXISTING_CODE_MARKER)}. No file changes were written. Retry with more concrete context or use edit.`,
    );
  }

  const outputBytes = byteLength(merged);
  if (outputBytes > settings.maxOutputBytes) {
    throw new Error(
      `Morph output is ${formatSize(outputBytes)}, over maxOutputBytes=${formatSize(settings.maxOutputBytes)}. No file changes were written.`,
    );
  }

  if (hasMarkers && original.length > 0) {
    const originalLineCount = original.split("\n").length;
    const mergedLineCount = merged.split("\n").length;
    const charLoss = (original.length - merged.length) / original.length;
    const lineLoss = (originalLineCount - mergedLineCount) / originalLineCount;

    if (charLoss > 0.6 && lineLoss > 0.5) {
      throw new Error(
        `Morph output looks destructively truncated (${Math.round(charLoss * 100)}% chars, ${Math.round(lineLoss * 100)}% lines lost). No file changes were written.`,
      );
    }
  }
}
