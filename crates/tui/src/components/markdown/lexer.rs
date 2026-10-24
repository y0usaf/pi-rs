//! A port of marked 18.0.11's lexer (gfm, no pedantic, no breaks), configured as upstream
//! `components/markdown.ts` configures it: strict `~~` strikethrough, marked-cjk-friendly emphasis,
//! and the LaTeX block and inline extensions. Offsets are UTF-8 byte offsets where marked uses
//! UTF-16 code units; every slice is taken at a boundary of the same text, so tokens agree.

use super::rules::*;
use fancy_regex::Regex;

type Captures<'t> = fancy_regex::Captures<'t, str>;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Space,
    Code,
    Heading,
    Hr,
    Blockquote,
    List,
    ListItem,
    Html,
    Def,
    Table,
    Paragraph,
    Text,
    LatexBlock,
    Latex,
    Escape,
    Link,
    Image,
    Strong,
    Em,
    Codespan,
    Br,
    Del,
    Checkbox,
}

/// A marked token. Fields not used by a kind stay at their defaults.
#[derive(Debug, Clone, Default)]
pub struct Token {
    pub kind: Kind,
    pub raw: String,
    pub text: String,
    pub tokens: Vec<Token>,
    pub depth: usize,
    pub lang: Option<String>,
    pub href: String,
    pub title: Option<String>,
    pub ordered: bool,
    pub start: Option<u64>,
    pub loose: bool,
    pub items: Vec<Token>,
    pub task: bool,
    pub checked: Option<bool>,
    pub pending: bool,
    pub tag: String,
    pub header: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
    /// Index into the inline queue whose result becomes `tokens`.
    inline_id: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub text: String,
    pub tokens: Vec<Token>,
    inline_id: Option<usize>,
}

impl Token {
    fn new(kind: Kind, raw: &str) -> Token {
        Token {
            kind,
            raw: raw.to_string(),
            ..Default::default()
        }
    }
    fn with_text(kind: Kind, raw: &str, text: &str) -> Token {
        Token {
            kind,
            raw: raw.to_string(),
            text: text.to_string(),
            ..Default::default()
        }
    }
}

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap_or_else(|e| panic!("bad markdown rule {s}: {e}"))
}

macro_rules! rule {
    ($name:ident, $src:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| re(&$src));
    };
}

rule!(R_NEWLINE, BLOCK_NEWLINE);
rule!(R_CODE, BLOCK_CODE);
rule!(R_FENCES, BLOCK_FENCES);
rule!(R_HEADING, BLOCK_HEADING);
rule!(R_HR, BLOCK_HR);
rule!(R_BLOCKQUOTE, BLOCK_BLOCKQUOTE);
rule!(R_LIST, BLOCK_LIST);
rule!(R_HTML, BLOCK_HTML);
rule!(R_DEF, BLOCK_DEF);
rule!(R_TABLE, BLOCK_TABLE);
rule!(R_LHEADING, BLOCK_LHEADING);
rule!(R_PARAGRAPH, BLOCK_PARAGRAPH);
rule!(R_TEXT, BLOCK_TEXT);
rule!(R_BACKPEDAL, INLINE_BACKPEDAL);
rule!(R_ANY_PUNCT, INLINE_ANY_PUNCTUATION);
rule!(R_AUTOLINK, INLINE_AUTOLINK);
rule!(R_BLOCK_SKIP, INLINE_BLOCK_SKIP);
rule!(R_BR, INLINE_BR);
rule!(R_CODESPAN, INLINE_CODE);
rule!(R_ESCAPE, INLINE_ESCAPE);
rule!(R_LINK, INLINE_LINK);
rule!(R_NOLINK, INLINE_NOLINK);
rule!(R_PUNCT, INLINE_PUNCTUATION);
rule!(R_REFLINK, INLINE_REFLINK);
rule!(R_REFLINK_SEARCH, INLINE_REFLINK_SEARCH);
rule!(R_TAG, INLINE_TAG);
rule!(R_INLINE_TEXT, INLINE_TEXT);
rule!(R_URL, INLINE_URL);
rule!(R_EM_L, INLINE_EM_STRONG_L_DELIM);

// marked `other` rules used here (JS whitespace spelled out as in rules.rs).
const WS: &str =
    r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
rule!(R_CODE_REMOVE_INDENT, r"(?m)^(?: {1,4}| {0,3}\t)");
rule!(R_INDENT_CODE_COMP, format!(r"^([{WS}]+)(?:```)"));
rule!(R_BEGINNING_SPACE, format!(r"^[{WS}]+"));
rule!(R_BLANK_LINE, r"^[ \t]*$");
rule!(R_DOUBLE_BLANK, r"\n[ \t]*\n[ \t]*$");
rule!(R_BQ_START, r"^ {0,3}>");
rule!(R_BQ_SETEXT, r"\n {0,3}((?:=+|-+) *)(?=\n|$)");
rule!(R_BQ_SETEXT2, r"(?m)^ {0,3}>[ \t]?");
rule!(R_LIST_IS_TASK, format!(r"^\[[ xX]\] +[^{WS}]"));
rule!(R_LIST_REPLACE_TASK, r"^\[[ xX]\] +");
rule!(R_LIST_TASK_CHECKBOX, r"\[[ xX]\]");
rule!(R_ANY_LINE, r"\n[^\n\r\x{2028}\x{2029}]*\n");
rule!(R_TABLE_ALIGN_CHARS, r"^\||\| *$");
rule!(R_TABLE_ROW_BLANK, r"\n[ \t]*$");
rule!(R_ALIGN_RIGHT, r"^ *-+: *$");
rule!(R_ALIGN_CENTER, r"^ *:-+: *$");
rule!(R_ALIGN_LEFT, r"^ *:-+ *$");
rule!(R_START_A, r"(?i)^<a ");
rule!(R_END_A, r"(?i)^</a>");
rule!(
    R_START_PRE,
    format!(r"(?i)^<(pre|code|kbd|script)([{WS}]|>)")
);
rule!(
    R_END_PRE,
    format!(r"(?i)^</(pre|code|kbd|script)([{WS}]|>)")
);
rule!(R_MULTI_SPACE, format!(r"[{WS}]+"));
rule!(R_UNICODE_ALNUM, r"[\p{L}\p{N}]");

// upstream markdown.ts: strict strikethrough and the LaTeX extensions.
rule!(
    R_STRICT_DEL,
    format!(
        r"^(~~)(?=[^{WS}~])((?:\\[^\n\r\x{{2028}}\x{{2029}}]|[^\\])*?(?:\\[^\n\r\x{{2028}}\x{{2029}}]|[^{WS}~\\]))~~(?=[^~]|$)"
    )
);
rule!(
    R_PENDING_MATH,
    r"\\[A-Za-z]+|[_^=+*/<>()\[\]|±≤≥≠≈∈→⇒∞∫∑√-]"
);
rule!(
    R_TABLE_DELIM_ROW,
    r"^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$"
);
rule!(
    R_LATEX_DOLLAR,
    r"^ {0,3}\$\$[ \t]*(?:\n)?([\s\S]*?)\$\$[ \t]*(?:\n|$)"
);
rule!(
    R_LATEX_BRACKET,
    r"^ {0,3}\\\[[ \t]*(?:\n)?([\s\S]*?)\\\][ \t]*(?:\n|$)"
);
rule!(
    R_LATEX_PENDING_BRACKET,
    r"^ {0,3}\\\[[ \t]*(?:\n)?([\s\S]*)$"
);
rule!(
    R_LATEX_PENDING_DOLLAR,
    r"^ {0,3}\$\$[ \t]*(?:\n)?([\s\S]*)$"
);
rule!(R_LATEX_BLOCK_START, r"(?:^|\n) {0,3}(?:\$\$|\\\[)");

pub(crate) fn test(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

pub(crate) fn exec<'t>(r: &Regex, s: &'t str) -> Option<Captures<'t>> {
    r.captures(s).ok().flatten()
}

fn group<'t>(c: &Captures<'t>, i: usize) -> Option<&'t str> {
    c.get(i).map(|m| m.as_str())
}

/// `group || ''` in JS: a missing or empty group is falsy.
fn g<'t>(c: &Captures<'t>, i: usize) -> &'t str {
    group(c, i).unwrap_or("")
}

/// JS `String.prototype.replace` with a global regex and a replacement closure.
pub(crate) fn replace_all(r: &Regex, s: &str, mut f: impl FnMut(&Captures) -> String) -> String {
    let mut out = String::new();
    let mut last = 0;
    for c in r.captures_iter(s).flatten() {
        let m = c.get(0).unwrap();
        out.push_str(&s[last..m.start()]);
        out.push_str(&f(&c));
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

/// JS `replace` with a non-global regex: only the first match.
fn replace_first(r: &Regex, s: &str, with: &str) -> String {
    match exec(r, s) {
        Some(c) => {
            let m = c.get(0).unwrap();
            format!("{}{}{}", &s[..m.start()], with, &s[m.end()..])
        }
        None => s.to_string(),
    }
}

/// Remove `\` before punctuation (`replace(anyPunctuation, '$1')`).
fn unescape_punct(s: &str) -> String {
    replace_all(&R_ANY_PUNCT, s, |c| g(c, 1).to_string())
}

/// JS `\s` / `String.prototype.trim` whitespace.
pub(crate) fn js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(js_ws)
}
pub(crate) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(js_ws)
}

/// Byte offset of `n` UTF-16 code units into `s` (clamped).
fn u16_offset(s: &str, n: usize) -> usize {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units >= n {
            return i;
        }
        units += c.len_utf16();
    }
    s.len()
}

/// `s.slice(n)` with `n` in UTF-16 units.
fn slice_from(s: &str, n: usize) -> &str {
    &s[u16_offset(s, n)..]
}

/// marked `rtrim(str, c)`: strip trailing `c` characters.
fn rtrim(s: &str, c: char) -> &str {
    s.trim_end_matches(c)
}

/// marked `splitCells`.
fn split_cells(row: &str, count: Option<usize>) -> Vec<String> {
    let bytes = row.as_bytes();
    let mut marked = String::new();
    for (i, ch) in row.char_indices() {
        if ch == '|' {
            let mut escaped = false;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                escaped = !escaped;
                j -= 1;
            }
            marked.push_str(if escaped { "|" } else { " |" });
        } else {
            marked.push(ch);
        }
    }
    let mut cells: Vec<String> = marked.split(" |").map(str::to_string).collect();
    if trim(&cells[0]).is_empty() {
        cells.remove(0);
    }
    if !cells.is_empty() && trim(cells.last().unwrap()).is_empty() {
        cells.pop();
    }
    if let Some(n) = count.filter(|&n| n > 0) {
        if cells.len() > n {
            cells.truncate(n);
        } else {
            while cells.len() < n {
                cells.push(String::new());
            }
        }
    }
    cells.iter().map(|c| trim(c).replace("\\|", "|")).collect()
}

/// marked `trimTrailingBlankLines`.
fn trim_trailing_blank_lines(s: &str) -> String {
    let lines: Vec<&str> = s.split('\n').collect();
    let mut end = lines.len() as isize - 1;
    while end >= 0 && test(&R_BLANK_LINE, lines[end as usize]) {
        end -= 1;
    }
    if lines.len() as isize - end <= 2 {
        return s.to_string();
    }
    lines[..(end + 1) as usize].join("\n")
}

/// marked `findClosingBracket(str, "()")`: -1 none, -2 unbalanced.
fn find_closing_paren(s: &str) -> isize {
    if !s.contains(')') {
        return -1;
    }
    let b = s.as_bytes();
    let mut level = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 1;
        } else if b[i] == b'(' {
            level += 1;
        } else if b[i] == b')' {
            level -= 1;
            if level < 0 {
                return i as isize;
            }
        }
        i += 1;
    }
    if level > 0 { -2 } else { -1 }
}

/// marked `expandTabs`.
fn expand_tabs(line: &str, indent: usize) -> String {
    let mut col = indent;
    let mut out = String::new();
    for ch in line.chars() {
        if ch == '\t' {
            let added = 4 - (col % 4);
            out.push_str(&" ".repeat(added));
            col += added;
        } else {
            out.push(ch);
            col += ch.len_utf16();
        }
    }
    out
}

/// Index of the first char that is not a space (`search(/[^ ]/)`), in bytes = units here.
fn search_non_space(s: &str) -> Option<usize> {
    s.find(|c| c != ' ')
}

fn first_line(s: &str) -> &str {
    s.split('\n').next().unwrap_or("")
}

fn indent_regex(kind: &str, indent: usize) -> Regex {
    let n = indent.clamp(1, 4) - 1;
    re(&match kind {
        "bullet" => format!(r"^ {{0,{n}}}(?:[*+-]|[0-9]{{1,9}}[.)])((?:[ \t][^\n]*)?(?:\n|$))"),
        "hr" => format!(r"^ {{0,{n}}}((?:- *){{3,}}|(?:_ *){{3,}}|(?:\* *){{3,}})(?:\n+|$)"),
        "fences" => format!(r"^ {{0,{n}}}(?:```|~~~)"),
        "heading" => format!(r"^ {{0,{n}}}#"),
        "html" => format!(r"(?i)^ {{0,{n}}}<(?:[a-z][^\n\r\x{{2028}}\x{{2029}}]*>|!--)"),
        _ => format!(r"^ {{0,{n}}}>"),
    })
}

#[derive(Default)]
pub(crate) struct Lexer {
    pub links: HashMap<String, (String, Option<String>)>,
    queue: Vec<String>,
    top: bool,
    in_link: bool,
    in_raw_block: bool,
    link_emitted: bool,
}

pub fn lex(src: &str) -> Vec<Token> {
    let mut lx = Lexer {
        top: true,
        ..Default::default()
    };
    let src = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut tokens = Vec::new();
    lx.block_tokens(&src, &mut tokens, false);
    let queue = std::mem::take(&mut lx.queue);
    let mut results = Vec::with_capacity(queue.len());
    for q in &queue {
        let mut out = Vec::new();
        lx.inline_tokens(q, &mut out);
        results.push(out);
    }
    fill(&mut tokens, &mut results);
    tokens
}

fn fill(tokens: &mut [Token], results: &mut [Vec<Token>]) {
    for t in tokens {
        if let Some(id) = t.inline_id.take() {
            let mut inl = std::mem::take(&mut results[id]);
            // a checkbox unshifted before the inline tokens stays first
            t.tokens.append(&mut inl);
        }
        fill(&mut t.tokens, results);
        fill(&mut t.items, results);
        for c in t.header.iter_mut().chain(t.rows.iter_mut().flatten()) {
            if let Some(id) = c.inline_id.take() {
                c.tokens = std::mem::take(&mut results[id]);
            }
        }
    }
}

fn is_para_or_text(t: Option<&Token>) -> bool {
    matches!(t.map(|t| t.kind), Some(Kind::Paragraph | Kind::Text))
}

fn append_raw(last: &mut Token, raw: &str) {
    if !last.raw.ends_with('\n') {
        last.raw.push('\n');
    }
    last.raw.push_str(raw);
}

impl Lexer {
    /// marked `lexer.inline(text)`: queue for the inline pass.
    fn inline(&mut self, text: &str) -> Option<usize> {
        self.queue.push(text.to_string());
        Some(self.queue.len() - 1)
    }

    fn set_last_queue(&mut self, text: &str) {
        if let Some(q) = self.queue.last_mut() {
            *q = text.to_string();
        }
    }

    pub(crate) fn block_tokens(
        &mut self,
        src: &str,
        tokens: &mut Vec<Token>,
        mut last_paragraph_clipped: bool,
    ) {
        let mut src = src;
        while !src.is_empty() {
            if let Some(t) = tokenize_block_latex(src) {
                src = &src[t.raw.len()..];
                tokens.push(t);
                continue;
            }
            if let Some(t) = self.space(src) {
                src = &src[t.raw.len()..];
                match tokens.last_mut() {
                    Some(last) if t.raw.len() == 1 => last.raw.push('\n'),
                    _ => tokens.push(t),
                }
                continue;
            }
            if let Some(t) = self.code(src) {
                src = &src[t.raw.len()..];
                if is_para_or_text(tokens.last()) {
                    let last = tokens.last_mut().unwrap();
                    append_raw(last, &t.raw);
                    last.text = format!("{}\n{}", last.text, t.text);
                    let text = last.text.clone();
                    self.set_last_queue(&text);
                } else {
                    tokens.push(t);
                }
                continue;
            }
            let simple = [
                Lexer::fences,
                Lexer::heading,
                Lexer::hr,
                Lexer::blockquote,
                Lexer::list,
                Lexer::html,
            ];
            if let Some(t) = simple.iter().find_map(|f| f(self, src)) {
                src = &src[t.raw.len()..];
                tokens.push(t);
                continue;
            }
            if let Some(t) = self.def(src) {
                src = &src[t.raw.len()..];
                if is_para_or_text(tokens.last()) {
                    let last = tokens.last_mut().unwrap();
                    append_raw(last, &t.raw);
                    last.text = format!("{}\n{}", last.text, t.raw);
                    let text = last.text.clone();
                    self.set_last_queue(&text);
                } else if !self.links.contains_key(&t.tag) {
                    self.links
                        .insert(t.tag.clone(), (t.href.clone(), t.title.clone()));
                    tokens.push(t);
                }
                continue;
            }
            if let Some(t) = self.table(src).or_else(|| self.lheading(src)) {
                src = &src[t.raw.len()..];
                tokens.push(t);
                continue;
            }
            // startBlock of the latexBlock extension, on src.slice(1)
            let mut cut = src;
            let mut chars = src.chars();
            chars.next();
            let rest = chars.as_str();
            let off = src.len() - rest.len();
            if let Some(m) = R_LATEX_BLOCK_START.find(rest).ok().flatten() {
                let idx = m.start() + if m.as_str().starts_with('\n') { 1 } else { 0 };
                cut = &src[..idx + off];
            }
            if self.top
                && let Some(t) = self.paragraph(cut)
            {
                let clipped_now = cut.len() != src.len();
                if last_paragraph_clipped && tokens.last().map(|t| t.kind) == Some(Kind::Paragraph)
                {
                    let last = tokens.last_mut().unwrap();
                    append_raw(last, &t.raw);
                    last.text = format!("{}\n{}", last.text, t.text);
                    let text = last.text.clone();
                    self.queue.pop();
                    self.set_last_queue(&text);
                } else {
                    tokens.push(t.clone());
                }
                last_paragraph_clipped = clipped_now;
                src = &src[t.raw.len()..];
                continue;
            }
            if let Some(t) = self.text(src) {
                src = &src[t.raw.len()..];
                if tokens.last().map(|t| t.kind) == Some(Kind::Text) {
                    let last = tokens.last_mut().unwrap();
                    append_raw(last, &t.raw);
                    last.text = format!("{}\n{}", last.text, t.text);
                    let text = last.text.clone();
                    self.queue.pop();
                    self.set_last_queue(&text);
                } else {
                    tokens.push(t);
                }
                continue;
            }
            break;
        }
        self.top = true;
    }

    fn space(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_NEWLINE, src)?;
        let raw = g(&c, 0);
        (!raw.is_empty()).then(|| Token::new(Kind::Space, raw))
    }

    fn code(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_CODE, src)?;
        let raw = trim_trailing_blank_lines(g(&c, 0));
        let text = replace_all(&R_CODE_REMOVE_INDENT, &raw, |_| String::new());
        Some(Token::with_text(Kind::Code, &raw, &text))
    }

    fn fences(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_FENCES, src)?;
        let raw = g(&c, 0);
        let text = indent_code_compensation(raw, g(&c, 3));
        let mut t = Token::with_text(Kind::Code, raw, &text);
        t.lang = group(&c, 2).map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                unescape_punct(trim(l))
            }
        });
        Some(t)
    }

    fn heading(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_HEADING, src)?;
        let mut text = trim(g(&c, 2)).to_string();
        if text.ends_with('#') {
            let trimmed = rtrim(&text, '#');
            if trimmed.is_empty() || trimmed.ends_with(' ') {
                text = trim(trimmed).to_string();
            }
        }
        let mut t = Token::with_text(Kind::Heading, rtrim(g(&c, 0), '\n'), &text);
        t.depth = g(&c, 1).len();
        t.inline_id = self.inline(&text);
        Some(t)
    }

    fn hr(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_HR, src)?;
        Some(Token::new(Kind::Hr, rtrim(g(&c, 0), '\n')))
    }
}

fn indent_code_compensation(raw: &str, text: &str) -> String {
    let Some(c) = exec(&R_INDENT_CODE_COMP, raw) else {
        return text.to_string();
    };
    let indent = g(&c, 1).encode_utf16().count();
    text.split('\n')
        .map(|node| match exec(&R_BEGINNING_SPACE, node) {
            Some(m) if g(&m, 0).encode_utf16().count() >= indent => {
                slice_from(node, indent).to_string()
            }
            _ => node.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Lexer {
    fn blockquote(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_BLOCKQUOTE, src)?;
        let whole = rtrim(g(&c, 0), '\n').to_string();
        let mut lines: Vec<String> = whole.split('\n').map(str::to_string).collect();
        let mut raw = String::new();
        let mut text = String::new();
        let mut tokens: Vec<Token> = Vec::new();
        while !lines.is_empty() {
            let mut in_bq = false;
            let mut current = Vec::new();
            let mut i = 0;
            while i < lines.len() {
                if test(&R_BQ_START, &lines[i]) {
                    current.push(lines[i].clone());
                    in_bq = true;
                } else if !in_bq {
                    current.push(lines[i].clone());
                } else {
                    break;
                }
                i += 1;
            }
            lines.drain(..i);
            let current_raw = current.join("\n");
            let current_text =
                replace_all(&R_BQ_SETEXT, &current_raw, |c| format!("\n    {}", g(c, 1)));
            let current_text = replace_all(&R_BQ_SETEXT2, &current_text, |_| String::new());
            raw = if raw.is_empty() {
                current_raw.clone()
            } else {
                format!("{raw}\n{current_raw}")
            };
            text = if text.is_empty() {
                current_text.clone()
            } else {
                format!("{text}\n{current_text}")
            };
            let top = self.top;
            self.top = true;
            self.block_tokens(&current_text, &mut tokens, true);
            self.top = top;
            if lines.is_empty() {
                break;
            }
            match tokens.last().map(|t| t.kind) {
                Some(Kind::Code) => break,
                Some(Kind::Blockquote) => {
                    let old = tokens.pop().unwrap();
                    let continuation = lines.join("\n");
                    let new_text = format!(
                        "{}\n{}",
                        old.raw,
                        replace_all(&R_BQ_SETEXT2, &continuation, |_| String::new())
                    );
                    let new_tok = self.blockquote(&new_text).unwrap();
                    raw = format!("{raw}\n{continuation}");
                    text = format!("{}{}", &text[..text.len() - old.text.len()], new_tok.text);
                    tokens.push(new_tok);
                    break;
                }
                Some(Kind::List) => {
                    let old = tokens.pop().unwrap();
                    let new_text = format!("{}\n{}", old.raw, lines.join("\n"));
                    let new_tok = self.list(&new_text).unwrap();
                    raw = format!("{}{}", &raw[..raw.len() - old.raw.len()], new_tok.raw);
                    text = format!("{}{}", &text[..text.len() - old.raw.len()], new_tok.raw);
                    lines = new_text[new_tok.raw.len()..]
                        .split('\n')
                        .map(str::to_string)
                        .collect();
                    tokens.push(new_tok);
                    continue;
                }
                _ => {}
            }
        }
        let mut t = Token::with_text(Kind::Blockquote, &raw, &text);
        t.tokens = tokens;
        Some(t)
    }

    fn list(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_LIST, src)?;
        let bull0 = trim(g(&c, 1)).to_string();
        let ordered = bull0.chars().count() > 1;
        let mut list = Token::new(Kind::List, "");
        list.raw.clear();
        list.ordered = ordered;
        if ordered {
            list.start = bull0[..bull0.len() - 1].parse().ok();
        }
        let bull = if ordered {
            format!(r"[0-9]{{1,9}}\{}", &bull0[bull0.len() - 1..])
        } else {
            format!(r"\{bull0}")
        };
        let item_re = re(&format!(r"^( {{0,3}}{bull})((?:[\t ][^\n]*)?(?:\n|$))"));
        let mut ends_with_blank = false;
        let mut src = src.to_string();
        while !src.is_empty() {
            let mut end_early = false;
            let Some(cap) = exec(&item_re, &src) else {
                break;
            };
            if test(&R_HR, &src) {
                break;
            }
            let mut raw = g(&cap, 0).to_string();
            let cap1 = g(&cap, 1).to_string();
            let mut line = expand_tabs(first_line(g(&cap, 2)), cap1.len());
            src = src[raw.len()..].to_string();
            let mut next_line = first_line(&src).to_string();
            let mut blank_line = trim(&line).is_empty();
            let mut item_contents = String::new();
            let mut indent;
            if blank_line {
                indent = cap1.len() + 1;
            } else {
                indent = search_non_space(&line).unwrap_or(usize::MAX);
                indent = if indent > 4 { 1 } else { indent };
                item_contents = line[indent..].to_string();
                indent += cap1.len();
            }
            if blank_line && test(&R_BLANK_LINE, &next_line) {
                raw.push_str(&next_line);
                raw.push('\n');
                src = src.get(next_line.len() + 1..).unwrap_or("").to_string();
                end_early = true;
            }
            if !end_early {
                let next_bullet = indent_regex("bullet", indent);
                let hr_re = indent_regex("hr", indent);
                let fences_re = indent_regex("fences", indent);
                let heading_re = indent_regex("heading", indent);
                let html_re = indent_regex("html", indent);
                let bq_re = indent_regex("blockquote", indent);
                while !src.is_empty() {
                    let raw_line = first_line(&src).to_string();
                    next_line = raw_line.clone();
                    let no_tabs = next_line.replace('\t', "    ");
                    if test(&fences_re, &next_line)
                        || test(&heading_re, &next_line)
                        || test(&html_re, &next_line)
                        || test(&bq_re, &next_line)
                        || test(&next_bullet, &next_line)
                        || test(&hr_re, &next_line)
                    {
                        break;
                    }
                    if search_non_space(&no_tabs).is_some_and(|i| i >= indent)
                        || trim(&next_line).is_empty()
                    {
                        item_contents.push('\n');
                        item_contents.push_str(no_tabs.get(indent..).unwrap_or(""));
                    } else {
                        if blank_line {
                            break;
                        }
                        if search_non_space(&line.replace('\t', "    ")).is_some_and(|i| i >= 4) {
                            break;
                        }
                        if test(&fences_re, &line)
                            || test(&heading_re, &line)
                            || test(&hr_re, &line)
                        {
                            break;
                        }
                        item_contents.push('\n');
                        item_contents.push_str(&next_line);
                    }
                    blank_line = trim(&next_line).is_empty();
                    raw.push_str(&raw_line);
                    raw.push('\n');
                    src = src.get(raw_line.len() + 1..).unwrap_or("").to_string();
                    line = no_tabs.get(indent..).unwrap_or("").to_string();
                }
            }
            if !list.loose {
                if ends_with_blank {
                    list.loose = true;
                } else if test(&R_DOUBLE_BLANK, &raw) {
                    ends_with_blank = true;
                }
            }
            let mut item = Token::with_text(Kind::ListItem, &raw, &item_contents);
            item.task = test(&R_LIST_IS_TASK, &item_contents);
            list.raw.push_str(&raw);
            list.items.push(item);
        }
        let last = list.items.last_mut()?;
        last.raw = trim_end(&last.raw).to_string();
        last.text = trim_end(&last.text).to_string();
        list.raw = trim_end(&list.raw).to_string();
        for i in 0..list.items.len() {
            self.top = false;
            let text = list.items[i].text.clone();
            let mut toks = Vec::new();
            self.block_tokens(&text, &mut toks, false);
            if !list.loose {
                list.loose = toks
                    .iter()
                    .any(|t| t.kind == Kind::Space && test(&R_ANY_LINE, &t.raw));
            }
            list.items[i].tokens = toks;
        }
        let loose = list.loose;
        for item in &mut list.items {
            let first_kind = item.tokens.first().map(|t| t.kind);
            if item.task && matches!(first_kind, Some(Kind::Text | Kind::Paragraph)) {
                item.text = replace_first(&R_LIST_REPLACE_TASK, &item.text, "");
                let t0 = &mut item.tokens[0];
                t0.raw = replace_first(&R_LIST_REPLACE_TASK, &t0.raw, "");
                t0.text = replace_first(&R_LIST_REPLACE_TASK, &t0.text, "");
                for q in self.queue.iter_mut().rev() {
                    if test(&R_LIST_IS_TASK, q) {
                        *q = replace_first(&R_LIST_REPLACE_TASK, q, "");
                        break;
                    }
                }
                if let Some(cb) = exec(&R_LIST_TASK_CHECKBOX, &item.raw) {
                    let cbraw = g(&cb, 0);
                    let mut check = Token::new(Kind::Checkbox, &format!("{cbraw} "));
                    check.checked = Some(cbraw != "[ ]");
                    item.checked = check.checked;
                    if loose {
                        // item.tokens[0] is a paragraph or text with inline tokens pending
                        let t0 = &mut item.tokens[0];
                        t0.raw = format!("{}{}", check.raw, t0.raw);
                        t0.text = format!("{}{}", check.raw, t0.text);
                        t0.tokens.insert(0, check);
                    } else {
                        item.tokens.insert(0, check);
                    }
                }
            } else if item.task {
                item.task = false;
            }
        }
        if loose {
            for item in &mut list.items {
                item.loose = true;
                for t in &mut item.tokens {
                    if t.kind == Kind::Text {
                        t.kind = Kind::Paragraph;
                    }
                }
            }
        }
        Some(list)
    }
}

impl Lexer {
    fn html(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_HTML, src)?;
        let raw = trim_trailing_blank_lines(g(&c, 0));
        Some(Token::with_text(Kind::Html, &raw, &raw))
    }

    fn def(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_DEF, src)?;
        let mut t = Token::new(Kind::Def, rtrim(g(&c, 0), '\n'));
        t.tag = R_MULTI_SPACE
            .replace_all(&g(&c, 1).to_lowercase(), " ")
            .into_owned();
        let href = g(&c, 2);
        t.href = if href.is_empty() {
            String::new()
        } else {
            let h = if href.starts_with('<') && href.ends_with('>') && href.len() >= 2 {
                &href[1..href.len() - 1]
            } else {
                href
            };
            unescape_punct(h)
        };
        t.title = group(&c, 3).map(|s| {
            if s.is_empty() {
                String::new()
            } else {
                unescape_punct(&s[1..s.len() - 1])
            }
        });
        Some(t)
    }

    fn table(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_TABLE, src)?;
        let align_src = g(&c, 2);
        if !align_src.contains([':', '|']) {
            return None;
        }
        let headers = split_cells(g(&c, 1), None);
        let aligns_s = replace_all(&R_TABLE_ALIGN_CHARS, align_src, |_| String::new());
        let aligns: Vec<&str> = aligns_s.split('|').collect();
        let rows_src = g(&c, 3);
        let rows: Vec<String> = if !trim(rows_src).is_empty() {
            replace_first(&R_TABLE_ROW_BLANK, rows_src, "")
                .split('\n')
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        };
        if headers.len() != aligns.len() {
            return None;
        }
        let mut t = Token::new(Kind::Table, rtrim(g(&c, 0), '\n'));
        for h in &headers {
            let id = self.inline(h);
            t.header.push(Cell {
                text: h.clone(),
                tokens: Vec::new(),
                inline_id: id,
            });
        }
        for row in &rows {
            let cells = split_cells(row, Some(t.header.len()));
            let mut r = Vec::new();
            for cell in cells {
                let id = self.inline(&cell);
                r.push(Cell {
                    text: cell,
                    tokens: Vec::new(),
                    inline_id: id,
                });
            }
            t.rows.push(r);
        }
        let _ = (&R_ALIGN_RIGHT, &R_ALIGN_CENTER, &R_ALIGN_LEFT); // alignment is not rendered upstream
        Some(t)
    }

    fn lheading(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_LHEADING, src)?;
        let text = trim(g(&c, 1)).to_string();
        let mut t = Token::with_text(Kind::Heading, rtrim(g(&c, 0), '\n'), &text);
        t.depth = if g(&c, 2).starts_with('=') { 1 } else { 2 };
        t.inline_id = self.inline(&text);
        Some(t)
    }

    fn paragraph(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_PARAGRAPH, src)?;
        let c1 = g(&c, 1);
        let text = c1.strip_suffix('\n').unwrap_or(c1).to_string();
        let mut t = Token::with_text(Kind::Paragraph, g(&c, 0), &text);
        t.inline_id = self.inline(&text);
        Some(t)
    }

    fn text(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_TEXT, src)?;
        let raw = g(&c, 0);
        let mut t = Token::with_text(Kind::Text, raw, raw);
        t.inline_id = self.inline(raw);
        Some(t)
    }
}

/// upstream `containsTableStart`.
fn contains_table_start(source: &str) -> bool {
    let lines: Vec<&str> = source.split('\n').collect();
    (0..lines.len().saturating_sub(1)).any(|i| {
        let delim = lines[i + 1];
        (delim.contains('|') || lines[i].contains('|')) && test(&R_TABLE_DELIM_ROW, delim)
    })
}

fn looks_like_pending_dollar_math(source: &str) -> bool {
    test(&R_PENDING_MATH, source)
}

/// upstream `tokenizeBlockLatex`.
fn tokenize_block_latex(source: &str) -> Option<Token> {
    let block = |raw: &str, text: &str, pending: bool| {
        let mut t = Token::with_text(Kind::LatexBlock, raw, text);
        t.pending = pending;
        t
    };
    for r in [&*R_LATEX_DOLLAR, &*R_LATEX_BRACKET] {
        if let Some(c) = exec(r, source) {
            let body = g(&c, 1);
            if !body.is_empty() && !contains_table_start(body) {
                return Some(block(g(&c, 0), trim(body), false));
            }
        }
    }
    if let Some(c) = exec(&R_LATEX_PENDING_BRACKET, source) {
        let body = g(&c, 1);
        if !body.is_empty() && !contains_table_start(body) {
            return Some(block(g(&c, 0), body, true));
        }
    }
    if let Some(c) = exec(&R_LATEX_PENDING_DOLLAR, source) {
        let body = g(&c, 1);
        if !body.is_empty() && looks_like_pending_dollar_math(body) && !contains_table_start(body) {
            return Some(block(g(&c, 0), body, true));
        }
    }
    None
}

// marked-cjk-friendly 0.1.2: CJK counts as punctuation for flanking.
const CJK: &str = r"\x{1100}-\x{11ff}\x{20a9}\x{2329}-\x{232a}\x{2630}-\x{2637}\x{268a}-\x{268f}\x{2e80}-\x{2e99}\x{2e9b}-\x{2ef3}\x{2f00}-\x{2fd5}\x{2ff0}-\x{303e}\x{3041}-\x{3096}\x{3099}-\x{30ff}\x{3105}-\x{312f}\x{3131}-\x{318e}\x{3190}-\x{31e5}\x{31ef}-\x{321e}\x{3220}-\x{3247}\x{3250}-\x{a48c}\x{a490}-\x{a4c6}\x{a960}-\x{a97c}\x{ac00}-\x{d7a3}\x{d7b0}-\x{d7c6}\x{d7cb}-\x{d7fb}\x{f900}-\x{faff}\x{fe10}-\x{fe19}\x{fe30}-\x{fe52}\x{fe54}-\x{fe66}\x{fe68}-\x{fe6b}\x{ff01}-\x{ffbe}\x{ffc2}-\x{ffc7}\x{ffca}-\x{ffcf}\x{ffd2}-\x{ffd7}\x{ffda}-\x{ffdc}\x{ffe0}-\x{ffe6}\x{ffe8}-\x{ffee}\x{16fe0}-\x{16fe4}\x{16ff0}-\x{16ff6}\x{17000}-\x{18cda}\x{18cff}-\x{18d20}\x{18d80}-\x{18df2}\x{18e00}-\x{19191}\x{191a0}-\x{191d2}\x{1aff0}-\x{1aff3}\x{1aff5}-\x{1affb}\x{1affd}-\x{1affe}\x{1b000}-\x{1b128}\x{1b132}\x{1b150}-\x{1b152}\x{1b155}\x{1b164}-\x{1b168}\x{1b170}-\x{1b2fb}\x{1d300}-\x{1d356}\x{1d360}-\x{1d376}\x{1f1ae}\x{1f200}\x{1f202}\x{1f210}-\x{1f219}\x{1f21b}-\x{1f22e}\x{1f230}-\x{1f231}\x{1f237}\x{1f23b}\x{1f240}-\x{1f248}\x{1f260}-\x{1f265}\x{1f7da}\x{20000}-\x{3fffd}";
rule!(R_CJK, format!("[{CJK}]"));
rule!(
    R_PUNCT_CJK,
    format!(r"^((?![*_])[{WS}\p{{P}}\p{{S}}{CJK}])")
);
static R_EM_R_AST: LazyLock<Regex> = LazyLock::new(|| {
    let p = format!(r"(?!~)[\p{{P}}\p{{S}}{CJK}]");
    let ps = format!(r"(?!~)[{WS}\p{{P}}\p{{S}}{CJK}]");
    let nps = format!(r"(?:[^{WS}\p{{P}}\p{{S}}{CJK}]|~)");
    re(&format!(
        r"^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)|[^*]+(?=[^*])|(?!\*){p}(\*+)(?=[{WS}]|$)|{nps}(\*+)(?!\*)(?={ps}|$)|(?!\*){ps}(\*+)(?={nps})|[{WS}](\*+)(?!\*)(?={p})|(?!\*){p}(\*+)(?!\*)(?={p})|{nps}(\*+)(?={nps})"
    ))
});
static R_EM_R_UND: LazyLock<Regex> = LazyLock::new(|| {
    let p = format!(r"[\p{{P}}\p{{S}}{CJK}]");
    let ps = format!(r"[{WS}\p{{P}}\p{{S}}{CJK}]");
    let nps = format!(r"[^{WS}\p{{P}}\p{{S}}{CJK}]");
    re(&format!(
        r"^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)|[^_]+(?=[^_])|(?!_){p}(_+)(?=[{WS}]|$)|{nps}(_+)(?!_)(?={ps}|$)|(?!_){ps}(_+)(?={nps})|[{WS}](_+)(?!_)(?={p})|(?!_){p}(_+)(?!_)(?={p})"
    ))
});

/// marked's `prevChar`: the last UTF-16 unit of the previous text token. An astral character
/// leaves a lone low surrogate there, which no regex class matches.
#[derive(Clone, Copy)]
enum Prev {
    Empty,
    Char(char),
    LoneSurrogate,
}

impl Prev {
    fn of(raw: &str) -> Prev {
        match raw.chars().next_back() {
            None => Prev::Empty,
            Some(c) if c.len_utf16() == 2 => Prev::LoneSurrogate,
            Some(c) => Prev::Char(c),
        }
    }
    fn matches(self, r: &Regex) -> bool {
        match self {
            Prev::Char(c) => test(r, c.encode_utf8(&mut [0; 4])),
            _ => false,
        }
    }
}

fn is_cjk(c: char) -> bool {
    test(&R_CJK, c.encode_utf8(&mut [0; 4]))
}

/// marked-cjk-friendly `isPrevCharCjk`; `pos` is where `src` starts in `masked`.
fn prev_is_cjk(prev: Prev, masked: &str, pos: usize) -> bool {
    if prev.matches(&R_PUNCT_CJK) {
        return true;
    }
    if matches!(prev, Prev::Empty) || pos == 0 {
        return false;
    }
    let mut before = masked[..pos].chars().rev();
    let Some(mut c) = before.next() else {
        return false;
    };
    if ('\u{FE00}'..='\u{FE0E}').contains(&c) {
        match before.next() {
            Some(b) => c = b,
            None => return is_cjk(c),
        }
    }
    if ('\u{E0100}'..='\u{E01EF}').contains(&c) {
        return true;
    }
    is_cjk(c)
}

fn first_char(s: &str) -> Option<char> {
    s.chars().next()
}

fn is_escaped(s: &str, index: usize) -> bool {
    s.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

fn find_closing_delimiter(s: &str, closing: &str, start: usize) -> Option<usize> {
    let mut from = start;
    loop {
        let i = from + s.get(from..)?.find(closing)?;
        if !is_escaped(s, i) {
            return Some(i);
        }
        from = i + closing.len();
    }
}

rule!(R_TRAILING_WS, format!(r"[{WS}]$"));
rule!(
    R_ENV_LIKE,
    format!(r"^[A-Z_][A-Z0-9_]*(?:[^A-Za-z0-9_{WS}])?$")
);
rule!(R_IDENT_START, r"^[A-Za-z_][A-Za-z0-9_]*");

/// upstream `tokenizeInlineLatex`.
fn tokenize_inline_latex(source: &str) -> Option<Token> {
    let (open, close) = if source.starts_with("$$") {
        ("$$", "$$")
    } else if source.starts_with("\\(") {
        ("\\(", "\\)")
    } else if source.starts_with("\\[") {
        ("\\[", "\\]")
    } else if source.starts_with('$') && !source[1..].starts_with(js_ws) {
        ("$", "$")
    } else {
        return None;
    };
    let closing = find_closing_delimiter(source, close, open.len());
    if let Some(ci) = closing
        && open == "$"
    {
        let inner = &source[1..ci];
        let after = &source[ci + 1..];
        if test(&R_TRAILING_WS, inner)
            || after.starts_with(|c: char| c.is_ascii_digit())
            || (test(&R_ENV_LIKE, inner) && test(&R_IDENT_START, after))
            || inner.contains('`')
        {
            return None;
        }
    }
    let Some(ci) = closing else {
        let pending = &source[open.len()..];
        if open.starts_with('\\') || looks_like_pending_dollar_math(pending) {
            let mut t = Token::with_text(Kind::Latex, source, pending);
            t.pending = true;
            return Some(t);
        }
        return None;
    };
    let text = &source[open.len()..ci];
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    Some(Token::with_text(
        Kind::Latex,
        &source[..ci + close.len()],
        text,
    ))
}

fn nonempty_group(c: &Captures, from: usize, to: usize) -> Option<usize> {
    (from..=to).find(|&i| !g(c, i).is_empty())
}

/// marked-cjk-friendly `isCjkAdjacentToDelimiter`.
fn cjk_adjacent(rm: &Captures, clipped: &str) -> bool {
    if nonempty_group(rm, 1, 4).is_none() {
        return false;
    }
    let m = rm.get(0).unwrap();
    first_char(m.as_str()).is_some_and(is_cjk)
        || first_char(&clipped[m.end()..]).is_some_and(is_cjk)
}

impl Lexer {
    fn link_in_text(&self, text: &str) -> bool {
        if !text.contains('[') {
            return false;
        }
        for m in R_BLOCK_SKIP.find_iter(text).flatten() {
            if test(&R_LINK, m.as_str()) && !text[..m.start()].ends_with('!') {
                return true;
            }
        }
        for c in R_REFLINK_SEARCH.captures_iter(text).flatten() {
            let m0 = g(&c, 0);
            let ref_start = m0.rfind('[').unwrap_or(0);
            if m0.starts_with('!') || !self.links.contains_key(&m0[ref_start + 1..m0.len() - 1]) {
                continue;
            }
            if ref_start > 1 && self.link_in_text(&m0[1..ref_start - 1]) {
                continue;
            }
            return true;
        }
        false
    }

    fn mask_reflinks(&self, s: &str) -> String {
        replace_all(&R_REFLINK_SEARCH, s, |c| {
            let m0 = g(c, 0);
            let ref_start = m0.rfind('[').unwrap_or(0);
            if !self.links.contains_key(&m0[ref_start + 1..m0.len() - 1]) {
                return m0.to_string();
            }
            if ref_start > 1 && !m0.starts_with('!') {
                let text = &m0[1..ref_start - 1];
                if self.link_in_text(text) {
                    return format!(
                        "[{}][{}]",
                        self.mask_reflinks(text),
                        "a".repeat(m0.len() - ref_start - 2)
                    );
                }
            }
            format!("[{}]", "a".repeat(m0.len() - 2))
        })
    }

    /// The masked source keeps every byte offset of `src` (marked keeps UTF-16 offsets).
    fn mask(&self, src: &str) -> String {
        let mut masked = src.to_string();
        if src.contains('[') {
            masked = self.mask_reflinks(&masked);
        }
        masked = replace_all(&R_ANY_PUNCT, &masked, |c| "+".repeat(g(c, 0).len()));
        replace_all(&R_BLOCK_SKIP, &masked, |c| {
            format!("[{}]", "a".repeat(g(c, 0).len() - 2))
        })
    }

    pub(crate) fn inline_tokens(&mut self, full: &str, tokens: &mut Vec<Token>) {
        let masked = self.mask(full);
        let mut src = full;
        let mut keep_prev = false;
        let mut prev = Prev::Empty;
        while !src.is_empty() {
            if !keep_prev {
                prev = Prev::Empty;
            }
            keep_prev = false;
            let tok = tokenize_inline_latex(src)
                .or_else(|| self.escape(src))
                .or_else(|| self.tag(src))
                .or_else(|| self.link(src));
            if let Some(t) = tok {
                src = &src[t.raw.len()..];
                tokens.push(t);
                continue;
            }
            if let Some(t) = self.reflink(src) {
                src = &src[t.raw.len()..];
                match tokens.last_mut() {
                    Some(last) if t.kind == Kind::Text && last.kind == Kind::Text => {
                        last.raw.push_str(&t.raw);
                        last.text.push_str(&t.text);
                    }
                    _ => tokens.push(t),
                }
                continue;
            }
            let pos = full.len() - src.len();
            let tok = self
                .em_strong(src, &masked, pos, prev)
                .or_else(|| codespan(src))
                .or_else(|| exec(&R_BR, src).map(|c| Token::new(Kind::Br, g(&c, 0))))
                .or_else(|| self.del(src))
                .or_else(|| autolink(src))
                .or_else(|| if self.in_link { None } else { url(src) });
            if let Some(t) = tok {
                src = &src[t.raw.len()..];
                tokens.push(t);
                continue;
            }
            // startInline of the latex extension, on src.slice(1)
            let first_len = first_char(src).map_or(0, char::len_utf8);
            let rest = &src[first_len..];
            let cut_at = ["$", "\\(", "\\["]
                .iter()
                .filter_map(|p| rest.find(p))
                .min();
            let cut = match cut_at {
                Some(i) => &src[..first_len + i],
                None => src,
            };
            let Some(c) = exec(&R_INLINE_TEXT, cut) else {
                break;
            };
            let raw = g(&c, 0);
            src = &src[raw.len()..];
            if !raw.ends_with('_') {
                prev = Prev::of(raw);
            }
            keep_prev = true;
            match tokens.last_mut() {
                Some(last) if last.kind == Kind::Text => {
                    last.raw.push_str(raw);
                    last.text.push_str(raw);
                }
                _ => tokens.push(Token::with_text(Kind::Text, raw, raw)),
            }
        }
    }

    fn escape(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_ESCAPE, src)?;
        Some(Token::with_text(Kind::Escape, g(&c, 0), g(&c, 1)))
    }

    fn tag(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_TAG, src)?;
        let raw = g(&c, 0);
        if !self.in_link && test(&R_START_A, raw) {
            self.in_link = true;
        } else if self.in_link && test(&R_END_A, raw) {
            self.in_link = false;
        }
        if !self.in_raw_block && test(&R_START_PRE, raw) {
            self.in_raw_block = true;
        } else if self.in_raw_block && test(&R_END_PRE, raw) {
            self.in_raw_block = false;
        }
        Some(Token::with_text(Kind::Html, raw, raw))
    }

    fn link(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_LINK, src)?;
        let mut cap0 = g(&c, 0).to_string();
        let cap1 = g(&c, 1);
        let mut cap2 = g(&c, 2).to_string();
        let mut cap3 = g(&c, 3).to_string();
        let trimmed_url = trim(&cap2).to_string();
        if trimmed_url.starts_with('<') {
            if !trimmed_url.ends_with('>') {
                return None;
            }
            let without = &trimmed_url[..trimmed_url.len() - 1];
            if (trimmed_url.len() - without.trim_end_matches('\\').len()).is_multiple_of(2) {
                return None;
            }
        } else {
            let last_paren = find_closing_paren(&cap2);
            if last_paren == -2 {
                return None;
            }
            if last_paren > -1 {
                let start = if cap0.starts_with('!') { 5 } else { 4 };
                let link_len = start + cap1.len() + last_paren as usize;
                cap2.truncate(last_paren as usize);
                cap0 = trim(cap0.get(..link_len).unwrap_or(&cap0)).to_string();
                cap3.clear();
            }
        }
        let title = if cap3.len() >= 2 {
            cap3[1..cap3.len() - 1].to_string()
        } else {
            String::new()
        };
        let mut href = trim(&cap2).to_string();
        if href.starts_with('<') {
            href = if href.len() >= 2 {
                href[1..href.len() - 1].to_string()
            } else {
                String::new()
            };
        }
        let href = unescape_punct(&href);
        let title = unescape_punct(&title);
        self.output_link(cap1, &cap0, href, (!title.is_empty()).then_some(title))
    }

    fn output_link(
        &mut self,
        cap1: &str,
        raw: &str,
        href: String,
        title: Option<String>,
    ) -> Option<Token> {
        let text = cap1.replace("\\[", "[").replace("\\]", "]");
        let is_image = raw.starts_with('!');
        self.in_link = true;
        let outer_emitted = self.link_emitted;
        let outer_raw_block = self.in_raw_block;
        self.link_emitted = false;
        let mut tokens = Vec::new();
        self.inline_tokens(&text, &mut tokens);
        let text_has_link = self.link_emitted;
        self.link_emitted = outer_emitted;
        self.in_link = false;
        if !is_image {
            if text_has_link {
                self.in_raw_block = outer_raw_block;
                return None;
            }
            self.link_emitted = true;
        }
        let mut t = Token::with_text(if is_image { Kind::Image } else { Kind::Link }, raw, &text);
        t.href = href;
        t.title = title;
        t.tokens = tokens;
        Some(t)
    }

    fn reflink(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_REFLINK, src).or_else(|| exec(&R_NOLINK, src))?;
        let label = if g(&c, 2).is_empty() {
            g(&c, 1)
        } else {
            g(&c, 2)
        };
        let key = R_MULTI_SPACE.replace_all(label, " ").to_lowercase();
        let Some((href, title)) = self.links.get(&key).cloned() else {
            let first = &g(&c, 0)[..1];
            return Some(Token::with_text(Kind::Text, first, first));
        };
        let raw = g(&c, 0).to_string();
        self.output_link(g(&c, 1), &raw, href, title.filter(|t| !t.is_empty()))
    }

    /// marked-cjk-friendly `emStrong`; `pos` is where `src` starts in `masked`.
    fn em_strong(&mut self, src: &str, masked: &str, pos: usize, prev: Prev) -> Option<Token> {
        let m = exec(&R_EM_L, src)?;
        nonempty_group(&m, 1, 4)?;
        if !g(&m, 4).is_empty() && prev.matches(&R_UNICODE_ALNUM) {
            return None;
        }
        let next_char = if g(&m, 1).is_empty() {
            g(&m, 3)
        } else {
            g(&m, 1)
        };
        let open_ok = next_char.is_empty()
            || matches!(prev, Prev::Empty)
            || prev.matches(&R_PUNCT)
            || prev_is_cjk(prev, masked, pos)
            || first_char(next_char).is_some_and(is_cjk);
        if !open_ok {
            return None;
        }
        let m0 = g(&m, 0);
        let l_length = (m0.len() - m0.chars().next_back().map_or(0, char::len_utf8)) as isize;
        let delim = m0.chars().next().unwrap();
        let prev_same = matches!(prev, Prev::Char(c) if c == delim);
        let end_re: &Regex = if delim == '*' {
            &R_EM_R_AST
        } else {
            &R_EM_R_UND
        };
        let clipped = &masked[pos + l_length as usize..];
        let mut delim_total = l_length;
        let mut mid_total = 0isize;
        for rm in end_re.captures_iter(clipped).flatten() {
            let Some(gi) = nonempty_group(&rm, 1, 6) else {
                continue;
            };
            let mut r_length = g(&rm, gi).len() as isize;
            let adjacent = cjk_adjacent(&rm, clipped);
            let left_only = gi == 3 || gi == 4;
            let both = gi == 5 || gi == 6;
            if left_only && !adjacent {
                delim_total += r_length;
                continue;
            }
            if both || adjacent {
                if l_length % 3 != 0 && (l_length + r_length) % 3 == 0 {
                    mid_total += r_length;
                    continue;
                }
                if prev_same && both {
                    break;
                }
            }
            delim_total -= r_length;
            if delim_total > 0 {
                continue;
            }
            r_length = r_length.min(r_length + delim_total + mid_total);
            let m = rm.get(0).unwrap();
            let last_char_len = first_char(m.as_str()).map_or(1, char::len_utf8) as isize;
            let end = (l_length + m.start() as isize + last_char_len + r_length).max(0) as usize;
            let raw = &src[..end.min(src.len())];
            let (kind, text) = if l_length.min(r_length) % 2 != 0 {
                (
                    Kind::Em,
                    raw.get(1..raw.len().saturating_sub(1)).unwrap_or(""),
                )
            } else {
                (
                    Kind::Strong,
                    raw.get(2..raw.len().saturating_sub(2)).unwrap_or(""),
                )
            };
            let mut t = Token::with_text(kind, raw, text);
            let mut inner = Vec::new();
            self.inline_tokens(text, &mut inner);
            t.tokens = inner;
            return Some(t);
        }
        None
    }

    /// upstream `StrictStrikethroughTokenizer.del`.
    fn del(&mut self, src: &str) -> Option<Token> {
        let c = exec(&R_STRICT_DEL, src)?;
        let text = g(&c, 2);
        let mut t = Token::with_text(Kind::Del, g(&c, 0), text);
        let mut inner = Vec::new();
        self.inline_tokens(text, &mut inner);
        t.tokens = inner;
        Some(t)
    }
}

fn codespan(src: &str) -> Option<Token> {
    let c = exec(&R_CODESPAN, src)?;
    let mut text = g(&c, 2).replace('\n', " ");
    if text.contains(|c| c != ' ') && text.starts_with(' ') && text.ends_with(' ') {
        text = text[1..text.len() - 1].to_string();
    }
    Some(Token::with_text(Kind::Codespan, g(&c, 0), &text))
}

fn link_token(raw: &str, text: &str, href: String) -> Token {
    let mut t = Token::with_text(Kind::Link, raw, text);
    t.href = href;
    t.tokens = vec![Token::with_text(Kind::Text, text, text)];
    t
}

fn autolink(src: &str) -> Option<Token> {
    let c = exec(&R_AUTOLINK, src)?;
    let text = g(&c, 1);
    let href = if g(&c, 2) == "@" {
        format!("mailto:{text}")
    } else {
        text.to_string()
    };
    Some(link_token(g(&c, 0), text, href))
}

fn url(src: &str) -> Option<Token> {
    let c = exec(&R_URL, src)?;
    if g(&c, 2) == "@" {
        let text = g(&c, 0);
        return Some(link_token(text, text, format!("mailto:{text}")));
    }
    let mut cap0 = g(&c, 0).to_string();
    loop {
        let next = R_BACKPEDAL
            .find(&cap0)
            .ok()
            .flatten()
            .map_or(String::new(), |m| m.as_str().to_string());
        if next == cap0 {
            break;
        }
        cap0 = next;
    }
    let href = if g(&c, 1) == "www." {
        format!("http://{cap0}")
    } else {
        cap0.clone()
    };
    Some(link_token(&cap0, &cap0, href))
}
