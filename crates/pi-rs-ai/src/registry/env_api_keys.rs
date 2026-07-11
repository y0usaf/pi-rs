//! Port of `env-api-keys.ts` — API keys from known environment
//! variables, plus the ambient-credential checks for google-vertex
//! (Application Default Credentials) and amazon-bedrock (AWS credential
//! sources).
//!
//! Compression notes:
//! - The spec's `/proc/self/environ` fallback works around a Bun
//!   compiled-binary bug (`process.env` empty in sandboxes); `std::env`
//!   has no such failure mode, so it is not represented.
//! - The spec caches the vertex ADC existence check (with a retry dance
//!   around Node's async module loading); the check here is cheap and
//!   uncached — behavior is identical, only the fs-stat count differs.

use std::path::Path;

/// Spec: `getApiKeyEnvVars(provider)` — the env-var name(s) that can
/// hold a provider's API key, in precedence order.
fn get_api_key_env_vars(provider: &str) -> Option<&'static [&'static str]> {
    let vars: &'static [&'static str] = match provider {
        "github-copilot" => &["COPILOT_GITHUB_TOKEN"],
        // ANTHROPIC_OAUTH_TOKEN takes precedence over ANTHROPIC_API_KEY
        "anthropic" => &["ANTHROPIC_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
        "ant-ling" => &["ANT_LING_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "azure-openai-responses" => &["AZURE_OPENAI_API_KEY"],
        "nvidia" => &["NVIDIA_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "google" => &["GEMINI_API_KEY"],
        "google-vertex" => &["GOOGLE_CLOUD_API_KEY"],
        "groq" => &["GROQ_API_KEY"],
        "cerebras" => &["CEREBRAS_API_KEY"],
        "xai" => &["XAI_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        "vercel-ai-gateway" => &["AI_GATEWAY_API_KEY"],
        "zai" => &["ZAI_API_KEY"],
        "zai-coding-cn" => &["ZAI_CODING_CN_API_KEY"],
        "mistral" => &["MISTRAL_API_KEY"],
        "minimax" => &["MINIMAX_API_KEY"],
        "minimax-cn" => &["MINIMAX_CN_API_KEY"],
        "moonshotai" => &["MOONSHOT_API_KEY"],
        "moonshotai-cn" => &["MOONSHOT_API_KEY"],
        "huggingface" => &["HF_TOKEN"],
        "fireworks" => &["FIREWORKS_API_KEY"],
        "together" => &["TOGETHER_API_KEY"],
        "opencode" => &["OPENCODE_API_KEY"],
        "opencode-go" => &["OPENCODE_API_KEY"],
        "kimi-coding" => &["KIMI_API_KEY"],
        "cloudflare-workers-ai" => &["CLOUDFLARE_API_KEY"],
        "cloudflare-ai-gateway" => &["CLOUDFLARE_API_KEY"],
        "xiaomi" => &["XIAOMI_API_KEY"],
        "xiaomi-token-plan-cn" => &["XIAOMI_TOKEN_PLAN_CN_API_KEY"],
        "xiaomi-token-plan-ams" => &["XIAOMI_TOKEN_PLAN_AMS_API_KEY"],
        "xiaomi-token-plan-sgp" => &["XIAOMI_TOKEN_PLAN_SGP_API_KEY"],
        _ => return None,
    };
    Some(vars)
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Spec: `hasVertexAdcCredentials` — `GOOGLE_APPLICATION_CREDENTIALS`
/// first, then the default ADC path.
fn has_vertex_adc_credentials() -> bool {
    if let Some(gac_path) = env_non_empty("GOOGLE_APPLICATION_CREDENTIALS") {
        return Path::new(&gac_path).exists();
    }
    let Some(home) = env_non_empty("HOME") else {
        return false;
    };
    Path::new(&home)
        .join(".config/gcloud/application_default_credentials.json")
        .exists()
}

/// Spec: `findEnvKeys(provider)` — configured env-var names that can
/// provide an API key. Only actual API-key variables; ambient credential
/// sources (AWS profiles, ADC) are intentionally excluded.
pub fn find_env_keys(provider: &str) -> Option<Vec<&'static str>> {
    let vars = get_api_key_env_vars(provider)?;
    let found: Vec<&'static str> = vars
        .iter()
        .copied()
        .filter(|var| env_non_empty(var).is_some())
        .collect();
    if found.is_empty() { None } else { Some(found) }
}

/// Spec: `getEnvApiKey(provider)` — the API key from known environment
/// variables; `"<authenticated>"` for the ambient-credential providers
/// (google-vertex ADC, amazon-bedrock AWS sources).
pub fn get_env_api_key(provider: &str) -> Option<String> {
    if let Some(keys) = find_env_keys(provider)
        && let Some(first) = keys.first()
    {
        return env_non_empty(first);
    }

    // Vertex AI supports either an explicit API key or Application
    // Default Credentials (`gcloud auth application-default login`).
    if provider == "google-vertex" {
        let has_credentials = has_vertex_adc_credentials();
        let has_project = env_non_empty("GOOGLE_CLOUD_PROJECT").is_some()
            || env_non_empty("GCLOUD_PROJECT").is_some();
        let has_location = env_non_empty("GOOGLE_CLOUD_LOCATION").is_some();
        if has_credentials && has_project && has_location {
            return Some("<authenticated>".to_owned());
        }
    }

    if provider == "amazon-bedrock" {
        // Amazon Bedrock supports multiple credential sources:
        // 1. AWS_PROFILE - named profile from ~/.aws/credentials
        // 2. AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY - standard IAM keys
        // 3. AWS_BEARER_TOKEN_BEDROCK - Bedrock bearer token
        // 4. AWS_CONTAINER_CREDENTIALS_RELATIVE_URI - ECS task roles
        // 5. AWS_CONTAINER_CREDENTIALS_FULL_URI - ECS task roles (full URI)
        // 6. AWS_WEB_IDENTITY_TOKEN_FILE - IRSA (IAM Roles for Service Accounts)
        let configured = env_non_empty("AWS_PROFILE").is_some()
            || (env_non_empty("AWS_ACCESS_KEY_ID").is_some()
                && env_non_empty("AWS_SECRET_ACCESS_KEY").is_some())
            || env_non_empty("AWS_BEARER_TOKEN_BEDROCK").is_some()
            || env_non_empty("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_some()
            || env_non_empty("AWS_CONTAINER_CREDENTIALS_FULL_URI").is_some()
            || env_non_empty("AWS_WEB_IDENTITY_TOKEN_FILE").is_some();
        if configured {
            return Some("<authenticated>".to_owned());
        }
    }

    None
}
