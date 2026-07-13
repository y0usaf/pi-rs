import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { PI_COMPACT_GLOBAL_KEY, DEFAULT_PI_COMPACT_SETTINGS } from "./types.js";
import { state } from "./state.js";
import { cloneGapRendering } from "./shared.js";
import { resolvePiCompactSettings } from "./settings.js";
import { registerJanitorMessageRenderers } from "./custom-messages.js";
import { recordAssistantThinkingTimingForEvent } from "./thinking-rendering.js";
import { hasStatusError, parseGapRenderingArg, parseThinkingArg, patchPiCompactComponents, statusMessage } from "./patching.js";

void patchPiCompactComponents();

export default function (pi: ExtensionAPI) {
  (globalThis as Record<string, unknown>)[PI_COMPACT_GLOBAL_KEY] = true;
  registerJanitorMessageRenderers(pi);

  pi.on("message_update", (event) => {
    recordAssistantThinkingTimingForEvent(event);
  });

  pi.on("message_end", (event) => {
    recordAssistantThinkingTimingForEvent(event, true);
  });

  pi.registerCommand("compact-status", {
    description: "Show pi-compact patch status",
    handler: async (_args, ctx) => {
      await patchPiCompactComponents();
      ctx.ui.notify(statusMessage(), hasStatusError() ? "error" : "info");
    },
  });

  pi.registerCommand("compact-user", {
    description: "Set user message rendering (normal|borderless|borderless-tight|compact|compact-tight|hidden)",
    handler: async (args, ctx) => {
      const next = parseGapRenderingArg(args, state.userRendering, DEFAULT_PI_COMPACT_SETTINGS.user);
      if (next === undefined) {
        ctx.ui.notify("Usage: /compact-user [normal|borderless|borderless-tight|compact|compact-tight|hidden|gap|no-gap|toggle|cycle|status]", "error");
        return;
      }

      state.userRendering = next;
      await patchPiCompactComponents();
      ctx.ui.notify(statusMessage(), hasStatusError() ? "error" : "info");
    },
  });

  pi.registerCommand("compact-tools", {
    description: "Set tool rendering (normal|borderless|borderless-tight|compact|compact-tight|hidden)",
    handler: async (args, ctx) => {
      const next = parseGapRenderingArg(args, state.toolRendering, DEFAULT_PI_COMPACT_SETTINGS.tools);
      if (next === undefined) {
        ctx.ui.notify("Usage: /compact-tools [normal|borderless|borderless-tight|compact|compact-tight|hidden|gap|no-gap|toggle|cycle|status]", "error");
        return;
      }

      state.toolRendering = next;
      await patchPiCompactComponents();
      ctx.ui.notify(statusMessage(), hasStatusError() ? "error" : "info");
    },
  });

  pi.registerCommand("compact-thinking", {
    description: "Set thinking rendering (normal|compact|hidden|toggle)",
    handler: async (args, ctx) => {
      const next = parseThinkingArg(args, state.thinkingMode);
      if (next === undefined) {
        ctx.ui.notify("Usage: /compact-thinking [normal|compact|hidden|toggle|status]", "error");
        return;
      }

      state.thinkingMode = next;
      await patchPiCompactComponents();
      ctx.ui.notify(statusMessage(), hasStatusError() ? "error" : "info");
    },
  });

  pi.on("session_start", async (_event, ctx) => {
    const settings = resolvePiCompactSettings(ctx.cwd);
    state.toolRendering = cloneGapRendering(settings.tools);
    state.userRendering = cloneGapRendering(settings.user);
    state.thinkingMode = settings.thinking.mode;
    state.activeTheme = ctx.hasUI ? ctx.ui.theme : undefined;

    await patchPiCompactComponents();
    if (!ctx.hasUI) return;
    if (hasStatusError()) ctx.ui.notify(statusMessage(), "error");
  });
}

