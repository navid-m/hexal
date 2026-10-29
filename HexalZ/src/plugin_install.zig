//! Installs plugin folders from the configured central Git repository.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const config_mod = @import("config.zig");

pub fn install(
    allocator: std.mem.Allocator,
    io: std.Io,
    environ: *const std.process.Environ.Map,
    name: []const u8,
    repository_url: []const u8,
    plugin_directory: []const u8,
) ![]u8 {
    if (!validPluginName(name)) return error.InvalidPluginName;
    if (std.mem.trim(u8, repository_url, " \t\r\n").len == 0) return error.EmptyRepository;

    try std.Io.Dir.cwd().createDirPath(io, plugin_directory);
    const destination = try std.fs.path.join(allocator, &.{ plugin_directory, name });
    errdefer allocator.free(destination);
    if (directoryExists(io, destination)) return error.AlreadyInstalled;

    const temporary = try temporaryDirectory(allocator, io, environ);
    defer allocator.free(temporary);
    defer std.Io.Dir.cwd().deleteTree(io, temporary) catch {};

    const clone_directory = try std.fs.path.join(allocator, &.{ temporary, "repository" });
    defer allocator.free(clone_directory);
    try cloneRepository(allocator, io, repository_url, clone_directory);

    const source = try std.fs.path.join(allocator, &.{ clone_directory, name });
    defer allocator.free(source);
    if (!directoryExists(io, source)) return error.PluginNotFound;

    const staging = try std.fmt.allocPrint(allocator, "{s}.installing-{x}", .{
        destination,
        uniqueId(io),
    });
    defer allocator.free(staging);
    copyDirectory(allocator, io, source, staging) catch |err| {
        std.Io.Dir.cwd().deleteTree(io, staging) catch {};
        return err;
    };
    std.Io.Dir.cwd().rename(staging, std.Io.Dir.cwd(), destination, io) catch |err| {
        std.Io.Dir.cwd().deleteTree(io, staging) catch {};
        return err;
    };
    return destination;
}

pub fn pluginDirectory(
    allocator: std.mem.Allocator,
    io: std.Io,
    environ: *const std.process.Environ.Map,
) ![]u8 {
    const executable_dir = std.process.executableDirPathAlloc(io, allocator) catch null;
    if (executable_dir) |directory| {
        defer allocator.free(directory);
        const portable = try std.fs.path.join(allocator, &.{ directory, "data", "plugins" });
        if (directoryExists(io, portable)) return portable;
        allocator.free(portable);
    }

    const local = try allocator.dupe(u8, "data/plugins");
    if (directoryExists(io, local)) return local;
    allocator.free(local);

    const home = try config_mod.hexalHomeDirectory(allocator, environ);
    defer allocator.free(home);
    const directory = try std.fs.path.join(allocator, &.{ home, "data", "plugins" });
    try std.Io.Dir.cwd().createDirPath(io, directory);
    return directory;
}

fn cloneRepository(
    allocator: std.mem.Allocator,
    io: std.Io,
    repository_url: []const u8,
    directory: []const u8,
) !void {
    const argv = [_][]const u8{
        "git", "clone", "--depth", "1", "--quiet", "--", repository_url, directory,
    };
    const result = std.process.run(allocator, io, .{
        .argv = &argv,
        .stdout_limit = .limited(64 * 1024),
        .stderr_limit = .limited(64 * 1024),
    }) catch |err| switch (err) {
        error.FileNotFound => return error.GitNotFound,
        else => return err,
    };
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);
    if (!result.term.success()) {
        const detail = std.mem.trim(u8, result.stderr, " \t\r\n");
        if (detail.len > 0) {
            std.log.err("git clone: {s}", .{detail});
        }
        return error.CloneFailed;
    }
}

fn copyDirectory(
    allocator: std.mem.Allocator,
    io: std.Io,
    source: []const u8,
    destination: []const u8,
) !void {
    try std.Io.Dir.cwd().createDirPath(io, destination);
    var dir = try std.Io.Dir.cwd().openDir(io, source, .{ .iterate = true });
    defer dir.close(io);
    var iterator = dir.iterate();
    while (try iterator.next(io)) |entry| {
        if (entry.kind == .sym_link or entry.kind == .unknown) continue;
        const from = try std.fs.path.join(allocator, &.{ source, entry.name });
        defer allocator.free(from);
        const to = try std.fs.path.join(allocator, &.{ destination, entry.name });
        defer allocator.free(to);
        if (entry.kind == .directory) {
            try copyDirectory(allocator, io, from, to);
        } else if (entry.kind == .file) {
            try std.Io.Dir.cwd().copyFile(from, std.Io.Dir.cwd(), to, io, .{});
        }
    }
}

fn uniqueId(io: std.Io) u128 {
    const nanos = std.Io.Clock.Timestamp.now(io, .awake).raw.nanoseconds;
    return if (nanos > 0) @intCast(nanos) else 0;
}

fn validPluginName(name: []const u8) bool {
    const trimmed = std.mem.trim(u8, name, " \t\r\n");
    if (trimmed.len == 0 or std.mem.eql(u8, trimmed, ".") or std.mem.eql(u8, trimmed, "..")) return false;
    for (trimmed) |byte| {
        if (byte == '/' or byte == '\\' or byte == 0) return false;
        if (byte < 0x20 or byte == 0x7f) return false;
    }
    return true;
}

fn directoryExists(io: std.Io, path: []const u8) bool {
    var dir = std.Io.Dir.cwd().openDir(io, path, .{}) catch return false;
    dir.close(io);
    return true;
}

fn temporaryDirectory(
    allocator: std.mem.Allocator,
    io: std.Io,
    environ: *const std.process.Environ.Map,
) ![]u8 {
    const root = if (@import("builtin").os.tag == .windows)
        (environ.get("TEMP") orelse environ.get("TMP") orelse ".")
    else
        (environ.get("TMPDIR") orelse "/tmp");
    const path = try std.fmt.allocPrint(allocator, "{s}/hexal-plugin-{x}", .{ root, uniqueId(io) });
    errdefer allocator.free(path);
    try std.Io.Dir.cwd().createDirPath(io, path);
    return path;
}

test "validates plugin names" {
    try std.testing.expect(validPluginName("demo"));
    try std.testing.expect(validPluginName("demo-2"));
    try std.testing.expect(!validPluginName(""));
    try std.testing.expect(!validPluginName(".."));
    try std.testing.expect(!validPluginName("../demo"));
    try std.testing.expect(!validPluginName("a/b"));
}
