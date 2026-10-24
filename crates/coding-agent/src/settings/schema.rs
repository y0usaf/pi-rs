//! `core/settings-schema.ts` and `core/settings-defaults.ts`: every settings key with its
//! default, the allowed values of the enumerated ones, and the shapes the manager resolves.

use serde_json::Value;

pub mod defaults {
    pub const TRANSPORT: &str = "auto";
    pub const SESSION_SHARE_DESTINATION: &str = "auto";
    pub const STEERING_MODE: &str = "one-at-a-time";
    pub const FOLLOW_UP_MODE: &str = "one-at-a-time";
    pub const COMPACTION_ENABLED: bool = true;
    pub const COMPACTION_RESERVE_TOKENS: u64 = 16384;
    pub const COMPACTION_KEEP_RECENT_TOKENS: u64 = 20000;
    pub const BRANCH_SUMMARY_RESERVE_TOKENS: f64 = 16384.0;
    pub const BRANCH_SUMMARY_SKIP_PROMPT: bool = false;
    pub const RETRY_ENABLED: bool = true;
    pub const RETRY_MAX_RETRIES: f64 = 3.0;
    pub const RETRY_BASE_DELAY_MS: f64 = 2000.0;
    pub const RETRY_MAX_AGENT_DELAY_MS: f64 = 60000.0;
    pub const RETRY_PROVIDER_MAX_RETRY_DELAY_MS: f64 = 60000.0;
    pub const HIDE_THINKING_BLOCK: bool = false;
    pub const SHOW_CACHE_MISS_NOTICES: bool = false;
    pub const QUIET_STARTUP: bool = false;
    pub const DEFAULT_PROJECT_TRUST: &str = "ask";
    pub const COLLAPSE_CHANGELOG: bool = false;
    pub const ENABLE_INSTALL_TELEMETRY: bool = true;
    pub const ENABLE_ANALYTICS: bool = false;
    pub const ENABLE_SKILL_COMMANDS: bool = true;
    pub const TERMINAL_SHOW_IMAGES: bool = true;
    pub const TERMINAL_IMAGE_WIDTH_CELLS: f64 = 60.0;
    pub const TERMINAL_CLEAR_ON_SHRINK: bool = false;
    pub const TERMINAL_SHOW_PROGRESS: bool = false;
    pub const IMAGES_AUTO_RESIZE: bool = true;
    pub const IMAGES_BLOCK_IMAGES: bool = false;
    pub const DOUBLE_ESCAPE_ACTION: &str = "tree";
    pub const TREE_FILTER_MODE: &str = "default";
    pub const EDITOR_PADDING_X: f64 = 0.0;
    pub const OUTPUT_PAD: u8 = 1;
    pub const AUTOCOMPLETE_MAX_VISIBLE: f64 = 5.0;
    pub const MARKDOWN_CODE_BLOCK_INDENT: &str = "  ";
    pub const MARKDOWN_MERMAID: &str = "streaming";
    pub const WARNINGS_ANTHROPIC_EXTRA_USAGE: bool = true;
    pub const CODEMODE_MODE: &str = "on";
    pub const CODEMODE_INLINE_BUDGET: f64 = 3000.0;
    pub const CACHE_WARMING: &str = "streaming";
    pub const TUI_MODE: &str = "fullscreen";
    pub const FULLSCREEN_EXIT_OUTPUT: &str = "transcript";
    pub const FULLSCREEN_SCROLLBAR: &str = "auto";
    pub const FULLSCREEN_COPY_ON_SELECT: bool = true;
    pub const HTTP_IDLE_TIMEOUT_MS: f64 = 300_000.0;
}

pub const SESSION_SHARE_DESTINATIONS: [&str; 3] = ["auto", "gist", "radius"];
pub const CACHE_WARMING_MODES: [&str; 3] = ["off", "streaming", "idle"];
pub const TUI_MODES: [&str; 2] = ["regular", "fullscreen"];
pub const MERMAID_MODES: [&str; 3] = ["off", "final", "streaming"];
pub const FULLSCREEN_EXIT_OUTPUTS: [&str; 2] = ["transcript", "resume-hint"];
pub const FULLSCREEN_SCROLLBARS: [&str; 3] = ["auto", "always", "hidden"];
pub const TREE_FILTER_MODES: [&str; 5] =
    ["default", "no-tools", "user-only", "labeled-only", "all"];
pub const PROJECT_TRUSTS: [&str; 3] = ["ask", "always", "never"];

/// `Number.isSafeInteger`, as a JSON number: non-negative and exact.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: defaults::COMPACTION_ENABLED,
            reserve_tokens: defaults::COMPACTION_RESERVE_TOKENS,
            keep_recent_tokens: defaults::COMPACTION_KEEP_RECENT_TOKENS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BranchSummarySettings {
    pub reserve_tokens: f64,
    pub skip_prompt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrySettings {
    pub enabled: bool,
    pub max_retries: f64,
    pub base_delay_ms: f64,
    pub max_agent_delay_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProviderRetrySettings {
    pub timeout_ms: Option<f64>,
    pub max_retries: Option<f64>,
    pub max_retry_delay_ms: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThinkingBudgets {
    pub minimal: Option<f64>,
    pub low: Option<f64>,
    pub medium: Option<f64>,
    pub high: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarningSettings {
    pub anthropic_extra_usage: bool,
}

impl Default for WarningSettings {
    fn default() -> Self {
        Self {
            anthropic_extra_usage: defaults::WARNINGS_ANTHROPIC_EXTRA_USAGE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuietStartup {
    On,
    Header,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelScrollLines {
    Auto,
    Lines(u8),
}

/// `String(value)`: how a rejected setting appears in a diagnostic. A JSON object is
/// `[object Object]`, an array joins its items with commas (`[]` is empty).
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// A token count setting: a non-negative safe integer, or the diagnostic to report.
pub fn token_count(value: &Value, setting: &str) -> Result<u64, String> {
    value
        .as_u64()
        .filter(|count| *count <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            format!(
                "Invalid {setting} setting: {}. Expected a non-negative safe integer.",
                js_string(value)
            )
        })
}
