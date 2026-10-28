//! Minimal asynchronous language-server client: JSON-RPC over stdio,
//! initialization, full-document synchronization, diagnostics, hover,
//! definitions, completions, code actions and document symbols.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::util::json::{self, Value};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub arguments: Vec<String>,
    pub language_id: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub line: i64,
    pub column: i64,
    pub severity: i64,
    pub message: String,
    pub source: String,
    pub end_line: i64,
    pub end_column: i64,
    pub tags: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct TextEdit {
    pub uri: String,
    pub start_line: i64,
    pub start_character: i64,
    pub end_line: i64,
    pub end_character: i64,
    pub new_text: String,
}

#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    pub kind: String,
    pub edits: Vec<TextEdit>,
    pub command: String,
    pub command_arguments: Option<Value>,
    pub needs_resolve: bool,
    pub raw: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub insert_text_format: i64,
    pub detail: String,
    pub filter_text: String,
    pub has_text_edit: bool,
    pub start_line: i64,
    pub start_character: i64,
    pub end_line: i64,
    pub end_character: i64,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub uri: String,
    pub line: i64,
    pub character: i64,
}

#[derive(Clone, Debug)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: String,
    pub kind: i64,
    pub uri: String,
    pub line: i64,
    pub character: i64,
    pub depth: usize,
}

#[derive(Clone, Debug)]
pub struct DocumentDiagnostics {
    pub uri: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Started,
    Initialized,
    Diagnostics {
        #[allow(dead_code)]
        diagnostics: Vec<Diagnostic>,
    },
    Error {
        server: String,
        message: String,
    },
    Stopped,
    Hover {
        text: String,
    },
    Definition {
        locations: Vec<Location>,
    },
    CodeActions {
        actions: Vec<CodeAction>,
    },
    CodeActionResolved {
        actions: Vec<CodeAction>,
    },
    WorkspaceDiagnostics,
    DocumentSymbols {
        symbols: Vec<DocumentSymbol>,
    },
    Completions {
        request_id: i64,
        items: Vec<CompletionItem>,
    },
}

enum Incoming {
    Message(Value),
    Stderr(String),
    Closed,
}

pub struct Manager {
    child: Option<Child>,
    outgoing: Option<Sender<Vec<u8>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    incoming: Option<Receiver<Incoming>>,
    reader: Option<std::thread::JoinHandle<()>>,
    stderr_reader: Option<std::thread::JoinHandle<()>>,
    config: Option<ServerConfig>,
    document_uri: String,
    root_path: String,
    root_uri: String,
    current_text: String,
    version: i64,
    next_request_id: i64,
    initialize_id: i64,
    initialized: bool,
    workspace_diagnostics_supported: bool,
    completion_triggers: HashSet<String>,
    pending: HashMap<i64, String>,
    diagnostics: HashMap<String, Vec<Diagnostic>>,
    log: VecDeque<String>,
    dropped_log: usize,
    pub events: Vec<Event>,
}

impl Default for Manager {
    fn default() -> Self {
        Manager {
            child: None,
            outgoing: None,
            writer: None,
            incoming: None,
            reader: None,
            stderr_reader: None,
            config: None,
            document_uri: String::new(),
            root_path: String::new(),
            root_uri: String::new(),
            current_text: String::new(),
            version: 0,
            next_request_id: 0,
            initialize_id: 0,
            initialized: false,
            workspace_diagnostics_supported: false,
            completion_triggers: HashSet::new(),
            pending: HashMap::new(),
            diagnostics: HashMap::new(),
            log: VecDeque::new(),
            dropped_log: 0,
            events: Vec::new(),
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Manager {
    pub fn new() -> Manager {
        Manager::default()
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn server_name(&self) -> String {
        self.config
            .as_ref()
            .map(|config| config.name.clone())
            .unwrap_or_default()
    }

    pub fn is_completion_trigger(&self, character: char) -> bool {
        let mut buffer = [0u8; 4];
        self.completion_triggers
            .contains(character.encode_utf8(&mut buffer))
    }

    #[cfg(test)]
    pub fn uri_for_test(&self) -> String {
        self.document_uri.clone()
    }

    #[allow(dead_code)]
    pub fn is_serving(&self, path: &str) -> bool {
        if self.child.is_none() {
            return false;
        }
        path_to_uri(path, false).is_ok_and(|uri| uri == self.document_uri)
    }

    pub fn count_diagnostics_for_path(&self, path: &str) -> usize {
        if path.is_empty() {
            return 0;
        }
        let Ok(uri) = path_to_uri(path, false) else {
            return 0;
        };
        self.diagnostics.get(&uri).map(Vec::len).unwrap_or(0)
    }

    pub fn diagnostics_for_path(&self, path: &str) -> Vec<Diagnostic> {
        let Ok(uri) = path_to_uri(path, false) else {
            return Vec::new();
        };
        self.diagnostics.get(&uri).cloned().unwrap_or_default()
    }

    pub fn snapshot_diagnostics(&self) -> Vec<DocumentDiagnostics> {
        self.diagnostics
            .iter()
            .map(|(uri, diagnostics)| DocumentDiagnostics {
                uri: uri.clone(),
                diagnostics: diagnostics.clone(),
            })
            .collect()
    }

    /// Starts a server, or reuses the running one when it serves the same
    /// workspace with the same executable.
    pub fn try_start(
        &mut self,
        config: ServerConfig,
        document_path: &str,
        text: &str,
        workspace_root: &str,
    ) -> Result<(), String> {
        if config.command.trim().is_empty() {
            return Err("language server command is empty".into());
        }
        if document_path.is_empty() {
            return Err("language servers require a file-backed buffer".into());
        }
        let full_path = absolute_path(Path::new(document_path));
        let working_directory = if !workspace_root.is_empty() && Path::new(workspace_root).is_dir()
        {
            absolute_path(Path::new(workspace_root))
        } else {
            full_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        let document_uri =
            path_to_uri(&full_path.to_string_lossy(), false).unwrap_or_else(|_| String::new());

        if self.child.is_some() {
            if self.can_reuse(&config, &working_directory) {
                if self.document_uri == document_uri {
                    return Ok(());
                }
                self.switch_document(
                    &config,
                    document_path,
                    &document_uri,
                    &working_directory,
                    text,
                );
                return Ok(());
            }
            self.stop();
        }

        if !crate::util::git::command_exists(&config.command) {
            return Err(format!(
                "language server not found in PATH: {}",
                config.command
            ));
        }

        self.root_path = working_directory.to_string_lossy().into_owned();
        self.root_uri = path_to_uri(&self.root_path, true).unwrap_or_default();
        self.document_uri = document_uri;
        self.version = 1;
        self.initialized = false;
        self.workspace_diagnostics_supported = false;
        self.completion_triggers.clear();
        self.pending.clear();
        self.diagnostics.clear();
        self.current_text = text.to_string();
        self.config = Some(config.clone());
        self.next_request_id = self.next_request_id.max(1);
        self.initialize_id = self.allocate_id();
        self.pending.insert(self.initialize_id, "initialize".into());

        let mut command = Command::new(&config.command);
        command
            .args(&config.arguments)
            .current_dir(&self.root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| {
            format!("cannot start language server '{}': {error}", config.command)
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let input = child.stdin.take();
        self.child = Some(child);

        // Writes happen on a dedicated thread so a slow or unresponsive
        // language server can never stall the editor's input loop.
        let (outgoing, writer_input) = mpsc::channel::<Vec<u8>>();
        self.outgoing = Some(outgoing);
        if let Some(mut input) = input {
            self.writer = Some(std::thread::spawn(move || {
                while let Ok(frame) = writer_input.recv() {
                    if input.write_all(&frame).is_err() {
                        break;
                    }
                }
                let _ = input.flush();
            }));
        }

        let (sender, receiver) = mpsc::channel();
        self.incoming = Some(receiver);
        if let Some(stdout) = stdout {
            let sender = sender.clone();
            self.reader = Some(std::thread::spawn(move || {
                read_loop(stdout, sender);
            }));
        }
        if let Some(stderr) = stderr {
            let sender = sender.clone();
            self.stderr_reader = Some(std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stderr);
                let mut buffer = String::new();
                loop {
                    buffer.clear();
                    match std::io::BufRead::read_line(&mut reader, &mut buffer) {
                        Ok(0) => break,
                        Ok(_) => {
                            let line = buffer.trim_end_matches(['\r', '\n']).to_string();
                            if !line.is_empty() && sender.send(Incoming::Stderr(line)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }

        self.log(format!(
            "starting {}: {} (cwd {})",
            config.name,
            display_path(&config.command),
            self.root_path
        ));
        self.send_initialize();
        self.events.push(Event::Started);
        Ok(())
    }

    fn can_reuse(&self, config: &ServerConfig, working_directory: &Path) -> bool {
        let Some(current) = &self.config else {
            return false;
        };
        if !current.command.eq_ignore_ascii_case(&config.command) {
            return false;
        }
        match (
            absolute_path(Path::new(&self.root_path)),
            absolute_path(working_directory),
        ) {
            (left, right) => left == right,
        }
    }

    fn switch_document(
        &mut self,
        config: &ServerConfig,
        document_path: &str,
        document_uri: &str,
        working_directory: &Path,
        text: &str,
    ) {
        if self.initialized && !self.document_uri.is_empty() {
            self.send_notification(
                "textDocument/didClose",
                object(vec![(
                    "textDocument",
                    object(vec![("uri", string(&self.document_uri))]),
                )]),
            );
            self.diagnostics.remove(&self.document_uri);
        }
        let _ = document_path;
        self.config = Some(config.clone());
        self.root_path = working_directory.to_string_lossy().into_owned();
        self.root_uri = path_to_uri(&self.root_path, true).unwrap_or_default();
        self.document_uri = document_uri.to_string();
        self.current_text = text.to_string();
        self.version = 1;
        self.pending.clear();

        if self.initialized {
            self.send_notification("textDocument/didOpen", self.did_open_params());
        }
    }

    pub fn did_change(&mut self, text: &str) {
        if self.child.is_none() || !self.initialized {
            self.current_text = text.to_string();
            return;
        }
        self.current_text = text.to_string();
        self.version += 1;
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![
                    ("uri", string(&self.document_uri)),
                    ("version", number(self.version)),
                ]),
            ),
            (
                "contentChanges",
                array(vec![object(vec![("text", string(text))])]),
            ),
        ]);
        self.send_notification("textDocument/didChange", parameters);
    }

    pub fn did_save(&mut self, text: &str) {
        if self.child.is_none() || !self.initialized {
            return;
        }
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![("uri", string(&self.document_uri))]),
            ),
            ("text", string(text)),
        ]);
        self.send_notification("textDocument/didSave", parameters);
    }

    fn can_request(&self, document_path: &str) -> bool {
        if self.child.is_none() || !self.initialized {
            return false;
        }
        path_to_uri(document_path, false).is_ok_and(|uri| uri == self.document_uri)
    }

    pub fn request_hover(&mut self, document_path: &str, line: i64, character: i64) -> bool {
        if !self.can_request(document_path) {
            return false;
        }
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![("uri", string(&self.document_uri))]),
            ),
            (
                "position",
                object(vec![
                    ("line", number(line)),
                    ("character", number(character)),
                ]),
            ),
        ]);
        self.send_request("textDocument/hover", parameters);
        true
    }

    pub fn request_definition(&mut self, document_path: &str, line: i64, character: i64) -> bool {
        if !self.can_request(document_path) {
            return false;
        }
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![("uri", string(&self.document_uri))]),
            ),
            (
                "position",
                object(vec![
                    ("line", number(line)),
                    ("character", number(character)),
                ]),
            ),
        ]);
        self.send_request("textDocument/definition", parameters);
        true
    }

    pub fn request_document_symbols(&mut self, document_path: &str) -> bool {
        if !self.can_request(document_path) {
            return false;
        }
        let parameters = object(vec![(
            "textDocument",
            object(vec![("uri", string(&self.document_uri))]),
        )]);
        self.send_request("textDocument/documentSymbol", parameters);
        true
    }

    pub fn request_completion(
        &mut self,
        document_path: &str,
        line: i64,
        character: i64,
        trigger: Option<char>,
    ) -> Option<i64> {
        if !self.can_request(document_path) {
            return None;
        }
        let mut context = vec![("triggerKind", number(if trigger.is_some() { 2 } else { 1 }))];
        if let Some(character) = trigger {
            context.push(("triggerCharacter", string(&character.to_string())));
        }
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![("uri", string(&self.document_uri))]),
            ),
            (
                "position",
                object(vec![
                    ("line", number(line)),
                    ("character", number(character)),
                ]),
            ),
            ("context", object(context)),
        ]);
        Some(self.send_request("textDocument/completion", parameters))
    }

    pub fn request_code_actions(
        &mut self,
        document_path: &str,
        line: i64,
        character: i64,
        diagnostics: &[Diagnostic],
    ) -> bool {
        if !self.can_request(document_path) {
            return false;
        }
        let mut context_diagnostics = Vec::new();
        for diagnostic in diagnostics {
            if diagnostic.line != line {
                continue;
            }
            let end_line = if diagnostic.end_line >= 0 {
                diagnostic.end_line
            } else {
                diagnostic.line
            };
            let end_column = if diagnostic.end_column >= 0 {
                diagnostic.end_column
            } else {
                diagnostic.column + 1
            };
            let mut entry = vec![
                (
                    "range",
                    object(vec![
                        (
                            "start",
                            object(vec![
                                ("line", number(diagnostic.line)),
                                ("character", number(diagnostic.column)),
                            ]),
                        ),
                        (
                            "end",
                            object(vec![
                                ("line", number(end_line)),
                                ("character", number(end_column)),
                            ]),
                        ),
                    ]),
                ),
                ("severity", number(diagnostic.severity)),
                ("source", string(&diagnostic.source)),
                ("message", string(&diagnostic.message)),
            ];
            if !diagnostic.tags.is_empty() {
                entry.push((
                    "tags",
                    array(diagnostic.tags.iter().map(|tag| number(*tag)).collect()),
                ));
            }
            context_diagnostics.push(object(entry));
        }
        let parameters = object(vec![
            (
                "textDocument",
                object(vec![("uri", string(&self.document_uri))]),
            ),
            (
                "range",
                object(vec![
                    (
                        "start",
                        object(vec![
                            ("line", number(line)),
                            ("character", number(character)),
                        ]),
                    ),
                    (
                        "end",
                        object(vec![
                            ("line", number(line)),
                            ("character", number(character)),
                        ]),
                    ),
                ]),
            ),
            (
                "context",
                object(vec![("diagnostics", array(context_diagnostics))]),
            ),
        ]);
        self.send_request("textDocument/codeAction", parameters);
        true
    }

    pub fn resolve_code_action(&mut self, action: &CodeAction) -> bool {
        let Some(raw) = &action.raw else {
            return false;
        };
        if self.child.is_none() || !self.initialized {
            return false;
        }
        self.send_request("codeAction/resolve", raw.clone());
        true
    }

    pub fn request_workspace_diagnostics(&mut self) -> bool {
        if self.child.is_none() || !self.initialized || !self.workspace_diagnostics_supported {
            return false;
        }
        self.send_request(
            "workspace/diagnostic",
            object(vec![("previousResultIds", array(Vec::new()))]),
        );
        true
    }

    pub fn execute_command(&mut self, command: &str, arguments: Option<&Value>) {
        if command.is_empty() || self.child.is_none() || !self.initialized {
            return;
        }
        self.send_request(
            "workspace/executeCommand",
            object(vec![
                ("command", string(command)),
                (
                    "arguments",
                    arguments.cloned().unwrap_or_else(|| array(Vec::new())),
                ),
            ]),
        );
    }

    /// Drains incoming traffic and returns true when the screen should repaint.
    pub fn poll(&mut self, timeout: Duration) -> bool {
        let mut redraw = false;
        let mut received = false;
        let started = Instant::now();
        loop {
            let message = {
                let Some(receiver) = &self.incoming else {
                    break;
                };
                match receiver.try_recv() {
                    Ok(message) => Some(message),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => None,
                }
            };
            let Some(message) = message else {
                if received || started.elapsed() >= timeout {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            received = true;
            redraw = true;
            match message {
                Incoming::Message(value) => self.handle_message(value),
                Incoming::Stderr(line) => {
                    self.log(format!("stderr {line}"));
                }
                Incoming::Closed => {
                    self.events.push(Event::Stopped);
                    self.initialized = false;
                }
            }
        }
        redraw
    }

    fn handle_message(&mut self, message: Value) {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = message.get("id").cloned();

        if let Some(method) = method {
            if let Some(id) = id {
                self.handle_server_request(&method, &id, &message);
            } else {
                self.handle_notification(&method, &message);
            }
            return;
        }

        let Some(id) = id.and_then(|id| id.as_i64()) else {
            return;
        };
        if id == self.initialize_id {
            self.handle_initialize_response(&message);
            return;
        }
        if let Some(method) = self.pending.remove(&id) {
            self.handle_response(&method, &message, id);
        }
    }

    fn handle_initialize_response(&mut self, message: &Value) {
        self.pending.remove(&self.initialize_id);
        if self.child.is_none() {
            return;
        }
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("initialize failed")
                .to_string();
            self.events.push(Event::Error {
                server: self.server_name(),
                message: text,
            });
            return;
        }

        if let Some(result) = message.get("result") {
            if let Some(capabilities) = result.get("capabilities") {
                if let Some(provider) = capabilities.get("diagnosticProvider") {
                    if matches!(provider, Value::Object(_))
                        && matches!(
                            provider.get("workspaceDiagnostics"),
                            Some(Value::Bool(true))
                        )
                    {
                        self.workspace_diagnostics_supported = true;
                    }
                }
                if let Some(completion) = capabilities.get("completionProvider") {
                    if let Some(triggers) = completion.get("triggerCharacters") {
                        if let Some(triggers) = triggers.as_array() {
                            for trigger in triggers {
                                if let Some(value) = trigger.as_str() {
                                    if !value.is_empty() {
                                        self.completion_triggers.insert(value.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.initialized = true;
        self.send_notification("initialized", object(Vec::new()));
        self.send_notification("textDocument/didOpen", self.did_open_params());
        self.events.push(Event::Initialized);
    }

    fn did_open_params(&self) -> Value {
        object(vec![(
            "textDocument",
            object(vec![
                ("uri", string(&self.document_uri)),
                (
                    "languageId",
                    string(
                        self.config
                            .as_ref()
                            .map(|config| config.language_id.as_str())
                            .unwrap_or(""),
                    ),
                ),
                ("version", number(self.version)),
                ("text", string(&self.current_text)),
            ]),
        )])
    }

    fn handle_response(&mut self, method: &str, message: &Value, request_id: i64) {
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(method)
                .to_string();
            if method != "workspace/diagnostic" {
                self.events.push(Event::Error {
                    server: self.server_name(),
                    message: text,
                });
            }
            return;
        }
        let result = message.get("result");
        match method {
            "textDocument/hover" => {
                let text = result.map(extract_hover_text).unwrap_or_default();
                self.events.push(Event::Hover { text });
            }
            "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/typeDefinition" => {
                let locations = result.map(parse_locations).unwrap_or_default();
                self.events.push(Event::Definition { locations });
            }
            "textDocument/documentSymbol" => {
                let symbols = result
                    .map(|result| parse_document_symbols(result, &self.document_uri))
                    .unwrap_or_default();
                self.events.push(Event::DocumentSymbols { symbols });
            }
            "textDocument/completion" => {
                let items = result.map(parse_completion_items).unwrap_or_default();
                self.events.push(Event::Completions { request_id, items });
            }
            "textDocument/codeAction" => {
                let actions = result.map(parse_code_actions).unwrap_or_default();
                self.events.push(Event::CodeActions { actions });
            }
            "codeAction/resolve" => {
                let actions = result.map(parse_code_actions).unwrap_or_default();
                self.events.push(Event::CodeActionResolved { actions });
            }
            "workspace/diagnostic" => {
                let reports = result.map(parse_workspace_diagnostics).unwrap_or_default();
                for report in &reports {
                    self.diagnostics
                        .insert(report.uri.clone(), report.diagnostics.clone());
                }
                self.events.push(Event::WorkspaceDiagnostics);
            }
            _ => {}
        }
    }

    fn handle_notification(&mut self, method: &str, message: &Value) {
        let Some(parameters) = message.get("params") else {
            return;
        };
        match method {
            "textDocument/publishDiagnostics" => {
                let uri = parameters
                    .get("textDocument")
                    .and_then(|text| text.get("uri"))
                    .and_then(Value::as_str)
                    .or_else(|| parameters.get("uri").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let diagnostics: Vec<Diagnostic> = parameters
                    .get("diagnostics")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(parse_diagnostic).collect())
                    .unwrap_or_default();
                self.diagnostics.insert(uri.clone(), diagnostics.clone());
                self.events.push(Event::Diagnostics { diagnostics });
            }
            "window/showMessage" => {
                let message = parameters
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.events.push(Event::Error {
                    server: self.server_name(),
                    message,
                });
            }
            "window/logMessage" => {
                if let Some(message) = parameters.get("message").and_then(Value::as_str) {
                    self.log(format!("server {message}"));
                }
            }
            _ => {}
        }
    }

    fn handle_server_request(&mut self, method: &str, id: &Value, message: &Value) {
        let result = match method {
            "workspace/configuration" => {
                let count = message
                    .get("params")
                    .and_then(|params| params.get("items"))
                    .and_then(Value::as_array)
                    .map(<[Value]>::len)
                    .unwrap_or(0);
                array((0..count).map(|_| Value::Null).collect())
            }
            "workspace/workspaceFolders" => {
                let name = Path::new(&self.root_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                array(vec![object(vec![
                    ("uri", string(&self.root_uri)),
                    ("name", string(&name)),
                ])])
            }
            _ => Value::Null,
        };
        self.send_response(id, result);
    }

    fn send_initialize(&mut self) {
        let workspace_name = Path::new(&self.root_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parameters = object(vec![
            ("processId", number(std::process::id() as i64)),
            ("clientInfo", object(vec![("name", string("Hexal"))])),
            ("rootUri", string(&self.root_uri)),
            ("rootPath", string(&self.root_path)),
            (
                "workspaceFolders",
                array(vec![object(vec![
                    ("uri", string(&self.root_uri)),
                    ("name", string(&workspace_name)),
                ])]),
            ),
            (
                "capabilities",
                object(vec![
                    (
                        "workspace",
                        object(vec![
                            ("workspaceFolders", bool_value(true)),
                            ("configuration", bool_value(true)),
                        ]),
                    ),
                    (
                        "textDocument",
                        object(vec![
                            (
                                "synchronization",
                                object(vec![
                                    ("dynamicRegistration", bool_value(false)),
                                    ("didSave", bool_value(true)),
                                ]),
                            ),
                            (
                                "publishDiagnostics",
                                object(vec![("relatedInformation", bool_value(true))]),
                            ),
                            (
                                "definition",
                                object(vec![("linkSupport", bool_value(true))]),
                            ),
                            (
                                "completion",
                                object(vec![
                                    (
                                        "completionItem",
                                        object(vec![
                                            ("snippetSupport", bool_value(true)),
                                            (
                                                "documentationFormat",
                                                array(vec![
                                                    string("markdown"),
                                                    string("plaintext"),
                                                ]),
                                            ),
                                            ("insertReplaceSupport", bool_value(true)),
                                        ]),
                                    ),
                                    ("contextSupport", bool_value(true)),
                                ]),
                            ),
                        ]),
                    ),
                ]),
            ),
        ]);
        let message = object(vec![
            ("jsonrpc", string("2.0")),
            ("id", number(self.initialize_id)),
            ("method", string("initialize")),
            ("params", parameters),
        ]);
        self.send_message(message);
    }

    fn allocate_id(&mut self) -> i64 {
        self.next_request_id += 1;
        self.next_request_id
    }

    fn send_request(&mut self, method: &str, parameters: Value) -> i64 {
        let id = self.allocate_id();
        self.pending.insert(id, method.to_string());
        let message = object(vec![
            ("jsonrpc", string("2.0")),
            ("id", number(id)),
            ("method", string(method)),
            ("params", parameters),
        ]);
        self.send_message(message);
        id
    }

    fn send_notification(&mut self, method: &str, parameters: Value) {
        let message = object(vec![
            ("jsonrpc", string("2.0")),
            ("method", string(method)),
            ("params", parameters),
        ]);
        self.send_message(message);
    }

    fn send_response(&mut self, id: &Value, result: Value) {
        let message = object(vec![
            ("jsonrpc", string("2.0")),
            ("id", id.clone()),
            ("result", result),
        ]);
        self.send_message(message);
    }

    fn send_message(&mut self, message: Value) {
        let body = encode(&message);
        if message.get("method").and_then(Value::as_str) == Some("textDocument/didChange") {
            self.log(format!(
                "--> textDocument/didChange (full document, {} bytes)",
                body.len()
            ));
        } else {
            let text = String::from_utf8_lossy(&body).into_owned();
            self.log(format!("--> {text}"));
        }
        let Some(outgoing) = &self.outgoing else {
            return;
        };
        let mut frame = Vec::with_capacity(body.len() + 32);
        frame.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        frame.extend_from_slice(&body);
        if outgoing.send(frame).is_err() {
            self.log("send failed: language server writer is gone".into());
            self.events.push(Event::Error {
                server: self.server_name(),
                message: "language server writer is gone".into(),
            });
        }
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn stop(&mut self) {
        if self.child.is_none()
            && self.outgoing.is_none()
            && self.writer.is_none()
            && self.incoming.is_none()
            && self.reader.is_none()
            && self.stderr_reader.is_none()
        {
            return;
        }

        // Tear down off the input thread. Killing the child unblocks any
        // pending pipe writes/reads, and waiting for descendants is done in
        // the background so switching documents never waits on a server.
        self.outgoing = None;
        let mut child = self.child.take();
        if let Some(child) = &mut child {
            let _ = child.kill();
        }
        let writer = self.writer.take();
        let reader = self.reader.take();
        let stderr_reader = self.stderr_reader.take();
        let incoming = self.incoming.take();
        if child.is_some()
            || writer.is_some()
            || reader.is_some()
            || stderr_reader.is_some()
            || incoming.is_some()
        {
            std::thread::spawn(move || {
                if let Some(mut child) = child {
                    let _ = child.wait();
                }
                if let Some(writer) = writer {
                    let _ = writer.join();
                }
                if let Some(reader) = reader {
                    let _ = reader.join();
                }
                if let Some(reader) = stderr_reader {
                    let _ = reader.join();
                }
                drop(incoming);
            });
        }
        self.config = None;
        self.initialized = false;
        self.workspace_diagnostics_supported = false;
        self.completion_triggers.clear();
        self.current_text.clear();
        self.document_uri.clear();
        self.pending.clear();
        self.diagnostics.clear();
        self.events.push(Event::Stopped);
    }

    pub fn build_debug_report(&self) -> String {
        let mut text = String::new();
        text.push_str("Hexal LSP diagnostics\n");
        text.push_str(&format!(
            "Server: {}\n",
            self.config
                .as_ref()
                .map(|config| config.name.as_str())
                .unwrap_or("(none)")
        ));
        text.push_str(&format!(
            "Command: {}\n",
            self.config
                .as_ref()
                .map(|config| config.command.as_str())
                .unwrap_or("(none)")
        ));
        text.push_str(&format!(
            "State: {}{}\n",
            if self.is_running() {
                "running"
            } else {
                "stopped"
            },
            if self.initialized {
                ", initialized"
            } else {
                ", not initialized"
            }
        ));
        text.push_str(&format!("Document: {}\n", self.document_uri));
        text.push_str(&format!("Root: {}\n", self.root_path));
        text.push_str(&format!("Document version: {}\n", self.version));
        text.push_str(&format!("Pending requests: {}\n", self.pending.len()));
        text.push_str(&format!(
            "Diagnostic documents: {}\n",
            self.diagnostics.len()
        ));
        text.push_str(&format!(
            "Workspace diagnostics: {}\n\n",
            if self.workspace_diagnostics_supported {
                "supported"
            } else {
                "not supported"
            }
        ));
        text.push_str(&format!("Log entries: {}\n", self.log.len()));
        text.push_str("----- LSP log (oldest first) -----\n");
        for entry in &self.log {
            text.push_str(entry);
            text.push('\n');
        }
        text
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
        self.dropped_log = 0;
    }

    fn log(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let text = if text.len() > 16 * 1024 {
            format!(
                "{}… truncated ({} bytes total)",
                &text[..16 * 1024],
                text.len()
            )
        } else {
            text
        };
        self.log.push_back(text);
        while self.log.len() > 4000 {
            self.log.pop_front();
            self.dropped_log += 1;
        }
    }
}

fn read_loop(mut stdout: impl Read, sender: Sender<Incoming>) {
    let mut pending = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = match stdout.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => break,
        };
        pending.extend_from_slice(&buffer[..read]);
        loop {
            let Some((body, consumed)) = take_message(&pending) else {
                break;
            };
            pending.drain(..consumed);
            match json::parse(&String::from_utf8_lossy(&body)) {
                Ok(value) => {
                    if sender.send(Incoming::Message(value)).is_err() {
                        return;
                    }
                }
                Err(_) => {}
            }
        }
    }
    let _ = sender.send(Incoming::Closed);
}

fn take_message(buffer: &[u8]) -> Option<(Vec<u8>, usize)> {
    let header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let header = &buffer[..header_end];
    let mut length = None;
    for line in header.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|byte| *byte == b':') else {
            continue;
        };
        let name = &line[..colon];
        if name.eq_ignore_ascii_case(b"content-length") {
            if let Ok(text) = std::str::from_utf8(&line[colon + 1..]) {
                length = text.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length?;
    let body_start = header_end + 4;
    if buffer.len() < body_start + length {
        return None;
    }
    Some((
        buffer[body_start..body_start + length].to_vec(),
        body_start + length,
    ))
}

// ---------------------------------------------------------------------------
// JSON construction and encoding
// ---------------------------------------------------------------------------

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn array(values: Vec<Value>) -> Value {
    Value::Array(values)
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn number(value: i64) -> Value {
    Value::Number(value as f64)
}

fn bool_value(value: bool) -> Value {
    Value::Bool(value)
}

fn encode(value: &Value) -> Vec<u8> {
    json::to_string(value, false).into_bytes()
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

fn extract_hover_text(hover: &Value) -> String {
    let Some(contents) = hover.get("contents") else {
        return String::new();
    };
    let mut parts = Vec::new();
    append_hover_contents(contents, &mut parts);
    parts.join("\n")
}

fn append_hover_contents(contents: &Value, parts: &mut Vec<String>) {
    match contents {
        Value::String(text) => {
            if !text.is_empty() {
                parts.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                append_hover_contents(item, parts);
            }
        }
        Value::Object(_) => {
            if let Some(text) = contents.get("value").and_then(Value::as_str) {
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
        }
        _ => {}
    }
}

fn read_string(element: &Value, property: &str) -> String {
    element
        .get(property)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn read_int(element: &Value, property: &str, fallback: i64) -> i64 {
    element
        .get(property)
        .and_then(Value::as_i64)
        .unwrap_or(fallback)
}

fn parse_completion_items(result: &Value) -> Vec<CompletionItem> {
    let items = if let Some(items) = result.as_array() {
        items
    } else if let Some(items) = result.get("items").and_then(Value::as_array) {
        items
    } else {
        return Vec::new();
    };
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        if !matches!(item, Value::Object(_)) {
            continue;
        }
        let label = read_string(item, "label");
        if label.is_empty() {
            continue;
        }
        let mut insert_text = read_string(item, "insertText");
        let insert_format = read_int(item, "insertTextFormat", 1);
        let mut has_edit = false;
        let mut start_line = 0;
        let mut start_character = 0;
        let mut end_line = 0;
        let mut end_character = 0;
        if let Some(text_edit) = item.get("textEdit") {
            if matches!(text_edit, Value::Object(_)) {
                if let Some(new_text) = text_edit.get("newText").and_then(Value::as_str) {
                    insert_text = new_text.to_string();
                }
                if let Some(range) = text_edit
                    .get("range")
                    .or_else(|| text_edit.get("insert"))
                    .or_else(|| text_edit.get("replace"))
                {
                    let (sl, sc, el, ec) = read_range(range);
                    start_line = sl;
                    start_character = sc;
                    end_line = el;
                    end_character = ec;
                    has_edit = true;
                }
            }
        }
        if insert_text.is_empty() {
            insert_text = label.clone();
        }
        output.push(CompletionItem {
            label,
            insert_text,
            insert_text_format: insert_format,
            detail: read_string(item, "detail"),
            filter_text: read_string(item, "filterText"),
            has_text_edit: has_edit,
            start_line,
            start_character,
            end_line,
            end_character,
        });
    }
    output
}

fn read_range(range: &Value) -> (i64, i64, i64, i64) {
    if !matches!(range, Value::Object(_)) {
        return (0, 0, 0, 0);
    }
    let start = range.get("start");
    let start_line = start.map(|start| read_int(start, "line", 0)).unwrap_or(0);
    let start_character = start
        .map(|start| read_int(start, "character", 0))
        .unwrap_or(0);
    let end = range.get("end");
    let end_line = end
        .map(|end| read_int(end, "line", start_line))
        .unwrap_or(start_line);
    let end_character = end
        .map(|end| read_int(end, "character", start_character))
        .unwrap_or(start_character);
    (start_line, start_character, end_line, end_character)
}

fn parse_locations(result: &Value) -> Vec<Location> {
    let mut locations = Vec::new();
    parse_locations_into(result, &mut locations);
    locations
}

fn parse_locations_into(result: &Value, locations: &mut Vec<Location>) {
    if let Some(items) = result.as_array() {
        for item in items {
            parse_locations_into(item, locations);
        }
        return;
    }
    if !matches!(result, Value::Object(_)) {
        return;
    }
    let uri = result
        .get("uri")
        .and_then(Value::as_str)
        .or_else(|| result.get("targetUri").and_then(Value::as_str))
        .unwrap_or("");
    if uri.is_empty() {
        return;
    }
    let range = result
        .get("range")
        .or_else(|| result.get("targetSelectionRange"))
        .or_else(|| result.get("targetRange"));
    let Some(range) = range else {
        return;
    };
    let Some(start) = range.get("start") else {
        return;
    };
    locations.push(Location {
        uri: uri.to_string(),
        line: read_int(start, "line", 0),
        character: read_int(start, "character", 0),
    });
}

fn parse_document_symbols(result: &Value, document_uri: &str) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    if let Some(items) = result.as_array() {
        parse_document_symbol_array(items, document_uri, &mut symbols, 0);
    }
    symbols
}

fn parse_document_symbol_array(
    items: &[Value],
    document_uri: &str,
    symbols: &mut Vec<DocumentSymbol>,
    depth: usize,
) {
    for item in items {
        if !matches!(item, Value::Object(_)) {
            continue;
        }
        let name = read_string(item, "name");
        if name.is_empty() {
            continue;
        }
        let detail = read_string(item, "detail");
        let kind = read_int(item, "kind", 0);
        let mut uri = document_uri.to_string();
        let mut line = 0;
        let mut character = 0;
        if let Some(location) = item.get("location") {
            if let Some(value) = location.get("uri").and_then(Value::as_str) {
                uri = value.to_string();
            }
            if let Some(range) = location.get("range") {
                (line, character) = range_start(range);
            }
        } else {
            let Some(range) = item.get("selectionRange").or_else(|| item.get("range")) else {
                continue;
            };
            (line, character) = range_start(range);
        }
        symbols.push(DocumentSymbol {
            name,
            detail,
            kind,
            uri: uri.clone(),
            line,
            character,
            depth,
        });
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            parse_document_symbol_array(children, &uri, symbols, depth + 1);
        }
    }
}

fn range_start(range: &Value) -> (i64, i64) {
    let Some(start) = range.get("start") else {
        return (0, 0);
    };
    (read_int(start, "line", 0), read_int(start, "character", 0))
}

fn parse_code_actions(result: &Value) -> Vec<CodeAction> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    let mut actions = Vec::with_capacity(items.len());
    for item in items {
        if !matches!(item, Value::Object(_)) {
            continue;
        }
        let edits = parse_workspace_edit(item, "edit");
        let mut command = String::new();
        let mut arguments = None;
        if let Some(command_value) = item.get("command") {
            if matches!(command_value, Value::Object(_)) {
                command = read_string(command_value, "command");
                if let Some(values) = command_value.get("arguments").and_then(Value::as_array) {
                    arguments = Some(Value::Array(values.to_vec()));
                }
            }
        }
        let has_data = item
            .get("data")
            .is_some_and(|value| !matches!(value, Value::Null));
        let raw = if matches!(item, Value::Object(_)) {
            Some(item.clone())
        } else {
            None
        };
        actions.push(CodeAction {
            title: read_string(item, "title"),
            kind: read_string(item, "kind"),
            needs_resolve: edits.is_empty() && command.is_empty() && has_data,
            edits,
            command,
            command_arguments: arguments,
            raw,
        });
    }
    actions
}

fn parse_workspace_edit(owner: &Value, property: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    let Some(edit) = owner.get(property) else {
        return edits;
    };
    if !matches!(edit, Value::Object(_)) {
        return edits;
    }
    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        for (uri, value) in changes {
            if let Some(items) = value.as_array() {
                parse_text_edits(uri, items, &mut edits);
            }
        }
    }
    if let Some(changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            let Some(text_document) = change.get("textDocument") else {
                continue;
            };
            let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
                continue;
            };
            if let Some(items) = change.get("edits").and_then(Value::as_array) {
                parse_text_edits(uri, items, &mut edits);
            }
        }
    }
    edits
}

fn parse_text_edits(uri: &str, items: &[Value], edits: &mut Vec<TextEdit>) {
    for item in items {
        let Some(range) = item.get("range") else {
            continue;
        };
        let Some(start) = range.get("start") else {
            continue;
        };
        let Some(end) = range.get("end") else {
            continue;
        };
        edits.push(TextEdit {
            uri: uri.to_string(),
            start_line: read_int(start, "line", 0),
            start_character: read_int(start, "character", 0),
            end_line: read_int(end, "line", read_int(start, "line", 0)),
            end_character: read_int(end, "character", read_int(start, "character", 0)),
            new_text: read_string(item, "newText"),
        });
    }
}

fn parse_workspace_diagnostics(result: &Value) -> Vec<DocumentDiagnostics> {
    let Some(items) = result.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut reports = Vec::new();
    for item in items {
        let Some(uri) = item.get("uri").and_then(Value::as_str) else {
            continue;
        };
        let diagnostics = item
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(parse_diagnostic).collect())
            .unwrap_or_default();
        reports.push(DocumentDiagnostics {
            uri: uri.to_string(),
            diagnostics,
        });
    }
    reports
}

fn parse_diagnostic(item: &Value) -> Diagnostic {
    let mut line = 0;
    let mut column = 0;
    let mut end_line = -1;
    let mut end_column = -1;
    if let Some(range) = item.get("range") {
        if let Some(start) = range.get("start") {
            line = read_int(start, "line", 0);
            column = read_int(start, "character", 0);
        }
        if let Some(end) = range.get("end") {
            end_line = read_int(end, "line", -1);
            end_column = read_int(end, "character", -1);
        }
    }
    let tags = item
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| tags.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    Diagnostic {
        line,
        column,
        severity: read_int(item, "severity", 1),
        message: read_string(item, "message"),
        source: read_string(item, "source"),
        end_line,
        end_column,
        tags,
    }
}

// ---------------------------------------------------------------------------
// URI and position helpers
// ---------------------------------------------------------------------------

pub fn absolute_path(path: &Path) -> PathBuf {
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in combined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

pub fn path_to_uri(path: &str, directory: bool) -> Result<String, String> {
    let mut full = absolute_path(Path::new(path));
    if directory {
        full.push("");
    }
    let text = full.to_string_lossy();
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'_' | b'.' | b'~' | b':')
        {
            uri.push(*byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    if directory && !uri.ends_with('/') {
        uri.push('/');
    }
    Ok(uri)
}

pub fn uri_to_path(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_string();
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    percent_decode(rest)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                output.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub fn utf16_column_to_byte_offset(line: &[u8], column: i64) -> usize {
    if column <= 0 {
        return 0;
    }
    let target = column as usize;
    let mut offset = 0usize;
    let mut units = 0usize;
    while offset < line.len() && units < target {
        let (character, length) = crate::util::decode_at(line, offset);
        units += if character as u32 > 0xFFFF { 2 } else { 1 };
        offset += length.max(1);
    }
    offset.min(line.len())
}

pub fn byte_offset_to_utf16_column(line: &[u8], byte_offset: usize) -> i64 {
    let mut offset = 0usize;
    let mut units = 0usize;
    let target = byte_offset.min(line.len());
    while offset < target {
        let (character, length) = crate::util::decode_at(line, offset);
        offset += length.max(1);
        units += if character as u32 > 0xFFFF { 2 } else { 1 };
    }
    units as i64
}

fn display_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::{path_to_uri, Event, Manager, ServerConfig};
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn client_talks_to_stdio_server() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("hexal-lsp-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let server = dir.join("server.py");
        fs::write(
            &server,
            r#"
import json, sys, os

def read():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line and line not in (b'\n', b'\r\n'):
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    length = int(headers.get('content-length', '0'))
    return json.loads(sys.stdin.buffer.read(length))

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode())
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

uri = ''
while True:
    message = read()
    if message is None:
        break
    method = message.get('method')
    if method == 'initialize':
        send({'jsonrpc':'2.0','id':message['id'],'result':{'capabilities':{
            'hoverProvider': True,
            'completionProvider': {'triggerCharacters':['.']},
            'diagnosticProvider': {'workspaceDiagnostics': True}
        }}})
    elif method == 'initialized':
        pass
    elif method == 'textDocument/didOpen':
        uri = message['params']['textDocument']['uri']
        send({'jsonrpc':'2.0','method':'textDocument/publishDiagnostics','params':{
            'uri':uri,'diagnostics':[{'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}},'severity':1,'message':'bad','source':'test'}]
        }})
    elif method == 'textDocument/hover':
        send({'jsonrpc':'2.0','id':message['id'],'result':{'contents':{'kind':'markdown','value':'hello'}}})
    elif method == 'textDocument/definition':
        send({'jsonrpc':'2.0','id':message['id'],'result':[{'uri':uri,'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}]})
    elif method == 'textDocument/completion':
        send({'jsonrpc':'2.0','id':message['id'],'result':[{'label':'foo','detail':'detail','insertText':'foo'}]})
    elif method == 'textDocument/documentSymbol':
        send({'jsonrpc':'2.0','id':message['id'],'result':[{'name':'sym','kind':12,'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}},'selectionRange':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}]})
    elif method == 'textDocument/codeAction':
        send({'jsonrpc':'2.0','id':message['id'],'result':[{'title':'fix','kind':'quickfix','edit':{'changes':{uri:[{'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}},'newText':'x'}]}}}]})
    elif 'id' in message:
        send({'jsonrpc':'2.0','id':message['id'],'result':None})
"#,
        )
        .unwrap();

        let document = dir.join("sample.rs");
        fs::write(&document, b"fn main() {}\n").unwrap();
        let workspace = dir.to_string_lossy().into_owned();
        let path = document.to_string_lossy().into_owned();
        let uri = path_to_uri(&path, false).unwrap();

        let mut manager = Manager::new();
        manager
            .try_start(
                ServerConfig {
                    name: "fake".into(),
                    command: "python3".into(),
                    arguments: vec![server.to_string_lossy().into_owned()],
                    language_id: "rust".into(),
                },
                &path,
                "fn main() {}\n",
                &workspace,
            )
            .unwrap();

        let mut initialized = false;
        let mut diagnostics = false;
        for _ in 0..100 {
            manager.poll(Duration::from_millis(5));
            for event in manager.take_events() {
                match event {
                    Event::Initialized => initialized = true,
                    Event::Diagnostics { diagnostics: items } => {
                        diagnostics = items.len() == 1 && items[0].message == "bad";
                    }
                    _ => {}
                }
            }
            if initialized && diagnostics {
                break;
            }
        }
        assert!(initialized);
        assert!(diagnostics);
        assert_eq!(manager.count_diagnostics_for_path(&path), 1);

        assert!(manager.request_hover(&path, 0, 0));
        let mut hover = None;
        for _ in 0..100 {
            manager.poll(Duration::from_millis(5));
            for event in manager.take_events() {
                if let Event::Hover { text } = event {
                    hover = Some(text);
                }
            }
            if hover.is_some() {
                break;
            }
        }
        assert_eq!(hover.as_deref(), Some("hello"));

        assert!(manager.request_completion(&path, 0, 0, None).is_some());
        manager.request_definition(&path, 0, 0);
        manager.request_document_symbols(&path);
        manager.request_code_actions(&path, 0, 0, &[]);
        let mut completions = 0;
        let mut definitions = 0;
        let mut symbols = 0;
        let mut actions = 0;
        for _ in 0..200 {
            manager.poll(Duration::from_millis(5));
            for event in manager.take_events() {
                match event {
                    Event::Completions { items, .. } => completions = items.len(),
                    Event::Definition { locations } => definitions = locations.len(),
                    Event::DocumentSymbols { symbols: items } => symbols = items.len(),
                    Event::CodeActions { actions: items } => actions = items.len(),
                    _ => {}
                }
            }
            if completions > 0 && definitions > 0 && symbols > 0 && actions > 0 {
                break;
            }
        }
        assert_eq!(completions, 1);
        assert_eq!(definitions, 1);
        assert_eq!(symbols, 1);
        assert_eq!(actions, 1);
        assert!(manager.is_serving(&path));
        assert_eq!(manager.uri_for_test(), uri);
        manager.stop();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_server_does_not_block_edits_or_stop() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("hexal-lsp-slow-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let server = dir.join("server.py");
        fs::write(
            &server,
            r#"
import json, sys, time

def read():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line and line not in (b'\n', b'\r\n'):
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    return json.loads(sys.stdin.buffer.read(int(headers.get('content-length', '0'))))

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode())
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

message = read()
send({'jsonrpc':'2.0','id':message['id'],'result':{'capabilities':{}}})
send({'jsonrpc':'2.0','method':'initialized','params':{}})
time.sleep(3)
"#,
        )
        .unwrap();

        let document = dir.join("sample.rs");
        fs::write(&document, b"fn main() {}\n").unwrap();
        let workspace = dir.to_string_lossy().into_owned();
        let path = document.to_string_lossy().into_owned();
        let mut manager = Manager::new();
        manager
            .try_start(
                ServerConfig {
                    name: "slow".into(),
                    command: "python3".into(),
                    arguments: vec![server.to_string_lossy().into_owned()],
                    language_id: "rust".into(),
                },
                &path,
                "fn main() {}\n",
                &workspace,
            )
            .unwrap();

        let mut initialized = false;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !initialized {
            manager.poll(Duration::from_millis(5));
            for event in manager.take_events() {
                if matches!(event, Event::Initialized) {
                    initialized = true;
                }
            }
        }
        assert!(initialized);

        let large_document = "x".repeat(2 * 1024 * 1024);
        let started = Instant::now();
        manager.did_change(&large_document);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "did_change blocked for {:?}",
            started.elapsed()
        );

        let started = Instant::now();
        manager.stop();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "stop blocked for {:?}",
            started.elapsed()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
