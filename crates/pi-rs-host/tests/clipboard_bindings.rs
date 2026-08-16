//! `pi.clipboard.read_image` — the `utils/clipboard-image.ts` port —
//! against scripted wl-paste/xclip stubs (the same technique as pi's
//! clipboard tests: control the tools, pin the probe order and format
//! policy). One test function: the stub directory is prepended to PATH
//! process-wide.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::{Mutex, MutexGuard};

use pi_rs_host::{Host, HostConfig};

// Serializes the tests that prepend a stub directory to the process-global
// `PATH`. Cargo runs the tests in this binary on parallel threads; without a
// shared lock, two tests writing `PATH` race and one can clobber the other's
// stub directory mid-probe, so `read_image` falls through to a nonexistent
// tool and records a spurious null. Holding the guard for the whole body keeps
// `PATH` stable for the duration of each test.
fn path_lock() -> MutexGuard<'static, ()> {
    static PATH_LOCK: Mutex<()> = Mutex::new(());
    PATH_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

const RUNNER: &str = r#"
local pi = ...
pi.register_command("clip-read", {
  handler = function(args)
    local opts = pi.json.decode(args)
    local image = pi.clipboard.read_image({ env = opts.env, platform = opts.platform })
    if not image then return { isNull = true } end
    return { mimeType = image.mimeType, size = #image.bytes, head = { image.bytes:byte(1, 8) } }
  end,
})
pi.register_command("clip-write", {
  handler = function(args)
    local opts = pi.json.decode(args)
    pi.clipboard.write_text(opts.text, { env = opts.env, platform = opts.platform })
    return { ok = true }
  end,
})
"#;

fn write_stub(dir: &std::path::Path, name: &str, body: &str) {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let mut perms = file.metadata().unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
}

fn call(host: &Host, env: serde_json::Value) -> serde_json::Value {
    host.call_command(
        "clip-read",
        &serde_json::json!({ "env": env, "platform": "linux" }).to_string(),
    )
    .expect("command")
    .expect("result")
}

#[test]
fn read_image_probes_tools_with_pi_policy() {
    let _guard = path_lock();
    let stub_dir = tempfile::tempdir().unwrap();
    let control = tempfile::tempdir().unwrap();
    let control_path = control.path().to_string_lossy().into_owned();

    // wl-paste: --list-types prints the control file; --type <mime>
    // prints the payload for that mime if present, else exits 1.
    write_stub(
        stub_dir.path(),
        "wl-paste",
        &format!(
            "#!/bin/sh\nC=\"{control_path}\"\nif [ \"$1\" = \"--list-types\" ]; then\n  [ -f \"$C/types.txt\" ] || exit 1\n  cat \"$C/types.txt\"\n  exit 0\nfi\nmime=\"$2\"\nsafe=$(printf %s \"$mime\" | tr '/;' '__')\n[ -f \"$C/wl-$safe.bin\" ] || exit 1\ncat \"$C/wl-$safe.bin\"\n"
        ),
    );
    // xclip: TARGETS prints the control targets; -t <mime> -o prints the
    // xclip payload for that mime if present, else exits 1.
    write_stub(
        stub_dir.path(),
        "xclip",
        &format!(
            "#!/bin/sh\nC=\"{control_path}\"\nmime=\"$4\"\nif [ \"$mime\" = \"TARGETS\" ]; then\n  [ -f \"$C/targets.txt\" ] || exit 1\n  cat \"$C/targets.txt\"\n  exit 0\nfi\nsafe=$(printf %s \"$mime\" | tr '/;' '__')\n[ -f \"$C/x-$safe.bin\" ] || exit 1\ncat \"$C/x-$safe.bin\"\n"
        ),
    );
    write_stub(
        stub_dir.path(),
        "wl-copy",
        &format!("#!/bin/sh\ncat > \"{control_path}/written.txt\"\n"),
    );

    let old_path = std::env::var("PATH").unwrap_or_default();
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{old_path}", stub_dir.path().to_string_lossy()),
        )
    };

    let host = Host::new(HostConfig::default()).expect("host");
    host.load("clip-test", RUNNER).expect("runner loads");
    let wayland = serde_json::json!({ "WAYLAND_DISPLAY": "wayland-1" });

    // Termux short-circuits before any probe.
    let got = call(
        &host,
        serde_json::json!({ "TERMUX_VERSION": "0.118", "WAYLAND_DISPLAY": "wayland-1" }),
    );
    assert_eq!(got["isNull"], true, "termux: {got}");

    // Non-Wayland, non-WSL Linux: no probe path (native addon not loaded).
    let got = call(&host, serde_json::json!({}));
    assert_eq!(got["isNull"], true, "bare linux: {got}");

    // Wayland with no types listed: wl-paste fails, xclip absent targets
    // walks the supported-type list and finds nothing.
    let got = call(&host, wayland.clone());
    assert_eq!(got["isNull"], true, "empty clipboard: {got}");

    // Preference order: image/png wins over earlier-listed image/webp;
    // the raw (parameterized) type is passed back to wl-paste while the
    // recorded mimeType is the lowercased base type.
    std::fs::write(
        control.path().join("types.txt"),
        "text/plain\nimage/webp\nimage/PNG;charset=binary\n",
    )
    .unwrap();
    std::fs::write(
        control.path().join("wl-image_PNG_charset=binary.bin"),
        b"PNGDATA-1",
    )
    .unwrap();
    let got = call(&host, wayland.clone());
    assert_eq!(got["mimeType"], "image/png", "{got}");
    assert_eq!(got["size"], 9);
    let head: Vec<u8> = got["head"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(&head, b"PNGDATA-");

    // No supported type: the first image/* candidate is taken, and an
    // unsupported format converts to PNG (spec: WSLg BMP case). The stub
    // serves a real 2x1 BMP; the result must be PNG bytes.
    std::fs::write(control.path().join("types.txt"), "image/bmp\n").unwrap();
    let bmp: &[u8] = &[
        0x42, 0x4D, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00, 0x28,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x18, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x13, 0x0B, 0x00, 0x00, 0x13, 0x0B, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00,
        0x00, 0x00,
    ];
    std::fs::write(control.path().join("wl-image_bmp.bin"), bmp).unwrap();
    let got = call(&host, wayland.clone());
    assert_eq!(got["mimeType"], "image/png", "bmp converts: {got}");
    let head: Vec<u8> = got["head"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(&head[1..4], b"PNG");

    // wl-paste empty: fall through to xclip, whose TARGETS pick jpeg.
    std::fs::remove_file(control.path().join("types.txt")).unwrap();
    std::fs::write(control.path().join("targets.txt"), "image/jpeg\n").unwrap();
    std::fs::write(control.path().join("x-image_jpeg.bin"), b"JPEGDATA").unwrap();
    let got = call(&host, wayland);
    assert_eq!(got["mimeType"], "image/jpeg", "xclip fallback: {got}");
    assert_eq!(got["size"], 8);

    let written = host
        .call_command(
            "clip-write",
            &serde_json::json!({
                "text": "copied text",
                "platform": "linux",
                "env": { "WAYLAND_DISPLAY": "wayland-1" },
            })
            .to_string(),
        )
        .expect("write command")
        .expect("write result");
    assert_eq!(written["ok"], true);
    assert_eq!(
        std::fs::read_to_string(control.path().join("written.txt")).unwrap(),
        "copied text"
    );
}

/// Regression for the `!copied` guard in `write_clipboard_text`: when the
/// native addon has already copied AND the session is remote, Pi falls straight
/// to OSC52 and must NOT run the per-platform tools (`pbcopy`/`clip`) a second
/// time. Binds a fake `@mariozechner/clipboard` through `load_native`, then
/// drives `write_text` on a non-Linux platform with the platform tool stubbed
/// to record whether it ran.
#[test]
fn native_copy_remote_skips_platform_tools() {
    let _guard = path_lock();
    let stub_dir = tempfile::tempdir().unwrap();
    let control = tempfile::tempdir().unwrap();
    let control_path = control.path().to_string_lossy().into_owned();
    // pbcopy stub: if the fallback wrongly ran, it writes a marker file.
    write_stub(
        stub_dir.path(),
        "pbcopy",
        &format!("#!/bin/sh\ncat > \"{control_path}/pbcopy-ran.txt\"\n"),
    );

    let old_path = std::env::var("PATH").unwrap_or_default();
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{old_path}", stub_dir.path().to_string_lossy()),
        )
    };

    let host = Host::new(HostConfig::default()).expect("host");
    // Fake native addon: records setText invocations.
    host.load(
        "native-copy-test",
        r#"
local pi = ...
pi.register_command("bind-native", {
  handler = function()
    local fake = {
      setText = function(text) _G.native_set_text_calls = (_G.native_set_text_calls or 0) + 1; return true end,
      hasImage = function() return false end,
      getImageBinary = function() return {} end,
    }
    local root = { ["@mariozechner/clipboard"] = fake }
    local out = pi.clipboard.load_native({ root }, "darwin", {})
    return { gate = out.gate, resolved = out.resolved }
  end,
})
pi.register_command("copy-remote", {
  handler = function(args)
    local text = pi.json.decode(args).text
    -- Remote SSH session on darwin with the native addon bound.
    local ok, err = pcall(pi.clipboard.write_text, text,
      { platform = "darwin", env = { SSH_CONNECTION = "host 12345 67890" } })
    return { ok = ok, err = tostring(err), native_calls = _G.native_set_text_calls or 0 }
  end,
})
"#,
    )
    .expect("runner loads");

    let bind = host
        .call_command("bind-native", "")
        .expect("bind")
        .expect("bind result");
    assert_eq!(bind["gate"], true, "gate should be open on darwin");
    assert_eq!(bind["resolved"], true, "native addon should resolve");

    let got = host
        .call_command("copy-remote", r#"{"text":"remote copy"}"#)
        .expect("copy")
        .expect("copy result");
    assert_eq!(got["ok"], true, "copy should succeed: {got}");
    assert_eq!(got["native_calls"], 1, "native addon should have copied");
    assert!(
        !control.path().join("pbcopy-ran.txt").exists(),
        "platform fallback must be skipped when native + remote"
    );
}

#[test]
fn clipboard_demo_example_exercises_the_public_surface() {
    let host = Host::new(HostConfig::default()).expect("host");
    let path = format!(
        "{}/../../examples/extensions/clipboard-demo.lua",
        env!("CARGO_MANIFEST_DIR")
    );
    host.load_file(&path).expect("example loads");
    let result = host
        .call_command("clipboard-demo", "")
        .expect("command")
        .expect("result");
    assert_eq!(result["termux_was_nil"], true);
    assert_eq!(result["no_session_was_nil"], true);
    assert_eq!(result["ext"], "jpg");
    assert_eq!(result["wrote_text"], true);
    assert_eq!(result["unsupported_ext_was_nil"], true);
    let temp_path = result["temp_path"].as_str().unwrap();
    assert!(temp_path.contains("pi-clipboard-"));
    assert!(temp_path.ends_with(".jpg"));
    assert!(
        result["wayland_kind"] == "nil" || result["wayland_kind"] == "image",
        "{result}"
    );
}
