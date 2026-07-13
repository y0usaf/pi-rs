import { Text } from "@earendil-works/pi-tui";
import { formatSize, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { resolveApiKey } from "./api.js";
import { applyMorphEdit } from "./apply.js";
import { DEFAULT_MODEL, EXISTING_CODE_MARKER } from "./constants.js";
import { appendMorphRoutingHint, buildMorphRoutingHint } from "./routing.js";
import { loadSettings } from "./settings.js";
import { updateStatus } from "./status.js";
import type { MorphEditParams } from "./types.js";

const morphEditSchema = Type.Object(
  {
    target_filepath: Type.String({ description: "Path of the existing file to modify" }),
    instructions: Type.String({
      description:
        "Brief first-person description of the intended edit, e.g. 'I am adding request logging to the middleware setup.'",
    }),
    code_edit: Type.String({
      description: `Partial code edit using ${JSON.stringify(EXISTING_CODE_MARKER)} markers for unchanged sections. Include unique context around each changed region.`,
    }),
  },
  { additionalProperties: false },
);

export default function piMorph(pi: ExtensionAPI) {
  pi.on("session_start", async (_event, ctx) => {
    await updateStatus(ctx);
  });

  pi.on("model_select", async (_event, ctx) => {
    await updateStatus(ctx);
  });

  pi.on("before_provider_request", async (event, ctx) => {
    const settings = loadSettings(ctx.cwd);
    const key = await resolveApiKey(ctx, settings).catch(() => undefined);
    return appendMorphRoutingHint(event.payload, buildMorphRoutingHint(settings, Boolean(key)));
  });

  pi.registerCommand("morph-status", {
    description: "Show pi-morph configuration and Vercel AI Gateway key status",
    handler: async (_args, ctx) => {
      const settings = loadSettings(ctx.cwd);
      const key = await resolveApiKey(ctx, settings).catch(() => undefined);
      const lines = [
        `pi-morph: ${settings.enabled ? "enabled" : "disabled"}`,
        `model: ${settings.model}`,
        `baseUrl: ${settings.baseUrl}`,
        `apiKeyProvider: ${settings.apiKeyProvider}`,
        `key: ${key ? "available" : "missing"}`,
        `maxFileBytes: ${formatSize(settings.maxFileBytes)}`,
        `maxOutputBytes: ${formatSize(settings.maxOutputBytes)}`,
        `allowFullReplacement: ${settings.allowFullReplacement}`,
        "config: ~/.pi/agent/settings.json#extensionSettings.morph, .pi/settings.json#extensionSettings.morph",
      ];
      ctx.ui.notify(lines.join("\n"), key && settings.enabled ? "info" : "warning");
      await updateStatus(ctx);
    },
  });

  pi.registerTool({
    name: "morph_edit",
    label: "Morph Edit",
    description: [
      `Edit an existing UTF-8 file using Morph via Vercel AI Gateway (${DEFAULT_MODEL} by default).`,
      `Provide a partial code_edit with ${JSON.stringify(EXISTING_CODE_MARKER)} markers for unchanged sections; Morph merges it into the full file.`,
      "Best for large files, multiple scattered changes, repetitive structures, or ambiguous exact replacements.",
      "Use Pi's regular edit for small exact changes and write for new files.",
      "The tool validates marker leakage, destructive truncation, and configured output size before writing.",
      "Credentials use Pi's normal Vercel AI Gateway provider lookup (AI_GATEWAY_API_KEY or auth.json provider vercel-ai-gateway).",
    ].join("\n"),
    promptSnippet: "Merge partial code edits into existing files via Morph on Vercel AI Gateway",
    promptGuidelines: [
      "Use morph_edit for large, scattered, whitespace-sensitive, repetitive, or ambiguous edits inside an existing file.",
      "Use morph_edit with code_edit wrapped by // ... existing code ... markers at both start and end so unchanged code is preserved.",
      "Use morph_edit with 1-2 unique context lines around each edited region to disambiguate repeated patterns.",
      "Use regular edit for small exact replacements and write for new files instead of morph_edit.",
      "If morph_edit fails, retry with more concrete context or fall back to regular edit.",
    ],
    parameters: morphEditSchema,

    renderCall(args: MorphEditParams, theme: any, context: any) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      text.setText(`${theme.fg("toolTitle", theme.bold("morph_edit"))} ${theme.fg("accent", args.target_filepath ?? "...")}`);
      return text;
    },

    renderResult(result, { isPartial }: { isPartial: boolean }, theme: any, context: any) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      if (isPartial) {
        text.setText(theme.fg("warning", "Morph merging..."));
        return text;
      }
      const body =
        result.content
          ?.map((entry: { type: string; text?: string }) => (entry.type === "text" ? entry.text ?? "" : ""))
          .filter((entry: string) => entry.length > 0)
          .join("\n") ?? "";
      text.setText(context.isError ? theme.fg("error", body) : body);
      return text;
    },

    async execute(_toolCallId, params: MorphEditParams, signal, onUpdate, ctx) {
      const settings = loadSettings(ctx.cwd);
      if (!settings.enabled) throw new Error("pi-morph is disabled by extensionSettings.morph.enabled=false.");

      onUpdate?.({ content: [{ type: "text" as const, text: `Morph merging ${params.target_filepath}...` }] });
      return applyMorphEdit(params, settings, signal, ctx);
    },
  });
}
