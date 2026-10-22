//! LaTeX math to terminal Unicode text: a port of upstream `latex.ts` (`renderLatex`).
//!
//! The parser walks Unicode scalar values where upstream walks UTF-16 code units; the two agree
//! except where upstream would split a surrogate pair (a lone astral character as a bare script or
//! argument, e.g. `x^😀`).

mod format;
mod tables;

use format::*;
use crate::utils::visible_width;

/// Upstream `renderLatex(source, { display })`: `None` when the expression holds unsupported or
/// malformed syntax.
pub fn render_latex(source: &str, display: bool) -> Option<String> {
    let mut nodes = Vec::new();
    let rendered = Parser::new(source, &mut nodes, display).render()?;
    let protected = PROTECTED_SPACE.to_string();
    if nodes.is_empty() {
        return Some(rendered.replace(&protected, " "));
    }
    let lines = render_layout(&rendered, &nodes).lines;
    let indentation = lines
        .iter()
        .filter(|l| !trim(l).is_empty())
        .map(|l| l.chars().count() - trim_start(l).chars().count())
        .min();
    let cut: Vec<&str> = lines
        .iter()
        .map(|l| match indentation {
            Some(n) => trim_end(l.char_indices().nth(n).map_or("", |(i, _)| &l[i..])),
            None => "",
        })
        .collect();
    Some(trim_end(&cut.join("\n")).replace(&protected, " "))
}

struct Parser<'a> {
    src: Vec<char>,
    nodes: &'a mut Vec<Node>,
    display: bool,
    pos: usize,
    supported: bool,
    stack_fractions: bool,
    script_depth: usize,
}

fn chars_str(c: &[char]) -> String {
    c.iter().collect()
}

fn find_from(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    (from..hay.len().saturating_sub(needle.len() - 1)).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// `body.split(/\\\\(?:\[[^\]\n]*\])?/)`.
fn split_environment_rows(body: &str) -> Vec<String> {
    let c: Vec<char> = body.chars().collect();
    let (mut rows, mut cur, mut i) = (Vec::new(), String::new(), 0);
    while i < c.len() {
        if c[i] == '\\' && c.get(i + 1) == Some(&'\\') {
            i += 2;
            if c.get(i) == Some(&'[') {
                let close = c[i + 1..].iter().position(|&x| x == ']' || x == '\n');
                if let Some(off) = close.filter(|&off| c[i + 1 + off] == ']') {
                    i += off + 2;
                }
            }
            rows.push(std::mem::take(&mut cur));
        } else {
            cur.push(c[i]);
            i += 1;
        }
    }
    rows.push(cur);
    rows
}

/// `value.replace(/,\s*$/, "")`.
fn strip_trailing_comma(value: &str) -> &str {
    let t = trim_end(value);
    match t.strip_suffix(',') {
        Some(s) => s,
        None => value,
    }
}

/// `/^\s*\{[^}]*\}/` removed once.
fn strip_column_spec(body: &str) -> &str {
    let t = trim_start(body);
    if let Some(rest) = t.strip_prefix('{') {
        if let Some(end) = rest.find('}') {
            return &rest[end + 1..];
        }
    }
    body
}

/// `/^(?:if|when|for|otherwise)\b/i`.
fn has_condition_word(s: &str) -> bool {
    ["if", "when", "for", "otherwise"].iter().any(|w| {
        s.len() >= w.len()
            && s.is_char_boundary(w.len())
            && s[..w.len()].eq_ignore_ascii_case(w)
            && !s[w.len()..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

impl<'a> Parser<'a> {
    fn new(source: &str, nodes: &'a mut Vec<Node>, display: bool) -> Self {
        Parser { src: source.chars().collect(), nodes, display, pos: 0, supported: true, stack_fractions: true, script_depth: 0 }
    }

    fn at(&self, i: usize) -> Option<char> {
        self.src.get(i).copied()
    }

    fn push_node(&mut self, node: Node) -> String {
        self.nodes.push(node);
        marker(self.nodes.len() - 1)
    }

    fn render(mut self) -> Option<String> {
        let rendered = self.parse_sequence(false);
        if !self.supported || self.pos != self.src.len() {
            return None;
        }
        Some(normalize_output(&rendered))
    }

    fn skip(&mut self, pred: fn(char) -> bool) {
        while self.at(self.pos).is_some_and(pred) {
            self.pos += 1;
        }
    }

    fn parse_sequence(&mut self, until_brace: bool) -> String {
        let mut result = String::new();
        while let Some(c) = self.at(self.pos) {
            if until_brace && c == '}' {
                self.pos += 1;
                return result;
            }
            if c == '}' {
                self.supported = false;
                return result;
            }
            if c == '{' {
                self.pos += 1;
                result += &self.parse_sequence(true);
                continue;
            }
            if c == '\\' {
                let command = self.parse_command();
                if command == NEGATIVE_SPACE {
                    result.truncate(trim_end(&result).len());
                    if result.ends_with(NAMED_OPERATOR_END) {
                        result.pop();
                    }
                } else {
                    result += &command;
                }
                continue;
            }
            if c == '^' || c == '_' {
                self.pos += 1;
                result.truncate(trim_end(&result).len());
                let script = self.parse_scripts(c);
                if result.ends_with(NAMED_OPERATOR_END) {
                    result.pop();
                    result += &script;
                    result.push(NAMED_OPERATOR_END);
                } else {
                    result += &script;
                }
                continue;
            }
            if is_space(c) {
                self.skip(is_space);
                result.push(' ');
                continue;
            }
            if matches!(c, '=' | '<' | '>') {
                result = format!("{} {c} ", trim_end(&result));
                self.pos += 1;
                continue;
            }
            if c == '&' {
                self.pos += 1;
                continue;
            }
            if c == '~' {
                self.pos += 1;
                result.push(' ');
                continue;
            }
            if c == '.' {
                if let Some(Node::Matrix { lines, .. }) = trailing_marker(&result).and_then(|i| self.nodes.get_mut(i)) {
                    if let Some(last) = lines.last_mut() {
                        last.push('.');
                    }
                    self.pos += 1;
                    continue;
                }
            }
            result.push(c);
            self.pos += 1;
        }
        if until_brace {
            self.supported = false;
        }
        result
    }

    fn parse_scripts(&mut self, initial: char) -> String {
        let (mut sub, mut sup): (Option<String>, Option<String>) = (None, None);
        let mut order = Vec::new();
        let mut parse = |p: &mut Self, marker: char| {
            p.script_depth += 1;
            let value = p.parse_required_argument(false);
            p.script_depth -= 1;
            let kind = if marker == '_' { ScriptKind::Sub } else { ScriptKind::Sup };
            if kind == ScriptKind::Sub { sub = Some(value) } else { sup = Some(value) }
            order.push(kind);
        };
        parse(self, initial);
        let mut next = self.pos;
        while self.at(next).is_some_and(is_space) {
            next += 1;
        }
        if let Some(m) = self.at(next).filter(|&m| (m == '^' || m == '_') && m != initial) {
            self.pos = next + 1;
            parse(self, m);
        }
        let sub_unicode = sub.as_deref().and_then(|s| format_unicode_script(s, ScriptKind::Sub));
        let sup_unicode = sup.as_deref().and_then(|s| format_unicode_script(s, ScriptKind::Sup));
        let can_use_layout = ![&sub, &sup].iter().any(|v| {
            v.as_deref().is_some_and(|v| {
                v.contains('/')
                    || (!v.contains(LAYOUT_MARKER_START)
                        && v.chars().count() > 1
                        && !v.chars().any(|c| c.is_ascii_uppercase() || c == '*' || c == '∗'))
            })
        });
        let needs_layout = self.display
            && can_use_layout
            && (self.script_depth > 0 || (sub.is_some() && sub_unicode.is_none()) || (sup.is_some() && sup_unicode.is_none()));
        if !needs_layout {
            return order
                .iter()
                .map(|&k| match k {
                    ScriptKind::Sub => sub_unicode.clone().unwrap_or_else(|| format_script(sub.as_deref().unwrap_or(""), k)),
                    ScriptKind::Sup => sup_unicode.clone().unwrap_or_else(|| format_script(sup.as_deref().unwrap_or(""), k)),
                })
                .collect();
        }
        let node = Node::Script { lower: sub.as_deref().map(normalize_output), upper: sup.as_deref().map(normalize_output) };
        self.push_node(node)
    }
}

fn is_space_or_tab(c: char) -> bool {
    c == ' ' || c == '\t'
}

impl Parser<'_> {
    fn parse_command(&mut self) -> String {
        self.pos += 1;
        let Some(first) = self.at(self.pos) else {
            self.supported = false;
            return String::new();
        };
        if first == '\n' || first == '\r' {
            self.pos += 1;
            if first == '\r' && self.at(self.pos) == Some('\n') {
                self.pos += 1;
            }
            return " ".into();
        }
        let command = if first.is_ascii_alphabetic() {
            let start = self.pos;
            self.skip(|c| c.is_ascii_alphabetic());
            chars_str(&self.src[start..self.pos])
        } else {
            self.pos += 1;
            first.to_string()
        };
        let cmd = command.as_str();
        if cmd == "\\" {
            return "\n".into();
        }
        if tables::is_spacing_command(cmd) {
            return " ".into();
        }
        if tables::is_negative_spacing_command(cmd) {
            return NEGATIVE_SPACE.into();
        }
        if tables::is_font_switch_command(cmd) {
            self.skip(is_space);
            return String::new();
        }
        if tables::is_ignored_command(cmd) {
            return String::new();
        }
        if matches!(cmd, "{" | "}" | "$" | "%" | "#" | "_" | "&") {
            return command;
        }
        if cmd == "|" {
            return "‖".into();
        }
        if cmd == "not" {
            let arg = self.parse_required_argument(false);
            let value = trim(&arg);
            if let Some(negated) = tables::negated_symbol(value) {
                return format!(" {negated} ");
            }
            let mut chars = value.chars();
            let Some(head) = chars.next() else {
                self.supported = false;
                return String::new();
            };
            return format!(" {head}\u{0338}{} ", chars.as_str());
        }
        if tables::is_limit_operator(cmd) {
            return self.parse_operator(command.clone(), true, true, true);
        }
        if let Some(symbol) = tables::symbol(cmd) {
            if tables::is_display_limit_symbol(cmd) {
                return self.parse_operator(symbol.to_string(), false, true, false);
            }
            let spaced = cmd == "cdot" || cmd == "times" || tables::is_relation_command(cmd);
            return if spaced { format!(" {symbol} ") } else { symbol.to_string() };
        }
        if tables::is_named_operator(cmd) {
            return format!("{NAMED_OPERATOR_START}{cmd}{NAMED_OPERATOR_END}");
        }
        if tables::is_size_command(cmd) {
            return String::new();
        }
        if matches!(cmd, "left" | "middle" | "right") {
            if self.at(self.pos) == Some('.') {
                self.pos += 1;
            }
            return String::new();
        }
        if matches!(cmd, "frac" | "dfrac" | "tfrac") {
            let stack = self.display && self.stack_fractions && cmd != "tfrac";
            let numerator = self.parse_required_argument(!stack);
            let denominator = self.parse_required_argument(!stack);
            if stack {
                let node = Node::Fraction { numerator: normalize_output(&numerator), denominator: normalize_output(&denominator) };
                return self.push_node(node);
            }
            return format_fraction(&numerator, &denominator);
        }
        if cmd == "sqrt" {
            let degree = self.parse_optional_argument().map(|d| trim(&d).to_string());
            let value = self.parse_required_argument(true);
            return match degree.as_deref() {
                None | Some("2") => format_root(&value, "√"),
                Some("3") => format_root(&value, "∛"),
                Some("4") => format_root(&value, "∜"),
                Some(d) => format!("{}{}", format_script(d, ScriptKind::Sup), format_root(&value, "√")),
            };
        }
        if cmd == "boxed" || cmd == "fbox" {
            return format!("[{}]", trim(&self.parse_required_argument(true)));
        }
        if matches!(cmd, "binom" | "dbinom" | "tbinom") {
            let a = self.parse_required_argument(true);
            let b = self.parse_required_argument(true);
            return format!("({a} choose {b})");
        }
        if let Some(accent) = tables::accent(cmd) {
            let value = self.parse_required_argument(true);
            return if value.chars().count() == 1 { format!("{value}{accent}") } else { format!("{cmd}({value})") };
        }
        if cmd == "mathbb" {
            let value = self.parse_required_argument(true);
            let mut buf = [0u8; 4];
            return value.chars().map(|c| tables::blackboard(c.encode_utf8(&mut buf)).map_or(c.to_string(), str::to_string)).collect();
        }
        if cmd == "operatorname" {
            let starred = self.at(self.pos) == Some('*');
            if starred {
                self.pos += 1;
            }
            let arg = self.parse_required_argument(true);
            let operator = trim(&normalize_output(&arg)).to_string();
            return self.parse_operator(operator, true, starred, true);
        }
        if cmd == "mod" || cmd == "bmod" {
            return " mod ".into();
        }
        if cmd == "pmod" || cmd == "pod" {
            let arg = self.parse_required_argument(true);
            let value = trim(&arg);
            return if cmd == "pmod" { format!(" (mod {value})") } else { format!(" ({value})") };
        }
        if cmd == "overset" || cmd == "stackrel" {
            let upper = self.parse_required_argument(true);
            let value = self.parse_required_argument(true);
            return format!("{}{}", trim(&value), format_script(&upper, ScriptKind::Sup));
        }
        if cmd == "underset" {
            let lower = self.parse_required_argument(true);
            let value = self.parse_required_argument(true);
            return format!("{}{}", trim(&value), format_script(&lower, ScriptKind::Sub));
        }
        if tables::is_plain_wrapper(cmd) {
            let value = self.parse_required_argument(true);
            return if cmd.starts_with("text") || cmd == "mbox" { value } else { trim(&value).to_string() };
        }
        if cmd == "begin" {
            return self.parse_environment();
        }
        if cmd == "end" {
            self.supported = false;
            return String::new();
        }
        self.supported = false;
        format!("\\{cmd}")
    }

    /// `bracket_lower`: inline lower limit as `op[lower]` instead of a subscript.
    fn parse_operator(&mut self, operator: String, bracket_lower: bool, display_limits: bool, spaced: bool) -> String {
        let mut use_display_limits = display_limits;
        let mut modifier_pos = self.pos;
        while self.at(modifier_pos).is_some_and(is_space_or_tab) {
            modifier_pos += 1;
        }
        for (name, limits) in [("limits", true), ("nolimits", false)] {
            let pattern: Vec<char> = format!("\\{name}").chars().collect();
            let end = modifier_pos + pattern.len();
            if self.src.get(modifier_pos..end) == Some(&pattern[..]) && !self.at(end).is_some_and(|c| c.is_ascii_alphabetic()) {
                use_display_limits = limits;
                self.pos = end;
                break;
            }
        }
        let (mut lower, mut upper): (Option<String>, Option<String>) = (None, None);
        loop {
            let mut script_pos = self.pos;
            while self.at(script_pos).is_some_and(is_space_or_tab) {
                script_pos += 1;
            }
            let kind = match self.at(script_pos) {
                Some(k @ ('_' | '^')) => k,
                _ => break,
            };
            self.pos = script_pos + 1;
            let value = normalize_output(&self.parse_required_argument(false)).replace(' ', "");
            let slot = if kind == '_' { &mut lower } else { &mut upper };
            if slot.is_some() {
                self.supported = false;
            }
            *slot = Some(value);
        }
        if self.display && use_display_limits && (lower.is_some() || upper.is_some()) {
            return self.push_node(Node::Operator { operator, lower, upper });
        }
        let mut rendered = operator;
        if let Some(l) = &lower {
            rendered += &if bracket_lower { format!("[{l}]") } else { format_script(l, ScriptKind::Sub) };
        }
        if let Some(u) = &upper {
            rendered += &format_script(u, ScriptKind::Sup);
        }
        if spaced { format!(" {rendered} ") } else { rendered }
    }

    fn parse_required_argument(&mut self, stack_fractions: bool) -> String {
        let previous = self.stack_fractions;
        self.stack_fractions = previous && stack_fractions;
        let value = self.parse_required_argument_value();
        self.stack_fractions = previous;
        value
    }

    fn parse_required_argument_value(&mut self) -> String {
        self.skip(is_space);
        let Some(c) = self.at(self.pos) else {
            self.supported = false;
            return String::new();
        };
        if c == '{' {
            self.pos += 1;
            return self.parse_sequence(true);
        }
        if c == '\\' {
            return self.parse_command();
        }
        self.pos += 1;
        c.to_string()
    }

    fn parse_optional_argument(&mut self) -> Option<String> {
        self.skip(is_space_or_tab);
        if self.at(self.pos) != Some('[') {
            return None;
        }
        let Some(end) = find_from(&self.src, &[']'], self.pos + 1) else {
            self.supported = false;
            return None;
        };
        let value = chars_str(&self.src[self.pos + 1..end]);
        self.pos = end + 1;
        Some(self.render_nested(&value, true))
    }
}

impl Parser<'_> {
    fn read_raw_group(&mut self) -> Option<String> {
        self.skip(is_space_or_tab);
        if self.at(self.pos) != Some('{') {
            self.supported = false;
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1;
        while let Some(c) = self.at(self.pos) {
            if c == '\\' {
                self.pos += 2;
                continue;
            }
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
            }
            if depth == 0 {
                let value = chars_str(&self.src[start..self.pos]);
                self.pos += 1;
                return Some(value);
            }
            self.pos += 1;
        }
        self.supported = false;
        None
    }

    fn parse_environment(&mut self) -> String {
        let environment = match self.read_raw_group() {
            Some(e) if !e.is_empty() => e,
            _ => return String::new(),
        };
        let end_marker: Vec<char> = format!("\\end{{{environment}}}").chars().collect();
        let Some(end) = find_from(&self.src, &end_marker, self.pos) else {
            self.supported = false;
            return String::new();
        };
        let body = chars_str(&self.src[self.pos..end]);
        self.pos = end + end_marker.len();
        let env = environment.as_str();

        if matches!(env, "equation" | "equation*" | "displaymath") {
            return trim(&self.render_nested(&body, true)).to_string();
        }
        if matches!(
            env,
            "aligned" | "align" | "align*" | "alignedat" | "alignat" | "alignat*" | "gather" | "gathered" | "multline" | "multline*" | "split"
        ) {
            let aligned_at = matches!(env, "alignedat" | "alignat" | "alignat*");
            let aligned_body = if aligned_at { strip_column_spec(&body) } else { &body };
            let mut rows = Vec::new();
            for row in split_environment_rows(aligned_body) {
                let cells: Vec<&str> = row.split('&').collect();
                let source = if aligned_at { cells.chunks(2).map(|c| c.concat()).collect::<Vec<_>>().join(" ") } else { cells.concat() };
                let rendered = trim(&self.render_nested(&source, true)).to_string();
                if !rendered.is_empty() {
                    rows.push(rendered);
                }
            }
            return rows.join("\n");
        }
        if env == "cases" || env == "cases*" {
            return self.render_cases(&body);
        }
        if matches!(env, "array" | "matrix" | "smallmatrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix") {
            let matrix_body = if env == "array" { strip_column_spec(&body) } else { &body };
            return self.render_matrix(env, matrix_body);
        }
        self.supported = false;
        body
    }

    fn render_cells(&mut self, body: &str) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for row in split_environment_rows(body) {
            let cells: Vec<String> = row.split('&').map(|cell| trim(&self.render_nested(cell, false)).to_string()).collect();
            if cells.iter().any(|c| !c.is_empty()) {
                out.push(cells);
            }
        }
        out
    }

    fn render_cases(&mut self, body: &str) -> String {
        let rows = self.render_cells(body);
        let first = |row: &Vec<String>| strip_trailing_comma(row.first().map_or("", String::as_str)).to_string();
        let value_width = rows.iter().map(|r| visible_width(&first(r))).max().unwrap_or(0);
        let contents: Vec<String> = rows
            .iter()
            .map(|row| {
                let value = first(row);
                let condition = row.get(1).map_or("", String::as_str);
                if condition.is_empty() {
                    return value;
                }
                let prefix = if has_condition_word(condition) { " " } else { " if " };
                let pad = PROTECTED_SPACE.to_string().repeat(value_width.saturating_sub(visible_width(&value)));
                format!("{value}{pad}{prefix}{condition}")
            })
            .collect();
        if contents.len() <= 1 {
            return contents.first().map_or(String::new(), |c| format!("⎧ {c}"));
        }
        let middle = contents.len() / 2;
        let mut visual: Vec<Option<&String>> = contents.iter().map(Some).collect();
        if contents.len() % 2 == 0 {
            visual.insert(middle, None);
        }
        let n = visual.len();
        let lines = visual
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let delimiter = if i == 0 { "⎧" } else if i == n - 1 { "⎩" } else { "⎨" };
                match content {
                    None => delimiter.to_string(),
                    Some(c) => format!("{delimiter} {c}"),
                }
            })
            .collect();
        self.push_node(Node::Matrix { lines, baseline: middle })
    }

    fn render_matrix(&mut self, environment: &str, body: &str) -> String {
        let matrix = self.render_cells(body);
        let column_count = matrix.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> =
            (0..column_count).map(|c| matrix.iter().map(|r| r.get(c).map_or(0, |s| visible_width(s))).max().unwrap_or(0)).collect();
        let rows: Vec<String> = matrix
            .iter()
            .map(|row| {
                (0..column_count)
                    .map(|c| {
                        let cell = row.get(c).map_or("", String::as_str);
                        format!("{cell}{}", PROTECTED_SPACE.to_string().repeat(widths[c].saturating_sub(visible_width(cell))))
                    })
                    .collect::<Vec<_>>()
                    .join(" │ ")
            })
            .collect();
        let lines: Vec<String> = if matches!(environment, "array" | "matrix" | "smallmatrix") {
            rows
        } else {
            let d: [&str; 6] = match environment {
                "pmatrix" => ["⎛", "⎞", "⎜", "⎟", "⎝", "⎠"],
                "bmatrix" => ["⎡", "⎤", "⎢", "⎥", "⎣", "⎦"],
                "Bmatrix" => ["⎧", "⎫", "⎨", "⎬", "⎩", "⎭"],
                "vmatrix" => ["│"; 6],
                "Vmatrix" => ["║"; 6],
                _ => {
                    self.supported = false;
                    return rows.join("\n");
                }
            };
            let n = rows.len();
            rows.iter()
                .enumerate()
                .map(|(i, row)| {
                    let (l, r) = if i == 0 { (d[0], d[1]) } else if i == n - 1 { (d[4], d[5]) } else { (d[2], d[3]) };
                    format!("{l} {row} {r}")
                })
                .collect()
        };
        if lines.len() <= 1 {
            return lines.into_iter().next().unwrap_or_default();
        }
        self.push_node(Node::Matrix { lines, baseline: 0 })
    }

    fn render_nested(&mut self, source: &str, stack_fractions: bool) -> String {
        let display = self.display && stack_fractions;
        match Parser::new(source, self.nodes, display).render() {
            Some(rendered) => rendered,
            None => {
                self.supported = false;
                source.to_string()
            }
        }
    }
}
