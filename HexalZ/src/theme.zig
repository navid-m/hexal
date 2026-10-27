//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub const Style = struct {
    fg: u32 = 7,
    bg: u32 = 0,
    bold: bool = false,
    dim: bool = false,
    reverse: bool = false,

    pub fn eql(a: Style, b: Style) bool {
        return a.fg == b.fg and a.bg == b.bg and a.bold == b.bold and
            a.dim == b.dim and a.reverse == b.reverse;
    }

    pub fn withBg(self: Style, bg: u32) Style {
        var result = self;
        result.bg = bg;
        return result;
    }
};

/// Palette and named styles matching the default palette used by HexalCs.
pub const Theme = struct {
    foreground: u32 = 15,
    background: u32 = 0,
    muted: u32 = 8,
    dimmed: u32 = 8,
    accent: u32 = 14,
    border: u32 = 8,
    err_color: u32 = 9,
    warning: u32 = 11,
    success: u32 = 10,

    selection_foreground: u32 = 15,
    selection_background: u32 = 4,
    status_foreground: u32 = 15,
    status_background: u32 = 4,
    insert_foreground: u32 = 0,
    insert_background: u32 = 2,
    select_foreground: u32 = 0,
    select_background: u32 = 3,
    prompt_foreground: u32 = 0,
    prompt_background: u32 = 6,
    command_foreground: u32 = 0,
    command_background: u32 = 6,
    match_foreground: u32 = 0,
    match_background: u32 = 3,
    current_match_foreground: u32 = 0,
    current_match_background: u32 = 11,

    keyword: u32 = 13,
    control: u32 = 12,
    type_: u32 = 14,
    constant: u32 = 11,
    string: u32 = 11,
    comment: u32 = 10,
    number: u32 = 11,

    pub fn base(self: Theme) Style {
        return .{ .fg = self.foreground, .bg = self.background };
    }

    pub fn text(self: Theme) Style {
        return self.base();
    }

    pub fn mutedStyle(self: Theme) Style {
        return .{ .fg = self.muted, .bg = self.background, .dim = true };
    }

    pub fn accentStyle(self: Theme) Style {
        return .{ .fg = self.accent, .bg = self.background, .bold = true };
    }

    pub fn borderStyle(self: Theme) Style {
        return .{ .fg = self.border, .bg = self.background, .dim = true };
    }

    pub fn errorStyle(self: Theme) Style {
        return .{ .fg = self.err_color, .bg = self.background, .bold = true };
    }

    pub fn warningStyle(self: Theme) Style {
        return .{ .fg = self.warning, .bg = self.background, .bold = true };
    }

    pub fn selectionStyle(self: Theme) Style {
        return .{ .fg = self.selection_foreground, .bg = self.selection_background };
    }

    pub fn selectedItemStyle(self: Theme) Style {
        return .{ .fg = self.selection_foreground, .bg = self.selection_background, .bold = true };
    }

    pub fn statusStyle(self: Theme) Style {
        return .{ .fg = self.status_foreground, .bg = self.status_background, .bold = true };
    }

    pub fn insertStatusStyle(self: Theme) Style {
        return .{ .fg = self.insert_foreground, .bg = self.insert_background, .bold = true };
    }

    pub fn selectStatusStyle(self: Theme) Style {
        return .{ .fg = self.select_foreground, .bg = self.select_background, .bold = true };
    }

    pub fn promptStatusStyle(self: Theme) Style {
        return .{ .fg = self.prompt_foreground, .bg = self.prompt_background, .bold = true };
    }

    pub fn commandStatusStyle(self: Theme) Style {
        return .{ .fg = self.command_foreground, .bg = self.command_background, .bold = true };
    }

    pub fn matchStyle(self: Theme) Style {
        return .{ .fg = self.match_foreground, .bg = self.match_background };
    }

    pub fn currentMatchStyle(self: Theme) Style {
        return .{
            .fg = self.current_match_foreground,
            .bg = self.current_match_background,
            .bold = true,
        };
    }

    pub fn syntaxStyle(self: Theme, kind: SyntaxKind) Style {
        const fg = switch (kind) {
            .keyword => self.keyword,
            .control => self.control,
            .type_ => self.type_,
            .constant => self.constant,
            .string => self.string,
            .comment => self.comment,
            .number => self.number,
            .default => self.foreground,
        };
        return .{ .fg = fg, .bg = self.background };
    }
};

pub const SyntaxKind = enum {
    default,
    keyword,
    control,
    type_,
    constant,
    string,
    comment,
    number,
};
