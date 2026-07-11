//! Port of `providers/cloudflare.ts`.

use pi_rs_ai_types::Model;

use super::ProtocolError;

/// Workers AI direct endpoint.
pub const CLOUDFLARE_WORKERS_AI_BASE_URL: &str =
    "https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1";

/// AI Gateway Unified API.
pub const CLOUDFLARE_AI_GATEWAY_COMPAT_BASE_URL: &str =
    "https://gateway.ai.cloudflare.com/v1/{CLOUDFLARE_ACCOUNT_ID}/{CLOUDFLARE_GATEWAY_ID}/compat";

/// AI Gateway → OpenAI passthrough.
pub const CLOUDFLARE_AI_GATEWAY_OPENAI_BASE_URL: &str =
    "https://gateway.ai.cloudflare.com/v1/{CLOUDFLARE_ACCOUNT_ID}/{CLOUDFLARE_GATEWAY_ID}/openai";

/// AI Gateway → Anthropic passthrough.
pub const CLOUDFLARE_AI_GATEWAY_ANTHROPIC_BASE_URL: &str = "https://gateway.ai.cloudflare.com/v1/{CLOUDFLARE_ACCOUNT_ID}/{CLOUDFLARE_GATEWAY_ID}/anthropic";

/// Spec: `isCloudflareProvider`.
pub fn is_cloudflare_provider(provider: &str) -> bool {
    provider == "cloudflare-workers-ai" || provider == "cloudflare-ai-gateway"
}

/// Spec: `resolveCloudflareBaseUrl` — substitute `{VAR}` placeholders in
/// a Cloudflare baseUrl from the environment. Placeholder names match
/// the spec's `\{([A-Z_][A-Z0-9_]*)\}`; anything else stays literal.
pub fn resolve_cloudflare_base_url(model: &Model) -> Result<String, ProtocolError> {
    let url = &model.base_url;
    if !url.contains('{') {
        return Ok(url.clone());
    }

    let mut resolved = String::with_capacity(url.len());
    let mut rest = url.as_str();
    while let Some(open) = rest.find('{') {
        resolved.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if is_placeholder_name(&after[..close]) => {
                let name = &after[..close];
                let value = std::env::var(name).unwrap_or_default();
                if value.is_empty() {
                    return Err(ProtocolError(format!(
                        "{name} is required for provider {} but is not set.",
                        model.provider
                    )));
                }
                resolved.push_str(&value);
                rest = &after[close + 1..];
            }
            _ => {
                resolved.push('{');
                rest = after;
            }
        }
    }
    resolved.push_str(rest);
    Ok(resolved)
}

fn is_placeholder_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
