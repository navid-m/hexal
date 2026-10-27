//! The about overlay, matching the C# branding screen.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::{COMMIT_HASH, VERSION};

const ABOUT_HEADER_ROWS: usize = 4;
const ABOUT_HINT_ROWS: usize = 1;
const ABOUT_SCROLL_LINES: usize = 3;

const ABOUT_BANNER: &str = r#"                                   ====
                               .=== -- -==.
                            :==-    --    -==-
                         ===:       --       :===
                      ===.          ==           ===
                   ===           ===  ===           ===
               .===           ===        ===.          ===.
            :==-          :==-              ===:          ===-
         ===-          -==-                    -==-          :==-
      ===.          ===:                          .===          .===
   ===           ===. =                               ===           ===
  ===         ===     =              .                   ===.        ===
  =  ===. .===        =           -==-                      -==: .===  =
  =     -=-          ==        -==: --                         -=-     =
  =      =       .===:=       =:    --                          =      =
  =      =     -==    =       =     -=========-     -========-  =      =
  =      =     -=     =       =     -===.     =-   ==     :=.   =      =
  =      =     -=     =       =     --  =.     == ==     :=     =      =
  =      =     -=     =       =     --   =:     ==-     =+      =      =
  =      =     -=     =       =     --    ==     .     ==       =      =
  =      =     -=     =========     --     ==         +-        =      =
  =      =     -=                   --      -=       =-         =      =
  =      =     -=                   --      -=       =-         =      =
  =      =     -=     =========     --     ==         =-        =      =
  =      =     -=     =       =     -=    ==           ==       =      =
  =      =     -=     =       =     --   +:     +=-     =+      =      =
  =      =     -=     =       =     =- .=.     == ==     :=     =      =
  =      =     -=     =       =     -===.     +-   ==     :=.   =      =
  =      =     -==    =       =    -==========-     -========-  =      =
  =      =       .===:=       =.-==-                            =      =
  =     -=-          ==       ==.                              -=-     =
  =  -==. :==-        =                                     -==: .===  =
  ===        .===     =          ===                     ===.        ===
   ===           ===  =             ==                ===           ===
      ===.          ===.          ==-             .===          .===
         ===:          -==:      =    .====.   .==-          :===
            -==-          :==-              -==-          -==-
               .===          .===        ===:          -==:
                   ===           ===  ===           ===.
                      ===           ==           ===
                         ===:       --       .===
                            -==-    --    :===
                               :==- -- -==:
                                  .====."#;

pub(super) struct AboutState {
    pub(super) visible: bool,
    top: usize,
    left: usize,
}

impl AboutState {
    pub(super) fn new() -> AboutState {
        AboutState {
            visible: false,
            top: 0,
            left: 0,
        }
    }

    fn banner_line_count() -> usize {
        ABOUT_BANNER.lines().count()
    }

    fn banner_width() -> usize {
        ABOUT_BANNER
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
    }

    fn viewport_rows(&self, rows: usize) -> usize {
        let available = rows.saturating_sub(ABOUT_HINT_ROWS).max(1);
        let header_rows = ABOUT_HEADER_ROWS.min(available);
        available.saturating_sub(header_rows).max(1)
    }

    fn max_top(&self, rows: usize) -> usize {
        Self::banner_line_count().saturating_sub(self.viewport_rows(rows))
    }

    fn max_left(&self, columns: usize) -> usize {
        Self::banner_width().saturating_sub(columns)
    }

    fn center(&mut self, columns: usize, rows: usize) {
        self.top = Self::banner_line_count().saturating_sub(self.viewport_rows(rows)) / 2;
        self.left = Self::banner_width().saturating_sub(columns) / 2;
    }
}

impl Editor {
    pub(super) fn open_about(&mut self) {
        self.clear_message();
        self.about.visible = true;
        self.about.top = 0;
        self.about.left = 0;
        self.about.center(self.cols, self.rows);
        self.force_full_redraw = true;
    }

    fn close_about(&mut self) {
        self.about.visible = false;
        self.about.top = 0;
        self.about.left = 0;
        self.force_full_redraw = true;
    }

    pub(super) fn handle_about_input(&mut self, event: Event) {
        let max_top = self.about.max_top(self.rows);
        let max_left = self.about.max_left(self.cols);

        match event.key {
            Key::Escape | Key::Enter => {
                self.close_about();
                return;
            }
            Key::Up | Key::ScrollUp => {
                let step = if event.key == Key::ScrollUp {
                    ABOUT_SCROLL_LINES
                } else {
                    1
                };
                self.about.top = self.about.top.saturating_sub(step);
                return;
            }
            Key::Down | Key::ScrollDown => {
                let step = if event.key == Key::ScrollDown {
                    ABOUT_SCROLL_LINES
                } else {
                    1
                };
                self.about.top = (self.about.top + step).min(max_top);
                return;
            }
            Key::Left => {
                self.about.left = self.about.left.saturating_sub(1);
                return;
            }
            Key::Right => {
                self.about.left = (self.about.left + 1).min(max_left);
                return;
            }
            Key::PageUp => {
                let step = self.about.viewport_rows(self.rows);
                self.about.top = self.about.top.saturating_sub(step);
                return;
            }
            Key::PageDown => {
                let step = self.about.viewport_rows(self.rows);
                self.about.top = (self.about.top + step).min(max_top);
                return;
            }
            Key::Home => {
                self.about.top = 0;
                self.about.left = 0;
                return;
            }
            Key::End => {
                self.about.top = max_top;
                self.about.left = max_left;
                return;
            }
            _ => {}
        }

        if event.key == Key::Char && !event.ctrl && !event.alt && matches!(event.ch, 'q' | 'Q') {
            self.close_about();
        }
    }

    pub(super) fn render_about(&mut self) {
        if !self.about.visible {
            return;
        }

        self.screen.clear(self.theme.base());

        let available = self.rows.saturating_sub(ABOUT_HINT_ROWS).max(1);
        let viewport_rows = self.about.viewport_rows(self.rows);
        self.about.top = self.about.top.min(self.about.max_top(self.rows));

        let line_count = AboutState::banner_line_count();
        let banner_width = AboutState::banner_width();
        let content_height = ABOUT_HEADER_ROWS + line_count;
        let header_y = if content_height <= available {
            (available - content_height) / 2
        } else {
            0
        };

        self.draw_centered_line(
            header_y,
            &format!("Hexal - {VERSION} ({COMMIT_HASH})"),
            self.theme.accent_style(),
        );
        self.draw_centered_line(
            header_y + 1,
            "Author: Navid Momtahen (C) 2026",
            self.theme.muted_style(),
        );

        self.about.left = self.about.left.min(self.about.max_left(self.cols));
        let art_x = if banner_width <= self.cols {
            (self.cols - banner_width) / 2
        } else {
            0
        };
        let art_y = header_y + ABOUT_HEADER_ROWS;

        for row in 0..viewport_rows {
            let index = self.about.top + row;
            if index >= line_count {
                break;
            }
            let line = ABOUT_BANNER.lines().nth(index).unwrap_or("");
            let column = self.about.left.min(line.chars().count());
            let visible: String = line.chars().skip(column).collect();
            self.screen.put_str(
                art_x as i64,
                (art_y + row) as i64,
                &visible,
                self.theme.text(),
                self.cols.saturating_sub(art_x),
            );
        }

        let hint = self.about_hint();
        self.draw_centered_line(self.rows.saturating_sub(1), &hint, self.theme.muted_style());
    }

    fn about_hint(&self) -> String {
        let mut parts = vec!["Esc close"];
        if AboutState::banner_line_count() > self.about.viewport_rows(self.rows) {
            parts.push("Up/Down scroll");
        }
        if AboutState::banner_width() > self.cols {
            parts.push("Left/Right pan");
        }
        parts.join("  |  ")
    }

    fn draw_centered_line(&mut self, y: usize, text: &str, style: crate::ui::theme::Style) {
        if y >= self.rows {
            return;
        }

        let width = util::text_width(text);
        let x = self.cols.saturating_sub(width) / 2;
        self.screen
            .put_str(x as i64, y as i64, text, style, self.cols.saturating_sub(x));
    }
}
