//! Lua-facing LSP runtime: documents, diagnostics, servers and test clients.
//!
//! The editor exposes the same module surface as the C# host. Lua servers are
//! routed in-process; no external process or async runtime is required.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Lua, Result as LuaResult, Table, Value as LuaValue};

use crate::scripting::json::{self, Value};
use crate::util::json as hosted_json;

pub const ERROR_PARSE: i64 = -32700;
pub const ERROR_INVALID_REQUEST: i64 = -32600;
pub const ERROR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERROR_INVALID_PARAMS: i64 = -32602;
pub const ERROR_INTERNAL: i64 = -32603;
pub const ERROR_SERVER_NOT_INITIALIZED: i64 = -32002;
pub const ERROR_REQUEST_CANCELLED: i64 = -32800;
pub const ERROR_CONTENT_MODIFIED: i64 = -32801;
pub const ERROR_REQUEST_FAILED: i64 = -32803;
pub const ERROR_SERVER_CANCELLED: i64 = -32802;

#[derive(Clone, Debug)]
pub struct LspRequest {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub language_id: String,
    pub initialization_options: Value,
}

#[derive(Clone, Debug)]
pub struct ProtocolError {
    pub code: i64,
    pub message: String,
    pub data: Value,
}

impl ProtocolError {
    fn new(code: i64, message: impl Into<String>) -> ProtocolError {
        ProtocolError {
            code,
            message: message.into(),
            data: Value::Null,
        }
    }

    fn to_value(&self) -> Value {
        let mut object = Value::Object(Vec::new());
        object.set("code", Value::Number(self.code as f64));
        object.set("message", Value::String(self.message.clone()));
        if !matches!(self.data, Value::Null) {
            object.set("data", self.data.clone());
        }
        object
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub uri: String,
    pub language_id: String,
    pub version: i64,
    pub text: String,
    pub is_open: bool,
    pub is_virtual: bool,
    pub encoding: String,
}

impl Document {
    pub fn new(
        uri: impl Into<String>,
        language_id: impl Into<String>,
        version: i64,
        text: impl Into<String>,
        is_open: bool,
        is_virtual: bool,
    ) -> Document {
        Document {
            uri: uri.into(),
            language_id: language_id.into(),
            version,
            text: text.into(),
            is_open,
            is_virtual,
            encoding: "utf-16".into(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    pub fn line_text(&self, line: usize) -> Option<&str> {
        if line >= self.line_count() {
            return None;
        }
        let mut current = 0usize;
        let mut start = 0usize;
        for (index, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                if current == line {
                    let mut end = index;
                    if end > start && self.text.as_bytes()[end - 1] == b'\r' {
                        end -= 1;
                    }
                    return Some(&self.text[start..end]);
                }
                current += 1;
                start = index + 1;
            }
        }
        if current == line {
            let mut end = self.text.len();
            if end > start && self.text.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
            return Some(&self.text[start..end]);
        }
        None
    }

    pub fn position_to_char_offset(&self, line: i64, character: i64) -> usize {
        let line = line.max(0) as usize;
        let character = character.max(0) as usize;
        let mut current = 0usize;
        let mut start = 0usize;
        let bytes = self.text.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                if current == line {
                    return start
                        + column_to_byte_offset(
                            &self.text[start..index],
                            character,
                            &self.encoding,
                        );
                }
                current += 1;
                start = index + 1;
            }
        }
        if current == line {
            return start + column_to_byte_offset(&self.text[start..], character, &self.encoding);
        }
        self.text.len()
    }

    pub fn char_offset_to_byte(&self, offset: usize) -> usize {
        let limit = offset.min(self.text.chars().count());
        self.text
            .char_indices()
            .nth(limit)
            .map(|(index, _)| index)
            .unwrap_or(self.text.len())
    }

    pub fn char_offset_to_position(&self, offset: usize) -> (usize, usize) {
        let limit = offset.min(self.text.chars().count());
        let mut line = 0usize;
        let mut line_start = 0usize;
        let mut chars = 0usize;
        for (byte_index, character) in self.text.char_indices() {
            if chars == limit {
                return (
                    line,
                    char_index_to_column(&self.text[line_start..byte_index], &self.encoding),
                );
            }
            if character == '\n' {
                line += 1;
                line_start = byte_index + 1;
            }
            chars += 1;
        }
        (
            line,
            char_index_to_column(&self.text[line_start..], &self.encoding),
        )
    }

    pub fn byte_offset_to_position(&self, byte_offset: i64) -> (usize, usize) {
        let mut offset = byte_offset.max(0) as usize;
        offset = offset.min(self.text.len());
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        self.char_offset_to_position(self.text[..offset].chars().count())
    }

    pub fn word_at(&self, line: i64, character: i64) -> Option<String> {
        let text = self.line_text(line.max(0) as usize)?;
        let index = column_to_char_index(text, character.max(0) as usize, &self.encoding);
        let (start, end) = word_bounds(text, index);
        (end > start).then(|| text[start..end].to_string())
    }

    pub fn word_range_at(&self, line: i64, character: i64) -> Option<Value> {
        let line = line.max(0) as usize;
        let text = self.line_text(line)?;
        let index = column_to_char_index(text, character.max(0) as usize, &self.encoding);
        let (start, end) = word_bounds(text, index);
        if end <= start {
            return None;
        }
        Some(range_value(
            line,
            char_index_to_column(&text[..start], &self.encoding),
            line,
            char_index_to_column(&text[..end], &self.encoding),
        ))
    }

    pub fn get_range(&self, range: &Value) -> String {
        let (start_line, start_character, end_line, end_character) = read_range(range);
        let start = self.position_to_char_offset(start_line, start_character);
        let mut end = self.position_to_char_offset(end_line, end_character);
        if end < start {
            end = start;
        }
        let start_byte = self.char_offset_to_byte(start);
        let end_byte = self.char_offset_to_byte(end);
        self.text[start_byte..end_byte].to_string()
    }
}

pub fn word_bounds(text: &str, index: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut start = index.min(chars.len());
    let mut end = start;
    let word =
        |character: char| character.is_alphanumeric() || character == '_' || character == '$';
    while start > 0 && word(chars[start - 1]) {
        start -= 1;
    }
    while end < chars.len() && word(chars[end]) {
        end += 1;
    }
    let byte_start = chars[..start].iter().map(|c| c.len_utf8()).sum();
    let byte_end = chars[..end].iter().map(|c| c.len_utf8()).sum();
    (byte_start, byte_end)
}

pub fn column_to_char_index(text: &str, column: usize, encoding: &str) -> usize {
    let bytes = text.as_bytes();
    let mut chars = 0usize;
    let mut seen = 0usize;
    let mut byte_index = 0usize;
    while byte_index < bytes.len() && seen < column {
        let length = utf8_length(bytes[byte_index]);
        let scalar = text[byte_index..].chars().next().unwrap_or('\0');
        let width = match encoding {
            "utf-8" => length,
            "utf-32" => 1,
            _ => scalar.len_utf16(),
        };
        if seen + width > column {
            break;
        }
        seen += width;
        chars += 1;
        byte_index += length;
    }
    chars
}

pub fn column_to_byte_offset(text: &str, column: usize, encoding: &str) -> usize {
    let index = column_to_char_index(text, column, encoding);
    text.char_indices()
        .nth(index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

pub fn char_index_to_column(text: &str, encoding: &str) -> usize {
    match encoding {
        "utf-8" => text.len(),
        "utf-32" => text.chars().count(),
        _ => text.encode_utf16().count(),
    }
}

fn utf8_length(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

pub fn range_value(
    start_line: usize,
    start_character: usize,
    end_line: usize,
    end_character: usize,
) -> Value {
    let mut range = Value::Object(Vec::new());
    range.set("start", position_value(start_line, start_character));
    range.set("end", position_value(end_line, end_character));
    range
}

pub fn position_value(line: usize, character: usize) -> Value {
    let mut value = Value::Object(Vec::new());
    value.set("line", Value::Number(line as f64));
    value.set("character", Value::Number(character as f64));
    value
}

pub fn read_position(value: &Value) -> (i64, i64) {
    (
        value.get("line").and_then(Value::as_i64).unwrap_or(0),
        value.get("character").and_then(Value::as_i64).unwrap_or(0),
    )
}

pub fn read_range(value: &Value) -> (i64, i64, i64, i64) {
    let (start_line, start_character) = value.get("start").map(read_position).unwrap_or((0, 0));
    let (end_line, end_character) = value.get("end").map(read_position).unwrap_or((0, 0));
    (start_line, start_character, end_line, end_character)
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root_path: String,
    pub root_uri: String,
    pub name: String,
}

impl Workspace {
    fn new(root_path: String) -> Workspace {
        let name = Path::new(&root_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| root_path.clone());
        Workspace {
            root_uri: uri_from_path(&root_path),
            root_path,
            name,
        }
    }

    fn folders(&self) -> Vec<(String, String)> {
        vec![(self.root_uri.clone(), self.name.clone())]
    }
}

#[derive(Clone, Debug)]
pub struct DiagnosticEntry {
    pub uri: String,
    pub namespace: String,
    pub items: Value,
}

#[derive(Clone)]
pub struct PendingRequest {
    pub callback: Option<Function>,
    pub completion_callbacks: Vec<Function>,
    pub done: bool,
    pub cancelled: bool,
    pub result: Value,
    pub error: Option<ProtocolError>,
}

#[derive(Clone)]
pub struct ClientState {
    pub initialized: bool,
    pub notifications: Vec<Value>,
    pub versions: HashMap<String, i64>,
}

impl ClientState {
    fn new() -> ClientState {
        ClientState {
            initialized: false,
            notifications: Vec::new(),
            versions: HashMap::new(),
        }
    }
}

pub struct Server {
    pub name: String,
    pub version: String,
    pub state: String,
    pub filetypes: Vec<String>,
    pub extensions: Vec<String>,
    pub root_markers: Vec<String>,
    pub initialization_options: Value,
    pub settings: Value,
    pub client_capabilities: Value,
    pub server_capabilities: Value,
    pub workspace: Workspace,
    pub request_handlers: HashMap<String, Function>,
    pub notification_handlers: HashMap<String, Function>,
    pub event_handlers: HashMap<String, Vec<Function>>,
    pub pending: HashMap<i64, PendingRequest>,
    pub next_request_id: i64,
    pub next_progress_id: i64,
    pub initialized: bool,
    pub shutting_down: bool,
    pub outgoing: Rc<RefCell<Vec<Value>>>,
}

impl Server {
    fn new(name: String, version: String, workspace: Workspace) -> Server {
        Server {
            name,
            version,
            state: "created".into(),
            filetypes: Vec::new(),
            extensions: Vec::new(),
            root_markers: Vec::new(),
            initialization_options: Value::Object(Vec::new()),
            settings: Value::Object(Vec::new()),
            client_capabilities: Value::Object(Vec::new()),
            server_capabilities: Value::Object(Vec::new()),
            workspace,
            request_handlers: HashMap::new(),
            notification_handlers: HashMap::new(),
            event_handlers: HashMap::new(),
            pending: HashMap::new(),
            next_request_id: 1,
            next_progress_id: 1,
            initialized: false,
            shutting_down: false,
            outgoing: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state.as_str(), "running" | "initializing" | "created")
    }

    pub fn registered_requests(&self) -> Vec<String> {
        self.request_handlers.keys().cloned().collect()
    }

    pub fn registered_notifications(&self) -> Vec<String> {
        self.notification_handlers.keys().cloned().collect()
    }
}

pub struct IndexState {
    pub files: HashMap<String, Vec<Value>>,
}

impl IndexState {
    fn new() -> IndexState {
        IndexState {
            files: HashMap::new(),
        }
    }

    fn symbol_count(&self) -> usize {
        self.files.values().map(Vec::len).sum()
    }
}

pub struct Coordinator {
    pub plugin_id: String,
    pub plugin_dir: String,
    pub null: LuaValue,
    pub documents: Vec<Document>,
    pub diagnostics: Vec<DiagnosticEntry>,
    pub servers: Vec<Rc<RefCell<Server>>>,
    pub modules: HashMap<String, Table>,
    pub workspace: Workspace,
    pub outgoing: Rc<RefCell<Vec<Value>>>,
    pub indexes: Vec<Rc<RefCell<IndexState>>>,
    pub external_requests: Vec<LspRequest>,
    pub external_stops: Vec<String>,
    pub diagnostic_handlers: Vec<(i64, Function)>,
    pub next_id: i64,
    pub trace: bool,
}

impl Coordinator {
    pub fn new(
        lua: &Lua,
        plugin_id: impl Into<String>,
        plugin_dir: impl Into<String>,
    ) -> Rc<RefCell<Coordinator>> {
        let plugin_dir = plugin_dir.into();
        let plugin_id = plugin_id.into();
        let root = Path::new(&plugin_dir)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_else(|| plugin_dir.clone());
        let null = LuaValue::Table(
            lua.create_table()
                .expect("cannot create JSON null sentinel"),
        );
        let outgoing = Rc::new(RefCell::new(Vec::new()));
        Rc::new(RefCell::new(Coordinator {
            plugin_id,
            plugin_dir,
            null,
            documents: Vec::new(),
            diagnostics: Vec::new(),
            servers: Vec::new(),
            modules: HashMap::new(),
            workspace: Workspace::new(root),
            outgoing,
            indexes: Vec::new(),
            external_requests: Vec::new(),
            external_stops: Vec::new(),
            diagnostic_handlers: Vec::new(),
            next_id: 1,
            trace: false,
        }))
    }

    pub fn set_workspace_root(&mut self, root: &str) {
        self.workspace = Workspace::new(root.to_string());
    }

    pub fn active_server(&self) -> Option<Rc<RefCell<Server>>> {
        self.servers
            .iter()
            .find(|server| server.borrow().is_running())
            .cloned()
            .or_else(|| self.servers.first().cloned())
    }

    pub fn find_server(&self, name: &str) -> Option<Rc<RefCell<Server>>> {
        self.servers
            .iter()
            .find(|server| server.borrow().name == name)
            .cloned()
    }

    pub fn document(&self, uri: &str) -> Option<&Document> {
        self.documents.iter().find(|document| document.uri == uri)
    }

    pub fn document_mut(&mut self, uri: &str) -> Option<&mut Document> {
        self.documents
            .iter_mut()
            .find(|document| document.uri == uri)
    }

    pub fn publish(&mut self, uri: &str, items: Value) {
        self.set_diagnostics(uri, "default", items);
    }

    pub fn set_diagnostics(&mut self, uri: &str, namespace: &str, items: Value) {
        if let Some(entry) = self
            .diagnostics
            .iter_mut()
            .find(|entry| entry.uri == uri && entry.namespace == namespace)
        {
            entry.items = items;
            return;
        }
        self.diagnostics.push(DiagnosticEntry {
            uri: uri.to_string(),
            namespace: namespace.to_string(),
            items,
        });
    }

    pub fn diagnostics(&self, uri: &str) -> Value {
        let mut result = Value::Array(Vec::new());
        for entry in self.diagnostics.iter().filter(|entry| entry.uri == uri) {
            for item in json::as_array(&entry.items) {
                json::push(&mut result, item.clone());
            }
        }
        result
    }

    pub fn clear_diagnostics(&mut self, uri: &str, namespace: Option<&str>) {
        match namespace {
            Some(namespace) => self
                .diagnostics
                .retain(|entry| !(entry.uri == uri && entry.namespace == namespace)),
            None => self.diagnostics.retain(|entry| entry.uri != uri),
        }
    }

    pub fn clear_all_diagnostics(&mut self) {
        self.diagnostics.clear();
    }
}

// ---------------------------------------------------------------------------
// URI helpers
// ---------------------------------------------------------------------------

pub fn uri_from_path(path: impl AsRef<Path>) -> String {
    let path = std::path::absolute(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
    let text = path.to_string_lossy().to_string().replace('\\', "/");
    let encoded = percent_encode(&text);
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

pub fn path_from_uri(uri: &str) -> String {
    let value = uri.strip_prefix("file://").unwrap_or(uri);
    let value = value.strip_prefix("localhost").unwrap_or(value);
    let decoded = percent_decode(value);
    if decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[2] == b':' {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

pub fn normalize_uri(uri: &str) -> String {
    if uri.starts_with("file://") {
        uri_from_path(path_from_uri(uri))
    } else {
        uri.to_string()
    }
}

pub fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/' | b':' | b'@')
        {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                output.push(high << 4 | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub fn register(lua: &Lua, root: &Table, coordinator: Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    let host = coordinator.clone();
    let null = host.borrow().null.clone();

    let lsp = module(lua, root, "lsp")?;
    let protocol = add_module(lua, &lsp, "protocol", "hexal.lsp.protocol", &coordinator)?;
    let documents = add_module(lua, &lsp, "documents", "hexal.lsp.documents", &coordinator)?;
    let workspace = add_module(lua, &lsp, "workspace", "hexal.lsp.workspace", &coordinator)?;
    let capabilities = add_module(
        lua,
        &lsp,
        "capabilities",
        "hexal.lsp.capabilities",
        &coordinator,
    )?;
    let diagnostics = add_module(
        lua,
        &lsp,
        "diagnostics",
        "hexal.lsp.diagnostics",
        &coordinator,
    )?;
    let completion = add_module(
        lua,
        &lsp,
        "completion",
        "hexal.lsp.completion",
        &coordinator,
    )?;
    let symbols = add_module(lua, &lsp, "symbols", "hexal.lsp.symbols", &coordinator)?;
    let edit = add_module(lua, &lsp, "edit", "hexal.lsp.edit", &coordinator)?;
    let types = add_module(lua, &lsp, "types", "hexal.lsp.types", &coordinator)?;
    let uri = add_module(lua, &lsp, "uri", "hexal.lsp.uri", &coordinator)?;
    let position = add_module(lua, &lsp, "position", "hexal.lsp.position", &coordinator)?;
    let semantic = add_module(
        lua,
        &lsp,
        "semantic_tokens",
        "hexal.lsp.semantic_tokens",
        &coordinator,
    )?;
    let inlay = add_module(
        lua,
        &lsp,
        "inlay_hint",
        "hexal.lsp.inlay_hint",
        &coordinator,
    )?;
    let signature = add_module(
        lua,
        &lsp,
        "signature_help",
        "hexal.lsp.signature_help",
        &coordinator,
    )?;
    let folding = add_module(
        lua,
        &lsp,
        "folding_range",
        "hexal.lsp.folding_range",
        &coordinator,
    )?;
    let code_lens = add_module(lua, &lsp, "code_lens", "hexal.lsp.code_lens", &coordinator)?;
    let server_module = add_module(lua, &lsp, "server", "hexal.lsp.server", &coordinator)?;

    let async_module = module(lua, root, "async")?;
    let json_module = module(lua, root, "json")?;
    let fs_module = module(lua, root, "fs")?;
    let log_module = module(lua, root, "log")?;
    let index_module = module(lua, root, "index")?;
    let process_module = module(lua, root, "process")?;
    let debug_module = module(lua, root, "debug")?;
    let testing_module = module(lua, root, "testing")?;
    let testing_lsp = add_module(
        lua,
        &testing_module,
        "lsp",
        "hexal.testing.lsp",
        &coordinator,
    )?;

    let _ = server_module;

    bind_protocol(lua, &protocol, &coordinator)?;
    bind_server(lua, &lsp, &coordinator)?;
    bind_documents(lua, &documents, &coordinator)?;
    bind_workspace(lua, &workspace, &coordinator)?;
    bind_capabilities(lua, &capabilities, &coordinator)?;
    bind_diagnostics(lua, &diagnostics, &coordinator)?;
    bind_completion(lua, &completion, &coordinator)?;
    bind_symbols(lua, &symbols, &coordinator)?;
    bind_edit(lua, &edit, &coordinator)?;
    bind_types(lua, &types, &coordinator)?;
    bind_uri(lua, &uri, &coordinator)?;
    bind_position(lua, &position, &coordinator)?;
    bind_semantic(lua, &semantic, &coordinator)?;
    bind_inlay(lua, &inlay, &coordinator)?;
    bind_signature(lua, &signature, &coordinator)?;
    bind_folding(lua, &folding, &coordinator)?;
    bind_code_lens(lua, &code_lens, &coordinator)?;
    bind_async(lua, &async_module, &coordinator)?;
    bind_json(lua, &json_module, &coordinator)?;
    bind_fs(lua, &fs_module, &coordinator)?;
    bind_log(lua, &log_module, &coordinator)?;
    bind_index(lua, &index_module, &coordinator)?;
    bind_process(lua, &process_module, &coordinator)?;
    bind_debug(lua, &debug_module, &coordinator)?;
    bind_testing(lua, &testing_lsp, &coordinator)?;

    let mut modules = host.borrow_mut().modules.clone();
    modules.insert("hexal.lsp".into(), lsp.clone());
    modules.insert("hexal.async".into(), async_module.clone());
    modules.insert("hexal.json".into(), json_module.clone());
    modules.insert("hexal.fs".into(), fs_module.clone());
    modules.insert("hexal.log".into(), log_module.clone());
    modules.insert("hexal.index".into(), index_module.clone());
    modules.insert("hexal.process".into(), process_module.clone());
    modules.insert("hexal.debug".into(), debug_module.clone());
    modules.insert("hexal.testing".into(), testing_module.clone());
    host.borrow_mut().modules = modules;

    let wrap: Function = lua.globals().get("__hexal_wrap_methods")?;
    let tables = [
        lsp.clone(),
        protocol,
        documents,
        workspace,
        capabilities,
        diagnostics,
        completion,
        symbols,
        edit,
        types,
        uri,
        position,
        semantic,
        inlay,
        signature,
        folding,
        code_lens,
        async_module,
        json_module,
        fs_module,
        log_module,
        index_module,
        process_module,
        debug_module,
        testing_lsp,
    ];
    for table in tables {
        let _ = wrap.call::<()>(table);
    }

    let mut loader = String::new();
    let mut names: Vec<String> = host.borrow().modules.keys().cloned().collect();
    names.sort();
    for name in names {
        loader.push_str(&format!(
            "package.preload[{:?}] = function() return {} end\n",
            name, name
        ));
    }
    loader.push_str("package.loaded['hexal'] = hexal\n");
    lua.load(loader).exec()?;
    let _ = null;
    Ok(())
}

fn module(lua: &Lua, root: &Table, name: &str) -> LuaResult<Table> {
    let table = lua.create_table()?;
    root.set(name, table.clone())?;
    Ok(table)
}

fn add_module(
    lua: &Lua,
    parent: &Table,
    short: &str,
    full: &str,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    parent.set(short, table.clone())?;
    coordinator
        .borrow_mut()
        .modules
        .insert(full.to_string(), table.clone());
    Ok(table)
}

fn set_fn<A, R>(
    lua: &Lua,
    table: &Table,
    name: &str,
    f: impl Fn(&Lua, A) -> LuaResult<R> + 'static,
) -> LuaResult<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    table.set(name, lua.create_function(f)?)
}

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

fn wrap(lua: &Lua, table: &Table) {
    if let Ok(wrap) = lua.globals().get::<Function>("__hexal_wrap_methods") {
        let _ = wrap.call::<()>(table.clone());
    }
}

fn table_string(table: &Table, key: &str) -> Option<String> {
    table.get::<Option<String>>(key).ok().flatten()
}

fn string_array(table: &Table, key: &str) -> Vec<String> {
    let Ok(Some(items)) = table.get::<Option<Table>>(key) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for value in items.sequence_values::<LuaValue>() {
        let Ok(value) = value else {
            break;
        };
        if let Some(text) = value.as_string() {
            result.push(text.to_string_lossy().to_string());
        }
    }
    result
}

fn task_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    result: Value,
    error: String,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let failed = !error.is_empty();
    let result = Rc::new(RefCell::new(result));
    let error = Rc::new(RefCell::new(error));
    let cancelled = Rc::new(RefCell::new(false));

    table.set("is_done", lua.create_function(|_, ()| Ok(true))?)?;
    {
        let cancelled = cancelled.clone();
        table.set(
            "is_cancelled",
            lua.create_function(move |_, ()| Ok(*cancelled.borrow()))?,
        )?;
    }
    {
        let coordinator = coordinator.clone();
        let result = result.clone();
        let failed = failed;
        table.set(
            "result",
            lua.create_function(move |lua, ()| {
                if failed {
                    return Ok(LuaValue::Nil);
                }
                to_lua(lua, &coordinator, &result.borrow())
            })?,
        )?;
    }
    {
        let error = error.clone();
        table.set(
            "error",
            lua.create_function(move |_, ()| Ok(error.borrow().clone()))?,
        )?;
    }
    {
        let cancelled = cancelled.clone();
        table.set(
            "cancel",
            lua.create_function(move |_, ()| {
                *cancelled.borrow_mut() = true;
                Ok(true)
            })?,
        )?;
    }
    {
        let coordinator = coordinator.clone();
        let result = result.clone();
        let error = error.clone();
        table.set(
            "on_complete",
            lua.create_function(move |lua, callback: Function| {
                let value = if failed {
                    LuaValue::Nil
                } else {
                    to_lua(lua, &coordinator, &result.borrow())?
                };
                let error_value = if error.borrow().is_empty() {
                    coordinator.borrow().null.clone()
                } else {
                    LuaValue::String(lua.create_string(error.borrow().as_str())?)
                };
                let _ = callback.call::<()>((value, error_value));
                Ok(true)
            })?,
        )?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn index_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    index: Rc<RefCell<IndexState>>,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    {
        let index = index.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "add_file",
            move |_, (uri, symbols): (String, Option<LuaValue>)| {
                let value = symbols
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let items = json::as_array(&value).to_vec();
                index.borrow_mut().files.insert(uri, items);
                Ok(true)
            },
        )?;
    }
    {
        let index = index.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "update_file",
            move |_, (uri, symbols): (String, Option<LuaValue>)| {
                let value = symbols
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let items = json::as_array(&value).to_vec();
                index.borrow_mut().files.insert(uri, items);
                Ok(true)
            },
        )?;
    }
    {
        let index = index.clone();
        set_fn(lua, &table, "remove_file", move |_, uri: String| {
            Ok(index.borrow_mut().files.remove(&uri).is_some())
        })?;
    }
    {
        let index = index.clone();
        set_fn(lua, &table, "invalidate", move |_, _uri: String| {
            let _ = &index;
            Ok(true)
        })?;
    }
    {
        let index = index.clone();
        set_fn(lua, &table, "clear", move |_, ()| {
            index.borrow_mut().files.clear();
            Ok(true)
        })?;
    }
    {
        let index = index.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "query_symbols",
            move |lua, (query, _options): (String, Option<LuaValue>)| {
                let mut result = Value::Array(Vec::new());
                let needle = query.to_lowercase();
                for symbols in index.borrow().files.values() {
                    for symbol in symbols {
                        let name = symbol
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_lowercase();
                        if needle.is_empty() || name.contains(&needle) {
                            json::push(&mut result, symbol.clone());
                        }
                    }
                }
                to_lua(lua, &coordinator, &result)
            },
        )?;
    }
    {
        let index = index.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "query_references",
            move |lua, (_symbol, _options): (Option<LuaValue>, Option<LuaValue>)| {
                let result = Value::Array(Vec::new());
                let _ = &index;
                to_lua(lua, &coordinator, &result)
            },
        )?;
    }
    {
        let index = index.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "statistics", move |lua, ()| {
            let mut stats = Value::Object(Vec::new());
            stats.set("files", Value::Number(index.borrow().files.len() as f64));
            stats.set(
                "symbols",
                Value::Number(index.borrow().symbol_count() as f64),
            );
            to_lua(lua, &coordinator, &stats)
        })?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn workspace_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server: Option<usize>,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    fill_workspace(lua, &table, coordinator, server)?;
    wrap(lua, &table);
    Ok(table)
}

fn fill_workspace(
    lua: &Lua,
    table: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
    server: Option<usize>,
) -> LuaResult<()> {
    let workspace = match server {
        Some(id) => coordinator
            .borrow()
            .servers
            .get(id)
            .map(|server| server.borrow().workspace.clone()),
        None => None,
    };
    let workspace = workspace.unwrap_or_else(|| coordinator.borrow().workspace.clone());

    {
        let workspace = workspace.clone();
        set_fn(lua, &table, "root_uri", move |_, ()| {
            Ok(workspace.root_uri.clone())
        })?;
    }
    {
        let workspace = workspace.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "folders", move |lua, ()| {
            let mut result = Value::Array(Vec::new());
            for (uri, name) in workspace.folders() {
                let mut folder = Value::Object(Vec::new());
                folder.set("uri", Value::String(uri));
                folder.set("name", Value::String(name));
                json::push(&mut result, folder);
            }
            to_lua(lua, &coordinator, &result)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "configuration",
            move |_, _section: Option<String>| {
                let _ = &coordinator;
                Ok(LuaValue::Nil)
            },
        )?;
    }
    {
        let workspace = workspace.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "find_files",
            move |lua, (pattern, _options): (String, Option<LuaValue>)| {
                let mut files = Vec::new();
                collect_files(Path::new(&workspace.root_path), 4096, &mut files);
                let matcher = GlobMatcher::new(&pattern);
                let mut result = Value::Array(Vec::new());
                for file in files {
                    let relative = Path::new(&file)
                        .strip_prefix(&workspace.root_path)
                        .map(|path| path.to_string_lossy().to_string().replace('\\', "/"))
                        .unwrap_or_else(|_| file.clone());
                    if matcher.matches(&relative) || matcher.matches(&file) {
                        json::push(&mut result, Value::String(uri_from_path(&file)));
                    }
                }
                let _ = &coordinator;
                task_table(lua, &coordinator, result, String::new())
            },
        )?;
    }
    {
        let workspace = workspace.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "read_file", move |lua, uri: String| {
            let text = coordinator
                .borrow()
                .document(&uri)
                .filter(|document| document.is_open)
                .map(|document| document.text.clone())
                .or_else(|| fs::read_to_string(path_from_uri(&uri)).ok())
                .unwrap_or_default();
            let _ = &workspace;
            task_table(lua, &coordinator, Value::String(text), String::new())
        })?;
    }
    {
        let workspace = workspace.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "stat", move |lua, uri: String| {
            let path = resolve_workspace_path(&workspace.root_path, &uri);
            task_table(
                lua,
                &coordinator,
                stat_value(&path.to_string_lossy()),
                String::new(),
            )
        })?;
    }
    {
        let workspace = workspace.clone();
        set_fn(
            lua,
            &table,
            "watch",
            move |lua, (glob, _callback): (String, Option<Function>)| {
                let watch_table = lua.create_table()?;
                let cancelled = Rc::new(RefCell::new(false));
                {
                    let cancelled = cancelled.clone();
                    set_fn(lua, &watch_table, "cancel", move |_, ()| {
                        *cancelled.borrow_mut() = true;
                        Ok(true)
                    })?;
                }
                {
                    let cancelled = cancelled.clone();
                    set_fn(lua, &watch_table, "is_active", move |_, ()| {
                        Ok(!*cancelled.borrow())
                    })?;
                }
                set_fn(lua, &watch_table, "is_cancelled", move |_, ()| {
                    Ok(*cancelled.borrow())
                })?;
                watch_table.set("glob", glob)?;
                let _ = workspace;
                wrap(lua, &watch_table);
                Ok(LuaValue::Table(watch_table))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "index",
            move |lua, _options: Option<LuaValue>| {
                let index = Rc::new(RefCell::new(IndexState::new()));
                coordinator.borrow_mut().indexes.push(index.clone());
                let table = index_table(lua, &coordinator, index)?;
                task_table(
                    lua,
                    &coordinator,
                    Value::Object(vec![("index".into(), Value::Null)]),
                    String::new(),
                )
                .and_then(|task| {
                    let _ = &table;
                    Ok(task)
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "progress",
            move |lua, (title, callback): (String, Option<Function>)| {
                let token = format!(
                    "hexal-progress-{}-{}",
                    coordinator.borrow().plugin_id,
                    coordinator.borrow().next_id
                );
                coordinator.borrow_mut().next_id += 1;
                let progress = progress_table(lua, &coordinator, token)?;
                if let Some(callback) = callback {
                    let _ = callback.call::<()>(progress.clone());
                }
                task_table(lua, &coordinator, Value::String(title), String::new())
            },
        )?;
    }
    Ok(())
}

fn progress_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    token: String,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("token", token)?;
    set_fn(
        lua,
        &table,
        "begin",
        |_, (_title, _options): (String, Option<LuaValue>)| Ok(true),
    )?;
    set_fn(lua, &table, "report", |_, _value: Option<LuaValue>| {
        Ok(true)
    })?;
    set_fn(lua, &table, "finish", |_, _message: Option<String>| {
        Ok(true)
    })?;
    let _ = coordinator;
    wrap(lua, &table);
    Ok(table)
}

pub fn document_table_public(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    document: Document,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("uri", document.uri.clone())?;
    table.set("language_id", document.language_id.clone())?;
    table.set("languageId", document.language_id.clone())?;
    table.set("version", document.version)?;
    table.set("text", document.text.clone())?;
    table.set("line_count", document.line_count())?;
    table.set("lineCount", document.line_count())?;
    table.set("is_open", document.is_open)?;
    table.set("isOpen", document.is_open)?;
    table.set("is_dirty", false)?;
    table.set("isDirty", false)?;
    table.set("encoding", document.encoding.clone())?;

    let uri = document.uri.clone();
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "get_text", move |_, ()| {
            Ok(coordinator
                .borrow()
                .document(&uri)
                .map(|document| document.text.clone())
                .unwrap_or_default())
        })?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "get_line", move |lua, line: Option<i64>| {
            let Some(line) = line else {
                return Ok(LuaValue::Nil);
            };
            match coordinator
                .borrow()
                .document(&uri)
                .and_then(|document| document.line_text(line.max(0) as usize).map(str::to_string))
            {
                Some(text) => Ok(LuaValue::String(lua.create_string(text)?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "get_range",
            move |lua, range: Option<LuaValue>| {
                let Some(range) = range else {
                    return Ok(LuaValue::String(lua.create_string("")?));
                };
                let value = from_lua(&coordinator, &range);
                let text = coordinator
                    .borrow()
                    .document(&uri)
                    .map(|document| document.get_range(&value))
                    .unwrap_or_default();
                Ok(LuaValue::String(lua.create_string(text)?))
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "word_at",
            move |lua, position: Option<LuaValue>| {
                let Some(position) = position else {
                    return Ok(LuaValue::Nil);
                };
                let value = from_lua(&coordinator, &position);
                let (line, character) = read_position(&value);
                match coordinator
                    .borrow()
                    .document(&uri)
                    .and_then(|document| document.word_at(line, character))
                {
                    Some(word) => Ok(LuaValue::String(lua.create_string(word)?)),
                    None => Ok(LuaValue::Nil),
                }
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "word_range_at",
            move |lua, position: Option<LuaValue>| {
                let Some(position) = position else {
                    return Ok(LuaValue::Nil);
                };
                let value = from_lua(&coordinator, &position);
                let (line, character) = read_position(&value);
                let range = coordinator
                    .borrow()
                    .document(&uri)
                    .and_then(|document| document.word_range_at(line, character));
                match range {
                    Some(range) => to_lua(lua, &coordinator, &range),
                    None => Ok(LuaValue::Nil),
                }
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "position_to_offset",
            move |_, position: Option<LuaValue>| {
                let Some(position) = position else {
                    return Ok(0i64);
                };
                let value = from_lua(&coordinator, &position);
                let (line, character) = read_position(&value);
                Ok(coordinator
                    .borrow()
                    .document(&uri)
                    .map(|document| {
                        let chars = document.position_to_char_offset(line, character);
                        document.text[..document.char_offset_to_byte(chars)].len() as i64
                    })
                    .unwrap_or(0))
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "offset_to_position",
            move |lua, offset: Option<i64>| {
                let (line, character) = coordinator
                    .borrow()
                    .document(&uri)
                    .map(|document| document.byte_offset_to_position(offset.unwrap_or(0)))
                    .unwrap_or((0, 0));
                to_lua(lua, &coordinator, &position_value(line, character))
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "get_changes_since",
            move |lua, _version: Option<i64>| {
                let result = Value::Array(Vec::new());
                let _ = &uri;
                to_lua(lua, &coordinator, &result)
            },
        )?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "snapshot", move |lua, ()| {
            let snapshot = coordinator
                .borrow()
                .document(&uri)
                .cloned()
                .unwrap_or_else(|| Document::new(uri.clone(), "plaintext", 0, "", false, false));
            let lua_table = snapshot_table(lua, &coordinator, &snapshot)?;
            Ok(LuaValue::Table(lua_table))
        })?;
    }
    {
        let uri = uri.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "is_open", move |_, ()| {
            Ok(coordinator
                .borrow()
                .document(&uri)
                .map(|document| document.is_open)
                .unwrap_or(false))
        })?;
    }
    set_fn(lua, &table, "is_dirty", |_, ()| Ok(false))?;
    wrap(lua, &table);
    Ok(table)
}

fn snapshot_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    document: &Document,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("uri", document.uri.clone())?;
    table.set("version", document.version)?;
    table.set("text", document.text.clone())?;
    table.set("line_count", document.line_count())?;
    table.set("lineCount", document.line_count())?;
    table.set("encoding", document.encoding.clone())?;
    let text = document.text.clone();
    let line_count = document.line_count();
    let encoding = document.encoding.clone();
    {
        let text = text.clone();
        set_fn(lua, &table, "get_text", move |_, ()| Ok(text.clone()))?;
    }
    {
        let text = text.clone();
        set_fn(lua, &table, "get_line", move |lua, line: Option<i64>| {
            let line = line.unwrap_or(-1);
            if line < 0 || line as usize >= line_count {
                return Ok(LuaValue::Nil);
            }
            let document = Document::new("", "plaintext", 0, text.clone(), false, false);
            Ok(match document.line_text(line as usize) {
                Some(value) => LuaValue::String(lua.create_string(value)?),
                None => LuaValue::Nil,
            })
        })?;
    }
    {
        let text = text.clone();
        let encoding = encoding.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "get_range",
            move |lua, range: Option<LuaValue>| {
                let Some(range) = range else {
                    return Ok(LuaValue::String(lua.create_string("")?));
                };
                let value = from_lua(&coordinator, &range);
                let mut document = Document::new("", "plaintext", 0, text.clone(), false, false);
                document.encoding = encoding.clone();
                Ok(LuaValue::String(
                    lua.create_string(document.get_range(&value))?,
                ))
            },
        )?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn resolve_workspace_path(root: &str, uri: &str) -> PathBuf {
    if uri.starts_with("file://") {
        return PathBuf::from(path_from_uri(uri));
    }
    let path = Path::new(uri);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(root).join(path)
    }
}

pub(crate) fn collect_workspace_files(root: &str, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(Path::new(root), limit, &mut files);
    let mut relative: Vec<String> = files
        .into_iter()
        .map(|file| {
            Path::new(&file)
                .strip_prefix(root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or(file)
        })
        .collect();
    relative.sort();
    relative
}

fn collect_files(root: &Path, limit: usize, output: &mut Vec<String>) {
    if output.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if output.len() >= limit {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if name == ".git" || name == "target" || name == "node_modules" {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&path, limit, output);
        } else if metadata.is_file() {
            output.push(path.to_string_lossy().to_string());
        }
    }
}

struct GlobMatcher {
    pattern: String,
}

impl GlobMatcher {
    fn new(pattern: &str) -> GlobMatcher {
        GlobMatcher {
            pattern: pattern.replace('\\', "/"),
        }
    }

    fn matches(&self, value: &str) -> bool {
        let value = value.replace('\\', "/");
        wildcard_match(self.pattern.as_bytes(), value.as_bytes())
    }
}

fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    match pattern[0] {
        b'*' => {
            if pattern.get(1) == Some(&b'*') {
                let rest = &pattern[2..];
                if wildcard_match(rest, value) {
                    return true;
                }
                for index in 0..=value.len() {
                    if wildcard_match(rest, &value[index..]) {
                        return true;
                    }
                }
                false
            } else {
                let rest = &pattern[1..];
                for index in 0..=value.len() {
                    if index > 0 && value[index - 1] == b'/' {
                        break;
                    }
                    if wildcard_match(rest, &value[index..]) {
                        return true;
                    }
                }
                false
            }
        }
        b'?' => !value.is_empty() && value[0] != b'/' && wildcard_match(&pattern[1..], &value[1..]),
        byte => !value.is_empty() && value[0] == byte && wildcard_match(&pattern[1..], &value[1..]),
    }
}

// ---------------------------------------------------------------------------
// Server bindings
// ---------------------------------------------------------------------------

fn server_by_id(coordinator: &Rc<RefCell<Coordinator>>, id: usize) -> Option<Rc<RefCell<Server>>> {
    coordinator.borrow().servers.get(id).cloned()
}

fn bind_server(lua: &Lua, lsp: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    let create = {
        let coordinator = coordinator.clone();
        lua.create_function(move |lua, options: Option<Table>| {
            let Some(options) = options else {
                return Ok(LuaValue::Nil);
            };
            let name = table_string(&options, "name").unwrap_or_default();
            if name.trim().is_empty() {
                return Ok(LuaValue::Nil);
            }
            let version = table_string(&options, "version").unwrap_or_default();
            let root = discover_root(
                &coordinator.borrow().plugin_dir,
                &string_array(&options, "root_markers"),
            );
            let mut server = Server::new(name.trim().to_string(), version, Workspace::new(root));
            server.filetypes = string_array(&options, "filetypes");
            server.extensions = string_array(&options, "extensions")
                .into_iter()
                .map(|extension| {
                    if extension.starts_with('.') {
                        extension
                    } else {
                        format!(".{extension}")
                    }
                })
                .collect();
            server.root_markers = string_array(&options, "root_markers");
            if let Ok(Some(value)) = options.get::<Option<Table>>("initialization_options") {
                server.initialization_options = from_table(&coordinator, &value);
            } else if let Ok(Some(value)) = options.get::<Option<Table>>("initializationOptions") {
                server.initialization_options = from_table(&coordinator, &value);
            }
            if let Ok(Some(value)) = options.get::<Option<Table>>("settings") {
                server.settings = from_table(&coordinator, &value);
            }
            if let Ok(Some(value)) = options.get::<Option<Table>>("capabilities") {
                server.server_capabilities =
                    translate_capabilities(&from_table(&coordinator, &value));
            }
            server.outgoing = coordinator.borrow().outgoing.clone();
            let server = Rc::new(RefCell::new(server));
            let id = {
                let mut coordinator = coordinator.borrow_mut();
                coordinator.servers.push(server);
                coordinator.servers.len() - 1
            };
            // Rebuild the table with the real id.
            let table = server_table(lua, &coordinator, id)?;
            Ok(LuaValue::Table(table))
        })?
    };
    lsp.set("create_server", create.clone())?;
    if let Ok(server_module) = lsp.get::<Table>("server") {
        server_module.set("create", create)?;
    }

    {
        let coordinator = coordinator.clone();
        set_fn(lua, lsp, "start", move |_, config: Option<Table>| {
            let Some(config) = config else {
                return Ok(false);
            };
            let command = table_string(&config, "command").unwrap_or_default();
            if command.trim().is_empty() {
                return Ok(false);
            }
            let initialization_options = if let Ok(Some(value)) =
                config.get::<Option<Table>>("initialization_options")
            {
                from_table(&coordinator, &value)
            } else if let Ok(Some(value)) = config.get::<Option<Table>>("initializationOptions") {
                from_table(&coordinator, &value)
            } else {
                Value::Object(Vec::new())
            };
            let request = LspRequest {
                name: table_string(&config, "name").unwrap_or_else(|| command.clone()),
                command: command.clone(),
                args: string_array(&config, "args"),
                language_id: table_string(&config, "language_id").unwrap_or_default(),
                initialization_options,
            };
            coordinator.borrow_mut().external_requests.push(request);
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, lsp, "stop", move |_, name: Option<LuaValue>| {
            let name = name
                .and_then(|value| {
                    value
                        .as_string()
                        .map(|text| text.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            coordinator.borrow_mut().external_stops.push(name);
            Ok(true)
        })?;
    }
    {
        set_fn(
            lua,
            lsp,
            "request",
            |_, (_method, _params, callback): (String, Option<LuaValue>, Option<Function>)| {
                if let Some(callback) = callback {
                    let _ = callback.call::<()>((
                        LuaValue::Nil,
                        "custom LSP requests are not supported by this host yet",
                    ));
                }
                Ok(false)
            },
        )?;
    }
    set_fn(
        lua,
        lsp,
        "notify",
        |_, (_method, _params): (String, Option<LuaValue>)| Ok(false),
    )?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, lsp, "go_to_definition", move |_, ()| {
            let _ = &coordinator;
            Ok(())
        })?;
    }
    set_fn(lua, lsp, "find_references", |_, ()| Ok(()))?;
    Ok(())
}

fn discover_root(plugin_directory: &str, markers: &[String]) -> String {
    let mut current = Path::new(plugin_directory)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(plugin_directory));
    let start = current.clone();
    loop {
        if markers.is_empty() {
            return start.to_string_lossy().to_string();
        }
        if markers.iter().any(|marker| current.join(marker).exists()) {
            return current.to_string_lossy().to_string();
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return start.to_string_lossy().to_string(),
        }
    }
}

fn server_table(lua: &Lua, coordinator: &Rc<RefCell<Coordinator>>, id: usize) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let server = server_by_id(coordinator, id);
    let name = server
        .as_ref()
        .map(|server| server.borrow().name.clone())
        .unwrap_or_default();
    table.set("__hexal_server_name", name.clone())?;

    {
        let server = server.clone();
        set_fn(lua, &table, "name", move |_, ()| {
            Ok(server
                .as_ref()
                .map(|server| server.borrow().name.clone())
                .unwrap_or_default())
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "version", move |_, ()| {
            Ok(server
                .as_ref()
                .map(|server| server.borrow().version.clone())
                .unwrap_or_default())
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "start", move |_, ()| {
            if let Some(server) = &server {
                let mut server = server.borrow_mut();
                if server.state == "running" || server.state == "initializing" {
                    return Ok(true);
                }
                server.state = "running".into();
                return Ok(true);
            }
            Ok(false)
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "stop", move |_, ()| {
            if let Some(server) = &server {
                let mut server = server.borrow_mut();
                server.state = "stopped".into();
                server.initialized = false;
                complete_all_pending(&mut server, "server stopped");
                return Ok(true);
            }
            Ok(false)
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "exit", move |_, _code: Option<i64>| {
            if let Some(server) = &server {
                let mut server = server.borrow_mut();
                server.state = "stopped".into();
                server.initialized = false;
                complete_all_pending(&mut server, "server exited");
                return Ok(true);
            }
            Ok(false)
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "state", move |_, ()| {
            Ok(server
                .as_ref()
                .map(|server| server.borrow().state.clone())
                .unwrap_or_else(|| "failed".into()))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "client", move |lua, ()| {
            let client = client_table(lua, &coordinator, id, None)?;
            Ok(LuaValue::Table(client))
        })?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "capabilities",
            move |_, options: Option<Table>| {
                let Some(options) = options else {
                    return Ok(false);
                };
                if let Some(server) = &server {
                    let translated = translate_capabilities(&from_table(&coordinator, &options));
                    let mut capabilities = server.borrow().server_capabilities.clone();
                    for (key, value) in json::as_object(&translated) {
                        set_object(&mut capabilities, key, value.clone());
                    }
                    server.borrow_mut().server_capabilities = capabilities;
                    return Ok(true);
                }
                Ok(false)
            },
        )?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "advertised_capabilities", move |lua, ()| {
            let value = server
                .as_ref()
                .map(|server| server.borrow().server_capabilities.clone())
                .unwrap_or_else(|| Value::Object(Vec::new()));
            Ok(LuaValue::Table(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            }))
        })?;
    }
    {
        let server = server.clone();
        set_fn(
            lua,
            &table,
            "register",
            move |_, (method, handler, options): (String, Function, Option<LuaValue>)| {
                if method.trim().is_empty() {
                    return Ok(false);
                }
                let Some(server) = &server else {
                    return Ok(false);
                };
                let replace = replace_requested(options.as_ref());
                let mut server = server.borrow_mut();
                if server.request_handlers.contains_key(&method) && !replace {
                    return Ok(false);
                }
                server.request_handlers.insert(method, handler);
                Ok(true)
            },
        )?;
    }
    {
        let server = server.clone();
        set_fn(
            lua,
            &table,
            "notify",
            move |_, (method, handler, options): (String, Function, Option<LuaValue>)| {
                if method.trim().is_empty() {
                    return Ok(false);
                }
                let Some(server) = &server else {
                    return Ok(false);
                };
                let replace = replace_requested(options.as_ref());
                let mut server = server.borrow_mut();
                if server.notification_handlers.contains_key(&method) && !replace {
                    return Ok(false);
                }
                server.notification_handlers.insert(method, handler);
                Ok(true)
            },
        )?;
    }
    {
        let server = server.clone();
        set_fn(
            lua,
            &table,
            "on",
            move |_, (event, handler): (String, Function)| {
                let Some(server) = &server else {
                    return Ok(false);
                };
                server
                    .borrow_mut()
                    .event_handlers
                    .entry(event)
                    .or_default()
                    .push(handler);
                Ok(true)
            },
        )?;
    }
    {
        let server = server.clone();
        set_fn(
            lua,
            &table,
            "off",
            move |_, (event, handler): (String, Function)| {
                let Some(server) = &server else {
                    return Ok(false);
                };
                let mut server = server.borrow_mut();
                if let Some(handlers) = server.event_handlers.get_mut(&event) {
                    if let Some(index) = handlers.iter().position(|item| item == &handler) {
                        handlers.remove(index);
                        return Ok(true);
                    }
                }
                Ok(false)
            },
        )?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "send_request",
            move |lua,
                  (method, params, callback, options): (
                String,
                Option<LuaValue>,
                Option<Function>,
                Option<LuaValue>,
            )| {
                let Some(server) = &server else {
                    return Ok(LuaValue::Nil);
                };
                let parameters = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let options = options.as_ref().map(|value| from_lua(&coordinator, value));
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    server,
                    &method,
                    parameters,
                    callback,
                    options.as_ref(),
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "send_notification",
            move |_, (method, params): (String, Option<LuaValue>)| {
                let Some(server) = &server else {
                    return Ok(false);
                };
                let parameters = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                send_notification(server, &method, parameters);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "documents", move |lua, ()| {
            let module = coordinator
                .borrow()
                .modules
                .get("hexal.lsp.documents")
                .cloned();
            Ok(match module {
                Some(module) => LuaValue::Table(module),
                None => LuaValue::Table(lua.create_table()?),
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "workspace", move |lua, ()| {
            let workspace = workspace_table(lua, &coordinator, Some(id))?;
            Ok(LuaValue::Table(workspace))
        })?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn replace_requested(options: Option<&LuaValue>) -> bool {
    match options {
        Some(LuaValue::Boolean(flag)) => *flag,
        Some(LuaValue::Table(table)) => table
            .get::<Option<bool>>("replace")
            .ok()
            .flatten()
            .unwrap_or(false),
        _ => false,
    }
}

fn set_object(object: &mut Value, key: &str, value: Value) {
    object.set(key, value);
}

fn translate_capabilities(value: &Value) -> Value {
    let mut result = Value::Object(Vec::new());
    for (key, value) in json::as_object(value) {
        let translated = translate_capability_value(key, value);
        match translated {
            Value::Object(entries) => {
                for (name, item) in entries {
                    result.set(&name, item);
                }
            }
            other => result.set(key, other),
        }
    }
    result
}

fn translate_capability_value(key: &str, value: &Value) -> Value {
    let Some((_, camel)) = CAPABILITY_NAMES.iter().find(|(name, _)| *name == key) else {
        return value.clone();
    };
    if matches!(value, Value::Bool(false)) {
        return Value::Null;
    }
    if matches!(value, Value::Bool(true)) {
        let mut object = Value::Object(Vec::new());
        object.set(camel, Value::Bool(true));
        return object;
    }
    let mut object = Value::Object(Vec::new());
    object.set(camel, value.clone());
    object
}

const CAPABILITY_NAMES: &[(&str, &str)] = &[
    ("hover", "hoverProvider"),
    ("completion", "completionProvider"),
    ("definition", "definitionProvider"),
    ("declaration", "declarationProvider"),
    ("type_definition", "typeDefinitionProvider"),
    ("implementation", "implementationProvider"),
    ("references", "referencesProvider"),
    ("document_highlight", "documentHighlightProvider"),
    ("document_symbol", "documentSymbolProvider"),
    ("workspace_symbol", "workspaceSymbolProvider"),
    ("code_action", "codeActionProvider"),
    ("code_lens", "codeLensProvider"),
    ("document_link", "documentLinkProvider"),
    ("document_formatting", "documentFormattingProvider"),
    (
        "document_range_formatting",
        "documentRangeFormattingProvider",
    ),
    (
        "document_on_type_formatting",
        "documentOnTypeFormattingProvider",
    ),
    ("rename", "renameProvider"),
    ("folding_range", "foldingRangeProvider"),
    ("selection_range", "selectionRangeProvider"),
    ("semantic_tokens", "semanticTokensProvider"),
    ("inlay_hint", "inlayHintProvider"),
    ("signature_help", "signatureHelpProvider"),
    ("workspace", "workspace"),
    ("text_document_sync", "textDocumentSync"),
];

fn complete_all_pending(server: &mut Server, reason: &str) {
    let ids: Vec<i64> = server
        .pending
        .iter()
        .filter(|(_, pending)| !pending.done)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if let Some(pending) = server.pending.get_mut(&id) {
            pending.done = true;
            pending.error = Some(ProtocolError::new(ERROR_REQUEST_CANCELLED, reason));
        }
    }
}

fn send_notification(server: &Rc<RefCell<Server>>, method: &str, params: Value) {
    if method.is_empty() {
        return;
    }
    let mut message = Value::Object(Vec::new());
    message.set("jsonrpc", Value::String("2.0".into()));
    message.set("method", Value::String(method.to_string()));
    if !matches!(params, Value::Null) {
        message.set("params", params);
    }
    server.borrow().outgoing.borrow_mut().push(message);
}

fn send_client_request(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server: &Rc<RefCell<Server>>,
    method: &str,
    params: Value,
    callback: Option<Function>,
    _options: Option<&Value>,
) -> LuaResult<Table> {
    let id = {
        let mut server = server.borrow_mut();
        let id = server.next_request_id;
        server.next_request_id += 1;
        server.pending.insert(
            id,
            PendingRequest {
                callback,
                completion_callbacks: Vec::new(),
                done: false,
                cancelled: false,
                result: Value::Null,
                error: None,
            },
        );
        id
    };
    let mut message = Value::Object(Vec::new());
    message.set("jsonrpc", Value::String("2.0".into()));
    message.set("id", Value::Number(id as f64));
    message.set("method", Value::String(method.to_string()));
    if !matches!(params, Value::Null) {
        message.set("params", params);
    }
    server.borrow().outgoing.borrow_mut().push(message);
    request_handle(lua, coordinator, server, id)
}

fn request_handle(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server: &Rc<RefCell<Server>>,
    id: i64,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    set_fn(lua, &table, "id", move |_, ()| Ok(id))?;
    {
        let server = server.clone();
        set_fn(lua, &table, "is_pending", move |_, ()| {
            Ok(server
                .borrow()
                .pending
                .get(&id)
                .map(|pending| !pending.done)
                .unwrap_or(false))
        })?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "result", move |lua, ()| {
            let value = server
                .borrow()
                .pending
                .get(&id)
                .map(|pending| pending.result.clone())
                .unwrap_or(Value::Null);
            to_lua(lua, &coordinator, &value)
        })?;
    }
    {
        let server = server.clone();
        set_fn(lua, &table, "cancel", move |_, ()| {
            let mut server = server.borrow_mut();
            if let Some(pending) = server.pending.get_mut(&id) {
                if pending.done {
                    return Ok(false);
                }
                pending.done = true;
                pending.cancelled = true;
                pending.error = Some(ProtocolError::new(
                    ERROR_REQUEST_CANCELLED,
                    "request cancelled",
                ));
                return Ok(true);
            }
            Ok(false)
        })?;
    }
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "on_complete",
            move |lua, callback: Function| {
                let pending = server.borrow().pending.get(&id).cloned();
                if let Some(pending) = pending {
                    if pending.done {
                        let result = if matches!(pending.result, Value::Null) {
                            LuaValue::Nil
                        } else {
                            to_lua(lua, &coordinator, &pending.result)?
                        };
                        let error = pending
                            .error
                            .as_ref()
                            .map(|error| error.to_value())
                            .unwrap_or(Value::Null);
                        let error = to_lua(lua, &coordinator, &error)?;
                        let _ = callback.call::<()>((result, error));
                    } else {
                        server
                            .borrow_mut()
                            .pending
                            .get_mut(&id)
                            .unwrap()
                            .completion_callbacks
                            .push(callback);
                    }
                }
                Ok(true)
            },
        )?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn client_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    state: Option<Rc<RefCell<ClientState>>>,
) -> LuaResult<Table> {
    let state = state.unwrap_or_else(|| Rc::new(RefCell::new(ClientState::new())));
    let table = lua.create_table()?;
    table.set("name", "Hexal")?;
    {
        let server = server_by_id(coordinator, server_id);
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "capabilities", move |lua, ()| {
            let value = server
                .as_ref()
                .map(|server| server.borrow().client_capabilities.clone())
                .unwrap_or_else(|| Value::Object(Vec::new()));
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let server = server_by_id(coordinator, server_id);
        set_fn(lua, &table, "supports", move |_, path: String| {
            let Some(server) = &server else {
                return Ok(false);
            };
            Ok(client_supports(&server.borrow().client_capabilities, &path))
        })?;
    }
    {
        let server = server_by_id(coordinator, server_id);
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "send_request",
            move |lua,
                  (method, params, callback, options): (
                String,
                Option<LuaValue>,
                Option<Function>,
                Option<LuaValue>,
            )| {
                let Some(server) = &server else {
                    return Ok(LuaValue::Nil);
                };
                let parameters = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let options = options.as_ref().map(|value| from_lua(&coordinator, value));
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    server,
                    &method,
                    parameters,
                    callback,
                    options.as_ref(),
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    {
        let server = server_by_id(coordinator, server_id);
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "send_notification",
            move |_, (method, params): (String, Option<LuaValue>)| {
                let Some(server) = &server else {
                    return Ok(false);
                };
                let parameters = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                send_notification(server, &method, parameters);
                Ok(true)
            },
        )?;
    }
    {
        let server = server_by_id(coordinator, server_id);
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "register_capability",
            move |lua, options: Option<LuaValue>| {
                let Some(server) = &server else {
                    return Ok(LuaValue::Nil);
                };
                let registrations = options
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let mut params = Value::Object(Vec::new());
                params.set("registrations", registrations);
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    server,
                    "client/registerCapability",
                    params,
                    None,
                    None,
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    {
        let server = server_by_id(coordinator, server_id);
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "unregister_capability",
            move |lua, options: Option<LuaValue>| {
                let Some(server) = &server else {
                    return Ok(LuaValue::Nil);
                };
                let registrations = options
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let mut params = Value::Object(Vec::new());
                params.set("unregisterations", registrations);
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    server,
                    "client/unregisterCapability",
                    params,
                    None,
                    None,
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    let _ = &state;
    wrap(lua, &table);
    Ok(table)
}

fn client_supports(capabilities: &Value, path: &str) -> bool {
    let mut current = capabilities;
    for part in path.split('.').filter(|part| !part.is_empty()) {
        let Some(value) = current.get(part) else {
            return false;
        };
        current = value;
    }
    current.as_bool().unwrap_or(false)
}

fn context_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    method: &str,
    request_id: Value,
) -> LuaResult<Table> {
    let table = lua.create_table()?;
    let server = server_by_id(coordinator, server_id);
    if let Some(server) = &server {
        let server_table = server_table(lua, coordinator, server_id)?;
        table.set("server", server_table)?;
        let client = client_table(lua, coordinator, server_id, None)?;
        table.set("client", client)?;
        if let Some(module) = coordinator
            .borrow()
            .modules
            .get("hexal.lsp.documents")
            .cloned()
        {
            table.set("documents", module)?;
        }
        table.set(
            "workspace",
            workspace_table(lua, coordinator, Some(server_id))?,
        )?;
        let initialization = server.borrow().initialization_options.clone();
        let settings = server.borrow().settings.clone();
        table.set(
            "initialization_options",
            to_lua(lua, coordinator, &initialization)?,
        )?;
        table.set("settings", to_lua(lua, coordinator, &settings)?)?;
    }
    table.set("method", method)?;
    if !matches!(request_id, Value::Null) {
        table.set("request_id", to_lua(lua, coordinator, &request_id)?)?;
    }
    set_fn(lua, &table, "cancelled", |_, ()| Ok(false))?;
    set_fn(lua, &table, "is_cancelled", |_, ()| Ok(false))?;
    {
        let server = server.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "progress", move |lua, ()| {
            let token = server
                .as_ref()
                .map(|server| {
                    let mut server = server.borrow_mut();
                    let id = server.next_progress_id;
                    server.next_progress_id += 1;
                    format!("hexal-progress-{}-{id}", server.name)
                })
                .unwrap_or_default();
            Ok(LuaValue::Table(progress_table(lua, &coordinator, token)?))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "reject",
            move |lua, (code, message, data): (i64, Option<String>, Option<LuaValue>)| {
                let error = ProtocolError {
                    code,
                    message: message.unwrap_or_default(),
                    data: data
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                };
                let mut value = error.to_value();
                value.set("__hexal_lsp_error", Value::Bool(true));
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    wrap(lua, &table);
    Ok(table)
}

fn event_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    event: &str,
) -> LuaResult<Table> {
    context_table(lua, coordinator, server_id, event, Value::Null)
}

fn dispatch_event(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    event: &str,
    argument: Value,
) -> LuaResult<Option<Value>> {
    let handlers = server_by_id(coordinator, server_id)
        .and_then(|server| server.borrow().event_handlers.get(event).cloned())
        .unwrap_or_default();
    if handlers.is_empty() {
        return Ok(None);
    }
    let context = event_table(lua, coordinator, server_id, event)?;
    let argument = to_lua(lua, coordinator, &argument)?;
    let mut result = None;
    for handler in handlers {
        match handler.call::<LuaValue>((argument.clone(), context.clone())) {
            Ok(value) => {
                if let LuaValue::Table(table) = value {
                    result = Some(from_table(coordinator, &table));
                }
            }
            Err(error) => {
                report(coordinator, &format!("event '{event}' failed: {error}"));
            }
        }
    }
    Ok(result)
}

fn report(coordinator: &Rc<RefCell<Coordinator>>, message: &str) {
    let _ = (coordinator, message);
}

fn dispatch_request(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    let server = server_by_id(coordinator, server_id).ok_or_else(|| {
        ProtocolError::new(ERROR_SERVER_NOT_INITIALIZED, "server is not available")
    })?;

    if method == "initialize" {
        {
            let mut state = server.borrow_mut();
            state.state = "running".into();
            state.initialized = true;
            state.client_capabilities = params
                .get("capabilities")
                .cloned()
                .unwrap_or_else(|| Value::Object(Vec::new()));
            if let Some(root_uri) = params.get("rootUri").and_then(Value::as_str) {
                state.workspace.root_uri = root_uri.to_string();
                state.workspace.root_path = path_from_uri(root_uri);
                state.workspace.name = Path::new(&state.workspace.root_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
        }
        let result = dispatch_event(lua, coordinator, server_id, "initialize", params.clone())
            .map_err(|error| ProtocolError::new(ERROR_INTERNAL, error.to_string()))?;
        return Ok(match result {
            Some(result) => result,
            None => {
                let advertised = server.borrow().server_capabilities.clone();
                let mut result = Value::Object(Vec::new());
                result.set("capabilities", advertised);
                result
            }
        });
    }
    if method == "shutdown" {
        server.borrow_mut().shutting_down = true;
        return Ok(Value::Null);
    }
    let handler = server.borrow().request_handlers.get(method).cloned();
    let Some(handler) = handler else {
        return Err(ProtocolError::new(
            ERROR_METHOD_NOT_FOUND,
            format!("method not found: {method}"),
        ));
    };
    let context = context_table(lua, coordinator, server_id, method, Value::Null)
        .map_err(|error| ProtocolError::new(ERROR_INTERNAL, error.to_string()))?;
    let params_lua = to_lua(lua, coordinator, &params)
        .map_err(|error| ProtocolError::new(ERROR_INTERNAL, error.to_string()))?;
    let result = handler
        .call::<LuaValue>((params_lua, context))
        .map_err(|error| ProtocolError::new(ERROR_INTERNAL, error.to_string()))?;
    let value = match &result {
        LuaValue::Nil => Value::Null,
        LuaValue::Table(table) => from_table(coordinator, table),
        _ => from_lua(coordinator, &result),
    };
    if let Some(error) = error_value(&value) {
        return Err(error);
    }
    Ok(normalize_result(method, value))
}

fn error_value(value: &Value) -> Option<ProtocolError> {
    if !json::get_bool(value.get("__hexal_lsp_error"), false) {
        return None;
    }
    Some(ProtocolError {
        code: json::get_i64(value.get("code"), ERROR_INTERNAL),
        message: json::get_str(value.get("message"), ""),
        data: value.get("data").cloned().unwrap_or(Value::Null),
    })
}

fn normalize_result(method: &str, value: Value) -> Value {
    if !method.starts_with("textDocument/semanticTokens/full") {
        return value;
    }
    if !matches!(value, Value::Array(_)) {
        return value;
    }
    let mut data = Value::Array(Vec::new());
    let mut previous_line = 0i64;
    let mut previous_character = 0i64;
    for item in json::as_array(&value) {
        let line = json::get_i64(item.get("line"), 0);
        let character = json::get_i64(item.get("character"), 0);
        let length = json::get_i64(item.get("length"), 0);
        let token_type = match item.get("type").and_then(Value::as_str) {
            Some(name) => TOKEN_TYPES
                .iter()
                .position(|candidate| *candidate == name)
                .unwrap_or(0) as i64,
            None => json::get_i64(item.get("type"), 0),
        };
        let modifiers = json::get_i64(item.get("modifiers"), 0);
        let delta_line = line - previous_line;
        let delta_start = if delta_line == 0 {
            character - previous_character
        } else {
            character
        };
        previous_line = line;
        previous_character = character;
        for value in [delta_line, delta_start, length, token_type, modifiers] {
            json::push(&mut data, Value::Number(value as f64));
        }
    }
    let mut result = Value::Object(Vec::new());
    result.set("data", data);
    result
}

fn dispatch_notification(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    method: &str,
    params: Value,
) {
    if method == "initialized" {
        if let Some(server) = server_by_id(coordinator, server_id) {
            server.borrow_mut().initialized = true;
        }
        let _ = dispatch_event(lua, coordinator, server_id, "initialized", params);
        return;
    }
    if method == "exit" {
        if let Some(server) = server_by_id(coordinator, server_id) {
            let code = json::get_i64(params.get("code"), 0);
            server.borrow_mut().state = "stopped".into();
            let _ = dispatch_event(
                lua,
                coordinator,
                server_id,
                "exit",
                Value::Number(code as f64),
            );
        }
        return;
    }
    if method == "textDocument/didOpen" {
        if let Some(document) = params.get("textDocument") {
            let uri = json::get_str(document.get("uri"), "");
            let language = json::get_str(document.get("languageId"), "plaintext");
            let version = json::get_i64(document.get("version"), 1);
            let text = json::get_str(document.get("text"), "");
            let mut state = coordinator.borrow_mut();
            if let Some(existing) = state.document_mut(&uri) {
                existing.language_id = language;
                existing.version = version;
                existing.text = text;
                existing.is_open = true;
            } else {
                state.documents.push(Document::new(
                    uri.clone(),
                    language,
                    version,
                    text,
                    true,
                    false,
                ));
            }
        }
        let _ = dispatch_event(
            lua,
            coordinator,
            server_id,
            "document_opened",
            params.clone(),
        );
    } else if method == "textDocument/didChange" {
        let uri = json::get_str(
            params.get("textDocument").and_then(|doc| doc.get("uri")),
            "",
        );
        let version = json::get_i64(
            params
                .get("textDocument")
                .and_then(|doc| doc.get("version")),
            0,
        );
        let changes = params
            .get("contentChanges")
            .map(json::as_array)
            .unwrap_or(&[]);
        if let Some(last) = changes.last() {
            if let Some(text) = last.get("text").and_then(Value::as_str) {
                if let Some(document) = coordinator.borrow_mut().document_mut(&uri) {
                    document.text = text.to_string();
                    document.version = version;
                }
            }
        }
        let _ = dispatch_event(
            lua,
            coordinator,
            server_id,
            "document_changed",
            params.clone(),
        );
    } else if method == "textDocument/didSave" {
        if let Some(text) = params.get("text").and_then(Value::as_str) {
            let uri = json::get_str(
                params.get("textDocument").and_then(|doc| doc.get("uri")),
                "",
            );
            if let Some(document) = coordinator.borrow_mut().document_mut(&uri) {
                document.text = text.to_string();
            }
        }
        let _ = dispatch_event(
            lua,
            coordinator,
            server_id,
            "document_saved",
            params.clone(),
        );
    } else if method == "textDocument/didClose" {
        let uri = json::get_str(
            params.get("textDocument").and_then(|doc| doc.get("uri")),
            "",
        );
        if let Some(document) = coordinator.borrow_mut().document_mut(&uri) {
            document.is_open = false;
        }
        let _ = dispatch_event(
            lua,
            coordinator,
            server_id,
            "document_closed",
            params.clone(),
        );
    }
    let handler = server_by_id(coordinator, server_id)
        .and_then(|server| server.borrow().notification_handlers.get(method).cloned());
    if let Some(handler) = handler {
        let context = match context_table(lua, coordinator, server_id, method, Value::Null) {
            Ok(table) => table,
            Err(_) => return,
        };
        let params_lua = match to_lua(lua, coordinator, &params) {
            Ok(value) => value,
            Err(_) => return,
        };
        if let Err(error) = handler.call::<()>((params_lua, context)) {
            report(
                coordinator,
                &format!("notification '{method}' failed: {error}"),
            );
        }
    }
    let _ = dispatch_event(
        lua,
        coordinator,
        server_id,
        "notification",
        Value::String(method.to_string()),
    );
}

fn handle_incoming(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    message: Value,
) {
    let method = json::get_str(message.get("method"), "");
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if !method.is_empty() {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if matches!(id, Value::Null) {
            dispatch_notification(lua, coordinator, server_id, &method, params);
        } else {
            let response = match dispatch_request(lua, coordinator, server_id, &method, params) {
                Ok(result) => response_value(&id, result),
                Err(error) => error_response(&id, &error),
            };
            if let Some(server) = server_by_id(coordinator, server_id) {
                server.borrow().outgoing.borrow_mut().push(response);
            }
        }
        return;
    }
    if !matches!(id, Value::Null) {
        let Some(id) = id.as_i64() else {
            return;
        };
        let Some(server) = server_by_id(coordinator, server_id) else {
            return;
        };
        let (result, error) = if let Some(error) = message.get("error") {
            (
                Value::Null,
                Some(ProtocolError {
                    code: json::get_i64(error.get("code"), ERROR_INTERNAL),
                    message: json::get_str(error.get("message"), "request failed"),
                    data: error.get("data").cloned().unwrap_or(Value::Null),
                }),
            )
        } else {
            (message.get("result").cloned().unwrap_or(Value::Null), None)
        };
        complete_pending(lua, coordinator, &server, id, result, error);
    }
}

fn complete_pending(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server: &Rc<RefCell<Server>>,
    id: i64,
    result: Value,
    error: Option<ProtocolError>,
) {
    let pending = {
        let mut server = server.borrow_mut();
        let Some(pending) = server.pending.get_mut(&id) else {
            return;
        };
        if pending.done {
            return;
        }
        pending.done = true;
        pending.result = result.clone();
        pending.error = error.clone();
        pending.clone()
    };
    let result_lua = to_lua(lua, coordinator, &result).unwrap_or(LuaValue::Nil);
    let error_lua = match &error {
        Some(error) => {
            let mut value = error.to_value();
            value.set("__hexal_lsp_error", Value::Bool(true));
            to_lua(lua, coordinator, &value).unwrap_or(LuaValue::Nil)
        }
        None => coordinator.borrow().null.clone(),
    };
    if let Some(callback) = pending.callback {
        let _ = callback.call::<()>((result_lua.clone(), error_lua.clone()));
    }
    for callback in pending.completion_callbacks {
        let _ = callback.call::<()>((result_lua.clone(), error_lua.clone()));
    }
}

fn response_value(id: &Value, result: Value) -> Value {
    let mut message = Value::Object(Vec::new());
    message.set("jsonrpc", Value::String("2.0".into()));
    message.set("id", id.clone());
    message.set("result", result);
    message
}

fn error_response(id: &Value, error: &ProtocolError) -> Value {
    let mut message = Value::Object(Vec::new());
    message.set("jsonrpc", Value::String("2.0".into()));
    message.set("id", id.clone());
    message.set("error", error.to_value());
    message
}

// ---------------------------------------------------------------------------
// Document, protocol, diagnostics and capability modules
// ---------------------------------------------------------------------------

fn bind_protocol(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let mut errors = Value::Object(Vec::new());
    for (name, code) in [
        ("PARSE_ERROR", ERROR_PARSE),
        ("INVALID_REQUEST", ERROR_INVALID_REQUEST),
        ("METHOD_NOT_FOUND", ERROR_METHOD_NOT_FOUND),
        ("INVALID_PARAMS", ERROR_INVALID_PARAMS),
        ("INTERNAL_ERROR", ERROR_INTERNAL),
        ("SERVER_NOT_INITIALIZED", ERROR_SERVER_NOT_INITIALIZED),
        ("REQUEST_CANCELLED", ERROR_REQUEST_CANCELLED),
        ("CONTENT_MODIFIED", ERROR_CONTENT_MODIFIED),
        ("REQUEST_FAILED", ERROR_REQUEST_FAILED),
        ("SERVER_CANCELLED", ERROR_SERVER_CANCELLED),
    ] {
        errors.set(name, Value::Number(code as f64));
    }
    module.set("error_codes", to_lua(lua, coordinator, &errors)?)?;
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "make_error",
            move |lua, (code, message, data): (i64, Option<String>, Option<LuaValue>)| {
                let mut value = Value::Object(Vec::new());
                value.set("__hexal_lsp_error", Value::Bool(true));
                value.set("code", Value::Number(code as f64));
                value.set("message", Value::String(message.unwrap_or_default()));
                value.set(
                    "data",
                    data.as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    set_fn(lua, module, "is_error", |_, value: Option<LuaValue>| {
        Ok(match value {
            Some(LuaValue::Table(table)) => table
                .get::<Option<bool>>("__hexal_lsp_error")
                .ok()
                .flatten()
                .unwrap_or(false),
            _ => false,
        })
    })?;
    Ok(())
}

fn bind_documents(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "get", move |lua, uri: String| {
            let document = coordinator.borrow().document(&uri).cloned();
            match document {
                Some(document) => Ok(LuaValue::Table(document_table_public(
                    lua,
                    &coordinator,
                    document,
                )?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "get_open", move |lua, uri: String| {
            let document = coordinator
                .borrow()
                .document(&uri)
                .filter(|document| document.is_open)
                .cloned();
            match document {
                Some(document) => Ok(LuaValue::Table(document_table_public(
                    lua,
                    &coordinator,
                    document,
                )?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "is_open", move |_, uri: String| {
            Ok(coordinator
                .borrow()
                .document(&uri)
                .map(|document| document.is_open)
                .unwrap_or(false))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "snapshot", move |lua, uri: String| {
            let document = coordinator.borrow().document(&uri).cloned();
            match document {
                Some(document) => Ok(LuaValue::Table(snapshot_table(
                    lua,
                    &coordinator,
                    &document,
                )?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "create_virtual",
            move |lua, (uri, text, language): (String, Option<String>, Option<String>)| {
                if uri.is_empty() {
                    return Ok(LuaValue::Nil);
                }
                let version = coordinator
                    .borrow()
                    .document(&uri)
                    .map(|document| document.version + 1)
                    .unwrap_or(1);
                let document = Document::new(
                    uri.clone(),
                    language.unwrap_or_else(|| "plaintext".into()),
                    version,
                    text.unwrap_or_default(),
                    true,
                    true,
                );
                {
                    let mut state = coordinator.borrow_mut();
                    state.documents.retain(|existing| existing.uri != uri);
                    state.documents.push(document.clone());
                }
                Ok(LuaValue::Table(document_table_public(
                    lua,
                    &coordinator,
                    document,
                )?))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "set_virtual_text",
            move |_, (uri, text): (String, String)| {
                let mut state = coordinator.borrow_mut();
                let Some(document) = state.document_mut(&uri) else {
                    return Ok(false);
                };
                if !document.is_virtual {
                    return Ok(false);
                }
                document.version += 1;
                document.text = text;
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "delete_virtual", move |_, uri: String| {
            let mut state = coordinator.borrow_mut();
            let Some(index) = state
                .documents
                .iter()
                .position(|document| document.uri == uri && document.is_virtual)
            else {
                return Ok(false);
            };
            state.documents.remove(index);
            state.clear_diagnostics(&uri, None);
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "close", move |_, uri: String| {
            let mut state = coordinator.borrow_mut();
            state.clear_diagnostics(&uri, None);
            if let Some(document) = state.document_mut(&uri) {
                document.is_open = false;
                return Ok(true);
            }
            Ok(false)
        })?;
    }
    Ok(())
}

fn bind_workspace(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    fill_workspace(lua, module, coordinator, None)?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "current", move |lua, ()| {
            let Some(server) = coordinator.borrow().active_server() else {
                return Ok(LuaValue::Nil);
            };
            let id = coordinator
                .borrow()
                .servers
                .iter()
                .position(|candidate| Rc::ptr_eq(candidate, &server));
            match id {
                Some(id) => Ok(LuaValue::Table(workspace_table(
                    lua,
                    &coordinator,
                    Some(id),
                )?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
    }
    Ok(())
}

fn bind_capabilities(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "client", move |lua, ()| {
            let value = coordinator
                .borrow()
                .active_server()
                .map(|server| server.borrow().client_capabilities.clone())
                .unwrap_or_else(|| Value::Object(Vec::new()));
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "server", move |lua, ()| {
            let value = coordinator
                .borrow()
                .active_server()
                .map(|server| server.borrow().server_capabilities.clone())
                .unwrap_or_else(|| Value::Object(Vec::new()));
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "supports_client", move |_, path: String| {
            let Some(server) = coordinator.borrow().active_server() else {
                return Ok(false);
            };
            let capabilities = server.borrow().client_capabilities.clone();
            Ok(client_supports(&capabilities, &path))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "register",
            move |lua, options: Option<LuaValue>| {
                let Some(server) = coordinator.borrow().active_server() else {
                    return Ok(LuaValue::Nil);
                };
                let registrations = options
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let mut params = Value::Object(Vec::new());
                params.set("registrations", registrations);
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    &server,
                    "client/registerCapability",
                    params,
                    None,
                    None,
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "unregister",
            move |lua, options: Option<LuaValue>| {
                let Some(server) = coordinator.borrow().active_server() else {
                    return Ok(LuaValue::Nil);
                };
                let registrations = options
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let mut params = Value::Object(Vec::new());
                params.set("unregisterations", registrations);
                let handle = send_client_request(
                    lua,
                    &coordinator,
                    &server,
                    "client/unregisterCapability",
                    params,
                    None,
                    None,
                )?;
                Ok(LuaValue::Table(handle))
            },
        )?;
    }
    Ok(())
}

fn bind_diagnostics(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let mut severity = Value::Object(Vec::new());
    severity.set("ERROR", Value::Number(1.0));
    severity.set("WARNING", Value::Number(2.0));
    severity.set("INFORMATION", Value::Number(3.0));
    severity.set("HINT", Value::Number(4.0));
    module.set("severity", to_lua(lua, coordinator, &severity)?)?;
    let mut tags = Value::Object(Vec::new());
    tags.set("UNNECESSARY", Value::Number(1.0));
    tags.set("DEPRECATED", Value::Number(2.0));
    module.set("tags", to_lua(lua, coordinator, &tags)?)?;

    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "publish",
            move |lua, (uri, items, _options): (String, Option<LuaValue>, Option<LuaValue>)| {
                let items = items
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                coordinator.borrow_mut().publish(&uri, items.clone());
                notify_diagnostics(&coordinator, &uri, &items);
                call_diagnostic_handlers(lua, &coordinator, &uri, &items);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "set",
            move |lua,
                  (uri, namespace, items, _options): (
                String,
                Option<String>,
                Option<LuaValue>,
                Option<LuaValue>,
            )| {
                let items = items
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                coordinator.borrow_mut().set_diagnostics(
                    &uri,
                    namespace.as_deref().unwrap_or("default"),
                    items.clone(),
                );
                notify_diagnostics(&coordinator, &uri, &items);
                call_diagnostic_handlers(lua, &coordinator, &uri, &items);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "get", move |lua, uri: String| {
            let items = coordinator.borrow().diagnostics(&uri);
            Ok(match to_lua(lua, &coordinator, &items)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "clear",
            move |_, (uri, name): (String, Option<String>)| {
                coordinator
                    .borrow_mut()
                    .clear_diagnostics(&uri, name.as_deref());
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "clear_all", move |_, ()| {
            coordinator.borrow_mut().clear_all_diagnostics();
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "on_change", move |_, callback: Function| {
            let id = {
                let mut state = coordinator.borrow_mut();
                let id = state.next_id;
                state.next_id += 1;
                id
            };
            coordinator
                .borrow_mut()
                .diagnostic_handlers
                .push((id, callback));
            Ok(id)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "off_change", move |_, id: i64| {
            let mut state = coordinator.borrow_mut();
            let before = state.diagnostic_handlers.len();
            state
                .diagnostic_handlers
                .retain(|(handler_id, _)| *handler_id != id);
            Ok(state.diagnostic_handlers.len() != before)
        })?;
    }
    Ok(())
}

fn notify_diagnostics(coordinator: &Rc<RefCell<Coordinator>>, uri: &str, items: &Value) {
    let Some(server) = coordinator.borrow().active_server() else {
        return;
    };
    let mut params = Value::Object(Vec::new());
    params.set("uri", Value::String(uri.to_string()));
    params.set("diagnostics", items.clone());
    send_notification(&server, "textDocument/publishDiagnostics", params);
}

fn call_diagnostic_handlers(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    uri: &str,
    items: &Value,
) {
    let handlers: Vec<Function> = coordinator
        .borrow()
        .diagnostic_handlers
        .iter()
        .map(|(_, handler)| handler.clone())
        .collect();
    if handlers.is_empty() {
        return;
    }
    let uri_value = LuaValue::String(match lua.create_string(uri) {
        Ok(value) => value,
        Err(_) => return,
    });
    let items_value = match to_lua(lua, coordinator, items) {
        Ok(value) => value,
        Err(_) => return,
    };
    for handler in handlers {
        let _ = handler.call::<()>((uri_value.clone(), items_value.clone()));
    }
}

// ---------------------------------------------------------------------------
// Completion, symbols, edits and types
// ---------------------------------------------------------------------------

fn normalize_object(value: &Value, mappings: &[(&str, &str)]) -> Value {
    let mut result = value.clone();
    for (from, to) in mappings {
        if from == to {
            continue;
        }
        if let Some(value) = result.get(from).cloned() {
            if result.get(to).is_none() {
                result.set(to, value);
            }
            if let Value::Object(entries) = &mut result {
                entries.retain(|(key, _)| key != from);
            }
        }
    }
    result
}

fn completion_kind(name: &str) -> Option<i64> {
    const KINDS: &[&str] = &[
        "TEXT",
        "METHOD",
        "FUNCTION",
        "CONSTRUCTOR",
        "FIELD",
        "VARIABLE",
        "CLASS",
        "INTERFACE",
        "MODULE",
        "PROPERTY",
        "UNIT",
        "VALUE",
        "ENUM",
        "KEYWORD",
        "SNIPPET",
        "COLOR",
        "FILE",
        "REFERENCE",
        "FOLDER",
        "ENUM_MEMBER",
        "CONSTANT",
        "STRUCT",
        "EVENT",
        "OPERATOR",
        "TYPE_PARAMETER",
    ];
    KINDS
        .iter()
        .position(|candidate| {
            candidate.eq_ignore_ascii_case(name)
                || candidate.replace('_', "").eq_ignore_ascii_case(name)
        })
        .map(|index| index as i64 + 1)
}

fn bind_completion(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let mut kinds = Value::Object(Vec::new());
    const NAMES: &[&str] = &[
        "TEXT",
        "METHOD",
        "FUNCTION",
        "CONSTRUCTOR",
        "FIELD",
        "VARIABLE",
        "CLASS",
        "INTERFACE",
        "MODULE",
        "PROPERTY",
        "UNIT",
        "VALUE",
        "ENUM",
        "KEYWORD",
        "SNIPPET",
        "COLOR",
        "FILE",
        "REFERENCE",
        "FOLDER",
        "ENUM_MEMBER",
        "CONSTANT",
        "STRUCT",
        "EVENT",
        "OPERATOR",
        "TYPE_PARAMETER",
    ];
    for (index, name) in NAMES.iter().enumerate() {
        let number = Value::Number(index as f64 + 1.0);
        kinds.set(name, number.clone());
        kinds.set(&name.replace('_', ""), number.clone());
        kinds.set(&name.replace('_', "").to_lowercase(), number);
    }
    module.set("kind", to_lua(lua, coordinator, &kinds)?)?;
    let mut formats = Value::Object(Vec::new());
    formats.set("PLAIN_TEXT", Value::Number(1.0));
    formats.set("SNIPPET", Value::Number(2.0));
    module.set("insert_text_format", to_lua(lua, coordinator, &formats)?)?;
    let mut triggers = Value::Object(Vec::new());
    triggers.set("INVOKED", Value::Number(1.0));
    triggers.set("TRIGGER_CHARACTER", Value::Number(2.0));
    triggers.set("TRIGGER_FOR_INCOMPLETE_COMPLETIONS", Value::Number(3.0));
    module.set("trigger_kind", to_lua(lua, coordinator, &triggers)?)?;

    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "item",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let source = from_lua(&coordinator, &options);
                let mut item = normalize_object(
                    &source,
                    &[
                        ("label_details", "labelDetails"),
                        ("sort_text", "sortText"),
                        ("filter_text", "filterText"),
                        ("insert_text", "insertText"),
                        ("insert_text_format", "insertTextFormat"),
                        ("insert_text_mode", "insertTextMode"),
                        ("text_edit", "textEdit"),
                        ("text_edit_text", "textEditText"),
                        ("additional_text_edits", "additionalTextEdits"),
                        ("commit_characters", "commitCharacters"),
                    ],
                );
                if let Some(kind) = item.get("kind").and_then(Value::as_str) {
                    if let Some(number) = completion_kind(kind) {
                        item.set("kind", Value::Number(number as f64));
                    }
                }
                to_lua(lua, &coordinator, &item)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "list", move |lua, items: Option<LuaValue>| {
            let mut result = Value::Object(Vec::new());
            result.set("isIncomplete", Value::Bool(false));
            result.set(
                "items",
                items
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new())),
            );
            Ok(match to_lua(lua, &coordinator, &result)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    set_fn(
        lua,
        module,
        "resolve",
        |_, (item, resolver): (Option<LuaValue>, Option<Function>)| {
            if let Some(resolver) = resolver {
                let _ = resolver.call::<()>(item.clone().unwrap_or(LuaValue::Nil));
            }
            Ok(item.unwrap_or(LuaValue::Nil))
        },
    )?;
    Ok(())
}

fn symbol_kind(name: &str) -> Option<i64> {
    const NAMES: &[&str] = &[
        "FILE",
        "MODULE",
        "NAMESPACE",
        "PACKAGE",
        "CLASS",
        "METHOD",
        "PROPERTY",
        "FIELD",
        "CONSTRUCTOR",
        "ENUM",
        "INTERFACE",
        "FUNCTION",
        "VARIABLE",
        "CONSTANT",
        "STRING",
        "NUMBER",
        "BOOLEAN",
        "ARRAY",
        "OBJECT",
        "KEY",
        "NULL",
        "ENUM_MEMBER",
        "STRUCT",
        "EVENT",
        "OPERATOR",
        "TYPE_PARAMETER",
    ];
    NAMES
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(name))
        .map(|index| index as i64 + 1)
}

fn bind_symbols(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let mut kinds = Value::Object(Vec::new());
    const NAMES: &[&str] = &[
        "FILE",
        "MODULE",
        "NAMESPACE",
        "PACKAGE",
        "CLASS",
        "METHOD",
        "PROPERTY",
        "FIELD",
        "CONSTRUCTOR",
        "ENUM",
        "INTERFACE",
        "FUNCTION",
        "VARIABLE",
        "CONSTANT",
        "STRING",
        "NUMBER",
        "BOOLEAN",
        "ARRAY",
        "OBJECT",
        "KEY",
        "NULL",
        "ENUM_MEMBER",
        "STRUCT",
        "EVENT",
        "OPERATOR",
        "TYPE_PARAMETER",
    ];
    for (index, name) in NAMES.iter().enumerate() {
        kinds.set(name, Value::Number(index as f64 + 1.0));
        kinds.set(&name.to_lowercase(), Value::Number(index as f64 + 1.0));
    }
    module.set("kind", to_lua(lua, coordinator, &kinds)?)?;
    let mut tags = Value::Object(Vec::new());
    tags.set("DEPRECATED", Value::Number(1.0));
    module.set("tag", to_lua(lua, coordinator, &tags)?)?;

    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "create",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let mut value = from_lua(&coordinator, &options);
                if let Some(kind) = value.get("kind").and_then(Value::as_str) {
                    if let Some(number) = symbol_kind(kind) {
                        value.set("kind", Value::Number(number as f64));
                    }
                }
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "reference",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("target_uri", "targetUri"),
                        ("target_range", "targetRange"),
                        ("target_selection_range", "targetSelectionRange"),
                        ("origin_selection_range", "originSelectionRange"),
                        ("container_name", "containerName"),
                        ("selection_range", "selectionRange"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "location",
            move |lua, (uri, range): (String, Option<LuaValue>)| {
                let mut value = Value::Object(Vec::new());
                value.set("uri", Value::String(uri));
                value.set(
                    "range",
                    range
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "location_link",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("target_uri", "targetUri"),
                        ("target_range", "targetRange"),
                        ("target_selection_range", "targetSelectionRange"),
                        ("origin_selection_range", "originSelectionRange"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "document_symbol",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("selection_range", "selectionRange"),
                        ("container_name", "containerName"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "workspace_symbol",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[("container_name", "containerName")],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    Ok(())
}

fn bind_edit(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "text_edit",
            move |lua, (range, text): (Option<LuaValue>, Option<String>)| {
                let mut value = Value::Object(Vec::new());
                value.set(
                    "range",
                    range
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                value.set("newText", Value::String(text.unwrap_or_default()));
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "workspace_edit",
            move |lua, changes: Option<LuaValue>| {
                let Some(changes) = changes else {
                    return Ok(LuaValue::Nil);
                };
                let source = from_lua(&coordinator, &changes);
                let mut edit = source.clone();
                if source.get("changes").is_none() && source.get("documentChanges").is_none() {
                    edit = Value::Object(Vec::new());
                    edit.set("changes", source);
                }
                to_lua(lua, &coordinator, &edit)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "document_change",
            move |lua, (uri, version, edits): (String, Option<LuaValue>, Option<LuaValue>)| {
                let mut document = Value::Object(Vec::new());
                document.set("uri", Value::String(uri));
                document.set(
                    "version",
                    version
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                let mut value = Value::Object(Vec::new());
                value.set("textDocument", document);
                value.set(
                    "edits",
                    edits
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Array(Vec::new())),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "apply_workspace_edit",
            move |_, edit: Option<LuaValue>| {
                let result = apply_workspace_edit(
                    &coordinator,
                    edit.as_ref().map(|value| from_lua(&coordinator, value)),
                );
                let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let error = result
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Ok((ok, error))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "apply_workspace_edit_ex",
            move |lua, edit: Option<LuaValue>| {
                let result = apply_workspace_edit(
                    &coordinator,
                    edit.as_ref().map(|value| from_lua(&coordinator, value)),
                );
                Ok(match to_lua(lua, &coordinator, &result)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    Ok(())
}

fn apply_workspace_edit(coordinator: &Rc<RefCell<Coordinator>>, edit: Option<Value>) -> Value {
    let mut result = Value::Object(Vec::new());
    result.set("ok", Value::Bool(false));
    result.set("error", Value::String(String::new()));
    let Some(edit) = edit else {
        result.set(
            "error",
            Value::String("workspace edit must be a table".into()),
        );
        return result;
    };
    let mut targets: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(changes) = edit.get("changes") {
        for (uri, edits) in json::as_object(changes) {
            if let Value::Array(items) = edits {
                targets.push((uri.clone(), items.clone()));
            }
        }
    }
    if let Some(document_changes) = edit.get("documentChanges") {
        for change in json::as_array(document_changes) {
            if let Some(kind) = change.get("kind").and_then(Value::as_str) {
                if let Err(error) = apply_resource_operation(kind, change) {
                    result.set("error", Value::String(error));
                    return result;
                }
                continue;
            }
            let uri = json::get_str(
                change.get("textDocument").and_then(|doc| doc.get("uri")),
                "",
            );
            let Some(edits) = change.get("edits") else {
                continue;
            };
            if uri.is_empty() {
                continue;
            }
            let mut items = json::as_array(edits).to_vec();
            if let Some(existing) = targets.iter_mut().find(|(target, _)| *target == uri) {
                existing.1.append(&mut items);
            } else {
                targets.push((uri, items));
            }
        }
    }

    for (uri, edits) in targets {
        let mut state = coordinator.borrow_mut();
        if let Some(document) = state.document_mut(&uri) {
            if let Err(error) = apply_text_edits(document, &edits) {
                drop(state);
                result.set("error", Value::String(error));
                return result;
            }
            continue;
        }
        let path = path_from_uri(&uri);
        let Ok(text) = fs::read_to_string(&path) else {
            drop(state);
            result.set("error", Value::String(format!("cannot read '{path}'")));
            return result;
        };
        let mut document = Document::new(uri, "plaintext", 0, text, false, false);
        if let Err(error) = apply_text_edits(&mut document, &edits) {
            drop(state);
            result.set("error", Value::String(error));
            return result;
        }
        if let Some(parent) = Path::new(&path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::write(&path, document.text) {
            drop(state);
            result.set("error", Value::String(error.to_string()));
            return result;
        }
    }
    result.set("ok", Value::Bool(true));
    result
}

fn apply_text_edits(document: &mut Document, edits: &[Value]) -> Result<(), String> {
    let mut resolved: Vec<(usize, usize, String)> = Vec::with_capacity(edits.len());
    for edit in edits {
        let Some(range) = edit.get("range") else {
            continue;
        };
        let (start_line, start_character, end_line, end_character) = read_range(range);
        let start = document.position_to_char_offset(start_line, start_character);
        let end = document.position_to_char_offset(end_line, end_character);
        let start_byte = document.char_offset_to_byte(start);
        let end_byte = document.char_offset_to_byte(end);
        resolved.push((start_byte, end_byte, json::get_str(edit.get("newText"), "")));
    }
    resolved.sort_by(|left, right| right.0.cmp(&left.0));
    let mut last_start = usize::MAX;
    for (start, end, text) in &resolved {
        if *end > last_start {
            return Err("code action contains overlapping edits".into());
        }
        last_start = *start;
        let _ = text;
    }
    for (start, end, text) in resolved {
        document.text.replace_range(start..end, &text);
    }
    Ok(())
}

fn apply_resource_operation(kind: &str, change: &Value) -> Result<(), String> {
    match kind {
        "create" => {
            let path = path_from_uri(&json::get_str(change.get("uri"), ""));
            if json::get_bool(
                change
                    .get("options")
                    .and_then(|value| value.get("overwrite")),
                false,
            ) {
                fs::write(&path, "").map_err(|error| error.to_string())?;
            } else if !Path::new(&path).exists() {
                fs::write(&path, "").map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        "rename" => {
            let old = path_from_uri(&json::get_str(change.get("oldUri"), ""));
            let new = path_from_uri(&json::get_str(change.get("newUri"), ""));
            if json::get_bool(
                change
                    .get("options")
                    .and_then(|value| value.get("overwrite")),
                false,
            ) {
                let _ = fs::remove_file(&new);
            }
            fs::rename(old, new).map_err(|error| error.to_string())
        }
        "delete" => {
            let path = path_from_uri(&json::get_str(change.get("uri"), ""));
            if Path::new(&path).is_dir() {
                fs::remove_dir_all(path).map_err(|error| error.to_string())
            } else {
                fs::remove_file(path).map_err(|error| error.to_string())
            }
        }
        _ => Err(format!("unsupported resource operation '{kind}'")),
    }
}

fn bind_types(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "position",
            move |lua, (line, character): (Option<i64>, Option<i64>)| {
                let value =
                    position_value(line.unwrap_or(0) as usize, character.unwrap_or(0) as usize);
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "range",
            move |lua, (start, end): (Option<LuaValue>, Option<LuaValue>)| {
                let mut value = Value::Object(Vec::new());
                value.set(
                    "start",
                    start
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                value.set(
                    "end",
                    end.as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "location",
            move |lua, (uri, range): (String, Option<LuaValue>)| {
                let mut value = Value::Object(Vec::new());
                value.set("uri", Value::String(uri));
                value.set(
                    "range",
                    range
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "text_edit",
            move |lua, (range, text): (Option<LuaValue>, Option<String>)| {
                let mut value = Value::Object(Vec::new());
                value.set(
                    "range",
                    range
                        .as_ref()
                        .map(|value| from_lua(&coordinator, value))
                        .unwrap_or(Value::Null),
                );
                value.set("newText", Value::String(text.unwrap_or_default()));
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "diagnostic",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("code_description", "codeDescription"),
                        ("related_information", "relatedInformation"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "completion_item",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let mut value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("label_details", "labelDetails"),
                        ("sort_text", "sortText"),
                        ("filter_text", "filterText"),
                        ("insert_text", "insertText"),
                        ("insert_text_format", "insertTextFormat"),
                        ("insert_text_mode", "insertTextMode"),
                        ("text_edit", "textEdit"),
                        ("additional_text_edits", "additionalTextEdits"),
                        ("commit_characters", "commitCharacters"),
                    ],
                );
                if let Some(kind) = value.get("kind").and_then(Value::as_str) {
                    if let Some(number) = completion_kind(kind) {
                        value.set("kind", Value::Number(number as f64));
                    }
                }
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "workspace_edit",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[("document_changes", "documentChanges")],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "document_symbol",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("selection_range", "selectionRange"),
                        ("container_name", "containerName"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "workspace_symbol",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[("container_name", "containerName")],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "inlay_hint",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("padding_left", "paddingLeft"),
                        ("padding_right", "paddingRight"),
                        ("text_edits", "textEdits"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "signature_information",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[("active_parameter", "activeParameter")],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    set_fn(
        lua,
        module,
        "validate",
        |_, (_method, _value): (String, Option<LuaValue>)| Ok(true),
    )?;
    set_fn(
        lua,
        module,
        "validate_position",
        |_, value: Option<LuaValue>| Ok(matches!(value, Some(LuaValue::Table(_)))),
    )?;
    set_fn(
        lua,
        module,
        "validate_range",
        |_, value: Option<LuaValue>| Ok(matches!(value, Some(LuaValue::Table(_)))),
    )?;
    set_fn(
        lua,
        module,
        "validate_diagnostic",
        |_, value: Option<LuaValue>| Ok(matches!(value, Some(LuaValue::Table(_)))),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// URI, position, semantic tokens and small protocol helpers
// ---------------------------------------------------------------------------

fn bind_uri(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    set_fn(lua, module, "from_path", |_, path: String| {
        Ok(uri_from_path(path))
    })?;
    set_fn(lua, module, "to_path", |_, uri: String| {
        Ok(path_from_uri(&uri))
    })?;
    set_fn(lua, module, "normalize", |_, uri: String| {
        Ok(normalize_uri(&uri))
    })?;
    set_fn(lua, module, "is_file", |_, uri: String| {
        Ok(uri.starts_with("file://"))
    })?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "parse", move |lua, uri: String| {
            let mut value = Value::Object(Vec::new());
            value.set(
                "scheme",
                Value::String(if uri.starts_with("file://") {
                    "file".into()
                } else {
                    "untitled".into()
                }),
            );
            value.set("path", Value::String(path_from_uri(&uri)));
            value.set("uri", Value::String(uri));
            to_lua(lua, &coordinator, &value)
        })?;
    }
    set_fn(
        lua,
        module,
        "join",
        |_, (left, right): (String, Option<String>)| {
            Ok(match right {
                Some(right) => Path::new(&left).join(right).to_string_lossy().to_string(),
                None => left,
            })
        },
    )?;
    set_fn(lua, module, "encode", |_, value: String| {
        Ok(percent_encode(&value))
    })?;
    set_fn(lua, module, "decode", |_, value: String| {
        Ok(percent_decode(&value))
    })?;
    Ok(())
}

fn bind_position(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "compare",
            move |_, (left, right): (Option<LuaValue>, Option<LuaValue>)| {
                let left = left
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let right = right
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (left_line, left_character) = read_position(&left);
                let (right_line, right_character) = read_position(&right);
                Ok(if left_line != right_line {
                    left_line.cmp(&right_line) as i64
                } else {
                    left_character.cmp(&right_character) as i64
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "contains",
            move |_, (range, position): (Option<LuaValue>, Option<LuaValue>)| {
                let range = range
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let position = position
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (start_line, start_character, end_line, end_character) = read_range(&range);
                let (line, character) = read_position(&position);
                Ok(
                    compare_position(line, character, start_line, start_character) >= 0
                        && compare_position(line, character, end_line, end_character) < 0,
                )
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "intersects",
            move |_, (left, right): (Option<LuaValue>, Option<LuaValue>)| {
                let left = left
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let right = right
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (a_start_line, a_start_character, a_end_line, a_end_character) =
                    read_range(&left);
                let (b_start_line, b_start_character, b_end_line, b_end_character) =
                    read_range(&right);
                Ok(!(compare_position(
                    a_end_line,
                    a_end_character,
                    b_start_line,
                    b_start_character,
                ) <= 0
                    || compare_position(
                        b_end_line,
                        b_end_character,
                        a_start_line,
                        a_start_character,
                    ) <= 0))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "validate",
            move |_, (position, document): (Option<LuaValue>, Option<LuaValue>)| {
                let position = position
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (line, character) = read_position(&position);
                if line < 0 || character < 0 {
                    return Ok(false);
                }
                if let Some(LuaValue::Table(table)) = document {
                    let text = table
                        .get::<Option<String>>("text")
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let doc = Document::new("", "plaintext", 0, text, false, false);
                    return Ok((line as usize) < doc.line_count()
                        && (character as usize)
                            <= doc.line_text(line as usize).unwrap_or("").chars().count());
                }
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "validate_range",
            move |_, (range, document): (Option<LuaValue>, Option<LuaValue>)| {
                let range = range
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (start_line, start_character, end_line, end_character) = read_range(&range);
                if start_line < 0
                    || end_line < 0
                    || compare_position(start_line, start_character, end_line, end_character) > 0
                {
                    return Ok(false);
                }
                if let Some(LuaValue::Table(table)) = document {
                    let text = table
                        .get::<Option<String>>("text")
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let doc = Document::new("", "plaintext", 0, text, false, false);
                    if start_line as usize >= doc.line_count()
                        || end_line as usize >= doc.line_count()
                    {
                        return Ok(false);
                    }
                    return Ok((start_character as usize)
                        <= doc
                            .line_text(start_line as usize)
                            .unwrap_or("")
                            .chars()
                            .count()
                        && (end_character as usize)
                            <= doc
                                .line_text(end_line as usize)
                                .unwrap_or("")
                                .chars()
                                .count());
                }
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "offset_to_position",
            move |lua, (document, offset): (Option<LuaValue>, Option<i64>)| {
                let Some(LuaValue::Table(table)) = document else {
                    return Ok(LuaValue::Nil);
                };
                let text = table
                    .get::<Option<String>>("text")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let encoding = table
                    .get::<Option<String>>("encoding")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "utf-16".into());
                let mut doc = Document::new("", "plaintext", 0, text, false, false);
                doc.encoding = encoding;
                let (line, character) = doc.byte_offset_to_position(offset.unwrap_or(0));
                to_lua(lua, &coordinator, &position_value(line, character))
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "position_to_offset",
            move |_, (document, position): (Option<LuaValue>, Option<LuaValue>)| {
                let Some(LuaValue::Table(table)) = document else {
                    return Ok(0i64);
                };
                let text = table
                    .get::<Option<String>>("text")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let encoding = table
                    .get::<Option<String>>("encoding")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "utf-16".into());
                let mut doc = Document::new("", "plaintext", 0, text, false, false);
                doc.encoding = encoding;
                let position = position
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (line, character) = read_position(&position);
                let offset = doc.position_to_char_offset(line, character);
                Ok(doc.char_offset_to_byte(offset) as i64)
            },
        )?;
    }
    Ok(())
}

fn compare_position(
    left_line: i64,
    left_character: i64,
    right_line: i64,
    right_character: i64,
) -> i64 {
    if left_line != right_line {
        left_line.cmp(&right_line) as i64
    } else {
        left_character.cmp(&right_character) as i64
    }
}

const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];

fn bind_semantic(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "encode",
            move |lua, items: Option<LuaValue>| {
                let items = items
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let normalized = normalize_result("textDocument/semanticTokens/full", items);
                let data = normalized
                    .get("data")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                Ok(match to_lua(lua, &coordinator, &data)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "legend",
            move |lua, _options: Option<LuaValue>| {
                let mut legend = Value::Object(Vec::new());
                legend.set(
                    "tokenTypes",
                    Value::Array(
                        TOKEN_TYPES
                            .iter()
                            .map(|name| Value::String((*name).into()))
                            .collect(),
                    ),
                );
                legend.set(
                    "tokenModifiers",
                    Value::Array(
                        [
                            "declaration",
                            "definition",
                            "readonly",
                            "static",
                            "deprecated",
                            "abstract",
                            "async",
                            "modification",
                            "documentation",
                            "defaultLibrary",
                        ]
                        .iter()
                        .map(|name| Value::String((*name).into()))
                        .collect(),
                    ),
                );
                Ok(match to_lua(lua, &coordinator, &legend)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    set_fn(lua, module, "register", |_, _options: Option<LuaValue>| {
        Ok(true)
    })?;
    set_fn(lua, module, "invalidate", |_, _uri: String| Ok(true))?;
    set_fn(lua, module, "clear", |_, _uri: String| Ok(true))?;
    let mut types = Value::Object(Vec::new());
    for name in TOKEN_TYPES {
        types.set(name, Value::String((*name).into()));
    }
    module.set("token_type", to_lua(lua, coordinator, &types)?)?;
    let mut modifiers = Value::Object(Vec::new());
    for name in [
        "declaration",
        "definition",
        "readonly",
        "static",
        "deprecated",
        "abstract",
        "async",
        "modification",
        "documentation",
        "defaultLibrary",
    ] {
        modifiers.set(name, Value::String(name.into()));
    }
    module.set("token_modifier", to_lua(lua, coordinator, &modifiers)?)?;
    Ok(())
}

fn bind_inlay(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "create",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("padding_left", "paddingLeft"),
                        ("padding_right", "paddingRight"),
                        ("text_edits", "textEdits"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    set_fn(
        lua,
        module,
        "resolve",
        |_, (hint, resolver): (Option<LuaValue>, Option<Function>)| {
            if let Some(resolver) = resolver {
                if let Ok(value) = resolver.call::<LuaValue>(hint.clone().unwrap_or(LuaValue::Nil))
                {
                    return Ok(value);
                }
            }
            Ok(hint.unwrap_or(LuaValue::Nil))
        },
    )?;
    Ok(())
}

fn bind_signature(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "create",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[
                        ("active_signature", "activeSignature"),
                        ("active_parameter", "activeParameter"),
                    ],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "signature",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                let value = normalize_object(
                    &from_lua(&coordinator, &options),
                    &[("active_parameter", "activeParameter")],
                );
                to_lua(lua, &coordinator, &value)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "parameter",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                to_lua(lua, &coordinator, &from_lua(&coordinator, &options))
            },
        )?;
    }
    Ok(())
}

fn bind_folding(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let coordinator = coordinator.clone();
    set_fn(
        lua,
        module,
        "create",
        move |lua, options: Option<LuaValue>| {
            let Some(options) = options else {
                return Ok(LuaValue::Nil);
            };
            let value = normalize_object(
                &from_lua(&coordinator, &options),
                &[
                    ("start_line", "startLine"),
                    ("start_character", "startCharacter"),
                    ("end_line", "endLine"),
                    ("end_character", "endCharacter"),
                    ("collapsed_text", "collapsedText"),
                ],
            );
            to_lua(lua, &coordinator, &value)
        },
    )?;
    Ok(())
}

fn bind_code_lens(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "create",
            move |lua, options: Option<LuaValue>| {
                let Some(options) = options else {
                    return Ok(LuaValue::Nil);
                };
                to_lua(lua, &coordinator, &from_lua(&coordinator, &options))
            },
        )?;
    }
    set_fn(
        lua,
        module,
        "resolve",
        |_, (lens, resolver): (Option<LuaValue>, Option<Function>)| {
            if let Some(resolver) = resolver {
                if let Ok(value) = resolver.call::<LuaValue>(lens.clone().unwrap_or(LuaValue::Nil))
                {
                    return Ok(value);
                }
            }
            Ok(lens.unwrap_or(LuaValue::Nil))
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-LSP modules
// ---------------------------------------------------------------------------

fn bind_async(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "run", move |lua, callback: Function| {
            let result = callback.call::<LuaValue>(()).unwrap_or(LuaValue::Nil);
            let value = match result {
                LuaValue::Nil => Value::Null,
                LuaValue::Table(ref table) => from_table(&coordinator, table),
                ref value => from_lua(&coordinator, value),
            };
            task_table(lua, &coordinator, value, String::new()).map(LuaValue::Table)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "spawn", move |lua, callback: Function| {
            let result = callback.call::<LuaValue>(()).unwrap_or(LuaValue::Nil);
            let value = match result {
                LuaValue::Nil => Value::Null,
                LuaValue::Table(ref table) => from_table(&coordinator, table),
                ref value => from_lua(&coordinator, value),
            };
            task_table(lua, &coordinator, value, String::new()).map(LuaValue::Table)
        })?;
    }
    set_fn(lua, module, "await", |_, operation: Option<LuaValue>| {
        if let Some(LuaValue::Table(table)) = &operation {
            if let Ok(Some(done)) = table.get::<Option<bool>>("done") {
                if done {
                    if let Ok(function) = table.get::<Function>("result") {
                        let _ = function.call::<()>(());
                    }
                }
            }
        }
        Ok(operation.unwrap_or(LuaValue::Nil))
    })?;
    set_fn(lua, module, "yield", |_, ()| Ok(LuaValue::Nil))?;
    set_fn(lua, module, "sleep", |_, _milliseconds: Option<i64>| {
        Ok(LuaValue::Nil)
    })?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "schedule", move |lua, callback: Function| {
            let result = callback.call::<LuaValue>(()).unwrap_or(LuaValue::Nil);
            let value = match result {
                LuaValue::Nil => Value::Null,
                LuaValue::Table(ref table) => from_table(&coordinator, table),
                ref value => from_lua(&coordinator, value),
            };
            task_table(lua, &coordinator, value, String::new()).map(LuaValue::Table)
        })?;
    }
    set_fn(lua, module, "cancel", |_, _task: Option<LuaValue>| {
        Ok(false)
    })?;
    set_fn(lua, module, "is_cancelled", |_, ()| Ok(false))?;
    set_fn(lua, module, "tick", |_, ()| Ok(true))?;
    set_fn(lua, module, "pump", |_, ()| Ok(true))?;
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "debounce",
            move |lua, (_key, _milliseconds, callback): (String, Option<i64>, Function)| {
                let result = callback.call::<LuaValue>(()).unwrap_or(LuaValue::Nil);
                let value = match result {
                    LuaValue::Nil => Value::Null,
                    LuaValue::Table(ref table) => from_table(&coordinator, table),
                    ref value => from_lua(&coordinator, value),
                };
                task_table(lua, &coordinator, value, String::new()).map(LuaValue::Table)
            },
        )?;
    }
    Ok(())
}

fn bind_json(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    module.set("null", coordinator.borrow().null.clone())?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "encode", move |_, value: Option<LuaValue>| {
            let value = value
                .as_ref()
                .map(|value| from_lua(&coordinator, value))
                .unwrap_or(Value::Null);
            Ok(hosted_json::to_string(&value, false))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "decode",
            move |lua, text: String| match hosted_json::parse(&text) {
                Ok(value) => to_lua(lua, &coordinator, &value),
                Err(_) => Ok(LuaValue::Nil),
            },
        )?;
    }
    set_fn(lua, module, "is_valid", |_, text: String| {
        Ok(hosted_json::parse(&text).is_ok())
    })?;
    {
        let null = coordinator.borrow().null.clone();
        set_fn(lua, module, "is_null", move |_, value: Option<LuaValue>| {
            let Some(value) = value else {
                return Ok(false);
            };
            if let LuaValue::Table(table) = &value {
                if let LuaValue::Table(sentinel) = &null {
                    return Ok(table == sentinel);
                }
            }
            Ok(false)
        })?;
    }
    set_fn(lua, module, "array", |lua, ()| lua.create_table())?;
    set_fn(lua, module, "object", |lua, ()| lua.create_table())?;
    Ok(())
}

fn bind_fs(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    set_fn(lua, module, "read_file", |lua, path: String| {
        Ok(match fs::read_to_string(path) {
            Ok(text) => LuaValue::String(lua.create_string(text)?),
            Err(_) => LuaValue::Nil,
        })
    })?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "read_file_async", move |lua, path: String| {
            let (value, error) = match fs::read_to_string(path) {
                Ok(text) => (Value::String(text), String::new()),
                Err(error) => (Value::Null, error.to_string()),
            };
            task_table(lua, &coordinator, value, error)
        })?;
    }
    set_fn(
        lua,
        module,
        "write_file",
        |_, (path, contents): (String, Option<String>)| {
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            Ok(fs::write(path, contents.unwrap_or_default()).is_ok())
        },
    )?;
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "write_file_async",
            move |lua, (path, contents): (String, Option<String>)| {
                let path = PathBuf::from(path);
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let (value, error) = match fs::write(path, contents.unwrap_or_default()) {
                    Ok(()) => (Value::Bool(true), String::new()),
                    Err(error) => (Value::Null, error.to_string()),
                };
                task_table(lua, &coordinator, value, error)
            },
        )?;
    }
    set_fn(lua, module, "exists", |_, path: String| {
        Ok(Path::new(&path).exists())
    })?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "stat", move |lua, path: String| {
            to_lua(lua, &coordinator, &stat_value(&path))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "stat_async", move |lua, path: String| {
            task_table(lua, &coordinator, stat_value(&path), String::new())
        })?;
    }
    set_fn(lua, module, "realpath", |_, path: String| {
        Ok(std::path::absolute(&path)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or(path))
    })?;
    set_fn(lua, module, "dirname", |_, path: String| {
        Ok(Path::new(&path)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default())
    })?;
    set_fn(lua, module, "basename", |_, path: String| {
        Ok(Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default())
    })?;
    set_fn(lua, module, "normalize", |_, path: String| {
        Ok(std::path::absolute(&path)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or(path))
    })?;
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "glob",
            move |lua, (pattern, _options): (String, Option<LuaValue>)| {
                let root = coordinator.borrow().workspace.root_path.clone();
                let mut files = Vec::new();
                collect_files(Path::new(&root), 4096, &mut files);
                let matcher = GlobMatcher::new(&pattern);
                let mut result = Value::Array(Vec::new());
                for file in files {
                    let relative = Path::new(&file)
                        .strip_prefix(&root)
                        .map(|path| path.to_string_lossy().to_string().replace('\\', "/"))
                        .unwrap_or_else(|_| file.clone());
                    if matcher.matches(&relative) || matcher.matches(&file) {
                        json::push(&mut result, Value::String(file));
                    }
                }
                Ok(match to_lua(lua, &coordinator, &result)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "workspace_files",
            move |lua, options: Option<LuaValue>| {
                let root = coordinator.borrow().workspace.root_path.clone();
                let mut max_results = 20_000usize;
                if let Some(LuaValue::Table(table)) = options {
                    if let Ok(Some(limit)) = table.get::<Option<usize>>("max_results") {
                        max_results = limit.max(1);
                    } else if let Ok(Some(limit)) = table.get::<Option<usize>>("maxResults") {
                        max_results = limit.max(1);
                    }
                }
                let result = Value::Array(
                    collect_workspace_files(&root, max_results)
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                );
                Ok(match to_lua(lua, &coordinator, &result)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    set_fn(
        lua,
        module,
        "mkdir",
        |_, (path, _options): (String, Option<LuaValue>)| Ok(fs::create_dir_all(path).is_ok()),
    )?;
    set_fn(lua, module, "remove", |_, path: String| {
        let path = Path::new(&path);
        Ok(if path.is_dir() {
            fs::remove_dir_all(path).is_ok()
        } else {
            fs::remove_file(path).is_ok()
        })
    })?;
    set_fn(lua, module, "rename", |_, (old, new): (String, String)| {
        Ok(fs::rename(old, new).is_ok())
    })?;
    Ok(())
}

fn stat_value(path: &str) -> Value {
    let mut value = Value::Object(Vec::new());
    value.set("path", Value::String(path.to_string()));
    value.set("uri", Value::String(uri_from_path(path)));
    value.set("exists", Value::Bool(false));
    value.set("directory", Value::Bool(false));
    value.set("file", Value::Bool(false));
    value.set("size", Value::Number(0.0));
    value.set("mtime", Value::Number(0.0));
    let Ok(metadata) = fs::metadata(path) else {
        return value;
    };
    value.set("exists", Value::Bool(true));
    if metadata.is_dir() {
        value.set("directory", Value::Bool(true));
    } else {
        value.set("file", Value::Bool(true));
        value.set("size", Value::Number(metadata.len() as f64));
    }
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            value.set("mtime", Value::Number(duration.as_millis() as f64));
        }
    }
    value
}

fn bind_log(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    let mut levels = Value::Object(Vec::new());
    for (name, number) in [
        ("TRACE", 0.0),
        ("DEBUG", 1.0),
        ("INFO", 2.0),
        ("WARN", 3.0),
        ("ERROR", 4.0),
    ] {
        levels.set(name, Value::Number(number));
    }
    module.set("level", to_lua(lua, coordinator, &levels)?)?;
    for (name, _prefix) in [
        ("trace", "trace"),
        ("debug", "debug"),
        ("info", "info"),
        ("warn", "warn"),
        ("error", "error"),
    ] {
        set_fn(
            lua,
            module,
            name,
            |_, (_message, _fields): (String, Option<LuaValue>)| Ok(true),
        )?;
    }
    set_fn(lua, module, "set_level", |_, _level: Option<LuaValue>| {
        Ok(true)
    })?;
    set_fn(
        lua,
        module,
        "set_destination",
        |_, _path: Option<String>| Ok(true),
    )?;
    Ok(())
}

fn bind_index(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    let create = {
        let coordinator = coordinator.clone();
        lua.create_function(move |lua, _options: Option<LuaValue>| {
            let index = Rc::new(RefCell::new(IndexState::new()));
            coordinator.borrow_mut().indexes.push(index.clone());
            index_table(lua, &coordinator, index).map(LuaValue::Table)
        })?
    };
    module.set("create", create.clone())?;
    module.set("new", create)?;
    Ok(())
}

fn bind_process(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "run",
            move |lua, (command, args, _options): (String, Option<LuaValue>, Option<LuaValue>)| {
                if command.trim().is_empty() {
                    return Ok(LuaValue::Table(task_table(
                        lua,
                        &coordinator,
                        Value::Null,
                        "empty command".into(),
                    )?));
                }
                let args = args
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let output = std::process::Command::new(&command)
                    .args(json::as_array(&args).iter().filter_map(Value::as_str))
                    .output();
                let (value, error) = match output {
                    Ok(output) => {
                        let mut result = Value::Object(Vec::new());
                        result.set(
                            "code",
                            Value::Number(output.status.code().unwrap_or(-1) as f64),
                        );
                        result.set(
                            "stdout",
                            Value::String(String::from_utf8_lossy(&output.stdout).into_owned()),
                        );
                        result.set(
                            "stderr",
                            Value::String(String::from_utf8_lossy(&output.stderr).into_owned()),
                        );
                        (result, String::new())
                    }
                    Err(error) => (Value::Null, error.to_string()),
                };
                Ok(LuaValue::Table(task_table(
                    lua,
                    &coordinator,
                    value,
                    error,
                )?))
            },
        )?;
    }
    set_fn(lua, module, "is_allowed", |_, ()| Ok(true))?;
    Ok(())
}

fn bind_debug(lua: &Lua, module: &Table, coordinator: &Rc<RefCell<Coordinator>>) -> LuaResult<()> {
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            module,
            "registered_handlers",
            move |lua, server_value: Option<LuaValue>| {
                let name = server_value
                    .as_ref()
                    .and_then(|value| match value {
                        LuaValue::Table(table) => table
                            .get::<Option<String>>("__hexal_server_name")
                            .ok()
                            .flatten(),
                        _ => None,
                    })
                    .unwrap_or_default();
                let server = if name.is_empty() {
                    coordinator.borrow().active_server()
                } else {
                    coordinator.borrow().find_server(&name)
                };
                let mut result = Value::Object(Vec::new());
                if let Some(server) = server {
                    for method in server.borrow().registered_requests() {
                        result.set(&method, Value::Bool(true));
                    }
                    for method in server.borrow().registered_notifications() {
                        result.set(&method, Value::Bool(true));
                    }
                }
                Ok(match to_lua(lua, &coordinator, &result)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "capabilities", move |lua, ()| {
            let value = coordinator
                .borrow()
                .active_server()
                .map(|server| server.borrow().server_capabilities.clone())
                .unwrap_or(Value::Object(Vec::new()));
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "documents", move |lua, ()| {
            let mut result = Value::Object(Vec::new());
            for document in &coordinator.borrow().documents {
                result.set(&document.uri, Value::Number(document.version as f64));
            }
            Ok(match to_lua(lua, &coordinator, &result)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "pending_requests", move |lua, ()| {
            let mut result = Value::Array(Vec::new());
            if let Some(server) = coordinator.borrow().active_server() {
                for id in server.borrow().pending.keys() {
                    json::push(&mut result, Value::Number(*id as f64));
                }
            }
            Ok(match to_lua(lua, &coordinator, &result)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "active_tasks", move |lua, ()| {
            Ok(
                match to_lua(lua, &coordinator, &Value::Object(Vec::new()))? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                },
            )
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "index_stats", move |lua, ()| {
            let state = coordinator.borrow();
            let mut result = Value::Object(Vec::new());
            result.set("documents", Value::Number(state.documents.len() as f64));
            result.set("diagnostics", Value::Number(state.diagnostics.len() as f64));
            result.set("servers", Value::Number(state.servers.len() as f64));
            Ok(match to_lua(lua, &coordinator, &result)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "set_trace", move |_, level: Option<String>| {
            coordinator.borrow_mut().trace = level.is_some();
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, module, "inspect", move |lua, ()| {
            let state = coordinator.borrow();
            let mut result = Value::Object(Vec::new());
            result.set("servers", Value::Number(state.servers.len() as f64));
            result.set("documents", Value::Number(state.documents.len() as f64));
            result.set("modules", Value::Number(state.modules.len() as f64));
            Ok(match to_lua(lua, &coordinator, &result)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    Ok(())
}

fn bind_testing(
    lua: &Lua,
    module: &Table,
    coordinator: &Rc<RefCell<Coordinator>>,
) -> LuaResult<()> {
    let coordinator = coordinator.clone();
    set_fn(
        lua,
        module,
        "create_client",
        move |lua, server_value: Option<LuaValue>| {
            let name = server_value
                .as_ref()
                .and_then(|value| match value {
                    LuaValue::Table(table) => table
                        .get::<Option<String>>("__hexal_server_name")
                        .ok()
                        .flatten(),
                    _ => None,
                })
                .unwrap_or_default();
            let id = if name.is_empty() {
                coordinator.borrow().active_server().and_then(|server| {
                    coordinator
                        .borrow()
                        .servers
                        .iter()
                        .position(|candidate| Rc::ptr_eq(candidate, &server))
                })
            } else {
                coordinator
                    .borrow()
                    .servers
                    .iter()
                    .position(|server| server.borrow().name == name)
            };
            let Some(id) = id else {
                return Ok(LuaValue::Nil);
            };
            let client = test_client_table(lua, &coordinator, id)?;
            Ok(LuaValue::Table(client))
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// In-memory test client
// ---------------------------------------------------------------------------

fn test_client_table(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
) -> LuaResult<Table> {
    let state = Rc::new(RefCell::new(ClientState::new()));
    let table = lua.create_table()?;

    {
        let coordinator = coordinator.clone();
        let state = state.clone();
        set_fn(
            lua,
            &table,
            "initialize",
            move |lua, options: Option<LuaValue>| {
                let server = server_by_id(&coordinator, server_id);
                let workspace = server
                    .as_ref()
                    .map(|server| server.borrow().workspace.clone())
                    .unwrap_or_else(|| coordinator.borrow().workspace.clone());
                let mut params = Value::Object(Vec::new());
                params.set("processId", Value::Number(std::process::id() as f64));
                let mut client_info = Value::Object(Vec::new());
                client_info.set("name", Value::String("hexal-testing".into()));
                params.set("clientInfo", client_info);
                params.set("rootUri", Value::String(workspace.root_uri.clone()));
                let mut folders = Value::Array(Vec::new());
                for (uri, name) in workspace.folders() {
                    let mut folder = Value::Object(Vec::new());
                    folder.set("uri", Value::String(uri));
                    folder.set("name", Value::String(name));
                    json::push(&mut folders, folder);
                }
                params.set("workspaceFolders", folders);
                let mut capabilities = Value::Object(Vec::new());
                let mut general = Value::Object(Vec::new());
                general.set(
                    "positionEncodings",
                    Value::Array(vec![
                        Value::String("utf-16".into()),
                        Value::String("utf-8".into()),
                        Value::String("utf-32".into()),
                    ]),
                );
                capabilities.set("general", general);
                let mut text_document = Value::Object(Vec::new());
                let mut hover = Value::Object(Vec::new());
                hover.set(
                    "contentFormat",
                    Value::Array(vec![
                        Value::String("markdown".into()),
                        Value::String("plaintext".into()),
                    ]),
                );
                text_document.set("hover", hover);
                let mut completion = Value::Object(Vec::new());
                let mut completion_item = Value::Object(Vec::new());
                completion_item.set("snippetSupport", Value::Bool(true));
                completion.set("completionItem", completion_item);
                text_document.set("completion", completion);
                let mut diagnostics = Value::Object(Vec::new());
                diagnostics.set("relatedInformation", Value::Bool(true));
                diagnostics.set("versionSupport", Value::Bool(true));
                text_document.set("publishDiagnostics", diagnostics);
                capabilities.set("textDocument", text_document);
                let mut window = Value::Object(Vec::new());
                window.set("workDoneProgress", Value::Bool(true));
                capabilities.set("window", window);
                params.set("capabilities", capabilities);
                if let Some(options) = options {
                    if let LuaValue::Table(extra) = options {
                        for pair in extra.pairs::<String, LuaValue>() {
                            if let Ok((key, value)) = pair {
                                params.set(&key, from_lua(&coordinator, &value));
                            }
                        }
                    }
                }
                match dispatch_request(lua, &coordinator, server_id, "initialize", params) {
                    Ok(result) => {
                        state.borrow_mut().initialized = true;
                        dispatch_notification(
                            lua,
                            &coordinator,
                            server_id,
                            "initialized",
                            Value::Object(Vec::new()),
                        );
                        match to_lua(lua, &coordinator, &result)? {
                            LuaValue::Table(table) => Ok(LuaValue::Table(table)),
                            _ => Ok(LuaValue::Nil),
                        }
                    }
                    Err(_) => Ok(LuaValue::Nil),
                }
            },
        )?;
    }
    {
        let state = state.clone();
        set_fn(lua, &table, "initialized", move |_, ()| {
            Ok(state.borrow().initialized)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        let state = state.clone();
        set_fn(
            lua,
            &table,
            "open_document",
            move |lua, (uri, language, text, version): (String, String, String, Option<i64>)| {
                let version = version.unwrap_or(1);
                let mut params = Value::Object(Vec::new());
                let mut document = Value::Object(Vec::new());
                document.set("uri", Value::String(uri.clone()));
                document.set("languageId", Value::String(language));
                document.set("version", Value::Number(version as f64));
                document.set("text", Value::String(text));
                params.set("textDocument", document);
                dispatch_notification(lua, &coordinator, server_id, "textDocument/didOpen", params);
                state.borrow_mut().versions.insert(uri, version);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        let state = state.clone();
        set_fn(
            lua,
            &table,
            "change_document",
            move |lua, (uri, changes, version): (String, Option<LuaValue>, Option<i64>)| {
                let version = version
                    .unwrap_or_else(|| state.borrow().versions.get(&uri).copied().unwrap_or(0) + 1);
                let changes = changes
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Array(Vec::new()));
                let mut params = Value::Object(Vec::new());
                let mut document = Value::Object(Vec::new());
                document.set("uri", Value::String(uri.clone()));
                document.set("version", Value::Number(version as f64));
                params.set("textDocument", document);
                params.set("contentChanges", changes);
                dispatch_notification(
                    lua,
                    &coordinator,
                    server_id,
                    "textDocument/didChange",
                    params,
                );
                state.borrow_mut().versions.insert(uri, version);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "save_document",
            move |lua, (uri, text): (String, Option<String>)| {
                let mut params = Value::Object(Vec::new());
                let mut document = Value::Object(Vec::new());
                document.set("uri", Value::String(uri));
                params.set("textDocument", document);
                if let Some(text) = text {
                    params.set("text", Value::String(text));
                }
                dispatch_notification(lua, &coordinator, server_id, "textDocument/didSave", params);
                Ok(true)
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        let state = state.clone();
        set_fn(lua, &table, "close_document", move |lua, uri: String| {
            let mut params = Value::Object(Vec::new());
            let mut document = Value::Object(Vec::new());
            document.set("uri", Value::String(uri.clone()));
            params.set("textDocument", document);
            dispatch_notification(
                lua,
                &coordinator,
                server_id,
                "textDocument/didClose",
                params,
            );
            state.borrow_mut().versions.remove(&uri);
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "request",
            move |lua, (method, params): (String, Option<LuaValue>)| {
                let params = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                match dispatch_request(lua, &coordinator, server_id, &method, params) {
                    Ok(result) => match to_lua(lua, &coordinator, &result)? {
                        LuaValue::Table(table) => Ok(LuaValue::Table(table)),
                        _ => Ok(LuaValue::Nil),
                    },
                    Err(_) => Ok(LuaValue::Nil),
                }
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "request_raw",
            move |lua, (method, params): (String, Option<LuaValue>)| {
                let params = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                let (result, error) =
                    match dispatch_request(lua, &coordinator, server_id, &method, params) {
                        Ok(result) => (result, String::new()),
                        Err(error) => (Value::Null, error.message),
                    };
                let mut value = Value::Object(Vec::new());
                value.set("result", result);
                value.set("error", Value::String(error.clone()));
                value.set("ok", Value::Bool(error.is_empty()));
                Ok(match to_lua(lua, &coordinator, &value)? {
                    LuaValue::Table(table) => table,
                    _ => lua.create_table()?,
                })
            },
        )?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(
            lua,
            &table,
            "notify",
            move |lua, (method, params): (String, Option<LuaValue>)| {
                let params = params
                    .as_ref()
                    .map(|value| from_lua(&coordinator, value))
                    .unwrap_or(Value::Null);
                dispatch_notification(lua, &coordinator, server_id, &method, params);
                Ok(true)
            },
        )?;
    }
    set_fn(lua, &table, "cancel", |_, _id: Option<LuaValue>| Ok(true))?;
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "diagnostics", move |lua, uri: String| {
            let items = coordinator.borrow().diagnostics(&uri);
            Ok(match to_lua(lua, &coordinator, &items)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let state = state.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "notifications", move |lua, ()| {
            let items = state.borrow().notifications.clone();
            let value = Value::Array(items);
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let state = state.clone();
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "messages", move |lua, ()| {
            let items = state.borrow().notifications.clone();
            let value = Value::Array(items);
            Ok(match to_lua(lua, &coordinator, &value)? {
                LuaValue::Table(table) => table,
                _ => lua.create_table()?,
            })
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "server", move |lua, ()| {
            Ok(LuaValue::Table(server_table(lua, &coordinator, server_id)?))
        })?;
    }
    {
        let coordinator = coordinator.clone();
        let state = state.clone();
        set_fn(lua, &table, "tick", move |lua, ()| {
            drain_outgoing(lua, &coordinator, server_id, &state);
            Ok(true)
        })?;
    }
    {
        let coordinator = coordinator.clone();
        set_fn(lua, &table, "shutdown", move |lua, ()| {
            let result = dispatch_request(lua, &coordinator, server_id, "shutdown", Value::Null);
            dispatch_notification(
                lua,
                &coordinator,
                server_id,
                "exit",
                Value::Object(vec![("code".into(), Value::Number(0.0))]),
            );
            Ok(result.is_ok())
        })?;
    }
    set_fn(lua, &table, "set_outgoing", |_, ()| Ok(true))?;
    wrap(lua, &table);
    Ok(table)
}

fn drain_outgoing(
    lua: &Lua,
    coordinator: &Rc<RefCell<Coordinator>>,
    server_id: usize,
    state: &Rc<RefCell<ClientState>>,
) {
    loop {
        let Some(server) = server_by_id(coordinator, server_id) else {
            return;
        };
        let message = server.borrow().outgoing.borrow_mut().first().cloned();
        let Some(message) = message else {
            return;
        };
        server.borrow().outgoing.borrow_mut().remove(0);
        let method = json::get_str(message.get("method"), "");
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        if !method.is_empty() {
            if matches!(id, Value::Null) {
                state.borrow_mut().notifications.push(message);
            } else {
                let result = auto_client_result(&method, &message, &server);
                let response = response_value(&id, result);
                handle_incoming(lua, coordinator, server_id, response);
            }
            continue;
        }
        if !matches!(id, Value::Null) {
            handle_incoming(lua, coordinator, server_id, message);
        }
    }
}

fn auto_client_result(method: &str, message: &Value, server: &Rc<RefCell<Server>>) -> Value {
    match method {
        "workspace/configuration" => {
            let count = message
                .get("params")
                .and_then(|params| params.get("items"))
                .map(json::as_array)
                .map(<[Value]>::len)
                .unwrap_or(0);
            Value::Array((0..count).map(|_| Value::Null).collect())
        }
        "workspace/workspaceFolders" => {
            let workspace = server.borrow().workspace.clone();
            let mut folders = Value::Array(Vec::new());
            for (uri, name) in workspace.folders() {
                let mut folder = Value::Object(Vec::new());
                folder.set("uri", Value::String(uri));
                folder.set("name", Value::String(name));
                json::push(&mut folders, folder);
            }
            folders
        }
        "workspace/applyEdit" => {
            let mut result = Value::Object(Vec::new());
            result.set("applied", Value::Bool(true));
            result
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::collect_workspace_files;
    use std::fs;

    #[test]
    fn workspace_files_are_relative_to_the_workspace_root() {
        let root =
            std::env::temp_dir().join(format!("hexal-workspace-files-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("README.md"), b"readme").unwrap();
        fs::write(root.join("src/main.rs"), b"main").unwrap();
        fs::write(root.join("src/nested/lib.rs"), b"lib").unwrap();

        let files = collect_workspace_files(&root.to_string_lossy(), 100);
        assert_eq!(
            files,
            vec![
                "README.md".to_string(),
                "src/main.rs".to_string(),
                "src/nested/lib.rs".to_string()
            ]
        );

        let _ = fs::remove_dir_all(&root);
    }
}
