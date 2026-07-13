import { constants } from "node:fs";
import { access as fsAccess } from "node:fs/promises";
import {
  createReadTool,
  DEFAULT_MAX_BYTES,
  DEFAULT_MAX_LINES,
  formatSize,
  truncateHead,
  type ExtensionAPI,
} from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";
import { resolveMutationTargetPath } from "./fs-write";
import { formatHashlineRegion, getVisibleLines } from "./hashline";
import { resolveToCwd } from "./path-utils";
import { throwIfAborted } from "./runtime";
import { isSupportedImageFile, loadTextFileWithSnapshot } from "./text-file";

function normalizePositiveInteger(value: number | undefined, name: "offset" | "limit"): number | undefined {
  if (value === undefined) return undefined;
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`Read request field "${name}" must be a positive integer.`);
  }
  return value;
}

function formatHashlineReadPreview(
  text: string,
  options: { offset?: number; limit?: number },
): { text: string; truncation?: ReturnType<typeof truncateHead>; nextOffset?: number } {
  const allLines = getVisibleLines(text);
  const totalLines = allLines.length;
  const startLine = normalizePositiveInteger(options.offset, "offset") ?? 1;

  if (totalLines === 0) {
    return {
      text: startLine === 1
        ? "File is empty. Use edit with prepend or append and omit pos to insert content."
        : `Offset ${startLine} is beyond end of file (0 lines total). The file is empty.`,
    };
  }

  if (startLine > totalLines) {
    return {
      text: `Offset ${startLine} is beyond end of file (${totalLines} lines total). Use offset=1 to read from the start, or offset=${totalLines} to read the last line.`,
    };
  }

  const limit = normalizePositiveInteger(options.limit, "limit");
  const endIndex = limit ? Math.min(startLine - 1 + limit, totalLines) : totalLines;
  const selected = allLines.slice(startLine - 1, endIndex);
  const formatted = formatHashlineRegion(selected, startLine);
  const truncation = truncateHead(formatted);

  if (truncation.firstLineExceedsLimit) {
    return {
      text: `[Line ${startLine} exceeds ${formatSize(truncation.maxBytes)}. Hashline output requires full lines; cannot compute hashes for a truncated preview.]`,
      truncation,
    };
  }

  let preview = truncation.content;
  let nextOffset: number | undefined;

  if (truncation.truncated) {
    const endLineDisplay = startLine + truncation.outputLines - 1;
    nextOffset = endLineDisplay + 1;
    preview += truncation.truncatedBy === "lines"
      ? `\n\n[Showing lines ${startLine}-${endLineDisplay} of ${totalLines}. Use offset=${nextOffset} to continue.]`
      : `\n\n[Showing lines ${startLine}-${endLineDisplay} of ${totalLines} (${formatSize(truncation.maxBytes)} limit). Use offset=${nextOffset} to continue.]`;
  } else if (endIndex < totalLines) {
    nextOffset = endIndex + 1;
    preview += `\n\n[Showing lines ${startLine}-${endIndex} of ${totalLines}. Use offset=${nextOffset} to continue.]`;
  }

  return {
    text: preview,
    ...(truncation.truncated ? { truncation } : {}),
    ...(nextOffset !== undefined ? { nextOffset } : {}),
  };
}

export function registerReadTool(pi: ExtensionAPI): void {
  pi.registerTool({
    name: "read",
    label: "Read",
    description: `Read a UTF-8 text file. Every returned line is prefixed as LINEID|content (hashline v2). Copy LINEID anchors into edit. Output is capped at ${DEFAULT_MAX_LINES} lines or ${formatSize(DEFAULT_MAX_BYTES)}. Supported images are delegated to Pi's built-in read tool.`,
    promptSnippet: "Read files with hashline v2 LINEID anchors for edit.",
    promptGuidelines: [
      "Use read before edit so you can copy full LINEID anchors exactly (e.g. 160sr).",
      "When read output is truncated, continue with the suggested offset before editing unseen lines.",
    ],
    parameters: Type.Object({
      path: Type.String({ description: "Path to the file to read (relative or absolute)" }),
      offset: Type.Optional(Type.Integer({ minimum: 1, description: "Line number to start reading from (1-indexed)" })),
      limit: Type.Optional(Type.Integer({ minimum: 1, description: "Maximum number of lines to read" })),
    }),

    renderCall(args, theme, context) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      const path = typeof args?.path === "string" ? args.path : "...";
      text.setText(`${theme.fg("toolTitle", theme.bold("read"))} ${theme.fg("accent", path)}`);
      return text;
    },

    renderResult(result, { isPartial }, theme, context) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      if (isPartial) {
        text.setText(theme.fg("warning", "Reading..."));
        return text;
      }
      const body = result.content
        ?.map((entry) => entry.type === "text" ? entry.text ?? "" : "[attachment]")
        .join("\n") ?? "";
      text.setText(context.isError ? theme.fg("error", body) : body);
      return text;
    },

    async execute(toolCallId, params, signal, onUpdate, ctx) {
      const path = params.path;
      const absolutePath = resolveToCwd(path, ctx.cwd);
      throwIfAborted(signal);

      try {
        await fsAccess(absolutePath, constants.R_OK);
      } catch (error: unknown) {
        const code = (error as NodeJS.ErrnoException).code;
        if (code === "ENOENT") throw new Error(`File not found: ${path}`);
        if (code === "EACCES" || code === "EPERM") throw new Error(`File is not readable: ${path}`);
        throw new Error(`Cannot access file: ${path}`);
      }

      if (await isSupportedImageFile(absolutePath)) {
        return createReadTool(ctx.cwd).execute(toolCallId, params, signal, onUpdate, ctx);
      }

      throwIfAborted(signal);
      const targetPath = await resolveMutationTargetPath(absolutePath);
      const file = await loadTextFileWithSnapshot(targetPath);
      const preview = formatHashlineReadPreview(file.text, {
        offset: params.offset,
        limit: params.limit,
      });

      return {
        content: [{ type: "text", text: preview.text }],
        details: {
          snapshotId: file.snapshot.snapshotId,
          ...(preview.truncation ? { truncation: preview.truncation } : {}),
          ...(preview.nextOffset !== undefined ? { nextOffset: preview.nextOffset } : {}),
        },
      };
    },
  });
}
