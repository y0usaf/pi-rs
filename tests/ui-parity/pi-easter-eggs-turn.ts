// Hidden-easter-egg driver: Pi's real animated components with deterministic
// timer state and capabilities.
import { readFileSync } from "node:fs";
import { ArminComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/armin.ts";
import { DaxnutsComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/daxnuts.ts";
import { EarendilAnnouncementComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/earendil-announcement.ts";
import { initTheme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
import { Container, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type Step = { name: string; kind: "armin" | "daxnuts" | "earendil"; tick?: number; final?: boolean; force?: boolean };
type Scenario = { columns: number; rows: number; steps: Step[] };

class CaptureTerminal implements Terminal {
  private chunks: string[] = [];
  kittyProtocolActive = true;
  constructor(public columns: number, public rows: number) {}
  start(_input: (data: string) => void, _resized: () => void): void {}
  async drainInput(): Promise<void> {}
  stop(): void {}
  write(data: string): void { this.chunks.push(data); }
  moveBy(lines: number): void { if (lines > 0) this.write(`\x1b[${lines}B`); else if (lines < 0) this.write(`\x1b[${-lines}A`); }
  hideCursor(): void { this.write("\x1b[?25l"); }
  showCursor(): void { this.write("\x1b[?25h"); }
  clearLine(): void { this.write("\x1b[K"); }
  clearFromCursor(): void { this.write("\x1b[J"); }
  clearScreen(): void { this.write("\x1b[2J\x1b[H"); }
  setTitle(): void {}
  setProgress(): void {}
  take(): string { const result = this.chunks.join(""); this.chunks = []; return result; }
}

const scenario = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Scenario;
initTheme("dark", false);
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);
const root = new Container();
ui.addChild(root);
ui.start();
const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];

const originalRandom = Math.random;
Math.random = () => 0;
const armin = new ArminComponent(ui);
Math.random = originalRandom;
armin.dispose();
const dax = new DaxnutsComponent(ui);
dax.dispose();
let arminTicks = 0;

async function main() {
  for (const step of scenario.steps) {
    root.clear();
    root.addChild(new Spacer(1));
    if (step.kind === "armin") {
      if (step.final) {
        while (!(armin as any).tickEffect()) {
          (armin as any).updateDisplay();
          arminTicks++;
        }
        (armin as any).updateDisplay();
      } else {
        while (arminTicks < (step.tick ?? 0)) {
          (armin as any).tickEffect();
          (armin as any).updateDisplay();
          arminTicks++;
        }
      }
      root.addChild(armin);
    } else if (step.kind === "daxnuts") {
      (dax as any).tick = step.tick ?? 0;
      dax.invalidate();
      root.addChild(dax);
    } else {
      root.addChild(new EarendilAnnouncementComponent());
    }
    ui.requestRender(step.force ?? false);
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
    frames.push({ name: step.name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
