//! Key decoding for legacy terminal sequences, xterm modifyOtherKeys and the Kitty keyboard
//! protocol (upstream `keys.ts`). Key ids are strings such as `"ctrl+c"` or `"shift+enter"`.

use std::sync::atomic::{AtomicBool, Ordering};

static KITTY_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Set the global Kitty keyboard protocol state (done by the terminal after detection).
pub fn set_kitty_protocol_active(active: bool) {
    KITTY_ACTIVE.store(active, Ordering::Relaxed);
}

/// Whether the Kitty keyboard protocol is active.
pub fn is_kitty_protocol_active() -> bool {
    KITTY_ACTIVE.load(Ordering::Relaxed)
}

const SYMBOL_KEYS: &str = "`-=[]\\;',./!@#$%^&*()_+|~{}:<>?";
const SHIFTED_SYMBOL_KEYS: &str = "~!@#$%^&*()_+|{}:<>?";

const SHIFT: i64 = 1;
const ALT: i64 = 2;
const CTRL: i64 = 4;
const SUPER: i64 = 8;
const LOCK_MASK: i64 = 64 + 128;

const CP_ESCAPE: i64 = 27;
const CP_TAB: i64 = 9;
const CP_ENTER: i64 = 13;
const CP_SPACE: i64 = 32;
const CP_BACKSPACE: i64 = 127;
const CP_KP_ENTER: i64 = 57414;

const UP: i64 = -1;
const DOWN: i64 = -2;
const RIGHT: i64 = -3;
const LEFT: i64 = -4;

const DELETE: i64 = -10;
const INSERT: i64 = -11;
const PAGE_UP: i64 = -12;
const PAGE_DOWN: i64 = -13;
const HOME: i64 = -14;
const END: i64 = -15;

/// JS 32-bit `&` on two numbers.
fn and(a: i64, b: i64) -> i64 {
    ((a as i32) & (b as i32)) as i64
}

/// `String.fromCharCode(cp)` as a char, when it is one (lone surrogates are not).
fn from_char_code(cp: i64) -> Option<char> {
    char::from_u32(cp.rem_euclid(65536) as u32)
}

fn is_symbol_key_cp(cp: i64, set: &str) -> bool {
    from_char_code(cp).is_some_and(|c| set.contains(c))
}

fn normalize_kitty_functional(cp: i64) -> i64 {
    match cp {
        57399..=57408 => cp - 57399 + 48,
        57409 => 46,
        57410 => 47,
        57411 => 42,
        57412 => 45,
        57413 => 43,
        57415 => 61,
        57416 => 44,
        57417 => LEFT,
        57418 => RIGHT,
        57419 => UP,
        57420 => DOWN,
        57421 => PAGE_UP,
        57422 => PAGE_DOWN,
        57423 => HOME,
        57424 => END,
        57425 => INSERT,
        57426 => DELETE,
        _ => cp,
    }
}

fn normalize_shifted_letter(cp: i64, modifier: i64) -> i64 {
    let effective = and(modifier, !LOCK_MASK);
    if and(effective, SHIFT) != 0 && (65..=90).contains(&cp) { cp + 32 } else { cp }
}

fn logical_shifted_symbol(cp: Option<i64>, modifier: i64, allowed: &str) -> Option<(i64, i64)> {
    let cp = cp?;
    if and(modifier, SHIFT) == 0 {
        return None;
    }
    let n = normalize_kitty_functional(cp);
    if !is_symbol_key_cp(n, allowed) {
        return None;
    }
    Some((n, and(modifier, !SHIFT)))
}

fn legacy_key_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1b[A", "\x1bOA"],
        "down" => &["\x1b[B", "\x1bOB"],
        "right" => &["\x1b[C", "\x1bOC"],
        "left" => &["\x1b[D", "\x1bOD"],
        "home" => &["\x1b[H", "\x1bOH", "\x1b[1~", "\x1b[7~"],
        "end" => &["\x1b[F", "\x1bOF", "\x1b[4~", "\x1b[8~"],
        "insert" => &["\x1b[2~"],
        "delete" => &["\x1b[3~"],
        "pageUp" => &["\x1b[5~", "\x1b[[5~"],
        "pageDown" => &["\x1b[6~", "\x1b[[6~"],
        "clear" => &["\x1b[E", "\x1bOE"],
        "f1" => &["\x1bOP", "\x1b[11~", "\x1b[[A"],
        "f2" => &["\x1bOQ", "\x1b[12~", "\x1b[[B"],
        "f3" => &["\x1bOR", "\x1b[13~", "\x1b[[C"],
        "f4" => &["\x1bOS", "\x1b[14~", "\x1b[[D"],
        "f5" => &["\x1b[15~", "\x1b[[E"],
        "f6" => &["\x1b[17~"],
        "f7" => &["\x1b[18~"],
        "f8" => &["\x1b[19~"],
        "f9" => &["\x1b[20~"],
        "f10" => &["\x1b[21~"],
        "f11" => &["\x1b[23~"],
        "f12" => &["\x1b[24~"],
        _ => &[],
    }
}

fn legacy_shift_sequence(key: &str) -> Option<&'static str> {
    Some(match key {
        "up" => "\x1b[a",
        "down" => "\x1b[b",
        "right" => "\x1b[c",
        "left" => "\x1b[d",
        "clear" => "\x1b[e",
        "insert" => "\x1b[2$",
        "delete" => "\x1b[3$",
        "pageUp" => "\x1b[5$",
        "pageDown" => "\x1b[6$",
        "home" => "\x1b[7$",
        "end" => "\x1b[8$",
        _ => return None,
    })
}

fn legacy_ctrl_sequence(key: &str) -> Option<&'static str> {
    Some(match key {
        "up" => "\x1bOa",
        "down" => "\x1bOb",
        "right" => "\x1bOc",
        "left" => "\x1bOd",
        "clear" => "\x1bOe",
        "insert" => "\x1b[2^",
        "delete" => "\x1b[3^",
        "pageUp" => "\x1b[5^",
        "pageDown" => "\x1b[6^",
        "home" => "\x1b[7^",
        "end" => "\x1b[8^",
        _ => return None,
    })
}

fn legacy_sequence_key_id(data: &str) -> Option<&'static str> {
    Some(match data {
        "\x1bOA" => "up",
        "\x1bOB" => "down",
        "\x1bOC" => "right",
        "\x1bOD" => "left",
        "\x1bOH" => "home",
        "\x1bOF" => "end",
        "\x1b[E" | "\x1bOE" => "clear",
        "\x1bOe" => "ctrl+clear",
        "\x1b[e" => "shift+clear",
        "\x1b[2~" => "insert",
        "\x1b[2$" => "shift+insert",
        "\x1b[2^" => "ctrl+insert",
        "\x1b[3$" => "shift+delete",
        "\x1b[3^" => "ctrl+delete",
        "\x1b[[5~" => "pageUp",
        "\x1b[[6~" => "pageDown",
        "\x1b[a" => "shift+up",
        "\x1b[b" => "shift+down",
        "\x1b[c" => "shift+right",
        "\x1b[d" => "shift+left",
        "\x1bOa" => "ctrl+up",
        "\x1bOb" => "ctrl+down",
        "\x1bOc" => "ctrl+right",
        "\x1bOd" => "ctrl+left",
        "\x1b[5$" => "shift+pageUp",
        "\x1b[6$" => "shift+pageDown",
        "\x1b[7$" => "shift+home",
        "\x1b[8$" => "shift+end",
        "\x1b[5^" => "ctrl+pageUp",
        "\x1b[6^" => "ctrl+pageDown",
        "\x1b[7^" => "ctrl+home",
        "\x1b[8^" => "ctrl+end",
        "\x1bOP" | "\x1b[11~" | "\x1b[[A" => "f1",
        "\x1bOQ" | "\x1b[12~" | "\x1b[[B" => "f2",
        "\x1bOR" | "\x1b[13~" | "\x1b[[C" => "f3",
        "\x1bOS" | "\x1b[14~" | "\x1b[[D" => "f4",
        "\x1b[[E" | "\x1b[15~" => "f5",
        "\x1b[17~" => "f6",
        "\x1b[18~" => "f7",
        "\x1b[19~" => "f8",
        "\x1b[20~" => "f9",
        "\x1b[21~" => "f10",
        "\x1b[23~" => "f11",
        "\x1b[24~" => "f12",
        "\x1bb" => "alt+left",
        "\x1bf" => "alt+right",
        "\x1bp" => "alt+up",
        "\x1bn" => "alt+down",
        _ => return None,
    })
}

fn matches_legacy_modifier(data: &str, key: &str, modifier: i64) -> bool {
    if modifier == SHIFT {
        return legacy_shift_sequence(key) == Some(data);
    }
    if modifier == CTRL {
        return legacy_ctrl_sequence(key) == Some(data);
    }
    false
}

/// Kitty event types (flag 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    Press,
    Repeat,
    Release,
}

fn has_event_marker(data: &str, n: char) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    ['u', '~', 'A', 'B', 'C', 'D', 'H', 'F'].iter().any(|f| data.contains(&format!(":{n}{f}")))
}

/// Whether the input is a Kitty key release event.
pub fn is_key_release(data: &str) -> bool {
    has_event_marker(data, '3')
}

/// Whether the input is a Kitty key repeat event.
pub fn is_key_repeat(data: &str) -> bool {
    has_event_marker(data, '2')
}

/// A byte cursor for the small fixed grammars upstream matches with regexes.
struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn new(s: &'a str) -> Self {
        Cur { b: s.as_bytes(), i: 0 }
    }
    fn lit(&mut self, s: &str) -> bool {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            true
        } else {
            false
        }
    }
    /// `\d*`, as text.
    fn digits(&mut self) -> &'a str {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i]).unwrap()
    }
    /// `\d+`, rolling back on failure.
    fn digits1(&mut self) -> Option<&'a str> {
        let d = self.digits();
        (!d.is_empty()).then_some(d)
    }
    /// `(?:<sep>(\d+))?`
    fn opt_num(&mut self, sep: &str) -> Option<&'a str> {
        let save = self.i;
        if self.lit(sep) {
            if let Some(d) = self.digits1() {
                return Some(d);
            }
        }
        self.i = save;
        None
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn end_with(&mut self, c: u8) -> bool {
        self.peek() == Some(c) && self.i + 1 == self.b.len()
    }
}

fn num(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(i64::MAX)
}

/// `^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$`:
/// codepoint, shifted, base, modifier, event.
fn match_csi_u(data: &str) -> Option<(i64, Option<i64>, Option<i64>, Option<i64>, Option<&str>)> {
    let mut c = Cur::new(data);
    if !c.lit("\x1b[") {
        return None;
    }
    let cp = num(c.digits1()?);
    let mut shifted = None;
    let save = c.i;
    if c.lit(":") {
        let d = c.digits();
        shifted = Some(d);
    } else {
        c.i = save;
    }
    let base = c.opt_num(":");
    let modv = c.opt_num(";");
    let event = c.opt_num(":");
    if !c.end_with(b'u') {
        return None;
    }
    let shifted = shifted.filter(|s| !s.is_empty()).map(num);
    Some((cp, shifted, base.map(num), modv.map(num), event))
}

#[derive(Debug, Clone, Copy)]
struct KittySeq {
    codepoint: i64,
    shifted: Option<i64>,
    base: Option<i64>,
    modifier: i64,
    #[allow(dead_code)]
    event: KeyEventType,
}

fn event_type(s: Option<&str>) -> KeyEventType {
    match s.map(num) {
        Some(2) => KeyEventType::Repeat,
        Some(3) => KeyEventType::Release,
        _ => KeyEventType::Press,
    }
}

fn parse_kitty(data: &str) -> Option<KittySeq> {
    if let Some((codepoint, shifted, base, modv, event)) = match_csi_u(data) {
        return Some(KittySeq { codepoint, shifted, base, modifier: modv.unwrap_or(1) - 1, event: event_type(event) });
    }
    // ESC [ 1 ; mod (:event)? (A|B|C|D|H|F)
    let mut c = Cur::new(data);
    if c.lit("\x1b[1;") {
        if let Some(m) = c.digits1() {
            let event = c.opt_num(":");
            if let Some(last) = c.peek() {
                if c.i + 1 == c.b.len() {
                    let cp = match last {
                        b'A' => Some(UP),
                        b'B' => Some(DOWN),
                        b'C' => Some(RIGHT),
                        b'D' => Some(LEFT),
                        b'H' => Some(HOME),
                        b'F' => Some(END),
                        _ => None,
                    };
                    if let Some(codepoint) = cp {
                        return Some(KittySeq { codepoint, shifted: None, base: None, modifier: num(m) - 1, event: event_type(event) });
                    }
                }
            }
        }
    }
    // ESC [ num (;mod)? (:event)? ~
    let mut c = Cur::new(data);
    if c.lit("\x1b[") {
        if let Some(k) = c.digits1() {
            let modv = c.opt_num(";");
            let event = c.opt_num(":");
            if c.end_with(b'~') {
                let cp = match num(k) {
                    2 => Some(INSERT),
                    3 => Some(DELETE),
                    5 => Some(PAGE_UP),
                    6 => Some(PAGE_DOWN),
                    7 => Some(HOME),
                    8 => Some(END),
                    _ => None,
                };
                if let Some(codepoint) = cp {
                    let modifier = modv.map_or(1, num) - 1;
                    return Some(KittySeq { codepoint, shifted: None, base: None, modifier, event: event_type(event) });
                }
            }
        }
    }
    None
}

fn matches_kitty(data: &str, expected_cp: i64, expected_mod: i64) -> bool {
    let Some(p) = parse_kitty(data) else { return false };
    let actual_mod = and(p.modifier, !LOCK_MASK);
    let expected = and(expected_mod, !LOCK_MASK);
    let cp = normalize_shifted_letter(normalize_kitty_functional(p.codepoint), p.modifier);
    let want = normalize_shifted_letter(normalize_kitty_functional(expected_cp), expected_mod);
    if actual_mod == expected && cp == want {
        return true;
    }
    if let Some((lcp, lmod)) = logical_shifted_symbol(p.shifted, actual_mod, SYMBOL_KEYS) {
        if and(lmod, !LOCK_MASK) == expected && lcp == normalize_kitty_functional(expected_cp) {
            return true;
        }
    }
    if actual_mod != expected {
        return false;
    }
    if p.base == Some(expected_cp) {
        let latin = (97..=122).contains(&cp);
        if !latin && !is_symbol_key_cp(cp, SYMBOL_KEYS) {
            return true;
        }
    }
    false
}

/// `^\x1b\[27;(\d+);(\d+)~$`: (codepoint, modifier).
fn parse_modify_other_keys(data: &str) -> Option<(i64, i64)> {
    let mut c = Cur::new(data);
    if !c.lit("\x1b[27;") {
        return None;
    }
    let m = num(c.digits1()?);
    if !c.lit(";") {
        return None;
    }
    let cp = num(c.digits1()?);
    if !c.end_with(b'~') {
        return None;
    }
    Some((cp, m - 1))
}

fn matches_modify_other_keys(data: &str, keycode: i64, modifier: i64) -> bool {
    parse_modify_other_keys(data) == Some((keycode, modifier))
}

fn is_windows_terminal_session() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    set("WT_SESSION") && !set("SSH_CONNECTION") && !set("SSH_CLIENT") && !set("SSH_TTY")
}

fn matches_raw_backspace(data: &str, modifier: i64) -> bool {
    if data == "\x7f" {
        return modifier == 0;
    }
    if data != "\x08" {
        return false;
    }
    if is_windows_terminal_session() { modifier == CTRL } else { modifier == 0 }
}

fn raw_ctrl_char(key: &str) -> Option<char> {
    let c = key.chars().next()?;
    if c.is_ascii_lowercase() || matches!(c, '[' | '\\' | ']' | '_') {
        return char::from_u32(c as u32 & 0x1f);
    }
    if c == '-' {
        return Some('\x1f');
    }
    None
}

fn matches_printable_modify_other_keys(data: &str, keycode: i64, modifier: i64) -> bool {
    let Some((cp, m)) = parse_modify_other_keys(data) else { return false };
    if m == modifier && normalize_shifted_letter(cp, m) == normalize_shifted_letter(keycode, modifier) {
        return true;
    }
    logical_shifted_symbol(Some(cp), m, SHIFTED_SYMBOL_KEYS) == Some((keycode, modifier))
}

fn format_key_name(key: &str, modifier: i64) -> Option<String> {
    let effective = and(modifier, !LOCK_MASK);
    if and(effective, !(SHIFT | CTRL | ALT | SUPER)) != 0 {
        return None;
    }
    let mut mods = Vec::new();
    if effective & SHIFT != 0 {
        mods.push("shift");
    }
    if effective & CTRL != 0 {
        mods.push("ctrl");
    }
    if effective & ALT != 0 {
        mods.push("alt");
    }
    if effective & SUPER != 0 {
        mods.push("super");
    }
    Some(if mods.is_empty() { key.to_string() } else { format!("{}+{key}", mods.join("+")) })
}

/// A key id split into its key and modifier bits.
fn parse_key_id(key_id: &str) -> Option<(String, i64)> {
    let mut key = key_id.to_lowercase();
    let mut seen = 0i64;
    loop {
        let found = [("shift", SHIFT), ("alt", ALT), ("ctrl", CTRL), ("super", SUPER)]
            .into_iter()
            .find(|(name, _)| key.starts_with(&format!("{name}+")));
        let Some((name, bit)) = found else { break };
        if seen & bit != 0 {
            return None;
        }
        seen |= bit;
        key = key[name.len() + 1..].to_string();
    }
    if key.is_empty() || (key.contains('+') && key != "+") {
        return None;
    }
    Some((key, seen))
}

/// Whether raw terminal input `data` is the key `key_id` (e.g. `"ctrl+c"`, `"shift+enter"`).
pub fn matches_key(data: &str, key_id: &str) -> bool {
    let Some((key, modifier)) = parse_key_id(key_id) else { return false };
    let kitty = is_kitty_protocol_active();
    let mk = |cp: i64, m: i64| matches_kitty(data, cp, m);
    let mo = |cp: i64, m: i64| matches_modify_other_keys(data, cp, m);
    let legacy = |name: &str| legacy_key_sequences(name).contains(&data);
    let functional = |name: &str, cp: i64| {
        if modifier == 0 {
            return legacy(name) || mk(cp, 0);
        }
        if matches_legacy_modifier(data, name, modifier) {
            return true;
        }
        mk(cp, modifier)
    };
    match key.as_str() {
        "escape" | "esc" => {
            return modifier == 0 && (data == "\x1b" || mk(CP_ESCAPE, 0) || mo(CP_ESCAPE, 0));
        }
        "space" => {
            if !kitty {
                if modifier == CTRL && data == "\x00" {
                    return true;
                }
                if modifier == ALT && data == "\x1b " {
                    return true;
                }
            }
            if modifier == 0 {
                return data == " " || mk(CP_SPACE, 0) || mo(CP_SPACE, 0);
            }
            return mk(CP_SPACE, modifier) || mo(CP_SPACE, modifier);
        }
        "tab" => {
            if modifier == SHIFT {
                return data == "\x1b[Z" || mk(CP_TAB, SHIFT) || mo(CP_TAB, SHIFT);
            }
            if modifier == 0 {
                return data == "\t" || mk(CP_TAB, 0);
            }
            return mk(CP_TAB, modifier) || mo(CP_TAB, modifier);
        }
        "enter" | "return" => {
            if modifier == SHIFT {
                if mk(CP_ENTER, SHIFT) || mk(CP_KP_ENTER, SHIFT) || mo(CP_ENTER, SHIFT) {
                    return true;
                }
                return kitty && (data == "\x1b\r" || data == "\n");
            }
            if modifier == ALT {
                if mk(CP_ENTER, ALT) || mk(CP_KP_ENTER, ALT) || mo(CP_ENTER, ALT) {
                    return true;
                }
                return !kitty && data == "\x1b\r";
            }
            if modifier == 0 {
                return data == "\r"
                    || (!kitty && data == "\n")
                    || data == "\x1bOM"
                    || mk(CP_ENTER, 0)
                    || mk(CP_KP_ENTER, 0);
            }
            return mk(CP_ENTER, modifier) || mk(CP_KP_ENTER, modifier) || mo(CP_ENTER, modifier);
        }
        "backspace" => {
            if modifier == ALT {
                if data == "\x1b\x7f" || data == "\x1b\x08" {
                    return true;
                }
                return mk(CP_BACKSPACE, ALT) || mo(CP_BACKSPACE, ALT);
            }
            if modifier == CTRL {
                if matches_raw_backspace(data, CTRL) {
                    return true;
                }
                return mk(CP_BACKSPACE, CTRL) || mo(CP_BACKSPACE, CTRL);
            }
            if modifier == 0 {
                return matches_raw_backspace(data, 0) || mk(CP_BACKSPACE, 0) || mo(CP_BACKSPACE, 0);
            }
            return mk(CP_BACKSPACE, modifier) || mo(CP_BACKSPACE, modifier);
        }
        "insert" => return functional("insert", INSERT),
        "delete" => return functional("delete", DELETE),
        "clear" => {
            if modifier == 0 {
                return legacy("clear");
            }
            return matches_legacy_modifier(data, "clear", modifier);
        }
        "home" => return functional("home", HOME),
        "end" => return functional("end", END),
        "pageup" => return functional("pageUp", PAGE_UP),
        "pagedown" => return functional("pageDown", PAGE_DOWN),
        "up" | "down" => {
            let (cp, alt_seq) = if key == "up" { (UP, "\x1bp") } else { (DOWN, "\x1bn") };
            if modifier == ALT {
                return data == alt_seq || mk(cp, ALT);
            }
            return functional(&key, cp);
        }
        "left" | "right" => {
            let left = key == "left";
            let cp = if left { LEFT } else { RIGHT };
            let (csi_alt, legacy_alt, esc_word, csi_ctrl) = if left {
                ("\x1b[1;3D", "\x1bB", "\x1bb", "\x1b[1;5D")
            } else {
                ("\x1b[1;3C", "\x1bF", "\x1bf", "\x1b[1;5C")
            };
            if modifier == ALT {
                return data == csi_alt || (!kitty && data == legacy_alt) || data == esc_word || mk(cp, ALT);
            }
            if modifier == CTRL {
                return data == csi_ctrl || matches_legacy_modifier(data, &key, CTRL) || mk(cp, CTRL);
            }
            return functional(&key, cp);
        }
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
            return modifier == 0 && legacy(&key);
        }
        _ => {}
    }
    let mut it = key.chars();
    let (Some(kc), None) = (it.next(), it.next()) else { return false };
    let is_letter = kc.is_ascii_lowercase();
    let is_digit = kc.is_ascii_digit();
    if !(is_letter || is_digit || SYMBOL_KEYS.contains(kc)) {
        return false;
    }
    let cp = kc as i64;
    let raw_ctrl = raw_ctrl_char(&key);
    if modifier == CTRL + ALT && !kitty {
        if let Some(rc) = raw_ctrl {
            if data == format!("\x1b{rc}") {
                return true;
            }
        }
    }
    if modifier == ALT && !kitty && data == format!("\x1b{kc}") {
        return true;
    }
    let mpo = |m: i64| matches_printable_modify_other_keys(data, cp, m);
    if modifier == CTRL {
        if raw_ctrl.is_some_and(|rc| data.len() == rc.len_utf8() && data.starts_with(rc)) {
            return true;
        }
        return mk(cp, CTRL) || mpo(CTRL);
    }
    if modifier == SHIFT + CTRL {
        return mk(cp, SHIFT + CTRL) || mpo(SHIFT + CTRL);
    }
    if modifier == SHIFT {
        if is_letter && data == kc.to_ascii_uppercase().to_string() {
            return true;
        }
        return mk(cp, SHIFT) || mpo(SHIFT);
    }
    if modifier != 0 {
        return mk(cp, modifier) || mpo(modifier);
    }
    data == key || mk(cp, 0) || mpo(0)
}

fn format_parsed_key(codepoint: i64, modifier: i64, base: Option<i64>) -> Option<String> {
    let n = normalize_kitty_functional(codepoint);
    let id = normalize_shifted_letter(n, modifier);
    let known = (97..=122).contains(&id) || (48..=57).contains(&id) || is_symbol_key_cp(id, SYMBOL_KEYS);
    let e = if known { id } else { base.unwrap_or(id) };
    let name: String = match e {
        CP_ESCAPE => "escape".into(),
        CP_TAB => "tab".into(),
        CP_ENTER | CP_KP_ENTER => "enter".into(),
        CP_SPACE => "space".into(),
        CP_BACKSPACE => "backspace".into(),
        DELETE => "delete".into(),
        INSERT => "insert".into(),
        HOME => "home".into(),
        END => "end".into(),
        PAGE_UP => "pageUp".into(),
        PAGE_DOWN => "pageDown".into(),
        UP => "up".into(),
        DOWN => "down".into(),
        LEFT => "left".into(),
        RIGHT => "right".into(),
        48..=57 | 97..=122 => from_char_code(e)?.to_string(),
        _ if is_symbol_key_cp(e, SYMBOL_KEYS) => from_char_code(e)?.to_string(),
        _ => return None,
    };
    format_key_name(&name, modifier)
}

/// The key id for raw input, if recognized.
pub fn parse_key(data: &str) -> Option<String> {
    if let Some(k) = parse_kitty(data) {
        let l = logical_shifted_symbol(k.shifted, k.modifier, SYMBOL_KEYS);
        let (cp, m) = l.unwrap_or((k.codepoint, k.modifier));
        return format_parsed_key(cp, m, k.base);
    }
    if let Some((cp0, m0)) = parse_modify_other_keys(data) {
        let (cp, m) = logical_shifted_symbol(Some(cp0), m0, SHIFTED_SYMBOL_KEYS).unwrap_or((cp0, m0));
        return format_parsed_key(cp, m, None);
    }
    let kitty = is_kitty_protocol_active();
    if kitty && (data == "\x1b\r" || data == "\n") {
        return Some("shift+enter".into());
    }
    if let Some(id) = legacy_sequence_key_id(data) {
        return Some(id.into());
    }
    let fixed = match data {
        "\x1b" => Some("escape"),
        "\x1c" => Some("ctrl+\\"),
        "\x1d" => Some("ctrl+]"),
        "\x1f" => Some("ctrl+-"),
        "\x1b\x1b" => Some("ctrl+alt+["),
        "\x1b\x1c" => Some("ctrl+alt+\\"),
        "\x1b\x1d" => Some("ctrl+alt+]"),
        "\x1b\x1f" => Some("ctrl+alt+-"),
        "\t" => Some("tab"),
        "\r" | "\x1bOM" => Some("enter"),
        "\n" if !kitty => Some("enter"),
        "\x00" => Some("ctrl+space"),
        " " => Some("space"),
        "\x7f" => Some("backspace"),
        "\x08" => Some(if is_windows_terminal_session() { "ctrl+backspace" } else { "backspace" }),
        "\x1b[Z" => Some("shift+tab"),
        "\x1b\r" if !kitty => Some("alt+enter"),
        "\x1b " if !kitty => Some("alt+space"),
        "\x1b\x7f" | "\x1b\x08" => Some("alt+backspace"),
        "\x1bB" if !kitty => Some("alt+left"),
        "\x1bF" if !kitty => Some("alt+right"),
        _ => None,
    };
    if let Some(f) = fixed {
        return Some(f.into());
    }
    let units: Vec<u16> = data.encode_utf16().collect();
    if !kitty && units.len() == 2 && units[0] == 0x1b {
        let code = units[1] as i64;
        if (1..=26).contains(&code) {
            return Some(format!("ctrl+alt+{}", from_char_code(code + 96)?));
        }
        if (97..=122).contains(&code) || (48..=57).contains(&code) || is_symbol_key_cp(code, SYMBOL_KEYS) {
            return Some(format!("alt+{}", from_char_code(code)?));
        }
    }
    let arrows = match data {
        "\x1b[A" => Some("up"),
        "\x1b[B" => Some("down"),
        "\x1b[C" => Some("right"),
        "\x1b[D" => Some("left"),
        "\x1b[H" | "\x1bOH" => Some("home"),
        "\x1b[F" | "\x1bOF" => Some("end"),
        "\x1b[3~" => Some("delete"),
        "\x1b[5~" => Some("pageUp"),
        "\x1b[6~" => Some("pageDown"),
        _ => None,
    };
    if let Some(a) = arrows {
        return Some(a.into());
    }
    if units.len() == 1 {
        let code = units[0] as i64;
        if (1..=26).contains(&code) {
            return Some(format!("ctrl+{}", from_char_code(code + 96)?));
        }
        if (32..=126).contains(&code) {
            return Some(data.to_string());
        }
    }
    None
}

/// The printable character of a Kitty CSI-u key event (plain or Shift only).
pub fn decode_kitty_printable(data: &str) -> Option<String> {
    let (cp, shifted, _, modv, _) = match_csi_u(data)?;
    let modifier = modv.unwrap_or(1) - 1;
    if and(modifier, !(SHIFT | LOCK_MASK)) != 0 {
        return None;
    }
    if modifier & (ALT | CTRL) != 0 {
        return None;
    }
    let mut e = cp;
    if modifier & SHIFT != 0 {
        if let Some(s) = shifted {
            e = s;
        }
    }
    e = normalize_kitty_functional(e);
    if e < 32 {
        return None;
    }
    from_code_point(e)
}

/// `String.fromCodePoint`, including lone surrogates as U+FFFD (Rust strings cannot hold them).
fn from_code_point(cp: i64) -> Option<String> {
    if cp > 0x10ffff {
        return None;
    }
    Some(char::from_u32(cp as u32).unwrap_or('\u{FFFD}').to_string())
}

fn decode_modify_other_keys_printable(data: &str) -> Option<String> {
    let (cp, m) = parse_modify_other_keys(data)?;
    if and(and(m, !LOCK_MASK), !SHIFT) != 0 || cp < 32 {
        return None;
    }
    from_code_point(cp)
}

/// The printable character of a Kitty or modifyOtherKeys key event.
pub fn decode_printable_key(data: &str) -> Option<String> {
    decode_kitty_printable(data).or_else(|| decode_modify_other_keys_printable(data))
}
