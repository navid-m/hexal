//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn load_plugins(&mut self) {
        let directory = util::plugin_install::plugin_directory();
        let _ = std::fs::create_dir_all(&directory);
        let mut manager = PluginManager::new(directory, None);
        self.sync_plugin_context(&mut manager);
        manager.load_all(&mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.fire_plugin_event("editor_started", serde_json_object(&[("api_version", 1.0)]));
        self.fire_plugin_buffer_event("buffer_opened");
        self.apply_plugin_effects();
        let manager = self.plugins.as_ref().unwrap();
        if let Some(error) = manager.errors().first() {
            self.set_error(&format!("plugin: {error}"));
        } else {
            let count = manager.plugin_count();
            let deferred = manager.deferred_count();
            if let Some(message) = plugin_loaded_message(count, deferred) {
                self.set_message(&message);
            }
        }
    }

    pub(super) fn load_lua_plugin(&mut self, path: &str) {
        if self.plugins.is_none() {
            self.plugins = Some(PluginManager::new(
                util::plugin_install::plugin_directory(),
                None,
            ));
        }
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        let result = manager.load_file_plugin(path, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        match result {
            Ok(()) => self.set_message(&format!("Loaded Lua plugin {path}")),
            Err(error) => self.set_error(&format!("lua: {error}")),
        }
        self.apply_plugin_effects();
    }

    pub(super) fn list_lua_commands(&mut self) {
        let Some(manager) = self.plugins.as_ref() else {
            self.set_message("No Lua plugins loaded.");
            return;
        };
        let names = manager.command_names();
        if names.is_empty() {
            self.set_message("No Lua commands registered.");
        } else {
            self.set_message(&format!("Lua commands: {}", names.join(", ")));
        }
    }

    pub(super) fn try_lua_command(&mut self, name: &str) -> bool {
        let Some(mut manager) = self.plugins.take() else {
            return false;
        };
        if !manager.has_command(name) {
            self.plugins = Some(manager);
            return false;
        }
        self.sync_plugin_context(&mut manager);
        let result = manager.execute_command(name, &[], &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        match result {
            Ok(true) => {
                self.anchor = self.cursor;
                self.selection_linewise = false;
                self.preferred_column = None;
                self.mode = Mode::Normal;
                self.snap_cursor();
                self.doc.break_undo_group();
                self.apply_plugin_effects();
            }
            Ok(false) => return false,
            Err(error) => {
                crate::util::log::write(&format!("plugin command '{name}' failed: {error}"));
                self.set_error(&format!("lua: {error}"));
                self.apply_plugin_effects();
            }
        }
        true
    }

    pub(super) fn sync_plugin_context(&self, manager: &mut PluginManager) {
        let buffer_id = self.doc.path.clone().unwrap_or_else(|| "[No Name]".into());
        let language = self
            .active_language()
            .map(|language| language.id.clone())
            .unwrap_or_default();
        // The workspace root is the project root, not the parent directory of
        // the current file. It must remain available even for scratch buffers,
        // otherwise plugins such as projtree report "no workspace root".
        let workspace_root = Some(self.sidebar.root.to_string_lossy().into_owned());
        let selection = if self.anchor == self.cursor {
            None
        } else {
            let (start, end) = if self.anchor < self.cursor {
                (self.anchor, self.cursor)
            } else {
                (self.cursor, self.anchor)
            };
            let start_line = self.doc.line_of_offset(start);
            let end_line = self.doc.line_of_offset(end);
            let start_column = String::from_utf8_lossy(&self.doc.range(
                self.doc.line_start(start_line),
                start.saturating_sub(self.doc.line_start(start_line)),
            ))
            .chars()
            .count();
            let end_column = String::from_utf8_lossy(&self.doc.range(
                self.doc.line_start(end_line),
                end.saturating_sub(self.doc.line_start(end_line)),
            ))
            .chars()
            .count();
            Some(crate::scripting::lua::PluginSelection {
                start_line,
                start_column,
                end_line,
                end_column,
                text: String::from_utf8_lossy(&self.doc.range(start, end - start)).into_owned(),
                linewise: self.selection_linewise,
            })
        };
        manager.set_context(
            buffer_id,
            language,
            workspace_root,
            self.cols,
            self.rows,
            selection,
        );
    }

    pub(super) fn poll_plugin_tasks(&mut self) {
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        let ran = manager.run_deferred(&mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        if ran {
            self.apply_plugin_effects();
        }
    }

    pub(super) fn apply_plugin_effects(&mut self) {
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        let effects = manager.take_effects();
        let pending_commands = manager.take_pending_commands();
        for (message, warning) in effects.messages {
            if warning {
                self.set_error(&message);
            } else {
                self.set_message(&message);
            }
        }
        for (key, value) in effects.statuses {
            match value {
                Some(value) => {
                    if let Some(existing) =
                        self.plugin_status.iter_mut().find(|(name, _)| *name == key)
                    {
                        existing.1 = value;
                    } else {
                        self.plugin_status.push((key, value));
                    }
                }
                None => self.plugin_status.retain(|(name, _)| *name != key),
            }
        }
        if let Some(clipboard) = effects.clipboard {
            self.store_clipboard(clipboard.as_bytes());
        }
        for (plugin_id, message) in effects.errors {
            crate::util::log::write(&format!("plugin {plugin_id}: {message}"));
            self.set_error(&format!("error: {plugin_id}: {message}"));
        }
        for stop in effects.lsp_stops {
            if stop.is_empty() || self.lsp.server_name() == stop {
                self.lsp.stop();
                self.lsp_dirty = false;
                self.lsp_synced_revision = 0;
            }
        }
        for request in effects.lsp_requests {
            self.start_lsp_from_plugin(lsp::ServerConfig {
                name: request.name,
                command: request.command,
                arguments: request.args,
                language_id: request.language_id,
            });
        }
        self.plugins = Some(manager);
        for (name, args) in pending_commands {
            let mut line = name;
            if !args.is_empty() {
                line.push(' ');
                line.push_str(&args.join(" "));
            }
            self.command_line = line;
            self.command_cursor = self.command_line.len();
            self.submit_command();
        }
        self.sync_plugin_panels();
    }

    pub(super) fn fire_plugin_event(&mut self, event: &str, data: crate::util::json::Value) {
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        manager.fire_event(event, &data, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.apply_plugin_effects();
    }

    pub(super) fn fire_plugin_buffer_event(&mut self, event: &str) {
        let mut data = serde_json_object(&[]);
        data.set(
            "id",
            crate::util::json::Value::String(
                self.doc.path.clone().unwrap_or_else(|| "[No Name]".into()),
            ),
        );
        data.set(
            "path",
            self.doc
                .path
                .clone()
                .map(crate::util::json::Value::String)
                .unwrap_or(crate::util::json::Value::Null),
        );
        data.set(
            "modified",
            crate::util::json::Value::Bool(self.doc.is_dirty()),
        );
        data.set(
            "language",
            crate::util::json::Value::String(
                self.active_language()
                    .map(|language| language.id.clone())
                    .unwrap_or_default(),
            ),
        );
        data.set(
            "lsp_running",
            crate::util::json::Value::Bool(self.lsp.is_running()),
        );
        self.fire_plugin_event(event, data);
    }

    pub(super) fn try_plugin_key(&mut self, event: &Event, mode: &str) -> Option<bool> {
        let token = key_token(event)?;
        let mut manager = self.plugins.take()?;
        self.sync_plugin_context(&mut manager);
        let result = manager.handle_key_token(&token, mode, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        if result.is_some() {
            self.apply_plugin_effects();
        }
        result
    }

    pub(super) fn plugin_command_names(&self) -> Vec<String> {
        self.plugins
            .as_ref()
            .map(|manager| manager.command_names())
            .unwrap_or_default()
    }
    pub(super) fn save_document(&mut self, path: Option<&str>) -> bool {
        self.save_document_inner(path, false)
    }

    pub(super) fn save_document_skip_format(&mut self, path: Option<&str>) -> bool {
        self.save_document_inner(path, true)
    }

    fn save_document_inner(&mut self, path: Option<&str>, skip_format: bool) -> bool {
        let language_format = self
            .active_language()
            .and_then(|language| language.format_on_save);
        let should_format = !skip_format && language_format.unwrap_or(self.config.format_on_save);
        if should_format {
            if let Some(formatter) = self
                .active_language()
                .and_then(|language| language.resolve_formatter().cloned())
            {
                let input = self.doc.all_text();
                let target = path
                    .map(PathBuf::from)
                    .or_else(|| self.doc.path.as_ref().map(PathBuf::from));
                if let Ok(output) = format::run(&formatter, &input, target.as_deref()) {
                    if output != input {
                        self.doc.replace_all_owned(output);
                        self.doc.break_undo_group();
                    }
                }
            }
        }

        if self.config.trim_trailing_whitespace_on_save {
            self.trim_trailing_whitespace_quiet();
        }

        let previous_path = self.doc.path.clone();
        let result = self.doc.save(path);
        match result {
            Ok(()) => {
                let path_changed = previous_path != self.doc.path;
                self.command_files = None;
                self.refresh_git_diff();
                self.set_message(&format!("Saved {}", self.doc.display_name()));
                self.notify_lsp_saved();
                self.fire_plugin_buffer_event("buffer_saved");
                if path_changed {
                    self.refresh_language();
                    self.auto_detect_indentation_on_open();
                    self.restart_lsp_for_buffer();
                    self.fire_plugin_buffer_event("buffer_opened");
                }
                true
            }
            Err(error) => {
                self.set_error(&format!("save failed: {error}"));
                false
            }
        }
    }

    pub(super) fn trim_trailing_whitespace(&mut self) {
        let changed = self.trim_trailing_whitespace_quiet();
        self.set_message(&format!(
            "Removed trailing whitespace from {changed} line(s)."
        ));
    }

    fn trim_trailing_whitespace_quiet(&mut self) -> usize {
        let line_count = self.doc.line_count();
        let mut changed = 0usize;
        for line in (0..line_count).rev() {
            let text = self.doc.line_text(line);
            let trimmed = util::trim_trailing_whitespace(&text);
            if trimmed.len() < text.len() {
                let start = self.doc.line_start(line);
                let remove_at = start + trimmed.len();
                let remove_len = text.len() - trimmed.len();
                self.doc.erase(remove_at, remove_len);
                self.cursor = shift_after_removal(self.cursor, remove_at, remove_len);
                self.anchor = shift_after_removal(self.anchor, remove_at, remove_len);
                changed += 1;
            }
        }
        if changed > 0 {
            self.preferred_column = None;
            self.snap_cursor();
            self.doc.break_undo_group();
            self.lsp_dirty = self.lsp.is_running();
            self.lsp_dirty_since = std::time::Instant::now();
        }
        changed
    }
}

fn shift_after_removal(position: usize, remove_at: usize, removed: usize) -> usize {
    if position >= remove_at + removed {
        position - removed
    } else if position > remove_at {
        remove_at
    } else {
        position
    }
}

fn plugin_loaded_message(count: usize, deferred: usize) -> Option<String> {
    let total = count + deferred;
    if total == 0 {
        return None;
    }
    let plural = if total == 1 { "" } else { "s" };
    let suffix = if deferred > 0 {
        format!(" ({deferred} deferred)")
    } else {
        String::new()
    };
    Some(format!("{total} plugin{plural} loaded{suffix}"))
}

fn serde_json_object(values: &[(&str, f64)]) -> crate::util::json::Value {
    let mut object = crate::util::json::Value::Object(Vec::new());
    for (key, value) in values {
        object.set(key, crate::util::json::Value::Number(*value));
    }
    object
}

fn key_token(event: &Event) -> Option<String> {
    if event.key == Key::Char {
        if event.ch == '\0' {
            return None;
        }
        let character = if event.ch == ' ' {
            "space".to_string()
        } else {
            event.ch.to_lowercase().to_string()
        };
        if event.ctrl {
            return Some(format!("ctrl+{character}"));
        }
        if event.alt {
            return Some(format!("alt+{character}"));
        }
        return Some(character);
    }
    Some(
        match event.key {
            Key::Escape => "esc",
            Key::Enter => "enter",
            Key::Backspace => "backspace",
            Key::Delete => "delete",
            Key::Tab => "tab",
            Key::BackTab => "shift+tab",
            Key::Up => "up",
            Key::Down => "down",
            Key::Left => "left",
            Key::Right => "right",
            Key::Home => "home",
            Key::End => "end",
            Key::PageUp => "pageup",
            Key::PageDown => "pagedown",
            Key::Insert => "insert",
            _ => return None,
        }
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::plugin_loaded_message;
    use crate::core::document::Document;
    use crate::scripting::plugin::PluginManager;
    use crate::ui::editor::Editor;
    use std::path::PathBuf;

    #[test]
    fn scratch_buffers_still_have_a_workspace_root() {
        let editor = Editor::new(Document::new(), 80, 24);
        let mut manager = PluginManager::new(PathBuf::from("."), None);
        editor.sync_plugin_context(&mut manager);
        assert_eq!(
            manager.workspace_root.as_deref(),
            Some(editor.sidebar.root.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn reports_loaded_and_deferred_plugins() {
        assert_eq!(
            plugin_loaded_message(2, 88).as_deref(),
            Some("90 plugins loaded (88 deferred)")
        );
        assert_eq!(
            plugin_loaded_message(1, 0).as_deref(),
            Some("1 plugin loaded")
        );
        assert_eq!(plugin_loaded_message(0, 0), None);
    }
}
