//! File-backed inline images driving retained-display presentation.
//!
//! The package below is the consumer: it owns the encoded payload, the cell
//! rectangle the image occupies, and — by reading the environment — which
//! terminal image protocol to ask for. The host only reserves nothing, places
//! the escape at that rectangle, and removes a placement by identity when it
//! changes or disappears.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

const TRANSCRIPT: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1
local env = pi.effects.v1.env

-- Protocol policy: this package picks the protocol from the environment it
-- already reads. The host names no terminal and detects nothing on its own.
local function protocol()
  local program = env.get("TERM_PROGRAM") or ""
  if program == "iTerm.app" then return "iterm2" end
  return "kitty"
end

local PAYLOAD = "iVBORw0KGgo="
local REPLACEMENT = "R0lGODlhAQAB"

local function screen(payload, with_image)
  local children = { 2 }
  local nodes = {
    {
      id = 1, rect = { x = 0, y = 0, width = 10, height = 3 },
      clip_children = true, content = { kind = "group" }, children = children,
    },
    {
      id = 2, rect = { x = 0, y = 0, width = 10, height = 1 },
      clip_children = true,
      content = { kind = "text", wrap = "clip", runs = { { text = "chart" } } },
    },
  }
  if with_image then
    children[2] = 3
    nodes[3] = {
      id = 3, rect = { x = 2, y = 1, width = 6, height = 2 },
      clip_children = true,
      content = { kind = "image", data = payload, protocol = protocol() },
    }
  end
  return {
    version = terminal.display_schema_version,
    viewport = { columns = 10, rows = 3 },
    root = 1,
    nodes = nodes,
  }
end

roots.register({
  kind="application", id="transcript-images", active=true, priority=0,
  dispatch=function()
    local display = terminal.display()
    local first = display:submit(screen(PAYLOAD, true))
    local unchanged = display:submit(screen(PAYLOAD, true))
    local replaced = display:submit(screen(REPLACEMENT, true))
    local removed = display:submit(screen(REPLACEMENT, false))

    roots.action("transcript_images", {
      schema_version=terminal.display_schema_version,
      protocol=protocol(),
      first={ansi=first.ansi, placed=first.placed_images,
             painted=first.painted_cells},
      unchanged={ansi=unchanged.ansi, placed=unchanged.placed_images},
      replaced={ansi=replaced.ansi, placed=replaced.placed_images},
      removed={ansi=removed.ansi, placed=removed.placed_images},
      -- An image occupies cells but is not glyphs, so it changes no
      -- measurement the package does for itself.
      label_width=terminal.text.width("chart"),
    })
  end,
})
"#;

const REFUSALS: &str = r#"
local pi = ...
local roots = pi.roots.v1
local terminal = pi.terminal.v1

local function fails(display, content)
  local ok, err = pcall(function()
    display:submit({
      version = terminal.display_schema_version,
      viewport = { columns = 8, rows = 2 },
      root = 1,
      nodes = {
        {
          id = 1, rect = { x = 0, y = 0, width = 8, height = 2 },
          clip_children = true, content = { kind = "group" }, children = { 2 },
        },
        {
          id = 2, rect = { x = 0, y = 0, width = 4, height = 1 },
          clip_children = true, content = content,
        },
      },
    })
  end)
  if ok then return "accepted" end
  return tostring(err)
end

local function image(data, protocol)
  return { kind = "image", data = data, protocol = protocol or "kitty" }
end

roots.register({
  kind="application", id="image-refusals", active=true, priority=0,
  dispatch=function()
    local default_display = terminal.display()
    local bounded = terminal.display({ max_image_bytes = 8 })

    roots.action("image_refusals", {
      empty=fails(default_display, image("")),
      escaped=fails(default_display, image("AAAA\27\\BBBB")),
      spaced=fails(default_display, image("AAAA BBBB")),
      unknown_protocol=fails(default_display, image("AAAA", "sixel")),
      unknown_kind=fails(default_display, { kind = "video", data = "AAAA" }),
      oversize=fails(bounded, image("AAAAAAAAAAAA")),
      accepted=fails(bounded, image("AAAAAAAA")),
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
fn an_image_node_is_placed_and_removed_out_of_band_by_identity() {
    let payload = run(TRANSCRIPT, "transcript-images");

    // A new content kind is a schema change, so the version packages read to
    // build a batch reports it.
    assert_eq!(payload["schema_version"], 3);
    // Protocol selection is package policy read from the environment; the test
    // environment is neither iTerm2 nor anything else the host inspects.
    assert_eq!(payload["protocol"], "kitty");

    let first = payload["first"]["ansi"].as_str().expect("first frame ansi");
    // The placement is addressed at the node's absolute top-left cell (row 1,
    // column 2 → 1-based 2;3), sized in cells, and asked not to move the
    // hardware cursor so the cursor restore stays authoritative.
    assert!(first.contains("\u{1b}[2;3H"), "{first:?}");
    assert!(
        first.contains("\u{1b}_Ga=T,f=100,q=2,C=1,c=6,r=2,i=1;iVBORw0KGgo=\u{1b}\\"),
        "{first:?}"
    );
    assert_eq!(payload["first"]["placed"], 1);
    // An image is not glyphs: it enters no cell, so only the text row above it
    // counts as painted.
    assert_eq!(payload["first"]["painted"], 5);
    assert_eq!(payload["label_width"], 5);

    // Resubmitting the identical tree emits nothing at all.
    assert_eq!(payload["unchanged"]["ansi"], "");
    assert_eq!(payload["unchanged"]["placed"], 0);

    // Same node, new payload: the identity is stable across frames, so the
    // terminal is told to drop that identity before the replacement arrives.
    let replaced = payload["replaced"]["ansi"]
        .as_str()
        .expect("replaced frame ansi");
    let delete_at = replaced.find("\u{1b}_Ga=d,d=I,i=1,q=2\u{1b}\\");
    let place_at = replaced.find(";R0lGODlhAQAB");
    assert!(delete_at.is_some(), "{replaced:?}");
    assert!(delete_at < place_at, "{replaced:?}");
    assert_eq!(payload["replaced"]["placed"], 1);

    // Dropping the node removes the placement by identity. Blanking the cells
    // it covered would not have removed the graphic.
    let removed = payload["removed"]["ansi"]
        .as_str()
        .expect("removed frame ansi");
    assert!(
        removed.contains("\u{1b}_Ga=d,d=I,i=1,q=2\u{1b}\\"),
        "{removed:?}"
    );
    assert!(!removed.contains("a=T"), "{removed:?}");
    assert_eq!(payload["removed"]["placed"], 0);
}

#[test]
fn image_payloads_are_refused_and_bounded_before_anything_is_retained() {
    let payload = run(REFUSALS, "image-refusals");

    // The payload is spliced verbatim into an escape sequence, so anything
    // outside the base64 alphabet could terminate it early and hand the rest to
    // the terminal as commands.
    for member in ["empty", "escaped", "spaced"] {
        assert!(
            payload[member]
                .as_str()
                .is_some_and(|error| error.contains("image payload")),
            "{member}: {}",
            payload[member]
        );
    }

    // The host encodes only the protocols it can actually emit, and names the
    // accepted set rather than silently falling back.
    assert!(
        payload["unknown_protocol"]
            .as_str()
            .is_some_and(|error| error.contains("protocol must be kitty or iterm2")),
        "{}",
        payload["unknown_protocol"]
    );
    assert!(
        payload["unknown_kind"]
            .as_str()
            .is_some_and(|error| error.contains("group, text, or image")),
        "{}",
        payload["unknown_kind"]
    );

    // The budget is per batch and is named in the diagnostic.
    assert!(
        payload["oversize"]
            .as_str()
            .is_some_and(|error| error.contains("image byte count 12 exceeds limit 8")),
        "{}",
        payload["oversize"]
    );
    // A payload inside the same budget submits normally.
    assert_eq!(payload["accepted"], "accepted");
    // Every refusal above happened before anything was retained.
    assert_eq!(payload["revision"], 0);
}
