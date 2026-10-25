//! The script's source format: JavaScript, optionally preceded by one options line
//! (`packages/codemode/src/source.ts`).

use serde_json::Value;

pub const CODEMODE_OPTIONS_PREFIX: &str = "// @options:";

const SUPPORTED_FIELDS_TEXT: &str = "`max_output_tokens` and `timeout_ms`";
/// Largest delay `setTimeout` supports, which bounds `timeout_ms`.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;
/// `Number.isSafeInteger`'s bound.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Token budget for the script's output, and the hard deadline for the whole script.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceOptions {
    pub max_output_tokens: Option<u64>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSource {
    /// The script with the options line replaced by an empty line, so line numbers are unchanged.
    pub code: String,
    pub options: SourceOptions,
}

fn parse_options(directive: &str) -> Result<SourceOptions, String> {
    if directive.is_empty() {
        return Err(format!(
            "@options must be a JSON object with supported fields {SUPPORTED_FIELDS_TEXT}"
        ));
    }
    let value: Value = serde_json::from_str(directive).map_err(|error| {
        format!(
            "@options must be valid JSON with supported fields {SUPPORTED_FIELDS_TEXT}: {error}"
        )
    })?;
    let Value::Object(fields) = value else {
        return Err(format!(
            "@options must be a JSON object with supported fields {SUPPORTED_FIELDS_TEXT}"
        ));
    };
    for key in fields.keys() {
        if !["max_output_tokens", "timeout_ms"].contains(&key.as_str()) {
            return Err(format!(
                "@options only supports {SUPPORTED_FIELDS_TEXT}; got `{key}`"
            ));
        }
    }
    let integer = |value: &Value| -> Option<u64> {
        let number = value.as_u64()?;
        (number <= MAX_SAFE_INTEGER).then_some(number)
    };
    let mut options = SourceOptions::default();
    if let Some(value) = fields.get("max_output_tokens") {
        options.max_output_tokens = Some(integer(value).ok_or_else(|| {
            "@options field `max_output_tokens` must be a non-negative safe integer".to_owned()
        })?);
    }
    if let Some(value) = fields.get("timeout_ms") {
        let timeout = integer(value).filter(|n| *n > 0 && *n <= MAX_TIMEOUT_MS);
        options.timeout_ms = Some(timeout.ok_or_else(|| {
            format!("@options field `timeout_ms` must be a positive integer up to {MAX_TIMEOUT_MS}")
        })?);
    }
    Ok(options)
}

/// Split an optional first-line `// @options: {...}` from the script. Refuses empty input, invalid
/// options, and an options line that is not the first line, which would otherwise be ignored.
pub fn parse(input: &str) -> Result<ParsedSource, String> {
    if input.trim().is_empty() {
        return Err(
            "Expected JavaScript source text (non-empty). Provide JS only, optionally with a first line `// @options: {\"max_output_tokens\": 1000}`."
                .to_owned(),
        );
    }
    let lines: Vec<&str> = input.split('\n').collect();
    if let Some(index) = lines
        .iter()
        .enumerate()
        .find(|(index, line)| *index > 0 && line.trim_start().starts_with(CODEMODE_OPTIONS_PREFIX))
        .map(|(index, _)| index)
    {
        return Err(format!(
            "The {CODEMODE_OPTIONS_PREFIX} line must be the first line of the input; found one on line {}",
            index + 1
        ));
    }
    let first = lines[0].trim_end_matches('\r').trim_start();
    if !first.starts_with(CODEMODE_OPTIONS_PREFIX) {
        return Ok(ParsedSource {
            code: input.to_owned(),
            options: SourceOptions::default(),
        });
    }
    let code = match input.find('\n') {
        Some(newline) => input[newline..].to_owned(),
        None => String::new(),
    };
    if code.trim().is_empty() {
        return Err(
            "The @options line must be followed by JavaScript source on subsequent lines"
                .to_owned(),
        );
    }
    Ok(ParsedSource {
        code,
        options: parse_options(first[CODEMODE_OPTIONS_PREFIX.len()..].trim())?,
    })
}
