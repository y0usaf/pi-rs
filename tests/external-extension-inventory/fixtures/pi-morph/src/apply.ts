import { mkdir, readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { formatSize, withFileMutationQueue, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { callAiGateway, buildPrompt, resolveApiKey } from "./api.js";
import { EXISTING_CODE_MARKER, FULL_REPLACEMENT_LINE_LIMIT } from "./constants.js";
import { resolveToCwd } from "./paths.js";
import {
  byteLength,
  detectLineEnding,
  normalizeCodeEditInput,
  normalizeLineEndings,
  summarizeChange,
  validateMergedOutput,
} from "./text.js";
import type { MorphEditParams, MorphSettings } from "./types.js";
import { throwIfAborted } from "./utils.js";

async function writeTextAtomically(path: string, content: string, mode: number): Promise<void> {
  const tempPath = join(dirname(path), `.pi-morph-${process.pid}-${Date.now()}-${Math.random().toString(36).slice(2)}.tmp`);
  try {
    await writeFile(tempPath, content, { encoding: "utf8", mode: mode & 0o7777 });
    await rename(tempPath, path);
  } catch (error) {
    await unlink(tempPath).catch(() => {});
    throw error;
  }
}

export async function applyMorphEdit(
  params: MorphEditParams,
  settings: MorphSettings,
  signal: AbortSignal | undefined,
  ctx: ExtensionContext,
) {
  const targetPath = resolveToCwd(params.target_filepath, ctx.cwd);
  const normalizedCodeEdit = normalizeCodeEditInput(params.code_edit);

  return withFileMutationQueue(targetPath, async () => {
    throwIfAborted(signal);

    let fileStat;
    try {
      fileStat = await stat(targetPath);
    } catch {
      throw new Error(`File not found: ${params.target_filepath}. Use write for new files; morph_edit edits existing files.`);
    }

    if (!fileStat.isFile()) throw new Error(`Not a regular file: ${params.target_filepath}`);
    if (fileStat.size > settings.maxFileBytes) {
      throw new Error(
        `Refusing to send ${params.target_filepath} (${formatSize(fileStat.size)}) to Morph; maxFileBytes=${formatSize(settings.maxFileBytes)}.`,
      );
    }

    const original = await readFile(targetPath, "utf8");
    const hasMarkers = normalizedCodeEdit.includes(EXISTING_CODE_MARKER);
    const originalLineCount = original.split("\n").length;
    if (!hasMarkers && !settings.allowFullReplacement && originalLineCount > FULL_REPLACEMENT_LINE_LIMIT) {
      throw new Error(
        `Missing ${JSON.stringify(EXISTING_CODE_MARKER)} markers. Without markers, Morph may replace the whole ${originalLineCount}-line file. Use markers or set allowFullReplacement=true.`,
      );
    }

    const apiKey = await resolveApiKey(ctx, settings);
    if (!apiKey) {
      throw new Error(
        `No Vercel AI Gateway API key found for provider ${JSON.stringify(settings.apiKeyProvider)}. Set AI_GATEWAY_API_KEY or store a key via Pi /login for Vercel AI Gateway.`,
      );
    }

    const prompt = buildPrompt(params.target_filepath, original, normalizedCodeEdit, params.instructions);
    const eol = detectLineEnding(original);
    const mergedRaw = await callAiGateway(settings, apiKey, prompt, signal);
    throwIfAborted(signal);

    const merged = normalizeLineEndings(mergedRaw, eol);
    validateMergedOutput(original, merged, normalizedCodeEdit, settings);

    const summary = summarizeChange(original, merged);
    if (summary.changed) {
      await mkdir(dirname(targetPath), { recursive: true });
      await writeTextAtomically(targetPath, merged, fileStat.mode);
    }

    const originalBytes = byteLength(original);
    const mergedBytes = byteLength(merged);
    const text = [
      `${summary.changed ? "Applied" : "No-op"} Morph edit to ${params.target_filepath}`,
      `${summary.oldLines} → ${summary.newLines} lines, ${formatSize(originalBytes)} → ${formatSize(mergedBytes)}`,
      "",
      summary.text,
    ].join("\n");

    return {
      content: [{ type: "text" as const, text }],
      details: {
        path: targetPath,
        model: settings.model,
        changed: summary.changed,
        oldLines: summary.oldLines,
        newLines: summary.newLines,
        oldBytes: originalBytes,
        newBytes: mergedBytes,
      },
    };
  });
}
