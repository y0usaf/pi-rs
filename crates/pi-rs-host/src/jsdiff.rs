//! 1:1 port of jsdiff 8.0.4 (`ref/pi/node_modules/diff`), restricted to the
//! entry points Pi's coding agent uses with default options:
//!
//! - `diffLines(old, new)` (`diff/line.js`) — `core/tools/edit-diff.ts`
//!   `generateDiffString`;
//! - `diffWords(old, new)` (`diff/word.js`) — `components/diff.ts`
//!   intra-line highlighting;
//! - `createTwoFilesPatch(...)` (`patch/create.js`) — `edit-diff.ts`
//!   `generateUnifiedPatch` (context 4, `FILE_HEADERS_ONLY`).
//!
//! The Myers loop, tie-breaking, token stitching, whitespace dedupe, and
//! patch assembly follow the vendored JavaScript exactly; parity is pinned
//! by `tests/jsdiff-parity/` fixtures generated from the vendored library.

use std::collections::HashMap;
use std::rc::Rc;

/// Errors mirroring jsdiff's internal `throw Error(...; this is a bug)`
/// guards plus the (unreachable with default options) exhausted-edit-length
/// outcome, so the port stays total without panicking.
#[derive(Debug, thiserror::Error)]
pub enum JsDiffError {
    #[error("jsdiff internal error: {0}")]
    Internal(String),
}

/// One change object, matching jsdiff 8's `ChangeObject` shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub value: String,
    pub count: usize,
    pub added: bool,
    pub removed: bool,
}

// ---------------------------------------------------------------------------
// diff/base.js — Myers O(ND) with jsdiff's edge-clamping optimization
// ---------------------------------------------------------------------------

struct Node {
    count: usize,
    added: bool,
    removed: bool,
    prev: Option<Rc<Node>>,
}

#[derive(Clone)]
struct Path {
    old_pos: isize,
    last: Option<Rc<Node>>,
}

struct Component {
    count: usize,
    added: bool,
    removed: bool,
}

fn add_to_path(path: &Path, added: bool, removed: bool, old_pos_inc: isize) -> Path {
    // base.js addToPath (options.oneChangePerToken is never set here).
    match &path.last {
        Some(last) if last.added == added && last.removed == removed => Path {
            old_pos: path.old_pos + old_pos_inc,
            last: Some(Rc::new(Node {
                count: last.count + 1,
                added,
                removed,
                prev: last.prev.clone(),
            })),
        },
        last => Path {
            old_pos: path.old_pos + old_pos_inc,
            last: Some(Rc::new(Node {
                count: 1,
                added,
                removed,
                prev: last.clone(),
            })),
        },
    }
}

fn extract_common(
    path: &mut Path,
    new_tokens: &[String],
    old_tokens: &[String],
    diagonal: isize,
    equals: &dyn Fn(&str, &str) -> bool,
) -> isize {
    let new_len = new_tokens.len() as isize;
    let old_len = old_tokens.len() as isize;
    let mut old_pos = path.old_pos;
    let mut new_pos = old_pos - diagonal;
    let mut common_count = 0usize;
    while new_pos + 1 < new_len
        && old_pos + 1 < old_len
        && equals(
            &old_tokens[(old_pos + 1) as usize],
            &new_tokens[(new_pos + 1) as usize],
        )
    {
        new_pos += 1;
        old_pos += 1;
        common_count += 1;
    }
    if common_count > 0 {
        path.last = Some(Rc::new(Node {
            count: common_count,
            added: false,
            removed: false,
            prev: path.last.clone(),
        }));
    }
    path.old_pos = old_pos;
    new_pos
}

fn components_of(last: Option<Rc<Node>>) -> Vec<Component> {
    // base.js buildValues step 1: reverse the linked list.
    let mut components = Vec::new();
    let mut cursor = last;
    while let Some(node) = cursor {
        components.push(Component {
            count: node.count,
            added: node.added,
            removed: node.removed,
        });
        cursor = node.prev.clone();
    }
    components.reverse();
    components
}

fn diff_core(
    old_tokens: &[String],
    new_tokens: &[String],
    equals: &dyn Fn(&str, &str) -> bool,
) -> Result<Vec<Component>, JsDiffError> {
    let new_len = new_tokens.len() as isize;
    let old_len = old_tokens.len() as isize;
    let max_edit_length = new_len + old_len;

    let mut best_path: HashMap<isize, Path> = HashMap::new();
    let mut path0 = Path {
        old_pos: -1,
        last: None,
    };
    // Seed editLength = 0, i.e. the content starts with the same values.
    let new_pos = extract_common(&mut path0, new_tokens, old_tokens, 0, equals);
    if path0.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
        return Ok(components_of(path0.last));
    }
    best_path.insert(0, path0);

    // -Infinity / Infinity in the JS source; diagonals are bounded by
    // ±editLength ≤ ±maxEditLength so quarter-range sentinels are safe.
    let mut min_diagonal = isize::MIN / 4;
    let mut max_diagonal = isize::MAX / 4;

    let mut edit_length: isize = 1;
    while edit_length <= max_edit_length {
        let mut diagonal = min_diagonal.max(-edit_length);
        let hi = max_diagonal.min(edit_length);
        while diagonal <= hi {
            // "No one else is going to attempt to use this value, clear it."
            let remove_path = best_path.remove(&(diagonal - 1));
            let add_path_old_pos = best_path.get(&(diagonal + 1)).map(|path| path.old_pos);

            let can_add = add_path_old_pos.is_some_and(|old_pos| {
                // What newPos will be after we do an insertion:
                let add_path_new_pos = old_pos - diagonal;
                add_path_new_pos >= 0 && add_path_new_pos < new_len
            });
            let can_remove = remove_path
                .as_ref()
                .is_some_and(|path| path.old_pos + 1 < old_len);

            if !can_add && !can_remove {
                // If this path is a terminal then prune.
                best_path.remove(&diagonal);
                diagonal += 2;
                continue;
            }

            // Select the prior path whose position in the old string is the
            // farthest from the origin and does not pass the graph bounds.
            let take_add = !can_remove
                || (can_add && remove_path.as_ref().map(|path| path.old_pos) < add_path_old_pos);
            let seed = if take_add {
                best_path.get(&(diagonal + 1)).cloned()
            } else {
                remove_path
            };
            let Some(seed) = seed else {
                // Unreachable: canAdd/canRemove guarantee the seed exists.
                best_path.remove(&diagonal);
                diagonal += 2;
                continue;
            };
            let mut base_path = if take_add {
                add_to_path(&seed, true, false, 0)
            } else {
                add_to_path(&seed, false, true, 1)
            };

            let new_pos = extract_common(&mut base_path, new_tokens, old_tokens, diagonal, equals);
            if base_path.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
                // We have hit the end of both strings.
                return Ok(components_of(base_path.last));
            }
            if base_path.old_pos + 1 >= old_len {
                max_diagonal = max_diagonal.min(diagonal - 1);
            }
            if new_pos + 1 >= new_len {
                min_diagonal = min_diagonal.max(diagonal + 1);
            }
            best_path.insert(diagonal, base_path);
            diagonal += 2;
        }
        edit_length += 1;
    }

    // Only reachable when maxEditLength is externally constrained, which the
    // exposed entry points never do.
    Err(JsDiffError::Internal(
        "edit length exhausted without a result".to_owned(),
    ))
}

fn build_values(
    components: Vec<Component>,
    new_tokens: &[String],
    old_tokens: &[String],
    join: &dyn Fn(&[String]) -> String,
) -> Vec<Change> {
    // base.js buildValues step 2 (useLongestToken is false for both
    // LineDiff and WordDiff).
    let mut changes = Vec::with_capacity(components.len());
    let mut new_pos = 0usize;
    let mut old_pos = 0usize;
    for component in components {
        let value = if !component.removed {
            let value = join(&new_tokens[new_pos..new_pos + component.count]);
            new_pos += component.count;
            if !component.added {
                old_pos += component.count;
            }
            value
        } else {
            let value = join(&old_tokens[old_pos..old_pos + component.count]);
            old_pos += component.count;
            value
        };
        changes.push(Change {
            value,
            count: component.count,
            added: component.added,
            removed: component.removed,
        });
    }
    changes
}

// ---------------------------------------------------------------------------
// diff/line.js
// ---------------------------------------------------------------------------

/// `tokenize` from line.js with default options: line content merged with
/// its `\n`/`\r\n` separator; final empty token dropped.
fn tokenize_lines(value: &str) -> Vec<String> {
    value
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>()
}

/// `Diff.diffLines(old, new)` with default options.
pub fn diff_lines(old: &str, new: &str) -> Result<Vec<Change>, JsDiffError> {
    let old_tokens = tokenize_lines(old);
    let new_tokens = tokenize_lines(new);
    let equals = |left: &str, right: &str| left == right;
    let components = diff_core(&old_tokens, &new_tokens, &equals)?;
    Ok(build_values(
        components,
        &new_tokens,
        &old_tokens,
        &|tokens| tokens.concat(),
    ))
}

// ---------------------------------------------------------------------------
// diff/word.js
// ---------------------------------------------------------------------------

/// `extendedWordChars` from word.js.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '_'
        || c == '\u{AD}'
        || ('\u{C0}'..='\u{D6}').contains(&c)
        || ('\u{D8}'..='\u{F6}').contains(&c)
        || ('\u{F8}'..='\u{2C6}').contains(&c)
        || ('\u{2C8}'..='\u{2D7}').contains(&c)
        || ('\u{2DE}'..='\u{2FF}').contains(&c)
        || ('\u{1E00}'..='\u{1EFF}').contains(&c)
}

/// JavaScript `/\s/` (and `String.prototype.trim`) whitespace set.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn trim_js(value: &str) -> &str {
    value
        .trim_start_matches(is_js_whitespace)
        .trim_end_matches(is_js_whitespace)
}

fn leading_ws(value: &str) -> &str {
    let end = value
        .char_indices()
        .find(|(_, c)| !is_js_whitespace(*c))
        .map_or(value.len(), |(i, _)| i);
    &value[..end]
}

fn trailing_ws(value: &str) -> &str {
    let start = value
        .char_indices()
        .rev()
        .find(|(_, c)| !is_js_whitespace(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    &value[start..]
}

/// `tokenizeIncludingWhitespace` alternation: word run | whitespace run |
/// single other char.
fn word_parts(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(&c) = chars.peek() {
        let mut part = String::new();
        if is_word_char(c) {
            while let Some(&next) = chars.peek() {
                if !is_word_char(next) {
                    break;
                }
                part.push(next);
                chars.next();
            }
        } else if is_js_whitespace(c) {
            while let Some(&next) = chars.peek() {
                if !is_js_whitespace(next) {
                    break;
                }
                part.push(next);
                chars.next();
            }
        } else {
            part.push(c);
            chars.next();
        }
        parts.push(part);
    }
    parts
}

/// word.js `WordDiff.tokenize` (no `intlSegmenter`): stitch whitespace parts
/// onto adjacent word/punctuation tokens.
fn tokenize_words(value: &str) -> Vec<String> {
    let parts = word_parts(value);
    let mut tokens: Vec<String> = Vec::new();
    let mut prev_part: Option<String> = None;
    for part in parts {
        let part_is_ws = part.chars().any(is_js_whitespace);
        if part_is_ws {
            match prev_part {
                None => tokens.push(part.clone()),
                Some(_) => {
                    let joined = match tokens.pop() {
                        Some(last) => last + &part,
                        None => part.clone(),
                    };
                    tokens.push(joined);
                }
            }
        } else if let Some(prev) = &prev_part
            && prev.chars().any(is_js_whitespace)
        {
            if tokens.last() == Some(prev) {
                let joined = match tokens.pop() {
                    Some(last) => last + &part,
                    None => part.clone(),
                };
                tokens.push(joined);
            } else {
                tokens.push(prev.clone() + &part);
            }
        } else {
            tokens.push(part.clone());
        }
        prev_part = Some(part);
    }
    tokens
}

/// word.js `WordDiff.join`.
fn join_words(tokens: &[String]) -> String {
    tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            if i == 0 {
                token.as_str()
            } else {
                token.trim_start_matches(is_js_whitespace)
            }
        })
        .collect()
}

// util/string.js helpers (char-wise; only ever applied around whitespace).

fn longest_common_prefix(str1: &str, str2: &str) -> String {
    str1.chars()
        .zip(str2.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect()
}

fn longest_common_suffix(str1: &str, str2: &str) -> String {
    if str1.is_empty() || str2.is_empty() || str1.chars().last() != str2.chars().last() {
        return String::new();
    }
    let chars1: Vec<char> = str1.chars().collect();
    let chars2: Vec<char> = str2.chars().collect();
    let mut i = 0usize;
    while i < chars1.len() && i < chars2.len() {
        if chars1[chars1.len() - (i + 1)] != chars2[chars2.len() - (i + 1)] {
            break;
        }
        i += 1;
    }
    chars1[chars1.len() - i..].iter().collect()
}

fn replace_prefix(string: &str, old_prefix: &str, new_prefix: &str) -> Result<String, JsDiffError> {
    let Some(rest) = string.strip_prefix(old_prefix) else {
        return Err(JsDiffError::Internal(format!(
            "string {string:?} doesn't start with prefix {old_prefix:?}; this is a bug"
        )));
    };
    Ok(format!("{new_prefix}{rest}"))
}

fn replace_suffix(string: &str, old_suffix: &str, new_suffix: &str) -> Result<String, JsDiffError> {
    if old_suffix.is_empty() {
        return Ok(format!("{string}{new_suffix}"));
    }
    let Some(rest) = string.strip_suffix(old_suffix) else {
        return Err(JsDiffError::Internal(format!(
            "string {string:?} doesn't end with suffix {old_suffix:?}; this is a bug"
        )));
    };
    Ok(format!("{rest}{new_suffix}"))
}

fn remove_prefix(string: &str, old_prefix: &str) -> Result<String, JsDiffError> {
    replace_prefix(string, old_prefix, "")
}

fn remove_suffix(string: &str, old_suffix: &str) -> Result<String, JsDiffError> {
    replace_suffix(string, old_suffix, "")
}

/// util/string.js `overlapCount` (KMP back-references) + `maximumOverlap`.
fn maximum_overlap(string1: &str, string2: &str) -> String {
    let a: Vec<char> = string1.chars().collect();
    let b: Vec<char> = string2.chars().collect();
    let start_a = a.len().saturating_sub(b.len());
    let end_b = b.len().min(a.len());
    if end_b == 0 {
        return String::new();
    }
    let mut map = vec![0usize; end_b];
    let mut k = 0usize;
    for j in 1..end_b {
        if b[j] == b[k] {
            map[j] = map[k];
        } else {
            map[j] = k;
        }
        while k > 0 && b[j] != b[k] {
            k = map[k];
        }
        if b[j] == b[k] {
            k += 1;
        }
    }
    k = 0;
    for &ch in a.iter().skip(start_a) {
        while k > 0 && ch != b[k] {
            k = map[k];
        }
        if ch == b[k] {
            k += 1;
        }
    }
    b[..k].iter().collect()
}

/// word.js `dedupeWhitespaceInChangeObjects` (no segmenter).
#[allow(clippy::too_many_lines)]
fn dedupe_whitespace(
    changes: &mut [Change],
    start_keep: Option<usize>,
    deletion: Option<usize>,
    insertion: Option<usize>,
    end_keep: Option<usize>,
) -> Result<(), JsDiffError> {
    match (deletion, insertion) {
        (Some(deletion), Some(insertion)) => {
            let old_ws_prefix = leading_ws(&changes[deletion].value).to_owned();
            let old_ws_suffix = trailing_ws(&changes[deletion].value).to_owned();
            let new_ws_prefix = leading_ws(&changes[insertion].value).to_owned();
            let new_ws_suffix = trailing_ws(&changes[insertion].value).to_owned();

            if let Some(start_keep) = start_keep {
                let common_ws_prefix = longest_common_prefix(&old_ws_prefix, &new_ws_prefix);
                changes[start_keep].value = replace_suffix(
                    &changes[start_keep].value,
                    &new_ws_prefix,
                    &common_ws_prefix,
                )?;
                changes[deletion].value =
                    remove_prefix(&changes[deletion].value, &common_ws_prefix)?;
                changes[insertion].value =
                    remove_prefix(&changes[insertion].value, &common_ws_prefix)?;
            }
            if let Some(end_keep) = end_keep {
                let common_ws_suffix = longest_common_suffix(&old_ws_suffix, &new_ws_suffix);
                changes[end_keep].value =
                    replace_prefix(&changes[end_keep].value, &new_ws_suffix, &common_ws_suffix)?;
                changes[deletion].value =
                    remove_suffix(&changes[deletion].value, &common_ws_suffix)?;
                changes[insertion].value =
                    remove_suffix(&changes[insertion].value, &common_ws_suffix)?;
            }
        }
        (None, Some(insertion)) => {
            // Each change object keeps its trailing whitespace; duplicate
            // leading whitespace is deleted where present.
            if start_keep.is_some() {
                let ws = leading_ws(&changes[insertion].value).len();
                changes[insertion].value = changes[insertion].value[ws..].to_owned();
            }
            if let Some(end_keep) = end_keep {
                let ws = leading_ws(&changes[end_keep].value).len();
                changes[end_keep].value = changes[end_keep].value[ws..].to_owned();
            }
        }
        (Some(deletion), None) => match (start_keep, end_keep) {
            (Some(start_keep), Some(end_keep)) => {
                let new_ws_full = leading_ws(&changes[end_keep].value).to_owned();
                let del_ws_start = leading_ws(&changes[deletion].value).to_owned();
                let del_ws_end = trailing_ws(&changes[deletion].value).to_owned();
                let new_ws_start = longest_common_prefix(&new_ws_full, &del_ws_start);
                changes[deletion].value = remove_prefix(&changes[deletion].value, &new_ws_start)?;
                let new_ws_end = longest_common_suffix(
                    &remove_prefix(&new_ws_full, &new_ws_start)?,
                    &del_ws_end,
                );
                changes[deletion].value = remove_suffix(&changes[deletion].value, &new_ws_end)?;
                changes[end_keep].value =
                    replace_prefix(&changes[end_keep].value, &new_ws_full, &new_ws_end)?;
                let start_replacement: String = new_ws_full
                    .chars()
                    .take(new_ws_full.chars().count() - new_ws_end.chars().count())
                    .collect();
                changes[start_keep].value =
                    replace_suffix(&changes[start_keep].value, &new_ws_full, &start_replacement)?;
            }
            (None, Some(end_keep)) => {
                // Start of the text: preserve endKeep whitespace, trim the
                // deletion's overlap with it.
                let end_keep_ws_prefix = leading_ws(&changes[end_keep].value).to_owned();
                let deletion_ws_suffix = trailing_ws(&changes[deletion].value).to_owned();
                let overlap = maximum_overlap(&deletion_ws_suffix, &end_keep_ws_prefix);
                changes[deletion].value = remove_suffix(&changes[deletion].value, &overlap)?;
            }
            (Some(start_keep), None) => {
                // End of the text: preserve startKeep whitespace, trim the
                // deletion's overlap with it.
                let start_keep_ws_suffix = trailing_ws(&changes[start_keep].value).to_owned();
                let deletion_ws_prefix = leading_ws(&changes[deletion].value).to_owned();
                let overlap = maximum_overlap(&start_keep_ws_suffix, &deletion_ws_prefix);
                changes[deletion].value = remove_prefix(&changes[deletion].value, &overlap)?;
            }
            (None, None) => {}
        },
        (None, None) => {}
    }
    Ok(())
}

/// word.js `WordDiff.postProcess`.
fn post_process_words(changes: &mut [Change]) -> Result<(), JsDiffError> {
    let mut last_keep: Option<usize> = None;
    let mut insertion: Option<usize> = None;
    let mut deletion: Option<usize> = None;
    for i in 0..changes.len() {
        if changes[i].added {
            insertion = Some(i);
        } else if changes[i].removed {
            deletion = Some(i);
        } else {
            if insertion.is_some() || deletion.is_some() {
                dedupe_whitespace(changes, last_keep, deletion, insertion, Some(i))?;
            }
            last_keep = Some(i);
            insertion = None;
            deletion = None;
        }
    }
    if insertion.is_some() || deletion.is_some() {
        dedupe_whitespace(changes, last_keep, deletion, insertion, None)?;
    }
    Ok(())
}

/// `Diff.diffWords(old, new)` with default options.
pub fn diff_words(old: &str, new: &str) -> Result<Vec<Change>, JsDiffError> {
    let old_tokens = tokenize_words(old);
    let new_tokens = tokenize_words(new);
    let equals = |left: &str, right: &str| trim_js(left) == trim_js(right);
    let components = diff_core(&old_tokens, &new_tokens, &equals)?;
    let mut changes = build_values(components, &new_tokens, &old_tokens, &|tokens| {
        join_words(tokens)
    });
    post_process_words(&mut changes)?;
    Ok(changes)
}

// ---------------------------------------------------------------------------
// patch/create.js
// ---------------------------------------------------------------------------

/// Header emission options (`INCLUDE_HEADERS` / `FILE_HEADERS_ONLY` /
/// `OMIT_HEADERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderOptions {
    Include,
    FileHeadersOnly,
    Omit,
}

impl HeaderOptions {
    fn include_index(self) -> bool {
        matches!(self, Self::Include)
    }
    fn include_underline(self) -> bool {
        matches!(self, Self::Include)
    }
    fn include_file_headers(self) -> bool {
        !matches!(self, Self::Omit)
    }
}

struct Hunk {
    old_start: isize,
    old_lines: isize,
    new_start: isize,
    new_lines: isize,
    lines: Vec<String>,
}

/// patch/create.js `splitLines`: split into lines that keep their trailing
/// newline (the final line keeps none when the text does not end with one).
fn split_lines_keep_newline(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_owned).collect()
}

fn context_slice(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| format!(" {line}")).collect()
}

fn structured_patch_hunks(
    old_str: &str,
    new_str: &str,
    context: usize,
) -> Result<Vec<Hunk>, JsDiffError> {
    let diff = diff_lines(old_str, new_str)?;
    // Append an empty value to make cleanup easier.
    let mut entries: Vec<(Vec<String>, bool, bool)> = diff
        .into_iter()
        .map(|change| {
            (
                split_lines_keep_newline(&change.value),
                change.added,
                change.removed,
            )
        })
        .collect();
    entries.push((Vec::new(), false, false));

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut old_range_start: isize = 0;
    let mut new_range_start: isize = 0;
    let mut cur_range: Vec<String> = Vec::new();
    let mut old_line: isize = 1;
    let mut new_line: isize = 1;
    let context_i = context as isize;

    for i in 0..entries.len() {
        let (lines, added, removed) = (entries[i].0.clone(), entries[i].1, entries[i].2);
        if added || removed {
            // If we have previous context, start with that.
            if old_range_start == 0 {
                old_range_start = old_line;
                new_range_start = new_line;
                if i > 0 {
                    let prev_lines = &entries[i - 1].0;
                    cur_range = if context > 0 {
                        let start = prev_lines.len().saturating_sub(context);
                        context_slice(&prev_lines[start..])
                    } else {
                        Vec::new()
                    };
                    old_range_start -= cur_range.len() as isize;
                    new_range_start -= cur_range.len() as isize;
                }
            }
            // Output our changes.
            for line in &lines {
                cur_range.push(format!("{}{line}", if added { '+' } else { '-' }));
            }
            // Track the updated file position.
            if added {
                new_line += lines.len() as isize;
            } else {
                old_line += lines.len() as isize;
            }
        } else {
            // Identical context lines. Track line changes.
            if old_range_start != 0 {
                // Close out any changes that have been output (or join overlapping).
                if lines.len() as isize <= context_i * 2 && i < entries.len() - 2 {
                    // Overlapping.
                    cur_range.extend(context_slice(&lines));
                } else {
                    // End the range and output.
                    let context_size = lines.len().min(context);
                    cur_range.extend(context_slice(&lines[..context_size]));
                    hunks.push(Hunk {
                        old_start: old_range_start,
                        old_lines: old_line - old_range_start + context_size as isize,
                        new_start: new_range_start,
                        new_lines: new_line - new_range_start + context_size as isize,
                        lines: std::mem::take(&mut cur_range),
                    });
                    old_range_start = 0;
                    new_range_start = 0;
                }
            }
            old_line += lines.len() as isize;
            new_line += lines.len() as isize;
        }
    }

    // Eliminate trailing `\n`; add "\ No newline at end of file" where needed.
    for hunk in &mut hunks {
        let mut i = 0usize;
        while i < hunk.lines.len() {
            if hunk.lines[i].ends_with('\n') {
                let trimmed = hunk.lines[i][..hunk.lines[i].len() - 1].to_owned();
                hunk.lines[i] = trimmed;
            } else {
                hunk.lines
                    .insert(i + 1, "\\ No newline at end of file".to_owned());
                i += 1; // Skip the line we just added.
            }
            i += 1;
        }
    }

    Ok(hunks)
}

/// `Diff.createTwoFilesPatch(oldFileName, newFileName, oldStr, newStr,
/// undefined, undefined, { context, headerOptions })`.
pub fn create_two_files_patch(
    old_file_name: &str,
    new_file_name: &str,
    old_str: &str,
    new_str: &str,
    context: usize,
    headers: HeaderOptions,
) -> Result<String, JsDiffError> {
    let hunks = structured_patch_hunks(old_str, new_str, context)?;

    // patch/create.js formatPatch.
    let mut ret: Vec<String> = Vec::new();
    if headers.include_index() && old_file_name == new_file_name {
        ret.push(format!("Index: {old_file_name}"));
    }
    if headers.include_underline() {
        ret.push("===================================================================".to_owned());
    }
    if headers.include_file_headers() {
        ret.push(format!("--- {old_file_name}"));
        ret.push(format!("+++ {new_file_name}"));
    }
    for hunk in hunks {
        // Unified Diff Format quirk: if the chunk size is 0, the first
        // number is one lower than one would expect.
        let old_start = if hunk.old_lines == 0 {
            hunk.old_start - 1
        } else {
            hunk.old_start
        };
        let new_start = if hunk.new_lines == 0 {
            hunk.new_start - 1
        } else {
            hunk.new_start
        };
        ret.push(format!(
            "@@ -{old_start},{} +{new_start},{} @@",
            hunk.old_lines, hunk.new_lines
        ));
        ret.extend(hunk.lines);
    }
    Ok(ret.join("\n") + "\n")
}
