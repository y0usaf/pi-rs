import { REPL_TOOL_NAME } from "./constants.js";
import { renderTemplate } from "./prompt-render.js";
import { ROOT_SYSTEM_PROMPT } from "./prompts.js";

// ── Root system prompt ───────────────────────────────────────────────

export function rootSystemPrompt(
  cwd: string,
  now = new Date(),
  mode = "repl",
  activeTools: string[] = [REPL_TOOL_NAME],
): string {
  const date = now.toISOString().slice(0, 10);
  return renderTemplate(ROOT_SYSTEM_PROMPT, {
    mode,
    toolName: REPL_TOOL_NAME,
    activeTools: activeTools.join(", "),
    date,
    cwd,
  });
}
