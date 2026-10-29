//! The small regular-expression subset used by `.hexdef` `linePatterns`.
//!
//! Anchors, literals, escapes, character classes, `.`, groups, alternation and
//! greedy/lazy quantifiers are supported. Matching is byte oriented.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

const Range = struct { start: u8, end: u8 };

const Node = union(enum) {
    empty,
    byte: u8,
    any,
    class: struct { negated: bool, ranges: []Range },
    start,
    end,
    seq: []Node,
    alt: []Node,
    repeat: struct { node: *Node, min: usize, max: usize },

    fn deinit(self: *Node, allocator: std.mem.Allocator) void {
        switch (self.*) {
            .class => |class| allocator.free(class.ranges),
            .seq => |items| {
                for (items) |*item| item.deinit(allocator);
                allocator.free(items);
            },
            .alt => |items| {
                for (items) |*item| item.deinit(allocator);
                allocator.free(items);
            },
            .repeat => |repeat| {
                repeat.node.deinit(allocator);
                allocator.destroy(repeat.node);
            },
            else => {},
        }
        self.* = .empty;
    }
};

pub const Regex = struct {
    allocator: std.mem.Allocator,
    root: Node,
    case_insensitive: bool,

    pub fn compile(allocator: std.mem.Allocator, pattern: []const u8) !Regex {
        var parser = Parser{
            .allocator = allocator,
            .bytes = pattern,
        };
        const root = try parser.parseAlternation();
        if (parser.position != pattern.len) return error.TrailingInput;
        return .{ .allocator = allocator, .root = root, .case_insensitive = parser.case_insensitive };
    }

    pub fn deinit(self: *Regex) void {
        self.root.deinit(self.allocator);
    }

    /// First non-empty match, as a byte range.
    pub fn find(self: Regex, text: []const u8) ?struct { start: usize, end: usize } {
        var start: usize = 0;
        while (start <= text.len) : (start += 1) {
            var ends: std.ArrayList(usize) = .empty;
            defer ends.deinit(std.heap.c_allocator);
            matchNode(self.root, text, start, self.case_insensitive, &ends);
            var best: ?usize = null;
            for (ends.items) |end| {
                if (end > start and (best == null or end > best.?)) best = end;
            }
            if (best) |end| return .{ .start = start, .end = end };
        }
        return null;
    }
};

const Parser = struct {
    allocator: std.mem.Allocator,
    bytes: []const u8,
    position: usize = 0,
    case_insensitive: bool = false,

    fn peek(self: *Parser) ?u8 {
        return if (self.position < self.bytes.len) self.bytes[self.position] else null;
    }

    fn bump(self: *Parser) ?u8 {
        const value = self.peek() orelse return null;
        self.position += 1;
        return value;
    }

    fn parseAlternation(self: *Parser) anyerror!Node {
        var branches: std.ArrayList(Node) = .empty;
        errdefer {
            for (branches.items) |*item| item.deinit(self.allocator);
            branches.deinit(self.allocator);
        }
        try branches.append(self.allocator, try self.parseSequence());
        while (self.peek() == '|') {
            _ = self.bump();
            try branches.append(self.allocator, try self.parseSequence());
        }
        if (branches.items.len == 1) {
            const only = branches.items[0];
            branches.deinit(self.allocator);
            return only;
        }
        return .{ .alt = try branches.toOwnedSlice(self.allocator) };
    }

    fn parseSequence(self: *Parser) anyerror!Node {
        var nodes: std.ArrayList(Node) = .empty;
        errdefer {
            for (nodes.items) |*item| item.deinit(self.allocator);
            nodes.deinit(self.allocator);
        }
        while (self.peek()) |byte| {
            if (byte == '|' or byte == ')') break;
            var node = try self.parseAtom();
            node = try self.parseQuantifier(node);
            try nodes.append(self.allocator, node);
        }
        if (nodes.items.len == 0) {
            nodes.deinit(self.allocator);
            return .empty;
        }
        if (nodes.items.len == 1) {
            const only = nodes.items[0];
            nodes.deinit(self.allocator);
            return only;
        }
        return .{ .seq = try nodes.toOwnedSlice(self.allocator) };
    }

    fn parseAtom(self: *Parser) anyerror!Node {
        const byte = self.bump() orelse return error.UnexpectedEnd;
        return switch (byte) {
            '^' => .start,
            '$' => .end,
            '.' => .any,
            '(' => {
                if (self.peek() == '?') {
                    _ = self.bump();
                    const marker = self.bump() orelse return error.UnsupportedGroup;
                    switch (marker) {
                        ':' => {},
                        'i' => {
                            self.case_insensitive = true;
                            if (self.peek() == ')') {
                                _ = self.bump();
                                return .empty;
                            }
                            if (self.bump() != ':') return error.UnsupportedGroup;
                        },
                        '=', '!' => return error.LookaheadUnsupported,
                        else => return error.UnsupportedGroup,
                    }
                }
                const inner = try self.parseAlternation();
                if (self.bump() != ')') return error.MissingParen;
                return inner;
            },
            ')' => error.UnexpectedParen,
            '[' => try self.parseClass(),
            '\\' => try self.parseEscape(),
            '*', '+', '?', '}' => error.UnexpectedQuantifier,
            else => .{ .byte = byte },
        };
    }

    fn parseEscape(self: *Parser) !Node {
        const byte = self.bump() orelse return error.UnterminatedEscape;
        return switch (byte) {
            'd' => try self.singleRange('0', '9', false),
            'D' => try self.singleRange('0', '9', true),
            'w' => try self.wordClass(false),
            'W' => try self.wordClass(true),
            's' => try self.spaceClass(false),
            'S' => try self.spaceClass(true),
            'b', 'B' => .empty,
            'n' => .{ .byte = '\n' },
            'r' => .{ .byte = '\r' },
            't' => .{ .byte = '\t' },
            else => .{ .byte = byte },
        };
    }

    fn singleRange(self: *Parser, start: u8, end: u8, negated: bool) !Node {
        const ranges = try self.allocator.alloc(Range, 1);
        ranges[0] = .{ .start = start, .end = end };
        return .{ .class = .{ .negated = negated, .ranges = ranges } };
    }

    fn wordClass(self: *Parser, negated: bool) !Node {
        const ranges = try self.allocator.alloc(Range, 4);
        ranges[0] = .{ .start = 'a', .end = 'z' };
        ranges[1] = .{ .start = 'A', .end = 'Z' };
        ranges[2] = .{ .start = '0', .end = '9' };
        ranges[3] = .{ .start = '_', .end = '_' };
        return .{ .class = .{ .negated = negated, .ranges = ranges } };
    }

    fn spaceClass(self: *Parser, negated: bool) !Node {
        const ranges = try self.allocator.alloc(Range, 5);
        ranges[0] = .{ .start = ' ', .end = ' ' };
        ranges[1] = .{ .start = '\t', .end = '\t' };
        ranges[2] = .{ .start = '\r', .end = '\r' };
        ranges[3] = .{ .start = '\n', .end = '\n' };
        ranges[4] = .{ .start = 0x0B, .end = 0x0C };
        return .{ .class = .{ .negated = negated, .ranges = ranges } };
    }

    fn parseClass(self: *Parser) !Node {
        var negated = false;
        if (self.peek() == '^') {
            _ = self.bump();
            negated = true;
        }
        var ranges: std.ArrayList(Range) = .empty;
        errdefer ranges.deinit(self.allocator);
        var first = true;
        while (true) {
            const byte = self.bump() orelse return error.UnterminatedClass;
            if (byte == ']' and !first) break;
            first = false;
            const start = if (byte == '\\') blk: {
                const escaped = self.bump() orelse return error.UnterminatedEscape;
                if (escaped == 'd') {
                    try ranges.append(self.allocator, .{ .start = '0', .end = '9' });
                    continue;
                }
                if (escaped == 'w') {
                    try ranges.append(self.allocator, .{ .start = 'a', .end = 'z' });
                    try ranges.append(self.allocator, .{ .start = 'A', .end = 'Z' });
                    try ranges.append(self.allocator, .{ .start = '0', .end = '9' });
                    try ranges.append(self.allocator, .{ .start = '_', .end = '_' });
                    continue;
                }
                if (escaped == 's') {
                    try ranges.append(self.allocator, .{ .start = ' ', .end = ' ' });
                    try ranges.append(self.allocator, .{ .start = '\t', .end = '\t' });
                    try ranges.append(self.allocator, .{ .start = '\r', .end = '\r' });
                    try ranges.append(self.allocator, .{ .start = '\n', .end = '\n' });
                    continue;
                }
                break :blk escaped;
            } else byte;

            if (self.peek() == '-' and self.position + 1 < self.bytes.len and
                self.bytes[self.position + 1] != ']')
            {
                _ = self.bump();
                const end = self.bump() orelse return error.UnterminatedRange;
                try ranges.append(self.allocator, .{ .start = start, .end = end });
            } else {
                try ranges.append(self.allocator, .{ .start = start, .end = start });
            }
        }
        return .{ .class = .{ .negated = negated, .ranges = try ranges.toOwnedSlice(self.allocator) } };
    }

    fn parseQuantifier(self: *Parser, node: Node) !Node {
        const byte = self.peek() orelse return node;
        var min: usize = 0;
        var max: usize = 0;
        switch (byte) {
            '*' => {
                _ = self.bump();
                max = std.math.maxInt(usize);
            },
            '+' => {
                _ = self.bump();
                min = 1;
                max = std.math.maxInt(usize);
            },
            '?' => {
                _ = self.bump();
                max = 1;
            },
            '{' => {
                const saved = self.position;
                _ = self.bump();
                var digits: usize = 0;
                while (self.peek()) |value| {
                    if (value < '0' or value > '9') break;
                    min = min * 10 + (value - '0');
                    digits += 1;
                    _ = self.bump();
                }
                if (digits == 0) {
                    self.position = saved;
                    return node;
                }
                max = min;
                if (self.peek() == ',') {
                    _ = self.bump();
                    var value: usize = 0;
                    var count: usize = 0;
                    while (self.peek()) |digit| {
                        if (digit < '0' or digit > '9') break;
                        value = value * 10 + (digit - '0');
                        count += 1;
                        _ = self.bump();
                    }
                    max = if (count == 0) std.math.maxInt(usize) else value;
                }
                if (self.peek() != '}') {
                    self.position = saved;
                    return node;
                }
                _ = self.bump();
            },
            else => return node,
        }
        if (self.peek() == '?') _ = self.bump();
        const boxed = try self.allocator.create(Node);
        boxed.* = node;
        return .{ .repeat = .{ .node = boxed, .min = min, .max = max } };
    }
};

fn asciiEql(left: u8, right: u8, insensitive: bool) bool {
    if (insensitive) return std.ascii.toLower(left) == std.ascii.toLower(right);
    return left == right;
}

fn classMatches(node: Node, byte: u8, insensitive: bool) bool {
    const class = switch (node) {
        .class => |class| class,
        else => return false,
    };
    var found = false;
    for (class.ranges) |range| {
        if (byte >= range.start and byte <= range.end) {
            found = true;
            break;
        }
    }
    if (!found and insensitive) {
        const lower = std.ascii.toLower(byte);
        const upper = std.ascii.toUpper(byte);
        for (class.ranges) |range| {
            if ((lower >= range.start and lower <= range.end) or
                (upper >= range.start and upper <= range.end))
            {
                found = true;
                break;
            }
        }
    }
    return if (class.negated) !found else found;
}

fn matchNode(
    node: Node,
    text: []const u8,
    position: usize,
    insensitive: bool,
    out: *std.ArrayList(usize),
) void {
    switch (node) {
        .empty => out.append(std.heap.c_allocator, position) catch {},
        .byte => |byte| {
            if (position < text.len and asciiEql(text[position], byte, insensitive)) {
                out.append(std.heap.c_allocator, position + 1) catch {};
            }
        },
        .any => {
            if (position < text.len and text[position] != '\n') {
                out.append(std.heap.c_allocator, position + 1) catch {};
            }
        },
        .class => {
            if (position < text.len and classMatches(node, text[position], insensitive)) {
                out.append(std.heap.c_allocator, position + 1) catch {};
            }
        },
        .start => {
            if (position == 0) out.append(std.heap.c_allocator, position) catch {};
        },
        .end => {
            if (position == text.len) out.append(std.heap.c_allocator, position) catch {};
        },
        .seq => |nodes| {
            var positions: std.ArrayList(usize) = .empty;
            defer positions.deinit(std.heap.c_allocator);
            positions.append(std.heap.c_allocator, position) catch return;
            for (nodes) |child| {
                var next: std.ArrayList(usize) = .empty;
                defer next.deinit(std.heap.c_allocator);
                for (positions.items) |pos| {
                    matchNode(child, text, pos, insensitive, &next);
                }
                sortUnique(&next);
                if (next.items.len == 0) return;
                positions.clearRetainingCapacity();
                positions.appendSlice(std.heap.c_allocator, next.items) catch return;
            }
            out.appendSlice(std.heap.c_allocator, positions.items) catch {};
        },
        .alt => |branches| {
            for (branches) |branch| matchNode(branch, text, position, insensitive, out);
        },
        .repeat => |repeat| {
            var current: std.ArrayList(usize) = .empty;
            defer current.deinit(std.heap.c_allocator);
            current.append(std.heap.c_allocator, position) catch return;
            if (repeat.min == 0) out.append(std.heap.c_allocator, position) catch {};
            const ceiling = @min(repeat.max, text.len + 1);
            var count: usize = 1;
            while (count <= ceiling) : (count += 1) {
                var next: std.ArrayList(usize) = .empty;
                defer next.deinit(std.heap.c_allocator);
                for (current.items) |pos| matchNode(repeat.node.*, text, pos, insensitive, &next);
                sortUnique(&next);
                var filtered: std.ArrayList(usize) = .empty;
                defer filtered.deinit(std.heap.c_allocator);
                for (next.items) |pos| {
                    if (!contains(current.items, pos)) {
                        filtered.append(std.heap.c_allocator, pos) catch {};
                    }
                }
                if (filtered.items.len == 0) break;
                current.clearRetainingCapacity();
                current.appendSlice(std.heap.c_allocator, filtered.items) catch return;
                if (count >= repeat.min) {
                    out.appendSlice(std.heap.c_allocator, current.items) catch {};
                }
            }
        },
    }
}

fn sortUnique(items: *std.ArrayList(usize)) void {
    std.mem.sort(usize, items.items, {}, std.sort.asc(usize));
    var write: usize = 0;
    for (items.items) |value| {
        if (write == 0 or items.items[write - 1] != value) {
            items.items[write] = value;
            write += 1;
        }
    }
    items.items.len = write;
}

fn contains(items: []const usize, value: usize) bool {
    for (items) |item| if (item == value) return true;
    return false;
}

test "matches diff prefixes and dotenv assignments" {
    const allocator = std.testing.allocator;
    var added = try Regex.compile(allocator, "^\\+.*");
    defer added.deinit();
    try std.testing.expect(added.find("+ hello") != null);
    try std.testing.expect(added.find("- hello") == null);

    var dotenv = try Regex.compile(allocator, "^(?:export\\s+)?[A-Za-z_][A-Za-z0-9_.]*\\s*=");
    defer dotenv.deinit();
    try std.testing.expect(dotenv.find("FOO=bar") != null);
    try std.testing.expect(dotenv.find("# FOO=bar") == null);
}
