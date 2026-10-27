//! Palette and named styles. Values are xterm 256-colour indices.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Style {
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub dim: bool,
    pub reverse: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fg: 15,
            bg: 0,
            bold: false,
            dim: false,
            reverse: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyntaxKind {
    Default,
    Keyword,
    Control,
    Type,
    Constant,
    String,
    Comment,
    Number,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub foreground: u32,
    pub background: u32,
    pub muted: u32,
    pub accent: u32,
    pub border: u32,
    pub error: u32,
    pub warning: u32,
    pub selection_fg: u32,
    pub selection_bg: u32,
    pub status_fg: u32,
    pub status_bg: u32,
    pub insert_fg: u32,
    pub insert_bg: u32,
    pub select_fg: u32,
    pub select_bg: u32,
    pub prompt_fg: u32,
    pub prompt_bg: u32,
    pub command_fg: u32,
    pub command_bg: u32,
    pub match_fg: u32,
    pub match_bg: u32,
    pub current_match_fg: u32,
    pub current_match_bg: u32,
    pub keyword: u32,
    pub control: u32,
    pub type_color: u32,
    pub constant: u32,
    pub string: u32,
    pub comment: u32,
    pub number: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            foreground: 15,
            background: 0,
            muted: 8,
            accent: 14,
            border: 8,
            error: 9,
            warning: 11,
            selection_fg: 15,
            selection_bg: 4,
            status_fg: 15,
            status_bg: 4,
            insert_fg: 0,
            insert_bg: 2,
            select_fg: 0,
            select_bg: 3,
            prompt_fg: 0,
            prompt_bg: 6,
            command_fg: 0,
            command_bg: 6,
            match_fg: 0,
            match_bg: 3,
            current_match_fg: 0,
            current_match_bg: 11,
            keyword: 13,
            control: 12,
            type_color: 14,
            constant: 11,
            string: 11,
            comment: 10,
            number: 11,
        }
    }
}

impl Theme {
    pub fn base(&self) -> Style {
        Style {
            fg: self.foreground,
            bg: self.background,
            ..Style::default()
        }
    }

    pub fn text(&self) -> Style {
        self.base()
    }

    pub fn muted_style(&self) -> Style {
        Style {
            fg: self.muted,
            bg: self.background,
            dim: true,
            ..Style::default()
        }
    }

    pub fn accent_style(&self) -> Style {
        Style {
            fg: self.accent,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn border_style(&self) -> Style {
        Style {
            fg: self.border,
            bg: self.background,
            dim: true,
            ..Style::default()
        }
    }

    pub fn error_style(&self) -> Style {
        Style {
            fg: self.error,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn warning_style(&self) -> Style {
        Style {
            fg: self.warning,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn selection_style(&self) -> Style {
        Style {
            fg: self.selection_fg,
            bg: self.selection_bg,
            ..Style::default()
        }
    }

    pub fn selected_item_style(&self) -> Style {
        Style {
            fg: self.selection_fg,
            bg: self.selection_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn status_style(&self) -> Style {
        Style {
            fg: self.status_fg,
            bg: self.status_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn insert_style(&self) -> Style {
        Style {
            fg: self.insert_fg,
            bg: self.insert_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn select_style(&self) -> Style {
        Style {
            fg: self.select_fg,
            bg: self.select_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn prompt_style(&self) -> Style {
        Style {
            fg: self.prompt_fg,
            bg: self.prompt_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn command_style(&self) -> Style {
        Style {
            fg: self.command_fg,
            bg: self.command_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn match_style(&self) -> Style {
        Style {
            fg: self.match_fg,
            bg: self.match_bg,
            ..Style::default()
        }
    }

    pub fn current_match_style(&self) -> Style {
        Style {
            fg: self.current_match_fg,
            bg: self.current_match_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn syntax_style(&self, kind: SyntaxKind) -> Style {
        let fg = match kind {
            SyntaxKind::Keyword => self.keyword,
            SyntaxKind::Control => self.control,
            SyntaxKind::Type => self.type_color,
            SyntaxKind::Constant => self.constant,
            SyntaxKind::String => self.string,
            SyntaxKind::Comment => self.comment,
            SyntaxKind::Number => self.number,
            SyntaxKind::Default => self.foreground,
        };
        Style {
            fg,
            bg: self.background,
            ..Style::default()
        }
    }
}
