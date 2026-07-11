//! The highlight.js 10.7.3 parse loop (`core.js` `_highlight`), the
//! TokenTree/HTML emitter collapsed to direct string emission (every
//! `openNode` has a kind, so the tree adds nothing), `highlightAuto`, and
//! the runtime halves of the named grammar callbacks.
//!
//! SAFE_MODE semantics are preserved: any engine failure that the original
//! would swallow (0-width-match stall, iteration guard, popping past the
//! root) yields the escaped source with relevance 0 instead of an error.

use std::collections::HashMap;
use std::rc::Rc;

use super::HljsError;
use super::grammar::{Callback, JsRegex, MatcherHit, Mode, Registry, RuleKind, SubLanguage};

/// A parse-stack frame (`Object.create(mode, { parent: { value: top } })`).
pub(crate) struct Frame {
    mode: usize,
    parent: Option<Rc<Frame>>,
}

/// Result of one `_highlight` run (`HighlightResult` fields Pi's coding
/// agent can observe).
pub(crate) struct Inner {
    pub relevance: f64,
    pub value: String,
    pub illegal: bool,
    pub language: Option<String>,
    top: Option<Rc<Frame>>,
}

/// Shared state for one top-level highlight call: the compiled grammars are
/// global and immutable, so the mutations the original applies to them
/// (`endSameAsBegin` end regexes, `Response.data`, matcher `regexIndex`)
/// live here, keyed by `(language, mode)`.
pub(crate) struct Ctx {
    registry: &'static Registry,
    regex_index: HashMap<(usize, usize), usize>,
    end_override: HashMap<(usize, usize), std::sync::Arc<JsRegex>>,
    begin_match: HashMap<(usize, usize), Option<String>>,
}

impl Ctx {
    pub(crate) fn new(registry: &'static Registry) -> Ctx {
        Ctx {
            registry,
            regex_index: HashMap::new(),
            end_override: HashMap::new(),
            begin_match: HashMap::new(),
        }
    }
}

enum Abort {
    /// `Illegal lexeme` (only raised when `ignoreIllegals` is off).
    Illegal,
    /// Anything SAFE_MODE would swallow.
    Safe,
}

pub(crate) fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// TokenTreeEmitter + HTMLRenderer, fused.
struct Emitter {
    buffer: String,
    depth: usize,
}

impl Emitter {
    fn new() -> Emitter {
        Emitter {
            buffer: String::new(),
            depth: 0,
        }
    }

    fn add_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.push_str(&escape_html(text));
    }

    fn open_node(&mut self, kind: &str) {
        self.buffer.push_str("<span class=\"hljs-");
        self.buffer.push_str(kind);
        self.buffer.push_str("\">");
        self.depth += 1;
    }

    fn close_node(&mut self) {
        if self.depth > 0 {
            self.buffer.push_str("</span>");
            self.depth -= 1;
        }
    }

    fn close_all_nodes(&mut self) {
        while self.depth > 0 {
            self.close_node();
        }
    }

    fn add_keyword(&mut self, text: &str, kind: &str) {
        if text.is_empty() {
            return;
        }
        self.open_node(kind);
        self.add_text(text);
        self.close_node();
    }

    /// `addSublanguage`: the sub-result's root is wrapped in an unprefixed
    /// span when a language name is present.
    fn add_sublanguage(&mut self, html: &str, name: Option<&str>) {
        match name {
            Some(name) => {
                self.buffer.push_str("<span class=\"");
                self.buffer.push_str(name);
                self.buffer.push_str("\">");
                self.buffer.push_str(html);
                self.buffer.push_str("</span>");
            }
            None => self.buffer.push_str(html),
        }
    }
}

fn plaintext_result(code: &str) -> Inner {
    Inner {
        relevance: 0.0,
        value: escape_html(code),
        illegal: false,
        language: None,
        top: None,
    }
}

/// `highlightAuto` (used by array `subLanguage` modes).
pub(crate) fn highlight_auto(
    ctx: &mut Ctx,
    code: &str,
    subset: Option<&[String]>,
) -> Result<Inner, HljsError> {
    let registry = ctx.registry;
    let candidates: Vec<usize> = match subset {
        Some(names) => names
            .iter()
            .filter_map(|name| registry.language(name).map(|(idx, _)| idx))
            .collect(),
        None => (0..registry.languages.len()).collect(),
    };
    let mut results = vec![plaintext_result(code)];
    for lang in candidates {
        if registry.languages[lang].disable_autodetect {
            continue;
        }
        results.push(highlight_language(ctx, lang, code, false, None)?);
    }
    results.sort_by(|a, b| {
        if a.relevance != b.relevance {
            return b
                .relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal);
        }
        if let (Some(a_name), Some(b_name)) = (&a.language, &b.language) {
            let a_superset = registry
                .language(a_name)
                .and_then(|(_, l)| l.superset_of.as_deref());
            let b_superset = registry
                .language(b_name)
                .and_then(|(_, l)| l.superset_of.as_deref());
            if a_superset == Some(b_name.as_str()) {
                return std::cmp::Ordering::Greater;
            }
            if b_superset == Some(a_name.as_str()) {
                return std::cmp::Ordering::Less;
            }
        }
        std::cmp::Ordering::Equal
    });
    results
        .into_iter()
        .next()
        .ok_or_else(|| HljsError::Catalog("empty auto-detect result set".into()))
}

/// One `_highlight` run for a known language index.
pub(crate) fn highlight_language(
    ctx: &mut Ctx,
    lang: usize,
    code: &str,
    ignore_illegals: bool,
    continuation: Option<Rc<Frame>>,
) -> Result<Inner, HljsError> {
    let mut run = Run {
        ctx,
        lang,
        code,
        ignore_illegals,
        emitter: Emitter::new(),
        mode_buffer: String::new(),
        relevance: 0.0,
        top: None,
        continuations: HashMap::new(),
        last_match: None,
        iterations: 0,
    };
    let outcome = run.run(continuation);
    let language_name = run.language_name().to_owned();
    match outcome {
        Ok(top) => Ok(Inner {
            relevance: run.relevance.floor(),
            value: run.emitter.buffer,
            illegal: false,
            language: Some(language_name),
            top,
        }),
        Err(Abort::Illegal) => Ok(Inner {
            relevance: 0.0,
            value: escape_html(code),
            illegal: true,
            language: None,
            top: None,
        }),
        Err(Abort::Safe) => Ok(Inner {
            relevance: 0.0,
            value: escape_html(code),
            illegal: false,
            language: Some(language_name),
            top: None,
        }),
    }
}

struct Run<'c> {
    ctx: &'c mut Ctx,
    lang: usize,
    code: &'c str,
    ignore_illegals: bool,
    emitter: Emitter,
    mode_buffer: String,
    relevance: f64,
    top: Option<Rc<Frame>>,
    continuations: HashMap<String, Rc<Frame>>,
    last_match: Option<(RuleKind, usize)>,
    iterations: usize,
}

impl<'c> Run<'c> {
    fn language_name(&self) -> &str {
        &self.ctx.registry.languages[self.lang].name
    }

    fn mode(&self, index: usize) -> &'static Mode {
        let registry: &'static Registry = self.ctx.registry;
        &registry.languages[self.lang].modes[index]
    }

    fn top_frame(&self) -> Result<&Rc<Frame>, Abort> {
        self.top.as_ref().ok_or(Abort::Safe)
    }

    fn top_mode(&self) -> Result<&'static Mode, Abort> {
        Ok(self.mode(self.top_frame()?.mode))
    }

    fn class_alias(&self, kind: &str) -> String {
        match self.ctx.registry.languages[self.lang]
            .class_name_aliases
            .get(kind)
        {
            Some(alias) if !alias.is_empty() => alias.clone(),
            _ => kind.to_owned(),
        }
    }

    // -- keywords ----------------------------------------------------------

    fn process_keywords(&mut self) -> Result<(), Abort> {
        let mode = self.top_mode()?;
        let source = self.mode_buffer.clone();
        let (Some(keywords), Some(pattern)) = (&mode.keywords, &mode.keyword_pattern) else {
            self.emitter.add_text(&source);
            return Ok(());
        };
        let case_insensitive = self.ctx.registry.languages[self.lang].case_insensitive;
        let mut last_index = 0usize;
        let mut buf = String::new();
        loop {
            let hit = pattern.captures_at(&source, last_index).map_err(safe)?;
            let Some(caps) = hit else { break };
            let Some(whole) = caps.get(0) else { break };
            buf.push_str(&source[last_index..whole.start()]);
            let text = whole.as_str();
            let match_text = if case_insensitive {
                text.to_lowercase()
            } else {
                text.to_owned()
            };
            match keywords.get(&match_text) {
                Some((kind, keyword_relevance)) => {
                    self.emitter.add_text(&buf);
                    buf.clear();
                    self.relevance += keyword_relevance;
                    if kind.starts_with('_') {
                        // Relevance only; no highlighting.
                        buf.push_str(text);
                    } else {
                        let css = self.class_alias(kind);
                        self.emitter.add_keyword(text, &css);
                    }
                }
                None => buf.push_str(text),
            }
            if whole.end() == whole.start() {
                // A zero-width keyword match would hang the JS engine too;
                // bail rather than loop forever.
                break;
            }
            last_index = whole.end();
        }
        buf.push_str(&source[last_index.min(source.len())..]);
        self.emitter.add_text(&buf);
        Ok(())
    }

    fn process_sub_language(&mut self) -> Result<(), Abort> {
        if self.mode_buffer.is_empty() {
            return Ok(());
        }
        let mode = self.top_mode()?;
        let buffer = self.mode_buffer.clone();
        let result = match &mode.sub_language {
            SubLanguage::Name(name) => {
                // `languages[top.subLanguage]` — the exact registry key,
                // aliases not consulted.
                let Some(lang) = self.ctx.registry.canonical(name) else {
                    self.emitter.add_text(&buffer);
                    return Ok(());
                };
                let continuation = self.continuations.get(name).cloned();
                let result = highlight_language(self.ctx, lang, &buffer, true, continuation)
                    .map_err(safe)?;
                if let Some(top) = &result.top {
                    self.continuations.insert(name.clone(), Rc::clone(top));
                }
                result
            }
            SubLanguage::Auto(list) => {
                let subset = if list.is_empty() {
                    None
                } else {
                    Some(list.as_slice())
                };
                highlight_auto(self.ctx, &buffer, subset).map_err(safe)?
            }
            SubLanguage::None => return Ok(()),
        };
        if mode.relevance > 0.0 {
            self.relevance += result.relevance;
        }
        self.emitter
            .add_sublanguage(&result.value, result.language.as_deref());
        Ok(())
    }

    fn process_buffer(&mut self) -> Result<(), Abort> {
        let mode = self.top_mode()?;
        if !matches!(mode.sub_language, SubLanguage::None) {
            self.process_sub_language()?;
        } else {
            self.process_keywords()?;
        }
        self.mode_buffer.clear();
        Ok(())
    }

    // -- mode transitions ---------------------------------------------------

    fn start_new_mode(&mut self, mode_index: usize) {
        let mode = self.mode(mode_index);
        if let Some(class_name) = &mode.class_name {
            let kind = self.class_alias(class_name);
            self.emitter.open_node(&kind);
        }
        self.top = Some(Rc::new(Frame {
            mode: mode_index,
            parent: self.top.take(),
        }));
    }

    fn end_re(&self, mode_index: usize) -> Option<std::sync::Arc<JsRegex>> {
        self.ctx
            .end_override
            .get(&(self.lang, mode_index))
            .cloned()
            .or_else(|| self.mode(mode_index).end_re.clone())
    }

    fn end_of_mode(
        &mut self,
        frame: &Rc<Frame>,
        hit: &MatcherHit,
        rest: &str,
    ) -> Result<Option<Rc<Frame>>, Abort> {
        let mode = self.mode(frame.mode);
        let mut matched = match self.end_re(frame.mode) {
            Some(re) => match re.find(rest).map_err(safe)? {
                Some((start, _)) => start == 0,
                None => false,
            },
            None => false,
        };
        if matched {
            if let Some(callback) = mode.on_end
                && self.run_callback(callback, frame.mode, hit)
            {
                matched = false;
            }
            if matched {
                let mut target = Rc::clone(frame);
                while self.mode(target.mode).ends_parent {
                    let Some(parent) = target.parent.clone() else {
                        break;
                    };
                    target = parent;
                }
                return Ok(Some(target));
            }
        }
        if mode.ends_with_parent
            && let Some(parent) = frame.parent.clone()
        {
            return self.end_of_mode(&parent, hit, rest);
        }
        Ok(None)
    }

    // -- callbacks ----------------------------------------------------------

    /// Returns true when the callback called `ignoreMatch()`.
    fn run_callback(&mut self, callback: Callback, mode_index: usize, hit: &MatcherHit) -> bool {
        let lexeme = hit.groups.first().and_then(|g| g.as_deref()).unwrap_or("");
        match callback {
            Callback::Shebang => hit.index != 0,
            Callback::EndSameAsBeginBegin => {
                let value = hit.groups.get(1).cloned().flatten();
                self.ctx.begin_match.insert((self.lang, mode_index), value);
                false
            }
            Callback::EndSameAsBeginEnd => {
                let stored = self
                    .ctx
                    .begin_match
                    .get(&(self.lang, mode_index))
                    .cloned()
                    .unwrap_or(None);
                let current = hit.groups.get(1).cloned().flatten();
                stored != current
            }
            Callback::IsTrulyOpeningTag => {
                let after = hit.index + lexeme.len();
                match self.code.as_bytes().get(after) {
                    // Nested type, e.g. `<Array<Array<number>>` — not a tag.
                    Some(b'<') => true,
                    Some(b'>') => {
                        // `<something>` without a matching closing tag is
                        // ignored.
                        let tag = format!("</{}", &lexeme[1.min(lexeme.len())..]);
                        !self.code[after.min(self.code.len())..].contains(&tag)
                    }
                    _ => false,
                }
            }
        }
    }

    fn skip_if_preceding_dot(&self, hit: &MatcherHit) -> bool {
        hit.index > 0 && self.code.as_bytes().get(hit.index - 1) == Some(&b'.')
    }

    // -- lexeme handling ----------------------------------------------------

    fn do_ignore(&mut self, lexeme: &str) -> Result<usize, Abort> {
        let key = (self.lang, self.top_frame()?.mode);
        let regex_index = self.ctx.regex_index.get(&key).copied().unwrap_or(0);
        if regex_index == 0 {
            // No more regexes to try here: move the cursor forward one unit.
            match lexeme.chars().next() {
                Some(ch) => {
                    self.mode_buffer.push(ch);
                    Ok(ch.len_utf8())
                }
                None => Ok(1),
            }
        } else {
            // Retry the remaining rules at this very position.
            Ok(0)
        }
    }

    fn do_begin_match(&mut self, hit: &MatcherHit) -> Result<(usize, bool), Abort> {
        let lexeme = hit
            .groups
            .first()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        let Some(new_mode_index) = self.mode(self.top_frame()?.mode).rules[hit.rule].mode else {
            return Err(Abort::Safe);
        };
        let new_mode = self.mode(new_mode_index);

        // First the internal before:begin callback, then the public one.
        if new_mode.before_begin_dot && self.skip_if_preceding_dot(hit) {
            let advance = self.do_ignore(&lexeme)?;
            return Ok((advance, advance == 0));
        }
        if let Some(callback) = new_mode.on_begin
            && self.run_callback(callback, new_mode_index, hit)
        {
            let advance = self.do_ignore(&lexeme)?;
            return Ok((advance, advance == 0));
        }

        if new_mode.end_same_as_begin {
            let literal = JsRegex::literal(&lexeme).map_err(safe)?;
            self.ctx
                .end_override
                .insert((self.lang, new_mode_index), std::sync::Arc::new(literal));
        }

        if new_mode.skip {
            self.mode_buffer.push_str(&lexeme);
        } else {
            if new_mode.exclude_begin {
                self.mode_buffer.push_str(&lexeme);
            }
            self.process_buffer()?;
            if !new_mode.return_begin && !new_mode.exclude_begin {
                self.mode_buffer = lexeme.clone();
            }
        }
        self.start_new_mode(new_mode_index);
        let advance = if new_mode.return_begin {
            0
        } else {
            lexeme.len()
        };
        Ok((advance, false))
    }

    /// `doEndMatch`; `None` is the NO_MATCH sentinel.
    fn do_end_match(&mut self, hit: &MatcherHit) -> Result<Option<usize>, Abort> {
        let lexeme = hit
            .groups
            .first()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        let rest = &self.code[hit.index.min(self.code.len())..];

        let top = Rc::clone(self.top_frame()?);
        let Some(end_frame) = self.end_of_mode(&top, hit, rest)? else {
            return Ok(None);
        };

        let origin_mode = self.mode(top.mode);
        if origin_mode.skip {
            self.mode_buffer.push_str(&lexeme);
        } else {
            if !(origin_mode.return_end || origin_mode.exclude_end) {
                self.mode_buffer.push_str(&lexeme);
            }
            self.process_buffer()?;
            if origin_mode.exclude_end {
                self.mode_buffer = lexeme.clone();
            }
        }

        let target = end_frame.parent.clone();
        loop {
            let frame = Rc::clone(self.top_frame()?);
            let mode = self.mode(frame.mode);
            if mode.class_name.is_some() {
                self.emitter.close_node();
            }
            if !mode.skip && matches!(mode.sub_language, SubLanguage::None) {
                self.relevance += mode.relevance;
            }
            self.top = frame.parent.clone();
            let done = match (&self.top, &target) {
                (None, None) => true,
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                _ => false,
            };
            if done {
                break;
            }
        }

        if let Some(starts) = self.mode(end_frame.mode).starts {
            if self.mode(end_frame.mode).end_same_as_begin
                && let Some(end_re) = self.end_re(end_frame.mode)
            {
                self.ctx.end_override.insert((self.lang, starts), end_re);
            }
            self.start_new_mode(starts);
        }
        Ok(Some(if origin_mode.return_end {
            0
        } else {
            lexeme.len()
        }))
    }

    fn process_lexeme(
        &mut self,
        text_before_match: &str,
        hit: Option<&MatcherHit>,
    ) -> Result<(usize, bool), Abort> {
        self.mode_buffer.push_str(text_before_match);
        let Some(hit) = hit else {
            self.process_buffer()?;
            return Ok((0, false));
        };
        let kind = self.mode(self.top_frame()?.mode).rules[hit.rule].kind;
        let lexeme = hit.groups.first().and_then(|g| g.as_deref()).unwrap_or("");

        // Stuck on a 0-width match: emit the skipped character and advance.
        // The original returns without updating lastMatch here.
        if let Some((RuleKind::Begin, last_index)) = self.last_match
            && kind == RuleKind::End
            && last_index == hit.index
            && lexeme.is_empty()
        {
            let rest = &self.code[hit.index.min(self.code.len())..];
            match rest.chars().next() {
                Some(ch) => {
                    self.mode_buffer.push(ch);
                    return Ok((ch.len_utf8(), false));
                }
                None => return Ok((1, false)),
            }
        }
        self.last_match = Some((kind, hit.index));

        match kind {
            RuleKind::Begin => return self.do_begin_match(hit),
            RuleKind::Illegal if !self.ignore_illegals => return Err(Abort::Illegal),
            RuleKind::End => {
                if let Some(processed) = self.do_end_match(hit)? {
                    return Ok((processed, false));
                }
            }
            _ => {}
        }

        // Illegal matching $ is a 0-width match that is not begin/end.
        if kind == RuleKind::Illegal && lexeme.is_empty() {
            let advance = next_char_boundary(self.code, hit.index) - hit.index;
            return Ok((advance, false));
        }

        if self.iterations > 100_000 && self.iterations > hit.index * 3 {
            return Err(Abort::Safe);
        }

        self.mode_buffer.push_str(lexeme);
        Ok((lexeme.len(), false))
    }

    // -- matcher driving ----------------------------------------------------

    fn consider_all(&mut self) -> Result<(), Abort> {
        let key = (self.lang, self.top_frame()?.mode);
        self.ctx.regex_index.insert(key, 0);
        Ok(())
    }

    fn matcher_exec(&mut self, index: usize) -> Result<Option<MatcherHit>, Abort> {
        let mode_index = self.top_frame()?.mode;
        let mode = self.mode(mode_index);
        let key = (self.lang, mode_index);
        let regex_index = self.ctx.regex_index.get(&key).copied().unwrap_or(0);
        let matcher = mode.matcher(regex_index).map_err(safe)?;
        let mut result = matcher.exec(mode, self.code, index).map_err(safe)?;

        if regex_index != 0 {
            let same_position = result.as_ref().map(|r| r.index == index).unwrap_or(false);
            if !same_position {
                let full = mode.matcher(0).map_err(safe)?;
                let next = next_char_boundary(self.code, index);
                result = full.exec(mode, self.code, next).map_err(safe)?;
            }
        }

        if let Some(hit) = &result {
            let advanced = regex_index + hit.position + 1;
            let new_index = if advanced == mode.begin_count {
                0
            } else {
                advanced
            };
            self.ctx.regex_index.insert(key, new_index);
        }
        Ok(result)
    }

    fn process_continuations(&mut self) {
        let mut class_names = Vec::new();
        let mut current = self.top.clone();
        while let Some(frame) = current {
            if frame.parent.is_none() {
                break;
            }
            if let Some(class_name) = &self.mode(frame.mode).class_name {
                class_names.push(class_name.clone());
            }
            current = frame.parent.clone();
        }
        for class_name in class_names.iter().rev() {
            // Raw class names: the original skips alias resolution here.
            self.emitter.open_node(class_name);
        }
    }

    fn run(&mut self, continuation: Option<Rc<Frame>>) -> Result<Option<Rc<Frame>>, Abort> {
        let root = self.ctx.registry.languages[self.lang].root;
        self.top = Some(continuation.unwrap_or_else(|| {
            Rc::new(Frame {
                mode: root,
                parent: None,
            })
        }));
        self.process_continuations();

        let mut index = 0usize;
        let mut resume_scan_at_same_position = false;
        loop {
            self.iterations += 1;
            if !resume_scan_at_same_position {
                self.consider_all()?;
            }
            let Some(hit) = self.matcher_exec(index)? else {
                break;
            };
            let before = &self.code[index.min(self.code.len())..hit.index.min(self.code.len())];
            let before = before.to_owned();
            let (processed, resume) = self.process_lexeme(&before, Some(&hit))?;
            resume_scan_at_same_position = resume;
            index = hit.index + processed;
        }
        let tail = self.code[index.min(self.code.len())..].to_owned();
        self.process_lexeme(&tail, None)?;
        self.emitter.close_all_nodes();
        Ok(self.top.clone())
    }
}

fn safe<E>(_: E) -> Abort {
    Abort::Safe
}

fn next_char_boundary(s: &str, index: usize) -> usize {
    let mut next = index + 1;
    while next < s.len() && !s.is_char_boundary(next) {
        next += 1;
    }
    next
}
