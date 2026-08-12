// Generates tests/platform-clipboard-parity/oracle.json by driving Pi's real
// `packages/coding-agent/src/utils/clipboard-native.ts`:
//   - `loadClipboardNative(requires?)` over scripted require roots — the
//     deterministic resolution core that decides addon-available vs fallback;
//   - the module-level `clipboard` gating (`!TERMUX_VERSION && hasDisplay`)
//     by importing Pi's module under manipulated env on this (linux) base.
// Run via scripts/platform-clipboard-oracle.
import { writeFileSync } from "node:fs";
import {
  loadClipboardNative,
  type ClipboardModule,
} from "../../ref/pi/packages/coding-agent/src/utils/clipboard-native.ts";

const fakeModule: ClipboardModule = {
  setText: async () => {},
  hasImage: () => false,
  getImageBinary: async () => [],
};
type FakeRequire = (name: string) => unknown;
// Builds an array of independent require closures matching Pi's `requires`
// argument: 0 = resolves the addon, 1 = throws (a real require on a bad root).
const buildRequires = (roots: Array<0 | 1>): FakeRequire[] =>
  roots.map((kind) => (name) => {
    if (name !== "@mariozechner/clipboard") return undefined;
    if (kind === 0) return fakeModule;
    throw new Error("module not found");
  });

const loadCases: Array<{ name: string; roots: Array<0 | 1> }> = [
  { name: "single-resolves", roots: [0] },
  { name: "single-absent", roots: [1] },
  { name: "first-fails-second-resolves", roots: [1, 0] },
  { name: "both-fail", roots: [1, 1] },
  { name: "third-resolves", roots: [1, 1, 0] },
];

const loadProbe = loadCases.map(({ name, roots }) => {
  const out = loadClipboardNative(buildRequires(roots));
  return {
    name,
    resolved: out !== null,
    shape: out
      ? {
          setText: typeof out.setText,
          hasImage: typeof out.hasImage,
          getImageBinary: typeof out.getImageBinary,
        }
      : null,
  };
});

// Empty require list: no resolution root — returns null, must not throw.
let noRootOut: unknown = "unreached";
try {
  noRootOut = loadClipboardNative([]) === null ? "null" : "non-null";
} catch (e) {
  noRootOut = `threw: ${e instanceof Error ? e.message : String(e)}`;
}

// Module-level gating on this linux base: `clipboard = !TERMUX_VERSION &&
// hasDisplay ? loadClipboardNative() : null`. Probe Pi's module in a fresh
// process per env state (probe-clipboard.ts) so the module-level initializer
// re-evaluates — pi-tsx/ESM caches the module across imports in one process.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const envCases: Array<{ name: string; termux: null | string; display: null | string; wayland: null | string }> = [
  { name: "linux-no-display", termux: null, display: null, wayland: null },
  { name: "linux-x11", termux: null, display: ":0", wayland: null },
  { name: "linux-wayland", termux: null, display: null, wayland: "wayland-1" },
  { name: "linux-termux", termux: "0.118", display: ":0", wayland: null },
];
const probePath = fileURLToPath(new URL("./probe-clipboard.ts", import.meta.url));
const envProbe: Array<{ name: string; clipboardLoaded: boolean }> = [];
for (const c of envCases) {
  const env: Record<string, string | undefined> = { ...process.env };
  if (c.termux === null) delete env.TERMUX_VERSION; else env.TERMUX_VERSION = c.termux;
  if (c.display === null) delete env.DISPLAY; else env.DISPLAY = c.display;
  if (c.wayland === null) delete env.WAYLAND_DISPLAY; else env.WAYLAND_DISPLAY = c.wayland;
  const proc = spawnSync("ref/pi/node_modules/.bin/tsx", ["--tsconfig", "ref/pi/tsconfig.json", probePath], {
    cwd: process.cwd(),
    env,
    encoding: "utf8",
  });
  let clipboardLoaded = false;
  if (proc.status === 0) {
    try {
      clipboardLoaded = JSON.parse(proc.stdout.trim()).clipboardLoaded;
    } catch {
      // Leave false; report the raw output for diagnosis.
    }
  }
  envProbe.push({ name: c.name, clipboardLoaded });
}
async function main() {
  const out = {
    oracle: "Pi v0.79.0 c5582102",
    platform: process.platform,
    loadProbe,
    noRootOut,
    envProbe,
  };
  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
  console.log(`wrote ${process.argv[2]}`);
}
void main();