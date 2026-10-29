//! Data-driven `.hexdef` grammar parsing and a cached, grammar-driven lexer.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const json = @import("json.zig");
const regex_mod = @import("regex.zig");
const theme = @import("theme.zig");

pub const Token = struct {
    start: usize,
    end: usize,
    kind: theme.SyntaxKind,
};

pub const FormatterConfig = struct {
    command: []u8,
    arguments: []const []u8,
    input: []u8,
    output: []u8,
    timeout_ms: u64,
};

pub const LspFallback = struct {
    name: []u8,
    command: []u8,
    arguments: []const []u8,
    language_id: []u8,
};

pub const Indentation = struct {
    use_tabs: bool,
    indent_width: usize,
    tab_width: usize,
};

pub const LineHighlight = struct {
    pattern: regex_mod.Regex,
    kind: theme.SyntaxKind,
};

pub const Language = struct {
    allocator: std.mem.Allocator,
    id: []u8,
    name: []u8,
    extensions: []const []u8,
    file_names: []const []u8,
    classifications: std.StringHashMap(theme.SyntaxKind),
    case_insensitive: bool,
    line_highlights: []LineHighlight = &.{},

    line_comment: ?[]u8 = null,
    block_start: ?[]u8 = null,
    block_end: ?[]u8 = null,
    quote_characters: []const []u8 = &.{},
    escape_character: []u8 = &.{},
    highlight_numbers: bool = true,

    formatter: ?FormatterConfig = null,
    fallback_formatter: ?FormatterConfig = null,
    fallback_lsp: ?LspFallback = null,
    format_on_save: bool = false,
    indentation: ?Indentation = null,
    auto_indent: bool = true,
    auto_close: bool = true,
    traditional_markup: bool = false,
    indent_after: []const []u8 = &.{},
    unindent_before: []const []u8 = &.{},

    pub fn deinit(self: *Language) void {
        const a = self.allocator;
        a.free(self.id);
        a.free(self.name);
        freeStrings(a, self.extensions);
        freeStrings(a, self.file_names);
        var iterator = self.classifications.iterator();
        while (iterator.next()) |entry| a.free(entry.key_ptr.*);
        self.classifications.deinit();
        if (self.line_comment) |value| a.free(value);
        if (self.block_start) |value| a.free(value);
        if (self.block_end) |value| a.free(value);
        freeStrings(a, self.quote_characters);
        a.free(self.escape_character);
        for (self.line_highlights) |*highlight| highlight.pattern.deinit();
        a.free(self.line_highlights);
        if (self.formatter) |*formatter| freeFormatter(a, formatter);
        if (self.fallback_formatter) |*formatter| freeFormatter(a, formatter);
        if (self.fallback_lsp) |*lsp| {
            a.free(lsp.name);
            a.free(lsp.command);
            freeStrings(a, lsp.arguments);
            a.free(lsp.language_id);
        }
        freeStrings(a, self.indent_after);
        freeStrings(a, self.unindent_before);
        self.* = undefined;
    }

    pub fn matchesExtension(self: Language, path: []const u8) bool {
        const filename = std.fs.path.basename(path);
        for (self.file_names) |name| {
            if (std.ascii.eqlIgnoreCase(filename, name)) return true;
        }
        for (self.extensions) |extension| {
            if (filename.len >= extension.len and
                std.ascii.eqlIgnoreCase(filename[filename.len - extension.len ..], extension))
            {
                return true;
            }
        }
        return false;
    }

    pub fn classify(self: Language, word: []const u8) theme.SyntaxKind {
        if (word.len == 0) return .default;
        if (self.case_insensitive) {
            var buffer: [256]u8 = undefined;
            if (word.len > buffer.len) return .default;
            const lower = std.ascii.lowerString(buffer[0..word.len], word);
            return self.classifications.get(lower) orelse .default;
        }
        return self.classifications.get(word) orelse .default;
    }

    pub fn resolveFormatter(self: Language) ?FormatterConfig {
        return self.formatter orelse self.fallback_formatter;
    }
};

fn freeStrings(allocator: std.mem.Allocator, items: []const []u8) void {
    for (items) |item| allocator.free(item);
    allocator.free(items);
}

fn freeFormatter(allocator: std.mem.Allocator, formatter: *FormatterConfig) void {
    allocator.free(formatter.command);
    freeStrings(allocator, formatter.arguments);
    allocator.free(formatter.input);
    allocator.free(formatter.output);
}

fn stringValue(value: ?json.Value) ?[]const u8 {
    const v = value orelse return null;
    return v.asString();
}

fn appendStringArray(
    allocator: std.mem.Allocator,
    destination: *std.ArrayList([]u8),
    root: json.Value,
    key: []const u8,
) !void {
    var values = try root.stringArray(allocator, key);
    defer {
        for (values.items) |item| allocator.free(item);
        values.deinit(allocator);
    }
    for (values.items) |item| {
        try destination.append(allocator, try allocator.dupe(u8, item));
    }
}

fn boolValue(value: json.Value, key: []const u8) ?bool {
    return if (value.get(key)) |v| v.asBool() else null;
}

fn normalizeExtensions(allocator: std.mem.Allocator, values: []const []u8) ![]const []u8 {
    var result: std.ArrayList([]u8) = .empty;
    errdefer {
        for (result.items) |item| allocator.free(item);
        result.deinit(allocator);
    }
    for (values) |raw| {
        const value = std.mem.trim(u8, raw, " \t\r\n");
        if (value.len == 0) continue;
        const extra_dot = value[0] != '.';
        const copy = try allocator.alloc(u8, value.len + @intFromBool(extra_dot));
        if (extra_dot) {
            copy[0] = '.';
            @memcpy(copy[1..], value);
        } else {
            @memcpy(copy, value);
        }
        _ = std.ascii.lowerString(copy, copy);
        try result.append(allocator, copy);
    }
    return result.toOwnedSlice(allocator);
}

fn normalizeFileNames(allocator: std.mem.Allocator, values: []const []u8) ![]const []u8 {
    var result: std.ArrayList([]u8) = .empty;
    errdefer {
        for (result.items) |item| allocator.free(item);
        result.deinit(allocator);
    }
    for (values) |raw| {
        const value = std.mem.trim(u8, raw, " \t\r\n");
        if (value.len == 0) continue;
        try result.append(allocator, try allocator.dupe(u8, value));
    }
    return result.toOwnedSlice(allocator);
}

fn tokenKind(name: []const u8) ?theme.SyntaxKind {
    var buffer: [64]u8 = undefined;
    if (name.len > buffer.len) return null;
    const lower = std.ascii.lowerString(buffer[0..name.len], std.mem.trim(u8, name, " \t\r\n"));
    if (std.mem.eql(u8, lower, "keyword")) return .keyword;
    if (std.mem.eql(u8, lower, "control") or std.mem.eql(u8, lower, "controlstatement") or
        std.mem.eql(u8, lower, "control_statement")) return .control;
    if (std.mem.eql(u8, lower, "type")) return .type_;
    if (std.mem.eql(u8, lower, "constant")) return .constant;
    if (std.mem.eql(u8, lower, "string") or std.mem.eql(u8, lower, "stringliteral") or
        std.mem.eql(u8, lower, "string_literal")) return .string;
    if (std.mem.eql(u8, lower, "comment")) return .comment;
    if (std.mem.eql(u8, lower, "number")) return .number;
    if (std.mem.eql(u8, lower, "added")) return .added;
    if (std.mem.eql(u8, lower, "deleted")) return .deleted;
    return null;
}

fn readSet(
    allocator: std.mem.Allocator,
    root: json.Value,
    key: []const u8,
    map: *std.StringHashMap(theme.SyntaxKind),
    kind: theme.SyntaxKind,
    case_insensitive: bool,
) !void {
    var values = try root.stringArray(allocator, key);
    defer {
        for (values.items) |value| allocator.free(value);
        values.deinit(allocator);
    }
    for (values.items) |value| {
        if (value.len == 0) continue;
        if (case_insensitive) {
            var buffer: [256]u8 = undefined;
            if (value.len > buffer.len) continue;
            const lower = std.ascii.lowerString(buffer[0..value.len], value);
            const copy = try allocator.dupe(u8, lower);
            errdefer allocator.free(copy);
            const entry = try map.getOrPut(copy);
            if (entry.found_existing) {
                allocator.free(copy);
            } else {
                entry.value_ptr.* = kind;
            }
        } else {
            const copy = try allocator.dupe(u8, value);
            errdefer allocator.free(copy);
            const entry = try map.getOrPut(copy);
            if (entry.found_existing) {
                allocator.free(copy);
            } else {
                entry.value_ptr.* = kind;
            }
        }
    }
}

fn parseFormatter(allocator: std.mem.Allocator, value: json.Value, field: []const u8) !?FormatterConfig {
    const object = value.get(field) orelse return null;
    if (object.asObject() == null) return error.InvalidFormatter;
    const command_raw = stringValue(object.get("command")) orelse "";
    const command = std.mem.trim(u8, command_raw, " \t\r\n");
    if (command.len == 0) return error.InvalidFormatter;
    const input = std.mem.trim(u8, stringValue(object.get("input")) orelse "stdin", " \t\r\n");
    const output = std.mem.trim(u8, stringValue(object.get("output")) orelse "stdout", " \t\r\n");
    if (!std.ascii.eqlIgnoreCase(input, "stdin") and !std.ascii.eqlIgnoreCase(input, "file")) {
        return error.InvalidFormatter;
    }
    if (!std.ascii.eqlIgnoreCase(output, "stdout") and !std.ascii.eqlIgnoreCase(output, "file")) {
        return error.InvalidFormatter;
    }
    var timeout_ms: u64 = 15000;
    if (object.get("timeout")) |timeout_value| {
        if (timeout_value.asI64()) |configured| {
            if (configured > 0) timeout_ms = @intCast(configured);
        }
    }
    var args = try object.stringArray(allocator, "arguments");
    if (args.items.len == 0) {
        const other = try object.stringArray(allocator, "args");
        args.deinit(allocator);
        args = other;
    }
    return .{
        .command = try allocator.dupe(u8, command),
        .arguments = try args.toOwnedSlice(allocator),
        .input = try allocator.dupe(u8, input),
        .output = try allocator.dupe(u8, output),
        .timeout_ms = timeout_ms,
    };
}

fn parseFallbackLsp(allocator: std.mem.Allocator, root: json.Value, id: []const u8) !?LspFallback {
    const object = root.get("fallbackLsp") orelse root.get("fallback_lsp") orelse return null;
    if (object.asObject() == null) return error.InvalidLsp;
    const command_raw = stringValue(object.get("command")) orelse "";
    const command = std.mem.trim(u8, command_raw, " \t\r\n");
    if (command.len == 0) return error.InvalidLsp;
    var name = std.mem.trim(u8, stringValue(object.get("name")) orelse command, " \t\r\n");
    if (name.len == 0) name = command;
    var args = try object.stringArray(allocator, "arguments");
    if (args.items.len == 0) {
        const other = try object.stringArray(allocator, "args");
        args.deinit(allocator);
        args = other;
    }
    const language_raw = stringValue(object.get("languageId")) orelse
        stringValue(object.get("language_id")) orelse id;
    return .{
        .name = try allocator.dupe(u8, name),
        .command = try allocator.dupe(u8, command),
        .arguments = try args.toOwnedSlice(allocator),
        .language_id = try allocator.dupe(u8, language_raw),
    };
}

fn parseIndentation(value: json.Value) !?Indentation {
    const object = value.get("indentation") orelse return null;
    if (object.asObject() == null) return error.InvalidIndentation;
    const style = stringValue(object.get("style")) orelse "spaces";
    const use_tabs = std.mem.eql(u8, style, "tabs");
    if (!use_tabs and !std.mem.eql(u8, style, "spaces")) return error.InvalidIndentation;
    var indent_width: usize = 4;
    if (object.get("width")) |width| {
        if (width.asI64()) |configured| {
            if (configured < 1 or configured > 16) return error.InvalidIndentation;
            indent_width = @intCast(configured);
        }
    }
    var tab_width: usize = if (use_tabs) indent_width else 4;
    if (object.get("tabWidth")) |width| {
        if (width.asI64()) |configured| {
            if (configured < 1 or configured > 16) return error.InvalidIndentation;
            tab_width = @intCast(configured);
        }
    }
    return .{ .use_tabs = use_tabs, .indent_width = indent_width, .tab_width = tab_width };
}

pub fn parseLanguage(allocator: std.mem.Allocator, text: []const u8, source_path: []const u8) !Language {
    var root = try json.parse(allocator, text);
    defer root.deinit(allocator);
    if (root.asObject() == null) return error.RootMustBeObject;

    const fallback_id = std.fs.path.stem(source_path);
    var id = std.mem.trim(u8, stringValue(root.get("id")) orelse "", " \t\r\n");
    if (id.len == 0) id = fallback_id;
    if (id.len == 0) return error.MissingLanguageId;
    var name = std.mem.trim(u8, stringValue(root.get("name")) orelse "", " \t\r\n");
    if (name.len == 0) name = id;

    var extensions_raw = try root.stringArray(allocator, "extensions");
    defer {
        for (extensions_raw.items) |item| allocator.free(item);
        extensions_raw.deinit(allocator);
    }
    var file_names_raw = try root.stringArray(allocator, "fileNames");
    defer {
        for (file_names_raw.items) |item| allocator.free(item);
        file_names_raw.deinit(allocator);
    }
    var file_names_alt = try root.stringArray(allocator, "file_names");
    defer {
        for (file_names_alt.items) |item| allocator.free(item);
        file_names_alt.deinit(allocator);
    }

    var combined_names: std.ArrayList([]u8) = .empty;
    defer combined_names.deinit(allocator);
    for (file_names_raw.items) |item| try combined_names.append(allocator, item);
    for (file_names_alt.items) |item| try combined_names.append(allocator, item);

    const case_insensitive = (boolValue(root, "caseInsensitive") orelse false);
    var classifications = std.StringHashMap(theme.SyntaxKind).init(allocator);
    errdefer {
        var iterator = classifications.iterator();
        while (iterator.next()) |entry| allocator.free(entry.key_ptr.*);
        classifications.deinit();
    }
    try readSet(allocator, root, "keywords", &classifications, .keyword, case_insensitive);
    try readSet(allocator, root, "controlStatements", &classifications, .control, case_insensitive);
    try readSet(allocator, root, "types", &classifications, .type_, case_insensitive);
    try readSet(allocator, root, "constants", &classifications, .constant, case_insensitive);

    var line_highlights: std.ArrayList(LineHighlight) = .empty;
    errdefer {
        for (line_highlights.items) |*item| item.pattern.deinit();
        line_highlights.deinit(allocator);
    }
    if (root.get("linePatterns")) |patterns| {
        const items = patterns.asArray() orelse return error.InvalidLinePatterns;
        for (items, 0..) |item, index| {
            _ = index;
            if (item.asObject() == null) return error.InvalidLinePatterns;
            const pattern_text = std.mem.trim(
                u8,
                stringValue(item.get("pattern")) orelse "",
                " \t\r\n",
            );
            if (pattern_text.len == 0) return error.InvalidLinePatterns;
            const kind_name = stringValue(item.get("kind")) orelse "";
            const kind = tokenKind(kind_name) orelse return error.InvalidLinePatterns;
            var compiled = try regex_mod.Regex.compile(allocator, pattern_text);
            errdefer compiled.deinit();
            try line_highlights.append(allocator, .{ .pattern = compiled, .kind = kind });
        }
    }

    var line_comment: ?[]u8 = null;
    var block_start: ?[]u8 = null;
    var block_end: ?[]u8 = null;
    if (root.get("comments")) |comments| {
        if (comments.asObject() != null) {
            if (stringValue(comments.get("line"))) |value| line_comment = try allocator.dupe(u8, value);
            if (stringValue(comments.get("blockStart")) orelse stringValue(comments.get("start"))) |value| {
                block_start = try allocator.dupe(u8, value);
            }
            if (stringValue(comments.get("blockEnd")) orelse stringValue(comments.get("end"))) |value| {
                block_end = try allocator.dupe(u8, value);
            }
        }
    }
    if (line_comment == null) {
        if (stringValue(root.get("lineComment"))) |value| line_comment = try allocator.dupe(u8, value);
    }
    if (root.get("blockComment")) |block| {
        if (block.asObject() != null) {
            if (block_start == null) {
                if (stringValue(block.get("start"))) |value| block_start = try allocator.dupe(u8, value);
            }
            if (block_end == null) {
                if (stringValue(block.get("end"))) |value| block_end = try allocator.dupe(u8, value);
            }
        }
    }
    if ((block_start == null) != (block_end == null)) return error.IncompleteBlockComment;
    if (line_comment) |value| {
        if (value.len == 0) {
            allocator.free(value);
            line_comment = null;
        }
    }
    if (block_start) |value| {
        if (value.len == 0) {
            allocator.free(value);
            block_start = null;
            if (block_end) |end| {
                allocator.free(end);
                block_end = null;
            }
        }
    }

    var quote_characters: std.ArrayList([]u8) = .empty;
    errdefer {
        for (quote_characters.items) |item| allocator.free(item);
        quote_characters.deinit(allocator);
    }
    var escape_character: []u8 = try allocator.alloc(u8, 0);
    var highlight_numbers = true;
    if (root.get("strings")) |strings| {
        if (strings.asObject() != null) {
            var quotes = try strings.stringArray(allocator, "quoteCharacters");
            defer {
                for (quotes.items) |item| allocator.free(item);
                quotes.deinit(allocator);
            }
            for (quotes.items) |quote| {
                if (quote.len == 0) continue;
                try quote_characters.append(allocator, try allocator.dupe(u8, quote));
            }
            if (stringValue(strings.get("escapeCharacter"))) |escape| {
                allocator.free(escape_character);
                escape_character = try allocator.dupe(u8, escape);
            }
        }
    }
    if (root.get("numbers")) |value| {
        if (value.asBool()) |enabled| highlight_numbers = enabled;
    }

    const format_on_save = boolValue(root, "formatOnSave") orelse
        boolValue(root, "format_on_save") orelse false;
    const formatter = try parseFormatter(allocator, root, "formatter");
    var fallback_formatter = try parseFormatter(allocator, root, "fallbackFormatter");
    if (fallback_formatter == null) {
        fallback_formatter = try parseFormatter(allocator, root, "fallback_formatter");
    }
    const fallback_lsp = try parseFallbackLsp(allocator, root, id);
    const indentation = try parseIndentation(root);
    const auto_indent = boolValue(root, "autoIndent") orelse
        boolValue(root, "auto_indent") orelse true;
    const auto_close = boolValue(root, "autoClose") orelse
        boolValue(root, "auto_close") orelse true;
    const traditional_markup = boolValue(root, "traditionalMarkupLanguage") orelse
        boolValue(root, "traditional_markup_language") orelse false;

    var indent_after = try root.stringArray(allocator, "indentAfter");
    try appendStringArray(allocator, &indent_after, root, "indent_after");
    var unindent_before = try root.stringArray(allocator, "unindentBefore");
    try appendStringArray(allocator, &unindent_before, root, "unindent_before");

    return .{
        .allocator = allocator,
        .id = try allocator.dupe(u8, id),
        .name = try allocator.dupe(u8, name),
        .extensions = try normalizeExtensions(allocator, extensions_raw.items),
        .file_names = try normalizeFileNames(allocator, combined_names.items),
        .classifications = classifications,
        .case_insensitive = case_insensitive,
        .line_highlights = try line_highlights.toOwnedSlice(allocator),
        .line_comment = line_comment,
        .block_start = block_start,
        .block_end = block_end,
        .quote_characters = try quote_characters.toOwnedSlice(allocator),
        .escape_character = escape_character,
        .highlight_numbers = highlight_numbers,
        .formatter = formatter,
        .fallback_formatter = fallback_formatter,
        .fallback_lsp = fallback_lsp,
        .format_on_save = format_on_save,
        .indentation = indentation,
        .auto_indent = auto_indent,
        .auto_close = auto_close,
        .traditional_markup = traditional_markup,
        .indent_after = try indent_after.toOwnedSlice(allocator),
        .unindent_before = try unindent_before.toOwnedSlice(allocator),
    };
}

pub const Registry = struct {
    allocator: std.mem.Allocator,
    languages: []Language = &.{},

    pub fn init(allocator: std.mem.Allocator) Registry {
        return .{ .allocator = allocator };
    }

    pub fn deinit(self: *Registry) void {
        for (self.languages) |*language| language.deinit();
        self.allocator.free(self.languages);
        self.* = undefined;
    }

    pub fn load(allocator: std.mem.Allocator, io: std.Io) !Registry {
        var registry = Registry.init(allocator);
        const directory = findGrammarDirectory(allocator, io) orelse {
            registry.languages = try allocator.alloc(Language, 0);
            return registry;
        };
        defer allocator.free(directory);

        var dir = std.Io.Dir.cwd().openDir(io, directory, .{ .iterate = true }) catch {
            registry.languages = try allocator.alloc(Language, 0);
            return registry;
        };
        defer dir.close(io);

        var list: std.ArrayList(Language) = .empty;
        errdefer {
            for (list.items) |*language| language.deinit();
            list.deinit(allocator);
        }
        var iterator = dir.iterate();
        while (try iterator.next(io)) |entry| {
            if (entry.kind != .file) continue;
            if (!std.mem.endsWith(u8, entry.name, ".hexdef")) continue;
            const path = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ directory, entry.name });
            defer allocator.free(path);
            const text = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(4 * 1024 * 1024)) catch continue;
            defer allocator.free(text);
            const language = parseLanguage(allocator, text, entry.name) catch continue;
            try list.append(allocator, language);
        }
        std.mem.sort(Language, list.items, {}, struct {
            fn lessThan(_: void, left: Language, right: Language) bool {
                return std.mem.lessThan(u8, left.id, right.id);
            }
        }.lessThan);
        registry.languages = try list.toOwnedSlice(allocator);
        return registry;
    }

    pub fn find(self: Registry, id: []const u8) ?*const Language {
        for (self.languages) |*language| {
            if (std.ascii.eqlIgnoreCase(language.id, id)) return language;
        }
        return null;
    }

    pub fn findByPath(self: Registry, path: []const u8) ?*const Language {
        for (self.languages) |*language| {
            if (language.matchesExtension(path)) return language;
        }
        return null;
    }

    pub fn findById(self: Registry, id: []const u8) ?*const Language {
        return self.find(id);
    }
};

pub fn findGrammarDirectory(allocator: std.mem.Allocator, io: std.Io) ?[]u8 {
    const candidates = [_][]const u8{
        "data/grammars",
        "../data/grammars",
        "../../data/grammars",
    };
    for (candidates) |candidate| {
        var dir = std.Io.Dir.cwd().openDir(io, candidate, .{}) catch continue;
        dir.close(io);
        return allocator.dupe(u8, candidate) catch null;
    }
    return null;
}

pub const Highlighter = struct {
    const CachedLine = struct { signature: u64, end_in_block: bool };

    cache: std.AutoHashMap(usize, CachedLine),
    allocator: std.mem.Allocator,
    line_buffer: std.ArrayList(u8) = .empty,
    // Scratch token list only used while replaying cached line state.
    tokens_scratch: std.ArrayList(Token) = .empty,

    pub fn init(allocator: std.mem.Allocator) Highlighter {
        return .{
            .cache = std.AutoHashMap(usize, CachedLine).init(allocator),
            .allocator = allocator,
        };
    }

    pub fn deinit(self: *Highlighter) void {
        self.cache.deinit();
        self.line_buffer.deinit(self.allocator);
        self.tokens_scratch.deinit(self.allocator);
        self.* = undefined;
    }

    pub fn clear(self: *Highlighter) void {
        self.cache.clearRetainingCapacity();
    }

    pub fn getLineTokens(
        self: *Highlighter,
        document: anytype,
        language: *const Language,
        line: usize,
        text: []const u8,
        tokens: *std.ArrayList(Token),
    ) !void {
        tokens.clearRetainingCapacity();
        const start_in_block = try self.resolveLineStart(document, language, line);
        const end_in_block = try tokenizeLine(self.allocator, text, language, start_in_block, tokens);
        if (self.cache.count() >= 50_000) self.cache.clearRetainingCapacity();
        try self.cache.put(line, .{ .signature = signature(text), .end_in_block = end_in_block });
    }

    fn resolveLineStart(self: *Highlighter, document: anytype, language: *const Language, line: usize) !bool {
        if (line == 0) return false;
        _ = self.cache.remove(line);
        var found_line: ?usize = null;
        var state = false;
        var search = line - 1;
        var steps: usize = 0;
        while (steps < 512) : (steps += 1) {
            if (self.cache.get(search)) |previous| {
                const previous_text = try document.lineText(search, &self.line_buffer);
                if (previous.signature == signature(previous_text)) {
                    state = previous.end_in_block;
                    found_line = search;
                    break;
                }
                _ = self.cache.remove(search);
            }
            if (search == 0) break;
            search -= 1;
        }
        const from_line = if (found_line) |found| found + 1 else if (line <= 1024) 0 else line;
        var current = from_line;
        while (current < line) : (current += 1) {
            const current_text = try document.lineText(current, &self.line_buffer);
            const end = try tokenizeLine(self.allocator, current_text, language, state, &self.tokens_scratch);
            try self.cache.put(current, .{ .signature = signature(current_text), .end_in_block = end });
            state = end;
        }
        return state;
    }
};

pub fn tokenizeLine(
    allocator: std.mem.Allocator,
    text: []const u8,
    language: *const Language,
    start_in_block: bool,
    tokens: *std.ArrayList(Token),
) !bool {
    var in_block = start_in_block;
    var offset: usize = 0;
    var line_highlight: ?struct { start: usize, end: usize, kind: theme.SyntaxKind } = null;
    if (!in_block) {
        for (language.line_highlights) |highlight| {
            if (highlight.pattern.find(text)) |match| {
                if (match.end > match.start) {
                    line_highlight = .{ .start = match.start, .end = match.end, .kind = highlight.kind };
                    break;
                }
            }
        }
    }

    while (offset < text.len) {
        if (in_block) {
            const end_marker = language.block_end orelse {
                try addToken(allocator, tokens, offset, text.len, .comment);
                break;
            };
            if (findBytes(text, offset, end_marker)) |end| {
                const token_end = end + end_marker.len;
                try addToken(allocator, tokens, offset, token_end, .comment);
                offset = token_end;
                in_block = false;
                continue;
            }
            try addToken(allocator, tokens, offset, text.len, .comment);
            break;
        }

        if (language.block_start) |start_marker| {
            if (language.block_end) |end_marker| {
                if (startsWith(text, offset, start_marker)) {
                    const content_start = offset + start_marker.len;
                    if (findBytes(text, content_start, end_marker)) |end| {
                        const token_end = end + end_marker.len;
                        try addToken(allocator, tokens, offset, token_end, .comment);
                        offset = token_end;
                    } else {
                        try addToken(allocator, tokens, offset, text.len, .comment);
                        in_block = true;
                        break;
                    }
                    continue;
                }
            }
        }

        if (language.line_comment) |marker| {
            if (startsWith(text, offset, marker)) {
                try addToken(allocator, tokens, offset, text.len, .comment);
                break;
            }
        }

        if (matchQuote(text, offset, language.quote_characters)) |quote_length| {
            const start = offset;
            offset += quote_length;
            while (offset < text.len) {
                if (language.escape_character.len > 0 and
                    startsWith(text, offset, language.escape_character))
                {
                    offset += language.escape_character.len;
                    skipRune(text, &offset);
                    continue;
                }
                if (matchQuote(text, offset, language.quote_characters)) |closing| {
                    offset += closing;
                    break;
                }
                skipRune(text, &offset);
            }
            try addToken(allocator, tokens, start, offset, .string);
            continue;
        }

        if (language.highlight_numbers and std.ascii.isDigit(text[offset])) {
            const start = offset;
            consumeNumber(text, &offset);
            try addToken(allocator, tokens, start, offset, .number);
            continue;
        }

        if (isIdentifierStart(text[offset])) {
            const start = offset;
            offset += 1;
            while (offset < text.len and isIdentifierPart(text[offset])) : (offset += 1) {}
            const kind = language.classify(text[start..offset]);
            try addToken(allocator, tokens, start, offset, kind);
            continue;
        }

        skipRune(text, &offset);
    }

    if (line_highlight) |highlight| {
        try mergeLineHighlight(allocator, tokens, highlight.start, highlight.end, highlight.kind);
    }
    return in_block;
}

fn addToken(
    allocator: std.mem.Allocator,
    tokens: *std.ArrayList(Token),
    start: usize,
    end: usize,
    kind: theme.SyntaxKind,
) !void {
    if (end > start and kind != .default) {
        try tokens.append(allocator, .{ .start = start, .end = end, .kind = kind });
    }
}

fn mergeLineHighlight(
    allocator: std.mem.Allocator,
    tokens: *std.ArrayList(Token),
    highlight_start: usize,
    highlight_end: usize,
    kind: theme.SyntaxKind,
) !void {
    var merged: std.ArrayList(Token) = .empty;
    errdefer merged.deinit(allocator);
    var index: usize = 0;
    while (index < tokens.items.len and tokens.items[index].end <= highlight_start) : (index += 1) {
        try merged.append(allocator, tokens.items[index]);
    }
    while (index < tokens.items.len and tokens.items[index].start < highlight_end) : (index += 1) {}
    try merged.append(allocator, .{ .start = highlight_start, .end = highlight_end, .kind = kind });
    while (index < tokens.items.len) : (index += 1) {
        try merged.append(allocator, tokens.items[index]);
    }
    tokens.clearRetainingCapacity();
    try tokens.appendSlice(allocator, merged.items);
    merged.deinit(allocator);
}

fn signature(text: []const u8) u64 {
    var hash: u64 = 0xcbf29ce484222325;
    for (text) |byte| {
        hash ^= byte;
        hash *%= 0x100000001b3;
    }
    return hash;
}

fn startsWith(text: []const u8, offset: usize, marker: []const u8) bool {
    return marker.len > 0 and offset <= text.len and
        marker.len <= text.len - offset and
        std.mem.eql(u8, text[offset .. offset + marker.len], marker);
}

fn findBytes(text: []const u8, offset: usize, marker: []const u8) ?usize {
    if (marker.len == 0 or offset >= text.len) return null;
    return std.mem.indexOfPos(u8, text, offset, marker);
}

fn matchQuote(text: []const u8, offset: usize, quotes: []const []u8) ?usize {
    for (quotes) |quote| {
        if (startsWith(text, offset, quote)) return quote.len;
    }
    return null;
}

fn skipRune(text: []const u8, offset: *usize) void {
    if (offset.* >= text.len) return;
    const length = std.unicode.utf8ByteSequenceLength(text[offset.*]) catch 1;
    offset.* = @min(text.len, offset.* + @max(length, 1));
}

fn consumeNumber(text: []const u8, offset: *usize) void {
    var hexadecimal = false;
    var binary = false;
    if (offset.* + 1 < text.len and text[offset.*] == '0') {
        const prefix = text[offset.* + 1];
        hexadecimal = prefix == 'x' or prefix == 'X';
        binary = prefix == 'b' or prefix == 'B';
    }
    if (hexadecimal or binary) {
        offset.* += 2;
        while (offset.* < text.len) {
            const value = text[offset.*];
            if (std.ascii.isDigit(value) or (hexadecimal and std.ascii.isHex(value)) or value == '_') {
                offset.* += 1;
                continue;
            }
            break;
        }
        return;
    }
    while (offset.* < text.len) {
        const value = text[offset.*];
        if (std.ascii.isDigit(value) or value == '.' or value == '_') {
            offset.* += 1;
            continue;
        }
        if (value == 'e' or value == 'E') {
            offset.* += 1;
            if (offset.* < text.len and (text[offset.*] == '+' or text[offset.*] == '-')) {
                offset.* += 1;
            }
            continue;
        }
        break;
    }
}

fn isIdentifierStart(byte: u8) bool {
    return std.ascii.isAlphabetic(byte) or byte == '_' or byte >= 0x80;
}

fn isIdentifierPart(byte: u8) bool {
    return isIdentifierStart(byte) or std.ascii.isDigit(byte);
}

test "parses a grammar and tokenizes with block state" {
    const allocator = std.testing.allocator;
    const text =
        \\{
        \\  "id": "c", "name": "C", "extensions": [".c"],
        \\  "keywords": ["struct"], "types": ["int"],
        \\  "comments": { "line": "//", "blockStart": "/*", "blockEnd": "*/" },
        \\  "strings": { "quoteCharacters": ["\""], "escapeCharacter": "\\" },
        \\  "linePatterns": [{ "pattern": "^\\+.*", "kind": "added" }]
        \\}
    ;
    var language = try parseLanguage(allocator, text, "c.hexdef");
    defer language.deinit();
    var tokens: std.ArrayList(Token) = .empty;
    defer tokens.deinit(allocator);
    const ends = try tokenizeLine(allocator, "int x; /* hi", &language, false, &tokens);
    try std.testing.expect(ends);
    try std.testing.expect(tokens.items.len >= 2);
    tokens.clearRetainingCapacity();
    _ = try tokenizeLine(allocator, "still comment */ struct", &language, true, &tokens);
    try std.testing.expect(tokens.items[0].kind == .comment);
}

test "loads the bundled grammar registry" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var registry = try Registry.load(allocator, threaded.io());
    defer registry.deinit();
    try std.testing.expect(registry.languages.len >= 100);
    try std.testing.expect(registry.find("cpp") != null);
    try std.testing.expect(registry.findByPath("src/main.zig") != null);
    try std.testing.expect(registry.findByPath("notes.md") != null);
    const diff = registry.find("diff") orelse return error.MissingDiffGrammar;
    try std.testing.expect(diff.line_highlights.len > 0);
}

test "every bundled grammar parses" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    const io = threaded.io();
    var dir = try std.Io.Dir.cwd().openDir(io, "data/grammars", .{ .iterate = true });
    defer dir.close(io);
    var iterator = dir.iterate();
    var failures: usize = 0;
    while (try iterator.next(io)) |entry| {
        if (entry.kind != .file or !std.mem.endsWith(u8, entry.name, ".hexdef")) continue;
        const path = try std.fmt.allocPrint(allocator, "data/grammars/{s}", .{entry.name});
        defer allocator.free(path);
        const text = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(4 * 1024 * 1024)) catch continue;
        defer allocator.free(text);
        var language = parseLanguage(allocator, text, entry.name) catch |err| {
            std.debug.print("grammar {s} failed: {s}\n", .{ entry.name, @errorName(err) });
            failures += 1;
            continue;
        };
        language.deinit();
    }
    try std.testing.expectEqual(@as(usize, 0), failures);
}
