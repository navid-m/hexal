//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const builtin = @import("builtin");
const windows = std.os.windows;
const is_windows = builtin.os.tag == .windows;
const posix = std.posix;

pub const Key = enum {
    none,
    character,
    enter,
    escape,
    backspace,
    delete,
    tab,
    backtab,
    up,
    down,
    left,
    right,
    home,
    end,
    page_up,
    page_down,
    insert,
    mouse_down,
    mouse_up,
    mouse_move,
    scroll_up,
    scroll_down,
    focus_in,
    focus_out,
    unknown,
};

pub const Event = struct {
    key: Key = .none,
    ch: u21 = 0,
    ctrl: bool = false,
    alt: bool = false,
    shift: bool = false,
    mouse_x: usize = 0,
    mouse_y: usize = 0,
};

pub const Terminal = struct {
    io: std.Io,
    original: if (is_windows) void else ?posix.termios = if (is_windows) {} else null,
    raw: bool = false,
    pending_byte: i32 = -1,
    width: usize = 80,
    height: usize = 24,
    input_handle: if (is_windows) windows.HANDLE else void = if (is_windows) undefined else {},
    output_handle: if (is_windows) windows.HANDLE else void = if (is_windows) undefined else {},
    original_input_mode: if (is_windows) u32 else void = if (is_windows) 0 else {},
    original_output_mode: if (is_windows) u32 else void = if (is_windows) 0 else {},
    original_input_code_page: if (is_windows) u32 else void = if (is_windows) 0 else {},
    original_output_code_page: if (is_windows) u32 else void = if (is_windows) 0 else {},

    pub fn init(io: std.Io) !Terminal {
        if (comptime is_windows) return initWindows(io);
        var term = Terminal{ .io = io };
        term.original = posix.tcgetattr(0) catch return error.NotATerminal;
        var raw = term.original.?;
        raw.iflag.IGNBRK = false;
        raw.iflag.BRKINT = false;
        raw.iflag.PARMRK = false;
        raw.iflag.ISTRIP = false;
        raw.iflag.INLCR = false;
        raw.iflag.IGNCR = false;
        raw.iflag.ICRNL = false;
        raw.iflag.IXON = false;
        raw.oflag.OPOST = false;
        raw.lflag.ECHO = false;
        raw.lflag.ECHONL = false;
        raw.lflag.ICANON = false;
        raw.lflag.ISIG = false;
        raw.lflag.IEXTEN = false;
        raw.cflag.CSIZE = .CS8;
        raw.cflag.PARENB = false;
        raw.cc[@backingInt(posix.V.MIN)] = 1;
        raw.cc[@backingInt(posix.V.TIME)] = 0;
        try posix.tcsetattr(0, .FLUSH, raw);
        term.raw = true;
        term.updateSize();
        try term.write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[?1000h\x1b[?1002h\x1b[?1006h");
        return term;
    }

    pub fn deinit(self: *Terminal) void {
        if (self.raw) {
            self.write("\x1b[0m\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l") catch {};
            if (comptime is_windows) {
                _ = SetConsoleMode(self.input_handle, self.original_input_mode);
                _ = SetConsoleMode(self.output_handle, self.original_output_mode);
                _ = SetConsoleOutputCP(self.original_output_code_page);
                _ = SetConsoleCP(self.original_input_code_page);
            } else {
                if (self.original) |original| {
                    posix.tcsetattr(0, .FLUSH, original) catch {};
                }
            }
            self.raw = false;
        }
    }

    pub fn write(self: *Terminal, bytes: []const u8) !void {
        if (bytes.len == 0) return;
        try std.Io.File.stdout().writeStreamingAll(self.io, bytes);
    }

    pub fn updateSize(self: *Terminal) void {
        if (comptime is_windows) {
            var info: ConsoleScreenBufferInfo = std.mem.zeroes(ConsoleScreenBufferInfo);
            if (GetConsoleScreenBufferInfo(self.output_handle, &info) != 0) {
                self.width = @intCast(@max(1, info.window.right - info.window.left + 1));
                self.height = @intCast(@max(1, info.window.bottom - info.window.top + 1));
            }
            return;
        }
        var ws: posix.winsize = std.mem.zeroes(posix.winsize);
        _ = posix.system.ioctl(0, posix.T.IOCGWINSZ, @intFromPtr(&ws));
        if (ws.col > 0 and ws.row > 0) {
            self.width = ws.col;
            self.height = ws.row;
        } else {
            self.width = 80;
            self.height = 24;
        }
    }

    /// Returns null on timeout. A timeout is used to poll for terminal resize
    /// and to keep the editor responsive without a separate event thread.
    pub fn readEvent(self: *Terminal, timeout_ms: i32) !?Event {
        const first = (try self.readByte(timeout_ms)) orelse return null;
        if (first == 0x1B) return try self.readEscape();

        var event = Event{};
        if (first == '\r' or first == '\n') {
            event.key = .enter;
            return event;
        }
        if (first == 0x7F or first == 0x08) {
            event.key = .backspace;
            return event;
        }
        if (first == 0x09) {
            event.key = .tab;
            return event;
        }
        if (first >= 0x01 and first <= 0x1A) {
            event.key = .character;
            event.ch = 'a' + (first - 1);
            event.ctrl = true;
            return event;
        }
        if (first < 0x20) {
            event.key = .unknown;
            return event;
        }
        if (first < 0x80) {
            event.key = .character;
            event.ch = first;
            return event;
        }

        // UTF-8 scalar input.
        var bytes: [4]u8 = undefined;
        bytes[0] = first;
        const needed = std.unicode.utf8ByteSequenceLength(first) catch 1;
        var count: usize = 1;
        while (count < needed) {
            const next = (try self.readByte(50)) orelse break;
            bytes[count] = next;
            count += 1;
        }
        event.key = .character;
        event.ch = std.unicode.utf8Decode(bytes[0..count]) catch 0xFFFD;
        return event;
    }

    fn readByte(self: *Terminal, timeout_ms: i32) !?u8 {
        if (self.pending_byte >= 0) {
            const value: u8 = @intCast(self.pending_byte);
            self.pending_byte = -1;
            return value;
        }
        if (comptime is_windows) {
            const wait = WaitForSingleObject(
                self.input_handle,
                if (timeout_ms < 0) 0xFFFFFFFF else @intCast(timeout_ms),
            );
            if (wait == WAIT_TIMEOUT) return null;
            if (wait != WAIT_OBJECT_0) return error.EndOfStream;
            var byte: [1]u8 = undefined;
            var read: u32 = 0;
            if (ReadFile(self.input_handle, &byte, 1, &read, null) == 0 or read == 0) {
                return error.EndOfStream;
            }
            return byte[0];
        }

        var fds = [_]posix.pollfd{.{
            .fd = 0,
            .events = posix.POLL.IN,
            .revents = 0,
        }};
        const ready = try posix.poll(&fds, timeout_ms);
        if (ready == 0) return null;
        if ((fds[0].revents & (posix.POLL.ERR | posix.POLL.HUP | posix.POLL.NVAL)) != 0) {
            return error.EndOfStream;
        }
        if ((fds[0].revents & posix.POLL.IN) == 0) return null;

        var buffer: [1]u8 = undefined;
        const count = try posix.read(0, &buffer);
        if (count == 0) return error.EndOfStream;
        return buffer[0];
    }

    fn readEscape(self: *Terminal) !Event {
        var event = Event{};
        const next = (try self.readByte(25)) orelse {
            event.key = .escape;
            return event;
        };
        if (next != '[' and next != 'O') {
            self.pending_byte = next;
            event.key = .escape;
            return event;
        }

        const csi = next == '[';
        var parameters: [32]u8 = undefined;
        var parameter_len: usize = 0;
        var final: u8 = 0;
        var i: usize = 0;
        while (i < 32) : (i += 1) {
            const value = (try self.readByte(25)) orelse {
                event.key = .escape;
                return event;
            };
            if (value >= 0x40 and value <= 0x7E) {
                final = value;
                break;
            }
            if (parameter_len < parameters.len) {
                parameters[parameter_len] = value;
                parameter_len += 1;
            }
        }
        if (final == 0) {
            event.key = .unknown;
            return event;
        }

        const params = parameters[0..parameter_len];
        if (csi and params.len > 0 and params[0] == '<') {
            return decodeMouse(params[1..], final);
        }

        if (!csi) {
            event.key = switch (final) {
                'A' => .up,
                'B' => .down,
                'C' => .right,
                'D' => .left,
                'H' => .home,
                'F' => .end,
                'P' => .unknown, // F1 is reserved for future function-key support.
                else => .unknown,
            };
            return event;
        }

        applyModifiers(params, &event);
        event.key = switch (final) {
            'A' => .up,
            'B' => .down,
            'C' => .right,
            'D' => .left,
            'H' => .home,
            'F' => .end,
            'I' => .focus_in,
            'O' => .focus_out,
            'Z' => .backtab,
            '~' => decodeTilde(params),
            else => .unknown,
        };
        return event;
    }
};

fn applyModifiers(parameters: []const u8, event: *Event) void {
    const semicolon = std.mem.lastIndexOfScalar(u8, parameters, ';') orelse return;
    var modifier: u32 = 0;
    var i = semicolon + 1;
    while (i < parameters.len) : (i += 1) {
        const c = parameters[i];
        if (c < '0' or c > '9') break;
        modifier = modifier * 10 + (c - '0');
    }
    if (modifier == 0) return;
    const mask = modifier - 1;
    event.shift = (mask & 1) != 0;
    event.alt = (mask & 2) != 0;
    event.ctrl = (mask & 4) != 0;
}

fn decodeTilde(parameters: []const u8) Key {
    var code: u32 = 0;
    for (parameters) |c| {
        if (c < '0' or c > '9') break;
        code = code * 10 + (c - '0');
    }
    return switch (code) {
        1, 7 => .home,
        4, 8 => .end,
        2 => .insert,
        3 => .delete,
        5 => .page_up,
        6 => .page_down,
        else => .unknown,
    };
}

fn decodeMouse(parameters: []const u8, final: u8) Event {
    var event = Event{};
    var index: usize = 0;
    const button = readNumber(parameters, &index);
    const x = readNumber(parameters, &index);
    const y = readNumber(parameters, &index);
    event.mouse_x = if (x > 0) x - 1 else 0;
    event.mouse_y = if (y > 0) y - 1 else 0;
    event.shift = (button & 4) != 0;
    event.alt = (button & 8) != 0;
    event.ctrl = (button & 16) != 0;
    const number = button & 3;
    if ((button & 64) != 0) {
        event.key = switch (number) {
            0 => .scroll_up,
            1 => .scroll_down,
            else => .unknown,
        };
        return event;
    }
    if (final != 'm' and (button & 32) != 0) {
        event.key = .mouse_move;
        return event;
    }
    event.key = if (final == 'm') .mouse_up else .mouse_down;
    return event;
}

fn readNumber(parameters: []const u8, index: *usize) usize {
    var value: usize = 0;
    var found = false;
    while (index.* < parameters.len and parameters[index.*] >= '0' and parameters[index.*] <= '9') {
        value = value * 10 + (parameters[index.*] - '0');
        found = true;
        index.* += 1;
    }
    if (index.* < parameters.len and parameters[index.*] == ';') index.* += 1;
    return if (found) value else 0;
}

const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x00000102;
const STD_INPUT_HANDLE: u32 = @bitCast(@as(i32, -10));
const STD_OUTPUT_HANDLE: u32 = @bitCast(@as(i32, -11));
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x0008;

const Coord = extern struct {
    x: i16 = 0,
    y: i16 = 0,
};

const SmallRect = extern struct {
    left: i16 = 0,
    top: i16 = 0,
    right: i16 = 0,
    bottom: i16 = 0,
};

const ConsoleScreenBufferInfo = extern struct {
    size: Coord = .{},
    cursor_position: Coord = .{},
    attributes: u16 = 0,
    window: SmallRect = .{},
    maximum_window_size: Coord = .{},
};

extern "kernel32" fn GetStdHandle(std_handle: u32) callconv(.winapi) windows.HANDLE;
extern "kernel32" fn GetConsoleMode(handle: windows.HANDLE, mode: *u32) callconv(.winapi) c_int;
extern "kernel32" fn SetConsoleMode(handle: windows.HANDLE, mode: u32) callconv(.winapi) c_int;
extern "kernel32" fn GetConsoleScreenBufferInfo(
    handle: windows.HANDLE,
    info: *ConsoleScreenBufferInfo,
) callconv(.winapi) c_int;
extern "kernel32" fn WaitForSingleObject(handle: windows.HANDLE, milliseconds: u32) callconv(.winapi) u32;
extern "kernel32" fn ReadFile(
    handle: windows.HANDLE,
    buffer: *anyopaque,
    bytes_to_read: u32,
    bytes_read: *u32,
    overlapped: ?*anyopaque,
) callconv(.winapi) c_int;
extern "kernel32" fn GetConsoleCP() callconv(.winapi) u32;
extern "kernel32" fn GetConsoleOutputCP() callconv(.winapi) u32;
extern "kernel32" fn SetConsoleCP(code_page: u32) callconv(.winapi) c_int;
extern "kernel32" fn SetConsoleOutputCP(code_page: u32) callconv(.winapi) c_int;

fn initWindows(io: std.Io) !Terminal {
    var term = Terminal{
        .io = io,
        .input_handle = GetStdHandle(STD_INPUT_HANDLE),
        .output_handle = GetStdHandle(STD_OUTPUT_HANDLE),
    };
    if (term.input_handle == windows.INVALID_HANDLE_VALUE or
        term.output_handle == windows.INVALID_HANDLE_VALUE)
    {
        return error.NotATerminal;
    }
    if (GetConsoleMode(term.input_handle, &term.original_input_mode) == 0 or
        GetConsoleMode(term.output_handle, &term.original_output_mode) == 0)
    {
        return error.NotATerminal;
    }
    term.original_input_code_page = GetConsoleCP();
    term.original_output_code_page = GetConsoleOutputCP();
    _ = SetConsoleCP(65001);
    _ = SetConsoleOutputCP(65001);
    if (SetConsoleMode(
        term.input_handle,
        ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_EXTENDED_FLAGS,
    ) == 0) {
        return error.NotATerminal;
    }
    if (SetConsoleMode(
        term.output_handle,
        ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN,
    ) == 0) {
        return error.NotATerminal;
    }
    term.raw = true;
    term.updateSize();
    try term.write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[?1000h\x1b[?1002h\x1b[?1006h");
    return term;
}
