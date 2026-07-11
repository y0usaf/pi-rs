//! Markdown-to-terminal rendering mechanism (pi `components/markdown.ts`).
//!
//! The render pipeline (token rendering, style contexts, list/table/quote
//! layout, wrapping, margins, background) is a 1:1 port.  Tokenization is a
//! Rust implementation of the `marked` constructs that pipeline consumes:
//! space, heading (ATX + setext), fenced code, hr, blockquote, list, table,
//! html, paragraph; inline text, strong, em, codespan, link, del, br, html.

use crate::utils::{apply_background_to_line, visible_width, wrap_text_with_ansi};

pub type StyleFn<'a> = Box<dyn Fn(&str) -> String + 'a>;
pub type HighlightFn<'a> = Box<dyn Fn(&str, Option<&str>) -> Vec<String> + 'a>;

/// Theme functions for markdown elements (pi `MarkdownTheme`).
pub struct MarkdownTheme<'a> {
    pub heading: StyleFn<'a>,
    pub link: StyleFn<'a>,
    pub link_url: StyleFn<'a>,
    pub code: StyleFn<'a>,
    pub code_block: StyleFn<'a>,
    pub code_block_border: StyleFn<'a>,
    pub quote: StyleFn<'a>,
    pub quote_border: StyleFn<'a>,
    pub hr: StyleFn<'a>,
    pub list_bullet: StyleFn<'a>,
    pub bold: StyleFn<'a>,
    pub italic: StyleFn<'a>,
    pub strikethrough: StyleFn<'a>,
    pub underline: StyleFn<'a>,
    pub highlight_code: Option<HighlightFn<'a>>,
    /// Prefix applied to each rendered code block line (default: `"  "`).
    pub code_block_indent: Option<String>,
}

fn identity() -> StyleFn<'static> {
    Box::new(|text: &str| text.to_owned())
}

impl<'a> MarkdownTheme<'a> {
    /// Theme with no styling; used by the plain rendering seam.
    pub fn plain() -> MarkdownTheme<'static> {
        MarkdownTheme {
            heading: identity(),
            link: identity(),
            link_url: identity(),
            code: identity(),
            code_block: identity(),
            code_block_border: identity(),
            quote: identity(),
            quote_border: identity(),
            hr: identity(),
            list_bullet: identity(),
            bold: identity(),
            italic: identity(),
            strikethrough: identity(),
            underline: identity(),
            highlight_code: None,
            code_block_indent: None,
        }
    }
}

/// Default text styling applied to all text unless overridden
/// (pi `DefaultTextStyle`).
#[derive(Default)]
pub struct DefaultTextStyle<'a> {
    pub color: Option<StyleFn<'a>>,
    pub bg_color: Option<StyleFn<'a>>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Default)]
pub struct MarkdownOptions {
    /// Preserve source ordered-list markers instead of normalizing them.
    pub preserve_ordered_list_markers: bool,
}

// ============================================================================
// Tokens
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Codespan(String),
    Link {
        href: String,
        text: String,
        tokens: Vec<Inline>,
    },
    Del(Vec<Inline>),
    Br,
    Html(String),
}

#[derive(Debug, Clone, PartialEq)]
struct ListItem {
    task: bool,
    checked: bool,
    raw_marker: Option<String>,
    tokens: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
struct ListToken {
    ordered: bool,
    start: i64,
    items: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Space,
    Heading {
        depth: usize,
        tokens: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    /// Tight list-item content (marked `text` block token).
    Text(Vec<Inline>),
    Code {
        lang: Option<String>,
        text: String,
    },
    List(ListToken),
    Blockquote(Vec<Block>),
    Hr,
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        raw: String,
    },
    Html(String),
}

impl Block {
    fn kind(&self) -> &'static str {
        match self {
            Block::Space => "space",
            Block::Heading { .. } => "heading",
            Block::Paragraph(_) => "paragraph",
            Block::Text(_) => "text",
            Block::Code { .. } => "code",
            Block::List(_) => "list",
            Block::Blockquote(_) => "blockquote",
            Block::Hr => "hr",
            Block::Table { .. } => "table",
            Block::Html(_) => "html",
        }
    }
}

// ============================================================================
// Block lexer
// ============================================================================

fn indent_width(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

struct Fence<'a> {
    marker: char,
    length: usize,
    indent: usize,
    lang: Option<&'a str>,
}

fn fence_open(line: &str) -> Option<Fence<'_>> {
    let indent = indent_width(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = rest.chars().take_while(|c| *c == marker).count();
    if length < 3 {
        return None;
    }
    let info = rest[length..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next();
    Some(Fence {
        marker,
        length,
        indent,
        lang,
    })
}

fn fence_close(line: &str, open: &Fence) -> bool {
    let indent = indent_width(line);
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let length = rest.chars().take_while(|c| *c == open.marker).count();
    length >= open.length && rest[length..].trim().is_empty()
}

fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let indent = indent_width(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let depth = rest.chars().take_while(|c| *c == '#').count();
    if depth == 0 || depth > 6 {
        return None;
    }
    let after = &rest[depth..];
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let mut text = after.trim();
    // Strip a closing hash run.
    if let Some(stripped) = text
        .trim_end_matches('#')
        .strip_suffix([' ', '\t'])
        .map(str::trim_end)
        && text.ends_with('#')
    {
        text = stripped;
    } else if text.chars().all(|c| c == '#') {
        text = "";
    }
    Some((depth, text))
}

fn is_hr(line: &str) -> bool {
    let indent = indent_width(line);
    if indent > 3 {
        return false;
    }
    let rest = line[indent..].replace([' ', '\t'], "");
    if rest.len() < 3 {
        return false;
    }
    let first = match rest.chars().next() {
        Some(c @ ('-' | '_' | '*')) => c,
        _ => return false,
    };
    rest.chars().all(|c| c == first)
}

fn is_blockquote_start(line: &str) -> bool {
    let indent = indent_width(line);
    indent <= 3 && line[indent..].starts_with('>')
}

struct ListMarker<'a> {
    ordered: bool,
    number: i64,
    marker: String,
    content: &'a str,
    content_indent: usize,
}

fn list_marker(line: &str) -> Option<ListMarker<'_>> {
    let indent = indent_width(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let (marker, ordered, number) = if let Some(c) = rest.chars().next()
        && matches!(c, '*' | '+' | '-')
    {
        (c.to_string(), false, 0)
    } else {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || digits.len() > 9 {
            return None;
        }
        let delim = rest[digits.len()..].chars().next()?;
        if delim != '.' && delim != ')' {
            return None;
        }
        let number = digits.parse().ok()?;
        (format!("{digits}{delim}"), true, number)
    };
    let after = &rest[marker.len()..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let spaces = after.chars().take_while(|c| *c == ' ').count();
    let content = &after[spaces.min(after.len())..];
    // Content indent: marker plus following spaces (blank content ⇒ marker+1).
    let content_indent = if content.is_empty() {
        indent + marker.len() + 1
    } else {
        indent + marker.len() + spaces
    };
    Some(ListMarker {
        ordered,
        number,
        marker,
        content,
        content_indent,
    })
}

fn table_delimiter_cells(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    if !trimmed.contains('-') {
        return None;
    }
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or_else(|| trimmed.strip_prefix('|').unwrap_or(trimmed));
    let mut count = 0;
    for cell in inner.split('|') {
        let cell = cell.trim();
        if cell.is_empty() {
            return None;
        }
        let body = cell
            .strip_prefix(':')
            .unwrap_or(cell)
            .strip_suffix(':')
            .unwrap_or_else(|| cell.strip_prefix(':').unwrap_or(cell));
        if body.is_empty() || !body.chars().all(|c| c == '-') {
            return None;
        }
        count += 1;
    }
    if count == 0 { None } else { Some(count) }
}

fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            current.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(current.trim().to_owned());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

fn is_table_start(lines: &[&str], index: usize) -> bool {
    lines.get(index).is_some_and(|line| line.contains('|'))
        && lines
            .get(index + 1)
            .and_then(|line| table_delimiter_cells(line))
            .is_some()
}

fn is_html_block_start(line: &str) -> bool {
    let indent = indent_width(line);
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    rest.starts_with('<')
        && rest[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '!' | '/' | '?'))
}

fn interrupts_paragraph(line: &str) -> bool {
    atx_heading(line).is_some()
        || fence_open(line).is_some()
        || is_hr(line)
        || is_blockquote_start(line)
        || list_marker(line).is_some_and(|m| !m.ordered || m.number == 1)
}

fn lex_blocks(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_blank(line) {
            while i < lines.len() && is_blank(lines[i]) {
                i += 1;
            }
            blocks.push(Block::Space);
            continue;
        }
        if let Some(fence) = fence_open(line) {
            let lang = fence.lang.map(str::to_owned);
            let mut body: Vec<String> = Vec::new();
            i += 1;
            while i < lines.len() && !fence_close(lines[i], &fence) {
                let content = lines[i];
                let strip = indent_width(content).min(fence.indent);
                body.push(content[strip..].to_owned());
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            blocks.push(Block::Code {
                lang,
                text: body.join("\n"),
            });
            continue;
        }
        if let Some((depth, text)) = atx_heading(line) {
            blocks.push(Block::Heading {
                depth,
                tokens: lex_inline(text),
            });
            i += 1;
            continue;
        }
        if is_hr(line) {
            blocks.push(Block::Hr);
            i += 1;
            continue;
        }
        if is_blockquote_start(line) {
            let mut inner: Vec<String> = Vec::new();
            while i < lines.len() && is_blockquote_start(lines[i]) {
                let indent = indent_width(lines[i]);
                let rest = &lines[i][indent + 1..];
                inner.push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
                i += 1;
            }
            blocks.push(Block::Blockquote(lex_blocks(&inner.join("\n"))));
            continue;
        }
        if let Some(marker) = list_marker(line) {
            let ordered = marker.ordered;
            blocks.push(lex_list(&lines, &mut i, ordered));
            continue;
        }
        // Table: header row followed by a delimiter row with matching cells.
        if line.contains('|')
            && i + 1 < lines.len()
            && let Some(delimiter_cells) = table_delimiter_cells(lines[i + 1])
        {
            let header_cells = split_table_row(line);
            if header_cells.len() == delimiter_cells {
                let mut raw = format!("{line}\n{}", lines[i + 1]);
                let mut rows = Vec::new();
                i += 2;
                while i < lines.len() && !is_blank(lines[i]) && lines[i].contains('|') {
                    raw.push('\n');
                    raw.push_str(lines[i]);
                    let mut cells = split_table_row(lines[i]);
                    cells.resize(delimiter_cells, String::new());
                    cells.truncate(delimiter_cells);
                    rows.push(cells.iter().map(|c| lex_inline(c)).collect());
                    i += 1;
                }
                blocks.push(Block::Table {
                    header: header_cells.iter().map(|c| lex_inline(c)).collect(),
                    rows,
                    raw,
                });
                continue;
            }
        }
        if is_html_block_start(line) {
            let mut raw = Vec::new();
            while i < lines.len() && !is_blank(lines[i]) {
                raw.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Html(raw.join("\n")));
            continue;
        }
        // Paragraph (with setext heading detection).
        let mut paragraph = vec![line];
        i += 1;
        loop {
            if i >= lines.len() || is_blank(lines[i]) {
                break;
            }
            // Setext underline promotes the accumulated paragraph. Checked
            // before paragraph interrupts: marked's lheading rule consumes
            // `text\n---` as a depth-2 heading before hr gets a chance.
            let candidate = lines[i].trim();
            let is_setext = indent_width(lines[i]) <= 3
                && !candidate.is_empty()
                && (candidate.chars().all(|c| c == '=') || candidate.chars().all(|c| c == '-'));
            if is_setext {
                let depth = if candidate.starts_with('=') { 1 } else { 2 };
                i += 1;
                blocks.push(Block::Heading {
                    depth,
                    tokens: lex_inline(paragraph.join("\n").trim()),
                });
                paragraph.clear();
                break;
            }
            // Marked's table block interrupts a paragraph even without a
            // blank line (`**Navigation**\n| Key |...` in /hotkeys).
            if is_table_start(&lines, i) {
                break;
            }
            if interrupts_paragraph(lines[i]) {
                break;
            }
            paragraph.push(lines[i]);
            i += 1;
        }
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(lex_inline(
                paragraph.join("\n").trim_end(),
            )));
        }
    }
    blocks
}

fn lex_list(lines: &[&str], i: &mut usize, ordered: bool) -> Block {
    let mut items: Vec<(String, Option<String>)> = Vec::new(); // (raw content, marker raw)
    let mut start = 1;
    let mut loose = false;
    let mut first = true;
    let mut pending_blank = false;
    while *i < lines.len() {
        let line = lines[*i];
        if is_blank(line) {
            pending_blank = true;
            *i += 1;
            continue;
        }
        if let Some(marker) = list_marker(line)
            && marker.ordered == ordered
        {
            if first && ordered {
                start = marker.number;
            }
            first = false;
            if pending_blank && !items.is_empty() {
                loose = true;
            }
            pending_blank = false;
            let content_indent = marker.content_indent;
            let mut content = vec![marker.content.to_owned()];
            *i += 1;
            let mut item_blank = false;
            while *i < lines.len() {
                let next = lines[*i];
                if is_blank(next) {
                    item_blank = true;
                    *i += 1;
                    continue;
                }
                if indent_width(next) >= content_indent {
                    if item_blank {
                        content.push(String::new());
                        loose = true;
                        item_blank = false;
                    }
                    content.push(next[content_indent.min(indent_width(next))..].to_owned());
                    *i += 1;
                    continue;
                }
                if item_blank {
                    pending_blank = true;
                }
                break;
            }
            if *i >= lines.len() && item_blank {
                pending_blank = true;
            }
            items.push((
                content.join("\n"),
                ordered.then(|| format!("{} ", marker.marker)),
            ));
            continue;
        }
        break;
    }
    // Rewind trailing blank consumption so a following Space token is emitted.
    if pending_blank {
        *i -= 1;
        while *i > 0 && is_blank(lines[*i - 1]) {
            *i -= 1;
        }
    }
    let items = items
        .into_iter()
        .map(|(raw, marker_raw)| {
            let (task, checked, content) = match raw.get(..4) {
                Some("[ ] ") => (true, false, raw[4..].to_owned()),
                Some("[x] ") | Some("[X] ") => (true, true, raw[4..].to_owned()),
                _ => (task_only(&raw), false, task_content(&raw)),
            };
            let mut tokens = lex_blocks(&content);
            if !loose {
                for token in &mut tokens {
                    if let Block::Paragraph(inline) = token {
                        *token = Block::Text(std::mem::take(inline));
                    }
                }
            }
            ListItem {
                task,
                checked,
                raw_marker: marker_raw,
                tokens,
            }
        })
        .collect();
    Block::List(ListToken {
        ordered,
        start,
        items,
    })
}

fn task_only(raw: &str) -> bool {
    raw == "[ ]" || raw == "[x]" || raw == "[X]"
}

fn task_content(raw: &str) -> String {
    if task_only(raw) {
        String::new()
    } else {
        raw.to_owned()
    }
}

// ============================================================================
// Inline lexer
// ============================================================================

const PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

fn lex_inline(src: &str) -> Vec<Inline> {
    let mut tokens: Vec<Inline> = Vec::new();
    let mut text = String::new();
    let bytes = src.as_bytes();
    let mut i = 0;

    fn flush(text: &mut String, tokens: &mut Vec<Inline>) {
        if !text.is_empty() {
            tokens.push(Inline::Text(std::mem::take(text)));
        }
    }

    while i < bytes.len() {
        let rest = &src[i..];
        let c = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        match c {
            '\\' => {
                if let Some(next) = rest[1..].chars().next() {
                    if next == '\n' {
                        flush(&mut text, &mut tokens);
                        tokens.push(Inline::Br);
                        i += 2;
                        continue;
                    }
                    if PUNCTUATION.contains(next) {
                        text.push(next);
                        i += 1 + next.len_utf8();
                        continue;
                    }
                }
                text.push('\\');
                i += 1;
            }
            '\n' => {
                // Hard break: two or more trailing spaces before the newline.
                let trailing_spaces = text.len() - text.trim_end_matches(' ').len();
                if trailing_spaces >= 2 && !rest[1..].trim().is_empty() {
                    text.truncate(text.trim_end_matches(' ').len());
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Br);
                } else {
                    text.truncate(text.trim_end_matches(' ').len());
                    text.push('\n');
                }
                i += 1;
            }
            '`' => {
                let run = rest.chars().take_while(|c| *c == '`').count();
                if let Some((content, consumed)) = codespan(rest, run) {
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Codespan(content));
                    i += consumed;
                } else {
                    text.push_str(&rest[..run]);
                    i += run;
                }
            }
            '*' | '_' => {
                let prev = src[..i].chars().next_back();
                if let Some((inline, consumed)) = emphasis(rest, c, prev) {
                    flush(&mut text, &mut tokens);
                    tokens.push(inline);
                    i += consumed;
                } else {
                    text.push(c);
                    i += c.len_utf8();
                }
            }
            '~' => {
                if let Some((inner, consumed)) = strikethrough(rest) {
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Del(lex_inline(&inner)));
                    i += consumed;
                } else {
                    text.push('~');
                    i += 1;
                }
            }
            '[' => {
                if let Some((link_text, href, consumed)) = link(rest) {
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Link {
                        href,
                        tokens: lex_inline(&link_text),
                        text: link_text,
                    });
                    i += consumed;
                } else {
                    text.push('[');
                    i += 1;
                }
            }
            '!' => {
                if rest[1..].starts_with('[')
                    && let Some((alt, _href, consumed)) = link(&rest[1..])
                {
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Text(alt));
                    i += 1 + consumed;
                } else {
                    text.push('!');
                    i += 1;
                }
            }
            '<' => {
                if let Some((token, consumed)) = angle_bracket(rest) {
                    flush(&mut text, &mut tokens);
                    tokens.push(token);
                    i += consumed;
                } else {
                    text.push('<');
                    i += 1;
                }
            }
            _ => {
                // GFM bare-URL autolink at a token boundary.
                let at_boundary = src[..i]
                    .chars()
                    .next_back()
                    .is_none_or(|p| !p.is_alphanumeric() && p != '/');
                if at_boundary
                    && (rest.starts_with("http://")
                        || rest.starts_with("https://")
                        || rest.starts_with("www."))
                    && let Some((href, display, consumed)) = bare_url(rest)
                {
                    flush(&mut text, &mut tokens);
                    tokens.push(Inline::Link {
                        href,
                        tokens: vec![Inline::Text(display.clone())],
                        text: display,
                    });
                    i += consumed;
                    continue;
                }
                text.push(c);
                i += c.len_utf8();
            }
        }
    }
    flush(&mut text, &mut tokens);
    tokens
}

fn codespan(rest: &str, run: usize) -> Option<(String, usize)> {
    let open = &rest[..run];
    let mut search = run;
    while let Some(found) = rest[search..].find(open) {
        let at = search + found;
        let closing_run = rest[at..].chars().take_while(|c| *c == '`').count();
        if closing_run == run {
            let mut content = rest[run..at].replace('\n', " ");
            if content.len() >= 2
                && content.starts_with(' ')
                && content.ends_with(' ')
                && !content.trim().is_empty()
            {
                content = content[1..content.len() - 1].to_owned();
            }
            return Some((content, at + run));
        }
        search = at + closing_run;
    }
    None
}

fn emphasis(rest: &str, delim: char, prev: Option<char>) -> Option<(Inline, usize)> {
    let run = rest.chars().take_while(|c| *c == delim).count();
    // Underscore emphasis cannot open inside a word.
    if delim == '_' && prev.is_some_and(|p| p.is_alphanumeric()) {
        return None;
    }
    if run >= 3 {
        // marked's emStrong: `***x***` is em wrapping strong.
        let closer: String = std::iter::repeat_n(delim, 3).collect();
        if let Some((inner, consumed)) = balanced_emphasis(&rest[3..], &closer, delim) {
            return Some((
                Inline::Em(vec![Inline::Strong(lex_inline(&inner))]),
                3 + consumed,
            ));
        }
    }
    if run >= 2 {
        let open = 2;
        let closer: String = std::iter::repeat_n(delim, 2).collect();
        if let Some((inner, consumed)) = balanced_emphasis(&rest[open..], &closer, delim) {
            return Some((Inline::Strong(lex_inline(&inner)), open + consumed));
        }
    }
    if run >= 1 {
        let closer = delim.to_string();
        if let Some((inner, consumed)) = balanced_emphasis(&rest[1..], &closer, delim) {
            return Some((Inline::Em(lex_inline(&inner)), 1 + consumed));
        }
    }
    None
}

fn balanced_emphasis(body: &str, closer: &str, delim: char) -> Option<(String, usize)> {
    // Content must start and end with non-whitespace.
    let first = body.chars().next()?;
    if first.is_whitespace() || first == delim {
        return None;
    }
    let mut search = 0;
    while let Some(found) = body[search..].find(closer) {
        let at = search + found;
        if at == 0 {
            return None;
        }
        let before = body[..at].chars().next_back()?;
        let escaped = body[..at].ends_with('\\');
        // A longer delimiter run than the closer belongs to inner content.
        let run_here = body[at..].chars().take_while(|c| *c == delim).count();
        if !before.is_whitespace() && !escaped && run_here == closer.len() {
            let after = body[at + closer.len()..].chars().next();
            if delim == '_' && after.is_some_and(|a| a.is_alphanumeric()) {
                search = at + run_here;
                continue;
            }
            return Some((body[..at].to_owned(), at + closer.len()));
        }
        search = at + run_here.max(1);
    }
    None
}

fn strikethrough(rest: &str) -> Option<(String, usize)> {
    // Port of pi's STRICT_STRIKETHROUGH_REGEX.
    if !rest.starts_with("~~") {
        return None;
    }
    let body = &rest[2..];
    let first = body.chars().next()?;
    if first.is_whitespace() || first == '~' {
        return None;
    }
    let mut search = 0;
    while let Some(found) = body[search..].find("~~") {
        let at = search + found;
        if at == 0 {
            return None;
        }
        let before = body[..at].chars().next_back()?;
        let escaped = body[..at].ends_with('\\') && !body[..at].ends_with("\\\\");
        let after = body[at + 2..].chars().next();
        if !before.is_whitespace() && before != '~' && !escaped && after != Some('~') {
            return Some((body[..at].to_owned(), at + 4));
        }
        search = at + 1;
    }
    None
}

fn link(rest: &str) -> Option<(String, String, usize)> {
    // rest starts with '['. Find the matching ']' (nesting-aware).
    let mut depth = 0;
    let mut close = None;
    for (index, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            '\\' => {}
            _ => {}
        }
    }
    let close = close?;
    let text = rest[1..close].to_owned();
    let after = &rest[close + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let mut paren_depth = 0;
    let mut end = None;
    for (index, c) in after.char_indices() {
        match c {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let mut href = after[1..end].trim().to_owned();
    // Drop an optional title.
    for quote in ['"', '\''] {
        if let Some(space) = href.find(&format!(" {quote}"))
            && href.ends_with(quote)
        {
            href = href[..space].trim_end().to_owned();
            break;
        }
    }
    if href.starts_with('<') && href.ends_with('>') {
        href = href[1..href.len() - 1].to_owned();
    }
    Some((text, href, close + 1 + end + 1))
}

fn angle_bracket(rest: &str) -> Option<(Inline, usize)> {
    let end = rest.find('>')?;
    let inner = &rest[1..end];
    if inner.contains('\n') || inner.is_empty() {
        return None;
    }
    if inner.contains("://") && !inner.contains(char::is_whitespace) {
        return Some((
            Inline::Link {
                href: inner.to_owned(),
                tokens: vec![Inline::Text(inner.to_owned())],
                text: inner.to_owned(),
            },
            end + 1,
        ));
    }
    if inner.contains('@') && !inner.contains(char::is_whitespace) && !inner.contains('<') {
        return Some((
            Inline::Link {
                href: format!("mailto:{inner}"),
                tokens: vec![Inline::Text(inner.to_owned())],
                text: inner.to_owned(),
            },
            end + 1,
        ));
    }
    let first = inner.chars().next()?;
    if first.is_ascii_alphabetic() || matches!(first, '/' | '!' | '?') {
        return Some((Inline::Html(rest[..end + 1].to_owned()), end + 1));
    }
    None
}

fn bare_url(rest: &str) -> Option<(String, String, usize)> {
    let mut end = rest
        .find(|c: char| c.is_whitespace() || c == '<')
        .unwrap_or(rest.len());
    // Back off trailing punctuation and unbalanced closing parens.
    loop {
        let candidate = &rest[..end];
        let last = candidate.chars().next_back()?;
        if matches!(
            last,
            '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '"' | '*' | '_' | '~'
        ) {
            end -= last.len_utf8();
            continue;
        }
        if last == ')' {
            let opens = candidate.matches('(').count();
            let closes = candidate.matches(')').count();
            if closes > opens {
                end -= 1;
                continue;
            }
        }
        break;
    }
    let display = rest[..end].to_owned();
    if display == "www." || display.len() < 5 {
        return None;
    }
    let href = if display.starts_with("www.") {
        format!("http://{display}")
    } else {
        display.clone()
    };
    Some((href, display, end))
}

// ============================================================================
// Renderer (1:1 port of markdown.ts)
// ============================================================================

struct InlineStyleContext<'a> {
    apply_text: Box<dyn Fn(&str) -> String + 'a>,
    style_prefix: String,
}

fn get_style_prefix(style: &dyn Fn(&str) -> String) -> String {
    let sentinel = "\u{0000}";
    let styled = style(sentinel);
    styled
        .find(sentinel)
        .map(|at| styled[..at].to_owned())
        .unwrap_or_default()
}

pub struct MarkdownRenderer<'a> {
    theme: &'a MarkdownTheme<'a>,
    default_style: Option<&'a DefaultTextStyle<'a>>,
    options: MarkdownOptions,
}

impl<'a> MarkdownRenderer<'a> {
    pub fn new(
        theme: &'a MarkdownTheme<'a>,
        default_style: Option<&'a DefaultTextStyle<'a>>,
        options: MarkdownOptions,
    ) -> Self {
        Self {
            theme,
            default_style,
            options,
        }
    }

    /// Render markdown to padded terminal lines (pi `Markdown.render`).
    pub fn render(
        &self,
        text: &str,
        width: usize,
        padding_x: usize,
        padding_y: usize,
    ) -> Vec<String> {
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        if text.trim().is_empty() {
            return Vec::new();
        }
        let normalized = text.replace('\t', "   ");
        let tokens = lex_blocks(&normalized);

        let mut rendered_lines: Vec<String> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let next_kind = tokens.get(index + 1).map(Block::kind);
            for line in self.render_token(token, content_width, next_kind, None) {
                rendered_lines.push(line);
            }
        }

        let mut wrapped_lines: Vec<String> = Vec::new();
        for line in rendered_lines {
            if crate::terminal_image::is_image_line(&line) {
                wrapped_lines.push(line);
            } else {
                wrapped_lines.extend(wrap_text_with_ansi(&line, content_width));
            }
        }

        let margin = " ".repeat(padding_x);
        let bg = self.default_style.and_then(|s| s.bg_color.as_ref());
        let mut content_lines: Vec<String> = Vec::new();
        for line in wrapped_lines {
            if crate::terminal_image::is_image_line(&line) {
                content_lines.push(line);
                continue;
            }
            let with_margins = format!("{margin}{line}{margin}");
            if let Some(bg) = bg {
                content_lines.push(apply_background_to_line(&with_margins, width, bg));
            } else {
                let visible = visible_width(&with_margins);
                let mut padded = with_margins;
                padded.push_str(&" ".repeat(width.saturating_sub(visible)));
                content_lines.push(padded);
            }
        }

        let empty = " ".repeat(width);
        let empty_line = if let Some(bg) = bg {
            apply_background_to_line(&empty, width, bg)
        } else {
            empty
        };
        let mut result: Vec<String> = Vec::new();
        for _ in 0..padding_y {
            result.push(empty_line.clone());
        }
        result.extend(content_lines);
        for _ in 0..padding_y {
            result.push(empty_line.clone());
        }
        if result.is_empty() {
            vec![String::new()]
        } else {
            result
        }
    }

    fn apply_default_style(&self, text: &str) -> String {
        let Some(style) = self.default_style else {
            return text.to_owned();
        };
        let mut styled = text.to_owned();
        if let Some(color) = style.color.as_ref() {
            styled = color(&styled);
        }
        if style.bold {
            styled = (self.theme.bold)(&styled);
        }
        if style.italic {
            styled = (self.theme.italic)(&styled);
        }
        if style.strikethrough {
            styled = (self.theme.strikethrough)(&styled);
        }
        if style.underline {
            styled = (self.theme.underline)(&styled);
        }
        styled
    }

    fn default_style_prefix(&self) -> String {
        if self.default_style.is_none() {
            return String::new();
        }
        get_style_prefix(&|text: &str| self.apply_default_style(text))
    }

    fn default_context(&self) -> InlineStyleContext<'_> {
        InlineStyleContext {
            apply_text: Box::new(move |text| self.apply_default_style(text)),
            style_prefix: self.default_style_prefix(),
        }
    }

    fn render_token(
        &self,
        token: &Block,
        width: usize,
        next_kind: Option<&str>,
        style_context: Option<&InlineStyleContext<'_>>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        match token {
            Block::Heading { depth, tokens } => {
                let heading_prefix = format!("{} ", "#".repeat(*depth));
                let depth = *depth;
                let theme = self.theme;
                let heading_style: Box<dyn Fn(&str) -> String> = if depth == 1 {
                    Box::new(move |text: &str| {
                        (theme.heading)(&(theme.bold)(&(theme.underline)(text)))
                    })
                } else {
                    Box::new(move |text: &str| (theme.heading)(&(theme.bold)(text)))
                };
                let heading_context = InlineStyleContext {
                    style_prefix: get_style_prefix(&heading_style),
                    apply_text: heading_style,
                };
                let heading_text = self.render_inline_tokens(tokens, Some(&heading_context));
                let styled = if depth >= 3 {
                    format!(
                        "{}{heading_text}",
                        (heading_context.apply_text)(&heading_prefix)
                    )
                } else {
                    heading_text
                };
                lines.push(styled);
                if next_kind.is_some_and(|kind| kind != "space") {
                    lines.push(String::new());
                }
            }
            Block::Paragraph(tokens) => {
                lines.push(self.render_inline_tokens(tokens, style_context));
                if next_kind.is_some_and(|kind| kind != "list" && kind != "space") {
                    lines.push(String::new());
                }
            }
            Block::Text(tokens) => {
                lines.push(self.render_inline_tokens(tokens, style_context));
            }
            Block::Code { lang, text } => {
                let indent = self.theme.code_block_indent.as_deref().unwrap_or("  ");
                lines.push((self.theme.code_block_border)(&format!(
                    "```{}",
                    lang.as_deref().unwrap_or("")
                )));
                if let Some(highlight) = self.theme.highlight_code.as_ref() {
                    for hl_line in highlight(text, lang.as_deref()) {
                        lines.push(format!("{indent}{hl_line}"));
                    }
                } else {
                    for code_line in text.split('\n') {
                        lines.push(format!("{indent}{}", (self.theme.code_block)(code_line)));
                    }
                }
                lines.push((self.theme.code_block_border)("```"));
                if next_kind.is_some_and(|kind| kind != "space") {
                    lines.push(String::new());
                }
            }
            Block::List(list) => {
                lines.extend(self.render_list(list, 0, width, style_context));
            }
            Block::Table { header, rows, raw } => {
                lines.extend(self.render_table(header, rows, raw, width, next_kind, style_context));
            }
            Block::Blockquote(tokens) => {
                let theme = self.theme;
                let quote_style = |text: &str| (theme.quote)(&(theme.italic)(text));
                let quote_prefix = get_style_prefix(&quote_style);
                let apply_quote_style = |line: &str| -> String {
                    if quote_prefix.is_empty() {
                        quote_style(line)
                    } else {
                        let reapplied = line.replace("\x1b[0m", &format!("\x1b[0m{quote_prefix}"));
                        quote_style(&reapplied)
                    }
                };
                let quote_width = width.saturating_sub(2).max(1);
                let quote_context = InlineStyleContext {
                    apply_text: Box::new(|text: &str| text.to_owned()),
                    style_prefix: quote_prefix.clone(),
                };
                let mut rendered: Vec<String> = Vec::new();
                for (index, quote_token) in tokens.iter().enumerate() {
                    let next = tokens.get(index + 1).map(Block::kind);
                    rendered.extend(self.render_token(
                        quote_token,
                        quote_width,
                        next,
                        Some(&quote_context),
                    ));
                }
                while rendered.last().is_some_and(|l| l.is_empty()) {
                    rendered.pop();
                }
                for quote_line in rendered {
                    let styled = apply_quote_style(&quote_line);
                    for wrapped in wrap_text_with_ansi(&styled, quote_width) {
                        lines.push(format!("{}{wrapped}", (self.theme.quote_border)("│ ")));
                    }
                }
                if next_kind.is_some_and(|kind| kind != "space") {
                    lines.push(String::new());
                }
            }
            Block::Hr => {
                lines.push((self.theme.hr)(&"─".repeat(width.min(80))));
                if next_kind.is_some_and(|kind| kind != "space") {
                    lines.push(String::new());
                }
            }
            Block::Html(raw) => {
                lines.push(self.apply_default_style(raw.trim()));
            }
            Block::Space => {
                lines.push(String::new());
            }
        }
        lines
    }

    fn render_inline_tokens(
        &self,
        tokens: &[Inline],
        style_context: Option<&InlineStyleContext<'_>>,
    ) -> String {
        let default_context;
        let context = match style_context {
            Some(context) => context,
            None => {
                default_context = self.default_context();
                &default_context
            }
        };
        let apply_with_newlines = |text: &str| -> String {
            text.split('\n')
                .map(|segment| (context.apply_text)(segment))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut result = String::new();
        for token in tokens {
            match token {
                Inline::Text(text) => result.push_str(&apply_with_newlines(text)),
                Inline::Strong(inner) => {
                    let content = self.render_inline_tokens(inner, Some(context));
                    result.push_str(&(self.theme.bold)(&content));
                    result.push_str(&context.style_prefix);
                }
                Inline::Em(inner) => {
                    let content = self.render_inline_tokens(inner, Some(context));
                    result.push_str(&(self.theme.italic)(&content));
                    result.push_str(&context.style_prefix);
                }
                Inline::Codespan(text) => {
                    result.push_str(&(self.theme.code)(text));
                    result.push_str(&context.style_prefix);
                }
                Inline::Link { href, text, tokens } => {
                    let link_text = self.render_inline_tokens(tokens, Some(context));
                    let styled = (self.theme.link)(&(self.theme.underline)(&link_text));
                    if crate::terminal_image::get_capabilities().hyperlinks {
                        // OSC 8: clickable hyperlink; the URL is never printed
                        // inline (pi markdown.ts link case).
                        result.push_str(&crate::terminal_image::hyperlink(&styled, href));
                    } else {
                        let href_comparison = href.strip_prefix("mailto:").unwrap_or(href);
                        if text == href || text == href_comparison {
                            result.push_str(&styled);
                        } else {
                            result.push_str(&styled);
                            result.push_str(&(self.theme.link_url)(&format!(" ({href})")));
                        }
                    }
                    result.push_str(&context.style_prefix);
                }
                Inline::Br => result.push('\n'),
                Inline::Del(inner) => {
                    let content = self.render_inline_tokens(inner, Some(context));
                    result.push_str(&(self.theme.strikethrough)(&content));
                    result.push_str(&context.style_prefix);
                }
                Inline::Html(raw) => result.push_str(&apply_with_newlines(raw)),
            }
        }
        while !context.style_prefix.is_empty() && result.ends_with(&context.style_prefix) {
            result.truncate(result.len() - context.style_prefix.len());
        }
        result
    }

    fn render_list(
        &self,
        list: &ListToken,
        depth: usize,
        width: usize,
        style_context: Option<&InlineStyleContext<'_>>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let indent = "    ".repeat(depth);
        for (index, item) in list.items.iter().enumerate() {
            let bullet = if list.ordered {
                if self.options.preserve_ordered_list_markers {
                    item.raw_marker
                        .clone()
                        .unwrap_or_else(|| format!("{}. ", list.start + index as i64))
                } else {
                    format!("{}. ", list.start + index as i64)
                }
            } else {
                "- ".to_owned()
            };
            let task_marker = if item.task {
                format!("[{}] ", if item.checked { "x" } else { " " })
            } else {
                String::new()
            };
            let marker = format!("{bullet}{task_marker}");
            let first_prefix = format!("{indent}{}", (self.theme.list_bullet)(&marker));
            let continuation_prefix = format!("{indent}{}", " ".repeat(visible_width(&marker)));
            let item_width = width.saturating_sub(visible_width(&first_prefix)).max(1);
            let mut rendered_any = false;
            for item_token in &item.tokens {
                if let Block::List(nested) = item_token {
                    lines.extend(self.render_list(nested, depth + 1, width, style_context));
                    rendered_any = true;
                    continue;
                }
                for line in self.render_token(item_token, item_width, None, style_context) {
                    for wrapped in wrap_text_with_ansi(&line, item_width) {
                        let prefix = if rendered_any {
                            &continuation_prefix
                        } else {
                            &first_prefix
                        };
                        lines.push(format!("{prefix}{wrapped}"));
                        rendered_any = true;
                    }
                }
            }
            if !rendered_any {
                lines.push(first_prefix);
            }
        }
        lines
    }

    fn longest_word_width(text: &str, max_width: usize) -> usize {
        let longest = text
            .split_whitespace()
            .map(visible_width)
            .max()
            .unwrap_or(0);
        longest.min(max_width)
    }

    #[allow(clippy::too_many_lines)]
    fn render_table(
        &self,
        header: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        raw: &str,
        available_width: usize,
        next_kind: Option<&str>,
        style_context: Option<&InlineStyleContext<'_>>,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let num_cols = header.len();
        if num_cols == 0 {
            return lines;
        }
        let border_overhead = 3 * num_cols + 1;
        let available_for_cells = available_width as i64 - border_overhead as i64;
        if available_for_cells < num_cols as i64 {
            let mut fallback: Vec<String> = if raw.is_empty() {
                Vec::new()
            } else {
                wrap_text_with_ansi(raw, available_width)
            };
            if next_kind.is_some_and(|kind| kind != "space") {
                fallback.push(String::new());
            }
            return fallback;
        }
        let available_for_cells = available_for_cells as usize;
        let max_unbroken = 30;

        let mut natural_widths = vec![0usize; num_cols];
        let mut min_word_widths = vec![1usize; num_cols];
        for (i, cell) in header.iter().enumerate() {
            let text = self.render_inline_tokens(cell, style_context);
            natural_widths[i] = visible_width(&text);
            min_word_widths[i] = Self::longest_word_width(&text, max_unbroken).max(1);
        }
        for row in rows {
            for (i, cell) in row.iter().enumerate().take(num_cols) {
                let text = self.render_inline_tokens(cell, style_context);
                natural_widths[i] = natural_widths[i].max(visible_width(&text));
                min_word_widths[i] =
                    min_word_widths[i].max(Self::longest_word_width(&text, max_unbroken).max(1));
            }
        }

        let mut min_column_widths = min_word_widths.clone();
        let mut min_cells_width: usize = min_column_widths.iter().sum();
        if min_cells_width > available_for_cells {
            min_column_widths = vec![1; num_cols];
            let remaining = available_for_cells.saturating_sub(num_cols);
            if remaining > 0 {
                let total_weight: usize = min_word_widths.iter().map(|w| w.saturating_sub(1)).sum();
                let growth: Vec<usize> = min_word_widths
                    .iter()
                    .map(|w| {
                        let weight = w.saturating_sub(1);
                        (weight * remaining).checked_div(total_weight).unwrap_or(0)
                    })
                    .collect();
                for (i, g) in growth.iter().enumerate() {
                    min_column_widths[i] += g;
                }
                let allocated: usize = growth.iter().sum();
                let mut leftover = remaining.saturating_sub(allocated);
                let mut i = 0;
                while leftover > 0 && i < num_cols {
                    min_column_widths[i] += 1;
                    leftover -= 1;
                    i += 1;
                }
            }
            min_cells_width = min_column_widths.iter().sum();
        }

        let total_natural: usize = natural_widths.iter().sum::<usize>() + border_overhead;
        let mut column_widths: Vec<usize>;
        if total_natural <= available_width {
            column_widths = natural_widths
                .iter()
                .zip(&min_column_widths)
                .map(|(n, m)| (*n).max(*m))
                .collect();
        } else {
            let total_grow: usize = natural_widths
                .iter()
                .zip(&min_column_widths)
                .map(|(n, m)| n.saturating_sub(*m))
                .sum();
            let extra = available_for_cells.saturating_sub(min_cells_width);
            column_widths = min_column_widths
                .iter()
                .zip(&natural_widths)
                .map(|(m, n)| {
                    let delta = n.saturating_sub(*m);
                    m + (delta * extra).checked_div(total_grow).unwrap_or(0)
                })
                .collect();
            let allocated: usize = column_widths.iter().sum();
            let mut remaining = available_for_cells.saturating_sub(allocated);
            while remaining > 0 {
                let mut grew = false;
                for i in 0..num_cols {
                    if remaining == 0 {
                        break;
                    }
                    if column_widths[i] < natural_widths[i] {
                        column_widths[i] += 1;
                        remaining -= 1;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
        }

        let borders = |left: &str, mid: &str, right: &str| -> String {
            let cells: Vec<String> = column_widths.iter().map(|w| "─".repeat(*w)).collect();
            format!("{left}─{}─{right}", cells.join(&format!("─{mid}─")))
        };
        lines.push(borders("┌", "┬", "┐"));

        let wrap_cell = |text: &str, max_width: usize| wrap_text_with_ansi(text, max_width.max(1));
        let header_cells: Vec<Vec<String>> = header
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                wrap_cell(
                    &self.render_inline_tokens(cell, style_context),
                    column_widths[i],
                )
            })
            .collect();
        let header_line_count = header_cells.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..header_line_count {
            let parts: Vec<String> = header_cells
                .iter()
                .enumerate()
                .map(|(col, cell_lines)| {
                    let text = cell_lines.get(line_index).cloned().unwrap_or_default();
                    let padded = format!(
                        "{text}{}",
                        " ".repeat(column_widths[col].saturating_sub(visible_width(&text)))
                    );
                    (self.theme.bold)(&padded)
                })
                .collect();
            lines.push(format!("│ {} │", parts.join(" │ ")));
        }

        let separator = borders("├", "┼", "┤");
        lines.push(separator.clone());

        for (row_index, row) in rows.iter().enumerate() {
            let row_cells: Vec<Vec<String>> = row
                .iter()
                .enumerate()
                .take(num_cols)
                .map(|(i, cell)| {
                    wrap_cell(
                        &self.render_inline_tokens(cell, style_context),
                        column_widths[i],
                    )
                })
                .collect();
            let row_line_count = row_cells.iter().map(Vec::len).max().unwrap_or(1);
            for line_index in 0..row_line_count {
                let parts: Vec<String> = (0..num_cols)
                    .map(|col| {
                        let text = row_cells
                            .get(col)
                            .and_then(|c| c.get(line_index))
                            .cloned()
                            .unwrap_or_default();
                        format!(
                            "{text}{}",
                            " ".repeat(column_widths[col].saturating_sub(visible_width(&text)))
                        )
                    })
                    .collect();
                lines.push(format!("│ {} │", parts.join(" │ ")));
            }
            if row_index < rows.len() - 1 {
                lines.push(separator.clone());
            }
        }
        lines.push(borders("└", "┴", "┘"));
        if next_kind.is_some_and(|kind| kind != "space") {
            lines.push(String::new());
        }
        lines
    }
}

/// Render markdown without theme styling (plain seam used by demos/tests).
pub fn render_markdown(
    text: &str,
    width: usize,
    padding_x: usize,
    padding_y: usize,
) -> Vec<String> {
    let theme = MarkdownTheme::plain();
    MarkdownRenderer::new(&theme, None, MarkdownOptions::default())
        .render(text, width, padding_x, padding_y)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn fixture_matches_plain_terminal_cells() {
        assert_eq!(
            render_markdown("# Hi\n\n- one\n- two", 12, 0, 0),
            [
                "Hi          ",
                "            ",
                "- one       ",
                "- two       "
            ]
        );
    }

    #[test]
    fn paragraph_spacing_follows_pi_rules() {
        // paragraph + space + paragraph keeps exactly one blank line.
        assert_eq!(
            render_markdown("one\n\ntwo", 8, 0, 0),
            ["one     ", "        ", "two     "]
        );
    }

    #[test]
    fn code_blocks_render_borders_and_indent() {
        assert_eq!(
            render_markdown("```lua\nprint(1)\n```", 14, 0, 0),
            ["```lua        ", "  print(1)    ", "```           "]
        );
    }

    #[test]
    fn ordered_lists_normalize_and_preserve_markers() {
        assert_eq!(
            render_markdown("3. three\n4. four", 12, 0, 0),
            ["3. three    ", "4. four     "]
        );
        let theme = MarkdownTheme::plain();
        let renderer = MarkdownRenderer::new(
            &theme,
            None,
            MarkdownOptions {
                preserve_ordered_list_markers: true,
            },
        );
        assert_eq!(renderer.render("3) three", 12, 0, 0), ["3) three    "]);
    }

    #[test]
    fn blockquote_uses_border_and_styles() {
        assert_eq!(render_markdown("> quoted", 12, 0, 0), ["│ quoted    "]);
    }

    #[test]
    fn inline_styles_use_theme_functions() {
        let theme = MarkdownTheme {
            bold: Box::new(|t: &str| format!("\x1b[1m{t}\x1b[22m")),
            ..MarkdownTheme::plain()
        };
        let renderer = MarkdownRenderer::new(&theme, None, MarkdownOptions::default());
        let lines = renderer.render("**hi** `code`", 20, 0, 0);
        assert_eq!(lines, ["\x1b[1mhi\x1b[22m code             "]);
    }

    #[test]
    fn default_color_applies_per_segment() {
        let theme = MarkdownTheme::plain();
        let style = DefaultTextStyle {
            color: Some(Box::new(|t: &str| format!("\x1b[38;5;10m{t}\x1b[39m"))),
            ..DefaultTextStyle::default()
        };
        let renderer = MarkdownRenderer::new(&theme, Some(&style), MarkdownOptions::default());
        let lines = renderer.render("hello", 10, 0, 0);
        assert_eq!(lines, ["\x1b[38;5;10mhello\x1b[39m     "]);
    }

    #[test]
    fn table_renders_box_borders() {
        let lines = render_markdown("| a | b |\n| - | - |\n| 1 | 2 |", 20, 0, 0);
        assert_eq!(lines[0].trim_end(), "┌───┬───┐");
        assert_eq!(lines[1].trim_end(), "│ a │ b │");
        assert_eq!(lines[2].trim_end(), "├───┼───┤");
        assert_eq!(lines[3].trim_end(), "│ 1 │ 2 │");
        assert_eq!(lines[4].trim_end(), "└───┴───┘");
    }

    #[test]
    fn table_interrupts_paragraph_without_blank_line() {
        let lines = render_markdown(
            "**Navigation**\n| Key | Action |\n| --- | --- |\n| Up | Move |",
            30,
            0,
            0,
        );
        assert_eq!(lines[0].trim_end(), "Navigation");
        assert_eq!(lines[2].trim_end(), "┌─────┬────────┐");
        assert_eq!(lines[3].trim_end(), "│ Key │ Action │");
    }

    #[test]
    fn triple_emphasis_is_em_wrapping_strong() {
        // marked lexes `***x***` as em(strong(x)).
        let theme = MarkdownTheme {
            bold: Box::new(|t: &str| format!("\x1b[1m{t}\x1b[22m")),
            italic: Box::new(|t: &str| format!("\x1b[3m{t}\x1b[23m")),
            ..MarkdownTheme::plain()
        };
        let renderer = MarkdownRenderer::new(&theme, None, MarkdownOptions::default());
        let lines = renderer.render("***both***", 20, 0, 0);
        assert_eq!(
            lines,
            ["\x1b[3m\x1b[1mboth\x1b[22m\x1b[23m                "]
        );
    }

    #[test]
    fn setext_underline_beats_hr_and_list_interrupts() {
        // marked's lheading consumes `text\n---` as a depth-2 heading before
        // the hr rule can interrupt the paragraph.
        assert_eq!(
            render_markdown("Config\n------\n\nafter", 10, 0, 0),
            ["Config    ", "          ", "after     "]
        );
        assert_eq!(render_markdown("Title\n===", 10, 0, 0), ["Title     "]);
    }

    #[test]
    fn links_show_href_when_text_differs() {
        // Pin capability-less rendering; the hyperlink branch is env-driven.
        crate::terminal_image::set_capabilities(crate::terminal_image::TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: false,
        });
        let lines = render_markdown("[click](https://x.dev)", 40, 0, 0);
        assert!(lines[0].contains("click (https://x.dev)"));
        let same = render_markdown("<https://x.dev>", 40, 0, 0);
        assert!(same[0].contains("https://x.dev"));
        assert!(!same[0].contains('('));
    }
}
