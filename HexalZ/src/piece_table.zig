//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

const MaxPieceSize = 8192;

pub const Eol = enum {
    lf,
    crlf,

    pub fn bytes(self: Eol) []const u8 {
        return switch (self) {
            .lf => "\n",
            .crlf => "\r\n",
        };
    }

    pub fn label(self: Eol) []const u8 {
        return switch (self) {
            .lf => "LF",
            .crlf => "CRLF",
        };
    }
};

const Node = struct {
    left: i32 = -1,
    right: i32 = -1,
    from_add: bool = false,
    offset: usize = 0,
    length: usize = 0,
    piece_newlines: usize = 0,
    bytes: usize = 0,
    newlines: usize = 0,
    priority: u64 = 0,
};

/// A piece table over immutable original text and an append-only add buffer.
///
/// The piece sequence is stored in an implicit treap. Every node is augmented
/// with byte and newline counts, giving expected O(log p) insertion, deletion,
/// offset-to-line and line-to-offset queries. Pieces are split on roughly
/// 8 KiB newline boundaries to bound newline scans.
pub const PieceTable = struct {
    allocator: std.mem.Allocator,
    original: []u8 = &.{},
    /// False when the current text is borrowed from an undo/redo snapshot.
    original_owned: bool = true,
    added: std.ArrayList(u8) = .empty,
    nodes: std.ArrayList(Node) = .empty,
    root: i32 = -1,
    free_head: i32 = -1,
    random_state: u64 = 0x9E3779B97F4A7C15,
    eol: Eol = .lf,
    line_starts: std.ArrayList(usize) = .empty,
    line_index_valid: bool = false,
    text_is_original: bool = true,

    pub fn init(allocator: std.mem.Allocator) PieceTable {
        return .{ .allocator = allocator };
    }

    pub fn deinit(self: *PieceTable) void {
        if (self.original_owned and self.original.len > 0) self.allocator.free(self.original);
        self.added.deinit(self.allocator);
        self.nodes.deinit(self.allocator);
        self.line_starts.deinit(self.allocator);
        self.* = undefined;
    }

    /// Materialize borrowed text before an incremental edit. A bulk
    /// replacement can leave the table pointing at an undo snapshot; normal
    /// edits need to own their backing store so old snapshots can be evicted
    /// independently.
    fn ensureOwned(self: *PieceTable) !void {
        if (self.original_owned) return;
        if (self.original.len > 0) {
            const copy = try self.allocator.dupe(u8, self.original);
            self.original = copy;
        }
        self.original_owned = true;
        if (self.line_index_valid) {
            self.line_index_valid = false;
            self.rebuildPieces();
        }
    }

    fn rebuildPieces(self: *PieceTable) void {
        self.nodes.clearRetainingCapacity();
        self.root = -1;
        self.free_head = -1;
        var position: usize = 0;
        while (position < self.original.len) {
            const end = @min(position + MaxPieceSize, self.original.len);
            const node = self.makeNode(false, position, end - position) catch return;
            self.root = self.merge(self.root, node) catch return;
            position = end;
        }
    }

    fn buildLineIndex(self: *PieceTable, data: []const u8) !void {
        self.line_starts.clearRetainingCapacity();
        try self.line_starts.ensureTotalCapacity(self.allocator, data.len / 24 + 2);
        try self.line_starts.append(self.allocator, 0);
        var position: usize = 0;
        while (std.mem.indexOfScalarPos(u8, data, position, '\n')) |index| {
            try self.line_starts.append(self.allocator, index + 1);
            position = index + 1;
        }
    }

    /// Reserve the nodes needed for a whole-buffer replacement so the
    /// replacement itself cannot fail after ownership has been transferred.
    pub fn reserveForText(self: *PieceTable, len: usize) !void {
        try self.nodes.ensureTotalCapacity(self.allocator, len / MaxPieceSize + 2);
    }

    /// Replace the entire buffer with `data`, taking a borrow of the caller's
    /// storage. The caller must keep `data` alive until the next mutation or
    /// table reset. This avoids copying a bulk plugin replacement into the
    /// append buffer.
    pub fn replaceAllBorrowed(self: *PieceTable, data: []u8) !void {
        try self.reserveForText(data.len);
        if (self.original_owned and self.original.len > 0) self.allocator.free(self.original);
        self.original = &.{};
        self.original_owned = false;
        self.added.deinit(self.allocator);
        self.added = .empty;
        self.nodes.clearRetainingCapacity();
        self.root = -1;
        self.free_head = -1;
        self.eol = detectEol(data);
        self.original = data;
        self.line_index_valid = false;
        self.line_starts.clearRetainingCapacity();
        self.text_is_original = true;

        var position: usize = 0;
        while (position < data.len) {
            const end = @min(position + MaxPieceSize, data.len);
            self.root = try self.merge(self.root, try self.makeNode(false, position, end - position));
            position = end;
        }
    }

    pub fn size(self: PieceTable) usize {
        return if (self.root < 0) 0 else self.nodes.items[@intCast(self.root)].bytes;
    }

    pub fn newlineCount(self: PieceTable) usize {
        return if (self.root < 0) 0 else self.nodes.items[@intCast(self.root)].newlines;
    }

    pub fn lineCount(self: PieceTable) usize {
        if (self.line_index_valid) return self.line_starts.items.len;
        return self.newlineCount() + 1;
    }

    /// Takes ownership of `data`.
    pub fn setOriginal(self: *PieceTable, data: []u8) !void {
        if (self.original_owned and self.original.len > 0) self.allocator.free(self.original);
        self.original = data;
        self.original_owned = true;
        self.added.clearRetainingCapacity();
        self.nodes.clearRetainingCapacity();
        self.root = -1;
        self.free_head = -1;
        self.eol = detectEol(data);
        self.line_index_valid = false;
        self.line_starts.clearRetainingCapacity();
        self.text_is_original = true;

        var position: usize = 0;
        while (position < data.len) {
            const end = @min(position + MaxPieceSize, data.len);
            self.root = try self.merge(self.root, try self.makeNode(false, position, end - position));
            position = end;
        }
    }

    /// Borrow text without taking ownership. Used for read-only file mappings;
    /// the first edit materializes a private copy.
    pub fn setOriginalBorrowed(self: *PieceTable, data: []u8) !void {
        if (self.original_owned and self.original.len > 0) self.allocator.free(self.original);
        self.original = data;
        self.original_owned = false;
        self.added.clearRetainingCapacity();
        self.nodes.clearRetainingCapacity();
        self.root = -1;
        self.free_head = -1;
        self.eol = detectEol(data);
        self.line_index_valid = false;
        self.line_starts.clearRetainingCapacity();
        self.text_is_original = true;
        if (data.len == 0) return;
        const index = try self.allocNode();
        self.nodes.items[@intCast(index)] = .{
            .from_add = false,
            .offset = 0,
            .length = data.len,
            .piece_newlines = countNewlines(data),
            .priority = self.nextPriority(),
        };
        self.update(index);
        self.root = index;
    }

    /// Build the offset-to-line table on demand. This is intentionally lazy:
    /// the first screenful of a huge file only needs a handful of line starts.
    pub fn ensureLineIndex(self: *PieceTable) !void {
        if (self.line_index_valid or !self.text_is_original) return;
        try self.buildLineIndex(self.original);
        self.line_index_valid = true;
    }

    pub fn insert(self: *PieceTable, offset: usize, text: []const u8) !void {
        if (text.len == 0) return;
        try self.ensureOwned();
        self.text_is_original = false;
        const clamped = @min(offset, self.size());
        const start = self.added.items.len;
        try self.added.appendSlice(self.allocator, text);
        const end = self.added.items.len;

        var inserted_root: i32 = -1;
        var position = start;
        while (position < end) {
            const piece_end = chooseEnd(self.added.items, position, end);
            inserted_root = try self.merge(
                inserted_root,
                try self.makeNode(true, position, piece_end - position),
            );
            position = piece_end;
        }

        const parts = try self.split(self.root, clamped);
        self.root = try self.merge(try self.merge(parts.left, inserted_root), parts.right);
    }

    pub fn erase(self: *PieceTable, offset: usize, length: usize) void {
        if (length == 0 or offset >= self.size()) return;
        self.ensureOwned() catch return;
        self.text_is_original = false;
        const actual = @min(length, self.size() - offset);
        const first = self.split(self.root, offset) catch return;
        const second = self.split(first.right, actual) catch return;
        self.freeTree(second.left);
        self.root = self.merge(first.left, second.right) catch return;
    }

    pub fn copyRange(self: PieceTable, offset: usize, length: usize, destination: []u8) usize {
        if (length == 0 or offset >= self.size()) return 0;
        const actual = @min(length, self.size() - offset);
        var written: usize = 0;
        self.collectRange(self.root, 0, offset, offset + actual, destination, &written);
        return written;
    }

    pub fn rangeAlloc(self: PieceTable, offset: usize, length: usize) ![]u8 {
        const actual = if (offset >= self.size()) 0 else @min(length, self.size() - offset);
        const result = try self.allocator.alloc(u8, actual);
        _ = self.copyRange(offset, actual, result);
        return result;
    }

    pub fn byteAt(self: PieceTable, offset: usize) u8 {
        if (offset >= self.size()) return 0;
        var base: usize = 0;
        var node_index = self.root;
        while (node_index >= 0) {
            const node = self.nodes.items[@intCast(node_index)];
            const left_bytes = self.bytesOf(node.left);
            const node_start = base + left_bytes;
            if (offset < node_start) {
                node_index = node.left;
                continue;
            }
            if (offset < node_start + node.length) {
                const source = self.sourceSlice(node);
                return source[offset - node_start];
            }
            base = node_start + node.length;
            node_index = node.right;
        }
        return 0;
    }

    /// Returns the byte offset of the first character of a zero-based line.
    pub fn lineStart(self: PieceTable, line: usize) usize {
        if (self.line_index_valid) {
            if (line < self.line_starts.items.len) return self.line_starts.items[line];
            return self.original.len;
        }
        if (line == 0 or self.root < 0) return 0;

        var remaining = line;
        var base: usize = 0;
        var node_index = self.root;
        while (node_index >= 0) {
            const node = self.nodes.items[@intCast(node_index)];
            const left_newlines = self.newlinesOf(node.left);
            if (remaining <= left_newlines) {
                node_index = node.left;
                continue;
            }
            remaining -= left_newlines;

            if (remaining <= node.piece_newlines) {
                const index = self.newlineOffsetInPiece(node, remaining);
                return base + self.bytesOf(node.left) + index + 1;
            }
            remaining -= node.piece_newlines;
            base += self.bytesOf(node.left) + node.length;
            node_index = node.right;
        }
        return self.size();
    }

    /// Returns the zero-based line containing a byte offset.
    pub fn lineOfOffset(self: PieceTable, offset: usize) usize {
        if (self.line_index_valid) {
            if (offset >= self.original.len) return self.line_starts.items.len - 1;
            var low: usize = 0;
            var high: usize = self.line_starts.items.len;
            while (low < high) {
                const middle = low + (high - low) / 2;
                if (self.line_starts.items[middle] <= offset) {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            return if (low == 0) 0 else low - 1;
        }
        if (offset >= self.size()) return self.newlineCount();

        var line: usize = 0;
        var base: usize = 0;
        var node_index = self.root;
        while (node_index >= 0) {
            const node = self.nodes.items[@intCast(node_index)];
            const left_bytes = self.bytesOf(node.left);
            const node_start = base + left_bytes;
            if (offset < node_start) {
                node_index = node.left;
                continue;
            }
            if (offset < node_start + node.length) {
                const source = self.sourceSlice(node);
                const local = offset - node_start;
                return line + self.newlinesOf(node.left) +
                    countNewlines(source[0..local]);
            }
            line += self.newlinesOf(node.left) + node.piece_newlines;
            base = node_start + node.length;
            node_index = node.right;
        }
        return line;
    }

    pub fn writeTo(self: PieceTable, file: std.Io.File, io: std.Io) !void {
        try self.writeNode(self.root, file, io);
    }

    fn writeNode(self: PieceTable, node_index: i32, file: std.Io.File, io: std.Io) !void {
        if (node_index < 0) return;
        const node = self.nodes.items[@intCast(node_index)];
        try self.writeNode(node.left, file, io);
        try file.writeStreamingAll(io, self.sourceSlice(node));
        try self.writeNode(node.right, file, io);
    }

    fn sourceSlice(self: PieceTable, node: Node) []const u8 {
        if (node.from_add) {
            return self.added.items[node.offset .. node.offset + node.length];
        }
        return self.original[node.offset .. node.offset + node.length];
    }

    fn bytesOf(self: PieceTable, node_index: i32) usize {
        return if (node_index < 0) 0 else self.nodes.items[@intCast(node_index)].bytes;
    }

    fn newlinesOf(self: PieceTable, node_index: i32) usize {
        return if (node_index < 0) 0 else self.nodes.items[@intCast(node_index)].newlines;
    }

    fn makeNode(self: *PieceTable, from_add: bool, offset: usize, length: usize) !i32 {
        const index = try self.allocNode();
        const source = if (from_add)
            self.added.items[offset .. offset + length]
        else
            self.original[offset .. offset + length];
        self.nodes.items[@intCast(index)] = .{
            .from_add = from_add,
            .offset = offset,
            .length = length,
            .piece_newlines = countNewlines(source),
            .priority = self.nextPriority(),
        };
        self.update(index);
        return index;
    }

    fn allocNode(self: *PieceTable) !i32 {
        if (self.free_head >= 0) {
            const index = self.free_head;
            self.free_head = self.nodes.items[@intCast(index)].left;
            self.nodes.items[@intCast(index)] = .{};
            return index;
        }
        try self.nodes.ensureUnusedCapacity(self.allocator, 1);
        const index: i32 = @intCast(self.nodes.items.len);
        self.nodes.appendAssumeCapacity(.{});
        return index;
    }

    fn freeNode(self: *PieceTable, index: i32) void {
        self.nodes.items[@intCast(index)].left = self.free_head;
        self.free_head = index;
    }

    fn freeTree(self: *PieceTable, root: i32) void {
        if (root < 0) return;
        const node = self.nodes.items[@intCast(root)];
        self.freeTree(node.left);
        self.freeTree(node.right);
        self.freeNode(root);
    }

    fn update(self: *PieceTable, index: i32) void {
        var node = self.nodes.items[@intCast(index)];
        node.bytes = self.bytesOf(node.left) + node.length + self.bytesOf(node.right);
        node.newlines = self.newlinesOf(node.left) + node.piece_newlines +
            self.newlinesOf(node.right);
        self.nodes.items[@intCast(index)] = node;
    }

    fn merge(self: *PieceTable, left: i32, right: i32) !i32 {
        if (left < 0) return right;
        if (right < 0) return left;

        if (self.nodes.items[@intCast(left)].priority >= self.nodes.items[@intCast(right)].priority) {
            const old_right = self.nodes.items[@intCast(left)].right;
            const merged = try self.merge(old_right, right);
            self.nodes.items[@intCast(left)].right = merged;
            self.update(left);
            return left;
        }
        const old_left = self.nodes.items[@intCast(right)].left;
        const merged = try self.merge(left, old_left);
        self.nodes.items[@intCast(right)].left = merged;
        self.update(right);
        return right;
    }

    const SplitResult = struct { left: i32, right: i32 };

    fn split(self: *PieceTable, tree: i32, offset: usize) !SplitResult {
        if (tree < 0) return .{ .left = -1, .right = -1 };

        const node = self.nodes.items[@intCast(tree)];
        const left_bytes = self.bytesOf(node.left);
        const node_length = node.length;

        if (offset < left_bytes) {
            const old_left = node.left;
            const parts = try self.split(old_left, offset);
            self.nodes.items[@intCast(tree)].left = parts.right;
            self.update(tree);
            return .{ .left = parts.left, .right = tree };
        }

        if (offset > left_bytes + node_length) {
            const old_right = node.right;
            const parts = try self.split(old_right, offset - left_bytes - node_length);
            self.nodes.items[@intCast(tree)].right = parts.left;
            self.update(tree);
            return .{ .left = tree, .right = parts.right };
        }

        if (offset == left_bytes) {
            const old_left = node.left;
            self.nodes.items[@intCast(tree)].left = -1;
            self.update(tree);
            return .{ .left = old_left, .right = tree };
        }

        if (offset == left_bytes + node_length) {
            const old_right = node.right;
            self.nodes.items[@intCast(tree)].right = -1;
            self.update(tree);
            return .{ .left = tree, .right = old_right };
        }

        // The split point is inside the piece. Both halves keep the original
        // priority so the heap invariant relative to the existing children is
        // preserved; no text is copied.
        const local_offset = offset - left_bytes;
        const source = self.sourceSlice(node);
        const left_newlines = countNewlines(source[0..local_offset]);

        const left_half = try self.allocNode();
        self.nodes.items[@intCast(left_half)] = .{
            .left = node.left,
            .right = -1,
            .from_add = node.from_add,
            .offset = node.offset,
            .length = local_offset,
            .piece_newlines = left_newlines,
            .priority = node.priority,
        };
        self.update(left_half);

        const right_half = try self.allocNode();
        self.nodes.items[@intCast(right_half)] = .{
            .left = -1,
            .right = node.right,
            .from_add = node.from_add,
            .offset = node.offset + local_offset,
            .length = node.length - local_offset,
            .piece_newlines = node.piece_newlines - left_newlines,
            .priority = node.priority,
        };
        self.update(right_half);

        self.freeNode(tree);
        return .{ .left = left_half, .right = right_half };
    }

    fn collectRange(
        self: PieceTable,
        node_index: i32,
        base: usize,
        begin: usize,
        end: usize,
        destination: []u8,
        written: *usize,
    ) void {
        if (node_index < 0 or begin >= end) return;
        const node = self.nodes.items[@intCast(node_index)];
        const left_bytes = self.bytesOf(node.left);
        const node_start = base + left_bytes;
        const node_end = node_start + node.length;

        if (begin < node_start) {
            self.collectRange(node.left, base, begin, @min(end, node_start), destination, written);
        }
        if (begin < node_end and end > node_start) {
            const from = @max(begin, node_start) - node_start;
            const to = @min(end, node_end) - node_start;
            const source = self.sourceSlice(node);
            @memcpy(destination[written.* .. written.* + (to - from)], source[from..to]);
            written.* += to - from;
        }
        if (end > node_end) {
            self.collectRange(node.right, node_end, @max(begin, node_end), end, destination, written);
        }
    }

    fn newlineOffsetInPiece(self: PieceTable, node: Node, nth: usize) usize {
        const source = self.sourceSlice(node);
        var seen: usize = 0;
        for (source, 0..) |byte, i| {
            if (byte == '\n') {
                seen += 1;
                if (seen == nth) return i;
            }
        }
        return node.length;
    }

    fn nextPriority(self: *PieceTable) u64 {
        self.random_state +%= 0x9E3779B97F4A7C15;
        var z = self.random_state;
        z = (z ^ (z >> 30)) *% 0xBF58476D1CE4E5B9;
        z = (z ^ (z >> 27)) *% 0x94D049BB133111EB;
        return z ^ (z >> 31);
    }
};

fn chooseEnd(source: []const u8, start: usize, limit: usize) usize {
    if (start >= limit) return limit;
    const max_end = @min(limit, start + MaxPieceSize);
    if (max_end < limit) {
        if (std.mem.findScalarLast(u8, source[start..max_end], '\n')) |newline| {
            return start + newline + 1;
        }
    }
    return max_end;
}

pub fn countNewlines(source: []const u8) usize {
    var count: usize = 0;
    for (source) |byte| {
        if (byte == '\n') count += 1;
    }
    return count;
}

fn detectEol(source: []const u8) Eol {
    for (source, 0..) |byte, i| {
        if (byte == '\n') {
            return if (i > 0 and source[i - 1] == '\r') .crlf else .lf;
        }
    }
    return .lf;
}

test "piece table insertion, deletion and line indexing" {
    const allocator = std.testing.allocator;
    var table = PieceTable.init(allocator);
    defer table.deinit();

    const original = try allocator.dupe(u8, "one\ntwo\nthree");
    try table.setOriginal(original);
    try std.testing.expectEqual(@as(usize, 13), table.size());
    try std.testing.expectEqual(@as(usize, 3), table.lineCount());
    try std.testing.expectEqual(@as(usize, 4), table.lineStart(1));
    try std.testing.expectEqual(@as(usize, 8), table.lineStart(2));
    try std.testing.expectEqual(@as(usize, 1), table.lineOfOffset(6));

    try table.insert(3, "!");
    const text = try table.rangeAlloc(0, table.size());
    defer allocator.free(text);
    try std.testing.expectEqualStrings("one!\ntwo\nthree", text);

    table.erase(4, 4);
    const text2 = try table.rangeAlloc(0, table.size());
    defer allocator.free(text2);
    try std.testing.expectEqualStrings("one!\nthree", text2);
}

test "piece table large randomized consistency" {
    const allocator = std.testing.allocator;
    var table = PieceTable.init(allocator);
    defer table.deinit();
    const original = try allocator.dupe(u8, "abcdef\nghijkl\n");
    try table.setOriginal(original);

    try table.insert(7, "XYZ");
    table.erase(2, 3);
    const text = try table.rangeAlloc(0, table.size());
    defer allocator.free(text);
    try std.testing.expectEqualStrings("abf\nXYZghijkl\n", text);
}
