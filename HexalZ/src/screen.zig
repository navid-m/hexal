//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const theme = @import("theme.zig");
const util = @import("util.zig");

pub const Style = theme.Style;

pub const Cell = struct {
    ch: u21 = ' ',
    style: Style = .{},
    wide_continuation: bool = false,

    pub fn eql(a: Cell, b: Cell) bool {
        return a.ch == b.ch and Style.eql(a.style, b.style) and
            a.wide_continuation == b.wide_continuation;
    }
};

/// Double-buffered cell grid. The renderer diffs two grids so only changed
/// cells reach the terminal.
pub const Screen = struct {
    width: usize = 0,
    height: usize = 0,
    cells: []Cell = &.{},

    pub fn deinit(self: *Screen, allocator: std.mem.Allocator) void {
        if (self.cells.len > 0) allocator.free(self.cells);
        self.* = .{};
    }

    pub fn resize(self: *Screen, allocator: std.mem.Allocator, width: usize, height: usize) !void {
        if (width == self.width and height == self.height) return;
        if (self.cells.len > 0) allocator.free(self.cells);
        self.width = width;
        self.height = height;
        self.cells = try allocator.alloc(Cell, width * height);
        self.clear(.{});
    }

    pub fn clear(self: *Screen, style: Style) void {
        const blank = Cell{ .ch = ' ', .style = style, .wide_continuation = false };
        @memset(self.cells, blank);
    }

    pub fn inBounds(self: Screen, x: i64, y: i64) bool {
        return x >= 0 and y >= 0 and
            @as(usize, @intCast(x)) < self.width and
            @as(usize, @intCast(y)) < self.height;
    }

    pub fn at(self: *Screen, x: usize, y: usize) *Cell {
        return &self.cells[y * self.width + x];
    }

    pub fn put(self: *Screen, x: i64, y: i64, cp: u21, style: Style, width: usize) void {
        if (width == 0 or !self.inBounds(x, y)) return;
        const ux: usize = @intCast(x);
        const uy: usize = @intCast(y);
        if (width > 1 and ux + width > self.width) return;

        const index = uy * self.width + ux;

        // Repair the other half of a wide scalar that we are overwriting.
        if (ux > 0 and self.cells[index].wide_continuation) {
            self.cells[index - 1] = .{ .ch = ' ', .style = self.cells[index - 1].style };
        }
        if (ux + 1 < self.width and self.cells[index + 1].wide_continuation) {
            self.cells[index + 1] = .{ .ch = ' ', .style = self.cells[index + 1].style };
        }

        self.cells[index] = .{ .ch = cp, .style = style, .wide_continuation = false };
        var offset: usize = 1;
        while (offset < width) : (offset += 1) {
            self.cells[index + offset] = .{
                .ch = ' ',
                .style = style,
                .wide_continuation = true,
            };
        }
    }

    pub fn putAscii(self: *Screen, x: i64, y: i64, text: []const u8, style: Style, max_width: usize) void {
        var column: usize = 0;
        for (text) |c| {
            if (column >= max_width or x + @as(i64, @intCast(column)) >= @as(i64, @intCast(self.width))) break;
            self.put(x + @as(i64, @intCast(column)), y, c, style, 1);
            column += 1;
        }
    }

    pub fn putString(self: *Screen, x: i64, y: i64, text: []const u8, style: Style, max_width: usize) void {
        var column: usize = 0;
        var offset: usize = 0;
        while (offset < text.len and column < max_width and
            x + @as(i64, @intCast(column)) < @as(i64, @intCast(self.width)))
        {
            const decoded = util.decodeAt(text, offset);
            if (decoded.len == 0) break;
            const width = util.displayWidth(decoded.codepoint);
            if (width == 0) {
                offset += decoded.len;
                continue;
            }
            if (x + @as(i64, @intCast(column + width)) > @as(i64, @intCast(self.width)) or
                column + width > max_width)
            {
                break;
            }
            self.put(x + @as(i64, @intCast(column)), y, decoded.codepoint, style, width);
            column += width;
            offset += decoded.len;
        }
    }

    pub fn putInt(self: *Screen, x: i64, y: i64, value: usize, style: Style, max_width: usize) void {
        var buffer: [32]u8 = undefined;
        const text = std.fmt.bufPrint(&buffer, "{d}", .{value}) catch return;
        self.putAscii(x, y, text, style, max_width);
    }

    pub fn fill(self: *Screen, x: i64, y: i64, width: usize, height: usize, cp: u21, style: Style) void {
        var row: usize = 0;
        while (row < height) : (row += 1) {
            var column: usize = 0;
            while (column < width) : (column += 1) {
                self.put(x + @as(i64, @intCast(column)), y + @as(i64, @intCast(row)), cp, style, 1);
            }
        }
    }

    pub fn blit(self: *Screen, source: Screen, destination_x: usize, destination_y: usize) void {
        if (source.width == 0 or source.height == 0) return;
        var row: usize = 0;
        while (row < source.height) : (row += 1) {
            const target_y = destination_y + row;
            if (target_y >= self.height) break;
            var column: usize = 0;
            while (column < source.width) : (column += 1) {
                const target_x = destination_x + column;
                if (target_x >= self.width) break;
                self.cells[target_y * self.width + target_x] = source.cells[row * source.width + column];
            }
        }
    }

    pub fn applyStyle(self: *Screen, x: usize, y: usize, style: Style) void {
        if (x >= self.width or y >= self.height) return;
        const index = y * self.width + x;
        self.cells[index].style = style;
        if (self.cells[index].wide_continuation and x > 0) {
            self.cells[index - 1].style = style;
        } else if (x + 1 < self.width and self.cells[index + 1].wide_continuation) {
            self.cells[index + 1].style = style;
        }
    }
};
