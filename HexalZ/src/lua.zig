//! Embedded Lua 5.4.9 runtime and the small `hexal.*` plugin API.
//!
//! A command invocation snapshots the current document into a line-indexed
//! buffer. Plugin edits are written back as one document replacement, which
//! keeps plugin commands atomic and makes the benchmark repeatable.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const document_mod = @import("document.zig");

const c = @import("lua_c.zig");

const registry_index: c_int = c.LUA_REGISTRYINDEX;

const Context = struct {
    allocator: std.mem.Allocator,
    text: std.ArrayList(u8) = .empty,
    line_starts: std.ArrayList(usize) = .empty,
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

    fn load(self: *Context, doc: *document_mod.Document, cursor_line: usize, cursor_column: usize) !void {
        const data = try doc.rangeAlloc(0, doc.size());
        defer self.allocator.free(data);
        self.text.clearRetainingCapacity();
        try self.text.appendSlice(self.allocator, data);
        try self.rebuildLines();
        self.cursor_line = @min(cursor_line, self.lineCount() - 1);
        self.cursor_column = cursor_column;
        self.dirty = false;
        self.clearMessage();
    }

    fn rebuildLines(self: *Context) !void {
        self.line_starts.clearRetainingCapacity();
        try self.line_starts.append(self.allocator, 0);
        for (self.text.items, 0..) |byte, index| {
            if (byte == '\n') try self.line_starts.append(self.allocator, index + 1);
        }
    }

    fn lineCount(self: Context) usize {
        return self.line_starts.items.len;
    }

    fn lineStart(self: Context, line: usize) usize {
        return self.line_starts.items[@min(line, self.lineCount() - 1)];
    }

    fn lineEnd(self: Context, line: usize) usize {
        if (line + 1 < self.lineCount()) {
            var end = self.line_starts.items[line + 1];
            if (end > 0 and self.text.items[end - 1] == '\n') {
                end -= 1;
                if (end > 0 and self.text.items[end - 1] == '\r') end -= 1;
            }
            return end;
        }
        return self.text.items.len;
    }

    fn lineBytes(self: Context, line: usize) ![]const u8 {
        if (line >= self.lineCount()) return error.InvalidLine;
        return self.text.items[self.lineStart(line)..self.lineEnd(line)];
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
        try self.rebuildLines();
        self.cursor_line = @min(self.cursor_line, self.lineCount() - 1);
        self.dirty = true;
    }

    fn byteOffset(self: Context) usize {
        const line = @min(self.cursor_line, self.lineCount() - 1);
        const start = self.lineStart(line);
        const end = self.lineEnd(line);
        return start + @min(self.cursor_column, end - start);
    }

    fn setCursor(self: *Context, line: usize, column: usize) void {
        self.cursor_line = @min(line, self.lineCount() - 1);
        const start = self.lineStart(self.cursor_line);
        const end = self.lineEnd(self.cursor_line);
        self.cursor_column = @min(column, end - start);
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

    fn lineOfOffset(self: Context, offset: usize) usize {
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
    ctx: *Context,
    last_error: ?[]u8 = null,

    pub fn init(allocator: std.mem.Allocator) !Runtime {
        const state = c.luaL_newstate() orelse return error.OutOfMemory;
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

        return .{ .allocator = allocator, .state = state, .ctx = ctx };
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        self.ctx.deinit();
        self.allocator.destroy(self.ctx);
        if (self.last_error) |message| self.allocator.free(message);
        self.* = undefined;
    }

    pub fn loadFile(self: *Runtime, path: []const u8) !void {
        self.clearError();
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

    pub fn invoke(self: *Runtime, name: []const u8, doc: *document_mod.Document, cursor: *usize) !i64 {
        self.clearError();
        const ref = self.ctx.commands.get(name) orelse return error.UnknownCommand;

        const line = doc.lineOfOffset(cursor.*);
        const column = cursor.* - doc.lineStart(line);
        if (self.ctx.loaded_revision == null or self.ctx.loaded_revision.? != doc.revision) {
            try self.ctx.load(doc, line, column);
            self.ctx.loaded_revision = doc.revision;
        } else {
            self.ctx.setCursor(line, column);
        }

        _ = c.lua_rawgeti(self.state, registry_index, ref);
        if (c.lua_pcallk(self.state, 0, 1, 0, 0, null) != c.LUA_OK) {
            self.ctx.loaded_revision = null;
            return self.captureError();
        }

        var is_number: c_int = 0;
        const checksum = c.lua_tointegerx(self.state, -1, &is_number);
        c.lua_settop(self.state, 0);

        if (self.ctx.dirty) {
            try doc.replace(0, doc.size(), self.ctx.text.items);
        }
        cursor.* = @min(self.ctx.byteOffset(), doc.size());
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
    const bytes = ctx.lineBytes(index) catch return fail(state, "get_line: invalid line");
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
    _ = c.lua_pushlstring(state, if (ctx.text.items.len == 0) "" else ctx.text.items.ptr, ctx.text.items.len);
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
