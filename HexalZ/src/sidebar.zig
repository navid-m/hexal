//! File explorer tree, selection and context-menu model.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub const Action = enum {
    open,
    new_folder,
    new_file,
    rename,
    duplicate,
    format,
    diff,
    copy,
    move,
    delete,
};

pub const MenuItem = struct {
    label: []const u8,
    hotkey: u8,
    action: Action,
};

pub const file_menu = [_]MenuItem{
    .{ .label = "Open", .hotkey = 'O', .action = .open },
    .{ .label = "New Folder", .hotkey = 'X', .action = .new_folder },
    .{ .label = "New File", .hotkey = 'E', .action = .new_file },
    .{ .label = "Rename", .hotkey = 'R', .action = .rename },
    .{ .label = "Duplicate", .hotkey = 'D', .action = .duplicate },
    .{ .label = "Format", .hotkey = 'F', .action = .format },
    .{ .label = "Copy to...", .hotkey = 'T', .action = .copy },
    .{ .label = "Move to...", .hotkey = 'M', .action = .move },
    .{ .label = "Delete", .hotkey = 'd', .action = .delete },
};

pub const folder_menu = [_]MenuItem{
    .{ .label = "Expand/Collapse", .hotkey = 'O', .action = .open },
    .{ .label = "New Folder", .hotkey = 'X', .action = .new_folder },
    .{ .label = "New File", .hotkey = 'E', .action = .new_file },
    .{ .label = "Rename", .hotkey = 'R', .action = .rename },
    .{ .label = "Duplicate", .hotkey = 'D', .action = .duplicate },
    .{ .label = "Format", .hotkey = 'F', .action = .format },
    .{ .label = "Copy to...", .hotkey = 'T', .action = .copy },
    .{ .label = "Move to...", .hotkey = 'M', .action = .move },
    .{ .label = "Delete", .hotkey = 'd', .action = .delete },
};

pub const multi_menu = [_]MenuItem{
    .{ .label = "Duplicate", .hotkey = 'D', .action = .duplicate },
    .{ .label = "Format", .hotkey = 'F', .action = .format },
    .{ .label = "Copy to...", .hotkey = 'T', .action = .copy },
    .{ .label = "Move to...", .hotkey = 'M', .action = .move },
    .{ .label = "Delete", .hotkey = 'd', .action = .delete },
};

pub const two_file_menu = [_]MenuItem{
    .{ .label = "Duplicate", .hotkey = 'D', .action = .duplicate },
    .{ .label = "Format", .hotkey = 'F', .action = .format },
    .{ .label = "Diff", .hotkey = 'I', .action = .diff },
    .{ .label = "Copy to...", .hotkey = 'T', .action = .copy },
    .{ .label = "Move to...", .hotkey = 'M', .action = .move },
    .{ .label = "Delete", .hotkey = 'd', .action = .delete },
};

const min_width: usize = 20;
const max_width: usize = 80;
const min_editor_width: usize = 16;
const max_depth: usize = 24;
const max_nodes: usize = 20_000;

const ignored_names = [_][]const u8{
    ".git",              ".hg",                 ".svn",      "node_modules", "target",
    "build",             "dist",                "vendor",    ".cache",       ".idea",
    ".vscode",           "out",                 "zig-cache", ".zig-cache",   "zig-out",
    "cmake-build-debug", "cmake-build-release", "bin",       "obj",
};

pub const Node = struct {
    path: []u8,
    name: []u8,
    depth: usize,
    is_directory: bool,
    expanded: bool = false,
};

pub const Sidebar = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    visible: bool = false,
    focused: bool = false,
    menu_visible: bool = false,
    menu_selected: usize = 0,
    selected: usize = 0,
    anchor: usize = 0,
    top: usize = 0,
    show_git_ignored: bool = false,
    nodes: std.ArrayList(Node) = .empty,
    expanded: std.StringHashMap(void),
    root: []u8 = &.{},
    width_override: ?usize = null,

    pub fn init(allocator: std.mem.Allocator, io: std.Io, root: []const u8) !Sidebar {
        return .{
            .allocator = allocator,
            .io = io,
            .expanded = std.StringHashMap(void).init(allocator),
            .root = try allocator.dupe(u8, root),
        };
    }

    pub fn deinit(self: *Sidebar) void {
        self.clearNodes();
        self.nodes.deinit(self.allocator);
        var iterator = self.expanded.keyIterator();
        while (iterator.next()) |key| self.allocator.free(key.*);
        self.expanded.deinit();
        if (self.root.len > 0) self.allocator.free(self.root);
        self.* = undefined;
    }

    fn clearNodes(self: *Sidebar) void {
        for (self.nodes.items) |node| {
            self.allocator.free(node.path);
            self.allocator.free(node.name);
        }
        self.nodes.clearRetainingCapacity();
    }

    pub fn setRoot(self: *Sidebar, root: []const u8) !void {
        if (std.mem.eql(u8, self.root, root)) return;
        self.clearNodes();
        self.selected = 0;
        self.anchor = 0;
        self.top = 0;
        var iterator = self.expanded.keyIterator();
        while (iterator.next()) |key| self.allocator.free(key.*);
        self.expanded.clearRetainingCapacity();
        if (self.root.len > 0) self.allocator.free(self.root);
        self.root = try self.allocator.dupe(u8, root);
    }

    pub fn width(self: *const Sidebar, columns: usize) usize {
        if (!self.visible or columns < 36) return 0;
        const maximum = @max(min_width, @min(max_width, columns -| min_editor_width));
        const chosen = self.width_override orelse @max(min_width, @min(40, columns / 3));
        return std.math.clamp(chosen, min_width, maximum);
    }

    pub fn resize(self: *Sidebar, columns: usize, delta: isize) void {
        const maximum = @max(min_width, @min(max_width, columns -| min_editor_width));
        const current: isize = @intCast(self.width(columns));
        const target: usize = @intCast(std.math.clamp(current + delta, @as(isize, @intCast(min_width)), @as(isize, @intCast(maximum))));
        self.width_override = target;
    }

    pub fn toggle(self: *Sidebar, select_path: ?[]const u8) void {
        if (!self.visible or !self.focused) {
            self.visible = true;
            self.focused = true;
            self.menu_visible = false;
            self.expandTo(select_path);
            self.refresh(select_path);
        } else {
            self.visible = false;
            self.focused = false;
            self.menu_visible = false;
        }
    }

    pub fn refresh(self: *Sidebar, select_path: ?[]const u8) void {
        const previous = select_path orelse self.selectedNodePath();
        var previous_owned: ?[]u8 = null;
        if (previous) |path| previous_owned = self.allocator.dupe(u8, path) catch null;
        defer if (previous_owned) |path| self.allocator.free(path);
        self.clearNodes();
        self.scanDirectory(self.root, 0);
        if (previous_owned) |path| self.selectPath(path);
        if (self.nodes.items.len == 0) {
            self.selected = 0;
            self.anchor = 0;
            self.top = 0;
            return;
        }
        self.selected = @min(self.selected, self.nodes.items.len - 1);
        self.anchor = @min(self.anchor, self.nodes.items.len - 1);
    }

    fn selectedNodePath(self: *const Sidebar) ?[]const u8 {
        if (self.selected >= self.nodes.items.len) return null;
        return self.nodes.items[self.selected].path;
    }

    fn scanDirectory(self: *Sidebar, directory: []const u8, depth: usize) void {
        if (depth > max_depth or self.nodes.items.len >= max_nodes) return;
        var dir = std.Io.Dir.cwd().openDir(self.io, directory, .{ .iterate = true }) catch return;
        defer dir.close(self.io);

        const Entry = struct {
            name: []u8,
            is_directory: bool,
        };
        var entries: std.ArrayList(Entry) = .empty;
        defer {
            for (entries.items) |entry| self.allocator.free(entry.name);
            entries.deinit(self.allocator);
        }
        var iterator = dir.iterate();
        while (iterator.next(self.io) catch null) |entry| {
            if (entry.kind == .sym_link) continue;
            if (entry.kind != .directory and entry.kind != .file) continue;
            if (entry.kind == .directory and isIgnoredName(entry.name)) continue;
            entries.append(self.allocator, .{
                .name = self.allocator.dupe(u8, entry.name) catch continue,
                .is_directory = entry.kind == .directory,
            }) catch continue;
        }
        std.mem.sort(Entry, entries.items, {}, struct {
            fn lessThan(_: void, left: Entry, right: Entry) bool {
                if (left.is_directory != right.is_directory) return left.is_directory;
                return std.mem.lessThan(u8, left.name, right.name);
            }
        }.lessThan);

        for (entries.items) |entry| {
            if (self.nodes.items.len >= max_nodes) break;
            const path = std.fs.path.join(self.allocator, &.{ directory, entry.name }) catch continue;
            const expanded = if (entry.is_directory) self.expanded.contains(path) else false;
            self.nodes.append(self.allocator, .{
                .path = path,
                .name = self.allocator.dupe(u8, entry.name) catch {
                    self.allocator.free(path);
                    continue;
                },
                .depth = depth,
                .is_directory = entry.is_directory,
                .expanded = expanded,
            }) catch {
                self.allocator.free(path);
                continue;
            };
            if (entry.is_directory and expanded) self.scanDirectory(path, depth + 1);
        }
    }

    fn markExpanded(self: *Sidebar, path: []const u8) !void {
        if (self.expanded.contains(path)) return;
        const key = try self.allocator.dupe(u8, path);
        errdefer self.allocator.free(key);
        try self.expanded.put(key, {});
    }

    fn unmarkExpanded(self: *Sidebar, path: []const u8) void {
        if (self.expanded.fetchRemove(path)) |entry| self.allocator.free(entry.key);
    }

    pub fn expandTo(self: *Sidebar, path_opt: ?[]const u8) void {
        const path = path_opt orelse return;
        var directory: ?[]const u8 = path;
        const stat = std.Io.Dir.cwd().statFile(self.io, path, .{}) catch null;
        if (stat == null or stat.?.kind != .directory) {
            directory = std.fs.path.dirname(path);
        }
        while (directory) |current| {
            if (std.mem.eql(u8, current, self.root) or !isInside(self.root, current)) break;
            self.markExpanded(current) catch return;
            directory = std.fs.path.dirname(current);
        }
    }

    pub fn selectPath(self: *Sidebar, path: []const u8) void {
        for (self.nodes.items, 0..) |node, index| {
            if (std.mem.eql(u8, node.path, path)) {
                self.selected = index;
                self.anchor = index;
                return;
            }
        }
    }

    pub fn selectedNode(self: *const Sidebar) ?*const Node {
        if (self.selected >= self.nodes.items.len) return null;
        return &self.nodes.items[self.selected];
    }

    pub fn selectionStart(self: *const Sidebar) usize {
        return @min(self.selected, self.anchor);
    }

    pub fn selectionEnd(self: *const Sidebar) usize {
        return @max(self.selected, self.anchor);
    }

    pub fn selectionCount(self: *const Sidebar) usize {
        if (self.nodes.items.len == 0) return 0;
        return @max(self.selected, self.anchor) - @min(self.selected, self.anchor) + 1;
    }

    pub fn updateTop(self: *Sidebar, rows: usize) void {
        const visible_rows = @max(rows, 1);
        if (self.selected < self.top) {
            self.top = self.selected;
        } else if (self.selected >= self.top + visible_rows) {
            self.top = self.selected + 1 - visible_rows;
        }
        self.top = @min(self.top, self.nodes.items.len -| visible_rows);
    }

    pub fn moveSelection(self: *Sidebar, delta: isize, wrap: bool, extend: bool) void {
        if (self.nodes.items.len == 0) return;
        const count: isize = @intCast(self.nodes.items.len);
        var index = @as(isize, @intCast(self.selected)) + delta;
        if (wrap) {
            index = @mod(index, count);
        } else {
            index = std.math.clamp(index, 0, count - 1);
        }
        self.setSelection(@intCast(index), extend);
    }

    pub fn setSelection(self: *Sidebar, index: usize, extend: bool) void {
        if (self.nodes.items.len == 0) {
            self.selected = 0;
            self.anchor = 0;
            return;
        }
        self.selected = @min(index, self.nodes.items.len - 1);
        if (!extend) self.anchor = self.selected;
    }

    pub fn collapseOrSelectParent(self: *Sidebar) void {
        const node = self.selectedNode() orelse return;
        const selected_index = self.selected;
        if (node.is_directory and node.expanded) {
            self.unmarkExpanded(node.path);
            self.refresh(node.path);
            self.selected = @min(selected_index, self.nodes.items.len -| 1);
            return;
        }
        if (std.fs.path.dirname(node.path)) |parent| {
            if (!std.mem.eql(u8, parent, self.root) and isInside(self.root, parent)) {
                self.selectPath(parent);
            }
        }
    }

    pub fn expandOrSelectChild(self: *Sidebar) void {
        const node = self.selectedNode() orelse return;
        const selected_index = self.selected;
        if (!node.is_directory) return;
        if (!node.expanded) {
            self.markExpanded(node.path) catch return;
            self.refresh(node.path);
            self.selected = @min(selected_index, self.nodes.items.len -| 1);
            return;
        }
        if (selected_index + 1 < self.nodes.items.len and
            self.nodes.items[selected_index + 1].depth > node.depth)
        {
            self.setSelection(selected_index + 1, false);
        }
    }

    pub fn toggleDirectory(self: *Sidebar) void {
        const node = self.selectedNode() orelse return;
        const selected_index = self.selected;
        if (!node.is_directory) return;
        if (self.expanded.contains(node.path)) {
            self.unmarkExpanded(node.path);
        } else {
            self.markExpanded(node.path) catch return;
        }
        self.refresh(node.path);
        self.selected = @min(selected_index, self.nodes.items.len -| 1);
    }

    pub fn removeExpandedUnder(self: *Sidebar, path: []const u8) void {
        var removed: std.ArrayList([]u8) = .empty;
        defer removed.deinit(self.allocator);
        var iterator = self.expanded.keyIterator();
        while (iterator.next()) |key| {
            if (std.mem.eql(u8, key.*, path) or isInside(path, key.*)) {
                removed.append(self.allocator, @constCast(key.*)) catch {};
            }
        }
        for (removed.items) |key| {
            if (self.expanded.fetchRemove(key)) |entry| self.allocator.free(entry.key);
        }
    }

    pub fn updateExpandedAfterRename(self: *Sidebar, old_path: []const u8, new_path: []const u8) void {
        var updated = std.StringHashMap(void).init(self.allocator);
        var old_keys: std.ArrayList([]u8) = .empty;
        defer old_keys.deinit(self.allocator);
        var iterator = self.expanded.keyIterator();
        while (iterator.next()) |key| {
            old_keys.append(self.allocator, @constCast(key.*)) catch continue;
        }
        for (old_keys.items) |key| {
            const replacement = if (std.mem.eql(u8, key, old_path))
                self.allocator.dupe(u8, new_path) catch continue
            else if (isInside(old_path, key))
                std.fs.path.join(self.allocator, &.{
                    new_path,
                    std.mem.trimStart(u8, key[old_path.len..], "/\\"),
                }) catch continue
            else
                self.allocator.dupe(u8, key) catch continue;
            updated.put(replacement, {}) catch {
                self.allocator.free(replacement);
                continue;
            };
        }
        iterator = self.expanded.keyIterator();
        while (iterator.next()) |key| self.allocator.free(key.*);
        self.expanded.deinit();
        self.expanded = updated;
    }

    pub fn menuItems(self: *const Sidebar) []const MenuItem {
        if (self.selectionCount() > 1) {
            if (self.selectionCount() == 2) {
                const start = self.selectionStart();
                const end = self.selectionEnd();
                if (start < self.nodes.items.len and end < self.nodes.items.len and
                    !self.nodes.items[start].is_directory and
                    !self.nodes.items[end].is_directory)
                {
                    return &two_file_menu;
                }
            }
            return &multi_menu;
        }
        const node = self.selectedNode() orelse return &file_menu;
        return if (node.is_directory) &folder_menu else &file_menu;
    }

    pub fn menuHotkey(self: *const Sidebar, character: u8) ?usize {
        const items = self.menuItems();
        for (items, 0..) |item, index| {
            if (item.hotkey == character) return index;
        }
        const upper = std.ascii.toUpper(character);
        for (items, 0..) |item, index| {
            if (item.hotkey == upper) return index;
        }
        return null;
    }

    pub fn selectedNodes(self: *const Sidebar, allocator: std.mem.Allocator) ![]const Node {
        if (self.nodes.items.len == 0) return allocator.alloc(Node, 0);
        const start = @min(self.selectionStart(), self.nodes.items.len - 1);
        const end = @min(self.selectionEnd(), self.nodes.items.len - 1);
        return allocator.dupe(Node, self.nodes.items[start .. end + 1]);
    }

    pub fn effectiveSelection(self: *const Sidebar, allocator: std.mem.Allocator) !std.ArrayList(Node) {
        const selected = try self.selectedNodes(allocator);
        defer allocator.free(selected);
        var result: std.ArrayList(Node) = .empty;
        errdefer result.deinit(allocator);
        if (selected.len < 2) {
            try result.appendSlice(allocator, selected);
            return result;
        }
        var has_directory = false;
        for (selected) |node| {
            if (node.is_directory) has_directory = true;
        }
        if (!has_directory) {
            try result.appendSlice(allocator, selected);
            return result;
        }
        for (selected) |node| {
            var covered = false;
            for (selected) |directory| {
                if (directory.is_directory and
                    !std.mem.eql(u8, node.path, directory.path) and
                    isInside(directory.path, node.path))
                {
                    covered = true;
                    break;
                }
            }
            if (!covered) try result.append(allocator, node);
        }
        return result;
    }
};

pub fn isInside(parent: []const u8, child: []const u8) bool {
    if (child.len <= parent.len or !std.mem.startsWith(u8, child, parent)) return false;
    const boundary = child[parent.len];
    return boundary == '/' or boundary == '\\';
}

pub fn isIgnoredName(name: []const u8) bool {
    for (ignored_names) |ignored| {
        if (std.mem.eql(u8, name, ignored)) return true;
    }
    return std.mem.startsWith(u8, name, ".zig-cache");
}

pub fn fileName(path: []const u8) []const u8 {
    return std.fs.path.basename(path);
}

pub fn parentOf(path: []const u8) []const u8 {
    return std.fs.path.dirname(path) orelse ".";
}
