//! The shipped transcript against the canonical experience grids.
//!
//! PLAN 5.1 accepts transcript/tool/stream presentation only if the canonical
//! frames selected by PLAN 0.2 match. This suite drives the shipped Lua
//! frontend root through the public host, replays its ANSI into a terminal
//! emulator, and compares the **transcript region** of the resulting screen,
//! cell by cell, with `tests/experience/canonical-v1.json`.
//!
//! Two deliberate bounds:
//!
//! * Only the transcript strip is compared. The header, working indicator,
//!   prompt chrome, and status line belong to PLAN 5.2/5.3 and are not claimed
//!   here.
//! * An untouched cell and a written space are the same screen, so glyphs are
//!   normalised to a space before comparison. Style is not normalised: a space
//!   painted with a background is a different cell from an untouched one.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use pi_rs_host::kernel::{DispatchBatch, DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};
use pi_rs_tui::ui_harness::{CellSnapshot, FrameRecorder, FrameSnapshot};
use serde_json::{Value, json};

const COLUMNS: u16 = 72;
const ROWS: u16 = 24;

// ---------------------------------------------------------------------------
// Canonical fixture
// ---------------------------------------------------------------------------

/// One decoded canonical frame: cells in row-major order plus its geometry.
struct CanonicalFrame {
    columns: u16,
    cells: Vec<CellSnapshot>,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/experience/canonical-v1.json")
}

fn empty_cell() -> CellSnapshot {
    CellSnapshot {
        text: String::new(),
        wide: false,
        wide_continuation: false,
        foreground: "default".to_owned(),
        background: "default".to_owned(),
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        inverse: false,
    }
}

fn flag(style: &Value, name: &str) -> bool {
    style.get(name).and_then(Value::as_bool).unwrap_or(false)
}

fn color(style: &Value, name: &str) -> String {
    style
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_owned()
}

/// Decode one named checkpoint into terminal cells.
///
/// The fixture stores a frame compactly: one glyph character per cell (`░`
/// untouched, `␠` a written space) plus `[row, start, end, style]` spans into a
/// deduplicated palette. Reading it here keeps `ui-diff` the owner of fixture
/// validation while product suites read the same bytes.
fn canonical(journey_name: &str, step_name: &str) -> CanonicalFrame {
    let raw = std::fs::read_to_string(fixture_path()).expect("read canonical fixture");
    let fixture: Value = serde_json::from_str(&raw).expect("parse canonical fixture");
    let palette = fixture
        .get("styles")
        .and_then(Value::as_object)
        .expect("fixture palette");

    let step = fixture
        .get("journeys")
        .and_then(Value::as_array)
        .expect("fixture journeys")
        .iter()
        .find(|journey| journey.get("name").and_then(Value::as_str) == Some(journey_name))
        .unwrap_or_else(|| panic!("journey {journey_name:?} missing"))
        .get("steps")
        .and_then(Value::as_array)
        .expect("journey steps")
        .iter()
        .find(|step| step.get("name").and_then(Value::as_str) == Some(step_name))
        .unwrap_or_else(|| panic!("step {step_name:?} missing"))
        .clone();

    let frame = step.get("frame").expect("step frame");
    let size = frame.get("size").and_then(Value::as_array).expect("size");
    let columns = u16::try_from(size[0].as_u64().unwrap()).unwrap();
    let rows = u16::try_from(size[1].as_u64().unwrap()).unwrap();

    let mut cells = vec![empty_cell(); usize::from(columns) * usize::from(rows)];
    for (row, glyphs) in frame
        .get("glyphs")
        .and_then(Value::as_array)
        .expect("glyph rows")
        .iter()
        .enumerate()
    {
        for (column, glyph) in glyphs.as_str().expect("glyph row").chars().enumerate() {
            cells[row * usize::from(columns) + column].text = match glyph {
                '░' => String::new(),
                '␠' => " ".to_owned(),
                other => other.to_string(),
            };
        }
    }
    for span in frame
        .get("styles")
        .and_then(Value::as_array)
        .expect("style spans")
    {
        let span = span.as_array().expect("style span");
        let row = usize::try_from(span[0].as_u64().unwrap()).unwrap();
        let start = usize::try_from(span[1].as_u64().unwrap()).unwrap();
        let end = usize::try_from(span[2].as_u64().unwrap()).unwrap();
        let name = span[3].as_str().expect("style name");
        let style = palette
            .get(name)
            .unwrap_or_else(|| panic!("unknown style {name:?}"));
        for column in start..end {
            let cell = &mut cells[row * usize::from(columns) + column];
            cell.foreground = color(style, "foreground");
            cell.background = color(style, "background");
            cell.bold = flag(style, "bold");
            cell.dim = flag(style, "dim");
            cell.italic = flag(style, "italic");
            cell.underline = flag(style, "underline");
            cell.inverse = flag(style, "inverse");
        }
    }

    CanonicalFrame { columns, cells }
}

// ---------------------------------------------------------------------------
// Shipped frontend under a terminal emulator
// ---------------------------------------------------------------------------

struct Frontend {
    host: Host,
    recorder: FrameRecorder,
    _directory: tempfile::TempDir,
}

impl Frontend {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = Host::new(HostConfig {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..HostConfig::default()
        })
        .unwrap();
        let frontend = pi_rs_builtins::package_root().join("frontend");
        for file in [
            "keys.lua",
            "editor.lua",
            "transcript.lua",
            "chrome.lua",
            "view.lua",
            "init.lua",
        ] {
            let path = frontend.join(file);
            host.load_package(PackageSource::File { path: &path })
                .unwrap_or_else(|error| panic!("load {file}: {error}"));
        }
        let mut harness = Self {
            host,
            recorder: FrameRecorder::new(COLUMNS, ROWS),
            _directory: directory,
        };
        harness.dispatch(json!({"kind": "resize", "columns": COLUMNS, "rows": ROWS}));
        harness.dispatch(json!({"kind": "startup"}));
        harness
    }

    fn dispatch(&mut self, event: Value) -> DispatchBatch {
        let batch = self
            .host
            .dispatch(DispatchRequest::new(RootKind::Frontend, event, Value::Null))
            .unwrap_or_else(|error| panic!("frontend dispatch failed: {error}"));
        for action in &batch.actions {
            if action.kind == "ansi"
                && let Some(data) = action.payload.get("data").and_then(Value::as_str)
            {
                self.recorder.process(data.as_bytes());
            }
        }
        batch
    }

    fn types(&mut self, text: &str) {
        self.dispatch(json!({"kind": "input", "data": text}));
    }

    fn agent(&mut self, actions: Value) {
        self.dispatch(json!({"kind": "agent", "actions": actions}));
    }

    fn frame(&mut self, name: &str) -> FrameSnapshot {
        // Incremental frames only paint what changed, so the readable screen is
        // recovered through the ordinary repaint path before snapshotting.
        self.dispatch(json!({"kind": "resize", "columns": COLUMNS, "rows": ROWS}));
        self.recorder.snapshot(name)
    }
}

/// Row index of the last transcript line: the row above the prompt.
fn last_transcript_row(frame: &FrameSnapshot) -> u16 {
    for row in 0..frame.rows {
        let cell = &frame.cells[usize::from(row) * usize::from(frame.columns)];
        if cell.text == ">" {
            return row - 1;
        }
    }
    panic!("prompt row not found");
}

fn glyph(cell: &CellSnapshot) -> String {
    if cell.text.is_empty() {
        " ".to_owned()
    } else {
        cell.text.clone()
    }
}

fn describe(cell: &CellSnapshot) -> String {
    format!(
        "{:?} fg={} bg={} bold={} dim={} italic={} underline={} inverse={}",
        glyph(cell),
        cell.foreground,
        cell.background,
        cell.bold,
        cell.dim,
        cell.italic,
        cell.underline,
        cell.inverse
    )
}

fn row_text(cells: &[CellSnapshot], columns: u16, row: usize) -> String {
    let start = row * usize::from(columns);
    cells[start..start + usize::from(columns)]
        .iter()
        .map(glyph)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Compare the last `count` transcript rows of both screens, cell by cell.
///
/// Both sides are bottom anchored on their own last transcript row, so the
/// comparison is independent of how much chrome sits above the transcript.
fn assert_transcript_matches(
    checkpoint: &str,
    expected: &CanonicalFrame,
    expected_last_row: usize,
    actual: &FrameSnapshot,
    count: usize,
) {
    assert_eq!(
        expected.columns, actual.columns,
        "{checkpoint}: canonical frame is {} columns wide, pi-rs painted {}",
        expected.columns, actual.columns
    );
    let actual_last_row = usize::from(last_transcript_row(actual));
    assert!(
        actual_last_row + 1 >= count,
        "{checkpoint}: pi-rs transcript holds {} rows, canonical strip needs {count}",
        actual_last_row + 1
    );

    for offset in (0..count).rev() {
        let expected_row = expected_last_row - offset;
        let actual_row = actual_last_row - offset;
        for column in 0..usize::from(expected.columns) {
            let expected_cell =
                &expected.cells[expected_row * usize::from(expected.columns) + column];
            let actual_cell = &actual.cells[actual_row * usize::from(actual.columns) + column];
            let same = glyph(expected_cell) == glyph(actual_cell)
                && expected_cell.foreground == actual_cell.foreground
                && expected_cell.background == actual_cell.background
                && expected_cell.bold == actual_cell.bold
                && expected_cell.dim == actual_cell.dim
                && expected_cell.italic == actual_cell.italic
                && expected_cell.underline == actual_cell.underline
                && expected_cell.inverse == actual_cell.inverse;
            assert!(
                same,
                "{checkpoint}: transcript cell differs at canonical row {expected_row}, \
                 pi-rs row {actual_row}, column {column}\n  canonical: {}\n  pi-rs:     {}\n  \
                 canonical row: {:?}\n  pi-rs row:     {:?}",
                describe(expected_cell),
                describe(actual_cell),
                row_text(&expected.cells, expected.columns, expected_row),
                row_text(&actual.cells, actual.columns, actual_row),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Canonical conversation
// ---------------------------------------------------------------------------

fn message(text: &str) -> Value {
    json!({"kind": "agent_message", "payload": {"text": text}})
}

/// The action the shipped agent emits when a turn completes.
///
/// Replaying it matters: while a turn is running, a submitted line is offered
/// to the agent's steering queue instead of starting a new one.
fn turn_complete() -> Value {
    json!({"kind": "agent_status", "payload": {"state": "idle"}})
}

/// Replay the canonical `stream-tool-cancel` conversation up to its tool call.
fn through_tool_call(frontend: &mut Frontend) {
    frontend.types("Say hello please\r");
    frontend.agent(json!([
        message("Hello! How can I help you today?"),
        turn_complete(),
    ]));
    frontend.types("Read notes.txt\r");
    frontend.agent(json!([
        message("I'll read the notes file."),
        {"kind": "agent_tool_start", "payload": {
            "id": "call-1",
            "name": "read",
            "arguments": {"path": "notes.txt"},
        }},
    ]));
}

#[test]
fn a_pending_tool_call_matches_the_canonical_transcript() {
    let mut frontend = Frontend::new();
    through_tool_call(&mut frontend);

    // Canonical `tool-pending` rows 1..=15: user block, assistant, user block,
    // assistant, and the pending tool block.
    let expected = canonical("stream-tool-cancel", "tool-pending");
    let actual = frontend.frame("tool-pending");
    assert_transcript_matches("tool-pending", &expected, 15, &actual, 15);
}

/// The action the shipped agent emits while a call's arguments stream in.
fn tool_delta(arguments: Value) -> Value {
    json!({"kind": "agent_tool_delta", "payload": {
        "id": "call-1",
        "name": "read",
        "arguments": arguments,
    }})
}

/// Replay the canonical `stream-tool-cancel` conversation up to the point
/// where the second turn's call has been named but only part of its argument
/// has arrived. The answer before it is streamed rather than settled, because
/// that is the state the canonical `tool-streaming` frame records: the message
/// has not finished, so no `agent_message` has been emitted yet.
fn through_tool_stream(frontend: &mut Frontend) {
    frontend.types("Say hello please\r");
    frontend.agent(json!([
        message("Hello! How can I help you today?"),
        turn_complete(),
    ]));
    frontend.types("Read notes.txt\r");
    frontend.agent(json!([
        {"kind": "agent_text_delta", "payload": {"text": "I'll read the notes file."}},
        tool_delta(json!({})),
        tool_delta(json!({"path": "no"})),
    ]));
}

#[test]
fn a_streaming_tool_call_matches_the_canonical_transcript() {
    let mut frontend = Frontend::new();
    through_tool_stream(&mut frontend);

    // Canonical `tool-streaming` rows 1..=15: the same blocks as
    // `tool-pending`, except the tool block carries only the argument text
    // that has arrived so far (`read no`, not `read notes.txt`).
    let expected = canonical("stream-tool-cancel", "tool-streaming");
    let actual = frontend.frame("tool-streaming");
    assert_transcript_matches("tool-streaming", &expected, 15, &actual, 15);
}

#[test]
fn a_streamed_call_becomes_the_same_block_when_it_starts() {
    let mut frontend = Frontend::new();
    through_tool_stream(&mut frontend);
    frontend.agent(json!([
        message("I'll read the notes file."),
        {"kind": "agent_tool_start", "payload": {
            "id": "call-1",
            "name": "read",
            "arguments": {"path": "notes.txt"},
        }},
    ]));

    // The canonical `tool-pending` strip, reached through the streaming path
    // instead of a single settled start. Matching it cell for cell is what
    // proves the streamed row was refined rather than joined by a second
    // block for the same call, and that the settled message did not repaint
    // the answer it had already streamed.
    let expected = canonical("stream-tool-cancel", "tool-pending");
    let actual = frontend.frame("tool-pending-after-stream");
    assert_transcript_matches("tool-pending-after-stream", &expected, 15, &actual, 15);
}

#[test]
fn a_settled_tool_call_and_cancellation_match_the_canonical_transcript() {
    let mut frontend = Frontend::new();
    through_tool_call(&mut frontend);
    frontend.agent(json!([
        {"kind": "agent_tool_result", "payload": {
            "id": "call-1",
            "name": "read",
            "ok": true,
            "output": "alpha\nbeta\ngamma",
        }},
        message("The file lists three Greek letters."),
        turn_complete(),
    ]));
    frontend.types("Tell me a story\r");
    frontend.agent(json!([
        {"kind": "agent_text_delta", "payload": {"text": "Once upon a time"}},
        {"kind": "agent_cancelled", "payload": {"reason": "interrupt"}},
    ]));

    // Canonical `cancelled` rows 3..=17: the assistant block before the tool
    // call, the settled tool block, the assistant answer, the next user block,
    // the partial answer, and the aborted row.
    let expected = canonical("stream-tool-cancel", "cancelled");
    let actual = frontend.frame("cancelled");
    assert_transcript_matches("cancelled", &expected, 17, &actual, 15);
}

#[test]
fn streaming_deltas_repaint_only_the_row_they_change() {
    let mut frontend = Frontend::new();
    frontend.types("Say hello please\r");
    frontend.agent(json!([{"kind": "agent_text_delta", "payload": {"text": "Hello"}}]));

    // A delta that neither wraps nor scrolls must not repaint the blocks above
    // it: the retained tree keeps their node identities and the differential
    // presenter emits only the changed cells.
    let batch = frontend.dispatch(json!({
        "kind": "agent",
        "actions": [{"kind": "agent_text_delta", "payload": {"text": "!"}}],
    }));
    let painted: String = batch
        .actions
        .iter()
        .filter(|action| action.kind == "ansi")
        .filter_map(|action| action.payload.get("data").and_then(Value::as_str))
        .collect();
    assert!(!painted.is_empty(), "a streamed delta must produce a frame");
    assert!(
        !painted.contains("Say hello please"),
        "an unchanged block was repainted: {painted:?}"
    );
    assert!(
        painted.len() < 200,
        "a one-character delta painted {} bytes: {painted:?}",
        painted.len()
    );
}

#[test]
fn a_long_transcript_costs_one_viewport_per_frame() {
    let mut frontend = Frontend::new();
    for index in 0..400 {
        frontend.types(&format!("message {index}\r"));
        frontend.agent(json!([
            message(&format!("answer {index}")),
            turn_complete()
        ]));
    }

    let batch = frontend.dispatch(json!({"kind": "resize", "columns": COLUMNS, "rows": ROWS}));
    let painted: usize = batch
        .actions
        .iter()
        .filter(|action| action.kind == "ansi")
        .filter_map(|action| action.payload.get("data").and_then(Value::as_str))
        .map(str::len)
        .sum();
    // A full repaint of 72x24 with styling is a few kilobytes; history length
    // must not enter that number.
    assert!(
        painted < 32_768,
        "a full repaint after 400 turns painted {painted} bytes"
    );

    let actual = frontend.frame("long-transcript");
    let last = usize::from(last_transcript_row(&actual));
    assert!(
        row_text(&actual.cells, actual.columns, last).contains("answer 399"),
        "the newest answer must sit against the prompt"
    );
}

// ---------------------------------------------------------------------------
// Canonical thinking blocks
// ---------------------------------------------------------------------------

/// The reasoning text the canonical `thinking-and-queues` journey records.
const CANONICAL_THINKING: &str = "Inspect the hidden implementation detail before answering.";

/// ctrl-t: the key the canonical journey presses to toggle thinking blocks.
const CTRL_T: &str = "\u{14}";

#[test]
fn hiding_thinking_blocks_matches_the_canonical_transcript() {
    let mut frontend = Frontend::new();
    frontend.agent(json!([
        {"kind": "agent_thinking", "payload": {"text": CANONICAL_THINKING}},
    ]));
    frontend.types(CTRL_T);

    // Canonical `thinking-hidden` rows 10..=12: the collapsed reasoning
    // block, the separator row, and the toggle's own status row.
    let expected = canonical("thinking-and-queues", "thinking-hidden");
    let actual = frontend.frame("thinking-hidden");
    assert_transcript_matches("thinking-hidden", &expected, 12, &actual, 3);
}

#[test]
fn showing_thinking_blocks_matches_the_canonical_transcript() {
    let mut frontend = Frontend::new();
    frontend.agent(json!([
        {"kind": "agent_thinking", "payload": {"text": CANONICAL_THINKING}},
    ]));
    frontend.types(CTRL_T);
    frontend.types(CTRL_T);

    // Canonical `thinking-visible` rows 10..=12. Toggling twice must leave
    // one status row, not two: the second announcement rewrites the first.
    let expected = canonical("thinking-and-queues", "thinking-visible");
    let actual = frontend.frame("thinking-visible");
    assert_transcript_matches("thinking-visible", &expected, 12, &actual, 3);
}

#[test]
fn streamed_reasoning_is_one_block_that_a_reply_closes() {
    let mut frontend = Frontend::new();
    frontend.types("Say hello please\r");
    frontend.agent(json!([
        {"kind": "agent_thinking_delta", "payload": {"text": "Inspect the hidden "}},
        {"kind": "agent_thinking_delta", "payload": {"text": "implementation detail "}},
        {"kind": "agent_thinking", "payload": {"text": CANONICAL_THINKING}},
        message("Hello!"),
    ]));

    // Three reasoning events must leave one reasoning block, and the reply
    // that follows must not be appended to it.
    let actual = frontend.frame("streamed-reasoning");
    let last = usize::from(last_transcript_row(&actual));
    assert_eq!(
        row_text(&actual.cells, actual.columns, last),
        " Hello!",
        "the reply must be its own block against the prompt"
    );
    assert_eq!(
        row_text(&actual.cells, actual.columns, last - 2),
        format!(" {CANONICAL_THINKING}"),
        "streamed reasoning must collapse into one block"
    );
    assert_eq!(
        row_text(&actual.cells, actual.columns, last - 1),
        "",
        "blocks stay separated by one untouched row"
    );
}

// ---------------------------------------------------------------------------
// Canonical queue rows
// ---------------------------------------------------------------------------

/// The prompt the canonical `thinking-and-queues` journey is steering.
const CANONICAL_PROMPT: &str = "Summarize the design";

/// alt+enter: the key the canonical journey presses to queue a follow-up.
const ALT_ENTER: &str = "\u{1b}\r";

/// The action the shipped agent emits when a queue accepts a message.
fn accepted(queue: &str, text: &str, depth: u64) -> Value {
    json!({"kind": "agent_queued", "payload": {
        "queue": queue,
        "text": text,
        "accepted": true,
        "depth": depth,
    }})
}

/// The `frontend_submit` intent one input batch produced.
fn submit_intent(batch: &DispatchBatch) -> Value {
    batch
        .actions
        .iter()
        .find(|action| action.kind == "frontend_submit")
        .map(|action| action.payload.clone())
        .expect("a submitted line must produce one frontend_submit intent")
}

/// Submit `CANONICAL_PROMPT`, then steer the running turn the way the
/// canonical journey does: type a line and press enter while it works.
fn steer(frontend: &mut Frontend) {
    frontend.types(&format!("{CANONICAL_PROMPT}\r"));
    let batch = frontend.dispatch(json!({"kind": "input", "data": "also check tests\r"}));
    let intent = submit_intent(&batch);
    assert_eq!(
        intent.get("queue").and_then(Value::as_str),
        Some("steer"),
        "enter during a turn must offer the line as steering: {intent}"
    );
    assert_eq!(
        intent.get("text").and_then(Value::as_str),
        Some("also check tests")
    );
    frontend.agent(json!([accepted("steer", "also check tests", 1)]));
}

#[test]
fn a_steered_message_matches_the_canonical_queue_rows() {
    let mut frontend = Frontend::new();
    steer(&mut frontend);

    // Canonical `steering-queued` rows 10..=15: the user block that started
    // the turn, the untouched separator, the steering row, and the hint.
    let expected = canonical("thinking-and-queues", "steering-queued");
    let actual = frontend.frame("steering-queued");
    assert_transcript_matches("steering-queued", &expected, 15, &actual, 6);
}

#[test]
fn a_queued_follow_up_matches_the_canonical_queue_rows() {
    let mut frontend = Frontend::new();
    steer(&mut frontend);

    let batch = frontend.dispatch(json!({
        "kind": "input",
        "data": format!("then run lint{ALT_ENTER}"),
    }));
    let intent = submit_intent(&batch);
    assert_eq!(
        intent.get("queue").and_then(Value::as_str),
        Some("follow_up"),
        "alt+enter during a turn must offer the line as a follow-up: {intent}"
    );
    frontend.agent(json!([accepted("follow_up", "then run lint", 1)]));

    // Canonical `follow-up-queued` rows 9..=15. Both queued messages sit in
    // one block with no internal separator, steering first.
    let expected = canonical("thinking-and-queues", "follow-up-queued");
    let actual = frontend.frame("follow-up-queued");
    assert_transcript_matches("follow-up-queued", &expected, 15, &actual, 7);
}

#[test]
fn a_drained_queue_row_becomes_the_ordinary_user_block() {
    let mut frontend = Frontend::new();
    steer(&mut frontend);

    // The turn takes the steered message: the pending row must leave as the
    // user block arrives, or the same text would be on screen twice.
    frontend.agent(json!([
        {"kind": "agent_steered", "payload": {"text": "also check tests"}},
    ]));

    let actual = frontend.frame("steering-drained");
    let last = usize::from(last_transcript_row(&actual));
    assert_eq!(
        row_text(&actual.cells, actual.columns, last - 1),
        " also check tests",
        "the drained message must become a user block"
    );
    for offset in 0..6 {
        let row = row_text(&actual.cells, actual.columns, last - offset);
        assert!(
            !row.contains("Steering:") && !row.contains("Alt+Up"),
            "a drained message must not stay queued: {row:?}"
        );
    }
}
