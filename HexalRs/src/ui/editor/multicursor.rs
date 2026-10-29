//! Multi-cursor selection state and helpers.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn capture_primary_selection(&self) -> Selection {
        Selection {
            cursor: self.cursor,
            anchor: self.anchor,
            linewise: self.selection_linewise,
            preferred_column: self.preferred_column,
        }
    }

    pub(super) fn load_selection(&mut self, selection: Selection) {
        self.cursor = selection.cursor;
        self.anchor = selection.anchor;
        self.selection_linewise = selection.linewise;
        self.preferred_column = selection.preferred_column;
    }

    pub(super) fn capture_all_selections(&self) -> Vec<Selection> {
        let mut selections = Vec::with_capacity(self.extra_selections.len() + 1);
        selections.push(self.capture_primary_selection());
        selections.extend(self.extra_selections.iter().copied());
        selections
    }

    pub(super) fn set_selections(&mut self, selections: Vec<Selection>) {
        let Some(&primary) = selections.first() else {
            return;
        };
        self.cursor = self.snap_offset(primary.cursor);
        self.anchor = self.snap_offset(primary.anchor);
        self.selection_linewise = primary.linewise;
        self.preferred_column = primary.preferred_column;

        self.extra_selections.clear();
        let mut seen = std::collections::HashSet::new();
        seen.insert(self.cursor);
        for selection in selections.into_iter().skip(1) {
            let cursor = self.snap_offset(selection.cursor);
            if !seen.insert(cursor) {
                continue;
            }
            self.extra_selections.push(Selection {
                cursor,
                anchor: self.snap_offset(selection.anchor),
                linewise: selection.linewise,
                preferred_column: selection.preferred_column,
            });
        }
    }

    fn snap_offset(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.doc.size());
        while offset > 0
            && offset < self.doc.size()
            && self
                .doc
                .byte_at(offset)
                .map(|byte| byte & 0xC0 == 0x80)
                .unwrap_or(false)
        {
            offset -= 1;
        }
        offset
    }

    /// Runs `action` once for every selection and restores all selections
    /// afterwards. Edits are processed from the highest offset to the lowest
    /// so earlier edits remain valid; each resulting selection is shifted when
    /// a later edit lands before it.
    pub(super) fn for_each_selection<F>(&mut self, mut action: F, editing: bool)
    where
        F: FnMut(&mut Editor),
    {
        let selections = self.capture_all_selections();
        if selections.len() == 1 {
            self.load_selection(selections[0]);
            action(self);
            return;
        }

        let mut results: Vec<Option<Selection>> = vec![None; selections.len()];
        let mut order: Vec<usize> = (0..selections.len()).collect();
        let mode_before = self.mode;

        if editing {
            order.sort_by(|&left, &right| {
                selections[right]
                    .cursor
                    .cmp(&selections[left].cursor)
                    .then_with(|| right.cmp(&left))
            });
            self.doc.begin_composite_edit();
        }

        for index in order {
            self.load_selection(selections[index]);
            self.mode = mode_before;
            let revision_before = self.doc.revision();
            let size_before = self.doc.size();
            action(self);

            if editing && self.doc.revision() != revision_before {
                let delta = self.doc.size() as isize - size_before as isize;
                if delta != 0 {
                    for result in results.iter_mut().flatten() {
                        result.cursor = shift_offset(result.cursor, delta);
                        result.anchor = shift_offset(result.anchor, delta);
                    }
                }
            }
            results[index] = Some(self.capture_primary_selection());
        }

        if editing {
            self.doc.end_composite_edit();
        }

        let results: Vec<Selection> = results.into_iter().flatten().collect();
        self.set_selections(results);
    }

    pub(super) fn add_cursor_below(&mut self) {
        let selections = self.capture_all_selections();
        let mut results = selections.clone();
        let mut added = false;

        for source in selections {
            if let Some(created) = self.create_selection_below(source) {
                results.push(created);
                added = true;
            }
        }

        if !added {
            self.set_message("Already at the last line.");
            return;
        }

        self.set_selections(results);
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn create_selection_below(&self, source: Selection) -> Option<Selection> {
        let source_line = self.doc.line_of_offset(source.cursor);
        if source_line + 1 >= self.doc.line_count() {
            return None;
        }

        let target_line = source_line + 1;
        let target_start = self.doc.line_start(target_line);
        let text = self.doc.line_text(target_line);
        let tab_width = self.doc.tab_width().max(1);
        let cursor_column = self.line_and_column_of_offset(source.cursor).1;
        let cursor_local = util::byte_offset_for_visual_column(&text, cursor_column, tab_width);
        let target_cursor = (target_start + cursor_local).min(self.doc.size());

        let mut target_anchor = target_cursor;
        if self.mode == Mode::Select
            && source.anchor != source.cursor
            && self.doc.line_of_offset(source.anchor) == source_line
        {
            let anchor_column = self.line_and_column_of_offset(source.anchor).1;
            let anchor_local = util::byte_offset_for_visual_column(&text, anchor_column, tab_width);
            target_anchor = (target_start + anchor_local).min(self.doc.size());
        }

        Some(Selection {
            cursor: target_cursor,
            anchor: target_anchor,
            linewise: source.linewise,
            preferred_column: Some(cursor_column),
        })
    }

    pub(super) fn keep_primary_selection(&mut self) {
        self.extra_selections.clear();
        self.doc.break_undo_group();
        self.clear_message();
    }
}

pub(super) fn adjust_offset(
    position: usize,
    offset: usize,
    removed: usize,
    inserted: usize,
) -> usize {
    if position < offset {
        return position;
    }
    if position >= offset + removed {
        return position + inserted - removed;
    }
    offset + (position - offset).min(inserted)
}

fn shift_offset(position: usize, delta: isize) -> usize {
    if delta >= 0 {
        position.saturating_add(delta as usize)
    } else {
        position.saturating_sub(delta.unsigned_abs())
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
    fn add_cursor_below_mirrors_the_column_and_repeats() {
        let mut editor = editor(b"one\ntwo\nthree\n");
        editor.cursor = 1;
        editor.anchor = 1;
        editor.add_cursor_below();
        assert_eq!(editor.extra_selections.len(), 1);
        assert_eq!(
            editor.extra_selections[0].cursor,
            editor.doc.line_start(1) + 1
        );
        editor.add_cursor_below();
        assert_eq!(editor.extra_selections.len(), 2);
        assert_eq!(
            editor.extra_selections[1].cursor,
            editor.doc.line_start(2) + 1
        );
    }

    #[test]
    fn editing_applies_to_every_selection_as_one_undo_step() {
        let mut editor = editor(b"one\ntwo\nthree\n");
        editor.mode = Mode::Insert;
        editor.cursor = editor.doc.line_start(0);
        editor.anchor = editor.cursor;
        editor.add_cursor_below();
        editor.add_cursor_below();
        editor.for_each_selection(|editor| editor.insert_character('X'), true);
        assert_eq!(editor.doc.all_text(), b"Xone\nXtwo\nXthree\n");
        assert_eq!(editor.extra_selections.len(), 2);
        editor.doc.undo();
        assert_eq!(editor.doc.all_text(), b"one\ntwo\nthree\n");
    }

    #[test]
    fn keep_primary_selection_removes_extra_cursors() {
        let mut editor = editor(b"one\ntwo\n");
        editor.add_cursor_below();
        assert_eq!(editor.extra_selections.len(), 1);
        editor.keep_primary_selection();
        assert!(editor.extra_selections.is_empty());
    }
}
