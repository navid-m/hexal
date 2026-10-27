//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const theme = @import("theme.zig");

pub const Token = struct {
    start: usize,
    end: usize,
    kind: theme.SyntaxKind,
};

pub const Language = struct {
    id: []const u8,
    name: []const u8,
    extensions: []const []const u8,
    keywords: []const []const u8 = &.{},
    control: []const []const u8 = &.{},
    types: []const []const u8 = &.{},
    constants: []const []const u8 = &.{},
    line_comment: []const u8 = "",
    block_start: []const u8 = "",
    block_end: []const u8 = "",
    quotes: []const u8 = "\"'",
    case_insensitive: bool = false,

    pub fn fromPath(path: []const u8) ?Language {
        const filename = std.fs.path.basename(path);
        if (std.mem.eql(u8, filename, "Makefile") or std.mem.eql(u8, filename, "GNUmakefile")) {
            return makefile;
        }
        if (std.mem.eql(u8, filename, ".gitignore") or std.mem.eql(u8, filename, ".dockerignore")) {
            return config;
        }
        for (languages) |lang| {
            for (lang.extensions) |ext| {
                if (std.mem.endsWith(u8, path, ext)) return lang;
            }
        }
        return null;
    }

    pub fn fromId(id: []const u8) ?Language {
        if (std.mem.eql(u8, id, makefile.id)) return makefile;
        if (std.mem.eql(u8, id, config.id)) return config;
        for (languages) |lang| {
            if (std.mem.eql(u8, lang.id, id)) return lang;
        }
        return null;
    }

    fn matches(self: Language, word: []const u8, list: []const []const u8) bool {
        if (self.case_insensitive) {
            for (list) |candidate| {
                if (std.ascii.eqlIgnoreCase(word, candidate)) return true;
            }
            return false;
        }
        for (list) |candidate| {
            if (std.mem.eql(u8, word, candidate)) return true;
        }
        return false;
    }
};

const zig_keywords = [_][]const u8{
    "addrspace", "align",       "allowzero",      "asm",      "async",    "await",  "break",       "callconv",
    "catch",     "comptime",    "const",          "continue", "defer",    "else",   "enum",        "errdefer",
    "error",     "export",      "extern",         "fn",       "for",      "if",     "inline",      "linksection",
    "noalias",   "noinline",    "nosuspend",      "opaque",   "or",       "orelse", "packed",      "pub",
    "resume",    "return",      "struct",         "suspend",  "switch",   "test",   "threadlocal", "try",
    "union",     "unreachable", "usingnamespace", "var",      "volatile", "while",  "and",
};
const zig_types = [_][]const u8{
    "anyerror",  "anyframe", "anytype",      "bool",           "c_int", "c_long", "c_short", "c_uint",
    "c_ulong",   "c_ushort", "comptime_int", "comptime_float", "f16",   "f32",    "f64",     "f80",
    "f128",      "i8",       "i16",          "i32",            "i64",   "i128",   "isize",   "noreturn",
    "u8",        "u16",      "u32",          "u64",            "u128",  "usize",  "void",    "type",
    "anyopaque",
};
const zig_constants = [_][]const u8{ "true", "false", "null", "undefined" };
const zig_control = [_][]const u8{
    "if",    "else",   "while", "for",      "switch",      "break",  "continue", "return", "try",
    "catch", "orelse", "defer", "errdefer", "unreachable", "resume", "suspend",
};

const c_keywords = [_][]const u8{
    "auto",     "alignas",      "alignof",  "asm",      "const",   "constexpr", "consteval",     "constinit",
    "decltype", "extern",       "friend",   "inline",   "mutable", "namespace", "noexcept",      "operator",
    "private",  "protected",    "public",   "register", "sizeof",  "static",    "static_assert", "struct",
    "template", "thread_local", "typedef",  "typename", "union",   "using",     "virtual",       "volatile",
    "class",    "concept",      "requires",
};
const c_types = [_][]const u8{
    "bool",    "char",    "char8_t",  "char16_t", "char32_t", "double",   "float",     "int",    "long",
    "short",   "signed",  "unsigned", "void",     "wchar_t",  "size_t",   "ptrdiff_t", "int8_t", "int16_t",
    "int32_t", "int64_t", "uint8_t",  "uint16_t", "uint32_t", "uint64_t",
};
const c_constants = [_][]const u8{ "true", "false", "NULL", "nullptr" };
const c_control = [_][]const u8{
    "if",       "else",   "switch", "case", "default", "for",   "while",    "do",        "break",
    "continue", "return", "goto",   "try",  "catch",   "throw", "co_await", "co_return", "co_yield",
};

const rust_keywords = [_][]const u8{
    "as",     "async", "await", "break", "const",  "continue", "crate", "dyn",   "else",   "enum",
    "extern", "false", "fn",    "for",   "if",     "impl",     "in",    "let",   "loop",   "match",
    "mod",    "move",  "mut",   "pub",   "ref",    "return",   "self",  "Self",  "static", "struct",
    "super",  "trait", "true",  "type",  "unsafe", "use",      "where", "while", "yield",
};
const rust_types = [_][]const u8{
    "bool",   "char", "f32",     "f64",  "i8",      "i16",     "i32",   "i64", "i128",   "isize",  "str",
    "String", "u8",   "u16",     "u32",  "u64",     "u128",    "usize", "Vec", "Option", "Result", "Box",
    "Rc",     "Arc",  "RefCell", "Cell", "HashMap", "HashSet",
};
const rust_constants = [_][]const u8{ "true", "false", "None", "Some", "Ok", "Err" };

const python_keywords = [_][]const u8{
    "and",  "as",     "assert",   "async",   "await", "break", "class",  "continue", "def",    "del",
    "elif", "else",   "except",   "finally", "for",   "from",  "global", "if",       "import", "in",
    "is",   "lambda", "nonlocal", "not",     "or",    "pass",  "raise",  "return",   "try",    "while",
    "with", "yield",
};
const python_types = [_][]const u8{
    "bool",   "bytes", "complex", "dict",  "float", "frozenset", "int", "list", "None",
    "object", "set",   "str",     "tuple", "type",
};
const python_constants = [_][]const u8{ "True", "False", "None", "NotImplemented", "Ellipsis" };

const shell_keywords = [_][]const u8{
    "case",   "do",     "done", "elif",   "else",  "esac",   "fi",     "for",      "function", "if",      "in",
    "select", "then",   "time", "until",  "while", "export", "local",  "readonly", "declare",  "typeset", "unset",
    "shift",  "source", "echo", "printf", "cd",    "exit",   "return", "set",      "trap",     "wait",    "eval",
    "exec",
};
const shell_constants = [_][]const u8{ "true", "false" };

const json_keywords = [_][]const u8{ "true", "false", "null" };
const js_keywords = [_][]const u8{
    "async",   "await",  "break", "case",       "catch",  "class",   "const",   "continue", "debugger",
    "default", "delete", "do",    "else",       "export", "extends", "finally", "for",      "function",
    "if",      "import", "in",    "instanceof", "let",    "new",     "return",  "static",   "super",
    "switch",  "this",   "throw", "try",        "typeof", "var",     "void",    "while",    "with",
    "yield",   "of",     "from",  "as",
};
const js_constants = [_][]const u8{ "true", "false", "null", "undefined", "NaN", "Infinity" };
const js_types = [_][]const u8{
    "Array",  "Boolean", "Date",   "Error", "Function", "JSON",   "Map",     "Math",    "Number",
    "Object", "Promise", "RegExp", "Set",   "String",   "Symbol", "WeakMap", "WeakSet",
};

const go_keywords = [_][]const u8{
    "break", "case",   "chan",   "const",  "continue", "default", "defer",     "else", "fallthrough",
    "for",   "func",   "go",     "goto",   "if",       "import",  "interface", "map",  "package",
    "range", "return", "select", "struct", "switch",   "type",    "var",
};
const go_types = [_][]const u8{
    "bool",   "byte",   "complex64", "complex128", "error", "float32", "float64", "int",
    "int8",   "int16",  "int32",     "int64",      "rune",  "string",  "uint",    "uint8",
    "uint16", "uint32", "uint64",    "uintptr",    "any",
};
const go_constants = [_][]const u8{ "true", "false", "nil", "iota" };

const lua_keywords = [_][]const u8{
    "and",   "break", "do",      "else", "elseif", "end", "false",  "for",    "function", "goto",
    "if",    "in",    "local",   "nil",  "not",    "or",  "repeat", "return", "then",     "true",
    "until", "while", "require", "self",
};

const markdown_keywords = [_][]const u8{""};

pub const languages = [_]Language{
    .{
        .id = "zig",
        .name = "Zig",
        .extensions = &.{".zig"},
        .keywords = &zig_keywords,
        .control = &zig_control,
        .types = &zig_types,
        .constants = &zig_constants,
        .line_comment = "//",
        .quotes = "\"'",
    },
    .{
        .id = "c",
        .name = "C",
        .extensions = &.{ ".c", ".h" },
        .keywords = &c_keywords,
        .control = &c_control,
        .types = &c_types,
        .constants = &c_constants,
        .line_comment = "//",
        .block_start = "/*",
        .block_end = "*/",
    },
    .{
        .id = "cpp",
        .name = "C++",
        .extensions = &.{ ".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx" },
        .keywords = &c_keywords,
        .control = &c_control,
        .types = &c_types,
        .constants = &c_constants,
        .line_comment = "//",
        .block_start = "/*",
        .block_end = "*/",
    },
    .{
        .id = "rust",
        .name = "Rust",
        .extensions = &.{".rs"},
        .keywords = &rust_keywords,
        .control = &rust_keywords,
        .types = &rust_types,
        .constants = &rust_constants,
        .line_comment = "//",
        .block_start = "/*",
        .block_end = "*/",
    },
    .{
        .id = "python",
        .name = "Python",
        .extensions = &.{ ".py", ".pyw", ".pyi" },
        .keywords = &python_keywords,
        .control = &python_keywords,
        .types = &python_types,
        .constants = &python_constants,
        .line_comment = "#",
    },
    .{
        .id = "shell",
        .name = "Shell Script",
        .extensions = &.{ ".sh", ".bash", ".zsh", ".ksh", ".dash", ".ash" },
        .keywords = &shell_keywords,
        .control = &shell_keywords,
        .constants = &shell_constants,
        .line_comment = "#",
        .quotes = "\"'`",
    },
    .{
        .id = "json",
        .name = "JSON",
        .extensions = &.{ ".json", ".jsonc", ".jsonl", ".ndjson", ".geojson" },
        .keywords = &json_keywords,
        .constants = &json_keywords,
        .line_comment = "//",
        .quotes = "\"",
    },
    .{
        .id = "javascript",
        .name = "JavaScript",
        .extensions = &.{ ".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx" },
        .keywords = &js_keywords,
        .control = &js_keywords,
        .types = &js_types,
        .constants = &js_constants,
        .line_comment = "//",
        .block_start = "/*",
        .block_end = "*/",
        .quotes = "\"'`",
    },
    .{
        .id = "go",
        .name = "Go",
        .extensions = &.{".go"},
        .keywords = &go_keywords,
        .control = &go_keywords,
        .types = &go_types,
        .constants = &go_constants,
        .line_comment = "//",
        .block_start = "/*",
        .block_end = "*/",
    },
    .{
        .id = "lua",
        .name = "Lua",
        .extensions = &.{ ".lua", ".rockspec" },
        .keywords = &lua_keywords,
        .control = &lua_keywords,
        .line_comment = "--",
        .block_start = "--[[",
        .block_end = "]]",
    },
    .{
        .id = "markdown",
        .name = "Markdown",
        .extensions = &.{ ".md", ".markdown", ".mdx" },
        .keywords = &markdown_keywords,
    },
    .{
        .id = "toml",
        .name = "TOML",
        .extensions = &.{".toml"},
        .line_comment = "#",
    },
};

const makefile = Language{
    .id = "make",
    .name = "Makefile",
    .extensions = &.{ ".mk", ".mak", ".make" },
    .line_comment = "#",
};

const config = Language{
    .id = "config",
    .name = "Config",
    .extensions = &.{ ".conf", ".ini" },
    .line_comment = "#",
};

pub fn tokenizeLine(
    allocator: std.mem.Allocator,
    text: []const u8,
    language: ?Language,
    tokens: *std.ArrayList(Token),
) !void {
    tokens.clearRetainingCapacity();
    const lang = language orelse return;
    var i: usize = 0;

    while (i < text.len) {
        // Line comments.
        if (lang.line_comment.len > 0 and
            std.mem.startsWith(u8, text[i..], lang.line_comment))
        {
            try tokens.append(allocator, .{
                .start = i,
                .end = text.len,
                .kind = .comment,
            });
            break;
        }

        // Block comments (line-local portion; the parser intentionally does
        // not carry block state between lines in this compact implementation).
        if (lang.block_start.len > 0 and
            std.mem.startsWith(u8, text[i..], lang.block_start))
        {
            var end = text.len;
            if (lang.block_end.len > 0) {
                if (std.mem.indexOfPos(u8, text, i + lang.block_start.len, lang.block_end)) |found| {
                    end = found + lang.block_end.len;
                }
            }
            try tokens.append(allocator, .{ .start = i, .end = end, .kind = .comment });
            i = end;
            continue;
        }

        const c = text[i];

        // Strings.
        if (std.mem.indexOfScalar(u8, lang.quotes, c) != null) {
            const quote = c;
            var j = i + 1;
            while (j < text.len) {
                if (text[j] == '\\' and j + 1 < text.len) {
                    j += 2;
                    continue;
                }
                if (text[j] == quote) {
                    j += 1;
                    break;
                }
                j += 1;
            }
            try tokens.append(allocator, .{ .start = i, .end = j, .kind = .string });
            i = j;
            continue;
        }

        // Numbers.
        if (std.ascii.isDigit(c)) {
            var j = i + 1;
            while (j < text.len) {
                const n = text[j];
                if (std.ascii.isAlphanumeric(n) or n == '_' or n == '.' or
                    n == 'x' or n == 'X' or n == '+' or n == '-')
                {
                    j += 1;
                } else break;
            }
            try tokens.append(allocator, .{ .start = i, .end = j, .kind = .number });
            i = j;
            continue;
        }

        // Identifiers / keywords.
        if (isIdentifierStart(c)) {
            var j = i + 1;
            while (j < text.len and isIdentifierContinue(text[j])) : (j += 1) {}
            const word = text[i..j];
            if (lang.matches(word, lang.constants)) {
                try tokens.append(allocator, .{ .start = i, .end = j, .kind = .constant });
            } else if (lang.matches(word, lang.types)) {
                try tokens.append(allocator, .{ .start = i, .end = j, .kind = .type_ });
            } else if (lang.matches(word, lang.control)) {
                try tokens.append(allocator, .{ .start = i, .end = j, .kind = .control });
            } else if (lang.matches(word, lang.keywords)) {
                try tokens.append(allocator, .{ .start = i, .end = j, .kind = .keyword });
            }
            i = j;
            continue;
        }

        i += 1;
    }
}

fn isIdentifierStart(c: u8) bool {
    return std.ascii.isAlphabetic(c) or c == '_' or c >= 0x80;
}

fn isIdentifierContinue(c: u8) bool {
    return std.ascii.isAlphanumeric(c) or c == '_' or c >= 0x80;
}

test "tokenize zig keywords and strings" {
    const allocator = std.testing.allocator;
    var tokens: std.ArrayList(Token) = .empty;
    defer tokens.deinit(allocator);

    try tokenizeLine(allocator, "pub fn main() void { const s = \"hi\"; }", Language.fromPath("x.zig"), &tokens);
    try std.testing.expect(tokens.items.len >= 4);
    var saw_keyword = false;
    var saw_string = false;
    for (tokens.items) |token| {
        if (token.kind == .keyword) saw_keyword = true;
        if (token.kind == .string) saw_string = true;
    }
    try std.testing.expect(saw_keyword);
    try std.testing.expect(saw_string);
}
