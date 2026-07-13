import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { getAgentDir } from "@earendil-works/pi-coding-agent";
import {
  DEFAULT_API_KEY_PROVIDER,
  DEFAULT_BASE_URL,
  DEFAULT_MAX_FILE_BYTES,
  DEFAULT_MAX_OUTPUT_BYTES,
  DEFAULT_MODEL,
  EXTENSION_SETTINGS_KEY,
} from "./constants.js";
import type { MorphSettings } from "./types.js";
import { isRecord, parsePositiveInteger } from "./utils.js";

export const DEFAULT_SETTINGS: MorphSettings = {
  enabled: true,
  model: DEFAULT_MODEL,
  baseUrl: DEFAULT_BASE_URL,
  apiKeyProvider: DEFAULT_API_KEY_PROVIDER,
  maxFileBytes: DEFAULT_MAX_FILE_BYTES,
  maxOutputBytes: DEFAULT_MAX_OUTPUT_BYTES,
  allowFullReplacement: false,
  showStatus: true,
};

export function parseSettings(raw: unknown): Partial<MorphSettings> {
  if (typeof raw === "boolean") return { enabled: raw };
  if (!isRecord(raw)) return {};

  const out: Partial<MorphSettings> = {};
  if (typeof raw.enabled === "boolean") out.enabled = raw.enabled;
  if (typeof raw.model === "string" && raw.model.trim()) out.model = raw.model.trim();
  if (typeof raw.baseUrl === "string" && raw.baseUrl.trim()) out.baseUrl = raw.baseUrl.trim().replace(/\/+$/, "");
  if (typeof raw.apiKeyProvider === "string" && raw.apiKeyProvider.trim()) out.apiKeyProvider = raw.apiKeyProvider.trim();
  if (typeof raw.allowFullReplacement === "boolean") out.allowFullReplacement = raw.allowFullReplacement;
  if (typeof raw.showStatus === "boolean") out.showStatus = raw.showStatus;

  const maxFileBytes = parsePositiveInteger(raw.maxFileBytes);
  if (maxFileBytes !== undefined) out.maxFileBytes = maxFileBytes;
  const maxOutputBytes = parsePositiveInteger(raw.maxOutputBytes);
  if (maxOutputBytes !== undefined) out.maxOutputBytes = maxOutputBytes;

  if (isRecord(raw.provider)) out.provider = raw.provider;
  if (isRecord(raw.providerOptions)) out.providerOptions = raw.providerOptions;

  return out;
}

function pickSettings(parsed: Record<string, unknown>): unknown {
  const extensionSettings = parsed.extensionSettings;
  if (!isRecord(extensionSettings)) return undefined;
  return extensionSettings[EXTENSION_SETTINGS_KEY] ?? extensionSettings["pi-morph"];
}

function readSettingsFile(path: string): Partial<MorphSettings> {
  if (!existsSync(path)) return {};
  try {
    const parsed = JSON.parse(readFileSync(path, "utf-8")) as unknown;
    if (!isRecord(parsed)) return {};
    return parseSettings(pickSettings(parsed));
  } catch {
    return {};
  }
}

export function loadSettings(cwd: string): MorphSettings {
  return {
    ...DEFAULT_SETTINGS,
    ...readSettingsFile(join(getAgentDir(), "settings.json")),
    ...readSettingsFile(join(cwd, ".pi", "settings.json")),
  };
}
