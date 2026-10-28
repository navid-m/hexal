//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn cursor_line(&self) -> usize {
        self.doc
            .line_of_offset(self.cursor)
            .min(self.doc.line_count() - 1)
    }

    pub(super) fn line_end_of_cursor(&self) -> usize {
        self.doc.line_end(self.cursor_line())
    }

    pub(super) fn cursor_visual_column(&self) -> usize {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        util::visual_column(&text, local, self.doc.tab_width().max(1))
    }

    fn line_and_column_of_offset(&self, offset: usize) -> (usize, usize) {
        let line = self.doc.line_of_offset(offset);
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = offset.saturating_sub(start).min(text.len());
        (
            line,
            util::visual_column(&text, local, self.doc.tab_width().max(1)),
        )
    }

    fn offset_for_line_and_column(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.doc.line_count().saturating_sub(1));
        let text = self.doc.line_text(line);
        let mut local =
            util::byte_offset_for_visual_column(&text, column, self.doc.tab_width().max(1))
                .min(text.len());
        // Formatters often change indentation widths. If the restored column
        // falls inside whitespace, step back over it so typing there does not
        // inject characters into the indentation.
        if local < text.len() && matches!(text[local], b' ' | b'\t') {
            while local > 0 && matches!(text[local - 1], b' ' | b'\t') {
                local -= 1;
            }
        }
        (self.doc.line_start(line) + local).min(self.doc.size())
    }

    pub(super) fn capture_cursor_snapshot(&self) -> CursorSnapshot {
        let (cursor_line, cursor_column) = self.line_and_column_of_offset(self.cursor);
        let (anchor_line, anchor_column) = self.line_and_column_of_offset(self.anchor);
        CursorSnapshot {
            cursor_line,
            cursor_column,
            anchor_line,
            anchor_column,
        }
    }

    pub(super) fn restore_cursor_snapshot(&mut self, snapshot: CursorSnapshot) {
        self.cursor = self.offset_for_line_and_column(snapshot.cursor_line, snapshot.cursor_column);
        self.anchor = self.offset_for_line_and_column(snapshot.anchor_line, snapshot.anchor_column);
        self.preferred_column = Some(snapshot.cursor_column);
        self.snap_cursor();
    }

    pub(super) fn next_scalar(&self, offset: usize) -> usize {
        if offset >= self.doc.size() {
            return self.doc.size();
        }
        let first = self.doc.byte_at(offset).unwrap_or(0);
        let length = match first {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 1,
        };
        (offset + length).min(self.doc.size())
    }

    pub(super) fn previous_scalar(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let mut position = offset - 1;
        while position > 0
            && self
                .doc
                .byte_at(position)
                .map(|byte| byte & 0xC0 == 0x80)
                .unwrap_or(false)
        {
            position -= 1;
        }
        position
    }

    pub(super) fn snap_cursor(&mut self) {
        if self.cursor > self.doc.size() {
            self.cursor = self.doc.size();
        }
        while self.cursor > 0
            && self.cursor < self.doc.size()
            && self
                .doc
                .byte_at(self.cursor)
                .map(|byte| byte & 0xC0 == 0x80)
                .unwrap_or(false)
        {
            self.cursor -= 1;
        }
        if self.anchor > self.doc.size() {
            self.anchor = self.doc.size();
        }
        self.clamp_viewport();
    }

    pub(super) fn move_left(&mut self) {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        if self.cursor > start {
            self.cursor = self.previous_scalar(self.cursor);
        } else if line > 0 {
            self.cursor = self.doc.line_end(line - 1);
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_right(&mut self) {
        let line = self.cursor_line();
        let end = self.doc.line_end(line);
        if self.cursor < end {
            self.cursor = self.next_scalar(self.cursor);
        } else if line + 1 < self.doc.line_count() {
            self.cursor = self.doc.line_start(line + 1);
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_vertical(&mut self, delta: isize) {
        let line = self.cursor_line();
        let last_line = self.doc.line_count().saturating_sub(1) as isize;
        let target = (line as isize + delta).clamp(0, last_line) as usize;
        if self.preferred_column.is_none() {
            self.preferred_column = Some(self.cursor_visual_column());
        }
        let target_start = self.doc.line_start(target);
        let text = self.doc.line_text(target);
        let local = util::byte_offset_for_visual_column(
            &text,
            self.preferred_column.unwrap_or(0),
            self.doc.tab_width().max(1),
        );
        self.cursor = (target_start + local).min(self.doc.size());
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_page(&mut self, direction: isize) {
        let visible = self.active_pane_rows().max(1);
        self.move_vertical(direction * visible as isize);
    }

    pub(super) fn move_to_line_start(&mut self) {
        self.cursor = self.doc.line_start(self.cursor_line());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_to_line_end(&mut self) {
        self.cursor = self.line_end_of_cursor();
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn jump_to_document_start(&mut self) {
        self.record_jump();
        self.cursor = 0;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn jump_to_document_end(&mut self) {
        self.record_jump();
        self.cursor = self.doc.size();
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_word_forward(&mut self) {
        let mut line = self.cursor_line();
        let mut start = self.doc.line_start(line);
        let mut text = self.doc.line_text(line);
        let mut local = (self.cursor - start).min(text.len());

        while local < text.len() && util::is_word_byte(text[local]) {
            local += 1;
        }
        while local < text.len() && !util::is_word_byte(text[local]) {
            local += 1;
        }

        if local >= text.len() {
            line += 1;
            while line < self.doc.line_count() {
                start = self.doc.line_start(line);
                text = self.doc.line_text(line);
                local = 0;
                while local < text.len() && !util::is_word_byte(text[local]) {
                    local += 1;
                }
                if local < text.len() {
                    break;
                }
                line += 1;
            }
            if line >= self.doc.line_count() {
                self.cursor = self.doc.size();
                self.snap_cursor();
                return;
            }
        }

        self.cursor = (start + local).min(self.doc.size());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut line = self.cursor_line();
        let mut start = self.doc.line_start(line);
        let mut text = self.doc.line_text(line);
        let mut local = (self.cursor - start).min(text.len());

        while local > 0 {
            let previous = util::previous_boundary(&text, local);
            if util::is_word_byte(text[previous]) {
                break;
            }
            local = previous;
        }
        while local > 0 {
            let previous = util::previous_boundary(&text, local);
            if !util::is_word_byte(text[previous]) {
                break;
            }
            local = previous;
        }

        if local == 0 && self.cursor == start && line > 0 {
            line -= 1;
            start = self.doc.line_start(line);
            text = self.doc.line_text(line);
            local = text.len();
            while local > 0 {
                let previous = util::previous_boundary(&text, local);
                if !util::is_word_byte(text[previous]) {
                    break;
                }
                local = previous;
            }
        }

        self.cursor = (start + local).min(self.doc.size());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    pub(super) fn record_jump(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.jumps.last() == Some(&self.cursor) {
            return;
        }
        self.jumps.push(self.cursor);
        if self.jumps.len() > 200 {
            self.jumps.remove(0);
        }
        self.jump_index = self.jumps.len() as isize - 1;
    }

    pub(super) fn move_to_previous_location(&mut self) {
        if self.jump_index > 0 {
            self.jump_index -= 1;
        }
        if self.jump_index >= 0 {
            if let Some(&offset) = self.jumps.get(self.jump_index as usize) {
                self.cursor = offset;
                self.preferred_column = None;
                self.snap_cursor();
            }
        }
    }

    pub(super) fn move_to_next_location(&mut self) {
        if self.jump_index + 1 < self.jumps.len() as isize {
            self.jump_index += 1;
        }
        if self.jump_index >= 0 {
            if let Some(&offset) = self.jumps.get(self.jump_index as usize) {
                self.cursor = offset;
                self.preferred_column = None;
                self.snap_cursor();
            }
        }
    }

    pub(super) fn undo(&mut self) {
        if self.doc.undo() {
            self.after_undo_redo();
        } else {
            self.set_message("Already at oldest change.");
        }
    }

    pub(super) fn redo(&mut self) {
        if self.doc.redo() {
            self.after_undo_redo();
        } else {
            self.set_message("Already at newest change.");
        }
    }

    pub(super) fn after_undo_redo(&mut self) {
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.doc.break_undo_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with_lines(lines: usize) -> Editor {
        let text = (0..lines)
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        Editor::new(Document::from_bytes(None, text.into_bytes()), 80, 24)
    }

    #[test]
    fn page_movement_clamps_to_the_document_edges() {
        let mut editor = editor_with_lines(40);

        editor.cursor = editor.doc.line_start(30);
        editor.move_page(1);
        assert_eq!(editor.cursor_line(), 39);

        editor.cursor = editor.doc.line_start(5);
        editor.move_page(-1);
        assert_eq!(editor.cursor_line(), 0);

        editor.cursor = editor.doc.line_start(20);
        editor.move_page(1);
        assert_eq!(editor.cursor_line(), 39);

        editor.move_page(1);
        assert_eq!(editor.cursor_line(), 39);
    }

    #[test]
    fn mouse_selection_starts_in_the_text_area_not_the_gutter() {
        let mut editor = editor_with_lines(3);
        let gutter = editor.gutter_width();
        assert_eq!(editor.mouse_offset_at(0, 0), Some(0));
        assert_eq!(editor.mouse_offset_at(gutter, 0), Some(0));
        assert_eq!(editor.mouse_offset_at(gutter + 1, 0), Some(1));
        assert_eq!(editor.mouse_offset_at(editor.cols, 0), None);

        let y = 1;
        editor.handle_editor_mouse(Event {
            key: Key::MouseDown,
            ch: '\0',
            ctrl: false,
            alt: false,
            shift: false,
            mouse_x: gutter,
            mouse_y: 0,
            mouse_button: MouseButton::Left,
        });
        editor.handle_editor_mouse(Event {
            key: Key::MouseMove,
            ch: '\0',
            ctrl: false,
            alt: false,
            shift: false,
            mouse_x: gutter + 2,
            mouse_y: y,
            mouse_button: MouseButton::Left,
        });
        assert!(editor.mode == Mode::Select);
        assert_eq!(editor.anchor, 0);
        assert_eq!(editor.cursor, editor.doc.line_start(1) + 1);
        assert_eq!(editor.selection_range(), (0, editor.doc.line_start(2)));
    }

    #[test]
    fn vertical_movement_at_the_last_line_stays_on_the_last_line() {
        let mut editor = editor_with_lines(3);
        editor.cursor = editor.doc.line_start(2);
        editor.move_vertical(1);
        assert_eq!(editor.cursor_line(), 2);
        editor.move_vertical(-1);
        assert_eq!(editor.cursor_line(), 1);
    }

    #[test]
    fn cursor_snapshot_follows_the_same_line_and_column_after_reformatting() {
        let mut editor = Editor::new(
            Document::from_bytes(None, b"alpha\nbeta\ngamma\n".to_vec()),
            80,
            24,
        );
        editor.cursor = editor.doc.line_start(1) + 2;
        editor.anchor = editor.cursor;
        let snapshot = editor.capture_cursor_snapshot();

        editor
            .doc
            .replace_all_owned(b"alpha extended\nbeta\ngamma\n".to_vec());
        editor.restore_cursor_snapshot(snapshot);

        assert_eq!(editor.cursor_line(), 1);
        assert_eq!(editor.cursor, editor.doc.line_start(1) + 2);
        assert_eq!(editor.anchor, editor.cursor);
    }

    #[test]
    fn cursor_snapshot_steps_back_out_of_new_indentation() {
        let mut editor = Editor::new(Document::from_bytes(None, b"    one\n".to_vec()), 80, 24);
        editor.cursor = editor.doc.line_start(0) + 6;
        editor.anchor = editor.cursor;
        let snapshot = editor.capture_cursor_snapshot();

        editor.doc.replace_all_owned(b"        one\n".to_vec());
        editor.restore_cursor_snapshot(snapshot);

        assert_eq!(editor.cursor, editor.doc.line_start(0));
    }
}

impl Editor {
    pub(super) fn handle_editor_mouse(&mut self, event: Event) {
        if event.key == Key::MouseDown
            && event.mouse_button == MouseButton::Left
            && let Some(clicked) = self.try_pane_at(event.mouse_x, event.mouse_y)
            && clicked != self.active_pane
        {
            self.switch_to_pane(clicked);
            self.force_full_redraw = true;
        }
        match event.key {
            Key::ScrollUp => self.move_page(-1),
            Key::ScrollDown => self.move_page(1),
            Key::MouseDown if event.mouse_button == MouseButton::Left => {
                let Some(offset) = self.mouse_offset_at(event.mouse_x, event.mouse_y) else {
                    return;
                };
                if self.mode == Mode::Select {
                    self.mode = Mode::Normal;
                    self.selection_linewise = false;
                }
                self.cursor = offset;
                self.anchor = offset;
                self.mouse_drag_anchor = offset;
                self.mouse_dragging = true;
                self.preferred_column = None;
                self.doc.break_undo_group();
                self.snap_cursor();
                self.clear_message();
            }
            Key::MouseMove if self.mouse_dragging && event.mouse_button == MouseButton::Left => {
                self.extend_mouse_selection(event.mouse_x, event.mouse_y);
            }
            Key::MouseUp if self.mouse_dragging && event.mouse_button == MouseButton::Left => {
                let mut extend = self.mode == Mode::Select;
                if !extend {
                    if let Some(offset) = self.mouse_offset_at(event.mouse_x, event.mouse_y) {
                        extend = offset != self.mouse_drag_anchor;
                    }
                }
                if extend {
                    self.extend_mouse_selection(event.mouse_x, event.mouse_y);
                    if self.mode == Mode::Select && self.anchor == self.cursor {
                        self.mode = Mode::Normal;
                        self.selection_linewise = false;
                    }
                }
                self.mouse_dragging = false;
            }
            _ => {}
        }
    }

    fn extend_mouse_selection(&mut self, x: usize, y: usize) {
        let Some(offset) = self.mouse_offset_at(x, y) else {
            return;
        };
        if self.mode != Mode::Select {
            self.mode = Mode::Select;
            self.selection_linewise = false;
        }
        self.anchor = self.mouse_drag_anchor;
        self.cursor = offset;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
        self.clear_message();
    }

    fn mouse_offset_at(&self, x: usize, y: usize) -> Option<usize> {
        let bounds = self.active_pane_bounds();
        if x < bounds.x
            || x >= bounds.x + bounds.width
            || y < bounds.y
            || y >= bounds.y + bounds.height
        {
            return None;
        }
        let left = bounds.x;
        let line = self.top_line + (y - bounds.y);
        if line >= self.doc.line_count() {
            return Some(self.doc.size());
        }
        let gutter = self.gutter_width_for(bounds.width);
        let local_x = x.saturating_sub(left);
        let visual = if local_x > gutter {
            self.left_column + (local_x - gutter)
        } else {
            0
        };
        let text = self.doc.line_text(line);
        let local = util::byte_offset_for_visual_column(&text, visual, self.doc.tab_width().max(1));
        let start = self.doc.line_start(line);
        let end = self.doc.line_end(line);
        Some((start + local).min(end))
    }
}
