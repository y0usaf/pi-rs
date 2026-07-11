//! highlight.js 10.7.3 port — the syntax-highlighting mechanism behind
//! `pi.hljs` (spec: Pi's coding agent depends on the library through
//! `utils/syntax-highlight.ts`; the vendored copy is the oracle,
//! `tests/hljs-parity/`).
//!
//! Split, like the jsdiff port, along Pi's own third-party boundary: the
//! library engine is Rust mechanism; everything Pi wrote on top of it
//! (`renderHighlightedHtml`, theme mapping, language validation policy)
//! stays Lua. Grammars are data (`data/hljs-grammars.json`), serialized
//! from the vendored library by `scripts/hljs-grammars` after the library
//! compiled them itself — see `grammar.rs` and PLAN.md 2b.3 for the scoped
//! language boundary.

mod grammar;
mod parse;

use grammar::Registry;

/// Typed errors for the highlight layer.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HljsError {
    /// The embedded grammar catalog failed to load.
    #[error("hljs catalog: {0}")]
    Catalog(String),

    /// A grammar regex failed to translate or execute.
    #[error("hljs regex: {0}")]
    Regex(String),

    /// `highlight()` with an unregistered language (the library throws).
    #[error("Unknown language: \"{0}\"")]
    UnknownLanguage(String),
}

/// `HighlightResult`, reduced to the fields Pi's coding agent observes.
pub struct Highlighted {
    /// The highlighted HTML markup (`result.value`).
    pub value: String,
    /// Whether an illegal match aborted highlighting.
    pub illegal: bool,
    /// Relevance score (floored to an integer on success).
    pub relevance: f64,
    /// Detected/requested language, when highlighting succeeded.
    pub language: Option<String>,
}

/// Compile a JS regex source through the engine's JS→fancy-regex
/// translation (shared mechanism: the session selector's `re:` search
/// mode needs `new RegExp(pattern, "i")` semantics).
pub(crate) fn js_regex(
    source: &str,
    case_insensitive: bool,
) -> Result<grammar::JsRegex, HljsError> {
    grammar::JsRegex::without_multiline(source, case_insensitive)
}

/// `hljs.getLanguage(name) !== undefined` (used by Pi's `supportsLanguage`).
pub fn supports_language(name: &str) -> bool {
    Registry::global()
        .map(|registry| registry.language(name).is_some())
        .unwrap_or(false)
}

/// Registered canonical language names, in registration order.
pub fn list_languages() -> Result<Vec<String>, HljsError> {
    Ok(Registry::global()?
        .languages
        .iter()
        .map(|lang| lang.name.clone())
        .collect())
}

/// `hljs.highlight(code, { language, ignoreIllegals })`. An unknown
/// language is an error, exactly like the library.
pub fn highlight(
    code: &str,
    language: &str,
    ignore_illegals: bool,
) -> Result<Highlighted, HljsError> {
    let registry = Registry::global()?;
    let Some((lang, _)) = registry.language(language) else {
        return Err(HljsError::UnknownLanguage(language.to_owned()));
    };
    let mut ctx = parse::Ctx::new(registry);
    let inner = parse::highlight_language(&mut ctx, lang, code, ignore_illegals, None)?;
    Ok(Highlighted {
        value: inner.value,
        illegal: inner.illegal,
        relevance: inner.relevance,
        language: inner.language,
    })
}

/// `hljs.highlightAuto(code, languageSubset)`.
pub fn highlight_auto(code: &str, subset: Option<&[String]>) -> Result<Highlighted, HljsError> {
    let registry = Registry::global()?;
    let mut ctx = parse::Ctx::new(registry);
    let inner = parse::highlight_auto(&mut ctx, code, subset)?;
    Ok(Highlighted {
        value: inner.value,
        illegal: inner.illegal,
        relevance: inner.relevance,
        language: inner.language,
    })
}
