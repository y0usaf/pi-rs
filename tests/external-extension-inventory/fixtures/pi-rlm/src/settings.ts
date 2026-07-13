import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { getAgentDir } from "@earendil-works/pi-coding-agent";

export type RlmModelRole = "default" | "llm" | "rlm";

export interface RlmSettings {
  model?: string;
  provider?: string;
  modelId?: string;
  models?: string[];
  maxConcurrent?: number;
  maxDepth?: number;
  roleModels?: Partial<Record<RlmModelRole, string>>;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function nonEmptyString(v: unknown): string | undefined {
  return typeof v === "string" && v.trim() ? v.trim() : undefined;
}

function integerLimit(v: unknown): number | undefined {
  if (typeof v !== "number" || !Number.isFinite(v)) return undefined;
  return Math.trunc(v);
}

function providerModel(provider: unknown, model: unknown): string | undefined {
  const p = nonEmptyString(provider);
  const m = nonEmptyString(model);
  if (!m) return undefined;
  if (!p || m.includes("/")) return m;
  return `${p}/${m}`;
}

function parseModels(raw: unknown): { models?: string[]; roleModels?: Partial<Record<RlmModelRole, string>> } {
  if (Array.isArray(raw)) {
    const models = raw.map(nonEmptyString).filter((v): v is string => Boolean(v));
    return models.length ? { models } : {};
  }
  if (!isRecord(raw)) return {};

  const roleModels: Partial<Record<RlmModelRole, string>> = {};
  const def = nonEmptyString(raw.default) ?? providerModel(raw.provider, raw.modelId ?? raw.model) ?? nonEmptyString(raw.model) ?? nonEmptyString(raw.modelId);
  const llm = nonEmptyString(raw.llm) ?? nonEmptyString(raw.leaf) ?? nonEmptyString(raw.llmModel);
  const rlm = nonEmptyString(raw.rlm) ?? nonEmptyString(raw.child) ?? nonEmptyString(raw.rlmModel) ?? nonEmptyString(raw.childModel);
  if (def) roleModels.default = def;
  if (llm) roleModels.llm = llm;
  if (rlm) roleModels.rlm = rlm;
  return Object.keys(roleModels).length ? { roleModels } : {};
}

export function parseRlmSettings(raw: unknown): RlmSettings {
  if (typeof raw === "string" && raw.trim()) return { models: [raw.trim()] };
  if (!isRecord(raw)) return {};

  const model = providerModel(raw.provider, raw.modelId ?? raw.model) ?? nonEmptyString(raw.model) ?? nonEmptyString(raw.modelId);
  const parsedModels = parseModels(raw.models);
  const roleModels: Partial<Record<RlmModelRole, string>> = { ...parsedModels.roleModels };

  const llm = nonEmptyString(raw.llmModel) ?? nonEmptyString(raw.leafModel);
  const rlm = nonEmptyString(raw.rlmModel) ?? nonEmptyString(raw.childModel);
  const maxConcurrent = integerLimit(raw.maxConcurrent ?? raw.max_concurrent ?? raw.max_concurrent_subcalls);
  const maxDepth = integerLimit(raw.maxDepth ?? raw.max_depth);
  if (llm) roleModels.llm = llm;
  if (rlm) roleModels.rlm = rlm;

  return {
    model,
    provider: nonEmptyString(raw.provider),
    modelId: nonEmptyString(raw.modelId),
    models: parsedModels.models,
    maxConcurrent,
    maxDepth,
    roleModels: Object.keys(roleModels).length ? roleModels : undefined,
  };
}

function pickSettings(parsed: Record<string, unknown>): unknown {
  const extensionSettings = parsed.extensionSettings;
  if (!isRecord(extensionSettings)) return undefined;
  return extensionSettings["pi-rlm"] ?? extensionSettings.rlm;
}

function readSettingsFile(path: string): RlmSettings {
  if (!existsSync(path)) return {};
  try {
    const parsed = JSON.parse(readFileSync(path, "utf-8")) as unknown;
    if (!isRecord(parsed)) return {};
    return parseRlmSettings(pickSettings(parsed));
  } catch {
    return {};
  }
}

function mergeRoleModels(
  base?: Partial<Record<RlmModelRole, string>>,
  override?: Partial<Record<RlmModelRole, string>>,
): Partial<Record<RlmModelRole, string>> | undefined {
  const merged: Partial<Record<RlmModelRole, string>> = { ...(base ?? {}) };
  if (override) {
    for (const [role, model] of Object.entries(override) as Array<[RlmModelRole, string | undefined]>) {
      if (typeof model === "string" && model.trim()) {
        merged[role] = model.trim();
      }
    }
  }
  return Object.keys(merged).length ? merged : undefined;
}

function mergeRlmSettings(base: RlmSettings, override: RlmSettings): RlmSettings {
  const roleModels = mergeRoleModels(base.roleModels, override.roleModels);
  if (roleModels && override.roleModels?.default === undefined && (override.model !== undefined || (override.models?.length ?? 0) > 0)) {
    delete roleModels.default;
  }

  return {
    model: override.model !== undefined ? override.model : (override.models?.length ? undefined : base.model),
    provider: override.provider ?? base.provider,
    modelId: override.modelId ?? base.modelId,
    models: override.models?.length ? override.models : base.models,
    maxConcurrent: override.maxConcurrent ?? base.maxConcurrent,
    maxDepth: override.maxDepth ?? base.maxDepth,
    roleModels: roleModels && Object.keys(roleModels).length ? roleModels : undefined,
  };
}

export function loadRlmSettings(cwd: string): RlmSettings {
  return mergeRlmSettings(
    readSettingsFile(join(getAgentDir(), "settings.json")),
    readSettingsFile(join(cwd, ".pi", "settings.json")),
  );
}

export function modelSelectorForRole(settings: RlmSettings, role: RlmModelRole = "default"): string | undefined {
  return settings.roleModels?.[role]
    ?? settings.roleModels?.default
    ?? settings.model
    ?? settings.models?.[0]
    ?? providerModel(settings.provider, settings.modelId);
}
