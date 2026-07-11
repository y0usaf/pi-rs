//! Port of `utils/oauth/pkce.ts` — PKCE code verifier + challenge.
//!
//! Spec uses the Web Crypto API; here `getrandom` + `sha2` with the same
//! base64url (no padding) encoding.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::error::AuthError;

/// Spec: the `{ verifier, challenge }` pair from `generatePKCE()`.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Spec: `generatePKCE()` — 32 random bytes base64url-encoded as the
/// verifier, SHA-256 of the verifier as the challenge.
pub fn generate_pkce() -> Result<Pkce, AuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| AuthError::Message(format!("failed to gather randomness: {e}")))?;
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// The S256 challenge for a given verifier (split out for tests).
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
