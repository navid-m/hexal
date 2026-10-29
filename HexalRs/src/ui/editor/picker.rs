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
        self.assign_active_document(new_doc);
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
        self.refresh_git_diff();

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
        self.top_line = target_line.saturating_sub(self.active_pane_rows() / 2);
        self.doc.break_undo_group();
        self.last_revision = self.doc.revision();
        self.restart_lsp_for_buffer();
        self.fire_plugin_buffer_event("buffer_opened");
        self.set_message(&format!("Opened {}", self.doc.display_name()));
    }

    pub(super) fn open_picker(&mut self, kind: PickerKind) {
        self.cancel_picker_work();
        if kind != PickerKind::DiffWith {
            self.diff_with_source = None;
        }
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
        self.picker.status = None;

        if Self::picker_needs_files(kind) && !self.picker.files_loaded {
            self.start_picker_scan();
        } else {
            self.rebuild_picker();
        }
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn picker_needs_files(kind: PickerKind) -> bool {
        matches!(
            kind,
            PickerKind::Files | PickerKind::Search | PickerKind::DiffWith
        )
    }

    fn next_picker_generation(&mut self) -> u64 {
        self.picker_generation = self.picker_generation.wrapping_add(1);
        self.picker_generation
    }

    pub(super) fn cancel_picker_work(&mut self) {
        if let Some(task) = self.picker_scan.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(task) = self.picker_search.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn start_picker_scan(&mut self) {
        self.cancel_picker_work();
        let generation = self.next_picker_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let root = self.sidebar.root.clone();
        let show_ignored = self.sidebar.show_git_ignored;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let files =
                files::scan_root_with_cancel(&root, 20_000, show_ignored, Some(&thread_cancel));
            if !thread_cancel.load(Ordering::Relaxed) {
                let _ = sender.send(PickerScanResult { generation, files });
            }
        });
        self.picker_scan = Some(PickerScanTask { receiver, cancel });
        self.picker.status = Some("Loading files...".into());
        self.force_full_redraw = true;
    }

    fn start_picker_search(&mut self) {
        if let Some(task) = self.picker_search.take() {
            task.cancel.store(true, Ordering::Relaxed);
        }
        let generation = self.next_picker_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let files = self.picker.files.clone();
        let root = self.sidebar.root.clone();
        let query = self.picker.query.clone();
        let match_case = self.search_match_case;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let hits = files::search(
                &root,
                files.as_slice(),
                &query,
                500,
                match_case,
                &thread_cancel,
            );
            if !thread_cancel.load(Ordering::Relaxed) {
                let _ = sender.send(PickerSearchResult { generation, hits });
            }
        });
        self.picker_search = Some(PickerSearchTask { receiver, cancel });
        self.picker.status = Some("Searching...".into());
        self.force_full_redraw = true;
    }

    pub(super) fn poll_picker_work(&mut self) {
        let mut scan_applied = false;
        if let Some(task) = self.picker_scan.take() {
            match task.receiver.try_recv() {
                Ok(result) => {
                    if result.generation == self.picker_generation {
                        self.picker.files = Arc::new(result.files);
                        self.picker.files_loaded = true;
                        self.picker.status = None;
                        scan_applied = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => self.picker_scan = Some(task),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.picker.files_loaded = false;
                    self.picker.status = Some("E: file scan stopped unexpectedly".into());
                    self.force_full_redraw = true;
                }
            }
        }
        if scan_applied {
            if self.picker.kind != PickerKind::None {
                self.rebuild_picker();
                self.force_full_redraw = true;
            }
        }

        let Some(task) = self.picker_search.take() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(result) => {
                if result.generation != self.picker_generation
                    || self.picker.kind != PickerKind::Search
                {
                    return;
                }
                self.picker.items.clear();
                self.picker.total = result.hits.len();
                for hit in result.hits {
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
                self.picker.selected = 0;
                self.picker.top = 0;
                self.picker.status = None;
                self.force_full_redraw = true;
            }
            Err(mpsc::TryRecvError::Empty) => self.picker_search = Some(task),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.picker.status = Some("E: search stopped unexpectedly".into());
                self.force_full_redraw = true;
            }
        }
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
        if self.picker.kind == PickerKind::DiffWith {
            self.diff_with_source = None;
        }
        self.cancel_picker_work();
        self.picker.kind = PickerKind::None;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();
        self.picker.status = None;
    }

    pub(super) fn rebuild_picker(&mut self) {
        let keep_search_results =
            self.picker.kind == PickerKind::Search && !self.picker.query.is_empty();
        if !keep_search_results {
            self.picker.items.clear();
            self.picker.total = 0;
        }
        let diff_with_source = if self.picker.kind == PickerKind::DiffWith {
            self.diff_with_source.clone()
        } else {
            None
        };

        match self.picker.kind {
            PickerKind::Files | PickerKind::DiffWith => {
                for (index, path) in self.picker.files.iter().enumerate() {
                    if let Some(source) = &diff_with_source {
                        if git::same_path(source, &self.sidebar.root.join(path)) {
                            continue;
                        }
                    }
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
                if self.picker.query.is_empty() {
                    self.picker.status = None;
                    if let Some(task) = self.picker_search.take() {
                        task.cancel.store(true, Ordering::Relaxed);
                    }
                } else if self.picker.files_loaded {
                    self.start_picker_search();
                } else if self.picker_scan.is_none() {
                    self.start_picker_scan();
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
                let same =
                    self.doc.path.as_deref().is_some_and(|current| {
                        git::same_path(Path::new(current), Path::new(&path))
                    });
                if same {
                    self.record_jump();
                    self.move_to_line_character(line as i64, column as i64);
                } else {
                    let byte =
                        super::lsp_ui::lsp_column_to_byte_offset(&path, line as i64, column as i64);
                    self.record_jump();
                    self.request_open(&path, line, byte, false);
                }
            }
            return;
        }
        if self.picker.kind == PickerKind::DiffWith {
            let target = self
                .sidebar
                .root
                .join(&self.picker.files[self.picker.items[self.picker.selected].file_index]);
            let source = self.diff_with_source.take();
            self.close_picker();
            if let Some(source) = source {
                self.begin_sidebar_diff(&[(source, false), (target, false)]);
            }
            return;
        }
        let item = &self.picker.items[self.picker.selected];
        let path = self
            .sidebar
            .root
            .join(&self.picker.files[item.file_index])
            .to_string_lossy()
            .into_owned();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;
    use std::time::{Duration, Instant};

    fn wait_for_picker_scan(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !editor.picker.files_loaded && Instant::now() < deadline {
            editor.poll_picker_work();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(editor.picker.files_loaded);
    }

    fn wait_for_picker_search(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while editor.picker.status.is_some() && Instant::now() < deadline {
            editor.poll_picker_work();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(editor.picker.status.is_none(), "{:?}", editor.picker.status);
    }

    fn problem_item(path: String, line: usize, column: usize) -> PickerItem {
        PickerItem {
            display: String::new(),
            file_index: 0,
            line: 0,
            column: 0,
            theme_id: None,
            target: None,
            palette_index: None,
            action_index: None,
            symbol_index: None,
            problem: Some((path, line, column)),
        }
    }

    #[test]
    fn file_picker_uses_the_workspace_root() {
        let root =
            std::env::temp_dir().join(format!("hexal-picker-root-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("alpha.txt"), b"alpha\n").unwrap();
        std::fs::write(root.join("sub/beta.txt"), b"beta\n").unwrap();

        let mut editor = Editor::with_options(Document::new(), 80, 24, Some(root.clone()), true);
        editor.open_picker(PickerKind::Files);
        wait_for_picker_scan(&mut editor);
        assert_eq!(editor.sidebar.root, root);
        assert!(editor.picker.files.contains(&"alpha.txt".to_string()));
        assert!(editor.picker.files.contains(&"sub/beta.txt".to_string()));

        let index = editor
            .picker
            .items
            .iter()
            .position(|item| editor.picker.files[item.file_index] == "sub/beta.txt")
            .unwrap();
        editor.picker.selected = index;
        editor.open_picker_selection();
        assert_eq!(
            editor.doc.path.as_deref(),
            Some(root.join("sub/beta.txt").to_string_lossy().as_ref())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_keeps_previous_results_while_typing() {
        let mut editor = Editor::new(Document::new(), 80, 24);
        editor.picker.kind = PickerKind::Search;
        editor.picker.files_loaded = true;
        editor.picker.query.push('n');
        editor.picker.total = 1;
        editor.picker.items.push(PickerItem {
            display: "previous result".into(),
            file_index: 0,
            line: 0,
            column: 0,
            theme_id: None,
            target: None,
            palette_index: None,
            action_index: None,
            symbol_index: None,
            problem: None,
        });

        editor.rebuild_picker();
        assert_eq!(editor.picker.items.len(), 1);
        assert_eq!(editor.picker.items[0].display, "previous result");
        editor.close_picker();
    }

    #[test]
    fn cross_file_search_runs_without_blocking_input() {
        let root =
            std::env::temp_dir().join(format!("hexal-picker-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for index in 0..3 {
            std::fs::write(
                root.join(format!("file{index}.txt")),
                b"first\nneedle here\nlast\n",
            )
            .unwrap();
        }

        let mut editor = Editor::with_options(Document::new(), 80, 24, Some(root.clone()), false);
        editor.open_picker(PickerKind::Search);
        wait_for_picker_scan(&mut editor);

        editor.picker.query.push_str("needle");
        editor.rebuild_picker();
        // The search is dispatched to a worker instead of scanning every file
        // on the input thread.
        assert_eq!(editor.picker.status.as_deref(), Some("Searching..."));

        wait_for_picker_search(&mut editor);
        assert_eq!(editor.picker.total, 3);
        assert_eq!(editor.picker.items.len(), 3);
        assert!(editor
            .picker
            .items
            .iter()
            .all(|item| item.display.contains("needle here")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostic_picker_jumps_to_the_reported_line() {
        let root = std::env::temp_dir().join(format!("hexal-picker-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.rs");
        let second = root.join("second.rs");
        let text = (0..20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&first, &text).unwrap();
        std::fs::write(&second, &text).unwrap();

        let mut editor = Editor::new(
            Document::from_bytes(
                Some(first.to_string_lossy().into_owned()),
                text.as_bytes().to_vec(),
            ),
            80,
            24,
        );
        editor.picker.kind = PickerKind::Problems;
        editor.picker.items = vec![problem_item(first.to_string_lossy().into_owned(), 7, 0)];
        editor.picker.selected = 0;
        editor.open_picker_selection();
        assert_eq!(editor.cursor_line(), 7);

        editor.picker.kind = PickerKind::Problems;
        editor.picker.items = vec![problem_item(second.to_string_lossy().into_owned(), 11, 0)];
        editor.picker.selected = 0;
        editor.open_picker_selection();
        assert_eq!(
            editor.doc.path.as_deref(),
            Some(second.to_string_lossy().as_ref())
        );
        assert_eq!(editor.cursor_line(), 11);

        let _ = std::fs::remove_dir_all(root);
    }
}
