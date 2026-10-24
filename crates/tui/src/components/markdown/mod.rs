//! Markdown rendering (upstream `components/markdown.ts`), on a port of marked's lexer.

pub mod lexer;
mod rules;

use crate::tui::{Component, is_image_line};
use crate::utils::{apply_background_to_line, visible_width, wrap_text_with_ansi};
use lexer::{Kind, Token, lex};
use std::rc::Rc;

pub type StyleFn = Rc<dyn Fn(&str) -> String>;
/// Highlights a code block (code, language) into lines.
pub type HighlightFn = Rc<dyn Fn(&str, Option<&str>) -> Vec<String>>;
/// Transforms the source before parsing, given the width available for content.
pub type TransformFn = Rc<dyn Fn(&str, usize) -> String>;

/// Default text styling, applied to all text unless markdown formatting overrides it.
#[derive(Clone, Default)]
pub struct DefaultTextStyle {
    pub color: Option<StyleFn>,
    /// Applied at the padding stage, so it spans the full line width.
    pub bg_color: Option<StyleFn>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
}

/// Styles for markdown elements.
#[derive(Clone)]
pub struct MarkdownTheme {
    pub heading: StyleFn,
    pub link: StyleFn,
    pub link_url: StyleFn,
    pub code: StyleFn,
    pub code_block: StyleFn,
    pub code_block_border: StyleFn,
    pub quote: StyleFn,
    pub quote_border: StyleFn,
    pub hr: StyleFn,
    pub list_bullet: StyleFn,
    pub bold: StyleFn,
    pub italic: StyleFn,
    pub strikethrough: StyleFn,
    pub underline: StyleFn,
    pub highlight_code: Option<HighlightFn>,
    /// Prefix of each rendered code block line (default two spaces).
    pub code_block_indent: Option<String>,
}

/// Renders a LaTeX expression as Unicode text (`display` for block math); `None` when unsupported.
pub type LatexRenderer = Rc<dyn Fn(&str, bool) -> Option<String>>;

#[derive(Clone, Default)]
pub struct MarkdownOptions {
    /// Keep source list markers instead of normalizing them.
    pub preserve_ordered_list_markers: bool,
    /// Keep source backslash escapes instead of normalizing escaped punctuation.
    pub preserve_backslash_escapes: bool,
    /// Transform the source before parsing, given the width available for content.
    pub transform: Option<TransformFn>,
    /// Upstream `renderLatex` (default true). Rendering itself goes through `latex`.
    pub render_latex: Option<bool>,
    /// Replaces the LaTeX renderer, [`crate::latex::render_latex`] by default (upstream `latex.ts`).
    /// Math the renderer rejects (`None`) stays as source, as upstream shows it.
    pub latex: Option<LatexRenderer>,
    /// Terminal OSC 8 hyperlink support (upstream reads `getCapabilities().hyperlinks`).
    pub hyperlinks: bool,
}

#[derive(Clone)]
struct StyleContext {
    apply: StyleFn,
    prefix: String,
}

pub struct Markdown {
    text: String,
    padding_x: usize,
    padding_y: usize,
    default_style: Option<DefaultTextStyle>,
    theme: MarkdownTheme,
    options: MarkdownOptions,
    cache: Option<(String, usize, Vec<String>)>,
    tokens: Option<(String, Rc<Vec<Token>>)>,
}

fn style_prefix(f: &dyn Fn(&str) -> String) -> String {
    let styled = f("\0");
    styled
        .find('\0')
        .map_or(String::new(), |i| styled[..i].to_string())
}

fn hyperlink(text: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

fn js_trim(s: &str) -> &str {
    lexer::trim(s)
}

/// upstream `applyDefaultStyle`: foreground and decorations (the background comes at padding).
fn apply_default(style: Option<&DefaultTextStyle>, theme: &MarkdownTheme, text: &str) -> String {
    let Some(s) = style else {
        return text.to_string();
    };
    let mut styled = text.to_string();
    if let Some(c) = &s.color {
        styled = c(&styled);
    }
    if s.bold {
        styled = (theme.bold)(&styled);
    }
    if s.italic {
        styled = (theme.italic)(&styled);
    }
    if s.strikethrough {
        styled = (theme.strikethrough)(&styled);
    }
    if s.underline {
        styled = (theme.underline)(&styled);
    }
    styled
}

/// upstream `trimPartialClosingFences`.
fn trim_partial_closing_fences(tokens: &mut [Token]) {
    let Some(t) = tokens.last_mut() else { return };
    match t.kind {
        Kind::List => {
            if let Some(item) = t.items.last_mut() {
                trim_partial_closing_fences(&mut item.tokens);
            }
        }
        Kind::Blockquote => trim_partial_closing_fences(&mut t.tokens),
        Kind::Code => {
            let marker_len = {
                let first = t.raw.chars().next();
                match first {
                    Some(c @ ('`' | '~')) => t.raw.chars().take_while(|&x| x == c).count(),
                    _ => 0,
                }
            };
            if marker_len < 3 {
                return;
            }
            let mc = t.raw.chars().next().unwrap();
            let last_line = t.raw.rsplit('\n').next().unwrap_or("");
            let n = last_line.chars().count();
            if last_line.is_empty() || n >= marker_len || last_line.chars().any(|c| c != mc) {
                return;
            }
            let cut = t.text.len().saturating_sub(last_line.len());
            let mut text = t.text[..cut].to_string();
            if text.ends_with('\n') {
                text.pop();
            }
            t.text = text;
        }
        _ => {}
    }
}

impl Markdown {
    pub fn new(
        text: &str,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
        default_style: Option<DefaultTextStyle>,
        options: MarkdownOptions,
    ) -> Self {
        Markdown {
            text: text.into(),
            padding_x,
            padding_y,
            default_style,
            theme,
            options,
            cache: None,
            tokens: None,
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.into();
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.cache = None;
    }

    fn apply_default_style(&self, text: &str) -> String {
        apply_default(self.default_style.as_ref(), &self.theme, text)
    }

    fn default_context(&self) -> StyleContext {
        let style = self.default_style.clone();
        let theme = self.theme.clone();
        let prefix = if style.is_some() {
            style_prefix(&|t: &str| apply_default(style.as_ref(), &theme, t))
        } else {
            String::new()
        };
        StyleContext {
            apply: Rc::new(move |t: &str| apply_default(style.as_ref(), &theme, t)),
            prefix,
        }
    }

    fn latex(&self, text: &str, display: bool) -> Option<String> {
        if self.options.render_latex == Some(false) {
            return None;
        }
        match &self.options.latex {
            Some(f) => f(text, display),
            None => crate::latex::render_latex(text, display),
        }
    }
}

impl Component for Markdown {
    fn render(&mut self, width: usize) -> Vec<String> {
        if let Some((t, w, lines)) = &self.cache
            && *t == self.text
            && *w == width
        {
            return lines.clone();
        }
        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let text = match &self.options.transform {
            Some(f) => f(&self.text, content_width),
            None => self.text.clone(),
        };
        if text.is_empty() || js_trim(&text).is_empty() {
            self.cache = Some((self.text.clone(), width, Vec::new()));
            return Vec::new();
        }
        let normalized = text.replace('\t', "   ");
        let tokens = match &self.tokens {
            Some((src, toks)) if *src == normalized => toks.clone(),
            _ => {
                let mut toks = lex(&normalized);
                trim_partial_closing_fences(&mut toks);
                let toks = Rc::new(toks);
                self.tokens = Some((normalized, toks.clone()));
                toks
            }
        };
        let mut rendered = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            let next = tokens.get(i + 1).map(|t| t.kind);
            rendered.extend(self.render_token(t, content_width, next, None));
        }
        let mut wrapped = Vec::new();
        for line in rendered {
            if is_image_line(&line) {
                wrapped.push(line);
            } else {
                wrapped.extend(wrap_text_with_ansi(&line, content_width));
            }
        }
        let margin = " ".repeat(self.padding_x);
        let bg = self.default_style.as_ref().and_then(|s| s.bg_color.clone());
        let mut content = Vec::new();
        for line in wrapped {
            if is_image_line(&line) {
                content.push(line);
                continue;
            }
            let with_margins = format!("{margin}{line}{margin}");
            match &bg {
                Some(bg) => content.push(apply_background_to_line(
                    &with_margins,
                    width,
                    &|s: &str| bg(s),
                )),
                None => {
                    let pad = width.saturating_sub(visible_width(&with_margins));
                    content.push(with_margins + &" ".repeat(pad));
                }
            }
        }
        let empty = " ".repeat(width);
        let empty_line = match &bg {
            Some(bg) => apply_background_to_line(&empty, width, &|s: &str| bg(s)),
            None => empty,
        };
        let mut result = vec![empty_line.clone(); self.padding_y];
        result.extend(content);
        result.extend(std::iter::repeat_n(empty_line, self.padding_y));
        self.cache = Some((self.text.clone(), width, result.clone()));
        if result.is_empty() {
            vec![String::new()]
        } else {
            result
        }
    }
}

fn not_space(next: Option<Kind>) -> bool {
    next.is_some_and(|k| k != Kind::Space)
}

impl Markdown {
    fn render_token(
        &self,
        token: &Token,
        width: usize,
        next: Option<Kind>,
        ctx: Option<&StyleContext>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let th = &self.theme;
        match token.kind {
            Kind::Heading => {
                let depth = token.depth;
                let prefix = format!("{} ", "#".repeat(depth));
                let theme = th.clone();
                let style: StyleFn = if depth == 1 {
                    Rc::new(move |t: &str| (theme.heading)(&(theme.bold)(&(theme.underline)(t))))
                } else {
                    Rc::new(move |t: &str| (theme.heading)(&(theme.bold)(t)))
                };
                let hctx = StyleContext {
                    prefix: style_prefix(&*style),
                    apply: style.clone(),
                };
                let text = self.render_inline(&token.tokens, Some(&hctx));
                lines.push(if depth >= 3 {
                    style(&prefix) + &text
                } else {
                    text
                });
                if not_space(next) {
                    lines.push(String::new());
                }
            }
            Kind::Paragraph => {
                lines.push(self.render_inline(&token.tokens, ctx));
                if next.is_some_and(|k| k != Kind::List && k != Kind::Space) {
                    lines.push(String::new());
                }
            }
            Kind::Text => lines.push(self.render_inline(std::slice::from_ref(token), ctx)),
            Kind::LatexBlock => {
                let rendered = if token.pending {
                    None
                } else {
                    self.latex(&token.text, true)
                };
                let rendered = rendered.unwrap_or_else(|| js_trim(&token.raw).to_string());
                for line in rendered.split('\n') {
                    lines.push(self.apply_default_style(line));
                }
                if not_space(next) {
                    lines.push(String::new());
                }
            }
            Kind::Code => {
                let indent = th.code_block_indent.clone().unwrap_or_else(|| "  ".into());
                lines.push((th.code_block_border)(&format!(
                    "```{}",
                    token.lang.as_deref().unwrap_or("")
                )));
                match &th.highlight_code {
                    Some(h) => {
                        for l in h(&token.text, token.lang.as_deref()) {
                            lines.push(format!("{indent}{l}"));
                        }
                    }
                    None => {
                        for l in token.text.split('\n') {
                            lines.push(format!("{indent}{}", (th.code_block)(l)));
                        }
                    }
                }
                lines.push((th.code_block_border)("```"));
                if not_space(next) {
                    lines.push(String::new());
                }
            }
            Kind::List => lines.extend(self.render_list(token, 0, width, ctx)),
            Kind::Table => lines.extend(self.render_table(token, width, next, ctx)),
            Kind::Blockquote if width < 3 => {
                for (i, t) in token.tokens.iter().enumerate() {
                    let n = token.tokens.get(i + 1).map(|t| t.kind);
                    lines.extend(self.render_token(t, width, n, ctx));
                }
            }
            Kind::Blockquote => {
                let theme = th.clone();
                let quote_style: StyleFn =
                    Rc::new(move |t: &str| (theme.quote)(&(theme.italic)(t)));
                let qprefix = style_prefix(&*quote_style);
                let content_width = width - 2;
                let qctx = StyleContext {
                    apply: Rc::new(|t: &str| t.to_string()),
                    prefix: qprefix.clone(),
                };
                let mut quote_lines = Vec::new();
                for (i, t) in token.tokens.iter().enumerate() {
                    let n = token.tokens.get(i + 1).map(|t| t.kind);
                    quote_lines.extend(self.render_token(t, content_width, n, Some(&qctx)));
                }
                while quote_lines.last().is_some_and(|l| l.is_empty()) {
                    quote_lines.pop();
                }
                for ql in quote_lines {
                    let styled = if qprefix.is_empty() {
                        quote_style(&ql)
                    } else {
                        quote_style(&ql.replace("\x1b[0m", &format!("\x1b[0m{qprefix}")))
                    };
                    for w in wrap_text_with_ansi(&styled, content_width) {
                        lines.push((th.quote_border)("│ ") + &w);
                    }
                }
                if not_space(next) {
                    lines.push(String::new());
                }
            }
            Kind::Hr => {
                lines.push((th.hr)(&"─".repeat(width.min(80))));
                if not_space(next) {
                    lines.push(String::new());
                }
            }
            Kind::Html => lines.push(self.apply_default_style(js_trim(&token.raw))),
            Kind::Space => lines.push(String::new()),
            // def has no text; inline kinds never reach here at block level
            Kind::Def | Kind::Checkbox => {}
            _ => lines.push(token.text.clone()),
        }
        lines
    }
}

impl Markdown {
    fn render_inline(&self, tokens: &[Token], ctx: Option<&StyleContext>) -> String {
        let owned;
        let ctx = match ctx {
            Some(c) => c,
            None => {
                owned = self.default_context();
                &owned
            }
        };
        let apply = &ctx.apply;
        let prefix = &ctx.prefix;
        let with_newlines = |t: &str| {
            t.split('\n')
                .map(|s| apply(s))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let th = &self.theme;
        let mut out = String::new();
        for t in tokens {
            match t.kind {
                Kind::Latex => {
                    let rendered = if t.pending {
                        None
                    } else {
                        self.latex(&t.text, false)
                    };
                    out += &with_newlines(rendered.as_deref().unwrap_or(&t.raw));
                }
                Kind::Escape => {
                    out += &with_newlines(if self.options.preserve_backslash_escapes {
                        &t.raw
                    } else {
                        &t.text
                    })
                }
                Kind::Text => {
                    if t.tokens.is_empty() {
                        out += &with_newlines(&t.text);
                    } else {
                        out += &self.render_inline(&t.tokens, Some(ctx));
                    }
                }
                Kind::Paragraph => out += &self.render_inline(&t.tokens, Some(ctx)),
                Kind::Strong => {
                    out += &((th.bold)(&self.render_inline(&t.tokens, Some(ctx))) + prefix)
                }
                Kind::Em => {
                    out += &((th.italic)(&self.render_inline(&t.tokens, Some(ctx))) + prefix)
                }
                Kind::Codespan => out += &((th.code)(&t.text) + prefix),
                Kind::Link => {
                    let text = self.render_inline(&t.tokens, Some(ctx));
                    let styled = (th.link)(&(th.underline)(&text));
                    if self.options.hyperlinks {
                        out += &(hyperlink(&styled, &t.href) + prefix);
                    } else {
                        let cmp = t.href.strip_prefix("mailto:").unwrap_or(&t.href);
                        if t.text == t.href || t.text == cmp {
                            out += &(styled + prefix);
                        } else {
                            out += &(styled + &(th.link_url)(&format!(" ({})", t.href)) + prefix);
                        }
                    }
                }
                Kind::Br => out.push('\n'),
                Kind::Del => {
                    out += &((th.strikethrough)(&self.render_inline(&t.tokens, Some(ctx))) + prefix)
                }
                Kind::Html => out += &with_newlines(&t.raw),
                // tokens without text (checkbox, space, hr, br-like) add nothing
                Kind::Checkbox | Kind::Space | Kind::Hr | Kind::Def | Kind::List | Kind::Table => {}
                _ => out += &with_newlines(&t.text),
            }
        }
        if !prefix.is_empty() {
            while out.ends_with(prefix.as_str()) {
                out.truncate(out.len() - prefix.len());
            }
        }
        out
    }

    fn ordered_marker(item: &Token) -> Option<String> {
        let b = item.raw.as_bytes();
        let sp = b.iter().take(3).take_while(|&&c| c == b' ').count();
        let digits = b[sp..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let d = sp + digits;
        if !matches!(b.get(d), Some(b'.' | b')')) || !matches!(b.get(d + 1), Some(b' ' | b'\t')) {
            return None;
        }
        Some(format!("{} ", &item.raw[sp..=d]))
    }

    fn unordered_marker(item: &Token) -> Option<String> {
        let b = item.raw.as_bytes();
        let sp = b.iter().take(3).take_while(|&&c| c == b' ').count();
        let m = *b.get(sp)?;
        if !matches!(m, b'-' | b'+' | b'*') {
            return None;
        }
        match b.get(sp + 1) {
            None | Some(b' ' | b'\t' | b'\n') => Some(format!("{} ", m as char)),
            Some(b'\r') if b.get(sp + 2) == Some(&b'\n') => Some(format!("{} ", m as char)),
            _ => None,
        }
    }

    fn render_list(
        &self,
        token: &Token,
        depth: usize,
        width: usize,
        ctx: Option<&StyleContext>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let indent = "    ".repeat(depth);
        let start = token.start.unwrap_or(1);
        let preserve = self.options.preserve_ordered_list_markers;
        for (i, item) in token.items.iter().enumerate() {
            let last = i + 1 == token.items.len();
            let bullet = if token.ordered {
                let n = format!("{}. ", start + i as u64);
                if preserve {
                    Self::ordered_marker(item).unwrap_or(n)
                } else {
                    n
                }
            } else if preserve {
                Self::unordered_marker(item).unwrap_or_else(|| "- ".into())
            } else {
                "- ".into()
            };
            let task = if item.task {
                format!("[{}] ", if item.checked == Some(true) { "x" } else { " " })
            } else {
                String::new()
            };
            let marker = bullet + &task;
            let first_prefix = indent.clone() + &(self.theme.list_bullet)(&marker);
            let cont_prefix = indent.clone() + &" ".repeat(visible_width(&marker));
            let item_width = width.saturating_sub(visible_width(&first_prefix)).max(1);
            let mut any = false;
            for it in &item.tokens {
                if it.kind == Kind::List {
                    lines.extend(self.render_list(it, depth + 1, width, ctx));
                    any = true;
                    continue;
                }
                for line in self.render_token(it, item_width, None, ctx) {
                    for w in wrap_text_with_ansi(&line, item_width) {
                        lines.push(format!(
                            "{}{w}",
                            if any { &cont_prefix } else { &first_prefix }
                        ));
                        any = true;
                    }
                }
            }
            if !any {
                lines.push(first_prefix);
            }
            if token.loose && !last {
                lines.push(String::new());
            }
        }
        lines
    }
}

fn longest_word_width(text: &str, max: usize) -> usize {
    text.split(lexer::js_ws)
        .filter(|w| !w.is_empty())
        .map(visible_width)
        .max()
        .unwrap_or(0)
        .min(max)
}

fn wrap_cell(text: &str, max_width: usize, prefix: &str) -> Vec<String> {
    let lines = wrap_text_with_ansi(text, max_width.max(1));
    let n = lines.len();
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let reset = if i + 1 < n {
                "\x1b[22;23;24;25;27;28;29;39m"
            } else {
                ""
            };
            format!("{l}{reset}{prefix}")
        })
        .collect()
}

impl Markdown {
    fn render_table(
        &self,
        token: &Token,
        avail: usize,
        next: Option<Kind>,
        ctx: Option<&StyleContext>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let cols = token.header.len();
        if cols == 0 {
            return lines;
        }
        let overhead = 3 * cols + 1;
        let for_cells = avail as isize - overhead as isize;
        if for_cells < cols as isize {
            let mut fallback = if token.raw.is_empty() {
                Vec::new()
            } else {
                wrap_text_with_ansi(&token.raw, avail)
            };
            if not_space(next) {
                fallback.push(String::new());
            }
            return fallback;
        }
        let for_cells = for_cells as usize;
        const MAX_WORD: usize = 30;
        let mut natural = vec![0usize; cols];
        let mut min_word = vec![1usize; cols];
        for (i, h) in token.header.iter().enumerate() {
            let t = self.render_inline(&h.tokens, ctx);
            natural[i] = visible_width(&t);
            min_word[i] = longest_word_width(&t, MAX_WORD).max(1);
        }
        for row in &token.rows {
            for (i, c) in row.iter().enumerate() {
                let t = self.render_inline(&c.tokens, ctx);
                natural[i] = natural[i].max(visible_width(&t));
                min_word[i] = min_word[i].max(longest_word_width(&t, MAX_WORD));
            }
        }
        let mut min_cols = min_word.clone();
        let mut min_total: usize = min_cols.iter().sum();
        if min_total > for_cells {
            min_cols = vec![1; cols];
            let remaining = for_cells - cols;
            if remaining > 0 {
                let total_weight: usize = min_word.iter().map(|w| w - 1).sum();
                let growth: Vec<usize> = min_word
                    .iter()
                    .map(|w| {
                        if total_weight > 0 {
                            ((w - 1) as f64 / total_weight as f64 * remaining as f64).floor()
                                as usize
                        } else {
                            0
                        }
                    })
                    .collect();
                for i in 0..cols {
                    min_cols[i] += growth[i];
                }
                let mut leftover = remaining - growth.iter().sum::<usize>();
                let mut i = 0;
                while leftover > 0 && i < cols {
                    min_cols[i] += 1;
                    leftover -= 1;
                    i += 1;
                }
            }
            min_total = min_cols.iter().sum();
        }
        let total_natural: usize = natural.iter().sum::<usize>() + overhead;
        let widths: Vec<usize> = if total_natural <= avail {
            natural
                .iter()
                .zip(&min_cols)
                .map(|(n, m)| *n.max(m))
                .collect()
        } else {
            let grow_potential: usize = natural
                .iter()
                .zip(&min_cols)
                .map(|(n, m)| n.saturating_sub(*m))
                .sum();
            let extra = for_cells.saturating_sub(min_total);
            let mut w: Vec<usize> = min_cols
                .iter()
                .zip(&natural)
                .map(|(m, n)| {
                    let delta = n.saturating_sub(*m);
                    let grow = if grow_potential > 0 {
                        (delta as f64 / grow_potential as f64 * extra as f64).floor() as usize
                    } else {
                        0
                    };
                    m + grow
                })
                .collect();
            let allocated: usize = w.iter().sum();
            let mut remaining = for_cells as isize - allocated as isize;
            while remaining > 0 {
                let mut grew = false;
                for i in 0..cols {
                    if remaining <= 0 {
                        break;
                    }
                    if w[i] < natural[i] {
                        w[i] += 1;
                        remaining -= 1;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            w
        };
        let bar = |l: &str, m: &str, r: &str| {
            format!(
                "{l}─{}─{r}",
                widths
                    .iter()
                    .map(|w| "─".repeat(*w))
                    .collect::<Vec<_>>()
                    .join(&format!("─{m}─"))
            )
        };
        lines.push(bar("┌", "┬", "┐"));
        let prefix = ctx.map_or("", |c| c.prefix.as_str());
        let header_cells: Vec<Vec<String>> = token
            .header
            .iter()
            .enumerate()
            .map(|(i, c)| wrap_cell(&self.render_inline(&c.tokens, ctx), widths[i], prefix))
            .collect();
        let pad = |text: &str, w: usize| {
            format!(
                "{text}{}",
                " ".repeat(w.saturating_sub(visible_width(text)))
            )
        };
        let header_n = header_cells.iter().map(Vec::len).max().unwrap_or(0);
        for li in 0..header_n {
            let parts: Vec<String> = header_cells
                .iter()
                .enumerate()
                .map(|(ci, cl)| {
                    (self.theme.bold)(&pad(cl.get(li).map_or("", String::as_str), widths[ci]))
                })
                .collect();
            lines.push(format!("│ {} │", parts.join(" │ ")));
        }
        let sep = bar("├", "┼", "┤");
        lines.push(sep.clone());
        for (ri, row) in token.rows.iter().enumerate() {
            let cells: Vec<Vec<String>> = row
                .iter()
                .enumerate()
                .map(|(i, c)| wrap_cell(&self.render_inline(&c.tokens, ctx), widths[i], prefix))
                .collect();
            let n = cells.iter().map(Vec::len).max().unwrap_or(0);
            for li in 0..n {
                let parts: Vec<String> = cells
                    .iter()
                    .enumerate()
                    .map(|(ci, cl)| pad(cl.get(li).map_or("", String::as_str), widths[ci]))
                    .collect();
                lines.push(format!("│ {} │", parts.join(" │ ")));
            }
            if ri + 1 < token.rows.len() {
                lines.push(sep.clone());
            }
        }
        lines.push(bar("└", "┴", "┘"));
        if not_space(next) {
            lines.push(String::new());
        }
        lines
    }
}
