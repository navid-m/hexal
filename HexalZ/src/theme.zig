//! Named colour palettes and named styles.
//!
//! Colours are ANSI palette indexes (0-255) or packed 24-bit RGB values marked
//! with `TRUE_COLOR_FLAG`. Themes are the JSON files in `data/themes`.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const json = @import("json.zig");

pub const TRUE_COLOR_FLAG: u32 = 0x0100_0000;

pub fn rgb(red: u8, green: u8, blue: u8) u32 {
    return TRUE_COLOR_FLAG | (@as(u32, red) << 16) | (@as(u32, green) << 8) | @as(u32, blue);
}

pub fn isTrueColor(color: u32) bool {
    return color & TRUE_COLOR_FLAG != 0;
}

pub const Style = struct {
    fg: u32 = 15,
    bg: u32 = 0,
    bold: bool = false,
    dim: bool = false,
    reverse: bool = false,

    pub fn eql(a: Style, b: Style) bool {
        return a.fg == b.fg and a.bg == b.bg and a.bold == b.bold and
            a.dim == b.dim and a.reverse == b.reverse;
    }

    pub fn withBg(self: Style, bg: u32) Style {
        var result = self;
        result.bg = bg;
        return result;
    }
};

pub const SyntaxKind = enum {
    default,
    keyword,
    control,
    type_,
    constant,
    string,
    comment,
    number,
    added,
    deleted,
};

pub const Theme = struct {
    id: []u8 = &.{},
    name: []u8 = &.{},
    description: []u8 = &.{},
    author: []u8 = &.{},
    license: []u8 = &.{},

    foreground: u32 = 15,
    background: u32 = 0,
    muted: u32 = 8,
    dimmed: u32 = 8,
    accent: u32 = 14,
    border: u32 = 8,
    err_color: u32 = 9,
    warning: u32 = 11,
    success: u32 = 10,

    diff_added: u32 = 2,
    diff_deleted: u32 = 1,
    diff_modified: u32 = 4,
    git_modified: u32 = 14,

    alert_foreground: u32 = 15,
    alert_background: u32 = 5,
    selection_foreground: u32 = 15,
    selection_background: u32 = 4,
    status_foreground: u32 = 15,
    status_background: u32 = 4,
    insert_foreground: u32 = 0,
    insert_background: u32 = 2,
    select_foreground: u32 = 0,
    select_background: u32 = 3,
    prompt_foreground: u32 = 0,
    prompt_background: u32 = 6,
    command_foreground: u32 = 0,
    command_background: u32 = 6,
    match_foreground: u32 = 0,
    match_background: u32 = 3,
    current_match_foreground: u32 = 0,
    current_match_background: u32 = 11,

    keyword: u32 = 13,
    control: u32 = 12,
    type_: u32 = 14,
    constant: u32 = 11,
    string: u32 = 11,
    comment: u32 = 10,
    number: u32 = 11,

    pub fn deinit(self: *Theme, allocator: std.mem.Allocator) void {
        if (self.id.len > 0) allocator.free(self.id);
        if (self.name.len > 0) allocator.free(self.name);
        if (self.description.len > 0) allocator.free(self.description);
        if (self.author.len > 0) allocator.free(self.author);
        if (self.license.len > 0) allocator.free(self.license);
        self.* = undefined;
    }

    pub fn base(self: Theme) Style {
        return .{ .fg = self.foreground, .bg = self.background };
    }

    pub fn text(self: Theme) Style {
        return self.base();
    }

    pub fn mutedStyle(self: Theme) Style {
        return .{ .fg = self.muted, .bg = self.background, .dim = true };
    }

    pub fn accentStyle(self: Theme) Style {
        return .{ .fg = self.accent, .bg = self.background, .bold = true };
    }

    pub fn borderStyle(self: Theme) Style {
        return .{ .fg = self.border, .bg = self.background, .dim = true };
    }

    pub fn errorStyle(self: Theme) Style {
        return .{ .fg = self.err_color, .bg = self.background, .bold = true };
    }

    pub fn warningStyle(self: Theme) Style {
        return .{ .fg = self.warning, .bg = self.background, .bold = true };
    }

    pub fn successStyle(self: Theme) Style {
        return .{ .fg = self.success, .bg = self.background, .bold = true };
    }

    pub fn selectionStyle(self: Theme) Style {
        return .{ .fg = self.selection_foreground, .bg = self.selection_background };
    }

    pub fn selectedItemStyle(self: Theme) Style {
        return .{ .fg = self.selection_foreground, .bg = self.selection_background, .bold = true };
    }

    pub fn statusStyle(self: Theme) Style {
        return .{ .fg = self.status_foreground, .bg = self.status_background, .bold = true };
    }

    pub fn insertStatusStyle(self: Theme) Style {
        return .{ .fg = self.insert_foreground, .bg = self.insert_background, .bold = true };
    }

    pub fn selectStatusStyle(self: Theme) Style {
        return .{ .fg = self.select_foreground, .bg = self.select_background, .bold = true };
    }

    pub fn promptStatusStyle(self: Theme) Style {
        return .{ .fg = self.prompt_foreground, .bg = self.prompt_background, .bold = true };
    }

    pub fn commandStatusStyle(self: Theme) Style {
        return .{ .fg = self.command_foreground, .bg = self.command_background, .bold = true };
    }

    pub fn alertStyle(self: Theme) Style {
        return .{ .fg = self.alert_foreground, .bg = self.alert_background, .bold = true };
    }

    pub fn matchStyle(self: Theme) Style {
        return .{ .fg = self.match_foreground, .bg = self.match_background };
    }

    pub fn currentMatchStyle(self: Theme) Style {
        return .{
            .fg = self.current_match_foreground,
            .bg = self.current_match_background,
            .bold = true,
        };
    }

    pub fn currentLineNumberStyle(self: Theme) Style {
        return .{ .fg = self.foreground, .bg = self.background, .bold = true };
    }

    pub fn gitModifiedStyle(self: Theme) Style {
        return .{ .fg = self.git_modified, .bg = self.background, .bold = true };
    }

    pub fn diffModifiedStyle(self: Theme) Style {
        return .{ .fg = self.diff_modified, .bg = self.background, .bold = true };
    }

    pub fn syntaxStyle(self: Theme, kind: SyntaxKind) Style {
        const fg = switch (kind) {
            .keyword => self.keyword,
            .control => self.control,
            .type_ => self.type_,
            .constant => self.constant,
            .string => self.string,
            .comment => self.comment,
            .number => self.number,
            .added => self.diff_added,
            .deleted => self.diff_deleted,
            .default => self.foreground,
        };
        return .{ .fg = fg, .bg = self.background };
    }

    /// Resolve a plugin panel style name. Unknown styles fall back to text.
    pub fn namedStyle(self: Theme, name: []const u8) Style {
        if (std.mem.eql(u8, name, "text") or std.mem.eql(u8, name, "default")) return self.text();
        if (std.mem.eql(u8, name, "muted")) return self.mutedStyle();
        if (std.mem.eql(u8, name, "accent")) return self.accentStyle();
        if (std.mem.eql(u8, name, "success")) return self.successStyle();
        if (std.mem.eql(u8, name, "warning")) return self.warningStyle();
        if (std.mem.eql(u8, name, "error")) return self.errorStyle();
        if (std.mem.eql(u8, name, "keyword")) return self.syntaxStyle(.keyword);
        if (std.mem.eql(u8, name, "control")) return self.syntaxStyle(.control);
        if (std.mem.eql(u8, name, "type")) return self.syntaxStyle(.type_);
        if (std.mem.eql(u8, name, "constant")) return self.syntaxStyle(.constant);
        if (std.mem.eql(u8, name, "string")) return self.syntaxStyle(.string);
        if (std.mem.eql(u8, name, "comment")) return self.syntaxStyle(.comment);
        if (std.mem.eql(u8, name, "number")) return self.syntaxStyle(.number);
        if (std.mem.eql(u8, name, "diff_added") or std.mem.eql(u8, name, "diffAdded")) {
            return self.syntaxStyle(.added);
        }
        if (std.mem.eql(u8, name, "diff_deleted") or std.mem.eql(u8, name, "diffDeleted")) {
            return self.syntaxStyle(.deleted);
        }
        if (std.mem.eql(u8, name, "diff_modified") or std.mem.eql(u8, name, "diffModified")) {
            return self.diffModifiedStyle();
        }
        return self.text();
    }
};

pub const Registry = struct {
    allocator: std.mem.Allocator,
    themes: []Theme = &.{},
    active: Theme = .{},

    pub fn init(allocator: std.mem.Allocator) Registry {
        return .{ .allocator = allocator };
    }

    pub fn deinit(self: *Registry) void {
        for (self.themes) |*theme| theme.deinit(self.allocator);
        self.allocator.free(self.themes);
        self.active.deinit(self.allocator);
        self.* = undefined;
    }

    pub fn load(allocator: std.mem.Allocator, io: std.Io, wanted: []const u8) !Registry {
        var registry = Registry.init(allocator);
        var list: std.ArrayList(Theme) = .empty;
        errdefer {
            for (list.items) |*theme| theme.deinit(allocator);
            list.deinit(allocator);
        }

        if (findThemeDirectory(allocator, io)) |directory| {
            defer allocator.free(directory);
            var dir = std.Io.Dir.cwd().openDir(io, directory, .{ .iterate = true }) catch {
                registry.themes = try allocator.alloc(Theme, 0);
                registry.active = .{};
                return registry;
            };
            defer dir.close(io);
            var names: std.ArrayList([]u8) = .empty;
            defer {
                for (names.items) |name| allocator.free(name);
                names.deinit(allocator);
            }
            var iterator = dir.iterate();
            while (try iterator.next(io)) |entry| {
                if (entry.kind != .file) continue;
                if (!std.mem.endsWith(u8, entry.name, ".json")) continue;
                try names.append(allocator, try allocator.dupe(u8, entry.name));
            }
            std.mem.sort([]u8, names.items, {}, struct {
                fn lessThan(_: void, left: []u8, right: []u8) bool {
                    return std.mem.lessThan(u8, left, right);
                }
            }.lessThan);
            for (names.items) |name| {
                const path = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ directory, name });
                defer allocator.free(path);
                const text = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024)) catch continue;
                defer allocator.free(text);
                const theme = parseTheme(allocator, text, name) catch continue;
                try list.append(allocator, theme);
            }
        } else {
            // Fall back to the default palette when the data directory is not
            // available (for example when running the installed binary).
        }

        if (list.items.len == 0) {
            try list.append(allocator, defaultTheme(allocator));
        }
        registry.themes = try list.toOwnedSlice(allocator);

        const active_id = if (wanted.len > 0) wanted else "hexal-dark";
        for (registry.themes) |theme| {
            if (std.ascii.eqlIgnoreCase(theme.id, active_id)) {
                registry.active = try cloneTheme(allocator, theme);
                return registry;
            }
        }
        registry.active = try cloneTheme(allocator, registry.themes[0]);
        return registry;
    }

    pub fn find(self: Registry, id: []const u8) ?*const Theme {
        for (self.themes) |*theme| {
            if (std.ascii.eqlIgnoreCase(theme.id, id)) return theme;
        }
        return null;
    }

    pub fn setActive(self: *Registry, id: []const u8) bool {
        const theme = self.find(id) orelse return false;
        self.active.deinit(self.allocator);
        self.active = cloneTheme(self.allocator, theme.*) catch return false;
        return true;
    }
};

fn defaultTheme(allocator: std.mem.Allocator) Theme {
    return .{
        .id = allocator.dupe(u8, "hexal-dark") catch &.{},
        .name = allocator.dupe(u8, "Hexal Dark") catch &.{},
        .description = allocator.dupe(u8, "Default Hexal theme.") catch &.{},
    };
}

fn cloneTheme(allocator: std.mem.Allocator, theme: Theme) !Theme {
    var copy = theme;
    copy.id = try allocator.dupe(u8, theme.id);
    copy.name = try allocator.dupe(u8, theme.name);
    copy.description = try allocator.dupe(u8, theme.description);
    copy.author = try allocator.dupe(u8, theme.author);
    copy.license = try allocator.dupe(u8, theme.license);
    return copy;
}

pub fn findThemeDirectory(allocator: std.mem.Allocator, io: std.Io) ?[]u8 {
    const candidates = [_][]const u8{
        "data/themes",
        "../data/themes",
        "../../data/themes",
    };
    for (candidates) |candidate| {
        var dir = std.Io.Dir.cwd().openDir(io, candidate, .{}) catch continue;
        dir.close(io);
        return allocator.dupe(u8, candidate) catch null;
    }

    if (std.process.executableDirPathAlloc(io, allocator)) |executable_dir| {
        defer allocator.free(executable_dir);
        var current: ?[]const u8 = executable_dir;
        var levels: usize = 0;
        while (current) |directory| : (levels += 1) {
            if (levels > 3) break;
            const candidate = std.fs.path.join(allocator, &.{ directory, "data/themes" }) catch break;
            defer allocator.free(candidate);
            var dir = std.Io.Dir.cwd().openDir(io, candidate, .{}) catch {
                current = std.fs.path.dirname(directory);
                continue;
            };
            dir.close(io);
            return allocator.dupe(u8, candidate) catch null;
        }
    } else |_| {}

    if (std.c.getenv("HOME")) |home_ptr| {
        const home = std.mem.span(home_ptr);
        const candidate = std.fs.path.join(allocator, &.{ home, ".hexal", "data", "themes" }) catch return null;
        defer allocator.free(candidate);
        var dir = std.Io.Dir.cwd().openDir(io, candidate, .{}) catch return null;
        dir.close(io);
        return allocator.dupe(u8, candidate) catch null;
    }
    return null;
}

pub fn parseTheme(allocator: std.mem.Allocator, text: []const u8, source_name: []const u8) !Theme {
    var root = try json.parse(allocator, text);
    defer root.deinit(allocator);
    if (root.asObject() == null) return error.RootMustBeObject;

    const fallback_id = std.fs.path.stem(source_name);
    const id_value = if (root.get("id")) |value| value.asString() orelse fallback_id else fallback_id;
    const id = std.mem.trim(u8, id_value, " \t\r\n");
    const name_value = if (root.get("name")) |value|
        value.asString() orelse if (id.len > 0) id else "theme"
    else if (id.len > 0)
        id
    else
        "theme";
    var theme = Theme{
        .id = try allocator.dupe(u8, if (id.len > 0) id else "theme"),
        .name = try allocator.dupe(u8, std.mem.trim(u8, name_value, " \t\r\n")),
        .description = try allocator.dupe(u8, std.mem.trim(
            u8,
            if (root.get("description")) |value| value.asString() orelse "" else "",
            " \t\r\n",
        )),
        .author = try allocator.dupe(u8, std.mem.trim(
            u8,
            if (root.get("author")) |value| value.asString() orelse "" else "",
            " \t\r\n",
        )),
        .license = try allocator.dupe(u8, std.mem.trim(
            u8,
            if (root.get("license")) |value| value.asString() orelse "" else "",
            " \t\r\n",
        )),
    };
    errdefer theme.deinit(allocator);

    const colors = root.get("colors") orelse return theme;
    if (colors.asObject() == null) return error.InvalidColors;
    theme.foreground = try colorField(colors, "foreground", theme.foreground);
    theme.background = try colorField(colors, "background", theme.background);
    theme.muted = try colorField(colors, "muted", theme.muted);
    theme.dimmed = try colorField(colors, "dimmed", theme.muted);
    theme.accent = try colorField(colors, "accent", theme.accent);
    theme.border = try colorField(colors, "border", theme.border);
    theme.err_color = try colorField(colors, "error", theme.err_color);
    theme.warning = try colorField(colors, "warning", theme.warning);
    theme.success = try colorField(colors, "success", theme.success);
    theme.git_modified = try colorField(colors, "gitModified", theme.git_modified);

    try pairField(colors, "alert", &theme.alert_foreground, &theme.alert_background);
    try pairField(colors, "selection", &theme.selection_foreground, &theme.selection_background);
    try pairField(colors, "status", &theme.status_foreground, &theme.status_background);
    try pairField(colors, "insert", &theme.insert_foreground, &theme.insert_background);
    try pairField(colors, "select", &theme.select_foreground, &theme.select_background);
    try pairField(colors, "prompt", &theme.prompt_foreground, &theme.prompt_background);
    try pairField(colors, "command", &theme.command_foreground, &theme.command_background);
    try pairField(colors, "match", &theme.match_foreground, &theme.match_background);
    try pairField(colors, "currentMatch", &theme.current_match_foreground, &theme.current_match_background);

    if (colors.get("syntax")) |syntax| {
        if (syntax.asObject() == null) return error.InvalidSyntax;
        theme.keyword = try colorField(syntax, "keyword", theme.keyword);
        theme.control = try colorField(syntax, "control", theme.control);
        theme.type_ = try colorField(syntax, "type", theme.type_);
        theme.constant = try colorField(syntax, "constant", theme.constant);
        theme.string = try colorField(syntax, "string", theme.string);
        theme.comment = try colorField(syntax, "comment", theme.comment);
        theme.number = try colorField(syntax, "number", theme.number);
    }
    return theme;
}

fn pairField(colors: json.Value, key: []const u8, fg: *u32, bg: *u32) !void {
    const pair = colors.get(key) orelse return;
    if (pair.asObject() == null) return error.InvalidColorPair;
    fg.* = try colorField(pair, "fg", fg.*);
    bg.* = try colorField(pair, "bg", bg.*);
}

fn colorField(value: json.Value, key: []const u8, fallback: u32) !u32 {
    const entry = value.get(key) orelse return fallback;
    return parseColor(entry) orelse error.InvalidColor;
}

pub fn parseColor(value: json.Value) ?u32 {
    if (value.asI64()) |number| {
        if (number >= 0 and number <= 255) return @intCast(number);
        return null;
    }
    const text = std.mem.trim(u8, value.asString() orelse return null, " \t\r\n");
    if (text.len == 0) return null;
    if (text[0] == '#') {
        var hex = text[1..];
        var expanded: [6]u8 = undefined;
        if (hex.len == 3) {
            expanded[0] = hex[0];
            expanded[1] = hex[0];
            expanded[2] = hex[1];
            expanded[3] = hex[1];
            expanded[4] = hex[2];
            expanded[5] = hex[2];
            hex = &expanded;
        }
        if (hex.len != 6) return null;
        const red = std.fmt.parseInt(u8, hex[0..2], 16) catch return null;
        const green = std.fmt.parseInt(u8, hex[2..4], 16) catch return null;
        const blue = std.fmt.parseInt(u8, hex[4..6], 16) catch return null;
        return rgb(red, green, blue);
    }
    if (std.fmt.parseInt(i64, text, 10)) |index| {
        if (index >= 0 and index <= 255) return @intCast(index);
        return null;
    } else |_| {}
    return namedColor(text);
}

fn namedColor(text: []const u8) ?u32 {
    var buffer: [32]u8 = undefined;
    if (text.len > buffer.len) return null;
    const lower = std.ascii.lowerString(buffer[0..text.len], text);
    const names = [_]struct { name: []const u8, index: u32 }{
        .{ .name = "black", .index = 0 },
        .{ .name = "red", .index = 1 },
        .{ .name = "green", .index = 2 },
        .{ .name = "yellow", .index = 3 },
        .{ .name = "blue", .index = 4 },
        .{ .name = "magenta", .index = 5 },
        .{ .name = "cyan", .index = 6 },
        .{ .name = "white", .index = 7 },
        .{ .name = "bright-black", .index = 8 },
        .{ .name = "gray", .index = 8 },
        .{ .name = "grey", .index = 8 },
        .{ .name = "bright-red", .index = 9 },
        .{ .name = "bright-green", .index = 10 },
        .{ .name = "bright-yellow", .index = 11 },
        .{ .name = "bright-blue", .index = 12 },
        .{ .name = "bright-magenta", .index = 13 },
        .{ .name = "bright-cyan", .index = 14 },
        .{ .name = "bright-white", .index = 15 },
    };
    for (names) |name| {
        if (std.mem.eql(u8, lower, name.name)) return name.index;
    }
    return null;
}

test "parses true colour and named colours" {
    const allocator = std.testing.allocator;
    const text =
        \\{
        \\  "id": "gruvbox", "name": "Gruvbox",
        \\  "colors": {
        \\    "foreground": "#ebdbb2",
        \\    "selection": { "fg": "#fbf1c7", "bg": "#458588" },
        \\    "syntax": { "keyword": "#fb4934", "type": "yellow" }
        \\  }
        \\}
    ;
    var theme = try parseTheme(allocator, text, "gruvbox.json");
    defer theme.deinit(allocator);
    try std.testing.expectEqual(rgb(0xeb, 0xdb, 0xb2), theme.foreground);
    try std.testing.expectEqual(rgb(0x45, 0x85, 0x88), theme.selection_background);
    try std.testing.expectEqual(@as(u32, 3), theme.type_);
}

test "loads the bundled theme registry" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var registry = try Registry.load(allocator, threaded.io(), "hexal-dark");
    defer registry.deinit();
    try std.testing.expect(registry.themes.len >= 5);
    try std.testing.expect(std.mem.eql(u8, registry.active.id, "hexal-dark"));
    try std.testing.expect(registry.find("gruvbox") != null);
    try std.testing.expect(isTrueColor(registry.active.foreground));
}
