//! Shared provider HTTP client construction.
//!
//! `reqwest::Client` owns a connection pool and is cheap to clone. Provider
//! families share this instance unless wire behavior requires a custom TLS
//! identity or proxy configuration.

use std::sync::LazyLock;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

#[must_use]
pub fn shared_http_client() -> reqwest::Client {
    HTTP_CLIENT.clone()
}
