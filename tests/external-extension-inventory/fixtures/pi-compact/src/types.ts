export const MAX_SUMMARY_LENGTH = 120;
export const MAX_RESULT_LENGTH = 72;
export const MAX_USER_MESSAGE_LENGTH = 512;
export const TOOL_RULE = "╱";
export const USER_PROMPT_MARKER = ":::";
export const THINKING_MARKER = "🧠";
export const JANITOR_MARKER = "🧹";

export const JANITOR_INDEX_CUSTOM_TYPE = "context-janitor-index";
export const JANITOR_RESTORE_CUSTOM_TYPE = "context-janitor-restore";
export const JANITOR_SUMMARY_CUSTOM_TYPE = "context-janitor-summary";
export const JANITOR_NOTICE_CUSTOM_TYPE = "context-janitor-notice";
export const JANITOR_CUSTOM_TYPES = new Set([JANITOR_INDEX_CUSTOM_TYPE, JANITOR_RESTORE_CUSTOM_TYPE, JANITOR_SUMMARY_CUSTOM_TYPE, JANITOR_NOTICE_CUSTOM_TYPE]);
export const PI_COMPACT_GLOBAL_KEY = "__piCompactEnabled";

export const OSC133_ZONE_START = "\x1b]133;A\x07";
export const OSC133_ZONE_END = "\x1b]133;B\x07";
export const OSC133_ZONE_FINAL = "\x1b]133;C\x07";

export type GapRenderingMode = "normal" | "borderless" | "compact" | "hidden";
export type ThinkingMode = "normal" | "compact" | "hidden";
export type ToolBgToken = "toolPendingBg" | "toolSuccessBg" | "toolErrorBg";
export type ThemeFgToken = "toolDiffAdded" | "toolDiffRemoved" | "muted";

export interface GapRendering {
  mode: GapRenderingMode;
  gap: boolean;
}

export type ToolsSettings = GapRendering;
export type UserSettings = GapRendering;

export interface ThinkingSettings {
  mode: ThinkingMode;
}

export interface PiCompactSettings {
  tools?: Partial<ToolsSettings>;
  user?: Partial<UserSettings>;
  thinking?: Partial<ThinkingSettings>;
}

export interface ResolvedPiCompactSettings {
  tools: ToolsSettings;
  user: UserSettings;
  thinking: ThinkingSettings;
}

export interface CompactThinkingState {
  charCount: number;
  startedAtMs: number;
  completedAtMs?: number;
  stopReason?: string;
}

export interface CompactThinkingTiming {
  startedAtMs?: number;
  completedAtMs?: number;
}

export type ThemeWithCompactColours = {
  bg(color: ToolBgToken, text: string): string;
  fg(color: ThemeFgToken, text: string): string;
};

export const DEFAULT_PI_COMPACT_SETTINGS: ResolvedPiCompactSettings = {
  tools: { mode: "compact", gap: false },
  user: { mode: "borderless", gap: true },
  thinking: { mode: "compact" },
};

export const TOOL_ORIGINAL_RENDER_KEY = "__piCompactOriginalToolRender";
export const TOOL_ORIGINAL_SET_EXPANDED_KEY = "__piCompactOriginalToolSetExpanded";
export const USER_ORIGINAL_RENDER_KEY = "__piCompactOriginalUserRender";
export const ASSISTANT_ORIGINAL_RENDER_KEY = "__piCompactOriginalAssistantRender";
export const CUSTOM_ORIGINAL_RENDER_KEY = "__piCompactOriginalCustomRender";
export const ASSISTANT_ORIGINAL_UPDATE_CONTENT_KEY = "__piCompactOriginalAssistantUpdateContent";
export const ASSISTANT_THINKING_STATE_KEY = "__piCompactThinkingState";
export const ASSISTANT_THINKING_APPLIED_MODE_KEY = "__piCompactThinkingAppliedMode";
export const ASSISTANT_THINKING_TIMING_KEY = "__piCompactThinkingTiming";
export const ANSI_PATTERN = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\)|_[^\x07]*(?:\x07|\x1b\\))/g;
export const FULL_SGR_RESET_PATTERN = /\x1b\[(?:0)?m/g;
export const BG_MARKER = "__pi_compact_bg_marker__";
export const BRAILLE_SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
export const TOOL_SPINNER_INTERVAL_MS = 80;
export const TOOL_SPINNER_INTERVAL_KEY = "__piCompactToolSpinnerInterval";
export const TOOL_SPINNER_FRAME_KEY = "__piCompactToolSpinnerFrame";
