//! Port of `providers/openai-prompt-cache.ts`.

/// Spec: `OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH`.
pub const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH: usize = 64;

/// Spec: `clampOpenAIPromptCacheKey` — truncate to 64 code points
/// (`Array.from(key)` splits on Unicode code points, as `chars()` does).
pub fn clamp_openai_prompt_cache_key(key: Option<&str>) -> Option<String> {
    let key = key?;
    if key.chars().count() <= OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH {
        return Some(key.to_string());
    }
    Some(
        key.chars()
            .take(OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH)
            .collect(),
    )
}
