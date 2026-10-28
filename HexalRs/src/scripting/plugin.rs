//! Plugin discovery, manifests, lazy loading, commands, keymaps and events.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::core::document::Document;
use crate::scripting::json::Value;
use crate::scripting::lua::{
    HostEffects, HostRequest, HostState, KeyBinding, LuaRuntime, PluginPanel, PluginSelection,
};

#[allow(dead_code)]
#[derive(Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub directory: String,
    pub entry_path: String,
    pub manifest_path: String,
    pub enabled: bool,
    pub running: bool,
    pub deferred: bool,
    pub error: String,
    pub commands: Vec<String>,
}

struct PluginEntry {
    id: String,
    name: String,
    version: String,
    description: String,
    author: String,
    license: String,
    directory: PathBuf,
    entry_path: PathBuf,
    #[allow(dead_code)]
    manifest_path: PathBuf,
    lazy_language: String,
    enabled: bool,
    deferred: bool,
    error: String,
    runtime: Option<LuaRuntime>,
}

pub struct PluginManager {
    pub host: Rc<RefCell<HostState>>,
    entries: Vec<PluginEntry>,
    errors: Vec<String>,
    disabled: HashSet<String>,
    plugin_dir: PathBuf,
    state_path: PathBuf,
    effects: HostEffects,
    pub buffer_id: String,
    pub language: String,
    pub workspace_root: Option<String>,
    pub columns: usize,
    pub rows: usize,
    pub selection: Option<PluginSelection>,
    pending_keys: Vec<String>,
    pending_mode: String,
    pending_since: Instant,
}

#[allow(dead_code)]
impl PluginManager {
    pub fn new(plugin_dir: impl Into<PathBuf>, state_path: Option<PathBuf>) -> PluginManager {
        let plugin_dir = plugin_dir.into();
        let state_path = state_path.unwrap_or_else(|| {
            crate::core::config::hexal_home_directory().join("disabled-plugins")
        });
        PluginManager {
            host: Rc::new(RefCell::new(HostState::default())),
            entries: Vec::new(),
            errors: Vec::new(),
            disabled: HashSet::new(),
            plugin_dir,
            state_path,
            effects: HostEffects::default(),
            buffer_id: "[No Name]".into(),
            language: String::new(),
            workspace_root: None,
            columns: 80,
            rows: 24,
            selection: None,
            pending_keys: Vec::new(),
            pending_mode: String::new(),
            pending_since: Instant::now(),
        }
    }

    pub fn set_context(
        &mut self,
        buffer_id: String,
        language: String,
        workspace_root: Option<String>,
        columns: usize,
        rows: usize,
        selection: Option<PluginSelection>,
    ) {
        self.buffer_id = buffer_id;
        self.language = language;
        self.workspace_root = workspace_root;
        self.columns = columns;
        self.rows = rows;
        self.selection = selection;
        if let Some(root) = self.workspace_root.clone() {
            for entry in &mut self.entries {
                if let Some(runtime) = &mut entry.runtime {
                    runtime.set_workspace_root(&root);
                }
            }
        }
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn plugin_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.runtime.is_some())
            .count()
    }

    pub fn deferred_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.deferred && entry.enabled && entry.runtime.is_none())
            .count()
    }

    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.entries
            .iter()
            .map(|entry| PluginInfo {
                id: entry.id.clone(),
                name: entry.name.clone(),
                version: entry.version.clone(),
                description: entry.description.clone(),
                author: entry.author.clone(),
                license: entry.license.clone(),
                directory: entry.directory.to_string_lossy().into_owned(),
                entry_path: entry.entry_path.to_string_lossy().into_owned(),
                manifest_path: entry.manifest_path.to_string_lossy().into_owned(),
                enabled: entry.enabled,
                running: entry.runtime.is_some(),
                deferred: entry.deferred && entry.enabled && entry.runtime.is_none(),
                error: entry.error.clone(),
                commands: self.commands_for(&entry.id),
            })
            .collect()
    }

    pub fn load_all(&mut self, document: &mut Document, cursor: &mut usize) {
        let _ = fs::create_dir_all(&self.plugin_dir);
        self.disabled = self.load_disabled();
        let mut directories: Vec<PathBuf> = match fs::read_dir(&self.plugin_dir) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect(),
            Err(error) => {
                self.errors.push(format!(
                    "cannot read plugin directory '{}': {error}",
                    self.plugin_dir.display()
                ));
                return;
            }
        };
        directories.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        for directory in directories {
            self.discover(directory, document, cursor);
        }
    }

    pub fn load_file_plugin(
        &mut self,
        path: &str,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(), String> {
        let path = PathBuf::from(path);
        let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let id = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "plugin".into());
        self.entries.push(PluginEntry {
            id: id.clone(),
            name: id.clone(),
            version: String::new(),
            description: String::new(),
            author: String::new(),
            license: String::new(),
            directory,
            entry_path: path,
            manifest_path: PathBuf::new(),
            lazy_language: String::new(),
            enabled: true,
            deferred: false,
            error: String::new(),
            runtime: None,
        });
        let index = self.entries.len() - 1;
        self.start_plugin(index, document, cursor);
        if self.entries[index].runtime.is_none() {
            return Err(self.entries[index].error.clone());
        }
        Ok(())
    }

    fn discover(&mut self, directory: PathBuf, document: &mut Document, cursor: &mut usize) {
        let manifest_path = directory.join("plugin.toml");
        let mut manifest = Vec::new();
        let mut manifest_error = String::new();
        if manifest_path.exists() {
            if let Err(error) = parse_manifest(&manifest_path, &mut manifest) {
                manifest_error = error;
            }
        }
        let fallback = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "plugin".into());
        let entry = PluginEntry {
            id: fallback.clone(),
            name: fallback.clone(),
            version: String::new(),
            description: String::new(),
            author: String::new(),
            license: String::new(),
            directory: directory.clone(),
            entry_path: directory.join("init.lua"),
            manifest_path: manifest_path.clone(),
            lazy_language: String::new(),
            enabled: false,
            deferred: false,
            error: String::new(),
            runtime: None,
        };
        self.entries.push(entry);
        let index = self.entries.len() - 1;
        if !manifest_error.is_empty() {
            self.record_error(index, manifest_error);
            return;
        }
        let value = |key: &str| -> Option<&str> {
            manifest
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str())
        };
        if let Some(id) = value("id").map(str::trim).filter(|id| !id.is_empty()) {
            self.entries[index].id = id.to_string();
        }
        let id = self.entries[index].id.clone();
        if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
            self.entries[index].id = fallback;
            self.record_error(index, "invalid plugin id".into());
            return;
        }
        self.entries[index].name = value("name")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&id)
            .to_string();
        self.entries[index].version = value("version").unwrap_or("").trim().to_string();
        self.entries[index].description = value("description").unwrap_or("").trim().to_string();
        self.entries[index].author = value("author").unwrap_or("").trim().to_string();
        self.entries[index].license = value("license").unwrap_or("").trim().to_string();
        let entry_file = value("entry")
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .unwrap_or("init.lua");
        let full_directory = fs::canonicalize(&directory).unwrap_or(directory.clone());
        let entry_path = directory.join(entry_file);
        let full_entry = fs::canonicalize(&entry_path)
            .unwrap_or_else(|_| entry_path.parent().unwrap_or(&directory).join(entry_file));
        if !full_entry.starts_with(&full_directory) {
            self.record_error(index, "entry point escapes the plugin directory".into());
            return;
        }
        self.entries[index].entry_path = entry_path.clone();
        if !entry_path.is_file() {
            self.record_error(index, format!("entry point '{entry_file}' not found"));
            return;
        }
        let manifest_enabled = value("enabled")
            .map(|value| !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let enabled = manifest_enabled && !self.disabled.contains(&id);
        let lazy = value("lazy")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.entries[index].lazy_language = value("language").unwrap_or("").trim().to_string();
        self.entries[index].enabled = enabled;
        if enabled && lazy && !self.entries[index].lazy_language.is_empty() {
            self.entries[index].deferred = true;
            return;
        }
        if enabled {
            self.start_plugin(index, document, cursor);
        }
    }

    fn record_error(&mut self, index: usize, message: String) {
        let id = self.entries[index].id.clone();
        crate::util::log::write(&format!("plugin {id}: {message}"));
        self.entries[index].error = message.clone();
        self.entries[index].enabled = false;
        self.errors.push(format!("{id}: {message}"));
    }

    fn start_plugin(&mut self, index: usize, document: &mut Document, cursor: &mut usize) {
        if self.entries[index].runtime.is_some() {
            self.entries[index].enabled = true;
            return;
        }
        let id = self.entries[index].id.clone();
        self.errors
            .retain(|error| !error.starts_with(&format!("{id}: ")));
        let directory = self.entries[index].directory.clone();
        let entry_path = self.entries[index].entry_path.clone();
        let mut runtime = match LuaRuntime::new(
            id.clone(),
            directory.to_string_lossy().into_owned(),
            self.host.clone(),
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                crate::util::log::write(&format!("plugin {id}: {error}"));
                self.entries[index].error = error.clone();
                self.entries[index].enabled = false;
                self.errors.push(format!("{id}: {error}"));
                return;
            }
        };
        self.set_host_context();
        if let Some(root) = &self.workspace_root {
            runtime.set_workspace_root(root);
        }
        runtime.sync_from_document(document, *cursor);
        if let Err(error) = runtime.load_file(&entry_path.to_string_lossy()) {
            crate::util::log::write(&format!("plugin {id}: {error}"));
            self.entries[index].error = error.clone();
            self.entries[index].enabled = false;
            self.errors.push(format!("{id}: {error}"));
            return;
        }
        let effects = runtime.apply_to_document(document, cursor);
        self.merge_effects(&runtime, effects);
        self.entries[index].runtime = Some(runtime);
        self.entries[index].error.clear();
        self.entries[index].enabled = true;
        self.entries[index].deferred = false;
    }

    fn set_host_context(&self) {
        let mut host = self.host.borrow_mut();
        host.buffer_id = self.buffer_id.clone();
        host.language = self.language.clone();
        host.workspace_root = self.workspace_root.clone();
        host.columns = self.columns;
        host.rows = self.rows;
        host.selection = self.selection.clone();
        if let Some(path) = self.buffer_id.strip_prefix("file://") {
            host.path = Some(path.to_string());
        } else if self.buffer_id.starts_with('/') {
            host.path = Some(self.buffer_id.clone());
        }
    }

    fn sync_host(&self, document: &mut Document, cursor: &mut usize) {
        self.set_host_context();
        if !self.entries.iter().any(|entry| entry.runtime.is_some()) {
            self.host.borrow_mut().load(document.all_text(), *cursor);
        }
    }

    fn merge_effects(&mut self, runtime: &LuaRuntime, effects: HostEffects) {
        let (requests, stops) = runtime.drain_requests();
        self.effects.messages.extend(effects.messages);
        self.effects.statuses.extend(effects.statuses);
        if effects.clipboard.is_some() {
            self.effects.clipboard = effects.clipboard;
        }
        self.effects.errors.extend(effects.errors);
        self.effects.lsp_requests.extend(requests);
        self.effects.lsp_stops.extend(stops);
        self.effects.panels_changed |= effects.panels_changed;
    }

    pub fn take_effects(&mut self) -> HostEffects {
        std::mem::take(&mut self.effects)
    }

    pub fn take_pending_commands(&mut self) -> Vec<(String, Vec<String>)> {
        std::mem::take(&mut self.host.borrow_mut().pending_commands)
    }

    pub fn host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host.borrow_mut().requests)
    }

    pub fn panels(&self) -> Vec<PluginPanel> {
        self.host.borrow().panels.clone()
    }

    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .iter()
            .filter_map(|entry| entry.runtime.as_ref())
            .flat_map(|runtime| runtime.command_names())
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.entries
            .iter()
            .filter_map(|entry| entry.runtime.as_ref())
            .any(|runtime| runtime.has_command(name))
    }

    pub fn command_description(&self, name: &str) -> Option<String> {
        for entry in &self.entries {
            let Some(runtime) = &entry.runtime else {
                continue;
            };
            if let Some(command) = runtime.find_command(name) {
                return Some(command.description);
            }
        }
        None
    }

    pub fn has_subscribers(&self, event: &str) -> bool {
        self.entries
            .iter()
            .filter_map(|entry| entry.runtime.as_ref())
            .any(|runtime| {
                runtime
                    .shared
                    .borrow()
                    .subscriptions
                    .iter()
                    .any(|subscription| subscription.event.eq_ignore_ascii_case(event))
            })
    }

    pub fn execute_command(
        &mut self,
        name: &str,
        args: &[String],
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<bool, String> {
        let index = self.entries.iter().position(|entry| {
            entry
                .runtime
                .as_ref()
                .map(|runtime| runtime.has_command(name))
                .unwrap_or(false)
        });
        let Some(index) = index else {
            return Ok(false);
        };
        self.sync_host(document, cursor);
        let Some(mut runtime) = self.entries[index].runtime.take() else {
            return Ok(false);
        };
        let result = runtime.call_command(name, args, document, cursor);
        match result {
            Ok((handled, effects)) => {
                self.merge_effects(&runtime, effects);
                self.entries[index].runtime = Some(runtime);
                Ok(handled)
            }
            Err(error) => {
                crate::util::log::write(&format!("plugin command '{name}' failed: {error}"));
                self.entries[index].runtime = Some(runtime);
                Err(error)
            }
        }
    }

    pub fn fire_event(
        &mut self,
        event: &str,
        data: &Value,
        document: &mut Document,
        cursor: &mut usize,
    ) {
        if event.eq_ignore_ascii_case("buffer_opened") {
            if let Some(language) = data.get("language").and_then(Value::as_str) {
                self.load_deferred(language, document, cursor);
            }
        }
        for index in 0..self.entries.len() {
            let Some(mut runtime) = self.entries[index].runtime.take() else {
                continue;
            };
            runtime.fire_event(event, data, document, cursor);
            let effects = runtime.apply_to_document(document, cursor);
            self.merge_effects(&runtime, effects);
            self.entries[index].runtime = Some(runtime);
        }
    }

    fn load_deferred(&mut self, language: &str, document: &mut Document, cursor: &mut usize) {
        let indexes: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.deferred
                    && entry.enabled
                    && entry.runtime.is_none()
                    && entry.lazy_language.eq_ignore_ascii_case(language)
            })
            .map(|(index, _)| index)
            .collect();
        for index in indexes {
            self.start_plugin(index, document, cursor);
        }
    }

    fn commands_for(&self, plugin_id: &str) -> Vec<String> {
        let mut names = Vec::new();
        for entry in &self.entries {
            if entry.id != plugin_id {
                continue;
            }
            if let Some(runtime) = &entry.runtime {
                names.extend(runtime.command_names());
            }
        }
        names.sort();
        names
    }

    fn load_disabled(&self) -> HashSet<String> {
        let mut disabled = HashSet::new();
        let Ok(text) = fs::read_to_string(&self.state_path) else {
            return disabled;
        };
        for line in text.lines() {
            let id = line.trim();
            if !id.is_empty() {
                disabled.insert(id.to_string());
            }
        }
        disabled
    }

    fn save_disabled(&self) {
        if self.state_path.as_os_str().is_empty() {
            return;
        }
        if let Some(directory) = self.state_path.parent() {
            let _ = fs::create_dir_all(directory);
        }
        let mut ids: Vec<&String> = self.disabled.iter().collect();
        ids.sort_by_key(|id| id.to_lowercase());
        let mut output = String::new();
        for id in ids {
            output.push_str(id);
            output.push('\n');
        }
        let _ = fs::write(&self.state_path, output);
    }

    pub fn set_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(), String> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err(format!("unknown plugin '{id}'"));
        };
        if enabled {
            if self.entries[index].runtime.is_none() {
                self.start_plugin(index, document, cursor);
            }
            if self.entries[index].runtime.is_none() {
                return Err(if self.entries[index].error.is_empty() {
                    format!("could not enable plugin '{id}'")
                } else {
                    self.entries[index].error.clone()
                });
            }
            self.disabled.remove(id);
        } else {
            self.stop_plugin(index);
            self.entries[index].enabled = false;
            self.disabled.insert(id.to_string());
        }
        self.save_disabled();
        Ok(())
    }

    fn stop_plugin(&mut self, index: usize) {
        let runtime = self.entries[index].runtime.take();
        if let Some(runtime) = runtime {
            let plugin_id = runtime.plugin_id.clone();
            drop(runtime);
            self.host
                .borrow_mut()
                .panels
                .retain(|panel| panel.plugin_id != plugin_id);
        }
        self.entries[index].enabled = false;
    }

    pub fn reload_plugin(
        &mut self,
        id: &str,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(), String> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err(format!("unknown plugin '{id}'"));
        };
        let directory = self.entries[index].directory.clone();
        let was_enabled = self.entries[index].enabled;
        self.stop_plugin(index);
        self.entries.remove(index);
        let before = self.entries.len();
        self.discover(directory, document, cursor);
        if self.entries.len() > before {
            if let Some(entry) = self.entries.get(before) {
                if !entry.error.is_empty() {
                    return Err(format!("{}: {}", entry.id, entry.error));
                }
                if was_enabled && entry.runtime.is_none() {
                    self.start_plugin(before, document, cursor);
                }
            }
        }
        Ok(())
    }

    pub fn reload_all(
        &mut self,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(), String> {
        for index in 0..self.entries.len() {
            self.stop_plugin(index);
        }
        self.entries.clear();
        self.errors.clear();
        self.load_all(document, cursor);
        if let Some(error) = self.errors.first() {
            return Err(error.clone());
        }
        Ok(())
    }

    pub fn handle_key_token(
        &mut self,
        token: &str,
        mode: &str,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Option<bool> {
        if !self.pending_keys.is_empty()
            && (self.pending_since.elapsed() > Duration::from_millis(800)
                || !self.pending_mode.eq_ignore_ascii_case(mode))
        {
            self.pending_keys.clear();
        }
        let index = self.pending_keys.len();
        let bindings = self.keybindings();
        if bindings.is_empty() {
            return None;
        }
        let candidates: Vec<KeyBinding> = bindings
            .into_iter()
            .filter(|binding| {
                binding.mode.eq_ignore_ascii_case(mode)
                    && binding.keys.len() > index
                    && binding.keys[index] == token
                    && (0..index).all(|position| {
                        binding.keys.get(position) == self.pending_keys.get(position)
                    })
            })
            .collect();
        if candidates.is_empty() {
            if !self.pending_keys.is_empty() {
                self.pending_keys.clear();
            }
            return None;
        }
        self.pending_keys.push(token.to_string());
        self.pending_mode = mode.to_string();
        self.pending_since = Instant::now();
        for binding in candidates {
            if binding.keys.len() != self.pending_keys.len() {
                continue;
            }
            self.pending_keys.clear();
            if let Some(command) = binding.command.clone() {
                let handled = self
                    .execute_command(&command, &[], document, cursor)
                    .unwrap_or(false);
                if !handled {
                    self.effects
                        .messages
                        .push((format!("unknown plugin command '{command}'"), true));
                    return Some(true);
                }
                return Some(handled);
            }
            if let Some(callback) = binding.callback.clone() {
                let Some(index) = self
                    .entries
                    .iter()
                    .position(|entry| entry.id == binding.plugin_id)
                else {
                    return Some(false);
                };
                let Some(mut runtime) = self.entries[index].runtime.take() else {
                    return Some(false);
                };
                let result = runtime.call_key_callback(&callback, document, cursor);
                match result {
                    Ok((handled, effects)) => {
                        self.merge_effects(&runtime, effects);
                        self.entries[index].runtime = Some(runtime);
                        return Some(handled);
                    }
                    Err(error) => {
                        self.entries[index].runtime = Some(runtime);
                        self.effects
                            .errors
                            .push((binding.plugin_id, format!("keymap failed: {error}")));
                        return Some(true);
                    }
                }
            }
            return Some(true);
        }
        Some(true)
    }

    pub fn keybindings(&self) -> Vec<KeyBinding> {
        let mut bindings: Vec<KeyBinding> = Vec::new();
        for entry in &self.entries {
            if let Some(runtime) = &entry.runtime {
                for binding in runtime.keybindings() {
                    bindings.retain(|existing| {
                        !(existing.mode.eq_ignore_ascii_case(&binding.mode)
                            && existing.keys == binding.keys)
                    });
                    bindings.push(binding);
                }
            }
        }
        bindings
    }

    pub fn plugin_key_handlers(&self) -> Vec<(i64, i64)> {
        let _ = &self.host;
        Vec::new()
    }

    pub fn resolve_prompt(&mut self, id: i64, value: Option<&str>) -> bool {
        for entry in &mut self.entries {
            if let Some(runtime) = &entry.runtime {
                if runtime.resolve_prompt(id, value) {
                    return true;
                }
            }
        }
        false
    }

    pub fn resolve_select(&mut self, id: i64, index: Option<usize>) -> bool {
        for entry in &mut self.entries {
            if let Some(runtime) = &entry.runtime {
                if runtime.resolve_select(id, index) {
                    return true;
                }
            }
        }
        false
    }

    pub fn has_due_deferred_tasks(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry
                .runtime
                .as_ref()
                .map(|runtime| runtime.has_due_deferred_tasks())
                .unwrap_or(false)
        })
    }

    /// Runs plugin work scheduled with `hexal.ui.defer` and delivers completed
    /// asynchronous filesystem scans. Returns true when any callback ran.
    pub fn run_deferred(&mut self, document: &mut Document, cursor: &mut usize) -> bool {
        self.sync_host(document, cursor);
        let mut ran = false;
        for index in 0..self.entries.len() {
            let Some(mut runtime) = self.entries[index].runtime.take() else {
                continue;
            };
            let (did_run, effects) = runtime.run_deferred(document, cursor);
            ran |= did_run;
            self.merge_effects(&runtime, effects);
            self.entries[index].runtime = Some(runtime);
        }
        ran
    }

    pub fn tick(&mut self) {
        let _ = &self.host;
    }

    pub fn handle_panel_key(
        &mut self,
        panel_id: i64,
        key: &str,
        selected: usize,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Option<Result<bool, String>> {
        let plugin_id = self.runtime_for_panel(panel_id)?;
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == plugin_id)?;
        let mut runtime = self.entries[index].runtime.take()?;
        let result = runtime.call_panel_key(panel_id, key, selected, document, cursor);
        self.entries[index].runtime = Some(runtime);
        match result {
            Ok(Some((handled, effects))) => {
                let runtime = self.entries[index].runtime.take()?;
                self.merge_effects(&runtime, effects);
                self.entries[index].runtime = Some(runtime);
                Some(Ok(handled))
            }
            Ok(None) => Some(Ok(false)),
            Err(error) => Some(Err(error)),
        }
    }

    pub fn runtime_for_panel(&self, panel_id: i64) -> Option<String> {
        for entry in &self.entries {
            if let Some(runtime) = &entry.runtime {
                if runtime
                    .shared
                    .borrow()
                    .panel_handlers
                    .contains_key(&panel_id)
                {
                    return Some(entry.id.clone());
                }
            }
        }
        None
    }
}

fn parse_manifest(path: &Path, values: &mut Vec<(String, String)>) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some(separator) = line.find('=') else {
            continue;
        };
        if separator == 0 {
            continue;
        }
        let key = line[..separator].trim().to_string();
        let mut value = line[separator + 1..].trim().to_string();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = value[1..value.len() - 1].to_string();
        }
        if !key.is_empty() {
            if let Some(existing) = values
                .iter_mut()
                .find(|(name, _)| name.eq_ignore_ascii_case(&key))
            {
                existing.1 = value;
            } else {
                values.push((key, value));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;

    fn temp_directory(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("hexal-plugin-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loads_plugins_registers_commands_and_fires_events() {
        let root = temp_directory("commands");
        let plugin = root.join("plugins").join("example");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "name = \"Example\"\nversion = \"1.2.3\"\ndescription = \"Demo\"\n",
        )
        .unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            hexal.commands.register("upper", function()
                hexal.set_text(string.upper(hexal.editor.get_text()))
                hexal.ui.message("UPPER_OK")
            end, "Upper case")
            hexal.events.on("cursor_moved", function(event)
                hexal.ui.message("MOVED:" .. tostring(event.line))
            end)
            "#,
        )
        .unwrap();

        let mut document = Document::from_bytes(Some("/tmp/example.txt".into()), b"hello".to_vec());
        let mut cursor = 0usize;
        let mut manager = PluginManager::new(root.join("plugins"), Some(root.join("state")));
        manager.set_context(
            "/tmp/example.txt".into(),
            "text".into(),
            Some("/tmp".into()),
            80,
            24,
            None,
        );
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        assert!(manager.has_command("upper"));
        assert!(manager.has_command("UPPER"));
        assert_eq!(manager.command_names(), vec!["upper".to_string()]);
        assert!(manager
            .execute_command("upper", &[], &mut document, &mut cursor)
            .unwrap());
        assert_eq!(document.all_text(), b"HELLO");
        let effects = manager.take_effects();
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "UPPER_OK"));

        let mut data = Value::Object(Vec::new());
        data.set("line", Value::Number(2.0));
        manager.fire_event("cursor_moved", &data, &mut document, &mut cursor);
        let effects = manager.take_effects();
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "MOVED:2"));

        manager
            .set_enabled("example", false, &mut document, &mut cursor)
            .unwrap();
        assert!(!manager.has_command("upper"));
        manager
            .set_enabled("example", true, &mut document, &mut cursor)
            .unwrap();
        assert!(manager.has_command("upper"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn events_without_subscribers_preserve_the_cursor() {
        let root = temp_directory("quiet-events");
        let plugin = root.join("plugins").join("quiet");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "name = \"Quiet\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        fs::write(
            plugin.join("init.lua"),
            "hexal.commands.register(\"noop\", function() end)\n",
        )
        .unwrap();

        let mut document =
            Document::from_bytes(Some("/tmp/quiet.txt".into()), b"one\ntwo\nthree\n".to_vec());
        let mut cursor = 0usize;
        let mut manager = PluginManager::new(root.join("plugins"), None);
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());

        // The host mirror still points at offset zero. Saving must not drag
        // the editor cursor back there just because no plugin listens for
        // `buffer_saved`.
        cursor = document.line_start(2) + 1;
        let data = Value::Object(Vec::new());
        manager.fire_event("buffer_saved", &data, &mut document, &mut cursor);
        assert_eq!(cursor, document.line_start(2) + 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn markdown_tools_plugin_works() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../HexalCs/Hexal/bin/Debug/net9.0/linux-x64/data/plugins/mdtools");
        if !source.is_dir() {
            return;
        }
        let root = temp_directory("mdtools");
        let plugin = root.join("plugins").join("mdtools");
        fs::create_dir_all(&plugin).unwrap();
        for entry in fs::read_dir(&source).unwrap().flatten() {
            fs::copy(entry.path(), plugin.join(entry.file_name())).unwrap();
        }
        let mut document =
            Document::from_bytes(Some("/tmp/notes.md".into()), b"- first\n- second".to_vec());
        let mut cursor = document.line_start(1) + 8;
        let mut manager = PluginManager::new(root.join("plugins"), Some(root.join("state")));
        manager.set_context(
            "/tmp/notes.md".into(),
            "markdown".into(),
            Some("/tmp".into()),
            120,
            40,
            None,
        );
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        let mut data = Value::Object(Vec::new());
        data.set("language", Value::String("markdown".into()));
        manager.fire_event("buffer_opened", &data, &mut document, &mut cursor);
        assert_eq!(
            manager.handle_key_token("enter", "insert", &mut document, &mut cursor),
            Some(true)
        );
        assert_eq!(document.all_text(), b"- first\n- second\n- ");
        assert_eq!(document.line_of_offset(cursor), 2);
        assert_eq!(
            manager.handle_key_token("enter", "insert", &mut document, &mut cursor),
            Some(true)
        );
        assert_eq!(document.all_text(), b"- first\n- second\n");
        let mut data = Value::Object(Vec::new());
        data.set("language", Value::String("text".into()));
        manager.fire_event("buffer_opened", &data, &mut document, &mut cursor);
        assert_eq!(
            manager.handle_key_token("enter", "insert", &mut document, &mut cursor),
            Some(false)
        );
        assert_eq!(document.all_text(), b"- first\n- second\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keymaps_handle_sequences_and_pass_through() {
        let root = temp_directory("keys");
        let plugin = root.join("plugins").join("keys");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("plugin.toml"), "name = \"Keys\"\n").unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            hexal.keymap.map_callback("enter", function()
                hexal.editor.insert_text("X")
                return true
            end, "insert")
            hexal.keymap.map_callback("esc", function()
                return false
            end, "normal")
            hexal.keymap.map_callback("ctrl+k ctrl+s", function()
                hexal.ui.message("SEQUENCE")
                return true
            end, "normal")
            "#,
        )
        .unwrap();
        let mut document = Document::from_bytes(Some("/tmp/keys.txt".into()), b"ab".to_vec());
        let mut cursor = 0usize;
        let mut manager = PluginManager::new(root.join("plugins"), Some(root.join("state")));
        manager.set_context("/tmp/keys.txt".into(), "text".into(), None, 80, 24, None);
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        assert_eq!(
            manager.handle_key_token("enter", "insert", &mut document, &mut cursor),
            Some(true)
        );
        assert_eq!(document.all_text(), b"Xab");
        assert_eq!(
            manager.handle_key_token("esc", "normal", &mut document, &mut cursor),
            Some(false)
        );
        assert_eq!(
            manager.handle_key_token("ctrl+k", "normal", &mut document, &mut cursor),
            Some(true)
        );
        assert_eq!(
            manager.handle_key_token("ctrl+s", "normal", &mut document, &mut cursor),
            Some(true)
        );
        let effects = manager.take_effects();
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "SEQUENCE"));
        assert_eq!(
            manager.handle_key_token("unknown", "normal", &mut document, &mut cursor),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn process_panel_and_config_api_work() {
        let root = temp_directory("panel");
        let plugin = root.join("plugins").join("api");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "name = \"API\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            local result = hexal.exec.run("echo", { "hello" })
            assert(result.ok, result.error)
            assert(result.stdout:find("hello", 1, true) ~= nil, result.stdout)
            assert(hexal.lsp.missing_api_key == nil, "missing lazy key")

            local panel = hexal.ui.panel({
                title = "Test",
                lines = {
                    { segments = { { text = "one", style = "accent" } } },
                    { segments = { { text = "two", style = "diff_added" } },
                      selectable = false },
                },
                selected = 1,
            }, function(key, selected)
                return key == "enter"
            end)
            panel:set_title("Changed")
            panel:set_footer("footer")
            panel:set_selectable(true)
            panel:set_selected(2)
            panel:set_lines({
                { segments = { { text = "three", style = "muted" } } },
            })

            hexal.config.set("answer", 42)
            assert(hexal.config.get("answer") == 42)
            hexal.config.set("nested.name", "hexal")
            assert(hexal.config.get("nested.name") == "hexal")
            hexal.ui.message("PROCESS_PANEL_OK")
            "#,
        )
        .unwrap();

        let mut document = Document::new();
        let mut cursor = 0usize;
        let mut manager = PluginManager::new(root.join("plugins"), Some(root.join("state")));
        manager.set_context("[No Name]".into(), "text".into(), None, 80, 24, None);
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        let effects = manager.take_effects();
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "PROCESS_PANEL_OK"));
        let panels = manager.panels();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].title, "Changed");
        assert_eq!(panels[0].footer, "footer");
        assert_eq!(panels[0].lines.len(), 1);
        assert_eq!(panels[0].lines[0].segments[0].text, "three");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installed_gitlog_plugin_registers_gl() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("plugins")
            .join("gitlog");
        let init = root.join("init.lua");
        if !init.is_file() {
            return;
        }
        let mut manager = PluginManager::new(root.parent().unwrap(), None);
        let mut document = Document::new();
        let mut cursor = 0usize;
        manager
            .load_file_plugin(&init.to_string_lossy(), &mut document, &mut cursor)
            .unwrap();
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        assert!(manager.has_command("gl"), "{:?}", manager.command_names());
    }

    #[test]
    fn get_workspace_root_returns_the_configured_project_root() {
        let root = temp_directory("workspace-root");
        let plugin = root.join("plugins").join("root");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            hexal.commands.register("root", function()
                hexal.ui.message(hexal.editor.get_workspace_root() or "NIL")
            end)
        "#,
        )
        .unwrap();
        let mut manager = PluginManager::new(plugin.parent().unwrap(), None);
        manager.set_context(
            "scratch".into(),
            "text".into(),
            Some("/tmp/hexal-workspace".into()),
            80,
            24,
            None,
        );
        let mut document = Document::new();
        let mut cursor = 0usize;
        manager.load_all(&mut document, &mut cursor);
        assert!(manager
            .execute_command("root", &[], &mut document, &mut cursor)
            .unwrap());
        let effects = manager.take_effects();
        assert_eq!(
            effects.messages.last().map(|(message, _)| message.as_str()),
            Some("/tmp/hexal-workspace")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_commands_open_panels_handle_keys_and_report_errors() {
        let root = temp_directory("panels");
        let plugin = root.join("plugins").join("panels");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "name = \"Panels\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            local panel
            hexal.commands.register("demo", function()
                panel = hexal.ui.panel({
                    title = " Demo ",
                    footer = "foot",
                    selected = 2,
                    lines = {
                        { segments = { { text = "one", style = "accent" } }, selectable = true },
                        { segments = { { text = "two", style = "text" } }, selectable = true },
                    },
                }, function(key, selected)
                    if key == "q" then
                        panel.close()
                        return true
                    end
                    return key == "enter" and selected == 2
                end)
            end, "Demo command")
            hexal.commands.register("fail", function()
                error("boom")
            end)
            "#,
        )
        .unwrap();

        let mut manager =
            crate::scripting::plugin::PluginManager::new(plugin.parent().unwrap(), None);
        let mut document = Document::new();
        let mut cursor = 0usize;
        manager.load_all(&mut document, &mut cursor);
        assert!(manager.errors().is_empty(), "{:?}", manager.errors());
        assert!(manager.has_command("demo"));
        assert!(manager.has_command("fail"));

        assert!(manager
            .execute_command("demo", &[], &mut document, &mut cursor)
            .unwrap());
        let panels = manager.panels();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].title, " Demo ");
        assert_eq!(panels[0].selected, 1);
        let panel_id = panels[0].id;

        assert_eq!(
            manager.handle_panel_key(panel_id, "enter", 1, &mut document, &mut cursor),
            Some(Ok(true))
        );
        assert_eq!(
            manager.handle_panel_key(panel_id, "q", 1, &mut document, &mut cursor),
            Some(Ok(true))
        );
        assert!(manager.panels().is_empty());

        let error = manager
            .execute_command("fail", &[], &mut document, &mut cursor)
            .unwrap_err();
        assert!(error.contains("boom"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lazy_plugins_start_for_their_language() {
        let root = temp_directory("lazy");
        let plugin = root.join("plugins").join("lazy");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            "name = \"Lazy\"\nlazy = true\nlanguage = \"rust\"\n",
        )
        .unwrap();
        fs::write(
            plugin.join("init.lua"),
            r#"
            hexal.ui.message("LAZY_STARTED")
            hexal.events.on("buffer_opened", function(event)
                hexal.ui.message("OPENED:" .. tostring(event.language))
            end)
            "#,
        )
        .unwrap();
        let mut document = Document::new();
        let mut cursor = 0usize;
        let mut manager = PluginManager::new(root.join("plugins"), Some(root.join("state")));
        manager.set_context("[No Name]".into(), String::new(), None, 80, 24, None);
        manager.load_all(&mut document, &mut cursor);
        assert_eq!(manager.plugin_count(), 0);
        assert_eq!(manager.deferred_count(), 1);
        let mut data = Value::Object(Vec::new());
        data.set("language", Value::String("text".into()));
        manager.fire_event("buffer_opened", &data, &mut document, &mut cursor);
        assert_eq!(manager.plugin_count(), 0);
        assert_eq!(manager.deferred_count(), 1);
        data.set("language", Value::String("rust".into()));
        manager.fire_event("buffer_opened", &data, &mut document, &mut cursor);
        assert_eq!(manager.plugin_count(), 1);
        assert_eq!(manager.deferred_count(), 0);
        let effects = manager.take_effects();
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "LAZY_STARTED"));
        assert!(effects
            .messages
            .iter()
            .any(|(message, _)| message == "OPENED:rust"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn language_server_associations_live_in_language_plugins() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("plugins");
        assert!(root.is_dir());
        assert!(!root
            .parent()
            .expect("data directory")
            .join("lsp_servers.json")
            .exists());

        for language in ["rust", "c", "typescript", "toml"] {
            let directory = root.join(language);
            let init = fs::read_to_string(directory.join("init.lua")).unwrap();
            assert!(
                init.contains("hexal.lsp.start"),
                "{language} init.lua does not start an LSP server"
            );
            let mut manifest = Vec::new();
            parse_manifest(&directory.join("plugin.toml"), &mut manifest).unwrap();
            let value = |key: &str| -> Option<&str> {
                manifest
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value.as_str())
            };
            assert_eq!(value("lazy"), Some("true"));
            assert_eq!(value("language"), Some(language));
            assert!(directory
                .join(value("entry").unwrap_or("init.lua"))
                .is_file());
        }
    }
}
