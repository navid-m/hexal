//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const util = @import("util.zig");

pub const SearchHit = struct {
    file_index: usize,
    line: usize,
    column: usize,
    preview: []u8,
};

const ignored_components = [_][]const u8{
    ".git",                ".hg",     ".svn",   "node_modules",
    "target",              "build",   "dist",   "zig-cache",
    ".zig-cache",          "zig-out", "vendor", ".cache",
    ".idea",               ".vscode", "out",    "cmake-build-debug",
    "cmake-build-release",
};

pub fn clearFiles(allocator: std.mem.Allocator, files: *std.ArrayList([]u8)) void {
    for (files.items) |path| allocator.free(path);
    files.clearRetainingCapacity();
}

pub fn clearHits(allocator: std.mem.Allocator, hits: *std.ArrayList(SearchHit)) void {
    for (hits.items) |hit| allocator.free(hit.preview);
    hits.clearRetainingCapacity();
}

/// Enumerates regular files below the current working directory, skipping
/// common build/VCS directories. The result is sorted so pickers are stable.
pub fn scan(allocator: std.mem.Allocator, io: std.Io, files: *std.ArrayList([]u8), max_files: usize) !void {
    clearFiles(allocator, files);

    var dir = try std.Io.Dir.cwd().openDir(io, ".", .{ .iterate = true });
    defer dir.close(io);

    var walker = try dir.walk(allocator);
    defer walker.deinit();

    while (files.items.len < max_files) {
        const maybe_entry = walker.next(io) catch break;
        const entry = maybe_entry orelse break;

        if (entry.kind == .directory) {
            if (shouldIgnore(entry.path)) walker.leave(io);
            continue;
        }
        if (entry.kind != .file) continue;
        if (shouldIgnore(entry.path)) continue;

        const copy = try allocator.dupe(u8, entry.path);
        errdefer allocator.free(copy);
        try files.append(allocator, copy);
    }

    std.mem.sort([]u8, files.items, {}, lessThanPath);
}

fn lessThanPath(_: void, left: []u8, right: []u8) bool {
    return std.mem.lessThan(u8, left, right);
}

fn shouldIgnore(path: []const u8) bool {
    var components = std.mem.splitScalar(u8, path, '/');
    while (components.next()) |component| {
        for (ignored_components) |ignored| {
            if (std.mem.eql(u8, component, ignored)) return true;
        }
        if (std.mem.startsWith(u8, component, ".zig-cache")) return true;
    }
    return false;
}

/// Literal, case-insensitive project search. At most `max_hits` results are
/// returned; callers should keep that small so the picker stays responsive.
pub fn search(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    query: []const u8,
    hits: *std.ArrayList(SearchHit),
    max_hits: usize,
) !void {
    clearHits(allocator, hits);
    if (query.len == 0) return;

    for (files, 0..) |path, file_index| {
        if (hits.items.len >= max_hits) break;
        if (shouldIgnore(path)) continue;

        const data = std.Io.Dir.cwd().readFileAlloc(
            io,
            path,
            allocator,
            .limited(4 * 1024 * 1024),
        ) catch continue;
        defer allocator.free(data);
        if (looksBinary(data)) continue;

        var line: usize = 0;
        var line_start: usize = 0;
        while (line_start <= data.len) {
            const relative_newline = std.mem.findScalar(u8, data[line_start..], '\n');
            const line_end = if (relative_newline) |n| line_start + n else data.len;
            const line_text = data[line_start..line_end];

            if (util.indexOfIgnoreCase(line_text, query)) |column| {
                var preview_end = line_end - line_start;
                while (preview_end > 0 and
                    (line_text[preview_end - 1] == ' ' or line_text[preview_end - 1] == '\t' or
                        line_text[preview_end - 1] == '\r'))
                {
                    preview_end -= 1;
                }
                preview_end = @min(preview_end, 180);
                const preview = try allocator.dupe(u8, line_text[0..preview_end]);
                errdefer allocator.free(preview);
                try hits.append(allocator, .{
                    .file_index = file_index,
                    .line = line,
                    .column = column,
                    .preview = preview,
                });
                if (hits.items.len >= max_hits) break;
            }

            if (relative_newline == null) break;
            line_start = line_end + 1;
            line += 1;
        }
    }
}

fn looksBinary(data: []const u8) bool {
    const limit = @min(data.len, 8192);
    for (data[0..limit]) |byte| {
        if (byte == 0) return true;
    }
    return false;
}

test "ignore rules" {
    try std.testing.expect(shouldIgnore(".git/config"));
    try std.testing.expect(shouldIgnore("src/node_modules/pkg/index.js"));
    try std.testing.expect(!shouldIgnore("src/main.zig"));
}
