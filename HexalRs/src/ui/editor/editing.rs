//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    // ---------------------------------------------------------------------
    // Editing
    // ---------------------------------------------------------------------

    pub(super) fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn enter_insert_from_selection(&mut self) {
        self.delete_selection();
        self.enter_insert();
    }

    pub(super) fn insert_text(&mut self, text: &str) {
        if self.mode == Mode::Select {
            self.delete_selection();
        }
        self.mode = Mode::Insert;
        self.doc.insert(self.cursor, text.as_bytes());
        self.cursor += text.len();
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn insert_newline(&mut self) {
        let line = self.cursor_line();
        let line_start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - line_start).min(text.len());

        let mut indent_end = 0usize;
        while indent_end < local && (text[indent_end] == b' ' || text[indent_end] == b'\t') {
            indent_end += 1;
        }
        let indent = &text[..indent_end];
        let previous = if local > 0 { text[local - 1] } else { 0 };
        let next = if local < text.len() { text[local] } else { 0 };
        let opens = matches!(previous, b'{' | b'(' | b'[' | b':');

        let eol = self.doc.eol.as_bytes();
        let mut builder = Vec::new();
        builder.extend_from_slice(eol);
        builder.extend_from_slice(indent);
        if opens {
            builder.extend_from_slice(b"    ");
        }

        if matches!(next, b'}' | b')' | b']') {
            builder.extend_from_slice(eol);
            builder.extend_from_slice(indent);
            self.doc.insert(self.cursor, &builder);
            let extra = if opens { INDENT_WIDTH } else { 0 };
            self.cursor += eol.len() + indent.len() + extra;
            self.anchor = self.cursor;
            self.preferred_column = None;
            self.mode = Mode::Insert;
            self.snap_cursor();
        } else {
            let text = String::from_utf8_lossy(&builder).into_owned();
            self.insert_text(&text);
        }
    }

    pub(super) fn insert_newline_below(&mut self) {
        let line = self.cursor_line();
        let end = self.doc.line_end(line);
        let text = self.doc.line_text(line);
        let mut indent = 0usize;
        while indent < text.len() && (text[indent] == b' ' || text[indent] == b'\t') {
            indent += 1;
        }
        let eol = self.doc.eol.as_bytes();
        let mut builder = Vec::new();
        builder.extend_from_slice(eol);
        builder.extend_from_slice(&text[..indent]);
        self.doc.insert(end, &builder);
        self.cursor = end + eol.len() + indent;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn insert_newline_above(&mut self) {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let mut indent = 0usize;
        while indent < text.len() && (text[indent] == b' ' || text[indent] == b'\t') {
            indent += 1;
        }
        let mut builder = Vec::new();
        builder.extend_from_slice(&text[..indent]);
        builder.extend_from_slice(self.doc.eol.as_bytes());
        self.doc.insert(start, &builder);
        self.cursor = start + indent;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut previous = self.previous_scalar(self.cursor);
        if previous > 0
            && self.doc.byte_at(previous) == Some(b'\n')
            && self.doc.byte_at(previous - 1) == Some(b'\r')
        {
            previous -= 1;
        }
        self.doc.erase(previous, self.cursor - previous);
        self.cursor = previous;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn delete_forward(&mut self) {
        if self.cursor >= self.doc.size() {
            return;
        }
        let line_end = self.line_end_of_cursor();
        let mut end = self.next_scalar(self.cursor);
        if self.cursor == line_end
            && self.doc.byte_at(self.cursor) == Some(b'\r')
            && self.doc.byte_at(self.cursor + 1) == Some(b'\n')
        {
            end = self.cursor + 2;
        }
        self.doc.erase(self.cursor, end - self.cursor);
        self.snap_cursor();
    }

    pub(super) fn delete_at_cursor(&mut self) {
        if self.mode == Mode::Select {
            self.delete_selection();
        } else {
            self.delete_forward();
        }
    }

    pub(super) fn delete_selection(&mut self) {
        if self.mode != Mode::Select {
            return;
        }
        let (start, end) = self.selection_range();
        if end > start {
            self.doc.replace(start, end - start, b"");
        }
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn indent_selection(&mut self, unindent: bool) {
        if self.mode != Mode::Select {
            self.set_message("Select lines before indenting.");
            return;
        }
        let (range_start, range_end) = self.selection_range();
        let first_line = self.doc.line_of_offset(range_start);
        let mut last_line = self.doc.line_of_offset(range_end);
        if last_line > first_line && range_end == self.doc.line_start(last_line) {
            last_line -= 1;
        }
        for line in first_line..=last_line {
            let start = self.doc.line_start(line);
            if unindent {
                let text = self.doc.line_text(line);
                let mut count = 0usize;
                while count < text.len() && count < INDENT_WIDTH && text[count] == b' ' {
                    count += 1;
                }
                if count == 0 && text.first() == Some(&b'\t') {
                    count = 1;
                }
                if count > 0 {
                    self.doc.erase(start, count);
                }
            } else {
                self.doc.insert(start, b"    ");
            }
        }
        self.doc.break_undo_group();
    }

    // ---------------------------------------------------------------------
    // Selection and clipboard
    // ---------------------------------------------------------------------

    pub(super) fn start_character_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn start_line_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = true;
        self.anchor = self.doc.line_start(self.cursor_line());
        self.cursor = self.anchor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn extend_line_selection(&mut self) {
        if !self.selection_linewise {
            self.selection_linewise = true;
            self.anchor = self.doc.line_start(self.cursor_line());
            self.cursor = self.anchor;
        } else {
            let line = self.cursor_line();
            if line + 1 < self.doc.line_count() {
                self.cursor = self.doc.line_start(line + 1);
            } else {
                self.set_message("Already at the last line.");
            }
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn select_all(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = 0;
        self.cursor = self.doc.size();
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn cancel_selection(&mut self) {
        self.mode = Mode::Normal;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn selection_range(&self) -> (usize, usize) {
        if self.selection_linewise {
            let anchor_line = self.doc.line_of_offset(self.anchor);
            let cursor_line = self.doc.line_of_offset(self.cursor);
            let first = anchor_line.min(cursor_line);
            let last = anchor_line.max(cursor_line);
            let start = self.doc.line_start(first);
            let end = if last + 1 < self.doc.line_count() {
                self.doc.line_start(last + 1)
            } else {
                self.doc.size()
            };
            return (start, end);
        }

        let mut range_start = self.anchor.min(self.cursor);
        let mut range_end = self.anchor.max(self.cursor);
        if range_end < self.doc.size() {
            let line = self.doc.line_of_offset(range_end);
            let line_end = self.doc.line_end(line);
            if range_end < line_end {
                range_end = self.next_scalar(range_end);
            } else if range_end == line_end && line + 1 < self.doc.line_count() {
                range_end = self.doc.line_start(line + 1);
            }
        }
        if range_start > range_end {
            range_start = range_end;
        }
        (range_start, range_end)
    }

    pub(super) fn yank_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to yank.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        if self.mode == Mode::Select {
            self.cursor = start;
            self.cancel_selection();
        }
        self.set_message(&format!("Yanked {} byte(s).", text.len()));
    }

    pub(super) fn cut_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to cut.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        self.doc.replace(start, end - start, b"");
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.set_message(&format!("Cut {} byte(s).", text.len()));
    }

    pub(super) fn store_clipboard(&mut self, text: &[u8]) {
        self.clipboard = String::from_utf8_lossy(text).into_owned();
        let encoded = util::base64_encode(text);
        self.pending_osc = Some(format!("\x1b]52;c;{encoded}\x07").into_bytes());
    }

    pub(super) fn paste_clipboard(&mut self, new_line: bool) {
        if self.clipboard.is_empty() {
            self.set_error("clipboard is empty");
            return;
        }
        let text = self.clipboard.clone();
        if new_line {
            let line = self.cursor_line();
            let end = self.doc.line_end(line);
            let mut builder = Vec::new();
            builder.extend_from_slice(self.doc.eol.as_bytes());
            builder.extend_from_slice(text.as_bytes());
            self.doc.insert(end, &builder);
            self.cursor = (end + builder.len()).min(self.doc.size());
        } else if self.mode == Mode::Select {
            let (start, end) = self.selection_range();
            self.doc.replace(start, end - start, text.as_bytes());
            self.cursor = (start + text.len()).min(self.doc.size());
        } else {
            self.doc.insert(self.cursor, text.as_bytes());
            self.cursor += text.len();
        }
        self.mode = Mode::Normal;
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.snap_cursor();
        self.set_message(&format!("Pasted {} byte(s).", text.len()));
    }
}
