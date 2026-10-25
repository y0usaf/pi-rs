//! Syntax highlighting for fenced code and tool previews (upstream utils/syntax-highlight.ts over
//! highlight.js). highlight.js is a grammar per language and a large dependency; this is one small
//! tokenizer that knows comments, strings, numbers and the keywords of the languages pi-rs meets
//! most. It names highlight.js's scopes, so upstream's theme mapping colors them the same way.

/// highlight.js's scope names, as far as the pi theme colors them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Comment,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Operator,
    Punctuation,
}

#[derive(Clone, Copy)]
struct Grammar {
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    triple_quotes: bool,
    keywords: &'static [&'static str],
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16",
    "i32", "i64", "i128", "isize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option",
    "Result", "Box", "Rc", "Arc",
];

const JAVASCRIPT: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

const TYPESCRIPT: &[&str] = &[
    "any",
    "boolean",
    "declare",
    "enum",
    "implements",
    "interface",
    "keyof",
    "namespace",
    "never",
    "number",
    "private",
    "protected",
    "public",
    "readonly",
    "string",
    "type",
    "unknown",
];

const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

const BASH: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in", "local",
    "return", "select", "then", "until", "while", "export", "echo", "cd", "set", "test",
];

const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "bool",
    "byte",
    "error",
    "float64",
    "int",
    "int64",
    "rune",
    "string",
    "uint",
    "uint64",
];

const C: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "class",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "bool",
    "true",
    "false",
    "nullptr",
];

const JAVA: &[&str] = &[
    "abstract",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "throws",
    "try",
    "void",
    "while",
    "true",
    "false",
    "null",
    "String",
];

const SQL: &[&str] = &[
    "ALTER", "AND", "AS", "ASC", "BY", "CASE", "CREATE", "DELETE", "DESC", "DISTINCT", "DROP",
    "ELSE", "END", "FROM", "GROUP", "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTO", "IS",
    "JOIN", "LEFT", "LIKE", "LIMIT", "NOT", "NULL", "OFFSET", "ON", "OR", "ORDER", "OUTER",
    "PRIMARY", "RIGHT", "SELECT", "SET", "TABLE", "THEN", "UNION", "UPDATE", "VALUES", "WHEN",
    "WHERE", "alter", "and", "as", "asc", "by", "case", "create", "delete", "desc", "distinct",
    "drop", "else", "end", "from", "group", "having", "in", "index", "inner", "insert", "into",
    "is", "join", "left", "like", "limit", "not", "null", "offset", "on", "or", "order", "outer",
    "primary", "right", "select", "set", "table", "then", "union", "update", "values", "when",
    "where",
];

const CSS: &[&str] = &[
    "important",
    "media",
    "supports",
    "font-face",
    "keyframes",
    "import",
];

const RUBY: &[&str] = &[
    "alias", "and", "begin", "break", "case", "class", "def", "do", "else", "elsif", "end",
    "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "raise", "redo",
    "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until",
    "when", "while", "yield",
];

const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const YAML: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

const BACKTICK: char = '\u{60}';

/// The languages this tokenizer has a grammar for, by the names a fence or a file extension uses.
fn grammar(language: &str) -> Option<Grammar> {
    let language = language.to_ascii_lowercase();
    let lines = |markers: &'static [&'static str]| markers;
    let grammar = match language.as_str() {
        "rust" | "rs" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"'],
            triple_quotes: false,
            keywords: RUST,
        },
        "javascript" | "js" | "jsx" | "mjs" | "cjs" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', BACKTICK],
            triple_quotes: false,
            keywords: JAVASCRIPT,
        },
        "typescript" | "ts" | "tsx" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', BACKTICK],
            triple_quotes: false,
            keywords: TYPESCRIPT,
        },
        "python" | "py" => Grammar {
            line_comment: lines(&["#"]),
            block_comment: None,
            quotes: &['"', '\''],
            triple_quotes: true,
            keywords: PYTHON,
        },
        "bash" | "sh" | "shell" | "zsh" | "fish" => Grammar {
            line_comment: lines(&["#"]),
            block_comment: None,
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: BASH,
        },
        "json" | "jsonc" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: None,
            quotes: &['"'],
            triple_quotes: false,
            keywords: &[],
        },
        "go" | "golang" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\'', BACKTICK],
            triple_quotes: false,
            keywords: GO,
        },
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: C,
        },
        "java" | "kotlin" | "kt" => Grammar {
            line_comment: lines(&["//"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: JAVA,
        },
        "ruby" | "rb" => Grammar {
            line_comment: lines(&["#"]),
            block_comment: None,
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: RUBY,
        },
        "lua" => Grammar {
            line_comment: lines(&["--"]),
            block_comment: None,
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: LUA,
        },
        "sql" => Grammar {
            line_comment: lines(&["--"]),
            block_comment: Some(("/*", "*/")),
            quotes: &['\'', '"'],
            triple_quotes: false,
            keywords: SQL,
        },
        "css" | "scss" | "sass" | "less" => Grammar {
            line_comment: &[],
            block_comment: Some(("/*", "*/")),
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: CSS,
        },
        "yaml" | "yml" | "toml" | "ini" => Grammar {
            line_comment: lines(&["#"]),
            block_comment: None,
            quotes: &['"', '\''],
            triple_quotes: false,
            keywords: YAML,
        },
        _ => return None,
    };
    Some(grammar)
}

/// supportsLanguage: whether a fence's language gets colors at all.
pub fn supports(language: &str) -> bool {
    grammar(language).is_some()
}

/// The language a file path's extension names, as upstream's getLanguageFromPath maps them.
pub fn language_from_path(path: &str) -> Option<&'static str> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    let language = match extension.as_str() {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "c" => "c",
        "h" | "hpp" | "hxx" | "cc" | "cpp" | "cxx" => "cpp",
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "fish",
        "sql" => "sql",
        "css" | "scss" | "sass" | "less" => "css",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "lua" => "lua",
        "html" | "htm" => "html",
        "xml" => "xml",
        "md" | "markdown" => "markdown",
        _ => return None,
    };
    Some(language)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn flush(out: &mut Vec<(Option<Scope>, String)>, plain: &mut String) {
    if !plain.is_empty() {
        out.push((None, std::mem::take(plain)));
    }
}

/// The scoped runs of one line of code.
fn tokens(grammar: &Grammar, line: &str) -> Vec<(Option<Scope>, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(Option<Scope>, String)> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if grammar
            .line_comment
            .iter()
            .any(|marker| rest.starts_with(*marker))
        {
            flush(&mut out, &mut plain);
            out.push((Some(Scope::Comment), rest));
            return out;
        }
        if let Some((open, close)) = grammar.block_comment
            && rest.starts_with(open)
        {
            flush(&mut out, &mut plain);
            let end = rest
                .find(close)
                .map(|at| at + close.len())
                .unwrap_or(rest.len());
            out.push((Some(Scope::Comment), rest[..end].to_string()));
            i += rest[..end].chars().count();
            continue;
        }
        let c = chars[i];
        if grammar.quotes.contains(&c) {
            flush(&mut out, &mut plain);
            let triple =
                grammar.triple_quotes && rest.chars().take(3).filter(|q| *q == c).count() == 3;
            let delimiter = if triple {
                std::iter::repeat_n(c, 3).collect::<String>()
            } else {
                c.to_string()
            };
            let body: Vec<char> = rest.chars().skip(delimiter.chars().count()).collect();
            let mut end = body.len();
            let mut j = 0;
            while j < body.len() {
                if !triple && body[j] == '\\' {
                    j += 2;
                    continue;
                }
                if triple {
                    if body[j..].iter().take(3).filter(|q| **q == c).count() == 3 {
                        end = j + 3;
                        break;
                    }
                } else if body[j] == c {
                    end = j + 1;
                    break;
                }
                j += 1;
            }
            let token: String = rest.chars().take(delimiter.chars().count() + end).collect();
            i += token.chars().count();
            out.push((Some(Scope::String), token));
            continue;
        }
        if c.is_ascii_digit() {
            flush(&mut out, &mut plain);
            let mut end = 0;
            while end < chars.len() - i
                && (chars[i + end].is_ascii_alphanumeric()
                    || chars[i + end] == '.'
                    || (chars[i + end] == '_' && end > 0))
            {
                end += 1;
            }
            out.push((
                Some(Scope::Number),
                chars[i..i + end].iter().collect::<String>(),
            ));
            i += end;
            continue;
        }
        if is_ident_start(c) {
            let mut end = 0;
            while end < chars.len() - i && is_ident(chars[i + end]) {
                end += 1;
            }
            let word: String = chars[i..i + end].iter().collect();
            let after = chars[i + end..]
                .iter()
                .find(|c| !c.is_whitespace())
                .copied();
            if grammar.keywords.contains(&word.as_str()) {
                flush(&mut out, &mut plain);
                out.push((Some(Scope::Keyword), word));
            } else if after == Some('(') {
                flush(&mut out, &mut plain);
                out.push((Some(Scope::Function), word));
            } else if word.starts_with(|c: char| c.is_uppercase()) {
                flush(&mut out, &mut plain);
                out.push((Some(Scope::Type), word));
            } else {
                plain.push_str(&word);
            }
            i += end;
            continue;
        }
        if ",;:()[]{}.<>".contains(c) {
            flush(&mut out, &mut plain);
            out.push((Some(Scope::Punctuation), c.to_string()));
            i += 1;
            continue;
        }
        if "+-*/%=!&|^~?".contains(c) {
            flush(&mut out, &mut plain);
            out.push((Some(Scope::Operator), c.to_string()));
            i += 1;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut out, &mut plain);
    out
}

fn styled(scope: Option<Scope>, text: &str) -> String {
    let token = match scope {
        None => return text.to_string(),
        Some(Scope::Comment) => "syntaxComment",
        Some(Scope::Keyword) => "syntaxKeyword",
        Some(Scope::Type) => "syntaxType",
        Some(Scope::Function) => "syntaxFunction",
        Some(Scope::String) => "syntaxString",
        Some(Scope::Number) => "syntaxNumber",
        Some(Scope::Operator) => "syntaxOperator",
        Some(Scope::Punctuation) => "syntaxPunctuation",
    };
    crate::interactive::theme::fg(token, text)
}

/// Highlighted lines, colored with the theme's syntax tokens as upstream's cli-highlight theme
/// colors highlight.js's scopes.
pub fn highlight(code: &str, language: &str) -> Vec<String> {
    let Some(grammar) = grammar(language) else {
        return code.split('\n').map(str::to_owned).collect();
    };
    code.split('\n')
        .map(|line| {
            tokens(&grammar, line)
                .into_iter()
                .map(|(scope, text)| styled(scope, &text))
                .collect::<String>()
        })
        .collect()
}
