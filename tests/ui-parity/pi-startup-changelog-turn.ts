// Startup changelog driver: Pi's real changelog utilities and startup notice
// component composition against pi-rs's embedded Lua policy.
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { DynamicBorder } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/dynamic-border.ts";
import { getMarkdownTheme, initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { getNewEntries, normalizeChangelogLinks, parseChangelog } from "../../ref/pi/packages/coding-agent/src/utils/changelog.ts";
import { Markdown } from "../../ref/pi/packages/tui/src/components/markdown.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

class CaptureTerminal implements Terminal {
  private input?: (data: string) => void; private resized?: () => void; private chunks: string[] = [];
  kittyProtocolActive = true;
  constructor(public columns: number, public rows: number) {}
  start(input: (data: string) => void, resized: () => void): void { this.input = input; this.resized = resized; }
  async drainInput(): Promise<void> {} stop(): void {}
  write(data: string): void { this.chunks.push(data); }
  moveBy(lines: number): void { if (lines > 0) this.write(`\x1b[${lines}B`); else if (lines < 0) this.write(`\x1b[${-lines}A`); }
  hideCursor(): void { this.write("\x1b[?25l"); } showCursor(): void { this.write("\x1b[?25h"); }
  clearLine(): void { this.write("\x1b[K"); } clearFromCursor(): void { this.write("\x1b[J"); }
  clearScreen(): void { this.write("\x1b[2J\x1b[H"); } setTitle(): void {} setProgress(): void {}
  send(data: string): void { this.input?.(data); }
  resize(columns: number, rows: number): void { this.columns = columns; this.rows = rows; this.resized?.(); }
  take(): string { const result = this.chunks.join(""); this.chunks = []; return result; }
}

type Step = { name: string; fresh?: boolean; lastVersion?: string; collapsed?: boolean; resumed?: boolean; force?: boolean; release?: { version: string; note?: string } };
type Scenario = { columns: number; rows: number; version: string; changelogText: string; steps: Step[] };
const scenario = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Scenario;
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
initTheme("dark", false);
const changelogPath = join(tmpdir(), `pi-startup-changelog-${process.pid}.md`);
writeFileSync(changelogPath, scenario.changelogText);
const entries = parseChangelog(changelogPath);
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const root = new Container();
ui.addChild(root);
ui.start();
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];

async function main() {
  for (const step of scenario.steps) {
    root.clear();
    if (step.release) {
      const action = theme.fg("accent", "pi update");
      const instruction = theme.fg("muted", `New version ${step.release.version} is available. Run `) + action;
      root.addChild(new Spacer(1));
      root.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
      root.addChild(new Text(`${theme.bold(theme.fg("warning", "Update Available"))}\n${instruction}`, 1, 0));
      if (step.release.note?.trim()) {
        root.addChild(new Spacer(1));
        root.addChild(new Markdown(step.release.note.trim(), 1, 0, getMarkdownTheme(), {
          color: (text) => theme.fg("muted", text),
        }));
        root.addChild(new Spacer(1));
      }
      root.addChild(new Text(theme.fg("muted", "Changelog: ") + theme.fg("accent", "https://pi.dev/changelog"), 1, 0));
      root.addChild(new DynamicBorder((text) => theme.fg("warning", text)));
    } else {
      const resumed = step.resumed ?? false;
      const lastVersion = step.fresh ? undefined : step.lastVersion;
      if (!resumed && lastVersion !== undefined) {
        const newer = getNewEntries(entries, lastVersion);
        if (newer.length > 0) {
          const markdown = newer.map((entry) => normalizeChangelogLinks(entry.content, entry)).join("\n\n");
          root.addChild(new DynamicBorder());
          if (step.collapsed) {
            const match = markdown.match(/##\s+\[?(\d+\.\d+\.\d+)\]?/);
            const latest = match ? match[1] : scenario.version;
            root.addChild(new Text(`Updated to v${latest}. Use ${theme.bold("/changelog")} to view full changelog.`, 1, 0));
          } else {
            root.addChild(new Text(theme.bold(theme.fg("accent", "What's New")), 1, 0));
            root.addChild(new Spacer(1));
            root.addChild(new Markdown(markdown.trim(), 1, 0, getMarkdownTheme()));
            root.addChild(new Spacer(1));
          }
          root.addChild(new DynamicBorder());
        }
      }
    }
    ui.requestRender(step.force ?? false);
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
    frames.push({ name: step.name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
  }
  ui.stop();
  rmSync(changelogPath, { force: true });
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => {
  rmSync(changelogPath, { force: true });
  console.error(error);
  process.exitCode = 1;
});
