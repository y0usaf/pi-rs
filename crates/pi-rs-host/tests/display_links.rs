//! File-backed hyperlink styling driving retained-display presentation.
//!
//! The package below is the consumer: it decides which span is a link, what the
//! target is, and when the target changes. The host only carries the target to
//! the cells that run painted and emits the out-of-band OSC 8 sequence around
//! them, so a link never becomes text and never leaks past its own cells.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const TRANSCRIPT: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local text = terminal.text

-- Link policy: this package owns the label, the target, and the styling that
-- marks a link as one. The host adds no appearance of its own.
local LABEL = "docs"
local LEAD = "see "
local TAIL = " now"

local function row(target)
  return {
    version = terminal.display_schema_version,
    viewport = { columns = 12, rows = 1 },
    root = 1,
    nodes = {
      {
        id = 1, rect = { x = 0, y = 0, width = 12, height = 1 },
        clip_children = true, content = { kind = "group" }, children = { 2 },
      },
      {
        id = 2, rect = { x = 0, y = 0, width = 12, height = 1 },
        clip_children = true,
        content = {
          kind = "text", wrap = "clip",
          runs = {
            { text = LEAD },
            { text = LABEL, link = target, style = { underline = true } },
            { text = TAIL },
          },
        },
      },
    },
  }
end

roots.register({
  kind="application", id="transcript-links", active=true, priority=0,
  dispatch=function()
    local display = terminal.display()
    local first = display:submit(row("https://example.test/a"))
    -- Same text, different target: the package changed only the link.
    local retargeted = display:submit(row("https://example.test/b"))
    local unchanged = display:submit(row("https://example.test/b"))

    roots.action("transcript_links", {
      schema_version=terminal.display_schema_version,
      first={ansi=first.ansi, painted=first.painted_cells,
             full_redraw=first.full_redraw},
      retargeted={ansi=retargeted.ansi, changed=retargeted.changed_cells,
                  full_redraw=retargeted.full_redraw},
      unchanged={ansi=unchanged.ansi},
      -- A link changes no cell arithmetic: the label still measures itself.
      label_width=text.width(LABEL),
      predicted=text.width(LEAD .. LABEL .. TAIL),
    })
  end,
})
"#;

const REFUSALS: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

local function fails(display, target)
  local ok, err = pcall(function()
    display:submit({
      version = terminal.display_schema_version,
      viewport = { columns = 8, rows = 1 },
      root = 1,
      nodes = {
        {
          id = 1, rect = { x = 0, y = 0, width = 8, height = 1 },
          clip_children = true,
          content = { kind = "text", runs = { { text = "link", link = target } } },
        },
      },
    })
  end)
  if ok then return "accepted" end
  return tostring(err)
end

roots.register({
  kind="application", id="link-refusals", active=true, priority=0,
  dispatch=function()
    local default_display = terminal.display()
    local bounded = terminal.display({ max_link_bytes = 8 })

    roots.action("link_refusals", {
      empty=fails(default_display, ""),
      bell=fails(default_display, "https://example.test/\7evil"),
      escape=fails(default_display, "https://example.test/\27]0;title"),
      newline=fails(default_display, "https://example.test/\na"),
      oversize=fails(bounded, "https://example.test/a"),
      accepted=fails(bounded, "id:short"),
      revision=default_display:revision(),
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
fn a_linked_run_is_presented_as_osc8_around_exactly_its_own_cells() {
    let payload = run(TRANSCRIPT, "transcript-links");

    // The run field was a schema change, so the version packages read to build
    // a batch reports the current schema.
    assert_eq!(payload["schema_version"], 3);

    // Hyperlink state and SGR state are two independent machines: the style
    // reset that ends the underlined label does not end the link, so this
    // checks where the OSC 8 pair sits relative to the painted glyphs rather
    // than assuming the two never interleave.
    let spans = |ansi: &str, target: &str| {
        let open = format!("\u{1b}]8;;{target}\u{1b}\\");
        let open_at = ansi.find(&open).unwrap_or_else(|| panic!("{ansi:?}"));
        let close_at = ansi[open_at..]
            .find("\u{1b}]8;;\u{1b}\\")
            .map(|offset| offset + open_at)
            .unwrap_or_else(|| panic!("{ansi:?}"));
        let label_at = ansi.find("docs").unwrap_or_else(|| panic!("{ansi:?}"));
        assert!(open_at < label_at, "{ansi:?}");
        assert!(label_at < close_at, "{ansi:?}");
        (open_at, close_at)
    };

    let first = payload["first"]["ansi"].as_str().expect("first frame ansi");
    let (_, close_at) = spans(first, "https://example.test/a");
    // The lead text is written before the link opens and the tail after it
    // closes, so the link covers its own run and nothing else.
    let tail_at = first.find(" now").unwrap_or_else(|| panic!("{first:?}"));
    assert!(close_at < tail_at, "{first:?}");
    assert!(
        first.starts_with("\u{1b}[?2026h\u{1b}[2J\u{1b}[1;1H"),
        "{first:?}"
    );
    // A hyperlink is out-of-band state, not glyphs: the target never appears as
    // painted text, so the row still paints twelve cells.
    assert_eq!(payload["first"]["painted"], 12);
    assert_eq!(payload["first"]["full_redraw"], true);

    // Changing only the target still changes those cells, so the differential
    // presenter repaints exactly the four cells of the label.
    assert_eq!(payload["retargeted"]["changed"], 4);
    assert_eq!(payload["retargeted"]["full_redraw"], false);
    let retargeted = payload["retargeted"]["ansi"]
        .as_str()
        .expect("retargeted frame ansi");
    spans(retargeted, "https://example.test/b");
    assert!(!retargeted.contains("example.test/a"), "{retargeted:?}");

    // Resubmitting the identical tree emits nothing at all.
    assert_eq!(payload["unchanged"]["ansi"], "");

    // Measurement is unaffected: a link is not text.
    assert_eq!(payload["label_width"], 4);
    assert_eq!(payload["predicted"], payload["first"]["painted"]);
}

#[test]
fn hyperlink_targets_are_refused_and_bounded_before_anything_is_retained() {
    let payload = run(REFUSALS, "link-refusals");

    // An empty target is the OSC 8 close sequence itself, so it can never open
    // a link.
    for member in ["empty", "bell", "escape", "newline"] {
        assert!(
            payload[member]
                .as_str()
                .is_some_and(|error| error.contains("hyperlink target")),
            "{member}: {}",
            payload[member]
        );
    }

    // The budget is per batch and is named in the diagnostic.
    assert!(
        payload["oversize"]
            .as_str()
            .is_some_and(|error| error.contains("hyperlink byte count 22 exceeds limit 8")),
        "{}",
        payload["oversize"]
    );
    // A target inside the same budget submits normally.
    assert_eq!(payload["accepted"], "accepted");
    // Every refusal above happened before anything was retained.
    assert_eq!(payload["revision"], 0);
}
