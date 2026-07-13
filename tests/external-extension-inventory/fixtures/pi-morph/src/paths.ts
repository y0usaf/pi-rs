import { isAbsolute, resolve } from "node:path";

export function expandPath(filePath: string): string {
  const normalized = filePath.startsWith("@") ? filePath.slice(1) : filePath;
  if (normalized === "~") return process.env.HOME ?? normalized;
  if (normalized.startsWith("~/")) return `${process.env.HOME ?? "~"}${normalized.slice(1)}`;
  return normalized;
}

export function resolveToCwd(filePath: string, cwd: string): string {
  const expanded = expandPath(filePath);
  return isAbsolute(expanded) ? expanded : resolve(cwd, expanded);
}
