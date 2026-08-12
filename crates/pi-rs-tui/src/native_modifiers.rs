//! Native modifier-key polling mechanism — spec `packages/tui/src/native-modifiers.ts`.
//!
//! Pi uses a native (NAPI) helper on macOS to recover Shift+Tab and Ctrl+Space
//! when a terminal loses modifier state, and Windows's virtual-terminal-input
//! console mode for the same recovery on `\x1b[Z`. The *decision policy* — which
//! platforms/architectures probe, how a (possibly unavailable) helper result is
//! coerced to a boolean, and how Apple-Terminal Shift+Enter is normalized — is
//! ported here as deterministic mechanism and pinned against a Pi-generated
//! oracle (`tests/platform-modifiers-parity/oracle.json`).
//!
//! Boundary (recorded): the binary helper itself (`darwin-modifiers.node`,
//! `win32-console-mode.node`) loads a native addon that pi-rs does not embed.
//! On this base the helper is exactly as unavailable as a Pi install where
//! `loadNativeModifiersHelper` finds no candidate — `isNativeModifierPressed`
//! returns `false` for every key. That is the behavior the oracle captures
//! (`modifierProbe`), and the platform-gated decision logic reproduces it.

/// Spec: `ModifierKey` — the four keys the native helper can report.
pub const MODIFIER_KEYS: [&str; 4] = ["shift", "command", "control", "option"];

/// Spec: the platforms that probe a native helper at all. Pi gates on
/// `process.platform === "darwin"`; any other platform (and any arch outside
/// x64/arm64) leaves `nativeModifiersHelper` null and every probe `false`.
pub fn supports_native_modifiers(platform: &str, arch: &str) -> bool {
    platform == "darwin" && (arch == "x64" || arch == "arm64")
}

/// Spec: `isNativeModifierPressed`. `helper` is the optional native-helper
/// result (None when the addon is unavailable/undecidable). Mirror of Pi's
/// `helper.isModifierPressed(key) === true` with the try/catch falsy default.
pub fn is_native_modifier_pressed(key: &str, helper: Option<fn(&str) -> bool>) -> bool {
    let Some(helper) = helper else {
        return false;
    };
    helper(key)
}

/// Spec: `normalizeAppleTerminalInput(data, isAppleTerminal, isShiftPressed)`.
/// Pi maps a plain `\r` from Apple Terminal to the Shift+Enter escape when the
/// native modifier poll reports shift. Pure — this is the deterministic core
/// (see `terminal.rs` `normalize_apple_terminal_input`).
pub use crate::terminal::normalize_apple_terminal_input;
