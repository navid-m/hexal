//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    // ---------------------------------------------------------------------
    // Commands and file operations
    // ---------------------------------------------------------------------

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
        match self.doc.save(path) {
            Ok(()) => {
                self.set_message(&format!("Saved {}", self.doc.display_name()));
                true
            }
            Err(error) => {
                self.set_error(&format!("save failed: {error}"));
                false
            }
        }
    }

    pub(super) fn trim_trailing_whitespace(&mut self) {
        let line_count = self.doc.line_count();
        let mut changed = 0usize;
        for line in (0..line_count).rev() {
            let text = self.doc.line_text(line);
            let trimmed = util::trim_trailing_whitespace(&text);
            if trimmed.len() < text.len() {
                let start = self.doc.line_start(line);
                self.doc
                    .erase(start + trimmed.len(), text.len() - trimmed.len());
                changed += 1;
            }
        }
        self.doc.break_undo_group();
        self.set_message(&format!(
            "Removed trailing whitespace from {changed} line(s)."
        ));
    }
}
