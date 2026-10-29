//! Find in file: literal search, regex find/replace and multi-cursor search.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

const MAX_FIND_MATCHES: usize = 20_000;

impl Editor {
    // ---------------------------------------------------------------------
    // Find in file
    // ---------------------------------------------------------------------

    pub(super) fn open_find(&mut self, multi_cursor: bool, regex: bool, replace: bool) {
        self.find.active = true;
        self.find.editing = true;
        self.find.multi_cursor = multi_cursor;
        self.find.regex = regex;
        self.find.replace = replace && regex;
        self.find.editing_replacement = false;
        self.find.error.clear();
        self.find.saved_selections = None;
        self.find.query.clear();
        self.find.query_cursor = 0;
        self.find.replacement.clear();
        self.find.replacement_cursor = 0;
        self.find.scope.clear();
        self.find.matches.clear();
        self.find.index = None;

        // A multi-cursor search started from SELECT mode only considers
        // matches fully contained in the active selection, so replacing with
        // Delete/I cannot affect text outside it. Keep the original selection
        // so cancelling the dialog can restore it.
        if multi_cursor && self.mode == Mode::Select {
            let selections = self.capture_all_selections();
            self.find.scope = self.find_scope(&selections);
            self.find.saved_selections = Some(selections);
        }

        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn close_find(&mut self) {
        let saved_selections = self.find.saved_selections.take();

        self.find.active = false;
        self.find.editing = false;
        self.find.multi_cursor = false;
        self.find.regex = false;
        self.find.replace = false;
        self.find.editing_replacement = false;
        self.find.error.clear();
        self.find.query.clear();
        self.find.query_cursor = 0;
        self.find.replacement.clear();
        self.find.replacement_cursor = 0;
        self.find.scope.clear();
        self.find.matches.clear();
        self.find.index = None;

        if let Some(selections) = saved_selections {
            self.set_selections(selections);
            self.mode = Mode::Select;
        }

        self.doc.break_undo_group();
    }

    fn find_scope(&self, selections: &[Selection]) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        for selection in selections {
            let (start, end) =
                self.selection_range_of(selection.anchor, selection.cursor, selection.linewise);
            if end > start {
                ranges.push((start, end));
            }
        }

        ranges.sort_by_key(|&(start, _)| start);
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
                continue;
            }
            merged.push((start, end));
        }
        merged
    }

    fn in_find_scope(&self, start: usize, end: usize) -> bool {
        if self.find.scope.is_empty() {
            return true;
        }
        let scope = &self.find.scope;
        let mut low = 0usize;
        let mut high = scope.len();
        let mut candidate = None;
        while low < high {
            let middle = low + (high - low) / 2;
            if scope[middle].0 <= start {
                candidate = Some(middle);
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        candidate.is_some_and(|index| end <= scope[index].1)
    }

    fn find_text(&self) -> std::borrow::Cow<'_, [u8]> {
        match self.doc.borrowed_slice() {
            Some(text) => std::borrow::Cow::Borrowed(text),
            None => std::borrow::Cow::Owned(self.doc.all_text()),
        }
    }

    fn active_find_field(&self) -> &String {
        if self.find.replace && self.find.editing_replacement {
            &self.find.replacement
        } else {
            &self.find.query
        }
    }

    fn active_find_cursor(&self) -> usize {
        if self.find.replace && self.find.editing_replacement {
            self.find.replacement_cursor
        } else {
            self.find.query_cursor
        }
    }

    fn set_active_find_cursor(&mut self, value: usize) {
        if self.find.replace && self.find.editing_replacement {
            self.find.replacement_cursor = value;
        } else {
            self.find.query_cursor = value;
        }
    }

    fn active_find_field_mut(&mut self) -> &mut String {
        if self.find.replace && self.find.editing_replacement {
            &mut self.find.replacement
        } else {
            &mut self.find.query
        }
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

        if matches!(event.key, Key::Tab | Key::BackTab) && self.find.replace {
            self.find.editing_replacement = !self.find.editing_replacement;
            return;
        }

        match event.key {
            Key::Enter => {
                if self.find.replace {
                    if !self.find.editing_replacement {
                        self.find.editing_replacement = true;
                        return;
                    }
                    self.replace_current_find_match();
                    return;
                }

                if self.find.multi_cursor {
                    self.apply_multi_cursor_find();
                    return;
                }
                self.find.editing = false;
            }
            Key::Up => self.find_step(-1),
            Key::Down => self.find_step(1),
            Key::Backspace => {
                let cursor = self.active_find_cursor();
                if cursor == 0 {
                    return;
                }
                let previous = util::previous_boundary(self.active_find_field().as_bytes(), cursor);
                self.active_find_field_mut()
                    .replace_range(previous..cursor, "");
                self.set_active_find_cursor(previous);
                self.rebuild_find_matches();
            }
            Key::Delete => {
                let cursor = self.active_find_cursor();
                if cursor >= self.active_find_field().len() {
                    return;
                }
                let next = util::next_boundary(self.active_find_field().as_bytes(), cursor);
                self.active_find_field_mut().replace_range(cursor..next, "");
                self.rebuild_find_matches();
            }
            Key::Left => {
                let cursor = self.active_find_cursor();
                let previous = util::previous_boundary(self.active_find_field().as_bytes(), cursor);
                self.set_active_find_cursor(previous);
            }
            Key::Right => {
                let cursor = self.active_find_cursor();
                let next = util::next_boundary(self.active_find_field().as_bytes(), cursor);
                self.set_active_find_cursor(next);
            }
            Key::Home => self.set_active_find_cursor(0),
            Key::End => {
                let length = self.active_find_field().len();
                self.set_active_find_cursor(length);
            }
            Key::Char => {
                if event.ctrl {
                    match event.ch {
                        'n' | 'N' => self.find_step(1),
                        'p' | 'P' => self.find_step(-1),
                        'a' | 'A' if self.find.replace => self.replace_all_find_matches(),
                        _ => {}
                    }
                    return;
                }
                if event.alt || event.ch < '\u{20}' || event.ch == '\u{7f}' {
                    return;
                }
                let cursor = self.active_find_cursor();
                self.active_find_field_mut().insert(cursor, event.ch);
                self.set_active_find_cursor(cursor + event.ch.len_utf8());
                self.rebuild_find_matches();
            }
            _ => {}
        }
    }

    pub(super) fn find_step(&mut self, delta: isize) {
        if self.find.matches.is_empty() {
            self.rebuild_find_matches();
            return;
        }
        let length = self.find.matches.len() as isize;
        let current = self.find.index.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(length) as usize;
        self.select_find_match(next);
    }

    fn select_find_match(&mut self, index: usize) {
        let Some(match_) = self.find.matches.get(index) else {
            self.find.index = None;
            return;
        };
        self.find.index = Some(index);
        self.cursor = match_.start;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn rebuild_find_matches(&mut self) {
        self.find.matches.clear();
        self.find.index = None;
        self.find.error.clear();

        let query = self.find.query.clone();
        if query.is_empty() {
            return;
        }

        if self.find.regex {
            self.rebuild_regex_find_matches(&query);
        } else {
            self.rebuild_literal_find_matches(&query);
        }

        if self.find.matches.is_empty() {
            return;
        }

        let selected = self
            .find
            .matches
            .iter()
            .position(|match_| match_.start >= self.cursor)
            .unwrap_or(0);
        self.select_find_match(selected);
    }

    fn rebuild_literal_find_matches(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        let needle = query.as_bytes();
        let match_case = self.search_match_case;
        let text = self.find_text();
        let line_count = self.doc.line_count();
        let mut matches = Vec::new();
        let mut line = 0usize;
        let mut line_start = 0usize;

        'lines: for raw_line in text.split(|&byte| byte == b'\n') {
            let line_text = if line + 1 < line_count && raw_line.last() == Some(&b'\r') {
                &raw_line[..raw_line.len() - 1]
            } else {
                raw_line
            };
            let mut search_start = 0usize;
            while search_start < line_text.len() {
                let Some(relative) =
                    util::index_of(&line_text[search_start..], needle, match_case)
                else {
                    break;
                };
                let index = search_start + relative;
                let match_length = needle.len().min(line_text.len() - index);
                let start = line_start + index;
                let end = start + match_length;

                if self.in_find_scope(start, end) {
                    matches.push(FindMatch {
                        start,
                        end,
                        line,
                        replacement: String::new(),
                    });
                    if matches.len() >= MAX_FIND_MATCHES {
                        break 'lines;
                    }
                }

                search_start = index + match_length.max(1);
            }

            line += 1;
            line_start += raw_line.len() + 1;
        }

        self.find.matches = matches;
    }

    fn rebuild_regex_find_matches(&mut self, pattern: &str) {
        let regex = match regex::RegexBuilder::new(pattern)
            .case_insensitive(!self.search_match_case)
            .size_limit(2 * 1024 * 1024)
            .build()
        {
            Ok(regex) => regex,
            Err(error) => {
                self.find.error = error.to_string();
                return;
            }
        };

        let replacement_pattern = self.find.replacement.clone();
        let text = self.find_text();
        let line_count = self.doc.line_count();
        let mut matches = Vec::new();
        let mut line = 0usize;
        let mut line_start = 0usize;

        'lines: for raw_line in text.split(|&byte| byte == b'\n') {
            let line_text = if line + 1 < line_count && raw_line.last() == Some(&b'\r') {
                &raw_line[..raw_line.len() - 1]
            } else {
                raw_line
            };
            let Ok(line_text) = std::str::from_utf8(line_text) else {
                line += 1;
                line_start += raw_line.len() + 1;
                continue;
            };

            for captures in regex.captures_iter(line_text) {
                let Some(match_) = captures.get(0) else {
                    continue;
                };
                let start = line_start + match_.start();
                let end = line_start + match_.end();
                let replacement = if self.find.replace {
                    let mut expanded = String::new();
                    captures.expand(&replacement_pattern, &mut expanded);
                    expanded
                } else {
                    String::new()
                };

                if !self.in_find_scope(start, end) {
                    continue;
                }

                matches.push(FindMatch {
                    start,
                    end,
                    line,
                    replacement,
                });
                if matches.len() >= MAX_FIND_MATCHES {
                    break 'lines;
                }
            }

            line += 1;
            line_start += raw_line.len() + 1;
        }

        self.find.matches = matches;
    }

    fn replace_current_find_match(&mut self) {
        if !self.find.error.is_empty() || self.find.matches.is_empty() {
            if self.find.matches.is_empty() {
                self.set_message("No matches to replace.");
            }
            return;
        }

        let index = self
            .find
            .index
            .filter(|index| *index < self.find.matches.len())
            .unwrap_or(0);
        let match_ = self.find.matches[index].clone();
        let replacement = match_.replacement.as_bytes();

        self.doc.break_undo_group();
        self.doc
            .replace(match_.start, match_.end - match_.start, replacement);
        self.doc.break_undo_group();

        self.cursor = (match_.start + replacement.len()).min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.snap_cursor();
        self.rebuild_find_matches();
        self.set_message(&format!(
            "Replaced 1 match ({} remaining).",
            self.find.matches.len()
        ));
    }

    fn replace_all_find_matches(&mut self) {
        if !self.find.error.is_empty() || self.find.matches.is_empty() {
            if self.find.matches.is_empty() {
                self.set_message("No matches to replace.");
            }
            return;
        }

        let count = self.find.matches.len();
        let first_start = self.find.matches[0].start;
        let replacements: Vec<(usize, usize, String)> = self
            .find
            .matches
            .iter()
            .map(|match_| (match_.start, match_.end, match_.replacement.clone()))
            .collect();

        self.doc.begin_composite_edit();
        for (start, end, replacement) in replacements.iter().rev() {
            self.doc
                .replace(*start, end - start, replacement.as_bytes());
        }
        self.doc.end_composite_edit();
        self.doc.break_undo_group();

        self.cursor = first_start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.snap_cursor();
        self.rebuild_find_matches();
        self.set_message(&format!(
            "Replaced {count} match{}.",
            if count == 1 { "" } else { "es" }
        ));
    }

    /// Turns every current find match into a selection and leaves FIND mode.
    /// The match nearest the cursor becomes the primary selection so the
    /// viewport stays where the user was searching.
    fn apply_multi_cursor_find(&mut self) {
        if self.find.matches.is_empty() {
            let query = self.find.query.clone();
            self.close_find();
            self.set_message(if query.is_empty() {
                "No search query."
            } else {
                "No matches for the search."
            });
            return;
        }

        let primary_index = self
            .find
            .index
            .filter(|index| *index < self.find.matches.len())
            .unwrap_or(0);

        let mut selections = Vec::with_capacity(self.find.matches.len());
        selections.push(self.create_find_selection(&self.find.matches[primary_index]));
        for (index, match_) in self.find.matches.iter().enumerate() {
            if index != primary_index {
                selections.push(self.create_find_selection(match_));
            }
        }

        let count = selections.len();
        self.close_find();
        self.set_selections(selections);
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.set_message(&format!("{count} matching selections."));
    }

    fn create_find_selection(&self, match_: &FindMatch) -> Selection {
        Selection {
            cursor: self.previous_scalar(match_.end.max(match_.start)),
            anchor: match_.start,
            linewise: false,
            preferred_column: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;

    fn editor(text: &[u8]) -> Editor {
        Editor::new(Document::from_bytes(None, text.to_vec()), 80, 24)
    }

    #[test]
    fn multi_cursor_find_selects_every_match() {
        let mut editor = editor(b"foo bar foo\nbaz foo\n");
        editor.open_find(true, false, false);
        editor.find.query = "foo".into();
        editor.rebuild_find_matches();
        assert_eq!(editor.find.matches.len(), 3);
        editor.apply_multi_cursor_find();

        assert!(matches!(editor.mode, Mode::Select));
        assert_eq!(editor.extra_selections.len(), 2);
        editor.for_each_selection(|editor| editor.delete_selection(), true);
        assert_eq!(editor.doc.all_text(), b" bar \nbaz \n");
    }

    #[test]
    fn regex_find_and_replace_expands_capture_groups() {
        let mut editor = editor(b"let one = 1;\nlet two = 2;\n");
        editor.open_find(false, true, true);
        editor.find.query = r"let (\w+) = (\d+);".into();
        editor.find.replacement = "$1=$2".into();
        editor.rebuild_find_matches();
        assert_eq!(editor.find.matches.len(), 2);
        editor.replace_all_find_matches();
        assert_eq!(editor.doc.all_text(), b"one=1\ntwo=2\n");
    }

    #[test]
    fn multi_cursor_find_can_be_scoped_to_a_selection() {
        let mut editor = editor(b"foo\nfoo\nfoo\n");
        editor.mode = Mode::Select;
        editor.anchor = editor.doc.line_start(1);
        editor.cursor = editor.doc.line_start(2);
        editor.open_find(true, false, false);
        editor.find.query = "foo".into();
        editor.rebuild_find_matches();
        assert_eq!(editor.find.matches.len(), 1);
        assert_eq!(editor.find.matches[0].line, 1);
    }

    #[test]
    fn find_scrolls_the_match_above_the_dialog() {
        let text = (0..60)
            .map(|line| {
                if line == 30 {
                    "needle".to_string()
                } else {
                    format!("line {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = editor(text.as_bytes());
        editor.open_find(false, false, false);
        editor.find.query = "needle".into();
        editor.rebuild_find_matches();
        assert_eq!(editor.find.index, Some(0));
        assert_eq!(editor.cursor_line(), 30);

        editor.ensure_cursor_visible();
        let find_top = editor.status_row() - editor.find_dialog_height();
        assert!(editor.top_line > 0);
        assert!(editor.cursor_line() >= editor.top_line);
        assert!(editor.cursor_line() - editor.top_line < find_top);
    }

    #[test]
    fn find_keeps_the_last_line_above_the_dialog_after_rendering() {
        let text = (0..20)
            .map(|line| {
                if line == 19 {
                    "needle".to_string()
                } else {
                    format!("line {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = editor(text.as_bytes());
        editor.screen.resize(editor.cols, editor.rows);
        editor.open_find(false, false, false);
        editor.find.query = "needle".into();
        editor.rebuild_find_matches();
        editor.ensure_cursor_visible();
        let top_line = editor.top_line;
        editor.render_panes();

        let find_top = editor.status_row() - editor.find_dialog_height();
        assert_eq!(editor.top_line, top_line);
        assert!(editor.cursor_line() >= editor.top_line);
        assert!(editor.cursor_line() - editor.top_line < find_top);
    }
}
