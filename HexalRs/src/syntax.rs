//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use crate::theme::SyntaxKind;

#[derive(Clone, Copy)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: SyntaxKind,
}

pub struct Language {
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub control: &'static [&'static str],
    pub types: &'static [&'static str],
    pub constants: &'static [&'static str],
    pub line_comment: &'static str,
    pub block_start: &'static str,
    pub block_end: &'static str,
    pub quotes: &'static str,
    pub case_insensitive: bool,
}

static RUST: Language = Language {
    id: "rust",
    name: "Rust",
    extensions: &[".rs"],
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while", "yield",
    ],
    control: &[
        "break", "continue", "else", "for", "if", "loop", "match", "return", "while", "yield",
    ],
    types: &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "String",
        "u8", "u16", "u32", "u64", "u128", "usize", "Vec", "Option", "Result", "Box", "Rc", "Arc",
        "RefCell", "Cell", "HashMap", "HashSet",
    ],
    constants: &["true", "false", "None", "Some", "Ok", "Err"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'",
    case_insensitive: false,
};

static ZIG: Language = Language {
    id: "zig",
    name: "Zig",
    extensions: &[".zig"],
    keywords: &[
        "addrspace",
        "align",
        "allowzero",
        "asm",
        "async",
        "await",
        "break",
        "callconv",
        "catch",
        "comptime",
        "const",
        "continue",
        "defer",
        "else",
        "enum",
        "errdefer",
        "error",
        "export",
        "extern",
        "fn",
        "for",
        "if",
        "inline",
        "linksection",
        "noalias",
        "noinline",
        "nosuspend",
        "opaque",
        "or",
        "orelse",
        "packed",
        "pub",
        "resume",
        "return",
        "struct",
        "suspend",
        "switch",
        "test",
        "threadlocal",
        "try",
        "union",
        "unreachable",
        "usingnamespace",
        "var",
        "volatile",
        "while",
        "and",
    ],
    control: &[
        "if",
        "else",
        "while",
        "for",
        "switch",
        "break",
        "continue",
        "return",
        "try",
        "catch",
        "orelse",
        "defer",
        "errdefer",
        "unreachable",
        "resume",
        "suspend",
    ],
    types: &[
        "anyerror",
        "anyframe",
        "anytype",
        "bool",
        "c_int",
        "c_long",
        "c_short",
        "c_uint",
        "c_ulong",
        "c_ushort",
        "comptime_int",
        "comptime_float",
        "f16",
        "f32",
        "f64",
        "f80",
        "f128",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "noreturn",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "void",
        "type",
        "anyopaque",
    ],
    constants: &["true", "false", "null", "undefined"],
    line_comment: "//",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: false,
};

static C: Language = Language {
    id: "c",
    name: "C",
    extensions: &[".c", ".h"],
    keywords: &[
        "auto",
        "alignas",
        "alignof",
        "asm",
        "const",
        "constexpr",
        "extern",
        "inline",
        "register",
        "restrict",
        "sizeof",
        "static",
        "static_assert",
        "struct",
        "thread_local",
        "typedef",
        "union",
        "volatile",
        "_Atomic",
        "_Generic",
        "_Noreturn",
        "_Static_assert",
    ],
    control: &[
        "if", "else", "switch", "case", "default", "for", "while", "do", "break", "continue",
        "return", "goto",
    ],
    types: &[
        "bool",
        "char",
        "double",
        "float",
        "int",
        "long",
        "short",
        "signed",
        "unsigned",
        "void",
        "size_t",
        "ptrdiff_t",
        "int8_t",
        "int16_t",
        "int32_t",
        "int64_t",
        "uint8_t",
        "uint16_t",
        "uint32_t",
        "uint64_t",
    ],
    constants: &["true", "false", "NULL"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'",
    case_insensitive: false,
};

static CPP: Language = Language {
    id: "cpp",
    name: "C++",
    extensions: &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"],
    keywords: &[
        "auto",
        "alignas",
        "alignof",
        "asm",
        "const",
        "constexpr",
        "consteval",
        "constinit",
        "decltype",
        "extern",
        "friend",
        "inline",
        "mutable",
        "namespace",
        "noexcept",
        "operator",
        "private",
        "protected",
        "public",
        "register",
        "sizeof",
        "static",
        "static_assert",
        "struct",
        "template",
        "thread_local",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "volatile",
        "class",
        "concept",
        "requires",
    ],
    control: &[
        "if", "else", "switch", "case", "default", "for", "while", "do", "break", "continue",
        "return", "goto", "try", "catch", "throw",
    ],
    types: &[
        "bool",
        "char",
        "char8_t",
        "char16_t",
        "char32_t",
        "double",
        "float",
        "int",
        "long",
        "short",
        "signed",
        "unsigned",
        "void",
        "wchar_t",
        "size_t",
        "ptrdiff_t",
        "int8_t",
        "int16_t",
        "int32_t",
        "int64_t",
        "uint8_t",
        "uint16_t",
        "uint32_t",
        "uint64_t",
    ],
    constants: &["true", "false", "NULL", "nullptr"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'",
    case_insensitive: false,
};

static CSHARP: Language = Language {
    id: "csharp",
    name: "C#",
    extensions: &[".cs"],
    keywords: &[
        "abstract",
        "as",
        "async",
        "await",
        "base",
        "break",
        "case",
        "catch",
        "checked",
        "class",
        "const",
        "continue",
        "default",
        "delegate",
        "do",
        "else",
        "enum",
        "event",
        "explicit",
        "extern",
        "finally",
        "fixed",
        "for",
        "foreach",
        "goto",
        "if",
        "implicit",
        "in",
        "interface",
        "internal",
        "is",
        "lock",
        "namespace",
        "new",
        "operator",
        "out",
        "override",
        "params",
        "private",
        "protected",
        "public",
        "readonly",
        "record",
        "ref",
        "return",
        "sealed",
        "sizeof",
        "stackalloc",
        "static",
        "struct",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "unchecked",
        "unsafe",
        "using",
        "virtual",
        "volatile",
        "while",
    ],
    control: &[
        "break", "case", "catch", "continue", "default", "do", "else", "finally", "for", "foreach",
        "goto", "if", "return", "switch", "throw", "try", "while",
    ],
    types: &[
        "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "nint",
        "nuint", "object", "sbyte", "short", "string", "uint", "ulong", "ushort", "var", "void",
    ],
    constants: &["true", "false", "null"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'",
    case_insensitive: false,
};

static PYTHON: Language = Language {
    id: "python",
    name: "Python",
    extensions: &[".py", ".pyw"],
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield",
    ],
    control: &[
        "break", "continue", "elif", "else", "except", "finally", "for", "if", "return", "try",
        "while", "with", "yield",
    ],
    types: &[
        "bool",
        "bytes",
        "complex",
        "dict",
        "float",
        "frozenset",
        "int",
        "list",
        "object",
        "set",
        "str",
        "tuple",
        "type",
    ],
    constants: &["True", "False", "None", "NotImplemented", "Ellipsis"],
    line_comment: "#",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: false,
};

static JAVASCRIPT: Language = Language {
    id: "javascript",
    name: "JavaScript",
    extensions: &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx"],
    keywords: &[
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
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
        "interface",
        "type",
        "enum",
        "implements",
        "private",
        "protected",
        "public",
        "readonly",
    ],
    control: &[
        "break", "case", "catch", "continue", "default", "do", "else", "finally", "for", "if",
        "return", "switch", "throw", "try", "while",
    ],
    types: &[
        "boolean", "number", "string", "symbol", "bigint", "any", "unknown", "never", "void",
        "object", "Array", "Promise", "Map", "Set",
    ],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'`",
    case_insensitive: false,
};

static GO: Language = Language {
    id: "go",
    name: "Go",
    extensions: &[".go"],
    keywords: &[
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
    ],
    control: &[
        "break",
        "case",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "go",
        "goto",
        "if",
        "range",
        "return",
        "select",
        "switch",
    ],
    types: &[
        "bool",
        "byte",
        "complex64",
        "complex128",
        "error",
        "float32",
        "float64",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "rune",
        "string",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
        "any",
    ],
    constants: &["true", "false", "nil", "iota"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"'`",
    case_insensitive: false,
};

static LUA: Language = Language {
    id: "lua",
    name: "Lua",
    extensions: &[".lua"],
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
        "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ],
    control: &[
        "break", "do", "else", "elseif", "for", "goto", "if", "repeat", "return", "then", "until",
        "while",
    ],
    types: &[],
    constants: &["true", "false", "nil"],
    line_comment: "--",
    block_start: "--[[",
    block_end: "]]",
    quotes: "\"'",
    case_insensitive: false,
};

static SHELL: Language = Language {
    id: "shell",
    name: "Shell",
    extensions: &[".sh", ".bash", ".zsh", ".ksh", ".dash", ".ash"],
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in",
        "select", "then", "time", "until", "while", "export", "local", "readonly", "declare",
        "typeset", "unset", "shift", "source", "echo", "printf", "cd", "exit", "return", "set",
        "trap", "wait", "eval", "exec",
    ],
    control: &[
        "case", "do", "done", "elif", "else", "esac", "fi", "for", "if", "select", "then", "until",
        "while",
    ],
    types: &[],
    constants: &["true", "false"],
    line_comment: "#",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: false,
};

static JSON: Language = Language {
    id: "json",
    name: "JSON",
    extensions: &[".json", ".jsonc", ".jsonl", ".ndjson", ".geojson"],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &["true", "false", "null"],
    line_comment: "//",
    block_start: "/*",
    block_end: "*/",
    quotes: "\"",
    case_insensitive: false,
};

static TOML: Language = Language {
    id: "toml",
    name: "TOML",
    extensions: &[".toml"],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &["true", "false"],
    line_comment: "#",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: false,
};

static YAML: Language = Language {
    id: "yaml",
    name: "YAML",
    extensions: &[".yaml", ".yml"],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &["true", "false", "null", "yes", "no"],
    line_comment: "#",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: false,
};

static MARKDOWN: Language = Language {
    id: "markdown",
    name: "Markdown",
    extensions: &[".md", ".markdown"],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &[],
    line_comment: "",
    block_start: "",
    block_end: "",
    quotes: "\"'`",
    case_insensitive: false,
};

static CONFIG: Language = Language {
    id: "config",
    name: "Config",
    extensions: &[".gitignore", ".dockerignore", ".npmignore", ".env"],
    keywords: &[],
    control: &[],
    types: &[],
    constants: &["true", "false"],
    line_comment: "#",
    block_start: "",
    block_end: "",
    quotes: "\"'",
    case_insensitive: true,
};

static LANGUAGES: &[&Language] = &[
    &RUST,
    &ZIG,
    &C,
    &CPP,
    &CSHARP,
    &PYTHON,
    &JAVASCRIPT,
    &GO,
    &LUA,
    &SHELL,
    &JSON,
    &TOML,
    &YAML,
    &MARKDOWN,
    &CONFIG,
];

pub fn from_path(path: &str) -> Option<&'static Language> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if file_name == "Makefile" || file_name == "GNUmakefile" {
        return Some(&SHELL);
    }
    for language in LANGUAGES {
        for extension in language.extensions {
            if path.ends_with(extension) {
                return Some(language);
            }
        }
    }
    None
}

pub fn from_id(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().copied().find(|language| language.id == id)
}

pub fn tokenize_line(text: &[u8], language: Option<&Language>) -> Vec<Token> {
    let Some(language) = language else {
        return Vec::new();
    };
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        if !language.line_comment.is_empty()
            && text[i..].starts_with(language.line_comment.as_bytes())
        {
            tokens.push(Token {
                start: i,
                end: text.len(),
                kind: SyntaxKind::Comment,
            });
            break;
        }

        if !language.block_start.is_empty()
            && text[i..].starts_with(language.block_start.as_bytes())
        {
            let mut end = text.len();
            if !language.block_end.is_empty() {
                let search_from = i + language.block_start.len();
                if let Some(found) = find_bytes(&text[search_from..], language.block_end.as_bytes())
                {
                    end = search_from + found + language.block_end.len();
                }
            }
            tokens.push(Token {
                start: i,
                end,
                kind: SyntaxKind::Comment,
            });
            i = end;
            continue;
        }

        let byte = text[i];
        if language.quotes.as_bytes().contains(&byte) {
            let quote = byte;
            let mut j = i + 1;
            while j < text.len() {
                if text[j] == b'\\' && j + 1 < text.len() {
                    j += 2;
                    continue;
                }
                if text[j] == quote {
                    j += 1;
                    break;
                }
                j += 1;
            }
            tokens.push(Token {
                start: i,
                end: j,
                kind: SyntaxKind::String,
            });
            i = j;
            continue;
        }

        if byte.is_ascii_digit() {
            let mut j = i + 1;
            while j < text.len() {
                let n = text[j];
                if n.is_ascii_alphanumeric() || n == b'_' || n == b'.' || n == b'+' || n == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            tokens.push(Token {
                start: i,
                end: j,
                kind: SyntaxKind::Number,
            });
            i = j;
            continue;
        }

        if is_identifier_start(byte) {
            let mut j = i + 1;
            while j < text.len() && is_identifier_continue(text[j]) {
                j += 1;
            }
            let word = &text[i..j];
            let kind = if matches(language, word, language.constants) {
                Some(SyntaxKind::Constant)
            } else if matches(language, word, language.types) {
                Some(SyntaxKind::Type)
            } else if matches(language, word, language.control) {
                Some(SyntaxKind::Control)
            } else if matches(language, word, language.keywords) {
                Some(SyntaxKind::Keyword)
            } else {
                None
            };
            if let Some(kind) = kind {
                tokens.push(Token {
                    start: i,
                    end: j,
                    kind,
                });
            }
            i = j;
            continue;
        }

        i += 1;
    }
    tokens
}

fn matches(language: &Language, word: &[u8], list: &[&str]) -> bool {
    list.iter().any(|candidate| {
        let candidate = candidate.as_bytes();
        if candidate.len() != word.len() {
            return false;
        }
        if language.case_insensitive {
            word.eq_ignore_ascii_case(candidate)
        } else {
            word == candidate
        }
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}
