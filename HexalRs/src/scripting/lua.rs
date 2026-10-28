//! Embedded Lua 5.4 runtime and a small, buffer-oriented plugin API.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::alloc::{alloc as rust_alloc, dealloc as rust_dealloc, Layout};

const LUA_GRAIN: usize = 16;
const LUA_SMALL_LIMIT: usize = 1024;
const LUA_CLASSES: usize = LUA_SMALL_LIMIT / LUA_GRAIN;
const LUA_MAX_CACHED: u16 = 512;

struct FreeNode {
    next: *mut FreeNode,
}

/// Lua's allocator is extremely hot while `string.gmatch` creates short
/// strings. Keep freed small blocks in size-class lists, mirroring the Zig
/// host.
struct LuaAllocator {
    heads: [*mut FreeNode; LUA_CLASSES],
    counts: [u16; LUA_CLASSES],
}

impl LuaAllocator {
    fn new() -> Box<LuaAllocator> {
        Box::new(LuaAllocator {
            heads: [std::ptr::null_mut(); LUA_CLASSES],
            counts: [0; LUA_CLASSES],
        })
    }

    #[inline]
    fn class(size: usize) -> Option<usize> {
        if size == 0 || size > LUA_SMALL_LIMIT {
            None
        } else {
            Some((size - 1) / LUA_GRAIN)
        }
    }

    #[inline]
    unsafe fn alloc(&mut self, size: usize) -> *mut u8 {
        if let Some(class) = Self::class(size) {
            let head = self.heads[class];
            if !head.is_null() {
                self.heads[class] = unsafe { (*head).next };
                self.counts[class] -= 1;
                return head as *mut u8;
            }
            let block = (class + 1) * LUA_GRAIN;
            return unsafe { rust_alloc(Layout::from_size_align_unchecked(block, LUA_GRAIN)) };
        }
        unsafe { rust_alloc(Layout::from_size_align_unchecked(size, LUA_GRAIN)) }
    }

    #[inline]
    unsafe fn dealloc(&mut self, pointer: *mut u8, size: usize) {
        if let Some(class) = Self::class(size) {
            if self.counts[class] < LUA_MAX_CACHED {
                let node = pointer as *mut FreeNode;
                unsafe { (*node).next = self.heads[class] };
                self.heads[class] = node;
                self.counts[class] += 1;
                return;
            }
            let block = (class + 1) * LUA_GRAIN;
            unsafe { rust_dealloc(pointer, Layout::from_size_align_unchecked(block, LUA_GRAIN)) };
            return;
        }
        unsafe { rust_dealloc(pointer, Layout::from_size_align_unchecked(size, LUA_GRAIN)) };
    }

    unsafe fn realloc(&mut self, pointer: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
        if let (Some(old_class), Some(new_class)) = (Self::class(old_size), Self::class(new_size)) {
            if old_class == new_class {
                return pointer;
            }
        }
        let replacement = unsafe { self.alloc(new_size) };
        if !replacement.is_null() && !pointer.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(pointer, replacement, old_size.min(new_size));
            }
        }
        if !pointer.is_null() {
            unsafe { self.dealloc(pointer, old_size) };
        }
        replacement
    }
}

impl Drop for LuaAllocator {
    fn drop(&mut self) {
        for class in 0..LUA_CLASSES {
            let mut node = self.heads[class];
            while !node.is_null() {
                let next = unsafe { (*node).next };
                unsafe {
                    rust_dealloc(
                        node as *mut u8,
                        Layout::from_size_align_unchecked((class + 1) * LUA_GRAIN, LUA_GRAIN),
                    )
                };
                node = next;
            }
        }
    }
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::raw::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::slice;
use std::sync::Arc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use mlua::{Function, Lua, Result as LuaResult, Table, Value as LuaValue};

use crate::core::document::Document;
use crate::scripting::json::{self, Value};
use crate::scripting::lsp::{Coordinator, LspRequest};
use crate::util::json as hosted_json;

#[derive(Clone, Debug)]
pub struct PluginSelection {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub text: String,
    pub linewise: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct PanelSegment {
    pub text: String,
    pub style: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct PanelLine {
    /// Shared so cloning a panel (or its line list) never duplicates segment
    /// text. This is what keeps plugin-panel synchronization cheap while a
    /// large tree is open.
    pub segments: Arc<[PanelSegment]>,
    pub selectable: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct PluginPanel {
    pub id: i64,
    pub plugin_id: String,
    pub title: String,
    pub footer: String,
    pub selectable: bool,
    pub selected: usize,
    /// Cloning is O(1); mutations go through `Arc::make_mut` when the editor
    /// also holds a snapshot.
    pub lines: Arc<Vec<PanelLine>>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum HostRequest {
    Prompt {
        id: i64,
        label: String,
    },
    Select {
        id: i64,
        title: String,
        items: Vec<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct HostEffects {
    pub messages: Vec<(String, bool)>,
    pub statuses: Vec<(String, Option<String>)>,
    pub clipboard: Option<String>,
    pub lsp_requests: Vec<LspRequest>,
    pub lsp_stops: Vec<String>,
    pub errors: Vec<(String, String)>,
    pub panels_changed: bool,
}

/// Mutable editor state mirrored into the Lua host for one invocation.
pub struct HostState {
    pub text: Vec<u8>,
    text_view: Option<(*const u8, usize)>,
    pub line_starts: Vec<usize>,
    pub cursor_line: usize,
    pub cursor_column: usize,
    pub dirty: bool,
    pub selection: Option<PluginSelection>,
    pub buffer_id: String,
    pub path: Option<String>,
    pub language: String,
    pub workspace_root: Option<String>,
    pub columns: usize,
    pub rows: usize,
    pub messages: Vec<(String, bool)>,
    pub statuses: Vec<(String, Option<String>)>,
    pub clipboard: Option<String>,
    pub lsp_requests: Vec<LspRequest>,
    pub lsp_stops: Vec<String>,
    pub errors: Vec<(String, String)>,
    pub requests: Vec<HostRequest>,
    pub panels: Vec<PluginPanel>,
    pub next_panel_id: i64,
    pub pending_commands: Vec<(String, Vec<String>)>,
}

impl Default for HostState {
    fn default() -> Self {
        HostState {
            text: Vec::new(),
            text_view: None,
            line_starts: vec![0],
            cursor_line: 0,
            cursor_column: 0,
            dirty: false,
            selection: None,
            buffer_id: "[No Name]".into(),
            path: None,
            language: String::new(),
            workspace_root: None,
            columns: 80,
            rows: 24,
            messages: Vec::new(),
            statuses: Vec::new(),
            clipboard: None,
            lsp_requests: Vec::new(),
            lsp_stops: Vec::new(),
            errors: Vec::new(),
            requests: Vec::new(),
            panels: Vec::new(),
            next_panel_id: 1,
            pending_commands: Vec::new(),
        }
    }
}

impl HostState {
    pub fn load(&mut self, text: Vec<u8>, cursor: usize) {
        self.text = text;
        self.text_view = None;
        self.rebuild_lines();
        let line = self.line_of_offset(cursor);
        self.cursor_line = line;
        self.cursor_column = scalar_column(
            &self.line_bounds(line).unwrap_or((0, 0)),
            &self.text,
            cursor,
        );
        self.dirty = false;
    }

    pub fn load_with_view(
        &mut self,
        text: Vec<u8>,
        view: Option<(*const u8, usize)>,
        cursor: usize,
    ) {
        self.text = text;
        self.text_view = view;
        self.rebuild_lines();
        let line = self.line_of_offset(cursor);
        self.cursor_line = line;
        self.cursor_column = scalar_column(
            &self.line_bounds(line).unwrap_or((0, 0)),
            self.bytes(),
            cursor,
        );
        self.dirty = false;
    }

    pub fn bytes(&self) -> &[u8] {
        match self.text_view {
            Some((pointer, length)) => unsafe { slice::from_raw_parts(pointer, length) },
            None => &self.text,
        }
    }

    pub fn materialize(&mut self) {
        let Some((pointer, length)) = self.text_view.take() else {
            return;
        };
        let bytes = unsafe { slice::from_raw_parts(pointer, length) };
        self.text.clear();
        self.text.extend_from_slice(bytes);
    }

    pub fn rebuild_lines(&mut self) {
        let mut starts = vec![0usize];
        {
            let bytes = self.bytes();
            let mut position = 0usize;
            while position < bytes.len() {
                let found = unsafe {
                    libc::memchr(
                        bytes[position..].as_ptr() as *const c_void,
                        b'\n' as c_int,
                        bytes.len() - position,
                    )
                };
                if found.is_null() {
                    break;
                }
                let index = found as usize - bytes.as_ptr() as usize;
                starts.push(index + 1);
                position = index + 1;
            }
        }
        self.line_starts = starts;
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    pub fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let text = self.bytes();
        let start = *self.line_starts.get(line)?;
        if start > text.len() {
            return None;
        }
        if line + 1 >= self.line_starts.len() {
            return Some((start, text.len()));
        }
        let mut end = self.line_starts[line + 1].saturating_sub(1).min(text.len());
        if end < start {
            end = start;
        }
        if end > start && text[end - 1] == b'\r' {
            end -= 1;
        }
        Some((start, end))
    }

    fn lines_are_consistent(&self) -> bool {
        let length = self.bytes().len();
        if self.line_starts.first().copied() != Some(0) {
            return false;
        }
        let mut previous = 0usize;
        for &start in &self.line_starts {
            if start < previous || start > length {
                return false;
            }
            previous = start;
        }
        true
    }

    fn ensure_lines(&mut self) {
        if !self.lines_are_consistent() {
            self.rebuild_lines();
        }
    }

    pub fn line_bytes(&self, line: usize) -> Option<&[u8]> {
        let (start, end) = self.line_bounds(line)?;
        Some(&self.bytes()[start..end])
    }

    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1)
            .min(self.line_starts.len().saturating_sub(1))
    }

    pub fn cursor_byte(&self) -> usize {
        let (start, end) = self.line_bounds(self.cursor_line).unwrap_or((0, 0));
        let text = &self.bytes()[start..end];
        start + column_to_byte(text, self.cursor_column)
    }

    pub fn set_cursor(&mut self, line: usize, column: usize) {
        self.cursor_line = line.min(self.line_count().saturating_sub(1));
        let (start, end) = self.line_bounds(self.cursor_line).unwrap_or((0, 0));
        let text = &self.bytes()[start..end];
        self.cursor_column = column.min(scalar_len(text));
    }

    pub fn set_text(&mut self, value: &[u8]) {
        self.text_view = None;
        self.text.clear();
        self.text.extend_from_slice(value);
        self.rebuild_lines();
        self.cursor_line = self.cursor_line.min(self.line_count().saturating_sub(1));
        self.dirty = true;
    }

    pub fn set_line(&mut self, line: usize, value: &[u8]) -> bool {
        if line >= self.line_count() {
            return false;
        }
        self.materialize();
        let (start, end) = self.line_bounds(line).unwrap_or((0, 0));
        self.text.splice(start..end, value.iter().copied());
        self.rebuild_lines();
        self.dirty = true;
        true
    }

    pub fn insert(&mut self, value: &[u8]) {
        self.materialize();
        let offset = self.cursor_byte();
        self.text.splice(offset..offset, value.iter().copied());
        self.rebuild_lines();
        let new_offset = offset + value.len();
        self.cursor_line = self.line_of_offset(new_offset);
        self.cursor_column = scalar_column(
            &self.line_bounds(self.cursor_line).unwrap_or((0, 0)),
            &self.text,
            new_offset,
        );
        self.dirty = true;
    }

    pub fn replace_range(&mut self, start: usize, end: usize, value: &[u8]) -> bool {
        if end < start || end > self.bytes().len() {
            return false;
        }
        self.materialize();
        self.text.splice(start..end, value.iter().copied());
        self.rebuild_lines();
        let new_offset = start + value.len();
        self.cursor_line = self.line_of_offset(new_offset);
        self.cursor_column = scalar_column(
            &self.line_bounds(self.cursor_line).unwrap_or((0, 0)),
            &self.text,
            new_offset,
        );
        self.dirty = true;
        true
    }

    pub fn take_effects(&mut self) -> HostEffects {
        HostEffects {
            messages: std::mem::take(&mut self.messages),
            statuses: std::mem::take(&mut self.statuses),
            clipboard: self.clipboard.take(),
            lsp_requests: std::mem::take(&mut self.lsp_requests),
            lsp_stops: std::mem::take(&mut self.lsp_stops),
            errors: std::mem::take(&mut self.errors),
            panels_changed: false,
        }
    }
}

#[inline]
fn scalar_len(bytes: &[u8]) -> usize {
    if bytes.is_ascii() {
        bytes.len()
    } else {
        String::from_utf8_lossy(bytes).chars().count()
    }
}

#[inline]
fn column_to_byte(bytes: &[u8], column: usize) -> usize {
    if bytes.is_ascii() {
        return column.min(bytes.len());
    }
    String::from_utf8_lossy(bytes)
        .char_indices()
        .nth(column)
        .map(|(index, _)| index)
        .unwrap_or(bytes.len())
}

#[inline]
fn scalar_column(bounds: &(usize, usize), bytes: &[u8], offset: usize) -> usize {
    let start = bounds.0.min(bytes.len());
    let end = bounds.1.max(start).min(bytes.len());
    let offset = offset.clamp(start, end);
    if bytes[start..offset].is_ascii() {
        offset - start
    } else {
        String::from_utf8_lossy(&bytes[start..offset])
            .chars()
            .count()
    }
}

// ---------------------------------------------------------------------------
// Plugin configuration
// ---------------------------------------------------------------------------

pub struct PluginConfigStore {
    path: PathBuf,
    values: Value,
}

impl PluginConfigStore {
    pub fn new(path: PathBuf) -> PluginConfigStore {
        PluginConfigStore {
            path,
            values: Value::Object(Vec::new()),
        }
    }

    pub fn load(&mut self) -> Result<(), String> {
        if !self.path.exists() {
            return Ok(());
        }
        let text = fs::read_to_string(&self.path).map_err(|error| error.to_string())?;
        let value = hosted_json::parse(&text)?;
        if !matches!(value, Value::Object(_)) {
            return Err("configuration root must be a JSON object".into());
        }
        self.values = value;
        Ok(())
    }

    pub fn get(&self, key: &str, fallback: Value) -> Value {
        if let Some(value) = self.values.get(key) {
            return value.clone();
        }
        let mut current = &self.values;
        for part in key.split('.').filter(|part| !part.is_empty()) {
            let Some(value) = current.get(part) else {
                return fallback;
            };
            current = value;
        }
        current.clone()
    }

    pub fn set(&mut self, key: &str, value: Value) {
        if key.is_empty() {
            return;
        }
        let parts: Vec<&str> = key.split('.').filter(|part| !part.is_empty()).collect();
        if parts.len() <= 1 {
            self.values.set(key, value);
            return;
        }
        let mut current = &mut self.values;
        for part in &parts[..parts.len() - 1] {
            if current.get(part).is_none() {
                current.set(part, Value::Object(Vec::new()));
            }
            let Value::Object(entries) = current else {
                return;
            };
            let Some((_, child)) = entries.iter_mut().find(|(name, _)| name == part) else {
                return;
            };
            current = child;
        }
        current.set(parts[parts.len() - 1], value);
    }

    pub fn snapshot(&self) -> Value {
        self.values.clone()
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
        let mut output = hosted_json::to_string(&self.values, true);
        output.push('\n');
        fs::write(&self.path, output).map_err(|error| error.to_string())
    }
}

// ---------------------------------------------------------------------------
// Script host
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CommandEntry {
    pub name: String,
    pub description: String,
    pub callback: Option<Function>,
    pub raw_reference: Option<i32>,
}

#[derive(Clone)]
pub struct KeyBinding {
    pub plugin_id: String,
    pub mode: String,
    pub keys: Vec<String>,
    pub command: Option<String>,
    pub callback: Option<Function>,
}

#[derive(Clone)]
pub struct Subscription {
    pub id: i64,
    pub event: String,
    pub callback: Function,
}

pub struct DeferredTask {
    pub due: Instant,
    pub callback: Function,
}

pub struct AsyncJob {
    pub receiver: mpsc::Receiver<Vec<String>>,
    pub callback: Function,
}

#[allow(dead_code)]
pub enum PromptCallback {
    Prompt(Function),
    Select(Function),
}

pub struct RuntimeShared {
    pub plugin_id: String,
    pub commands: HashMap<String, CommandEntry>,
    pub command_version: u64,
    pub keybindings: Vec<KeyBinding>,
    pub subscriptions: Vec<Subscription>,
    pub next_subscription: i64,
    pub next_request: i64,
    pub prompts: HashMap<i64, PromptCallback>,
    pub panel_handlers: HashMap<i64, Function>,
    pub deferred: Vec<DeferredTask>,
    pub async_jobs: Vec<AsyncJob>,
    pub config: PluginConfigStore,
    pub lsp: Rc<RefCell<Coordinator>>,
}

pub struct RawCtx {
    host: Rc<RefCell<HostState>>,
    shared: Rc<RefCell<RuntimeShared>>,
}

pub struct LuaRuntime {
    lua: &'static Lua,
    state: *mut mlua::ffi::lua_State,
    _allocator: Box<LuaAllocator>,
    pub host: Rc<RefCell<HostState>>,
    pub shared: Rc<RefCell<RuntimeShared>>,
    pub plugin_id: String,
    loaded_revision: Option<u64>,
    loaded_document_id: Option<u64>,
    synced_editor_cursor: Option<usize>,
    last_command: Option<(String, u64, CommandEntry)>,
    _raw: Box<RawCtx>,
}

impl LuaRuntime {
    pub fn new(
        plugin_id: impl Into<String>,
        plugin_dir: impl Into<String>,
        host: Rc<RefCell<HostState>>,
    ) -> Result<LuaRuntime, String> {
        let plugin_id = plugin_id.into();
        let plugin_dir = plugin_dir.into();
        let mut allocator = LuaAllocator::new();
        let state = unsafe {
            mlua::ffi::lua_newstate(
                lua_alloc,
                (&mut *allocator) as *mut LuaAllocator as *mut c_void,
            )
        };
        if state.is_null() {
            return Err("cannot create Lua state".into());
        }
        let lua: &'static Lua = unsafe { Lua::get_or_init_from_ptr(state) };
        unsafe {
            mlua::ffi::luaL_requiref(state, c"_G".as_ptr(), mlua::ffi::luaopen_base, 1);
            mlua::ffi::lua_pop(state, 1);
        }
        if let Err(error) = lua.load_std_libs(mlua::StdLib::ALL_SAFE) {
            unsafe { mlua::ffi::lua_close(state) };
            return Err(error.to_string());
        }

        let root = lua.create_table().map_err(|error| error.to_string())?;
        lua.globals()
            .set("hexal", root.clone())
            .map_err(|error| error.to_string())?;
        lua.load(PRELUDE)
            .set_name("hexal-prelude")
            .exec()
            .map_err(|error| error.to_string())?;

        let coordinator = Coordinator::new(lua, plugin_id.clone(), plugin_dir.clone());
        let config_path = crate::core::config::hexal_home_directory()
            .join("plugin-config")
            .join(format!("{}.json", sanitize_file_name(&plugin_id)));
        let mut config = PluginConfigStore::new(config_path);
        let config_error = config.load().err().unwrap_or_default();

        let shared = Rc::new(RefCell::new(RuntimeShared {
            plugin_id: plugin_id.clone(),
            commands: HashMap::new(),
            command_version: 0,
            keybindings: Vec::new(),
            subscriptions: Vec::new(),
            next_subscription: 1,
            next_request: 1,
            prompts: HashMap::new(),
            panel_handlers: HashMap::new(),
            deferred: Vec::new(),
            async_jobs: Vec::new(),
            config,
            lsp: coordinator.clone(),
        }));

        let raw = Box::new(RawCtx {
            host: host.clone(),
            shared: shared.clone(),
        });
        register_host_api(lua, &root, &host, &shared, &coordinator, &*raw)
            .map_err(|error| error.to_string())?;
        crate::scripting::lsp::register(lua, &root, coordinator.clone())
            .map_err(|error| error.to_string())?;
        register_async_fs(lua, &root, coordinator.clone(), &shared)
            .map_err(|error| error.to_string())?;
        augment_lsp_modules(lua, &root, &shared, &coordinator)
            .map_err(|error| error.to_string())?;

        let escaped = plugin_dir.replace('\\', "/").replace('\'', "\\'");
        lua.load(format!(
            "package.path = '{escaped}/?.lua;{escaped}/?/init.lua;' .. package.path"
        ))
        .set_name("hexal-package-path")
        .exec()
        .map_err(|error| error.to_string())?;

        if !config_error.is_empty() {
            host.borrow_mut()
                .errors
                .push((plugin_id.clone(), format!("config: {config_error}")));
        }

        Ok(LuaRuntime {
            lua,
            state,
            _allocator: allocator,
            host,
            shared,
            plugin_id,
            loaded_revision: None,
            loaded_document_id: None,
            synced_editor_cursor: None,
            last_command: None,
            _raw: raw,
        })
    }

    pub fn load_file(&mut self, path: &str) -> Result<(), String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("cannot read plugin '{path}': {error}"))?;
        self.lua
            .load(source)
            .set_name(path)
            .exec()
            .map_err(|error| error.to_string())
    }

    pub fn invoke(
        &mut self,
        name: &str,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<i64, String> {
        let Some(command) = self.cached_command(name) else {
            return Err(format!("unknown Lua command '{name}'"));
        };
        self.sync_from_document(document, *cursor);
        let checksum = if let Some(reference) = command.raw_reference {
            self.call_raw_reference(reference, &[])
                .map(|value| value as i64)?
        } else {
            let result = match command.callback {
                Some(callback) => callback.call::<LuaValue>(mlua::MultiValue::new()),
                None => return Ok(0),
            };
            match result {
                Ok(LuaValue::Integer(value)) => value,
                Ok(LuaValue::Number(value)) => value as i64,
                Ok(LuaValue::Boolean(value)) => i64::from(value),
                Ok(_) => 0,
                Err(error) => return Err(error.to_string()),
            }
        };
        let _effects = self.apply_to_document(document, cursor);
        Ok(checksum)
    }

    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .shared
            .borrow()
            .commands
            .values()
            .map(|command| command.name.clone())
            .collect();
        names.sort();
        names
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.find_command(name).is_some()
    }

    pub fn find_command(&self, name: &str) -> Option<CommandEntry> {
        let shared = self.shared.borrow();
        shared
            .commands
            .get(&name.to_lowercase())
            .cloned()
            .or_else(|| {
                shared
                    .commands
                    .values()
                    .find(|command| command.name.eq_ignore_ascii_case(name))
                    .cloned()
            })
    }

    fn cached_command(&mut self, name: &str) -> Option<CommandEntry> {
        let version = self.shared.borrow().command_version;
        if let Some((cached_name, cached_version, command)) = &self.last_command {
            if cached_name == name && *cached_version == version {
                return Some(command.clone());
            }
        }
        let command = self.find_command(name)?;
        self.last_command = Some((name.to_string(), version, command.clone()));
        Some(command)
    }

    pub fn call_command(
        &mut self,
        name: &str,
        args: &[String],
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(bool, HostEffects), String> {
        let Some(command) = self.cached_command(name) else {
            return Ok((false, HostEffects::default()));
        };
        self.sync_from_document(document, *cursor);
        if let Some(reference) = command.raw_reference {
            let result = self.call_raw_reference(reference, args);
            let effects = self.apply_to_document(document, cursor);
            result?;
            return Ok((true, effects));
        }
        let values: Vec<LuaValue> = args
            .iter()
            .filter_map(|argument| self.lua.create_string(argument).ok().map(LuaValue::String))
            .collect();
        let result = match command.callback {
            Some(callback) => callback
                .call::<LuaValue>(mlua::MultiValue::from_vec(values))
                .map(|_| ())
                .map_err(|error| error.to_string()),
            None => return Ok((false, HostEffects::default())),
        };
        let effects = self.apply_to_document(document, cursor);
        if let Err(error) = result {
            return Err(error);
        }
        Ok((true, effects))
    }

    pub fn call_key_callback(
        &mut self,
        callback: &Function,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<(bool, HostEffects), String> {
        self.sync_from_document(document, *cursor);
        let result = callback.call::<LuaValue>(());
        let effects = self.apply_to_document(document, cursor);
        match result {
            Ok(LuaValue::Boolean(false)) => Ok((false, effects)),
            Ok(_) => Ok((true, effects)),
            Err(error) => Err(error.to_string()),
        }
    }

    fn call_raw_reference(
        &self,
        reference: i32,
        args: &[String],
    ) -> Result<mlua::ffi::lua_Integer, String> {
        unsafe { call_raw_on_state(self.lua.state(), reference, args) }
    }

    pub fn fire_event(
        &mut self,
        event: &str,
        data: &Value,
        document: &mut Document,
        cursor: &mut usize,
    ) {
        let subscriptions: Vec<Function> = self
            .shared
            .borrow()
            .subscriptions
            .iter()
            .filter(|subscription| subscription.event.eq_ignore_ascii_case(event))
            .map(|subscription| subscription.callback.clone())
            .collect();
        if subscriptions.is_empty() {
            return;
        }
        self.sync_from_document(document, *cursor);
        let null = self.shared.borrow().lsp.borrow().null.clone();
        let Ok(value) = json::to_lua(self.lua, data, &null) else {
            return;
        };
        for callback in subscriptions {
            if let Err(error) = callback.call::<()>(value.clone()) {
                self.host.borrow_mut().errors.push((
                    self.plugin_id.clone(),
                    format!("event '{event}' failed: {error}"),
                ));
            }
        }
    }

    pub fn set_workspace_root(&mut self, root: &str) {
        let coordinator = self.shared.borrow().lsp.clone();
        coordinator.borrow_mut().set_workspace_root(root);
    }

    pub fn sync_from_document(&mut self, document: &Document, cursor: usize) {
        if self.host.borrow().dirty {
            return;
        }
        let revision = document.revision();
        if self.loaded_document_id != Some(document.id()) || self.loaded_revision != Some(revision)
        {
            if let Some((pointer, length)) = document.borrowed_text() {
                self.host
                    .borrow_mut()
                    .load_with_view(Vec::new(), Some((pointer, length)), cursor);
            } else {
                self.host.borrow_mut().load(document.all_text(), cursor);
            }
            self.loaded_revision = Some(revision);
            self.loaded_document_id = Some(document.id());
            self.synced_editor_cursor = Some(cursor);
            return;
        }
        if self.synced_editor_cursor != Some(cursor) {
            let line = document.line_of_offset(cursor);
            let line_start = document.line_start(line);
            let before = document.range(line_start, cursor.saturating_sub(line_start));
            let column = String::from_utf8_lossy(&before).chars().count();
            let mut host = self.host.borrow_mut();
            host.cursor_line = line;
            host.cursor_column = column;
            self.synced_editor_cursor = Some(cursor);
        }
    }

    pub fn call_panel_key(
        &mut self,
        panel_id: i64,
        key: &str,
        selected: usize,
        document: &mut Document,
        cursor: &mut usize,
    ) -> Result<Option<(bool, HostEffects)>, String> {
        let callback = self
            .shared
            .borrow()
            .panel_handlers
            .get(&panel_id)
            .cloned();
        let Some(callback) = callback else {
            return Ok(None);
        };
        self.sync_from_document(document, *cursor);
        let result = callback.call::<LuaValue>((key.to_string(), selected as i64 + 1));
        let effects = self.apply_to_document(document, cursor);
        match result {
            Ok(value) => Ok(Some((matches!(value, LuaValue::Boolean(true)), effects))),
            Err(error) => {
                self.host.borrow_mut().errors.push((
                    self.plugin_id.clone(),
                    format!("panel key handler failed: {error}"),
                ));
                Ok(Some((false, effects)))
            }
        }
    }

    pub fn has_due_deferred_tasks(&self) -> bool {
        let now = Instant::now();
        self.shared
            .borrow()
            .deferred
            .iter()
            .any(|task| task.due <= now)
    }

    /// Runs callbacks that plugins scheduled with `hexal.ui.defer`, plus any
    /// completed asynchronous filesystem scans. Callers are expected to do
    /// this from the UI loop (after a frame has been painted) so long-running
    /// work is split across frames instead of blocking the first one.
    pub fn run_deferred(
        &mut self,
        document: &mut Document,
        cursor: &mut usize,
    ) -> (bool, HostEffects) {
        let completed_jobs: Vec<(Function, Vec<String>)> = {
            let mut shared = self.shared.borrow_mut();
            if shared.async_jobs.is_empty() {
                Vec::new()
            } else {
                let mut completed = Vec::new();
                let mut remaining = Vec::with_capacity(shared.async_jobs.len());
                for job in shared.async_jobs.drain(..) {
                    match job.receiver.try_recv() {
                        Ok(files) => completed.push((job.callback, files)),
                        Err(TryRecvError::Empty) => remaining.push(job),
                        Err(TryRecvError::Disconnected) => {
                            completed.push((job.callback, Vec::new()));
                        }
                    }
                }
                shared.async_jobs = remaining;
                completed
            }
        };

        let due_tasks: Vec<Function> = {
            let mut shared = self.shared.borrow_mut();
            if shared.deferred.is_empty() {
                Vec::new()
            } else {
                let now = Instant::now();
                let mut due = Vec::new();
                let mut remaining = Vec::with_capacity(shared.deferred.len());
                for task in shared.deferred.drain(..) {
                    if task.due <= now {
                        due.push(task.callback);
                    } else {
                        remaining.push(task);
                    }
                }
                shared.deferred = remaining;
                due
            }
        };

        if completed_jobs.is_empty() && due_tasks.is_empty() {
            return (false, HostEffects::default());
        }

        self.sync_from_document(document, *cursor);
        let coordinator = self.shared.borrow().lsp.clone();
        for (callback, files) in completed_jobs {
            let value = Value::Array(files.into_iter().map(Value::String).collect());
            let result = match to_lua(self.lua, &coordinator, &value) {
                Ok(value) => callback.call::<()>(value),
                Err(error) => {
                    self.host.borrow_mut().errors.push((
                        self.plugin_id.clone(),
                        format!("async callback failed: {error}"),
                    ));
                    continue;
                }
            };
            if let Err(error) = result {
                self.host.borrow_mut().errors.push((
                    self.plugin_id.clone(),
                    format!("async callback failed: {error}"),
                ));
            }
        }
        for callback in due_tasks {
            if let Err(error) = callback.call::<()>(()) {
                self.host.borrow_mut().errors.push((
                    self.plugin_id.clone(),
                    format!("deferred callback failed: {error}"),
                ));
            }
        }
        (true, self.apply_to_document(document, cursor))
    }

    pub fn apply_to_document(
        &mut self,
        document: &mut Document,
        cursor: &mut usize,
    ) -> HostEffects {
        let (dirty, data, cursor_byte) = {
            let host = self.host.borrow();
            (
                host.dirty,
                if host.dirty {
                    Some(host.bytes().to_vec())
                } else {
                    None
                },
                {
                    let (start, end) = host.line_bounds(host.cursor_line).unwrap_or((0, 0));
                    let bytes = host.bytes();
                    start + column_to_byte(&bytes[start..end.min(bytes.len())], host.cursor_column)
                },
            )
        };
        if dirty {
            let data = data.unwrap_or_default();
            document.replace_all_owned(data);
            document.break_undo_group();
        }
        *cursor = cursor_byte.min(document.size());
        self.loaded_revision = Some(document.revision());
        self.loaded_document_id = Some(document.id());
        self.synced_editor_cursor = Some(*cursor);
        let mut host = self.host.borrow_mut();
        host.dirty = false;
        host.text_view = document.borrowed_text();
        if host.text_view.is_none() && host.text.is_empty() && document.size() > 0 {
            host.load(document.all_text(), *cursor);
        } else {
            host.ensure_lines();
        }
        host.take_effects()
    }

    pub fn drain_requests(&self) -> (Vec<LspRequest>, Vec<String>) {
        let lsp = self.shared.borrow().lsp.clone();
        let mut state = lsp.borrow_mut();
        (
            std::mem::take(&mut state.external_requests),
            std::mem::take(&mut state.external_stops),
        )
    }

    #[allow(dead_code)]
    pub fn resolve_prompt(&self, id: i64, value: Option<&str>) -> bool {
        let callback = self.shared.borrow_mut().prompts.remove(&id);
        let Some(PromptCallback::Prompt(callback)) = callback else {
            return false;
        };
        let _ = callback.call::<()>(match value {
            Some(value) => match self.lua.create_string(value) {
                Ok(value) => LuaValue::String(value),
                Err(_) => LuaValue::Nil,
            },
            None => LuaValue::Nil,
        });
        true
    }

    #[allow(dead_code)]
    pub fn resolve_select(&self, id: i64, index: Option<usize>) -> bool {
        let callback = self.shared.borrow_mut().prompts.remove(&id);
        let Some(PromptCallback::Select(callback)) = callback else {
            return false;
        };
        let _ = callback.call::<()>(match index {
            Some(index) => LuaValue::Integer(index as i64 + 1),
            None => LuaValue::Nil,
        });
        true
    }

    pub fn keybindings(&self) -> Vec<KeyBinding> {
        self.shared.borrow().keybindings.clone()
    }
}

impl Drop for LuaRuntime {
    fn drop(&mut self) {
        // Release every Lua handle while the state is still alive. mlua's
        // handles unref their registry entries on drop, which must never
        // happen after `lua_close`.
        if let Ok(mut shared) = self.shared.try_borrow_mut() {
            shared.commands.clear();
            shared.keybindings.clear();
            shared.subscriptions.clear();
            shared.prompts.clear();
            shared.panel_handlers.clear();
            if let Ok(mut coordinator) = shared.lsp.try_borrow_mut() {
                coordinator.null = LuaValue::Nil;
                coordinator.modules.clear();
                coordinator.servers.clear();
                coordinator.diagnostic_handlers.clear();
            }
        }
        unsafe {
            mlua::ffi::lua_close(self.state);
        }
    }
}

const PRELUDE: &str = r##"
(function()
    __hexal_wrap_methods = function(t)
        if type(t) ~= "table" or rawget(t, "__hexal_wrapped") then
            return t
        end
        rawset(t, "__hexal_wrapped", true)
        local functions = {}
        for k, v in pairs(t) do
            if type(v) == "function" then
                functions[k] = v
            end
        end
        for k, f in pairs(functions) do
            rawset(t, k, function(...)
                local n = select("#", ...)
                local args = table.pack(...)
                local first = 1
                if n > 0 and args[1] == t then
                    first = 2
                end
                return f(table.unpack(args, first, n))
            end)
        end
        return t
    end
end)()
"##;

fn sanitize_file_name(id: &str) -> String {
    let value: String = id
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character => character,
        })
        .collect();
    if value.is_empty() {
        "plugin".into()
    } else {
        value
    }
}

// ---------------------------------------------------------------------------
// Core API
// ---------------------------------------------------------------------------

fn from_lua(coordinator: &Rc<RefCell<Coordinator>>, value: &LuaValue) -> Value {
    let null = coordinator.borrow().null.clone();
    json::from_lua(value, &null)
}

fn from_table(coordinator: &Rc<RefCell<Coordinator>>, table: &Table) -> Value {
    from_lua(coordinator, &LuaValue::Table(table.clone()))
}

fn to_lua(lua: &Lua, coordinator: &Rc<RefCell<Coordinator>>, value: &Value) -> LuaResult<LuaValue> {
    let null = coordinator.borrow().null.clone();
    json::to_lua(lua, value, &null)
}

fn set_fn<A, R>(
    lua: &Lua,
    table: &Table,
    name: &str,
    function: impl Fn(&Lua, A) -> LuaResult<R> + 'static,
) -> LuaResult<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    table.set(name, lua.create_function(function)?)
}

fn string_list(value: &Value) -> Vec<String> {
    json::as_array(value)
        .iter()
        .map(|value| match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => String::new(),
        })
        .collect()
}

fn arguments(values: &[LuaValue]) -> Vec<String> {
    values
        .iter()
        .map(|value| match value {
            LuaValue::String(text) => text.to_string_lossy().to_string(),
            LuaValue::Integer(number) => number.to_string(),
            LuaValue::Number(number) => number.to_string(),
            LuaValue::Boolean(flag) => flag.to_string(),
            _ => String::new(),
        })
        .collect()
}

fn register_async_fs(
    lua: &Lua,
    root: &Table,
    coordinator: Rc<RefCell<Coordinator>>,
    shared: &Rc<RefCell<RuntimeShared>>,
) -> LuaResult<()> {
    let fs = match root.get::<LuaValue>("fs")? {
        LuaValue::Table(fs) => fs,
        _ => return Ok(()),
    };
    let shared = shared.clone();
    set_fn(
        lua,
        &fs,
        "workspace_files_async",
        move |_, (options, callback): (Option<LuaValue>, Function)| {
            let root = coordinator.borrow().workspace.root_path.clone();
            let mut max_results = 20_000usize;
            if let Some(LuaValue::Table(table)) = options {
                if let Ok(Some(limit)) = table.get::<Option<usize>>("max_results") {
                    max_results = limit.max(1);
                } else if let Ok(Some(limit)) = table.get::<Option<usize>>("maxResults") {
                    max_results = limit.max(1);
                }
            }
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let files = crate::scripting::lsp::collect_workspace_files(&root, max_results);
                let _ = sender.send(files);
            });
            shared
                .borrow_mut()
                .async_jobs
                .push(AsyncJob { receiver, callback });
            Ok(true)
        },
    )?;
    Ok(())
}

fn register_host_api(
    lua: &Lua,
    root: &Table,
    host: &Rc<RefCell<HostState>>,
    shared: &Rc<RefCell<RuntimeShared>>,
    coordinator: &Rc<RefCell<Coordinator>>,
    raw: &RawCtx,
) -> LuaResult<()> {
    root.set("api_version", 1)?;

    let editor = module(lua, root, "editor")?;
    {
        let host = host.clone();
        set_fn(lua, &editor, "get_current_buffer", move |_, ()| {
            Ok(host.borrow().buffer_id.clone())
        })?;
    }
    {
        let host = host.clone();
        set_fn(lua, &editor, "get_workspace_root", move |_, ()| {
            Ok(host.borrow().workspace_root.clone())
        })?;
    }
    {
        let host = host.clone();
        set_fn(lua, &editor, "get_text", move |lua, ()| {
            let text = String::from_utf8_lossy(host.borrow().bytes()).into_owned();
            Ok(LuaValue::String(lua.create_string(text)?))
        })?;
    }
    {
        let host = host.clone();
        set_fn(lua, &editor, "get_line", move |lua, line: i64| {
            if line < 0 {
                host.borrow_mut()
                    .messages
                    .push((format!("line {line} is out of range"), true));
                return Ok(LuaValue::Nil);
            }
            let text = {
                let host = host.borrow();
                host.line_bytes(line as usize)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            };
            match text {
                Some(text) => Ok(LuaValue::String(lua.create_string(text)?)),
                None => {
                    host.borrow_mut()
                        .messages
                        .push((format!("line {line} is out of range"), true));
                    Ok(LuaValue::Nil)
                }
            }
        })?;
    }
    {
        let host = host.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &editor, "get_cursor", move |lua, ()| {
            let (line, column) = {
                let host = host.borrow();
                (host.cursor_line, host.cursor_column)
            };
            let mut value = Value::Object(Vec::new());
            value.set("line", Value::Number(line as f64));
            value.set("column", Value::Number(column as f64));
            to_lua(lua, &coordinator, &value)
        })?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &editor,
            "set_cursor",
            move |_, (line, column): (i64, i64)| {
                if line < 0 || column < 0 {
                    host.borrow_mut()
                        .messages
                        .push(("line and column must not be negative".into(), true));
                    return Ok(false);
                }
                let mut host = host.borrow_mut();
                if line as usize >= host.line_count() {
                    let message = format!("line {line} is out of range");
                    host.messages.push((message, true));
                    return Ok(false);
                }
                host.set_cursor(line as usize, column as usize);
                Ok(true)
            },
        )?;
    }
    {
        let host = host.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &editor, "get_selection", move |lua, ()| {
            let selection = host.borrow().selection.clone();
            let Some(selection) = selection else {
                return Ok(LuaValue::Nil);
            };
            let mut value = Value::Object(Vec::new());
            value.set("start_line", Value::Number(selection.start_line as f64));
            value.set("start_column", Value::Number(selection.start_column as f64));
            value.set("end_line", Value::Number(selection.end_line as f64));
            value.set("end_column", Value::Number(selection.end_column as f64));
            value.set("text", Value::String(selection.text));
            value.set("linewise", Value::Bool(selection.linewise));
            to_lua(lua, &coordinator, &value)
        })?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &editor,
            "insert_text",
            move |_, text: Option<String>| {
                if let Some(text) = text {
                    host.borrow_mut().insert(text.as_bytes());
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &editor, "replace_range", move |lua, (start, finish, text): (Option<LuaValue>, Option<LuaValue>, Option<String>)| {
            let to_offset = |value: Option<LuaValue>| -> Option<usize> {
                match value {
                    Some(LuaValue::Integer(value)) if value >= 0 => Some(value as usize),
                    Some(LuaValue::Number(value)) if value >= 0.0 => Some(value as usize),
                    Some(LuaValue::Table(table)) => {
                        let value = from_table(&coordinator, &table);
                        let line = value.get("line").and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
                        let column = value
                            .get("column")
                            .or_else(|| value.get("character"))
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            .max(0) as usize;
                        let host = host.borrow();
                        let (line_start, line_end) = host.line_bounds(line)?;
                        let bytes = host.bytes();
                        Some(line_start + column_to_byte(&bytes[line_start..line_end], column))
                    }
                    _ => None,
                }
            };
            let Some(start) = to_offset(start) else {
                host.borrow_mut().messages.push(("invalid range".into(), true));
                return Ok(false);
            };
            let Some(end) = to_offset(finish) else {
                host.borrow_mut().messages.push(("invalid range".into(), true));
                return Ok(false);
            };
            let ok = host.borrow_mut().replace_range(start, end, text.unwrap_or_default().as_bytes());
            if !ok {
                host.borrow_mut().messages.push(("range is outside the document".into(), true));
            }
            let _ = lua;
            Ok(ok)
        })?;
    }

    let commands = module(lua, root, "commands")?;
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &commands,
            "register",
            move |_, (name, callback, description): (String, Option<Function>, Option<String>)| {
                let Some(callback) = callback else {
                    return Ok(false);
                };
                if name.trim().is_empty() {
                    return Ok(false);
                }
                {
                    let mut shared = shared.borrow_mut();
                    shared.commands.insert(
                        name.to_lowercase(),
                        CommandEntry {
                            name,
                            description: description.unwrap_or_default(),
                            callback: Some(callback),
                            raw_reference: None,
                        },
                    );
                    shared.command_version = shared.command_version.wrapping_add(1);
                }
                Ok(true)
            },
        )?;
    }
    {
        let shared = shared.clone();
        set_fn(lua, &commands, "unregister", move |_, name: String| {
            let mut shared = shared.borrow_mut();
            shared.command_version = shared.command_version.wrapping_add(1);
            Ok(shared.commands.remove(&name.to_lowercase()).is_some())
        })?;
    }
    {
        let shared = shared.clone();
        let host = host.clone();
        set_fn(
            lua,
            &commands,
            "execute",
            move |lua, (name, args): (String, mlua::Variadic<LuaValue>)| {
                let entries = arguments(args.iter().cloned().collect::<Vec<_>>().as_slice());
                let command = shared.borrow().commands.get(&name.to_lowercase()).cloned();
                if let Some(command) = command {
                    if let Some(reference) = command.raw_reference {
                        let state = lua.state();
                        let result = unsafe { call_raw_on_state(state, reference, &entries) };
                        if let Err(error) = result {
                            host.borrow_mut()
                                .errors
                                .push((shared.borrow().plugin_id.clone(), error));
                            return Ok(false);
                        }
                        return Ok(true);
                    }
                    if let Some(callback) = command.callback {
                        let values: Vec<LuaValue> = entries
                            .iter()
                            .filter_map(|argument| {
                                lua.create_string(argument).ok().map(LuaValue::String)
                            })
                            .collect();
                        return match callback.call::<LuaValue>(mlua::MultiValue::from_vec(values)) {
                            Ok(_) => Ok(true),
                            Err(error) => {
                                host.borrow_mut()
                                    .errors
                                    .push((shared.borrow().plugin_id.clone(), error.to_string()));
                                Ok(false)
                            }
                        };
                    }
                }
                host.borrow_mut().pending_commands.push((name, entries));
                Ok(true)
            },
        )?;
    }

    let keymap = module(lua, root, "keymap")?;
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &keymap,
            "map",
            move |_, (keys, command, mode): (String, Option<String>, Option<String>)| {
                let Some(command) = command else {
                    return Ok(false);
                };
                add_binding(&shared, &keys, Some(command), None, mode.as_deref())
            },
        )?;
    }
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &keymap,
            "map_callback",
            move |_, (keys, callback, mode): (String, Option<Function>, Option<String>)| {
                let Some(callback) = callback else {
                    return Ok(false);
                };
                add_binding(&shared, &keys, None, Some(callback), mode.as_deref())
            },
        )?;
    }
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &keymap,
            "unmap",
            move |_, (keys, mode): (String, Option<String>)| {
                let keys = normalize_keys(&keys);
                if keys.is_empty() {
                    return Ok(false);
                }
                let mode = mode.unwrap_or_else(|| "normal".into());
                let mut shared = shared.borrow_mut();
                let before = shared.keybindings.len();
                shared.keybindings.retain(|binding| {
                    !(binding.mode.eq_ignore_ascii_case(&mode) && binding.keys == keys)
                });
                Ok(shared.keybindings.len() != before)
            },
        )?;
    }

    let ui = module(lua, root, "ui")?;
    {
        let host = host.clone();
        set_fn(lua, &ui, "message", move |_, text: Option<String>| {
            host.borrow_mut()
                .messages
                .push((text.unwrap_or_default(), false));
            Ok(())
        })?;
    }
    {
        let host = host.clone();
        set_fn(lua, &ui, "warn", move |_, text: Option<String>| {
            host.borrow_mut()
                .messages
                .push((text.unwrap_or_default(), true));
            Ok(())
        })?;
    }
    {
        let host = host.clone();
        set_fn(lua, &ui, "copy", move |_, text: Option<String>| {
            host.borrow_mut().clipboard = Some(text.unwrap_or_default());
            Ok(true)
        })?;
    }
    {
        let host = host.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &ui, "terminal_size", move |lua, ()| {
            let mut value = Value::Object(Vec::new());
            let host = host.borrow();
            value.set("columns", Value::Number(host.columns as f64));
            value.set("rows", Value::Number(host.rows as f64));
            to_lua(lua, &coordinator, &value)
        })?;
    }
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &ui,
            "defer",
            move |_, (callback, delay_ms): (Function, Option<i64>)| {
                let delay = delay_ms.unwrap_or(0).max(0) as u64;
                shared.borrow_mut().deferred.push(DeferredTask {
                    due: Instant::now() + Duration::from_millis(delay),
                    callback,
                });
                Ok(true)
            },
        )?;
    }
    {
        let host = host.clone();
        let shared = shared.clone();
        set_fn(
            lua,
            &ui,
            "prompt",
            move |_, (label, callback): (String, Function)| {
                let id = next_request(&shared);
                shared
                    .borrow_mut()
                    .prompts
                    .insert(id, PromptCallback::Prompt(callback));
                host.borrow_mut()
                    .requests
                    .push(HostRequest::Prompt { id, label });
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        let shared = shared.clone();
        set_fn(
            lua,
            &ui,
            "select",
            move |_, (title, items, callback): (String, Option<LuaValue>, Function)| {
                let items = items
                    .as_ref()
                    .map(|value| string_list(&from_lua(&shared.borrow().lsp, value)))
                    .unwrap_or_default();
                if items.is_empty() {
                    let _ = callback.call::<()>(LuaValue::Nil);
                    return Ok(());
                }
                let id = next_request(&shared);
                shared
                    .borrow_mut()
                    .prompts
                    .insert(id, PromptCallback::Select(callback));
                host.borrow_mut()
                    .requests
                    .push(HostRequest::Select { id, title, items });
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &ui,
            "set_status",
            move |_, (key, text): (String, Option<String>)| {
                if key.is_empty() {
                    return Ok(false);
                }
                let mut host = host.borrow_mut();
                if let Some(existing) = host.statuses.iter_mut().find(|(name, _)| *name == key) {
                    existing.1 = text;
                } else {
                    host.statuses.push((key, text));
                }
                Ok(true)
            },
        )?;
    }
    {
        let host = host.clone();
        let shared = shared.clone();
        set_fn(
            lua,
            &ui,
            "panel",
            move |lua, (options, on_key): (Option<Table>, Option<Function>)| {
                let options = match options {
                    Some(options) => options,
                    None => lua.create_table()?,
                };
                let title = match options.raw_get::<LuaValue>("title") {
                    Ok(LuaValue::String(title)) => title.to_string_lossy(),
                    _ => "Plugin".into(),
                };
                let footer = match options.raw_get::<LuaValue>("footer") {
                    Ok(LuaValue::String(footer)) => footer.to_string_lossy(),
                    _ => String::new(),
                };
                let selectable = match options.raw_get::<LuaValue>("selectable") {
                    Ok(LuaValue::Boolean(selectable)) => selectable,
                    _ => true,
                };
                let selected = match options.raw_get::<LuaValue>("selected") {
                    Ok(LuaValue::Integer(selected)) => selected,
                    Ok(LuaValue::Number(selected)) => selected as i64,
                    _ => 1,
                }
                .max(1) as usize
                    - 1;
                // Parse the line table directly instead of round-tripping
                // through the generic JSON value model. Large panels (the
                // project tree can hold tens of thousands of rows) dominate
                // the open cost otherwise.
                let lines = match options.raw_get::<LuaValue>("lines") {
                    Ok(lines) => parse_panel_lines(&lines),
                    Err(_) => Vec::new(),
                };
                let id = {
                    let mut host = host.borrow_mut();
                    let id = host.next_panel_id;
                    host.next_panel_id += 1;
                    host.panels.push(PluginPanel {
                        id,
                        plugin_id: shared.borrow().plugin_id.clone(),
                        title,
                        footer,
                        selectable,
                        selected,
                        lines: Arc::new(lines),
                    });
                    id
                };
                if let Some(on_key) = on_key {
                    shared.borrow_mut().panel_handlers.insert(id, on_key);
                }
                let panel = panel_table(lua, &host, id)?;
                Ok(LuaValue::Table(panel))
            },
        )?;
    }

    let events = module(lua, root, "events")?;
    {
        let shared = shared.clone();
        set_fn(
            lua,
            &events,
            "on",
            move |_, (event, callback): (String, Option<Function>)| {
                let Some(callback) = callback else {
                    return Ok(0i64);
                };
                if event.trim().is_empty() {
                    return Ok(0i64);
                }
                let mut shared = shared.borrow_mut();
                let id = shared.next_subscription;
                shared.next_subscription += 1;
                shared.subscriptions.push(Subscription {
                    id,
                    event,
                    callback,
                });
                Ok(id)
            },
        )?;
    }
    {
        let shared = shared.clone();
        set_fn(lua, &events, "off", move |_, id: i64| {
            let mut shared = shared.borrow_mut();
            let before = shared.subscriptions.len();
            shared
                .subscriptions
                .retain(|subscription| subscription.id != id);
            Ok(shared.subscriptions.len() != before)
        })?;
    }

    let exec = module(lua, root, "exec")?;
    {
        let host = host.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &exec,
            "run",
            move |lua, (command, args, options): (String, Option<LuaValue>, Option<LuaValue>)| {
                let value = run_process(
                    &host,
                    &coordinator,
                    &command,
                    args.as_ref(),
                    options.as_ref(),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }

    let config = module(lua, root, "config")?;
    {
        let shared = shared.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &config,
            "get",
            move |lua, (key, fallback): (String, Option<LuaValue>)| {
                let fallback = fallback
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let value = shared.borrow().config.get(&key, fallback);
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let shared = shared.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &config,
            "set",
            move |_, (key, value): (String, Option<LuaValue>)| {
                if key.is_empty() {
                    return Ok(false);
                }
                let value = value
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                shared.borrow_mut().config.set(&key, value);
                Ok(true)
            },
        )?;
    }
    {
        let host = host.clone();
        let shared = shared.clone();
        set_fn(lua, &config, "save", move |_, ()| {
            match shared.borrow().config.save() {
                Ok(()) => Ok(true),
                Err(error) => {
                    host.borrow_mut()
                        .messages
                        .push((format!("config save failed: {error}"), true));
                    Ok(false)
                }
            }
        })?;
    }
    {
        let shared = shared.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &config, "get_all", move |lua, ()| {
            let value = shared.borrow().config.snapshot();
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }

    // Legacy buffer API retained for existing Rust plugins. These functions
    // are on the hottest benchmark path, so they use the raw Lua C API instead
    // of mlua's value conversion layer.
    let raw_ptr = raw as *const RawCtx as *mut c_void;
    add_raw_fn(lua, "line_count", raw_ptr, raw_line_count)?;
    add_raw_fn(lua, "get_line", raw_ptr, raw_get_line)?;
    add_raw_fn(lua, "set_line", raw_ptr, raw_set_line)?;
    add_raw_fn(lua, "get_text", raw_ptr, raw_get_text)?;
    add_raw_fn(lua, "set_text", raw_ptr, raw_set_text)?;
    add_raw_fn(lua, "cursor", raw_ptr, raw_cursor)?;
    add_raw_fn(lua, "set_cursor", raw_ptr, raw_set_cursor)?;
    add_raw_fn(lua, "insert", raw_ptr, raw_insert)?;
    add_raw_fn(lua, "register_command", raw_ptr, raw_register_command)?;

    // Expose `command` as an alias for the legacy `register_command`.
    if let Ok(value) = root.get::<LuaValue>("register_command") {
        root.set("command", value)?;
    }
    Ok(())
}

fn augment_lsp_modules(
    lua: &Lua,
    root: &Table,
    shared: &Rc<RefCell<RuntimeShared>>,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let lsp = root.get::<Table>("lsp")?;
    {
        let shared = shared.clone();
        set_fn(lua, &lsp, "on_diagnostics", move |_, callback: Function| {
            let mut shared = shared.borrow_mut();
            let id = shared.next_subscription;
            shared.next_subscription += 1;
            shared.subscriptions.push(Subscription {
                id,
                event: "__lsp_diagnostics".into(),
                callback,
            });
            Ok(id)
        })?;
    }

    let documents = lsp.get::<Table>("documents")?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &documents, "open_documents", move |lua, ()| {
            let documents: Vec<crate::scripting::lsp::Document> =
                coordinator.borrow().documents.clone();
            let index = Rc::new(RefCell::new(0usize));
            let coordinator = coordinator.clone();
            lua.create_function(move |lua, ()| {
                let mut index = index.borrow_mut();
                let Some(document) = documents.get(*index).cloned() else {
                    return Ok(LuaValue::Nil);
                };
                *index += 1;
                let table =
                    crate::scripting::lsp::document_table_public(lua, &coordinator, document)?;
                Ok(LuaValue::Table(table))
            })
        })?;
    }

    let fs = root.get::<Table>("fs")?;
    {
        set_fn(
            lua,
            &fs,
            "join",
            move |_, parts: mlua::Variadic<LuaValue>| {
                let text = parts
                    .iter()
                    .filter_map(|value| {
                        value
                            .as_string()
                            .map(|text| text.to_string_lossy().to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(std::path::MAIN_SEPARATOR_STR);
                Ok(text)
            },
        )?;
    }
    for table in [lsp, documents, fs] {
        let _ = table.raw_set("__hexal_wrapped", LuaValue::Nil);
        if let Ok(wrap) = lua.globals().get::<Function>("__hexal_wrap_methods") {
            let _ = wrap.call::<()>(table);
        }
    }
    Ok(())
}

fn add_raw_fn(
    lua: &Lua,
    name: &str,
    context: *mut c_void,
    function: mlua::ffi::lua_CFunction,
) -> LuaResult<()> {
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    unsafe {
        let state = lua.state();
        mlua::ffi::lua_getglobal(state, c"hexal".as_ptr());
        mlua::ffi::lua_pushlightuserdata(state, context);
        mlua::ffi::lua_pushcclosure(state, function, 1);
        mlua::ffi::lua_setfield(state, -2, c_name.as_ptr());
        mlua::ffi::lua_pop(state, 1);
    }
    Ok(())
}

unsafe fn raw_context(state: *mut mlua::ffi::lua_State) -> &'static RawCtx {
    unsafe {
        let pointer = mlua::ffi::lua_touserdata(state, mlua::ffi::lua_upvalueindex(1));
        &*(pointer as *const RawCtx)
    }
}

unsafe fn raw_arg_int(state: *mut mlua::ffi::lua_State, index: i32) -> Option<i64> {
    unsafe {
        let mut is_number = 0;
        let value = mlua::ffi::lua_tointegerx(state, index, &mut is_number);
        (is_number != 0).then_some(value)
    }
}

unsafe fn raw_arg_bytes<'a>(state: *mut mlua::ffi::lua_State, index: i32) -> Option<&'a [u8]> {
    unsafe {
        let mut length = 0usize;
        let pointer = mlua::ffi::lua_tolstring(state, index, &mut length);
        if pointer.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(pointer as *const u8, length))
        }
    }
}

unsafe fn raw_push_bytes(state: *mut mlua::ffi::lua_State, bytes: &[u8]) {
    unsafe {
        mlua::ffi::lua_pushlstring(state, bytes.as_ptr() as *const i8, bytes.len());
    }
}

unsafe extern "C-unwind" fn raw_line_count(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let count = context.host.borrow().line_count();
        mlua::ffi::lua_pushinteger(state, count as mlua::ffi::lua_Integer);
    }
    1
}

unsafe extern "C-unwind" fn raw_get_line(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let Some(line) = raw_arg_int(state, 1) else {
            return 0;
        };
        if line < 0 {
            return 0;
        }
        let line = line as usize;
        let text = {
            let host = context.host.borrow();
            host.line_bytes(line).map(|bytes| bytes.to_vec())
        };
        match text {
            Some(text) => {
                raw_push_bytes(state, &text);
                1
            }
            None => {
                mlua::ffi::lua_pushnil(state);
                1
            }
        }
    }
}

unsafe extern "C-unwind" fn raw_set_line(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let Some(line) = raw_arg_int(state, 1) else {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        };
        let Some(text) = raw_arg_bytes(state, 2).map(|bytes| bytes.to_vec()) else {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        };
        let changed = if line < 0 {
            false
        } else {
            context.host.borrow_mut().set_line(line as usize, &text)
        };
        mlua::ffi::lua_pushboolean(state, i32::from(changed));
    }
    1
}

unsafe extern "C-unwind" fn raw_get_text(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let host = context.host.borrow();
        raw_push_bytes(state, host.bytes());
    }
    1
}

unsafe extern "C-unwind" fn raw_set_text(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        if let Some(text) = raw_arg_bytes(state, 1).map(|bytes| bytes.to_vec()) {
            context.host.borrow_mut().set_text(&text);
        }
    }
    0
}

unsafe extern "C-unwind" fn raw_cursor(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let (line, column) = {
            let host = context.host.borrow();
            (host.cursor_line, host.cursor_column)
        };
        mlua::ffi::lua_pushinteger(state, line as mlua::ffi::lua_Integer);
        mlua::ffi::lua_pushinteger(state, column as mlua::ffi::lua_Integer);
    }
    2
}

unsafe extern "C-unwind" fn raw_set_cursor(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let (Some(line), Some(column)) = (raw_arg_int(state, 1), raw_arg_int(state, 2)) else {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        };
        if line < 0 || column < 0 {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        }
        context
            .host
            .borrow_mut()
            .set_cursor(line as usize, column as usize);
        mlua::ffi::lua_pushboolean(state, 1);
    }
    1
}

unsafe extern "C-unwind" fn raw_insert(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        if let Some(text) = raw_arg_bytes(state, 1).map(|bytes| bytes.to_vec()) {
            context.host.borrow_mut().insert(&text);
        }
    }
    0
}

unsafe fn call_raw_on_state(
    state: *mut mlua::ffi::lua_State,
    reference: i32,
    args: &[String],
) -> Result<mlua::ffi::lua_Integer, String> {
    unsafe {
        mlua::ffi::lua_rawgeti(
            state,
            mlua::ffi::LUA_REGISTRYINDEX,
            reference as mlua::ffi::lua_Integer,
        );
        for argument in args {
            mlua::ffi::lua_pushlstring(state, argument.as_ptr() as *const i8, argument.len());
        }
        let status = mlua::ffi::lua_pcallk(state, args.len() as i32, 1, 0, 0, None);
        if status != mlua::ffi::LUA_OK {
            let mut length = 0usize;
            let pointer = mlua::ffi::lua_tolstring(state, -1, &mut length);
            let message = if pointer.is_null() {
                "Lua error".to_string()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(pointer as *const u8, length))
                    .into_owned()
            };
            mlua::ffi::lua_settop(state, 0);
            return Err(message);
        }
        let mut is_number = 0;
        let value = mlua::ffi::lua_tointegerx(state, -1, &mut is_number);
        mlua::ffi::lua_settop(state, 0);
        Ok(value)
    }
}

unsafe extern "C-unwind" fn raw_register_command(state: *mut mlua::ffi::lua_State) -> i32 {
    unsafe {
        let context = raw_context(state);
        let Some(name) =
            raw_arg_bytes(state, 1).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        else {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        };
        if name.trim().is_empty() || mlua::ffi::lua_type(state, 2) != mlua::ffi::LUA_TFUNCTION {
            mlua::ffi::lua_pushboolean(state, 0);
            return 1;
        }
        let description = raw_arg_bytes(state, 3)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        mlua::ffi::lua_pushvalue(state, 2);
        let reference = mlua::ffi::luaL_ref(state, mlua::ffi::LUA_REGISTRYINDEX);
        let mut shared = context.shared.borrow_mut();
        shared.command_version = shared.command_version.wrapping_add(1);
        if let Some(old) = shared.commands.insert(
            name.to_lowercase(),
            CommandEntry {
                name,
                description,
                callback: None,
                raw_reference: Some(reference),
            },
        ) {
            if let Some(old) = old.raw_reference {
                mlua::ffi::luaL_unref(state, mlua::ffi::LUA_REGISTRYINDEX, old);
            }
        }
        mlua::ffi::lua_pushboolean(state, 1);
    }
    1
}

fn module(lua: &Lua, root: &Table, name: &str) -> LuaResult<Table> {
    let table = lua.create_table()?;
    root.set(name, table.clone())?;
    Ok(table)
}

fn next_request(shared: &Rc<RefCell<RuntimeShared>>) -> i64 {
    let mut shared = shared.borrow_mut();
    let id = shared.next_request;
    shared.next_request += 1;
    id
}

fn add_binding(
    shared: &Rc<RefCell<RuntimeShared>>,
    keys: &str,
    command: Option<String>,
    callback: Option<Function>,
    mode: Option<&str>,
) -> LuaResult<bool> {
    let keys = normalize_keys(keys);
    if keys.is_empty() || (command.is_none() && callback.is_none()) {
        return Ok(false);
    }
    let mode = mode.unwrap_or("normal").trim().to_lowercase();
    let mut shared = shared.borrow_mut();
    shared
        .keybindings
        .retain(|binding| !(binding.mode.eq_ignore_ascii_case(&mode) && binding.keys == keys));
    let plugin_id = shared.plugin_id.clone();
    shared.keybindings.push(KeyBinding {
        plugin_id,
        mode,
        keys,
        command,
        callback,
    });
    Ok(true)
}

pub fn normalize_keys(keys: &str) -> Vec<String> {
    let mut result = Vec::new();
    for part in keys.split_whitespace() {
        let lower = part.to_lowercase();
        let part = match lower.as_str() {
            "escape" => "esc",
            "space" => "space",
            "return" => "enter",
            value if value.starts_with("ctrl+") => {
                result.push(format!("ctrl+{}", &value[5..]));
                continue;
            }
            value if value.starts_with("alt+") => {
                result.push(format!("alt+{}", &value[4..]));
                continue;
            }
            _ => part,
        };
        result.push(part.to_string());
    }
    result
}

fn parse_panel_lines(value: &LuaValue) -> Vec<PanelLine> {
    let LuaValue::Table(table) = value else {
        return Vec::new();
    };
    let count = table.raw_len();
    let mut lines = Vec::with_capacity(count);
    for index in 1..=count {
        let Ok(item) = table.raw_get::<LuaValue>(index) else {
            continue;
        };
        lines.push(parse_panel_line(&item));
    }
    lines
}

fn parse_panel_line(item: &LuaValue) -> PanelLine {
    let LuaValue::Table(table) = item else {
        return PanelLine {
            segments: Arc::from([PanelSegment {
                text: if let LuaValue::String(text) = item {
                    text.to_string_lossy()
                } else {
                    String::new()
                },
                style: "text".into(),
            }]),
            selectable: true,
        };
    };

    let selectable = match table.raw_get::<LuaValue>("selectable") {
        Ok(LuaValue::Boolean(selectable)) => selectable,
        _ => true,
    };

    let segments = match table.raw_get::<LuaValue>("segments") {
        Ok(LuaValue::Table(segments)) => parse_panel_segments(&segments),
        Ok(LuaValue::Nil) | Err(_) => {
            let text = table.raw_get::<LuaValue>("text").unwrap_or(LuaValue::Nil);
            if matches!(text, LuaValue::Nil) {
                let mut segments = Vec::new();
                for pair in table.clone().pairs::<LuaValue, LuaValue>() {
                    if let Ok((_, value)) = pair
                        && let Some(segment) = parse_segment(&value)
                    {
                        segments.push(segment);
                    }
                }
                segments
            } else {
                parse_segment(&text).into_iter().collect()
            }
        }
        Ok(_) => Vec::new(),
    };

    PanelLine {
        segments: segments.into(),
        selectable,
    }
}

fn parse_panel_segments(table: &Table) -> Vec<PanelSegment> {
    let count = table.raw_len();
    let mut segments = Vec::with_capacity(count);
    for index in 1..=count {
        if let Ok(value) = table.raw_get::<LuaValue>(index)
            && let Some(segment) = parse_segment(&value)
        {
            segments.push(segment);
        }
    }
    segments
}

fn parse_segment(value: &LuaValue) -> Option<PanelSegment> {
    match value {
        LuaValue::String(text) => Some(PanelSegment {
            text: text.to_string_lossy(),
            style: "text".into(),
        }),
        LuaValue::Table(table) => {
            let text = match table.raw_get::<LuaValue>("text") {
                Ok(LuaValue::String(text)) => text.to_string_lossy(),
                _ => String::new(),
            };
            if text.is_empty() {
                return None;
            }
            let style = match table.raw_get::<LuaValue>("style") {
                Ok(LuaValue::String(style)) => style.to_string_lossy(),
                _ => "text".into(),
            };
            Some(PanelSegment { text, style })
        }
        _ => None,
    }
}

fn panel_table(
    lua: &Lua,
    host: &Rc<RefCell<HostState>>,
    id: i64,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("id", id)?;
    {
        let host = host.clone();
        set_fn(
            lua,
            &table,
            "set_lines",
            move |_, lines: Option<LuaValue>| {
                let lines = lines
                    .as_ref()
                    .map(parse_panel_lines)
                    .unwrap_or_default();
                if let Some(panel) = host
                    .borrow_mut()
                    .panels
                    .iter_mut()
                    .find(|panel| panel.id == id)
                {
                    panel.lines = Arc::new(lines);
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &table,
            "splice_lines",
            move |_, (start, delete_count, lines): (Option<i64>, Option<i64>, Option<LuaValue>)| {
                let start = start.unwrap_or(1).max(1) as usize - 1;
                let delete_count = delete_count.unwrap_or(0).max(0) as usize;
                let replacement = lines
                    .as_ref()
                    .map(parse_panel_lines)
                    .unwrap_or_default();
                if delete_count == 0 && replacement.is_empty() {
                    return Ok(());
                }
                if let Some(panel) = host
                    .borrow_mut()
                    .panels
                    .iter_mut()
                    .find(|panel| panel.id == id)
                {
                    let lines = Arc::make_mut(&mut panel.lines);
                    let start = start.min(lines.len());
                    let end = start.saturating_add(delete_count).min(lines.len());
                    lines.splice(start..end, replacement);
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(lua, &table, "set_title", move |_, title: Option<String>| {
            if let Some(panel) = host
                .borrow_mut()
                .panels
                .iter_mut()
                .find(|panel| panel.id == id)
            {
                panel.title = title.unwrap_or_default();
            }
            Ok(())
        })?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &table,
            "set_footer",
            move |_, footer: Option<String>| {
                if let Some(panel) = host
                    .borrow_mut()
                    .panels
                    .iter_mut()
                    .find(|panel| panel.id == id)
                {
                    panel.footer = footer.unwrap_or_default();
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &table,
            "set_selected",
            move |_, selected: Option<i64>| {
                if let Some(panel) = host
                    .borrow_mut()
                    .panels
                    .iter_mut()
                    .find(|panel| panel.id == id)
                {
                    panel.selected = selected.unwrap_or(1).max(1) as usize - 1;
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(
            lua,
            &table,
            "set_selectable",
            move |_, selectable: Option<bool>| {
                if let Some(panel) = host
                    .borrow_mut()
                    .panels
                    .iter_mut()
                    .find(|panel| panel.id == id)
                {
                    panel.selectable = selectable.unwrap_or(true);
                }
                Ok(())
            },
        )?;
    }
    {
        let host = host.clone();
        set_fn(lua, &table, "close", move |_, ()| {
            host.borrow_mut().panels.retain(|panel| panel.id != id);
            Ok(())
        })?;
    }
    if let Ok(wrap) = lua.globals().get::<Function>("__hexal_wrap_methods") {
        let _ = wrap.call::<()>(table.clone());
    }
    Ok(table)
}

fn run_process(
    host: &Rc<RefCell<HostState>>,
    coordinator: &Rc<RefCell<Coordinator>>,
    command: &str,
    args: Option<&LuaValue>,
    options: Option<&LuaValue>,
) -> Value {
    let mut result = Value::Object(Vec::new());
    result.set("ok", Value::Bool(false));
    result.set("code", Value::Number(0.0));
    result.set("stdout", Value::String(String::new()));
    result.set("stderr", Value::String(String::new()));
    result.set("error", Value::String(String::new()));
    if command.trim().is_empty() {
        result.set("error", Value::String("empty command".into()));
        return result;
    }
    let args = args
        .map(|value| from_lua(coordinator, value))
        .unwrap_or(Value::Array(Vec::new()));
    let options = options
        .map(|value| from_lua(coordinator, value))
        .unwrap_or(Value::Object(Vec::new()));
    let mut process = Command::new(command);
    process.args(json::as_array(&args).iter().filter_map(Value::as_str));
    let directory = options
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| host.borrow().workspace_root.clone());
    if let Some(directory) = directory.filter(|directory| !directory.is_empty()) {
        if !Path::new(&directory).is_dir() {
            result.set(
                "error",
                Value::String(format!("working directory not found: {directory}")),
            );
            return result;
        }
        process.current_dir(directory);
    }
    let timeout = options
        .get("timeout")
        .and_then(Value::as_i64)
        .unwrap_or(15_000)
        .clamp(100, 120_000);
    process.stdout(Stdio::piped()).stderr(Stdio::piped());
    let Ok(mut child) = process.spawn() else {
        result.set("error", Value::String(format!("cannot start '{command}'")));
        return result;
    };
    let stdout_reader = child.stdout.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = stream.read_to_string(&mut output);
            output
        })
    });
    let stderr_reader = child.stderr.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = stream.read_to_string(&mut output);
            output
        })
    });
    let deadline = Instant::now() + Duration::from_millis(timeout as u64);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_reader
                    .and_then(|reader| reader.join().ok())
                    .unwrap_or_default();
                let stderr = stderr_reader
                    .and_then(|reader| reader.join().ok())
                    .unwrap_or_default();
                result.set("ok", Value::Bool(status.success()));
                result.set("code", Value::Number(status.code().unwrap_or(-1) as f64));
                result.set("stdout", Value::String(stdout));
                if !status.success() {
                    result.set(
                        "error",
                        Value::String(if stderr.trim().is_empty() {
                            format!("process exited with code {}", status.code().unwrap_or(-1))
                        } else {
                            stderr.trim().to_string()
                        }),
                    );
                }
                result.set("stderr", Value::String(stderr));
                return result;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.and_then(|reader| reader.join().ok());
                    let _ = stderr_reader.and_then(|reader| reader.join().ok());
                    result.set(
                        "error",
                        Value::String(format!("process timed out after {timeout} ms")),
                    );
                    return result;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.and_then(|reader| reader.join().ok());
                let _ = stderr_reader.and_then(|reader| reader.join().ok());
                result.set("error", Value::String(error.to_string()));
                return result;
            }
        }
    }
}

unsafe extern "C" fn lua_alloc(
    userdata: *mut c_void,
    pointer: *mut c_void,
    old_size: usize,
    new_size: usize,
) -> *mut c_void {
    unsafe {
        let allocator = &mut *(userdata as *mut LuaAllocator);
        if new_size == 0 {
            if !pointer.is_null() {
                allocator.dealloc(pointer as *mut u8, old_size);
            }
            return std::ptr::null_mut();
        }
        if pointer.is_null() {
            allocator.alloc(new_size) as *mut c_void
        } else {
            allocator.realloc(pointer as *mut u8, old_size, new_size) as *mut c_void
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HostState, LuaRuntime};
    use crate::core::document::Document;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn runtime() -> (LuaRuntime, Rc<RefCell<HostState>>) {
        let host = Rc::new(RefCell::new(HostState::default()));
        let runtime = LuaRuntime::new("test", ".", host.clone()).expect("create Lua runtime");
        (runtime, host)
    }

    #[test]
    fn replaces_cached_borrowed_view_when_document_changes_identity() {
        let (mut runtime, host) = runtime();
        let first = Document::from_bytes(None, b"first\n".to_vec());
        let second = Document::from_bytes(None, b"second\n".to_vec());
        assert_eq!(first.revision(), second.revision());
        assert_ne!(first.id(), second.id());

        runtime.sync_from_document(&first, 0);
        assert_eq!(host.borrow().bytes(), b"first\n");

        // A different document with the same revision must invalidate the
        // borrowed pointer, otherwise a subsequent plugin call could read
        // freed memory from the previous piece table.
        runtime.sync_from_document(&second, 0);
        assert_eq!(host.borrow().bytes(), b"second\n");
    }

    #[test]
    fn stale_line_indexes_cannot_index_past_the_buffer() {
        let mut host = HostState::default();
        host.line_starts = vec![0, 4, 8, 12];
        assert_eq!(host.bytes(), b"");
        assert_eq!(host.line_bounds(0), Some((0, 0)));
        assert_eq!(host.line_bounds(2), None);
        assert_eq!(host.line_bytes(1), None);
        assert_eq!(host.cursor_byte(), 0);

        host.ensure_lines();
        assert_eq!(host.line_count(), 1);
        assert_eq!(host.line_bounds(0), Some((0, 0)));
    }

    #[test]
    fn host_line_operations_clamp_and_handle_multibyte_text() {
        let mut host = HostState::default();
        host.load("a\n\u{263a}b\nlast".as_bytes().to_vec(), 2);
        assert_eq!(host.line_count(), 3);
        assert_eq!(host.line_bytes(1), Some("\u{263a}b".as_bytes()));
        assert_eq!(host.line_bytes(99), None);

        assert!(host.set_line(1, "wide \u{263a} line".as_bytes()));
        assert_eq!(host.line_bytes(1), Some("wide \u{263a} line".as_bytes()));

        host.set_cursor(99, 99);
        assert_eq!(host.cursor_line, 2);
        assert!(host.cursor_column <= host.line_bytes(2).map_or(0, |line| line.len()));

        assert!(host.replace_range(0, 0, b"prefix "));
        assert!(host.bytes().starts_with(b"prefix a\n"));
        assert!(!host.replace_range(5, 1, b"x"));
    }

    #[test]
    fn apply_to_document_keeps_the_host_view_consistent() {
        let (mut runtime, host) = runtime();
        let mut document = Document::from_bytes(None, b"one\ntwo\nthree\n".to_vec());
        document.insert(0, b"x");
        let mut cursor = 0usize;
        runtime.sync_from_document(&document, cursor);
        assert_eq!(host.borrow().bytes(), document.all_text());

        let effects = runtime.apply_to_document(&mut document, &mut cursor);
        assert!(effects.errors.is_empty(), "{:?}", effects.errors);
        assert_eq!(host.borrow().bytes(), document.all_text());
        assert_eq!(host.borrow().line_count(), document.line_count());
        assert!(host.borrow().line_bytes(document.line_count() - 1).is_some());
    }

    #[test]
    fn lua_command_can_read_and_edit_the_buffer() {
        let (mut runtime, host) = runtime();
        runtime
            .lua
            .load(
                r#"
                hexal.commands.register("test-edit", function()
                    local first = hexal.editor.get_line(0)
                    hexal.editor.insert_text("X")
                    return #first
                end)
                "#,
            )
            .exec()
            .unwrap();
        let mut document = Document::from_bytes(None, b"hello\n".to_vec());
        let mut cursor = 1usize;
        let (handled, effects) = runtime
            .call_command("test-edit", &[], &mut document, &mut cursor)
            .unwrap();
        assert!(handled);
        assert!(effects.errors.is_empty(), "{:?}", effects.errors);
        assert_eq!(document.all_text(), b"hXello\n");
        assert_eq!(cursor, 2);
        assert!(!host.borrow().dirty);
    }

    #[test]
    fn lua_callbacks_can_reenter_commands_events_and_panels() {
        let (mut runtime, host) = runtime();
        runtime
            .lua
            .load(
                r#"
                hexal.commands.register("inner", function()
                    hexal.editor.insert_text("I")
                    return 1
                end)
                hexal.commands.register("outer", function()
                    local panel = hexal.ui.panel({
                        title = "Panel",
                        lines = { "one" },
                    })
                    panel.set_lines({ "two" })
                    local ok = hexal.commands.execute("inner")
                    hexal.commands.register("dynamic", function()
                        return 0
                    end)
                    return ok and 1 or 0
                end)
                hexal.events.on("custom", function()
                    return hexal.commands.execute("inner")
                end)
                "#,
            )
            .exec()
            .unwrap();
        assert!(runtime.has_command("dynamic") == false);

        let mut document = Document::from_bytes(None, b"hello\n".to_vec());
        let mut cursor = 0usize;
        let (handled, effects) = runtime
            .call_command("outer", &[], &mut document, &mut cursor)
            .unwrap();
        assert!(handled);
        assert!(effects.errors.is_empty(), "{:?}", effects.errors);
        assert_eq!(document.all_text(), b"Ihello\n");
        assert!(runtime.has_command("dynamic"));
        assert_eq!(host.borrow().panels.len(), 1);
        assert_eq!(host.borrow().panels[0].lines[0].segments[0].text, "two");

        runtime.fire_event(
            "custom",
            &crate::util::json::Value::Object(Vec::new()),
            &mut document,
            &mut cursor,
        );
        runtime.apply_to_document(&mut document, &mut cursor);
        assert_eq!(document.all_text(), b"IIhello\n");
        assert!(
            host.borrow().errors.is_empty(),
            "{:?}",
            host.borrow().errors
        );
    }

    #[test]
    fn exec_run_drains_large_output_without_deadlocking() {
        let (mut runtime, _host) = runtime();
        runtime
            .lua
            .load(
                r#"
                hexal.commands.register("large-output", function()
                    local result = hexal.exec.run("sh", {
                        "-c", "yes x | head -c 200000",
                    })
                    if result.ok and #result.stdout == 200000 then
                        hexal.editor.insert_text("Y")
                        return 1
                    end
                    return 0
                end)
                "#,
            )
            .exec()
            .unwrap();
        let mut document = Document::from_bytes(None, b"body\n".to_vec());
        let mut cursor = 0usize;
        let (handled, effects) = runtime
            .call_command("large-output", &[], &mut document, &mut cursor)
            .unwrap();
        assert!(handled);
        assert!(effects.errors.is_empty(), "{:?}", effects.errors);
        assert_eq!(document.all_text(), b"Ybody\n");
    }
}
