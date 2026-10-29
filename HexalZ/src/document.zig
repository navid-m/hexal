//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const PieceTable = @import("piece_table.zig").PieceTable;
const Eol = @import("piece_table.zig").Eol;

const GroupKind = enum { insert, erase, mixed };

const Op = struct {
    offset: usize,
    removed: []u8,
    inserted: []u8,
    replace_all: bool = false,
};

const Group = struct {
    kind: GroupKind,
    ops: std.ArrayList(Op) = .empty,
};

const MaxUndoGroups = 4096;
const Bom = [_]u8{ 0xEF, 0xBB, 0xBF };

/// The document coordinates follow Hexal's three-layer model:
///   * storage and cursor positions are UTF-8 byte offsets
///   * cursor movement respects Unicode scalar boundaries
///   * the UI converts byte offsets to display cells
pub const Document = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    table: PieceTable,
    path: ?[]u8 = null,
    eol: Eol = .lf,
    bom: bool = false,
    undo: std.ArrayList(Group) = .empty,
    redo: std.ArrayList(Group) = .empty,
    group_open: bool = false,
    group_kind: GroupKind = .mixed,
    group_last_offset: usize = 0,
    revision: u64 = 0,
    saved_revision: u64 = 0,

    pub fn init(allocator: std.mem.Allocator, io: std.Io) Document {
        return .{
            .allocator = allocator,
            .io = io,
            .table = PieceTable.init(allocator),
        };
    }

    pub fn deinit(self: *Document) void {
        if (self.path) |path| self.allocator.free(path);
        self.table.deinit();
        self.clearGroups(&self.undo);
        self.clearGroups(&self.redo);
        self.undo.deinit(self.allocator);
        self.redo.deinit(self.allocator);
        self.* = undefined;
    }

    pub fn open(allocator: std.mem.Allocator, io: std.Io, path: []const u8) !Document {
        var doc = Document.init(allocator, io);
        errdefer doc.deinit();

        doc.path = try allocator.dupe(u8, path);
        const cwd = std.Io.Dir.cwd();
        const read = cwd.readFileAlloc(io, path, allocator, .limited(2 * 1024 * 1024 * 1024)) catch |err| switch (err) {
            error.FileNotFound => null,
            else => return err,
        };

        if (read) |raw| {
            var data: []u8 = undefined;
            if (raw.len >= 3 and raw[0] == Bom[0] and raw[1] == Bom[1] and raw[2] == Bom[2]) {
                doc.bom = true;
                data = allocator.alloc(u8, raw.len - 3) catch |err| {
                    allocator.free(raw);
                    return err;
                };
                @memcpy(data, raw[3..]);
                allocator.free(raw);
            } else {
                // Take ownership of the read buffer directly; the piece table
                // never mutates original text.
                data = raw;
            }
            // `setOriginal` takes ownership before it can fail, so the
            // document's deinit is the only owner on the error path.
            try doc.table.setOriginal(data);
        } else {
            const empty = try allocator.alloc(u8, 0);
            try doc.table.setOriginal(empty);
        }
        doc.eol = doc.table.eol;
        return doc;
    }

    pub fn displayName(self: Document) []const u8 {
        if (self.path) |path| {
            const name = std.fs.path.basename(path);
            return if (name.len > 0) name else path;
        }
        return "[No Name]";
    }

    pub fn isDirty(self: Document) bool {
        return self.revision != self.saved_revision;
    }

    pub fn lineCount(self: Document) usize {
        return self.table.lineCount();
    }

    pub fn size(self: Document) usize {
        return self.table.size();
    }

    pub fn lineStart(self: Document, line: usize) usize {
        return self.table.lineStart(line);
    }

    pub fn lineOfOffset(self: Document, offset: usize) usize {
        return self.table.lineOfOffset(offset);
    }

    pub fn lineEnd(self: Document, line: usize) usize {
        if (line + 1 >= self.lineCount()) return self.size();
        var end = self.table.lineStart(line + 1);
        if (end > 0 and self.table.byteAt(end - 1) == '\n') {
            end -= 1;
            if (end > 0 and self.table.byteAt(end - 1) == '\r') end -= 1;
        }
        return end;
    }

    pub fn lineText(self: *Document, line: usize, buffer: *std.ArrayList(u8)) ![]u8 {
        const begin = self.lineStart(line);
        const end = self.lineEnd(line);
        const length = end - begin;
        buffer.clearRetainingCapacity();
        try buffer.ensureTotalCapacity(self.allocator, length);
        buffer.items.len = length;
        _ = self.table.copyRange(begin, length, buffer.items[0..length]);
        return buffer.items;
    }

    pub fn rangeAlloc(self: *Document, offset: usize, length: usize) ![]u8 {
        return self.table.rangeAlloc(offset, length);
    }

    pub fn insert(self: *Document, offset: usize, text: []const u8) !void {
        if (text.len == 0) return;
        const clamped = @min(offset, self.size());
        const inserted = try self.allocator.dupe(u8, text);
        errdefer self.allocator.free(inserted);

        if (!self.group_open or self.group_kind != .insert or
            self.group_last_offset != clamped)
        {
            if (self.group_open) self.closeGroup();
            try self.startGroup(.insert);
        }

        const group = &self.undo.items[self.undo.items.len - 1];
        try group.ops.append(self.allocator, .{
            .offset = clamped,
            .removed = try self.allocator.alloc(u8, 0),
            .inserted = inserted,
        });

        try self.table.insert(clamped, text);
        self.group_last_offset = clamped + text.len;
        self.revision += 1;
    }

    pub fn erase(self: *Document, offset: usize, length: usize) !void {
        if (length == 0 or offset >= self.size()) return;
        const actual = @min(length, self.size() - offset);
        const removed = try self.table.rangeAlloc(offset, actual);
        errdefer self.allocator.free(removed);

        const continuous = self.group_open and self.group_kind == .erase and
            (self.group_last_offset == offset or
                self.group_last_offset == offset + actual);
        if (!continuous) {
            if (self.group_open) self.closeGroup();
            try self.startGroup(.erase);
        }

        const group = &self.undo.items[self.undo.items.len - 1];
        try group.ops.append(self.allocator, .{
            .offset = offset,
            .removed = removed,
            .inserted = try self.allocator.alloc(u8, 0),
        });

        self.table.erase(offset, actual);
        self.group_last_offset = offset;
        self.revision += 1;
    }

    /// Replaces a range as one undoable operation.
    pub fn replace(self: *Document, offset: usize, remove_length: usize, text: []const u8) !void {
        if (offset == 0 and remove_length >= self.size()) {
            const owned = try self.allocator.dupe(u8, text);
            return self.replaceAllOwned(owned);
        }

        const clamped = @min(offset, self.size());
        const actual = @min(remove_length, self.size() - clamped);
        const removed = try self.table.rangeAlloc(clamped, actual);
        errdefer self.allocator.free(removed);
        const inserted = try self.allocator.dupe(u8, text);
        errdefer self.allocator.free(inserted);

        if (self.group_open) self.closeGroup();
        try self.startGroup(.mixed);
        const group = &self.undo.items[self.undo.items.len - 1];
        try group.ops.append(self.allocator, .{
            .offset = clamped,
            .removed = removed,
            .inserted = inserted,
        });

        self.table.erase(clamped, actual);
        try self.table.insert(clamped, text);
        self.revision += 1;
        self.closeGroup();
    }

    /// Replace the whole buffer with `text`, taking ownership of the
    /// allocation. The piece table borrows it instead of copying it again;
    /// the undo group keeps the owning reference.
    pub fn replaceAllOwned(self: *Document, text: []u8) !void {
        errdefer self.allocator.free(text);
        const removed = try self.table.rangeAlloc(0, self.size());
        errdefer self.allocator.free(removed);
        // Reserve before publishing the undo group so the table reset cannot
        // fail after the group owns `text` and `removed`.
        try self.table.reserveForText(text.len);

        if (self.group_open) self.closeGroup();
        try self.startGroup(.mixed);
        const group = &self.undo.items[self.undo.items.len - 1];
        try group.ops.append(self.allocator, .{
            .offset = 0,
            .removed = removed,
            .inserted = text,
            .replace_all = true,
        });

        try self.table.replaceAllBorrowed(text);
        self.revision += 1;
        self.closeGroup();
    }

    pub fn breakUndoGroup(self: *Document) void {
        self.closeGroup();
    }

    pub fn undoEdit(self: *Document) bool {
        if (self.undo.items.len == 0) return false;
        // Reserve before moving the group: a whole-buffer replacement may be
        // borrowing the snapshot we are about to restore.
        self.redo.ensureUnusedCapacity(self.allocator, 1) catch return false;
        self.closeGroup();
        const group = self.undo.pop().?;
        var i = group.ops.items.len;
        while (i > 0) {
            i -= 1;
            const op = group.ops.items[i];
            if (op.replace_all) {
                self.table.replaceAllBorrowed(op.removed) catch {};
            } else {
                self.table.erase(op.offset, op.inserted.len);
                self.table.insert(op.offset, op.removed) catch {};
            }
        }
        self.redo.appendAssumeCapacity(group);
        self.revision += 1;
        return true;
    }

    pub fn redoEdit(self: *Document) bool {
        if (self.redo.items.len == 0) return false;
        self.undo.ensureUnusedCapacity(self.allocator, 1) catch return false;
        self.closeGroup();
        const group = self.redo.pop().?;
        for (group.ops.items) |op| {
            if (op.replace_all) {
                self.table.replaceAllBorrowed(op.inserted) catch {};
            } else {
                self.table.erase(op.offset, op.removed.len);
                self.table.insert(op.offset, op.inserted) catch {};
            }
        }
        self.undo.appendAssumeCapacity(group);
        self.revision += 1;
        return true;
    }

    pub fn save(self: *Document, save_path: ?[]const u8) !void {
        const target = save_path orelse self.path orelse return error.NoFileName;
        if (target.len == 0) return error.NoFileName;

        const cwd = std.Io.Dir.cwd();
        var permissions = std.Io.File.Permissions.default_file;
        if (cwd.statFile(self.io, target, .{})) |stat| {
            permissions = stat.permissions;
        } else |_| {}

        var atomic = try cwd.createFileAtomic(self.io, target, .{
            .replace = true,
            .permissions = permissions,
        });
        defer atomic.deinit(self.io);

        if (self.bom) try atomic.file.writeStreamingAll(self.io, &Bom);
        try self.table.writeTo(atomic.file, self.io);
        try atomic.replace(self.io);

        // Duplicate the new name before freeing the old one: `target` may
        // point into the current path allocation when called as `:w`.
        const new_path = try self.allocator.dupe(u8, target);
        if (self.path) |old| self.allocator.free(old);
        self.path = new_path;
        self.saved_revision = self.revision;
        self.closeGroup();
    }

    fn startGroup(self: *Document, kind: GroupKind) !void {
        const group = Group{ .kind = kind };
        if (self.undo.items.len >= MaxUndoGroups) {
            const oldest = self.undo.orderedRemove(0);
            self.freeGroup(oldest);
        }
        try self.undo.append(self.allocator, group);
        self.group_open = true;
        self.group_kind = kind;
        self.group_last_offset = 0;
        self.clearRedo();
    }

    fn closeGroup(self: *Document) void {
        self.group_open = false;
        self.group_kind = .mixed;
    }

    fn clearGroups(self: *Document, groups: *std.ArrayList(Group)) void {
        for (groups.items) |group| self.freeGroup(group);
        groups.clearRetainingCapacity();
    }

    fn clearRedo(self: *Document) void {
        self.clearGroups(&self.redo);
    }

    fn freeGroup(self: *Document, group: Group) void {
        for (group.ops.items) |op| {
            self.allocator.free(op.removed);
            self.allocator.free(op.inserted);
        }
        var mutable = group;
        mutable.ops.deinit(self.allocator);
    }
};

test "document undo and redo groups adjacent typing" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var doc = try Document.open(allocator, threaded.io(), "hexalz-test-does-not-exist.txt");
    defer doc.deinit();

    try doc.insert(0, "h");
    try doc.insert(1, "i");
    try std.testing.expect(doc.isDirty());
    try std.testing.expect(doc.undoEdit());
    const text = try doc.rangeAlloc(0, doc.size());
    defer allocator.free(text);
    try std.testing.expectEqualStrings("", text);

    try std.testing.expect(doc.redoEdit());
    const text2 = try doc.rangeAlloc(0, doc.size());
    defer allocator.free(text2);
    try std.testing.expectEqualStrings("hi", text2);
}

test "document whole-buffer replacement remains undoable and editable" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var doc = Document.init(allocator, threaded.io());
    defer doc.deinit();

    try doc.insert(0, "one\ntwo");
    doc.breakUndoGroup();

    try doc.replaceAllOwned(try allocator.dupe(u8, "alpha\nbeta"));
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("alpha\nbeta", text);
    }

    // A normal edit after a bulk replacement must materialize the borrowed
    // table storage without invalidating the undo snapshot.
    try doc.insert(doc.size(), "!");
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("alpha\nbeta!", text);
    }
    try std.testing.expect(doc.undoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("alpha\nbeta", text);
    }

    try std.testing.expect(doc.undoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("one\ntwo", text);
    }
    try std.testing.expect(doc.redoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("alpha\nbeta", text);
    }
}

test "document repeated whole-buffer replacements keep independent snapshots" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var doc = Document.init(allocator, threaded.io());
    defer doc.deinit();

    try doc.replaceAllOwned(try allocator.dupe(u8, "first\nbuffer"));
    try doc.replaceAllOwned(try allocator.dupe(u8, "second\nbuffer"));
    try doc.replaceAllOwned(try allocator.dupe(u8, "third\nbuffer"));

    try std.testing.expect(doc.undoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("second\nbuffer", text);
    }
    try std.testing.expect(doc.undoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("first\nbuffer", text);
    }
    try std.testing.expect(doc.redoEdit());
    try std.testing.expect(doc.redoEdit());
    {
        const text = try doc.rangeAlloc(0, doc.size());
        defer allocator.free(text);
        try std.testing.expectEqualStrings("third\nbuffer", text);
    }
}
