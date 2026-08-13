//! `pi.clipboard` — OS clipboard mechanisms: text writes (spec:
//! `utils/clipboard.ts` `copyToClipboard`) and image reads (spec:
//! `utils/clipboard-image.ts` `readClipboardImage`).
//!
//! Mechanism only (DESIGN difference 5): platform-tool probing, bounded
//! subprocess I/O, OSC 52, image format preference, and PNG conversion.
//! What to copy/read and how to present the outcome stays in
//! `interactive.lua` policy.
//!
//! Native addon path (`utils/clipboard-native.ts`): `pi.clipboard.load_native`
//! reproduces Pi's `loadClipboardNative` resolution — walk require roots in
//! order; the first that yields a `{setText,hasImage,getImageBinary}` module
//! wins, else null. The module-level `clipboard` gate (`!TERMUX_VERSION &&
//! hasDisplay`) and the addon-preference ordering in `copyToClipboard` /
//! `readClipboardImage` (`clipboard && p !== "linux"` for text; native image
//! read on non-Linux and non-Wayland Linux) are reproduced and pinned against a
//! Pi-generated oracle (`tests/platform-clipboard-parity/oracle.json`). The
//! binary addon itself is not embedded; Lua policy resolves a bound module via
//! `load_native` from real require roots, and on a base where none resolve the
//! behavior equals a Pi install with `clipboard = null`. Text writes remain
//! complete: Pi deliberately skips that addon on Linux.

use std::collections::HashMap;
use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mlua::{AnyUserData, Lua, Table, UserData, UserDataMethods};

/// Spec `ClipboardModule`: a resolved native clipboard addon, bound by Lua
/// policy via `load_native`.
#[derive(Clone)]
pub(crate) struct NativeClipboard {
    pub(crate) set_text: mlua::Function,
    pub(crate) has_image: mlua::Function,
    pub(crate) get_image_binary: mlua::Function,
}

impl NativeClipboard {
    /// Spec `ClipboardModule.setText` — invoke the bound addon's write.
    pub(crate) fn set_text(&self, text: &str) -> bool {
        self.set_text.call::<mlua::Value>(text).is_ok()
    }
    /// Spec `ClipboardModule.hasImage`.
    pub(crate) fn has_image(&self) -> bool {
        self.has_image.call::<bool>(()).unwrap_or(false)
    }
    /// Spec `ClipboardModule.getImageBinary` — read the byte array.
    pub(crate) fn get_image_binary(&self) -> Option<Vec<u8>> {
        let value = self.get_image_binary.call::<mlua::Value>(()).ok()?;
        let table = value.as_table()?;
        let bytes: mlua::Result<Vec<u8>> = table.sequence_values().collect();
        bytes.ok()
    }
}

/// Lua-userdata wrapper so a resolved addon can live in the registry and be
/// handed out. The mutative Lua-native calls stay synchronous (the addon's
/// methods are JS functions already awaited by Lua policy).
struct LuaNativeClipboard(NativeClipboard);
impl UserData for LuaNativeClipboard {
    fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

/// Spec `loadClipboardNative(requires?)`: walk require roots in order; the first
/// that resolves `@mariozechner/clipboard` to a module with the three methods
/// wins. A root that throws (as Pi's `require` does on a bad root) is skipped,
/// matching Pi's try/catch-per-require. `None` = addon unavailable (fallback).
pub(crate) fn load_native_clipboard(
    roots: Option<Vec<mlua::Table>>,
) -> mlua::Result<Option<NativeClipboard>> {
    let Some(roots) = roots else {
        return Ok(None);
    };
    for root in roots {
        // Per-root require: a raising metatable `__index` or a read error is a
        // throw; mirror Pi's try/catch and advance to the next root.
        let module = match root.get::<mlua::Table>("@mariozechner/clipboard") {
            Ok(module) => module,
            Err(_) => continue,
        };
        let set_text = module.get::<Option<mlua::Function>>("setText")?;
        let has_image = module.get::<Option<mlua::Function>>("hasImage")?;
        let get_image_binary = module.get::<Option<mlua::Function>>("getImageBinary")?;
        if let (Some(set_text), Some(has_image), Some(get_image_binary)) =
            (set_text, has_image, get_image_binary)
        {
            return Ok(Some(NativeClipboard {
                set_text,
                has_image,
                get_image_binary,
            }));
        }
    }
    Ok(None)
}

/// Spec module-level gate: `clipboard = !TERMUX_VERSION && hasDisplay ?
/// loadClipboardNative() : null`, with `hasDisplay = platform !== "linux" ||
/// DISPLAY || WAYLAND_DISPLAY`. This is the deterministic decision core the
/// oracle's `envProbe` exercises. `!TERMUX_VERSION` uses JS truthiness, so an
/// *empty* `TERMUX_VERSION=""` counts as absent (clipboard stays available).
pub(crate) fn clipboard_gate(platform: &str, env: &Env) -> bool {
    if env_truthy(env, "TERMUX_VERSION") {
        return false;
    }
    platform != "linux" || env_truthy(env, "DISPLAY") || env_truthy(env, "WAYLAND_DISPLAY")
}

const SUPPORTED_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Registry slot holding the resolved native addon (a `LuaNativeClipboard`).
const NATIVE_SLOT: &str = "pi.host.clipboard.native";
const DEFAULT_LIST_TIMEOUT_MS: u64 = 1000;
const DEFAULT_READ_TIMEOUT_MS: u64 = 3000;
const DEFAULT_POWERSHELL_TIMEOUT_MS: u64 = 5000;
const DEFAULT_MAX_BUFFER_BYTES: usize = 50 * 1024 * 1024;

/// Spec: `ClipboardImage`.
pub(crate) struct ClipboardImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

type Env = HashMap<String, String>;

fn env_var(env: &Env, key: &str) -> Option<String> {
    env.get(key).cloned()
}

fn env_truthy(env: &Env, key: &str) -> bool {
    // JS `Boolean(env.X)` — empty string is falsy.
    env.get(key).is_some_and(|value| !value.is_empty())
}

/// Spec: `isWaylandSession`.
fn is_wayland_session(env: &Env) -> bool {
    env_truthy(env, "WAYLAND_DISPLAY")
        || env_var(env, "XDG_SESSION_TYPE").as_deref() == Some("wayland")
}

/// Spec: `baseMimeType`.
fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_lowercase()
}

/// Spec: `extensionForImageMimeType`.
pub(crate) fn extension_for_image_mime_type(mime_type: &str) -> Option<&'static str> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// Spec: `selectPreferredImageMimeType`.
fn select_preferred_image_mime_type(mime_types: &[String]) -> Option<String> {
    let normalized: Vec<(String, String)> = mime_types
        .iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .map(|t| {
            let base = base_mime_type(&t);
            (t, base)
        })
        .collect();
    for preferred in SUPPORTED_IMAGE_MIME_TYPES {
        if let Some((raw, _)) = normalized.iter().find(|(_, base)| base == preferred) {
            return Some(raw.clone());
        }
    }
    normalized
        .iter()
        .find(|(_, base)| base.starts_with("image/"))
        .map(|(raw, _)| raw.clone())
}

/// Spec: `isSupportedImageMimeType`.
fn is_supported_image_mime_type(mime_type: &str) -> bool {
    let base = base_mime_type(mime_type);
    SUPPORTED_IMAGE_MIME_TYPES.iter().any(|t| *t == base)
}

/// Spec: `runCommand` — `spawnSync` with a timeout and a 50MB output cap;
/// spawn errors, non-zero exits, timeouts, and cap overruns all report
/// `ok: false`.
async fn run_command(command: &str, args: &[&str], timeout_ms: u64) -> Option<Vec<u8>> {
    let mut child = tokio::process::Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;

    let wait = async {
        use tokio::io::AsyncReadExt as _;
        let mut stdout = Vec::new();
        if let Some(mut pipe) = child.stdout.take() {
            let mut buf = [0u8; 64 * 1024];
            loop {
                match pipe.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        stdout.extend_from_slice(&buf[..n]);
                        if stdout.len() > DEFAULT_MAX_BUFFER_BYTES {
                            return None;
                        }
                    }
                    Err(_) => return None,
                }
            }
        }
        let status = child.wait().await.ok()?;
        if status.success() { Some(stdout) } else { None }
    };

    match tokio::time::timeout(Duration::from_millis(timeout_ms), wait).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            None
        }
    }
}

/// Spec: `readClipboardImageViaWlPaste`.
async fn read_via_wl_paste() -> Option<ClipboardImage> {
    let list = run_command("wl-paste", &["--list-types"], DEFAULT_LIST_TIMEOUT_MS).await?;
    let types: Vec<String> = String::from_utf8_lossy(&list)
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    let selected = select_preferred_image_mime_type(&types)?;
    let data = run_command(
        "wl-paste",
        &["--type", &selected, "--no-newline"],
        DEFAULT_READ_TIMEOUT_MS,
    )
    .await?;
    if data.is_empty() {
        return None;
    }
    Some(ClipboardImage {
        bytes: data,
        mime_type: base_mime_type(&selected),
    })
}

/// Spec: `isWSL`.
fn is_wsl(env: &Env) -> bool {
    if env_truthy(env, "WSL_DISTRO_NAME") || env_truthy(env, "WSLENV") {
        return true;
    }
    match std::fs::read_to_string("/proc/version") {
        Ok(release) => {
            let lower = release.to_lowercase();
            lower.contains("microsoft") || lower.contains("wsl")
        }
        Err(_) => false,
    }
}

/// Spec: `readClipboardImageViaPowerShell` — the WSL fallback for Windows
/// screenshots that never reach the Linux clipboard.
async fn read_via_powershell() -> Option<ClipboardImage> {
    let tmp_file = std::env::temp_dir().join(format!(
        "pi-wsl-clip-{}.png",
        pi_rs_session::uuid::random_uuid()
    ));
    let tmp_str = tmp_file.to_string_lossy().into_owned();
    let result = async {
        let win_path_out =
            run_command("wslpath", &["-w", &tmp_str], DEFAULT_LIST_TIMEOUT_MS).await?;
        let win_path = String::from_utf8_lossy(&win_path_out).trim().to_owned();
        if win_path.is_empty() {
            return None;
        }
        let ps_quoted = win_path.replace('\'', "''");
        let ps_script = [
            "Add-Type -AssemblyName System.Windows.Forms".to_owned(),
            "Add-Type -AssemblyName System.Drawing".to_owned(),
            format!("$path = '{ps_quoted}'"),
            "$img = [System.Windows.Forms.Clipboard]::GetImage()".to_owned(),
            "if ($img) { $img.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok' } else { Write-Output 'empty' }".to_owned(),
        ]
        .join("; ");
        let output = run_command(
            "powershell.exe",
            &["-NoProfile", "-Command", &ps_script],
            DEFAULT_POWERSHELL_TIMEOUT_MS,
        )
        .await?;
        if String::from_utf8_lossy(&output).trim() != "ok" {
            return None;
        }
        let bytes = std::fs::read(&tmp_file).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(ClipboardImage {
            bytes,
            mime_type: "image/png".to_owned(),
        })
    }
    .await;
    let _ = std::fs::remove_file(&tmp_file);
    result
}

/// Spec: `readClipboardImageViaXclip`.
async fn read_via_xclip() -> Option<ClipboardImage> {
    let targets = run_command(
        "xclip",
        &["-selection", "clipboard", "-t", "TARGETS", "-o"],
        DEFAULT_LIST_TIMEOUT_MS,
    )
    .await;
    let candidate_types: Vec<String> = match &targets {
        Some(out) => String::from_utf8_lossy(out)
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    };
    let preferred = if candidate_types.is_empty() {
        None
    } else {
        select_preferred_image_mime_type(&candidate_types)
    };
    let mut try_types: Vec<String> = Vec::new();
    if let Some(preferred) = preferred {
        try_types.push(preferred);
    }
    try_types.extend(SUPPORTED_IMAGE_MIME_TYPES.iter().map(|t| (*t).to_owned()));

    for mime_type in try_types {
        if let Some(data) = run_command(
            "xclip",
            &["-selection", "clipboard", "-t", &mime_type, "-o"],
            DEFAULT_READ_TIMEOUT_MS,
        )
        .await
            && !data.is_empty()
        {
            return Some(ClipboardImage {
                bytes: data,
                mime_type: base_mime_type(&mime_type),
            });
        }
    }
    None
}

/// Spec: `readClipboardImageViaNativeClipboard` — the native-addon branch.
/// Honored only when Lua policy bound a resolved addon (via `load_native`).
async fn read_via_native_clipboard(native: &NativeClipboard) -> Option<ClipboardImage> {
    if !native.has_image() {
        return None;
    }
    let bytes = native.get_image_binary()?;
    if bytes.is_empty() {
        return None;
    }
    Some(ClipboardImage {
        bytes,
        mime_type: "image/png".to_owned(),
    })
}

/// Spec: `readClipboardImage(options?)`.
pub(crate) async fn read_clipboard_image(
    env: Env,
    platform: &str,
    native: Option<&NativeClipboard>,
) -> Option<ClipboardImage> {
    if env_truthy(&env, "TERMUX_VERSION") {
        return None;
    }

    let mut image: Option<ClipboardImage> = None;
    if platform == "linux" {
        let wsl = is_wsl(&env);
        let wayland = is_wayland_session(&env);
        if wayland || wsl {
            image = read_via_wl_paste().await;
            if image.is_none() {
                image = read_via_xclip().await;
            }
        }
        if image.is_none() && wsl {
            image = read_via_powershell().await;
        }
        // Spec: `!image && !wayland` → native clipboard (only if bound).
        if image.is_none()
            && !wayland
            && let Some(native) = native
        {
            image = read_via_native_clipboard(native).await;
        }
    } else if let Some(native) = native {
        // Spec: non-linux platforms → native clipboard.
        image = read_via_native_clipboard(native).await;
    }

    let image = image?;
    // Convert unsupported formats (e.g., BMP from WSLg) to PNG
    if !is_supported_image_mime_type(&image.mime_type) {
        let png = crate::image::convert_bytes_to_png(&image.bytes)?;
        return Some(ClipboardImage {
            bytes: png,
            mime_type: "image/png".to_owned(),
        });
    }
    Some(image)
}

const MAX_OSC52_ENCODED_LENGTH: usize = 100_000;

async fn write_command(command: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = tokio::process::Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        if stdin.write_all(text.as_bytes()).await.is_err() {
            let _ = child.kill().await;
            return false;
        }
    }
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) => false,
        Err(_) => {
            let _ = child.kill().await;
            false
        }
    }
}

fn emit_osc52(text: &str) -> bool {
    let encoded = BASE64.encode(text);
    if encoded.len() > MAX_OSC52_ENCODED_LENGTH {
        return false;
    }
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{encoded}\x07")
        .and_then(|()| stdout.flush())
        .is_ok()
}

async fn write_clipboard_text(
    text: &str,
    env: &Env,
    platform: &str,
    native: Option<&NativeClipboard>,
) -> mlua::Result<()> {
    let remote = env_truthy(env, "SSH_CONNECTION")
        || env_truthy(env, "SSH_CLIENT")
        || env_truthy(env, "MOSH_CONNECTION");
    let mut copied = false;

    // Spec (clipboard.ts): prefer the native addon on non-Linux platforms
    // (`clipboard && p !== "linux"`), then fall through to platform tools.
    // Pi deliberately skips the addon on Linux.
    if let Some(native) = native
        && platform != "linux"
        && native.set_text(text)
    {
        copied = true;
    }
    // Mirror of `if (copied && !remote) return;`.
    if copied && !remote {
        return Ok(());
    }

    // Mirror of Pi's `if (!copied) { ... }` guard: platform tools only run when
    // the native addon did not copy. Pi deliberately skips the addon on Linux,
    // so on Linux this branch is always the primary path.
    if !copied {
        if platform == "darwin" {
            copied = write_command("pbcopy", &[], text).await;
        } else if platform == "win32" {
            copied = write_command("clip", &[], text).await;
        } else {
            if env_truthy(env, "TERMUX_VERSION") {
                copied = write_command("termux-clipboard-set", &[], text).await;
            }
            if !copied && is_wayland_session(env) && env_truthy(env, "WAYLAND_DISPLAY") {
                copied = write_command("wl-copy", &[], text).await;
            }
            if !copied && env_truthy(env, "DISPLAY") {
                copied = write_command("xclip", &["-selection", "clipboard"], text).await
                    || write_command("xsel", &["--clipboard", "--input"], text).await;
            }
        }
    }

    if remote || !copied {
        copied = emit_osc52(text) || copied;
    }
    if copied {
        Ok(())
    } else {
        Err(mlua::Error::runtime("Failed to copy to clipboard"))
    }
}

fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => {
            if other == "linux" {
                "linux"
            } else {
                "other"
            }
        }
    }
}

pub(crate) fn install(lua: &Lua, pi: &Table) -> mlua::Result<()> {
    let _ = lua.unset_named_registry_value(NATIVE_SLOT);
    let clipboard = lua.create_table()?;
    // Pure resolution core — mirror of `loadClipboardNative(requires?)`:
    // walk require roots in order; first non-throwing `@mariozechner/clipboard`
    // wins. No gate. Reports `resolved` + function `shape` for the differential.
    clipboard.set(
        "resolve_native",
        lua.create_function(|lua, roots: Option<Vec<Table>>| {
            let (resolved, shape) = match load_native_clipboard(roots)? {
                Some(_native) => {
                    let shape = lua.create_table()?;
                    shape.set("setText", "function")?;
                    shape.set("hasImage", "function")?;
                    shape.set("getImageBinary", "function")?;
                    (true, shape)
                }
                None => (false, lua.create_table()?),
            };
            let result = lua.create_table()?;
            result.set("resolved", resolved)?;
            result.set("shape", shape)?;
            Ok(result)
        })?,
    )?;
    // Module-level gate — mirror of `clipboard = !TERMUX_VERSION && hasDisplay ?
    // loadClipboardNative() : null`, where `hasDisplay = platform !== "linux" ||
    // DISPLAY || WAYLAND_DISPLAY`. Pure; drives the oracle's `envProbe`.
    clipboard.set(
        "has_display",
        lua.create_function(|_, (platform, env): (String, Option<Table>)| {
            let (platform, env) = resolve_env_platform(Some(platform), env)?;
            Ok(clipboard_gate(&platform, &env))
        })?,
    )?;
    // The real mechanism a policy calls once at startup: apply the gate, and
    // when it is open resolve through the given require roots and store the
    // addon in a registry slot so `read_image`/`write_text` prefer it. Returns
    // `{ gate, resolved, shape }` so the differential can observe everything.
    clipboard.set(
        "load_native",
        lua.create_function(
            move |lua,
                  (roots, platform, env): (Option<Vec<Table>>, Option<String>, Option<Table>)| {
                let (platform, env) = resolve_env_platform(platform, env)?;
                // Mirror of `clipboard = !TERMUX_VERSION && hasDisplay ? loadClipboardNative() : null`.
                let gate = clipboard_gate(&platform, &env);
                let (stored, resolved, shape) = if gate {
                    match load_native_clipboard(roots)? {
                        Some(native) => {
                            let shape = lua.create_table()?;
                            shape.set("setText", "function")?;
                            shape.set("hasImage", "function")?;
                            shape.set("getImageBinary", "function")?;
                            (Some(lua.create_userdata(LuaNativeClipboard(native))?), true, shape)
                        }
                        None => (None, false, lua.create_table()?),
                    }
                } else {
                    // Gate closed: Pi does not attempt the load, so clipboard
                    // is null regardless of resolvable roots.
                    (None, false, lua.create_table()?)
                };
                lua.set_named_registry_value(NATIVE_SLOT, stored)?;
                let result = lua.create_table()?;
                result.set("gate", gate)?;
                result.set("resolved", resolved)?;
                result.set("shape", shape)?;
                Ok(result)
            },
        )?,
    )?;
    clipboard.set(
        "read_image",
        lua.create_async_function(|lua, options: Option<Table>| async move {
            let (env, platform) = match &options {
                Some(options) => {
                    let env = match options.get::<Option<Table>>("env")? {
                        Some(env_table) => {
                            let mut env = Env::new();
                            for pair in env_table.pairs::<String, String>() {
                                let (key, value) = pair?;
                                env.insert(key, value);
                            }
                            env
                        }
                        None => std::env::vars().collect(),
                    };
                    let platform = options
                        .get::<Option<String>>("platform")?
                        .unwrap_or_else(|| node_platform().to_owned());
                    (env, platform)
                }
                None => (std::env::vars().collect(), node_platform().to_owned()),
            };
            let native = native_from_registry(&lua);
            match read_clipboard_image(env, &platform, native.as_ref()).await {
                None => Ok(mlua::Value::Nil),
                Some(image) => {
                    let table = lua.create_table()?;
                    table.set("bytes", lua.create_string(&image.bytes)?)?;
                    table.set("mimeType", image.mime_type)?;
                    Ok(mlua::Value::Table(table))
                }
            }
        })?,
    )?;
    clipboard.set(
        "extension_for_mime_type",
        lua.create_function(|_, mime_type: String| {
            Ok(extension_for_image_mime_type(&mime_type).map(str::to_owned))
        })?,
    )?;
    clipboard.set(
        "write_text",
        lua.create_async_function(|lua, (text, options): (String, Option<Table>)| async move {
            let (env, platform) = match options {
                Some(options) => {
                    let (platform, env) =
                        resolve_env_platform(options.get("platform")?, options.get("env")?)?;
                    (env, platform)
                }
                None => (std::env::vars().collect(), node_platform().to_owned()),
            };
            let native = native_from_registry(&lua);
            write_clipboard_text(&text, &env, &platform, native.as_ref()).await
        })?,
    )?;
    pi.set("clipboard", clipboard)?;

    // Node `crypto.randomUUID()` and `os.tmpdir()` mechanisms — the Lua
    // paste policy composes the spec's temp path from them.
    pi.set(
        "random_uuid",
        lua.create_function(|_, ()| Ok(pi_rs_session::uuid::random_uuid()))?,
    )?;
    Ok(())
}

/// Resolve platform + env from Lua, defaulting to the live process.
fn native_from_registry(lua: &Lua) -> Option<NativeClipboard> {
    lua.named_registry_value::<Option<AnyUserData>>(NATIVE_SLOT)
        .ok()
        .flatten()
        .and_then(|data| data.borrow::<LuaNativeClipboard>().ok())
        .map(|data| data.0.clone())
}

/// Resolve platform + env from Lua, defaulting to the live process.
fn resolve_env_platform(
    platform: Option<String>,
    env: Option<Table>,
) -> mlua::Result<(String, Env)> {
    let platform = platform.unwrap_or_else(|| node_platform().to_owned());
    let env = match env {
        Some(env_table) => {
            let mut env = Env::new();
            for pair in env_table.pairs::<String, String>() {
                let (key, value) = pair?;
                env.insert(key, value);
            }
            env
        }
        None => std::env::vars().collect(),
    };
    Ok((platform, env))
}
