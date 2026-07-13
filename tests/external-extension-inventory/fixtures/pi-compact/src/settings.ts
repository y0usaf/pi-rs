import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { getAgentDir } from "@earendil-works/pi-coding-agent";
import { state } from "./state.js";
import { DEFAULT_PI_COMPACT_SETTINGS, type GapRenderingMode, type PiCompactSettings, type ResolvedPiCompactSettings, type ThinkingMode, type ThinkingSettings, type ToolsSettings, type UserSettings } from "./types.js";
import { isRecord } from "./shared.js";

function hasSettings(value: object): boolean {
  return Object.keys(value).length > 0;
}

export function parseGapRenderingMode(value: unknown): GapRenderingMode | undefined {
  if (typeof value !== "string") return undefined;

  switch (value.trim().toLowerCase()) {
    case "normal":
      return "normal";
    case "borderless":
      return "borderless";
    case "compact":
      return "compact";
    case "hidden":
    case "hide":
    case "off":
      return "hidden";
    default:
      return undefined;
  }
}

export function parseThinkingMode(value: unknown): ThinkingMode | undefined {
  if (typeof value !== "string") return undefined;

  switch (value.trim().toLowerCase()) {
    case "normal":
      return "normal";
    case "compact":
      return "compact";
    case "hidden":
    case "hide":
    case "off":
      return "hidden";
    default:
      return undefined;
  }
}

export function parseToolsSettings(raw: unknown): Partial<ToolsSettings> | undefined {
  if (!isRecord(raw)) return undefined;

  const settings: Partial<ToolsSettings> = {};
  const mode = parseGapRenderingMode(raw.mode);
  if (mode) settings.mode = mode;
  if (typeof raw.gap === "boolean") settings.gap = raw.gap;
  return hasSettings(settings) ? settings : undefined;
}

export function parseUserSettings(raw: unknown): Partial<UserSettings> | undefined {
  if (!isRecord(raw)) return undefined;

  const settings: Partial<UserSettings> = {};
  const mode = parseGapRenderingMode(raw.mode);
  if (mode) settings.mode = mode;
  if (typeof raw.gap === "boolean") settings.gap = raw.gap;
  return hasSettings(settings) ? settings : undefined;
}

export function parseThinkingSettings(raw: unknown): Partial<ThinkingSettings> | undefined {
  if (!isRecord(raw)) return undefined;

  const settings: Partial<ThinkingSettings> = {};
  const mode = parseThinkingMode(raw.mode);
  if (mode) settings.mode = mode;
  return hasSettings(settings) ? settings : undefined;
}

export function parseSettings(raw: unknown): Partial<PiCompactSettings> {
  if (!isRecord(raw)) return {};

  const settings: Partial<PiCompactSettings> = {};
  const tools = parseToolsSettings(raw.tools);
  const user = parseUserSettings(raw.user);
  const thinking = parseThinkingSettings(raw.thinking);

  if (tools) settings.tools = tools;
  if (user) settings.user = user;
  if (thinking) settings.thinking = thinking;
  return settings;
}

export function pickSettings(parsed: Record<string, unknown>): unknown {
  const extensionSettings = parsed.extensionSettings;
  if (!isRecord(extensionSettings)) return undefined;
  return extensionSettings["pi-compact"];
}

export function readSettingsFile(path: string): Partial<PiCompactSettings> {
  if (!existsSync(path)) return {};
  try {
    const parsed = JSON.parse(readFileSync(path, "utf-8")) as unknown;
    if (!isRecord(parsed)) return {};
    return parseSettings(pickSettings(parsed));
  } catch (error) {
    state.lastConfigError = error instanceof Error ? error.stack ?? error.message : String(error);
    return {};
  }
}

export function mergePiCompactSettings(...items: Partial<PiCompactSettings>[]): Partial<PiCompactSettings> {
  const merged: Partial<PiCompactSettings> = {};

  for (const item of items) {
    if (item.tools) merged.tools = { ...merged.tools, ...item.tools };
    if (item.user) merged.user = { ...merged.user, ...item.user };
    if (item.thinking) merged.thinking = { ...merged.thinking, ...item.thinking };
  }

  return merged;
}

export function readPiCompactSettings(cwd: string): Partial<PiCompactSettings> {
  try {
    state.lastConfigError = undefined;
    return mergePiCompactSettings(
      readSettingsFile(join(getAgentDir(), "settings.json")),
      readSettingsFile(join(cwd, ".pi", "settings.json")),
    );
  } catch (error) {
    state.lastConfigError = error instanceof Error ? error.stack ?? error.message : String(error);
    return {};
  }
}

export function resolvePiCompactSettings(cwd: string): ResolvedPiCompactSettings {
  const settings = readPiCompactSettings(cwd);
  return {
    tools: { ...DEFAULT_PI_COMPACT_SETTINGS.tools, ...settings.tools },
    user: { ...DEFAULT_PI_COMPACT_SETTINGS.user, ...settings.user },
    thinking: { ...DEFAULT_PI_COMPACT_SETTINGS.thinking, ...settings.thinking },
  };

}
