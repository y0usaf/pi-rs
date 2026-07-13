//! Shared OAuth HTTP client construction.
//!
//! OAuth remains below `pi-rs-ai` transport in the dependency graph, so token
//! exchanges use one auth-local pooled client and provider-specific requests
//! differ only in their wire data.

use std::sync::LazyLock;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

#[must_use]
pub(crate) fn shared_http_client() -> reqwest::Client {
    HTTP_CLIENT.clone()
}
