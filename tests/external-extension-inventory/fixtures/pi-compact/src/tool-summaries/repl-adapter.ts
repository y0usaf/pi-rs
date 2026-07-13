import type { ToolSummaryAdapter } from "./types.js";
import { clip, firstTextLine, squash } from "../shared.js";

export const REPL_TOOL_NAME = "repl";

const REPL_BOOTSTRAP_MODULES = new Set([
  "collections",
  "datetime",
  "glob",
  "itertools",
  "json",
  "math",
  "os",
  "pathlib",
  "re",
  "shutil",
  "subprocess",
  "sys",
  "tempfile",
  "textwrap",
  "typing",
]);

export function isReplTool(toolName: string): boolean {
  return toolName.trim().toLowerCase() === REPL_TOOL_NAME.toLowerCase();
}

function importedModuleName(part: string): string | undefined {
  const match = part.trim().match(/^([A-Za-z_][\w.]*)(?:\s+as\s+[A-Za-z_]\w*)?$/i);
  return match?.[1]?.split(".")[0];
}

export function isReplBootstrapImportLine(line: string): boolean {
  const trimmed = line.trim().replace(/\s+#.*$/, "").replace(/[;。]\s*$/, "");
  if (!trimmed || trimmed.includes(";")) return false;

  const fromMatch = trimmed.match(/^from\s+([A-Za-z_][\w.]*)\s+import\s+(.+)$/);
  if (fromMatch) {
    const moduleName = fromMatch[1].split(".")[0];
    return REPL_BOOTSTRAP_MODULES.has(moduleName);
  }

  const importMatch = trimmed.match(/^import\s+(.+)$/);
  if (!importMatch) return false;

  const modules = importMatch[1]
    .split(",")
    .map(importedModuleName)
    .filter((moduleName): moduleName is string => Boolean(moduleName));
  return modules.length > 0 && modules.every((moduleName) => REPL_BOOTSTRAP_MODULES.has(moduleName));
}

function isPythonNoopLine(line: string): boolean {
  const trimmed = line.trim();
  return trimmed === "pass" || trimmed === "...";
}

function pythonBodyLines(value: unknown): string[] {
  if (typeof value !== "string") return [];
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
}

function isOnlyReplBootstrapSetup(setup: unknown): boolean {
  const lines = pythonBodyLines(setup);
  return lines.length > 0 && lines.every(isReplBootstrapImportLine);
}

function isOnlyReplBootstrapOrNoopCode(code: unknown): boolean {
  const lines = pythonBodyLines(code);
  return lines.length > 0 && lines.every((line) => isReplBootstrapImportLine(line) || isPythonNoopLine(line));
}

function hasReplBootstrapImport(value: unknown): boolean {
  return pythonBodyLines(value).some(isReplBootstrapImportLine);
}

export function isReplBootstrapOnlyArgs(args: any): boolean {
  if (!args || typeof args !== "object") return false;
  const setupBootstraps = isOnlyReplBootstrapSetup(args.setup);
  const codeIsBootstrapOrNoop = isOnlyReplBootstrapOrNoopCode(args.code);
  return codeIsBootstrapOrNoop && (setupBootstraps || hasReplBootstrapImport(args.code));
}

export function summarizeReplCode(code: unknown, setup: unknown): string {
  if (typeof code !== "string" || !code.trim()) return "";

  const setupIsOnlyBootstrap = isOnlyReplBootstrapSetup(setup);

  let skippedBootstrap = false;
  for (const rawLine of code.split("\n")) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (isReplBootstrapImportLine(line)) {
      skippedBootstrap = true;
      continue;
    }
    if (setupIsOnlyBootstrap && isPythonNoopLine(line)) return "";
    return squash(line);
  }

  return skippedBootstrap ? "" : squash(code);
}

function cleanReplProgressLine(line: string): string {
  return squash(line)
    .replace(/^repl:\s*/i, "")
    .replace(/^rlm_query(?:_batched)?:\s*/i, "rlm ")
    .replace(/^llm_query(?:_batched)?:\s*/i, "llm ")
    .trim();
}

function formatBudgetShort(prefix: string, used: unknown, max: unknown): string | undefined {
  if (typeof used !== "number" || !Number.isFinite(used)) return undefined;
  const usedText = Math.trunc(used);
  if (typeof max === "number" && Number.isFinite(max) && max > 0) return `${prefix}${usedText}/${Math.trunc(max)}`;
  return `${prefix}${usedText}`;
}

function summarizeRlmDetails(result: any): string | undefined {
  const details = result?.details;
  const kind = details?.kind === "rlm" || details?.kind === "llm" ? details.kind : undefined;
  if (!kind) return undefined;

  const parts: string[] = [kind];
  if (details?.batch === true && typeof details?.batchSize === "number" && details.batchSize > 1) parts.push(`×${Math.trunc(details.batchSize)}`);
  if (typeof details?.depth === "number") parts.push(`d${Math.trunc(details.depth)}`);
  if (kind === "rlm" && typeof details?.turns === "number") {
    parts.push(
      typeof details?.maxTurns === "number" && Number.isFinite(details.maxTurns) && details.maxTurns > 0
        ? `t${Math.trunc(details.turns)}/${Math.trunc(details.maxTurns)}`
        : `t${Math.trunc(details.turns)}`,
    );
  }

  const calls = formatBudgetShort("c", details?.callsUsed, details?.maxCalls);
  const queries = formatBudgetShort("q", details?.queriesUsed, details?.maxQueries);
  if (calls) parts.push(calls);
  if (queries) parts.push(queries);

  const status = cleanReplProgressLine(firstTextLine(result));
  return clip([parts.join(" · "), status].filter(Boolean).join(" • "), 96);
}

export function summarizeReplProgress(result: any, args?: any): string | undefined {
  const detailed = summarizeRlmDetails(result);
  if (detailed) return detailed;

  const line = cleanReplProgressLine(firstTextLine(result));
  if (line) return clip(line, 96);

  const code = summarizeReplCode(args?.code, args?.setup);
  return code ? `running ${clip(code, 72)}` : undefined;
}

function isReplBootstrapRenderedLine(line: string): boolean {
  const plain = squash(line);
  const match = plain.match(/^repl\s+(.+)$/i);
  return Boolean(match && isReplBootstrapImportLine(match[1]));
}

function cleanReplStatusLine(line: string): string {
  const plain = squash(line);
  if (!/^[✓✗⠋]\s+repl\b/i.test(plain)) return line;
  return line.replace(/\s+vars=[^\x1b\n]*/g, "");
}

export function filterToolViewLines(lines: string[]): string[] {
  return lines
    .filter((line) => !isReplBootstrapRenderedLine(line))
    .map(cleanReplStatusLine);
}

export const replToolSummary: ToolSummaryAdapter = {
  summarizeArgs(args) {
    return summarizeReplCode(args?.code, args?.setup);
  },
};
