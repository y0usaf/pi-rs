//! Behavioral pins for `util::json_parse` against the spec's
//! `utils/json-parse.ts` (`repairJson`, `parseJsonWithRepair`,
//! `parseStreamingJson` + the `partial-json` fallback).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_ai::util::{
    parse_json_with_repair, parse_partial_json, parse_streaming_json, repair_json,
};
use serde_json::json;

#[test]
fn repair_escapes_raw_control_characters_in_strings() {
    assert_eq!(repair_json("{\"a\": \"x\ny\"}"), "{\"a\": \"x\\ny\"}");
    assert_eq!(repair_json("{\"a\": \"x\ty\"}"), "{\"a\": \"x\\ty\"}");
    assert_eq!(
        repair_json("{\"a\": \"x\u{1}y\"}"),
        "{\"a\": \"x\\u0001y\"}"
    );
    // Control characters outside strings are untouched.
    assert_eq!(repair_json("{\n\"a\": 1}"), "{\n\"a\": 1}");
}

#[test]
fn repair_doubles_invalid_escapes() {
    assert_eq!(repair_json(r#"{"a": "c:\path"}"#), r#"{"a": "c:\\path"}"#);
    // Valid escapes survive.
    assert_eq!(repair_json(r#"{"a": "x\ny"}"#), r#"{"a": "x\ny"}"#);
    // Valid unicode escapes survive; `\u` with bad digits also survives
    // (spec: `u` is in VALID_JSON_ESCAPES, so the escape is kept as-is).
    assert_eq!(repair_json(r#"{"a": "\u0041"}"#), r#"{"a": "\u0041"}"#);
    assert_eq!(repair_json(r#"{"a": "\uZZ"}"#), r#"{"a": "\uZZ"}"#);
    // Trailing backslash is doubled.
    assert_eq!(repair_json(r#"{"a": "x\"#), r#"{"a": "x\\"#);
}

#[test]
fn parse_with_repair_round_trips() {
    assert_eq!(
        parse_json_with_repair("{\"a\": \"x\ny\"}").unwrap(),
        json!({ "a": "x\ny" })
    );
    assert!(parse_json_with_repair("not json").is_err());
}

#[test]
fn partial_objects_complete() {
    assert_eq!(
        parse_partial_json(r#"{"path": "src/m"#).unwrap(),
        json!({ "path": "src/m" })
    );
    assert_eq!(
        parse_partial_json(r#"{"a": 1, "b"#).unwrap(),
        json!({ "a": 1 })
    );
    assert_eq!(parse_partial_json(r#"{"a":"#).unwrap(), json!({}));
    assert_eq!(parse_partial_json("{").unwrap(), json!({}));
    assert_eq!(
        parse_partial_json(r#"{"a": tr"#).unwrap(),
        json!({ "a": true })
    );
}

#[test]
fn partial_arrays_and_scalars_complete() {
    assert_eq!(
        parse_partial_json(r#"[1, 2, {"a": "x"#).unwrap(),
        json!([1, 2, { "a": "x" }])
    );
    assert_eq!(parse_partial_json("\"hello").unwrap(), json!("hello"));
    assert_eq!(parse_partial_json("123.").unwrap(), json!(123));
    assert_eq!(parse_partial_json("1.5e").unwrap(), json!(1.5));
    assert_eq!(parse_partial_json("nul").unwrap(), json!(null));
    assert!(parse_partial_json("-").is_err());
}

#[test]
fn partial_strings_drop_truncated_escapes() {
    assert_eq!(parse_partial_json("\"ab\\").unwrap(), json!("ab"));
    assert_eq!(parse_partial_json("\"ab\\u26").unwrap(), json!("ab"));
    // A complete escape decodes.
    assert_eq!(parse_partial_json("\"ab\\n").unwrap(), json!("ab\n"));
    // Surrogate pairs split across the escape boundary survive when
    // complete, and unpaired halves are dropped (sanitizeSurrogates).
    assert_eq!(
        parse_partial_json("\"\\ud83d\\ude48\"").unwrap(),
        json!("\u{1F648}")
    );
    assert_eq!(parse_partial_json("\"x\\ud83d y\"").unwrap(), json!("x y"));
}

#[test]
fn streaming_json_never_fails() {
    assert_eq!(parse_streaming_json(""), json!({}));
    assert_eq!(parse_streaming_json("   "), json!({}));
    assert_eq!(parse_streaming_json("{\"a\": 1}"), json!({ "a": 1 }));
    assert_eq!(parse_streaming_json(r#"{"a": "b"#), json!({ "a": "b" }));
    // Raw newline inside a partial string: repaired then partial-parsed.
    assert_eq!(
        parse_streaming_json("{\"a\": \"x\ny"),
        json!({ "a": "x\ny" })
    );
    assert_eq!(parse_streaming_json("total garbage \\"), json!({}));
    // Valid non-object JSON passes through, as in the spec.
    assert_eq!(parse_streaming_json("null"), json!(null));
    assert_eq!(parse_streaming_json("[1]"), json!([1]));
}
