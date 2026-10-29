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
const sidebar_mod = @import("sidebar.zig");
const config_mod = @import("config.zig");
const help_mod = @import("help.zig");
const regex_mod = @import("regex.zig");

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
    delete,
    diff,
};

const SidebarSource = struct {
    path: []u8,
    is_directory: bool,
};

const SidebarPromptKind = enum {
    new_folder,
    new_file,
    rename,
    copy,
    move,
};

const SidebarPrompt = struct {
    label: std.ArrayList(u8) = .empty,
    text: std.ArrayList(u8) = .empty,
    cursor: usize = 0,
    kind: SidebarPromptKind,
    parent: []u8 = &.{},
    old_path: []u8 = &.{},
    is_directory: bool = false,
    sources: std.ArrayList(SidebarSource) = .empty,

    fn deinit(self: *SidebarPrompt, allocator: std.mem.Allocator) void {
        self.label.deinit(allocator);
        self.text.deinit(allocator);
        if (self.parent.len > 0) allocator.free(self.parent);
        if (self.old_path.len > 0) allocator.free(self.old_path);
        for (self.sources.items) |source| allocator.free(source.path);
        self.sources.deinit(allocator);
        self.* = undefined;
    }
};

const PendingDiff = struct {
    text: []u8,
    summary: []u8,
};

const Range = struct {
    start: usize,
    end: usize,
};

const Selection = struct {
    cursor: usize,
    anchor: usize,
    linewise: bool = false,
    preferred_column: ?usize = null,
};

const PaneOrientation = enum { vertical, horizontal };

const PaneNode = union(enum) {
    leaf: u64,
    split: struct {
        orientation: PaneOrientation,
        ratio: f64,
        first: *PaneNode,
        second: *PaneNode,
    },
};

const PaneView = struct {
    id: u64,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
};

const PaneState = struct {
    id: u64,
    mode: Mode = .normal,
    cursor: usize = 0,
    anchor: usize = 0,
    linewise: bool = false,
    preferred_column: ?usize = null,
    top_line: usize = 0,
    left_column: usize = 0,
    pending_g: bool = false,
    extra: std.ArrayList(Selection) = .empty,
};

const PaletteItem = struct {
    title: []u8,
    command: []u8,
};

const PaletteState = struct {
    visible: bool = false,
    query: std.ArrayList(u8) = .empty,
    all: std.ArrayList(PaletteItem) = .empty,
    filtered: std.ArrayList(usize) = .empty,
    selected: usize = 0,
    top: usize = 0,
};

const PluginManagerState = struct {
    visible: bool = false,
    selected: usize = 0,
    top: usize = 0,
    filter: std.ArrayList(u8) = .empty,
    filtering: bool = false,
};

const PickerKind = enum {
    none,
    files,
    search,
    themes,
};

const PickerItem = struct {
    display: []u8,
    file_index: usize,
    line: usize,
    column: usize,
};

const preview_max_lines: usize = 10;
const preview_max_line_length: usize = 400;
const preview_max_bytes: u64 = 4 * 1024 * 1024;
const preview_cache_entries: usize = 128;

const FilePreview = struct {
    path: []u8 = &.{},
    lines: std.ArrayList([]u8) = .empty,
    binary: bool = false,
    truncated: bool = false,
    error_message: ?[]u8 = null,

    fn deinit(self: *FilePreview, allocator: std.mem.Allocator) void {
        if (self.path.len > 0) allocator.free(self.path);
        for (self.lines.items) |line| allocator.free(line);
        self.lines.deinit(allocator);
        if (self.error_message) |message| allocator.free(message);
        self.* = .{};
    }

    fn lineCount(self: FilePreview) usize {
        return self.lines.items.len;
    }
};

/// Owned by `Editor` while a background picker scan is running. The worker
/// writes into `files` and then publishes completion through `done`.
const PickerScanContext = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    files: std.ArrayList([]u8) = .empty,
    cancel: std.atomic.Value(bool) = .init(false),
    done: std.atomic.Value(bool) = .init(false),
    failed: bool = false,
};

/// Owned by `Editor` while a background cross-file search is running.
const PickerSearchContext = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    files: []const []u8,
    query: []u8,
    regex: bool = false,
    case_insensitive: bool = true,
    hits: std.ArrayList(files_mod.SearchHit) = .empty,
    cancel: std.atomic.Value(bool) = .init(false),
    done: std.atomic.Value(bool) = .init(false),
    failed: bool = false,
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
    status: ?[]const u8 = null,
    regex: bool = false,
    replace: bool = false,
    replacement: std.ArrayList(u8) = .empty,
    editing_replacement: bool = false,
    scan_thread: ?std.Thread = null,
    scan_context: ?*PickerScanContext = null,
    search_thread: ?std.Thread = null,
    search_context: ?*PickerSearchContext = null,
};

const FindMatch = struct {
    start: usize,
    end: usize,
};

const FindState = struct {
    active: bool = false,
    editing: bool = true,
    query: std.ArrayList(u8) = .empty,
    replacement: std.ArrayList(u8) = .empty,
    matches: std.ArrayList(FindMatch) = .empty,
    replacements: std.ArrayList([]u8) = .empty,
    index: isize = -1,
    origin: usize = 0,
    multi_cursor: bool = false,
    regex: bool = false,
    replace: bool = false,
    editing_replacement: bool = false,
    error_message: ?[]u8 = null,
};

const FindField = enum { query, replacement };

const SelectionAction = union(enum) {
    move_left,
    move_right,
    move_vertical: i8,
    move_page: i8,
    move_line_start,
    move_line_end,
    document_start,
    document_end,
    word_forward,
    word_backward,
    delete_at_cursor,
    delete_selection,
    insert_char: u21,
    insert_newline,
    insert_indent,
    backspace,
    delete_forward,
    enter_insert,
    enter_insert_from_selection,
    select_all,
    start_char_selection,
    start_line_selection,
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
    theme_registry: theme_mod.Registry = .{ .allocator = undefined },
    syntax_registry: syntax.Registry = .{ .allocator = undefined },
    highlighter: syntax.Highlighter = undefined,

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
    picker_preview: FilePreview = .{},
    picker_preview_cache: std.ArrayList(FilePreview) = .empty,
    sidebar: sidebar_mod.Sidebar,
    sidebar_prompt: ?SidebarPrompt = null,
    pending_sidebar_deletes: std.ArrayList(SidebarSource) = .empty,
    pending_sidebar_diff: ?PendingDiff = null,
    find: FindState = .{},
    extra_selections: std.ArrayList(Selection) = .empty,
    config: config_mod.Config,
    palette: PaletteState = .{},
    plugin_manager: PluginManagerState = .{},
    help: help_mod.HelpState = .{},
    panes: std.ArrayList(PaneState) = .empty,
    pane_root: ?*PaneNode = null,
    active_pane: u64 = 0,
    next_pane_id: u64 = 1,
    pane_screen: scr.Screen = .{},
    theme_before_picker: ?[]u8 = null,
    plugins_loaded: bool = false,
    lua: ?lua_mod.Manager = null,
    workspace_root: []u8 = &.{},
    plugin_panel_id: ?i64 = null,
    plugin_prompt_id: ?i64 = null,
    plugin_prompt_label: std.ArrayList(u8) = .empty,
    plugin_prompt_text: std.ArrayList(u8) = .empty,
    mouse_dragging: bool = false,
    mouse_anchor: usize = 0,

    pub fn init(
        allocator: std.mem.Allocator,
        io: std.Io,
        terminal: *term.Terminal,
        file: ?[]const u8,
        config: config_mod.Config,
    ) !Editor {
        var doc = if (file) |path|
            try document_mod.Document.open(allocator, io, path)
        else
            document_mod.Document.init(allocator, io);
        errdefer doc.deinit();

        var syntax_registry = syntax.Registry.load(allocator, io) catch syntax.Registry.init(allocator);
        errdefer syntax_registry.deinit();
        const preferred_theme = if (config.theme.len > 0) config.theme else "hexal-dark";
        var theme_registry = theme_mod.Registry.load(allocator, io, preferred_theme) catch
            theme_mod.Registry.init(allocator);
        errdefer theme_registry.deinit();
        const highlighter = syntax.Highlighter.init(allocator);
        const workspace_root: []u8 = std.Io.Dir.cwd().realPathFileAlloc(io, ".", allocator) catch
            (allocator.dupe(u8, ".") catch &.{});
        errdefer if (workspace_root.len > 0) allocator.free(workspace_root);
        var sidebar = try sidebar_mod.Sidebar.init(allocator, io, workspace_root);
        errdefer sidebar.deinit();

        // Terminal.init already queried the window size; avoid a second ioctl
        // on the startup path.
        var self = Editor{
            .allocator = allocator,
            .io = io,
            .terminal = terminal,
            .doc = doc,
            .cols = terminal.width,
            .rows = terminal.height,
            .theme = theme_registry.active,
            .theme_registry = theme_registry,
            .syntax_registry = syntax_registry,
            .highlighter = highlighter,
            .workspace_root = workspace_root,
            .sidebar = sidebar,
            .config = config,
        };
        self.sidebar.show_git_ignored = self.config.show_git_ignored;
        self.initializePanes() catch |err| {
            self.deinit();
            return err;
        };
        if (file != null) {
            self.message.clearRetainingCapacity();
            self.appendMessageFmt("Opened {s}", .{self.doc.displayName()});
        }
        return self;
    }

    fn preferredTheme(allocator: std.mem.Allocator, io: std.Io) []const u8 {
        _ = allocator;
        _ = io;
        return "hexal-dark";
    }

    pub fn deinit(self: *Editor) void {
        if (self.lua) |*runtime| runtime.deinit();
        if (self.workspace_root.len > 0) self.allocator.free(self.workspace_root);
        self.plugin_prompt_label.deinit(self.allocator);
        self.plugin_prompt_text.deinit(self.allocator);
        self.message.deinit(self.allocator);
        self.command_line.deinit(self.allocator);
        self.clipboard.deinit(self.allocator);
        self.line_buffer.deinit(self.allocator);
        self.tokens.deinit(self.allocator);
        if (self.pending_path) |path| self.allocator.free(path);
        if (self.language_override) |id| self.allocator.free(id);
        self.jumps.deinit(self.allocator);
        self.find.query.deinit(self.allocator);
        self.find.replacement.deinit(self.allocator);
        self.find.matches.deinit(self.allocator);
        for (self.find.replacements.items) |replacement| self.allocator.free(replacement);
        self.find.replacements.deinit(self.allocator);
        if (self.find.error_message) |message| self.allocator.free(message);
        self.extra_selections.deinit(self.allocator);
        self.palette.query.deinit(self.allocator);
        for (self.palette.all.items) |item| {
            self.allocator.free(item.title);
            self.allocator.free(item.command);
        }
        self.palette.all.deinit(self.allocator);
        self.palette.filtered.deinit(self.allocator);
        self.plugin_manager.filter.deinit(self.allocator);
        for (self.panes.items) |*pane| pane.extra.deinit(self.allocator);
        self.panes.deinit(self.allocator);
        if (self.pane_root) |root| freePaneNode(self.allocator, root);
        self.pane_screen.deinit(self.allocator);
        self.config.deinit();
        if (self.theme_before_picker) |id| self.allocator.free(id);
        self.cancelPickerSearch();
        self.cancelPickerScan();
        self.pickerClearItems();
        self.picker.query.deinit(self.allocator);
        self.picker.replacement.deinit(self.allocator);
        files_mod.clearFiles(self.allocator, &self.picker.files);
        self.picker.files.deinit(self.allocator);
        files_mod.clearHits(self.allocator, &self.picker.hits);
        self.picker.hits.deinit(self.allocator);
        self.picker_preview.deinit(self.allocator);
        for (self.picker_preview_cache.items) |*preview| preview.deinit(self.allocator);
        self.picker_preview_cache.deinit(self.allocator);
        if (self.sidebar_prompt) |*prompt| prompt.deinit(self.allocator);
        for (self.pending_sidebar_deletes.items) |source| self.allocator.free(source.path);
        self.pending_sidebar_deletes.deinit(self.allocator);
        if (self.pending_sidebar_diff) |diff| {
            self.allocator.free(diff.text);
            self.allocator.free(diff.summary);
        }
        self.sidebar.deinit();
        self.renderer.deinit(self.allocator);
        self.screen.deinit(self.allocator);
        self.highlighter.deinit();
        self.syntax_registry.deinit();
        self.theme_registry.deinit();
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

        // Load plugins after the first frame, mirroring the Rust editor. A
        // lazy language plugin may mirror the whole buffer into Lua; that cost
        // belongs after the visible startup, not on the first paint.
        if (!self.plugins_loaded) {
            self.plugins_loaded = true;
            self.loadPlugins() catch |err| {
                self.setErrorFmt("lua plugin load: {s}", .{@errorName(err)});
            };
        }

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

            const maybe_event = self.terminal.readEvent(30) catch break;
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

            try self.pumpPickerWork();
            try self.pumpPlugins();
            try self.render();
        }
    }

    fn handleEvent(self: *Editor, event: term.Event) !void {
        if (self.palette.visible) {
            try self.handlePalette(event);
            return;
        }
        if (self.plugin_manager.visible) {
            try self.handlePluginManager(event);
            return;
        }
        if (self.help.visible) {
            _ = self.help.handle(event, self.cols, self.rows, 3);
            return;
        }
        if (event.key == .character and event.ctrl and (event.ch == 'p' or event.ch == 'P')) {
            try self.openPalette();
            return;
        }
        if (self.plugin_prompt_id != null) {
            try self.handlePluginPrompt(event);
            return;
        }
        if (self.plugin_panel_id != null) {
            try self.handlePluginPanelKey(event);
            return;
        }
        if (event.key == .character and event.ctrl and (event.ch == 'b' or event.ch == 'B')) {
            self.toggleSidebar();
            return;
        }
        if (self.sidebar.visible and event.mouse_x < self.sidebar.width(self.cols) and
            (event.key == .mouse_down or event.key == .scroll_up or event.key == .scroll_down))
        {
            self.handleSidebarMouse(event);
            return;
        }
        if (event.key == .scroll_up) {
            const amount: usize = 3;
            self.top_line = self.top_line -| amount;
            return;
        }
        if (event.key == .scroll_down) {
            const amount: usize = 3;
            const last_line = self.doc.lineCount() - 1;
            self.top_line = @min(self.top_line + amount, last_line);
            return;
        }
        if (event.key == .mouse_down or event.key == .mouse_move or event.key == .mouse_up) {
            try self.handleMouse(event);
            return;
        }
        if (self.picker.kind != .none) {
            try self.handlePicker(event);
            return;
        }
        if (self.sidebar_prompt != null) {
            try self.handleSidebarPrompt(event);
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
        if (self.sidebar.visible and self.sidebar.focused) {
            try self.handleSidebar(event);
            return;
        }
        switch (self.mode) {
            .command => try self.handleCommandInput(event),
            .insert => try self.handleInsert(event),
            .select => try self.handleSelect(event),
            .normal => try self.handleNormal(event),
        }
    }

    fn handleMouse(self: *Editor, event: term.Event) !void {
        if (self.rows == 0 or self.cols == 0) return;
        const text_rows = self.textRows();
        if (event.mouse_y >= text_rows) return;
        const gutter = self.gutterWidth();
        if (event.key == .mouse_down and event.mouse_x < gutter) return;
        const line = self.top_line + event.mouse_y;
        if (line >= self.doc.lineCount()) return;
        const text = self.doc.lineText(line, &self.line_buffer) catch return;
        var visual = self.left_column;
        if (event.mouse_x >= gutter) visual += event.mouse_x - gutter;
        const local = util.byteOffsetForVisualColumn(text, visual, TabWidth);
        const offset = @min(self.doc.lineStart(line) + local, self.doc.size());

        switch (event.key) {
            .mouse_down => {
                self.mouse_dragging = true;
                self.mouse_anchor = offset;
                self.anchor = offset;
                self.cursor = offset;
                self.selection_linewise = false;
                self.mode = .select;
                self.preferred_column = null;
                self.doc.breakUndoGroup();
                self.snapCursor();
            },
            .mouse_move => {
                if (!self.mouse_dragging) return;
                self.cursor = offset;
                self.anchor = self.mouse_anchor;
                self.selection_linewise = false;
                self.mode = .select;
                self.preferred_column = null;
                self.snapCursor();
            },
            .mouse_up => {
                if (!self.mouse_dragging) return;
                self.mouse_dragging = false;
                self.cursor = offset;
                self.anchor = self.mouse_anchor;
                if (self.cursor == self.anchor) {
                    self.cancelSelection();
                } else {
                    self.selection_linewise = false;
                    self.mode = .select;
                }
                self.snapCursor();
            },
            else => {},
        }
    }

    fn panelKeyName(event: term.Event, buffer: *[32]u8) []const u8 {
        var length: usize = 0;
        if (event.ctrl) {
            @memcpy(buffer[0..5], "ctrl+");
            length = 5;
        } else if (event.alt) {
            @memcpy(buffer[0..4], "alt+");
            length = 4;
        } else if (event.shift) {
            @memcpy(buffer[0..6], "shift+");
            length = 6;
        }
        const name: []const u8 = switch (event.key) {
            .enter => "enter",
            .escape => "esc",
            .backspace => "backspace",
            .delete => "delete",
            .tab => "tab",
            .backtab => "backtab",
            .up => "up",
            .down => "down",
            .left => "left",
            .right => "right",
            .home => "home",
            .end => "end",
            .page_up => "pageup",
            .page_down => "pagedown",
            .character => {
                var encoded: [4]u8 = undefined;
                const count = std.unicode.utf8Encode(event.ch, &encoded) catch return buffer[0..0];
                @memcpy(buffer[length .. length + count], encoded[0..count]);
                return buffer[0 .. length + count];
            },
            else => return buffer[0..0],
        };
        @memcpy(buffer[length .. length + name.len], name);
        return buffer[0 .. length + name.len];
    }

    fn handlePluginPanelKey(self: *Editor, event: term.Event) !void {
        const manager = if (self.lua) |*value| value else {
            self.plugin_panel_id = null;
            return;
        };
        const id = self.plugin_panel_id orelse return;
        if (manager.findPanel(id) == null) {
            self.plugin_panel_id = null;
            return;
        }
        var buffer: [32]u8 = undefined;
        const key = panelKeyName(event, &buffer);
        if (key.len == 0) return;
        const selected = manager.findPanel(id).?.selected;
        const handled = manager.handlePanelKey(id, key, selected, &self.doc, &self.cursor) catch |err| blk: {
            self.setErrorFmt("plugin panel: {s}", .{@errorName(err)});
            break :blk null;
        };
        if (handled orelse false) {
            try self.flushPluginEffects();
            if (manager.findPanel(id) == null) self.plugin_panel_id = null;
            return;
        }
        if (std.mem.eql(u8, key, "q") or std.mem.eql(u8, key, "esc")) {
            manager.closePanel(id);
            self.plugin_panel_id = null;
            self.clearMessage();
            return;
        }
        if (std.mem.eql(u8, key, "j") or std.mem.eql(u8, key, "down")) {
            self.movePluginPanelSelection(1);
        } else if (std.mem.eql(u8, key, "k") or std.mem.eql(u8, key, "up")) {
            self.movePluginPanelSelection(-1);
        } else if (std.mem.eql(u8, key, "pageup")) {
            self.movePluginPanelSelection(-10);
        } else if (std.mem.eql(u8, key, "pagedown")) {
            self.movePluginPanelSelection(10);
        } else if (std.mem.eql(u8, key, "home")) {
            self.setPluginPanelSelection(0);
        } else if (std.mem.eql(u8, key, "end")) {
            if (manager.findPanel(id)) |panel| self.setPluginPanelSelection(panel.lines.items.len -| 1);
        }
        try self.flushPluginEffects();
    }

    fn movePluginPanelSelection(self: *Editor, delta: isize) void {
        const manager = if (self.lua) |*value| value else return;
        const id = self.plugin_panel_id orelse return;
        const panel = manager.findPanel(id) orelse return;
        if (panel.lines.items.len == 0) return;
        const selectable = panel.selectable;
        var index: isize = @intCast(panel.selected);
        const step: isize = if (delta < 0) -1 else 1;
        var remaining: usize = @intCast(@abs(delta));
        while (remaining > 0) {
            var candidate = index + step;
            if (candidate < 0 or candidate >= @as(isize, @intCast(panel.lines.items.len))) break;
            while (selectable and !panel.lines.items[@intCast(candidate)].selectable) {
                candidate += step;
                if (candidate < 0 or candidate >= @as(isize, @intCast(panel.lines.items.len))) break;
            }
            if (candidate < 0 or candidate >= @as(isize, @intCast(panel.lines.items.len))) break;
            index = candidate;
            remaining -= 1;
        }
        panel.selected = @intCast(index);
    }

    fn setPluginPanelSelection(self: *Editor, index: usize) void {
        const manager = if (self.lua) |*value| value else return;
        const id = self.plugin_panel_id orelse return;
        const panel = manager.findPanel(id) orelse return;
        panel.selected = @min(index, panel.lines.items.len -| 1);
    }

    fn handlePluginPrompt(self: *Editor, event: term.Event) !void {
        const id = self.plugin_prompt_id orelse return;
        if (event.key == .escape) {
            self.finishPluginPrompt(id, null) catch {};
            return;
        }
        if (event.key == .enter) {
            const value = try self.allocator.dupe(u8, self.plugin_prompt_text.items);
            defer self.allocator.free(value);
            self.finishPluginPrompt(id, value) catch {};
            return;
        }
        if (event.key == .backspace) {
            if (self.plugin_prompt_text.items.len > 0) {
                const previous = util.previousBoundary(self.plugin_prompt_text.items, self.plugin_prompt_text.items.len);
                self.plugin_prompt_text.items.len = previous;
            }
            return;
        }
        if (event.key == .character and !event.ctrl and !event.alt) {
            var buffer: [4]u8 = undefined;
            const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
            try self.plugin_prompt_text.appendSlice(self.allocator, buffer[0..length]);
        }
    }

    fn finishPluginPrompt(self: *Editor, id: i64, value: ?[]const u8) !void {
        const manager = if (self.lua) |*value_manager| value_manager else return;
        self.plugin_prompt_id = null;
        self.plugin_prompt_label.clearRetainingCapacity();
        self.plugin_prompt_text.clearRetainingCapacity();
        if (self.mode == .command) self.mode = .normal;
        _ = manager.resolvePrompt(id, value);
        self.updatePluginContext();
        try self.flushPluginEffects();
    }

    // -----------------------------------------------------------------------
    // SIDEBAR
    // -----------------------------------------------------------------------

    fn toggleSidebar(self: *Editor) void {
        self.sidebar.setRoot(self.workspace_root) catch {};
        const select = self.docAbsolutePath();
        defer if (select) |path| self.allocator.free(path);
        self.sidebar.toggle(select);
        if (self.sidebar.visible) {
            self.sidebar.menu_visible = false;
            self.mode = .normal;
            self.setMessage("Sidebar focused (Ctrl+B to close)");
        } else {
            self.clearMessage();
        }
    }

    fn docAbsolutePath(self: *Editor) ?[]u8 {
        const path = self.doc.path orelse return null;
        return std.Io.Dir.cwd().realPathFileAlloc(self.io, path, self.allocator) catch
            (self.allocator.dupe(u8, path) catch null);
    }

    fn handleSidebar(self: *Editor, event: term.Event) !void {
        if (self.sidebar.menu_visible) {
            self.handleSidebarMenu(event);
            return;
        }
        if (event.key == .left and event.ctrl) {
            self.sidebar.resize(self.cols, -2);
            self.setMessageFmt("Sidebar width: {d}", .{self.sidebar.width(self.cols)});
            return;
        }
        if (event.key == .right and event.ctrl) {
            self.sidebar.resize(self.cols, 2);
            self.setMessageFmt("Sidebar width: {d}", .{self.sidebar.width(self.cols)});
            return;
        }

        const rows = self.textRows();
        switch (event.key) {
            .escape => {
                if (self.sidebar.selectionCount() > 1) {
                    self.sidebar.anchor = self.sidebar.selected;
                } else {
                    self.sidebar.focused = false;
                    self.clearMessage();
                }
            },
            .tab => self.sidebar.moveSelection(1, true, false),
            .backtab => self.sidebar.moveSelection(-1, true, false),
            .up => self.sidebar.moveSelection(-1, false, event.shift),
            .down => self.sidebar.moveSelection(1, false, event.shift),
            .page_up => self.sidebar.moveSelection(-@as(isize, @intCast(rows)), false, event.shift),
            .page_down => self.sidebar.moveSelection(@intCast(rows), false, event.shift),
            .home => self.sidebar.setSelection(0, event.shift),
            .end => self.sidebar.setSelection(self.sidebar.nodes.items.len -| 1, event.shift),
            .left => self.sidebar.collapseOrSelectParent(),
            .right => self.sidebar.expandOrSelectChild(),
            .enter => try self.activateSidebarNode(),
            .character => {
                if (event.ctrl or event.alt) return;
                switch (event.ch) {
                    'c', 'C' => self.openSidebarContextMenu(),
                    ' ' => {
                        self.sidebar.focused = false;
                        self.openSpaceMenu();
                    },
                    else => {},
                }
            },
            else => {},
        }
        self.sidebar.updateTop(rows);
    }

    fn activateSidebarNode(self: *Editor) !void {
        const node = self.sidebar.selectedNode() orelse return;
        if (node.is_directory) {
            self.sidebar.toggleDirectory();
            return;
        }
        const path = try self.allocator.dupe(u8, node.path);
        defer self.allocator.free(path);
        self.sidebar.menu_visible = false;
        try self.requestOpen(path, 0, 0, false);
    }

    fn openSidebarContextMenu(self: *Editor) void {
        if (self.sidebar.selectedNode() == null) return;
        self.sidebar.menu_visible = true;
        self.sidebar.menu_selected = 0;
        self.sidebar.focused = true;
        self.clearMessage();
    }

    fn handleSidebarMenu(self: *Editor, event: term.Event) void {
        const items = self.sidebar.menuItems();
        if (items.len == 0) return;
        switch (event.key) {
            .escape => {
                self.sidebar.menu_visible = false;
                return;
            },
            .tab => {
                self.sidebar.menu_selected = (self.sidebar.menu_selected + 1) % items.len;
                return;
            },
            .backtab => {
                self.sidebar.menu_selected = (self.sidebar.menu_selected + items.len - 1) % items.len;
                return;
            },
            .up => {
                self.sidebar.menu_selected -|= 1;
                return;
            },
            .down => {
                self.sidebar.menu_selected = @min(self.sidebar.menu_selected + 1, items.len - 1);
                return;
            },
            .enter => {
                self.activateSidebarMenuItem();
                return;
            },
            .character => {},
            else => return,
        }
        if (event.ctrl or event.alt) return;
        if (event.ch == ' ') {
            self.sidebar.menu_visible = false;
            self.sidebar.focused = false;
            self.openSpaceMenu();
            return;
        }
        if (self.sidebar.menuHotkey(@intCast(event.ch))) |index| {
            self.sidebar.menu_selected = index;
            self.activateSidebarMenuItem();
        } else if (event.ch == 'c' or event.ch == 'C') {
            self.sidebar.menu_visible = false;
        }
    }

    fn activateSidebarMenuItem(self: *Editor) void {
        const items = self.sidebar.menuItems();
        if (self.sidebar.menu_selected >= items.len) {
            self.sidebar.menu_visible = false;
            return;
        }
        const action = items[self.sidebar.menu_selected].action;
        self.sidebar.menu_visible = false;
        self.sidebar.focused = true;
        self.runSidebarAction(action) catch |err| {
            self.setErrorFmt("sidebar: {s}", .{@errorName(err)});
        };
    }

    fn collectSidebarSources(self: *Editor) !std.ArrayList(SidebarSource) {
        var nodes = try self.sidebar.effectiveSelection(self.allocator);
        defer nodes.deinit(self.allocator);
        var sources: std.ArrayList(SidebarSource) = .empty;
        errdefer self.freeSidebarSources(&sources);
        for (nodes.items) |node| {
            try sources.append(self.allocator, .{
                .path = try self.allocator.dupe(u8, node.path),
                .is_directory = node.is_directory,
            });
        }
        return sources;
    }

    fn freeSidebarSources(self: *Editor, sources: *std.ArrayList(SidebarSource)) void {
        for (sources.items) |source| self.allocator.free(source.path);
        sources.clearRetainingCapacity();
    }

    fn runSidebarAction(self: *Editor, action: sidebar_mod.Action) !void {
        const node = self.sidebar.selectedNode() orelse return;
        switch (action) {
            .open => try self.activateSidebarNode(),
            .new_folder, .new_file => {
                const parent = if (node.is_directory)
                    try self.allocator.dupe(u8, node.path)
                else
                    try self.allocator.dupe(u8, std.fs.path.dirname(node.path) orelse self.workspace_root);
                defer self.allocator.free(parent);
                if (action == .new_folder) {
                    try self.startSidebarPrompt(.new_folder, "New folder name: ", "", parent, "", false);
                } else {
                    try self.startSidebarPrompt(.new_file, "New file name: ", "", parent, "", false);
                }
            },
            .rename => {
                var label_buf: [512]u8 = undefined;
                const label = try std.fmt.bufPrint(&label_buf, "Rename {s}: ", .{node.name});
                try self.startSidebarPrompt(.rename, label, node.name, "", node.path, node.is_directory);
            },
            .duplicate => {
                var sources = try self.collectSidebarSources();
                defer self.freeSidebarSources(&sources);
                self.duplicateSidebarSources(sources.items);
            },
            .format => {
                var sources = try self.collectSidebarSources();
                defer self.freeSidebarSources(&sources);
                self.formatSidebarSources(sources.items);
            },
            .diff => {
                var sources = try self.collectSidebarSources();
                defer self.freeSidebarSources(&sources);
                self.beginSidebarDiff(sources.items);
            },
            .copy, .move => {
                var sources = try self.collectSidebarSources();
                errdefer self.freeSidebarSources(&sources);
                const moving = action == .move;
                const initial = if (!moving and sources.items.len == 1)
                    try uniqueCopyPath(self.allocator, self.io, sources.items[0].path, sources.items[0].is_directory)
                else
                    try self.allocator.dupe(u8, "");
                defer self.allocator.free(initial);
                const label = if (moving) "Move to: " else "Copy to: ";
                try self.startSidebarPrompt(
                    if (moving) .move else .copy,
                    label,
                    initial,
                    "",
                    "",
                    false,
                );
                if (self.sidebar_prompt) |*prompt| {
                    prompt.sources = sources;
                    sources = .empty;
                }
            },
            .delete => {
                var sources = try self.collectSidebarSources();
                defer self.freeSidebarSources(&sources);
                try self.requestDeleteSidebarSources(sources.items);
            },
        }
    }

    fn startSidebarPrompt(
        self: *Editor,
        kind: SidebarPromptKind,
        label: []const u8,
        initial: []const u8,
        parent: []const u8,
        old_path: []const u8,
        is_directory: bool,
    ) !void {
        if (self.sidebar_prompt) |*existing| {
            existing.deinit(self.allocator);
            self.sidebar_prompt = null;
        }
        var prompt = SidebarPrompt{ .kind = kind, .is_directory = is_directory };
        errdefer prompt.deinit(self.allocator);
        try prompt.label.appendSlice(self.allocator, label);
        try prompt.text.appendSlice(self.allocator, initial);
        prompt.cursor = prompt.text.items.len;
        if (parent.len > 0) prompt.parent = try self.allocator.dupe(u8, parent);
        if (old_path.len > 0) prompt.old_path = try self.allocator.dupe(u8, old_path);
        self.sidebar_prompt = prompt;
        self.sidebar.focused = self.sidebar.visible;
        self.clearMessage();
    }

    fn handleSidebarPrompt(self: *Editor, event: term.Event) !void {
        const prompt = if (self.sidebar_prompt) |*value| value else return;
        switch (event.key) {
            .escape => {
                var finished = prompt.*;
                self.sidebar_prompt = null;
                finished.deinit(self.allocator);
                self.sidebar.focused = self.sidebar.visible;
                self.setMessage("Cancelled.");
            },
            .enter => {
                const text = try self.allocator.dupe(u8, std.mem.trim(u8, prompt.text.items, " \t\r\n"));
                defer self.allocator.free(text);
                var finished = prompt.*;
                self.sidebar_prompt = null;
                self.sidebar.focused = self.sidebar.visible;
                self.completeSidebarPrompt(&finished, text);
                finished.deinit(self.allocator);
            },
            .backspace => {
                if (prompt.cursor > 0) {
                    const previous = util.previousBoundary(prompt.text.items, prompt.cursor);
                    std.mem.copyForwards(u8, prompt.text.items[previous..], prompt.text.items[prompt.cursor..]);
                    const removed = prompt.cursor - previous;
                    prompt.text.items.len -= removed;
                    prompt.cursor = previous;
                }
            },
            .delete => {
                if (prompt.cursor < prompt.text.items.len) {
                    const next = util.nextBoundary(prompt.text.items, prompt.cursor);
                    std.mem.copyForwards(u8, prompt.text.items[prompt.cursor..], prompt.text.items[next..]);
                    prompt.text.items.len -= (next - prompt.cursor);
                }
            },
            .left => prompt.cursor = util.previousBoundary(prompt.text.items, prompt.cursor),
            .right => prompt.cursor = util.nextBoundary(prompt.text.items, prompt.cursor),
            .home => prompt.cursor = 0,
            .end => prompt.cursor = prompt.text.items.len,
            .character => {
                if (event.ctrl or event.alt) return;
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try prompt.text.insertSlice(self.allocator, prompt.cursor, buffer[0..length]);
                prompt.cursor += length;
            },
            else => {},
        }
    }

    fn completeSidebarPrompt(self: *Editor, prompt: *SidebarPrompt, text: []const u8) void {
        switch (prompt.kind) {
            .new_folder, .new_file => {
                if (text.len == 0 or !validName(text)) {
                    self.setError("Invalid name.");
                    self.sidebar.refresh(null);
                    return;
                }
                const target = std.fs.path.join(self.allocator, &.{ prompt.parent, text }) catch {
                    self.setError("Invalid path.");
                    return;
                };
                defer self.allocator.free(target);
                if (pathExists(self.io, target)) {
                    self.setErrorFmt("'{s}' already exists.", .{text});
                    self.sidebar.refresh(null);
                    return;
                }
                if (prompt.kind == .new_folder) {
                    std.Io.Dir.cwd().createDir(self.io, target, .default_dir) catch |err| {
                        self.setErrorFmt("Create folder failed: {s}", .{@errorName(err)});
                        self.sidebar.refresh(null);
                        return;
                    };
                    self.setMessageFmt("Created folder {s}", .{text});
                } else {
                    var file = std.Io.Dir.cwd().createFile(self.io, target, .{
                        .exclusive = true,
                        .truncate = false,
                    }) catch |err| {
                        self.setErrorFmt("Create file failed: {s}", .{@errorName(err)});
                        self.sidebar.refresh(null);
                        return;
                    };
                    file.close(self.io);
                    self.setMessageFmt("Created file {s}", .{text});
                }
                self.sidebar.expandTo(target);
                self.sidebar.refresh(target);
            },
            .rename => {
                if (text.len == 0 or !validName(text)) {
                    self.setError("Invalid file or folder name.");
                    self.sidebar.refresh(prompt.old_path);
                    return;
                }
                const parent = std.fs.path.dirname(prompt.old_path) orelse self.workspace_root;
                const target = std.fs.path.join(self.allocator, &.{ parent, text }) catch {
                    self.setError("Invalid path.");
                    return;
                };
                defer self.allocator.free(target);
                if (std.mem.eql(u8, target, prompt.old_path)) {
                    self.setMessage("Name unchanged.");
                    self.sidebar.refresh(prompt.old_path);
                    return;
                }
                if (pathExists(self.io, target)) {
                    self.setErrorFmt("'{s}' already exists.", .{text});
                    self.sidebar.refresh(prompt.old_path);
                    return;
                }
                std.Io.Dir.renameAbsolute(prompt.old_path, target, self.io) catch |err| {
                    self.setErrorFmt("Rename failed: {s}", .{@errorName(err)});
                    self.sidebar.refresh(prompt.old_path);
                    return;
                };
                if (prompt.is_directory) {
                    self.sidebar.updateExpandedAfterRename(prompt.old_path, target);
                }
                self.renameOpenDocument(prompt.old_path, target, prompt.is_directory);
                self.sidebar.expandTo(target);
                self.sidebar.refresh(target);
                self.setMessageFmt("Renamed to {s}", .{text});
            },
            .copy, .move => {
                if (text.len == 0) {
                    self.setMessage("Cancelled.");
                    self.sidebar.refresh(null);
                    return;
                }
                if (prompt.sources.items.len == 0) return;
                var destination = text;
                const owned_destination = if (!std.fs.path.isAbsolute(destination))
                    std.fs.path.join(self.allocator, &.{ self.workspace_root, destination }) catch null
                else
                    null;
                defer if (owned_destination) |value| self.allocator.free(value);
                if (owned_destination) |value| destination = value;

                const destination_is_dir = isDirectory(self.io, destination);
                if (!destination_is_dir and prompt.sources.items.len > 1) {
                    self.setError("Destination folder does not exist.");
                    self.sidebar.refresh(null);
                    return;
                }

                var done: usize = 0;
                var last_target: ?[]u8 = null;
                defer if (last_target) |value| self.allocator.free(value);
                var first_error: ?[]const u8 = null;
                for (prompt.sources.items) |source| {
                    const target = if (destination_is_dir or prompt.sources.items.len > 1)
                        std.fs.path.join(self.allocator, &.{ destination, sidebar_mod.fileName(source.path) }) catch continue
                    else
                        self.allocator.dupe(u8, destination) catch continue;
                    defer self.allocator.free(target);
                    if (pathExists(self.io, target)) {
                        if (first_error == null) first_error = "destination already exists";
                        continue;
                    }
                    if (prompt.kind == .move) {
                        if (source.is_directory and
                            (std.mem.eql(u8, destination, source.path) or sidebar_mod.isInside(source.path, destination)))
                        {
                            if (first_error == null) first_error = "cannot move a folder inside itself";
                            continue;
                        }
                        std.Io.Dir.renameAbsolute(source.path, target, self.io) catch |err| {
                            if (first_error == null) first_error = @errorName(err);
                            continue;
                        };
                        if (source.is_directory) self.sidebar.updateExpandedAfterRename(source.path, target);
                        self.renameOpenDocument(source.path, target, source.is_directory);
                    } else {
                        copyPath(self.allocator, self.io, source.path, target, source.is_directory) catch |err| {
                            if (first_error == null) first_error = @errorName(err);
                            continue;
                        };
                    }
                    done += 1;
                    if (last_target) |old| self.allocator.free(old);
                    last_target = self.allocator.dupe(u8, target) catch null;
                }
                if (last_target) |value| {
                    self.sidebar.expandTo(value);
                    self.sidebar.refresh(value);
                } else {
                    self.sidebar.refresh(null);
                }
                if (first_error) |message| {
                    self.setErrorFmt("Completed {d} item(s); {s}", .{ done, message });
                } else {
                    self.setMessageFmt("Completed {d} item(s)", .{done});
                }
            },
        }
    }

    fn renameOpenDocument(self: *Editor, old_path: []const u8, new_path: []const u8, is_directory: bool) void {
        const current = self.doc.path orelse return;
        const absolute = std.Io.Dir.cwd().realPathFileAlloc(self.io, current, self.allocator) catch
            (self.allocator.dupe(u8, current) catch return);
        defer self.allocator.free(absolute);
        var updated: ?[]u8 = null;
        if (std.mem.eql(u8, absolute, old_path)) {
            updated = self.allocator.dupe(u8, new_path) catch null;
        } else if (is_directory and sidebar_mod.isInside(old_path, absolute)) {
            updated = std.fs.path.join(self.allocator, &.{
                new_path,
                std.mem.trimStart(u8, absolute[old_path.len..], "/\\"),
            }) catch null;
        }
        if (updated) |path| {
            self.allocator.free(current);
            self.doc.path = path;
            self.highlighter.clear();
        }
    }

    fn duplicateSidebarSources(self: *Editor, sources: []const SidebarSource) void {
        if (sources.len == 0) return;
        var duplicated: usize = 0;
        var first_error: ?[]const u8 = null;
        var last_target: ?[]u8 = null;
        defer if (last_target) |value| self.allocator.free(value);
        for (sources) |source| {
            const target = uniqueCopyPath(self.allocator, self.io, source.path, source.is_directory) catch continue;
            defer self.allocator.free(target);
            copyPath(self.allocator, self.io, source.path, target, source.is_directory) catch |err| {
                if (first_error == null) first_error = @errorName(err);
                continue;
            };
            duplicated += 1;
            if (last_target) |old| self.allocator.free(old);
            last_target = self.allocator.dupe(u8, target) catch null;
        }
        if (last_target) |value| {
            self.sidebar.expandTo(value);
            self.sidebar.refresh(value);
        } else {
            self.sidebar.refresh(null);
        }
        if (first_error) |message| {
            self.setErrorFmt("Duplicated {d} of {d} items; {s}", .{ duplicated, sources.len, message });
        } else if (duplicated == 1) {
            self.setMessageFmt("Duplicated as {s}", .{sidebar_mod.fileName(last_target orelse "")});
        } else {
            self.setMessageFmt("Duplicated {d} items", .{duplicated});
        }
    }

    fn formatSidebarSources(self: *Editor, sources: []const SidebarSource) void {
        if (sources.len == 0) return;
        var files: std.ArrayList([]u8) = .empty;
        defer {
            for (files.items) |path| self.allocator.free(path);
            files.deinit(self.allocator);
        }
        for (sources) |source| {
            if (source.is_directory) {
                collectFiles(self.allocator, self.io, source.path, &files);
            } else if (pathExists(self.io, source.path)) {
                files.append(self.allocator, self.allocator.dupe(u8, source.path) catch continue) catch continue;
            }
        }
        const open_path = self.docAbsolutePath();
        defer if (open_path) |path| self.allocator.free(path);

        var formatted: usize = 0;
        var changed: usize = 0;
        var failed: usize = 0;
        var skipped: usize = 0;
        var first_error: ?[]const u8 = null;
        for (files.items) |path| {
            if (open_path) |current| {
                if (std.mem.eql(u8, current, path)) {
                    skipped += 1;
                    continue;
                }
            }
            const language = self.syntax_registry.findByPath(path) orelse {
                skipped += 1;
                continue;
            };
            const formatter = language.resolveFormatter() orelse {
                skipped += 1;
                continue;
            };
            const input = std.Io.Dir.cwd().readFileAlloc(self.io, path, self.allocator, .limited(128 * 1024 * 1024)) catch {
                failed += 1;
                if (first_error == null) first_error = @errorName(error.ReadFailed);
                continue;
            };
            defer self.allocator.free(input);
            const output = self.runFormatter(&formatter, input, path) catch {
                failed += 1;
                if (first_error == null) first_error = "formatter failed";
                continue;
            };
            defer self.allocator.free(output);
            formatted += 1;
            if (!std.mem.eql(u8, input, output)) {
                std.Io.Dir.cwd().writeFile(self.io, .{ .sub_path = path, .data = output }) catch {
                    failed += 1;
                    continue;
                };
                changed += 1;
            }
        }
        self.sidebar.refresh(null);
        if (first_error) |message| {
            self.setErrorFmt("Formatted {d} file(s), {d} failed: {s}", .{ formatted, failed, message });
        } else if (formatted == 0) {
            self.setMessage("No files were formatted.");
        } else {
            self.setMessageFmt("Formatted {d} file(s), {d} changed, {d} skipped", .{ formatted, changed, skipped });
        }
    }

    fn beginSidebarDiff(self: *Editor, sources: []const SidebarSource) void {
        if (sources.len != 2 or sources[0].is_directory or sources[1].is_directory) {
            self.setError("Diff requires exactly two files.");
            return;
        }
        const text = self.createDiff(sources[0].path, sources[1].path) catch {
            self.setError("Cannot create diff.");
            return;
        };
        const summary = std.fmt.allocPrint(self.allocator, "{s} vs {s}", .{
            sidebar_mod.fileName(sources[0].path),
            sidebar_mod.fileName(sources[1].path),
        }) catch {
            self.allocator.free(text);
            self.setError("Out of memory.");
            return;
        };
        if (self.doc.isDirty()) {
            self.pending_sidebar_diff = .{ .text = text, .summary = summary };
            self.pending = .diff;
            self.clearMessage();
            return;
        }
        self.showSidebarDiff(text, summary);
    }

    fn createDiff(self: *Editor, first: []const u8, second: []const u8) ![]u8 {
        var child = try std.process.spawn(self.io, .{
            .argv = &.{ "diff", "-u", "--label", sidebar_mod.fileName(first), "--label", sidebar_mod.fileName(second), first, second },
            .stdout = .pipe,
            .stderr = .pipe,
        });
        defer child.kill(self.io);
        var buffer: [4096]u8 = undefined;
        var output: []u8 = &.{};
        if (child.stdout) |file| {
            var reader = file.reader(self.io, &buffer);
            output = try reader.interface.allocRemaining(self.allocator, .limited(16 * 1024 * 1024));
            file.close(self.io);
            child.stdout = null;
        }
        if (child.stderr) |file| {
            file.close(self.io);
            child.stderr = null;
        }
        _ = child.wait(self.io) catch {};
        return output;
    }

    fn showSidebarDiff(self: *Editor, diff_text: []u8, summary: []u8) void {
        defer self.allocator.free(summary);
        self.sidebar.menu_visible = false;
        self.sidebar.focused = false;
        self.doc.deinit();
        self.doc = document_mod.Document.init(self.allocator, self.io);
        self.doc.replaceAllOwned(diff_text) catch {
            self.setError("Out of memory.");
            return;
        };
        if (self.language_override) |old| self.allocator.free(old);
        self.language_override = self.allocator.dupe(u8, "diff") catch null;
        self.highlighter.clear();
        self.mode = .normal;
        self.cursor = 0;
        self.anchor = 0;
        self.top_line = 0;
        self.left_column = 0;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.find.active = false;
        self.find.query.clearRetainingCapacity();
        self.find.matches.clearRetainingCapacity();
        self.find.index = -1;
        self.doc.breakUndoGroup();
        self.snapCursor();
        self.setMessageFmt("Diff: {s}", .{summary});
    }

    fn requestDeleteSidebarSources(self: *Editor, sources: []const SidebarSource) !void {
        if (sources.len == 0) return;
        if (self.docAbsolutePath()) |document| {
            defer self.allocator.free(document);
            for (sources) |source| {
                if (std.mem.eql(u8, document, source.path) or
                    (source.is_directory and sidebar_mod.isInside(source.path, document)))
                {
                    self.setError("Cannot delete the open file or its parent folder.");
                    return;
                }
            }
        }
        for (self.pending_sidebar_deletes.items) |source| self.allocator.free(source.path);
        self.pending_sidebar_deletes.clearRetainingCapacity();
        for (sources) |source| {
            try self.pending_sidebar_deletes.append(self.allocator, .{
                .path = try self.allocator.dupe(u8, source.path),
                .is_directory = source.is_directory,
            });
        }
        self.pending = .delete;
        self.clearMessage();
    }

    fn deleteSidebarSources(self: *Editor) void {
        const items = self.pending_sidebar_deletes.items;
        var deleted: usize = 0;
        var first_error: ?[]const u8 = null;
        for (items) |source| {
            std.Io.Dir.cwd().deleteTree(self.io, source.path) catch |err| {
                if (first_error == null) first_error = @errorName(err);
                continue;
            };
            if (source.is_directory) self.sidebar.removeExpandedUnder(source.path);
            deleted += 1;
        }
        for (items) |source| self.allocator.free(source.path);
        self.pending_sidebar_deletes.clearRetainingCapacity();
        self.sidebar.refresh(null);
        if (first_error) |message| {
            self.setErrorFmt("Deleted {d} of {d} items: {s}", .{ deleted, items.len, message });
        } else {
            self.setMessageFmt("Deleted {d} item(s)", .{deleted});
        }
    }

    fn handleSidebarMouse(self: *Editor, event: term.Event) void {
        const width = self.sidebar.width(self.cols);
        if (width == 0 or event.mouse_x >= width) return;
        self.sidebar.focused = true;
        self.mode = .normal;
        self.sidebar.menu_visible = false;
        switch (event.key) {
            .scroll_up => self.sidebar.moveSelection(-3, false, false),
            .scroll_down => self.sidebar.moveSelection(3, false, false),
            .mouse_down => {
                const index = self.sidebar.top + event.mouse_y;
                if (index < self.sidebar.nodes.items.len) {
                    self.sidebar.setSelection(index, false);
                }
            },
            else => {},
        }
        self.sidebar.updateTop(self.textRows());
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
                self.toggleComment();
            }
            return;
        }
        if (event.key == .character and event.alt) {
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
                try self.eachSelection(.move_left, false);
                return;
            },
            .right => {
                try self.eachSelection(.move_right, false);
                return;
            },
            .up => {
                try self.eachSelection(.{ .move_vertical = -1 }, false);
                return;
            },
            .down => {
                try self.eachSelection(.{ .move_vertical = 1 }, false);
                return;
            },
            .page_up => {
                try self.eachSelection(.{ .move_page = -1 }, false);
                return;
            },
            .page_down => {
                try self.eachSelection(.{ .move_page = 1 }, false);
                return;
            },
            .home => {
                if (event.ctrl) try self.eachSelection(.document_start, false) else try self.eachSelection(.move_line_start, false);
                return;
            },
            .end => {
                if (event.ctrl) try self.eachSelection(.document_end, false) else try self.eachSelection(.move_line_end, false);
                return;
            },
            .escape => {
                self.doc.breakUndoGroup();
                self.pending_g = false;
                self.clearMessage();
                return;
            },
            .delete => {
                try self.eachSelection(.delete_at_cursor, true);
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
            '/' => try self.openFind(false, false, false),
            '\\' => try self.openFind(false, true, false),
            '\'' => try self.openFind(false, true, true),
            '.' => try self.openFind(true, false, false),
            'i', 'I' => try self.eachSelection(.enter_insert, false),
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
            'h' => try self.eachSelection(.move_left, false),
            'l' => try self.eachSelection(.move_right, false),
            'w', 'W' => try self.eachSelection(.word_forward, false),
            'q', 'Q' => try self.eachSelection(.word_backward, false),
            'j' => try self.eachSelection(.{ .move_vertical = 1 }, false),
            'k' => try self.eachSelection(.{ .move_vertical = -1 }, false),
            'd', 'D' => try self.eachSelection(.delete_at_cursor, true),
            'u' => {
                if (self.doc.undoEdit()) self.afterUndoRedo();
            },
            'U' => {
                if (self.doc.redoEdit()) self.afterUndoRedo();
            },
            'g', 'G' => self.pending_g = true,
            'v', 'V' => try self.eachSelection(.start_char_selection, false),
            '%' => try self.eachSelection(.select_all, false),
            'x', 'X' => try self.eachSelection(.start_line_selection, false),
            '<' => self.indentSelection(true),
            '>' => self.indentSelection(false),
            'C' => self.addCursorBelow(),
            ',' => self.keepPrimarySelection(),
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
                try self.eachSelection(.move_left, false);
                return;
            },
            .right => {
                try self.eachSelection(.move_right, false);
                return;
            },
            .up => {
                try self.eachSelection(.{ .move_vertical = -1 }, false);
                return;
            },
            .down => {
                try self.eachSelection(.{ .move_vertical = 1 }, false);
                return;
            },
            .page_up => {
                try self.eachSelection(.{ .move_page = -1 }, false);
                return;
            },
            .page_down => {
                try self.eachSelection(.{ .move_page = 1 }, false);
                return;
            },
            .home => {
                if (event.ctrl) try self.eachSelection(.document_start, false) else try self.eachSelection(.move_line_start, false);
                return;
            },
            .end => {
                if (event.ctrl) try self.eachSelection(.document_end, false) else try self.eachSelection(.move_line_end, false);
                return;
            },
            .enter => {
                try self.eachSelection(.insert_newline, true);
                return;
            },
            .backspace => {
                try self.eachSelection(.backspace, true);
                return;
            },
            .delete => {
                try self.eachSelection(.delete_forward, true);
                return;
            },
            .tab => {
                try self.eachSelection(.insert_indent, true);
                return;
            },
            .character => {
                try self.eachSelection(.{ .insert_char = event.ch }, true);
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
                self.toggleComment();
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
                try self.eachSelection(.move_left, false);
                return;
            },
            .right => {
                try self.eachSelection(.move_right, false);
                return;
            },
            .up => {
                try self.eachSelection(.{ .move_vertical = -1 }, false);
                return;
            },
            .down => {
                try self.eachSelection(.{ .move_vertical = 1 }, false);
                return;
            },
            .page_up => {
                try self.eachSelection(.{ .move_page = -1 }, false);
                return;
            },
            .page_down => {
                try self.eachSelection(.{ .move_page = 1 }, false);
                return;
            },
            .home => {
                if (event.ctrl) try self.eachSelection(.document_start, false) else try self.eachSelection(.move_line_start, false);
                return;
            },
            .end => {
                if (event.ctrl) try self.eachSelection(.document_end, false) else try self.eachSelection(.move_line_end, false);
                return;
            },
            .escape => {
                self.cancelSelection();
                return;
            },
            .delete => {
                try self.eachSelection(.delete_selection, true);
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
            '%' => try self.eachSelection(.select_all, false),
            '.' => try self.openFind(true, false, false),
            'x', 'X' => self.extendLineSelection(),
            'd', 'D' => try self.eachSelection(.delete_selection, true),
            '<' => self.indentSelection(true),
            '>' => self.indentSelection(false),
            'h' => try self.eachSelection(.move_left, false),
            'l' => try self.eachSelection(.move_right, false),
            'w', 'W' => try self.eachSelection(.word_forward, false),
            'q', 'Q' => try self.eachSelection(.word_backward, false),
            'j' => try self.eachSelection(.{ .move_vertical = 1 }, false),
            'k' => try self.eachSelection(.{ .move_vertical = -1 }, false),
            'i', 'I' => try self.eachSelection(.enter_insert_from_selection, false),
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
            '/' => try self.openPicker(.search),
            '\\' => try self.openRegexPicker(false),
            '.' => try self.openRegexPicker(true),
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
            .tab => {
                try self.completeCommandLine();
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

    fn completeCommandLine(self: *Editor) !void {
        const line = self.command_line.items;
        if (line.len == 0) return;
        var first_end: usize = 0;
        while (first_end < line.len and line[first_end] != ' ' and line[first_end] != '\t') : (first_end += 1) {}
        var lower: [64]u8 = undefined;
        const command = if (first_end <= lower.len)
            std.ascii.lowerString(lower[0..first_end], line[0..first_end])
        else
            line[0..first_end];
        if (!std.mem.eql(u8, command, "e") and !std.mem.eql(u8, command, "edit")) return;

        if (first_end >= line.len) {
            try self.command_line.append(self.allocator, ' ');
            self.command_cursor = self.command_line.items.len;
            return;
        }
        var argument_start = first_end;
        while (argument_start < line.len and (line[argument_start] == ' ' or line[argument_start] == '\t')) : (argument_start += 1) {}
        const cursor = @min(self.command_cursor, line.len);
        const prefix = line[argument_start..@max(argument_start, cursor)];

        var local_files: std.ArrayList([]u8) = .empty;
        defer {
            files_mod.clearFiles(self.allocator, &local_files);
            local_files.deinit(self.allocator);
        }
        const files: []const []u8 = if (self.picker.files_loaded)
            self.picker.files.items
        else blk: {
            var cancel = std.atomic.Value(bool).init(false);
            files_mod.scan(self.allocator, self.io, &local_files, 20_000, &cancel) catch break :blk &.{};
            break :blk local_files.items;
        };
        if (files.len == 0) return;

        var best: ?[]const u8 = null;
        for (files) |path| {
            if (prefix.len > 0 and util.indexOfIgnoreCase(path, prefix) == null) continue;
            if (best == null or path.len < best.?.len) best = path;
        }
        const candidate = best orelse return;
        self.command_line.items.len = argument_start;
        if (std.mem.indexOfAny(u8, candidate, " \t") != null) {
            try self.command_line.append(self.allocator, '"');
            try self.command_line.appendSlice(self.allocator, candidate);
            try self.command_line.append(self.allocator, '"');
        } else {
            try self.command_line.appendSlice(self.allocator, candidate);
        }
        self.command_cursor = self.command_line.items.len;
    }

    fn runExLine(self: *Editor, line: []const u8) !void {
        self.command_line.clearRetainingCapacity();
        try self.command_line.appendSlice(self.allocator, line);
        self.command_cursor = self.command_line.items.len;
        try self.submitCommand();
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
        var lower_name: [256]u8 = undefined;
        const command_name = if (name.len <= lower_name.len)
            std.ascii.lowerString(lower_name[0..name.len], name)
        else
            name;
        var args = std.mem.trim(u8, line[split..], " \t");
        if (args.len >= 2 and
            ((args[0] == '"' and args[args.len - 1] == '"') or
                (args[0] == '\'' and args[args.len - 1] == '\'')))
        {
            args = args[1 .. args.len - 1];
        }

        self.doc.breakUndoGroup();

        if (std.mem.eql(u8, command_name, "w") or std.mem.eql(u8, command_name, "write")) {
            _ = try self.saveDocument(if (args.len > 0) args else null);
        } else if (std.mem.eql(u8, command_name, "wq")) {
            if (try self.saveDocument(if (args.len > 0) args else null)) {
                self.running = false;
            }
        } else if (std.mem.eql(u8, command_name, "q") or std.mem.eql(u8, command_name, "quit")) {
            if (force) {
                self.running = false;
            } else if (self.doc.isDirty() and self.config.confirm_quit_without_saving) {
                self.setPending(.quit);
            } else {
                self.running = false;
            }
        } else if (std.mem.eql(u8, command_name, "e") or std.mem.eql(u8, command_name, "edit")) {
            if (args.len == 0) {
                self.setError("usage: :e <filename>");
            } else {
                try self.requestOpen(args, 0, 0, force);
            }
        } else if (std.mem.eql(u8, command_name, "path")) {
            if (self.doc.path) |path| self.setMessage(path) else self.setMessage("[No Name]");
        } else if (std.mem.eql(u8, command_name, "lang")) {
            if (args.len == 0) {
                self.setError("usage: :lang <language>");
            } else if (self.syntax_registry.find(args)) |language| {
                if (self.language_override) |old| self.allocator.free(old);
                self.language_override = try self.allocator.dupe(u8, args);
                self.highlighter.clear();
                self.setMessageFmt("Language: {s}", .{language.name});
                self.firePluginEvent("buffer_opened");
            } else {
                self.setErrorFmt("unknown language: {s}", .{args});
            }
        } else if (std.mem.eql(u8, command_name, "theme") or std.mem.eql(u8, command_name, "themes")) {
            if (args.len == 0) {
                try self.openThemePicker();
            } else if (self.theme_registry.setActive(args)) {
                self.theme = self.theme_registry.active;
                self.renderer.reset(self.allocator);
                self.setMessageFmt("Theme: {s}", .{self.theme.name});
            } else {
                self.setErrorFmt("unknown theme: {s}", .{args});
            }
        } else if (std.mem.eql(u8, command_name, "cw")) {
            try self.trimTrailingWhitespace();
        } else if (std.mem.eql(u8, command_name, "lua")) {
            if (args.len == 0) {
                self.setError("usage: :lua <plugin.lua>");
            } else {
                self.loadLuaPlugin(args);
            }
        } else if (std.mem.eql(u8, command_name, "plugins")) {
            self.listLuaCommands() catch |err| self.setErrorFmt("lua: {s}", .{@errorName(err)});
        } else if (std.mem.eql(u8, command_name, "vs")) {
            try self.splitPane(.vertical);
        } else if (std.mem.eql(u8, command_name, "hs")) {
            try self.splitPane(.horizontal);
        } else if (std.mem.eql(u8, command_name, "cb")) {
            self.closeCurrentBuffer();
        } else if (std.mem.eql(u8, command_name, "tc")) {
            self.toggleComment();
        } else if (std.mem.eql(u8, command_name, "sb")) {
            self.toggleSidebar();
        } else if (std.mem.eql(u8, command_name, "pm")) {
            self.openPluginManager();
        } else if (std.mem.eql(u8, command_name, "files")) {
            try self.openPicker(.files);
        } else if (std.mem.eql(u8, command_name, "search")) {
            try self.openPicker(.search);
        } else if (std.mem.eql(u8, command_name, "regex")) {
            try self.openRegexPicker(true);
        } else if (std.mem.eql(u8, command_name, "cfg")) {
            try self.openConfigFile();
        } else if (std.mem.eql(u8, command_name, "rcfg")) {
            try self.reloadConfiguration();
        } else if (std.mem.eql(u8, command_name, "help")) {
            self.help.open();
            self.clearMessage();
        } else if (std.mem.eql(u8, command_name, "rla")) {
            self.reloadAllFiles(force);
        } else if (std.mem.eql(u8, command_name, "open-log") or std.mem.eql(u8, command_name, "ol")) {
            self.openLogFile();
        } else if (std.mem.eql(u8, command_name, "about")) {
            self.setMessage("HexalZ 0.1.0 - a modal text editor written in Zig 0.17.0.");
        } else if (std.mem.eql(u8, command_name, "fmt")) {
            try self.formatDocument();
        } else {
            if (self.tryLuaCommand(command_name)) return;
            self.setErrorFmt("unknown command: {s}", .{name});
        }
    }

    fn loadPlugins(self: *Editor) !void {
        if (self.lua == null) self.lua = try lua_mod.Manager.init(self.allocator, self.io);
        self.updatePluginContext();
        const manager = if (self.lua) |*value| value else return;
        try manager.loadAll(&self.doc, &self.cursor);
        if (std.fs.path.dirname(self.config.source_path)) |config_dir| {
            const plugins_dir = std.fs.path.join(self.allocator, &.{ config_dir, "data", "plugins" }) catch null;
            if (plugins_dir) |directory| {
                defer self.allocator.free(directory);
                try manager.loadDirectory(directory, &self.doc, &self.cursor);
            }
        }
        try self.flushPluginEffects();
        self.firePluginEvent("buffer_opened");
    }

    fn updatePluginContext(self: *Editor) void {
        const manager = if (self.lua) |*value| value else return;
        const language = if (self.activeLanguage()) |lang| lang.id else "";
        const selection: ?lua_mod.Host.Selection = if (self.mode == .select) blk: {
            const range = self.selectionRange();
            if (range.end <= range.start) break :blk null;
            const start_line = self.doc.lineOfOffset(range.start);
            const end_line = self.doc.lineOfOffset(range.end);
            const text = self.doc.rangeAlloc(range.start, range.end - range.start) catch break :blk null;
            break :blk .{
                .start_line = start_line,
                .start_column = range.start - self.doc.lineStart(start_line),
                .end_line = end_line,
                .end_column = range.end - self.doc.lineStart(end_line),
                .text = text,
                .linewise = self.selection_linewise,
            };
        } else null;
        manager.setContext(
            self.doc.path orelse self.doc.displayName(),
            self.doc.path,
            language,
            self.workspace_root,
            self.cols,
            self.rows,
            selection,
        );
    }

    fn firePluginEvent(self: *Editor, event: []const u8) void {
        const manager = if (self.lua) |*value| value else return;
        self.updatePluginContext();
        manager.fireEvent(event, &self.doc, &self.cursor);
        self.flushPluginEffects() catch {};
    }

    fn flushPluginEffects(self: *Editor) !void {
        const manager = if (self.lua) |*value| value else return;
        var index: usize = 0;
        while (index < manager.host.messages.items.len) : (index += 1) {
            const item = manager.host.messages.items[index];
            if (item.is_error) self.setError(item.text) else self.setMessage(item.text);
        }
        for (manager.host.messages.items) |item| self.allocator.free(item.text);
        manager.host.messages.clearRetainingCapacity();

        if (manager.host.clipboard) |text| {
            self.storeClipboard(text);
            self.allocator.free(text);
            manager.host.clipboard = null;
        }

        if (self.plugin_panel_id == null and manager.host.panels.items.len > 0) {
            self.plugin_panel_id = manager.host.panels.items[manager.host.panels.items.len - 1].id;
        }

        try self.maybeStartPluginPrompt();

        while (manager.host.pending_commands.items.len > 0) {
            var command = manager.host.pending_commands.orderedRemove(0);
            defer {
                self.allocator.free(command.name);
                for (command.args.items) |arg| self.allocator.free(arg);
                command.args.deinit(self.allocator);
            }
            var line: std.ArrayList(u8) = .empty;
            defer line.deinit(self.allocator);
            try line.appendSlice(self.allocator, command.name);
            for (command.args.items) |arg| {
                try line.append(self.allocator, ' ');
                const quoted = std.mem.indexOfAny(u8, arg, " \t") != null;
                if (quoted) try line.append(self.allocator, '"');
                try line.appendSlice(self.allocator, arg);
                if (quoted) try line.append(self.allocator, '"');
            }
            try self.runExLine(line.items);
        }
    }

    fn maybeStartPluginPrompt(self: *Editor) !void {
        if (self.plugin_prompt_id != null) return;
        const manager = if (self.lua) |*value| value else return;
        if (manager.host.prompts.items.len == 0) return;
        const request = manager.host.prompts.orderedRemove(0);
        defer self.allocator.free(request.label);
        self.plugin_prompt_id = request.id;
        self.plugin_prompt_label.clearRetainingCapacity();
        try self.plugin_prompt_label.appendSlice(self.allocator, request.label);
        self.plugin_prompt_text.clearRetainingCapacity();
        self.mode = .command;
        self.command_line.clearRetainingCapacity();
        self.command_cursor = 0;
    }

    fn pumpPlugins(self: *Editor) !void {
        if (self.lua) |*manager| {
            _ = manager.runDeferred(&self.doc, &self.cursor);
        }
        try self.flushPluginEffects();
    }

    fn loadLuaPlugin(self: *Editor, path: []const u8) void {
        if (self.lua == null) {
            self.lua = lua_mod.Manager.init(self.allocator, self.io) catch |err| {
                self.setErrorFmt("lua init failed: {s}", .{@errorName(err)});
                return;
            };
        }
        self.updatePluginContext();
        const manager = if (self.lua) |*value| value else return;
        manager.loadFilePlugin(path, &self.doc, &self.cursor) catch |err| {
            if (manager.errors.items.len > 0) {
                self.setError(manager.errors.items[manager.errors.items.len - 1]);
            } else {
                self.setErrorFmt("lua plugin {s}: {s}", .{ path, @errorName(err) });
            }
            return;
        };
        self.flushPluginEffects() catch {};
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
            if (self.lua.?.errors.items.len > 0) {
                self.setError(self.lua.?.errors.items[0]);
            } else {
                self.setMessage("No Lua commands registered.");
            }
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
        const manager = if (self.lua) |*value| value else return false;
        if (!manager.hasCommand(name)) return false;

        self.updatePluginContext();
        _ = manager.executeCommand(name, &.{}, &self.doc, &self.cursor) catch |err| {
            if (manager.errors.items.len > 0) {
                self.setError(manager.errors.items[manager.errors.items.len - 1]);
            } else {
                self.setErrorFmt("lua command {s}: {s}", .{ name, @errorName(err) });
            }
            return true;
        };
        self.flushPluginEffects() catch {};

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

    fn formatDocument(self: *Editor) !void {
        const language = self.activeLanguage() orelse {
            self.setError("no language is active");
            return;
        };
        const formatter = language.formatter orelse {
            self.setErrorFmt("no formatter is configured for {s}", .{language.id});
            return;
        };
        const contents = try self.doc.rangeAlloc(0, self.doc.size());
        defer self.allocator.free(contents);
        const output = self.runFormatter(&formatter, contents, self.doc.path) catch return;
        try self.doc.replaceAllOwned(output);
        if (self.cursor > self.doc.size()) self.cursor = self.doc.size();
        self.anchor = self.cursor;
        self.snapCursor();
        self.setMessageFmt("Formatted {s}", .{self.doc.displayName()});
    }

    fn runFormatter(
        self: *Editor,
        formatter: *const syntax.FormatterConfig,
        contents: []const u8,
        source_path: ?[]const u8,
    ) ![]u8 {
        var temp_path: ?[]u8 = null;
        defer if (temp_path) |path| self.allocator.free(path);
        const use_file_input = std.ascii.eqlIgnoreCase(formatter.input, "file");
        if (use_file_input) {
            const now = std.Io.Clock.Timestamp.now(self.io, .awake).raw.nanoseconds;
            const path = try std.fmt.allocPrint(self.allocator, ".hexal-fmt-{d}.tmp", .{now});
            temp_path = path;
            std.Io.Dir.cwd().writeFile(self.io, .{ .sub_path = path, .data = contents }) catch |err| {
                self.setErrorFmt("formatter input failed: {s}", .{@errorName(err)});
                return error.FormatterFailed;
            };
        }

        var argv: std.ArrayList([]const u8) = .empty;
        defer argv.deinit(self.allocator);
        try argv.append(self.allocator, formatter.command);
        for (formatter.arguments) |argument| {
            if (std.mem.eql(u8, argument, "{file}")) {
                if (temp_path) |path| {
                    try argv.append(self.allocator, path);
                } else if (source_path) |path| {
                    try argv.append(self.allocator, path);
                } else {
                    try argv.append(self.allocator, "-");
                }
            } else {
                try argv.append(self.allocator, argument);
            }
        }

        var child = std.process.spawn(self.io, .{
            .argv = argv.items,
            .stdin = .pipe,
            .stdout = .pipe,
            .stderr = .pipe,
        }) catch |err| {
            self.setErrorFmt("cannot start formatter '{s}': {s}", .{ formatter.command, @errorName(err) });
            return error.FormatterFailed;
        };
        var killed = false;
        defer if (!killed) child.kill(self.io);

        if (!use_file_input) {
            if (child.stdin) |stdin| {
                stdin.writeStreamingAll(self.io, contents) catch {};
                stdin.close(self.io);
                child.stdin = null;
            }
        } else if (child.stdin) |stdin| {
            stdin.close(self.io);
            child.stdin = null;
        }

        var stdout_buffer: [4096]u8 = undefined;
        var stderr_buffer: [4096]u8 = undefined;
        var stdout: []u8 = &.{};
        defer if (stdout.len > 0) self.allocator.free(stdout);
        var stderr: []u8 = &.{};
        defer if (stderr.len > 0) self.allocator.free(stderr);
        if (child.stdout) |file| {
            var reader = file.reader(self.io, &stdout_buffer);
            stdout = reader.interface.allocRemaining(self.allocator, .limited(128 * 1024 * 1024)) catch &.{};
            file.close(self.io);
            child.stdout = null;
        }
        if (child.stderr) |file| {
            var reader = file.reader(self.io, &stderr_buffer);
            stderr = reader.interface.allocRemaining(self.allocator, .limited(1024 * 1024)) catch &.{};
            file.close(self.io);
            child.stderr = null;
        }
        const exit_term = child.wait(self.io) catch |err| {
            self.setErrorFmt("formatter failed: {s}", .{@errorName(err)});
            return error.FormatterFailed;
        };
        killed = true;

        if (!exit_term.success()) {
            const message = std.mem.trim(u8, stderr, " \t\r\n");
            if (message.len > 0) {
                self.setErrorFmt("formatter failed: {s}", .{message});
            } else {
                self.setError("formatter exited with an error");
            }
            return error.FormatterFailed;
        }

        var output: []const u8 = stdout;
        var file_output: ?[]u8 = null;
        defer if (file_output) |value| self.allocator.free(value);
        if (std.ascii.eqlIgnoreCase(formatter.output, "file")) {
            const path = temp_path orelse {
                self.setError("formatter output=file requires an input file");
                return error.FormatterFailed;
            };
            file_output = std.Io.Dir.cwd().readFileAlloc(self.io, path, self.allocator, .limited(128 * 1024 * 1024)) catch {
                self.setError("formatter did not produce output");
                return error.FormatterFailed;
            };
            output = file_output.?;
        }
        if (output.len == 0) {
            self.setError("formatter produced empty output");
            return error.FormatterFailed;
        }
        return self.allocator.dupe(u8, output);
    }

    fn saveDocument(self: *Editor, path: ?[]const u8) !bool {
        if (self.config.trim_trailing_whitespace_on_save) {
            self.trimTrailingWhitespace() catch {};
        }
        const format_on_save = self.config.format_on_save or
            (if (self.activeLanguage()) |language| language.format_on_save else false);
        if (format_on_save) {
            self.formatDocument() catch {};
        }
        self.doc.save(path) catch |err| {
            self.setErrorFmt("save failed: {s}", .{@errorName(err)});
            return false;
        };
        if (path) |p| {
            self.setMessageFmt("Saved {s}", .{p});
        } else {
            self.setMessageFmt("Saved {s}", .{self.doc.displayName()});
        }
        self.firePluginEvent("buffer_saved");
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
                .delete => {
                    self.pending = .none;
                    self.deleteSidebarSources();
                },
                .diff => {
                    const diff = self.pending_sidebar_diff orelse {
                        self.clearPending();
                        return;
                    };
                    self.pending = .none;
                    self.pending_sidebar_diff = null;
                    self.showSidebarDiff(diff.text, diff.summary);
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
        for (self.pending_sidebar_deletes.items) |source| self.allocator.free(source.path);
        self.pending_sidebar_deletes.clearRetainingCapacity();
        if (self.pending_sidebar_diff) |diff| {
            self.allocator.free(diff.text);
            self.allocator.free(diff.summary);
            self.pending_sidebar_diff = null;
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
        self.find.multi_cursor = false;
        self.find.regex = false;
        self.find.replace = false;
        self.find.editing_replacement = false;
        self.find.query.clearRetainingCapacity();
        self.find.replacement.clearRetainingCapacity();
        self.clearFindMatches();
        self.find.index = -1;
        self.extra_selections.clearRetainingCapacity();
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
        self.firePluginEvent("buffer_opened");
    }

    fn openPicker(self: *Editor, kind: PickerKind) !void {
        self.cancelPickerSearch();
        self.cancelPickerScan();
        self.picker.kind = kind;
        self.picker.query.clearRetainingCapacity();
        self.picker.replacement.clearRetainingCapacity();
        self.picker.regex = false;
        self.picker.replace = false;
        self.picker.editing_replacement = false;
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.status = null;
        self.pickerClearItems();

        if (kind == .themes or kind == .none) {
            try self.rebuildPicker();
        } else if (!self.picker.files_loaded) {
            try self.startPickerScan();
        } else {
            try self.rebuildPicker();
        }
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn openThemePicker(self: *Editor) !void {
        if (self.theme_before_picker) |old| self.allocator.free(old);
        self.theme_before_picker = self.allocator.dupe(u8, self.theme.id) catch null;
        try self.openPicker(.themes);
    }

    fn openRegexPicker(self: *Editor, replace: bool) !void {
        try self.openPicker(.search);
        self.picker.regex = true;
        self.picker.replace = replace;
        self.picker.replacement.clearRetainingCapacity();
        self.picker.editing_replacement = false;
    }

    fn closePicker(self: *Editor) void {
        if (self.picker.kind == .themes) {
            if (self.theme_before_picker) |id| {
                if (self.theme_registry.setActive(id)) self.theme = self.theme_registry.active;
                self.allocator.free(id);
                self.theme_before_picker = null;
            }
        }
        self.cancelPickerSearch();
        self.cancelPickerScan();
        self.picker.kind = .none;
        self.picker.query.clearRetainingCapacity();
        self.picker.replacement.clearRetainingCapacity();
        self.picker.regex = false;
        self.picker.replace = false;
        self.picker.editing_replacement = false;
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.status = null;
        self.pickerClearItems();
    }

    fn pickerClearItems(self: *Editor) void {
        for (self.picker.items.items) |item| self.allocator.free(item.display);
        self.picker.items.clearRetainingCapacity();
    }

    fn cancelPickerSearch(self: *Editor) void {
        const ctx = self.picker.search_context orelse return;
        ctx.cancel.store(true, .release);
        if (self.picker.search_thread) |thread| thread.join();
        self.picker.search_thread = null;
        self.picker.search_context = null;
        files_mod.clearHits(self.allocator, &ctx.hits);
        ctx.hits.deinit(self.allocator);
        self.allocator.free(ctx.query);
        self.allocator.destroy(ctx);
    }

    fn cancelPickerScan(self: *Editor) void {
        const ctx = self.picker.scan_context orelse return;
        ctx.cancel.store(true, .release);
        if (self.picker.scan_thread) |thread| thread.join();
        self.picker.scan_thread = null;
        self.picker.scan_context = null;
        files_mod.clearFiles(self.allocator, &ctx.files);
        ctx.files.deinit(self.allocator);
        self.allocator.destroy(ctx);
    }

    fn startPickerScan(self: *Editor) !void {
        self.cancelPickerScan();
        const ctx = try self.allocator.create(PickerScanContext);
        ctx.* = .{ .allocator = self.allocator, .io = self.io };
        self.picker.scan_context = ctx;
        self.picker.scan_thread = std.Thread.spawn(.{}, pickerScanWorker, .{ctx}) catch null;
        if (self.picker.scan_thread == null) pickerScanWorker(ctx);
        self.picker.status = "Loading files...";
    }

    fn startPickerSearch(self: *Editor) !void {
        self.cancelPickerSearch();
        files_mod.clearHits(self.allocator, &self.picker.hits);
        const ctx = try self.allocator.create(PickerSearchContext);
        errdefer self.allocator.destroy(ctx);
        const query = try self.allocator.dupe(u8, self.picker.query.items);
        errdefer self.allocator.free(query);
        ctx.* = .{
            .allocator = self.allocator,
            .io = self.io,
            .files = self.picker.files.items,
            .query = query,
            .regex = self.picker.regex,
            .case_insensitive = !self.config.search_match_case_enabled,
        };
        self.picker.search_context = ctx;
        self.picker.search_thread = std.Thread.spawn(.{}, pickerSearchWorker, .{ctx}) catch null;
        if (self.picker.search_thread == null) pickerSearchWorker(ctx);
        self.picker.status = "Searching...";
    }

    fn pumpPickerWork(self: *Editor) !void {
        var scan_completed = false;
        if (self.picker.scan_context) |ctx| {
            if (ctx.done.load(.acquire)) {
                if (self.picker.scan_thread) |thread| thread.join();
                self.picker.scan_thread = null;
                self.picker.scan_context = null;
                if (ctx.failed) {
                    self.picker.status = "E: file scan failed";
                } else {
                    files_mod.clearFiles(self.allocator, &self.picker.files);
                    self.picker.files.deinit(self.allocator);
                    self.picker.files = ctx.files;
                    ctx.files = .empty;
                    self.picker.files_loaded = true;
                    self.picker.status = null;
                }
                files_mod.clearFiles(self.allocator, &ctx.files);
                ctx.files.deinit(self.allocator);
                self.allocator.destroy(ctx);
                scan_completed = true;
            }
        }
        if (scan_completed and self.picker.kind != .none) {
            try self.rebuildPicker();
        }

        if (self.picker.search_context) |ctx| {
            if (ctx.done.load(.acquire)) {
                if (self.picker.search_thread) |thread| thread.join();
                self.picker.search_thread = null;
                self.picker.search_context = null;
                if (ctx.failed) {
                    self.picker.status = "E: search failed";
                } else {
                    files_mod.clearHits(self.allocator, &self.picker.hits);
                    self.picker.hits.deinit(self.allocator);
                    self.picker.hits = ctx.hits;
                    ctx.hits = .empty;
                    try self.populateSearchItems();
                    self.picker.status = null;
                }
                files_mod.clearHits(self.allocator, &ctx.hits);
                ctx.hits.deinit(self.allocator);
                self.allocator.free(ctx.query);
                self.allocator.destroy(ctx);
            }
        }
    }

    fn populateSearchItems(self: *Editor) !void {
        self.pickerClearItems();
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
        self.picker.selected = 0;
        self.picker.top = 0;
    }

    fn rebuildPicker(self: *Editor) !void {
        const query = self.picker.query.items;
        // Keep the previous results visible while a new cross-file search is
        // running; clearing them on every keystroke makes the picker flicker.
        const keep_search_results = self.picker.kind == .search and query.len > 0;
        if (!keep_search_results) {
            self.pickerClearItems();
            self.picker.total = 0;
        }

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
            .themes => {
                for (self.theme_registry.themes, 0..) |theme, index| {
                    if (query.len > 0 and
                        util.indexOfIgnoreCase(theme.name, query) == null and
                        util.indexOfIgnoreCase(theme.description, query) == null)
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    const display = try self.allocator.dupe(u8, theme.name);
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
                if (query.len == 0) {
                    self.cancelPickerSearch();
                    files_mod.clearHits(self.allocator, &self.picker.hits);
                    self.picker.status = null;
                } else if (!self.picker.files_loaded) {
                    self.picker.status = "Loading files...";
                    if (self.picker.scan_context == null) try self.startPickerScan();
                } else {
                    try self.startPickerSearch();
                }
            },
            .none => {},
        }

        if (self.picker.selected >= self.picker.items.items.len) {
            self.picker.selected = if (self.picker.items.items.len == 0) 0 else self.picker.items.items.len - 1;
        }
        if (self.picker.top > self.picker.selected) self.picker.top = self.picker.selected;
    }

    fn previewThemeSelection(self: *Editor) void {
        if (self.picker.kind != .themes) return;
        if (self.picker.selected >= self.picker.items.items.len) return;
        const item = self.picker.items.items[self.picker.selected];
        if (item.file_index >= self.theme_registry.themes.len) return;
        const id = self.theme_registry.themes[item.file_index].id;
        if (self.theme_registry.setActive(id)) self.theme = self.theme_registry.active;
    }

    fn handlePicker(self: *Editor, event: term.Event) !void {
        if (event.key == .escape) {
            self.closePicker();
            return;
        }

        if (self.picker.regex and (event.key == .tab or event.key == .backtab)) {
            self.picker.replace = true;
            self.picker.editing_replacement = !self.picker.editing_replacement;
            return;
        }

        if (self.picker.editing_replacement) {
            if (event.key == .character and !event.ctrl and !event.alt) {
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try self.picker.replacement.appendSlice(self.allocator, buffer[0..length]);
                return;
            }
            if (event.key == .backspace) {
                if (self.picker.replacement.items.len > 0) {
                    self.picker.replacement.items.len = util.previousBoundary(
                        self.picker.replacement.items,
                        self.picker.replacement.items.len,
                    );
                }
                return;
            }
            if (event.key == .enter) {
                try self.replaceAcrossFiles();
                return;
            }
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
                self.previewThemeSelection();
                return;
            },
            .down => {
                if (self.picker.selected + 1 < self.picker.items.items.len) {
                    self.picker.selected += 1;
                }
                self.previewThemeSelection();
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
                if (self.picker.replace) {
                    self.picker.editing_replacement = true;
                    return;
                }
                try self.openPickerSelection();
                return;
            },
            else => return,
        }
    }

    fn replaceAcrossFiles(self: *Editor) !void {
        if (!self.picker.files_loaded or self.picker.query.items.len == 0) {
            self.setError("No search pattern.");
            return;
        }
        self.cancelPickerSearch();
        const files = self.picker.files.items;
        var cancel = std.atomic.Value(bool).init(false);
        const stats = files_mod.replaceRegex(
            self.allocator,
            self.io,
            files,
            self.picker.query.items,
            self.picker.replacement.items,
            !self.config.search_match_case_enabled,
            &cancel,
            500,
        ) catch |err| {
            self.setErrorFmt("replace: {s}", .{@errorName(err)});
            return;
        };
        try self.rebuildPicker();
        self.setMessageFmt(
            "Replaced {d} match(es) in {d} file(s).",
            .{ stats.replacements, stats.files_changed },
        );
    }

    fn openPickerSelection(self: *Editor) !void {
        if (self.picker.items.items.len == 0) return;
        if (self.picker.selected >= self.picker.items.items.len) return;
        const item = self.picker.items.items[self.picker.selected];
        if (self.picker.kind == .themes) {
            if (item.file_index < self.theme_registry.themes.len) {
                const id = self.theme_registry.themes[item.file_index].id;
                if (self.theme_registry.setActive(id)) {
                    self.theme = self.theme_registry.active;
                    self.renderer.reset(self.allocator);
                    if (self.theme_before_picker) |old| {
                        self.allocator.free(old);
                        self.theme_before_picker = null;
                    }
                    self.setMessageFmt("Theme: {s}", .{self.theme.name});
                }
            }
            self.closePicker();
            return;
        }
        const path = self.picker.files.items[item.file_index];
        const line = item.line;
        const column = item.column;
        self.closePicker();
        try self.requestOpen(path, line, column, false);
    }

    fn pickerPreviewPath(self: *Editor) ?[]const u8 {
        if (self.picker.kind != .files) return null;
        if (self.picker.selected >= self.picker.items.items.len) return null;
        const item = self.picker.items.items[self.picker.selected];
        if (item.file_index >= self.picker.files.items.len) return null;
        return self.picker.files.items[item.file_index];
    }

    fn cacheCurrentPreview(self: *Editor) void {
        if (self.picker_preview.path.len == 0) return;
        if (self.picker_preview_cache.items.len >= preview_cache_entries) {
            var evicted = self.picker_preview_cache.orderedRemove(0);
            evicted.deinit(self.allocator);
        }
        self.picker_preview_cache.append(self.allocator, self.picker_preview) catch {
            self.picker_preview.deinit(self.allocator);
            self.picker_preview = .{};
            return;
        };
        self.picker_preview = .{};
    }

    fn takeCachedPreview(self: *Editor, path: []const u8) ?FilePreview {
        for (self.picker_preview_cache.items, 0..) |cached, index| {
            if (std.mem.eql(u8, cached.path, path)) {
                return self.picker_preview_cache.swapRemove(index);
            }
        }
        return null;
    }

    fn updatePickerPreview(self: *Editor) void {
        if (self.picker.kind != .files) return;
        const path = self.pickerPreviewPath() orelse {
            if (self.picker_preview.path.len != 0) {
                self.cacheCurrentPreview();
            }
            return;
        };
        if (std.mem.eql(u8, self.picker_preview.path, path)) return;
        self.cacheCurrentPreview();
        if (self.takeCachedPreview(path)) |cached| {
            self.picker_preview = cached;
            return;
        }
        self.picker_preview = readFilePreview(self.allocator, self.io, path);
    }

    fn pickerPreviewHeight(self: *Editor, available: usize) usize {
        if (self.picker.kind != .files or available < 10) return 0;
        const preview = @min(12, @max(4, if (available > 5) available - 5 else 0));
        return if (available >= preview + 1 and available - preview - 1 >= 4) preview else 0;
    }

    fn pickerDialogHeight(self: *Editor, available: usize) usize {
        const preview = self.pickerPreviewHeight(available);
        if (preview == 0) return self.pickerHeight();
        if (available < preview + 1) return self.pickerHeight();
        return @max(4, @min(18, available - preview - 1));
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

    fn clearFindMatches(self: *Editor) void {
        self.find.matches.clearRetainingCapacity();
        for (self.find.replacements.items) |replacement| self.allocator.free(replacement);
        self.find.replacements.clearRetainingCapacity();
        if (self.find.error_message) |message| {
            self.allocator.free(message);
            self.find.error_message = null;
        }
    }

    fn setFindError(self: *Editor, message: []const u8) void {
        if (self.find.error_message) |old| self.allocator.free(old);
        self.find.error_message = self.allocator.dupe(u8, message) catch null;
    }

    fn openFind(self: *Editor, multi_cursor: bool, regex: bool, replace: bool) !void {
        self.find.active = true;
        self.find.editing = true;
        self.find.multi_cursor = multi_cursor;
        self.find.regex = regex;
        self.find.replace = replace and regex;
        self.find.editing_replacement = false;
        self.find.query.clearRetainingCapacity();
        self.find.replacement.clearRetainingCapacity();
        self.clearFindMatches();
        self.find.index = -1;
        self.find.origin = self.cursor;
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn closeFind(self: *Editor) void {
        self.find.active = false;
        self.find.editing = false;
        self.find.multi_cursor = false;
        self.find.regex = false;
        self.find.replace = false;
        self.find.editing_replacement = false;
        self.find.query.clearRetainingCapacity();
        self.find.replacement.clearRetainingCapacity();
        self.clearFindMatches();
        self.find.index = -1;
        self.doc.breakUndoGroup();
    }

    fn activeFindField(self: *Editor) *std.ArrayList(u8) {
        return if (self.find.replace and self.find.editing_replacement)
            &self.find.replacement
        else
            &self.find.query;
    }

    fn activeFindCursor(self: *Editor) usize {
        return if (self.find.replace and self.find.editing_replacement)
            self.find.replacement.items.len
        else
            self.find.query.items.len;
    }

    fn rebuildFindMatches(self: *Editor) !void {
        self.clearFindMatches();
        if (self.find.query.items.len == 0) {
            self.find.index = -1;
            return;
        }
        if (self.find.regex) {
            try self.rebuildRegexFindMatches();
        } else {
            try self.rebuildLiteralFindMatches();
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
        self.jumpToFindMatch();
    }

    fn rebuildLiteralFindMatches(self: *Editor) !void {
        const data = try self.doc.rangeAlloc(0, self.doc.size());
        defer self.allocator.free(data);
        const needle = self.find.query.items;
        var search_from: usize = 0;
        while (search_from < data.len) {
            const relative = if (self.config.search_match_case_enabled)
                std.mem.indexOf(u8, data[search_from..], needle)
            else
                util.indexOfIgnoreCase(data[search_from..], needle);
            const offset = relative orelse break;
            const start = search_from + offset;
            const end = start + needle.len;
            try self.find.matches.append(self.allocator, .{ .start = start, .end = end });
            if (self.find.matches.items.len >= 20000) break;
            search_from = if (end > start) end else start + 1;
        }
    }

    fn rebuildRegexFindMatches(self: *Editor) !void {
        var regex = regex_mod.Regex.compileWithOptions(
            self.allocator,
            self.find.query.items,
            !self.config.search_match_case_enabled,
        ) catch |err| switch (err) {
            error.OutOfMemory => return err,
            else => {
                self.setFindError("invalid regular expression");
                return;
            },
        };
        defer regex.deinit();
        const data = try self.doc.rangeAlloc(0, self.doc.size());
        defer self.allocator.free(data);
        var search_from: usize = 0;
        while (search_from <= data.len) {
            const match = regex.findCaptures(data, search_from) orelse break;
            try self.find.matches.append(self.allocator, .{ .start = match.start, .end = match.end });
            if (self.find.replace) {
                const expanded = try regex_mod.expandReplacement(
                    self.allocator,
                    self.find.replacement.items,
                    data,
                    match,
                );
                try self.find.replacements.append(self.allocator, expanded);
            }
            search_from = if (match.end > match.start) match.end else match.end + 1;
            if (self.find.matches.items.len >= 20000) break;
        }
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

        if (self.find.replace and (event.key == .tab or event.key == .backtab)) {
            self.find.editing_replacement = !self.find.editing_replacement;
            return;
        }

        switch (event.key) {
            .enter => {
                if (self.find.replace) {
                    if (!self.find.editing_replacement) {
                        self.find.editing_replacement = true;
                        return;
                    }
                    if (event.ctrl) {
                        try self.replaceAllFindMatches();
                    } else {
                        try self.replaceCurrentFindMatch();
                    }
                    return;
                }
                if (self.find.multi_cursor) {
                    self.applyMultiCursorFind();
                    return;
                }
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
                const field = self.activeFindField();
                if (field.items.len > 0) {
                    const previous = util.previousBoundary(field.items, field.items.len);
                    field.items.len = previous;
                    try self.rebuildFindMatches();
                }
                return;
            },
            .character => {
                if (event.ctrl) {
                    if (event.ch == 'a' or event.ch == 'A') try self.replaceAllFindMatches();
                    return;
                }
                if (event.alt) return;
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                const field = self.activeFindField();
                try field.appendSlice(self.allocator, buffer[0..length]);
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
        self.mode = if (self.find.active) .normal else self.mode;
        self.snapCursor();
        self.doc.breakUndoGroup();
    }

    fn replaceCurrentFindMatch(self: *Editor) !void {
        if (self.find.matches.items.len == 0 or self.find.replacements.items.len == 0) {
            self.setMessage("No matches to replace.");
            return;
        }
        const index: usize = if (self.find.index >= 0)
            @min(@as(usize, @intCast(self.find.index)), self.find.matches.items.len - 1)
        else
            0;
        const match = self.find.matches.items[index];
        const replacement = self.find.replacements.items[index];
        self.doc.breakUndoGroup();
        try self.doc.replace(match.start, match.end - match.start, replacement);
        self.doc.breakUndoGroup();
        self.cursor = @min(match.start + replacement.len, self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.snapCursor();
        try self.rebuildFindMatches();
        self.setMessage("Replaced 1 match.");
    }

    fn replaceAllFindMatches(self: *Editor) !void {
        if (self.find.matches.items.len == 0 or self.find.replacements.items.len == 0) {
            self.setMessage("No matches to replace.");
            return;
        }
        const count = self.find.matches.items.len;
        const first_start = self.find.matches.items[0].start;
        self.doc.breakUndoGroup();
        var index = count;
        while (index > 0) {
            index -= 1;
            const match = self.find.matches.items[index];
            const replacement = self.find.replacements.items[index];
            try self.doc.replace(match.start, match.end - match.start, replacement);
        }
        self.doc.breakUndoGroup();
        self.cursor = @min(first_start, self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.snapCursor();
        try self.rebuildFindMatches();
        self.setMessageFmt("Replaced {d} match(es).", .{count});
    }

    fn applyMultiCursorFind(self: *Editor) void {
        if (self.find.matches.items.len == 0) {
            self.closeFind();
            self.setMessage("No matches for the search.");
            return;
        }
        const primary_index: usize = if (self.find.index >= 0)
            @min(@as(usize, @intCast(self.find.index)), self.find.matches.items.len - 1)
        else
            0;
        var selections: std.ArrayList(Selection) = .empty;
        defer selections.deinit(self.allocator);
        const primary_match = self.find.matches.items[primary_index];
        selections.append(self.allocator, .{
            .cursor = primary_match.end,
            .anchor = primary_match.start,
            .linewise = false,
        }) catch return;
        for (self.find.matches.items, 0..) |match, index| {
            if (index == primary_index) continue;
            selections.append(self.allocator, .{
                .cursor = match.end,
                .anchor = match.start,
                .linewise = false,
            }) catch break;
        }
        const count = selections.items.len;
        self.closeFind();
        self.setSelections(selections.items);
        self.mode = .select;
        self.selection_linewise = false;
        self.preferred_column = null;
        self.doc.breakUndoGroup();
        self.setMessageFmt("{d} matching selections.", .{count});
    }

    // -----------------------------------------------------------------------
    // MULTI CURSOR
    // -----------------------------------------------------------------------

    fn snapOffset(self: *Editor, offset: usize) usize {
        var value = @min(offset, self.doc.size());
        while (value > 0 and value < self.doc.size() and
            (self.doc.table.byteAt(value) & 0xC0) == 0x80)
        {
            value -= 1;
        }
        return value;
    }

    fn captureSelection(self: *Editor) Selection {
        return .{
            .cursor = self.cursor,
            .anchor = self.anchor,
            .linewise = self.selection_linewise,
            .preferred_column = self.preferred_column,
        };
    }

    fn loadSelection(self: *Editor, selection: Selection) void {
        self.cursor = self.snapOffset(selection.cursor);
        self.anchor = self.snapOffset(selection.anchor);
        self.selection_linewise = selection.linewise;
        self.preferred_column = selection.preferred_column;
    }

    fn captureAllSelections(self: *Editor) !std.ArrayList(Selection) {
        var selections: std.ArrayList(Selection) = .empty;
        errdefer selections.deinit(self.allocator);
        try selections.append(self.allocator, self.captureSelection());
        try selections.appendSlice(self.allocator, self.extra_selections.items);
        return selections;
    }

    fn setSelections(self: *Editor, selections: []const Selection) void {
        if (selections.len == 0) return;
        self.loadSelection(selections[0]);
        self.extra_selections.clearRetainingCapacity();
        for (selections[1..]) |selection| {
            const cursor = self.snapOffset(selection.cursor);
            var duplicate = cursor == self.cursor;
            if (!duplicate) {
                for (self.extra_selections.items) |existing| {
                    if (existing.cursor == cursor) {
                        duplicate = true;
                        break;
                    }
                }
            }
            if (duplicate) continue;
            self.extra_selections.append(self.allocator, .{
                .cursor = cursor,
                .anchor = self.snapOffset(selection.anchor),
                .linewise = selection.linewise,
                .preferred_column = selection.preferred_column,
            }) catch break;
        }
    }

    fn shiftSelections(self: *Editor, at: usize, removed: usize, inserted: usize) void {
        const adjust = struct {
            fn offset(position: usize, at_offset: usize, removed_len: usize, inserted_len: usize) usize {
                if (position < at_offset) return position;
                if (position >= at_offset + removed_len) {
                    if (inserted_len >= removed_len) return position + (inserted_len - removed_len);
                    return position - (removed_len - inserted_len);
                }
                return at_offset + @min(position - at_offset, inserted_len);
            }
        }.offset;
        self.cursor = adjust(self.cursor, at, removed, inserted);
        self.anchor = adjust(self.anchor, at, removed, inserted);
        for (self.extra_selections.items) |*selection| {
            selection.cursor = adjust(selection.cursor, at, removed, inserted);
            selection.anchor = adjust(selection.anchor, at, removed, inserted);
        }
    }

    fn applySelectionAction(self: *Editor, action: SelectionAction) !void {
        switch (action) {
            .move_left => self.moveLeft(),
            .move_right => self.moveRight(),
            .move_vertical => |delta| self.moveVertical(delta),
            .move_page => |delta| self.movePage(delta),
            .move_line_start => self.moveToLineStart(),
            .move_line_end => self.moveToLineEnd(),
            .document_start => self.jumpToDocumentStart(),
            .document_end => self.jumpToDocumentEnd(),
            .word_forward => self.moveWordForward(),
            .word_backward => self.moveWordBackward(),
            .delete_at_cursor => self.deleteAtCursor(),
            .delete_selection => self.deleteSelection(),
            .insert_char => |character| {
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(character, &buffer) catch return;
                try self.insertText(buffer[0..length]);
            },
            .insert_newline => try self.insertNewline(),
            .insert_indent => try self.insertIndent(),
            .backspace => self.backspace(),
            .delete_forward => self.deleteForward(),
            .enter_insert => self.enterInsert(),
            .enter_insert_from_selection => self.enterInsertFromSelection(),
            .select_all => self.selectAll(),
            .start_char_selection => self.startCharacterSelection(),
            .start_line_selection => self.startLineSelection(),
        }
    }

    fn eachSelection(self: *Editor, action: SelectionAction, editing: bool) !void {
        var selections = try self.captureAllSelections();
        defer selections.deinit(self.allocator);
        if (selections.items.len <= 1) {
            self.loadSelection(selections.items[0]);
            try self.applySelectionAction(action);
            return;
        }
        const results = try self.allocator.alloc(?Selection, selections.items.len);
        defer self.allocator.free(results);
        @memset(results, null);
        const order = try self.allocator.alloc(usize, selections.items.len);
        defer self.allocator.free(order);
        for (order, 0..) |*entry, index| entry.* = index;
        if (editing) {
            std.mem.sort(usize, order, selections.items, struct {
                fn lessThan(items: []const Selection, left: usize, right: usize) bool {
                    return items[left].cursor > items[right].cursor;
                }
            }.lessThan);
        }
        const mode_before = self.mode;
        for (order) |index| {
            self.loadSelection(selections.items[index]);
            self.mode = mode_before;
            const size_before = self.doc.size();
            try self.applySelectionAction(action);
            if (editing and self.doc.size() != size_before) {
                const delta: isize = @as(isize, @intCast(self.doc.size())) - @as(isize, @intCast(size_before));
                for (results) |*result| {
                    if (result.*) |*selection| {
                        selection.cursor = shiftOffset(selection.cursor, delta);
                        selection.anchor = shiftOffset(selection.anchor, delta);
                    }
                }
            }
            results[index] = self.captureSelection();
        }
        var compact: std.ArrayList(Selection) = .empty;
        defer compact.deinit(self.allocator);
        for (results) |result| if (result) |selection| try compact.append(self.allocator, selection);
        self.setSelections(compact.items);
    }

    fn addCursorBelow(self: *Editor) void {
        var selections = self.captureAllSelections() catch return;
        defer selections.deinit(self.allocator);
        var results: std.ArrayList(Selection) = .empty;
        defer results.deinit(self.allocator);
        results.appendSlice(self.allocator, selections.items) catch return;
        var added = false;
        for (selections.items) |source| {
            const source_line = self.doc.lineOfOffset(source.cursor);
            if (source_line + 1 >= self.doc.lineCount()) continue;
            const target_line = source_line + 1;
            const target_start = self.doc.lineStart(target_line);
            const text = self.doc.lineText(target_line, &self.line_buffer) catch continue;
            const column = self.cursorVisualColumnFor(source.cursor);
            const local = util.byteOffsetForVisualColumn(text, column, TabWidth);
            const target_cursor = @min(target_start + local, self.doc.size());
            var target_anchor = target_cursor;
            if (self.mode == .select and source.anchor != source.cursor and
                self.doc.lineOfOffset(source.anchor) == source_line)
            {
                const anchor_column = self.cursorVisualColumnFor(source.anchor);
                const anchor_local = util.byteOffsetForVisualColumn(text, anchor_column, TabWidth);
                target_anchor = @min(target_start + anchor_local, self.doc.size());
            }
            results.append(self.allocator, .{
                .cursor = target_cursor,
                .anchor = target_anchor,
                .linewise = source.linewise,
                .preferred_column = column,
            }) catch break;
            added = true;
        }
        if (!added) {
            self.setMessage("Already at the last line.");
            return;
        }
        self.setSelections(results.items);
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn keepPrimarySelection(self: *Editor) void {
        self.extra_selections.clearRetainingCapacity();
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn cursorVisualColumnFor(self: *Editor, offset: usize) usize {
        const line = self.doc.lineOfOffset(offset);
        const start = self.doc.lineStart(line);
        const text = self.doc.lineText(line, &self.line_buffer) catch return 0;
        const local = @min(offset - start, text.len);
        return util.visualColumn(text, local, TabWidth);
    }

    fn renderExtraSelections(self: *Editor, first_line: usize, rows: usize, gutter: usize) void {
        if (self.extra_selections.items.len == 0) return;
        const saved_cursor = self.cursor;
        const saved_anchor = self.anchor;
        const saved_linewise = self.selection_linewise;
        const saved_mode = self.mode;
        defer {
            self.cursor = saved_cursor;
            self.anchor = saved_anchor;
            self.selection_linewise = saved_linewise;
            self.mode = saved_mode;
        }
        for (self.extra_selections.items) |selection| {
            self.cursor = selection.cursor;
            self.anchor = selection.anchor;
            self.selection_linewise = selection.linewise;
            self.mode = .select;
            self.renderSelection(first_line, rows, gutter);
        }
    }

    fn renderExtraCursors(self: *Editor, first_line: usize, rows: usize, gutter: usize) void {
        if (self.extra_selections.items.len == 0) return;
        const saved_cursor = self.cursor;
        const saved_anchor = self.anchor;
        const saved_linewise = self.selection_linewise;
        defer {
            self.cursor = saved_cursor;
            self.anchor = saved_anchor;
            self.selection_linewise = saved_linewise;
        }
        for (self.extra_selections.items) |selection| {
            self.cursor = selection.cursor;
            self.anchor = selection.anchor;
            self.selection_linewise = selection.linewise;
            self.renderCursor(first_line, rows, gutter);
        }
    }

    // -----------------------------------------------------------------------
    // PANES
    // -----------------------------------------------------------------------

    fn initializePanes(self: *Editor) !void {
        const root = try self.allocator.create(PaneNode);
        root.* = .{ .leaf = 0 };
        self.pane_root = root;
        self.panes.append(self.allocator, .{ .id = 0 }) catch {
            self.allocator.destroy(root);
            self.pane_root = null;
            return error.OutOfMemory;
        };
        self.active_pane = 0;
        self.next_pane_id = 1;
        self.capturePaneState(0);
    }

    fn paneIndex(self: *Editor, id: u64) ?usize {
        for (self.panes.items, 0..) |pane, index| if (pane.id == id) return index;
        return null;
    }

    fn capturePaneState(self: *Editor, id: u64) void {
        const index = self.paneIndex(id) orelse return;
        const pane = &self.panes.items[index];
        pane.mode = self.mode;
        pane.cursor = self.cursor;
        pane.anchor = self.anchor;
        pane.linewise = self.selection_linewise;
        pane.preferred_column = self.preferred_column;
        pane.top_line = self.top_line;
        pane.left_column = self.left_column;
        pane.pending_g = self.pending_g;
        pane.extra.clearRetainingCapacity();
        pane.extra.appendSlice(self.allocator, self.extra_selections.items) catch {};
    }

    fn loadPaneState(self: *Editor, id: u64) void {
        const index = self.paneIndex(id) orelse return;
        const pane = &self.panes.items[index];
        self.mode = pane.mode;
        self.cursor = pane.cursor;
        self.anchor = pane.anchor;
        self.selection_linewise = pane.linewise;
        self.preferred_column = pane.preferred_column;
        self.top_line = pane.top_line;
        self.left_column = pane.left_column;
        self.pending_g = pane.pending_g;
        self.extra_selections.clearRetainingCapacity();
        self.extra_selections.appendSlice(self.allocator, pane.extra.items) catch {};
    }

    fn findPaneLeaf(_: *Editor, node: *PaneNode, target: u64) ?*PaneNode {
        switch (node.*) {
            .leaf => |id| return if (id == target) node else null,
            .split => |split| return findPaneLeafStatic(split.first, target) orelse
                findPaneLeafStatic(split.second, target),
        }
    }

    fn findPaneLeafStatic(node: *PaneNode, target: u64) ?*PaneNode {
        switch (node.*) {
            .leaf => |id| return if (id == target) node else null,
            .split => |split| return findPaneLeafStatic(split.first, target) orelse
                findPaneLeafStatic(split.second, target),
        }
    }

    fn removePaneLeaf(_: *Editor, node: *PaneNode, target: u64) bool {
        switch (node.*) {
            .split => |split| {
                if (split.first.* == .leaf and split.first.leaf == target) {
                    const keep = split.second;
                    const drop = split.first;
                    node.* = keep.*;
                    self_destroy(drop);
                    self_destroy(keep);
                    return true;
                }
                if (split.second.* == .leaf and split.second.leaf == target) {
                    const keep = split.first;
                    const drop = split.second;
                    node.* = keep.*;
                    self_destroy(drop);
                    self_destroy(keep);
                    return true;
                }
                return removePaneLeafStatic(split.first, target) or removePaneLeafStatic(split.second, target);
            },
            .leaf => return false,
        }
    }

    fn collectPaneViews(self: *Editor, node: *PaneNode, x: usize, y: usize, width: usize, height: usize, views: *std.ArrayList(PaneView)) void {
        switch (node.*) {
            .leaf => |id| views.append(self.allocator, .{
                .id = id,
                .x = x,
                .y = y,
                .width = width,
                .height = height,
            }) catch {},
            .split => |split| {
                if (split.orientation == .vertical) {
                    if (width < 3) {
                        self.collectPaneViews(split.first, x, y, width, height, views);
                        return;
                    }
                    const available = width - 1;
                    var first = @as(usize, @intFromFloat(@round(@as(f64, @floatFromInt(available)) * split.ratio)));
                    first = std.math.clamp(first, 1, available - 1);
                    self.collectPaneViews(split.first, x, y, first, height, views);
                    self.collectPaneViews(split.second, x + first + 1, y, available - first, height, views);
                } else {
                    if (height < 3) {
                        self.collectPaneViews(split.first, x, y, width, height, views);
                        return;
                    }
                    const available = height - 1;
                    var first = @as(usize, @intFromFloat(@round(@as(f64, @floatFromInt(available)) * split.ratio)));
                    first = std.math.clamp(first, 1, available - 1);
                    self.collectPaneViews(split.first, x, y, width, first, views);
                    self.collectPaneViews(split.second, x, y + first + 1, width, available - first, views);
                }
            },
        }
    }

    fn paneViews(self: *Editor) !std.ArrayList(PaneView) {
        var views: std.ArrayList(PaneView) = .empty;
        const left = self.editorLeft();
        const width = self.cols -| left;
        const height = self.textRows();
        if (self.pane_root) |root| self.collectPaneViews(root, left, 0, width, height, &views);
        return views;
    }

    fn splitPane(self: *Editor, orientation: PaneOrientation) !void {
        var views = try self.paneViews();
        defer views.deinit(self.allocator);
        var active_bounds: ?PaneView = null;
        for (views.items) |view| if (view.id == self.active_pane) {
            active_bounds = view;
            break;
        };
        const bounds = active_bounds orelse return;
        if (orientation == .vertical and bounds.width < 25) {
            self.setError("Pane is too narrow to split vertically.");
            return;
        }
        if (orientation == .horizontal and bounds.height < 5) {
            self.setError("Pane is too short to split horizontally.");
            return;
        }
        self.capturePaneState(self.active_pane);
        const index = self.paneIndex(self.active_pane) orelse return;
        var created = self.panes.items[index];
        created.id = self.next_pane_id;
        self.next_pane_id += 1;
        created.mode = .normal;
        created.pending_g = false;
        created.extra = .empty;
        created.extra.appendSlice(self.allocator, self.extra_selections.items) catch {};
        const created_id = created.id;
        try self.panes.append(self.allocator, created);

        const first = try self.allocator.create(PaneNode);
        first.* = .{ .leaf = self.active_pane };
        const second = try self.allocator.create(PaneNode);
        second.* = .{ .leaf = created_id };
        const replacement = PaneNode{ .split = .{
            .orientation = orientation,
            .ratio = 0.5,
            .first = first,
            .second = second,
        } };
        const leaf = if (self.pane_root) |root| findPaneLeafStatic(root, self.active_pane) else null;
        if (leaf == null) {
            self.allocator.destroy(first);
            self.allocator.destroy(second);
            _ = self.panes.pop();
            self.setError("Cannot split the active pane.");
            return;
        }
        leaf.?.* = replacement;
        self.active_pane = created_id;
        self.loadPaneState(created_id);
        self.snapCursor();
        self.ensureCursorVisible();
        self.setMessage(if (orientation == .vertical) "Split pane vertically." else "Split pane horizontally.");
    }

    fn closeCurrentBuffer(self: *Editor) void {
        if (self.panes.items.len <= 1) {
            self.setMessage("Only one buffer is open; use :q or :q! to quit Hexal.");
            return;
        }
        const active = self.active_pane;
        const index = self.paneIndex(active) orelse return;
        if (self.pane_root == null or !removePaneLeafStatic(self.pane_root.?, active)) {
            self.setError("Cannot close the active pane.");
            return;
        }
        _ = self.panes.orderedRemove(index);
        const target = if (self.panes.items.len > 0) self.panes.items[0].id else 0;
        self.active_pane = target;
        self.loadPaneState(target);
        self.snapCursor();
        self.ensureCursorVisible();
        self.setMessage("Closed pane.");
    }

    fn renderPaneContent(self: *Editor, view: PaneView) !void {
        if (view.width == 0 or view.height == 0) return;
        var temp: scr.Screen = .{};
        try temp.resize(self.allocator, view.width, view.height);
        temp.clear(self.theme.base());
        const saved_screen = self.screen;
        const saved_cols = self.cols;
        const saved_rows = self.rows;
        const saved_sidebar = self.sidebar.visible;
        const saved_focused = self.sidebar.focused;
        self.screen = temp;
        self.cols = view.width;
        self.rows = view.height + 2;
        self.sidebar.visible = false;
        self.sidebar.focused = false;
        self.renderPaneText() catch |err| {
            temp.deinit(self.allocator);
            self.screen = saved_screen;
            self.cols = saved_cols;
            self.rows = saved_rows;
            self.sidebar.visible = saved_sidebar;
            self.sidebar.focused = saved_focused;
            return err;
        };
        temp = self.screen;
        self.screen = saved_screen;
        self.cols = saved_cols;
        self.rows = saved_rows;
        self.sidebar.visible = saved_sidebar;
        self.sidebar.focused = saved_focused;
        self.pane_screen.deinit(self.allocator);
        self.pane_screen = temp;
    }

    fn renderPaneText(self: *Editor) !void {
        const rows = self.textRows();
        const gutter = self.gutterWidth();
        const first_line = self.top_line;
        const language = self.activeLanguage();
        self.ensureCursorVisible();
        var row: usize = 0;
        while (row < rows) : (row += 1) {
            const line = first_line + row;
            if (line >= self.doc.lineCount()) break;
            self.renderLineNumber(line, row);
            self.renderLineText(line, row, gutter, language);
        }
        self.renderSelection(first_line, rows, gutter);
        self.renderExtraSelections(first_line, rows, gutter);
        self.renderFindMatches(first_line, rows, gutter);
        self.renderCursor(first_line, rows, gutter);
        self.renderExtraCursors(first_line, rows, gutter);
    }

    fn renderPanes(self: *Editor) !void {
        self.capturePaneState(self.active_pane);
        var views = try self.paneViews();
        defer views.deinit(self.allocator);
        for (views.items) |view| {
            self.loadPaneState(view.id);
            try self.renderPaneContent(view);
            self.screen.blit(self.pane_screen, view.x, view.y);
            self.capturePaneState(view.id);
        }
        self.loadPaneState(self.active_pane);
        const style = self.theme.borderStyle();
        var index: usize = 0;
        while (index + 1 < views.items.len) : (index += 1) {
            const view = views.items[index];
            const next = views.items[index + 1];
            if (next.x > view.x) {
                var row: usize = 0;
                while (row < view.height) : (row += 1) {
                    self.screen.put(@intCast(next.x - 1), @intCast(view.y + row), 0x2502, style, 1);
                }
            } else if (next.y > view.y) {
                var column: usize = 0;
                while (column < view.width) : (column += 1) {
                    self.screen.put(@intCast(view.x + column), @intCast(next.y - 1), 0x2500, style, 1);
                }
            }
        }
    }

    fn reloadAllFiles(self: *Editor, force: bool) void {
        const path = self.doc.path orelse {
            self.setMessage("No file-backed buffers to reload.");
            return;
        };
        if (!force and self.doc.isDirty()) {
            self.setMessage("Kept 1 modified file; nothing reloaded.");
            return;
        }
        const cursor_line = self.doc.lineOfOffset(self.cursor);
        const cursor_column = self.cursor - self.doc.lineStart(cursor_line);
        const anchor_line = self.doc.lineOfOffset(self.anchor);
        const anchor_column = self.anchor - self.doc.lineStart(anchor_line);
        const top_line = self.top_line;
        const left_column = self.left_column;

        const loaded = document_mod.Document.open(self.allocator, self.io, path) catch |err| {
            self.setErrorFmt("Reloaded 0 file(s); cannot open '{s}': {s}.", .{ path, @errorName(err) });
            return;
        };
        self.doc.deinit();
        self.doc = loaded;
        if (self.language_override) |old| self.allocator.free(old);
        self.language_override = null;
        self.find.active = false;
        self.find.editing = false;
        self.find.query.clearRetainingCapacity();
        self.find.replacement.clearRetainingCapacity();
        self.clearFindMatches();
        self.find.index = -1;
        self.extra_selections.clearRetainingCapacity();
        self.preferred_column = null;

        const line = @min(cursor_line, self.doc.lineCount() - 1);
        self.cursor = @min(self.doc.lineStart(line) + cursor_column, self.doc.size());
        const anchor = @min(anchor_line, self.doc.lineCount() - 1);
        self.anchor = @min(self.doc.lineStart(anchor) + anchor_column, self.doc.size());
        self.top_line = @min(top_line, self.doc.lineCount() - 1);
        self.left_column = left_column;
        self.renderer.reset(self.allocator);
        self.screen.clear(self.theme.base());
        self.snapCursor();
        self.ensureCursorVisible();
        self.setMessage("Reloaded 1 file.");
        self.firePluginEvent("buffer_opened");
    }

    fn openLogFile(self: *Editor) void {
        const config_dir = std.fs.path.dirname(self.config.source_path) orelse {
            self.setError("cannot open Hexal log: configuration path is unavailable");
            return;
        };
        std.Io.Dir.cwd().createDirPath(self.io, config_dir) catch |err| {
            self.setErrorFmt("cannot open Hexal log: {s}", .{@errorName(err)});
            return;
        };
        const path = std.fs.path.join(self.allocator, &.{ config_dir, "hexal.log" }) catch return;
        defer self.allocator.free(path);
        if (!fileExists(self.io, path)) {
            std.Io.Dir.cwd().writeFile(self.io, .{ .sub_path = path, .data = "" }) catch |err| {
                self.setErrorFmt("cannot open Hexal log: {s}", .{@errorName(err)});
                return;
            };
        }
        self.requestOpen(path, 0, 0, false) catch |err| {
            self.setErrorFmt("cannot open Hexal log: {s}", .{@errorName(err)});
        };
    }

    // -----------------------------------------------------------------------
    // CONFIGURATION
    // -----------------------------------------------------------------------

    fn openConfigFile(self: *Editor) !void {
        const path = self.config.source_path;
        if (!fileExists(self.io, path)) {
            if (std.fs.path.dirname(path)) |directory| {
                std.Io.Dir.cwd().createDirPath(self.io, directory) catch {};
            }
            std.Io.Dir.cwd().writeFile(self.io, .{
                .sub_path = path,
                .data = config_mod.DEFAULT_CONTENTS,
            }) catch |err| {
                self.setErrorFmt("cannot create configuration: {s}", .{@errorName(err)});
                return;
            };
        }
        try self.requestOpen(path, 0, 0, false);
    }

    fn reloadConfiguration(self: *Editor) !void {
        const next = config_mod.load(self.allocator, self.io, self.config.source_path) catch |err| {
            self.setErrorFmt("configuration: {s}", .{@errorName(err)});
            return;
        };
        if (self.theme_registry.setActive(next.theme)) {
            self.theme = self.theme_registry.active;
            self.renderer.reset(self.allocator);
        }
        self.sidebar.show_git_ignored = next.show_git_ignored;
        self.config.deinit();
        self.config = next;
        self.setMessage("Configuration reloaded.");
    }

    // -----------------------------------------------------------------------
    // COMMAND PALETTE
    // -----------------------------------------------------------------------

    fn addPaletteItem(self: *Editor, title: []const u8, command: []const u8) !void {
        try self.palette.all.append(self.allocator, .{
            .title = try self.allocator.dupe(u8, title),
            .command = try self.allocator.dupe(u8, command),
        });
    }

    fn openPalette(self: *Editor) !void {
        for (self.palette.all.items) |item| {
            self.allocator.free(item.title);
            self.allocator.free(item.command);
        }
        self.palette.all.clearRetainingCapacity();
        self.palette.query.clearRetainingCapacity();
        self.palette.selected = 0;
        self.palette.top = 0;
        self.palette.visible = true;

        const builtins = [_]struct { title: []const u8, command: []const u8 }{
            .{ .title = "Open file picker", .command = "files" },
            .{ .title = "Open theme selector", .command = "themes" },
            .{ .title = "Search across files", .command = "search" },
            .{ .title = "Regex find/replace across files", .command = "regex" },
            .{ .title = "Split pane vertically", .command = "vs" },
            .{ .title = "Split pane horizontally", .command = "hs" },
            .{ .title = "Close current buffer pane", .command = "cb" },
            .{ .title = "Toggle sidebar", .command = "sb" },
            .{ .title = "Manage plugins", .command = "pm" },
            .{ .title = "Reload configuration", .command = "rcfg" },
            .{ .title = "Open configuration file", .command = "cfg" },
            .{ .title = "Format document", .command = "fmt" },
            .{ .title = "Clean trailing whitespace", .command = "cw" },
            .{ .title = "Toggle comment", .command = "tc" },
            .{ .title = "Show key help", .command = "help" },
            .{ .title = "Reload all files", .command = "rla" },
            .{ .title = "Open Hexal log", .command = "ol" },
            .{ .title = "Save file", .command = "w" },
            .{ .title = "Save and quit", .command = "wq" },
            .{ .title = "Quit", .command = "q" },
        };
        for (builtins) |item| try self.addPaletteItem(item.title, item.command);

        if (self.lua) |*manager| {
            const names = manager.commandNames(self.allocator) catch &.{};
            defer {
                for (names) |name| self.allocator.free(name);
                if (names.len > 0) self.allocator.free(names);
            }
            for (names) |name| {
                const title = std.fmt.allocPrint(self.allocator, "Plugin: {s}", .{name}) catch continue;
                defer self.allocator.free(title);
                try self.addPaletteItem(title, name);
            }
        }
        try self.rebuildPaletteFilter();
    }

    fn rebuildPaletteFilter(self: *Editor) !void {
        self.palette.filtered.clearRetainingCapacity();
        const query = self.palette.query.items;
        for (self.palette.all.items, 0..) |item, index| {
            if (query.len > 0 and util.indexOfIgnoreCase(item.title, query) == null) continue;
            try self.palette.filtered.append(self.allocator, index);
        }
        if (self.palette.filtered.items.len == 0) {
            self.palette.selected = 0;
        } else {
            self.palette.selected = @min(self.palette.selected, self.palette.filtered.items.len - 1);
        }
        if (self.palette.top > self.palette.selected) self.palette.top = self.palette.selected;
    }

    fn closePalette(self: *Editor) void {
        self.palette.visible = false;
        self.palette.query.clearRetainingCapacity();
        self.palette.filtered.clearRetainingCapacity();
        self.palette.selected = 0;
        self.palette.top = 0;
    }

    fn handlePalette(self: *Editor, event: term.Event) !void {
        switch (event.key) {
            .escape => self.closePalette(),
            .up => if (self.palette.selected > 0) {
                self.palette.selected -= 1;
            },
            .down => if (self.palette.selected + 1 < self.palette.filtered.items.len) {
                self.palette.selected += 1;
            },
            .page_up => self.palette.selected -|= 10,
            .page_down => {
                if (self.palette.filtered.items.len > 0) {
                    self.palette.selected = @min(
                        self.palette.filtered.items.len - 1,
                        self.palette.selected + 10,
                    );
                }
            },
            .home => self.palette.selected = 0,
            .end => {
                if (self.palette.filtered.items.len > 0) {
                    self.palette.selected = self.palette.filtered.items.len - 1;
                }
            },
            .enter => {
                if (self.palette.selected < self.palette.filtered.items.len) {
                    const item = self.palette.all.items[self.palette.filtered.items[self.palette.selected]];
                    const command = self.allocator.dupe(u8, item.command) catch return;
                    defer self.allocator.free(command);
                    self.closePalette();
                    try self.runExLine(command);
                }
            },
            .backspace => {
                if (self.palette.query.items.len > 0) {
                    self.palette.query.items.len = util.previousBoundary(
                        self.palette.query.items,
                        self.palette.query.items.len,
                    );
                    try self.rebuildPaletteFilter();
                }
            },
            .character => {
                if (event.ctrl or event.alt) return;
                var buffer: [4]u8 = undefined;
                const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                try self.palette.query.appendSlice(self.allocator, buffer[0..length]);
                self.palette.selected = 0;
                self.palette.top = 0;
                try self.rebuildPaletteFilter();
            },
            else => {},
        }
    }

    fn renderPalette(self: *Editor) void {
        if (!self.palette.visible) return;
        const available = self.statusRow();
        if (available < 2) return;
        const width = @min(self.cols -| 2, 88);
        const height = @min(available - 1, 18);
        if (width < 24 or height < 5) return;
        const x = (self.cols - width) / 2;
        const y = (self.rows - height) / 2;
        self.drawBox(x, y, width, height, " Command palette ");
        self.screen.putString(@intCast(x + 2), @intCast(y + 1), "> ", self.theme.accentStyle(), 2);
        self.screen.putString(@intCast(x + 4), @intCast(y + 1), self.palette.query.items, self.theme.text(), width -| 6);
        const rows = height - 3;
        const max_top = self.palette.filtered.items.len -| rows;
        if (self.palette.top > max_top) self.palette.top = max_top;
        if (self.palette.selected < self.palette.top) self.palette.top = self.palette.selected;
        if (self.palette.selected >= self.palette.top + rows) self.palette.top = self.palette.selected + 1 - rows;
        var row: usize = 0;
        while (row < rows) : (row += 1) {
            const index = self.palette.top + row;
            if (index >= self.palette.filtered.items.len) break;
            const item = self.palette.all.items[self.palette.filtered.items[index]];
            const style = if (index == self.palette.selected) self.theme.selectedItemStyle() else self.theme.text();
            if (index == self.palette.selected) {
                self.screen.fill(@intCast(x + 1), @intCast(y + 2 + row), width - 2, 1, ' ', style);
            }
            self.screen.putString(@intCast(x + 2), @intCast(y + 2 + row), item.title, style, width -| 4);
        }
        self.screen.putString(
            @intCast(x + 2),
            @intCast(y + height - 2),
            "type to filter · Enter run · Esc close",
            self.theme.mutedStyle(),
            width -| 4,
        );
    }

    // -----------------------------------------------------------------------
    // PLUGIN MANAGER
    // -----------------------------------------------------------------------

    fn openPluginManager(self: *Editor) void {
        self.plugin_manager.visible = true;
        self.plugin_manager.filtering = false;
        self.plugin_manager.filter.clearRetainingCapacity();
        self.plugin_manager.selected = 0;
        self.plugin_manager.top = 0;
        self.clearMessage();
    }

    fn closePluginManager(self: *Editor) void {
        self.plugin_manager.visible = false;
        self.plugin_manager.filter.clearRetainingCapacity();
        self.plugin_manager.filtering = false;
    }

    fn pluginCount(self: *Editor) usize {
        if (self.lua) |*manager| return manager.entries.items.len;
        return 0;
    }

    fn pluginMatchesFilter(self: *Editor, index: usize) bool {
        if (self.plugin_manager.filter.items.len == 0) return true;
        const manager = if (self.lua) |*value| value else return false;
        if (index >= manager.entries.items.len) return false;
        const entry = manager.entries.items[index];
        return util.indexOfIgnoreCase(entry.name, self.plugin_manager.filter.items) != null or
            util.indexOfIgnoreCase(entry.id, self.plugin_manager.filter.items) != null or
            util.indexOfIgnoreCase(entry.description, self.plugin_manager.filter.items) != null;
    }

    fn pluginVisibleCount(self: *Editor) usize {
        var count: usize = 0;
        var index: usize = 0;
        while (index < self.pluginCount()) : (index += 1) if (self.pluginMatchesFilter(index)) {
            count += 1;
        };
        return count;
    }

    fn pluginIndexAt(self: *Editor, ordinal: usize) ?usize {
        var count: usize = 0;
        var index: usize = 0;
        while (index < self.pluginCount()) : (index += 1) {
            if (!self.pluginMatchesFilter(index)) continue;
            if (count == ordinal) return index;
            count += 1;
        }
        return null;
    }

    fn toggleSelectedPlugin(self: *Editor) void {
        const index = self.pluginIndexAt(self.plugin_manager.selected) orelse return;
        const manager = if (self.lua) |*value| value else return;
        const entry = manager.entries.items[index];
        manager.setPluginEnabled(index, !entry.enabled, &self.doc, &self.cursor) catch |err| {
            self.setErrorFmt("plugin: {s}", .{@errorName(err)});
            return;
        };
        self.flushPluginEffects() catch {};
        self.setMessageFmt("Plugin {s} {s}.", .{ entry.id, if (entry.enabled) "disabled" else "enabled" });
    }

    fn reloadSelectedPlugin(self: *Editor) void {
        const index = self.pluginIndexAt(self.plugin_manager.selected) orelse return;
        const manager = if (self.lua) |*value| value else return;
        manager.reloadPlugin(index, &self.doc, &self.cursor) catch |err| {
            self.setErrorFmt("plugin: {s}", .{@errorName(err)});
            return;
        };
        self.flushPluginEffects() catch {};
        self.setMessage("Reloaded plugin.");
    }

    fn openSelectedPluginSource(self: *Editor) void {
        const index = self.pluginIndexAt(self.plugin_manager.selected) orelse return;
        const manager = if (self.lua) |*value| value else return;
        const path = manager.entries.items[index].entry_path;
        const copy = self.allocator.dupe(u8, path) catch return;
        defer self.allocator.free(copy);
        self.closePluginManager();
        self.requestOpen(copy, 0, 0, false) catch |err| {
            self.setErrorFmt("open plugin: {s}", .{@errorName(err)});
        };
    }

    fn handlePluginManager(self: *Editor, event: term.Event) !void {
        if (self.plugin_manager.filtering) {
            switch (event.key) {
                .escape => {
                    self.plugin_manager.filter.clearRetainingCapacity();
                    self.plugin_manager.filtering = false;
                    self.plugin_manager.selected = 0;
                },
                .enter => self.plugin_manager.filtering = false,
                .backspace => if (self.plugin_manager.filter.items.len > 0) {
                    self.plugin_manager.filter.items.len = util.previousBoundary(
                        self.plugin_manager.filter.items,
                        self.plugin_manager.filter.items.len,
                    );
                    self.plugin_manager.selected = 0;
                },
                .character => {
                    if (event.ctrl or event.alt) return;
                    var buffer: [4]u8 = undefined;
                    const length = std.unicode.utf8Encode(event.ch, &buffer) catch return;
                    try self.plugin_manager.filter.appendSlice(self.allocator, buffer[0..length]);
                    self.plugin_manager.selected = 0;
                },
                else => {},
            }
            return;
        }
        switch (event.key) {
            .escape => {
                if (self.plugin_manager.filter.items.len > 0) {
                    self.plugin_manager.filter.clearRetainingCapacity();
                } else {
                    self.closePluginManager();
                }
            },
            .up => if (self.plugin_manager.selected > 0) {
                self.plugin_manager.selected -= 1;
            },
            .down => if (self.plugin_manager.selected + 1 < self.pluginVisibleCount()) {
                self.plugin_manager.selected += 1;
            },
            .page_up => self.plugin_manager.selected -|= 10,
            .page_down => if (self.pluginVisibleCount() > 0) {
                self.plugin_manager.selected = @min(
                    self.pluginVisibleCount() - 1,
                    self.plugin_manager.selected + 10,
                );
            },
            .home => self.plugin_manager.selected = 0,
            .end => if (self.pluginVisibleCount() > 0) {
                self.plugin_manager.selected = self.pluginVisibleCount() - 1;
            },
            .enter => self.toggleSelectedPlugin(),
            .character => {
                if (event.ctrl or event.alt) return;
                switch (event.ch) {
                    '/' => self.plugin_manager.filtering = true,
                    'q', 'Q' => self.closePluginManager(),
                    'j', 'J' => if (self.plugin_manager.selected + 1 < self.pluginVisibleCount()) {
                        self.plugin_manager.selected += 1;
                    },
                    'k', 'K' => if (self.plugin_manager.selected > 0) {
                        self.plugin_manager.selected -= 1;
                    },
                    ' ', 'e', 'E', 't', 'T' => self.toggleSelectedPlugin(),
                    's', 'S' => self.openSelectedPluginSource(),
                    'r' => self.reloadSelectedPlugin(),
                    else => {},
                }
            },
            else => {},
        }
    }

    fn putDetailLine(
        self: *Editor,
        x: usize,
        row: *usize,
        bottom: usize,
        width: usize,
        text: []const u8,
        style: theme_mod.Style,
    ) void {
        if (row.* >= bottom or width == 0) return;
        self.screen.putString(@intCast(x), @intCast(row.*), text, style, width);
        row.* += 1;
    }

    fn putDetailWrapped(
        self: *Editor,
        x: usize,
        row: *usize,
        bottom: usize,
        width: usize,
        text: []const u8,
        style: theme_mod.Style,
    ) void {
        var remaining = text;
        while (remaining.len > 0 and row.* < bottom) {
            const newline = std.mem.indexOfScalar(u8, remaining, '\n') orelse remaining.len;
            var chunk = remaining[0..newline];
            var consumed = newline;
            if (util.displayWidthString(chunk) > width) {
                var cut = displayPrefixBytes(chunk, width);
                if (std.mem.lastIndexOfScalar(u8, chunk[0..cut], ' ')) |space| {
                    if (space > 0) cut = space;
                }
                chunk = chunk[0..cut];
                consumed = cut;
            }
            self.screen.putString(@intCast(x), @intCast(row.*), chunk, style, width);
            row.* += 1;
            remaining = remaining[consumed..];
            while (remaining.len > 0 and (remaining[0] == ' ' or remaining[0] == '\r')) remaining = remaining[1..];
            if (remaining.len > 0 and remaining[0] == '\n') remaining = remaining[1..];
        }
    }

    fn renderPluginManager(self: *Editor) void {
        if (!self.plugin_manager.visible) return;
        const width = std.math.clamp(self.cols -| 4, 20, 100);
        const height = std.math.clamp(self.rows -| 2, 5, 24);
        if (width < 24 or height < 5 or width > self.cols or height > self.rows) return;
        const x = (self.cols -| width) / 2;
        const y = (self.rows -| height) / 2;
        var title_buf: [512]u8 = undefined;
        const title = if (self.plugin_manager.filter.items.len > 0 or self.plugin_manager.filtering)
            std.fmt.bufPrint(&title_buf, " Plugins · /{s}{s} ", .{
                self.plugin_manager.filter.items,
                if (self.plugin_manager.filtering) "▏" else " ",
            }) catch " Plugins "
        else
            " Plugins ";
        self.drawBox(x, y, width, height, title);

        const content_rows = @max(height -| 3, 1);
        const list_width = std.math.clamp(width / 3, 14, 34);
        const detail_x = x + 2 + list_width;
        const detail_width = width -| (list_width + 4);
        const border_style = self.theme.borderStyle();
        var divider_row: usize = 0;
        while (divider_row < content_rows) : (divider_row += 1) {
            self.screen.put(@intCast(x + 1 + list_width), @intCast(y + 1 + divider_row), 0x2502, border_style, 1);
        }

        const count = self.pluginVisibleCount();
        if (count == 0) {
            const message = if (self.plugin_manager.filter.items.len > 0)
                "No matching plugins."
            else
                "No plugins discovered.";
            self.screen.putString(@intCast(x + 2), @intCast(y + 1), message, self.theme.mutedStyle(), detail_width);
        } else {
            const max_top = count -| content_rows;
            if (self.plugin_manager.top > max_top) self.plugin_manager.top = max_top;
            if (self.plugin_manager.selected < self.plugin_manager.top) self.plugin_manager.top = self.plugin_manager.selected;
            if (self.plugin_manager.selected >= self.plugin_manager.top + content_rows) {
                self.plugin_manager.top = self.plugin_manager.selected + 1 - content_rows;
            }
            var row: usize = 0;
            while (row < content_rows) : (row += 1) {
                const ordinal = self.plugin_manager.top + row;
                const index = self.pluginIndexAt(ordinal) orelse break;
                const manager = if (self.lua) |*value| value else break;
                const entry = manager.entries.items[index];
                const style = if (ordinal == self.plugin_manager.selected)
                    self.theme.selectedItemStyle()
                else if (entry.enabled)
                    self.theme.text()
                else
                    self.theme.mutedStyle();
                if (ordinal == self.plugin_manager.selected) {
                    self.screen.fill(@intCast(x + 1), @intCast(y + 1 + row), list_width, 1, ' ', style);
                }
                var label_buf: [512]u8 = undefined;
                const label = std.fmt.bufPrint(&label_buf, "{s} [{s}]", .{
                    entry.name,
                    if (entry.enabled) "on" else "off",
                }) catch entry.name;
                self.screen.putString(@intCast(x + 2), @intCast(y + 1 + row), label, style, list_width -| 1);
            }

            if (self.pluginIndexAt(self.plugin_manager.selected)) |index| {
                const manager = if (self.lua) |*value| value else return;
                const entry = manager.entries.items[index];
                var detail_row = y + 1;
                const bottom = y + height - 2;
                const accent = self.theme.accentStyle();
                const text_style = self.theme.text();
                const muted = self.theme.mutedStyle();
                var buf: [1024]u8 = undefined;
                self.putDetailLine(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Name: {s}", .{entry.name}) catch "Name", accent);
                self.putDetailLine(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Id: {s}", .{entry.id}) catch "Id", text_style);
                if (entry.version.len > 0) {
                    self.putDetailLine(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Version: {s}", .{entry.version}) catch "Version", text_style);
                }
                const status = if (entry.lazy and entry.runtime == null)
                    "Status: enabled (loads when its language is opened)"
                else if (entry.enabled)
                    "Status: enabled"
                else
                    "Status: disabled";
                self.putDetailLine(detail_x, &detail_row, bottom, detail_width, status, if (entry.enabled) text_style else muted);
                if (entry.author.len > 0) {
                    self.putDetailLine(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Author: {s}", .{entry.author}) catch "Author", text_style);
                }
                if (entry.license.len > 0) {
                    self.putDetailLine(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "License: {s}", .{entry.license}) catch "License", text_style);
                }
                if (entry.description.len > 0) {
                    self.putDetailWrapped(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Description: {s}", .{entry.description}) catch "Description", text_style);
                }
                const commands = manager.commandNamesFor(index, self.allocator) catch &.{};
                defer {
                    for (commands) |command| self.allocator.free(command);
                    if (commands.len > 0) self.allocator.free(commands);
                }
                if (commands.len == 0) {
                    self.putDetailLine(detail_x, &detail_row, bottom, detail_width, "Commands: none", text_style);
                } else {
                    var commands_buf: std.ArrayList(u8) = .empty;
                    defer commands_buf.deinit(self.allocator);
                    commands_buf.appendSlice(self.allocator, "Commands: ") catch {};
                    for (commands, 0..) |command, command_index| {
                        if (command_index > 0) commands_buf.appendSlice(self.allocator, ", ") catch {};
                        commands_buf.appendSlice(self.allocator, command) catch {};
                    }
                    self.putDetailWrapped(detail_x, &detail_row, bottom, detail_width, commands_buf.items, text_style);
                }
                if (entry.error_message) |message| {
                    self.putDetailWrapped(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Error: {s}", .{message}) catch "Error", self.theme.errorStyle());
                }
                if (entry.entry_path.len > 0) {
                    self.putDetailWrapped(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Entry: {s}", .{entry.entry_path}) catch "Entry", muted);
                }
                if (entry.manifest_path.len > 0) {
                    self.putDetailWrapped(detail_x, &detail_row, bottom, detail_width, std.fmt.bufPrint(&buf, "Manifest: {s}", .{entry.manifest_path}) catch "Manifest", muted);
                }
            }
        }

        const footer = if (self.plugin_manager.filtering)
            "type to filter · ↑↓ move · Enter apply · Esc clear"
        else
            "j/k move · Enter toggle · s source · r reload · R all · / search · Esc close";
        self.screen.putString(@intCast(x + 2), @intCast(y + height - 2), footer, self.theme.mutedStyle(), width -| 4);
    }

    // -----------------------------------------------------------------------
    // COMMENTS
    // -----------------------------------------------------------------------

    fn toggleComment(self: *Editor) void {
        const language = self.activeLanguage() orelse {
            self.setError("No comment syntax is available for this language.");
            return;
        };
        const line_marker = language.line_comment;
        const block_start = language.block_start;
        const block_end = language.block_end;
        if (line_marker == null and (block_start == null or block_end == null)) {
            self.setError("No comment syntax is available for this language.");
            return;
        }
        var lines: std.ArrayList(usize) = .empty;
        defer lines.deinit(self.allocator);
        var selections = self.captureAllSelections() catch return;
        defer selections.deinit(self.allocator);
        for (selections.items) |selection| {
            if (self.mode != .select or selection.anchor == selection.cursor) {
                self.addTouchedLine(&lines, self.doc.lineOfOffset(selection.cursor)) catch return;
                continue;
            }
            const start = @min(selection.anchor, selection.cursor);
            const end = @max(selection.anchor, selection.cursor);
            const first = self.doc.lineOfOffset(start);
            const last = if (end > start) self.doc.lineOfOffset(end - 1) else first;
            var line = first;
            while (line <= last) : (line += 1) self.addTouchedLine(&lines, line) catch return;
        }
        if (lines.items.len == 0) return;
        self.doc.breakUndoGroup();
        if (line_marker) |marker_text| {
            self.toggleLineComments(lines.items, marker_text);
        } else {
            self.toggleBlockComments(lines.items, block_start.?, block_end.?);
        }
        self.doc.breakUndoGroup();
        self.clearMessage();
    }

    fn addTouchedLine(self: *Editor, lines: *std.ArrayList(usize), line: usize) !void {
        for (lines.items) |existing| if (existing == line) return;
        try lines.append(self.allocator, line);
    }

    fn toggleLineComments(self: *Editor, lines: []const usize, marker: []const u8) void {
        var all_commented = true;
        for (lines) |line| {
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const indent = indentLength(text);
            if (!std.mem.startsWith(u8, text[indent..], marker)) {
                all_commented = false;
                break;
            }
        }
        const Change = struct { offset: usize, remove: usize, insert: []const u8 };
        var changes: std.ArrayList(Change) = .empty;
        defer changes.deinit(self.allocator);
        var inserts: std.ArrayList([]u8) = .empty;
        defer {
            for (inserts.items) |item| self.allocator.free(item);
            inserts.deinit(self.allocator);
        }
        for (lines) |line| {
            const line_start = self.doc.lineStart(line);
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const indent = indentLength(text);
            if (all_commented) {
                var remove = marker.len;
                if (indent + remove < text.len and text[indent + remove] == ' ') remove += 1;
                changes.append(self.allocator, .{ .offset = line_start + indent, .remove = remove, .insert = "" }) catch return;
            } else {
                const insert = if (indent < text.len)
                    std.fmt.allocPrint(self.allocator, "{s} ", .{marker}) catch return
                else
                    self.allocator.dupe(u8, marker) catch return;
                inserts.append(self.allocator, insert) catch return;
                changes.append(self.allocator, .{ .offset = line_start + indent, .remove = 0, .insert = insert }) catch return;
            }
        }
        std.mem.sort(Change, changes.items, {}, struct {
            fn lessThan(_: void, left: Change, right: Change) bool {
                return left.offset > right.offset;
            }
        }.lessThan);
        for (changes.items) |change| {
            self.shiftSelections(change.offset, change.remove, change.insert.len);
            self.doc.replace(change.offset, change.remove, change.insert) catch return;
        }
        self.snapCursor();
    }

    fn toggleBlockComments(self: *Editor, lines: []const usize, start: []const u8, end: []const u8) void {
        const Change = struct { offset: usize, remove: usize, insert: []const u8 };
        var changes: std.ArrayList(Change) = .empty;
        defer changes.deinit(self.allocator);
        var inserts: std.ArrayList([]u8) = .empty;
        defer {
            for (inserts.items) |item| self.allocator.free(item);
            inserts.deinit(self.allocator);
        }
        var all_blocked = true;
        for (lines) |line| {
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const indent = indentLength(text);
            const content_end = trimWhitespaceEnd(text, indent);
            if (content_end <= indent or
                !std.mem.startsWith(u8, text[indent..content_end], start) or
                !std.mem.endsWith(u8, text[indent..content_end], end))
            {
                all_blocked = false;
                break;
            }
        }
        for (lines) |line| {
            const line_start = self.doc.lineStart(line);
            const text = self.doc.lineText(line, &self.line_buffer) catch continue;
            const indent = indentLength(text);
            const content_end = trimWhitespaceEnd(text, indent);
            if (all_blocked) {
                changes.append(self.allocator, .{ .offset = line_start + content_end - end.len, .remove = end.len, .insert = "" }) catch return;
                changes.append(self.allocator, .{ .offset = line_start + indent, .remove = start.len, .insert = "" }) catch return;
            } else if (content_end <= indent) {
                const insert = std.fmt.allocPrint(self.allocator, "{s} {s}", .{ start, end }) catch return;
                inserts.append(self.allocator, insert) catch return;
                changes.append(self.allocator, .{ .offset = line_start + indent, .remove = 0, .insert = insert }) catch return;
            } else {
                const leading = std.fmt.allocPrint(self.allocator, "{s} ", .{start}) catch return;
                const trailing = std.fmt.allocPrint(self.allocator, " {s}", .{end}) catch return;
                inserts.append(self.allocator, leading) catch return;
                inserts.append(self.allocator, trailing) catch return;
                changes.append(self.allocator, .{ .offset = line_start + content_end, .remove = 0, .insert = trailing }) catch return;
                changes.append(self.allocator, .{ .offset = line_start + indent, .remove = 0, .insert = leading }) catch return;
            }
        }
        std.mem.sort(Change, changes.items, {}, struct {
            fn lessThan(_: void, left: Change, right: Change) bool {
                return left.offset > right.offset;
            }
        }.lessThan);
        for (changes.items) |change| {
            self.shiftSelections(change.offset, change.remove, change.insert.len);
            self.doc.replace(change.offset, change.remove, change.insert) catch return;
        }
        self.snapCursor();
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
        const last_line: isize = @intCast(self.doc.lineCount() - 1);
        const target_signed = std.math.clamp(@as(isize, @intCast(line)) + delta, 0, last_line);
        const target: usize = @intCast(target_signed);

        if (self.preferred_column == null) self.preferred_column = self.cursorVisualColumn();
        const target_start = self.doc.lineStart(target);
        const text = self.doc.lineText(target, &self.line_buffer) catch empty_text;
        const local = util.byteOffsetForVisualColumn(text, self.preferred_column.?, TabWidth);
        self.cursor = @min(target_start + local, self.doc.size());
        self.doc.breakUndoGroup();
        self.snapCursor();
    }

    fn movePage(self: *Editor, direction: isize) void {
        const visible = @max(1, self.textRows());
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

    fn editorLeft(self: *Editor) usize {
        return self.sidebar.width(self.cols);
    }

    fn gutterWidth(self: *Editor) usize {
        return self.editorLeft() + util.digitCount(self.doc.lineCount()) + 1;
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

        if (self.panes.items.len > 1) {
            try self.renderPanes();
        } else {
            self.ensureCursorVisible();

            const rows = self.textRows();
            if (self.top_line + rows > 2048) self.doc.ensureLineIndex() catch {};
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
            self.renderExtraSelections(first_line, rows, gutter);
            self.renderFindMatches(first_line, rows, gutter);
            self.renderCursor(first_line, rows, gutter);
            self.renderExtraCursors(first_line, rows, gutter);
        }
        self.renderSidebar();
        self.renderStatusLine();
        self.renderBottomLine();
        self.renderSpaceMenu();
        self.renderSidebarContextMenu();
        self.renderPicker();
        self.renderFind();
        self.renderPluginPanel();
        self.renderPalette();
        self.renderPluginManager();
        self.renderHelp();

        const output = try self.renderer.render(self.allocator, self.screen, false);
        try self.terminal.write(output);
    }

    fn renderLineNumber(self: *Editor, line: usize, row: usize) void {
        const digits = util.digitCount(self.doc.lineCount());
        const number = line + 1;
        const x = self.editorLeft() + digits - util.digitCount(number);
        const style = if (line == self.cursorLine()) self.theme.accentStyle() else self.theme.mutedStyle();
        self.screen.putInt(@intCast(x), @intCast(row), number, style, digits);
    }

    fn renderLineText(
        self: *Editor,
        line: usize,
        row: usize,
        gutter: usize,
        language: ?*const syntax.Language,
    ) void {
        if (self.cols <= gutter) return;
        const text = self.doc.lineText(line, &self.line_buffer) catch return;
        if (language) |lang| {
            self.highlighter.getLineTokens(&self.doc, lang, line, text, &self.tokens) catch {};
        } else {
            self.tokens.clearRetainingCapacity();
        }
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

    fn renderPluginPanel(self: *Editor) void {
        const id = self.plugin_panel_id orelse return;
        const manager = if (self.lua) |*value| value else return;
        const panel = manager.findPanel(id) orelse return;
        const available = self.statusRow();
        if (available < 3 or self.cols < 20) return;
        var width = if (self.cols > 4) self.cols - 4 else self.cols;
        if (width > 108) width = 108;
        if (width < 20) return;
        const height = @min(panel.lines.items.len + 2, if (available > 2) available - 1 else 0);
        if (height < 3) return;
        const x = (self.cols - width) / 2;
        const y = if (available > height) (available - height) / 2 else 0;
        self.drawBox(x, y, width, height, panel.title);

        const list_rows = height - 2;
        if (panel.selected < panel.top) panel.top = panel.selected;
        if (panel.selected >= panel.top + list_rows) panel.top = panel.selected - list_rows + 1;
        const max_top = panel.lines.items.len -| list_rows;
        if (panel.top > max_top) panel.top = max_top;

        var row: usize = 0;
        while (row < list_rows) : (row += 1) {
            const index = panel.top + row;
            if (index >= panel.lines.items.len) break;
            const screen_row = y + 1 + row;
            const selected = index == panel.selected;
            const base_style = if (selected) self.theme.selectedItemStyle() else self.theme.text();
            self.screen.fill(@intCast(x + 1), @intCast(screen_row), width - 2, 1, ' ', base_style);
            var column: usize = x + 1;
            const limit = x + width - 1;
            for (panel.lines.items[index].segments) |segment| {
                if (column >= limit) break;
                const style = if (selected) base_style else self.theme.namedStyle(segment.style);
                const remaining = limit - column;
                self.screen.putString(@intCast(column), @intCast(screen_row), segment.text, style, remaining);
                const written = util.displayWidthString(segment.text);
                column += @min(written, remaining);
            }
        }
        if (panel.footer.len > 0 and width > 4) {
            self.screen.putString(
                @intCast(x + 2),
                @intCast(y + height - 1),
                panel.footer,
                self.theme.accentStyle(),
                width - 4,
            );
        }
    }

    fn renderSidebar(self: *Editor) void {
        if (!self.sidebar.visible) return;
        const width = self.sidebar.width(self.cols);
        const rows = self.textRows();
        if (width == 0 or rows == 0) return;
        self.screen.fill(0, 0, width, rows, ' ', self.theme.base());
        const border = if (self.sidebar.focused) self.theme.accentStyle() else self.theme.borderStyle();
        var row: usize = 0;
        while (row < rows) : (row += 1) {
            self.screen.put(@intCast(width - 1), @intCast(row), 0x2502, border, 1);
        }
        if (self.sidebar.nodes.items.len == 0) {
            self.screen.putString(1, 0, "No files", self.theme.mutedStyle(), width -| 2);
            return;
        }

        self.sidebar.updateTop(rows);
        const selected_style = self.theme.selectedItemStyle();
        const folder_style = self.theme.accentStyle();
        const file_style = self.theme.text();
        const start = self.sidebar.selectionStart();
        const end = self.sidebar.selectionEnd();
        row = 0;
        while (row < rows) : (row += 1) {
            const index = self.sidebar.top + row;
            if (index >= self.sidebar.nodes.items.len) break;
            const node = self.sidebar.nodes.items[index];
            const selected = index >= start and index <= end;
            const style = if (selected)
                selected_style
            else if (node.is_directory)
                folder_style
            else
                file_style;
            if (selected) self.screen.fill(0, @intCast(row), width - 1, 1, ' ', style);
            const indent = 1 + node.depth * 2;
            if (indent >= width - 1) continue;
            const marker: u21 = if (node.is_directory) (if (node.expanded) 0x25BE else 0x25B8) else ' ';
            self.screen.put(@intCast(indent), @intCast(row), marker, style, 1);
            const name_x = indent + 2;
            if (name_x < width - 1) {
                self.screen.putString(@intCast(name_x), @intCast(row), node.name, style, width - name_x - 1);
            }
        }
    }

    fn renderSidebarContextMenu(self: *Editor) void {
        if (!self.sidebar.menu_visible) return;
        const node = self.sidebar.selectedNode() orelse return;
        const items = self.sidebar.menuItems();
        var width: usize = 4;
        for (items) |item| width = @max(width, item.label.len + 7);
        const rows = self.textRows();
        const height = items.len + 2;
        if (rows < 3 or self.cols < 10) return;
        width = @min(width, self.cols - 2);
        const anchor_row = 1 + (self.sidebar.selected -| self.sidebar.top);
        const y = @min(anchor_row, rows -| height);
        const x: usize = 1;
        var title_buf: [64]u8 = undefined;
        const title = if (self.sidebar.selectionCount() > 1)
            std.fmt.bufPrint(&title_buf, "{d} selected", .{self.sidebar.selectionCount()}) catch "Selected"
        else if (node.is_directory)
            "Folder"
        else
            "File";
        self.drawBox(x, y, width, height, title);

        const selected_style = self.theme.selectedItemStyle();
        const item_style = self.theme.text();
        for (items, 0..) |item, index| {
            const screen_row = y + 1 + index;
            const style = if (index == self.sidebar.menu_selected) selected_style else item_style;
            self.screen.fill(@intCast(x + 1), @intCast(screen_row), width - 2, 1, ' ', style);
            self.screen.putString(@intCast(x + 2), @intCast(screen_row), item.label, style, width -| 7);
            var key_buf: [1]u8 = .{item.hotkey};
            self.screen.putString(@intCast(x + width - 3), @intCast(screen_row), &key_buf, style, 2);
        }
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

        if (self.sidebar_prompt) |*prompt| {
            var label_buf: [768]u8 = undefined;
            const text = std.fmt.bufPrint(&label_buf, "{s}{s}", .{
                prompt.label.items,
                prompt.text.items,
            }) catch prompt.label.items;
            self.screen.putString(1, @intCast(row), text, self.theme.accentStyle(), if (self.cols > 1) self.cols - 1 else 0);
            const visual = util.visualColumn(text, prompt.label.items.len + prompt.cursor, 1);
            const x = 1 + visual;
            if (x < self.cols) {
                var style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.applyStyle(x, row, style);
            }
            return;
        }

        if (self.plugin_prompt_id != null) {
            const label = self.plugin_prompt_label.items;
            self.screen.putString(1, @intCast(row), label, self.theme.accentStyle(), if (self.cols > 1) self.cols - 1 else 0);
            const prompt_x = 1 + util.displayWidthString(label);
            if (prompt_x < self.cols) {
                self.screen.putString(
                    @intCast(prompt_x),
                    @intCast(row),
                    self.plugin_prompt_text.items,
                    self.theme.text(),
                    self.cols - prompt_x,
                );
                const visual = util.displayWidthString(self.plugin_prompt_text.items);
                const cursor_x = prompt_x + visual;
                if (cursor_x < self.cols) {
                    var style = self.screen.at(cursor_x, row).style;
                    style.reverse = true;
                    self.screen.applyStyle(cursor_x, row, style);
                }
            }
            return;
        }

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
                .delete => if (self.pending_sidebar_deletes.items.len == 1)
                    "Delete the selected item? (y/N)"
                else
                    "Delete the selected items? (y/N)",
                .diff => "Unsaved changes. Discard and show diff? (y/N)",
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
        if (self.sidebar_prompt != null) return "PROMPT";
        if (self.plugin_prompt_id != null) return "PROMPT";
        if (self.plugin_panel_id != null) return "PANEL";
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
        if (self.plugin_prompt_id != null or self.plugin_panel_id != null) return self.theme.promptStatusStyle();
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
        self.updatePickerPreview();
        const available = self.statusRow();
        const preview_height = self.pickerPreviewHeight(available);
        const height = self.pickerDialogHeight(available);
        if (height < 4 or self.cols < 20) return;
        const width = @min(self.cols - 2, 88);
        if (width < 20) return;
        const x = (self.cols - width) / 2;
        const total_height = height + if (preview_height > 0) preview_height + 1 else 0;
        const y = if (available >= total_height) (available - total_height) / 2 else 0;
        const title = switch (self.picker.kind) {
            .files => "Open file",
            .themes => "Select theme",
            .search => if (self.picker.replace)
                (if (self.picker.regex) "Regex replace across files" else "Replace across files")
            else if (self.picker.regex)
                "Regex search across files"
            else
                "Search across files",
            .none => "",
        };
        self.drawBox(x, y, width, height, title);

        const prompt_style = self.theme.accentStyle();
        self.screen.put(@intCast(x + 2), @intCast(y + 1), '>', prompt_style, 1);
        const query_width = if (width > 5) width - 5 else 0;
        self.screen.putString(@intCast(x + 4), @intCast(y + 1), self.picker.query.items, self.theme.text(), query_width);
        const query_column = util.visualColumn(self.picker.query.items, self.picker.query.items.len, 1);
        if (!self.picker.editing_replacement and query_column < query_width) {
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

        var status_buf: [320]u8 = undefined;
        const status = if (self.picker.replace and self.picker.replacement.items.len > 0)
            std.fmt.bufPrint(&status_buf, "Replace: {s}  Tab: edit pattern  Enter: apply", .{self.picker.replacement.items}) catch "Replace"
        else if (self.picker.replace)
            "Type pattern, Tab, then replacement; Enter applies to all files"
        else if (self.picker.status) |picker_status|
            picker_status
        else if (self.picker.kind == .search)
            if (self.picker.query.items.len == 0)
                "Type to search"
            else if (self.picker.total == 0)
                "No matches"
            else
                std.fmt.bufPrint(&status_buf, "{d} match{s}", .{
                    self.picker.total,
                    if (self.picker.total == 1) "" else "es",
                }) catch "matches"
        else if (self.picker.total == 0)
            "No items"
        else
            std.fmt.bufPrint(&status_buf, "{d} item(s)", .{self.picker.total}) catch "items";
        self.screen.putString(@intCast(x + 2), @intCast(y + height - 1), status, self.theme.accentStyle(), width - 4);

        if (preview_height > 0) {
            self.renderPickerPreview(x, y + height + 1, width, preview_height);
        }
    }

    fn renderPickerPreview(self: *Editor, x: usize, y: usize, width: usize, height: usize) void {
        const preview = &self.picker_preview;
        var title_buf: [1024]u8 = undefined;
        var status_buf: [512]u8 = undefined;
        const status: []const u8 = if (preview.path.len == 0)
            "No file selected"
        else if (preview.error_message) |message|
            std.fmt.bufPrint(&status_buf, "E: {s}", .{message}) catch "E: preview failed"
        else if (preview.binary)
            "Binary file"
        else if (preview.lines.items.len == 0)
            "Empty file"
        else if (preview.truncated)
            std.fmt.bufPrint(&status_buf, "First {d} lines (truncated)", .{preview.lines.items.len}) catch "truncated"
        else
            std.fmt.bufPrint(&status_buf, "First {d} line{s}", .{
                preview.lines.items.len,
                if (preview.lines.items.len == 1) "" else "s",
            }) catch "preview";

        const title = if (preview.path.len == 0)
            std.fmt.bufPrint(&title_buf, "Preview - {s}", .{status}) catch "Preview"
        else
            std.fmt.bufPrint(&title_buf, "Preview: {s} - {s}", .{ preview.path, status }) catch "Preview";
        self.drawBox(x, y, width, height, title);

        const content_rows = if (height > 2) height - 2 else 0;
        const content_width = if (width > 4) width - 4 else 0;
        if (preview.error_message != null) {
            self.screen.putString(@intCast(x + 2), @intCast(y + 1), status, self.theme.errorStyle(), content_width);
            return;
        }
        if (preview.binary) {
            self.screen.putString(@intCast(x + 2), @intCast(y + 1), "Binary file", self.theme.mutedStyle(), content_width);
            return;
        }
        if (preview.path.len == 0 or preview.lines.items.len == 0) {
            self.screen.putString(@intCast(x + 2), @intCast(y + 1), status, self.theme.mutedStyle(), content_width);
            return;
        }

        const language = if (preview.path.len > 0) self.syntax_registry.findByPath(preview.path) else null;
        var in_block = false;
        var row: usize = 0;
        while (row < content_rows and row < preview.lines.items.len) : (row += 1) {
            const expanded = self.expandPreviewLine(preview.lines.items[row]) catch continue;
            defer self.allocator.free(expanded);
            if (language) |lang| {
                in_block = syntax.tokenizeLine(self.allocator, expanded, lang, in_block, &self.tokens) catch in_block;
                self.renderPreviewHighlightedLine(x + 2, y + 1 + row, expanded, content_width);
            } else {
                self.screen.putString(@intCast(x + 2), @intCast(y + 1 + row), expanded, self.theme.text(), content_width);
            }
        }
    }

    fn expandPreviewLine(self: *Editor, line: []const u8) ![]u8 {
        var output: std.ArrayList(u8) = .empty;
        errdefer output.deinit(self.allocator);
        var column: usize = 0;
        var offset: usize = 0;
        while (offset < line.len) {
            const decoded = util.decodeAt(line, offset);
            if (decoded.len == 0) break;
            offset += decoded.len;
            const cp = decoded.codepoint;
            if (cp == '\t') {
                const spaces = TabWidth - (column % TabWidth);
                try output.appendNTimes(self.allocator, ' ', spaces);
                column += spaces;
            } else if (cp < 0x20 or cp == 0x7F) {
                try output.append(self.allocator, ' ');
                column += 1;
            } else {
                try output.appendSlice(self.allocator, line[offset - decoded.len .. offset]);
                column += 1;
            }
        }
        return output.toOwnedSlice(self.allocator);
    }

    fn renderPreviewHighlightedLine(self: *Editor, x: usize, y: usize, line: []const u8, width: usize) void {
        if (width == 0) return;
        var column: usize = 0;
        var offset: usize = 0;
        var token_index: usize = 0;
        while (offset < line.len) {
            const rune_start = offset;
            while (token_index < self.tokens.items.len and self.tokens.items[token_index].end <= rune_start) {
                token_index += 1;
            }
            const style = if (token_index < self.tokens.items.len and
                self.tokens.items[token_index].start <= rune_start)
                self.theme.syntaxStyle(self.tokens.items[token_index].kind)
            else
                self.theme.text();
            const decoded = util.decodeAt(line, offset);
            if (decoded.len == 0) break;
            offset += decoded.len;
            const rune_width = if (decoded.codepoint == '\t')
                TabWidth - (column % TabWidth)
            else
                util.displayWidth(decoded.codepoint);
            if (rune_width == 0) continue;
            if (column >= width) break;
            if (column + rune_width > width) break;
            if (decoded.codepoint == '\t') {
                var i: usize = 0;
                while (i < rune_width) : (i += 1) {
                    self.screen.put(@intCast(x + column + i), @intCast(y), ' ', style, 1);
                }
            } else {
                self.screen.put(@intCast(x + column), @intCast(y), decoded.codepoint, style, rune_width);
            }
            column += rune_width;
        }
    }

    fn renderFind(self: *Editor) void {
        if (!self.find.active) return;
        const available = self.statusRow();
        const height: usize = if (self.find.replace) 5 else 4;
        const width = @min(if (self.cols > 2) self.cols - 2 else 0, 78);
        if (available < height + 1 or self.cols < 20 or width < 20) return;
        const x = (self.cols - width) / 2;
        const y = available - height;
        const title = if (self.find.replace)
            if (self.find.regex) "Regex find / replace" else "Find / replace"
        else if (self.find.regex) "Regex find" else "Find";
        self.drawBox(x, y, width, height, title);

        const query_editing = !self.find.replace or !self.find.editing_replacement;
        self.screen.put(
            @intCast(x + 2),
            @intCast(y + 1),
            if (query_editing) '>' else ' ',
            self.theme.accentStyle(),
            1,
        );
        const query_width = if (width > 6) width - 6 else 0;
        self.screen.putString(@intCast(x + 4), @intCast(y + 1), self.find.query.items, self.theme.text(), query_width);
        if (query_editing and self.find.editing) {
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

        var status_buf: [192]u8 = undefined;
        if (self.find.replace) {
            self.screen.put(
                @intCast(x + 2),
                @intCast(y + 2),
                if (!query_editing) '>' else ' ',
                self.theme.accentStyle(),
                1,
            );
            self.screen.putString(
                @intCast(x + 4),
                @intCast(y + 2),
                self.find.replacement.items,
                self.theme.text(),
                query_width,
            );
            if (!query_editing and self.find.editing) {
                const column = util.visualColumn(self.find.replacement.items, self.find.replacement.items.len, 1);
                if (column < query_width) {
                    const cursor_x = x + 4 + column;
                    if (cursor_x < x + width - 1) {
                        var style = self.screen.at(cursor_x, y + 2).style;
                        style.reverse = true;
                        self.screen.applyStyle(cursor_x, y + 2, style);
                    }
                }
            }
        }

        const status_row = y + height - 2;
        const status = if (self.find.error_message) |message|
            message
        else if (self.find.query.items.len == 0)
            "Type a pattern  Tab: replacement  Enter: next/apply  Esc: cancel"
        else if (self.find.matches.items.len == 0)
            "No matches"
        else if (self.find.index >= 0)
            std.fmt.bufPrint(&status_buf, "{d}/{d} matches  Tab: replacement  Enter: apply  Ctrl+A: all", .{
                @as(usize, @intCast(self.find.index)) + 1,
                self.find.matches.items.len,
            }) catch "matches"
        else
            "matches";
        self.screen.putString(
            @intCast(x + 2),
            @intCast(status_row),
            status,
            if (self.find.error_message != null) self.theme.errorStyle() else self.theme.accentStyle(),
            width - 4,
        );
    }

    fn renderHelp(self: *Editor) void {
        if (!self.help.visible) return;
        self.screen.clear(self.theme.base());
        const layout = self.help.layout(self.cols, self.rows);
        self.drawBox(layout.x, layout.y, layout.width, layout.height, " Help ");
        self.help.top = @min(self.help.top, self.help.maxTop(self.cols, self.rows));

        const section_style = self.theme.accentStyle();
        var key_style = self.theme.syntaxStyle(.default);
        key_style.fg = self.theme.accent;
        const text_style = self.theme.text();
        const muted_style = self.theme.mutedStyle();
        const key_width = @min(
            std.math.clamp(layout.width / 3, 8, 30),
            @max(layout.width -| 8, 4),
        );

        var row: usize = 0;
        while (row < layout.rows) : (row += 1) {
            const index = self.help.top + row;
            if (index >= help_mod.LINES.len) break;
            const line = help_mod.LINES[index];
            const screen_row = layout.y + 1 + row;
            switch (line.kind) {
                .section => self.screen.putString(
                    @intCast(layout.x + 2),
                    @intCast(screen_row),
                    line.left,
                    section_style,
                    layout.width -| 4,
                ),
                .entry => {
                    self.screen.putString(
                        @intCast(layout.x + 2),
                        @intCast(screen_row),
                        line.left,
                        key_style,
                        key_width,
                    );
                    self.screen.putString(
                        @intCast(layout.x + 2 + key_width),
                        @intCast(screen_row),
                        line.right,
                        text_style,
                        layout.width -| (key_width + 4),
                    );
                },
                .text => self.screen.putString(
                    @intCast(layout.x + 2),
                    @intCast(screen_row),
                    line.left,
                    muted_style,
                    layout.width -| 4,
                ),
                .blank => {},
            }
        }

        const max_top = self.help.maxTop(self.cols, self.rows);
        if (max_top > 0 and layout.height > 2) {
            const track_height = @max(layout.height -| 2, 1);
            const thumb_height = @max(
                1,
                @as(usize, @intFromFloat(@round(
                    (@as(f64, @floatFromInt(track_height)) * @as(f64, @floatFromInt(layout.rows))) /
                        @as(f64, @floatFromInt(help_mod.LINES.len)),
                ))),
            );
            const thumb_top = (track_height - thumb_height) * self.help.top / max_top;
            var track_row: usize = 0;
            while (track_row < track_height) : (track_row += 1) {
                const thumb = track_row >= thumb_top and track_row < thumb_top + thumb_height;
                self.screen.put(
                    @intCast(layout.x + layout.width - 2),
                    @intCast(layout.y + 1 + track_row),
                    if (thumb) 0x2588 else 0x2502,
                    if (thumb) key_style else muted_style,
                    1,
                );
            }
        }

        const hint_row = layout.y + layout.height;
        if (hint_row < self.rows) {
            const hint = "Esc / q close    Up/Down scroll    PgUp/PgDn page    Home/End";
            const width = util.displayWidthString(hint);
            const col = (self.cols -| width) / 2;
            self.screen.putString(
                @intCast(col),
                @intCast(hint_row),
                hint,
                muted_style,
                self.cols -| col,
            );
        }
    }

    // -----------------------------------------------------------------------
    // SMALL HELPERS
    // -----------------------------------------------------------------------

    fn activeLanguage(self: *Editor) ?*const syntax.Language {
        if (self.language_override) |id| {
            if (self.syntax_registry.find(id)) |lang| return lang;
        }
        if (self.doc.path) |path| return self.syntax_registry.findByPath(path);
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

fn previousScalarIn(doc: *document_mod.Document, offset: usize) usize {
    var value = offset;
    while (value > 0 and value < doc.size() and (doc.table.byteAt(value) & 0xC0) == 0x80) {
        value -= 1;
    }
    return value;
}

fn displayPrefixBytes(text: []const u8, max_width: usize) usize {
    var offset: usize = 0;
    var width: usize = 0;
    while (offset < text.len) {
        const decoded = util.decodeAt(text, offset);
        if (decoded.len == 0) break;
        const cell_width = util.displayWidth(decoded.codepoint);
        if (width + cell_width > max_width) break;
        width += cell_width;
        offset += decoded.len;
    }
    return offset;
}

fn indentLength(text: []const u8) usize {
    var offset: usize = 0;
    while (offset < text.len and (text[offset] == ' ' or text[offset] == '\t')) offset += 1;
    return offset;
}

fn trimWhitespaceEnd(text: []const u8, minimum: usize) usize {
    var end = text.len;
    while (end > minimum and (text[end - 1] == ' ' or text[end - 1] == '\t')) end -= 1;
    return end;
}

fn shiftOffset(position: usize, delta: isize) usize {
    if (delta >= 0) return position +| @as(usize, @intCast(delta));
    return position -| @as(usize, @intCast(-delta));
}

fn removePaneLeafStatic(node: *PaneNode, target: u64) bool {
    switch (node.*) {
        .split => |split| {
            if (split.first.* == .leaf and split.first.leaf == target) {
                const keep = split.second;
                const drop = split.first;
                node.* = keep.*;
                self_destroy(drop);
                self_destroy(keep);
                return true;
            }
            if (split.second.* == .leaf and split.second.leaf == target) {
                const keep = split.first;
                const drop = split.second;
                node.* = keep.*;
                self_destroy(drop);
                self_destroy(keep);
                return true;
            }
            return removePaneLeafStatic(split.first, target) or removePaneLeafStatic(split.second, target);
        },
        .leaf => return false,
    }
}

fn self_destroy(node: *PaneNode) void {
    @import("std").heap.c_allocator.destroy(node);
}

fn freePaneNode(allocator: std.mem.Allocator, node: *PaneNode) void {
    switch (node.*) {
        .leaf => {},
        .split => |split| {
            freePaneNode(allocator, split.first);
            freePaneNode(allocator, split.second);
            allocator.destroy(split.first);
            allocator.destroy(split.second);
        },
    }
    allocator.destroy(node);
}

fn fileExists(io: std.Io, path: []const u8) bool {
    _ = std.Io.Dir.cwd().statFile(io, path, .{}) catch return false;
    return true;
}

fn pathExists(io: std.Io, path: []const u8) bool {
    _ = std.Io.Dir.cwd().statFile(io, path, .{}) catch return false;
    return true;
}

fn isDirectory(io: std.Io, path: []const u8) bool {
    const stat = std.Io.Dir.cwd().statFile(io, path, .{}) catch return false;
    return stat.kind == .directory;
}

fn validName(name: []const u8) bool {
    if (name.len == 0 or std.mem.eql(u8, name, ".") or std.mem.eql(u8, name, "..")) return false;
    for (name) |byte| {
        if (byte == '/' or byte == '\\' or byte == 0) return false;
    }
    return true;
}

fn uniqueCopyPath(
    allocator: std.mem.Allocator,
    io: std.Io,
    source: []const u8,
    is_directory: bool,
) ![]u8 {
    const parent = std.fs.path.dirname(source) orelse ".";
    const name = sidebar_mod.fileName(source);
    const stem = if (is_directory) name else std.fs.path.stem(name);
    const extension = if (is_directory) "" else std.fs.path.extension(name);
    var attempt: usize = 1;
    while (attempt < 10_000) : (attempt += 1) {
        const candidate_name = if (attempt == 1)
            try std.fmt.allocPrint(allocator, "{s} copy{s}", .{ stem, extension })
        else
            try std.fmt.allocPrint(allocator, "{s} copy {d}{s}", .{ stem, attempt, extension });
        defer allocator.free(candidate_name);
        const candidate = try std.fs.path.join(allocator, &.{ parent, candidate_name });
        if (!pathExists(io, candidate)) return candidate;
        allocator.free(candidate);
    }
    const fallback = try std.fmt.allocPrint(allocator, "{s} copy{s}", .{ stem, extension });
    defer allocator.free(fallback);
    return std.fs.path.join(allocator, &.{ parent, fallback });
}

fn copyPath(
    allocator: std.mem.Allocator,
    io: std.Io,
    source: []const u8,
    target: []const u8,
    is_directory: bool,
) !void {
    if (!is_directory) {
        try std.Io.Dir.copyFileAbsolute(source, target, io, .{});
        return;
    }
    if (sidebar_mod.isInside(source, target) or std.mem.eql(u8, source, target)) {
        return error.InvalidDestination;
    }
    try std.Io.Dir.cwd().createDirPath(io, target);
    var dir = try std.Io.Dir.cwd().openDir(io, source, .{ .iterate = true });
    defer dir.close(io);
    var walker = try dir.walk(allocator);
    defer walker.deinit();
    while (try walker.next(io)) |entry| {
        const child_source = try std.fs.path.join(allocator, &.{ source, entry.path });
        defer allocator.free(child_source);
        const child_target = try std.fs.path.join(allocator, &.{ target, entry.path });
        defer allocator.free(child_target);
        switch (entry.kind) {
            .directory => try std.Io.Dir.cwd().createDirPath(io, child_target),
            .file => try std.Io.Dir.copyFileAbsolute(child_source, child_target, io, .{}),
            else => {},
        }
    }
}

fn collectFiles(
    allocator: std.mem.Allocator,
    io: std.Io,
    directory: []const u8,
    files: *std.ArrayList([]u8),
) void {
    var dir = std.Io.Dir.cwd().openDir(io, directory, .{ .iterate = true }) catch return;
    defer dir.close(io);
    var walker = dir.walk(allocator) catch return;
    defer walker.deinit();
    while (walker.next(io) catch null) |entry| {
        if (entry.kind != .file) continue;
        const path = std.fs.path.join(allocator, &.{ directory, entry.path }) catch continue;
        files.append(allocator, path) catch allocator.free(path);
    }
}

fn readFilePreview(
    allocator: std.mem.Allocator,
    io: std.Io,
    path: []const u8,
) FilePreview {
    var preview = FilePreview{
        .path = allocator.dupe(u8, path) catch &.{},
    };
    if (preview.path.len == 0) return preview;

    const stat = std.Io.Dir.cwd().statFile(io, path, .{}) catch |err| {
        if (err == error.FileNotFound) {
            preview.error_message = allocator.dupe(u8, "file does not exist") catch null;
        } else {
            preview.error_message = std.fmt.allocPrint(allocator, "{s}", .{@errorName(err)}) catch null;
        }
        return preview;
    };
    if (stat.size > preview_max_bytes) {
        preview.error_message = allocator.dupe(u8, "file is too large to preview") catch null;
        return preview;
    }

    const bytes = std.Io.Dir.cwd().readFileAlloc(
        io,
        path,
        allocator,
        .limited(preview_max_bytes + 1),
    ) catch |err| {
        preview.error_message = std.fmt.allocPrint(allocator, "{s}", .{@errorName(err)}) catch null;
        return preview;
    };
    defer allocator.free(bytes);

    var data = bytes;
    if (data.len >= 3 and data[0] == 0xEF and data[1] == 0xBB and data[2] == 0xBF) {
        data = data[3..];
    }
    if (data.len == 0) return preview;

    var newline_count: usize = 0;
    for (data) |byte| {
        if (byte == '\n') newline_count += 1;
    }
    const total_lines = newline_count + @intFromBool(data[data.len - 1] != '\n');
    preview.truncated = total_lines > preview_max_lines;

    var lines = std.mem.splitScalar(u8, data, '\n');
    var index: usize = 0;
    while (index < preview_max_lines) : (index += 1) {
        const raw = lines.next() orelse break;
        var line = raw;
        if (line.len > 0 and line[line.len - 1] == '\r') line = line[0 .. line.len - 1];
        if (std.mem.indexOfScalar(u8, line, 0) != null) {
            preview.binary = true;
            break;
        }
        const copied = previewLineAlloc(allocator, line, &preview.truncated) catch break;
        preview.lines.append(allocator, copied) catch {
            allocator.free(copied);
            break;
        };
    }
    return preview;
}

fn previewLineAlloc(allocator: std.mem.Allocator, line: []const u8, truncated: *bool) ![]u8 {
    var offset: usize = 0;
    var characters: usize = 0;
    while (offset < line.len) {
        if (characters >= preview_max_line_length) {
            const output = try allocator.alloc(u8, offset + 3);
            @memcpy(output[0..offset], line[0..offset]);
            @memcpy(output[offset..], "\u{2026}");
            truncated.* = true;
            return output;
        }
        const decoded = util.decodeAt(line, offset);
        offset += if (decoded.len == 0) 1 else decoded.len;
        characters += 1;
    }
    return allocator.dupe(u8, line);
}

fn pickerScanWorker(ctx: *PickerScanContext) void {
    files_mod.scan(ctx.allocator, ctx.io, &ctx.files, 20000, &ctx.cancel) catch {
        ctx.failed = true;
    };
    ctx.done.store(true, .release);
}

fn pickerSearchWorker(ctx: *PickerSearchContext) void {
    if (ctx.regex) {
        files_mod.searchRegex(
            ctx.allocator,
            ctx.io,
            ctx.files,
            ctx.query,
            &ctx.hits,
            500,
            &ctx.cancel,
            ctx.case_insensitive,
        ) catch {
            ctx.failed = true;
        };
    } else {
        files_mod.search(
            ctx.allocator,
            ctx.io,
            ctx.files,
            ctx.query,
            &ctx.hits,
            500,
            &ctx.cancel,
        ) catch {
            ctx.failed = true;
        };
    }
    ctx.done.store(true, .release);
}
