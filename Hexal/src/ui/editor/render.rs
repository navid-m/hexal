//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn status_row(&self) -> usize {
        if self.rows >= 2 {
            self.rows - 2
        } else {
            0
        }
    }

    pub(super) fn command_row(&self) -> usize {
        self.rows.saturating_sub(1)
    }

    pub(super) fn text_rows(&self) -> usize {
        self.status_row()
    }

    pub(super) fn gutter_width(&self) -> usize {
        let width = self.cols.saturating_sub(self.editor_left());
        self.gutter_width_for(width)
    }

    pub(super) fn gutter_width_for(&self, width: usize) -> usize {
        let marker = usize::from(self.sidebar.show_modified);
        let gutter = util::digit_count(self.doc.line_count()) + 1 + marker;
        if width < gutter + 10 {
            0
        } else {
            gutter
        }
    }

    pub(super) fn clamp_viewport(&mut self) {
        let last_line = self.doc.line_count().saturating_sub(1);
        if self.top_line > last_line {
            self.top_line = last_line;
        }
        let max_left = self.cols.saturating_sub(1);
        if self.left_column > max_left {
            self.left_column = max_left;
        }
    }

    pub(super) fn pane_rows_above_find(&self, bounds: &PaneView) -> usize {
        let find_height = self.find_dialog_height();
        if find_height == 0 {
            return bounds.height;
        }
        let find_top = self.status_row().saturating_sub(find_height);
        let pane_bottom = bounds.y.saturating_add(bounds.height);
        pane_bottom.min(find_top).saturating_sub(bounds.y)
    }

    pub(super) fn ensure_cursor_visible(&mut self) {
        let bounds = self.active_pane_bounds();
        let rows = self.pane_rows_above_find(&bounds);
        if rows == 0 {
            self.top_line = self.cursor_line();
            return;
        }
        let line = self.cursor_line();
        if self.word_wrap {
            self.ensure_wrapped_cursor_visible(line, rows, bounds.width);
            return;
        }
        if line < self.top_line {
            self.top_line = line;
        }
        if line >= self.top_line + rows {
            self.top_line = line + 1 - rows;
        }
        let max_top = self.doc.line_count().saturating_sub(rows);
        if self.top_line > max_top {
            self.top_line = max_top;
        }

        let gutter = self.gutter_width_for(bounds.width);
        if bounds.width <= gutter {
            return;
        }
        let width = bounds.width - gutter;
        let column = self.cursor_visual_column();
        if column < self.left_column {
            self.left_column = column;
        }
        if column >= self.left_column + width {
            self.left_column = column + 1 - width;
        }
        if self.left_column > column {
            self.left_column = column;
        }
        let max_left = bounds.width.saturating_sub(1);
        if self.left_column > max_left {
            self.left_column = max_left;
        }
    }

    pub(super) fn render(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        self.screen.resize(self.cols, self.rows);
        self.screen.clear(self.theme.base());
        if self.cols == 0 || self.rows == 0 {
            return Ok(());
        }

        self.ensure_cursor_visible();
        self.render_panes();
        self.render_sidebar();
        self.render_status_line();
        self.render_bottom_line();
        self.render_space_menu();
        self.render_sidebar_context_menu();
        self.render_picker();
        self.render_find();
        self.render_cross_find();
        self.render_hover();
        self.render_completion();
        command_completion::render_command_completion(self);
        self.render_debug_variables();
        self.render_lsp_debug();
        self.render_dap_debug();
        self.render_plugin_panel();
        self.render_plugin_manager();
        self.render_about();
        self.render_startup_benchmark();
        self.render_help();

        let force = self.force_full_redraw;
        self.force_full_redraw = false;
        let output = self.renderer.render(&self.screen, force);
        terminal.write(output)
    }

    pub(super) fn render_line_number(&mut self, line: usize, row: usize) {
        let gutter = self.gutter_width();
        if gutter == 0 {
            return;
        }
        let digits = util::digit_count(self.doc.line_count());
        let number = line + 1;
        let marker_width = usize::from(self.sidebar.show_modified);
        let x = self.editor_left() + marker_width + digits - util::digit_count(number);
        let highlight = self.line_highlight_style(line);
        if let Some(highlight) = highlight {
            let left = self.editor_left();
            self.screen.fill(
                left as i64,
                row as i64,
                self.cols.saturating_sub(left),
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
        let mut style = if line == self.cursor_line() {
            self.theme.current_line_number_style()
        } else {
            self.theme.muted_style()
        };
        if let Some(highlight) = highlight {
            style.bg = highlight.bg;
        }
        self.screen
            .put_str(x as i64, row as i64, &number.to_string(), style, digits);
    }

    pub(super) fn render_line_text(
        &mut self,
        line: usize,
        row: usize,
        gutter: usize,
        language: Option<&Language>,
    ) {
        if self.cols <= gutter {
            return;
        }
        self.doc.copy_line(line, &mut self.line_buffer);
        let text = std::mem::take(&mut self.line_buffer);
        let text_width = self.cols - gutter;
        let mut tokens = std::mem::take(&mut self.line_tokens);
        tokens.clear();
        if let Some(language) = language {
            self.highlighter
                .get_line_tokens(&self.doc, language, line, &text, &mut tokens);
        }
        let tab_width = self.doc.tab_width().max(1);
        let line_highlight = self.line_highlight_style(line);
        if let Some(highlight) = line_highlight {
            self.screen
                .fill(gutter as i64, row as i64, text_width, 1, ' ', highlight);
        }

        let mut offset = 0usize;
        let mut visual = 0usize;
        let mut token_index = 0usize;

        while offset < text.len() {
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
            if let Some(ranges) = self.unnecessary_diagnostics.get(&line) {
                for (start, end) in ranges {
                    let start_byte = crate::lsp::utf16_column_to_byte_offset(&text, *start);
                    let end_byte = if *end == i64::MAX {
                        text.len()
                    } else {
                        crate::lsp::utf16_column_to_byte_offset(&text, *end)
                    };
                    if rune_start < end_byte && offset > start_byte {
                        style.fg = self.theme.dimmed;
                        break;
                    }
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
                // Rendered as spaces below.
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

            let next_visual = visual + width;
            if next_visual <= self.left_column {
                visual = next_visual;
                continue;
            }

            if visual < self.left_column {
                let visible = next_visual - self.left_column;
                for i in 0..visible.min(text_width) {
                    self.screen
                        .put((gutter + i) as i64, row as i64, ' ', style, 1);
                }
                visual = next_visual;
                continue;
            }

            let screen_x = gutter + (visual - self.left_column);
            if screen_x >= self.cols {
                break;
            }
            let available = self.cols - screen_x;
            if width <= available {
                if cp == '\t' {
                    for i in 0..width {
                        self.screen
                            .put((screen_x + i) as i64, row as i64, ' ', style, 1);
                    }
                } else {
                    self.screen
                        .put(screen_x as i64, row as i64, cp, style, width);
                }
            } else {
                for i in 0..available {
                    self.screen
                        .put((screen_x + i) as i64, row as i64, ' ', style, 1);
                }
            }

            visual = next_visual;
            if visual - self.left_column >= text_width {
                break;
            }
        }
        self.line_buffer = text;
        self.line_tokens = tokens;
    }

    pub(super) fn render_selection(&mut self, first_line: usize, rows: usize, gutter: usize) {
        if (self.mode != Mode::Select && !self.command_selection) || rows == 0 {
            return;
        }
        for selection in self.capture_all_selections() {
            let (range_start, range_end) =
                self.selection_range_of(selection.anchor, selection.cursor, selection.linewise);
            if range_end > range_start {
                self.render_selection_range(
                    range_start,
                    range_end,
                    selection.linewise,
                    first_line,
                    rows,
                    gutter,
                );
            }
        }
    }

    fn render_selection_range(
        &mut self,
        range_start: usize,
        range_end: usize,
        linewise: bool,
        first_line: usize,
        rows: usize,
        gutter: usize,
    ) {
        let style = self.theme.selection_style();

        for row in 0..rows {
            let line = first_line + row;
            if line >= self.doc.line_count() {
                break;
            }
            let line_start = self.doc.line_start(line);
            let line_end = self.doc.line_end(line);
            let next_line_start = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };

            if linewise {
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
                if range_start == line_start
                    && range_end > line_start
                    && line_end == line_start
                    && gutter < self.cols
                {
                    self.screen.apply_style(gutter, row, style);
                }
                continue;
            }

            let text = self.doc.line_text(line);
            let local_from = (from - line_start).min(text.len());
            let local_to = (to - line_start).min(text.len());
            let visual_from = util::visual_column(&text, local_from, self.doc.tab_width().max(1));
            let visual_to = util::visual_column(&text, local_to, self.doc.tab_width().max(1));
            let first_x = gutter.max(gutter + visual_from.saturating_sub(self.left_column));
            let last_x = self
                .cols
                .min(gutter + visual_to.saturating_sub(self.left_column));
            for x in first_x..last_x {
                self.screen.apply_style(x, row, style);
            }
        }
    }

    pub(super) fn render_find_matches(&mut self, first_line: usize, rows: usize, gutter: usize) {
        if !self.find.active || self.find.matches.is_empty() || rows == 0 {
            return;
        }
        let match_style = self.theme.match_style();
        let current_style = self.theme.current_match_style();

        for (index, match_) in self.find.matches.iter().enumerate() {
            let line = match_.line;
            if line < first_line || line >= first_line + rows {
                continue;
            }
            let line_start = self.doc.line_start(line);
            let line_end = self.doc.line_end(line);
            if match_.start > line_end {
                continue;
            }
            let text = self.doc.line_text(line);
            let local_from = (match_.start - line_start).min(text.len());
            let local_to = (match_.end - line_start).min(text.len());
            if local_to <= local_from {
                continue;
            }
            let visual_from = util::visual_column(&text, local_from, self.doc.tab_width().max(1));
            let visual_to = util::visual_column(&text, local_to, self.doc.tab_width().max(1));
            if visual_to <= self.left_column {
                continue;
            }
            let first_x = gutter.max(gutter + visual_from.saturating_sub(self.left_column));
            let last_x = self
                .cols
                .min(gutter + visual_to.saturating_sub(self.left_column));
            let style = if self.find.index == Some(index) {
                current_style
            } else {
                match_style
            };
            for x in first_x..last_x {
                self.screen.apply_style(x, line - first_line, style);
            }
        }
    }

    pub(super) fn render_cursor(&mut self, first_line: usize, rows: usize, gutter: usize) {
        for selection in self.capture_all_selections() {
            self.render_cursor_at(selection.cursor, first_line, rows, gutter);
        }
    }

    fn render_cursor_at(&mut self, cursor: usize, first_line: usize, rows: usize, gutter: usize) {
        let line = self.doc.line_of_offset(cursor);
        if line < first_line || line >= first_line + rows {
            return;
        }
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = cursor.saturating_sub(start).min(text.len());
        let column = util::visual_column(&text, local, self.doc.tab_width().max(1));
        if column < self.left_column {
            return;
        }
        let x = gutter + (column - self.left_column);
        if x >= self.cols {
            return;
        }
        let row = line - first_line;
        let mut style = self.screen.at(x, row).style;
        style.reverse = true;
        self.screen.apply_style(x, row, style);
    }

    pub(super) fn render_status_line(&mut self) {
        if self.rows == 0 {
            return;
        }
        let row = self.status_row();
        let style = self.status_style();
        self.screen.fill(0, row as i64, self.cols, 1, ' ', style);

        let line = self.cursor_line();
        let column = self.cursor_visual_column();
        let language = self.active_language();
        let mut left = format!("{}    {}", self.mode_label(), self.doc.display_name());
        if self.doc.is_dirty() {
            left.push_str(" [+]");
        }
        if let Some(language) = language {
            left.push_str(&format!("  [{}]", language.name));
        }
        let pane_count = self.pane_count();
        if pane_count > 1 {
            left.push_str(&format!(
                "  [pane {}/{}]",
                self.active_pane_ordinal(),
                pane_count
            ));
        }
        if self.lsp.is_running() {
            let name = self.lsp.server_name();
            let count = self
                .doc
                .path
                .as_deref()
                .map(|path| self.lsp.count_diagnostics_for_path(path))
                .unwrap_or(0);
            if count > 0 {
                left.push_str(&format!("  LSP: {name} ({count})"));
            } else {
                left.push_str(&format!("  LSP: {name}"));
            }
        }
        let debug_status = self.debug_status_text();
        if !debug_status.is_empty() {
            left.push_str("  ");
            left.push_str(debug_status);
        }
        let diagnostic_count = self
            .doc
            .path
            .as_deref()
            .map(|path| self.lsp.count_diagnostics_for_path(path))
            .unwrap_or(0);
        let mut right = if diagnostic_count > 0 {
            format!(
                "● {}  {}:{}  {}/{}  {}",
                diagnostic_count,
                line + 1,
                column + 1,
                line + 1,
                self.doc.line_count(),
                self.doc.eol.label()
            )
        } else {
            format!(
                "{}:{}  {}/{}  {}",
                line + 1,
                column + 1,
                line + 1,
                self.doc.line_count(),
                self.doc.eol.label()
            )
        };
        if self.doc.bom {
            right.push_str(" BOM");
        }

        let right_width = util::text_width(&right);
        let right_x = if self.cols > right_width + 1 {
            self.cols - right_width - 1
        } else {
            1
        };
        let left_max = right_x.saturating_sub(1);
        if left_max > 0 {
            self.screen.put_str(1, row as i64, &left, style, left_max);
        }
        if right_x < self.cols {
            self.screen.put_str(
                right_x as i64,
                row as i64,
                &right,
                style,
                self.cols - right_x,
            );
        }
    }

    pub(super) fn render_bottom_line(&mut self) {
        if self.rows == 0 {
            return;
        }
        let row = self.command_row();
        self.screen
            .fill(0, row as i64, self.cols, 1, ' ', self.theme.base());
        let width = self.cols.saturating_sub(1);

        if let Some(prompt) = &self.sidebar_prompt {
            let text = format!("{}{}", prompt.label, prompt.text);
            self.screen
                .put_str(1, row as i64, &text, self.theme.accent_style(), width);
            let column = util::text_width(&prompt.label)
                + util::visual_column(prompt.text.as_bytes(), prompt.cursor, 1);
            let x = 1 + column;
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

        if self.mode == Mode::Command {
            self.screen
                .put(1, row as i64, ':', self.theme.accent_style(), 1);
            self.screen.put_str(
                2,
                row as i64,
                &self.command_line,
                self.theme.text(),
                self.cols.saturating_sub(2),
            );
            let cursor_column =
                util::visual_column(self.command_line.as_bytes(), self.command_cursor, 1);
            let x = 2 + cursor_column;
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

        if let Some(input) = &self.goto_prompt {
            let text = format!("Go to line: {input}");
            self.screen
                .put_str(1, row as i64, &text, self.theme.accent_style(), width);
            let x = 1 + util::text_width(&text);
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

        if self.pending != Pending::None {
            let text = match self.pending {
                Pending::Quit => "Unsaved changes. Quit without saving? (y/N)",
                Pending::Open => "Unsaved changes. Discard and open file? (y/N)",
                Pending::SidebarDelete => {
                    if self.pending_sidebar_deletes.len() == 1 {
                        "Delete the selected item? (y/N)"
                    } else {
                        "Delete the selected items? (y/N)"
                    }
                }
                Pending::SidebarDiff => "Unsaved changes. Discard and show diff? (y/N)",
                Pending::None => "",
            };
            self.screen
                .put_str(1, row as i64, text, self.theme.warning_style(), width);
            return;
        }

        if !self.message.is_empty() {
            let style = if self.message_is_error {
                self.theme.error_style()
            } else {
                self.theme.text()
            };
            self.screen
                .put_str(1, row as i64, &self.message, style, width);
        }
    }

    pub(super) fn render_space_menu(&mut self) {
        if !self.space_menu_open {
            return;
        }

        let mut content_width = 0usize;
        for item in SPACE_ITEMS {
            content_width = content_width.max(item.label.len() + 4);
        }
        let mut width = content_width.max(14) + 2;
        let height = SPACE_ITEMS.len() + 2;
        let status_row = self.status_row();
        if self.cols < 10 || status_row < height + 1 {
            return;
        }
        if width > self.cols - 2 {
            width = self.cols - 2;
        }
        let x = if self.cols >= width + 1 {
            self.cols - width - 1
        } else {
            0
        };
        let y = if status_row >= height {
            status_row - height
        } else {
            0
        };
        self.draw_box(x, y, width, height, "SPACE");

        for (index, item) in SPACE_ITEMS.iter().enumerate() {
            let row = y + 1 + index;
            let key = item.key.to_string();
            self.screen.put_str(
                (x + 2) as i64,
                row as i64,
                &key,
                self.theme.accent_style(),
                width.saturating_sub(3),
            );
            self.screen.put_str(
                (x + 4) as i64,
                row as i64,
                item.label,
                self.theme.text(),
                width.saturating_sub(5),
            );
        }
    }

    pub(super) fn draw_box(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        title: &str,
    ) {
        if width < 2 || height < 2 {
            return;
        }
        self.screen
            .fill(x as i64, y as i64, width, height, ' ', self.theme.base());
        let border = self.theme.border_style();

        for column in 1..width.saturating_sub(1) {
            self.screen
                .put((x + column) as i64, y as i64, '─', border, 1);
            self.screen
                .put((x + column) as i64, (y + height - 1) as i64, '─', border, 1);
        }
        for row in 1..height.saturating_sub(1) {
            self.screen.put(x as i64, (y + row) as i64, '│', border, 1);
            self.screen
                .put((x + width - 1) as i64, (y + row) as i64, '│', border, 1);
        }
        self.screen.put(x as i64, y as i64, '┌', border, 1);
        self.screen
            .put((x + width - 1) as i64, y as i64, '┐', border, 1);
        self.screen
            .put(x as i64, (y + height - 1) as i64, '└', border, 1);
        self.screen.put(
            (x + width - 1) as i64,
            (y + height - 1) as i64,
            '┘',
            border,
            1,
        );

        if !title.is_empty() && width > 6 {
            let text = format!(" {title} ");
            self.screen.put_str(
                (x + 2) as i64,
                y as i64,
                &text,
                self.theme.accent_style(),
                width.saturating_sub(4),
            );
        }
    }

    pub(super) fn render_picker(&mut self) {
        if self.picker.kind == PickerKind::None {
            return;
        }
        self.update_picker_preview();
        let available = self.status_row();
        let preview_height = self.picker_preview_height(available);
        let height = self.picker_dialog_height(available);
        if height < 4 || self.cols < 20 {
            return;
        }
        let width = (self.cols - 2).min(88);
        if width < 20 {
            return;
        }
        let total_height = height
            + if preview_height > 0 {
                preview_height + 1
            } else {
                0
            };
        let x = (self.cols - width) / 2;
        let y = if available >= total_height {
            (available - total_height) / 2
        } else {
            0
        };
        let title = match self.picker.kind {
            PickerKind::Files => "Open file",
            PickerKind::DiffWith => "Diff with",
            PickerKind::Search => "Search across files",
            PickerKind::Themes => "Select theme",
            PickerKind::Palette => "Command palette",
            PickerKind::CodeActions => "Code actions",
            PickerKind::Symbols => "Symbols",
            PickerKind::Problems => {
                if self.problems_workspace {
                    "Workspace diagnostics"
                } else {
                    "Document diagnostics"
                }
            }
            PickerKind::Folders => {
                if self
                    .sidebar_transfer
                    .as_ref()
                    .map(|(_, moving)| *moving)
                    .unwrap_or(false)
                {
                    "Move to folder"
                } else {
                    "Copy to folder"
                }
            }
            PickerKind::None => "",
        };
        self.draw_box(x, y, width, height, title);

        self.screen.put(
            (x + 2) as i64,
            (y + 1) as i64,
            '>',
            self.theme.accent_style(),
            1,
        );
        let query_width = width.saturating_sub(5);
        self.screen.put_str(
            (x + 4) as i64,
            (y + 1) as i64,
            &self.picker.query,
            self.theme.text(),
            query_width,
        );
        let query_column =
            util::visual_column(self.picker.query.as_bytes(), self.picker.query.len(), 1);
        if query_column < query_width {
            let cursor_x = x + 4 + query_column;
            if cursor_x < x + width - 1 {
                let mut style = self.screen.at(cursor_x, y + 1).style;
                style.reverse = true;
                self.screen.apply_style(cursor_x, y + 1, style);
            }
        }

        let list_rows = if height >= 3 { height - 3 } else { 1 };
        if self.picker.selected < self.picker.top {
            self.picker.top = self.picker.selected;
        }
        if self.picker.selected >= self.picker.top + list_rows {
            self.picker.top = self.picker.selected + 1 - list_rows;
        }
        let max_top = self.picker.items.len().saturating_sub(list_rows);
        if self.picker.top > max_top {
            self.picker.top = max_top;
        }

        for row in 0..list_rows {
            let index = self.picker.top + row;
            if index >= self.picker.items.len() {
                break;
            }
            let screen_row = y + 2 + row;
            let style = if index == self.picker.selected {
                self.theme.selected_item_style()
            } else {
                self.theme.text()
            };
            self.screen
                .fill((x + 1) as i64, screen_row as i64, width - 2, 1, ' ', style);
            let display = &self.picker.items[index].display;
            self.screen.put_str(
                (x + 2) as i64,
                screen_row as i64,
                display,
                style,
                width.saturating_sub(4),
            );
        }

        let status = if let Some(status) = &self.picker.status {
            status.clone()
        } else if self.picker.kind == PickerKind::Search {
            if self.picker.query.is_empty() {
                "Type to search".to_string()
            } else if self.picker.total == 0 {
                "No matches".to_string()
            } else {
                format!(
                    "{} match{}",
                    self.picker.total,
                    if self.picker.total == 1 { "" } else { "es" }
                )
            }
        } else if self.picker.kind == PickerKind::Palette {
            if self.picker.total == 0 {
                "No matching commands".to_string()
            } else {
                format!(
                    "{} command{}",
                    self.picker.total,
                    if self.picker.total == 1 { "" } else { "s" }
                )
            }
        } else if self.picker.total == 0 {
            "No items".to_string()
        } else {
            format!("{} item(s)", self.picker.total)
        };
        self.screen.put_str(
            (x + 2) as i64,
            (y + height - 1) as i64,
            &status,
            self.theme.accent_style(),
            width.saturating_sub(4),
        );

        if preview_height > 0 {
            self.render_picker_preview(x, y + height + 1, width, preview_height);
        }
    }

    pub(super) fn find_dialog_height(&self) -> usize {
        if !self.find.active {
            return 0;
        }
        let available = self.status_row();
        let height = if self.find.replace { 6usize } else { 4usize };
        let width = self.cols.saturating_sub(2).min(72);
        if available < height + 1 || self.cols < 20 || width < 20 {
            return 0;
        }
        height
    }

    pub(super) fn render_find(&mut self) {
        let height = self.find_dialog_height();
        if height == 0 {
            return;
        }
        let available = self.status_row();
        let width = self.cols.saturating_sub(2).min(72);
        let x = (self.cols - width) / 2;
        let y = available - height;
        let title = if self.find.replace {
            "Regex replace"
        } else if self.find.multi_cursor {
            if self.find.regex {
                if self.find.scope.is_empty() {
                    "Regex find (multi-cursor)"
                } else {
                    "Regex find in selection (multi-cursor)"
                }
            } else if self.find.scope.is_empty() {
                "Find (multi-cursor)"
            } else {
                "Find in selection (multi-cursor)"
            }
        } else if self.find.regex {
            "Regex find"
        } else {
            "Find"
        };
        self.draw_box(x, y, width, height, title);

        let prompt_style = self.theme.accent_style();
        let muted_style = self.theme.muted_style();
        let text_style = self.theme.text();
        let query = self.find.query.clone();
        let replacement = self.find.replacement.clone();

        if !self.find.replace {
            self.screen
                .put((x + 2) as i64, (y + 1) as i64, '>', prompt_style, 1);
            let query_width = width.saturating_sub(5);
            self.screen.put_str(
                (x + 4) as i64,
                (y + 1) as i64,
                &query,
                text_style,
                query_width,
            );
            if self.find.editing {
                self.render_find_cursor(x + 4, y + 1, &query, self.find.query_cursor, query_width);
            }
        } else {
            let label_width = 9usize;
            let field_x = x + 2 + label_width;
            let field_width = width.saturating_sub(label_width + 4);
            let find_label_style = if self.find.editing_replacement {
                muted_style
            } else {
                prompt_style
            };
            let replace_label_style = if self.find.editing_replacement {
                prompt_style
            } else {
                muted_style
            };

            self.screen.put_str(
                (x + 2) as i64,
                (y + 1) as i64,
                "Find",
                find_label_style,
                label_width,
            );
            self.screen.put_str(
                field_x as i64,
                (y + 1) as i64,
                &query,
                text_style,
                field_width,
            );
            self.screen.put_str(
                (x + 2) as i64,
                (y + 2) as i64,
                "Replace",
                replace_label_style,
                label_width,
            );
            self.screen.put_str(
                field_x as i64,
                (y + 2) as i64,
                &replacement,
                text_style,
                field_width,
            );

            if self.find.editing {
                if self.find.editing_replacement {
                    self.render_find_cursor(
                        field_x,
                        y + 2,
                        &replacement,
                        self.find.replacement_cursor,
                        field_width,
                    );
                } else {
                    self.render_find_cursor(
                        field_x,
                        y + 1,
                        &query,
                        self.find.query_cursor,
                        field_width,
                    );
                }
            }
        }

        let mut status_style = self.theme.accent_style();
        let status = if !self.find.error.is_empty() {
            status_style = self.theme.error_style();
            self.find.error.clone()
        } else if query.is_empty() {
            if self.find.regex {
                "Type a regular expression".to_string()
            } else {
                "Type to search".to_string()
            }
        } else if self.find.matches.is_empty() {
            "No matches".to_string()
        } else {
            let position = self.find.index.unwrap_or(0) + 1;
            if self.find.replace {
                format!("{position}/{} matches", self.find.matches.len())
            } else {
                let enter_hint = if self.find.multi_cursor {
                    "Enter: select all"
                } else {
                    "Enter: navigate"
                };
                if self.find.editing {
                    format!(
                        "{position}/{} matches  {enter_hint}  Esc: cancel",
                        self.find.matches.len()
                    )
                } else {
                    format!(
                        "{position}/{} matches  n: next  p: previous  Esc: exit",
                        self.find.matches.len()
                    )
                }
            }
        };

        if self.find.replace {
            self.screen.put_str(
                (x + 2) as i64,
                (y + 3) as i64,
                &status,
                status_style,
                width.saturating_sub(4),
            );
            let hint = if self.find.error.is_empty() {
                "Tab: switch  Enter: replace  Ctrl+A: replace all  Esc: close"
            } else {
                "Esc: close"
            };
            self.screen.put_str(
                (x + 2) as i64,
                (y + 4) as i64,
                hint,
                muted_style,
                width.saturating_sub(4),
            );
        } else {
            self.screen.put_str(
                (x + 2) as i64,
                (y + 2) as i64,
                &status,
                status_style,
                width.saturating_sub(4),
            );
        }
    }

    pub(super) fn render_find_cursor(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        cursor: usize,
        width: usize,
    ) {
        if width == 0 {
            return;
        }
        let column = util::visual_column(text.as_bytes(), cursor.min(text.len()), 1);
        if column >= width {
            return;
        }
        let cursor_x = x + column;
        if cursor_x >= x + width || cursor_x >= self.cols {
            return;
        }
        let mut style = self.screen.at(cursor_x, y).style;
        style.reverse = true;
        self.screen.apply_style(cursor_x, y, style);
    }

    pub(super) fn render_help(&mut self) {
        if !self.help.visible {
            return;
        }
        self.screen.clear(self.theme.base());
        let (width, height, x, y, rows) = self.help.layout(self.cols, self.rows);
        self.draw_box(x, y, width, height, " Help ");
        self.help.top = self.help.top.min(self.help.max_top(self.cols, self.rows));

        let section_style = self.theme.accent_style();
        let mut key_style = self.theme.syntax_style(SyntaxKind::Default);
        key_style.fg = self.theme.accent;
        let text_style = self.theme.text();
        let muted_style = self.theme.muted_style();
        let mut key_width = (width / 3).clamp(8, 30);
        key_width = key_width.min(width.saturating_sub(8).max(4));

        for row in 0..rows {
            let index = self.help.top + row;
            if index >= HELP_LINES.len() {
                break;
            }
            let line = HELP_LINES[index];
            let screen_row = y + 1 + row;
            match line.kind {
                HelpLineKind::Section => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        section_style,
                        width.saturating_sub(4),
                    );
                }
                HelpLineKind::Entry => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        key_style,
                        key_width,
                    );
                    self.screen.put_str(
                        (x + 2 + key_width) as i64,
                        screen_row as i64,
                        line.right,
                        text_style,
                        width.saturating_sub(key_width + 4),
                    );
                }
                HelpLineKind::Text => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        muted_style,
                        width.saturating_sub(4),
                    );
                }
                HelpLineKind::Blank => {}
            }
        }

        let max_top = self.help.max_top(self.cols, self.rows);
        if max_top > 0 && height > 2 {
            let track_height = height.saturating_sub(2).max(1);
            let thumb_height =
                ((track_height as f64 * rows as f64) / HELP_LINES.len() as f64).round() as usize;
            let thumb_height = thumb_height.max(1);
            let thumb_top = (track_height - thumb_height) * self.help.top / max_top;
            for row in 0..track_height {
                let thumb = row >= thumb_top && row < thumb_top + thumb_height;
                self.screen.put(
                    (x + width - 2) as i64,
                    (y + 1 + row) as i64,
                    if thumb { '█' } else { '│' },
                    if thumb { key_style } else { muted_style },
                    1,
                );
            }
        }

        let hint_row = y + height;
        if hint_row < self.rows {
            let hint = "Esc / q close    Up/Down scroll    PgUp/PgDn page    Home/End";
            let width = util::text_width(hint);
            let col = self.cols.saturating_sub(width) / 2;
            self.screen.put_str(
                col as i64,
                hint_row as i64,
                hint,
                muted_style,
                self.cols.saturating_sub(col),
            );
        }
    }

    pub(super) fn reload_themes(&mut self) {
        let active = self.theme.id.clone();
        self.themes = ThemeRegistry::load_all();
        if !self.themes.set_active(&active) {
            if let Some(first) = self.themes.themes().first().cloned() {
                self.theme = first;
            }
        } else if let Some(theme) = self.themes.find(&active) {
            self.theme = theme.clone();
        }
        self.force_full_redraw = true;
        if let Some(error) = self.themes.errors().first() {
            self.set_error(&format!("theme: {error}"));
        } else {
            self.set_message(&format!("Reloaded {} theme(s)", self.themes.themes().len()));
        }
    }
}
