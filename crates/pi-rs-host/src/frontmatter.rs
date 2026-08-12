//! YAML frontmatter parser for markdown files (PLAN 9.7).
//!
//! Skills (SKILL.md), prompts (PROMPT.md), and resources use embedded
//! YAML frontmatter delimited by `---\n` lines.  This module exposes the
//! Rust parser and the Lua binding `pi.parse_frontmatter(content)`.

use serde_json::Value;

/// A document split into optional YAML frontmatter and the remaining body.
#[derive(Debug, Clone)]
pub(crate) struct FrontmatterDocument {
    /// Parsed frontmatter as JSON, or `None` when no frontmatter exists.
    pub(crate) frontmatter: Option<Value>,
    /// The body after the closing `---\n` delimiter (trimmed), or the full
    /// text when no frontmatter was detected.
    pub(crate) body: String,
}

/// Parse YAML frontmatter from markdown text.
///
/// Recognises `---\n` as opening delimiter only at the very start of text.
/// The closing delimiter is the next `---\n`. Everything between is YAML.
///
/// Edge cases:
/// - No frontmatter: frontmatter=None, body=full text.
/// - No closing `---`: treated as no frontmatter (full text as body).
/// - Empty frontmatter (nothing between delimiters): frontmatter=None.
/// - Only a bare `---\n` with nothing after: frontmatter=None.
/// - Invalid YAML: frontmatter wraps an error object, body preserved.
pub(crate) fn parse_frontmatter(text: &str) -> FrontmatterDocument {
    if !text.starts_with("---\n") {
        return FrontmatterDocument {
            frontmatter: None,
            body: text.to_owned(),
        };
    }

    let rest = &text[4..];

    // Find closing `---\n` in the remainder.
    // We search for `---\n` (not `\n---\n`) to handle empty content
    // where the closing delimiter immediately follows the opening one.
    let end = if let Some(pos) = rest.find("---\n") {
        // Found a `---\n` in the rest. The yaml content is everything
        // from text[4..] up to the start of this match.
        // Convert rest-relative position to text-absolute: 4 + pos
        4 + pos
    } else {
        // No closing delimiter found -- treat as no frontmatter.
        return FrontmatterDocument {
            frontmatter: None,
            body: text.to_owned(),
        };
    };

    // yaml_raw is the content between the two delimiters (exclusive)
    let yaml_raw = &text[4..end];
    // body_start is past the closing `---\n` (3 dashes + 1 newline = 4 bytes)
    let body_start = end + 4;
    let body = if body_start >= text.len() {
        String::new()
    } else {
        text[body_start..].trim().to_owned()
    };

    if yaml_raw.trim().is_empty() {
        return FrontmatterDocument {
            frontmatter: None,
            body,
        };
    }

    let frontmatter = match serde_yaml::from_str::<Value>(yaml_raw) {
        Ok(val) => Some(val),
        Err(e) => Some(serde_json::json!({ "_error": e.to_string() })),
    };

    FrontmatterDocument { frontmatter, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let doc = parse_frontmatter("hello world");
        assert!(doc.frontmatter.is_none());
        assert_eq!(doc.body, "hello world");
    }

    #[test]
    fn basic_frontmatter() {
        let text = "---\nname: my-skill\ndescription: Does something useful\n---\n\nBody text here.\n";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("my-skill"));
        assert_eq!(fm.get("description").and_then(|v| v.as_str()), Some("Does something useful"));
        assert_eq!(doc.body, "Body text here.");
    }

    #[test]
    fn empty_frontmatter() {
        let text = "---\n---\n\nBody.\n";
        let doc = parse_frontmatter(text);
        assert!(doc.frontmatter.is_none());
        assert_eq!(doc.body, "Body.");
    }

    #[test]
    fn no_closing_delimiter() {
        let text = "---\nname: skill\nNo closing delimiter here.";
        let doc = parse_frontmatter(text);
        assert!(doc.frontmatter.is_none());
        assert_eq!(doc.body, text);
    }

    #[test]
    fn only_delimiter() {
        let text = "---\n";
        let doc = parse_frontmatter(text);
        assert!(doc.frontmatter.is_none());
        assert_eq!(doc.body, text);
    }

    #[test]
    fn body_without_newline_trailing() {
        let text = "---\nname: test\n---\nBody without trailing newline";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("test"));
        assert_eq!(doc.body, "Body without trailing newline");
    }

    #[test]
    fn multiline_yaml() {
        let text = "---\ndescription: |\n  Multi-line\n  text.\n---\nBody";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("description").and_then(|v| v.as_str()), Some("Multi-line\ntext.\n"));
        assert_eq!(doc.body, "Body");
    }

    #[test]
    fn integer_value() {
        let text = "---\ncount: 42\n---\nBody";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("count").and_then(|v| v.as_i64()), Some(42));
    }

    #[test]
    fn boolean_value() {
        let text = "---\nenabled: true\n---\nBody";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("enabled").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn nested_map() {
        let text = "---\nkey: value\nnested:\n  inner: val\n---\nBody";
        let doc = parse_frontmatter(text);
        let fm = doc.frontmatter.expect("expected frontmatter");
        assert_eq!(fm.get("nested").and_then(|v| v.get("inner")).and_then(|v| v.as_str()), Some("val"));
    }
}
