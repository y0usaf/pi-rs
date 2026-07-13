import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { resolveApiKey } from "./api.js";
import { loadSettings } from "./settings.js";

export async function updateStatus(ctx: ExtensionContext): Promise<void> {
  const settings = loadSettings(ctx.cwd);
  if (!settings.showStatus) {
    ctx.ui.setStatus("morph", undefined);
    return;
  }

  if (!settings.enabled) {
    ctx.ui.setStatus("morph", ctx.ui.theme.fg("dim", "morph:off"));
    return;
  }

  const key = await resolveApiKey(ctx, settings).catch(() => undefined);
  ctx.ui.setStatus("morph", key ? ctx.ui.theme.fg("accent", "morph") : ctx.ui.theme.fg("warning", "morph:no-key"));
}
