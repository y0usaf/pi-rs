import {
  DIFF_DELETE_PREFIX_RE,
  HASH_LENGTH,
  HASH_RE,
  HASHLINE_BIGRAMS,
  HASHLINE_BIGRAMS_COUNT,
  HASHLINE_BIGRAM_RE_SRC,
  HASHLINE_PLUS_PREFIX_RE,
  HASHLINE_PREFIX_RE,
  SIGNIFICANT_RE,
  STRUCTURAL_STRIP_RE,
} from "./constants";
import {
  joinTextLineRecords,
  normalizeToLF,
  splitTextLineRecords,
  type LineEnding,
  type LineTerminator,
  type TextLineRecord,
} from "./text-file";

const DEFAULT_ANCHOR_TEXT_BUDGET_BYTES = 50 * 1024;

export type Anchor = {
  line: number;
  hash: string;
};

export type HashlineLoc =
  | "append"
  | "prepend"
  | { append: string }
  | { prepend: string }
  | { range: { pos: string; end: string } };

export type RawEdit = {
  op?: string;
  pos?: string;
  end?: string;
  lines?: string[] | string | null;
  oldText?: string;
  newText?: string;
  loc?: HashlineLoc;
  content?: string[] | string | null;
};

export type EditRequest = {
  path: string;
  edits: RawEdit[];
};

type LineEditKind = "replace" | "append" | "prepend";

type LineEdit = {
  requestIndex: number;
  label: string;
  kind: LineEditKind;
  start: number;
  end: number;
  lines: string[];
};

type StaleAnchor = {
  requested: Anchor;
  actual?: string;
  reason?: string;
};

const ANCHOR_REBASE_WINDOW = 5;

function tryRebaseAnchor(
  anchor: { line: number; hash: string },
  fileLines: string[],
  window: number = ANCHOR_REBASE_WINDOW,
): number | null {
  const lo = Math.max(1, anchor.line - window);
  const hi = Math.min(fileLines.length, anchor.line + window);
  let found: number | null = null;
  for (let line = lo; line <= hi; line++) {
    if (line === anchor.line) continue;
    if (computeLineHash(line, fileLines[line - 1] ?? "") !== anchor.hash) continue;
    if (found !== null) return null;
    found = line;
  }
  return found;
}

function structuralBigram(lineNumber: number): string {
  const mod100 = lineNumber % 100;
  if (mod100 >= 11 && mod100 <= 13) return "th";
  switch (lineNumber % 10) {
    case 1:
      return "st";
    case 2:
      return "nd";
    case 3:
      return "rd";
    default:
      return "th";
  }
}

export function computeLineHash(lineNumber: number, line: string): string {
  const normalized = line.replace(/\r/g, "").trimEnd();
  if (normalized.replace(STRUCTURAL_STRIP_RE, "").length === 0) {
    return structuralBigram(lineNumber);
  }

  const seed = SIGNIFICANT_RE.test(normalized) ? 0 : lineNumber;
  const xxHash32 = (globalThis as unknown as { Bun?: { hash?: { xxHash32?: (value: string, seed?: number) => number } } }).Bun?.hash?.xxHash32;
  if (typeof xxHash32 !== "function") {
    throw new Error("[E_HASH_UNAVAILABLE] Bun.hash.xxHash32 is required for hashline v2 anchors.");
  }
  return HASHLINE_BIGRAMS[xxHash32(normalized, seed) % HASHLINE_BIGRAMS_COUNT]!;
}

export function getVisibleLines(text: string): string[] {
  if (text.length === 0) return [];
  const lines = text.split("\n");
  if (text.endsWith("\n")) lines.pop();
  return lines;
}

function joinVisibleLines(lines: string[], preserveTerminalNewline: boolean): string {
  if (lines.length === 0) return "";
  const joined = lines.join("\n");
  return preserveTerminalNewline ? `${joined}\n` : joined;
}

export function formatHashlineRegion(lines: string[], startLine: number): string {
  return lines
    .map((line, index) => {
      const lineNumber = startLine + index;
      return `${lineNumber}${computeLineHash(lineNumber, line)}|${line}`;
    })
    .join("\n");
}

function parseAnchor(ref: string): Anchor {
  const core = ref.replace(/^\s*[>+-]*\s*/, "").trimEnd();
  const match = core.match(new RegExp(`^([0-9]+)(${HASHLINE_BIGRAM_RE_SRC})(?:[:|].*)?$`, "s"));
  if (!match) {
    throw new Error(`[E_BAD_REF] Invalid line reference ${JSON.stringify(ref)}. Expected hashline v2 anchor from read output, e.g. "12th".`);
  }

  const line = Number.parseInt(match[1]!, 10);
  if (!Number.isInteger(line) || line < 1) {
    throw new Error(`[E_BAD_REF] Line number must be >= 1 in ${JSON.stringify(ref)}.`);
  }

  const hash = match[2]!;
  if (hash.length !== HASH_LENGTH || !HASH_RE.test(hash)) {
    throw new Error(`[E_BAD_REF] Invalid hash in ${JSON.stringify(ref)}. Hashes are two-letter hashline v2 bigrams.`);
  }

  return { line, hash };
}

function stringifyAnchor(anchor: Anchor): string {
  return `${anchor.line}${anchor.hash}`;
}

function parseEditLines(value: string[] | string | null | undefined, editIndex: number, fieldName = "lines"): string[] {
  if (value === undefined) {
    throw new Error(`Edit ${editIndex} requires a "${fieldName}" field.`);
  }
  if (value === null) return [];
  const lines = typeof value === "string"
    ? (value.endsWith("\n") ? value.slice(0, -1) : value).replaceAll("\r", "").split("\n")
    : value.map((line) => line.replaceAll("\r", ""));

  for (const line of lines) {
    if (
      HASHLINE_PREFIX_RE.test(line) ||
      HASHLINE_PLUS_PREFIX_RE.test(line) ||
      DIFF_DELETE_PREFIX_RE.test(line)
    ) {
      throw new Error(`[E_INVALID_PATCH] edits[${editIndex}].${fieldName} must contain literal file content, not rendered hashline anchors or diff prefixes. Offending line: ${JSON.stringify(line)}`);
    }
  }

  return lines;
}

function validateAnchor(anchor: Anchor, fileLines: string[], staleAnchors: StaleAnchor[]): void {
  const current = fileLines[anchor.line - 1];
  if (current === undefined) {
    staleAnchors.push({
      requested: anchor,
      reason: `line ${anchor.line} is outside current file range (1-${fileLines.length})`,
    });
    return;
  }

  const actual = computeLineHash(anchor.line, current);
  if (actual === anchor.hash) return;

  const rebased = tryRebaseAnchor(anchor, fileLines);
  if (rebased !== null) {
    anchor.line = rebased;
    return;
  }

  staleAnchors.push({ requested: anchor, actual });
}

function formatStaleAnchorError(staleAnchors: StaleAnchor[], fileLines: string[]): string {
  const retryLines = new Set<number>();
  for (const stale of staleAnchors) {
    const line = stale.requested.line;
    if (line >= 1 && line <= fileLines.length) retryLines.add(line);
  }

  const displayLines = new Set<number>();
  for (const stale of staleAnchors) {
    const line = Math.max(1, Math.min(stale.requested.line, fileLines.length));
    for (let i = Math.max(1, line - 2); i <= Math.min(fileLines.length, line + 2); i++) {
      displayLines.add(i);
    }
  }

  const out = [
    `[E_STALE_ANCHOR] ${staleAnchors.length} stale or invalid anchor${staleAnchors.length === 1 ? "" : "s"}. Retry with the >>> LINEID|content lines below, or call read again.`,
    "",
  ];

  for (const stale of staleAnchors) {
    const requested = stringifyAnchor(stale.requested);
    if (stale.reason) {
      out.push(`- ${requested}: ${stale.reason}`);
    } else {
      out.push(`- ${requested}: current hash is ${stale.actual}`);
    }
  }

  const sorted = [...displayLines].sort((a, b) => a - b);
  if (sorted.length > 0) {
    out.push("");
    let previous = -1;
    for (const lineNumber of sorted) {
      if (previous !== -1 && lineNumber > previous + 1) out.push("    ...");
      previous = lineNumber;
      const line = fileLines[lineNumber - 1]!;
      const prefix = `${lineNumber}${computeLineHash(lineNumber, line)}`;
      out.push(`${retryLines.has(lineNumber) ? ">>>" : "   "} ${prefix}|${line}`);
    }
  }

  if (fileLines.length === 0) {
    out.push("Current file is empty. Use prepend/append with no pos to insert content.");
  }

  return out.join("\n");
}

function describeLineEdit(edit: RawEdit): string {
  if (edit.loc !== undefined) return `loc ${JSON.stringify(edit.loc)}`;
  switch (edit.op) {
    case "replace":
      return edit.end ? `replace ${edit.pos}-${edit.end}` : `replace ${edit.pos}`;
    case "append":
      return edit.pos ? `append after ${edit.pos}` : "append at EOF";
    case "prepend":
      return edit.pos ? `prepend before ${edit.pos}` : "prepend at BOF";
    default:
      return edit.op ?? "edit";
  }
}

function resolveLocEdit(index: number, edit: RawEdit, fileLines: string[], staleAnchors: StaleAnchor[]): LineEdit {
  const lines = parseEditLines(edit.content, index, "content");
  const loc = edit.loc;

  if (loc === "append") {
    return { requestIndex: index, label: describeLineEdit(edit), kind: "append", start: fileLines.length, end: fileLines.length, lines };
  }
  if (loc === "prepend") {
    return { requestIndex: index, label: describeLineEdit(edit), kind: "prepend", start: 0, end: 0, lines };
  }
  if (!loc || typeof loc !== "object") {
    throw new Error(`[E_BAD_OP] Edit ${index} loc must be "append", "prepend", {append}, {prepend}, or {range}.`);
  }

  if ("append" in loc) {
    const pos = parseAnchor(loc.append);
    validateAnchor(pos, fileLines, staleAnchors);
    return { requestIndex: index, label: describeLineEdit(edit), kind: "append", start: pos.line, end: pos.line, lines };
  }
  if ("prepend" in loc) {
    const pos = parseAnchor(loc.prepend);
    validateAnchor(pos, fileLines, staleAnchors);
    return { requestIndex: index, label: describeLineEdit(edit), kind: "prepend", start: pos.line - 1, end: pos.line - 1, lines };
  }
  if ("range" in loc) {
    const pos = parseAnchor(loc.range.pos);
    const end = parseAnchor(loc.range.end);
    validateAnchor(pos, fileLines, staleAnchors);
    validateAnchor(end, fileLines, staleAnchors);
    if (end.line < pos.line) {
      throw new Error(`[E_BAD_REF] Edit ${index} has end before pos (${stringifyAnchor(end)} < ${stringifyAnchor(pos)}).`);
    }
    return { requestIndex: index, label: describeLineEdit(edit), kind: "replace", start: pos.line - 1, end: end.line, lines };
  }

  throw new Error(`[E_BAD_OP] Edit ${index} loc must be "append", "prepend", {append}, {prepend}, or {range}.`);
}

function resolveLineEdits(edits: RawEdit[], fileLines: string[]): LineEdit[] {
  const staleAnchors: StaleAnchor[] = [];
  const resolved: LineEdit[] = [];

  for (const [index, edit] of edits.entries()) {
    if (edit.op === "replace_text") continue;
    if (edit.loc !== undefined) {
      resolved.push(resolveLocEdit(index, edit, fileLines, staleAnchors));
      continue;
    }
    const lines = parseEditLines(edit.lines, index);
    const pos = edit.pos ? parseAnchor(edit.pos) : undefined;
    const end = edit.end ? parseAnchor(edit.end) : undefined;

    if (pos) validateAnchor(pos, fileLines, staleAnchors);
    if (end) validateAnchor(end, fileLines, staleAnchors);

    switch (edit.op) {
      case "replace": {
        if (!pos) throw new Error(`Edit ${index} with op "replace" requires a pos anchor.`);
        const endAnchor = end ?? pos;
        if (endAnchor.line < pos.line) {
          throw new Error(`[E_BAD_REF] Edit ${index} has end before pos (${stringifyAnchor(endAnchor)} < ${stringifyAnchor(pos)}).`);
        }
        resolved.push({
          requestIndex: index,
          label: describeLineEdit(edit),
          kind: "replace",
          start: pos.line - 1,
          end: endAnchor.line,
          lines,
        });
        break;
      }
      case "append": {
        resolved.push({
          requestIndex: index,
          label: describeLineEdit(edit),
          kind: "append",
          start: pos ? pos.line : fileLines.length,
          end: pos ? pos.line : fileLines.length,
          lines,
        });
        break;
      }
      case "prepend": {
        resolved.push({
          requestIndex: index,
          label: describeLineEdit(edit),
          kind: "prepend",
          start: pos ? pos.line - 1 : 0,
          end: pos ? pos.line - 1 : 0,
          lines,
        });
        break;
      }
      default:
        throw new Error(`[E_BAD_OP] Unknown edit op ${JSON.stringify(edit.op)}. Expected replace, append, prepend, or replace_text.`);
  }
  }

  if (staleAnchors.length > 0) {
    throw new Error(formatStaleAnchorError(staleAnchors, fileLines));
  }

  const sorted = [...resolved].sort((a, b) => a.start - b.start || a.end - b.end);
  for (let i = 1; i < sorted.length; i++) {
    const previous = sorted[i - 1]!;
    const current = sorted[i]!;
    if (current.start <= previous.end) {
      throw new Error(
        `[E_EDIT_CONFLICT] Edits ${previous.requestIndex} (${previous.label}) and ${current.requestIndex} (${current.label}) overlap or are adjacent. Merge them into one edit or split the request.`,
      );
    }
  }

  return sorted;
}

function applyLineEdits(originalLines: string[], edits: LineEdit[]): string[] {
  const next = [...originalLines];
  for (const edit of [...edits].sort((a, b) => b.start - a.start || b.end - a.end)) {
    next.splice(edit.start, edit.end - edit.start, ...edit.lines);
  }
  return next;
}

type NonEmptyLineTerminator = Exclude<LineTerminator, "">;

function isNonEmptyLineTerminator(ending: LineTerminator): ending is NonEmptyLineTerminator {
  return ending.length > 0;
}

function resolveFallbackLineTerminator(ending: LineEnding | LineTerminator | undefined): NonEmptyLineTerminator {
  return ending && ending.length > 0 ? ending as NonEmptyLineTerminator : "\n";
}

function cloneLineRecords(records: TextLineRecord[]): TextLineRecord[] {
  return records.map((record) => ({ text: record.text, ending: record.ending }));
}

function findBackwardLineTerminator(records: TextLineRecord[], start: number): NonEmptyLineTerminator | undefined {
  for (let index = Math.min(start, records.length - 1); index >= 0; index--) {
    const ending = records[index]?.ending ?? "";
    if (isNonEmptyLineTerminator(ending)) return ending;
  }
  return undefined;
}

function findForwardLineTerminator(records: TextLineRecord[], start: number): NonEmptyLineTerminator | undefined {
  for (let index = Math.max(0, start); index < records.length; index++) {
    const ending = records[index]?.ending ?? "";
    if (isNonEmptyLineTerminator(ending)) return ending;
  }
  return undefined;
}

function getPreferredLineTerminator(
  edit: LineEdit,
  originalRecords: TextLineRecord[],
  fallback: NonEmptyLineTerminator,
): NonEmptyLineTerminator {
  if (edit.kind === "replace") {
    for (let index = edit.start; index < edit.end; index++) {
      const ending = originalRecords[index]?.ending ?? "";
      if (isNonEmptyLineTerminator(ending)) return ending;
    }
    return findForwardLineTerminator(originalRecords, edit.start) ??
      findBackwardLineTerminator(originalRecords, edit.start - 1) ??
      fallback;
  }

  if (edit.kind === "append") {
    return findBackwardLineTerminator(originalRecords, edit.start - 1) ??
      findForwardLineTerminator(originalRecords, edit.start) ??
      fallback;
  }

  return findForwardLineTerminator(originalRecords, edit.start) ??
    findBackwardLineTerminator(originalRecords, edit.start - 1) ??
    fallback;
}

function normalizeLineRecordTerminatorState(records: TextLineRecord[], originalHadFinalNewline: boolean, fallback: NonEmptyLineTerminator): void {
  if (records.length === 0) return;

  for (let index = 0; index < records.length - 1; index++) {
    if (!isNonEmptyLineTerminator(records[index]!.ending)) records[index]!.ending = fallback;
  }

  const last = records[records.length - 1]!;
  if (originalHadFinalNewline) {
    if (!isNonEmptyLineTerminator(last.ending)) last.ending = fallback;
  } else {
    last.ending = "";
  }
}

function applyLineRecordEdits(
  originalRecords: TextLineRecord[],
  edits: RawEdit[],
  fallback: NonEmptyLineTerminator,
): TextLineRecord[] {
  const originalLines = originalRecords.map((record) => record.text);
  const lineEdits = resolveLineEdits(edits, originalLines);
  const next = cloneLineRecords(originalRecords);
  const originalHadFinalNewline = originalRecords.length > 0 && isNonEmptyLineTerminator(originalRecords[originalRecords.length - 1]!.ending);

  for (const edit of [...lineEdits].sort((a, b) => b.start - a.start || b.end - a.end)) {
    const preferredEnding = getPreferredLineTerminator(edit, originalRecords, fallback);
    const replacementRecords = edit.lines.map((line) => ({ text: line, ending: preferredEnding }));
    next.splice(edit.start, edit.end - edit.start, ...replacementRecords);
  }

  normalizeLineRecordTerminatorState(next, originalHadFinalNewline, fallback);
  return next;
}

function firstLineTerminatorInText(text: string): NonEmptyLineTerminator | undefined {
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (char === "\r") return text[index + 1] === "\n" ? "\r\n" : "\r";
    if (char === "\n") return "\n";
  }
  return undefined;
}

function lastLineTerminatorInText(text: string): NonEmptyLineTerminator | undefined {
  for (let index = text.length - 1; index >= 0; index--) {
    const char = text[index];
    if (char === "\n") return index > 0 && text[index - 1] === "\r" ? "\r\n" : "\n";
    if (char === "\r") return text[index + 1] === "\n" ? undefined : "\r";
  }
  return undefined;
}

function restoreLineTerminator(text: string, ending: NonEmptyLineTerminator): string {
  const normalized = normalizeToLF(text);
  return ending === "\n" ? normalized : normalized.replace(/\n/g, ending);
}

function buildNormalizedOffsetToRawOffsetMap(text: string): number[] {
  const map: number[] = [];
  let rawOffset = 0;
  let normalizedOffset = 0;

  while (rawOffset < text.length) {
    map[normalizedOffset] = rawOffset;
    if (text[rawOffset] === "\r" && text[rawOffset + 1] === "\n") {
      rawOffset += 2;
    } else {
      rawOffset++;
    }
    normalizedOffset++;
  }

  map[normalizedOffset] = rawOffset;
  return map;
}

function findUniqueNormalizedMatch(content: string, normalizedOld: string): number {
  if (normalizedOld.length === 0) {
    throw new Error("[E_BAD_OP] replace_text requires non-empty oldText.");
  }

  const matches: number[] = [];
  let from = 0;
  while (from <= content.length - normalizedOld.length) {
    const index = content.indexOf(normalizedOld, from);
    if (index === -1) break;
    matches.push(index);
    from = index + 1;
  }

  if (matches.length === 0) {
    throw new Error("[E_NO_MATCH] replace_text found no exact match in the current file. Re-read and use hashline anchors.");
  }
  if (matches.length > 1) {
    throw new Error("[E_MULTI_MATCH] replace_text found multiple matches in the current file. Re-read and use hashline anchors.");
  }

  return matches[0]!;
}

function applyExactUniqueReplacePreservingLineEndings(
  rawContent: string,
  oldText: string,
  newText: string,
  fallback: NonEmptyLineTerminator,
): string {
  const normalizedContent = normalizeToLF(rawContent);
  const normalizedOld = normalizeToLF(oldText);
  const normalizedNew = normalizeToLF(newText);
  const start = findUniqueNormalizedMatch(normalizedContent, normalizedOld);
  const offsetMap = buildNormalizedOffsetToRawOffsetMap(rawContent);
  const rawStart = offsetMap[start]!;
  const rawEnd = offsetMap[start + normalizedOld.length]!;
  const preferredEnding = firstLineTerminatorInText(rawContent.slice(rawStart, rawEnd)) ??
    lastLineTerminatorInText(rawContent.slice(0, rawStart)) ??
    firstLineTerminatorInText(rawContent.slice(rawEnd)) ??
    fallback;

  return rawContent.slice(0, rawStart) + restoreLineTerminator(normalizedNew, preferredEnding) + rawContent.slice(rawEnd);
}

function applyExactUniqueReplace(content: string, oldText: string, newText: string): string {
  const normalizedOld = normalizeToLF(oldText);
  const normalizedNew = normalizeToLF(newText);
  const start = findUniqueNormalizedMatch(content, normalizedOld);
  return content.slice(0, start) + normalizedNew + content.slice(start + normalizedOld.length);
}

export function applyEditsToContent(original: string, edits: RawEdit[]): string {
  const textEdits = edits.filter((edit) => edit.op === "replace_text");
  if (textEdits.length > 0) {
    if (edits.length !== 1) {
      throw new Error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.");
    }
    const edit = textEdits[0]!;
    if (typeof edit.oldText !== "string" || typeof edit.newText !== "string") {
      throw new Error("[E_BAD_OP] replace_text requires string oldText and newText.");
    }
    return applyExactUniqueReplace(original, edit.oldText, edit.newText);
  }

  const preserveTerminalNewline = original.endsWith("\n");
  const originalLines = getVisibleLines(original);
  const lineEdits = resolveLineEdits(edits, originalLines);
  const nextLines = applyLineEdits(originalLines, lineEdits);
  return joinVisibleLines(nextLines, preserveTerminalNewline);
}

export function applyEditsToRawContentPreservingLineEndings(
  originalRaw: string,
  edits: RawEdit[],
  options: { defaultLineEnding?: LineEnding | LineTerminator } = {},
): string {
  const fallback = resolveFallbackLineTerminator(options.defaultLineEnding);
  const textEdits = edits.filter((edit) => edit.op === "replace_text");
  if (textEdits.length > 0) {
    if (edits.length !== 1) {
      throw new Error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.");
    }
    const edit = textEdits[0]!;
    if (typeof edit.oldText !== "string" || typeof edit.newText !== "string") {
      throw new Error("[E_BAD_OP] replace_text requires string oldText and newText.");
    }
    return applyExactUniqueReplacePreservingLineEndings(originalRaw, edit.oldText, edit.newText, fallback);
  }

  const nextRecords = applyLineRecordEdits(splitTextLineRecords(originalRaw), edits, fallback);
  return joinTextLineRecords(nextRecords);
}

export function computeEditLineMetrics(original: string, edits: RawEdit[]): { addedLines: number; removedLines: number } {
  const textEdits = edits.filter((edit) => edit.op === "replace_text");
  if (textEdits.length > 0) {
    if (edits.length !== 1) {
      throw new Error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.");
    }
    const edit = textEdits[0]!;
    if (typeof edit.oldText !== "string" || typeof edit.newText !== "string") {
      throw new Error("[E_BAD_OP] replace_text requires string oldText and newText.");
    }
    return {
      addedLines: getVisibleLines(edit.newText).length,
      removedLines: getVisibleLines(edit.oldText).length,
    };
  }

  const originalLines = getVisibleLines(original);
  const lineEdits = resolveLineEdits(edits, originalLines);
  return lineEdits.reduce(
    (metrics, edit) => ({
      addedLines: metrics.addedLines + edit.lines.length,
      removedLines: metrics.removedLines + edit.end - edit.start,
    }),
    { addedLines: 0, removedLines: 0 },
  );
}

function computeChangedLineRange(oldText: string, newText: string): {
  first: number;
  last: number;
  addedLines: number;
  removedLines: number;
} | undefined {
  const oldLines = getVisibleLines(oldText);
  const newLines = getVisibleLines(newText);

  let prefix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  ) {
    prefix++;
  }

  let oldEnd = oldLines.length - 1;
  let newEnd = newLines.length - 1;
  while (
    oldEnd >= prefix &&
    newEnd >= prefix &&
    oldLines[oldEnd] === newLines[newEnd]
  ) {
    oldEnd--;
    newEnd--;
  }

  if (prefix > oldEnd && prefix > newEnd) return undefined;

  if (newLines.length === 0) {
    return {
      first: 1,
      last: 1,
      addedLines: Math.max(0, newEnd - prefix + 1),
      removedLines: Math.max(0, oldEnd - prefix + 1),
    };
  }

  const first = Math.min(prefix + 1, newLines.length);
  const last = Math.max(first, Math.min(newEnd + 1, newLines.length));
  return {
    first,
    last,
    addedLines: Math.max(0, newEnd - prefix + 1),
    removedLines: Math.max(0, oldEnd - prefix + 1),
  };
}

export function buildChangedAnchorResponse(
  original: string,
  result: string,
  options: { maxBytes?: number } = {},
): {
  text: string;
  firstChangedLine?: number;
  addedLines: number;
  removedLines: number;
} {
  const range = computeChangedLineRange(original, result);
  if (!range) {
    return {
      text: "No changes made. The requested edits produced identical content.",
      addedLines: 0,
      removedLines: 0,
    };
  }

  const resultLines = getVisibleLines(result);
  if (resultLines.length === 0) {
    return {
      text: "File is empty. Use edit with prepend or append and omit pos to insert content.",
      firstChangedLine: 1,
      addedLines: range.addedLines,
      removedLines: range.removedLines,
    };
  }

  const start = Math.max(1, range.first - 2);
  const end = Math.min(resultLines.length, range.last + 2);
  const region = resultLines.slice(start - 1, end);
  const anchors = `--- Anchors ${start}-${end} ---\n${formatHashlineRegion(region, start)}`;
  const text = Buffer.byteLength(anchors, "utf8") > (options.maxBytes ?? DEFAULT_ANCHOR_TEXT_BUDGET_BYTES)
    ? "Anchors omitted; changed region is too large. Use read for subsequent edits."
    : anchors;

  return {
    text,
    firstChangedLine: range.first,
    addedLines: range.addedLines,
    removedLines: range.removedLines,
  };
}
