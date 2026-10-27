//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const Screen = @import("screen.zig").Screen;
const Cell = @import("screen.zig").Cell;
const Style = @import("theme.zig").Style;

/// Produces ANSI escape sequences for changed cells only.
pub const Renderer = struct {
    previous: []Cell = &.{},
    previous_width: usize = 0,
    previous_height: usize = 0,
    output: std.ArrayList(u8) = .empty,

    pub fn deinit(self: *Renderer, allocator: std.mem.Allocator) void {
        if (self.previous.len > 0) allocator.free(self.previous);
        self.output.deinit(allocator);
        self.* = .{};
    }

    pub fn reset(self: *Renderer, allocator: std.mem.Allocator) void {
        if (self.previous.len > 0) allocator.free(self.previous);
        self.previous = &.{};
        self.previous_width = 0;
        self.previous_height = 0;
        self.output.clearRetainingCapacity();
    }

    pub fn render(
        self: *Renderer,
        allocator: std.mem.Allocator,
        screen: Screen,
        force_full: bool,
    ) ![]const u8 {
        self.output.clearRetainingCapacity();
        const width = screen.width;
        const height = screen.height;
        if (width == 0 or height == 0) {
            if (self.previous.len > 0) allocator.free(self.previous);
            self.previous = &.{};
            self.previous_width = width;
            self.previous_height = height;
            return self.output.items;
        }

        const cells = screen.cells;
        var full = force_full;
        const size_changed = width != self.previous_width or
            height != self.previous_height or self.previous.len != cells.len;
        if (size_changed) {
            try self.output.appendSlice(allocator, "\x1b[2J");
            full = true;
        }

        var wrote_any = size_changed;

        var y: usize = 0;
        while (y < height) : (y += 1) {
            var x: usize = 0;
            while (x < width) {
                const index = y * width + x;
                const changed = full or index >= self.previous.len or
                    !Cell.eql(cells[index], self.previous[index]);
                if (!changed or cells[index].wide_continuation) {
                    x += 1;
                    continue;
                }

                const run_start = x;
                while (x < width) {
                    const run_index = y * width + x;
                    const run_changed = full or run_index >= self.previous.len or
                        !Cell.eql(cells[run_index], self.previous[run_index]);
                    if (!run_changed) break;
                    x += 1;
                }

                wrote_any = true;
                try self.output.appendSlice(allocator, "\x1b[");
                try appendInt(&self.output, allocator, y + 1);
                try self.output.append(allocator, ';');
                try appendInt(&self.output, allocator, run_start + 1);
                try self.output.append(allocator, 'H');

                var current: Style = .{};
                var current_valid = false;
                var column = run_start;
                while (column < x) : (column += 1) {
                    const cell = cells[y * width + column];
                    if (cell.wide_continuation) continue;
                    if (!current_valid or !Style.eql(cell.style, current)) {
                        try appendStyle(&self.output, allocator, cell.style);
                        current = cell.style;
                        current_valid = true;
                    }
                    try appendCodepoint(&self.output, allocator, cell.ch);
                }
            }
        }

        if (wrote_any) try self.output.appendSlice(allocator, "\x1b[0m");

        if (self.previous.len != cells.len) {
            if (self.previous.len > 0) allocator.free(self.previous);
            self.previous = try allocator.alloc(Cell, cells.len);
        }
        @memcpy(self.previous, cells);
        self.previous_width = width;
        self.previous_height = height;
        return self.output.items;
    }
};

fn appendStyle(output: *std.ArrayList(u8), allocator: std.mem.Allocator, style: Style) !void {
    try output.appendSlice(allocator, "\x1b[0");
    if (style.bold) try output.appendSlice(allocator, ";1");
    if (style.dim) try output.appendSlice(allocator, ";2");
    if (style.reverse) try output.appendSlice(allocator, ";7");
    try appendColor(output, allocator, style.fg, true);
    try appendColor(output, allocator, style.bg, false);
    try output.append(allocator, 'm');
}

fn appendColor(
    output: *std.ArrayList(u8),
    allocator: std.mem.Allocator,
    color: u32,
    foreground: bool,
) !void {
    if (color < 8) {
        try output.appendSlice(allocator, if (foreground) ";3" else ";4");
        try appendInt(output, allocator, color);
    } else if (color < 16) {
        try output.appendSlice(allocator, if (foreground) ";9" else ";10");
        try appendInt(output, allocator, color - 8);
    } else {
        try output.appendSlice(allocator, if (foreground) ";38;5;" else ";48;5;");
        try appendInt(output, allocator, color & 0xFF);
    }
}

fn appendCodepoint(
    output: *std.ArrayList(u8),
    allocator: std.mem.Allocator,
    codepoint: u21,
) !void {
    var buffer: [4]u8 = undefined;
    const length = std.unicode.utf8Encode(codepoint, &buffer) catch {
        try output.appendSlice(allocator, "\xEF\xBF\xBD");
        return;
    };
    try output.appendSlice(allocator, buffer[0..length]);
}

fn appendInt(output: *std.ArrayList(u8), allocator: std.mem.Allocator, value: usize) !void {
    var buffer: [32]u8 = undefined;
    const text = std.fmt.bufPrint(&buffer, "{d}", .{value}) catch return;
    try output.appendSlice(allocator, text);
}
