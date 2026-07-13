import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  MAX_CONTEXT_MANIFEST_CHARS,
  MAX_CONTEXT_TREE_DEPTH,
  MAX_CONTEXT_TREE_ENTRIES,
} from "./constants.js";
import type { ContextMode, ContextSource, ContextSourceKind, ContextStore } from "./constants.js";
import { renderTemplate, xmlEscape } from "./prompt-render.js";
import { CONTEXT_STORE_PROMPT } from "./prompts.js";
import { clip, errorText, normalizeContextMode, normPaths, normSources } from "./utils.js";

// ── File-backed context store ───────────────────────────────────────

export function formatBytes(bytes?: number): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes)) return "? bytes";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let i = 1; i < units.length && value >= 1024; i++) {
    value /= 1024;
    unit = units[i];
  }
  return `${value.toFixed(value >= 10 ? 1 : 2)} ${unit}`;
}

export function absPathFor(cwd: string, input: string): string {
  return path.isAbsolute(input) ? path.normalize(input) : path.resolve(cwd, input);
}

export function relPathFor(cwd: string, abs: string): string {
  const rel = path.relative(cwd, abs);
  return rel && !rel.startsWith("..") && !path.isAbsolute(rel) ? rel : abs;
}

export function skipDirName(name: string): boolean {
  return new Set([".git", "node_modules", ".direnv", ".next", "dist", "build", "target", ".venv", "venv", "__pycache__"]).has(name);
}

export async function statContextSource(cwd: string, input: string, id: string, name?: string): Promise<ContextSource> {
  const abs = absPathFor(cwd, input);
  const relPath = relPathFor(cwd, abs);
  try {
    const st = await fs.lstat(abs);
    const kind: ContextSourceKind = st.isFile() ? "file" : st.isDirectory() ? "dir" : "other";
    return {
      id,
      name,
      label: name || input,
      input,
      path: abs,
      relPath,
      kind,
      sizeBytes: st.isFile() ? st.size : undefined,
    };
  } catch (e) {
    return {
      id,
      name,
      label: name || input,
      input,
      path: abs,
      relPath,
      kind: "missing",
      error: errorText(e),
    };
  }
}

export async function collectTreeLines(cwd: string, abs: string, depth: number, state: { count: number; truncated: boolean }): Promise<string[]> {
  if (depth > MAX_CONTEXT_TREE_DEPTH || state.count >= MAX_CONTEXT_TREE_ENTRIES) {
    state.truncated = true;
    return [];
  }

  let entries;
  try {
    entries = await fs.readdir(abs, { withFileTypes: true });
  } catch (e) {
    return [`${"  ".repeat(depth)}[cannot read ${relPathFor(cwd, abs)}: ${errorText(e)}]`];
  }

  entries.sort((a, b) => Number(b.isDirectory()) - Number(a.isDirectory()) || a.name.localeCompare(b.name));
  const lines: string[] = [];
  for (const entry of entries) {
    if (entry.isDirectory() && skipDirName(entry.name)) continue;
    if (state.count >= MAX_CONTEXT_TREE_ENTRIES) {
      state.truncated = true;
      break;
    }
    state.count++;
    const child = path.join(abs, entry.name);
    const childRel = relPathFor(cwd, child);
    let size = "";
    try {
      const st = await fs.lstat(child);
      if (st.isFile()) size = ` ${formatBytes(st.size)}`;
    } catch {
      // ignore size failures in manifest preview
    }
    lines.push(`${"  ".repeat(depth)}- ${childRel}${entry.isDirectory() ? "/" : ""}${size}`);
    if (entry.isDirectory()) {
      if (depth + 1 <= MAX_CONTEXT_TREE_DEPTH) {
        lines.push(...await collectTreeLines(cwd, child, depth + 1, state));
      } else {
        state.truncated = true;
      }
    }
  }
  return lines;
}

export function contextSourceSummary(source: ContextSource): string {
  const name = source.name ? ` (${source.name})` : "";
  const size = source.sizeBytes !== undefined ? `, ${formatBytes(source.sizeBytes)}` : "";
  const error = source.error ? `, error=${source.error}` : "";
  return `${source.id}${name}: ${source.kind} ${source.label} -> ${source.relPath}${size}${error}`;
}

export async function buildContextManifest(cwd: string, store: Omit<ContextStore, "manifestText">): Promise<string> {
  const lines: string[] = [
    "# RLM file-backed context manifest",
    "",
    `Context store: ${store.dir}`,
    `Scratch workspace: ${store.scratchDir}`,
    `Notes dir: ${store.notesDir}`,
    `Artifacts dir: ${store.artifactsDir}`,
    "",
    "Sources:",
    ...store.sources.map((s) => `- ${contextSourceSummary(s)}`),
    "",
    "Tree preview / file inventory (capped):",
  ];

  for (const source of store.sources) {
    lines.push("", `## ${source.id}: ${source.label}`);
    if (source.kind === "dir") {
      const state = { count: 0, truncated: false };
      lines.push(...await collectTreeLines(cwd, source.path, 0, state));
      if (state.truncated) lines.push(`[truncated tree after ${state.count} entries]`);
      source.entries = state.count;
    } else {
      lines.push(contextSourceSummary(source));
    }
  }

  return clip(lines.join("\n"), MAX_CONTEXT_MANIFEST_CHARS);
}

export function contextStoreReadme(store: Omit<ContextStore, "manifestText">): string {
  return `# Pi RLM temporary context store

This directory is ephemeral and deleted after the child RLM returns.

- manifest.txt: capped source manifest / tree preview
- manifest.json: machine-readable source metadata
- scratch/: write intermediate artifacts here
- notes/: note outputs
- artifacts/: artifact outputs

Use compact observations only. Do not dump whole context files into chat. In the REPL, sources are loaded as context/context_N payloads; use normal Python slicing/searching/open/os/pathlib to inspect them.

Sources:
${store.sources.map((s) => `- ${contextSourceSummary(s)}`).join("\n")}
`;
}

export async function prepareContextStore(cwd: string, params: { context?: string; contextMode?: ContextMode; paths?: string[]; sources?: Array<{ name?: string; path: string }>; contextName?: string }): Promise<ContextStore | undefined> {
  normalizeContextMode(params.contextMode);
  const paths = normPaths(params.paths);
  const namedSources = normSources(params.sources);
  const context = typeof params.context === "string" ? params.context : "";
  // Upstream-style REPL semantics: explicit context is loaded into the child
  // REPL as context/context_N. The file-backed store is only transport; the
  // model-visible API remains the REPL variables, not ctx.*.
  const materializeContext = context.trim().length > 0;
  const needsStore = paths.length > 0 || namedSources.length > 0 || materializeContext;
  if (!needsStore) return undefined;

  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "pi-rlm-"));
  try {
    const scratchDir = path.join(dir, "scratch");
    const notesDir = path.join(dir, "notes");
    const artifactsDir = path.join(dir, "artifacts");
    await fs.mkdir(scratchDir, { recursive: true });
    await fs.mkdir(notesDir, { recursive: true });
    await fs.mkdir(artifactsDir, { recursive: true });

    const sources: ContextSource[] = [];
    if (materializeContext) {
      const contextPath = path.join(dir, "inline-context.txt");
      await fs.writeFile(contextPath, context, "utf8");
      sources.push({
        id: `s${sources.length}`,
        name: typeof params.contextName === "string" && params.contextName.trim() ? params.contextName.trim() : undefined,
        label: typeof params.contextName === "string" && params.contextName.trim() ? params.contextName.trim() : "inline context",
        path: contextPath,
        relPath: contextPath,
        kind: "inline",
        sizeBytes: Buffer.byteLength(context, "utf8"),
      });
    }

    for (const p of paths) {
      sources.push(await statContextSource(cwd, p, `s${sources.length}`));
    }
    for (const src of namedSources) {
      sources.push(await statContextSource(cwd, src.path, `s${sources.length}`, src.name));
    }

    const partial = {
      dir,
      scratchDir,
      notesDir,
      artifactsDir,
      manifestPath: path.join(dir, "manifest.txt"),
      manifestJsonPath: path.join(dir, "manifest.json"),
      readmePath: path.join(dir, "README.md"),
      sources,
    };
    const manifestText = await buildContextManifest(cwd, partial);
    const store: ContextStore = { ...partial, manifestText };

    await fs.writeFile(store.manifestPath, manifestText, "utf8");
    await fs.writeFile(store.manifestJsonPath, JSON.stringify({
      dir: store.dir,
      scratchDir: store.scratchDir,
      notesDir: store.notesDir,
      artifactsDir: store.artifactsDir,
      manifestPath: store.manifestPath,
      manifestJsonPath: store.manifestJsonPath,
      sources: store.sources,
    }, null, 2), "utf8");
    await fs.writeFile(store.readmePath, contextStoreReadme(store), "utf8");
    return store;
  } catch (e) {
    await fs.rm(dir, { recursive: true, force: true }).catch(() => undefined);
    throw e;
  }
}

export async function cleanupContextStore(store?: ContextStore): Promise<void> {
  if (!store) return;
  await fs.rm(store.dir, { recursive: true, force: true }).catch(() => undefined);
}

export function contextMaterialized(store?: ContextStore): boolean {
  return Boolean(store?.sources.some((s) => s.kind === "inline"));
}

export function contextStorePromptBlock(store: ContextStore): string {
  const sourceLines = store.sources.length
    ? store.sources.map((s, i) => `    <source slot="context_${i}">${xmlEscape(contextSourceSummary(s))}</source>`).join("\n")
    : '    <source slot="none">(no sources loaded)</source>';
  return renderTemplate(CONTEXT_STORE_PROMPT, {
    tempDir: store.dir,
    scratchDir: store.scratchDir,
    notesDir: store.notesDir,
    artifactsDir: store.artifactsDir,
    manifestPath: store.manifestPath,
    manifestJsonPath: store.manifestJsonPath,
    readmePath: store.readmePath,
    sourceLines,
  });
}
