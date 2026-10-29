//! Help window contents and scrolling state.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");
const term = @import("terminal.zig");

pub const LineKind = enum { section, entry, text, blank };

pub const Line = struct {
    kind: LineKind,
    left: []const u8,
    right: []const u8 = "",
};

fn section(left: []const u8) Line {
    return .{ .kind = .section, .left = left };
}
fn entry(left: []const u8, right: []const u8) Line {
    return .{ .kind = .entry, .left = left, .right = right };
}
fn text(left: []const u8) Line {
    return .{ .kind = .text, .left = left };
}
fn blank() Line {
    return .{ .kind = .blank, .left = "" };
}

pub const LINES = [_]Line{
    section("Fundamentals"),
    entry("i / I", "Insert before the cursor / at the start of the line"),
    entry("a / A", "Append after the cursor / at the end of the line"),
    entry("o / O", "Open a new line below / above and enter INSERT"),
    entry("Esc", "Return to NORMAL, cancel SELECT, or dismiss a prompt"),
    entry(":", "Open the Ex command line"),
    entry("Ctrl+P", "Open the command palette"),
    entry(":w / :wq / :q", "Write, write and quit, or quit"),
    entry(":q!", "Quit and discard unsaved changes"),
    entry("Ctrl+B", "Toggle and focus the file sidebar"),
    blank(),
    section("Movement"),
    entry("h j k l / arrows", "Move left, down, up, right"),
    entry("w / W", "Next WORD (whitespace-delimited)"),
    entry("q / Q", "Previous WORD (whitespace-delimited)"),
    entry("0 / $  Home / End", "Start / end of line"),
    entry("gg / ge", "First / last line"),
    entry("PageUp / PageDown", "Move one screen up / down"),
    entry("Ctrl+Home / Ctrl+End", "Start / end of document"),
    entry("g p", "Jump to the previous location"),
    entry("Space g", "Go to a line number"),
    blank(),
    section("Selection and editing"),
    entry("v / V", "Start character / line selection"),
    entry("x / X", "Select current line / extend line selection"),
    entry("d / D / Delete", "Delete the character or selection"),
    entry("u / U / Ctrl+R", "Undo / redo"),
    entry("Ctrl+C", "Toggle comment on the selection or line"),
    entry("< / >", "Unindent / indent the selection or current line"),
    entry(":rc", "Remove comments from the active selection"),
    entry("Space y", "Copy the selection to the system clipboard"),
    entry("Space X", "Cut the selection to the system clipboard"),
    entry("Space p / Space P", "Paste at the cursor / on a new line"),
    entry("Ctrl+Space", "Request language-server completions in INSERT"),
    blank(),
    section("Search"),
    entry("/", "Literal search in the current file"),
    entry("\\", "Regular-expression search in the current file"),
    entry("'", "Regular-expression search and replace"),
    entry(".", "Select every match for multi-cursor editing"),
    entry("Space /", "Search across all project files"),
    entry("Space .", "Find and replace across matching files"),
    entry("Space \\", "Regex find and replace across matching files"),
    entry("Space f", "Open the file picker"),
    blank(),
    section("Language server"),
    entry("Space K", "Show hover information at the cursor"),
    entry("Space A", "List code actions at the cursor"),
    entry("Space S", "Pick a symbol from the current document"),
    entry("Space d / Space D", "Show document / workspace diagnostics"),
    entry("Space Z", "Show the diagnostic on the current line"),
    blank(),
    section("Git"),
    entry(":gi", "Toggle git-ignored files in the sidebar and pickers"),
    entry(":gdl", "Toggle added/deleted/modified gutter markers"),
    entry(":gj / :gjp", "Jump to the next / previous Git diff line"),
    blank(),
    section("Sidebar"),
    entry("C", "Open the context menu for the selected item(s)"),
    entry("Ctrl+Left / Ctrl+Right", "Shrink / grow the sidebar"),
    entry("Shift+Up / Shift+Down", "Extend the multi-selection"),
    entry("Enter", "Open a file or expand/collapse a folder"),
    entry("Space", "Leave the sidebar and open the Space menu"),
    entry("O X E", "Menu: open, new folder, new file"),
    entry("R D F", "Menu: rename, duplicate, format"),
    entry("I", "Menu: diff (exactly two files selected)"),
    entry("T M d", "Menu: copy to, move to, delete"),
    blank(),
    section("Panes"),
    entry("Ctrl+W h j k l", "Focus an adjacent split pane"),
    entry("Ctrl+W + / -", "Resize the current pane"),
    entry("Ctrl+W q", "Close the current pane"),
    entry(":vs / :hs", "Split vertically / horizontally"),
    blank(),
    section("Formatting and configuration"),
    entry(":fmt", "Format the buffer, or just the active selection"),
    entry(":fmta <ext>", "Format all non-ignored workspace files with an extension"),
    entry(":ww", "Toggle word wrap (persisted to .hexcfg)"),
    entry(":cw", "Trim trailing whitespace from every line"),
    entry(":cfg / :rcfg", "Open .hexcfg / reload configuration"),
    entry(":rla[!]", "Reload all open files and hard-refresh the interface"),
    entry(":open-log / :ol", "Open the Hexal log (~/.hexal/hexal.log)"),
    entry(":path", "Show the current file's absolute path"),
    entry(":lang <id>", "Override the language for the current buffer"),
    entry(":adi", "Detect and apply the buffer's indentation"),
    blank(),
    section("Debugging"),
    entry("Ctrl+E", "Toggle a breakpoint on the current line"),
    entry(":debug [exe]", "Start a debug session"),
    entry(":dbg-cont", "Continue execution"),
    entry(":dbg-next / :dbg-step", "Step over / step into"),
    entry(":dbg-out / :dbg-pause", "Step out / pause"),
    entry(":dbg-sv", "Show scopes, variables and watches"),
    entry(":dbg-stop", "Stop the debug session"),
    blank(),
    section("Plugins, themes and reload"),
    entry(":rcfg", "Reload grammars, themes, plugins and language servers"),
    entry("Ctrl+P then type", "Search every command, including plugin commands"),
    entry(":help", "Open this help window"),
    blank(),
    text("NOTE: The majority of commands accept lowercase or uppercase names, but arguments"),
    text("keep their exact spelling (for example :E MyFile.cs)."),
};

pub const HelpState = struct {
    visible: bool = false,
    top: usize = 0,

    pub fn open(self: *HelpState) void {
        self.visible = true;
        self.top = 0;
    }

    pub fn close(self: *HelpState) void {
        self.visible = false;
        self.top = 0;
    }

    pub const Layout = struct {
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        rows: usize,
    };

    pub fn layout(self: HelpState, columns: usize, rows: usize) Layout {
        _ = self;
        var width = std.math.clamp(columns -| 6, 20, 110);
        var height = std.math.clamp(rows -| 4, 5, 38);
        width = @min(width, @max(columns -| 2, 10));
        height = @min(height, @max(rows -| 2, 3));
        return .{
            .width = width,
            .height = height,
            .x = (columns -| width) / 2,
            .y = (rows -| height) / 2,
            .rows = @max(height -| 2, 1),
        };
    }

    pub fn maxTop(self: HelpState, columns: usize, rows: usize) usize {
        return LINES.len -| self.layout(columns, rows).rows;
    }

    pub fn handle(self: *HelpState, event: term.Event, columns: usize, rows: usize, scroll_lines: usize) bool {
        const list_rows = self.layout(columns, rows).rows;
        switch (event.key) {
            .escape, .enter => self.close(),
            .up => self.top -|= 1,
            .down => self.top = @min(self.top + 1, self.maxTop(columns, rows)),
            .scroll_up => self.top -|= scroll_lines,
            .scroll_down => self.top = @min(self.top + scroll_lines, self.maxTop(columns, rows)),
            .page_up => self.top -|= list_rows,
            .page_down => self.top = @min(self.top + list_rows, self.maxTop(columns, rows)),
            .home => self.top = 0,
            .end => self.top = self.maxTop(columns, rows),
            .character => {
                if (event.ctrl or event.alt) return false;
                if (event.ch != 'q' and event.ch != 'Q') return false;
                self.close();
            },
            else => return false,
        }
        return true;
    }
};
