//! A small JSON reader used by grammars, themes and plugin configuration.
//!
//! Hexal's data files are JSON with `//` and `/* */` comments and trailing
//! commas, so this is deliberately a little more permissive than RFC 8259.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub const Member = struct {
    key: []u8,
    value: Value,
};

pub const Value = union(enum) {
    null,
    boolean: bool,
    number: f64,
    string: []u8,
    array: std.ArrayList(Value),
    object: std.ArrayList(Member),

    pub fn deinit(self: *Value, allocator: std.mem.Allocator) void {
        switch (self.*) {
            .string => |text| allocator.free(text),
            .array => |*items| {
                for (items.items) |*item| item.deinit(allocator);
                items.deinit(allocator);
            },
            .object => |*items| {
                for (items.items) |*item| {
                    allocator.free(item.key);
                    item.value.deinit(allocator);
                }
                items.deinit(allocator);
            },
            else => {},
        }
        self.* = .null;
    }

    pub fn get(self: Value, key: []const u8) ?Value {
        switch (self) {
            .object => |items| {
                for (items.items) |item| {
                    if (std.mem.eql(u8, item.key, key)) return item.value;
                }
            },
            else => {},
        }
        return null;
    }

    pub fn asString(self: Value) ?[]const u8 {
        return switch (self) {
            .string => |text| text,
            else => null,
        };
    }

    pub fn asBool(self: Value) ?bool {
        return switch (self) {
            .boolean => |value| value,
            else => null,
        };
    }

    pub fn asNumber(self: Value) ?f64 {
        return switch (self) {
            .number => |value| value,
            else => null,
        };
    }

    pub fn asI64(self: Value) ?i64 {
        const value = self.asNumber() orelse return null;
        return @intFromFloat(value);
    }

    pub fn asArray(self: Value) ?[]Value {
        return switch (self) {
            .array => |items| items.items,
            else => null,
        };
    }

    pub fn asObject(self: Value) ?[]Member {
        return switch (self) {
            .object => |items| items.items,
            else => null,
        };
    }

    pub fn stringArray(self: Value, allocator: std.mem.Allocator, key: []const u8) !std.ArrayList([]u8) {
        var result: std.ArrayList([]u8) = .empty;
        errdefer {
            for (result.items) |item| allocator.free(item);
            result.deinit(allocator);
        }
        const value = self.get(key) orelse return result;
        switch (value) {
            .string => |text| try result.append(allocator, try allocator.dupe(u8, text)),
            .array => |items| {
                for (items.items) |item| {
                    if (item.asString()) |text| {
                        try result.append(allocator, try allocator.dupe(u8, text));
                    }
                }
            },
            else => {},
        }
        return result;
    }
};

pub fn parse(allocator: std.mem.Allocator, text: []const u8) !Value {
    var parser = Parser{
        .allocator = allocator,
        .bytes = text,
    };
    const value = try parser.parseValue();
    parser.skipIgnored();
    if (parser.position != text.len) return error.TrailingCharacters;
    return value;
}

const Parser = struct {
    allocator: std.mem.Allocator,
    bytes: []const u8,
    position: usize = 0,

    fn peek(self: *Parser) ?u8 {
        if (self.position >= self.bytes.len) return null;
        return self.bytes[self.position];
    }

    fn bump(self: *Parser) ?u8 {
        const value = self.peek() orelse return null;
        self.position += 1;
        return value;
    }

    fn skipIgnored(self: *Parser) void {
        while (self.position < self.bytes.len) {
            const byte = self.bytes[self.position];
            if (byte == ' ' or byte == '\t' or byte == '\r' or byte == '\n') {
                self.position += 1;
                continue;
            }
            if (byte == '/' and self.position + 1 < self.bytes.len) {
                if (self.bytes[self.position + 1] == '/') {
                    self.position += 2;
                    while (self.position < self.bytes.len and self.bytes[self.position] != '\n') {
                        self.position += 1;
                    }
                    continue;
                }
                if (self.bytes[self.position + 1] == '*') {
                    self.position += 2;
                    while (self.position + 1 < self.bytes.len) {
                        if (self.bytes[self.position] == '*' and self.bytes[self.position + 1] == '/') {
                            self.position += 2;
                            break;
                        }
                        self.position += 1;
                    }
                    continue;
                }
            }
            break;
        }
    }

    fn parseValue(self: *Parser) anyerror!Value {
        self.skipIgnored();
        const byte = self.peek() orelse return error.UnexpectedEndOfInput;
        return switch (byte) {
            '{' => self.parseObject(),
            '[' => self.parseArray(),
            '"' => .{ .string = try self.parseString() },
            't' => blk: {
                try self.expectWord("true");
                break :blk .{ .boolean = true };
            },
            'f' => blk: {
                try self.expectWord("false");
                break :blk .{ .boolean = false };
            },
            'n' => blk: {
                try self.expectWord("null");
                break :blk .null;
            },
            '-', '0'...'9' => .{ .number = try self.parseNumber() },
            else => error.ExpectedValue,
        };
    }

    fn parseObject(self: *Parser) anyerror!Value {
        _ = self.bump();
        var items: std.ArrayList(Member) = .empty;
        errdefer {
            for (items.items) |*item| {
                self.allocator.free(item.key);
                item.value.deinit(self.allocator);
            }
            items.deinit(self.allocator);
        }
        while (true) {
            self.skipIgnored();
            if (self.peek() == '}') {
                _ = self.bump();
                break;
            }
            if (self.peek() == null) return error.UnexpectedEndOfInput;
            if (self.peek() != '"') return error.ExpectedString;
            const key = try self.parseString();
            errdefer self.allocator.free(key);
            self.skipIgnored();
            if (self.bump() != ':') return error.ExpectedColon;
            const value = try self.parseValue();
            try items.append(self.allocator, .{ .key = key, .value = value });
            self.skipIgnored();
            switch (self.bump() orelse return error.UnexpectedEndOfInput) {
                ',' => {
                    self.skipIgnored();
                    if (self.peek() == '}') {
                        _ = self.bump();
                        break;
                    }
                },
                '}' => break,
                else => return error.ExpectedCommaOrEnd,
            }
        }
        return .{ .object = items };
    }

    fn parseArray(self: *Parser) anyerror!Value {
        _ = self.bump();
        var items: std.ArrayList(Value) = .empty;
        errdefer {
            for (items.items) |*item| item.deinit(self.allocator);
            items.deinit(self.allocator);
        }
        while (true) {
            self.skipIgnored();
            if (self.peek() == ']') {
                _ = self.bump();
                break;
            }
            if (self.peek() == null) return error.UnexpectedEndOfInput;
            const value = try self.parseValue();
            try items.append(self.allocator, value);
            self.skipIgnored();
            switch (self.bump() orelse return error.UnexpectedEndOfInput) {
                ',' => {
                    self.skipIgnored();
                    if (self.peek() == ']') {
                        _ = self.bump();
                        break;
                    }
                },
                ']' => break,
                else => return error.ExpectedCommaOrEnd,
            }
        }
        return .{ .array = items };
    }

    fn parseString(self: *Parser) ![]u8 {
        if (self.bump() != '"') return error.ExpectedString;
        var output: std.ArrayList(u8) = .empty;
        errdefer output.deinit(self.allocator);
        while (self.position < self.bytes.len) {
            const byte = self.bump().?;
            if (byte == '"') return output.toOwnedSlice(self.allocator);
            if (byte == '\\') {
                const escape = self.bump() orelse return error.UnterminatedString;
                switch (escape) {
                    '"' => try output.append(self.allocator, '"'),
                    '\\' => try output.append(self.allocator, '\\'),
                    '/' => try output.append(self.allocator, '/'),
                    'b' => try output.append(self.allocator, 0x08),
                    'f' => try output.append(self.allocator, 0x0C),
                    'n' => try output.append(self.allocator, '\n'),
                    'r' => try output.append(self.allocator, '\r'),
                    't' => try output.append(self.allocator, '\t'),
                    'u' => {
                        var scalar: u32 = 0;
                        var i: usize = 0;
                        while (i < 4) : (i += 1) {
                            const digit = self.bump() orelse return error.UnterminatedString;
                            scalar = scalar * 16 + (std.fmt.charToDigit(digit, 16) catch return error.InvalidEscape);
                        }
                        var buffer: [4]u8 = undefined;
                        const length = std.unicode.utf8Encode(@intCast(scalar), &buffer) catch 0;
                        if (length > 0) try output.appendSlice(self.allocator, buffer[0..length]);
                    },
                    else => return error.InvalidEscape,
                }
                continue;
            }
            try output.append(self.allocator, byte);
        }
        return error.UnterminatedString;
    }

    fn parseNumber(self: *Parser) !f64 {
        const start = self.position;
        while (self.position < self.bytes.len) {
            const byte = self.bytes[self.position];
            if (byte == '-' or byte == '+' or byte == '.' or byte == 'e' or byte == 'E' or
                (byte >= '0' and byte <= '9'))
            {
                self.position += 1;
            } else break;
        }
        return std.fmt.parseFloat(f64, self.bytes[start..self.position]);
    }

    fn expectWord(self: *Parser, word: []const u8) !void {
        if (self.position + word.len > self.bytes.len or
            !std.mem.eql(u8, self.bytes[self.position .. self.position + word.len], word))
        {
            return error.InvalidLiteral;
        }
        self.position += word.len;
    }
};

test "parses comments, trailing commas and escapes" {
    const allocator = std.testing.allocator;
    var value = try parse(allocator, "{\n // hi\n \"a\": [1, 2,], \"b\": \"x\\n\",\n}");
    defer value.deinit(allocator);
    try std.testing.expectEqual(@as(f64, 1), value.get("a").?.asArray().?[0].asNumber().?);
    try std.testing.expectEqualStrings("x\n", value.get("b").?.asString().?);
}
