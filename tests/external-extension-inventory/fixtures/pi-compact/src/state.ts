import { DEFAULT_PI_COMPACT_SETTINGS, type CompactThinkingTiming, type ThinkingMode, type ThemeWithCompactColours } from "./types.js";

export const state = {
  patchPromise: undefined as Promise<boolean> | undefined,
  lastToolPatchError: undefined as string | undefined,
  lastUserPatchError: undefined as string | undefined,
  lastAssistantPatchError: undefined as string | undefined,
  lastCustomPatchError: undefined as string | undefined,
  lastConfigError: undefined as string | undefined,
  toolRendering: { ...DEFAULT_PI_COMPACT_SETTINGS.tools },
  userRendering: { ...DEFAULT_PI_COMPACT_SETTINGS.user },
  thinkingMode: DEFAULT_PI_COMPACT_SETTINGS.thinking.mode as ThinkingMode,
  activeTheme: undefined as ThemeWithCompactColours | undefined,
};

export const thinkingTimings = new Map<string, CompactThinkingTiming>();
