import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerEditTool } from "./edit-tool";
import { registerReadTool } from "./read-tool";

export default function (pi: ExtensionAPI): void {
  registerReadTool(pi);
  registerEditTool(pi);

  const debug = process.env.PI_HASHLINE_DEBUG;
  if (debug === "1" || debug === "true") {
    pi.on("session_start", (_event, ctx) => {
      if (ctx.hasUI) ctx.ui.notify("pi-hashline active", "info");
    });
  }
}
