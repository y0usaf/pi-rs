import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AgentState } from "../../ref/pi/packages/agent/src/types.ts";
import { exportSessionToHtml } from "../../ref/pi/packages/coding-agent/src/core/export-html/index.ts";
import { SessionManager } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";
import { initTheme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";

async function main() {
  const casePath = process.argv[2];
  if (!casePath) throw new Error("usage: gen-oracle.ts <case.json>");
  const fixture = JSON.parse(readFileSync(casePath, "utf8")) as {
    session: string;
    systemPrompt: string;
    tools: Array<{ name: string; description: string; parameters: unknown }>;
  };
  const dir = mkdtempSync(join(tmpdir(), "pi-export-html-parity-"));
  try {
    const sessionPath = join(dir, "session.jsonl");
    const outputPath = join(dir, "session.html");
    writeFileSync(sessionPath, fixture.session);
    initTheme("dark", false);
    const manager = SessionManager.open(sessionPath);
    const state = { systemPrompt: fixture.systemPrompt, tools: fixture.tools } as unknown as AgentState;
    await exportSessionToHtml(manager, state, { outputPath, themeName: "dark" });
    const html = readFileSync(outputPath, "utf8");
    if (process.env.EXPORT_HTML_DEBUG) writeFileSync(process.env.EXPORT_HTML_DEBUG, html);
    const encoded = html.split('<script id="session-data" type="application/json">')[1]?.split("</script>")[0];
    if (!encoded) throw new Error("session payload marker missing");
    const payload = Buffer.from(encoded, "base64").toString("utf8");
    const sha256 = (value: string) => createHash("sha256").update(value).digest("hex");
    process.stdout.write(`${JSON.stringify({ payload, htmlSha256: sha256(html) }, null, 2)}\n`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
