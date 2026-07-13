import { constants } from "node:fs";
import { access as fsAccess } from "node:fs/promises";
import { StringEnum } from "@earendil-works/pi-ai";
import { DEFAULT_MAX_BYTES, withFileMutationQueue, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";
import { resolveMutationTargetPath, writeTextFileAtomically } from "./fs-write";
import {
  applyEditsToRawContentPreservingLineEndings,
  buildChangedAnchorResponse,
  computeEditLineMetrics,
  type EditRequest,
} from "./hashline";
import { resolveToCwd } from "./path-utils";
import { throwIfAborted } from "./runtime";
import { getFileSnapshot, sameFileSnapshot } from "./snapshot";
import { isSupportedImageFile, loadTextFileWithSnapshot, normalizeToLF } from "./text-file";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const ROOT_KEYS = new Set(["path", "edits"]);
const EDIT_KEYS = new Set(["op", "pos", "end", "lines", "oldText", "newText", "loc", "content"]);

function prepareEditArguments(args: unknown): unknown {
  if (!isRecord(args) || Array.isArray(args.edits)) return args;
  const path = args.path;
  if (typeof path !== "string") return args;

  if (typeof args.oldText === "string" && typeof args.newText === "string") {
    return {
      path,
      edits: [{ op: "replace_text", oldText: args.oldText, newText: args.newText }],
    };
  }

  if (typeof args.old_text === "string" && typeof args.new_text === "string") {
    return {
      path,
      edits: [{ op: "replace_text", oldText: args.old_text, newText: args.new_text }],
    };
  }

  return args;
}

function assertEditRequest(value: unknown): asserts value is EditRequest {
  if (!isRecord(value)) throw new Error("Edit request must be an object.");
  const unknownRootKeys = Object.keys(value).filter((key) => !ROOT_KEYS.has(key));
  if (unknownRootKeys.length > 0) {
    throw new Error(`Edit request contains unknown or unsupported fields: ${unknownRootKeys.join(", ")}.`);
  }
  if (typeof value.path !== "string" || value.path.length === 0) {
    throw new Error('Edit request requires a non-empty "path" string.');
  }
  if (!Array.isArray(value.edits) || value.edits.length === 0) {
    throw new Error('Edit request requires a non-empty "edits" array.');
  }

  for (const [index, edit] of value.edits.entries()) {
    if (!isRecord(edit)) throw new Error(`Edit ${index} must be an object.`);
    const unknownEditKeys = Object.keys(edit).filter((key) => !EDIT_KEYS.has(key));
    if (unknownEditKeys.length > 0) {
      throw new Error(`Edit ${index} contains unknown or unsupported fields: ${unknownEditKeys.join(", ")}.`);
    }
    const op = edit.op;
    const hasLoc = "loc" in edit;
    if (hasLoc) {
      if (op !== undefined || "pos" in edit || "end" in edit || "lines" in edit || "oldText" in edit || "newText" in edit) {
        throw new Error(`Edit ${index} with v2 loc only supports loc and content.`);
      }
      const loc = edit.loc;
      const validBoundary = loc === "append" || loc === "prepend";
      const validObject = isRecord(loc) && (
        (typeof loc.append === "string" && Object.keys(loc).length === 1) ||
        (typeof loc.prepend === "string" && Object.keys(loc).length === 1) ||
        (isRecord(loc.range) && typeof loc.range.pos === "string" && typeof loc.range.end === "string" && Object.keys(loc).length === 1)
      );
      if (!validBoundary && !validObject) {
        throw new Error(`Edit ${index} loc must be "append", "prepend", {append}, {prepend}, or {range:{pos,end}}.`);
      }
      if (!("content" in edit)) throw new Error(`Edit ${index} requires a "content" field.`);
      if (
        edit.content !== null &&
        typeof edit.content !== "string" &&
        !(Array.isArray(edit.content) && edit.content.every((line) => typeof line === "string"))
      ) {
        throw new Error(`Edit ${index} field "content" must be a string array, string, or null.`);
      }
      continue;
    }

    if (op !== "replace" && op !== "append" && op !== "prepend" && op !== "replace_text") {
      throw new Error(`Edit ${index} uses unknown op ${JSON.stringify(op)}. Expected v2 loc/content or legacy replace, append, prepend, replace_text.`);
    }

    if ("pos" in edit && typeof edit.pos !== "string") {
      throw new Error(`Edit ${index} field "pos" must be a string when provided.`);
    }
    if ("end" in edit && typeof edit.end !== "string") {
      throw new Error(`Edit ${index} field "end" must be a string when provided.`);
    }

    if (op === "replace_text") {
      if (typeof edit.oldText !== "string" || typeof edit.newText !== "string") {
        throw new Error(`Edit ${index} with op "replace_text" requires string oldText and newText.`);
      }
      if ("pos" in edit || "end" in edit || "lines" in edit || "content" in edit) {
        throw new Error(`Edit ${index} with op "replace_text" only supports oldText and newText.`);
      }
      continue;
    }

    if (!("lines" in edit)) {
      throw new Error(`Edit ${index} requires a "lines" field.`);
    }
    if (
      edit.lines !== null &&
      typeof edit.lines !== "string" &&
      !(Array.isArray(edit.lines) && edit.lines.every((line) => typeof line === "string"))
    ) {
      throw new Error(`Edit ${index} field "lines" must be a string array, string, or null.`);
    }
    if ("oldText" in edit || "newText" in edit || "content" in edit) {
      throw new Error(`Edit ${index} with op "${op}" does not support oldText/newText/content; use loc/content or op "replace_text".`);
    }
    if (op === "replace" && typeof edit.pos !== "string") {
      throw new Error(`Edit ${index} with op "replace" requires a pos anchor.`);
    }
    if ((op === "append" || op === "prepend") && "end" in edit) {
      throw new Error(`Edit ${index} with op "${op}" does not support end.`);
    }
  }
}

export function registerEditTool(pi: ExtensionAPI): void {
  const editLinesSchema = Type.Union([
    Type.Array(Type.String(), { description: "literal replacement content lines" }),
    Type.String({ description: "literal replacement content split on newlines" }),
    Type.Null({ description: "delete target range" }),
  ]);

  const locSchema = Type.Union([
    Type.Literal("append"),
    Type.Literal("prepend"),
    Type.Object({ append: Type.String({ description: "LINEID anchor" }) }, { additionalProperties: false }),
    Type.Object({ prepend: Type.String({ description: "LINEID anchor" }) }, { additionalProperties: false }),
    Type.Object({
      range: Type.Object({
        pos: Type.String({ description: "first LINEID anchor, inclusive" }),
        end: Type.String({ description: "last LINEID anchor, inclusive" }),
      }, { additionalProperties: false }),
    }, { additionalProperties: false }),
  ]);

  const editItemSchema = Type.Object(
    {
      loc: Type.Optional(locSchema),
      content: Type.Optional(editLinesSchema),
      op: Type.Optional(StringEnum(["replace", "append", "prepend", "replace_text"] as const, {
        description: "legacy edit operation",
      })),
      pos: Type.Optional(Type.String({ description: "legacy LINEID anchor" })),
      end: Type.Optional(Type.String({ description: "legacy inclusive LINEID end anchor for replace" })),
      lines: Type.Optional(editLinesSchema),
      oldText: Type.Optional(Type.String({ description: "exact text for replace_text" })),
      newText: Type.Optional(Type.String({ description: "replacement text for replace_text" })),
    },
    { additionalProperties: false },
  );

  const editSchema = Type.Object(
    {
      path: Type.String({ description: "Path to the file to edit (relative or absolute)" }),
      edits: Type.Array(editItemSchema, { minItems: 1, description: "Hashline edits for this file" }),
    },
    { additionalProperties: false },
  );

  pi.registerTool({
    name: "edit",
    label: "Edit",
    description: [
      "Patch a UTF-8 text file using hashline v2 LINEID anchors copied from read output (e.g. 160sr).",
      "Preferred entries: {loc,content}. loc: \"append\", \"prepend\", {append:LINEID}, {prepend:LINEID}, {range:{pos,end}}.",
      "content is literal file content lines (string[]/string) or null to delete.",
      "Anchors are strict; stale hash mismatches are rejected with fresh retry anchors.",
      "Multiple anchor edits validate against the same pre-edit snapshot and apply bottom-up. Merge overlapping or adjacent edits.",
      "Legacy op/pos/end/lines and replace_text remain accepted for compatibility.",
    ].join("\n"),
    promptSnippet: "Patch files using hashline v2 LINEID anchors from read output.",
    promptGuidelines: [
      "Use read before edit; copy full LINEID anchors exactly (e.g. 160sr, not sr).",
      "Use loc/content: {range:{pos,end}} for replacements/deletes, {append}/{prepend} for inserts.",
      "Use literal file content in content lines, without LINEID| prefixes or diff prefixes.",
      "Merge overlapping or adjacent edits in the same file into one replace range.",
    ],
    parameters: editSchema,
    prepareArguments: prepareEditArguments,

    renderCall(args, theme, context) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      const path = isRecord(args) && typeof args.path === "string" ? args.path : "...";
      const count = isRecord(args) && Array.isArray(args.edits) ? args.edits.length : 0;
      const suffix = count > 0 ? theme.fg("muted", ` (${count} edit${count === 1 ? "" : "s"})`) : "";
      text.setText(`${theme.fg("toolTitle", theme.bold("edit"))} ${theme.fg("accent", path)}${suffix}`);
      return text;
    },

    renderResult(result, { isPartial }, theme, context) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      if (isPartial) {
        text.setText(theme.fg("warning", "Editing..."));
        return text;
      }
      const body = result.content
        ?.map((entry) => entry.type === "text" ? entry.text ?? "" : "")
        .filter((entry) => entry.length > 0)
        .join("\n") ?? "";
      text.setText(context.isError ? theme.fg("error", body) : body);
      return text;
    },

    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      assertEditRequest(params);
      const path = params.path;
      const absolutePath = resolveToCwd(path, ctx.cwd);
      const mutationTargetPath = await resolveMutationTargetPath(absolutePath);

      return withFileMutationQueue(mutationTargetPath, async () => {
        throwIfAborted(signal);
        const targetPath = await resolveMutationTargetPath(absolutePath);
        if (targetPath !== mutationTargetPath) {
          throw new Error("[E_PATH_CHANGED] File path resolved to a different target while waiting to edit. Re-read and retry.");
        }

        try {
          await fsAccess(targetPath, constants.R_OK | constants.W_OK);
        } catch (error: unknown) {
          const code = (error as NodeJS.ErrnoException).code;
          if (code === "ENOENT") throw new Error(`File not found: ${path}`);
          if (code === "EACCES" || code === "EPERM") throw new Error(`File is not writable: ${path}`);
          throw new Error(`Cannot access file: ${path}`);
        }

        if (await isSupportedImageFile(targetPath)) {
          throw new Error(`Path is an image file: ${path}. Hashline edit only supports UTF-8 text files.`);
        }

        throwIfAborted(signal);
        let file = await loadTextFileWithSnapshot(targetPath);
        let original = file.text;
        let snapshot = file.snapshot;
        let resultRaw = applyEditsToRawContentPreservingLineEndings(file.rawText, params.edits, {
          defaultLineEnding: file.lineEnding,
        });
        let result = normalizeToLF(resultRaw);

        if (result === original) {
          return {
            content: [{ type: "text", text: "No changes made. The requested edits produced identical content." }],
            details: { classification: "noop", snapshotId: snapshot.snapshotId },
          };
        }

        throwIfAborted(signal);
        let latestSnapshot = await getFileSnapshot(targetPath);
        if (!sameFileSnapshot(snapshot, latestSnapshot)) {
          file = await loadTextFileWithSnapshot(targetPath);
          original = file.text;
          snapshot = file.snapshot;
          resultRaw = applyEditsToRawContentPreservingLineEndings(file.rawText, params.edits, {
            defaultLineEnding: file.lineEnding,
          });
          result = normalizeToLF(resultRaw);
          if (result === original) {
            return {
              content: [{ type: "text", text: "No changes made. The requested edits produced identical content." }],
              details: { classification: "noop", snapshotId: snapshot.snapshotId },
            };
          }
          latestSnapshot = snapshot;
        }

        const persisted = file.bom + resultRaw;
        const updatedSnapshot = await writeTextFileAtomically(targetPath, persisted, {
          expectedSnapshot: latestSnapshot,
        });

        const response = buildChangedAnchorResponse(original, result, { maxBytes: DEFAULT_MAX_BYTES });
        const metrics = computeEditLineMetrics(original, params.edits);
        return {
          content: [{ type: "text", text: response.text }],
          details: {
            firstChangedLine: response.firstChangedLine,
            snapshotId: updatedSnapshot.snapshotId,
            metrics: {
              edits_attempted: params.edits.length,
              added_lines: metrics.addedLines,
              removed_lines: metrics.removedLines,
            },
          },
        };
      });
    },
  });
}
