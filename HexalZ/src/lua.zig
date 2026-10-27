//! Embedded Lua 5.4.9 runtime and the small `hexal.*` plugin API.
//!
//! A command invocation snapshots the current document into a line-indexed
//! buffer. Plugin edits are written back as one document replacement, which
//! keeps plugin commands atomic and makes the benchmark repeatable.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const alloc_mod = @import("alloc.zig");
const document_mod = @import("document.zig");

const c = @import("lua_c.zig");

const registry_index: c_int = c.LUA_REGISTRYINDEX;

const Context = struct {
    allocator: std.mem.Allocator,
    text: std.ArrayList(u8) = .empty,
    /// Non-owning view of the current document text. Bulk write-back hands the
    /// host buffer to the document; the host keeps reading it from here until
    /// a mutating API call materializes a private copy.
    text_view: ?[]const u8 = null,
    line_starts: std.ArrayList(usize) = .empty,
    crlf: bool = false,
    cursor_line: usize = 0,
    cursor_column: usize = 0,
    dirty: bool = false,
    message: ?[]u8 = null,
    commands: std.StringHashMap(c_int),
    loaded_revision: ?u64 = null,

    fn init(allocator: std.mem.Allocator) !Context {
        var ctx = Context{
            .allocator = allocator,
            .commands = std.StringHashMap(c_int).init(allocator),
        };
        try ctx.line_starts.append(allocator, 0);
        return ctx;
    }

    fn deinit(self: *Context) void {
        if (self.message) |message| self.allocator.free(message);
        self.text.deinit(self.allocator);
        self.line_starts.deinit(self.allocator);
        var iterator = self.commands.iterator();
        while (iterator.next()) |entry| self.allocator.free(entry.key_ptr.*);
        self.commands.deinit();
        self.* = undefined;
    }

    fn bytes(self: *const Context) []const u8 {
        return self.text_view orelse self.text.items;
    }

    fn materialize(self: *Context) !void {
        const view = self.text_view orelse return;
        self.text_view = null;
        self.text.clearRetainingCapacity();
        try self.text.appendSlice(self.allocator, view);
    }

    fn load(self: *Context, doc: *document_mod.Document, cursor_line: usize, cursor_column: usize) !void {
        const data = try doc.rangeAlloc(0, doc.size());
        defer self.allocator.free(data);
        self.text_view = null;
        self.text.clearRetainingCapacity();
        try self.text.appendSlice(self.allocator, data);
        self.crlf = doc.eol == .crlf;
        try self.rebuildLines();
        self.cursor_line = @min(cursor_line, self.lineCount() - 1);
        self.cursor_column = cursor_column;
        self.dirty = false;
        self.clearMessage();
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
        // Non-final lines always have a terminator. Avoid reloading it just
        // to strip a byte the line index already proves is there.
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
        try self.materialize();
        const start = self.lineStart(line);
        const end = self.lineEnd(line);
        try self.text.replaceRange(self.allocator, start, end - start, value);
        try self.rebuildLines();
        self.dirty = true;
    }

    fn setText(self: *Context, value: []const u8) !void {
        self.text_view = null;
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
        try self.materialize();
        const offset = self.byteOffset();
        try self.text.insertSlice(self.allocator, offset, value);
        try self.rebuildLines();
        const new_offset = offset + value.len;
        const line = self.lineOfOffset(new_offset);
        self.cursor_line = line;
        self.cursor_column = new_offset - self.lineStart(line);
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

    fn setMessage(self: *Context, message: []const u8) !void {
        self.clearMessage();
        self.message = try self.allocator.dupe(u8, message);
    }

    fn clearMessage(self: *Context) void {
        if (self.message) |message| {
            self.allocator.free(message);
            self.message = null;
        }
    }

    fn takeMessage(self: *Context) ?[]u8 {
        const message = self.message;
        self.message = null;
        return message;
    }
};

pub const Runtime = struct {
    allocator: std.mem.Allocator,
    state: *c.lua_State,
    lua_alloc: *LuaAllocator,
    ctx: *Context,
    last_error: ?[]u8 = null,
    synced_cursor: ?usize = null,
    last_command: ?CommandRef = null,

    const CommandRef = struct {
        name: []u8,
        reference: c_int,
    };

    pub fn init(allocator: std.mem.Allocator) !Runtime {
        // Route Lua's allocations through the editor allocator so large
        // strings and buffers use the same huge-page-backed, reusable path
        // as the document and plugin snapshots.
        const lua_alloc = try allocator.create(LuaAllocator);
        errdefer allocator.destroy(lua_alloc);
        lua_alloc.* = .{ .allocator = allocator };

        const state = c.lua_newstate(luaAlloc, lua_alloc) orelse return error.OutOfMemory;
        errdefer c.lua_close(state);
        openSafeLibs(state);

        const ctx = try allocator.create(Context);
        errdefer allocator.destroy(ctx);
        ctx.* = try Context.init(allocator);
        errdefer ctx.deinit();

        c.lua_newtable(state);
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
        addFunction(state, ctx, "command", apiRegisterCommand);
        c.lua_setglobal(state, "hexal");

        return .{
            .allocator = allocator,
            .state = state,
            .lua_alloc = lua_alloc,
            .ctx = ctx,
        };
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        self.lua_alloc.deinitSmall();
        self.allocator.destroy(self.lua_alloc);
        self.ctx.deinit();
        self.allocator.destroy(self.ctx);
        if (self.last_error) |message| self.allocator.free(message);
        if (self.last_command) |cached| self.allocator.free(cached.name);
        self.* = undefined;
    }

    pub fn loadFile(self: *Runtime, path: []const u8) !void {
        self.clearError();
        self.clearCommandCache();
        const path_z = try self.allocator.alloc(u8, path.len + 1);
        defer self.allocator.free(path_z);
        @memcpy(path_z[0..path.len], path);
        path_z[path.len] = 0;

        if (c.luaL_loadfilex(self.state, path_z.ptr, null) != c.LUA_OK) {
            return self.captureError();
        }
        if (c.lua_pcallk(self.state, 0, 0, 0, 0, null) != c.LUA_OK) {
            return self.captureError();
        }
    }

    pub fn hasCommand(self: Runtime, name: []const u8) bool {
        return self.ctx.commands.contains(name);
    }

    pub fn commandNames(self: Runtime, allocator: std.mem.Allocator) ![]const []const u8 {
        var names = std.ArrayList([]const u8).empty;
        errdefer {
            for (names.items) |name| allocator.free(name);
            names.deinit(allocator);
        }
        var iterator = self.ctx.commands.keyIterator();
        while (iterator.next()) |name| {
            try names.append(allocator, try allocator.dupe(u8, name.*));
        }
        std.mem.sort([]const u8, names.items, {}, lessThan);
        return names.toOwnedSlice(allocator);
    }

    fn clearCommandCache(self: *Runtime) void {
        if (self.last_command) |cached| {
            self.allocator.free(cached.name);
            self.last_command = null;
        }
    }

    fn commandReference(self: *Runtime, name: []const u8) !c_int {
        if (self.last_command) |cached| {
            if (std.mem.eql(u8, cached.name, name)) return cached.reference;
        }
        const reference = self.ctx.commands.get(name) orelse return error.UnknownCommand;
        const copy = try self.allocator.dupe(u8, name);
        self.clearCommandCache();
        self.last_command = .{ .name = copy, .reference = reference };
        return reference;
    }

    pub fn invoke(self: *Runtime, name: []const u8, doc: *document_mod.Document, cursor: *usize) !i64 {
        self.clearError();
        const ref = try self.commandReference(name);

        if (self.ctx.loaded_revision == null or self.ctx.loaded_revision.? != doc.revision) {
            const line = doc.lineOfOffset(cursor.*);
            const column = cursor.* - doc.lineStart(line);
            try self.ctx.load(doc, line, column);
            self.ctx.loaded_revision = doc.revision;
            self.synced_cursor = cursor.*;
        } else if (self.synced_cursor != cursor.*) {
            const line = doc.lineOfOffset(cursor.*);
            const column = cursor.* - doc.lineStart(line);
            self.ctx.setCursor(line, column);
            self.synced_cursor = cursor.*;
        }

        _ = c.lua_rawgeti(self.state, registry_index, ref);
        if (c.lua_pcallk(self.state, 0, 1, 0, 0, null) != c.LUA_OK) {
            self.ctx.loaded_revision = null;
            self.synced_cursor = null;
            return self.captureError();
        }

        var is_number: c_int = 0;
        const checksum = c.lua_tointegerx(self.state, -1, &is_number);
        c.lua_settop(self.state, 0);

        if (self.ctx.dirty) {
            const owned = if (self.ctx.text.items.len == 0)
                try self.allocator.alloc(u8, 0)
            else
                try self.ctx.text.toOwnedSlice(self.allocator);
            doc.replaceAllOwned(owned) catch |err| {
                self.ctx.loaded_revision = null;
                self.synced_cursor = null;
                return err;
            };
            // The table now borrows this buffer and the undo group owns it.
            // Keep the host's view on the same bytes instead of copying.
            self.ctx.text_view = owned;
        }
        cursor.* = @min(self.ctx.byteOffset(), doc.size());
        self.synced_cursor = cursor.*;
        self.ctx.dirty = false;
        self.ctx.loaded_revision = doc.revision;
        return checksum;
    }

    pub fn takeMessage(self: *Runtime) ?[]u8 {
        return self.ctx.takeMessage();
    }

    pub fn takeError(self: *Runtime) ?[]u8 {
        const error_message = self.last_error;
        self.last_error = null;
        return error_message;
    }

    fn clearError(self: *Runtime) void {
        if (self.last_error) |message| {
            self.allocator.free(message);
            self.last_error = null;
        }
    }

    fn captureError(self: *Runtime) error{LuaError} {
        var length: usize = 0;
        const pointer = c.lua_tolstring(self.state, -1, &length);
        if (pointer != null and length > 0) {
            self.last_error = self.allocator.dupe(u8, pointer[0..length]) catch null;
        }
        c.lua_settop(self.state, 0);
        return error.LuaError;
    }
};

fn lessThan(_: void, left: []const u8, right: []const u8) bool {
    return std.mem.lessThan(u8, left, right);
}

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

fn luaAlloc(
    userdata: ?*anyopaque,
    pointer: ?*anyopaque,
    old_size: usize,
    new_size: usize,
) callconv(.c) ?*anyopaque {
    const context: *LuaAllocator = @ptrCast(@alignCast(userdata orelse return null));
    const allocator = context.allocator;
    const large = alloc_mod.HugeAllocator.large_threshold;

    // Lua churns through many tiny strings and table nodes. Recycle those
    // through a small size-class cache so repeated API calls (one `get_line`
    // per line, for example) do not go to libc each time. Only large operands
    // use the editor's huge-page-backed allocator.
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

fn contextFrom(state: *c.lua_State) *Context {
    const pointer = c.lua_touserdata(state, registry_index - 1) orelse unreachable;
    return @ptrCast(@alignCast(pointer));
}

fn fail(state: *c.lua_State, message: [*:0]const u8) c_int {
    _ = c.lua_pushstring(state, message);
    return c.lua_error(state);
}

fn addFunction(
    state: *c.lua_State,
    ctx: *Context,
    name: [*:0]const u8,
    function: c.lua_CFunction,
) void {
    c.lua_pushlightuserdata(state, ctx);
    c.lua_pushcclosure(state, function, 1);
    c.lua_setfield(state, -2, name);
}

fn checkLine(ctx: *Context, line: i64) ?usize {
    if (line < 0) return null;
    const index: usize = @intCast(line);
    if (index >= ctx.lineCount()) return null;
    return index;
}

fn apiLineCount(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    c.lua_pushinteger(state, @intCast(ctx.lineCount()));
    return 1;
}

fn apiGetLine(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var is_number: c_int = 0;
    const line = c.lua_tointegerx(state, 1, &is_number);
    if (is_number == 0) return fail(state, "get_line: expected a line number");
    const index = checkLine(ctx, line) orelse return fail(state, "get_line: line out of range");
    const text = ctx.bytes();
    const bounds = ctx.lineBounds(index);
    const bytes = text[bounds.start..bounds.end];
    _ = c.lua_pushlstring(state, if (bytes.len == 0) "" else bytes.ptr, bytes.len);
    return 1;
}

fn apiSetLine(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var is_number: c_int = 0;
    const line = c.lua_tointegerx(state, 1, &is_number);
    if (is_number == 0) return fail(state, "set_line: expected a line number");
    var length: usize = 0;
    const pointer = c.lua_tolstring(state, 2, &length) orelse return fail(state, "set_line: expected a string");
    const index = checkLine(ctx, line) orelse return fail(state, "set_line: line out of range");
    ctx.setLine(index, pointer[0..length]) catch return fail(state, "set_line: out of memory");
    return 0;
}

fn apiGetText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    const text = ctx.bytes();
    _ = c.lua_pushlstring(state, if (text.len == 0) "" else text.ptr, text.len);
    return 1;
}

fn apiSetText(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var length: usize = 0;
    const pointer = c.lua_tolstring(state, 1, &length) orelse return fail(state, "set_text: expected a string");
    ctx.setText(pointer[0..length]) catch return fail(state, "set_text: out of memory");
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
    var line_number: c_int = 0;
    var column_number: c_int = 0;
    const line = c.lua_tointegerx(state, 1, &line_number);
    const column = c.lua_tointegerx(state, 2, &column_number);
    if (line_number == 0 or column_number == 0 or line < 0 or column < 0) {
        return fail(state, "set_cursor: expected non-negative line and column");
    }
    ctx.setCursor(@intCast(line), @intCast(column));
    return 0;
}

fn apiInsert(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var length: usize = 0;
    const pointer = c.lua_tolstring(state, 1, &length) orelse return fail(state, "insert: expected a string");
    ctx.insert(pointer[0..length]) catch return fail(state, "insert: out of memory");
    return 0;
}

fn apiMessage(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var length: usize = 0;
    const pointer = c.lua_tolstring(state, 1, &length) orelse return fail(state, "message: expected a string");
    ctx.setMessage(pointer[0..length]) catch return fail(state, "message: out of memory");
    return 0;
}

fn apiRegisterCommand(state_opt: ?*c.lua_State) callconv(.c) c_int {
    const state = state_opt orelse return 0;
    const ctx = contextFrom(state);
    var length: usize = 0;
    const name_pointer = c.lua_tolstring(state, 1, &length) orelse return fail(state, "register_command: expected a name");
    if (c.lua_type(state, 2) != c.LUA_TFUNCTION) {
        return fail(state, "register_command: expected a function");
    }
    const name = name_pointer[0..length];

    c.lua_pushvalue(state, 2);
    const reference = c.luaL_ref(state, registry_index);

    const key = ctx.allocator.dupe(u8, name) catch {
        c.luaL_unref(state, registry_index, reference);
        return fail(state, "register_command: out of memory");
    };
    const entry = ctx.commands.getOrPut(key) catch {
        ctx.allocator.free(key);
        c.luaL_unref(state, registry_index, reference);
        return fail(state, "register_command: out of memory");
    };
    if (entry.found_existing) {
        ctx.allocator.free(key);
        c.luaL_unref(state, registry_index, entry.value_ptr.*);
    }
    entry.value_ptr.* = reference;
    c.lua_pushboolean(state, 1);
    return 1;
}
