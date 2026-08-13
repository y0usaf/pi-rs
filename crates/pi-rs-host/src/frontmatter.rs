//! YAML frontmatter parser for markdown files (PLAN 9.7).
//!
//! Skills (SKILL.md), prompts (PROMPT.md), and resources use embedded
//! YAML frontmatter delimited by `---` lines.  This module is a faithful
//! port of Pi's `utils/frontmatter.ts` (`parseFrontmatter`), pinned by
//! `tests/frontmatter-parity` against Pi's real implementation.  The Lua
//! binding is `pi.parse_frontmatter(content)`.

use serde_json::Value;

/// A document split into YAML frontmatter and the remaining body.
#[derive(Debug, Clone)]
pub(crate) struct FrontmatterDocument {
    /// Parsed frontmatter as JSON. Pi returns `{}` when no frontmatter
    /// (or empty) was present, so this is always an object on success.
    pub(crate) frontmatter: serde_json::Map<String, Value>,
    /// The body after the closing delimiter (trimmed), or the normalized
    /// full text when no frontmatter was detected.
    pub(crate) body: String,
}

/// Pi (`extractFrontmatter`): normalize `\r\n` then `\r` to `\n`.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Parse YAML frontmatter from markdown text, matching Pi's
/// `parseFrontmatter` exactly. Returns `Err` only when the embedded YAML
/// fails to parse (Pi's `yaml.parse` throws in that case).
pub(crate) fn parse_frontmatter(text: &str) -> Result<FrontmatterDocument, String> {
    let normalized = normalize_newlines(text);

    // Pi: `if (!normalized.startsWith("---"))` => { {}, normalized }.
    if !normalized.starts_with("---") {
        return Ok(FrontmatterDocument {
            frontmatter: serde_json::Map::new(),
            body: normalized,
        });
    }

    // Pi: `normalized.indexOf("\n---", 3)` — the closing delimiter is a
    // newline followed by three dashes. Search from index 3 is equivalent
    // to a plain search here because `---` occupies indices 0..2 when the
    // document starts with `---`, so no `\n---` can occur before index 3.
    let end_index = normalized.find("\n---");

    // Pi: no closing delimiter => { {}, normalized }.
    let Some(end_index) = end_index else {
        return Ok(FrontmatterDocument {
            frontmatter: serde_json::Map::new(),
            body: normalized,
        });
    };

    // Pi: `yamlString = normalized.slice(4, endIndex)` (indices 0..3 are
    // the opening `---`; index 3 is the newline Pi consumes by reading from
    // byte 4); `body = normalized.slice(endIndex + 4).trim()` (past the
    // newline + three closing dashes). JS `slice(begin, end)` returns "" when
    // `end <= begin`, so `endIndex <= 4` yields an empty yaml string in Rust
    // as well (only the `---\n---` and `---\n\n---` cases).
    let yaml_raw = if end_index <= 4 {
        ""
    } else {
        &normalized[4..end_index]
    };
    let body = normalized[end_index + 4..].trim().to_owned();

    // Pi: `if (!yamlString) return { frontmatter: {}, body }`.
    if yaml_raw.trim().is_empty() {
        return Ok(FrontmatterDocument {
            frontmatter: serde_json::Map::new(),
            body,
        });
    }

    // Pi: `parse(yamlString)`; a null result becomes `{}`. Parse errors
    // propagate (Pi's yaml library throws).
    //
    // Chomping parity: Pi's `yaml` package (eemeli/yaml, YAML 1.2 clip
    // chomping) ALWAYS retains one trailing newline for `|`/`>` block
    // scalars, even when the last content line has no line break in the
    // source. serde_yaml clips based on whether the source ends in a
    // newline. In frontmatter the yamlString slice excludes the closing
    // `\n---`, so its final content line never carries the newline; the two
    // parsers therefore disagree on block-scalar values. Appending the
    // one line break the closing delimiter consumes makes serde_yaml's clip
    // chomping match eemeli/yaml's (a trailing newline is a no-op for flow,
    // plain, quoted, and empty scalars, and matches `|`/`>` keep/strip).
    let parsed =
        serde_yaml::from_str::<Value>(&format!("{yaml_raw}\n")).map_err(|e| e.to_string())?;
    let frontmatter = parsed.as_object().cloned().unwrap_or_default();
    Ok(FrontmatterDocument { frontmatter, body })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn parse(text: &str) -> FrontmatterDocument {
        parse_frontmatter(text).expect("frontmatter parses")
    }

    #[test]
    fn no_frontmatter_returns_empty_object_full_body() {
        let doc = parse("hello world");
        assert!(doc.frontmatter.is_empty());
        assert_eq!(doc.body, "hello world");
    }

    #[test]
    fn basic_frontmatter() {
        let doc = parse(
            "---\nname: my-skill\ndescription: Does something useful\n---\n\nBody text here.\n",
        );
        assert_eq!(
            doc.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("my-skill")
        );
        assert_eq!(doc.body, "Body text here.");
    }

    #[test]
    fn empty_frontmatter_returns_empty_object() {
        let doc = parse("---\n---\n\nBody.\n");
        assert!(doc.frontmatter.is_empty());
        assert_eq!(doc.body, "Body.");
    }

    #[test]
    fn no_closing_delimiter_returns_full_body() {
        let text = "---\nname: skill\nNo closing delimiter here.";
        let doc = parse(text);
        assert!(doc.frontmatter.is_empty());
        assert_eq!(doc.body, text);
    }

    #[test]
    fn crlf_and_cr_are_normalized_like_pi() {
        let doc = parse("---\r\nname: win\r\n---\r\nBody after CRLF.\r\n");
        assert_eq!(
            doc.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("win")
        );
        assert_eq!(doc.body, "Body after CRLF.");
        let cr = parse("---\rname: cr\r---\rBody after CR.\r");
        assert_eq!(
            cr.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("cr")
        );
        assert_eq!(cr.body, "Body after CR.");
    }

    #[test]
    fn closing_not_followed_by_newline_still_counts() {
        let doc = parse("---\nname: nl\n---End of body, dash not followed by newline");
        assert_eq!(
            doc.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("nl")
        );
        assert_eq!(doc.body, "End of body, dash not followed by newline");
    }

    #[test]
    fn empty_body_no_frontmatter_line() {
        let doc = parse("---\nname: x\n---\n");
        assert_eq!(
            doc.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("x")
        );
        assert_eq!(doc.body, "");
    }

    #[test]
    fn indented_delimiters_are_not_frontmatter() {
        let doc = parse("  ---\nname: indented\n---\nBody (delimiter indented)");
        assert!(doc.frontmatter.is_empty());
        assert_eq!(
            doc.body,
            "  ---\nname: indented\n---\nBody (delimiter indented)"
        );
    }

    #[test]
    fn invalid_yaml_raises() {
        let err = parse_frontmatter("---\nname: [unclosed\n---\nBody").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn dash_inside_yaml_is_data() {
        let doc = parse("---\na: ----\n---\nBody");
        assert_eq!(
            doc.frontmatter.get("a").and_then(|v| v.as_str()),
            Some("----")
        );
        assert_eq!(doc.body, "Body");
    }

    #[test]
    fn multiline_yaml() {
        let doc = parse("---\ndescription: |\n  Multi-line\n  text.\n---\nBody");
        assert_eq!(
            doc.frontmatter.get("description").and_then(|v| v.as_str()),
            Some("Multi-line\ntext.\n")
        );
        assert_eq!(doc.body, "Body");
    }

    #[test]
    fn scalar_values() {
        let doc = parse("---\ncount: 42\nenabled: true\n---\nBody");
        assert_eq!(
            doc.frontmatter.get("count").and_then(|v| v.as_i64()),
            Some(42)
        );
        assert_eq!(
            doc.frontmatter.get("enabled").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn list_values() {
        let doc = parse("---\ntags:\n  - a\n  - b\n---\nBody");
        let tags = doc.frontmatter.get("tags").unwrap();
        assert_eq!(tags, &serde_json::json!(["a", "b"]));
    }

    #[test]
    fn empty_string() {
        let doc = parse("");
        assert!(doc.frontmatter.is_empty());
        assert_eq!(doc.body, "");
    }

    #[test]
    fn frontmatter_only_no_body() {
        let doc = parse("---\nname: snippet\n---");
        assert_eq!(
            doc.frontmatter.get("name").and_then(|v| v.as_str()),
            Some("snippet")
        );
        assert_eq!(doc.body, "");
    }
}
