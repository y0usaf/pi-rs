//! pi-rs-ai-types — shared type vocabulary for the `packages/ai` port.
//!
//! Spec: `ref/pi` @ `c5582102` (pi v0.79.0). Modules mirror the spec files:
//!
//! - [`types`] ← `packages/ai/src/types.ts`
//! - [`diagnostics`] ← `packages/ai/src/utils/diagnostics.ts`
//! - [`models`] ← `packages/ai/src/models.ts` (pure helpers; the catalog
//!   registry lands with WS2.4)
//!
//! Serde shapes are pinned by round-trip fixtures in `tests/fixtures/`
//! (recorded from pi sessions and transcribed from the generated catalogs).
//! The event-stream mechanism (`utils/event-stream.ts`) is transport, not
//! vocabulary — it lands with WS2.2.

pub mod diagnostics;
pub mod models;
pub mod types;

pub use diagnostics::{AssistantMessageDiagnostic, DiagnosticCode, DiagnosticErrorInfo};
pub use models::{
    EXTENDED_THINKING_LEVELS, calculate_cost, clamp_thinking_level, clamp_thinking_level_for,
    get_supported_thinking_levels, models_are_equal, supported_thinking_levels_for,
};
pub use types::{
    AnthropicMessagesCompat, Api, AssistantContent, AssistantImages, AssistantMessage,
    AssistantMessageEvent, AssistantRole, CacheControlFormat, CacheRetention, Context,
    DataCollection, ImageContent, ImageType, ImagesApi, ImagesContext, ImagesModel, ImagesProvider,
    ImagesStopReason, KNOWN_APIS, KNOWN_IMAGES_APIS, KNOWN_IMAGES_PROVIDERS, KNOWN_PROVIDERS,
    MaxTokensField, Message, Modality, Model, ModelCost, ModelThinkingLevel,
    OpenAICompletionsCompat, OpenAIResponsesCompat, OpenRouterRouting, Provider, ProviderResponse,
    StopReason, TextContent, TextOrImageContent, TextSignaturePhase, TextSignatureV1, TextType,
    ThinkingBudgets, ThinkingContent, ThinkingFormat, ThinkingLevel, ThinkingLevelMap,
    ThinkingType, Tool, ToolCall, ToolCallType, ToolResultMessage, ToolResultRole, Transport,
    Usage, UsageCost, UserContent, UserMessage, UserRole, VercelGatewayRouting, now_ms,
};
