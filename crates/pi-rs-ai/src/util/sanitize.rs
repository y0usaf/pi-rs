//! Port of `utils/sanitize-unicode.ts`.

/// Spec: `sanitizeSurrogates` — strips unpaired UTF-16 surrogates, which
/// break JSON serialization at many providers.
///
/// Rust strings are valid UTF-8 and *cannot* contain unpaired surrogates,
/// so this is the identity function. It exists so every spec call site
/// has a literal counterpart here, keeping the port `diff`-auditable; the
/// actual stripping happens where surrogates could enter — see
/// `util::json_parse`, which drops unpaired surrogate escapes when
/// decoding partial JSON strings.
pub fn sanitize_surrogates(text: &str) -> &str {
    text
}
