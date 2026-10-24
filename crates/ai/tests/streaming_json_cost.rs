//! `streaming-json.test.ts`, "StreamingJsonParser cost": the parser allocates linearly in the
//! streamed text, never per accumulated text.
//!
//! The JavaScript tests measure V8 heap growth; this one counts allocated bytes with a global
//! allocator, in a single test so nothing else allocates while a measurement runs. The texts are ten
//! times smaller than the JavaScript ones so the test stays quick in a debug build; the ratios it
//! asserts are the same.
//!
//! `streaming-tool-arguments.test.ts`'s provider cost tests are not ported: `ToolCallArgumentsStream`
//! hands back an owned `JsonObject` every delta, because `ToolCall::arguments` is an owned map where
//! V8 shares one object. That per-delta copy belongs to the `ToolCall` data model, not to the parser.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use pi_ai::utils::streaming_json::StreamingJsonParser;
use serde_json::Value;

struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(size, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocated<T>(run: impl FnOnce() -> T) -> usize {
    let before = ALLOCATED.load(Ordering::Relaxed);
    let value = run();
    let after = ALLOCATED.load(Ordering::Relaxed);
    drop(value);
    after - before
}

fn file_content(size: usize) -> String {
    let mut text = String::new();
    let mut line = 0;
    while text.len() < size {
        text.push_str(&format!("const line{line} = \"value {line}\"; // \\\\ done\\n"));
        line += 1;
    }
    text.truncate(size);
    text
}

/// `write` arguments with `size` characters of file content, as their JSON text.
fn write_arguments(size: usize) -> String {
    format!(
        "{{\"path\":\"/src/file.ts\",\"content\":{}}}",
        Value::String(file_content(size))
    )
}

fn deltas(text: &str, size: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let mut end = (index + size).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == index {
            end += text[index..].chars().next().map_or(1, char::len_utf8);
        }
        pieces.push(&text[index..end]);
        index = end;
    }
    pieces
}

/// Stream `text` into a parser, taking a snapshot after every piece, as a provider does.
fn stream_snapshots(text: &str, delta: usize) -> Option<Value> {
    let mut parser = StreamingJsonParser::new();
    for piece in deltas(text, delta) {
        parser.append(piece);
        parser.snapshot();
    }
    parser.snapshot().cloned()
}

#[test]
fn parser_cost_is_linear_in_the_streamed_text() {
    // Tool-call arguments: ten times the text must cost about ten times the memory, not a hundred.
    let small = write_arguments(5_000);
    let large = write_arguments(50_000);
    let small_bytes = allocated(|| stream_snapshots(&small, 32));
    let large_bytes = allocated(|| stream_snapshots(&large, 32));
    assert!(
        large_bytes / large.len() < 200,
        "{} bytes for {} characters",
        large_bytes,
        large.len()
    );
    assert!(
        large_bytes / small_bytes < 20,
        "ten times the text cost {} times the memory",
        large_bytes / small_bytes
    );

    // A very large open array keeps its preview behind, which keeps the cost linear.
    let elements = |count: usize| {
        let items: Vec<String> = (0..count).map(|index| index.to_string()).collect();
        format!("{{\"items\":[{}]}}", items.join(","))
    };
    let small_array = elements(1_000);
    let large_array = elements(10_000);
    let small_bytes = allocated(|| stream_snapshots(&small_array, 16));
    let large_bytes = allocated(|| stream_snapshots(&large_array, 16));
    assert!(
        large_bytes / large_array.len() < 2000,
        "{} bytes for {} characters",
        large_bytes,
        large_array.len()
    );
    assert!(
        large_bytes / small_bytes < 20,
        "ten times the text cost {} times the memory",
        large_bytes / small_bytes
    );

    // The preview only moves forward, and ends on the whole value.
    let mut parser = StreamingJsonParser::new();
    let mut shown = 0;
    for piece in deltas(&large_array, 16) {
        parser.append(piece);
        let items = parser
            .snapshot()
            .and_then(|snapshot| snapshot.get("items"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        assert!(items >= shown, "the preview only moves forward");
        shown = items;
    }
    // Copying costs the open path's bytes, so the preview stays a little behind the text; it never
    // shows anything but the elements read so far.
    let expected: Value = serde_json::from_str(&large_array).expect("valid JSON");
    let expected_items = expected["items"].as_array().cloned().unwrap_or_default();
    let shown_items = parser
        .snapshot()
        .and_then(|snapshot| snapshot.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        shown_items.len() >= expected_items.len() * 8 / 10,
        "the preview kept up: {} of {} elements",
        shown_items.len(),
        expected_items.len()
    );
    // Every element but the last, which may still be arriving, matches what is read so far.
    let settled = shown_items.len().saturating_sub(1);
    assert_eq!(shown_items[..settled], expected_items[..settled]);

    // One huge piece, such as a provider's complete arguments, is also linear.
    let bytes = allocated(|| {
        let mut parser = StreamingJsonParser::new();
        parser.append(&large);
        parser.snapshot();
    });
    assert!(bytes / large.len() < 100, "{} bytes for {}", bytes, large.len());
}
