//! Golden port of the read/write/ls/edit/bash/grep/find sections of the
//! spec's `test/tools.test.ts` (pi v0.79.0), run against the embedded
//! builtin tools pack through the public host path (`load_embedded` +
//! `call_tool`) — the same seam the WS4 agent loop will use.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_rs_app::builtins::TOOLS_PACK;
use pi_rs_host::{Host, HostConfig};
use serde_json::{Value, json};

fn host(cwd: &std::path::Path) -> Host {
    let host = Host::new(HostConfig {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host boots");
    let report = host.load_embedded(&[TOOLS_PACK]);
    assert!(report.errors.is_empty(), "load errors: {:?}", report.errors);
    host
}

/// The spec test's `getTextOutput` helper.
fn text_output(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|c| c["type"] == "text")
                .map(|c| c["text"].as_str().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn call(host: &Host, tool: &str, params: Value) -> Result<Value, String> {
    host.call_tool(tool, "test-call", &params)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

#[test]
fn pack_registers_the_slice_tools_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host(dir.path());
    let names: Vec<String> = host
        .tools()
        .expect("tools mirror")
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(
        names,
        ["read", "bash", "edit", "write", "grep", "find", "ls"]
    );
}

// ---------------------------------------------------------------------------
// read tool
// ---------------------------------------------------------------------------

#[test]
fn read_file_contents_that_fit_within_limits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("test.txt");
    let content = "Hello, world!\nLine 2\nLine 3";
    std::fs::write(&file, content).unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    assert_eq!(text_output(&result), content);
    assert!(!text_output(&result).contains("Use offset="));
    assert!(result.get("details").is_none());
}

#[test]
fn read_handles_non_existent_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("nonexistent.txt");
    let err = call(&host(dir.path()), "read", json!({ "path": file })).unwrap_err();
    assert!(err.contains("ENOENT"), "got: {err}");
}

#[test]
fn read_truncates_files_exceeding_line_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("large.txt");
    let lines: Vec<String> = (1..=2500).map(|i| format!("Line {i}")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    let output = text_output(&result);
    assert!(output.contains("Line 1"));
    assert!(output.contains("Line 2000"));
    assert!(!output.contains("Line 2001"));
    assert!(output.contains("[Showing lines 1-2000 of 2500. Use offset=2001 to continue.]"));
}

#[test]
fn read_truncates_when_byte_limit_exceeded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("large-bytes.txt");
    // Exceeds the 50KB byte limit with fewer than 2000 lines.
    let lines: Vec<String> = (1..=500)
        .map(|i| format!("Line {i}: {}", "x".repeat(200)))
        .collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    let output = text_output(&result);
    assert!(output.contains("Line 1:"));
    // Spec regex: /\[Showing lines 1-\d+ of 500 \(.* limit\)\. Use offset=\d+ to continue\.\]/
    assert!(output.contains("[Showing lines 1-"), "got tail: {output}");
    assert!(
        output.contains(" of 500 (50.0KB limit). Use offset="),
        "got tail: {output}"
    );
}

#[test]
fn read_handles_offset_parameter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("offset-test.txt");
    let lines: Vec<String> = (1..=100).map(|i| format!("Line {i}")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(
        &host(dir.path()),
        "read",
        json!({ "path": file, "offset": 51 }),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(!output.contains("Line 50\n"));
    assert!(output.contains("Line 51"));
    assert!(output.contains("Line 100"));
    assert!(!output.contains("Use offset="));
}

#[test]
fn read_handles_limit_parameter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("limit-test.txt");
    let lines: Vec<String> = (1..=100).map(|i| format!("Line {i}")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(
        &host(dir.path()),
        "read",
        json!({ "path": file, "limit": 10 }),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(output.contains("Line 1"));
    assert!(output.contains("Line 10"));
    assert!(!output.contains("Line 11"));
    assert!(output.contains("[90 more lines in file. Use offset=11 to continue.]"));
}

#[test]
fn read_handles_offset_and_limit_together() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("offset-limit-test.txt");
    let lines: Vec<String> = (1..=100).map(|i| format!("Line {i}")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(
        &host(dir.path()),
        "read",
        json!({ "path": file, "offset": 41, "limit": 20 }),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(!output.contains("Line 40\n"));
    assert!(output.contains("Line 41"));
    assert!(output.contains("Line 60"));
    assert!(!output.contains("Line 61"));
    assert!(output.contains("[40 more lines in file. Use offset=61 to continue.]"));
}

#[test]
fn read_errors_when_offset_is_beyond_file_length() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("short.txt");
    std::fs::write(&file, "Line 1\nLine 2\nLine 3").unwrap();

    let err = call(
        &host(dir.path()),
        "read",
        json!({ "path": file, "offset": 100 }),
    )
    .unwrap_err();
    assert!(
        err.contains("Offset 100 is beyond end of file (3 lines total)"),
        "got: {err}"
    );
}

#[test]
fn read_includes_truncation_details_when_truncated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("large-file.txt");
    let lines: Vec<String> = (1..=2500).map(|i| format!("Line {i}")).collect();
    std::fs::write(&file, lines.join("\n")).unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    let truncation = &result["details"]["truncation"];
    assert_eq!(truncation["truncated"], true);
    assert_eq!(truncation["truncatedBy"], "lines");
    assert_eq!(truncation["totalLines"], 2500);
    assert_eq!(truncation["outputLines"], 2000);
}

/// The spec test's 1x1 transparent PNG fixture, decoded from its base64
/// literal.
fn png_1x1() -> Vec<u8> {
    const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAX+XDSwAAAABJRU5ErkJggg==";
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let vals: Vec<u32> = B64
        .bytes()
        .filter(|b| *b != b'=')
        .map(|b| CHARS.iter().position(|c| *c == b).unwrap() as u32)
        .collect();
    for chunk in vals.chunks(4) {
        let mut v: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            v |= c << (18 - 6 * i);
        }
        let bytes = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    out
}

#[test]
fn read_detects_image_mime_type_from_file_magic_not_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("image.txt");
    std::fs::write(&file, png_1x1()).unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    assert_eq!(result["content"][0]["type"], "text");
    assert!(text_output(&result).contains("Read image file [image/png]"));

    let image = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "image")
        .expect("image block");
    assert_eq!(image["mimeType"], "image/png");
    assert!(!image["data"].as_str().unwrap().is_empty());
}

#[test]
fn read_treats_image_extension_with_non_image_content_as_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("not-an-image.png");
    std::fs::write(&file, "definitely not a png").unwrap();

    let result = call(&host(dir.path()), "read", json!({ "path": file })).unwrap();
    let output = text_output(&result);
    assert!(output.contains("definitely not a png"));
    assert!(
        !result["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["type"] == "image")
    );
}

// ---------------------------------------------------------------------------
// write tool
// ---------------------------------------------------------------------------

#[test]
fn write_writes_file_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("write-test.txt");

    let result = call(
        &host(dir.path()),
        "write",
        json!({ "path": file, "content": "Test content" }),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(output.contains("Successfully wrote"));
    assert!(output.contains(file.to_str().unwrap()));
    assert!(result.get("details").is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Test content");
}

#[test]
fn write_creates_parent_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("nested").join("dir").join("test.txt");

    let result = call(
        &host(dir.path()),
        "write",
        json!({ "path": file, "content": "Nested content" }),
    )
    .unwrap();
    assert!(text_output(&result).contains("Successfully wrote"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Nested content");
}

// ---------------------------------------------------------------------------
// ls tool
// ---------------------------------------------------------------------------

#[test]
fn ls_lists_dotfiles_and_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".hidden-file"), "secret").unwrap();
    std::fs::create_dir(dir.path().join(".hidden-dir")).unwrap();

    let result = call(&host(dir.path()), "ls", json!({ "path": dir.path() })).unwrap();
    let output = text_output(&result);
    assert!(output.contains(".hidden-file"));
    assert!(output.contains(".hidden-dir/"));
}

#[test]
fn ls_reports_missing_and_non_directory_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host(dir.path());

    let err = call(&host, "ls", json!({ "path": dir.path().join("missing") })).unwrap_err();
    assert!(err.contains("Path not found:"), "got: {err}");

    let file = dir.path().join("plain.txt");
    std::fs::write(&file, "x").unwrap();
    let err = call(&host, "ls", json!({ "path": file })).unwrap_err();
    assert!(err.contains("Not a directory:"), "got: {err}");
}

#[test]
fn ls_caps_entries_and_reports_the_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..10 {
        std::fs::write(dir.path().join(format!("f{i:02}")), "x").unwrap();
    }

    let result = call(
        &host(dir.path()),
        "ls",
        json!({ "path": dir.path(), "limit": 4 }),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(output.contains("f00"));
    assert!(output.contains("f03"));
    assert!(!output.contains("f04"));
    assert!(output.contains("[4 entries limit reached. Use limit=8 for more]"));
    assert_eq!(result["details"]["entryLimitReached"], 4);
}

#[test]
fn ls_defaults_to_the_tool_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("here.txt"), "x").unwrap();

    let result = call(&host(dir.path()), "ls", json!({})).unwrap();
    assert!(text_output(&result).contains("here.txt"));
}

// ---------------------------------------------------------------------------
// edit tool (spec tools.test.ts edit/fuzzy/CRLF sections)
// ---------------------------------------------------------------------------

#[test]
fn edit_replaces_and_returns_details() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    std::fs::write(&file, "Hello, world!").unwrap();
    let result = call(
        &host(dir.path()),
        "edit",
        json!({"path": file, "edits": [{"oldText":"world","newText":"testing"}]}),
    )
    .unwrap();
    assert!(text_output(&result).contains("Successfully replaced 1 block(s)"));
    assert!(
        result["details"]["diff"]
            .as_str()
            .unwrap()
            .contains("testing")
    );
    let patch = result["details"]["patch"].as_str().unwrap();
    assert!(patch.contains("--- ") && patch.contains("+++ ") && patch.contains("@@"));
    assert!(patch.contains("-Hello, world!") && patch.contains("+Hello, testing!"));
    assert_eq!(std::fs::read_to_string(file).unwrap(), "Hello, testing!");
}

/// Pi's `generateDiffString`/`generateUnifiedPatch` run over jsdiff
/// `diffLines`: an insertion shifts later lines without marking them
/// changed, and the unified patch is jsdiff's (FILE_HEADERS_ONLY,
/// context 4). Expectations derived from `ref/pi` `edit-diff.ts` on the
/// vendored jsdiff 8.0.4.
#[test]
fn edit_diff_details_match_pi_jsdiff_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    let result = call(
        &host(dir.path()),
        "edit",
        json!({"path": file, "edits": [{"oldText":"alpha\nbeta","newText":"alpha\nnew\nbeta"}]}),
    )
    .unwrap();
    assert_eq!(
        result["details"]["diff"].as_str().unwrap(),
        " 1 alpha\n+2 new\n 2 beta\n 3 gamma"
    );
    assert_eq!(result["details"]["firstChangedLine"], 2);
    let path_str = file.to_string_lossy();
    assert_eq!(
        result["details"]["patch"].as_str().unwrap(),
        format!("--- {path_str}\n+++ {path_str}\n@@ -1,3 +1,4 @@\n alpha\n+new\n beta\n gamma\n")
    );
}

#[test]
fn edit_validates_failures_without_partial_writes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    let original = "one\ntwo\nthree\n";
    std::fs::write(&file, original).unwrap();
    let h = host(dir.path());
    let cases = [
        (json!({"path": file, "edits": []}), "edits must contain"),
        (
            json!({"path": file, "edits": [{"oldText":"","newText":"x"}]}),
            "oldText must not be empty",
        ),
        (
            json!({"path": file, "edits": [{"oldText":"missing","newText":"x"}]}),
            "Could not find",
        ),
        (
            json!({"path": file, "edits": [{"oldText":"one\ntwo\n","newText":"x"},{"oldText":"two\nthree\n","newText":"y"}]}),
            "overlap",
        ),
    ];
    for (params, message) in cases {
        let err = call(&h, "edit", params).unwrap_err();
        assert!(err.contains(message), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    }
    std::fs::write(&file, "foo foo foo").unwrap();
    let err = call(
        &h,
        "edit",
        json!({"path": file, "edits": [{"oldText":"foo","newText":"bar"}]}),
    )
    .unwrap_err();
    assert!(err.contains("Found 3 occurrences"), "{err}");
}

#[test]
fn edit_applies_disjoint_edits_against_original() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multi.txt");
    std::fs::write(&file, "foo\nbar\nbaz\n").unwrap();
    call(
        &host(dir.path()),
        "edit",
        json!({"path": file, "edits": [
            {"oldText":"foo\n","newText":"foo bar\n"},
            {"oldText":"bar\n","newText":"BAR\n"}
        ]}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "foo bar\nBAR\nbaz\n"
    );
}

#[test]
fn edit_fuzzy_matches_nfkc_quotes_spaces_and_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fuzzy.txt");
    std::fs::write(
        &file,
        "ＡＢＣ１２３   \ncafe\u{301}\nconsole.log(‘hello’);\nhello\u{a0}world\n",
    )
    .unwrap();
    call(
        &host(dir.path()),
        "edit",
        json!({"path": file, "edits": [
            {"oldText":"ABC123\ncafé\n","newText":"XYZ789\ncoffee\n"},
            {"oldText":"console.log('hello');\n","newText":"console.log('world');\n"},
            {"oldText":"hello world\n","newText":"hello universe\n"}
        ]}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "XYZ789\ncoffee\nconsole.log('world');\nhello universe\n"
    );
}

#[test]
fn edit_preserves_crlf_and_bom() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("crlf.txt");
    std::fs::write(&file, "\u{feff}first\r\nsecond\r\nthird\r\n").unwrap();
    call(
        &host(dir.path()),
        "edit",
        json!({"path": file, "edits": [{"oldText":"second\n","newText":"REPLACED\n"}]}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "\u{feff}first\r\nREPLACED\r\nthird\r\n"
    );
}

#[test]
fn edit_prepares_legacy_and_stringified_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("legacy.txt");
    std::fs::write(&file, "before\nsecond\n").unwrap();
    let h = host(dir.path());
    call(
        &h,
        "edit",
        json!({"path": file, "oldText":"before", "newText":"after"}),
    )
    .unwrap();
    call(
        &h,
        "edit",
        json!({
            "path": file,
            "edits": "[{\"oldText\":\"second\",\"newText\":\"next\"}]"
        }),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(file).unwrap(), "after\nnext\n");
}

// ---------------------------------------------------------------------------
// bash tool — named ports from the spec's tools.test.ts
// ---------------------------------------------------------------------------

#[test]
fn bash_executes_simple_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = call(
        &host(dir.path()),
        "bash",
        json!({ "command": "echo 'test output'" }),
    )
    .unwrap();
    assert!(text_output(&result).contains("test output"));
    assert!(result.get("details").is_none());
}

#[test]
fn bash_handles_command_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = call(&host(dir.path()), "bash", json!({ "command": "exit 1" })).unwrap_err();
    assert!(error.contains("Command exited with code 1"), "got: {error}");
}

#[test]
fn bash_respects_timeout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = call(
        &host(dir.path()),
        "bash",
        json!({ "command": "sleep 5", "timeout": 0.05 }),
    )
    .unwrap_err();
    assert!(
        error.contains("Command timed out after 0.05 seconds"),
        "got: {error}"
    );
}

#[test]
fn bash_streams_updates() {
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().expect("tempdir");
    let updates = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = Arc::clone(&updates);
    let result = host(dir.path())
        .call_tool_with_updates(
            "bash",
            "streaming-test",
            &json!({ "command": "printf first; sleep 0.02; printf second" }),
            Some(Arc::new(move |update| {
                sink.lock().expect("updates lock").push(update)
            })),
        )
        .expect("bash runs");
    assert_eq!(text_output(&result), "firstsecond");
    let updates = updates.lock().expect("updates lock");
    assert!(!updates.is_empty());
    assert!(
        updates[0]["content"].as_array().is_some_and(Vec::is_empty)
            || updates[0]["content"]
                .as_object()
                .is_some_and(serde_json::Map::is_empty)
    );
    assert!(
        updates
            .iter()
            .any(|update| text_output(update).contains("second"))
    );
}

#[test]
fn bash_does_not_count_trailing_newline_as_an_extra_truncated_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = call(&host(dir.path()), "bash", json!({ "command": "seq 4000" })).unwrap();
    let output = text_output(&result);
    assert_eq!(result["details"]["truncation"]["totalLines"], 4000);
    assert_eq!(result["details"]["truncation"]["outputLines"], 2000);
    assert!(output.contains("[Showing lines 2001-4000 of 4000. Full output: "));
}

#[test]
fn bash_persists_full_output_when_truncated_by_line_count() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = call(&host(dir.path()), "bash", json!({ "command": "seq 3000" })).unwrap();
    assert_eq!(result["details"]["truncation"]["truncatedBy"], "lines");
    let path = result["details"]["fullOutputPath"]
        .as_str()
        .expect("full output path");
    let full = std::fs::read_to_string(path).expect("full output file");
    assert!(full.starts_with("1\n2\n3\n"));
    assert!(full.ends_with("2998\n2999\n3000\n"));
}

#[test]
fn bash_persists_all_chunks_and_coalesces_updates() {
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().unwrap();
    let updates = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = Arc::clone(&updates);
    let result = host(dir.path())
        .call_tool_with_updates(
            "bash",
            "complete-spill",
            &json!({"command": "head -c 60000 /dev/zero | tr '\\000' x; sleep 0.15; printf THE_END"}),
            Some(Arc::new(move |v| sink.lock().unwrap().push(v))),
        )
        .unwrap();
    let path = result["details"]["fullOutputPath"].as_str().unwrap();
    let full = std::fs::read(path).unwrap();
    assert_eq!(full.len(), 60007);
    assert!(full.ends_with(b"THE_END"));
    let updates = updates.lock().unwrap();
    assert!(
        updates.len() <= 4,
        "updates were not coalesced: {}",
        updates.len()
    );
    assert!(text_output(updates.last().unwrap()).ends_with("THE_END"));
}

#[test]
fn bash_disables_nonpositive_timeouts() {
    let dir = tempfile::tempdir().unwrap();
    for timeout in [0.0, -1.0] {
        let result = call(
            &host(dir.path()),
            "bash",
            json!({"command":"printf ok", "timeout":timeout}),
        )
        .unwrap();
        assert_eq!(text_output(&result), "ok");
    }
}

#[test]
fn streaming_exec_and_tool_update_example_exercises_both_callbacks() {
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().expect("tempdir");
    let host = Host::new(HostConfig {
        cwd: Some(dir.path().to_string_lossy().into_owned()),
        ..HostConfig::default()
    })
    .expect("host");
    host.load(
        "examples/extensions/streaming-tool-demo.lua",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/streaming-tool-demo.lua"
        )),
    )
    .expect("example loads");
    let updates = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = Arc::clone(&updates);
    let result = host
        .call_tool_with_updates(
            "streaming-demo",
            "example",
            &json!({}),
            Some(Arc::new(move |update| {
                sink.lock().expect("lock").push(update)
            })),
        )
        .expect("tool runs");
    assert_eq!(text_output(&result), "firstsecond");
    assert!(
        updates
            .lock()
            .expect("lock")
            .iter()
            .any(|update| text_output(update).contains("second"))
    );
}

// ---------------------------------------------------------------------------
// grep / find tools
// ---------------------------------------------------------------------------

#[test]
fn grep_includes_filename_for_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("example.txt");
    std::fs::write(&file, "first line\nmatch line\nlast line").unwrap();
    let result = call(
        &host(dir.path()),
        "grep",
        json!({"pattern":"match", "path":file}),
    )
    .unwrap();
    assert!(
        text_output(&result).contains("example.txt:2: match line"),
        "{}",
        text_output(&result)
    );
}

#[test]
fn grep_respects_global_limit_and_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("context.txt");
    std::fs::write(
        &file,
        "before\nmatch one\nafter\nmiddle\nmatch two\nafter two",
    )
    .unwrap();
    let result = call(
        &host(dir.path()),
        "grep",
        json!({"pattern":"match", "path":file, "limit":1, "context":1}),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(output.contains("context.txt-1- before"), "{output}");
    assert!(output.contains("context.txt:2: match one"));
    assert!(output.contains("context.txt-3- after"));
    assert!(output.contains("[1 matches limit reached. Use limit=2 for more, or refine pattern]"));
    assert!(!output.contains("match two"));
    assert_eq!(result["details"]["matchLimitReached"], 1);
}

#[test]
fn grep_treats_flag_like_pattern_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("grep-injection-marker");
    let payload = dir.path().join("payload.sh");
    std::fs::write(
        &payload,
        format!(
            "#!/bin/sh\necho executed > {}\ncat \"$1\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::write(dir.path().join("target.txt"), "target\n").unwrap();
    let result = call(
        &host(dir.path()),
        "grep",
        json!({"pattern":format!("--pre={}", payload.display()), "path":dir.path()}),
    )
    .unwrap();
    assert!(text_output(&result).contains("No matches found"));
    assert!(!marker.exists());
}

#[test]
fn grep_supports_literal_case_glob_and_truncates_long_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("one.txt"),
        format!("PREFIX A.B {}", "x".repeat(600)),
    )
    .unwrap();
    std::fs::write(dir.path().join("two.rs"), "prefix axb").unwrap();
    let result = call(&host(dir.path()), "grep", json!({"pattern":"a.b", "path":dir.path(), "glob":"*.txt", "ignoreCase":true, "literal":true})).unwrap();
    let output = text_output(&result);
    assert!(output.contains("one.txt:1: PREFIX A.B"), "{output}");
    assert!(!output.contains("two.rs"));
    assert!(output.contains("... [truncated]"));
    assert_eq!(result["details"]["linesTruncated"], true);
}

#[test]
fn find_includes_hidden_files_and_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".secret")).unwrap();
    std::fs::write(dir.path().join(".secret/hidden.txt"), "hidden").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "visible").unwrap();
    std::fs::write(dir.path().join("ignored.txt"), "ignored").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    let result = call(
        &host(dir.path()),
        "find",
        json!({"pattern":"**/*.txt", "path":dir.path()}),
    )
    .unwrap();
    let output = text_output(&result);
    assert!(output.contains("visible.txt"));
    assert!(output.contains(".secret/hidden.txt"));
    assert!(!output.contains("ignored.txt"));
}

#[test]
fn find_surfaces_glob_errors_and_treats_flags_as_patterns() {
    let dir = tempfile::tempdir().unwrap();
    let err = call(
        &host(dir.path()),
        "find",
        json!({"pattern":"[", "path":dir.path()}),
    )
    .unwrap_err();
    assert!(
        err.to_lowercase().contains("glob") || err.contains("fd exited with code"),
        "{err}"
    );
    let result = call(
        &host(dir.path()),
        "find",
        json!({"pattern":"--help", "path":dir.path()}),
    )
    .unwrap();
    assert!(text_output(&result).contains("No files found matching pattern"));
}

#[test]
fn find_matches_path_globs_and_reports_limits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/a.spec.ts"), "").unwrap();
    std::fs::write(dir.path().join("src/nested/b.spec.ts"), "").unwrap();
    std::fs::write(dir.path().join("root.spec.ts"), "").unwrap();
    for pattern in ["src/**/*.spec.ts", "**/src/**/*.spec.ts"] {
        let result = call(
            &host(dir.path()),
            "find",
            json!({"pattern":pattern, "path":dir.path()}),
        )
        .unwrap();
        let output = text_output(&result);
        assert!(
            output.contains("src/a.spec.ts"),
            "pattern={pattern}: {output}"
        );
        assert!(
            output.contains("src/nested/b.spec.ts"),
            "pattern={pattern}: {output}"
        );
        assert!(
            !output.contains("root.spec.ts"),
            "pattern={pattern}: {output}"
        );
    }
    let result = call(
        &host(dir.path()),
        "find",
        json!({"pattern":"*.spec.ts", "path":dir.path(), "limit":1}),
    )
    .unwrap();
    assert!(
        text_output(&result)
            .contains("[1 results limit reached. Use limit=2 for more, or refine pattern]")
    );
    assert_eq!(result["details"]["resultLimitReached"], 1);
}
