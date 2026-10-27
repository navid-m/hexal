//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const term = @import("terminal.zig");
const scr = @import("screen.zig");
const theme_mod = @import("theme.zig");
const renderer_mod = @import("renderer.zig");
const document_mod = @import("document.zig");
const syntax = @import("syntax.zig");
const util = @import("util.zig");
const files_mod = @import("files.zig");
const lua_mod = @import("lua.zig");

const TabWidth = 4;
const IndentWidth = 4;
const empty_text: []const u8 = "";

const Mode = enum {
    normal,
    insert,
    select,
    command,
};

const Pending = enum {
    none,
    quit,
    open,
};

const Range = struct {
    start: usize,
    end: usize,
};

const PickerKind = enum {
    none,
    files,
    search,
};

const PickerItem = struct {
    display: []u8,
    file_index: usize,
    line: usize,
    column: usize,
};

const Picker = struct {
    kind: PickerKind = .none,
    query: std.ArrayList(u8) = .empty,
    files: std.ArrayList([]u8) = .empty,
    files_loaded: bool = false,
    items: std.ArrayList(PickerItem) = .empty,
    hits: std.ArrayList(files_mod.SearchHit) = .empty,
    selected: usize = 0,
    top: usize = 0,
    total: usize = 0,
};

const FindMatch = struct {
    start: usize,
    end: usize,
};

const FindState = struct {
    active: bool = false,
    editing: bool = true,
    query: std.ArrayList(u8) = .empty,
    matches: std.ArrayList(FindMatch) = .empty,
    index: isize = -1,
    origin: usize = 0,
};

const SpaceItem = struct {
    key: u8,
    label: []const u8,
};

const space_items = [_]SpaceItem{
    .{ .key = '/', .label = "Search across files" },
    .{ .key = '.', .label = "Find/replace across files" },
    .{ .key = '\\', .label = "Regex find/replace across files" },
    .{ .key = 'f', .label = "Open file picker" },
    .{ .key = 'y', .label = "Yank selection to clipboard" },
    .{ .key = 'X', .label = "Cut selection to clipboard" },
    .{ .key = 'p', .label = "Paste on the current line" },
    .{ .key = 'P', .label = "Paste on a new line" },
    .{ .key = 'K', .label = "LSP hover" },
    .{ .key = 'A', .label = "LSP code actions" },
    .{ .key = 'S', .label = "Document symbols" },
    .{ .key = 'D', .label = "Workspace diagnostics" },
    .{ .key = 'd', .label = "Document diagnostics" },
    .{ .key = 'Z', .label = "Diagnostic on this line" },
    .{ .key = 'b', .label = "Toggle breakpoint" },
};

pub const Editor = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    terminal: *term.Terminal,
    doc: document_mod.Document,
    screen: scr.Screen = .{},
    renderer: renderer_mod.Renderer = .{},
    theme: theme_mod.Theme = .{},

    mode: Mode = .normal,
    running: bool = true,
    cols: usize = 80,
    rows: usize = 24,

    cursor: usize = 0,
    anchor: usize = 0,
    selection_linewise: bool = false,
    preferred_column: ?usize = null,
    top_line: usize = 0,
    left_column: usize = 0,

    message: std.ArrayList(u8) = .empty,
    message_is_error: bool = false,
    command_line: std.ArrayList(u8) = .empty,
    command_cursor: usize = 0,
    clipboard: std.ArrayList(u8) = .empty,
    line_buffer: std.ArrayList(u8) = .empty,
    tokens: std.ArrayList(syntax.Token) = .empty,

    pending_g: bool = false,
    space_menu_open: bool = false,
    pending: Pending = .none,
    pending_path: ?[]u8 = null,
    pending_line: usize = 0,
    pending_column: usize = 0,

    language_override: ?[]u8 = null,
    jumps: std.ArrayList(usize) = .empty,
    jump_index: isize = -1,

    picker: Picker = .{},
    find: FindState = .{},
    lua: ?lua_mod.Runtime = null,

    pub fn init(
        allocator: std.mem.Allocator,
        io: std.Io,
        terminal: *term.Terminal,
        file: ?[]const u8,
    ) !Editor {
        var doc = if (file) |path|
            try document_mod.Document.open(allocator, io, path)
        else
            document_mod.Document.init(allocator, io);
        errdefer doc.deinit();

        // Terminal.init already queried the window size; avoid a second ioctl
        // on the startup path.
        var self = Editor{
            .allocator = allocator,
            .io = io,
            .terminal = terminal,
            .doc = doc,
            .cols = terminal.width,
            .rows = terminal.height,
        };
        if (file != null) {
            self.message.clearRetainingCapacity();
            self.appendMessageFmt("Opened {s}", .{self.doc.displayName()});
        }
        self.loadPlugins() catch |err| {
            self.setErrorFmt("lua plugin load: {s}", .{@errorName(err)});
        };
        return self;
    }

    pub fn deinit(self: *Editor) void {
        if (self.lua) |*runtime| runtime.deinit();
        self.message.deinit(self.allocator);
        self.command_line.deinit(self.allocator);
        self.clipboard.deinit(self.allocator);
        self.line_buffer.deinit(self.allocator);
        self.tokens.deinit(self.allocator);
        if (self.pending_path) |path| self.allocator.free(path);
        if (self.language_override) |id| self.allocator.free(id);
        self.jumps.deinit(self.allocator);
        self.find.query.deinit(self.allocator);
        self.find.matches.deinit(self.allocator);
        self.pickerClearItems();
        self.picker.query.deinit(self.allocator);
        files_mod.clearFiles(self.allocator, &self.picker.files);
        self.picker.files.deinit(self.allocator);
        files_mod.clearHits(self.allocator, &self.picker.hits);
        self.picker.hits.deinit(self.allocator);
        self.renderer.deinit(self.allocator);
        self.screen.deinit(self.allocator);
        self.doc.deinit();
        self.* = undefined;
    }

    pub fn run(self: *Editor) !void {
        var last_width = self.cols;
        var last_height = self.rows;

        // Paint the first frame before waiting for input. The loop below reads
        // with a 30 ms timeout to poll for terminal resizes, so reading first
        // made an idle launch wait for the full timeout before showing anything.
        try self.render();

        while (self.running) {
            self.terminal.updateSize();
            if (self.terminal.width != last_width or self.terminal.height != last_height) {
                last_width = self.terminal.width;
                last_height = self.terminal.height;
                self.cols = last_width;
                self.rows = last_height;
                try self.screen.resize(self.allocator, self.cols, self.rows);
                self.clampViewport();
            }

            const maybe_event = self.terminal.readEvent(30) catch |err| switch (err) {
                error.EndOfStream => break,
                else => return err,
            };
            if (maybe_event) |event| {
                self.handleEvent(event) catch |err| {
                    self.setErrorFmt("internal error: {s}", .{@errorName(err)});
                };
            }

            self.terminal.updateSize();
            if (self.terminal.width != self.cols or self.terminal.height != self.rows) {
                self.cols = self.terminal.width;
                self.rows = self.terminal.height;
                last_width = self.cols;
                last_height = self.rows;
                try self.screen.resize(self.allocator, self.cols, self.rows);
                self.clampViewport();
            }

            try self.render();
        }
    }

    fn handleEvent(self: *Editor, event: term.Event) !void {
        if (self.picker.kind != .none) {
            try self.handlePicker(event);
            return;
        }
        if (self.find.active) {
            try self.handleFind(event);
            return;
        }
        if (self.space_menu_open) {
            try self.handleSpaceMenu(event);
            return;
        }
        if (self.pending != .none) {
            try self.handlePrompt(event);
            return;
        }
        switch (self.mode) {
            .command => try self.handleCommandInput(event),
            .insert => try self.handleInsert(event),
            .select => try self.handleSelect(event),
            .normal => try self.handleNormal(event),
        }
    }

    // -----------------------------------------------------------------------
    // NORMAL MODE
    // -----------------------------------------------------------------------

    fn handleNormal(self: *Editor, event: term.Event) !void {
        if (event.key == .character and event.ctrl) {
            self.pending_g = false;
            if (event.ch == 'r' or event.ch == 'R') {
                if (self.doc.redoEdit()) self.afterUndoRedo();
            } else if (event.ch == 'c' or event.ch == 'C') {
                self.setMessage("Comment toggling is not implemented in this Zig port.");
            }
            return;
        }
        if (event.key == .character and event.alt) {
            self.pending_g = false;
            return;
        }
        if (event.key == .character and event.shift) {
            self.pending_g = false;
            return;
        }

        if (self.pending_g) {
            self.pending_g = false;
            if (event.key == .character) {
                switch (event.ch) {
                    'g', 'G' => self.jumpToDocumentStart(),
                    'e', 'E' => self.jumpToDocumentEnd(),
                    'p', 'P' => self.moveToPreviousLocation(),
                    else => {},
                }
            }
            return;
        }

        switch (event.key) {
            .left => {
                self.moveLeft();
                return;
            },
            .right => {
                self.moveRight();
                return;
            },
            .up => {
                self.moveVertical(-1);
                return;
            },
            .down => {
                self.moveVertical(1);
                return;
            },
            .page_up => {
                self.movePage(-1);
                return;
            },
            .page_down => {
                self.movePage(1);
                return;
            },
            .home => {
                if (event.ctrl) self.jumpToDocumentStart() else self.moveToLineStart();
                return;
            },
            .end => {
                if (event.ctrl) self.jumpToDocumentEnd() else self.moveToLineEnd();
                return;
            },
            .escape => {
                self.doc.breakUndoGroup();
                self.pending_g = false;
                self.clearMessage();
                return;
            },
            .delete => {
                self.deleteAtCursor();
                return;
            },
            else => {},
        }

        if (event.key != .character) return;

        switch (event.ch) {
            ' ' => self.openSpaceMenu(),
            ':' => {
                self.mode = .command;
                self.command_line.clearRetainingCapacity();
                self.command_cursor = 0;
                self.doc.breakUndoGroup();
                self.clearMessage();
            },
            '/' => try self.openFind(),
            '\\' => {
                self.setMessage("Regex search is not implemented; opening literal find.");
                try self.openFind();
            },
            '\'' => self.setMessage("Regex replace is not implemented in this Zig port."),
            '.' => {
                self.setMessage("Multi-cursor find is not implemented in this Zig port.");
            },
            'i', 'I' => self.enterInsert(),
            'a' => {
                const end = self.lineEndOfCursor();
                if (self.cursor < end) self.cursor = self.nextScalar(self.cursor);
                self.enterInsert();
            },
            'A' => {
                self.cursor = self.lineEndOfCursor();
                self.preferred_column = null;
                self.enterInsert();
            },
            'o' => try self.insertNewlineBelow(),
            'O' => try self.insertNewlineAbove(),
            'h' => self.moveLeft(),
            'l' => self.moveRight(),
            'w', 'W' => self.moveWordForward(),
            'q', 'Q' => self.moveWordBackward(),
            'j' => self.moveVertical(1),
            'k' => self.moveVertical(-1),
            'd', 'D' => self.deleteAtCursor(),
            'u' => {
                if (self.doc.undoEdit()) self.afterUndoRedo();
            },
            'U' => {
                if (self.doc.redoEdit()) self.afterUndoRedo();
            },
            'g', 'G' => self.pending_g = true,
            'v', 'V' => self.startCharacterSelection(),
            '%' => self.selectAll(),
            'x', 'X' => self.startLineSelection(),
            '<' => self.indentSelection(true),
            '>' => self.indentSelection(false),
            'C' => self.setMessage("Multi-cursor is not implemented in this Zig port."),
            ',' => self.setMessage("Only the primary cursor is active."),
            else => {},
        }
    }

    // -----------------------------------------------------------------------
    // INSERT MODE
    // -----------------------------------------------------------------------

    fn handleInsert(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.mode = .normal;
            self.pending_g = false;
            self.doc.breakUndoGroup();
            return;
        }

        if (event.key == .character and event.ctrl) {
            if (event.ch == 'r' or event.ch == 'R') {
                if (self.doc.redoEdit()) self.afterUndoRedo();
            }
            return;
        }
        if (event.key == .character and event.alt) return;

        switch (event.key) {
            .left => {
                self.moveLeft();
                return;
            },
            .right => {
                self.moveRight();
                return;
            },
            .up => {
                self.moveVertical(-1);
                return;
            },
            .down => {
                self.moveVertical(1);
                return;
            },
            .page_up => {
                self.movePage(-1);
                return;
            },
            .page_down => {
                self.movePage(1);
                return;
            },
            .home => {
                if (event.ctrl) self.jumpToDocumentStart() else self.moveToLineStart();
                return;
            },
            .end => {
                if (event.ctrl) self.jumpToDocumentEnd() else self.moveToLineEnd();
                return;
            },
            .enter => {
                try self.insertNewline();
                return;
            },
            .backspace => {
                self.backspace();
                return;
            },
            .delete => {
                self.deleteForward();
                return;
            },
            .tab => {
                try self.insertIndent();
                return;
            },
            .character => {
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try self.insertText(buffer[0..length]);
                return;
            },
            else => return,
        }
    }

    // -----------------------------------------------------------------------
    // SELECT MODE
    // -----------------------------------------------------------------------

    fn handleSelect(self: *Editor, event: term.Event) !void {
        if (event.key == .character and event.ctrl) {
            if (event.ch == 'r' or event.ch == 'R') {
                self.cancelSelection();
                if (self.doc.redoEdit()) self.afterUndoRedo();
            } else if (event.ch == 'c' or event.ch == 'C') {
                self.setMessage("Comment toggling is not implemented in this Zig port.");
            }
            return;
        }
        if (event.key == .character and event.alt) return;

        if (self.pending_g) {
            self.pending_g = false;
            if (event.key == .character) {
                switch (event.ch) {
                    'g', 'G' => self.jumpToDocumentStart(),
                    'e', 'E' => self.jumpToDocumentEnd(),
                    'p', 'P' => self.moveToPreviousLocation(),
                    else => {},
                }
            }
            return;
        }

        switch (event.key) {
            .left => {
                self.moveLeft();
                return;
            },
            .right => {
                self.moveRight();
                return;
            },
            .up => {
                self.moveVertical(-1);
                return;
            },
            .down => {
                self.moveVertical(1);
                return;
            },
            .page_up => {
                self.movePage(-1);
                return;
            },
            .page_down => {
                self.movePage(1);
                return;
            },
            .home => {
                if (event.ctrl) self.jumpToDocumentStart() else self.moveToLineStart();
                return;
            },
            .end => {
                if (event.ctrl) self.jumpToDocumentEnd() else self.moveToLineEnd();
                return;
            },
            .escape => {
                self.cancelSelection();
                return;
            },
            .delete => {
                self.deleteSelection();
                return;
            },
            else => {},
        }

        if (event.key != .character) return;
        switch (event.ch) {
            ' ' => self.openSpaceMenu(),
            ':' => {
                self.mode = .command;
                self.command_line.clearRetainingCapacity();
                self.command_cursor = 0;
                self.doc.breakUndoGroup();
                self.clearMessage();
            },
            '%' => self.selectAll(),
            '.' => self.setMessage("Multi-cursor find is not implemented in this Zig port."),
            'x', 'X' => self.extendLineSelection(),
            'd', 'D' => self.deleteSelection(),
            '<' => self.indentSelection(true),
            '>' => self.indentSelection(false),
            'h' => self.moveLeft(),
            'l' => self.moveRight(),
            'w', 'W' => self.moveWordForward(),
            'q', 'Q' => self.moveWordBackward(),
            'j' => self.moveVertical(1),
            'k' => self.moveVertical(-1),
            'i', 'I' => self.enterInsertFromSelection(),
            else => {},
        }
    }

    // -----------------------------------------------------------------------
    // SPACE MENU
    // -----------------------------------------------------------------------

    fn openSpaceMenu(self: *Editor) void {
        self.space_menu_open = true;
        self.pending_g = false;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn handleSpaceMenu(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.space_menu_open = false;
            return;
        }
        if (event.key != .character or event.ctrl or event.alt) return;

        self.space_menu_open = false;
        switch (event.ch) {
            '/', '.', '\\' => {
                if (event.ch == '\\') {
                    self.setMessage("Regex cross-file search is not implemented; using literal search.");
                } else if (event.ch == '.') {
                    self.setMessage("Cross-file replace is not implemented; Enter opens a match.");
                }
                try self.openPicker(.search);
            },
            'f', 'F' => try self.openPicker(.files),
            'y', 'Y' => self.yankSelection(),
            'x', 'X' => self.cutSelection(),
            'p' => try self.pasteClipboard(false),
            'P' => try self.pasteClipboard(true),
            'K', 'k' => self.setMessage("LSP hover is not available in this build."),
            'A', 'a' => self.setMessage("LSP code actions are not available in this build."),
            'S', 's' => self.setMessage("Document symbols are not available in this build."),
            'D' => self.setMessage("Workspace diagnostics are not available in this build."),
            'd' => self.setMessage("Document diagnostics are not available in this build."),
            'Z', 'z' => self.setMessage("Line diagnostics are not available in this build."),
            'b', 'B' => self.setMessage("Debug breakpoints are not available in this build."),
            else => {},
        }
    }

    // -----------------------------------------------------------------------
    // COMMAND MODE
    // -----------------------------------------------------------------------

    fn handleCommandInput(self: *Editor, event: term.Event) !void {
        if (event.key == .character and event.ctrl) return;
        switch (event.key) {
            .escape => {
                self.mode = .normal;
                self.command_line.clearRetainingCapacity();
                self.command_cursor = 0;
                return;
            },
            .enter => {
                try self.submitCommand();
                return;
            },
            .backspace => {
                if (self.command_cursor > 0) {
                    const previous = util.previousBoundary(self.command_line.items, self.command_cursor);
                    const len = self.command_cursor - previous;
                    std.mem.copyForwards(
                        u8,
                        self.command_line.items[previous..],
                        self.command_line.items[self.command_cursor..],
                    );
                    self.command_line.items.len -= len;
                    self.command_cursor = previous;
                }
                return;
            },
            .delete => {
                if (self.command_cursor < self.command_line.items.len) {
                    const next = util.nextBoundary(self.command_line.items, self.command_cursor);
                    const len = next - self.command_cursor;
                    std.mem.copyForwards(
                        u8,
                        self.command_line.items[self.command_cursor..],
                        self.command_line.items[next..],
                    );
                    self.command_line.items.len -= len;
                }
                return;
            },
            .left => {
                self.command_cursor = util.previousBoundary(self.command_line.items, self.command_cursor);
                return;
            },
            .right => {
                self.command_cursor = util.nextBoundary(self.command_line.items, self.command_cursor);
                return;
            },
            .home => {
                self.command_cursor = 0;
                return;
            },
            .end => {
                self.command_cursor = self.command_line.items.len;
                return;
            },
            .character => {
                if (event.alt) return;
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try self.command_line.insertSlice(self.allocator, self.command_cursor, buffer[0..length]);
                self.command_cursor += length;
                return;
            },
            else => return,
        }
    }

    fn submitCommand(self: *Editor) !void {
        const trimmed = std.mem.trim(u8, self.command_line.items, " \t\r\n");
        const line = try self.allocator.dupe(u8, trimmed);
        defer self.allocator.free(line);

        self.mode = .normal;
        self.command_line.clearRetainingCapacity();
        self.command_cursor = 0;
        if (line.len == 0) return;

        var split: usize = 0;
        while (split < line.len and line[split] != ' ' and line[split] != '\t') : (split += 1) {}
        var name = line[0..split];
        var force = false;
        if (name.len > 0 and name[name.len - 1] == '!') {
            force = true;
            name = name[0 .. name.len - 1];
        }
        var args = std.mem.trim(u8, line[split..], " \t");
        if (args.len >= 2 and
            ((args[0] == '"' and args[args.len - 1] == '"') or
                (args[0] == '\'' and args[args.len - 1] == '\'')))
        {
            args = args[1 .. args.len - 1];
        }

        self.doc.breakUndoGroup();

        if (std.mem.eql(u8, name, "w") or std.mem.eql(u8, name, "write")) {
            _ = try self.saveDocument(if (args.len > 0) args else null);
        } else if (std.mem.eql(u8, name, "wq")) {
            if (try self.saveDocument(if (args.len > 0) args else null)) {
                self.running = false;
            }
        } else if (std.mem.eql(u8, name, "q") or std.mem.eql(u8, name, "quit")) {
            if (force) {
                self.running = false;
            } else if (self.doc.isDirty()) {
                self.setPending(.quit);
            } else {
                self.running = false;
            }
        } else if (std.mem.eql(u8, name, "e") or std.mem.eql(u8, name, "edit")) {
            if (args.len == 0) {
                self.setError("usage: :e <filename>");
            } else {
                try self.requestOpen(args, 0, 0, force);
            }
        } else if (std.mem.eql(u8, name, "path")) {
            if (self.doc.path) |path| self.setMessage(path) else self.setMessage("[No Name]");
        } else if (std.mem.eql(u8, name, "lang")) {
            if (args.len == 0) {
                self.setError("usage: :lang <language>");
            } else if (syntax.Language.fromId(args)) |language| {
                if (self.language_override) |old| self.allocator.free(old);
                self.language_override = try self.allocator.dupe(u8, args);
                self.setMessageFmt("Language: {s}", .{language.name});
            } else {
                self.setErrorFmt("unknown language: {s}", .{args});
            }
        } else if (std.mem.eql(u8, name, "cw")) {
            try self.trimTrailingWhitespace();
        } else if (std.mem.eql(u8, name, "lua")) {
            if (args.len == 0) {
                self.setError("usage: :lua <plugin.lua>");
            } else {
                self.loadLuaPlugin(args);
            }
        } else if (std.mem.eql(u8, name, "plugins")) {
            self.listLuaCommands() catch |err| self.setErrorFmt("lua: {s}", .{@errorName(err)});
        } else if (std.mem.eql(u8, name, "help")) {
            self.setMessage("i insert  v select  x line  Space menu  :w :wq :q :e  u/U undo/redo  gg/ge/p");
        } else if (std.mem.eql(u8, name, "about")) {
            self.setMessage("HexalZ 0.1.0 - a modal text editor written in Zig 0.17.0.");
        } else if (std.mem.eql(u8, name, "cfg")) {
            self.setMessage("Configuration files are not implemented in this Zig port.");
        } else if (std.mem.eql(u8, name, "rcfg")) {
            self.setMessage("Configuration reload is not implemented in this Zig port.");
        } else if (std.mem.eql(u8, name, "fmt")) {
            self.setError("no formatter is configured in this build");
        } else {
            if (self.tryLuaCommand(name)) return;
            self.setErrorFmt("unknown command: {s}", .{name});
        }
    }

    fn loadPlugins(self: *Editor) !void {
        var dir = std.Io.Dir.cwd().openDir(self.io, "plugins", .{ .iterate = true }) catch return;
        defer dir.close(self.io);

        var paths = std.ArrayList([]u8).empty;
        defer {
            for (paths.items) |path| self.allocator.free(path);
            paths.deinit(self.allocator);
        }

        var iterator = dir.iterate();
        while (try iterator.next(self.io)) |entry| {
            if (entry.kind == .directory) continue;
            if (!std.mem.endsWith(u8, entry.name, ".lua")) continue;
            try paths.append(self.allocator, try std.fmt.allocPrint(
                self.allocator,
                "plugins/{s}",
                .{entry.name},
            ));
        }
        if (paths.items.len == 0) return;
        std.mem.sort([]u8, paths.items, {}, lessThanPath);

        self.lua = try lua_mod.Runtime.init(self.allocator);
        var loaded: usize = 0;
        for (paths.items) |path| {
            self.lua.?.loadFile(path) catch |err| {
                if (self.lua.?.takeError()) |message| {
                    defer self.allocator.free(message);
                    self.setError(message);
                } else {
                    self.setErrorFmt("lua plugin {s}: {s}", .{ path, @errorName(err) });
                }
                continue;
            };
            loaded += 1;
        }
        if (loaded > 0) {
            self.setMessageFmt("Loaded {d} Lua plugin(s)", .{loaded});
        }
    }

    fn loadLuaPlugin(self: *Editor, path: []const u8) void {
        if (self.lua == null) {
            self.lua = lua_mod.Runtime.init(self.allocator) catch |err| {
                self.setErrorFmt("lua init failed: {s}", .{@errorName(err)});
                return;
            };
        }
        self.lua.?.loadFile(path) catch |err| {
            if (self.lua.?.takeError()) |message| {
                defer self.allocator.free(message);
                self.setError(message);
            } else {
                self.setErrorFmt("lua plugin {s}: {s}", .{ path, @errorName(err) });
            }
            return;
        };
        self.setMessageFmt("Loaded Lua plugin {s}", .{path});
    }

    fn listLuaCommands(self: *Editor) !void {
        if (self.lua == null) {
            self.setMessage("No Lua plugins loaded.");
            return;
        }
        const names = try self.lua.?.commandNames(self.allocator);
        defer {
            for (names) |name| self.allocator.free(name);
            self.allocator.free(names);
        }
        if (names.len == 0) {
            self.setMessage("No Lua commands registered.");
            return;
        }
        var builder = std.ArrayList(u8).empty;
        defer builder.deinit(self.allocator);
        try builder.appendSlice(self.allocator, "Lua commands: ");
        for (names, 0..) |name, index| {
            if (index > 0) try builder.appendSlice(self.allocator, ", ");
            try builder.appendSlice(self.allocator, name);
        }
        self.setMessage(builder.items);
    }

    fn tryLuaCommand(self: *Editor, name: []const u8) bool {
        if (self.lua == null) return false;
        if (!self.lua.?.hasCommand(name)) return false;

        if (self.lua.?.invoke(name, &self.doc, &self.cursor)) |_| {
            if (self.lua.?.takeMessage()) |message| {
                defer self.allocator.free(message);
                self.setMessage(message);
            }
        } else |err| {
            if (self.lua.?.takeError()) |message| {
                defer self.allocator.free(message);
                self.setError(message);
            } else {
                self.setErrorFmt("lua command {s}: {s}", .{ name, @errorName(err) });
            }
        }

        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.mode = .normal;
        self.doc.breakUndoGroup();
        self.snapCursor();
        return true;
    }

    fn lessThanPath(_: void, left: []u8, right: []u8) bool {
        return std.mem.lessThan(u8, left, right);
    }

    fn saveDocument(self: *Editor, path: ?[]const u8) !bool {
        self.doc.save(path) catch |err| {
            self.setErrorFmt("save failed: {s}", .{@errorName(err)});
            return false;
        };
        if (path) |p| {
            self.setMessageFmt("Saved {s}", .{p});
        } else {
            self.setMessageFmt("Saved {s}", .{self.doc.displayName()});
        }
        return true;
    }

    fn trimTrailingWhitespace(self: *Editor) !void {
        var line = self.doc.lineCount();
        var changed: usize = 0;
        while (line > 0) {
            line -= 1;
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const trimmed = util.trimTrailingWhitespace(text);
            if (trimmed.len < text.len) {
                try self.doc.erase(self.doc.lineStart(line) + trimmed.len, text.len - trimmed.len);
                changed += 1;
            }
        }
        self.setMessageFmt("Removed trailing whitespace from {d} line(s).", .{changed});
    }

    // -----------------------------------------------------------------------
    // PROMPTS
    // -----------------------------------------------------------------------

    fn setPending(self: *Editor, pending: Pending) void {
        self.pending = pending;
    }

    fn handlePrompt(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.clearPending();
            self.setMessage("Cancelled.");
            return;
        }
        if (event.key != .character or event.ctrl or event.alt) return;

        if (event.ch == 'y' or event.ch == 'Y') {
            switch (self.pending) {
                .quit => {
                    self.clearPending();
                    self.running = false;
                },
                .open => {
                    const path = self.pending_path orelse {
                        self.clearPending();
                        return;
                    };
                    const line = self.pending_line;
                    const column = self.pending_column;
                    self.pending = .none;
                    self.pending_path = null;
                    defer self.allocator.free(path);
                    try self.loadFile(path, line, column);
                },
                .none => {},
            }
            return;
        }

        if (event.ch == 'n' or event.ch == 'N') {
            self.clearPending();
            self.setMessage("Cancelled.");
        }
    }

    fn clearPending(self: *Editor) void {
        self.pending = .none;
        if (self.pending_path) |path| {
            self.allocator.free(path);
            self.pending_path = null;
        }
    }

    // -----------------------------------------------------------------------
    // FILE OPENING
    // -----------------------------------------------------------------------

    fn requestOpen(self: *Editor, raw_path: []const u8, line: usize, column: usize, force: bool) !void {
        const expanded = try util.expandHome(self.allocator, raw_path);
        if (self.doc.isDirty() and !force) {
            if (self.pending_path) |old| self.allocator.free(old);
            self.pending_path = expanded;
            self.pending_line = line;
            self.pending_column = column;
            self.pending = .open;
            return;
        }
        defer self.allocator.free(expanded);
        try self.loadFile(expanded, line, column);
    }

    fn loadFile(self: *Editor, path: []const u8, line: usize, column: usize) !void {
        const new_doc = document_mod.Document.open(self.allocator, self.io, path) catch |err| {
            self.setErrorFmt("cannot open '{s}': {s}", .{ path, @errorName(err) });
            return;
        };
        self.doc.deinit();
        self.doc = new_doc;

        self.mode = .normal;
        self.anchor = 0;
        self.cursor = 0;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.top_line = 0;
        self.left_column = 0;
        self.pending_g = false;
        self.find.active = false;
        self.find.editing = false;
        self.find.query.clearRetainingCapacity();
        self.find.matches.clearRetainingCapacity();
        self.find.index = -1;
        if (self.language_override) |old| self.allocator.free(old);
        self.language_override = null;
        self.jumps.clearRetainingCapacity();
        self.jump_index = -1;

        const target_line = @min(line, self.doc.lineCount() - 1);
        const start = self.doc.lineStart(target_line);
        const text = self.doc.lineText(target_line, &self.line_buffer) catch empty_text;
        const local = util.byteOffsetForVisualColumn(text, column, TabWidth);
        self.cursor = @min(start + local, self.doc.size());
        self.snapCursor();
        self.anchor = self.cursor;
        self.doc.breakUndoGroup();
        self.setMessageFmt("Opened {s}", .{self.doc.displayName()});
    }

    fn openPicker(self: *Editor, kind: PickerKind) !void {
        self.picker.kind = kind;
        self.picker.query.clearRetainingCapacity();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.pickerClearItems();

        if (!self.picker.files_loaded) {
            files_mod.scan(self.allocator, self.io, &self.picker.files, 20000) catch |err| {
                self.setErrorFmt("file scan failed: {s}", .{@errorName(err)});
            };
            self.picker.files_loaded = true;
        }
        try self.rebuildPicker();
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn closePicker(self: *Editor) void {
        self.picker.kind = .none;
        self.picker.query.clearRetainingCapacity();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.pickerClearItems();
    }

    fn pickerClearItems(self: *Editor) void {
        for (self.picker.items.items) |item| self.allocator.free(item.display);
        self.picker.items.clearRetainingCapacity();
    }

    fn rebuildPicker(self: *Editor) !void {
        self.pickerClearItems();
        self.picker.total = 0;
        const query = self.picker.query.items;

        switch (self.picker.kind) {
            .files => {
                for (self.picker.files.items, 0..) |path, index| {
                    if (query.len > 0 and util.indexOfIgnoreCase(path, query) == null) continue;
                    self.picker.total += 1;
                    if (self.picker.items.items.len >= 1000) continue;
                    const display = try self.allocator.dupe(u8, path);
                    errdefer self.allocator.free(display);
                    try self.picker.items.append(self.allocator, .{
                        .display = display,
                        .file_index = index,
                        .line = 0,
                        .column = 0,
                    });
                }
            },
            .search => {
                if (query.len == 0) return;
                try files_mod.search(
                    self.allocator,
                    self.io,
                    self.picker.files.items,
                    query,
                    &self.picker.hits,
                    500,
                );
                self.picker.total = self.picker.hits.items.len;
                for (self.picker.hits.items) |hit| {
                    const path = self.picker.files.items[hit.file_index];
                    const display = try std.fmt.allocPrint(
                        self.allocator,
                        "{s}:{d}: {s}",
                        .{ path, hit.line + 1, hit.preview },
                    );
                    errdefer self.allocator.free(display);
                    try self.picker.items.append(self.allocator, .{
                        .display = display,
                        .file_index = hit.file_index,
                        .line = hit.line,
                        .column = hit.column,
                    });
                }
                files_mod.clearHits(self.allocator, &self.picker.hits);
            },
            .none => {},
        }

        if (self.picker.selected >= self.picker.items.items.len) {
            self.picker.selected = if (self.picker.items.items.len == 0) 0 else self.picker.items.items.len - 1;
        }
        if (self.picker.top > self.picker.selected) self.picker.top = self.picker.selected;
    }

    fn handlePicker(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.closePicker();
            return;
        }

        if (event.key == .character and !event.ctrl and !event.alt) {
            var buffer: [4]u8 = undefined;
            const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
            try self.picker.query.appendSlice(self.allocator, buffer[0..length]);
            try self.rebuildPicker();
            self.picker.selected = 0;
            self.picker.top = 0;
            return;
        }

        switch (event.key) {
            .backspace => {
                if (self.picker.query.items.len > 0) {
                    const previous = util.previousBoundary(self.picker.query.items, self.picker.query.items.len);
                    self.picker.query.items.len = previous;
                    try self.rebuildPicker();
                    self.picker.selected = 0;
                    self.picker.top = 0;
                }
                return;
            },
            .up => {
                if (self.picker.items.items.len > 0 and self.picker.selected > 0) {
                    self.picker.selected -= 1;
                }
                return;
            },
            .down => {
                if (self.picker.selected + 1 < self.picker.items.items.len) {
                    self.picker.selected += 1;
                }
                return;
            },
            .page_up => {
                const rows = self.pickerListRows();
                self.picker.selected -|= rows;
                return;
            },
            .page_down => {
                const rows = self.pickerListRows();
                if (self.picker.items.items.len > 0) {
                    self.picker.selected = @min(
                        self.picker.items.items.len - 1,
                        self.picker.selected + rows,
                    );
                }
                return;
            },
            .home => {
                self.picker.selected = 0;
                return;
            },
            .end => {
                if (self.picker.items.items.len > 0) {
                    self.picker.selected = self.picker.items.items.len - 1;
                }
                return;
            },
            .enter => {
                try self.openPickerSelection();
                return;
            },
            else => return,
        }
    }

    fn openPickerSelection(self: *Editor) !void {
        if (self.picker.items.items.len == 0) return;
        if (self.picker.selected >= self.picker.items.items.len) return;
        const item = self.picker.items.items[self.picker.selected];
        const path = self.picker.files.items[item.file_index];
        const line = item.line;
        const column = item.column;
        self.closePicker();
        try self.requestOpen(path, line, column, false);
    }

    fn pickerHeight(self: *Editor) usize {
        const available = self.statusRow();
        if (available == 0) return 0;
        return @min(18, available - 1);
    }

    fn pickerListRows(self: *Editor) usize {
        const height = self.pickerHeight();
        return if (height >= 3) height - 3 else 1;
    }

    // -----------------------------------------------------------------------
    // FIND
    // -----------------------------------------------------------------------

    fn openFind(self: *Editor) !void {
        self.find.active = true;
        self.find.editing = true;
        self.find.query.clearRetainingCapacity();
        self.find.matches.clearRetainingCapacity();
        self.find.index = -1;
        self.find.origin = self.cursor;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn closeFind(self: *Editor) void {
        self.find.active = false;
        self.find.editing = false;
        self.find.query.clearRetainingCapacity();
        self.find.matches.clearRetainingCapacity();
        self.find.index = -1;
        self.doc.breakUndoGroup();
    }

    fn rebuildFindMatches(self: *Editor) !void {
        self.find.matches.clearRetainingCapacity();
        if (self.find.query.items.len == 0) {
            self.find.index = -1;
            return;
        }
        const data = try self.doc.rangeAlloc(0, self.doc.size());
        defer self.allocator.free(data);
        var search_from: usize = 0;
        while (search_from < data.len) {
            const relative = util.indexOfIgnoreCase(data[search_from..], self.find.query.items) orelse break;
            const start = search_from + relative;
            const end = start + self.find.query.items.len;
            try self.find.matches.append(self.allocator, .{ .start = start, .end = end });
            if (self.find.matches.items.len >= 20000) break;
            search_from = if (end > start) end else start + 1;
        }

        if (self.find.matches.items.len == 0) {
            self.find.index = -1;
            return;
        }
        var index: isize = 0;
        for (self.find.matches.items, 0..) |match, i| {
            if (match.start >= self.find.origin) {
                index = @intCast(i);
                break;
            }
            index = @intCast(i);
        }
        self.find.index = index;
    }

    fn handleFind(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.closeFind();
            return;
        }

        if (!self.find.editing) {
            switch (event.key) {
                .enter => {
                    self.find.editing = true;
                    return;
                },
                .up => {
                    self.findStep(-1);
                    return;
                },
                .down => {
                    self.findStep(1);
                    return;
                },
                .character => {
                    switch (event.ch) {
                        'n', 'N' => self.findStep(1),
                        'p', 'P' => self.findStep(-1),
                        '/' => self.find.editing = true,
                        else => {},
                    }
                    return;
                },
                else => return,
            }
        }

        switch (event.key) {
            .enter => {
                self.find.editing = false;
                if (self.find.index >= 0) self.jumpToFindMatch();
                return;
            },
            .up => {
                self.findStep(-1);
                return;
            },
            .down => {
                self.findStep(1);
                return;
            },
            .backspace => {
                if (self.find.query.items.len > 0) {
                    const previous = util.previousBoundary(self.find.query.items, self.find.query.items.len);
                    self.find.query.items.len = previous;
                    try self.rebuildFindMatches();
                }
                return;
            },
            .character => {
                if (event.ctrl or event.alt) return;
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try self.find.query.appendSlice(self.allocator, buffer[0..length]);
                try self.rebuildFindMatches();
                return;
            },
            else => return,
        }
    }

    fn findStep(self: *Editor, delta: isize) void {
        if (self.find.matches.items.len == 0) return;
        var index = self.find.index;
        if (index < 0) index = 0;
        index += delta;
        if (index < 0) index = @intCast(self.find.matches.items.len - 1);
        if (index >= @as(isize, @intCast(self.find.matches.items.len))) index = 0;
        self.find.index = index;
        self.jumpToFindMatch();
    }

    fn jumpToFindMatch(self: *Editor) void {
        if (self.find.index < 0) return;
        const index: usize = @intCast(self.find.index);
        if (index >= self.find.matches.items.len) return;
        const match = self.find.matches.items[index];
        self.cursor = match.start;
        self.anchor = match.start;
        self.selection_linewise = false;
        self.mode = .normal;
        self.snapCursor();
        self.doc.breakUndoGroup();
    }

    // -----------------------------------------------------------------------
    // EDITING
    // -----------------------------------------------------------------------

    fn enterInsert(self: *Editor) void {
        self.mode = .insert;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn enterInsertFromSelection(self: *Editor) void {
        self.deleteSelection();
        self.enterInsert();
    }

    fn insertText(self: *Editor, text: []const u8) !void {
        if (self.mode == .select) {
            self.deleteSelection();
            self.mode = .insert;
        }
        try self.doc.insert(self.cursor, text);
        self.cursor += text.len;
        self.anchor = self.cursor;
        self.preferred_column = null;
        self.snapCursor();
    }

    fn insertNewline(self: *Editor) !void {
        const line = self.cursorLine();
        const line_start = self.doc.lineStart(line);
        const text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
        const local = @min(self.cursor - line_start, text.len);
        var indent_end: usize = 0;
        while (indent_end < local and (text[indent_end] == ' ' or text[indent_end] == '\t')) {
            indent_end += 1;
        }

        var builder: std.ArrayList(u8) = .empty;
        defer builder.deinit(self.allocator);
        try builder.append(self.allocator, '\n');
        try builder.appendSlice(self.allocator, text[0..indent_end]);

        const previous = if (local > 0) text[local - 1] else 0;
        if (previous == '{' or previous == '(' or previous == '[' or previous == ':') {
            var i: usize = 0;
            while (i < IndentWidth) : (i += 1) try builder.append(self.allocator, ' ');
        }

        const next = if (local < text.len) text[local] else 0;
        if (next == '}' or next == ')' or next == ']') {
            try builder.append(self.allocator, '\n');
            try builder.appendSlice(self.allocator, text[0..indent_end]);
            const inserted = try self.doc.allocator.dupe(u8, builder.items);
            defer self.doc.allocator.free(inserted);
            try self.doc.insert(self.cursor, inserted);
            // Keep the cursor on the new indent line before the closing token.
            const extra_indent: usize = if (previous == '{' or previous == '(' or previous == '[' or previous == ':')
                IndentWidth
            else
                0;
            self.cursor += 1 + indent_end + extra_indent;
            self.anchor = self.cursor;
            self.preferred_column = null;
            self.snapCursor();
            return;
        }

        try self.insertText(builder.items);
    }

    fn insertNewlineBelow(self: *Editor) !void {
        const line = self.cursorLine();
        const end = self.doc.lineEnd(line);
        var builder: std.ArrayList(u8) = .empty;
        defer builder.deinit(self.allocator);
        try builder.append(self.allocator, '\n');
        const text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
        var indent: usize = 0;
        while (indent < text.len and (text[indent] == ' ' or text[indent] == '\t')) indent += 1;
        try builder.appendSlice(self.allocator, text[0..indent]);
        try self.doc.insert(end, builder.items);
        self.cursor = end + 1 + indent;
        self.anchor = self.cursor;
        self.mode = .insert;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
    }

    fn insertNewlineAbove(self: *Editor) !void {
        const line = self.cursorLine();
        const start = self.doc.lineStart(line);
        const text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
        var indent: usize = 0;
        while (indent < text.len and (text[indent] == ' ' or text[indent] == '\t')) indent += 1;
        var builder: std.ArrayList(u8) = .empty;
        defer builder.deinit(self.allocator);
        try builder.appendSlice(self.allocator, text[0..indent]);
        try builder.append(self.allocator, '\n');
        try self.doc.insert(start, builder.items);
        self.cursor = start + indent;
        self.anchor = self.cursor;
        self.mode = .insert;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
    }

    fn insertIndent(self: *Editor) !void {
        var spaces: [IndentWidth]u8 = @splat(' ');
        try self.insertText(&spaces);
    }

    fn backspace(self: *Editor) void {
        if (self.cursor == 0) return;
        var previous = self.previousScalar(self.cursor);
        // Join CRLF lines as one deletion.
        if (previous > 0 and self.doc.table.byteAt(previous) == '\n' and
            self.doc.table.byteAt(previous - 1) == '\r')
        {
            previous -= 1;
        }
        self.doc.erase(previous, self.cursor - previous) catch {};
        self.cursor = previous;
        self.anchor = self.cursor;
        self.preferred_column = null;
        self.snapCursor();
    }

    fn deleteForward(self: *Editor) void {
        if (self.cursor >= self.doc.size()) return;
        var end = self.nextScalar(self.cursor);
        // A line break is a single user-visible character even when stored as
        // CRLF.
        if (self.cursor == self.lineEndOfCursor() and
            self.doc.table.byteAt(self.cursor) == '\r' and
            self.cursor + 1 < self.doc.size() and
            self.doc.table.byteAt(self.cursor + 1) == '\n')
        {
            end = self.cursor + 2;
        }
        self.doc.erase(self.cursor, end - self.cursor) catch {};
        self.snapCursor();
    }

    fn deleteAtCursor(self: *Editor) void {
        if (self.mode == .select) {
            self.deleteSelection();
            return;
        }
        self.deleteForward();
    }

    fn deleteSelection(self: *Editor) void {
        if (self.mode != .select) return;
        const range = self.selectionRange();
        if (range.end > range.start) {
            self.doc.replace(range.start, range.end - range.start, "") catch {};
        }
        self.cursor = @min(range.start, self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = .normal;
        self.preferred_column = null;
        self.snapCursor();
    }

    fn indentSelection(self: *Editor, unindent: bool) void {
        if (self.mode != .select) {
            self.setMessage("Select lines before indenting.");
            return;
        }
        const range = self.selectionRange();
        const first_line = self.doc.lineOfOffset(range.start);
        var last_line = self.doc.lineOfOffset(range.end);
        if (last_line > first_line and range.end == self.doc.lineStart(last_line)) {
            last_line -= 1;
        }
        var line = first_line;
        while (line <= last_line) : (line += 1) {
            const start = self.doc.lineStart(line);
            if (unindent) {
                const text = self.doc.lineText(line, &self.line_buffer) catch continue;
                var count: usize = 0;
                while (count < text.len and count < IndentWidth and text[count] == ' ') count += 1;
                if (count == 0 and text.len > 0 and text[0] == '\t') count = 1;
                if (count > 0) self.doc.erase(start, count) catch {};
            } else {
                const spaces: [IndentWidth]u8 = @splat(' ');
                self.doc.insert(start, &spaces) catch {};
            }
        }
        self.doc.breakUndoGroup();
    }

    fn insertNewlineWithIndentFallback(self: *Editor) !void {
        try self.insertNewline();
    }

    // -----------------------------------------------------------------------
    // SELECTION
    // -----------------------------------------------------------------------

    fn startCharacterSelection(self: *Editor) void {
        self.mode = .select;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn startLineSelection(self: *Editor) void {
        self.mode = .select;
        self.selection_linewise = true;
        self.anchor = self.doc.lineStart(self.cursorLine());
        self.cursor = self.anchor;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn extendLineSelection(self: *Editor) void {
        if (!self.selection_linewise) {
            self.selection_linewise = true;
            self.anchor = self.doc.lineStart(self.cursorLine());
            self.cursor = self.anchor;
        } else {
            const line = self.cursorLine();
            if (line + 1 < self.doc.lineCount()) {
                self.cursor = self.doc.lineStart(line + 1);
            } else {
                self.setMessage("Already at the last line.");
            }
        }
        self.preferred_column = null;
        self.doc.breakUndoGroup();
    }

    fn selectAll(self: *Editor) void {
        self.mode = .select;
        self.selection_linewise = false;
        self.anchor = 0;
        self.cursor = self.doc.size();
        self.preferred_column = null;
        self.doc.breakUndoGroup();
    }

    fn cancelSelection(self: *Editor) void {
        self.mode = .normal;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.pending_g = false;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn selectionRange(self: *Editor) Range {
        if (self.selection_linewise) {
            const anchor_line = self.doc.lineOfOffset(self.anchor);
            const cursor_line = self.doc.lineOfOffset(self.cursor);
            const first = @min(anchor_line, cursor_line);
            const last = @max(anchor_line, cursor_line);
            const start = self.doc.lineStart(first);
            const end = if (last + 1 < self.doc.lineCount()) self.doc.lineStart(last + 1) else self.doc.size();
            return .{ .start = start, .end = end };
        }

        var range_start = @min(self.anchor, self.cursor);
        var range_end = @max(self.anchor, self.cursor);
        if (range_end < self.doc.size()) {
            const line = self.doc.lineOfOffset(range_end);
            const line_start = self.doc.lineStart(line);
            const line_end = self.doc.lineEnd(line);
            if (range_end < line_end) {
                range_end = self.nextScalar(range_end);
            } else if (range_end == line_end and line + 1 < self.doc.lineCount()) {
                range_end = self.doc.lineStart(line + 1);
            }
            _ = line_start;
        }
        if (range_start > range_end) range_start = range_end;
        return .{ .start = range_start, .end = range_end };
    }

    fn yankSelection(self: *Editor) void {
        const range = if (self.mode == .select)
            self.selectionRange()
        else
            Range{
                .start = self.doc.lineStart(self.cursorLine()),
                .end = if (self.cursorLine() + 1 < self.doc.lineCount())
                    self.doc.lineStart(self.cursorLine() + 1)
                else
                    self.doc.size(),
            };
        if (range.end <= range.start) return;
        const text = self.doc.rangeAlloc(range.start, range.end - range.start) catch return;
        defer self.allocator.free(text);
        self.storeClipboard(text);
        if (self.mode == .select) {
            self.cursor = range.start;
            self.cancelSelection();
        }
        self.setMessageFmt("Yanked {d} byte(s).", .{text.len});
    }

    fn cutSelection(self: *Editor) void {
        const range = if (self.mode == .select)
            self.selectionRange()
        else
            Range{
                .start = self.doc.lineStart(self.cursorLine()),
                .end = if (self.cursorLine() + 1 < self.doc.lineCount())
                    self.doc.lineStart(self.cursorLine() + 1)
                else
                    self.doc.size(),
            };
        if (range.end <= range.start) return;
        const text = self.doc.rangeAlloc(range.start, range.end - range.start) catch return;
        defer self.allocator.free(text);
        self.storeClipboard(text);
        self.doc.replace(range.start, range.end - range.start, "") catch {};
        self.cursor = @min(range.start, self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = .normal;
        self.snapCursor();
        self.setMessageFmt("Cut {d} byte(s).", .{text.len});
    }

    fn storeClipboard(self: *Editor, text: []const u8) void {
        self.clipboard.clearRetainingCapacity();
        self.clipboard.appendSlice(self.allocator, text) catch return;

        // OSC 52 provides a best-effort system clipboard integration that also
        // works over SSH. The internal clipboard is always authoritative.
        const encoded_len = std.base64.standard.Encoder.calcSize(text.len);
        const encoded = self.allocator.alloc(u8, encoded_len) catch return;
        defer self.allocator.free(encoded);
        _ = std.base64.standard.Encoder.encode(encoded, text);
        var sequence: std.ArrayList(u8) = .empty;
        defer sequence.deinit(self.allocator);
        sequence.appendSlice(self.allocator, "\x1b]52;c;") catch return;
        sequence.appendSlice(self.allocator, encoded) catch return;
        sequence.append(self.allocator, 0x07) catch return;
        self.terminal.write(sequence.items) catch {};
    }

    fn pasteClipboard(self: *Editor, new_line: bool) !void {
        if (self.clipboard.items.len == 0) {
            self.setError("clipboard is empty");
            return;
        }

        if (new_line) {
            const line = self.cursorLine();
            const end = self.doc.lineEnd(line);
            var builder: std.ArrayList(u8) = .empty;
            defer builder.deinit(self.allocator);
            try builder.append(self.allocator, '\n');
            try builder.appendSlice(self.allocator, self.clipboard.items);
            try self.doc.insert(end, builder.items);
            self.cursor = @min(end + builder.items.len, self.doc.size());
        } else {
            if (self.mode == .select) {
                const range = self.selectionRange();
                try self.doc.replace(range.start, range.end - range.start, self.clipboard.items);
                self.cursor = @min(range.start + self.clipboard.items.len, self.doc.size());
            } else {
                try self.doc.insert(self.cursor, self.clipboard.items);
                self.cursor += self.clipboard.items.len;
            }
        }

        self.mode = .normal;
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.snapCursor();
        self.setMessageFmt("Pasted {d} byte(s).", .{self.clipboard.items.len});
    }

    // -----------------------------------------------------------------------
    // MOVEMENT
    // -----------------------------------------------------------------------

    fn cursorLine(self: *Editor) usize {
        return @min(self.doc.lineOfOffset(self.cursor), self.doc.lineCount() - 1);
    }

    fn lineStartOfCursor(self: *Editor) usize {
        return self.doc.lineStart(self.cursorLine());
    }

    fn lineEndOfCursor(self: *Editor) usize {
        return self.doc.lineEnd(self.cursorLine());
    }

    fn cursorVisualColumn(self: *Editor) usize {
        const line = self.cursorLine();
        const start = self.doc.lineStart(line);
        const text = self.doc.lineText(line, &self.line_buffer) catch return 0;
        const local = @min(self.cursor - start, text.len);
        return util.visualColumn(text, local, TabWidth);
    }

    fn nextScalar(self: *Editor, offset: usize) usize {
        if (offset >= self.doc.size()) return self.doc.size();
        const first = self.doc.table.byteAt(offset);
        const length = std.unicode.utf8ByteSequenceLength(first) catch 1;
        return @min(self.doc.size(), offset + length);
    }

    fn previousScalar(self: *Editor, offset: usize) usize {
        if (offset == 0) return 0;
        var pos = offset - 1;
        while (pos > 0 and (self.doc.table.byteAt(pos) & 0xC0) == 0x80) pos -= 1;
        return pos;
    }

    fn snapCursor(self: *Editor) void {
        if (self.cursor > self.doc.size()) self.cursor = self.doc.size();
        while (self.cursor > 0 and self.cursor < self.doc.size() and
            (self.doc.table.byteAt(self.cursor) & 0xC0) == 0x80)
        {
            self.cursor -= 1;
        }
        if (self.anchor > self.doc.size()) self.anchor = self.doc.size();
        self.clampViewport();
    }

    fn moveLeft(self: *Editor) void {
        const line = self.cursorLine();
        const start = self.doc.lineStart(line);
        if (self.cursor > start) {
            self.cursor = self.previousScalar(self.cursor);
        } else if (line > 0) {
            self.cursor = self.doc.lineEnd(line - 1);
        }
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn moveRight(self: *Editor) void {
        const line = self.cursorLine();
        const end = self.doc.lineEnd(line);
        if (self.cursor < end) {
            self.cursor = self.nextScalar(self.cursor);
        } else if (line + 1 < self.doc.lineCount()) {
            self.cursor = self.doc.lineStart(line + 1);
        }
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn moveVertical(self: *Editor, delta: isize) void {
        const line = self.cursorLine();
        const target_signed: isize = @as(isize, @intCast(line)) + delta;
        if (target_signed < 0) return;
        const target: usize = @intCast(target_signed);
        if (target >= self.doc.lineCount()) return;

        if (self.preferred_column == null) self.preferred_column = self.cursorVisualColumn();
        const target_start = self.doc.lineStart(target);
        const text = self.doc.lineText(target, &self.line_buffer) catch empty_text;
        const local = util.byteOffsetForVisualColumn(text, self.preferred_column.?, TabWidth);
        self.cursor = @min(target_start + local, self.doc.size());
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn movePage(self: *Editor, direction: isize) void {
        const visible = if (self.rows > 3) self.rows - 3 else 1;
        self.moveVertical(direction * @as(isize, @intCast(visible)));
    }

    fn moveToLineStart(self: *Editor) void {
        self.cursor = self.lineStartOfCursor();
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn moveToLineEnd(self: *Editor) void {
        self.cursor = self.lineEndOfCursor();
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn jumpToDocumentStart(self: *Editor) void {
        self.recordJump();
        self.cursor = 0;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn jumpToDocumentEnd(self: *Editor) void {
        self.recordJump();
        self.cursor = self.doc.size();
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn moveWordForward(self: *Editor) void {
        var line = self.cursorLine();
        var start = self.doc.lineStart(line);
        var text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
        var local = @min(self.cursor - start, text.len);

        while (local < text.len and util.isWordByte(text[local])) local += 1;
        while (local < text.len and !util.isWordByte(text[local])) local += 1;

        if (local >= text.len) {
            line += 1;
            while (line < self.doc.lineCount()) : (line += 1) {
                start = self.doc.lineStart(line);
                text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
                local = 0;
                while (local < text.len and !util.isWordByte(text[local])) local += 1;
                if (local < text.len) break;
            }
            if (line >= self.doc.lineCount()) {
                self.cursor = self.doc.size();
                self.snapCursor();
                return;
            }
        }

        self.cursor = @min(start + local, self.doc.size());
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn moveWordBackward(self: *Editor) void {
        if (self.cursor == 0) return;
        var line = self.cursorLine();
        var start = self.doc.lineStart(line);
        var text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
        var local = @min(self.cursor - start, text.len);

        while (local > 0) {
            const previous = util.previousBoundary(text, local);
            if (util.isWordByte(text[previous])) break;
            local = previous;
        }
        while (local > 0) {
            const previous = util.previousBoundary(text, local);
            if (!util.isWordByte(text[previous])) break;
            local = previous;
        }

        if (local == 0 and self.cursor == start and line > 0) {
            line -= 1;
            start = self.doc.lineStart(line);
            text = self.doc.lineText(line, &self.line_buffer) catch empty_text;
            local = text.len;
            while (local > 0) {
                const previous = util.previousBoundary(text, local);
                if (!util.isWordByte(text[previous])) break;
                local = previous;
            }
        }

        self.cursor = @min(start + local, self.doc.size());
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn recordJump(self: *Editor) void {
        if (self.cursor == 0) return;
        if (self.jumps.items.len > 0 and self.jumps.items[self.jumps.items.len - 1] == self.cursor) return;
        self.jumps.append(self.allocator, self.cursor) catch return;
        if (self.jumps.items.len > 200) {
            _ = self.jumps.orderedRemove(0);
        }
        self.jump_index = @intCast(self.jumps.items.len - 1);
    }

    fn moveToPreviousLocation(self: *Editor) void {
        if (self.jump_index > 0) self.jump_index -= 1;
        if (self.jump_index >= 0) {
            const index: usize = @intCast(self.jump_index);
            if (index < self.jumps.items.len) {
                self.cursor = self.jumps.items[index];
                self.preferred_column = null;
                self.snapCursor();
            }
        }
    }

    fn afterUndoRedo(self: *Editor) void {
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = .normal;
        self.snapCursor();
    }

    // -----------------------------------------------------------------------
    // VIEWPORT
    // -----------------------------------------------------------------------

    fn statusRow(self: *Editor) usize {
        return if (self.rows >= 2) self.rows - 2 else 0;
    }

    fn commandRow(self: *Editor) usize {
        return if (self.rows > 0) self.rows - 1 else 0;
    }

    fn textRows(self: *Editor) usize {
        return self.statusRow();
    }

    fn gutterWidth(self: *Editor) usize {
        return util.digitCount(self.doc.lineCount()) + 1;
    }

    fn clampViewport(self: *Editor) void {
        const last_line = self.doc.lineCount() - 1;
        if (self.top_line > last_line) self.top_line = last_line;
        const max_left = if (self.cols > 1) self.cols - 1 else 0;
        if (self.left_column > max_left) self.left_column = max_left;
    }

    fn ensureCursorVisible(self: *Editor) void {
        const rows = self.textRows();
        if (rows == 0) {
            self.top_line = self.cursorLine();
            return;
        }

        const line = self.cursorLine();
        if (line < self.top_line) self.top_line = line;
        if (line >= self.top_line + rows) self.top_line = line - rows + 1;

        const gutter = self.gutterWidth();
        if (self.cols <= gutter) return;
        const width = self.cols - gutter;
        const column = self.cursorVisualColumn();
        if (column < self.left_column) self.left_column = column;
        if (column >= self.left_column + width) self.left_column = column - width + 1;
        self.clampViewport();
    }

    // -----------------------------------------------------------------------
    // RENDERING
    // -----------------------------------------------------------------------

    fn render(self: *Editor) !void {
        try self.screen.resize(self.allocator, self.cols, self.rows);
        self.screen.clear(self.theme.base());
        if (self.cols == 0 or self.rows == 0) return;

        self.ensureCursorVisible();

        const rows = self.textRows();
        const gutter = self.gutterWidth();
        const first_line = self.top_line;
        const language = self.activeLanguage();

        var row: usize = 0;
        while (row < rows) : (row += 1) {
            const line = first_line + row;
            if (line >= self.doc.lineCount()) break;
            self.renderLineNumber(line, row);
            self.renderLineText(line, row, gutter, language);
        }

        self.renderSelection(first_line, rows, gutter);
        self.renderFindMatches(first_line, rows, gutter);
        self.renderCursor(first_line, rows, gutter);
        self.renderStatusLine();
        self.renderBottomLine();
        self.renderSpaceMenu();
        self.renderPicker();
        self.renderFind();

        const output = try self.renderer.render(self.allocator, self.screen, false);
        try self.terminal.write(output);
    }

    fn renderLineNumber(self: *Editor, line: usize, row: usize) void {
        const digits = util.digitCount(self.doc.lineCount());
        const number = line + 1;
        const x = digits - util.digitCount(number);
        const style = if (line == self.cursorLine()) self.theme.accentStyle() else self.theme.mutedStyle();
        self.screen.putInt(@intCast(x), @intCast(row), number, style, digits);
    }

    fn renderLineText(
        self: *Editor,
        line: usize,
        row: usize,
        gutter: usize,
        language: ?syntax.Language,
    ) void {
        if (self.cols <= gutter) return;
        const text = self.doc.lineText(line, &self.line_buffer) catch return;
        syntax.tokenizeLine(self.allocator, text, language, &self.tokens) catch {};
        const text_width = self.cols - gutter;

        var offset: usize = 0;
        var visual: usize = 0;
        var token_index: usize = 0;

        while (offset < text.len) {
            const rune_start = offset;
            while (token_index < self.tokens.items.len and self.tokens.items[token_index].end <= rune_start) {
                token_index += 1;
            }
            var kind: theme_mod.SyntaxKind = .default;
            if (token_index < self.tokens.items.len and self.tokens.items[token_index].start <= rune_start) {
                kind = self.tokens.items[token_index].kind;
            }
            const style = self.theme.syntaxStyle(kind);

            const decoded = util.decodeAt(text, offset);
            if (decoded.len == 0) break;
            offset += decoded.len;

            var cp = decoded.codepoint;
            var width = util.displayWidth(cp);
            if (cp == '\t') {
                width = TabWidth - (visual % TabWidth);
            } else if (width == 0) {
                if (cp < 0x20 or cp == 0x7F) {
                    cp = util.replacement;
                    width = 1;
                } else {
                    continue;
                }
            } else if (cp < 0x20 or cp == 0x7F) {
                cp = util.replacement;
                width = 1;
            }

            const next_visual = visual + width;
            if (next_visual <= self.left_column) {
                visual = next_visual;
                continue;
            }

            if (visual < self.left_column) {
                const visible = next_visual - self.left_column;
                var i: usize = 0;
                while (i < visible and i < text_width) : (i += 1) {
                    self.screen.put(@intCast(gutter + i), @intCast(row), ' ', style, 1);
                }
                visual = next_visual;
                continue;
            }

            const screen_x = gutter + (visual - self.left_column);
            if (screen_x >= self.cols) break;
            const available = self.cols - screen_x;
            if (width <= available) {
                if (cp == '\t') {
                    var i: usize = 0;
                    while (i < width) : (i += 1) {
                        self.screen.put(@intCast(screen_x + i), @intCast(row), ' ', style, 1);
                    }
                } else {
                    self.screen.put(@intCast(screen_x), @intCast(row), cp, style, width);
                }
            } else {
                var i: usize = 0;
                while (i < available) : (i += 1) {
                    self.screen.put(@intCast(screen_x + i), @intCast(row), ' ', style, 1);
                }
            }

            visual = next_visual;
            if (visual - self.left_column >= text_width) break;
        }
    }

    fn renderSelection(self: *Editor, first_line: usize, rows: usize, gutter: usize) void {
        if (self.mode != .select or rows == 0) return;
        const range = self.selectionRange();
        if (range.end <= range.start) return;
        const style = self.theme.selectionStyle();

        var row: usize = 0;
        while (row < rows) : (row += 1) {
            const line = first_line + row;
            if (line >= self.doc.lineCount()) break;
            const line_start = self.doc.lineStart(line);
            const line_end = self.doc.lineEnd(line);
            const next_line_start = if (line + 1 < self.doc.lineCount())
                self.doc.lineStart(line + 1)
            else
                self.doc.size();

            if (self.selection_linewise) {
                if (range.start < next_line_start and range.end > line_start) {
                    var x = gutter;
                    while (x < self.cols) : (x += 1) {
                        self.screen.applyStyle(x, row, style);
                    }
                }
                continue;
            }

            const from = @max(range.start, line_start);
            const to = @min(range.end, line_end);
            if (to <= from) {
                if (range.start == line_start and range.end > line_start and line_end == line_start and gutter < self.cols) {
                    self.screen.applyStyle(gutter, row, style);
                }
                continue;
            }

            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const local_from = @min(from - line_start, text.len);
            const local_to = @min(to - line_start, text.len);
            const visual_from = util.visualColumn(text, local_from, TabWidth);
            const visual_to = util.visualColumn(text, local_to, TabWidth);
            const first_x = @max(gutter, gutter + visual_from -| self.left_column);
            const last_x = @min(self.cols, gutter + visual_to -| self.left_column);
            var x = first_x;
            while (x < last_x) : (x += 1) {
                self.screen.applyStyle(x, row, style);
            }
        }
    }

    fn renderFindMatches(self: *Editor, first_line: usize, rows: usize, gutter: usize) void {
        if (!self.find.active or self.find.matches.items.len == 0 or rows == 0) return;
        const match_style = self.theme.matchStyle();
        const current_style = self.theme.currentMatchStyle();

        for (self.find.matches.items, 0..) |match, index| {
            const line = self.doc.lineOfOffset(match.start);
            if (line < first_line or line >= first_line + rows) continue;
            const line_start = self.doc.lineStart(line);
            const line_end = self.doc.lineEnd(line);
            if (match.start > line_end) continue;
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const local_from = @min(match.start - line_start, text.len);
            const local_to = @min(match.end - line_start, text.len);
            if (local_to <= local_from) continue;
            const visual_from = util.visualColumn(text, local_from, TabWidth);
            const visual_to = util.visualColumn(text, local_to, TabWidth);
            if (visual_to <= self.left_column) continue;
            const first_x = @max(gutter, gutter + visual_from -| self.left_column);
            const last_x = @min(self.cols, gutter + visual_to -| self.left_column);
            const style = if (self.find.index >= 0 and @as(usize, @intCast(self.find.index)) == index)
                current_style
            else
                match_style;
            var x = first_x;
            while (x < last_x) : (x += 1) {
                self.screen.applyStyle(x, line - first_line, style);
            }
        }
    }

    fn renderCursor(self: *Editor, first_line: usize, rows: usize, gutter: usize) void {
        const line = self.cursorLine();
        if (line < first_line or line >= first_line + rows) return;
        const column = self.cursorVisualColumn();
        if (column < self.left_column) return;
        const x = gutter + (column - self.left_column);
        if (x >= self.cols) return;
        const row = line - first_line;
        var style = self.screen.at(x, row).style;
        style.reverse = true;
        self.screen.applyStyle(x, row, style);
    }

    fn renderStatusLine(self: *Editor) void {
        if (self.rows == 0) return;
        const row = self.statusRow();
        const style = self.statusStyle();
        self.screen.fill(0, @intCast(row), self.cols, 1, ' ', style);

        const line = self.cursorLine();
        const column = self.cursorVisualColumn();
        const mode = self.modeLabel();
        const language = self.activeLanguage();

        var left_buf: [512]u8 = undefined;
        var left_len: usize = 0;
        left_len += (std.fmt.bufPrint(left_buf[left_len..], "{s}    {s}", .{
            mode,
            self.doc.displayName(),
        }) catch empty_text).len;
        if (self.doc.isDirty()) {
            left_len += (std.fmt.bufPrint(left_buf[left_len..], " [+]", .{}) catch empty_text).len;
        }
        if (language) |lang| {
            left_len += (std.fmt.bufPrint(left_buf[left_len..], "  [{s}]", .{lang.name}) catch empty_text).len;
        }

        var right_buf: [160]u8 = undefined;
        var bom_buf: [192]u8 = undefined;
        var right = std.fmt.bufPrint(&right_buf, "{d}:{d}  {d}/{d}  {s}", .{
            line + 1,
            column + 1,
            line + 1,
            self.doc.lineCount(),
            self.doc.eol.label(),
        }) catch empty_text;
        if (self.doc.bom) {
            right = std.fmt.bufPrint(&bom_buf, "{s} BOM", .{right}) catch right;
        }

        var right_x = if (self.cols > right.len + 1) self.cols - right.len - 1 else 1;
        if (right_x < 1) right_x = 1;
        const left_max = if (right_x > 1) right_x - 1 else 0;
        if (left_max > 0) {
            self.screen.putString(1, @intCast(row), left_buf[0..left_len], style, left_max);
        }
        if (right_x < self.cols) {
            self.screen.putString(
                @intCast(right_x),
                @intCast(row),
                right,
                style,
                self.cols - right_x,
            );
        }
    }

    fn renderBottomLine(self: *Editor) void {
        if (self.rows == 0) return;
        const row = self.commandRow();
        self.screen.fill(0, @intCast(row), self.cols, 1, ' ', self.theme.base());

        if (self.mode == .command) {
            self.screen.put(1, @intCast(row), ':', self.theme.accentStyle(), 1);
            self.screen.putString(2, @intCast(row), self.command_line.items, self.theme.text(), if (self.cols > 2) self.cols - 2 else 0);
            const visual = util.visualColumn(self.command_line.items, self.command_cursor, 1);
            const x = 2 + visual;
            if (x < self.cols) {
                var style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.applyStyle(x, row, style);
            }
            return;
        }

        if (self.pending != .none) {
            var buf: [640]u8 = undefined;
            const text = switch (self.pending) {
                .quit => "Unsaved changes. Quit without saving? (y/N)",
                .open => std.fmt.bufPrint(&buf, "Unsaved changes. Discard and open '{s}'? (y/N)", .{
                    self.pending_path orelse "",
                }) catch "Unsaved changes. Discard? (y/N)",
                .none => "",
            };
            self.screen.putString(1, @intCast(row), text, self.theme.warningStyle(), if (self.cols > 1) self.cols - 1 else 0);
            return;
        }

        if (self.message.items.len > 0) {
            const style = if (self.message_is_error) self.theme.errorStyle() else self.theme.text();
            self.screen.putString(1, @intCast(row), self.message.items, style, if (self.cols > 1) self.cols - 1 else 0);
        }
    }

    fn modeLabel(self: *Editor) []const u8 {
        if (self.pending != .none) return "CONFIRM";
        if (self.find.active) return "FIND";
        if (self.picker.kind != .none) return "PICKER";
        return switch (self.mode) {
            .normal => "NORMAL",
            .insert => "INSERT",
            .select => "SELECT",
            .command => "COMMAND",
        };
    }

    fn statusStyle(self: *Editor) theme_mod.Style {
        if (self.pending != .none) return self.theme.promptStatusStyle();
        return switch (self.mode) {
            .insert => self.theme.insertStatusStyle(),
            .select => self.theme.selectStatusStyle(),
            .command => self.theme.commandStatusStyle(),
            .normal => self.theme.statusStyle(),
        };
    }

    fn renderSpaceMenu(self: *Editor) void {
        if (!self.space_menu_open) return;

        var content_width: usize = 0;
        for (space_items) |item| {
            content_width = @max(content_width, item.label.len + 4);
        }
        var width = @max(14, content_width + 2);
        const height = space_items.len + 2;
        const status_row = self.statusRow();
        if (self.cols < 10 or status_row < height + 1) return;
        if (width > self.cols - 2) width = self.cols - 2;

        const x = if (self.cols >= width + 1) self.cols - width - 1 else 0;
        const y = if (status_row >= height) status_row - height else 0;
        self.drawBox(x, y, width, height, "SPACE");

        for (space_items, 0..) |item, index| {
            const row = y + 1 + index;
            var key_buf: [1]u8 = .{item.key};
            self.screen.putString(@intCast(x + 2), @intCast(row), &key_buf, self.theme.accentStyle(), width - 3);
            self.screen.putString(@intCast(x + 4), @intCast(row), item.label, self.theme.text(), width - 5);
        }
    }

    fn drawBox(self: *Editor, x: usize, y: usize, width: usize, height: usize, title: []const u8) void {
        if (width < 2 or height < 2) return;
        self.screen.fill(@intCast(x), @intCast(y), width, height, ' ', self.theme.base());
        const border = self.theme.borderStyle();

        var column: usize = 1;
        while (column + 1 < width) : (column += 1) {
            self.screen.put(@intCast(x + column), @intCast(y), 0x2500, border, 1);
            self.screen.put(@intCast(x + column), @intCast(y + height - 1), 0x2500, border, 1);
        }
        var row: usize = 1;
        while (row + 1 < height) : (row += 1) {
            self.screen.put(@intCast(x), @intCast(y + row), 0x2502, border, 1);
            self.screen.put(@intCast(x + width - 1), @intCast(y + row), 0x2502, border, 1);
        }
        self.screen.put(@intCast(x), @intCast(y), 0x250C, border, 1);
        self.screen.put(@intCast(x + width - 1), @intCast(y), 0x2510, border, 1);
        self.screen.put(@intCast(x), @intCast(y + height - 1), 0x2514, border, 1);
        self.screen.put(@intCast(x + width - 1), @intCast(y + height - 1), 0x2518, border, 1);

        if (title.len > 0 and width > 6) {
            var title_buf: [128]u8 = undefined;
            const text = std.fmt.bufPrint(&title_buf, " {s} ", .{title}) catch return;
            self.screen.putString(@intCast(x + 2), @intCast(y), text, self.theme.accentStyle(), width - 4);
        }
    }

    fn renderPicker(self: *Editor) void {
        if (self.picker.kind == .none) return;
        const available = self.statusRow();
        const height = self.pickerHeight();
        if (height < 4 or self.cols < 20) return;
        const width = @min(self.cols - 2, 88);
        if (width < 20) return;
        const x = (self.cols - width) / 2;
        const y = if (available >= height) (available - height) / 2 else 0;
        const title = switch (self.picker.kind) {
            .files => "Open file",
            .search => "Search across files",
            .none => "",
        };
        self.drawBox(x, y, width, height, title);

        const prompt_style = self.theme.accentStyle();
        self.screen.put(@intCast(x + 2), @intCast(y + 1), '>', prompt_style, 1);
        const query_width = if (width > 5) width - 5 else 0;
        self.screen.putString(@intCast(x + 4), @intCast(y + 1), self.picker.query.items, self.theme.text(), query_width);
        const query_column = util.visualColumn(self.picker.query.items, self.picker.query.items.len, 1);
        if (query_column < query_width) {
            const cursor_x = x + 4 + query_column;
            if (cursor_x < x + width - 1) {
                var style = self.screen.at(cursor_x, y + 1).style;
                style.reverse = true;
                self.screen.applyStyle(cursor_x, y + 1, style);
            }
        }

        const list_rows = if (height >= 3) height - 3 else 1;
        if (self.picker.selected < self.picker.top) self.picker.top = self.picker.selected;
        if (self.picker.selected >= self.picker.top + list_rows) {
            self.picker.top = self.picker.selected - list_rows + 1;
        }
        const max_top = if (self.picker.items.items.len > list_rows)
            self.picker.items.items.len - list_rows
        else
            0;
        if (self.picker.top > max_top) self.picker.top = max_top;

        var row: usize = 0;
        while (row < list_rows) : (row += 1) {
            const index = self.picker.top + row;
            if (index >= self.picker.items.items.len) break;
            const screen_row = y + 2 + row;
            const style = if (index == self.picker.selected)
                self.theme.selectedItemStyle()
            else
                self.theme.text();
            self.screen.fill(@intCast(x + 1), @intCast(screen_row), width - 2, 1, ' ', style);
            self.screen.putString(@intCast(x + 2), @intCast(screen_row), self.picker.items.items[index].display, style, width - 4);
        }

        var status_buf: [160]u8 = undefined;
        const status = if (self.picker.kind == .search and self.picker.query.items.len == 0)
            "Type to search"
        else if (self.picker.total == 0)
            "No items"
        else
            std.fmt.bufPrint(&status_buf, "{d} item(s)", .{self.picker.total}) catch "items";
        self.screen.putString(@intCast(x + 2), @intCast(y + height - 1), status, self.theme.accentStyle(), width - 4);
    }

    fn renderFind(self: *Editor) void {
        if (!self.find.active) return;
        const available = self.statusRow();
        const height: usize = 4;
        const width = @min(if (self.cols > 2) self.cols - 2 else 0, 72);
        if (available < height + 1 or self.cols < 20 or width < 20) return;
        const x = (self.cols - width) / 2;
        const y = available - height;
        self.drawBox(x, y, width, height, "Find");

        self.screen.put(@intCast(x + 2), @intCast(y + 1), '>', self.theme.accentStyle(), 1);
        const query_width = if (width > 6) width - 6 else 0;
        self.screen.putString(@intCast(x + 4), @intCast(y + 1), self.find.query.items, self.theme.text(), query_width);
        if (self.find.editing) {
            const column = util.visualColumn(self.find.query.items, self.find.query.items.len, 1);
            if (column < query_width) {
                const cursor_x = x + 4 + column;
                if (cursor_x < x + width - 1) {
                    var style = self.screen.at(cursor_x, y + 1).style;
                    style.reverse = true;
                    self.screen.applyStyle(cursor_x, y + 1, style);
                }
            }
        }

        var status_buf: [160]u8 = undefined;
        const status = if (self.find.query.items.len == 0)
            "Type to search"
        else if (self.find.matches.items.len == 0)
            "No matches"
        else if (self.find.index >= 0)
            std.fmt.bufPrint(&status_buf, "{d}/{d} matches  Enter: navigate  n/p: next/previous  Esc: cancel", .{
                @as(usize, @intCast(self.find.index)) + 1,
                self.find.matches.items.len,
            }) catch "matches"
        else
            "matches";
        self.screen.putString(@intCast(x + 2), @intCast(y + 2), status, self.theme.accentStyle(), width - 4);
    }

    // -----------------------------------------------------------------------
    // SMALL HELPERS
    // -----------------------------------------------------------------------

    fn activeLanguage(self: *Editor) ?syntax.Language {
        if (self.language_override) |id| {
            if (syntax.Language.fromId(id)) |lang| return lang;
        }
        if (self.doc.path) |path| return syntax.Language.fromPath(path);
        return null;
    }

    fn appendMessageFmt(self: *Editor, comptime fmt: []const u8, args: anytype) void {
        var buf: [1024]u8 = undefined;
        const text = std.fmt.bufPrint(&buf, fmt, args) catch return;
        self.message.appendSlice(self.allocator, text) catch {};
    }

    fn setMessage(self: *Editor, text: []const u8) void {
        self.message.clearRetainingCapacity();
        self.message.appendSlice(self.allocator, text) catch {};
        self.message_is_error = false;
    }

    fn setMessageFmt(self: *Editor, comptime fmt: []const u8, args: anytype) void {
        var buf: [1024]u8 = undefined;
        const text = std.fmt.bufPrint(&buf, fmt, args) catch return;
        self.setMessage(text);
    }

    fn setError(self: *Editor, text: []const u8) void {
        self.message.clearRetainingCapacity();
        if (!std.mem.startsWith(u8, text, "E: ")) {
            self.message.appendSlice(self.allocator, "E: ") catch {};
        }
        self.message.appendSlice(self.allocator, text) catch {};
        self.message_is_error = true;
    }

    fn setErrorFmt(self: *Editor, comptime fmt: []const u8, args: anytype) void {
        var buf: [1024]u8 = undefined;
        const text = std.fmt.bufPrint(&buf, fmt, args) catch return;
        self.setError(text);
    }

    fn clearMessage(self: *Editor) void {
        self.message.clearRetainingCapacity();
        self.message_is_error = false;
    }
};
