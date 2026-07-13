import { join } from "node:path";
import { getAgentDir } from "@earendil-works/pi-coding-agent";
import type { JanitorSettings } from "./types.js";

export const SETTINGS_DIR = join(getAgentDir(), "context-janitor");
export const SETTINGS_PATH = join(SETTINGS_DIR, "settings.json");

export const INDEX_CUSTOM_TYPE = "context-janitor-index";
export const RESTORE_CUSTOM_TYPE = "context-janitor-restore";
export const SUMMARY_CUSTOM_TYPE = "context-janitor-summary";
export const NOTICE_CUSTOM_TYPE = "context-janitor-notice";
export const STATUS_KEY = "context-janitor";
export const PI_COMPACT_GLOBAL_KEY = "__piCompactEnabled";
export const JANITOR_CUSTOM_TYPES = new Set([INDEX_CUSTOM_TYPE, RESTORE_CUSTOM_TYPE, SUMMARY_CUSTOM_TYPE, NOTICE_CUSTOM_TYPE]);

// Keep projected tool results protocol-valid while adding no visible transcript text.
export const CONTEXT_HIDDEN_TEXT = "\u200B";
export const DEBOUNCE_MS = 900;
export const HYSTERESIS_MIN_TOOL_CALLS = 6;
export const HYSTERESIS_MIN_RAW_CHARS = 16_000;
export const HYSTERESIS_MAX_AGE_MS = 60_000;
export const HYSTERESIS_RECHECK_MS = 5_000;
export const MAX_DECIDER_INPUT_CHARS = 60_000;
export const MAX_RECORDS_PER_PASS = 24;
export const MAX_DECIDER_TOKENS = 1_000;

export const STATUS_ENABLED_IDLE = "janitor ⣿";
export const STATUS_DISABLED = "janitor";
export const STATUS_SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
export const STATUS_SPINNER_MS = 120;

export const DECIDER_SYSTEM_PROMPT = `You are Context Janitor, a conservative background context cleaner for a coding agent.

You receive JSON objects representing completed tool results. Each object has an id and a hash. Decide which tool-result outputs are safe to replace with a hidden placeholder in future model context.

Output JSON only:
{"actions":[{"target":{"id":"...","hash":"..."},"action":"truncate|keep","reason":"..."}]}

Policy:
- Truncate only operational clutter: duplicate/noisy output, progress logs, stale failed attempts that were corrected, typo commands, irrelevant exploration, or huge output with no durable fact.
- Keep unresolved errors, the latest test/build/lint result, file contents/snippets likely needed, command outputs with side effects, permission/network failures, and anything uncertain.
- Be conservative. If unsure, keep.
- Never invent ids or hashes. Use only the provided id/hash pairs.`;

export const DEFAULT_SETTINGS: JanitorSettings = {
	enabled: true,
};

export const AUTO_MODEL_CANDIDATES = [
	{ provider: "openai", modelId: "gpt-5.4-mini" },
	{ provider: "anthropic", modelId: "claude-haiku-4-5" },
	{ provider: "vercel-ai-gateway", modelId: "openai/gpt-5-nano" },
] as const;
