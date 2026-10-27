//! The C# help window, line for line.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use crate::platform::terminal::{Event, Key};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HelpLineKind { Section, Entry, Text, Blank }

#[derive(Clone, Copy)]
pub struct HelpLine {
    pub kind: HelpLineKind,
    pub left: &'static str,
    pub right: &'static str,
}

const fn section(left: &'static str) -> HelpLine { HelpLine { kind: HelpLineKind::Section, left, right: "" } }
const fn entry(left: &'static str, right: &'static str) -> HelpLine { HelpLine { kind: HelpLineKind::Entry, left, right } }
const fn text(left: &'static str) -> HelpLine { HelpLine { kind: HelpLineKind::Text, left, right: "" } }
const fn blank() -> HelpLine { HelpLine { kind: HelpLineKind::Blank, left: "", right: "" } }

pub const HELP_LINES: &[HelpLine] = &[
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
    entry("\\\\", "Regular-expression search in the current file"),
    entry("'", "Regular-expression search and replace"),
    entry(".", "Select every match for multi-cursor editing"),
    entry("Space /", "Search across all project files"),
    entry("Space .", "Find and replace across matching files"),
    entry("Space \\\\", "Regex find and replace across matching files"),
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
];

pub struct HelpState {
    pub visible: bool,
    pub top: usize,
}

impl HelpState {
    pub fn new() -> HelpState {
        HelpState { visible: false, top: 0 }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.top = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.top = 0;
    }

    pub fn layout(&self, columns: usize, rows: usize) -> (usize, usize, usize, usize, usize) {
        let mut width = columns.saturating_sub(6).clamp(20, 110);
        let mut height = rows.saturating_sub(4).clamp(5, 38);
        width = width.min(columns.saturating_sub(2).max(10));
        height = height.min(rows.saturating_sub(2).max(3));
        let x = columns.saturating_sub(width) / 2;
        let y = rows.saturating_sub(height) / 2;
        (width, height, x, y, height.saturating_sub(2).max(1))
    }

    pub fn max_top(&self, columns: usize, rows: usize) -> usize {
        let (_, _, _, _, list_rows) = self.layout(columns, rows);
        HELP_LINES.len().saturating_sub(list_rows)
    }

    /// Returns true when the key was handled and the screen must repaint.
    pub fn handle(&mut self, event: Event, columns: usize, rows: usize, scroll_lines: usize) -> bool {
        let (_, _, _, _, list_rows) = self.layout(columns, rows);
        match event.key {
            Key::Escape | Key::Enter => {
                self.close();
                true
            }
            Key::Up | Key::ScrollUp => {
                let step = if event.key == Key::ScrollUp { scroll_lines } else { 1 };
                self.top = self.top.saturating_sub(step);
                true
            }
            Key::Down | Key::ScrollDown => {
                let step = if event.key == Key::ScrollDown { scroll_lines } else { 1 };
                self.top = (self.top + step).min(self.max_top(columns, rows));
                true
            }
            Key::PageUp => {
                self.top = self.top.saturating_sub(list_rows);
                true
            }
            Key::PageDown => {
                self.top = (self.top + list_rows).min(self.max_top(columns, rows));
                true
            }
            Key::Home => {
                self.top = 0;
                true
            }
            Key::End => {
                self.top = self.max_top(columns, rows);
                true
            }
            Key::Char if !event.ctrl && !event.alt && matches!(event.ch, 'q' | 'Q') => {
                self.close();
                true
            }
            _ => false,
        }
    }
}
