//! Language-server UI: lifecycle, completion, hover, code actions, symbols,
//! diagnostics and debug traffic.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::languages::highlight;

impl Editor {
    /// Stops the current server and clears request state. Language plugins
    /// start the replacement server from their `buffer_opened` handler.
    pub(super) fn restart_lsp_for_buffer(&mut self) {
        self.reset_lsp_interaction();
        self.lsp_fallback_blocked = false;
        self.lsp.stop();
    }

    /// Starts an LSP server requested by a language plugin's `init.lua`.
    /// This mirrors the C# host: when the requested executable is missing and
    /// the language defines a `.hexdef` fallback, the fallback is selected
    /// instead. A configured-but-failing executable blocks the fallback.
    pub(super) fn start_lsp_from_plugin(&mut self, config: lsp::ServerConfig) -> bool {
        let Some(path) = self.doc.path.clone() else {
            return false;
        };
        let language_fallback = if config.language_id.is_empty() {
            None
        } else {
            self.languages
                .find(&config.language_id)
                .and_then(|language| language.fallback_lsp.clone())
        }
        .or_else(|| {
            self.active_language()
                .and_then(|language| language.fallback_lsp.clone())
        });

        let primary_present = crate::util::git::command_exists(&config.command);
        let mut selected = config.clone();
        let mut using_fallback = false;
        if !primary_present && !self.lsp_fallback_blocked {
            if let Some(fallback) = language_fallback {
                if crate::util::git::command_exists(&fallback.command) {
                    selected = lsp::ServerConfig {
                        name: fallback.name,
                        command: fallback.command,
                        arguments: fallback.arguments,
                        language_id: if config.language_id.is_empty() {
                            fallback.language_id
                        } else {
                            config.language_id.clone()
                        },
                        initialization_options: config.initialization_options.clone(),
                    };
                    using_fallback = true;
                }
            }
        }

        self.lsp_dirty = false;
        let text = String::from_utf8_lossy(&self.doc.all_text()).into_owned();
        let root = Path::new(&path)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default();
        match self.lsp.try_start(selected, &path, &text, &root) {
            Ok(()) => {
                self.lsp_synced_revision = self.doc.revision();
                true
            }
            Err(_) => {
                if primary_present || using_fallback {
                    self.lsp_fallback_blocked = true;
                }
                false
            }
        }
    }

    fn reset_lsp_interaction(&mut self) {
        self.lsp_dirty = false;
        self.lsp_synced_revision = u64::MAX;
        self.hover_request_pending = false;
        self.definition_request_pending = false;
        self.code_action_request_pending = false;
        self.pending_code_action_resolve = false;
        self.symbol_request_pending = false;
        self.completion.close();
        self.code_actions.clear();
        self.document_symbols.clear();
        self.unnecessary_diagnostics.clear();
    }

    pub(super) fn poll_background(&mut self) {
        if self.startup_work_pending {
            self.startup_work_pending = false;
            self.auto_detect_indentation_on_open();
        }
        self.poll_sidebar_format();
        self.poll_picker_work();
        self.poll_git_diff();
        self.sidebar.poll_git_status();
        self.lsp.poll(std::time::Duration::ZERO);
        let events = self.lsp.take_events();
        for event in events {
            match event {
                lsp::Event::Started | lsp::Event::Initialized => {}
                lsp::Event::Diagnostics { .. } => {
                    if self.picker.kind == PickerKind::Problems {
                        self.rebuild_picker();
                    }
                    if self
                        .plugins
                        .as_ref()
                        .map(|plugins| plugins.has_subscribers("__lsp_diagnostics"))
                        .unwrap_or(false)
                    {
                        let path = self.doc.path.clone().unwrap_or_default();
                        let diagnostics = self.lsp.diagnostics_for_path(&path);
                        let mut data = crate::util::json::Value::Object(Vec::new());
                        data.set(
                            "server",
                            crate::util::json::Value::String(self.lsp.server_name()),
                        );
                        data.set(
                            "uri",
                            crate::util::json::Value::String(
                                lsp::manager::path_to_uri(&path, false).unwrap_or_default(),
                            ),
                        );
                        let mut items = crate::util::json::Value::Array(Vec::new());
                        for diagnostic in diagnostics {
                            let mut item = crate::util::json::Value::Object(Vec::new());
                            item.set(
                                "line",
                                crate::util::json::Value::Number(diagnostic.line as f64),
                            );
                            item.set(
                                "column",
                                crate::util::json::Value::Number(diagnostic.column as f64),
                            );
                            item.set(
                                "severity",
                                crate::util::json::Value::Number(diagnostic.severity as f64),
                            );
                            item.set(
                                "message",
                                crate::util::json::Value::String(diagnostic.message),
                            );
                            item.set(
                                "source",
                                crate::util::json::Value::String(diagnostic.source),
                            );
                            let tags = diagnostic
                                .tags
                                .into_iter()
                                .map(|tag| crate::util::json::Value::Number(tag as f64))
                                .collect();
                            item.set("tags", crate::util::json::Value::Array(tags));
                            if let crate::util::json::Value::Array(values) = &mut items {
                                values.push(item);
                            }
                        }
                        data.set("diagnostics", items);
                        self.fire_plugin_event("__lsp_diagnostics", data);
                    }
                }
                lsp::Event::Hover { text } => {
                    if !self.hover_request_pending {
                        continue;
                    }
                    self.hover_request_pending = false;
                    if text.is_empty() {
                        self.set_message("No hover information available.");
                    } else {
                        self.show_hover_popup(&text, "Hover");
                    }
                }
                lsp::Event::Definition { locations } => {
                    if !self.definition_request_pending {
                        continue;
                    }
                    self.definition_request_pending = false;
                    if locations.is_empty() {
                        self.set_message("No definition found.");
                    } else {
                        self.navigate_to_location(&locations[0]);
                    }
                }
                lsp::Event::CodeActions { actions } => {
                    if !self.code_action_request_pending {
                        continue;
                    }
                    self.code_action_request_pending = false;
                    if actions.is_empty() {
                        self.set_message("No code actions available.");
                    } else {
                        self.code_actions = actions;
                        self.pending_code_action_resolve = false;
                        self.open_picker(PickerKind::CodeActions);
                    }
                }
                lsp::Event::CodeActionResolved { actions } => {
                    if !self.pending_code_action_resolve {
                        continue;
                    }
                    self.pending_code_action_resolve = false;
                    if let Some(action) = actions.first() {
                        let action = action.clone();
                        self.apply_code_action(&action);
                    } else {
                        self.set_message(
                            "The language server returned no changes for this code action.",
                        );
                    }
                }
                lsp::Event::WorkspaceDiagnostics => {
                    if let Some(path) = self.doc.path.clone() {
                        let diagnostics = self.lsp.diagnostics_for_path(&path);
                        self.update_diagnostic_decorations(&path, &diagnostics);
                    }
                    if self.picker.kind == PickerKind::Problems && self.problems_workspace {
                        self.rebuild_picker();
                    }
                }
                lsp::Event::DocumentSymbols { symbols } => {
                    if !self.symbol_request_pending {
                        continue;
                    }
                    self.symbol_request_pending = false;
                    self.open_document_symbol_picker(symbols);
                }
                lsp::Event::Completions { request_id, items } => {
                    if !self.completion.pending || request_id != self.completion.request_id {
                        continue;
                    }
                    self.open_completion(items);
                }
                lsp::Event::CommandResponse {
                    command,
                    result,
                    error,
                } => {
                    self.handle_lsp_debug_command(&command, result, &error);
                }
                lsp::Event::Error { server, message } => {
                    self.pending_code_action_resolve = false;
                    self.set_error(&format!("LSP {server}: {message}"));
                }
                lsp::Event::Stopped => {
                    self.lsp_dirty = false;
                    self.lsp_synced_revision = u64::MAX;
                }
            }
        }

        if self.lsp_dirty
            && self.doc.path.is_some()
            && self.lsp.is_initialized()
            && self.lsp_dirty_since.elapsed() >= std::time::Duration::from_millis(150)
        {
            self.sync_lsp_document_now();
        }

        let debug_deadline = self
            .pending_debug
            .as_ref()
            .map(|pending| (pending.started, pending.timeout));
        if let Some((started, timeout)) = debug_deadline {
            if !self.lsp.is_running() {
                self.pending_debug = None;
                self.set_error("debug: the language server stopped while starting the debugger");
            } else if started.elapsed() >= timeout {
                self.pending_debug = None;
                self.set_error("debug: timed out waiting for the language server");
            }
        }

        self.poll_dap();

        if self.lsp_debug.visible {
            self.refresh_lsp_debug_lines();
        }
    }

    fn sync_lsp_document_now(&mut self) {
        if !self.lsp.is_running() || !self.lsp.is_initialized() {
            return;
        }
        if !self.lsp_dirty && self.doc.revision() == self.lsp_synced_revision {
            return;
        }
        let text = String::from_utf8_lossy(&self.doc.all_text()).into_owned();
        self.lsp.did_change(&text);
        self.lsp_dirty = false;
        self.lsp_synced_revision = self.doc.revision();
    }

    pub(super) fn ensure_lsp_ready(&mut self, feature: &str) -> bool {
        if !self.lsp.is_running() {
            self.set_error("No LSP configured");
            return false;
        }
        if !self.lsp.is_initialized() {
            self.set_error(&format!(
                "LSP is still starting; {feature} is not available yet"
            ));
            return false;
        }
        if self.doc.path.is_none() {
            self.set_error(&format!("{feature} requires a file-backed buffer"));
            return false;
        }
        true
    }

    pub(super) fn notify_lsp_saved(&mut self) {
        if self.lsp.is_initialized() {
            let text = String::from_utf8_lossy(&self.doc.all_text()).into_owned();
            self.lsp.did_save(&text);
        }
    }

    fn update_diagnostic_decorations(&mut self, path: &str, diagnostics: &[lsp::Diagnostic]) {
        let Some(current) = self.doc.path.as_deref() else {
            return;
        };
        if !git::same_path(Path::new(current), Path::new(path)) {
            return;
        }
        self.unnecessary_diagnostics.clear();
        let last_line = self.doc.line_count().saturating_sub(1);
        for diagnostic in diagnostics {
            if diagnostic.line < 0 || diagnostic.line as usize > last_line {
                continue;
            }
            if !diagnostic.tags.contains(&1) {
                continue;
            }
            let mut end_line = if diagnostic.end_line >= 0 {
                diagnostic.end_line
            } else {
                diagnostic.line
            };
            let mut end_column = if diagnostic.end_column >= 0 {
                diagnostic.end_column
            } else {
                diagnostic.column + 1
            };
            if end_line < diagnostic.line {
                end_line = diagnostic.line;
                end_column = diagnostic.column + 1;
            }
            end_line = end_line.min(last_line as i64);
            for line in diagnostic.line..=end_line {
                let start = if line == diagnostic.line {
                    diagnostic.column.max(0)
                } else {
                    0
                };
                let end = if line == end_line {
                    end_column.max(start)
                } else {
                    i64::MAX
                };
                if end > start {
                    self.unnecessary_diagnostics
                        .entry(line as usize)
                        .or_default()
                        .push((start, end));
                }
            }
        }
    }

    fn cursor_lsp_position(&self) -> (i64, i64) {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        (line as i64, lsp::byte_offset_to_utf16_column(&text, local))
    }

    // ---------------------------------------------------------------------
    // Hover and definition
    // ---------------------------------------------------------------------

    pub(super) fn show_hover(&mut self) {
        if !self.ensure_lsp_ready("hover") {
            return;
        }
        self.sync_lsp_document_now();
        let (line, character) = self.cursor_lsp_position();
        let path = self.doc.path.clone().unwrap_or_default();
        if !self.lsp.request_hover(&path, line, character) {
            self.set_error("No LSP configured");
            return;
        }
        self.hover_request_pending = true;
        self.set_message("Requesting hover information...");
    }

    pub(super) fn go_to_definition(&mut self) {
        if !self.ensure_lsp_ready("go to definition") {
            return;
        }
        self.sync_lsp_document_now();
        let (line, character) = self.cursor_lsp_position();
        let path = self.doc.path.clone().unwrap_or_default();
        if !self.lsp.request_definition(&path, line, character) {
            self.set_error("No LSP configured");
            return;
        }
        self.definition_request_pending = true;
        self.set_message("Requesting definition...");
    }

    fn navigate_to_location(&mut self, location: &lsp::Location) {
        let path = lsp::uri_to_path(&location.uri);
        if path.is_empty() {
            self.set_error("The language server returned an invalid definition location.");
            return;
        }
        let same = self
            .doc
            .path
            .as_deref()
            .is_some_and(|current| git::same_path(Path::new(current), Path::new(&path)));
        self.record_jump();
        if same {
            self.move_to_line_character(location.line, location.character);
        } else {
            let byte = lsp_column_to_byte_offset(&path, location.line, location.character);
            self.request_open(&path, location.line.max(0) as usize, byte, false);
        }
    }

    pub(super) fn move_to_line_character(&mut self, line: i64, character: i64) {
        let line = line.max(0) as usize;
        if line >= self.doc.line_count() {
            return;
        }
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = lsp::utf16_column_to_byte_offset(&text, character);
        self.cursor = (start + local).min(self.doc.size());
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.ensure_cursor_visible();
        self.clear_message();
    }

    // ---------------------------------------------------------------------
    // Completion
    // ---------------------------------------------------------------------

    pub(super) fn request_lsp_completion(&mut self) {
        if !self.ensure_lsp_ready("completion") {
            return;
        }
        self.sync_lsp_document_now();
        if self.try_request_lsp_completion(None) {
            self.set_message("Requesting completions...");
        }
    }

    pub(super) fn try_request_lsp_completion(&mut self, trigger: Option<char>) -> bool {
        if !self.lsp.is_running() || !self.lsp.is_initialized() || self.doc.path.is_none() {
            return false;
        }
        self.sync_lsp_document_now();
        let (line, character) = self.cursor_lsp_position();
        let path = self.doc.path.clone().unwrap_or_default();
        let Some(request_id) = self.lsp.request_completion(&path, line, character, trigger) else {
            return false;
        };
        self.completion.request_id = request_id;
        self.completion.pending = true;
        true
    }

    fn open_completion(&mut self, items: Vec<lsp::CompletionItem>) {
        self.completion.pending = false;
        if items.is_empty() {
            self.completion.close();
            self.set_message("No completions available.");
            return;
        }
        self.completion.all_items = items;
        self.completion.filter.clear();
        self.apply_completion_filter();
        self.completion.visible = true;
        self.clear_message();
    }

    fn apply_completion_filter(&mut self) {
        self.completion.items.clear();
        if self.completion.filter.is_empty() {
            self.completion
                .items
                .extend(self.completion.all_items.iter().cloned());
        } else {
            for item in &self.completion.all_items {
                let candidate = if item.filter_text.is_empty() {
                    &item.label
                } else {
                    &item.filter_text
                };
                if candidate
                    .to_ascii_lowercase()
                    .starts_with(&self.completion.filter.to_ascii_lowercase())
                {
                    self.completion.items.push(item.clone());
                }
            }
        }
        self.completion.selected = 0;
        self.completion.top = 0;
    }

    fn completion_rows(&self) -> usize {
        self.completion.items.len().clamp(1, 10)
    }

    pub(super) fn handle_completion_input(&mut self, event: &Event) -> bool {
        match event.key {
            Key::Escape => {
                self.completion.close();
                true
            }
            Key::Up | Key::ScrollUp => {
                self.move_completion_selection(-1);
                true
            }
            Key::Down | Key::ScrollDown => {
                self.move_completion_selection(1);
                true
            }
            Key::PageUp => {
                self.move_completion_selection(-(self.completion_rows() as isize));
                true
            }
            Key::PageDown => {
                self.move_completion_selection(self.completion_rows() as isize);
                true
            }
            Key::Home => {
                self.completion.selected = 0;
                true
            }
            Key::End => {
                self.completion.selected = self.completion.items.len().saturating_sub(1);
                true
            }
            Key::Enter | Key::Tab => {
                if self.completion.items.is_empty() {
                    self.completion.close();
                    return false;
                }
                self.accept_completion();
                true
            }
            Key::Backspace => {
                if !self.completion.filter.is_empty() {
                    self.completion.filter.pop();
                    self.apply_completion_filter();
                    return false;
                }
                self.completion.close();
                false
            }
            Key::Char if !event.ctrl && !event.alt && is_completion_filter_character(event.ch) => {
                self.completion.filter.push(event.ch);
                self.apply_completion_filter();
                false
            }
            _ => {
                self.completion.close();
                false
            }
        }
    }

    fn move_completion_selection(&mut self, delta: isize) {
        let count = self.completion.items.len();
        if count == 0 {
            return;
        }
        let next = self.completion.selected as isize + delta;
        self.completion.selected = next.rem_euclid(count as isize) as usize;
        let rows = self.completion_rows();
        if self.completion.selected < self.completion.top {
            self.completion.top = self.completion.selected;
        } else if self.completion.selected >= self.completion.top + rows {
            self.completion.top = self.completion.selected + 1 - rows;
        }
    }

    fn accept_completion(&mut self) {
        if self.completion.selected >= self.completion.items.len() {
            return;
        }
        let item = self.completion.items[self.completion.selected].clone();
        let mut start = self.cursor;
        let mut end = self.cursor;
        if item.has_text_edit && self.doc.path.is_some() {
            start = self.position_to_offset(item.start_line, item.start_character);
            end = self.position_to_offset(item.end_line, item.end_character);
            if end < start {
                std::mem::swap(&mut start, &mut end);
            }
            start = start.min(self.doc.size());
            end = end.clamp(start, self.doc.size());
            if !self.completion.filter.is_empty() && self.cursor >= end {
                end = self.cursor;
            }
        }
        let (mut text, cursor_units) = if item.insert_text_format == 2 {
            expand_snippet(&item.insert_text)
        } else {
            (
                item.insert_text.clone(),
                item.insert_text.encode_utf16().count(),
            )
        };
        if text.is_empty() {
            text = item.label.clone();
        }
        self.doc.break_undo_group();
        self.doc.replace(start, end - start, text.as_bytes());
        let cursor_bytes = byte_length_for_utf16_units(&text, cursor_units);
        self.cursor = start + cursor_bytes;
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.completion.close();
        self.snap_cursor();
        self.preferred_column = None;
        self.lsp_dirty = self.lsp.is_running();
        self.lsp_dirty_since = std::time::Instant::now();
        self.clear_message();
    }

    fn position_to_offset(&self, line: i64, character: i64) -> usize {
        let line = line.max(0) as usize;
        if line >= self.doc.line_count() {
            return self.doc.size();
        }
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        start + lsp::utf16_column_to_byte_offset(&text, character)
    }

    // ---------------------------------------------------------------------
    // Code actions and symbols
    // ---------------------------------------------------------------------

    pub(super) fn show_code_actions(&mut self) {
        if !self.ensure_lsp_ready("code actions") {
            return;
        }
        self.sync_lsp_document_now();
        let (line, character) = self.cursor_lsp_position();
        let path = self.doc.path.clone().unwrap_or_default();
        let diagnostics = self.lsp.diagnostics_for_path(&path);
        self.pending_code_action_resolve = false;
        if !self
            .lsp
            .request_code_actions(&path, line, character, &diagnostics)
        {
            self.set_error("No LSP configured");
            return;
        }
        self.code_action_request_pending = true;
        self.set_message("Requesting code actions...");
    }

    pub(super) fn apply_code_action(&mut self, action: &lsp::CodeAction) {
        self.pending_code_action_resolve = false;
        if action.needs_resolve
            || (action.edits.is_empty() && action.command.is_empty() && action.raw.is_some())
        {
            if self.lsp.resolve_code_action(action) {
                self.pending_code_action_resolve = true;
                self.set_message(&format!("Resolving code action: {}", action.title));
                return;
            }
        }

        let mut changed_current = false;
        let mut changed_files = 0usize;
        let mut failures = Vec::new();
        let mut edits_by_uri: Vec<(String, Vec<lsp::TextEdit>)> = Vec::new();
        for edit in &action.edits {
            if let Some((_, group)) = edits_by_uri.iter_mut().find(|(uri, _)| *uri == edit.uri) {
                group.push(edit.clone());
            } else {
                edits_by_uri.push((edit.uri.clone(), vec![edit.clone()]));
            }
        }

        for (uri, edits) in edits_by_uri {
            let path = lsp::uri_to_path(&uri);
            let is_current = self
                .doc
                .path
                .as_deref()
                .is_some_and(|current| git::same_path(Path::new(current), Path::new(&path)));
            if is_current {
                if let Err(error) = apply_text_edits(&mut self.doc, &edits) {
                    failures.push(error);
                } else {
                    changed_current = true;
                    changed_files += 1;
                }
                continue;
            }

            match Document::open(&path) {
                Ok(mut document) => {
                    if let Err(error) = apply_text_edits(&mut document, &edits) {
                        failures.push(error);
                        continue;
                    }
                    if let Err(error) = document.save(None, self.config.fsync_on_save) {
                        failures.push(error.to_string());
                        continue;
                    }
                    changed_files += 1;
                }
                Err(error) => failures.push(error.to_string()),
            }
        }

        if changed_current {
            self.cursor = self.cursor.min(self.doc.size());
            self.preferred_column = None;
            self.doc.break_undo_group();
            self.snap_cursor();
            self.lsp_dirty = self.lsp.is_running();
            self.lsp_dirty_since = std::time::Instant::now();
        }
        if !action.command.is_empty() {
            self.lsp
                .execute_command(&action.command, action.command_arguments.as_ref());
        }

        if let Some(error) = failures.first() {
            self.set_error(&format!("code action failed: {error}"));
        } else if changed_files > 0 {
            self.set_message(&format!(
                "Applied code action: {} ({} file{})",
                action.title,
                changed_files,
                if changed_files == 1 { "" } else { "s" }
            ));
        } else if !action.command.is_empty() {
            self.set_message(&format!("Ran code action: {}", action.title));
        } else {
            self.set_message(&format!("Code action '{}' made no changes.", action.title));
        }
    }

    pub(super) fn show_document_symbols(&mut self) {
        if !self.ensure_lsp_ready("document symbols") {
            return;
        }
        self.sync_lsp_document_now();
        let path = self.doc.path.clone().unwrap_or_default();
        if !self.lsp.request_document_symbols(&path) {
            self.set_error("No LSP configured");
            return;
        }
        self.symbol_request_pending = true;
        self.set_message("Requesting document symbols...");
    }

    fn open_document_symbol_picker(&mut self, symbols: Vec<lsp::DocumentSymbol>) {
        if symbols.is_empty() {
            self.set_message("No symbols found.");
            return;
        }
        self.document_symbols = symbols;
        self.open_picker(PickerKind::Symbols);
    }

    // ---------------------------------------------------------------------
    // Diagnostics
    // ---------------------------------------------------------------------

    pub(super) fn show_problems(&mut self, workspace: bool) {
        self.problems_workspace = workspace;
        if !self.lsp.is_running() {
            self.set_error("No LSP configured");
            return;
        }
        if !workspace && self.doc.path.is_none() {
            self.set_error("Document diagnostics require a file-backed buffer");
            return;
        }
        self.lsp.request_workspace_diagnostics();
        self.open_picker(PickerKind::Problems);
    }

    pub(super) fn show_line_diagnostic(&mut self) {
        let Some(path) = self.doc.path.clone() else {
            self.set_error("Diagnostics require a file-backed buffer");
            return;
        };
        if !self.lsp.is_running() {
            self.set_error("No LSP configured");
            return;
        }
        let line = self.cursor_line() as i64;
        let mut diagnostics: Vec<lsp::Diagnostic> = self
            .lsp
            .diagnostics_for_path(&path)
            .into_iter()
            .filter(|diagnostic| {
                let end_line = if diagnostic.end_line >= 0 {
                    diagnostic.end_line
                } else {
                    diagnostic.line
                };
                line >= diagnostic.line && line <= end_line
            })
            .collect();
        diagnostics.sort_by(|left, right| {
            diagnostic_rank(left.severity)
                .cmp(&diagnostic_rank(right.severity))
                .then(left.column.cmp(&right.column))
        });
        if diagnostics.is_empty() {
            self.set_message("No diagnostics on this line.");
            return;
        }

        let mut text = String::new();
        for diagnostic in &diagnostics {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if diagnostics.len() > 1 {
                text.push_str(diagnostic_severity_name(diagnostic.severity));
                text.push_str(&format!(
                    " ({}:{})",
                    diagnostic.line + 1,
                    diagnostic.column + 1
                ));
                if !diagnostic.source.is_empty() {
                    text.push_str(&format!(" [{}]", diagnostic.source));
                }
                text.push('\n');
            }
            text.push_str(if diagnostic.message.is_empty() {
                "(no diagnostic message)"
            } else {
                &diagnostic.message
            });
        }
        let title = if diagnostics.len() == 1 {
            diagnostic_severity_name(diagnostics[0].severity).to_string()
        } else {
            format!("Diagnostics ({})", diagnostics.len())
        };
        self.show_hover_popup(&text, &title);
    }

    pub(super) fn populate_problem_items(&mut self) {
        self.picker.items.clear();
        let query = self.picker.query.clone();
        let root = self.sidebar.root.clone();
        let current = self.doc.path.clone().map(PathBuf::from);
        let mut problems = Vec::new();
        for report in self.lsp.snapshot_diagnostics() {
            let path = lsp::uri_to_path(&report.uri);
            let path = PathBuf::from(path);
            if !self.problems_workspace {
                let same = current
                    .as_ref()
                    .is_some_and(|current| git::same_path(current, &path));
                if !same {
                    continue;
                }
            } else if !path.starts_with(&root) {
                continue;
            }
            for diagnostic in report.diagnostics {
                problems.push((path.clone(), diagnostic));
            }
        }
        problems.sort_by(|left, right| {
            left.0
                .to_string_lossy()
                .to_ascii_lowercase()
                .cmp(&right.0.to_string_lossy().to_ascii_lowercase())
                .then(left.1.line.cmp(&right.1.line))
        });

        for (path, diagnostic) in problems {
            let relative = path
                .strip_prefix(&root)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            let severity = match diagnostic.severity {
                1 => "E",
                2 => "W",
                3 => "I",
                _ => "H",
            };
            let display = format!(
                "{severity} {relative}:{}:{}  {}",
                diagnostic.line + 1,
                diagnostic.column + 1,
                diagnostic.message
            );
            if !query.is_empty()
                && !display
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
            {
                continue;
            }
            self.picker.total += 1;
            self.picker.items.push(PickerItem {
                display,
                file_index: 0,
                line: 0,
                column: 0,
                theme_id: None,
                target: None,
                palette_index: None,
                action_index: None,
                symbol_index: None,
                problem: Some((
                    path.to_string_lossy().into_owned(),
                    diagnostic.line.max(0) as usize,
                    diagnostic.column.max(0) as usize,
                )),
            });
        }
    }

    // ---------------------------------------------------------------------
    // Hover popup rendering and input
    // ---------------------------------------------------------------------

    pub(super) fn show_hover_popup(&mut self, text: &str, title: &str) {
        self.hover.lines.clear();
        let width = (self.cols.saturating_sub(6)).clamp(20, 88);
        let content_width = width.saturating_sub(4).max(1);
        let mut code_info: Option<String> = None;
        let mut fence_char = '\0';
        let mut fence_length = 0usize;
        let mut in_code = false;
        let mut code_start_in_block = false;

        for raw_line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
            if let Some((character, length, info)) = try_hover_fence(raw_line) {
                if in_code {
                    if character == fence_char && length >= fence_length && info.is_empty() {
                        in_code = false;
                        continue;
                    }
                } else {
                    in_code = true;
                    fence_char = character;
                    fence_length = length;
                    code_info = Some(info);
                    code_start_in_block = false;
                    continue;
                }
            }

            if in_code {
                let language = resolve_hover_language(&self.languages, code_info.as_deref());
                add_hover_code_line(
                    raw_line,
                    language,
                    content_width,
                    &mut self.hover.lines,
                    &mut code_start_in_block,
                    self.doc.tab_width().max(1),
                );
                continue;
            }

            if raw_line.is_empty() {
                self.hover.lines.push(HoverLine {
                    segments: Vec::new(),
                });
                continue;
            }
            wrap_hover_line(raw_line, content_width, &mut self.hover.lines);
        }

        if self.hover.lines.is_empty() {
            self.set_message("No hover information available.");
            return;
        }
        self.hover.title = title.to_string();
        self.hover.top = 0;
        self.hover.visible = true;
        self.clear_message();
    }

    pub(super) fn handle_hover_input(&mut self, event: Event) -> bool {
        match event.key {
            Key::Escape | Key::Enter => {
                self.hover.close();
                true
            }
            Key::Up | Key::ScrollUp => {
                self.hover.top = self.hover.top.saturating_sub(1);
                true
            }
            Key::Down | Key::ScrollDown => {
                let rows = self.hover_content_rows();
                if self.hover.top + rows < self.hover.lines.len() {
                    self.hover.top += 1;
                }
                true
            }
            Key::PageUp => {
                self.hover.top = self
                    .hover
                    .top
                    .saturating_sub(self.hover_content_rows().max(1));
                true
            }
            Key::PageDown => {
                let rows = self.hover_content_rows().max(1);
                let max_top = self.hover.lines.len().saturating_sub(rows);
                self.hover.top = (self.hover.top + rows).min(max_top);
                true
            }
            Key::Home => {
                self.hover.top = 0;
                true
            }
            Key::End => {
                self.hover.top = self
                    .hover
                    .lines
                    .len()
                    .saturating_sub(self.hover_content_rows().max(1));
                true
            }
            Key::Char if !event.ctrl && !event.alt && matches!(event.ch, 'q' | 'Q') => {
                self.hover.close();
                true
            }
            _ => false,
        }
    }

    fn hover_content_rows(&self) -> usize {
        let height = self.hover_height();
        height.saturating_sub(2).max(1)
    }

    fn hover_height(&self) -> usize {
        let available = self.status_row();
        self.hover
            .lines
            .len()
            .saturating_add(2)
            .min(available.saturating_sub(1))
    }

    pub(super) fn render_hover(&mut self) {
        if !self.hover.visible || self.hover.lines.is_empty() {
            return;
        }
        let available = self.status_row();
        let width = (self.cols.saturating_sub(6)).clamp(20, 88);
        let height = self.hover_height();
        if available < height + 1 || width < 20 {
            return;
        }
        let x = (self.cols.saturating_sub(width)) / 2;
        let y = (available.saturating_sub(height)) / 2;
        let title = self.hover.title.clone();
        self.draw_box(x, y, width, height, &title);
        let rows = height.saturating_sub(2);
        let max_top = self.hover.lines.len().saturating_sub(rows);
        self.hover.top = self.hover.top.min(max_top);
        for row in 0..rows {
            let index = self.hover.top + row;
            if index >= self.hover.lines.len() {
                break;
            }
            let line = &self.hover.lines[index];
            let mut column = 0usize;
            for segment in &line.segments {
                if column >= width.saturating_sub(4) {
                    break;
                }
                let style = self.theme.syntax_style(segment.kind);
                self.screen.put_str(
                    (x + 2 + column) as i64,
                    (y + 1 + row) as i64,
                    &segment.text,
                    style,
                    width.saturating_sub(4 + column),
                );
                column += util::text_width(&segment.text);
            }
        }
    }

    pub(super) fn render_completion(&mut self) {
        if !self.completion.visible || self.completion.items.is_empty() {
            return;
        }
        let rows = self.completion_rows();
        self.completion.selected = self
            .completion
            .selected
            .min(self.completion.items.len() - 1);
        let mut width = 24usize;
        for item in self
            .completion
            .items
            .iter()
            .skip(self.completion.top)
            .take(rows)
        {
            width = width.max(completion_display(item).len() + 4);
        }
        width = width.min(self.cols.saturating_sub(4).max(10));
        let height = rows + 2;
        let available = self.status_row();
        if available < height + 1 || width < 10 {
            return;
        }
        let cursor_line = self.cursor_line();
        let cursor_row = cursor_line.saturating_sub(self.top_line);
        let cursor_column = self.cursor_visual_column();
        let gutter = self.editor_left() + self.gutter_width();
        let cursor_x = gutter + cursor_column.saturating_sub(self.left_column);

        let mut y = cursor_row + 1;
        if y + height > available {
            y = cursor_row.saturating_sub(height);
        }
        y = y.min(available.saturating_sub(height));
        let x = (cursor_x + 1).min(self.cols.saturating_sub(width + 1));
        self.draw_box(
            x,
            y,
            width,
            height,
            &format!(
                " Completions {}/{} ",
                self.completion.selected + 1,
                self.completion.items.len()
            ),
        );
        for row in 0..rows {
            let index = self.completion.top + row;
            if index >= self.completion.items.len() {
                break;
            }
            let style = if index == self.completion.selected {
                self.theme.selected_item_style()
            } else {
                self.theme.text()
            };
            self.screen.fill(
                (x + 1) as i64,
                (y + 1 + row) as i64,
                width - 2,
                1,
                ' ',
                style,
            );
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1 + row) as i64,
                &completion_display(&self.completion.items[index]),
                style,
                width.saturating_sub(4),
            );
        }
    }

    // ---------------------------------------------------------------------
    // LSP debug modal
    // ---------------------------------------------------------------------

    pub(super) fn open_lsp_debug(&mut self) {
        self.lsp_debug.visible = true;
        self.lsp_debug.top = 0;
        self.lsp_debug.status.clear();
        self.refresh_lsp_debug_lines();
        self.clear_message();
        self.force_full_redraw = true;
    }

    fn refresh_lsp_debug_lines(&mut self) {
        let width = self.cols.saturating_sub(4).max(10);
        let report = self.lsp.build_debug_report();
        self.lsp_debug_lines.clear();
        for raw_line in report.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
            if raw_line.is_empty() {
                self.lsp_debug_lines.push(String::new());
                continue;
            }
            let wrapped = wrap_text(raw_line, width);
            if wrapped.is_empty() {
                self.lsp_debug_lines.push(String::new());
            } else {
                self.lsp_debug_lines.extend(wrapped);
            }
        }
    }

    fn lsp_debug_rows(&self) -> usize {
        self.rows.saturating_sub(3).max(1)
    }

    pub(super) fn handle_lsp_debug_input(&mut self, event: Event) {
        let rows = self.lsp_debug_rows();
        let max_top = self.lsp_debug_lines.len().saturating_sub(rows);
        match event.key {
            Key::Escape | Key::Enter => {
                self.lsp_debug.visible = false;
                self.force_full_redraw = true;
            }
            Key::Up | Key::ScrollUp => {
                self.lsp_debug.top = self.lsp_debug.top.saturating_sub(1);
            }
            Key::Down | Key::ScrollDown => {
                self.lsp_debug.top = (self.lsp_debug.top + 1).min(max_top);
            }
            Key::PageUp => {
                self.lsp_debug.top = self.lsp_debug.top.saturating_sub(rows);
            }
            Key::PageDown => {
                self.lsp_debug.top = (self.lsp_debug.top + rows).min(max_top);
            }
            Key::Home => self.lsp_debug.top = 0,
            Key::End => self.lsp_debug.top = max_top,
            Key::Char if !event.ctrl && !event.alt => match event.ch {
                'q' | 'Q' => {
                    self.lsp_debug.visible = false;
                    self.force_full_redraw = true;
                }
                'j' | 'J' => self.lsp_debug.top = (self.lsp_debug.top + 1).min(max_top),
                'k' | 'K' => self.lsp_debug.top = self.lsp_debug.top.saturating_sub(1),
                'g' | 'G' => {
                    self.refresh_lsp_debug_lines();
                    let max_top = self.lsp_debug_lines.len().saturating_sub(rows);
                    self.lsp_debug.top = if event.ch == 'g' { 0 } else { max_top };
                }
                'y' | 'Y' => {
                    let report = self.lsp.build_debug_report();
                    if self.store_clipboard(report.as_bytes()) {
                        self.lsp_debug.status =
                            format!("Copied {} characters to the clipboard.", report.len());
                    } else {
                        self.lsp_debug.status = "Copy failed.".to_string();
                    }
                }
                'c' | 'C' => {
                    self.lsp.clear_log();
                    self.lsp_debug.status = "Log cleared.".into();
                    self.refresh_lsp_debug_lines();
                    self.lsp_debug.top = 0;
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub(super) fn render_lsp_debug(&mut self) {
        if !self.lsp_debug.visible {
            return;
        }
        let width = self.cols;
        let height = self.rows;
        if width < 4 || height < 3 {
            return;
        }
        self.screen.clear(self.theme.base());
        self.draw_box(0, 0, width, height, " LSP Debug ");
        let rows = self.lsp_debug_rows();
        let max_top = self.lsp_debug_lines.len().saturating_sub(rows);
        self.lsp_debug.top = self.lsp_debug.top.min(max_top);
        let content_width = width.saturating_sub(4);
        for row in 0..rows {
            let index = self.lsp_debug.top + row;
            if index >= self.lsp_debug_lines.len() {
                break;
            }
            self.screen.put_str(
                2,
                (1 + row) as i64,
                &self.lsp_debug_lines[index],
                self.theme.text(),
                content_width,
            );
        }
        let hint = "Esc close  |  j/k/arrows scroll  |  Home/End  |  y copy  |  c clear";
        let footer = if self.lsp_debug.status.is_empty() {
            hint.to_string()
        } else {
            format!("{}  |  {hint}", self.lsp_debug.status)
        };
        self.screen.put_str(
            2,
            (height - 2) as i64,
            &footer,
            self.theme.muted_style(),
            content_width,
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
fn resolve_hover_language<'a>(languages: &'a Registry, info: Option<&str>) -> Option<&'a Language> {
    let info = info?.trim();
    if info.is_empty() {
        return None;
    }
    let mut id = info;
    if id.len() >= 2 && id.starts_with('{') && id.ends_with('}') {
        id = id[1..id.len() - 1].trim();
    }
    let id = id
        .split(|character: char| character.is_whitespace() || character == ',')
        .next()
        .unwrap_or("")
        .trim_matches('.')
        .to_ascii_lowercase();
    let id = match id.as_str() {
        "cs" | "c#" => "csharp".to_string(),
        "c++" | "cxx" | "cc" | "hpp" => "cpp".to_string(),
        "py" => "python".to_string(),
        "golang" => "go".to_string(),
        "pyx" => "cython".to_string(),
        other => other.to_string(),
    };
    languages.find(&id)
}

fn diagnostic_rank(severity: i64) -> i64 {
    if (1..=4).contains(&severity) {
        severity
    } else {
        5
    }
}

fn diagnostic_severity_name(severity: i64) -> &'static str {
    match severity {
        1 => "Error",
        2 => "Warning",
        3 => "Information",
        4 => "Hint",
        _ => "Diagnostic",
    }
}

fn is_completion_filter_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

fn completion_display(item: &lsp::CompletionItem) -> String {
    if item.detail.is_empty() {
        item.label.clone()
    } else {
        format!("{}  {}", item.label, item.detail)
    }
}

pub(super) fn lsp_column_to_byte_offset(path: &str, line: i64, character: i64) -> usize {
    if line < 0 {
        return 0;
    }
    let Ok(data) = std::fs::read(path) else {
        return character.max(0) as usize;
    };
    let mut current = 0i64;
    let mut start = 0usize;
    let mut end = data.len();
    for (index, byte) in data.iter().enumerate() {
        if *byte == b'\n' {
            if current == line {
                end = if index > start && data[index - 1] == b'\r' {
                    index - 1
                } else {
                    index
                };
                break;
            }
            current += 1;
            start = index + 1;
        }
    }
    if current != line {
        return character.max(0) as usize;
    }
    lsp::utf16_column_to_byte_offset(&data[start..end], character)
}

fn apply_text_edits(document: &mut Document, edits: &[lsp::TextEdit]) -> Result<(), String> {
    let mut resolved = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.start_line < 0
            || edit.end_line < 0
            || edit.start_line as usize >= document.line_count()
            || edit.end_line as usize >= document.line_count()
        {
            return Err("code action references a line outside the document".into());
        }
        let start =
            line_position_to_offset(document, edit.start_line as usize, edit.start_character);
        let end = line_position_to_offset(document, edit.end_line as usize, edit.end_character);
        if end < start {
            return Err("code action has an inverted edit range".into());
        }
        resolved.push((start, end, edit.new_text.as_bytes().to_vec()));
    }
    resolved.sort_by(|left, right| right.0.cmp(&left.0));
    let mut last_start = usize::MAX;
    for (start, end, _) in &resolved {
        if *end > last_start {
            return Err("code action contains overlapping edits".into());
        }
        last_start = *start;
    }
    document.break_undo_group();
    for (start, end, text) in resolved {
        document.replace(start, end - start, &text);
    }
    document.break_undo_group();
    Ok(())
}

fn line_position_to_offset(document: &Document, line: usize, character: i64) -> usize {
    let line = line.min(document.line_count().saturating_sub(1));
    let start = document.line_start(line);
    let end = document.line_end(line);
    let text = document.line_text(line);
    let column = lsp::utf16_column_to_byte_offset(&text, character);
    (start + column).clamp(start, end)
}

fn expand_snippet(snippet: &str) -> (String, usize) {
    let mut result = String::with_capacity(snippet.len());
    let mut cursor_offset: Option<usize> = None;
    let mut final_offset: Option<usize> = None;
    let chars: Vec<char> = snippet.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let character = chars[index];
        if character == '\\'
            && index + 1 < chars.len()
            && matches!(chars[index + 1], '$' | '}' | '\\')
        {
            result.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if character != '$' || index + 1 >= chars.len() {
            result.push(character);
            index += 1;
            continue;
        }
        let next = chars[index + 1];
        if next.is_ascii_digit() {
            let mut end = index + 1;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
            let number: usize = chars[index + 1..end]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            if number == 0 {
                if final_offset.is_none() {
                    final_offset = Some(result.encode_utf16().count());
                }
            } else if cursor_offset.is_none() {
                cursor_offset = Some(result.encode_utf16().count());
            }
            index = end;
            continue;
        }
        if next != '{' {
            result.push('$');
            index += 1;
            continue;
        }
        let Some(close) = find_snippet_closing_brace(&chars, index + 2) else {
            result.push('$');
            index += 1;
            continue;
        };
        let inner: String = chars[index + 2..close].iter().collect();
        let digits_end = inner
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit())
            .last()
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);
        if digits_end > 0 {
            let number: usize = inner[..digits_end].parse().unwrap_or(0);
            let rest = &inner[digits_end..];
            if let Some(text) = rest.strip_prefix(':') {
                result.push_str(text);
            } else if let Some(choices) = rest.strip_prefix('|') {
                let choices = choices.split('|').next().unwrap_or(choices);
                result.push_str(choices.split(',').next().unwrap_or(choices));
            }
            if number == 0 {
                if final_offset.is_none() {
                    final_offset = Some(result.encode_utf16().count());
                }
            } else if cursor_offset.is_none() {
                cursor_offset = Some(result.encode_utf16().count());
            }
        }
        index = close + 1;
    }
    let fallback = result.encode_utf16().count();
    (result, cursor_offset.or(final_offset).unwrap_or(fallback))
}

fn find_snippet_closing_brace(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = start;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn byte_length_for_utf16_units(text: &str, units: usize) -> usize {
    let mut count = 0usize;
    let mut bytes = 0usize;
    for character in text.chars() {
        let width = character.len_utf16();
        if count + width > units {
            break;
        }
        count += width;
        bytes += character.len_utf8();
    }
    bytes
}

fn try_hover_fence(line: &str) -> Option<(char, usize, String)> {
    let offset = line.len() - line.trim_start_matches(char::is_whitespace).len();
    let rest = &line[offset..];
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = rest
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if count < 3 {
        return None;
    }
    Some((marker, count, rest[count..].trim().to_string()))
}

fn add_hover_code_line(
    line: &str,
    language: Option<&Language>,
    width: usize,
    destination: &mut Vec<HoverLine>,
    start_in_block: &mut bool,
    tab_width: usize,
) {
    if line.is_empty() {
        destination.push(HoverLine {
            segments: Vec::new(),
        });
        return;
    }
    let text = line.as_bytes();
    let mut tokens = Vec::new();
    let end_in_block = if let Some(language) = language {
        highlight::tokenize_line(language, text, *start_in_block, &mut tokens)
    } else {
        false
    };
    *start_in_block = end_in_block;

    let mut current = HoverLine {
        segments: Vec::new(),
    };
    let mut offset = 0usize;
    let mut token_index = 0usize;
    let mut row_width = 0usize;
    let mut visual = 0usize;
    while offset < text.len() {
        let rune_start = offset;
        while token_index < tokens.len() && tokens[token_index].end <= rune_start {
            token_index += 1;
        }
        let kind = if token_index < tokens.len() && tokens[token_index].start <= rune_start {
            tokens[token_index].kind
        } else {
            SyntaxKind::Default
        };
        let (character, length) = util::decode_at(text, offset);
        offset += length.max(1);
        let mut output = String::new();
        let cell_width = if character == '\t' {
            let width = tab_width - (visual % tab_width);
            output.push_str(&" ".repeat(width));
            width
        } else {
            let width = util::display_width(character);
            if width == 0 {
                if character < ' ' || character == '\u{7F}' {
                    output.push(util::REPLACEMENT);
                    1
                } else {
                    continue;
                }
            } else {
                output.push(character);
                width
            }
        };
        if row_width > 0 && row_width + cell_width > width {
            destination.push(current);
            current = HoverLine {
                segments: Vec::new(),
            };
            row_width = 0;
            visual = 0;
        }
        add_hover_segment(&mut current, &output, kind);
        row_width += cell_width;
        visual += cell_width;
    }
    if !current.segments.is_empty() {
        destination.push(current);
    }
}

fn add_hover_segment(line: &mut HoverLine, text: &str, kind: SyntaxKind) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = line.segments.last_mut() {
        if last.kind == kind {
            last.text.push_str(text);
            return;
        }
    }
    line.segments.push(HoverSegment {
        text: text.to_string(),
        kind,
    });
}

fn wrap_hover_line(line: &str, width: usize, destination: &mut Vec<HoverLine>) {
    let width = width.max(1);
    let chars: Vec<char> = line.chars().collect();
    let mut offset = 0usize;
    while offset < chars.len() {
        let mut take = width.min(chars.len() - offset);
        if offset + take < chars.len() && take > 0 {
            if let Some(space) = chars[offset..offset + take]
                .iter()
                .rposition(|character| *character == ' ')
            {
                if space > 0 {
                    take = space;
                }
            }
        }
        if take == 0 {
            take = 1;
        }
        let text: String = chars[offset..offset + take].iter().collect();
        destination.push(HoverLine {
            segments: vec![HoverSegment {
                text: text.trim_end().to_string(),
                kind: SyntaxKind::Default,
            }],
        });
        offset += take;
        while offset < chars.len() && chars[offset] == ' ' {
            offset += 1;
        }
    }
}

pub(super) fn wrap_text(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars: Vec<char> = line.chars().collect();
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset < chars.len() {
        let take = width.min(chars.len() - offset);
        result.push(chars[offset..offset + take].iter().collect());
        offset += take;
    }
    result
}
