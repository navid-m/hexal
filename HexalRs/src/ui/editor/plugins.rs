//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn load_plugins(&mut self) {
        let Ok(entries) = std::fs::read_dir(Path::new("plugins")) else {
            return;
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("lua"))
            .collect();
        files.sort();
        if files.is_empty() {
            return;
        }

        let mut runtime = match LuaRuntime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.set_error(&format!("lua init failed: {error}"));
                return;
            }
        };
        let mut loaded = 0usize;
        let mut first_error: Option<String> = None;
        for file in &files {
            let path = file.to_string_lossy().into_owned();
            match runtime.load_file(&path) {
                Ok(()) => loaded += 1,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {error}", file.display()));
                    }
                }
            }
        }
        if loaded > 0 {
            self.lua = Some(runtime);
            self.set_message(&format!("Loaded {loaded} Lua plugin(s)"));
        } else if let Some(error) = first_error {
            self.set_error(&format!("lua: {error}"));
        }
    }

    pub(super) fn load_lua_plugin(&mut self, path: &str) {
        if self.lua.is_none() {
            match LuaRuntime::new() {
                Ok(runtime) => self.lua = Some(runtime),
                Err(error) => {
                    self.set_error(&format!("lua init failed: {error}"));
                    return;
                }
            }
        }
        let result = self.lua.as_mut().unwrap().load_file(path);
        match result {
            Ok(()) => self.set_message(&format!("Loaded Lua plugin {path}")),
            Err(error) => self.set_error(&format!("lua: {error}")),
        }
    }

    pub(super) fn list_lua_commands(&mut self) {
        let Some(runtime) = self.lua.as_ref() else {
            self.set_message("No Lua plugins loaded.");
            return;
        };
        let names = runtime.command_names();
        if names.is_empty() {
            self.set_message("No Lua commands registered.");
        } else {
            self.set_message(&format!("Lua commands: {}", names.join(", ")));
        }
    }

    pub(super) fn try_lua_command(&mut self, name: &str) -> bool {
        let (result, message) = {
            let Some(runtime) = self.lua.as_mut() else {
                return false;
            };
            if !runtime.has_command(name) {
                return false;
            }
            let result = runtime.invoke(name, &mut self.doc, &mut self.cursor);
            let message = runtime.take_message();
            (result, message)
        };

        match result {
            Ok(_) => {
                self.anchor = self.cursor;
                self.selection_linewise = false;
                self.preferred_column = None;
                self.mode = Mode::Normal;
                self.snap_cursor();
                self.doc.break_undo_group();
                if let Some(message) = message {
                    self.set_message(&message);
                }
            }
            Err(error) => self.set_error(&format!("lua: {error}")),
        }
        true
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
                self.set_message(&format!("Saved {}", self.doc.display_name()));
                self.notify_lsp_saved();
                if path_changed {
                    self.refresh_language();
                    self.auto_detect_indentation_on_open();
                    self.restart_lsp_for_buffer();
                } else if !self.lsp.is_running() {
                    self.start_lsp_for_buffer();
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
