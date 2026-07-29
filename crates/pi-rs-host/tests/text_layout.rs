//! File-backed Unicode measurement driving retained-display layout.
//!
//! The package below is the consumer: it decides every visible thing (where to
//! wrap, what the ellipsis is, which row the caret sits on) and asks the host
//! only for cell arithmetic. The assertions pin the invariant that makes the
//! surface usable: what Lua measures is what the rasterizer paints.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const LAYOUT: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local text = terminal.text

local PARAGRAPH = "abc 世界 x\ndone"
local INPUT = "世界x"
local COLUMNS = 8

roots.register({
  kind="application", id="text-layout", active=true, priority=0,
  dispatch=function()
    -- Layout policy: how wide the body is, and how many rows it may take.
    local exact = text.measure(PARAGRAPH, { width = 10 })
    local metrics = text.measure(PARAGRAPH, { width = COLUMNS })
    local rows, overflow = text.wrap(PARAGRAPH, { width = COLUMNS })

    -- Footer policy: this package picks the ellipsis and the budget.
    local footer, footer_width, footer_truncated =
      text.truncate("abc 世界 x", { width = 6, ellipsis = "…" })

    -- Caret policy: the host reports cluster widths, Lua counts columns.
    local clusters, cluster_count = text.graphemes(INPUT)
    local caret = 0
    for index = 1, 2 do caret = caret + clusters[index].width end
    local second = string.sub(INPUT, clusters[2].byte, clusters[2].byte + #clusters[2].text - 1)

    local tabbed = text.measure("a\tb", { width = 20, tab_width = 4 })

    local nodes = {
      {
        id=1, rect={x=0, y=0, width=12, height=6},
        content={kind="group"}, children={2, 3, 4},
      },
      {
        id=2, rect={x=1, y=1, width=COLUMNS, height=metrics.rows},
        content={kind="text", wrap="grapheme", runs={{text=PARAGRAPH}}},
      },
      {
        id=3, rect={x=1, y=4, width=10, height=1}, focusable=true,
        content={kind="text", wrap="clip", runs={{text=INPUT}}},
      },
      {
        id=4, rect={x=1, y=5, width=10, height=1},
        content={kind="text", wrap="clip", runs={{text=footer}}},
      },
    }
    local display = terminal.display()
    local submitted = display:submit({
      version=terminal.display_schema_version,
      viewport={columns=12, rows=6}, root=1, nodes=nodes,
      focused=3, cursor={node=3, row=0, column=caret, shape="bar"},
    })

    roots.action("text_layout", {
      exact={rows=exact.rows, max_width=exact.max_width,
             last_width=exact.last_width, cells=exact.cells},
      metrics={rows=metrics.rows, max_width=metrics.max_width,
               last_width=metrics.last_width, cells=metrics.cells},
      rows=rows, overflow=overflow,
      footer={text=footer, width=footer_width, truncated=footer_truncated},
      caret=caret, clusters=cluster_count, second=second,
      tabbed={cells=tabbed.cells, last_width=tabbed.last_width},
      predicted=metrics.cells + footer_width + text.width(INPUT),
      painted=submitted.painted_cells,
    })
  end,
})
"#;

const REFUSALS: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local text = terminal.text

local function fails(callable, ...)
  local ok, err = pcall(callable, ...)
  if ok then return "accepted" end
  return tostring(err)
end

roots.register({
  kind="application", id="text-refusals", active=true, priority=0,
  dispatch=function()
    local clipped_rows, clipped_overflow =
      text.wrap("abcdefghij", { width = 4, limit = 1 })
    local window, total = text.graphemes("e\u{301}xy", { offset = 1, limit = 1 })
    local narrow, narrow_width = text.truncate("abcdef", { width = 1, ellipsis = "..." })
    local untouched, _, untouched_truncated =
      text.truncate("世界", { width = 4, ellipsis = "…" })

    roots.action("text_refusals", {
      tab=fails(text.width, "a\tb"),
      newline=fails(text.width, "a\nb"),
      escape=fails(text.width, "a\27[0m"),
      display_escape=fails(function()
        terminal.display():submit({
          version=terminal.display_schema_version,
          viewport={columns=4, rows=1}, root=1,
          nodes={{id=1, rect={x=0, y=0, width=4, height=1},
                  content={kind="text", runs={{text="a\27[0m"}}}}},
        })
      end),
      zero_width=fails(text.measure, "a", { width = 0 }),
      zero_tab=fails(text.measure, "a", { width = 4, tab_width = 0 }),
      no_width=fails(text.measure, "a", {}),
      bad_wrap=fails(text.measure, "a", { width = 4, wrap = "word" }),
      zero_limit=fails(text.graphemes, "abc", { limit = 0 }),
      huge_limit=fails(text.wrap, "abc", { width = 4, limit = 99999 }),
      oversize=fails(text.width, string.rep("a", terminal.text.max_bytes + 1)),
      clipped={rows=clipped_rows, overflow=clipped_overflow},
      window={text=window[1].text, width=window[1].width,
              byte=window[1].byte, count=#window, total=total},
      narrow={text=narrow, width=narrow_width},
      untouched={text=untouched, truncated=untouched_truncated},
      bounds={max_bytes=text.max_bytes, max_graphemes=text.max_graphemes,
              default_max_graphemes=text.default_max_graphemes,
              max_rows=text.max_rows, default_max_rows=text.default_max_rows},
    })
  end,
})
"#;

fn run(source: &str, name: &str) -> serde_json::Value {
    let directory = tempfile::tempdir().expect("temporary package directory");
    let path = directory.path().join(format!("{name}.lua"));
    std::fs::write(&path, source).expect("write file-backed package");
    let host = Host::new(HostConfig::default()).expect("host starts");
    host.load_package(PackageSource::File { path: &path })
        .expect("package loads");
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({ "kind": "probe" }),
            serde_json::json!({}),
        ))
        .expect("dispatch");
    batch.actions[0].payload.clone()
}

#[test]
fn measured_text_matches_the_cells_the_display_paints() {
    let payload = run(LAYOUT, "text-layout");

    // "abc 世界 x" is exactly ten cells wide, so a ten-column node keeps it on
    // one row: the wrap test is `column + width > width`, not `>=`.
    assert_eq!(
        payload["exact"],
        serde_json::json!({"rows": 2, "max_width": 10, "last_width": 4, "cells": 14})
    );
    // At eight columns the space before "x" no longer fits, so the row breaks
    // before it. Grapheme wrapping never moves a word; word policy is Lua's.
    assert_eq!(
        payload["metrics"],
        serde_json::json!({"rows": 3, "max_width": 8, "last_width": 4, "cells": 14})
    );
    assert_eq!(
        payload["rows"],
        serde_json::json!(["abc 世界", " x", "done"])
    );
    assert_eq!(payload["overflow"], false);

    // A wide cluster that does not fit the remaining budget is dropped whole.
    assert_eq!(
        payload["footer"],
        serde_json::json!({"text": "abc …", "width": 5, "truncated": true})
    );

    // Two clusters of width two put the caret in column four, and cluster byte
    // offsets index the source string directly.
    assert_eq!(payload["caret"], 4);
    assert_eq!(payload["clusters"], 3);
    assert_eq!(payload["second"], "界");

    // A tab expands to the next multiple of tab_width: "a" + three spaces + "b".
    assert_eq!(
        payload["tabbed"],
        serde_json::json!({"cells": 5, "last_width": 5})
    );

    // The invariant: the package predicted the painted cell count from
    // measurement alone, before submitting anything.
    assert_eq!(payload["predicted"], payload["painted"]);
    assert_eq!(payload["painted"], 24);
}

#[test]
fn text_primitives_refuse_what_the_display_refuses_and_stay_bounded() {
    let payload = run(REFUSALS, "text-refusals");

    // Single-line arithmetic refuses the two graphemes that change row or
    // column by layout rather than by width.
    for member in ["tab", "newline"] {
        assert!(
            payload[member]
                .as_str()
                .is_some_and(|error| error.contains("must not contain a newline or tab")),
            "{member}: {}",
            payload[member]
        );
    }
    // Measurement and submission share one control-data rule, so text that
    // measures is text that submits.
    for member in ["escape", "display_escape"] {
        assert!(
            payload[member]
                .as_str()
                .is_some_and(|error| error.contains("terminal control data")),
            "{member}: {}",
            payload[member]
        );
    }
    assert!(
        payload["zero_width"]
            .as_str()
            .is_some_and(|error| error.contains("layout width must be non-zero"))
    );
    assert!(
        payload["zero_tab"]
            .as_str()
            .is_some_and(|error| error.contains("invalid tab width 0"))
    );
    assert!(
        payload["no_width"]
            .as_str()
            .is_some_and(|error| error.contains("requires a width"))
    );
    assert!(
        payload["bad_wrap"]
            .as_str()
            .is_some_and(|error| error.contains("wrap must be grapheme or clip"))
    );
    assert!(
        payload["zero_limit"]
            .as_str()
            .is_some_and(|error| error.contains("limit must be in 1..=16384"))
    );
    assert!(
        payload["huge_limit"]
            .as_str()
            .is_some_and(|error| error.contains("limit must be in 1..=16384"))
    );
    assert!(
        payload["oversize"]
            .as_str()
            .is_some_and(|error| error.contains("limit is 1048576"))
    );

    // A row budget clips instead of allocating: the caller learns the text did
    // not fit and decides what to do with the remainder.
    assert_eq!(
        payload["clipped"],
        serde_json::json!({"rows": ["abcd"], "overflow": true})
    );
    // "e" plus a combining acute is one cluster of one cell, so the requested
    // window starts at "x" three bytes in.
    assert_eq!(
        payload["window"],
        serde_json::json!({"text": "x", "width": 1, "byte": 4, "count": 1, "total": 3})
    );
    // An ellipsis wider than the whole budget is dropped rather than overflowing.
    assert_eq!(
        payload["narrow"],
        serde_json::json!({"text": "a", "width": 1})
    );
    // Text that already fits is returned unchanged and reports no truncation.
    assert_eq!(
        payload["untouched"],
        serde_json::json!({"text": "世界", "truncated": false})
    );
    assert_eq!(
        payload["bounds"],
        serde_json::json!({
            "max_bytes": 1_048_576,
            "max_graphemes": 16_384,
            "default_max_graphemes": 1_024,
            "max_rows": 16_384,
            "default_max_rows": 1_024
        })
    );
}
