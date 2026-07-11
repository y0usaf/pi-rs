//! Port of `packages/ai/src/types.ts` (spec: `ref/pi` @ `c5582102`, pi v0.79.0).
//!
//! Serde shapes are pinned by the fixtures in `tests/fixtures/` — every type
//! here must round-trip pi-produced JSON unchanged (modulo JSON number
//! representation, normalized by the test harness).
//!
//! Divergence notes:
//! - The spec's `KnownApi | (string & {})` open unions become `String` type
//!   aliases plus `KNOWN_*` constant lists (no closed enums at the seam).
//! - TS literal `type`/`role` discriminants become single-variant marker enums
//!   so deserialization *validates* the tag (serde's struct-level `tag`
//!   serializes but does not check on deserialize) and unions stay `untagged`.
//! - The options families (`StreamOptions`, `ImagesOptions`, …) carry
//!   `AbortSignal` and callbacks — runtime surface, not data. They land with
//!   the transport layer (WS2.2). The serde-able pieces they reference
//!   (`ThinkingBudgets`, `CacheRetention`, `Transport`, `ProviderResponse`)
//!   are ported here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diagnostics::AssistantMessageDiagnostic;

/// Spec: `Api = KnownApi | (string & {})`.
pub type Api = String;
/// Spec: `Provider = KnownProvider | string`.
pub type Provider = String;
/// Spec: `ImagesApi = KnownImagesApi | (string & {})`.
pub type ImagesApi = String;
/// Spec: `ImagesProvider = KnownImagesProvider | string`.
pub type ImagesProvider = String;

/// Spec: `KnownApi`.
pub const KNOWN_APIS: &[&str] = &[
    "openai-completions",
    "mistral-conversations",
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
    "anthropic-messages",
    "bedrock-converse-stream",
    "google-generative-ai",
    "google-vertex",
];

/// Spec: `KnownImagesApi`.
pub const KNOWN_IMAGES_APIS: &[&str] = &["openrouter-images"];

/// Spec: `KnownProvider`.
pub const KNOWN_PROVIDERS: &[&str] = &[
    "amazon-bedrock",
    "ant-ling",
    "anthropic",
    "google",
    "google-vertex",
    "openai",
    "azure-openai-responses",
    "openai-codex",
    "nvidia",
    "deepseek",
    "github-copilot",
    "xai",
    "groq",
    "cerebras",
    "openrouter",
    "vercel-ai-gateway",
    "zai",
    "zai-coding-cn",
    "mistral",
    "minimax",
    "minimax-cn",
    "moonshotai",
    "moonshotai-cn",
    "huggingface",
    "fireworks",
    "together",
    "opencode",
    "opencode-go",
    "kimi-coding",
    "cloudflare-workers-ai",
    "cloudflare-ai-gateway",
    "xiaomi",
    "xiaomi-token-plan-cn",
    "xiaomi-token-plan-ams",
    "xiaomi-token-plan-sgp",
];

/// Spec: `KnownImagesProvider`.
pub const KNOWN_IMAGES_PROVIDERS: &[&str] = &["openrouter"];

/// Spec: `ThinkingLevel = "minimal" | "low" | "medium" | "high" | "xhigh" | "max"`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// Spec: `ModelThinkingLevel = "off" | ThinkingLevel`.
///
/// Variant order matches the spec's `EXTENDED_THINKING_LEVELS` so the derived
/// `Ord` is the clamp order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl From<ThinkingLevel> for ModelThinkingLevel {
    fn from(level: ThinkingLevel) -> Self {
        match level {
            ThinkingLevel::Minimal => Self::Minimal,
            ThinkingLevel::Low => Self::Low,
            ThinkingLevel::Medium => Self::Medium,
            ThinkingLevel::High => Self::High,
            ThinkingLevel::XHigh => Self::XHigh,
            ThinkingLevel::Max => Self::Max,
        }
    }
}

/// Spec: `ThinkingLevelMap = Partial<Record<ModelThinkingLevel, string | null>>`.
///
/// A missing key uses provider defaults; an explicit `null` marks the level
/// unsupported — the distinction is load-bearing (see `models.rs`).
pub type ThinkingLevelMap = BTreeMap<ModelThinkingLevel, Option<String>>;

/// Spec: `ThinkingBudgets` — token budgets per thinking level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingBudgets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<u64>,
}

/// Spec: `CacheRetention = "none" | "short" | "long"`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention {
    None,
    Short,
    Long,
}

/// Spec: `Transport = "sse" | "websocket" | "websocket-cached" | "auto"`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Sse,
    Websocket,
    WebsocketCached,
    Auto,
}

/// Spec: `ProviderResponse` — status/headers handed to `onResponse`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
}

/// Spec: `TextSignatureV1["phase"]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSignaturePhase {
    Commentary,
    FinalAnswer,
}

/// Spec: `TextSignatureV1` — structured `textSignature` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSignatureV1 {
    pub v: u8,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<TextSignaturePhase>,
}

// ---------------------------------------------------------------------------
// Literal discriminant markers (`type` / `role` fields)
// ---------------------------------------------------------------------------

/// Literal `"type": "text"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TextType {
    #[default]
    #[serde(rename = "text")]
    Text,
}

/// Literal `"type": "thinking"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ThinkingType {
    #[default]
    #[serde(rename = "thinking")]
    Thinking,
}

/// Literal `"type": "image"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ImageType {
    #[default]
    #[serde(rename = "image")]
    Image,
}

/// Literal `"type": "toolCall"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ToolCallType {
    #[default]
    #[serde(rename = "toolCall")]
    ToolCall,
}

/// Literal `"role": "user"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum UserRole {
    #[default]
    #[serde(rename = "user")]
    User,
}

/// Literal `"role": "assistant"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum AssistantRole {
    #[default]
    #[serde(rename = "assistant")]
    Assistant,
}

/// Literal `"role": "toolResult"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ToolResultRole {
    #[default]
    #[serde(rename = "toolResult")]
    ToolResult,
}

// ---------------------------------------------------------------------------
// Content blocks
// ---------------------------------------------------------------------------

/// Spec: `TextContent`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    pub r#type: TextType,
    pub text: String,
    /// Spec: message metadata (legacy id string or `TextSignatureV1` JSON).
    #[serde(rename = "textSignature", skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
}

impl TextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            r#type: TextType::Text,
            text: text.into(),
            text_signature: None,
        }
    }
}

/// Spec: `ThinkingContent`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThinkingContent {
    pub r#type: ThinkingType,
    pub thinking: String,
    #[serde(rename = "thinkingSignature", skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
    /// Spec: when true, thinking was redacted by safety filters; the opaque
    /// payload lives in `thinking_signature` for multi-turn continuity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted: Option<bool>,
}

/// Spec: `ImageContent` — base64 `data` + `mimeType`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageContent {
    pub r#type: ImageType,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// Spec: `ToolCall`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub r#type: ToolCallType,
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// Spec: Google-specific opaque signature for reusing thought context.
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

/// Spec: the `TextContent | ImageContent` union (user content, tool-result
/// content, images input/output).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextOrImageContent {
    Text(TextContent),
    Image(ImageContent),
}

/// Spec: the `TextContent | ThinkingContent | ToolCall` union
/// (`AssistantMessage["content"]` elements).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssistantContent {
    Text(TextContent),
    Thinking(ThinkingContent),
    ToolCall(ToolCall),
}

// ---------------------------------------------------------------------------
// Usage / stop reasons
// ---------------------------------------------------------------------------

/// Spec: `Usage["cost"]` — dollars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// Spec: `Usage` — tokens plus cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: UsageCost,
}

/// Spec: `StopReason = "stop" | "length" | "toolUse" | "error" | "aborted"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    #[default]
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Spec: `UserMessage["content"] = string | (TextContent | ImageContent)[]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<TextOrImageContent>),
}

/// Spec: `UserMessage`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub role: UserRole,
    pub content: UserContent,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
}

impl UserMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            role: UserRole::User,
            content: UserContent::Text(text.into()),
            timestamp: now_ms(),
        }
    }
}

/// Spec: `AssistantMessage`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub role: AssistantRole,
    pub content: Vec<AssistantContent>,
    pub api: Api,
    pub provider: Provider,
    pub model: String,
    /// Spec: concrete `chunk.model` when different from the requested model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    /// Spec: provider-specific response/message identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Spec: redacted provider/runtime diagnostics for failures and recoveries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<AssistantMessageDiagnostic>>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
}

impl AssistantMessage {
    /// Spec (`utils/diagnostics.ts`): `appendAssistantMessageDiagnostic`.
    pub fn append_diagnostic(&mut self, diagnostic: AssistantMessageDiagnostic) {
        self.diagnostics
            .get_or_insert_with(Vec::new)
            .push(diagnostic);
    }
}

/// Spec: `ToolResultMessage<TDetails = any>` — `details` stays raw JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    pub role: ToolResultRole,
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<TextOrImageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub is_error: bool,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
}

/// Spec: `Message = UserMessage | AssistantMessage | ToolResultMessage`.
///
/// Untagged: the `role` marker field in each struct is the discriminant and
/// is validated on deserialize, so overlap between variants is impossible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

// ---------------------------------------------------------------------------
// Tools / context
// ---------------------------------------------------------------------------

/// Spec: `Tool<TParameters extends TSchema>` — `parameters` is a JSON Schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Spec: `Context`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

/// Spec: `ImagesContext`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImagesContext {
    pub input: Vec<TextOrImageContent>,
}

/// Spec: `ImagesStopReason = "stop" | "error" | "aborted"`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImagesStopReason {
    #[default]
    Stop,
    Error,
    Aborted,
}

/// Spec: `AssistantImages`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantImages {
    pub api: ImagesApi,
    pub provider: ImagesProvider,
    pub model: String,
    pub output: Vec<TextOrImageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub stop_reason: ImagesStopReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix timestamp in milliseconds.
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// Stream events
// ---------------------------------------------------------------------------

/// Spec: `AssistantMessageEvent`.
///
/// The spec narrows `reason` per terminal variant (`Extract<StopReason, …>`);
/// here both carry `StopReason` — producers must uphold the narrowing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantMessageEvent {
    #[serde(rename = "start")]
    Start { partial: AssistantMessage },
    #[serde(rename = "text_start", rename_all = "camelCase")]
    TextStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "text_delta", rename_all = "camelCase")]
    TextDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "text_end", rename_all = "camelCase")]
    TextEnd {
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_start", rename_all = "camelCase")]
    ThinkingStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_delta", rename_all = "camelCase")]
    ThinkingDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "thinking_end", rename_all = "camelCase")]
    ThinkingEnd {
        content_index: usize,
        content: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_start", rename_all = "camelCase")]
    ToolCallStart {
        content_index: usize,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_delta", rename_all = "camelCase")]
    ToolCallDelta {
        content_index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    #[serde(rename = "toolcall_end", rename_all = "camelCase")]
    ToolCallEnd {
        content_index: usize,
        tool_call: ToolCall,
        partial: AssistantMessage,
    },
    #[serde(rename = "done")]
    Done {
        /// Spec narrowing: `"stop" | "length" | "toolUse"`.
        reason: StopReason,
        message: AssistantMessage,
    },
    #[serde(rename = "error")]
    Error {
        /// Spec narrowing: `"aborted" | "error"`.
        reason: StopReason,
        error: AssistantMessage,
    },
}

// ---------------------------------------------------------------------------
// Compat settings
// ---------------------------------------------------------------------------

/// Spec: `OpenAICompletionsCompat["maxTokensField"]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    MaxCompletionTokens,
    MaxTokens,
}

/// Spec: `OpenAICompletionsCompat["thinkingFormat"]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingFormat {
    Openai,
    Openrouter,
    Deepseek,
    Together,
    Zai,
    Qwen,
    QwenChatTemplate,
    StringThinking,
    AntLing,
}

/// Spec: `OpenAICompletionsCompat["cacheControlFormat"]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheControlFormat {
    Anthropic,
}

/// Spec: `OpenRouterRouting["data_collection"]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataCollection {
    Deny,
    Allow,
}

/// Spec: `OpenRouterRouting` — OpenRouter provider routing preferences.
///
/// The polymorphic pass-through preferences (`sort`, `max_price`,
/// `preferred_min_throughput`, `preferred_max_latency`) stay raw JSON: pi
/// forwards them verbatim as the `provider` request field.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenRouterRouting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<DataCollection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_distillable_text: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantizations: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_min_throughput: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_max_latency: Option<serde_json::Value>,
}

/// Spec: `VercelGatewayRouting`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VercelGatewayRouting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

/// Spec: `OpenAICompletionsCompat`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAICompletionsCompat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_developer_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_usage_in_streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens_field: Option<MaxTokensField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_tool_result_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_assistant_after_tool_result: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_thinking_as_text: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_reasoning_content_on_assistant_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_format: Option<ThinkingFormat>,
    #[serde(rename = "openRouterRouting", skip_serializing_if = "Option::is_none")]
    pub open_router_routing: Option<OpenRouterRouting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vercel_gateway_routing: Option<VercelGatewayRouting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zai_tool_stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_strict_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control_format: Option<CacheControlFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_session_affinity_headers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_long_cache_retention: Option<bool>,
}

/// Spec: `OpenAIResponsesCompat`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAIResponsesCompat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_developer_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_session_id_header: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_long_cache_retention: Option<bool>,
}

/// Spec: `AnthropicMessagesCompat`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnthropicMessagesCompat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_eager_tool_input_streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_long_cache_retention: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_session_affinity_headers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_cache_control_on_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_temperature: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_adaptive_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_empty_signature: Option<bool>,
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// Spec: `Model["input"]` / `ImagesModel["output"]` element.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
}

/// Spec: `ModelCostTier` — request-wide rates above an input-token threshold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostTier {
    pub input_tokens_above: u64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Spec: `Model["cost"]` — dollars per million tokens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<ModelCostTier>,
}

/// Spec: `Model<TApi>`.
///
/// The spec's api-conditional `compat` type is runtime-erased in TS; here it
/// stays raw JSON on the model row, decoded on demand via [`Model::compat`]
/// by the protocol that knows which compat family applies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: Api,
    pub provider: Provider,
    pub base_url: String,
    pub reasoning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<ThinkingLevelMap>,
    pub input: Vec<Modality>,
    pub cost: ModelCost,
    pub context_window: u64,
    pub max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compat: Option<serde_json::Value>,
}

impl Model {
    /// Decode the `compat` field as a specific compat family
    /// ([`OpenAICompletionsCompat`], [`OpenAIResponsesCompat`], or
    /// [`AnthropicMessagesCompat`]).
    pub fn compat<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, serde_json::Error> {
        match &self.compat {
            None => Ok(None),
            Some(value) => serde_json::from_value(value.clone()).map(Some),
        }
    }
}

/// Spec: `ImagesModel<TApi>` — `Model` minus
/// `api`/`provider`/`reasoning`/`contextWindow`/`maxTokens`/`compat`, plus
/// images-specific `api`/`provider`/`output`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesModel {
    pub id: String,
    pub name: String,
    pub api: ImagesApi,
    pub provider: ImagesProvider,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<ThinkingLevelMap>,
    pub input: Vec<Modality>,
    pub cost: ModelCost,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    pub output: Vec<Modality>,
}

/// Current Unix timestamp in milliseconds (the spec's `Date.now()`).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
