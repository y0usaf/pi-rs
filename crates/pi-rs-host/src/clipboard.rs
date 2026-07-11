//! `pi.clipboard` — OS clipboard mechanisms: text writes (spec:
//! `utils/clipboard.ts` `copyToClipboard`) and image reads (spec:
//! `utils/clipboard-image.ts` `readClipboardImage`).
//!
//! Mechanism only (DESIGN difference 5): platform-tool probing, bounded
//! subprocess I/O, OSC 52, image format preference, and PNG conversion.
//! What to copy/read and how to present the outcome stays in
//! `interactive.lua` policy.
//!
//! Boundary (recorded): the spec's native-addon path
//! (`@mariozechner/clipboard`, a NAPI wrapper over a vendored
//! clipboard-rs — macOS/Windows and non-Wayland X11 Linux) is not
//! ported; image reads behave like a pi install where
//! `loadClipboardNative` could not resolve the addon (`clipboard = null`).
//! Text writes remain complete: Pi deliberately skips that addon on Linux,
//! while macOS/Windows fall through to the standard pbcopy/clip tools.

use std::collections::HashMap;
use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mlua::{Lua, Table};

const SUPPORTED_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];

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

/// Spec: `readClipboardImage(options?)`. The native-addon branch resolves
/// to "addon not loaded" (module-doc boundary).
pub(crate) async fn read_clipboard_image(env: Env, platform: &str) -> Option<ClipboardImage> {
    if env_var(&env, "TERMUX_VERSION").is_some() {
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
        // Spec: `!image && !wayland` → native clipboard (not loaded here).
    }
    // Spec: non-linux platforms → native clipboard (not loaded here).

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

async fn write_clipboard_text(text: &str, env: &Env, platform: &str) -> mlua::Result<()> {
    // The optional native addon is deliberately unavailable (module boundary).
    let remote = env_truthy(env, "SSH_CONNECTION")
        || env_truthy(env, "SSH_CLIENT")
        || env_truthy(env, "MOSH_CONNECTION");
    let mut copied = false;

    if platform == "darwin" {
        copied = write_command("pbcopy", &[], text).await;
    } else if platform == "win32" {
        copied = write_command("clip", &[], text).await;
    } else {
        if env_var(env, "TERMUX_VERSION").is_some() {
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
    let clipboard = lua.create_table()?;
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
            match read_clipboard_image(env, &platform).await {
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
        lua.create_async_function(|_, (text, options): (String, Option<Table>)| async move {
            let (env, platform) = match options {
                Some(options) => {
                    let env = match options.get::<Option<Table>>("env")? {
                        Some(table) => {
                            let mut env = Env::new();
                            for pair in table.pairs::<String, String>() {
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
            write_clipboard_text(&text, &env, &platform).await
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
