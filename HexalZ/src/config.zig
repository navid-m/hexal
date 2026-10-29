//! Persistent `.hexcfg` configuration.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const json = @import("json.zig");

pub const DEFAULT_FILE_NAME = ".hexcfg";
pub const DEFAULT_CENTRAL_REPOSITORY_URL = "https://github.com/navid-m/hexal-plugins";

pub const DEFAULT_CONTENTS =
    \\{
    \\  "version": 1,
    \\  "centralRepositoryUrl": "https://github.com/navid-m/hexal-plugins",
    \\  "theme": "hexal-dark",
    \\  "formatOnSave": false,
    \\  "trimTrailingWhitespaceOnSave": false,
    \\  "smartBackspace": true,
    \\  "autoDetectIndentationOnStartup": true,
    \\  "startupBenchmarker": false,
    \\  "syntaxHighlightPickerPreview": false,
    \\  "debugAdapter": true,
    \\  "showDiffLines": true,
    \\  "diagnosticLineHighlight": true,
    \\  "showGitIgnored": false,
    \\  "showModifiedIndicators": true,
    \\  "wordWrap": false,
    \\  "keepSidebarFocusOnOpen": true,
    \\  "confirmQuitWithoutSaving": true,
    \\  "searchMatchCaseEnabled": false,
    \\  "searchSkipFilesOverSizeMb": 4
    \\}
    \\
;

pub const Config = struct {
    allocator: std.mem.Allocator,
    source_path: []u8,
    central_repository_url: []u8,
    theme: []u8,
    format_on_save: bool = false,
    trim_trailing_whitespace_on_save: bool = false,
    smart_backspace: bool = true,
    auto_detect_indentation_on_startup: bool = true,
    startup_benchmarker: bool = false,
    syntax_highlight_picker_preview: bool = false,
    debug_adapter: bool = true,
    show_diff_lines: bool = true,
    diagnostic_line_highlight: bool = true,
    show_git_ignored: bool = false,
    show_modified_indicators: bool = true,
    word_wrap: bool = false,
    keep_sidebar_focus_on_open: bool = true,
    confirm_quit_without_saving: bool = true,
    search_match_case_enabled: bool = false,
    search_skip_files_over_size_mb: u64 = 4,

    pub fn init(allocator: std.mem.Allocator, source_path: []const u8) !Config {
        return .{
            .allocator = allocator,
            .source_path = try allocator.dupe(u8, source_path),
            .central_repository_url = try allocator.dupe(u8, DEFAULT_CENTRAL_REPOSITORY_URL),
            .theme = try allocator.dupe(u8, "hexal-dark"),
        };
    }

    pub fn deinit(self: *Config) void {
        self.allocator.free(self.source_path);
        self.allocator.free(self.central_repository_url);
        self.allocator.free(self.theme);
        self.* = undefined;
    }
};

pub fn loadDefault(
    allocator: std.mem.Allocator,
    io: std.Io,
    environ: *const std.process.Environ.Map,
) !Config {
    const path = try resolvePath(allocator, io, environ);
    defer allocator.free(path);
    return load(allocator, io, path);
}

pub fn load(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !Config {
    const text = std.Io.Dir.cwd().readFileAlloc(io, path, allocator, .limited(1024 * 1024)) catch |err| switch (err) {
        error.FileNotFound => return Config.init(allocator, path),
        else => return err,
    };
    defer allocator.free(text);
    return parse(allocator, text, path);
}

pub fn parse(allocator: std.mem.Allocator, text: []const u8, path: []const u8) !Config {
    var root = try json.parse(allocator, text);
    defer root.deinit(allocator);
    if (root.asObject() == null) return error.InvalidConfiguration;

    var config = try Config.init(allocator, path);
    errdefer config.deinit();

    if (readString(root, "centralRepositoryUrl", "central_repository_url")) |value| {
        if (value.len > 0) {
            allocator.free(config.central_repository_url);
            config.central_repository_url = try allocator.dupe(u8, value);
        }
    }
    if (readString(root, "theme", "theme")) |value| {
        if (value.len > 0) {
            allocator.free(config.theme);
            config.theme = try allocator.dupe(u8, value);
        }
    }
    config.format_on_save = readBool(root, "formatOnSave", "format_on_save") orelse false;
    config.trim_trailing_whitespace_on_save = readBool(
        root,
        "trimTrailingWhitespaceOnSave",
        "trim_trailing_whitespace_on_save",
    ) orelse false;
    config.smart_backspace = readBool(root, "smartBackspace", "smart_backspace") orelse true;
    config.auto_detect_indentation_on_startup = readBool(
        root,
        "autoDetectIndentationOnStartup",
        "auto_detect_indentation_on_startup",
    ) orelse true;
    config.startup_benchmarker = readBool(root, "startupBenchmarker", "startup_benchmarker") orelse false;
    config.syntax_highlight_picker_preview = readBool(
        root,
        "syntaxHighlightPickerPreview",
        "syntax_highlight_picker_preview",
    ) orelse false;
    config.debug_adapter = readBool(root, "debugAdapter", "debug_adapter") orelse true;
    config.show_diff_lines = readBool(root, "showDiffLines", "show_diff_lines") orelse true;
    config.diagnostic_line_highlight = readBool(
        root,
        "diagnosticLineHighlight",
        "diagnostic_line_highlight",
    ) orelse true;
    config.show_git_ignored = readBool(root, "showGitIgnored", "show_git_ignored") orelse false;
    config.show_modified_indicators = readBool(
        root,
        "showModifiedIndicators",
        "show_modified_indicators",
    ) orelse true;
    config.word_wrap = readBool(root, "wordWrap", "word_wrap") orelse false;
    config.keep_sidebar_focus_on_open = readBool(
        root,
        "keepSidebarFocusOnOpen",
        "keep_sidebar_focus_on_open",
    ) orelse true;
    config.confirm_quit_without_saving = readBool(
        root,
        "confirmQuitWithoutSaving",
        "confirm_quit_without_saving",
    ) orelse true;
    config.search_match_case_enabled = readBool(
        root,
        "searchMatchCaseEnabled",
        "search_match_case_enabled",
    ) orelse false;
    config.search_skip_files_over_size_mb = readSize(
        root,
        "searchSkipFilesOverSizeMb",
        "search_skip_files_over_size_mb",
    ) orelse 4;
    return config;
}

fn readString(root: json.Value, name: []const u8, alias: []const u8) ?[]const u8 {
    if (root.get(name)) |value| if (value.asString()) |text| return text;
    if (root.get(alias)) |value| if (value.asString()) |text| return text;
    if (root.get("editor")) |editor| {
        if (editor.get(name)) |value| if (value.asString()) |text| return text;
        if (editor.get(alias)) |value| if (value.asString()) |text| return text;
    }
    return null;
}

fn readBool(root: json.Value, name: []const u8, alias: []const u8) ?bool {
    if (root.get(name)) |value| if (value.asBool()) |flag| return flag;
    if (root.get(alias)) |value| if (value.asBool()) |flag| return flag;
    if (root.get("editor")) |editor| {
        if (editor.get(name)) |value| if (value.asBool()) |flag| return flag;
        if (editor.get(alias)) |value| if (value.asBool()) |flag| return flag;
    }
    return null;
}

fn readSize(root: json.Value, name: []const u8, alias: []const u8) ?u64 {
    const value = readNumber(root, name, alias) orelse return null;
    if (value < 0) return null;
    return @intFromFloat(value);
}

fn readNumber(root: json.Value, name: []const u8, alias: []const u8) ?f64 {
    if (root.get(name)) |value| if (value.asNumber()) |number| return number;
    if (root.get(alias)) |value| if (value.asNumber()) |number| return number;
    if (root.get("editor")) |editor| {
        if (editor.get(name)) |value| if (value.asNumber()) |number| return number;
        if (editor.get(alias)) |value| if (value.asNumber()) |number| return number;
    }
    return null;
}

pub fn resolvePath(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) ![]u8 {
    const home = environ.get("HOME") orelse environ.get("USERPROFILE") orelse ".";
    const directory = try std.fs.path.join(allocator, &.{ home, ".hexal" });
    defer allocator.free(directory);
    const primary = try std.fs.path.join(allocator, &.{ directory, DEFAULT_FILE_NAME });
    std.Io.Dir.cwd().access(io, primary, .{}) catch {
        const alternate = try std.fs.path.join(allocator, &.{ directory, "hexcfg" });
        if (fileExists(io, alternate)) {
            allocator.free(primary);
            return alternate;
        }
        allocator.free(alternate);
    };
    return primary;
}

pub fn ensureDefaultFile(
    allocator: std.mem.Allocator,
    io: std.Io,
    environ: *const std.process.Environ.Map,
) ![]u8 {
    const path = try resolvePath(allocator, io, environ);
    errdefer allocator.free(path);
    const directory = std.fs.path.dirname(path) orelse return path;
    std.Io.Dir.cwd().createDirPath(io, directory) catch |err| switch (err) {
        error.PathAlreadyExists => {},
        else => return err,
    };
    if (!fileExists(io, path)) {
        try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = path, .data = DEFAULT_CONTENTS });
    }
    return path;
}

pub fn hexalHomeDirectory(
    allocator: std.mem.Allocator,
    environ: *const std.process.Environ.Map,
) ![]u8 {
    const home = environ.get("HOME") orelse environ.get("USERPROFILE") orelse ".";
    return std.fs.path.join(allocator, &.{ home, ".hexal" });
}

fn fileExists(io: std.Io, path: []const u8) bool {
    _ = std.Io.Dir.cwd().statFile(io, path, .{}) catch return false;
    return true;
}

test "parses aliases and editor object" {
    const allocator = std.testing.allocator;
    var config = try parse(
        allocator,
        \\{
        \\  "format_on_save": true,
        \\  "editor": { "showGitIgnored": true, "search_match_case_enabled": true },
        \\  "wordWrap": true
        \\}
    ,
        "/tmp/test.hexcfg",
    );
    defer config.deinit();
    try std.testing.expect(config.format_on_save);
    try std.testing.expect(config.show_git_ignored);
    try std.testing.expect(config.search_match_case_enabled);
    try std.testing.expect(config.word_wrap);
}
