//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const util = @import("util.zig");
const regex_mod = @import("regex.zig");

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
    "cmake-build-release", "bin",     "obj",
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
pub fn scan(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: *std.ArrayList([]u8),
    max_files: usize,
    cancel: *const std.atomic.Value(bool),
) !void {
    clearFiles(allocator, files);

    var dir = try std.Io.Dir.cwd().openDir(io, ".", .{ .iterate = true });
    defer dir.close(io);

    var walker = try dir.walk(allocator);
    defer walker.deinit();

    while (files.items.len < max_files) {
        if (cancel.load(.acquire)) break;
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

const max_search_file_bytes: usize = 4 * 1024 * 1024;

const SearchTask = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    query: []const u8,
    hits: std.ArrayList(SearchHit) = .empty,
    max_hits: usize,
    cancel: *const std.atomic.Value(bool),
    cursor: *std.atomic.Value(usize),
    thread: ?std.Thread = null,
    err: ?anyerror = null,
};

fn searchWorkerCount(file_count: usize) usize {
    if (file_count < 64) return 1;
    const cpus = std.Thread.getCpuCount() catch 1;
    return @min(@max(cpus, 1), file_count);
}

fn runSearchTask(task: *SearchTask) void {
    searchRange(
        task.allocator,
        task.io,
        task.files,
        task.query,
        &task.hits,
        task.max_hits,
        task.cancel,
        task.cursor,
    ) catch |err| {
        task.err = err;
    };
}

fn lessThanHit(_: void, left: SearchHit, right: SearchHit) bool {
    if (left.file_index != right.file_index) return left.file_index < right.file_index;
    if (left.line != right.line) return left.line < right.line;
    return left.column < right.column;
}

/// Literal, case-insensitive project search. At most `max_hits` results are
/// returned; callers should keep that small so the picker stays responsive.
/// Files are searched in parallel, mirroring the Rust editor's worker pool.
pub fn search(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    query: []const u8,
    hits: *std.ArrayList(SearchHit),
    max_hits: usize,
    cancel: *const std.atomic.Value(bool),
) !void {
    clearHits(allocator, hits);
    if (query.len == 0 or max_hits == 0 or files.len == 0) return;

    const worker_count = searchWorkerCount(files.len);
    var cursor: std.atomic.Value(usize) = .init(0);
    if (worker_count <= 1) {
        try searchRange(allocator, io, files, query, hits, max_hits, cancel, &cursor);
        return;
    }

    const tasks = try allocator.alloc(SearchTask, worker_count);
    defer allocator.free(tasks);
    var started: usize = 0;
    for (0..worker_count) |_| {
        tasks[started] = .{
            .allocator = allocator,
            .io = io,
            .files = files,
            .query = query,
            .max_hits = max_hits,
            .cancel = cancel,
            .cursor = &cursor,
        };
        tasks[started].thread = std.Thread.spawn(.{}, runSearchTask, .{&tasks[started]}) catch blk: {
            // Thread creation is rare to fail; finishing the work inline is
            // still correct, just slower.
            runSearchTask(&tasks[started]);
            break :blk null;
        };
        started += 1;
    }
    for (tasks[0..started]) |*task| {
        if (task.thread) |thread| thread.join();
    }

    var failure: ?anyerror = null;
    for (tasks[0..started]) |*task| {
        if (task.err) |err| {
            failure = err;
            break;
        }
    }
    if (failure) |err| {
        for (tasks[0..started]) |*task| {
            clearHits(allocator, &task.hits);
            task.hits.deinit(allocator);
        }
        return err;
    }

    // Workers pull files in completion order, so sort before truncating to
    // keep the picker stable.
    var merge_failed = false;
    for (tasks[0..started]) |*task| {
        if (merge_failed) continue;
        hits.appendSlice(allocator, task.hits.items) catch {
            merge_failed = true;
            continue;
        };
        // Ownership of the previews moved to `hits`.
        task.hits.items.len = 0;
    }
    for (tasks[0..started]) |*task| {
        clearHits(allocator, &task.hits);
        task.hits.deinit(allocator);
    }
    if (merge_failed) {
        clearHits(allocator, hits);
        return error.OutOfMemory;
    }

    std.mem.sort(SearchHit, hits.items, {}, lessThanHit);
    if (hits.items.len > max_hits) {
        for (hits.items[max_hits..]) |hit| allocator.free(hit.preview);
        hits.shrinkRetainingCapacity(max_hits);
    }
}

/// Searches `content` for the first match on each line. The needle is located
/// over the whole buffer so files without a match are scanned exactly once;
/// line numbers are only reconstructed when a hit is actually found.
fn scanContent(
    allocator: std.mem.Allocator,
    content: []const u8,
    query: []const u8,
    file_index: usize,
    hits: *std.ArrayList(SearchHit),
    max_hits: usize,
) !void {
    var line: usize = 0;
    var line_start: usize = 0;
    var search_from: usize = 0;

    while (search_from < content.len) {
        const relative = util.indexOfIgnoreCase(content[search_from..], query) orelse return;
        const match_position = search_from + relative;

        var line_end = content.len;
        var cursor = line_start;
        while (true) {
            const newline = std.mem.findScalarPos(u8, content, cursor, '\n');
            if (newline) |index| {
                if (index >= match_position) {
                    line_end = index;
                    break;
                }
                line += 1;
                line_start = index + 1;
                cursor = line_start;
            } else {
                line_end = content.len;
                break;
            }
        }

        const line_text = content[line_start..line_end];
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
            .column = match_position - line_start,
            .preview = preview,
        });
        if (hits.items.len >= max_hits) return;

        // The editor reports at most one hit per line, so resume below this
        // line instead of finding the same match again.
        if (line_end >= content.len) return;
        line += 1;
        line_start = line_end + 1;
        search_from = line_end + 1;
    }
}

fn searchRange(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    query: []const u8,
    hits: *std.ArrayList(SearchHit),
    max_hits: usize,
    cancel: *const std.atomic.Value(bool),
    cursor: *std.atomic.Value(usize),
) !void {
    const cwd = std.Io.Dir.cwd();
    // Read into a reused buffer instead of stat-ing every file first. The
    // extra byte distinguishes "exactly at the limit" from "too large", so a
    // single read handles the size check as well.
    const buffer = try allocator.alloc(u8, max_search_file_bytes + 1);
    defer allocator.free(buffer);

    while (hits.items.len < max_hits) {
        if (cancel.load(.acquire)) return;
        const file_index = cursor.fetchAdd(1, .monotonic);
        if (file_index >= files.len) return;
        const path = files[file_index];
        if (shouldIgnore(path)) continue;

        var file = cwd.openFile(io, path, .{}) catch continue;
        defer file.close(io);

        // Probe the first block first: binaries and files over the size
        // limit are rejected without pulling their contents through the
        // page cache.
        const probe_len = @min(buffer.len, 8192);
        var data_len = file.readPositionalAll(io, buffer[0..probe_len], 0) catch continue;
        if (data_len == 0) continue;
        if (std.mem.indexOfScalar(u8, buffer[0..data_len], 0) != null) continue;
        if (data_len == probe_len and probe_len < buffer.len) {
            const rest = file.readPositionalAll(io, buffer[probe_len..], probe_len) catch continue;
            data_len += rest;
        }
        if (data_len > max_search_file_bytes) continue;

        const content = buffer[0..data_len];

        try scanContent(allocator, content, query, file_index, hits, max_hits);
    }
}

test "ignore rules" {
    try std.testing.expect(shouldIgnore(".git/config"));
    try std.testing.expect(shouldIgnore("src/node_modules/pkg/index.js"));
    try std.testing.expect(!shouldIgnore("src/main.zig"));
}


pub const ReplaceStats = struct {
    files_changed: usize = 0,
    replacements: usize = 0,
};

/// Regex variant of `search`. One hit is reported per matching line.
pub fn searchRegex(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    pattern: []const u8,
    hits: *std.ArrayList(SearchHit),
    max_hits: usize,
    cancel: *const std.atomic.Value(bool),
    case_insensitive: bool,
) !void {
    clearHits(allocator, hits);
    if (pattern.len == 0 or max_hits == 0) return;
    var regex = regex_mod.Regex.compileWithOptions(allocator, pattern, case_insensitive) catch |err| switch (err) {
        error.OutOfMemory => return err,
        else => return error.InvalidRegex,
    };
    defer regex.deinit();

    const cwd = std.Io.Dir.cwd();
    const buffer = try allocator.alloc(u8, max_search_file_bytes + 1);
    defer allocator.free(buffer);

    for (files, 0..) |path, file_index| {
        if (hits.items.len >= max_hits) break;
        if (cancel.load(.acquire)) return;
        if (shouldIgnore(path)) continue;
        var file = cwd.openFile(io, path, .{}) catch continue;
        defer file.close(io);
        const probe_len = @min(buffer.len, 8192);
        var data_len = file.readPositionalAll(io, buffer[0..probe_len], 0) catch continue;
        if (data_len == 0) continue;
        if (std.mem.indexOfScalar(u8, buffer[0..data_len], 0) != null) continue;
        if (data_len == probe_len and probe_len < buffer.len) {
            data_len += file.readPositionalAll(io, buffer[probe_len..], probe_len) catch 0;
        }
        if (data_len > max_search_file_bytes) continue;
        const content = buffer[0..data_len];

        var line: usize = 0;
        var lines = std.mem.splitScalar(u8, content, '\n');
        while (lines.next()) |line_text| {
            if (hits.items.len >= max_hits) break;
            const match = regex.findCaptures(line_text, 0) orelse {
                line += 1;
                continue;
            };
            var preview_end = line_text.len;
            while (preview_end > 0 and (line_text[preview_end - 1] == ' ' or
                line_text[preview_end - 1] == '\t' or line_text[preview_end - 1] == '\r'))
            {
                preview_end -= 1;
            }
            preview_end = @min(preview_end, 180);
            const preview = try allocator.dupe(u8, line_text[0..preview_end]);
            errdefer allocator.free(preview);
            try hits.append(allocator, .{
                .file_index = file_index,
                .line = line,
                .column = match.start,
                .preview = preview,
            });
            line += 1;
        }
    }
    std.mem.sort(SearchHit, hits.items, {}, lessThanHit);
}

/// Replaces every regex match in every matching file. Files are rewritten in
/// place; binary and oversized files are skipped just like search.
pub fn replaceRegex(
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    pattern: []const u8,
    replacement: []const u8,
    case_insensitive: bool,
    cancel: *const std.atomic.Value(bool),
    max_replacements: usize,
) !ReplaceStats {
    var stats = ReplaceStats{};
    if (pattern.len == 0 or max_replacements == 0) return stats;
    var regex = regex_mod.Regex.compileWithOptions(allocator, pattern, case_insensitive) catch |err| switch (err) {
        error.OutOfMemory => return err,
        else => return error.InvalidRegex,
    };
    defer regex.deinit();

    const cwd = std.Io.Dir.cwd();
    const buffer = try allocator.alloc(u8, max_search_file_bytes + 1);
    defer allocator.free(buffer);

    for (files) |path| {
        if (cancel.load(.acquire) or stats.replacements >= max_replacements) break;
        if (shouldIgnore(path)) continue;
        var file = cwd.openFile(io, path, .{}) catch continue;
        defer file.close(io);
        const probe_len = @min(buffer.len, 8192);
        var data_len = file.readPositionalAll(io, buffer[0..probe_len], 0) catch continue;
        if (data_len == 0) continue;
        if (std.mem.indexOfScalar(u8, buffer[0..data_len], 0) != null) continue;
        if (data_len == probe_len and probe_len < buffer.len) {
            data_len += file.readPositionalAll(io, buffer[probe_len..], probe_len) catch 0;
        }
        if (data_len > max_search_file_bytes) continue;
        file.close(io);

        const content = buffer[0..data_len];
        var output: std.ArrayList(u8) = .empty;
        defer output.deinit(allocator);
        var search_from: usize = 0;
        var file_changes: usize = 0;
        while (search_from <= content.len and stats.replacements < max_replacements) {
            const match = regex.findCaptures(content, search_from) orelse break;
            try output.appendSlice(allocator, content[search_from..match.start]);
            const expanded = try regex_mod.expandReplacement(allocator, replacement, content, match);
            defer allocator.free(expanded);
            try output.appendSlice(allocator, expanded);
            search_from = if (match.end > match.start) match.end else match.end + 1;
            stats.replacements += 1;
            file_changes += 1;
        }
        if (file_changes == 0) continue;
        try output.appendSlice(allocator, content[search_from..]);
        try cwd.writeFile(io, .{ .sub_path = path, .data = output.items });
        stats.files_changed += 1;
    }
    return stats;
}
