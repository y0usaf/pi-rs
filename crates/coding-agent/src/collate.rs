//! `String.prototype.localeCompare` as Node's ICU does it in the root locale, for the file names
//! that ls, find and the resource loaders sort. Letters compare by base letter first, then accents,
//! then case; punctuation and symbols come before digits, digits before letters, in DUCET order.

use std::cmp::Ordering;
use unicode_normalization::UnicodeNormalization;

/// ASCII punctuation and symbols in the order of the Unicode collation table.
const VARIABLE_ORDER: &str = "\t\n\u{b}\u{c}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn primary(c: char) -> (u32, u32) {
    if let Some(i) = VARIABLE_ORDER.find(c) {
        return (1, i as u32);
    }
    if c.is_ascii_digit() {
        return (3, c as u32);
    }
    if c.is_alphabetic() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        return (4, lower as u32);
    }
    if c.is_whitespace() {
        return (1, 0);
    }
    (2, c as u32)
}

struct Keys {
    primary: Vec<(u32, u32)>,
    secondary: Vec<u32>,
    tertiary: Vec<u8>,
}

fn keys(text: &str) -> Keys {
    let mut keys = Keys {
        primary: Vec::new(),
        secondary: Vec::new(),
        tertiary: Vec::new(),
    };
    for c in text.nfd() {
        if unicode_normalization::char::is_combining_mark(c) {
            keys.secondary.push(c as u32);
            continue;
        }
        keys.primary.push(primary(c));
        keys.secondary.push(0);
        keys.tertiary.push(u8::from(c.is_uppercase()));
    }
    keys
}

pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let (ka, kb) = (keys(a), keys(b));
    ka.primary
        .cmp(&kb.primary)
        .then_with(|| ka.secondary.cmp(&kb.secondary))
        .then_with(|| ka.tertiary.cmp(&kb.tertiary))
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_icu() {
        let mut v = vec![
            "b", "_a", ".git", "A", "a", "10", "9", "é", "e", "f", "-x", "~y", "$z", "+p", "#h",
            "B",
        ];
        v.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(v.join(" "), "_a -x .git #h +p ~y $z 10 9 a A b B e é f");
    }
}
