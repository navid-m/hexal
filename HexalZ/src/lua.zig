//! Embedded Lua 5.4 plugin host.
//!
//! Each plugin gets its own Lua state and a small, documented `hexal.*` API.
//! The host mirrors the active buffer into the plugin, applies plugin edits as
//! one undoable replacement, and owns plugin panels, commands and callbacks.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const alloc_mod = @import("alloc.zig");
const document_mod = @import("document.zig");
const files_mod = @import("files.zig");
const json_mod = @import("json.zig");

const c = @import("lua_c.zig");
const registry_index: c_int = c.LUA_REGISTRYINDEX;

pub const Message = struct {
    text: []u8,
    is_error: bool,
};

pub const PendingCommand = struct {
    name: []u8,
    args: std.ArrayList([]u8) = .empty,
};

pub const PanelSegment = struct {
    text: []u8,
    style: []u8,
};

pub const PanelLine = struct {
    segments: []PanelSegment,
    selectable: bool = true,
};

pub const Panel = struct {
    id: i64,
    plugin_id: []u8,
    title: []u8,
    footer: []u8,
    selectable: bool = true,
    selected: usize = 0,
    top: usize = 0,
    lines: std.ArrayList(PanelLine) = .empty,

    fn deinit(self: *Panel, allocator: std.mem.Allocator) void {
        allocator.free(self.plugin_id);
        allocator.free(self.title);
        allocator.free(self.footer);
        freePanelLines(allocator, &self.lines);
        self.lines.deinit(allocator);
    }
};

fn freePanelLines(allocator: std.mem.Allocator, lines: *std.ArrayList(PanelLine)) void {
    for (lines.items) |line| {
        for (line.segments) |segment| {
            allocator.free(segment.text);
            allocator.free(segment.style);
        }
        allocator.free(line.segments);
    }
    lines.clearRetainingCapacity();
}

pub const Host = struct {
    allocator: std.mem.Allocator,
    columns: usize = 80,
    rows: usize = 24,
    buffer_id: []u8 = &.{},
    path: ?[]u8 = null,
    language: []u8 = &.{},
    workspace_root: ?[]u8 = null,
    messages: std.ArrayList(Message) = .empty,
    clipboard: ?[]u8 = null,
    panels: std.ArrayList(Panel) = .empty,
    next_panel_id: i64 = 1,
    pending_commands: std.ArrayList(PendingCommand) = .empty,
    next_request: i64 = 1,
    prompts: std.ArrayList(PromptRequest) = .empty,
    selection: ?Selection = null,

    pub const Selection = struct {
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
        text: []u8,
        linewise: bool,
    };

    fn init(allocator: std.mem.Allocator) Host {
        return .{ .allocator = allocator };
    }

    fn deinit(self: *Host) void {
        const a = self.allocator;
        if (self.buffer_id.len > 0) a.free(self.buffer_id);
        if (self.path) |value| a.free(value);
        if (self.language.len > 0) a.free(self.language);
        if (self.workspace_root) |value| a.free(value);
        for (self.messages.items) |message| a.free(message.text);
        self.messages.deinit(a);
        if (self.clipboard) |value| a.free(value);
        for (self.panels.items) |*panel| panel.deinit(a);
        self.panels.deinit(a);
        for (self.pending_commands.items) |*command| {
            a.free(command.name);
            for (command.args.items) |arg| a.free(arg);
            command.args.deinit(a);
        }
        self.pending_commands.deinit(a);
        for (self.prompts.items) |prompt| a.free(prompt.label);
        self.prompts.deinit(a);
        if (self.selection) |selection| a.free(selection.text);
        self.* = undefined;
    }

    fn setBufferId(self: *Host, value: []const u8) void {
        if (self.buffer_id.len > 0) self.allocator.free(self.buffer_id);
        self.buffer_id = self.allocator.dupe(u8, value) catch &.{};
    }

    fn setPath(self: *Host, value: ?[]const u8) void {
        if (self.path) |old| self.allocator.free(old);
        self.path = if (value) |path| self.allocator.dupe(u8, path) catch null else null;
    }

    fn setLanguage(self: *Host, value: []const u8) void {
        if (self.language.len > 0) self.allocator.free(self.language);
        self.language = self.allocator.dupe(u8, value) catch &.{};
    }

    fn setWorkspaceRoot(self: *Host, value: []const u8) void {
        if (self.workspace_root) |old| self.allocator.free(old);
        self.workspace_root = self.allocator.dupe(u8, value) catch null;
    }

    fn pushMessage(self: *Host, text: []const u8, is_error: bool) void {
        const copy = self.allocator.dupe(u8, text) catch return;
        self.messages.append(self.allocator, .{ .text = copy, .is_error = is_error }) catch {
            self.allocator.free(copy);
        };
    }

    fn setClipboard(self: *Host, text: []const u8) void {
        if (self.clipboard) |old| self.allocator.free(old);
        self.clipboard = self.allocator.dupe(u8, text) catch null;
    }

    fn appendPendingCommand(self: *Host, name: []const u8, args: []const []const u8) void {
        const name_copy = self.allocator.dupe(u8, name) catch return;
        var command = PendingCommand{ .name = name_copy };
        for (args) |arg| {
            const copy = self.allocator.dupe(u8, arg) catch continue;
            command.args.append(self.allocator, copy) catch self.allocator.free(copy);
        }
        self.pending_commands.append(self.allocator, command) catch {
            self.allocator.free(name_copy);
            for (command.args.items) |arg| self.allocator.free(arg);
            command.args.deinit(self.allocator);
        };
    }
};

pub const PromptRequest = struct {
    id: i64,
    label: []u8,
};

const CommandEntry = struct {
    name: []u8,
    description: []u8,
    reference: c_int,
};

const Subscription = struct {
    id: i64,
    event: []u8,
    reference: c_int,
};

const DeferredTask = struct {
    due_ns: i96,
    reference: c_int,
    arg_reference: c_int = c.LUA_NOREF,
};

const PanelHandler = struct {
    panel_id: i64,
    reference: c_int,
};

const PromptCallback = struct {
    id: i64,
    reference: c_int,
};

const KeyBinding = struct {
    keys: []u8,
    mode: []u8,
    command: ?[]u8 = null,
    reference: c_int = c.LUA_NOREF,
};

/// State shared by all API callbacks belonging to one plugin.
pub const Context = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    host: *Host,
    plugin_id: []u8,
    plugin_dir: []u8,
    text: std.ArrayList(u8) = .empty,
    line_starts: std.ArrayList(usize) = .empty,
    crlf: bool = false,
    cursor_line: usize = 0,
    cursor_column: usize = 0,
    dirty: bool = false,
    revision: ?u64 = null,
    synced_cursor: ?usize = null,
    commands: std.StringHashMap(CommandEntry),
    subscriptions: std.ArrayList(Subscription) = .empty,
    deferred: std.ArrayList(DeferredTask) = .empty,
    panel_handlers: std.ArrayList(PanelHandler) = .empty,
    prompts: std.ArrayList(PromptCallback) = .empty,
    keybindings: std.ArrayList(KeyBinding) = .empty,
    config_ref: c_int = c.LUA_NOREF,
    next_subscription: i64 = 1,
    last_error: ?[]u8 = null,

    fn init(
        allocator: std.mem.Allocator,
        io: std.Io,
        host: *Host,
        plugin_id: []const u8,
        plugin_dir: []const u8,
    ) !Context {
        var ctx = Context{
            .allocator = allocator,
            .io = io,
            .host = host,
            .plugin_id = try allocator.dupe(u8, plugin_id),
            .plugin_dir = try allocator.dupe(u8, plugin_dir),
            .commands = std.StringHashMap(CommandEntry).init(allocator),
        };
        try ctx.line_starts.append(allocator, 0);
        return ctx;
    }

    fn deinit(self: *Context) void {
        const a = self.allocator;
        a.free(self.plugin_id);
        a.free(self.plugin_dir);
        self.text.deinit(a);
        self.line_starts.deinit(a);
        var command_iterator = self.commands.iterator();
        while (command_iterator.next()) |entry| {
            a.free(entry.key_ptr.*);
            a.free(entry.value_ptr.name);
            a.free(entry.value_ptr.description);
        }
        self.commands.deinit();
        for (self.subscriptions.items) |subscription| {
            a.free(subscription.event);
        }
        self.subscriptions.deinit(a);
        self.deferred.deinit(a);
        self.panel_handlers.deinit(a);
        self.prompts.deinit(a);
        for (self.keybindings.items) |binding| {
            a.free(binding.keys);
            a.free(binding.mode);
            if (binding.command) |command| a.free(command);
        }
        self.keybindings.deinit(a);
        if (self.last_error) |message| a.free(message);
        self.* = undefined;
    }

    fn bytes(self: *const Context) []const u8 {
        return self.text.items;
    }

    fn load(self: *Context, doc: *document_mod.Document, cursor_line: usize, cursor_column: usize) !void {
        const data = try doc.rangeAlloc(0, doc.size());
        defer self.allocator.free(data);
        self.text.clearRetainingCapacity();
        try self.text.appendSlice(self.allocator, data);
        self.crlf = doc.eol == .crlf;
        try self.rebuildLines();
        self.cursor_line = @min(cursor_line, self.lineCount() - 1);
        self.cursor_column = cursor_column;
        self.dirty = false;
    }

    fn rebuildLines(self: *Context) !void {
        const text = self.bytes();
        const newlines = std.mem.count(u8, text, "\n");
        self.line_starts.clearRetainingCapacity();
        try self.line_starts.ensureTotalCapacity(self.allocator, newlines + 1);
        self.line_starts.appendAssumeCapacity(0);
        var search_from: usize = 0;
        while (std.mem.indexOfScalarPos(u8, text, search_from, '\n')) |index| {
            self.line_starts.appendAssumeCapacity(index + 1);
            search_from = index + 1;
        }
    }

    fn lineCount(self: *const Context) usize {
        return self.line_starts.items.len;
    }

    fn lineStart(self: *const Context, line: usize) usize {
        return self.line_starts.items[@min(line, self.lineCount() - 1)];
    }

    fn lineEnd(self: *const Context, line: usize) usize {
        const text = self.bytes();
        if (line + 1 < self.lineCount()) {
            var end = self.line_starts.items[line + 1];
            if (end > 0 and text[end - 1] == '\n') {
                end -= 1;
                if (end > 0 and text[end - 1] == '\r') end -= 1;
            }
            return end;
        }
        return text.len;
    }

    const Bounds = struct { start: usize, end: usize };

    fn lineBounds(self: *const Context, line: usize) Bounds {
        const starts = self.line_starts.items;
        const text = self.bytes();
        const start = starts[line];
        if (line + 1 >= starts.len) return .{ .start = start, .end = text.len };
        var end = starts[line + 1] - 1;
        if (self.crlf and end > start and text[end - 1] == '\r') end -= 1;
        return .{ .start = start, .end = end };
    }

    fn lineBytes(self: *const Context, line: usize) ![]const u8 {
        if (line >= self.lineCount()) return error.InvalidLine;
        const bounds = self.lineBounds(line);
        return self.bytes()[bounds.start..bounds.end];
    }

    fn setLine(self: *Context, line: usize, value: []const u8) !void {
        if (line >= self.lineCount()) return error.InvalidLine;
        const start = self.lineStart(line);
        const end = self.lineEnd(line);
        try self.text.replaceRange(self.allocator, start, end - start, value);
        try self.rebuildLines();
        self.dirty = true;
    }

    fn setText(self: *Context, value: []const u8) !void {
        self.text.clearRetainingCapacity();
        try self.text.appendSlice(self.allocator, value);
        self.crlf = if (std.mem.indexOfScalar(u8, value, '\n')) |index|
            index > 0 and value[index - 1] == '\r'
        else
            false;
        try self.rebuildLines();
        self.cursor_line = @min(self.cursor_line, self.lineCount() - 1);
        self.dirty = true;
    }

    fn byteOffset(self: *const Context) usize {
        const line = @min(self.cursor_line, self.line_starts.items.len - 1);
        if (self.cursor_column == 0) return self.line_starts.items[line];
        const bounds = self.lineBounds(line);
        return bounds.start + @min(self.cursor_column, bounds.end - bounds.start);
    }

    fn setCursor(self: *Context, line: usize, column: usize) void {
        self.cursor_line = @min(line, self.line_starts.items.len - 1);
        if (column == 0) {
            self.cursor_column = 0;
            return;
        }
        const bounds = self.lineBounds(self.cursor_line);
        self.cursor_column = @min(column, bounds.end - bounds.start);
    }

    fn insert(self: *Context, value: []const u8) !void {
        const offset = self.byteOffset();
        try self.text.insertSlice(self.allocator, offset, value);
        try self.rebuildLines();
        const new_offset = offset + value.len;
        const line = self.lineOfOffset(new_offset);
        self.cursor_line = line;
        self.cursor_column = new_offset - self.lineStart(line);
        self.dirty = true;
    }

    fn replaceRange(self: *Context, start: usize, end: usize, value: []const u8) !void {
        if (end < start or end > self.text.items.len) return error.InvalidRange;
        try self.text.replaceRange(self.allocator, start, end - start, value);
        try self.rebuildLines();
        self.cursor_line = @min(self.cursor_line, self.lineCount() - 1);
        self.dirty = true;
    }

    fn lineOfOffset(self: *const Context, offset: usize) usize {
        var low: usize = 0;
        var high: usize = self.lineCount();
        while (low + 1 < high) {
            const middle = low + (high - low) / 2;
            if (self.line_starts.items[middle] <= offset) low = middle else high = middle;
        }
        return low;
    }

    fn setLastError(self: *Context, message: []const u8) void {
        if (self.last_error) |old| self.allocator.free(old);
        self.last_error = self.allocator.dupe(u8, message) catch null;
    }

    fn findPanel(self: *Context, id: i64) ?*Panel {
        for (self.host.panels.items) |*panel| {
            if (panel.id == id) return panel;
        }
        return null;
    }
};

const LuaAllocator = struct {
    const grain = 16;
    const small_limit = 1024;
    const class_count = small_limit / grain;
    const max_cached_per_class = 512;
    const Node = struct { next: ?*Node };

    allocator: std.mem.Allocator,
    heads: [class_count]?*Node = @splat(null),
    counts: [class_count]u16 = @splat(0),

    fn classFor(size: usize) ?usize {
        if (size == 0 or size > small_limit) return null;
        return (size + grain - 1) / grain - 1;
    }

    fn classSize(index: usize) usize {
        return (index + 1) * grain;
    }

    fn allocSmall(self: *LuaAllocator, size: usize) ?*anyopaque {
        const index = classFor(size) orelse return null;
        if (self.heads[index]) |head| {
            self.heads[index] = head.next;
            self.counts[index] -= 1;
            return head;
        }
        return std.c.malloc(classSize(index));
    }

    fn freeSmall(self: *LuaAllocator, pointer: ?*anyopaque, size: usize) bool {
        const index = classFor(size) orelse return false;
        if (self.counts[index] >= max_cached_per_class) return false;
        const node: *Node = @ptrCast(@alignCast(pointer.?));
        node.next = self.heads[index];
        self.heads[index] = node;
        self.counts[index] += 1;
        return true;
    }

    fn deinitSmall(self: *LuaAllocator) void {
        for (&self.heads) |*head| {
            while (head.*) |node| {
                head.* = node.next;
                std.c.free(node);
            }
        }
    }
};

fn contextCommandRef(ctx: *Context, name: []const u8) ?c_int {
    var buffer: [256]u8 = undefined;
    if (name.len > buffer.len) return null;
    const lower = std.ascii.lowerString(buffer[0..name.len], name);
    const entry = ctx.commands.get(lower) orelse return null;
    return entry.reference;
}

pub const Runtime = struct {
    allocator: std.mem.Allocator,
    state: *c.lua_State,
    lua_alloc: *LuaAllocator,
    ctx: *Context,
    plugin_id: []u8,
    plugin_dir: []u8,
    entry_path: []u8,
    language: []u8,
    lazy: bool,
    loaded: bool = false,
    error_message: ?[]u8 = null,

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        self.lua_alloc.deinitSmall();
        self.ctx.deinit();
        self.allocator.destroy(self.ctx);
        self.allocator.destroy(self.lua_alloc);
        self.allocator.free(self.plugin_id);
        self.allocator.free(self.plugin_dir);
        self.allocator.free(self.entry_path);
        self.allocator.free(self.language);
        if (self.error_message) |message| self.allocator.free(message);
        self.allocator.destroy(self);
    }

    fn start(
        allocator: std.mem.Allocator,
        io: std.Io,
        host: *Host,
        plugin_id: []const u8,
        plugin_dir: []const u8,
        entry_path: []const u8,
        language: []const u8,
        lazy: bool,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !*Runtime {
        const runtime = try allocator.create(Runtime);
        errdefer allocator.destroy(runtime);
        const lua_alloc = try allocator.create(LuaAllocator);
        errdefer allocator.destroy(lua_alloc);
        lua_alloc.* = .{ .allocator = allocator };
        const state = c.lua_newstate(luaAlloc, lua_alloc) orelse return error.OutOfMemory;
        errdefer c.lua_close(state);
        openSafeLibs(state);

        const ctx = try allocator.create(Context);
        errdefer allocator.destroy(ctx);
        ctx.* = try Context.init(allocator, io, host, plugin_id, plugin_dir);
        errdefer ctx.deinit();

        runtime.* = .{
            .allocator = allocator,
            .state = state,
            .lua_alloc = lua_alloc,
            .ctx = ctx,
            .plugin_id = try allocator.dupe(u8, plugin_id),
            .plugin_dir = try allocator.dupe(u8, plugin_dir),
            .entry_path = try allocator.dupe(u8, entry_path),
            .language = try allocator.dupe(u8, language),
            .lazy = lazy,
        };
        errdefer {
            allocator.free(runtime.plugin_id);
            allocator.free(runtime.plugin_dir);
            allocator.free(runtime.entry_path);
            allocator.free(runtime.language);
        }

        registerApi(runtime) catch |err| {
            runtime.setErrorFmt("cannot register plugin API: {s}", .{@errorName(err)});
            return err;
        };
        runtime.syncFromDocument(doc, cursor.*);
        runtime.loadFile() catch |err| {
            if (runtime.ctx.last_error) |message| {
                runtime.setError(message);
                runtime.ctx.host.pushMessage(message, true);
            } else {
                runtime.setErrorFmt("cannot load plugin: {s}", .{@errorName(err)});
            }
            runtime.processEffects(doc, cursor);
            return err;
        };
        runtime.loaded = true;
        runtime.processEffects(doc, cursor);
        return runtime;
    }

    fn setError(self: *Runtime, message: []const u8) void {
        if (self.error_message) |old| self.allocator.free(old);
        self.error_message = self.allocator.dupe(u8, message) catch null;
    }

    fn setErrorFmt(self: *Runtime, comptime fmt: []const u8, args: anytype) void {
        var buffer: [1024]u8 = undefined;
        const text = std.fmt.bufPrint(&buffer, fmt, args) catch return;
        self.setError(text);
    }

    fn loadFile(self: *Runtime) !void {
        const path_z = try self.allocator.alloc(u8, self.entry_path.len + 1);
        defer self.allocator.free(path_z);
        @memcpy(path_z[0..self.entry_path.len], self.entry_path);
        path_z[self.entry_path.len] = 0;
        if (c.luaL_loadfilex(self.state, path_z.ptr, null) != c.LUA_OK) {
            return self.captureError();
        }
        if (c.lua_pcallk(self.state, 0, 0, 0, 0, null) != c.LUA_OK) {
            return self.captureError();
        }
    }

    fn syncFromDocument(self: *Runtime, doc: *document_mod.Document, cursor: usize) void {
        if (self.ctx.revision != null and self.ctx.revision.? == doc.revision and !self.ctx.dirty) {
            if (self.ctx.synced_cursor != cursor) {
                const line = doc.lineOfOffset(cursor);
                const column = cursor - doc.lineStart(line);
                self.ctx.setCursor(line, column);
                self.ctx.synced_cursor = cursor;
            }
            return;
        }
        const line = doc.lineOfOffset(cursor);
        const column = cursor - doc.lineStart(line);
        self.ctx.load(doc, line, column) catch {};
        self.ctx.revision = doc.revision;
        self.ctx.synced_cursor = cursor;
    }

    fn processEffects(self: *Runtime, doc: *document_mod.Document, cursor: *usize) void {
        if (self.ctx.dirty) {
            const previous_offset = @min(self.ctx.byteOffset(), self.ctx.text.items.len);
            const owned = self.ctx.text.toOwnedSlice(self.allocator) catch {
                self.ctx.dirty = false;
                return;
            };
            doc.replaceAllOwned(owned) catch {
                self.allocator.free(owned);
                self.ctx.dirty = false;
                return;
            };
            self.ctx.text = .empty;
            const offset = @min(previous_offset, doc.size());
            const line = doc.lineOfOffset(offset);
            const column = offset - doc.lineStart(line);
            self.ctx.load(doc, line, column) catch {};
            self.ctx.synced_cursor = offset;
            cursor.* = offset;
            self.ctx.dirty = false;
        } else if (self.ctx.synced_cursor) |synced| {
            const offset = @min(self.ctx.byteOffset(), doc.size());
            if (offset != synced) {
                cursor.* = offset;
                self.ctx.synced_cursor = offset;
            }
        }
        self.ctx.revision = doc.revision;
    }

    fn captureError(self: *Runtime) error{LuaError} {
        var length: usize = 0;
        const pointer = c.lua_tolstring(self.state, -1, &length);
        if (pointer != null and length > 0) {
            const message = pointer[0..length];
            self.ctx.setLastError(message);
        }
        c.lua_settop(self.state, 0);
        return error.LuaError;
    }

    fn commandRef(self: *Runtime, name: []const u8) ?c_int {
        return contextCommandRef(self.ctx, name);
    }

    fn callCommand(
        self: *Runtime,
        name: []const u8,
        args: []const []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !bool {
        const reference = self.commandRef(name) orelse return false;
        self.syncFromDocument(doc, cursor.*);
        _ = c.lua_rawgeti(self.state, registry_index, reference);
        for (args) |arg| pushLString(self.state, arg);
        const status = c.lua_pcallk(self.state, @intCast(args.len), 0, 0, 0, null);
        self.processEffects(doc, cursor);
        if (status != c.LUA_OK) return self.captureError();
        return true;
    }

    fn callReference(self: *Runtime, reference: c_int, args: []const []const u8, results: c_int) !c_int {
        _ = c.lua_rawgeti(self.state, registry_index, reference);
        for (args) |arg| pushLString(self.state, arg);
        const status = c.lua_pcallk(self.state, @intCast(args.len), results, 0, 0, null);
        if (status != c.LUA_OK) return self.captureError();
        return c.lua_gettop(self.state);
    }

    fn fireEvent(
        self: *Runtime,
        event: []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
        extra_language: ?[]const u8,
    ) void {
        self.syncFromDocument(doc, cursor.*);
        var any = false;
        for (self.ctx.subscriptions.items) |subscription| {
            if (!std.ascii.eqlIgnoreCase(subscription.event, event)) continue;
            any = true;
            pushEventTable(self.state, self.ctx, event, extra_language);
            _ = c.lua_rawgeti(self.state, registry_index, subscription.reference);
            c.lua_pushvalue(self.state, -2);
            if (c.lua_pcallk(self.state, 1, 0, 0, 0, null) != c.LUA_OK) {
                self.captureError() catch {};
                const message = self.ctx.last_error orelse "event callback failed";
                self.ctx.host.pushMessage(message, true);
            }
            c.lua_settop(self.state, 0);
        }
        if (any) self.processEffects(doc, cursor);
    }

    fn runDeferred(self: *Runtime, doc: *document_mod.Document, cursor: *usize) bool {
        const now = std.Io.Clock.Timestamp.now(self.ctx.io, .awake).raw.nanoseconds;
        var ran = false;
        var index: usize = 0;
        while (index < self.ctx.deferred.items.len) {
            const task = self.ctx.deferred.items[index];
            if (task.due_ns > now) {
                index += 1;
                continue;
            }
            _ = self.ctx.deferred.orderedRemove(index);
            self.syncFromDocument(doc, cursor.*);
            _ = c.lua_rawgeti(self.state, registry_index, task.reference);
            var nargs: c_int = 0;
            if (task.arg_reference != c.LUA_NOREF) {
                _ = c.lua_rawgeti(self.state, registry_index, task.arg_reference);
                nargs = 1;
            }
            if (c.lua_pcallk(self.state, nargs, 0, 0, 0, null) != c.LUA_OK) {
                self.captureError() catch {};
                const message = self.ctx.last_error orelse "deferred callback failed";
                self.ctx.host.pushMessage(message, true);
            }
            c.lua_settop(self.state, 0);
            c.luaL_unref(self.state, registry_index, task.reference);
            if (task.arg_reference != c.LUA_NOREF) {
                c.luaL_unref(self.state, registry_index, task.arg_reference);
            }
            ran = true;
            index = 0;
        }
        if (ran) self.processEffects(doc, cursor);
        return ran;
    }

    fn handlePanelKey(
        self: *Runtime,
        panel_id: i64,
        key: []const u8,
        selected: usize,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !?bool {
        var reference: ?c_int = null;
        for (self.ctx.panel_handlers.items) |handler| {
            if (handler.panel_id == panel_id) reference = handler.reference;
        }
        const ref = reference orelse return null;
        self.syncFromDocument(doc, cursor.*);
        _ = c.lua_rawgeti(self.state, registry_index, ref);
        pushLString(self.state, key);
        c.lua_pushinteger(self.state, @intCast(selected + 1));
        const status = c.lua_pcallk(self.state, 2, 1, 0, 0, null);
        if (status != c.LUA_OK) {
            self.processEffects(doc, cursor);
            return self.captureError();
        }
        const handled = c.lua_toboolean(self.state, -1) != 0;
        c.lua_settop(self.state, 0);
        self.processEffects(doc, cursor);
        return handled;
    }

    fn resolvePrompt(self: *Runtime, id: i64, value: ?[]const u8) bool {
        var index: ?usize = null;
        for (self.ctx.prompts.items, 0..) |prompt, i| {
            if (prompt.id == id) index = i;
        }
        const i = index orelse return false;
        const prompt = self.ctx.prompts.orderedRemove(i);
        _ = c.lua_rawgeti(self.state, registry_index, prompt.reference);
        if (value) |text| pushLString(self.state, text) else c.lua_pushnil(self.state);
        if (c.lua_pcallk(self.state, 1, 0, 0, 0, null) != c.LUA_OK) {
            self.captureError() catch {};
        }
        c.lua_settop(self.state, 0);
        c.luaL_unref(self.state, registry_index, prompt.reference);
        return true;
    }
};

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub const PluginEntry = struct {
    id: []u8,
    name: []u8,
    version: []u8,
    description: []u8,
    author: []u8,
    license: []u8,
    directory: []u8,
    entry_path: []u8,
    manifest_path: []u8,
    language: []u8,
    lazy: bool,
    enabled: bool,
    runtime: ?*Runtime = null,
    error_message: ?[]u8 = null,
};

pub const Manager = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    host: *Host,
    entries: std.ArrayList(PluginEntry) = .empty,
    errors: std.ArrayList([]u8) = .empty,

    pub fn init(allocator: std.mem.Allocator, io: std.Io) !Manager {
        const host = try allocator.create(Host);
        host.* = Host.init(allocator);
        return .{ .allocator = allocator, .io = io, .host = host };
    }

    pub fn deinit(self: *Manager) void {
        for (self.entries.items) |*entry| {
            if (entry.runtime) |runtime| runtime.deinit();
            self.allocator.free(entry.id);
            self.allocator.free(entry.name);
            self.allocator.free(entry.version);
            self.allocator.free(entry.description);
            self.allocator.free(entry.author);
            self.allocator.free(entry.license);
            self.allocator.free(entry.directory);
            self.allocator.free(entry.entry_path);
            self.allocator.free(entry.manifest_path);
            self.allocator.free(entry.language);
            if (entry.error_message) |message| self.allocator.free(message);
        }
        self.entries.deinit(self.allocator);
        for (self.errors.items) |message| self.allocator.free(message);
        self.errors.deinit(self.allocator);
        self.host.deinit();
        self.allocator.destroy(self.host);
        self.* = undefined;
    }

    pub fn setContext(
        self: *Manager,
        buffer_id: []const u8,
        path: ?[]const u8,
        language: []const u8,
        workspace_root: []const u8,
        columns: usize,
        rows: usize,
        selection: ?Host.Selection,
    ) void {
        self.host.setBufferId(buffer_id);
        self.host.setPath(path);
        self.host.setLanguage(language);
        self.host.setWorkspaceRoot(workspace_root);
        self.host.columns = columns;
        self.host.rows = rows;
        if (self.host.selection) |old| self.allocator.free(old.text);
        self.host.selection = selection;
    }

    pub fn loadAll(
        self: *Manager,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !void {
        const candidates = [_][]const u8{ "data/plugins", "plugins", "../data/plugins" };
        for (candidates) |candidate| try self.loadDirectory(candidate, doc, cursor);

        if (std.process.executableDirPathAlloc(self.io, self.allocator)) |executable_dir| {
            defer self.allocator.free(executable_dir);
            var current: ?[]const u8 = executable_dir;
            var levels: usize = 0;
            while (current) |directory| : (levels += 1) {
                if (levels > 3) break;
                const candidate = std.fs.path.join(self.allocator, &.{ directory, "data/plugins" }) catch break;
                defer self.allocator.free(candidate);
                try self.loadDirectory(candidate, doc, cursor);
                current = std.fs.path.dirname(directory);
            }
        } else |_| {}
    }

    pub fn loadDirectory(
        self: *Manager,
        directory_path: []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !void {
        var dir = std.Io.Dir.cwd().openDir(self.io, directory_path, .{ .iterate = true }) catch return;
        defer dir.close(self.io);
        var names: std.ArrayList([]u8) = .empty;
        defer {
            for (names.items) |name| self.allocator.free(name);
            names.deinit(self.allocator);
        }
        var iterator = dir.iterate();
        while (try iterator.next(self.io)) |entry| {
            if (entry.kind != .directory) continue;
            try names.append(self.allocator, try self.allocator.dupe(u8, entry.name));
        }
        std.mem.sort([]u8, names.items, {}, struct {
            fn lessThan(_: void, left: []u8, right: []u8) bool {
                return std.ascii.lessThanIgnoreCase(left, right);
            }
        }.lessThan);
        for (names.items) |name| {
            const directory = try std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ directory_path, name });
            defer self.allocator.free(directory);
            var already_loaded = false;
            for (self.entries.items) |existing| {
                if (std.mem.eql(u8, existing.directory, directory)) {
                    already_loaded = true;
                    break;
                }
            }
            if (!already_loaded) self.discover(directory, doc, cursor);
        }
    }

    pub fn loadFilePlugin(
        self: *Manager,
        path: []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !void {
        const directory = std.fs.path.dirname(path) orelse ".";
        const id = std.fs.path.stem(path);
        const entry = PluginEntry{
            .id = try self.allocator.dupe(u8, id),
            .name = try self.allocator.dupe(u8, id),
            .version = try self.allocator.dupe(u8, ""),
            .description = try self.allocator.dupe(u8, ""),
            .author = try self.allocator.dupe(u8, ""),
            .license = try self.allocator.dupe(u8, ""),
            .directory = try self.allocator.dupe(u8, directory),
            .entry_path = try self.allocator.dupe(u8, path),
            .manifest_path = try self.allocator.dupe(u8, ""),
            .language = try self.allocator.dupe(u8, ""),
            .lazy = false,
            .enabled = true,
        };
        try self.entries.append(self.allocator, entry);
        const index = self.entries.items.len - 1;
        self.startEntry(index, doc, cursor);
        if (self.entries.items[index].runtime == null) return error.PluginLoadFailed;
    }

    fn discover(self: *Manager, directory: []const u8, doc: *document_mod.Document, cursor: *usize) void {
        const fallback = std.fs.path.basename(directory);
        var id = fallback;
        var name = fallback;
        var version: []const u8 = "";
        var description: []const u8 = "";
        var author: []const u8 = "";
        var license: []const u8 = "";
        var entry_file: []const u8 = "init.lua";
        var language: []const u8 = "";
        var lazy = false;
        var enabled = true;

        const manifest_path = std.fmt.allocPrint(self.allocator, "{s}/plugin.toml", .{directory}) catch return;
        defer self.allocator.free(manifest_path);
        const manifest_text: ?[]u8 =
            std.Io.Dir.cwd().readFileAlloc(self.io, manifest_path, self.allocator, .limited(512 * 1024)) catch null;
        defer if (manifest_text) |text| self.allocator.free(text);
        if (manifest_text) |text| {
            var lines = std.mem.splitScalar(u8, text, '\n');
            while (lines.next()) |raw_line| {
                const line = std.mem.trim(u8, raw_line, " \t\r");
                if (line.len == 0 or line[0] == '#' or line[0] == '[') continue;
                const separator = std.mem.indexOfScalar(u8, line, '=') orelse continue;
                const key = std.mem.trim(u8, line[0..separator], " \t");
                var value = std.mem.trim(u8, line[separator + 1 ..], " \t");
                if (value.len >= 2 and
                    ((value[0] == '"' and value[value.len - 1] == '"') or
                        (value[0] == '\'' and value[value.len - 1] == '\'')))
                {
                    value = value[1 .. value.len - 1];
                }
                if (std.ascii.eqlIgnoreCase(key, "id") and value.len > 0) id = value else if
                    (std.ascii.eqlIgnoreCase(key, "name") and value.len > 0) name = value else if
                    (std.ascii.eqlIgnoreCase(key, "version")) version = value else if
                    (std.ascii.eqlIgnoreCase(key, "description")) description = value else if
                    (std.ascii.eqlIgnoreCase(key, "author")) author = value else if
                    (std.ascii.eqlIgnoreCase(key, "license")) license = value else if
                    (std.ascii.eqlIgnoreCase(key, "entry") and value.len > 0) entry_file = value else if
                    (std.ascii.eqlIgnoreCase(key, "language")) language = value else if
                    (std.ascii.eqlIgnoreCase(key, "lazy")) lazy = std.ascii.eqlIgnoreCase(value, "true") else if
                    (std.ascii.eqlIgnoreCase(key, "enabled")) enabled = !std.ascii.eqlIgnoreCase(value, "false");
            }
        }

        const entry_path = std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ directory, entry_file }) catch return;
        defer self.allocator.free(entry_path);
        std.Io.Dir.cwd().access(self.io, entry_path, .{}) catch return;

        const plugin_entry = PluginEntry{
            .id = self.allocator.dupe(u8, id) catch return,
            .name = self.allocator.dupe(u8, name) catch return,
            .version = self.allocator.dupe(u8, version) catch return,
            .description = self.allocator.dupe(u8, description) catch return,
            .author = self.allocator.dupe(u8, author) catch return,
            .license = self.allocator.dupe(u8, license) catch return,
            .directory = self.allocator.dupe(u8, directory) catch return,
            .entry_path = self.allocator.dupe(u8, entry_path) catch return,
            .manifest_path = self.allocator.dupe(u8, manifest_path) catch return,
            .language = self.allocator.dupe(u8, language) catch return,
            .lazy = lazy,
            .enabled = enabled,
        };
        self.entries.append(self.allocator, plugin_entry) catch {
            self.allocator.free(plugin_entry.id);
            self.allocator.free(plugin_entry.name);
            self.allocator.free(plugin_entry.version);
            self.allocator.free(plugin_entry.description);
            self.allocator.free(plugin_entry.author);
            self.allocator.free(plugin_entry.license);
            self.allocator.free(plugin_entry.directory);
            self.allocator.free(plugin_entry.entry_path);
            self.allocator.free(plugin_entry.manifest_path);
            self.allocator.free(plugin_entry.language);
            return;
        };
        const index = self.entries.items.len - 1;
        if (!enabled) return;
        if (lazy and language.len > 0) return;
        self.startEntry(index, doc, cursor);
    }

    fn startEntry(self: *Manager, index: usize, doc: *document_mod.Document, cursor: *usize) void {
        if (index >= self.entries.items.len) return;
        var entry = &self.entries.items[index];
        if (entry.runtime != null) return;
        const runtime = Runtime.start(
            self.allocator,
            self.io,
            self.host,
            entry.id,
            entry.directory,
            entry.entry_path,
            entry.language,
            entry.lazy,
            doc,
            cursor,
        ) catch |err| {
            if (entry.error_message) |old| self.allocator.free(old);
            const message = if (self.host.messages.items.len > 0)
                self.allocator.dupe(u8, self.host.messages.items[self.host.messages.items.len - 1].text) catch
                    self.allocator.dupe(u8, @errorName(err)) catch null
            else
                self.allocator.dupe(u8, @errorName(err)) catch null;
            entry.error_message = message;
            if (message) |text| {
                const formatted = std.fmt.allocPrint(self.allocator, "{s}: {s}", .{ entry.id, text }) catch
                    self.allocator.dupe(u8, text) catch null;
                if (formatted) |value| self.errors.append(self.allocator, value) catch self.allocator.free(value);
            }
            return;
        };
        entry.runtime = runtime;
        if (entry.error_message) |old| {
            self.allocator.free(old);
            entry.error_message = null;
        }
    }

    fn loadDeferred(self: *Manager, language: []const u8, doc: *document_mod.Document, cursor: *usize) void {
        for (self.entries.items, 0..) |entry, index| {
            if (!entry.lazy or entry.runtime != null or entry.language.len == 0) continue;
            if (!std.ascii.eqlIgnoreCase(entry.language, language)) continue;
            self.startEntry(index, doc, cursor);
        }
    }

    pub fn setPluginEnabled(
        self: *Manager,
        index: usize,
        enabled: bool,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !void {
        if (index >= self.entries.items.len) return error.UnknownPlugin;
        const entry = &self.entries.items[index];
        if (enabled) {
            entry.enabled = true;
            if (entry.runtime == null) self.startEntry(index, doc, cursor);
        } else {
            entry.enabled = false;
            if (entry.runtime) |runtime| {
                runtime.deinit();
                entry.runtime = null;
            }
        }
    }

    pub fn reloadPlugin(
        self: *Manager,
        index: usize,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !void {
        if (index >= self.entries.items.len) return error.UnknownPlugin;
        const entry = &self.entries.items[index];
        if (entry.runtime) |runtime| {
            runtime.deinit();
            entry.runtime = null;
        }
        if (entry.error_message) |message| {
            self.allocator.free(message);
            entry.error_message = null;
        }
        if (entry.enabled) self.startEntry(index, doc, cursor);
    }

    pub fn fireEvent(
        self: *Manager,
        event: []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
    ) void {
        if (std.ascii.eqlIgnoreCase(event, "buffer_opened")) {
            self.loadDeferred(self.host.language, doc, cursor);
        }
        for (self.entries.items) |entry| {
            if (entry.runtime) |runtime| {
                runtime.fireEvent(event, doc, cursor, self.host.language);
            }
        }
    }

    pub fn runDeferred(self: *Manager, doc: *document_mod.Document, cursor: *usize) bool {
        var ran = false;
        for (self.entries.items) |entry| {
            if (entry.runtime) |runtime| {
                if (runtime.runDeferred(doc, cursor)) ran = true;
            }
        }
        return ran;
    }

    pub fn executeCommand(
        self: *Manager,
        name: []const u8,
        args: []const []const u8,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !bool {
        for (self.entries.items) |entry| {
            const runtime = entry.runtime orelse continue;
            if (runtime.commandRef(name) == null) continue;
            return runtime.callCommand(name, args, doc, cursor);
        }
        return false;
    }

    pub fn hasCommand(self: Manager, name: []const u8) bool {
        for (self.entries.items) |entry| {
            const runtime = entry.runtime orelse continue;
            if (runtime.commandRef(name) != null) return true;
        }
        return false;
    }

    pub fn commandNamesFor(
        self: Manager,
        index: usize,
        allocator: std.mem.Allocator,
    ) ![]const []const u8 {
        if (index >= self.entries.items.len) return &.{};
        const runtime = self.entries.items[index].runtime orelse return &.{};
        var names: std.ArrayList([]const u8) = .empty;
        errdefer {
            for (names.items) |name| allocator.free(name);
            names.deinit(allocator);
        }
        var iterator = runtime.ctx.commands.valueIterator();
        while (iterator.next()) |command| {
            try names.append(allocator, try allocator.dupe(u8, command.name));
        }
        std.mem.sort([]const u8, names.items, {}, struct {
            fn lessThan(_: void, left: []const u8, right: []const u8) bool {
                return std.ascii.lessThanIgnoreCase(left, right);
            }
        }.lessThan);
        return names.toOwnedSlice(allocator);
    }

    pub fn commandNames(self: Manager, allocator: std.mem.Allocator) ![]const []const u8 {
        var names: std.ArrayList([]const u8) = .empty;
        errdefer {
            for (names.items) |name| allocator.free(name);
            names.deinit(allocator);
        }
        for (self.entries.items) |entry| {
            const runtime = entry.runtime orelse continue;
            var iterator = runtime.ctx.commands.valueIterator();
            while (iterator.next()) |command| {
                try names.append(allocator, try allocator.dupe(u8, command.name));
            }
        }
        std.mem.sort([]const u8, names.items, {}, struct {
            fn lessThan(_: void, left: []const u8, right: []const u8) bool {
                return std.ascii.lessThanIgnoreCase(left, right);
            }
        }.lessThan);
        return names.toOwnedSlice(allocator);
    }

    pub fn handlePanelKey(
        self: *Manager,
        panel_id: i64,
        key: []const u8,
        selected: usize,
        doc: *document_mod.Document,
        cursor: *usize,
    ) !?bool {
        for (self.entries.items) |entry| {
            const runtime = entry.runtime orelse continue;
            const handled = try runtime.handlePanelKey(panel_id, key, selected, doc, cursor);
            if (handled != null) return handled;
        }
        return null;
    }

    pub fn resolvePrompt(self: *Manager, id: i64, value: ?[]const u8) bool {
        for (self.entries.items) |entry| {
            const runtime = entry.runtime orelse continue;
            if (runtime.resolvePrompt(id, value)) return true;
        }
        return false;
    }

    pub fn panels(self: Manager) []Panel {
        return self.host.panels.items;
    }

    pub fn findPanel(self: *Manager, id: i64) ?*Panel {
        for (self.host.panels.items) |*panel| {
            if (panel.id == id) return panel;
        }
        return null;
    }

    pub fn closePanel(self: *Manager, id: i64) void {
        var index: usize = 0;
        while (index < self.host.panels.items.len) {
            if (self.host.panels.items[index].id == id) {
                var panel = self.host.panels.orderedRemove(index);
                panel.deinit(self.allocator);
                for (self.entries.items) |entry| {
                    const runtime = entry.runtime orelse continue;
                    var handler_index: usize = 0;
                    while (handler_index < runtime.ctx.panel_handlers.items.len) {
                        if (runtime.ctx.panel_handlers.items[handler_index].panel_id == id) {
                            const handler = runtime.ctx.panel_handlers.orderedRemove(handler_index);
                            c.luaL_unref(runtime.state, registry_index, handler.reference);
                            break;
                        }
                        handler_index += 1;
                    }
                }
                return;
            }
            index += 1;
        }
    }
};

// ---------------------------------------------------------------------------
// Lua helpers and API callbacks
// ---------------------------------------------------------------------------

fn luaAlloc(
    userdata: ?*anyopaque,
    pointer: ?*anyopaque,
    old_size: usize,
    new_size: usize,
) callconv(.c) ?*anyopaque {
    const context: *LuaAllocator = @ptrCast(@alignCast(userdata orelse return null));
    const allocator = context.allocator;
    const large = alloc_mod.HugeAllocator.large_threshold;

    if (new_size == 0) {
        if (pointer) |ptr| {
            if (old_size < large) {
                if (!context.freeSmall(ptr, old_size)) std.c.free(ptr);
            } else {
                const bytes: [*]u8 = @ptrCast(ptr);
                allocator.rawFree(bytes[0..old_size], .of(u8), @returnAddress());
            }
        }
        return null;
    }

    if (pointer == null) {
        if (new_size < large) {
            return context.allocSmall(new_size) orelse std.c.malloc(new_size);
        }
        const bytes = allocator.rawAlloc(new_size, .of(u8), @returnAddress()) orelse return null;
        return @ptrCast(bytes);
    }

    if (old_size < large and new_size < large) {
        if (LuaAllocator.classFor(old_size)) |old_class| {
            if (LuaAllocator.classFor(new_size) == old_class) return pointer;
        }
        const destination = context.allocSmall(new_size) orelse
            (std.c.malloc(new_size) orelse return null);
        const copy_len = @min(old_size, new_size);
        @memcpy(@as([*]u8, @ptrCast(destination))[0..copy_len], @as([*]const u8, @ptrCast(pointer.?))[0..copy_len]);
        if (!context.freeSmall(pointer, old_size)) std.c.free(pointer);
        return destination;
    }

    const old_bytes: [*]u8 = @ptrCast(pointer.?);
    const old = old_bytes[0..old_size];
    if (old_size >= large and new_size >= large) {
        if (allocator.rawRemap(old, .of(u8), new_size, @returnAddress())) |bytes| {
            return @ptrCast(bytes);
        }
    }

    const bytes: ?[*]u8 = if (new_size < large)
        (if (context.allocSmall(new_size)) |p|
            @as([*]u8, @ptrCast(p))
        else if (std.c.malloc(new_size)) |p|
            @as([*]u8, @ptrCast(p))
        else
            null)
    else
        allocator.rawAlloc(new_size, .of(u8), @returnAddress());
    const destination = bytes orelse return null;
    const copy_len = @min(old_size, new_size);
    @memcpy(destination[0..copy_len], old[0..copy_len]);
    if (old_size < large) {
        if (!context.freeSmall(pointer, old_size)) std.c.free(pointer);
    } else {
        allocator.rawFree(old, .of(u8), @returnAddress());
    }
    return destination;
}

fn openSafeLibs(state: *c.lua_State) void {
    const Lib = struct {
        name: [*:0]const u8,
        open: c.lua_CFunction,
    };
    const libs = [_]Lib{
        .{ .name = "base", .open = c.luaopen_base },
        .{ .name = "package", .open = c.luaopen_package },
        .{ .name = "coroutine", .open = c.luaopen_coroutine },
        .{ .name = "table", .open = c.luaopen_table },
        .{ .name = "io", .open = c.luaopen_io },
        .{ .name = "os", .open = c.luaopen_os },
        .{ .name = "string", .open = c.luaopen_string },
        .{ .name = "math", .open = c.luaopen_math },
        .{ .name = "utf8", .open = c.luaopen_utf8 },
    };
    for (libs) |lib| {
        c.luaL_requiref(state, lib.name, lib.open, 1);
        c.lua_settop(state, -2);
    }
}

const prelude =
    \\(function()
    \\  __hexal_wrap_methods = function(t)
    \\    if type(t) ~= "table" or rawget(t, "__hexal_wrapped") then return t end
    \\    rawset(t, "__hexal_wrapped", true)
    \\    local functions = {}
    \\    for k, v in pairs(t) do if type(v) == "function" then functions[k] = v end end
    \\    for k, f in pairs(functions) do
    \\      rawset(t, k, function(...)
    \\        local n = select("#", ...)
    \\        local args = table.pack(...)
    \\        local first = 1
    \\        if n > 0 and args[1] == t then first = 2 end
    \\        return f(table.unpack(args, first, n))
    \\      end)
    \\    end
    \\    return t
    \\  end
    \\end)()
;

fn contextFrom(state: *c.lua_State) *Context {
    const pointer = c.lua_touserdata(state, c.lua_upvalueindex(1)) orelse unreachable;
    return @ptrCast(@alignCast(pointer));
}

fn fail(state: *c.lua_State, message: [*:0]const u8) c_int {
    _ = c.lua_pushstring(state, message);
    return c.lua_error(state);
}

fn pushLString(state: *c.lua_State, bytes: []const u8) void {
    _ = c.lua_pushlstring(state, if (bytes.len == 0) "" else bytes.ptr, bytes.len);
}

fn argString(state: *c.lua_State, index: c_int) ?[]const u8 {
    var length: usize = 0;
    const pointer = c.lua_tolstring(state, index, &length);
    if (pointer == null) return null;
    return pointer[0..length];
}

fn argInteger(state: *c.lua_State, index: c_int) ?i64 {
    var is_number: c_int = 0;
    const value = c.lua_tointegerx(state, index, &is_number);
    if (is_number == 0) return null;
    return value;
}

fn optBool(state: *c.lua_State, index: c_int, fallback: bool) bool {
    if (c.lua_type(state, index) != c.LUA_TBOOLEAN) return fallback;
    return c.lua_toboolean(state, index) != 0;
}

fn fieldString(state: *c.lua_State, index: c_int, key: [*:0]const u8) ?[]const u8 {
    if (c.lua_getfield(state, index, key) != c.LUA_TSTRING) {
        c.lua_settop(state, -2);
        return null;
    }
    const value = argString(state, -1);
    c.lua_settop(state, -2);
    return value;
}

fn fieldInteger(state: *c.lua_State, index: c_int, key: [*:0]const u8) ?i64 {
    if (c.lua_getfield(state, index, key) != c.LUA_TNUMBER) {
        c.lua_settop(state, -2);
        return null;
    }
    const value = argInteger(state, -1);
    c.lua_settop(state, -2);
    return value;
}

fn fieldBool(state: *c.lua_State, index: c_int, key: [*:0]const u8, fallback: bool) bool {
    if (c.lua_getfield(state, index, key) != c.LUA_TBOOLEAN) {
        c.lua_settop(state, -2);
        return fallback;
    }
    const value = c.lua_toboolean(state, -1) != 0;
    c.lua_settop(state, -2);
    return value;
}

fn addFunction(state: *c.lua_State, ctx: *Context, name: [*:0]const u8, function: c.lua_CFunction) void {
    c.lua_pushlightuserdata(state, ctx);
    c.lua_pushcclosure(state, function, 1);
    c.lua_setfield(state, -2, name);
}

fn newModule(state: *c.lua_State, parent: c_int, name: [*:0]const u8) c_int {
    c.lua_newtable(state);
    const index = c.lua_gettop(state);
    c.lua_pushvalue(state, index);
    c.lua_setfield(state, parent, name);
    return index;
}

fn registerApi(runtime: *Runtime) !void {
    const state = runtime.state;
    const ctx = runtime.ctx;
    c.lua_newtable(state);
    const root = c.lua_gettop(state);
    c.lua_pushinteger(state, 1);
    c.lua_setfield(state, root, "api_version");

    addFunction(state, ctx, "line_count", apiLineCount);
    addFunction(state, ctx, "get_line", apiGetLine);
    addFunction(state, ctx, "set_line", apiSetLine);
    addFunction(state, ctx, "get_text", apiGetText);
    addFunction(state, ctx, "set_text", apiSetText);
    addFunction(state, ctx, "cursor", apiCursor);
    addFunction(state, ctx, "set_cursor", apiSetCursor);
    addFunction(state, ctx, "insert", apiInsert);
    addFunction(state, ctx, "message", apiMessage);
    addFunction(state, ctx, "register_command", apiRegisterCommand);
    _ = c.lua_getfield(state, root, "register_command");
    c.lua_setfield(state, root, "command");

    _ = newModule(state, root, "editor");
    addFunction(state, ctx, "get_current_buffer", apiGetCurrentBuffer);
    addFunction(state, ctx, "get_workspace_root", apiGetWorkspaceRoot);
    addFunction(state, ctx, "get_text", apiEditorGetText);
    addFunction(state, ctx, "get_line", apiEditorGetLine);
    addFunction(state, ctx, "get_cursor", apiEditorGetCursor);
    addFunction(state, ctx, "set_cursor", apiEditorSetCursor);
    addFunction(state, ctx, "get_selection", apiGetSelection);
    addFunction(state, ctx, "insert_text", apiInsertText);
    addFunction(state, ctx, "replace_range", apiReplaceRange);

    _ = newModule(state, root, "commands");
    addFunction(state, ctx, "register", apiRegisterCommand);
    addFunction(state, ctx, "unregister", apiUnregisterCommand);
    addFunction(state, ctx, "execute", apiExecuteCommand);

    _ = newModule(state, root, "keymap");
    addFunction(state, ctx, "map", apiKeymapMap);
    addFunction(state, ctx, "map_callback", apiKeymapMapCallback);
    addFunction(state, ctx, "unmap", apiKeymapUnmap);

    _ = newModule(state, root, "ui");
    addFunction(state, ctx, "message", apiMessage);
    addFunction(state, ctx, "warn", apiWarn);
    addFunction(state, ctx, "copy", apiCopy);
    addFunction(state, ctx, "terminal_size", apiTerminalSize);
    addFunction(state, ctx, "defer", apiDefer);
    addFunction(state, ctx, "prompt", apiPrompt);
    addFunction(state, ctx, "select", apiSelect);
    addFunction(state, ctx, "set_status", apiSetStatus);
    addFunction(state, ctx, "panel", apiPanel);

    _ = newModule(state, root, "events");
    addFunction(state, ctx, "on", apiEventsOn);
    addFunction(state, ctx, "off", apiEventsOff);

    _ = newModule(state, root, "exec");
    addFunction(state, ctx, "run", apiExecRun);

    _ = newModule(state, root, "config");
    addFunction(state, ctx, "get", apiConfigGet);
    addFunction(state, ctx, "set", apiConfigSet);
    addFunction(state, ctx, "save", apiConfigSave);
    addFunction(state, ctx, "get_all", apiConfigGetAll);

    _ = newModule(state, root, "fs");
    addFunction(state, ctx, "read_file", apiFsReadFile);
    addFunction(state, ctx, "write_file", apiFsWriteFile);
    addFunction(state, ctx, "exists", apiFsExists);
    addFunction(state, ctx, "stat", apiFsStat);
    addFunction(state, ctx, "dirname", apiFsDirname);
    addFunction(state, ctx, "basename", apiFsBasename);
    addFunction(state, ctx, "join", apiFsJoin);
    addFunction(state, ctx, "glob", apiFsGlob);
    addFunction(state, ctx, "workspace_files", apiFsWorkspaceFiles);
    addFunction(state, ctx, "workspace_files_async", apiFsWorkspaceFilesAsync);
    addFunction(state, ctx, "mkdir", apiFsMkdir);
    addFunction(state, ctx, "remove", apiFsRemove);
    addFunction(state, ctx, "rename", apiFsRename);
    addFunction(state, ctx, "realpath", apiFsRealpath);
    addFunction(state, ctx, "normalize", apiFsRealpath);

    _ = newModule(state, root, "async");
    addFunction(state, ctx, "spawn", apiAsyncSpawn);

    const lsp = newModule(state, root, "lsp");
    addFunction(state, ctx, "start", apiLspStart);
    addFunction(state, ctx, "stop", apiLspStop);
    addFunction(state, ctx, "request", apiLspRequest);
    addFunction(state, ctx, "notify", apiLspNotify);
    addFunction(state, ctx, "on_diagnostics", apiLspOnDiagnostics);
    addFunction(state, ctx, "go_to_definition", apiNoop);
    addFunction(state, ctx, "find_references", apiNoop);
    addFunction(state, ctx, "create_server", apiLspCreateServer);
    _ = newModule(state, lsp, "uri");
    addFunction(state, ctx, "from_path", apiLspUriFromPath);
    addFunction(state, ctx, "to_path", apiLspUriToPath);
    addFunction(state, ctx, "join", apiFsJoin);
    _ = newModule(state, lsp, "documents");
    addFunction(state, ctx, "open_documents", apiLspOpenDocuments);
    _ = newModule(state, lsp, "diagnostics");
    addFunction(state, ctx, "clear_all", apiNoop);
    _ = newModule(state, lsp, "workspace");
    addFunction(state, ctx, "root", apiGetWorkspaceRoot);

    _ = newModule(state, root, "log");
    addFunction(state, ctx, "trace", apiNoop);
    addFunction(state, ctx, "debug", apiNoop);
    addFunction(state, ctx, "info", apiNoop);
    addFunction(state, ctx, "warn", apiNoop);
    addFunction(state, ctx, "error", apiNoop);

    _ = newModule(state, root, "json");
    _ = newModule(state, root, "process");
    addFunction(state, ctx, "run", apiExecRun);
    _ = newModule(state, root, "index");
    _ = newModule(state, root, "debug");
    const testing = newModule(state, root, "testing");
    _ = newModule(state, testing, "lsp");

    // Plugin configuration is a plain Lua table owned by the plugin.
    c.lua_newtable(state);
    ctx.config_ref = c.luaL_ref(state, registry_index);

    c.lua_pushvalue(state, root);
    c.lua_setglobal(state, "hexal");
    c.lua_settop(state, 0);

    if (c.luaL_loadbufferx(state, prelude.ptr, prelude.len, "hexal-prelude", null) != c.LUA_OK) {
        return runtime.captureError();
    }
    if (c.lua_pcallk(state, 0, 0, 0, 0, null) != c.LUA_OK) {
        return runtime.captureError();
    }

    // Add the plugin directory to package.path.
    var path_buf: [4096]u8 = undefined;
    const path = std.fmt.bufPrint(
        &path_buf,
        "package.path = '{s}/?.lua;{s}/?/init.lua;' .. package.path",
        .{ ctx.plugin_dir, ctx.plugin_dir },
    ) catch "";
    if (path.len > 0) {
        if (c.luaL_loadbufferx(state, path.ptr, path.len, "hexal-package-path", null) == c.LUA_OK) {
            _ = c.lua_pcallk(state, 0, 0, 0, 0, null);
            c.lua_settop(state, 0);
        }
    }
}

// --- legacy buffer API -----------------------------------------------------

fn apiLineCount(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    c.lua_pushinteger(state, @intCast(ctx.lineCount()));
    return 1;
}

fn apiGetLine(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const line = argInteger(state, 1) orelse return fail(state, "get_line: expected a line number");
    if (line < 0 or @as(usize, @intCast(line)) >= ctx.lineCount()) {
        return fail(state, "get_line: line out of range");
    }
    const bytes = ctx.lineBytes(@intCast(line)) catch return fail(state, "get_line: line out of range");
    pushLString(state, bytes);
    return 1;
}

fn apiSetLine(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const line = argInteger(state, 1) orelse return fail(state, "set_line: expected a line number");
    const value = argString(state, 2) orelse return fail(state, "set_line: expected a string");
    if (line < 0 or @as(usize, @intCast(line)) >= ctx.lineCount()) {
        return fail(state, "set_line: line out of range");
    }
    ctx.setLine(@intCast(line), value) catch return fail(state, "set_line: out of memory");
    return 0;
}

fn apiGetText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    pushLString(state, ctx.bytes());
    return 1;
}

fn apiSetText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const value = argString(state, 1) orelse return fail(state, "set_text: expected a string");
    ctx.setText(value) catch return fail(state, "set_text: out of memory");
    return 0;
}

fn apiCursor(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    c.lua_pushinteger(state, @intCast(ctx.cursor_line));
    c.lua_pushinteger(state, @intCast(ctx.cursor_column));
    return 2;
}

fn apiSetCursor(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const line = argInteger(state, 1) orelse return fail(state, "set_cursor: expected a line");
    const column = argInteger(state, 2) orelse return fail(state, "set_cursor: expected a column");
    if (line < 0 or column < 0) return fail(state, "set_cursor: expected non-negative values");
    ctx.setCursor(@intCast(line), @intCast(column));
    return 0;
}

fn apiInsert(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const value = argString(state, 1) orelse return fail(state, "insert: expected a string");
    ctx.insert(value) catch return fail(state, "insert: out of memory");
    return 0;
}

fn apiMessage(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const value = argString(state, 1) orelse return 0;
    contextFrom(state).host.pushMessage(value, false);
    return 0;
}

fn apiWarn(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const value = argString(state, 1) orelse return 0;
    contextFrom(state).host.pushMessage(value, true);
    return 0;
}

fn apiRegisterCommand(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const name = argString(state, 1) orelse return fail(state, "register_command: expected a name");
    if (c.lua_type(state, 2) != c.LUA_TFUNCTION) {
        return fail(state, "register_command: expected a function");
    }
    const description = argString(state, 3) orelse "";
    c.lua_pushvalue(state, 2);
    const reference = c.luaL_ref(state, registry_index);

    var lower_buf: [256]u8 = undefined;
    if (name.len > lower_buf.len) {
        c.luaL_unref(state, registry_index, reference);
        return fail(state, "register_command: name is too long");
    }
    const lower = std.ascii.lowerString(lower_buf[0..name.len], name);
    const key = ctx.allocator.dupe(u8, lower) catch {
        c.luaL_unref(state, registry_index, reference);
        return fail(state, "register_command: out of memory");
    };
    const command = CommandEntry{
        .name = ctx.allocator.dupe(u8, name) catch {
            ctx.allocator.free(key);
            c.luaL_unref(state, registry_index, reference);
            return fail(state, "register_command: out of memory");
        },
        .description = ctx.allocator.dupe(u8, description) catch {
            ctx.allocator.free(key);
            c.luaL_unref(state, registry_index, reference);
            return fail(state, "register_command: out of memory");
        },
        .reference = reference,
    };
    const entry = ctx.commands.getOrPut(key) catch {
        ctx.allocator.free(key);
        c.luaL_unref(state, registry_index, reference);
        return fail(state, "register_command: out of memory");
    };
    if (entry.found_existing) {
        ctx.allocator.free(key);
        ctx.allocator.free(entry.value_ptr.name);
        ctx.allocator.free(entry.value_ptr.description);
        c.luaL_unref(state, registry_index, entry.value_ptr.reference);
    }
    entry.value_ptr.* = command;
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiUnregisterCommand(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const name = argString(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    var buffer: [256]u8 = undefined;
    if (name.len > buffer.len) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    const lower = std.ascii.lowerString(buffer[0..name.len], name);
    if (ctx.commands.fetchRemove(lower)) |removed| {
        ctx.allocator.free(removed.key);
        ctx.allocator.free(removed.value.name);
        ctx.allocator.free(removed.value.description);
        c.luaL_unref(state, registry_index, removed.value.reference);
        c.lua_pushboolean(state, 1);
    } else {
        c.lua_pushboolean(state, 0);
    }
    return 1;
}

fn apiExecuteCommand(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const name = argString(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    var args: std.ArrayList([]const u8) = .empty;
    defer args.deinit(ctx.allocator);
    const top = c.lua_gettop(state);
    var index: c_int = 2;
    while (index <= top) : (index += 1) {
        if (argString(state, index)) |value| args.append(ctx.allocator, value) catch {};
    }
    if (contextCommandRef(ctx, name)) |reference| {
        _ = c.lua_rawgeti(state, registry_index, reference);
        for (args.items) |arg| pushLString(state, arg);
        const status = c.lua_pcallk(state, @intCast(args.items.len), 0, 0, 0, null);
        if (status != c.LUA_OK) {
            _ = c.lua_pushboolean(state, 0);
            return 1;
        }
        c.lua_pushboolean(state, 1);
        return 1;
    }
    ctx.host.appendPendingCommand(name, args.items);
    c.lua_pushboolean(state, 1);
    return 1;
}

// --- editor API ------------------------------------------------------------

fn apiGetCurrentBuffer(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    pushLString(state, contextFrom(state).host.buffer_id);
    return 1;
}

fn apiGetWorkspaceRoot(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    if (contextFrom(state).host.workspace_root) |root| {
        pushLString(state, root);
    } else {
        c.lua_pushnil(state);
    }
    return 1;
}

fn apiEditorGetText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    pushLString(state, contextFrom(state).bytes());
    return 1;
}

fn apiEditorGetLine(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const line = argInteger(state, 1) orelse {
        c.lua_pushnil(state);
        return 1;
    };
    if (line < 0 or @as(usize, @intCast(line)) >= ctx.lineCount()) {
        c.lua_pushnil(state);
        return 1;
    }
    pushLString(state, ctx.lineBytes(@intCast(line)) catch "");
    return 1;
}

fn apiEditorGetCursor(state_opt: ?*c.lua_State) callconv(.c) c_int {
    return apiCursor(state_opt);
}

fn apiEditorSetCursor(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const line = argInteger(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const column = argInteger(state, 2) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    if (line < 0 or column < 0 or @as(usize, @intCast(line)) >= ctx.lineCount()) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    ctx.setCursor(@intCast(line), @intCast(column));
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiGetSelection(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const selection = ctx.host.selection orelse {
        c.lua_pushnil(state);
        return 1;
    };
    c.lua_newtable(state);
    c.lua_pushinteger(state, @intCast(selection.start_line));
    c.lua_setfield(state, -2, "start_line");
    c.lua_pushinteger(state, @intCast(selection.start_column));
    c.lua_setfield(state, -2, "start_column");
    c.lua_pushinteger(state, @intCast(selection.end_line));
    c.lua_setfield(state, -2, "end_line");
    c.lua_pushinteger(state, @intCast(selection.end_column));
    c.lua_setfield(state, -2, "end_column");
    pushLString(state, selection.text);
    c.lua_setfield(state, -2, "text");
    c.lua_pushboolean(state, @intFromBool(selection.linewise));
    c.lua_setfield(state, -2, "linewise");
    return 1;
}

fn apiInsertText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const value = argString(state, 1) orelse return 0;
    ctx.insert(value) catch return fail(state, "insert_text: out of memory");
    return 0;
}

fn apiReplaceRange(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const start = argInteger(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const finish = argInteger(state, 2) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const value = argString(state, 3) orelse "";
    if (start < 0 or finish < 0) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    ctx.replaceRange(@intCast(start), @intCast(finish), value) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

// --- keymap ----------------------------------------------------------------

fn apiKeymapMap(state_opt: ?*c.lua_State) callconv(.c) c_int {
    return keymapAdd(state_opt, false);
}

fn apiKeymapMapCallback(state_opt: ?*c.lua_State) callconv(.c) c_int {
    return keymapAdd(state_opt, true);
}

fn keymapAdd(state_opt: ?*c.lua_State, callback_mode: bool) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const keys = argString(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const mode = argString(state, 3) orelse "normal";
    const key_copy = ctx.allocator.dupe(u8, keys) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const mode_copy = ctx.allocator.dupe(u8, mode) catch {
        ctx.allocator.free(key_copy);
        c.lua_pushboolean(state, 0);
        return 1;
    };
    var binding = KeyBinding{ .keys = key_copy, .mode = mode_copy };
    if (callback_mode) {
        if (c.lua_type(state, 2) != c.LUA_TFUNCTION) {
            ctx.allocator.free(key_copy);
            ctx.allocator.free(mode_copy);
            c.lua_pushboolean(state, 0);
            return 1;
        }
        c.lua_pushvalue(state, 2);
        binding.reference = c.luaL_ref(state, registry_index);
    } else {
        const command = argString(state, 2) orelse "";
        binding.command = ctx.allocator.dupe(u8, command) catch {
            ctx.allocator.free(key_copy);
            ctx.allocator.free(mode_copy);
            c.lua_pushboolean(state, 0);
            return 1;
        };
    }
    ctx.keybindings.append(ctx.allocator, binding) catch {
        ctx.allocator.free(binding.keys);
        ctx.allocator.free(binding.mode);
        if (binding.command) |command| ctx.allocator.free(command);
        if (binding.reference != c.LUA_NOREF) c.luaL_unref(state, registry_index, binding.reference);
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiKeymapUnmap(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const keys = argString(state, 1) orelse "";
    const mode = argString(state, 2) orelse "normal";
    var index: usize = 0;
    var removed = false;
    while (index < ctx.keybindings.items.len) {
        const binding = ctx.keybindings.items[index];
        if (std.mem.eql(u8, binding.keys, keys) and std.ascii.eqlIgnoreCase(binding.mode, mode)) {
            const old = ctx.keybindings.orderedRemove(index);
            ctx.allocator.free(old.keys);
            ctx.allocator.free(old.mode);
            if (old.command) |command| ctx.allocator.free(command);
            if (old.reference != c.LUA_NOREF) c.luaL_unref(state, registry_index, old.reference);
            removed = true;
            continue;
        }
        index += 1;
    }
    c.lua_pushboolean(state, @intFromBool(removed));
    return 1;
}

// --- UI --------------------------------------------------------------------

fn apiCopy(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const value = argString(state, 1) orelse "";
    contextFrom(state).host.setClipboard(value);
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiTerminalSize(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const host = contextFrom(state).host;
    c.lua_newtable(state);
    c.lua_pushinteger(state, @intCast(host.columns));
    c.lua_setfield(state, -2, "columns");
    c.lua_pushinteger(state, @intCast(host.rows));
    c.lua_setfield(state, -2, "rows");
    return 1;
}

fn apiDefer(state_opt: ?*c.lua_State) callconv(.c) c_int {
    return deferCallback(state_opt, false);
}

fn apiAsyncSpawn(state_opt: ?*c.lua_State) callconv(.c) c_int {
    return deferCallback(state_opt, false);
}

fn deferCallback(state_opt: ?*c.lua_State, with_arg: bool) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    if (c.lua_type(state, 1) != c.LUA_TFUNCTION) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    const delay = if (with_arg) 0 else (argInteger(state, 2) orelse 0);
    c.lua_pushvalue(state, 1);
    const reference = c.luaL_ref(state, registry_index);
    const now = std.Io.Clock.Timestamp.now(ctx.io, .awake).raw.nanoseconds;
    ctx.deferred.append(ctx.allocator, .{
        .due_ns = now + @as(i96, @intCast(@max(delay, 0))) * std.time.ns_per_ms,
        .reference = reference,
    }) catch {
        c.luaL_unref(state, registry_index, reference);
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiPrompt(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const label = argString(state, 1) orelse "";
    if (c.lua_type(state, 2) != c.LUA_TFUNCTION) return 0;
    c.lua_pushvalue(state, 2);
    const reference = c.luaL_ref(state, registry_index);
    const id = ctx.host.next_request;
    ctx.host.next_request += 1;
    ctx.prompts.append(ctx.allocator, .{ .id = id, .reference = reference }) catch {
        c.luaL_unref(state, registry_index, reference);
        return 0;
    };
    const label_copy = ctx.allocator.dupe(u8, label) catch ctx.allocator.alloc(u8, 0) catch return 0;
    ctx.host.prompts.append(ctx.allocator, .{ .id = id, .label = label_copy }) catch {
        ctx.allocator.free(label_copy);
        return 0;
    };
    return 0;
}

fn apiSelect(state_opt: ?*c.lua_State) callconv(.c) c_int {
    // A selection menu is not part of the reduced plugin API. Call the
    // callback with nil so plugins may treat it as a cancelled selection.
    const state = state_opt orelse return 0;
    if (c.lua_type(state, 3) == c.LUA_TFUNCTION) {
        c.lua_pushvalue(state, 3);
        c.lua_pushnil(state);
        _ = c.lua_pcallk(state, 1, 0, 0, 0, null);
        c.lua_settop(state, 0);
    }
    return 0;
}

fn apiSetStatus(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushboolean(state, 1);
    return 1;
}

// --- events ----------------------------------------------------------------

fn apiEventsOn(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const event = argString(state, 1) orelse {
        c.lua_pushinteger(state, 0);
        return 1;
    };
    if (c.lua_type(state, 2) != c.LUA_TFUNCTION) {
        c.lua_pushinteger(state, 0);
        return 1;
    }
    c.lua_pushvalue(state, 2);
    const reference = c.luaL_ref(state, registry_index);
    const id = ctx.next_subscription;
    ctx.next_subscription += 1;
    ctx.subscriptions.append(ctx.allocator, .{
        .id = id,
        .event = ctx.allocator.dupe(u8, event) catch {
            c.luaL_unref(state, registry_index, reference);
            c.lua_pushinteger(state, 0);
            return 1;
        },
        .reference = reference,
    }) catch {
        c.luaL_unref(state, registry_index, reference);
        c.lua_pushinteger(state, 0);
        return 1;
    };
    c.lua_pushinteger(state, id);
    return 1;
}

fn apiEventsOff(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const id = argInteger(state, 1) orelse 0;
    var index: usize = 0;
    var removed = false;
    while (index < ctx.subscriptions.items.len) {
        if (ctx.subscriptions.items[index].id == id) {
            const subscription = ctx.subscriptions.orderedRemove(index);
            ctx.allocator.free(subscription.event);
            c.luaL_unref(state, registry_index, subscription.reference);
            removed = true;
            continue;
        }
        index += 1;
    }
    c.lua_pushboolean(state, @intFromBool(removed));
    return 1;
}

fn pushEventTable(state: *c.lua_State, ctx: *Context, event: []const u8, language: ?[]const u8) void {
    c.lua_newtable(state);
    pushLString(state, event);
    c.lua_setfield(state, -2, "event");
    pushLString(state, ctx.host.buffer_id);
    c.lua_setfield(state, -2, "buffer");
    pushLString(state, language orelse ctx.host.language);
    c.lua_setfield(state, -2, "language");
    if (ctx.host.path) |path| {
        pushLString(state, path);
        c.lua_setfield(state, -2, "path");
    }
    c.lua_pushinteger(state, @intCast(ctx.cursor_line));
    c.lua_setfield(state, -2, "line");
    c.lua_pushinteger(state, @intCast(ctx.cursor_column));
    c.lua_setfield(state, -2, "column");
}

// --- panels ----------------------------------------------------------------

fn parsePanelLines(ctx: *Context, state: *c.lua_State, index: c_int) !std.ArrayList(PanelLine) {
    var lines: std.ArrayList(PanelLine) = .empty;
    errdefer freePanelLines(ctx.allocator, &lines);
    if (c.lua_type(state, index) != c.LUA_TTABLE) return lines;
    const count: usize = @intCast(c.lua_rawlen(state, index));
    var i: usize = 1;
    while (i <= count) : (i += 1) {
        _ = c.lua_rawgeti(state, index, @intCast(i));
        const line = try parsePanelLine(ctx, state, c.lua_gettop(state));
        c.lua_settop(state, -2);
        try lines.append(ctx.allocator, line);
    }
    return lines;
}

fn parsePanelLine(ctx: *Context, state: *c.lua_State, index: c_int) !PanelLine {
    if (c.lua_type(state, index) == c.LUA_TSTRING) {
        const text = argString(state, index) orelse "";
        const segment = try ctx.allocator.alloc(PanelSegment, 1);
        segment[0] = .{
            .text = try ctx.allocator.dupe(u8, text),
            .style = try ctx.allocator.dupe(u8, "text"),
        };
        return .{ .segments = segment, .selectable = true };
    }
    if (c.lua_type(state, index) != c.LUA_TTABLE) {
        return .{ .segments = try ctx.allocator.alloc(PanelSegment, 0), .selectable = true };
    }
    const selectable = fieldBool(state, index, "selectable", true);
    if (c.lua_getfield(state, index, "segments") == c.LUA_TTABLE) {
        const segments_index = c.lua_gettop(state);
        const count: usize = @intCast(c.lua_rawlen(state, segments_index));
        var segments: std.ArrayList(PanelSegment) = .empty;
        errdefer {
            for (segments.items) |segment| {
                ctx.allocator.free(segment.text);
                ctx.allocator.free(segment.style);
            }
            segments.deinit(ctx.allocator);
        }
        var i: usize = 1;
        while (i <= count) : (i += 1) {
            _ = c.lua_rawgeti(state, segments_index, @intCast(i));
            if (try parseSegment(ctx, state, c.lua_gettop(state))) |segment| {
                try segments.append(ctx.allocator, segment);
            }
            c.lua_settop(state, -2);
        }
        c.lua_settop(state, -2);
        return .{ .segments = try segments.toOwnedSlice(ctx.allocator), .selectable = selectable };
    }
    c.lua_settop(state, -2);
    const text = fieldString(state, index, "text");
    if (text) |value| {
        const segment = try ctx.allocator.alloc(PanelSegment, 1);
        segment[0] = .{
            .text = try ctx.allocator.dupe(u8, value),
            .style = try ctx.allocator.dupe(u8, "text"),
        };
        return .{ .segments = segment, .selectable = selectable };
    }
    return .{ .segments = try ctx.allocator.alloc(PanelSegment, 0), .selectable = selectable };
}

fn parseSegment(ctx: *Context, state: *c.lua_State, index: c_int) !?PanelSegment {
    if (c.lua_type(state, index) == c.LUA_TSTRING) {
        const text = argString(state, index) orelse "";
        return .{
            .text = try ctx.allocator.dupe(u8, text),
            .style = try ctx.allocator.dupe(u8, "text"),
        };
    }
    if (c.lua_type(state, index) != c.LUA_TTABLE) return null;
    const text = fieldString(state, index, "text") orelse return null;
    if (text.len == 0) return null;
    const style = fieldString(state, index, "style") orelse "text";
    return .{
        .text = try ctx.allocator.dupe(u8, text),
        .style = try ctx.allocator.dupe(u8, style),
    };
}

fn apiPanel(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const options = if (c.lua_type(state, 1) == c.LUA_TTABLE) c.lua_absindex(state, 1) else 0;
    const title = if (options != 0) fieldString(state, options, "title") orelse "Plugin" else "Plugin";
    const footer = if (options != 0) fieldString(state, options, "footer") orelse "" else "";
    const selectable = if (options != 0) fieldBool(state, options, "selectable", true) else true;
    const selected = if (options != 0) fieldInteger(state, options, "selected") orelse 1 else 1;
    var lines: std.ArrayList(PanelLine) = .empty;
    if (options != 0) {
        if (c.lua_getfield(state, options, "lines") == c.LUA_TTABLE) {
            lines = parsePanelLines(ctx, state, c.lua_gettop(state)) catch .empty;
            c.lua_settop(state, -2);
        } else {
            c.lua_settop(state, -2);
        }
    }

    const id = ctx.host.next_panel_id;
    ctx.host.next_panel_id += 1;
    const panel = Panel{
        .id = id,
        .plugin_id = ctx.allocator.dupe(u8, ctx.plugin_id) catch &.{},
        .title = ctx.allocator.dupe(u8, title) catch &.{},
        .footer = ctx.allocator.dupe(u8, footer) catch &.{},
        .selectable = selectable,
        .selected = @intCast(@max(selected, 1) - 1),
        .lines = lines,
    };
    ctx.host.panels.append(ctx.allocator, panel) catch {
        var discard = panel;
        discard.deinit(ctx.allocator);
        c.lua_pushnil(state);
        return 1;
    };
    if (c.lua_type(state, 2) == c.LUA_TFUNCTION) {
        c.lua_pushvalue(state, 2);
        const reference = c.luaL_ref(state, registry_index);
        ctx.panel_handlers.append(ctx.allocator, .{ .panel_id = id, .reference = reference }) catch {
            c.luaL_unref(state, registry_index, reference);
        };
    }
    return pushPanelTable(state, ctx, id);
}

fn addPanelFunction(state: *c.lua_State, ctx: *Context, table: c_int, name: [*:0]const u8, function: c.lua_CFunction, id: i64) void {
    c.lua_pushlightuserdata(state, ctx);
    c.lua_pushinteger(state, @intCast(id));
    c.lua_pushcclosure(state, function, 2);
    c.lua_setfield(state, table, name);
}

fn panelIdFrom(state: *c.lua_State) i64 {
    var is_number: c_int = 0;
    return c.lua_tointegerx(state, c.lua_upvalueindex(2), &is_number);
}

fn pushPanelTable(state: *c.lua_State, ctx: *Context, id: i64) c_int {
    c.lua_newtable(state);
    const table = c.lua_gettop(state);
    c.lua_pushinteger(state, @intCast(id));
    c.lua_setfield(state, table, "id");
    addPanelFunction(state, ctx, table, "set_lines", apiPanelSetLines, id);
    addPanelFunction(state, ctx, table, "splice_lines", apiPanelSpliceLines, id);
    addPanelFunction(state, ctx, table, "set_title", apiPanelSetTitle, id);
    addPanelFunction(state, ctx, table, "set_footer", apiPanelSetFooter, id);
    addPanelFunction(state, ctx, table, "set_selected", apiPanelSetSelected, id);
    addPanelFunction(state, ctx, table, "set_selectable", apiPanelSetSelectable, id);
    addPanelFunction(state, ctx, table, "close", apiPanelClose, id);
    wrapMethods(state, table);
    return 1;
}

fn wrapMethods(state: *c.lua_State, table: c_int) void {
    _ = c.lua_getglobal(state, "__hexal_wrap_methods");
    if (c.lua_type(state, -1) != c.LUA_TFUNCTION) {
        c.lua_settop(state, table);
        return;
    }
    c.lua_pushvalue(state, table);
    _ = c.lua_pcallk(state, 1, 1, 0, 0, null);
    c.lua_settop(state, table);
}

fn apiPanelSetLines(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const id = panelIdFrom(state);
    const panel = ctx.findPanel(id) orelse return 0;
    const lines = parsePanelLines(ctx, state, 1) catch return 0;
    freePanelLines(ctx.allocator, &panel.lines);
    panel.lines.deinit(ctx.allocator);
    panel.lines = lines;
    return 0;
}

fn apiPanelSpliceLines(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const id = panelIdFrom(state);
    const panel = ctx.findPanel(id) orelse return 0;
    const start_value = argInteger(state, 1) orelse 1;
    const delete_value = argInteger(state, 2) orelse 0;
    var replacement = parsePanelLines(ctx, state, 3) catch std.ArrayList(PanelLine).empty;
    const start: usize = @intCast(@max(start_value, 1) - 1);
    const delete_count: usize = @intCast(@max(delete_value, 0));
    const actual_start = @min(start, panel.lines.items.len);
    const actual_end = @min(actual_start + delete_count, panel.lines.items.len);
    for (panel.lines.items[actual_start..actual_end]) |line| {
        for (line.segments) |segment| {
            ctx.allocator.free(segment.text);
            ctx.allocator.free(segment.style);
        }
        ctx.allocator.free(line.segments);
    }
    if (delete_count == 0) {
        var i: usize = 0;
        while (i < replacement.items.len) : (i += 1) {
            panel.lines.insert(ctx.allocator, actual_start + i, replacement.items[i]) catch {};
        }
        replacement.deinit(ctx.allocator);
    } else {
        panel.lines.replaceRange(ctx.allocator, actual_start, actual_end - actual_start, replacement.items) catch {};
        replacement.deinit(ctx.allocator);
    }
    return 0;
}

fn apiPanelSetTitle(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const panel = ctx.findPanel(panelIdFrom(state)) orelse return 0;
    const title = argString(state, 1) orelse "";
    ctx.allocator.free(panel.title);
    panel.title = ctx.allocator.dupe(u8, title) catch &.{};
    return 0;
}

fn apiPanelSetFooter(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const panel = ctx.findPanel(panelIdFrom(state)) orelse return 0;
    const footer = argString(state, 1) orelse "";
    ctx.allocator.free(panel.footer);
    panel.footer = ctx.allocator.dupe(u8, footer) catch &.{};
    return 0;
}

fn apiPanelSetSelected(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const panel = ctx.findPanel(panelIdFrom(state)) orelse return 0;
    const selected = argInteger(state, 1) orelse 1;
    panel.selected = @intCast(@max(selected, 1) - 1);
    return 0;
}

fn apiPanelSetSelectable(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const panel = ctx.findPanel(panelIdFrom(state)) orelse return 0;
    panel.selectable = optBool(state, 1, true);
    return 0;
}

fn apiPanelClose(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const id = panelIdFrom(state);
    var index: usize = 0;
    while (index < ctx.host.panels.items.len) {
        if (ctx.host.panels.items[index].id == id) {
            var panel = ctx.host.panels.orderedRemove(index);
            panel.deinit(ctx.allocator);
            break;
        }
        index += 1;
    }
    return 0;
}

// --- process / filesystem --------------------------------------------------

fn apiExecRun(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const command = argString(state, 1) orelse "";
    var argv: std.ArrayList([]const u8) = .empty;
    defer argv.deinit(ctx.allocator);
    argv.append(ctx.allocator, command) catch {};
    var cwd: ?[]const u8 = null;
    var timeout_ms: i64 = 15000;
    if (c.lua_type(state, 2) == c.LUA_TTABLE) {
        const args_index = c.lua_absindex(state, 2);
        const count: usize = @intCast(c.lua_rawlen(state, args_index));
        var i: usize = 1;
        while (i <= count) : (i += 1) {
            _ = c.lua_rawgeti(state, args_index, @intCast(i));
            if (argString(state, -1)) |value| argv.append(ctx.allocator, value) catch {};
            c.lua_settop(state, -2);
        }
    }
    if (c.lua_type(state, 3) == c.LUA_TTABLE) {
        const options = c.lua_absindex(state, 3);
        cwd = fieldString(state, options, "cwd");
        timeout_ms = fieldInteger(state, options, "timeout") orelse timeout_ms;
    }
    if (cwd == null) {
        if (ctx.host.workspace_root) |root| cwd = root;
    }

    c.lua_newtable(state);
    const result = c.lua_gettop(state);
    c.lua_pushboolean(state, 0);
    c.lua_setfield(state, result, "ok");
    c.lua_pushinteger(state, 0);
    c.lua_setfield(state, result, "code");
    pushLString(state, "");
    c.lua_setfield(state, result, "stdout");
    pushLString(state, "");
    c.lua_setfield(state, result, "stderr");
    pushLString(state, "");
    c.lua_setfield(state, result, "error");

    if (command.len == 0) {
        pushLString(state, "empty command");
        c.lua_setfield(state, result, "error");
        return 1;
    }

    const run_result = std.process.run(ctx.allocator, ctx.io, .{
        .argv = argv.items,
        .cwd = if (cwd) |path| .{ .path = path } else .inherit,
        .timeout = .{ .duration = .{ .raw = std.Io.Duration.fromMilliseconds(@max(timeout_ms, 100)), .clock = .awake } },
    });
    if (run_result) |run| {
        defer ctx.allocator.free(run.stdout);
        defer ctx.allocator.free(run.stderr);
        const success = run.term.success();
        c.lua_pushboolean(state, @intFromBool(success));
        c.lua_setfield(state, result, "ok");
        const code: i64 = switch (run.term) {
            .exited => |value| value,
            else => -1,
        };
        c.lua_pushinteger(state, @intCast(code));
        c.lua_setfield(state, result, "code");
        pushLString(state, run.stdout);
        c.lua_setfield(state, result, "stdout");
        pushLString(state, run.stderr);
        c.lua_setfield(state, result, "stderr");
        if (!success) {
            const message = std.mem.trim(u8, run.stderr, " \t\r\n");
            if (message.len > 0) {
                pushLString(state, message);
            } else {
                var buffer: [96]u8 = undefined;
                const text = std.fmt.bufPrint(&buffer, "process exited with code {d}", .{code}) catch "process failed";
                pushLString(state, text);
            }
            c.lua_setfield(state, result, "error");
        }
    } else |err| {
        var buffer: [256]u8 = undefined;
        const text = switch (err) {
            error.Timeout => std.fmt.bufPrint(
                &buffer,
                "process timed out after {d} ms",
                .{timeout_ms},
            ) catch "process timed out",
            else => std.fmt.bufPrint(&buffer, "cannot start '{s}': {s}", .{ command, @errorName(err) }) catch "cannot start process",
        };
        pushLString(state, text);
        c.lua_setfield(state, result, "error");
    }
    return 1;
}

fn apiFsReadFile(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse {
        c.lua_pushnil(state);
        return 1;
    };
    const text = std.Io.Dir.cwd().readFileAlloc(ctx.io, path, ctx.allocator, .limited(64 * 1024 * 1024)) catch {
        c.lua_pushnil(state);
        return 1;
    };
    defer ctx.allocator.free(text);
    pushLString(state, text);
    return 1;
}

fn apiFsWriteFile(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    const contents = argString(state, 2) orelse "";
    if (std.fs.path.dirname(path)) |parent| {
        if (parent.len > 0) std.Io.Dir.cwd().createDirPath(ctx.io, parent) catch {};
    }
    std.Io.Dir.cwd().writeFile(ctx.io, .{ .sub_path = path, .data = contents }) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiFsExists(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    std.Io.Dir.cwd().access(ctx.io, path, .{}) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiFsStat(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse "";
    c.lua_newtable(state);
    const table = c.lua_gettop(state);
    pushLString(state, path);
    c.lua_setfield(state, table, "path");
    c.lua_pushboolean(state, 0);
    c.lua_setfield(state, table, "exists");
    c.lua_pushboolean(state, 0);
    c.lua_setfield(state, table, "directory");
    c.lua_pushboolean(state, 0);
    c.lua_setfield(state, table, "file");
    c.lua_pushinteger(state, 0);
    c.lua_setfield(state, table, "size");
    const stat = std.Io.Dir.cwd().statFile(ctx.io, path, .{}) catch return 1;
    c.lua_pushboolean(state, 1);
    c.lua_setfield(state, table, "exists");
    c.lua_pushboolean(state, @intFromBool(stat.kind == .directory));
    c.lua_setfield(state, table, "directory");
    c.lua_pushboolean(state, @intFromBool(stat.kind == .file));
    c.lua_setfield(state, table, "file");
    c.lua_pushinteger(state, @intCast(stat.size));
    c.lua_setfield(state, table, "size");
    return 1;
}

fn apiFsDirname(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const path = argString(state, 1) orelse "";
    pushLString(state, std.fs.path.dirname(path) orelse "");
    return 1;
}

fn apiFsBasename(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const path = argString(state, 1) orelse "";
    pushLString(state, std.fs.path.basename(path));
    return 1;
}

fn apiFsJoin(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var parts: std.ArrayList([]const u8) = .empty;
    defer parts.deinit(ctx.allocator);
    const top = c.lua_gettop(state);
    var i: c_int = 1;
    while (i <= top) : (i += 1) {
        if (argString(state, i)) |part| parts.append(ctx.allocator, part) catch {};
    }
    const joined = std.fs.path.join(ctx.allocator, parts.items) catch {
        c.lua_pushnil(state);
        return 1;
    };
    defer ctx.allocator.free(joined);
    pushLString(state, joined);
    return 1;
}

fn apiFsRealpath(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse "";
    const real = std.Io.Dir.cwd().realPathFileAlloc(ctx.io, path, ctx.allocator) catch {
        pushLString(state, path);
        return 1;
    };
    defer ctx.allocator.free(real);
    pushLString(state, real);
    return 1;
}

fn pushFilesTable(state: *c.lua_State, ctx: *Context, max_results: usize) void {
    var files: std.ArrayList([]u8) = .empty;
    defer {
        files_mod.clearFiles(ctx.allocator, &files);
        files.deinit(ctx.allocator);
    }
    var cancel: std.atomic.Value(bool) = .init(false);
    files_mod.scan(ctx.allocator, ctx.io, &files, max_results, &cancel) catch {};
    c.lua_newtable(state);
    const table = c.lua_gettop(state);
    for (files.items, 0..) |file, index| {
        pushLString(state, file);
        c.lua_seti(state, table, @intCast(index + 1));
    }
}

fn apiFsWorkspaceFiles(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var max_results: usize = 20000;
    if (c.lua_type(state, 1) == c.LUA_TTABLE) {
        const options = c.lua_absindex(state, 1);
        if (fieldInteger(state, options, "max_results") orelse fieldInteger(state, options, "maxResults")) |value| {
            max_results = @intCast(@max(value, 1));
        }
    }
    pushFilesTable(state, ctx, max_results);
    return 1;
}

fn apiFsWorkspaceFilesAsync(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    if (c.lua_type(state, 2) != c.LUA_TFUNCTION) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    var max_results: usize = 20000;
    if (c.lua_type(state, 1) == c.LUA_TTABLE) {
        const options = c.lua_absindex(state, 1);
        if (fieldInteger(state, options, "max_results") orelse fieldInteger(state, options, "maxResults")) |value| {
            max_results = @intCast(@max(value, 1));
        }
    }
    pushFilesTable(state, ctx, max_results);
    const files_reference = c.luaL_ref(state, registry_index);
    c.lua_pushvalue(state, 2);
    const callback_reference = c.luaL_ref(state, registry_index);
    const now = std.Io.Clock.Timestamp.now(ctx.io, .awake).raw.nanoseconds;
    ctx.deferred.append(ctx.allocator, .{
        .due_ns = now,
        .reference = callback_reference,
        .arg_reference = files_reference,
    }) catch {
        c.luaL_unref(state, registry_index, callback_reference);
        c.luaL_unref(state, registry_index, files_reference);
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiFsGlob(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const pattern = argString(state, 1) orelse "*";
    var files: std.ArrayList([]u8) = .empty;
    defer {
        files_mod.clearFiles(ctx.allocator, &files);
        files.deinit(ctx.allocator);
    }
    var cancel: std.atomic.Value(bool) = .init(false);
    files_mod.scan(ctx.allocator, ctx.io, &files, 20000, &cancel) catch {};
    c.lua_newtable(state);
    const table = c.lua_gettop(state);
    var count: usize = 0;
    for (files.items) |file| {
        if (!globMatch(pattern, file)) continue;
        count += 1;
        pushLString(state, file);
        c.lua_seti(state, table, @intCast(count));
    }
    return 1;
}

fn globMatch(pattern: []const u8, text: []const u8) bool {
    if (pattern.len == 0) return true;
    if (std.mem.eql(u8, pattern, "*") or std.mem.eql(u8, pattern, "**/*")) return true;
    return std.mem.indexOf(u8, text, std.mem.trim(u8, pattern, "*")) != null or
        std.mem.endsWith(u8, text, pattern);
}

fn apiFsMkdir(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse "";
    std.Io.Dir.cwd().createDirPath(ctx.io, path) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiFsRemove(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse "";
    const stat = std.Io.Dir.cwd().statFile(ctx.io, path, .{}) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    if (stat.kind == .directory) {
        std.Io.Dir.cwd().deleteDir(ctx.io, path) catch {
            c.lua_pushboolean(state, 0);
            return 1;
        };
    } else {
        std.Io.Dir.cwd().deleteFile(ctx.io, path) catch {
            c.lua_pushboolean(state, 0);
            return 1;
        };
    }
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiFsRename(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const old = argString(state, 1) orelse "";
    const new = argString(state, 2) orelse "";
    const cwd = std.Io.Dir.cwd();
    cwd.rename(old, cwd, new, ctx.io) catch {
        c.lua_pushboolean(state, 0);
        return 1;
    };
    c.lua_pushboolean(state, 1);
    return 1;
}

// --- config ----------------------------------------------------------------

fn pushFieldSlice(state: *c.lua_State, index: c_int, key: []const u8) c_int {
    pushLString(state, key);
    return c.lua_gettable(state, index);
}

fn apiConfigGet(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const key = argString(state, 1) orelse "";
    _ = c.lua_rawgeti(state, registry_index, ctx.config_ref);
    const table = c.lua_gettop(state);
    var current = table;
    var parts = std.mem.splitScalar(u8, key, '.');
    var found = true;
    var depth: usize = 0;
    while (parts.next()) |part| {
        if (part.len == 0) continue;
        if (c.lua_type(state, current) != c.LUA_TTABLE) {
            found = false;
            break;
        }
        const value_type = pushFieldSlice(state, current, part);
        if (value_type == c.LUA_TNIL) {
            found = false;
            break;
        }
        current = c.lua_gettop(state);
        depth += 1;
    }
    if (!found) {
        c.lua_settop(state, table - 1);
        if (c.lua_type(state, 2) == c.LUA_TNIL) c.lua_pushnil(state) else c.lua_pushvalue(state, 2);
        return 1;
    }
    if (depth == 0) c.lua_pushvalue(state, table);
    c.lua_replace(state, 1);
    c.lua_settop(state, 1);
    return 1;
}

fn apiConfigSet(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const key = argString(state, 1) orelse "";
    if (key.len == 0) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    var parts: std.ArrayList([:0]u8) = .empty;
    defer {
        for (parts.items) |part| ctx.allocator.free(part);
        parts.deinit(ctx.allocator);
    }
    var iterator = std.mem.splitScalar(u8, key, '.');
    while (iterator.next()) |part| {
        if (part.len > 0) parts.append(ctx.allocator, ctx.allocator.dupeSentinel(u8, part, 0) catch continue) catch {};
    }
    if (parts.items.len == 0) {
        c.lua_pushboolean(state, 0);
        return 1;
    }
    _ = c.lua_rawgeti(state, registry_index, ctx.config_ref);
    var current = c.lua_gettop(state);
    for (parts.items[0 .. parts.items.len - 1]) |part| {
        const value_type = pushFieldSlice(state, current, part);
        if (value_type != c.LUA_TTABLE) {
            c.lua_settop(state, -2);
            c.lua_newtable(state);
            c.lua_pushvalue(state, -1);
            c.lua_setfield(state, current, part.ptr);
        }
        current = c.lua_gettop(state);
    }
    if (c.lua_type(state, 2) == c.LUA_TNIL) c.lua_pushnil(state) else c.lua_pushvalue(state, 2);
    c.lua_setfield(state, current, parts.items[parts.items.len - 1].ptr);
    c.lua_settop(state, 0);
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiConfigSave(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiConfigGetAll(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    _ = c.lua_rawgeti(state, registry_index, ctx.config_ref);
    return 1;
}

// --- LSP compatibility stubs ----------------------------------------------

fn apiLspStart(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    _ = ctx;
    c.lua_pushboolean(state, 0);
    return 1;
}

fn apiLspStop(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushboolean(state, 1);
    return 1;
}

fn apiLspRequest(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    if (c.lua_type(state, 3) == c.LUA_TFUNCTION) {
        c.lua_pushvalue(state, 3);
        c.lua_pushnil(state);
        pushLString(state, "custom LSP requests are not supported by this build");
        _ = c.lua_pcallk(state, 2, 0, 0, 0, null);
        c.lua_settop(state, 0);
    }
    c.lua_pushboolean(state, 0);
    return 1;
}

fn apiLspNotify(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushboolean(state, 0);
    return 1;
}

fn apiLspOnDiagnostics(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushinteger(state, 0);
    return 1;
}

fn apiLspCreateServer(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_newtable(state);
    addFunction(state, contextFrom(state), "start", apiLspStart);
    addFunction(state, contextFrom(state), "stop", apiLspStop);
    return 1;
}

fn apiLspOpenDocuments(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    c.lua_pushnil(state);
    return 1;
}

fn apiLspUriFromPath(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const path = argString(state, 1) orelse "";
    var buffer: [4096]u8 = undefined;
    const uri = std.fmt.bufPrint(&buffer, "file://{s}", .{path}) catch path;
    pushLString(state, uri);
    _ = ctx;
    return 1;
}

fn apiLspUriToPath(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const uri = argString(state, 1) orelse "";
    if (std.mem.startsWith(u8, uri, "file://")) pushLString(state, uri[7..]) else pushLString(state, uri);
    return 1;
}

fn apiNoop(state_opt: ?*c.lua_State) callconv(.c) c_int {
    _ = state_opt;
    return 0;
}

test "loads bundled git and project tree plugins" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var document = document_mod.Document.init(allocator, threaded.io());
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, threaded.io());
    defer manager.deinit();
    manager.setContext("sample.zig", null, "zig", ".", 80, 24, null);
    try manager.loadAll(&document, &cursor);
    try std.testing.expect(manager.hasCommand("gl"));
    try std.testing.expect(manager.hasCommand("pt"));
}

test "git log plugin produces a panel" {
    var huge: alloc_mod.HugeAllocator = .{};
    defer huge.deinit();
    const allocator = huge.allocator();
    var threaded: std.Io.Threaded = .init_single_threaded;
    var document = document_mod.Document.init(allocator, threaded.io());
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, threaded.io());
    defer manager.deinit();
    const root = try std.Io.Dir.cwd().realPathFileAlloc(threaded.io(), ".", allocator);
    defer allocator.free(root);
    manager.setContext("sample.zig", null, "zig", root, 120, 40, null);
    try manager.loadAll(&document, &cursor);
    const handled = try manager.executeCommand("gl", &.{}, &document, &cursor);
    try std.testing.expect(handled);
}

test "project tree plugin loads rows through deferred callbacks" {
    var huge: alloc_mod.HugeAllocator = .{};
    defer huge.deinit();
    const allocator = huge.allocator();
    var threaded: std.Io.Threaded = .init_single_threaded;
    var document = document_mod.Document.init(allocator, threaded.io());
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, threaded.io());
    defer manager.deinit();
    const root = try std.Io.Dir.cwd().realPathFileAlloc(threaded.io(), ".", allocator);
    defer allocator.free(root);
    manager.setContext("sample.zig", null, "zig", root, 120, 40, null);
    try manager.loadAll(&document, &cursor);
    const handled = try manager.executeCommand("pt", &.{}, &document, &cursor);
    try std.testing.expect(handled);
    var i: usize = 0;
    while (i < 200) : (i += 1) {
        _ = manager.runDeferred(&document, &cursor);
    }
    try std.testing.expect(manager.panels().len > 0);
}

test "plugin API exposes every module" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    const io = threaded.io();
    const path = "/tmp/hexalz-api-probe.lua";
    try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = path, .data =
        \\assert(hexal.editor, "editor")
        \\assert(hexal.commands, "commands")
        \\assert(hexal.keymap, "keymap")
        \\assert(hexal.ui, "ui")
        \\assert(hexal.events, "events")
        \\assert(hexal.exec, "exec")
        \\assert(hexal.config, "config")
        \\assert(hexal.fs, "fs")
        \\assert(hexal.async, "async")
        \\assert(hexal.lsp, "lsp")
        \\assert(hexal.log, "log")
        \\hexal.config.set("answer", 42)
        \\assert(hexal.config.get("answer") == 42, "config get")
        \\hexal.config.set("nested.name", "hexal")
        \\assert(hexal.config.get("nested.name") == "hexal", "nested config")
        \\local panel = hexal.ui.panel({ title = "Probe", lines = { "one" } }, function() return false end)
        \\panel:set_title("Changed")
        \\panel:set_lines({ { segments = { { text = "two", style = "accent" } } } })
        \\hexal.commands.register("probe", function() return 1 end)
    });
    defer std.Io.Dir.cwd().deleteFile(io, path) catch {};
    var document = document_mod.Document.init(allocator, io);
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, io);
    defer manager.deinit();
    manager.setContext("probe", null, "text", ".", 80, 24, null);
    try manager.loadFilePlugin(path, &document, &cursor);
    try std.testing.expect(manager.hasCommand("probe"));
}

test "lazy json plugin loads on buffer_opened" {
    const allocator = std.testing.allocator;
    var threaded: std.Io.Threaded = .init_single_threaded;
    var document = document_mod.Document.init(allocator, threaded.io());
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, threaded.io());
    defer manager.deinit();
    manager.setContext("sample.json", null, "json", ".", 80, 24, null);
    try manager.loadAll(&document, &cursor);
    manager.fireEvent("buffer_opened", &document, &cursor);
    try std.testing.expect(manager.errors.items.len == 0);
}

test "lazy json plugin loads with the huge allocator" {
    var huge: alloc_mod.HugeAllocator = .{};
    defer huge.deinit();
    const allocator = huge.allocator();
    var threaded: std.Io.Threaded = .init_single_threaded;
    var document = document_mod.Document.init(allocator, threaded.io());
    defer document.deinit();
    var cursor: usize = 0;
    var manager = try Manager.init(allocator, threaded.io());
    defer manager.deinit();
    manager.setContext("sample.json", null, "json", ".", 80, 24, null);
    try manager.loadAll(&document, &cursor);
    manager.fireEvent("buffer_opened", &document, &cursor);
    try std.testing.expect(manager.errors.items.len == 0);
}
