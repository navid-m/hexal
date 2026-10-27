//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    // ---------------------------------------------------------------------
    // Find in file
    // ---------------------------------------------------------------------

    pub(super) fn open_find(&mut self) {
        self.find.active = true;
        self.find.editing = true;
        self.find.query.clear();
        self.find.matches.clear();
        self.find.index = None;
        self.find.origin = self.cursor;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn close_find(&mut self) {
        self.find.active = false;
        self.find.editing = false;
        self.find.query.clear();
        self.find.matches.clear();
        self.find.index = None;
        self.doc.break_undo_group();
    }

    pub(super) fn rebuild_find_matches(&mut self) {
        self.find.matches.clear();
        if self.find.query.is_empty() {
            self.find.index = None;
            return;
        }
        let data = self.doc.all_text();
        let needle = self.find.query.as_bytes();
        let mut search_from = 0usize;
        while search_from < data.len() && self.find.matches.len() < 20_000 {
            let Some(relative) =
                util::index_of(&data[search_from..], needle, self.search_match_case)
            else {
                break;
            };
            let start = search_from + relative;
            let end = start + needle.len();
            self.find.matches.push((start, end));
            search_from = if end > start { end } else { start + 1 };
        }

        if self.find.matches.is_empty() {
            self.find.index = None;
            return;
        }
        let index = self
            .find
            .matches
            .iter()
            .position(|&(start, _)| start >= self.find.origin)
            .unwrap_or(0);
        self.find.index = Some(index);
    }

    pub(super) fn toggle_search_match_case(&mut self) {
        self.search_match_case = !self.search_match_case;
        self.persist_display_settings();

        if self.find.active {
            self.rebuild_find_matches();
        }
        if self.picker.kind == PickerKind::Search {
            self.rebuild_picker();
        }

        self.force_full_redraw = true;
        self.set_message(if self.search_match_case {
            "Search is case-sensitive."
        } else {
            "Search is case-insensitive."
        });
    }

    pub(super) fn handle_find(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.close_find();
            return;
        }

        if !self.find.editing {
            match event.key {
                Key::Enter => self.find.editing = true,
                Key::Up => self.find_step(-1),
                Key::Down => self.find_step(1),
                Key::Char => match event.ch {
                    'n' | 'N' => self.find_step(1),
                    'p' | 'P' => self.find_step(-1),
                    '/' => self.find.editing = true,
                    _ => {}
                },
                _ => {}
            }
            return;
        }

        match event.key {
            Key::Enter => {
                self.find.editing = false;
                if self.find.index.is_some() {
                    self.jump_to_find_match();
                }
            }
            Key::Up => self.find_step(-1),
            Key::Down => self.find_step(1),
            Key::Backspace => {
                self.find.query.pop();
                self.rebuild_find_matches();
            }
            Key::Char => {
                if event.ctrl || event.alt || event.ch == '\0' {
                    return;
                }
                self.find.query.push(event.ch);
                self.rebuild_find_matches();
            }
            _ => {}
        }
    }

    pub(super) fn find_step(&mut self, delta: isize) {
        if self.find.matches.is_empty() {
            return;
        }
        let length = self.find.matches.len() as isize;
        let current = self.find.index.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(length) as usize;
        self.find.index = Some(next);
        self.jump_to_find_match();
    }

    pub(super) fn jump_to_find_match(&mut self) {
        let Some(index) = self.find.index else {
            return;
        };
        let Some(&(start, _)) = self.find.matches.get(index) else {
            return;
        };
        self.cursor = start;
        self.anchor = start;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.doc.break_undo_group();
    }
}
