//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn request_open(&mut self, raw_path: &str, line: usize, column: usize, force: bool) {
        let expanded = util::expand_home(raw_path);
        if self.doc.is_dirty() && !force {
            self.pending_path = Some(expanded);
            self.pending_line = line;
            self.pending_column = column;
            self.pending = Pending::Open;
            return;
        }
        self.load_file(&expanded, line, column);
    }

    pub(super) fn load_file(&mut self, path: &str, line: usize, column: usize) {
        let new_doc = match Document::open(path) {
            Ok(doc) => doc,
            Err(error) => {
                self.set_error(&format!("cannot open '{path}': {error}"));
                return;
            }
        };
        self.fire_plugin_buffer_event("buffer_closed");
        self.doc = new_doc;
        self.mode = Mode::Normal;
        self.anchor = 0;
        self.cursor = 0;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.top_line = 0;
        self.left_column = 0;
        self.pending_g = false;
        self.find = FindState::new();
        self.language_override = None;
        self.jumps.clear();
        self.jump_index = -1;
        self.refresh_language();
        self.auto_detect_indentation_on_open();

        let target_line = line.min(self.doc.line_count() - 1);
        let start = self.doc.line_start(target_line);
        let text = self.doc.line_text(target_line);
        let mut local = column.min(text.len());
        // Search matches and LSP positions are byte offsets. Invalid UTF-8 can
        // leave a column inside a scalar; advance to the next boundary.
        while local < text.len() && (text[local] & 0xC0) == 0x80 {
            local += 1;
        }
        self.cursor = (start + local).min(self.doc.line_end(target_line));
        self.snap_cursor();
        self.anchor = self.cursor;
        self.top_line = target_line.saturating_sub(self.text_rows() / 2);
        self.doc.break_undo_group();
        self.last_revision = self.doc.revision();
        self.restart_lsp_for_buffer();
        self.fire_plugin_buffer_event("buffer_opened");
        self.set_message(&format!("Opened {}", self.doc.display_name()));
    }

    pub(super) fn open_picker(&mut self, kind: PickerKind) {
        if kind == PickerKind::Themes {
            let active = self.theme.id.clone();
            self.themes = ThemeRegistry::load_all();
            self.themes.set_active(&active);
            self.theme_before_picker = Some(active);
        } else {
            self.theme_before_picker = None;
        }
        self.picker.kind = kind;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();

        if kind != PickerKind::Palette && !self.picker.files_loaded {
            self.picker.files = files::scan_with_ignored(20_000, self.sidebar.show_git_ignored);
            self.picker.files_loaded = true;
        }
        self.rebuild_picker();
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn cancel_picker(&mut self) {
        if self.picker.kind == PickerKind::Folders {
            self.sidebar_transfer = None;
            self.set_message("Cancelled.");
        }
        if self.picker.kind == PickerKind::Themes {
            if let Some(previous) = self.theme_before_picker.take() {
                if let Some(theme) = self.themes.find(&previous).cloned() {
                    self.theme = theme;
                    self.themes.set_active(&previous);
                }
            }
        }
        self.close_picker();
    }

    pub(super) fn close_picker(&mut self) {
        self.picker.kind = PickerKind::None;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();
    }

    pub(super) fn rebuild_picker(&mut self) {
        self.picker.items.clear();
        self.picker.total = 0;

        match self.picker.kind {
            PickerKind::Files => {
                for (index, path) in self.picker.files.iter().enumerate() {
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(path.as_bytes(), self.picker.query.as_bytes())
                            .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    if self.picker.items.len() >= 1000 {
                        continue;
                    }
                    self.picker.items.push(PickerItem {
                        display: path.clone(),
                        file_index: index,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: None,
                        palette_index: None,
                        action_index: None,
                        symbol_index: None,
                        problem: None,
                    });
                }
            }
            PickerKind::Search => {
                if !self.picker.query.is_empty() {
                    let hits = files::search(
                        &self.picker.files,
                        &self.picker.query,
                        500,
                        self.search_match_case,
                    );
                    self.picker.total = hits.len();
                    for hit in hits {
                        let path = &self.picker.files[hit.file_index];
                        self.picker.items.push(PickerItem {
                            display: format!("{}:{}: {}", path, hit.line + 1, hit.preview),
                            file_index: hit.file_index,
                            line: hit.line,
                            column: hit.column,
                            theme_id: None,
                            target: None,
                            palette_index: None,
                            action_index: None,
                            symbol_index: None,
                            problem: None,
                        });
                    }
                }
            }
            PickerKind::Folders => {
                let labels = ui_sidebar::folder_labels(&self.sidebar_folders, &self.sidebar.root);
                for (folder, label) in self.sidebar_folders.iter().zip(labels) {
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(
                            label.as_bytes(),
                            self.picker.query.as_bytes(),
                        )
                        .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: label,
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: Some(folder.clone()),
                        palette_index: None,
                        action_index: None,
                        symbol_index: None,
                        problem: None,
                    });
                }
            }
            PickerKind::Themes => {
                for theme in self.themes.themes() {
                    let haystack = format!(
                        "{} {} {} {}",
                        theme.name, theme.description, theme.author, theme.license
                    );
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(
                            haystack.as_bytes(),
                            self.picker.query.as_bytes(),
                        )
                        .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: theme.name.clone(),
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: Some(theme.id.clone()),
                        target: None,
                        palette_index: None,
                        action_index: None,
                        symbol_index: None,
                        problem: None,
                    });
                }
            }
            PickerKind::Palette => {
                for (index, command) in self.palette_commands.iter().enumerate() {
                    let haystack = format!("{} {}", command.title, command.description);
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(
                            haystack.as_bytes(),
                            self.picker.query.as_bytes(),
                        )
                        .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    if self.picker.items.len() >= 1000 {
                        continue;
                    }
                    self.picker.items.push(PickerItem {
                        display: command.title.clone(),
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: None,
                        palette_index: Some(index),
                        action_index: None,
                        symbol_index: None,
                        problem: None,
                    });
                }
            }
            PickerKind::CodeActions => {
                for (index, action) in self.code_actions.iter().enumerate() {
                    let label = if action.kind.is_empty() {
                        action.title.clone()
                    } else {
                        format!("[{}] {}", action.kind, action.title)
                    };
                    if !self.picker.query.is_empty()
                        && !label
                            .to_ascii_lowercase()
                            .contains(&self.picker.query.to_ascii_lowercase())
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: label,
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: None,
                        palette_index: None,
                        action_index: Some(index),
                        symbol_index: None,
                        problem: None,
                    });
                }
            }
            PickerKind::Symbols => {
                for (index, symbol) in self.document_symbols.iter().enumerate() {
                    let indent = " ".repeat(symbol.depth.min(12) * 2);
                    let mut label =
                        format!("{indent}{} {}", symbol_kind_name(symbol.kind), symbol.name);
                    if !symbol.detail.is_empty() {
                        label.push_str(&format!("  {}", symbol.detail));
                    }
                    if !self.picker.query.is_empty()
                        && !label
                            .to_ascii_lowercase()
                            .contains(&self.picker.query.to_ascii_lowercase())
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: label,
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: None,
                        palette_index: None,
                        action_index: None,
                        symbol_index: Some(index),
                        problem: None,
                    });
                }
            }
            PickerKind::Problems => self.populate_problem_items(),
            PickerKind::None => {}
        }

        if self.picker.selected >= self.picker.items.len() {
            self.picker.selected = self.picker.items.len().saturating_sub(1);
        }
        if self.picker.top > self.picker.selected {
            self.picker.top = self.picker.selected;
        }
    }

    pub(super) fn handle_picker(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.cancel_picker();
            return;
        }

        if event.key == Key::Char && !event.ctrl && !event.alt && event.ch != '\0' {
            self.picker.query.push(event.ch);
            self.picker.selected = 0;
            self.picker.top = 0;
            self.rebuild_picker();
            return;
        }

        match event.key {
            Key::Backspace => {
                if self.picker.query.pop().is_some() {
                    self.picker.selected = 0;
                    self.picker.top = 0;
                    self.rebuild_picker();
                }
            }
            Key::Up => {
                if self.picker.selected > 0 {
                    self.picker.selected -= 1;
                }
                self.preview_theme_selection();
            }
            Key::Down => {
                if self.picker.selected + 1 < self.picker.items.len() {
                    self.picker.selected += 1;
                }
                self.preview_theme_selection();
            }
            Key::PageUp => {
                let rows = self.picker_list_rows();
                self.picker.selected = self.picker.selected.saturating_sub(rows);
                self.preview_theme_selection();
            }
            Key::PageDown => {
                let rows = self.picker_list_rows();
                if !self.picker.items.is_empty() {
                    self.picker.selected =
                        (self.picker.selected + rows).min(self.picker.items.len() - 1);
                }
                self.preview_theme_selection();
            }
            Key::Home => {
                self.picker.selected = 0;
                self.preview_theme_selection();
            }
            Key::End => {
                self.picker.selected = self.picker.items.len().saturating_sub(1);
                self.preview_theme_selection();
            }
            Key::Enter => self.open_picker_selection(),
            _ => {}
        }
    }

    pub(super) fn open_picker_selection(&mut self) {
        if self.picker.items.is_empty() || self.picker.selected >= self.picker.items.len() {
            return;
        }
        if self.picker.kind == PickerKind::Folders {
            let target = self.picker.items[self.picker.selected].target.clone();
            self.close_picker();
            if let Some(target) = target {
                self.complete_transfer(target);
            }
            return;
        }
        if self.picker.kind == PickerKind::Themes {
            let item = &self.picker.items[self.picker.selected];
            if let Some(id) = item.theme_id.clone() {
                self.apply_theme(&id, true);
            }
            self.theme_before_picker = None;
            self.close_picker();
            return;
        }
        if self.picker.kind == PickerKind::Palette {
            let command = self.picker.items[self.picker.selected].palette_index;
            self.close_picker();
            if let Some(index) = command {
                self.run_palette_command(index);
            }
            return;
        }
        if self.picker.kind == PickerKind::CodeActions {
            let action = self.picker.items[self.picker.selected]
                .action_index
                .and_then(|index| self.code_actions.get(index).cloned());
            self.close_picker();
            if let Some(action) = action {
                self.apply_code_action(&action);
            }
            return;
        }
        if self.picker.kind == PickerKind::Symbols {
            let symbol = self.picker.items[self.picker.selected]
                .symbol_index
                .and_then(|index| self.document_symbols.get(index).cloned());
            self.close_picker();
            if let Some(symbol) = symbol {
                let path = lsp::uri_to_path(&symbol.uri);
                if path.is_empty() {
                    self.set_error("The language server returned an invalid symbol location.");
                    return;
                }
                let same =
                    self.doc.path.as_deref().is_some_and(|current| {
                        git::same_path(Path::new(current), Path::new(&path))
                    });
                self.record_jump();
                if same {
                    self.move_to_line_character(symbol.line, symbol.character);
                } else {
                    let byte = super::lsp_ui::lsp_column_to_byte_offset(
                        &path,
                        symbol.line,
                        symbol.character,
                    );
                    self.request_open(&path, symbol.line.max(0) as usize, byte, false);
                }
            }
            return;
        }
        if self.picker.kind == PickerKind::Problems {
            let problem = self.picker.items[self.picker.selected].problem.clone();
            self.close_picker();
            if let Some((path, line, column)) = problem {
                let byte =
                    super::lsp_ui::lsp_column_to_byte_offset(&path, line as i64, column as i64);
                self.request_open(&path, line, byte, false);
            }
            return;
        }
        let item = &self.picker.items[self.picker.selected];
        let path = self.picker.files[item.file_index].clone();
        let line = item.line;
        let column = item.column;
        self.close_picker();
        self.request_open(&path, line, column, false);
    }

    pub(super) fn preview_theme_selection(&mut self) {
        if self.picker.kind != PickerKind::Themes {
            return;
        }
        let Some(item) = self.picker.items.get(self.picker.selected) else {
            return;
        };
        let Some(id) = item.theme_id.clone() else {
            return;
        };
        if let Some(theme) = self.themes.find(&id).cloned() {
            self.theme = theme;
        }
    }

    pub(super) fn apply_theme(&mut self, id: &str, persist: bool) {
        let Some(theme) = self.themes.find(id).cloned() else {
            self.set_error(&format!("unknown theme '{id}'"));
            return;
        };
        let name = theme.name.clone();
        self.theme = theme;
        self.themes.set_active(id);
        if persist {
            if let Err(error) = crate::ui::theme::save_active_theme(id) {
                self.set_error(&format!("theme: {error}"));
                return;
            }
        }
        self.force_full_redraw = true;
        self.set_message(&format!("Theme: {name}"));
    }

    pub(super) fn picker_height(&self) -> usize {
        let available = self.status_row();
        if available == 0 {
            return 0;
        }
        available.saturating_sub(1).min(18)
    }

    pub(super) fn picker_list_rows(&self) -> usize {
        let height = self.picker_height();
        if height >= 3 {
            height - 3
        } else {
            1
        }
    }
}

fn symbol_kind_name(kind: i64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Symbol",
    }
}
