//! Soft word wrapping: segmenting lines and rendering wrapped viewport rows.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

#[derive(Clone, Copy)]
pub(super) struct WrapSegment {
    pub start_byte: usize,
    pub length_bytes: usize,
    pub visual_start: usize,
    pub visual_width: usize,
}

#[derive(Clone, Copy)]
pub(super) struct WrapRow {
    pub line: usize,
    pub segment: WrapSegment,
    pub first_segment: bool,
}

impl Editor {
    pub(super) fn toggle_word_wrap(&mut self) {
        self.word_wrap = !self.word_wrap;
        if self.word_wrap {
            self.left_column = 0;
        }
        self.force_full_redraw = true;
        self.ensure_cursor_visible();
        match Config::update_word_wrap(&self.config.source_path, self.word_wrap) {
            Ok(()) => {
                self.config.word_wrap = self.word_wrap;
                self.set_message(if self.word_wrap {
                    "Word wrap enabled."
                } else {
                    "Word wrap disabled."
                });
            }
            Err(error) => self.set_error(&format!("config: {error}")),
        }
    }

    pub(super) fn wrap_segments_for_line(&self, line: usize, width: usize) -> Vec<WrapSegment> {
        let width = width.max(1);
        let text = self.doc.line_text(line);
        if text.is_empty() {
            return vec![WrapSegment {
                start_byte: 0,
                length_bytes: 0,
                visual_start: 0,
                visual_width: 0,
            }];
        }

        let tab_width = self.doc.tab_width().max(1);
        let mut segments = Vec::new();
        let mut offset = 0usize;
        let mut segment_start = 0usize;
        let mut last_break: Option<usize> = None;
        let mut visual = 0usize;
        let mut segment_visual_start = 0usize;
        let mut last_break_visual = 0usize;

        while offset < text.len() {
            let rune_start = offset;
            let (cp, length) = util::decode_at(&text, offset);
            if length == 0 {
                break;
            }
            offset += length;
            let cell_width = if cp == '\t' {
                tab_width - (visual % tab_width)
            } else {
                let width = util::display_width(cp);
                if width == 0 && (cp < ' ' || cp == '\u{7F}') {
                    1
                } else {
                    width
                }
            };
            let is_space = cp == ' ' || cp == '\t';

            if visual + cell_width > segment_visual_start + width && rune_start > segment_start {
                let (break_at, end_visual) = match last_break {
                    Some(last) if last > segment_start => (last, last_break_visual),
                    _ => (rune_start, visual),
                };
                segments.push(WrapSegment {
                    start_byte: segment_start,
                    length_bytes: break_at - segment_start,
                    visual_start: segment_visual_start,
                    visual_width: end_visual.saturating_sub(segment_visual_start),
                });
                segment_start = break_at;
                segment_visual_start = end_visual;
                last_break = None;
                offset = break_at;
                visual = end_visual;
                continue;
            }

            visual += cell_width;
            if is_space {
                last_break = Some(offset);
                last_break_visual = visual;
            }
        }

        segments.push(WrapSegment {
            start_byte: segment_start,
            length_bytes: text.len() - segment_start,
            visual_start: segment_visual_start,
            visual_width: visual.saturating_sub(segment_visual_start),
        });
        segments
    }

    pub(super) fn build_wrap_rows(
        &self,
        first_line: usize,
        rows: usize,
        width: usize,
    ) -> Vec<WrapRow> {
        let mut result = Vec::new();
        let total = self.doc.line_count();
        for line in first_line..total {
            if result.len() >= rows {
                break;
            }
            let segments = self.wrap_segments_for_line(line, width);
            for (index, segment) in segments.into_iter().enumerate() {
                if result.len() >= rows {
                    break;
                }
                result.push(WrapRow {
                    line,
                    segment,
                    first_segment: index == 0,
                });
            }
        }
        result
    }

    fn wrap_segment_index_for_visual(segments: &[WrapSegment], column: usize) -> usize {
        for (index, segment) in segments.iter().enumerate() {
            if column < segment.visual_start + segment.visual_width || index == segments.len() - 1 {
                return index;
            }
        }
        0
    }

    fn wrap_row_index_in_viewport(
        &self,
        first_line: usize,
        target_line: usize,
        target_visual: usize,
        width: usize,
    ) -> (usize, usize) {
        let mut row_index = 0usize;
        let total = self.doc.line_count();
        let end = target_line.min(total);
        for line in first_line..end {
            row_index += self.wrap_segments_for_line(line, width).len();
        }
        let segments = self.wrap_segments_for_line(target_line.min(total.saturating_sub(1)), width);
        let segment_index = Self::wrap_segment_index_for_visual(&segments, target_visual);
        (row_index + segment_index, segment_index)
    }

    pub(super) fn ensure_wrapped_cursor_visible(
        &mut self,
        line: usize,
        rows: usize,
        pane_width: usize,
    ) {
        let gutter = self.gutter_width_for(pane_width);
        let text_width = pane_width.saturating_sub(gutter).max(1);
        let column = self.cursor_visual_column();

        if line < self.top_line {
            self.top_line = line;
        } else {
            let (cursor_row, segment_index) =
                self.wrap_row_index_in_viewport(self.top_line, line, column, text_width);
            if cursor_row >= rows {
                let mut candidate = line;
                let mut rows_included = segment_index + 1;
                while candidate > 0 && rows_included < rows {
                    candidate -= 1;
                    rows_included += self.wrap_segments_for_line(candidate, text_width).len();
                }
                self.top_line = candidate;
            }
        }

        let max_top = self.doc.line_count().saturating_sub(1);
        self.top_line = self.top_line.min(max_top);
        self.left_column = 0;
    }

    pub(super) fn wrapped_offset_at_screen(
        &self,
        local_x: usize,
        local_y: usize,
        pane_width: usize,
        pane_height: usize,
    ) -> Option<usize> {
        let gutter = self.gutter_width_for(pane_width);
        let text_width = pane_width.saturating_sub(gutter).max(1);
        let rows = self.build_wrap_rows(self.top_line, pane_height, text_width);
        if local_y >= rows.len() {
            return Some(self.doc.size());
        }
        let row = rows[local_y];
        let text = self.doc.line_text(row.line);
        let visual = if local_x > gutter {
            row.segment.visual_start + (local_x - gutter)
        } else {
            row.segment.visual_start
        };
        let local = util::byte_offset_for_visual_column(&text, visual, self.doc.tab_width().max(1));
        let start = self.doc.line_start(row.line);
        let end = self.doc.line_end(row.line);
        Some((start + local).min(end))
    }

    pub(super) fn render_wrapped_line_number(&mut self, line: usize, row: usize, gutter: usize) {
        if gutter == 0 {
            return;
        }
        let marker_width = usize::from(self.sidebar.show_modified);
        let digits = gutter.saturating_sub(1 + marker_width);
        let highlight = self.line_highlight_style(line);
        if let Some(highlight) = highlight {
            self.screen.fill(
                self.editor_left() as i64,
                row as i64,
                self.cols.saturating_sub(self.editor_left()),
                1,
                ' ',
                highlight,
            );
        }
        if marker_width > 0 {
            if let Some((marker, mut marker_style)) = self.git_diff_marker(line) {
                if let Some(highlight) = highlight {
                    marker_style.bg = highlight.bg;
                }
                self.screen.put(
                    self.editor_left() as i64,
                    row as i64,
                    marker,
                    marker_style,
                    1,
                );
            }
        }
        let number = line + 1;
        let x = digits.saturating_sub(util::digit_count(number));
        let mut style = if line == self.cursor_line() {
            self.theme.current_line_number_style()
        } else {
            self.theme.muted_style()
        };
        if let Some(highlight) = highlight {
            style.bg = highlight.bg;
        }
        self.screen.put_str(
            (self.editor_left() + marker_width + x) as i64,
            row as i64,
            &number.to_string(),
            style,
            digits,
        );
    }

    pub(super) fn render_wrapped_text_lines(
        &mut self,
        first_line: usize,
        rows: usize,
        gutter: usize,
        language: Option<&Language>,
    ) {
        if rows == 0 || self.cols <= gutter {
            return;
        }
        let text_width = self.cols - gutter;
        let wrap_rows = self.build_wrap_rows(first_line, rows, text_width);
        let tab_width = self.doc.tab_width().max(1);

        for (row, wrapped) in wrap_rows.iter().enumerate() {
            let text = self.doc.line_text(wrapped.line);
            let mut tokens = Vec::new();
            if let Some(language) = language {
                self.highlighter.get_line_tokens(
                    &self.doc,
                    language,
                    wrapped.line,
                    &text,
                    &mut tokens,
                );
            }
            let line_highlight = self.line_highlight_style(wrapped.line);
            if let Some(highlight) = line_highlight {
                self.screen
                    .fill(gutter as i64, row as i64, text_width, 1, ' ', highlight);
            }
            let unnecessary = self
                .unnecessary_diagnostics
                .get(&wrapped.line)
                .cloned()
                .unwrap_or_default();

            let start = wrapped.segment.start_byte.min(text.len());
            let end = (wrapped.segment.start_byte + wrapped.segment.length_bytes).min(text.len());
            let mut token_index = 0usize;
            while token_index < tokens.len() && tokens[token_index].end <= start {
                token_index += 1;
            }
            let mut offset = start;
            let mut visual = wrapped.segment.visual_start;
            while offset < end {
                let rune_start = offset;
                while token_index < tokens.len() && tokens[token_index].end <= rune_start {
                    token_index += 1;
                }
                let kind = if token_index < tokens.len()
                    && tokens[token_index].start <= rune_start
                    && rune_start < tokens[token_index].end
                {
                    tokens[token_index].kind
                } else {
                    SyntaxKind::Default
                };
                let mut style = self.theme.syntax_style(kind);
                if let Some(highlight) = line_highlight {
                    style.bg = highlight.bg;
                }
                for (range_start, range_end) in &unnecessary {
                    let start_byte = crate::lsp::utf16_column_to_byte_offset(&text, *range_start);
                    let end_byte = if *range_end == i64::MAX {
                        text.len()
                    } else {
                        crate::lsp::utf16_column_to_byte_offset(&text, *range_end)
                    };
                    if rune_start < end_byte && offset > start_byte {
                        style.fg = self.theme.dimmed;
                        break;
                    }
                }

                let (mut cp, length) = util::decode_at(&text, offset);
                if length == 0 {
                    break;
                }
                offset += length;
                let mut width = if cp == '\t' {
                    tab_width - (visual % tab_width)
                } else {
                    util::display_width(cp)
                };
                if cp == '\t' {
                    // Rendered as spaces.
                } else if width == 0 {
                    if cp < ' ' || cp == '\u{7F}' {
                        cp = util::REPLACEMENT;
                        width = 1;
                    } else {
                        continue;
                    }
                } else if cp < ' ' || cp == '\u{7F}' {
                    cp = util::REPLACEMENT;
                    width = 1;
                }

                let screen_x = gutter + visual.saturating_sub(wrapped.segment.visual_start);
                let available = self.cols.saturating_sub(screen_x);
                if available > 0 {
                    if cp == '\t' {
                        for index in 0..width.min(available) {
                            self.screen
                                .put((screen_x + index) as i64, row as i64, ' ', style, 1);
                        }
                    } else {
                        let drawn = width.min(available);
                        self.screen
                            .put(screen_x as i64, row as i64, cp, style, drawn);
                    }
                }
                visual += width;
            }
        }
    }

    pub(super) fn render_wrapped_selection(
        &mut self,
        first_line: usize,
        rows: usize,
        gutter: usize,
    ) {
        if (self.mode != Mode::Select && !self.command_selection) || rows == 0 {
            return;
        }
        let style = self.theme.selection_style();
        let text_width = self.cols.saturating_sub(gutter).max(1);
        let wrap_rows = self.build_wrap_rows(first_line, rows, text_width);

        for selection in self.capture_all_selections() {
            let (range_start, range_end) =
                self.selection_range_of(selection.anchor, selection.cursor, selection.linewise);
            if range_end <= range_start {
                continue;
            }
            for (row, wrapped) in wrap_rows.iter().enumerate() {
                let line = wrapped.line;
                let line_start = self.doc.line_start(line);
                let line_end = self.doc.line_end(line);
                let next_line_start = if line + 1 < self.doc.line_count() {
                    self.doc.line_start(line + 1)
                } else {
                    self.doc.size()
                };
                if selection.linewise {
                    if range_start < next_line_start && range_end > line_start {
                        for x in gutter..self.cols {
                            self.screen.apply_style(x, row, style);
                        }
                    }
                    continue;
                }

                let from = range_start.max(line_start);
                let to = range_end.min(line_end);
                if to <= from {
                    if wrapped.first_segment
                        && range_start == line_start
                        && range_end > line_start
                        && line_end == line_start
                        && gutter < self.cols
                    {
                        self.screen.apply_style(gutter, row, style);
                    }
                    continue;
                }

                let text = self.doc.line_text(line);
                let segment_start = line_start + wrapped.segment.start_byte;
                let segment_end = line_end.min(segment_start + wrapped.segment.length_bytes);
                let from_segment = from.max(segment_start);
                let to_segment = to.min(segment_end);
                if to_segment <= from_segment {
                    continue;
                }
                let local_from = (from_segment - line_start).min(text.len());
                let local_to = (to_segment - line_start).min(text.len());
                let visual_from =
                    util::visual_column(&text, local_from, self.doc.tab_width().max(1));
                let visual_to = util::visual_column(&text, local_to, self.doc.tab_width().max(1));
                let first_x =
                    gutter.max(gutter + visual_from.saturating_sub(wrapped.segment.visual_start));
                let last_x = self
                    .cols
                    .min(gutter + visual_to.saturating_sub(wrapped.segment.visual_start));
                for x in first_x..last_x {
                    self.screen.apply_style(x, row, style);
                }
            }
        }
    }

    pub(super) fn render_wrapped_find_matches(
        &mut self,
        first_line: usize,
        rows: usize,
        gutter: usize,
    ) {
        if !self.find.active || self.find.matches.is_empty() || rows == 0 {
            return;
        }
        let match_style = self.theme.match_style();
        let current_style = self.theme.current_match_style();
        let text_width = self.cols.saturating_sub(gutter).max(1);
        let wrap_rows = self.build_wrap_rows(first_line, rows, text_width);

        for (match_index, match_) in self.find.matches.iter().enumerate() {
            let text = self.doc.line_text(match_.line);
            let line_start = self.doc.line_start(match_.line);
            let style = if self.find.index == Some(match_index) {
                current_style
            } else {
                match_style
            };
            for (row, wrapped) in wrap_rows.iter().enumerate() {
                if wrapped.line != match_.line {
                    continue;
                }
                let segment_start = line_start + wrapped.segment.start_byte;
                let segment_end = segment_start + wrapped.segment.length_bytes;
                let from = match_.start.max(segment_start);
                let to = match_.end.min(segment_end);
                if to <= from {
                    continue;
                }
                let local_from = (from - line_start).min(text.len());
                let local_to = (to - line_start).min(text.len());
                let visual_from =
                    util::visual_column(&text, local_from, self.doc.tab_width().max(1));
                let visual_to = util::visual_column(&text, local_to, self.doc.tab_width().max(1));
                let first_x =
                    gutter.max(gutter + visual_from.saturating_sub(wrapped.segment.visual_start));
                let last_x = self
                    .cols
                    .min(gutter + visual_to.saturating_sub(wrapped.segment.visual_start));
                for x in first_x..last_x {
                    self.screen.apply_style(x, row, style);
                }
            }
        }
    }

    pub(super) fn render_wrapped_cursor(&mut self, first_line: usize, rows: usize, gutter: usize) {
        for selection in self.capture_all_selections() {
            self.render_wrapped_cursor_at(selection.cursor, first_line, rows, gutter);
        }
    }

    fn render_wrapped_cursor_at(
        &mut self,
        cursor: usize,
        first_line: usize,
        rows: usize,
        gutter: usize,
    ) {
        let line = self.doc.line_of_offset(cursor);
        if line < first_line {
            return;
        }
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = cursor.saturating_sub(start).min(text.len());
        let column = util::visual_column(&text, local, self.doc.tab_width().max(1));
        let text_width = self.cols.saturating_sub(gutter).max(1);
        let wrap_rows = self.build_wrap_rows(first_line, rows, text_width);
        let mut candidate: Option<usize> = None;
        for (row, wrapped) in wrap_rows.iter().enumerate() {
            if wrapped.line != line {
                continue;
            }
            candidate = Some(row);
            let segment_end = wrapped.segment.visual_start + wrapped.segment.visual_width;
            if column < segment_end {
                let x = gutter + column.saturating_sub(wrapped.segment.visual_start);
                if x >= gutter && x < self.cols {
                    let mut style = self.screen.at(x, row).style;
                    style.reverse = true;
                    self.screen.apply_style(x, row, style);
                }
                return;
            }
        }
        if let Some(row) = candidate {
            let wrapped = wrap_rows[row];
            let x = (gutter + column.saturating_sub(wrapped.segment.visual_start))
                .clamp(gutter, self.cols.saturating_sub(1));
            let mut style = self.screen.at(x, row).style;
            style.reverse = true;
            self.screen.apply_style(x, row, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;
    use crate::core::document::Document;

    #[test]
    fn wraps_at_spaces_when_a_line_is_too_wide() {
        let editor = Editor::new(
            Document::from_bytes(None, b"one two three four\n".to_vec()),
            80,
            24,
        );
        let segments = editor.wrap_segments_for_line(0, 8);
        assert!(segments.len() > 1);
        assert_eq!(segments[0].start_byte, 0);
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.length_bytes)
                .sum::<usize>(),
            "one two three four".len()
        );
    }

    #[test]
    fn toggling_word_wrap_updates_the_setting() {
        let mut editor = Editor::new(Document::new(), 80, 24);
        let directory =
            std::env::temp_dir().join(format!("hexal-wrap-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        editor.config.source_path = directory.join(".hexcfg");
        let before = editor.word_wrap;
        editor.toggle_word_wrap();
        assert_ne!(editor.word_wrap, before);
        let _ = std::fs::remove_dir_all(directory);
    }
}
