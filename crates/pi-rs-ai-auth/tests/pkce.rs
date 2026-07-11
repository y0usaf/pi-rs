//! Parity tests for `pkce.rs` (spec: `utils/oauth/pkce.ts`),
//! `parse_authorization_input` (spec: `parseAuthorizationInput` in
//! `utils/oauth/anthropic.ts`), and `oauth_page.rs` (spec:
//! `utils/oauth/oauth-page.ts`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai_auth::{
    challenge_for, generate_pkce, oauth_error_html, oauth_success_html, parse_authorization_input,
};

fn is_base64url(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[test]
fn pkce_shape() {
    let pkce = generate_pkce().unwrap();
    // 32 random bytes → 43 base64url chars, no padding.
    assert_eq!(pkce.verifier.len(), 43);
    assert_eq!(pkce.challenge.len(), 43);
    assert_ne!(pkce.verifier, pkce.challenge);
    assert!(is_base64url(&pkce.verifier));
    assert!(is_base64url(&pkce.challenge));
    assert_eq!(pkce.challenge, challenge_for(&pkce.verifier));
    // Two generations never collide.
    assert_ne!(pkce.verifier, generate_pkce().unwrap().verifier);
}

#[test]
fn challenge_is_s256() {
    // sha256("test") base64url-encoded, no padding.
    assert_eq!(
        challenge_for("test"),
        "n4bQgYhMfWWaL-qgxVrQFaO_TxsrC4Is0V1sFbDwCgg"
    );
}

#[test]
fn parse_authorization_input_cases() {
    // Full redirect URL.
    assert_eq!(
        parse_authorization_input("https://x.test/cb?code=abc&state=st"),
        (Some("abc".into()), Some("st".into()))
    );
    // URL without query params.
    assert_eq!(parse_authorization_input("https://x.test/cb"), (None, None));
    // Scheme-only strings are URLs in both JS and Rust (no params).
    assert_eq!(parse_authorization_input("abc:def"), (None, None));
    // code#state pair.
    assert_eq!(
        parse_authorization_input("abc#def"),
        (Some("abc".into()), Some("def".into()))
    );
    // Spec: JS `split("#", 2)` truncates — "c" is dropped, not kept.
    assert_eq!(
        parse_authorization_input("a#b#c"),
        (Some("a".into()), Some("b".into()))
    );
    // Trailing hash keeps the empty state (spec truthiness handles it).
    assert_eq!(
        parse_authorization_input("abc#"),
        (Some("abc".into()), Some(String::new()))
    );
    // Query-string syntax.
    assert_eq!(
        parse_authorization_input("code=abc&state=st"),
        (Some("abc".into()), Some("st".into()))
    );
    // Bare code, whitespace trimmed.
    assert_eq!(
        parse_authorization_input("  rawcode  "),
        (Some("rawcode".into()), None)
    );
    // Empty input.
    assert_eq!(parse_authorization_input("   "), (None, None));
}

#[test]
fn oauth_pages() {
    let success = oauth_success_html("All done. You can close this window.");
    assert!(success.contains("<title>Authentication successful</title>"));
    assert!(success.contains("<h1>Authentication successful</h1>"));
    assert!(success.contains("All done. You can close this window."));
    assert!(!success.contains(r#"<div class="details">"#));

    let error = oauth_error_html("Login <failed> & \"bad\"", Some("Error: 'denied'"));
    assert!(error.contains("<title>Authentication failed</title>"));
    // HTML is escaped like the spec's escapeHtml.
    assert!(error.contains("Login &lt;failed&gt; &amp; &quot;bad&quot;"));
    assert!(error.contains(r#"<div class="details">Error: &#39;denied&#39;</div>"#));
}
