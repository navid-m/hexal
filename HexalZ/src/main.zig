//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const terminal = @import("terminal.zig");
const Editor = @import("editor.zig").Editor;
const document_mod = @import("document.zig");
const lua_mod = @import("lua.zig");
const alloc_mod = @import("alloc.zig");

const version = "0.1.0";

const help_text =
    \\hexalz [options] [file]
    \\
    \\A modal, terminal-based text editor written in Zig.
    \\
    \\Options:
    \\  -h, --help       Print this help and exit
    \\  -v, --version    Print the version and exit
    \\  --               Treat remaining arguments as file names
    \\
    \\Keys:
    \\  i / I            Insert mode
    \\  v / V            Character selection
    \\  x / X            Line selection
    \\  Esc              Normal mode / cancel selection
    \\  h j k l, arrows  Move
    \\  Home / End       Line start / end
    \\  Ctrl+Home/End    Document start / end
    \\  PageUp/PageDown  Move by one screen
    \\  gg / ge / gp     First line / last line / previous jump
    \\  d / Delete       Delete character or selection
    \\  u / U            Undo / redo
    \\  :                Ex command line (:w, :wq, :q, :e, :lang, :path)
    \\  Space            Open the SPACE menu (bottom-right)
    \\  Space f          File picker
    \\  Space /          Search across files
    \\  Space y / X      Yank / cut selection
    \\  Space p / P      Paste / paste on a new line
    \\  :lua <file>      Load a Lua plugin from a file
    \\  :plugins         List registered Lua commands
    \\
    \\Lua benchmark:
    \\  --lua-bench <plugin.lua> <command> <iterations> [file]
    \\
    \\The SPACE menu is intentionally modelled on the HexalCs which-key menu.
    \\
;

pub fn main(init: std.process.Init) !void {
    // Large editor buffers (documents, plugin snapshots, Lua strings) benefit
    // from huge-page-backed mappings and reuse. Small allocations continue to
    // use the C allocator.
    var huge_allocator = alloc_mod.HugeAllocator{};
    defer huge_allocator.deinit();
    const allocator = huge_allocator.allocator();
    const io = init.io;

    var args_it = try std.process.Args.Iterator.initAllocator(init.minimal.args, allocator);
    defer args_it.deinit();
    _ = args_it.next(); // executable name

    var file: ?[]const u8 = null;
    var options_ended = false;
    while (args_it.next()) |arg| {
        if (!options_ended and std.mem.eql(u8, arg, "--")) {
            options_ended = true;
            continue;
        }
        if (!options_ended and (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help"))) {
            try writeStdout(io, help_text);
            return;
        }
        if (!options_ended and (std.mem.eql(u8, arg, "-v") or std.mem.eql(u8, arg, "--version"))) {
            var buffer: [128]u8 = undefined;
            const text = try std.fmt.bufPrint(&buffer, "hexalz {s} (Zig {s})\n", .{
                version,
                @import("builtin").zig_version_string,
            });
            try writeStdout(io, text);
            return;
        }
        if (!options_ended and std.mem.eql(u8, arg, "--lua-bench")) {
            const plugin = args_it.next() orelse {
                try writeStderr(io, "hexalz: --lua-bench needs a plugin path\n");
                std.process.exit(2);
            };
            const command = args_it.next() orelse {
                try writeStderr(io, "hexalz: --lua-bench needs a command name\n");
                std.process.exit(2);
            };
            const iterations = args_it.next() orelse {
                try writeStderr(io, "hexalz: --lua-bench needs an iteration count\n");
                std.process.exit(2);
            };
            const bench_file = args_it.next();
            runLuaBench(allocator, io, plugin, command, iterations, bench_file) catch |err| {
                var buffer: [256]u8 = undefined;
                const text = try std.fmt.bufPrint(&buffer, "hexalz: lua bench: {s}\n", .{@errorName(err)});
                try writeStderr(io, text);
                std.process.exit(1);
            };
            return;
        }
        if (!options_ended and arg.len > 1 and arg[0] == '-') {
            var buffer: [512]u8 = undefined;
            const text = try std.fmt.bufPrint(&buffer, "hexalz: unknown option '{s}'\n\n", .{arg});
            try writeStderr(io, text);
            try writeStderr(io, help_text);
            std.process.exit(2);
        }
        if (file != null) {
            try writeStderr(io, "hexalz: the Zig port opens one file at a time\n");
            std.process.exit(2);
        }
        file = arg;
    }

    var terminal_state = terminal.Terminal.init(io) catch |err| {
        var buffer: [256]u8 = undefined;
        const text = try std.fmt.bufPrint(&buffer, "hexalz: cannot initialize terminal: {s}\n", .{@errorName(err)});
        try writeStderr(io, text);
        std.process.exit(1);
    };
    defer terminal_state.deinit();

    var editor = Editor.init(allocator, io, &terminal_state, file) catch |err| {
        var buffer: [320]u8 = undefined;
        const text = try std.fmt.bufPrint(&buffer, "hexalz: cannot open buffer: {s}\n", .{@errorName(err)});
        try writeStderr(io, text);
        std.process.exit(1);
    };
    defer editor.deinit();

    editor.run() catch |err| {
        var buffer: [256]u8 = undefined;
        const text = try std.fmt.bufPrint(&buffer, "hexalz: editor error: {s}\n", .{@errorName(err)});
        try writeStderr(io, text);
        std.process.exit(1);
    };
}

fn writeStdout(io: std.Io, text: []const u8) !void {
    try std.Io.File.stdout().writeStreamingAll(io, text);
}

fn writeStderr(io: std.Io, text: []const u8) !void {
    try std.Io.File.stderr().writeStreamingAll(io, text);
}

fn runLuaBench(
    allocator: std.mem.Allocator,
    io: std.Io,
    plugin: []const u8,
    command: []const u8,
    iterations_text: []const u8,
    file: ?[]const u8,
) !void {
    const iterations = std.fmt.parseInt(usize, iterations_text, 10) catch {
        try writeStderr(io, "hexalz: lua bench: invalid iteration count\n");
        return error.InvalidArguments;
    };
    if (iterations == 0) {
        try writeStderr(io, "hexalz: lua bench: iteration count must be greater than zero\n");
        return error.InvalidArguments;
    }

    var document = if (file) |path|
        try document_mod.Document.open(allocator, io, path)
    else
        document_mod.Document.init(allocator, io);
    defer document.deinit();

    var cursor: usize = 0;
    const start = std.Io.Clock.Timestamp.now(io, .awake);
    var manager = try lua_mod.Manager.init(allocator, io);
    defer manager.deinit();
    manager.setContext("[No Name]", file, "text", ".", 80, 24, null);
    manager.loadFilePlugin(plugin, &document, &cursor) catch |err| {
        for (manager.errors.items) |message| {
            try writeStderr(io, message);
            try writeStderr(io, "\n");
        }
        return err;
    };
    const init_ns = start.durationTo(std.Io.Clock.Timestamp.now(io, .awake)).raw.nanoseconds;

    _ = manager.executeCommand(command, &.{}, &document, &cursor) catch |err| {
        for (manager.errors.items) |message| {
            try writeStderr(io, message);
            try writeStderr(io, "\n");
        }
        return err;
    };

    var total_ns: i128 = 0;
    var checksum: i64 = 0;
    var iteration: usize = 0;
    while (iteration < iterations) : (iteration += 1) {
        const iteration_start = std.Io.Clock.Timestamp.now(io, .awake);
        const handled = manager.executeCommand(command, &.{}, &document, &cursor) catch |err| {
            for (manager.errors.items) |message| {
                try writeStderr(io, message);
                try writeStderr(io, "\n");
            }
            return err;
        };
        if (handled) checksum += 1;
        total_ns += iteration_start.durationTo(std.Io.Clock.Timestamp.now(io, .awake)).raw.nanoseconds;
    }

    var buffer: [256]u8 = undefined;
    const text = try std.fmt.bufPrint(&buffer, "lua_bench\t{d}\t{d}\t{d}\t{d}\n", .{
        init_ns,
        total_ns,
        iterations,
        checksum,
    });
    try writeStdout(io, text);
}
