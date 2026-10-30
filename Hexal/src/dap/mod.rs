//! Minimal Debug Adapter Protocol (DAP) client.
//!
//! The protocol handling mirrors the C# editor's `DapManager`: an adapter is
//! started over stdio, the initialize/launch/configurationDone handshake is
//! performed, and stopped/exited/output notifications are forwarded to the
//! editor. Launch configurations come from the language's `.hexdef` file.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::util::json::{self, Value};

const MAX_LOG_ENTRIES: usize = 2000;
const MAX_LOG_ENTRY_LENGTH: usize = 16 * 1024;
const ADAPTER_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How a debug adapter is obtained for a language.
#[derive(Clone, Debug)]
pub enum DebuggerStart {
    /// The adapter is an executable that speaks DAP over stdin/stdout.
    Stdio,
    /// The language server starts the adapter. The editor runs the configured
    /// request/response steps, then connects to the returned endpoint.
    Lsp(LspDebuggerStart),
}

/// A declarative sequence of LSP commands used to obtain a debug endpoint.
#[derive(Clone, Debug)]
pub struct LspDebuggerStart {
    pub steps: Vec<LspDebuggerStep>,
    /// Values merged into the launch configuration after the steps run.
    pub configuration: Value,
    /// Maximum time to wait for the language server to start the adapter.
    pub timeout_ms: u64,
}

#[derive(Clone, Debug)]
pub struct LspDebuggerStep {
    pub command: String,
    pub arguments: Option<Value>,
    pub save: String,
    pub select: Option<LspDebuggerSelect>,
    pub endpoint: Option<LspDebuggerEndpoint>,
}

#[derive(Clone, Debug)]
pub struct LspDebuggerSelect {
    pub field: String,
    pub equals: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LspDebuggerEndpoint {
    Port,
    Uri,
}

/// Optional debug-adapter metadata from a `.hexdef` language definition.
#[derive(Clone, Debug)]
pub struct DebuggerConfig {
    pub adapter: String,
    pub arguments: Vec<String>,
    pub configuration: Value,
    pub start: DebuggerStart,
}

#[derive(Clone, Debug)]
pub struct DapBreakpoint {
    pub line: i64,
    pub verified: bool,
    pub message: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DapScope {
    pub name: String,
    pub variables_reference: i64,
    pub expensive: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DapVariable {
    pub name: String,
    pub value: String,
    pub type_name: String,
    pub variables_reference: i64,
    pub evaluate_name: String,
}

#[derive(Clone, Debug)]
pub struct DapScopeVariables {
    pub name: String,
    pub variables: Vec<DapVariable>,
}

#[derive(Clone, Debug)]
pub struct DapWatchValue {
    pub expression: String,
    pub value: String,
    pub type_name: String,
    pub error: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Event {
    Started {
        adapter: String,
    },
    Initialized {
        adapter: String,
    },
    Stopped {
        adapter: String,
        thread_id: i64,
        reason: String,
    },
    StoppedLocation {
        adapter: String,
        path: String,
        line: i64,
        reason: String,
    },
    Continued {
        adapter: String,
    },
    Exited {
        adapter: String,
        exit_code: i64,
        message: String,
    },
    Terminated {
        adapter: String,
        message: String,
    },
    Output {
        adapter: String,
        category: String,
        output: String,
    },
    Breakpoints {
        adapter: String,
        breakpoints: Vec<DapBreakpoint>,
    },
    Scopes {
        adapter: String,
        scopes: Vec<DapScope>,
    },
    Variables {
        adapter: String,
        scoped_variables: Vec<DapScopeVariables>,
    },
    WatchValue {
        adapter: String,
        watch: DapWatchValue,
    },
    Error {
        adapter: String,
        message: String,
    },
}

#[derive(Clone, Debug)]
struct PendingRequest {
    command: String,
    generation: i64,
    variables_reference: i64,
    expression: String,
}

impl PendingRequest {
    fn command(command: &str) -> PendingRequest {
        PendingRequest {
            command: command.to_string(),
            generation: 0,
            variables_reference: 0,
            expression: String::new(),
        }
    }
}

enum Incoming {
    Message(Value, String),
    Invalid(String),
    Stderr(String),
    Closed,
}

pub struct Manager {
    child: Option<Child>,
    input: Option<Box<dyn Write + Send>>,
    incoming: Option<Receiver<Incoming>>,
    reader: Option<std::thread::JoinHandle<()>>,
    stderr_reader: Option<std::thread::JoinHandle<()>>,

    config: Option<DebuggerConfig>,
    launch_configuration: Option<Value>,
    adapter_name: String,
    adapter_path: String,
    source_path: String,
    workspace_root: String,
    stopped_path: String,
    stop_reason: String,

    next_sequence: i64,
    stopped_frame_id: i64,
    stopped_thread_id: i64,
    stopped_line: i64,
    variables_generation: i64,
    pending_variables_responses: usize,

    initialized: bool,
    adapter_initialized: bool,
    launch_sent: bool,
    configuration_done: bool,
    stopped: bool,

    pending_requests: HashMap<i64, PendingRequest>,
    breakpoints: BTreeMap<String, BTreeSet<usize>>,
    current_scopes: Vec<DapScope>,
    variables_by_reference: HashMap<i64, Vec<DapVariable>>,
    log: VecDeque<String>,
    events: Vec<Event>,
}

impl Default for Manager {
    fn default() -> Self {
        Manager {
            child: None,
            input: None,
            incoming: None,
            reader: None,
            stderr_reader: None,
            config: None,
            launch_configuration: None,
            adapter_name: String::new(),
            adapter_path: String::new(),
            source_path: String::new(),
            workspace_root: String::new(),
            stopped_path: String::new(),
            stop_reason: String::new(),
            next_sequence: 0,
            stopped_frame_id: 0,
            stopped_thread_id: 0,
            stopped_line: -1,
            variables_generation: 0,
            pending_variables_responses: 0,
            initialized: false,
            adapter_initialized: false,
            launch_sent: false,
            configuration_done: false,
            stopped: false,
            pending_requests: HashMap::new(),
            breakpoints: BTreeMap::new(),
            current_scopes: Vec::new(),
            variables_by_reference: HashMap::new(),
            log: VecDeque::new(),
            events: Vec::new(),
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

impl Manager {
    pub fn new() -> Manager {
        Manager::default()
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some() || self.input.is_some()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    #[allow(dead_code)]
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Starts an adapter, replacing any running debug session.
    pub fn try_start(
        &mut self,
        config: DebuggerConfig,
        source_path: &str,
        workspace_root: &str,
        launch_configuration: Value,
        breakpoints: &BTreeMap<String, BTreeSet<usize>>,
    ) -> Result<(), String> {
        if !matches!(config.start, DebuggerStart::Stdio) {
            return Err("this debug adapter is started by the language server".into());
        }
        if config.adapter.trim().is_empty() {
            return Err("debug adapter is empty".into());
        }
        let Some(adapter) = find_adapter(&config.adapter) else {
            return Err(format!(
                "debug adapter not found in PATH: {}",
                config.adapter
            ));
        };

        let source_full = normalize_path(source_path);
        let workspace_full =
            if !workspace_root.trim().is_empty() && Path::new(workspace_root).is_dir() {
                normalize_path(workspace_root)
            } else {
                Path::new(&source_full)
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned())
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| ".".into())
            };

        self.stop_internal();

        self.adapter_name = config.adapter.clone();
        self.adapter_path = adapter.clone();
        self.source_path = source_full.clone();
        self.workspace_root = workspace_full.clone();
        self.launch_configuration = Some(launch_configuration);
        self.breakpoints = breakpoints.clone();
        self.pending_requests.clear();
        self.next_sequence = 0;
        self.initialized = false;
        self.adapter_initialized = false;
        self.launch_sent = false;
        self.configuration_done = false;
        self.stopped = false;
        self.stopped_thread_id = 0;
        self.stopped_path.clear();
        self.stopped_line = -1;
        self.stop_reason.clear();
        self.clear_debug_state();

        let arguments = config.arguments.join(" ");
        self.log(
            "info",
            &format!(
                "starting {}: {}{} (cwd {})",
                config.adapter,
                adapter,
                if arguments.is_empty() {
                    String::new()
                } else {
                    format!(" {arguments}")
                },
                workspace_full
            ),
        );

        let mut command = Command::new(&adapter);
        command
            .args(&config.arguments)
            .current_dir(&workspace_full)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let message = format!("cannot start debug adapter '{}': {error}", config.adapter);
                self.stop_internal();
                return Err(message);
            }
        };

        let process_id = child.id();
        self.log(
            "info",
            &format!("adapter process started (pid {process_id})"),
        );

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let input = child.stdin.take();
        self.child = Some(child);
        self.input = input.map(|input| Box::new(input) as Box<dyn Write + Send>);
        self.config = Some(config);

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

        self.send_initialize();
        self.events.push(Event::Started {
            adapter: self.adapter_name.clone(),
        });
        file_log(&format!(
            "DAP started: {} ({}) for {}",
            self.adapter_name, adapter, source_full
        ));
        Ok(())
    }

    /// Connects to a debug adapter that a language server has already started
    /// on a TCP endpoint.
    #[allow(clippy::too_many_arguments)]
    pub fn try_start_server(
        &mut self,
        config: DebuggerConfig,
        host: &str,
        port: u16,
        source_path: &str,
        workspace_root: &str,
        launch_configuration: Value,
        breakpoints: &BTreeMap<String, BTreeSet<usize>>,
    ) -> Result<(), String> {
        if host.trim().is_empty() || port == 0 {
            return Err("debug adapter endpoint is invalid".into());
        }

        let source_full = normalize_path(source_path);
        let workspace_full =
            if !workspace_root.trim().is_empty() && Path::new(workspace_root).is_dir() {
                normalize_path(workspace_root)
            } else {
                Path::new(&source_full)
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned())
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| ".".into())
            };

        let address = (host, port)
            .to_socket_addrs()
            .map_err(|error| {
                format!("cannot resolve debug adapter endpoint {host}:{port}: {error}")
            })?
            .next()
            .ok_or_else(|| format!("cannot resolve debug adapter endpoint {host}:{port}"))?;
        let stream = TcpStream::connect_timeout(&address, ADAPTER_CONNECT_TIMEOUT)
            .map_err(|error| format!("cannot connect to debug adapter {host}:{port}: {error}"))?;
        let reader = stream
            .try_clone()
            .map_err(|error| format!("cannot connect to debug adapter {host}:{port}: {error}"))?;
        let _ = stream.set_nodelay(true);

        self.stop_internal();

        self.child = None;
        self.adapter_name = config.adapter.clone();
        self.adapter_path = format!("{host}:{port}");
        self.source_path = source_full.clone();
        self.workspace_root = workspace_full.clone();
        self.launch_configuration = Some(launch_configuration);
        self.breakpoints = breakpoints.clone();
        self.pending_requests.clear();
        self.next_sequence = 0;
        self.initialized = false;
        self.adapter_initialized = false;
        self.launch_sent = false;
        self.configuration_done = false;
        self.stopped = false;
        self.stopped_thread_id = 0;
        self.stopped_path.clear();
        self.stopped_line = -1;
        self.stop_reason.clear();
        self.clear_debug_state();
        self.config = Some(config);

        let (sender, receiver) = mpsc::channel();
        self.incoming = Some(receiver);
        self.input = Some(Box::new(stream));
        self.reader = Some(std::thread::spawn(move || {
            read_loop(reader, sender);
        }));

        self.log(
            "info",
            &format!(
                "connecting to {} at {host}:{port} (cwd {workspace_full})",
                self.adapter_name
            ),
        );
        self.send_initialize();
        self.events.push(Event::Started {
            adapter: self.adapter_name.clone(),
        });
        file_log(&format!(
            "DAP started: {} ({host}:{port}) for {source_full}",
            self.adapter_name
        ));
        Ok(())
    }

    pub fn set_breakpoints(&mut self, path: &str, lines: &BTreeSet<usize>) {
        let full_path = normalize_path(path);
        let mut set = BTreeSet::new();
        for line in lines {
            set.insert(*line);
        }
        if set.is_empty() {
            self.breakpoints.remove(&full_path);
        } else {
            self.breakpoints.insert(full_path.clone(), set);
        }

        if self.is_running() && self.initialized && self.configuration_done {
            self.send_breakpoints(&full_path);
        }
    }

    pub fn continue_debuggee(&mut self) -> bool {
        self.send_thread_request("continue", true)
    }

    pub fn next(&mut self) -> bool {
        self.send_thread_request("next", true)
    }

    pub fn step_in(&mut self) -> bool {
        self.send_thread_request("stepIn", true)
    }

    pub fn step_out(&mut self) -> bool {
        self.send_thread_request("stepOut", true)
    }

    pub fn pause(&mut self) -> bool {
        self.send_thread_request("pause", false)
    }

    /// Requests scopes and variables for the current stopped frame. One
    /// variables request is sent per scope; a combined Variables event is
    /// emitted when every response has arrived.
    pub fn request_debug_variables(&mut self) -> bool {
        if !self.is_running() || !self.initialized || !self.stopped || self.stopped_frame_id <= 0 {
            return false;
        }

        self.variables_generation += 1;
        let generation = self.variables_generation;
        self.current_scopes.clear();
        self.variables_by_reference.clear();
        self.pending_variables_responses = 0;
        self.send_request(
            PendingRequest {
                command: "scopes".into(),
                generation,
                variables_reference: 0,
                expression: String::new(),
            },
            object(vec![("frameId", number(self.stopped_frame_id))]),
        );
        true
    }

    pub fn request_evaluate(&mut self, expression: &str) -> bool {
        if expression.trim().is_empty() {
            return false;
        }
        if !self.is_running() || !self.initialized || !self.stopped || self.stopped_frame_id <= 0 {
            return false;
        }

        let generation = self.variables_generation;
        self.send_request(
            PendingRequest {
                command: "evaluate".into(),
                generation,
                variables_reference: 0,
                expression: expression.to_string(),
            },
            object(vec![
                ("expression", string(expression)),
                ("frameId", number(self.stopped_frame_id)),
                ("context", string("watch")),
            ]),
        );
        true
    }

    pub fn stop(&mut self) {
        if self.is_running() && self.initialized {
            self.send_request(
                PendingRequest::command("disconnect"),
                object(vec![("terminateDebuggee", bool_value(true))]),
            );
        }
        self.stop_internal();
    }

    /// Drains adapter traffic and returns true when state changed.
    pub fn poll(&mut self) -> bool {
        let mut redraw = false;
        loop {
            let message = {
                let Some(receiver) = &self.incoming else {
                    break;
                };
                match receiver.try_recv() {
                    Ok(message) => Some(message),
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
                }
            };
            let Some(message) = message else {
                break;
            };
            redraw = true;
            match message {
                Incoming::Message(value, raw) => {
                    self.log("<--", &raw);
                    self.handle_message(value);
                }
                Incoming::Invalid(raw) => {
                    self.log("<--", &raw);
                    self.log("info", "invalid DAP message");
                }
                Incoming::Stderr(line) => {
                    self.log("stderr", &line);
                    file_log(&format!("DAP stderr [{}]: {line}", self.adapter_name));
                }
                Incoming::Closed => {
                    self.log("info", "adapter output closed");
                    self.events.push(Event::Terminated {
                        adapter: self.adapter_name.clone(),
                        message: "adapter output closed".into(),
                    });
                    self.initialized = false;
                }
            }
        }
        redraw
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn build_debug_report(&self) -> String {
        let arguments = self
            .config
            .as_ref()
            .map(|config| config.arguments.join(" "))
            .unwrap_or_default();
        let pending_methods = {
            let mut methods: Vec<&str> = Vec::new();
            for request in self.pending_requests.values() {
                if !methods.contains(&request.command.as_str()) {
                    methods.push(request.command.as_str());
                }
            }
            methods.join(", ")
        };
        let process_id = self
            .child
            .as_ref()
            .map(|child| child.id().to_string())
            .unwrap_or_else(|| "(none)".into());

        let mut text = String::new();
        text.push_str("Hexal DAP diagnostics\n");
        text.push_str(&format!("Generated: {}\n", timestamp_date_time()));
        text.push_str(&format!(
            "Adapter: {}\n",
            if self.adapter_name.is_empty() {
                "(none)"
            } else {
                &self.adapter_name
            }
        ));
        text.push_str(&format!(
            "Command: {}\n",
            if self.adapter_path.is_empty() {
                "(none)"
            } else {
                &self.adapter_path
            }
        ));
        text.push_str(&format!(
            "Arguments: {}\n",
            if arguments.is_empty() {
                "(none)"
            } else {
                &arguments
            }
        ));
        text.push_str(&format!(
            "State: {}{}{}{}\n",
            if self.is_running() {
                "running"
            } else {
                "stopped"
            },
            if self.initialized {
                ", initialized"
            } else {
                ", not initialized"
            },
            if self.configuration_done {
                ", configured"
            } else {
                ", not configured"
            },
            if self.stopped {
                ", paused"
            } else {
                ", running"
            }
        ));
        text.push_str(&format!("Process ID: {process_id}\n"));
        text.push_str(&format!("Workspace: {}\n", self.workspace_root));
        text.push_str(&format!("Source: {}\n", self.source_path));
        text.push_str(&format!("Stopped thread: {}\n", self.stopped_thread_id));
        text.push_str(&format!(
            "Stop location: {}\n",
            if self.stopped_line >= 0 {
                format!("{}:{}", self.stopped_path, self.stopped_line + 1)
            } else {
                "(none)".into()
            }
        ));
        text.push_str(&format!(
            "Pending requests: {}",
            self.pending_requests.len()
        ));
        if !pending_methods.is_empty() {
            text.push_str("  ");
            text.push_str(&pending_methods);
        }
        text.push('\n');
        text.push_str(&format!("Breakpoints: {}\n", self.breakpoints.len()));
        for (path, lines) in &self.breakpoints {
            let lines = lines
                .iter()
                .map(|line| (line + 1).to_string())
                .collect::<Vec<String>>()
                .join(", ");
            text.push_str(&format!("  {path}: {lines}\n"));
        }
        text.push_str(&format!("File log: {}\n", log_path().display()));
        text.push('\n');
        text.push_str(&format!("Log entries: {}\n", self.log.len()));
        text.push_str("----- DAP log (oldest first) -----\n");
        for entry in &self.log {
            text.push_str(entry);
            text.push('\n');
        }
        text
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
        self.log("info", "log cleared");
    }

    /// Resolves a configured adapter name to an executable, including the
    /// version-suffixed LLVM fallback used by lldb-dap installations.
    #[allow(dead_code)]
    pub fn find_adapter(adapter: &str) -> Option<String> {
        find_adapter(adapter)
    }

    // -----------------------------------------------------------------
    // Protocol handshake and requests
    // -----------------------------------------------------------------

    fn send_initialize(&mut self) {
        let adapter_id = self.adapter_name.clone();
        self.send_request(
            PendingRequest::command("initialize"),
            object(vec![
                ("clientID", string("hexal")),
                ("clientName", string("Hexal")),
                ("adapterID", string(&adapter_id)),
                ("pathFormat", string("path")),
                ("linesStartAt1", bool_value(true)),
                ("columnsStartAt1", bool_value(true)),
                ("supportsVariableType", bool_value(true)),
                ("supportsVariablePaging", bool_value(true)),
                ("supportsRunInTerminalRequest", bool_value(false)),
                ("locale", string("en-US")),
            ]),
        );
    }

    fn handle_initialize_response(&mut self) {
        self.initialized = true;
        self.log("info", "initialize response received");

        let mut launch = self
            .launch_configuration
            .clone()
            .unwrap_or_else(|| Value::Object(Vec::new()));
        let request = launch
            .get("request")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|request| !request.trim().is_empty())
            .unwrap_or_else(|| "launch".into());
        if launch.get("request").is_none() {
            launch.set("request", string(&request));
        }

        self.launch_sent = true;
        self.send_request(PendingRequest::command(&request), launch);
        self.configure_adapter_if_ready();
    }

    fn handle_initialized_event(&mut self) {
        self.adapter_initialized = true;
        self.events.push(Event::Initialized {
            adapter: self.adapter_name.clone(),
        });
        self.configure_adapter_if_ready();
    }

    fn configure_adapter_if_ready(&mut self) {
        if !self.initialized
            || !self.launch_sent
            || !self.adapter_initialized
            || self.configuration_done
        {
            return;
        }

        let paths: Vec<String> = self.breakpoints.keys().cloned().collect();
        for path in paths {
            self.send_breakpoints(&path);
        }

        self.send_request(
            PendingRequest::command("configurationDone"),
            object(Vec::new()),
        );
        self.configuration_done = true;
        self.log("info", "configurationDone sent");
    }

    fn send_breakpoints(&mut self, path: &str) {
        let mut breakpoints = Vec::new();
        if let Some(lines) = self.breakpoints.get(path) {
            for line in lines {
                breakpoints.push(object(vec![("line", number((line + 1) as i64))]));
            }
        }
        self.send_request(
            PendingRequest::command("setBreakpoints"),
            object(vec![
                ("source", object(vec![("path", string(path))])),
                ("breakpoints", Value::Array(breakpoints)),
                ("linesStartAt1", bool_value(true)),
                ("sourceModified", bool_value(false)),
            ]),
        );
    }

    fn send_thread_request(&mut self, command: &str, clear_stopped: bool) -> bool {
        if !self.is_running() || !self.initialized {
            self.enqueue_error("debugger is not running");
            return false;
        }
        if self.stopped_thread_id <= 0 {
            self.enqueue_error("the program is not stopped");
            return false;
        }

        self.send_request(
            PendingRequest::command(command),
            object(vec![("threadId", number(self.stopped_thread_id))]),
        );

        if clear_stopped {
            self.stopped = false;
            self.stopped_path.clear();
            self.stopped_line = -1;
            self.clear_debug_state();
        }
        true
    }

    fn send_request(&mut self, request: PendingRequest, arguments: Value) -> i64 {
        self.next_sequence += 1;
        let sequence = self.next_sequence;
        let command = request.command.clone();
        self.pending_requests.insert(sequence, request);
        self.send_message(object(vec![
            ("seq", number(sequence)),
            ("type", string("request")),
            ("command", string(&command)),
            ("arguments", arguments),
        ]));
        sequence
    }

    fn send_message(&mut self, message: Value) {
        let text = json::to_string(&message, false);
        let abbreviated = message.get("type").and_then(Value::as_str) == Some("request")
            && message.get("command").and_then(Value::as_str) == Some("stackTrace");
        if abbreviated {
            self.log("-->", "stackTrace");
        } else {
            self.log("-->", &text);
        }

        let Some(input) = &mut self.input else {
            return;
        };
        let header = format!("Content-Length: {}\r\n\r\n", text.len());
        if let Err(error) = input
            .write_all(header.as_bytes())
            .and_then(|_| input.write_all(text.as_bytes()))
            .and_then(|_| input.flush())
        {
            self.log("info", &format!("send failed: {error}"));
            self.enqueue_error(&error.to_string());
        }
    }

    // -----------------------------------------------------------------
    // Incoming messages
    // -----------------------------------------------------------------

    fn handle_message(&mut self, message: Value) {
        let message_type = read_string(&message, "type");
        match message_type.as_str() {
            "event" => {
                let event_name = read_string(&message, "event");
                let body = message.get("body").cloned().unwrap_or(Value::Null);
                self.handle_event(&event_name, &body);
            }
            "response" => {
                let request_sequence = read_long(&message, "request_seq");
                let command = read_string(&message, "command");
                let success = read_bool(&message, "success");
                let request = self
                    .pending_requests
                    .remove(&request_sequence)
                    .unwrap_or_else(|| PendingRequest::command(&command));
                self.handle_response(request, &message, success);
            }
            "request" => {
                let command = read_string(&message, "command");
                let request_sequence = read_long(&message, "seq");
                self.next_sequence += 1;
                let sequence = self.next_sequence;
                self.send_message(object(vec![
                    ("seq", number(sequence)),
                    ("type", string("response")),
                    ("request_seq", number(request_sequence)),
                    ("success", bool_value(false)),
                    ("command", string(&command)),
                    (
                        "message",
                        string(&format!("unsupported DAP request: {command}")),
                    ),
                ]));
            }
            _ => {}
        }
    }

    fn handle_event(&mut self, event_name: &str, body: &Value) {
        match event_name {
            "initialized" => self.handle_initialized_event(),
            "stopped" => {
                let thread_id = read_int(body, "threadId");
                let reason = read_string(body, "reason");
                self.stopped = true;
                self.stopped_thread_id = thread_id;
                self.stop_reason = reason.clone();
                self.clear_debug_state();
                self.events.push(Event::Stopped {
                    adapter: self.adapter_name.clone(),
                    thread_id,
                    reason,
                });
                if thread_id > 0 {
                    self.send_request(
                        PendingRequest::command("stackTrace"),
                        object(vec![
                            ("threadId", number(thread_id)),
                            ("startFrame", number(0)),
                            ("levels", number(1)),
                        ]),
                    );
                }
            }
            "continued" => {
                self.stopped = false;
                self.stopped_thread_id = 0;
                self.stopped_path.clear();
                self.stopped_line = -1;
                self.clear_debug_state();
                self.events.push(Event::Continued {
                    adapter: self.adapter_name.clone(),
                });
            }
            "exited" => {
                let exit_code = read_int(body, "exitCode");
                self.events.push(Event::Exited {
                    adapter: self.adapter_name.clone(),
                    exit_code,
                    message: format!("exit code {exit_code}"),
                });
            }
            "terminated" => {
                self.events.push(Event::Terminated {
                    adapter: self.adapter_name.clone(),
                    message: "debug session terminated".into(),
                });
                self.stop_internal();
            }
            "output" => {
                self.events.push(Event::Output {
                    adapter: self.adapter_name.clone(),
                    category: read_string(body, "category"),
                    output: read_string(body, "output"),
                });
            }
            "breakpoint" => {
                if let Some(breakpoint) = parse_breakpoint(body) {
                    self.events.push(Event::Breakpoints {
                        adapter: self.adapter_name.clone(),
                        breakpoints: vec![breakpoint],
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_response(&mut self, request: PendingRequest, root: &Value, success: bool) {
        let command = request.command.clone();
        if !success {
            let mut message = read_string(root, "message");
            if message.is_empty() {
                if let Some(body) = root.get("body") {
                    message = read_string(body, "error");
                }
            }
            if message.is_empty() {
                message = format!("{command} failed");
            }

            if command == "variables" {
                self.complete_variables_request(request, Vec::new());
                return;
            }
            if command == "evaluate" {
                if request.generation != self.variables_generation {
                    return;
                }
                self.events.push(Event::WatchValue {
                    adapter: self.adapter_name.clone(),
                    watch: DapWatchValue {
                        expression: request.expression,
                        value: String::new(),
                        type_name: String::new(),
                        error: message,
                    },
                });
                return;
            }

            self.enqueue_error(&message);
            return;
        }

        let result = root.get("body").cloned().unwrap_or(Value::Null);
        match command.as_str() {
            "initialize" => self.handle_initialize_response(),
            "launch" | "attach" => self.configure_adapter_if_ready(),
            "setBreakpoints" => {
                self.events.push(Event::Breakpoints {
                    adapter: self.adapter_name.clone(),
                    breakpoints: parse_breakpoints(&result),
                });
            }
            "stackTrace" => {
                let mut path = String::new();
                let mut line = -1i64;
                let mut frame_id = 0i64;
                if let Some(frames) = result.get("stackFrames").and_then(Value::as_array) {
                    if let Some(frame) = frames.first() {
                        frame_id = read_int(frame, "id");
                        if let Some(source) = frame.get("source") {
                            path = read_string(source, "path");
                            if path.is_empty() {
                                path = read_string(source, "name");
                            }
                        }
                        let one_based_line = read_int(frame, "line");
                        if one_based_line > 0 {
                            line = one_based_line - 1;
                        }
                    }
                }
                self.stopped_frame_id = frame_id;
                self.stopped_path = path.clone();
                self.stopped_line = line;
                self.events.push(Event::StoppedLocation {
                    adapter: self.adapter_name.clone(),
                    path,
                    line,
                    reason: self.stop_reason.clone(),
                });
            }
            "scopes" => {
                if request.generation != self.variables_generation {
                    return;
                }
                let scopes = parse_scopes(&result);
                self.current_scopes.clear();
                self.current_scopes.extend(scopes.iter().cloned());
                self.variables_by_reference.clear();
                self.pending_variables_responses = 0;
                for scope in &scopes {
                    if scope.variables_reference > 0 {
                        self.pending_variables_responses += 1;
                    }
                }

                self.events.push(Event::Scopes {
                    adapter: self.adapter_name.clone(),
                    scopes,
                });

                if self.pending_variables_responses == 0 {
                    self.emit_variables();
                    return;
                }

                let references: Vec<i64> = self
                    .current_scopes
                    .iter()
                    .filter(|scope| scope.variables_reference > 0)
                    .map(|scope| scope.variables_reference)
                    .collect();
                for variables_reference in references {
                    self.send_request(
                        PendingRequest {
                            command: "variables".into(),
                            generation: request.generation,
                            variables_reference,
                            expression: String::new(),
                        },
                        object(vec![("variablesReference", number(variables_reference))]),
                    );
                }
            }
            "variables" => {
                self.complete_variables_request(request, parse_variables(&result));
            }
            "evaluate" => {
                if request.generation != self.variables_generation {
                    return;
                }
                self.events.push(Event::WatchValue {
                    adapter: self.adapter_name.clone(),
                    watch: DapWatchValue {
                        expression: request.expression,
                        value: read_string(&result, "result"),
                        type_name: read_string(&result, "type"),
                        error: String::new(),
                    },
                });
            }
            _ => {}
        }
    }

    fn complete_variables_request(&mut self, request: PendingRequest, variables: Vec<DapVariable>) {
        if request.generation != self.variables_generation {
            return;
        }
        self.variables_by_reference
            .insert(request.variables_reference, variables);
        if self.pending_variables_responses > 0 {
            self.pending_variables_responses -= 1;
        }
        if self.pending_variables_responses == 0 {
            self.emit_variables();
        }
    }

    fn emit_variables(&mut self) {
        let mut scoped = Vec::with_capacity(self.current_scopes.len());
        for scope in &self.current_scopes {
            let variables = if scope.variables_reference > 0 {
                self.variables_by_reference
                    .get(&scope.variables_reference)
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            scoped.push(DapScopeVariables {
                name: scope.name.clone(),
                variables,
            });
        }
        self.events.push(Event::Variables {
            adapter: self.adapter_name.clone(),
            scoped_variables: scoped,
        });
    }

    // -----------------------------------------------------------------
    // Lifecycle, logging and diagnostics
    // -----------------------------------------------------------------

    fn stop_internal(&mut self) {
        if self.child.is_none() && self.input.is_none() {
            return;
        }

        self.log("info", &format!("stopping {}", self.adapter_name));
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.input = None;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        self.incoming = None;

        self.config = None;
        self.launch_configuration = None;
        self.initialized = false;
        self.adapter_initialized = false;
        self.launch_sent = false;
        self.configuration_done = false;
        self.stopped = false;
        self.stopped_thread_id = 0;
        self.stopped_path.clear();
        self.stopped_line = -1;
        self.stop_reason.clear();
        self.pending_requests.clear();
        self.clear_debug_state();
    }

    fn clear_debug_state(&mut self) {
        self.stopped_frame_id = 0;
        self.variables_generation += 1;
        self.current_scopes.clear();
        self.variables_by_reference.clear();
        self.pending_variables_responses = 0;
    }

    fn enqueue_error(&mut self, message: &str) {
        self.events.push(Event::Error {
            adapter: self.adapter_name.clone(),
            message: message.to_string(),
        });
    }

    fn log(&mut self, channel: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = if text.chars().count() > MAX_LOG_ENTRY_LENGTH {
            let truncated: String = text.chars().take(MAX_LOG_ENTRY_LENGTH).collect();
            format!(
                "{truncated}\n… truncated ({} chars total)",
                text.chars().count()
            )
        } else {
            text.to_string()
        };
        self.log
            .push_back(format!("{} {channel} {text}", timestamp_time()));
        while self.log.len() > MAX_LOG_ENTRIES {
            self.log.pop_front();
        }
    }
}

// ---------------------------------------------------------------------------
// Process I/O
// ---------------------------------------------------------------------------

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
            let raw = String::from_utf8_lossy(&body).into_owned();
            match json::parse(&raw) {
                Ok(value) => {
                    if sender.send(Incoming::Message(value, raw)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    if sender.send(Incoming::Invalid(raw)).is_err() {
                        return;
                    }
                }
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
// Adapter discovery, paths and logging
// ---------------------------------------------------------------------------

fn find_adapter(adapter: &str) -> Option<String> {
    if let Some(path) = find_executable(adapter) {
        return Some(path);
    }

    // LLVM installs version-suffixed DAP binaries on several distributions
    // (for example lldb-dap-18). Prefer the newest one we can find.
    if adapter.eq_ignore_ascii_case("lldb-dap") {
        for version in (14..=20).rev() {
            if let Some(path) = find_executable(&format!("lldb-dap-{version}")) {
                return Some(path);
            }
        }
    }
    None
}

fn find_executable(command: &str) -> Option<String> {
    crate::util::git::command_path(command)
        .map(|path| path.to_string_lossy().into_owned())
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    crate::util::git::absolute(Path::new(path))
        .to_string_lossy()
        .into_owned()
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn log_path() -> PathBuf {
    home_directory().join(".hexal/hexal.log")
}

fn file_log(message: &str) {
    const MAX_LOG_BYTES: u64 = 1024 * 1024;
    let path = log_path();
    let Some(directory) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        let _ = std::fs::write(&path, []);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{} {message}", timestamp_utc());
    }
}

// ---------------------------------------------------------------------------
// Time stamps
// ---------------------------------------------------------------------------

fn timestamp_time() -> String {
    let now = crate::util::time::now_local();
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        now.hour, now.minute, now.second, now.millis
    )
}

fn timestamp_date_time() -> String {
    let now = crate::util::time::now_local();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        now.year, now.month, now.day, now.hour, now.minute, now.second, now.millis
    )
}

fn timestamp_utc() -> String {
    crate::util::time::format_utc(crate::util::time::now_utc())
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
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

fn read_int(element: &Value, property: &str) -> i64 {
    element.get(property).and_then(Value::as_i64).unwrap_or(0)
}

fn read_long(element: &Value, property: &str) -> i64 {
    read_int(element, property)
}

fn read_string(element: &Value, property: &str) -> String {
    element
        .get(property)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn read_bool(element: &Value, property: &str) -> bool {
    element
        .get(property)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn parse_scopes(result: &Value) -> Vec<DapScope> {
    let Some(items) = result.get("scopes").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| DapScope {
            name: read_string(item, "name"),
            variables_reference: read_int(item, "variablesReference"),
            expensive: read_bool(item, "expensive"),
        })
        .collect()
}

fn parse_variables(result: &Value) -> Vec<DapVariable> {
    let Some(items) = result.get("variables").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| DapVariable {
            name: read_string(item, "name"),
            value: read_string(item, "value"),
            type_name: read_string(item, "type"),
            variables_reference: read_int(item, "variablesReference"),
            evaluate_name: read_string(item, "evaluateName"),
        })
        .collect()
}

fn parse_breakpoints(result: &Value) -> Vec<DapBreakpoint> {
    let Some(items) = result.get("breakpoints").and_then(Value::as_array) else {
        return Vec::new();
    };
    items.iter().filter_map(parse_breakpoint).collect()
}

fn parse_breakpoint(element: &Value) -> Option<DapBreakpoint> {
    let line = read_int(element, "line");
    if line <= 0 {
        return None;
    }
    Some(DapBreakpoint {
        line,
        verified: read_bool(element, "verified"),
        message: read_string(element, "message"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn performs_handshake_breakpoints_variables_and_evaluate() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let dir = std::env::temp_dir().join(format!("hexal-dap-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("sample.rs");
        fs::write(&source, b"fn main() {\n    let x = 42;\n}\n").unwrap();
        let source_text = source.to_string_lossy().into_owned();

        let adapter = dir.join("adapter.py");
        fs::write(
            &adapter,
            format!(
                r#"
import json, sys

source = {source_text:?}
next_seq = 1

def read():
    headers = {{}}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line and line not in (b'\n', b'\r\n'):
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    return json.loads(sys.stdin.buffer.read(int(headers.get('content-length', '0'))))

def send(message):
    global next_seq
    if 'seq' not in message:
        message['seq'] = next_seq
        next_seq += 1
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode())
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

def respond(request, body=None):
    send({{'type':'response','request_seq':request['seq'],'success':True,
          'command':request['command'],'body':body or {{}}}})

while True:
    message = read()
    if message is None:
        break
    command = message.get('command')
    if command == 'initialize':
        respond(message, {{'supportsConfigurationDoneRequest': True}})
        send({{'type':'event','event':'initialized','body':{{}}}})
    elif command == 'configurationDone':
        respond(message)
        send({{'type':'event','event':'stopped','body':{{'threadId':1,'reason':'breakpoint'}}}})
    elif command == 'setBreakpoints':
        points = [{{'line': point['line'], 'verified': True}} for point in message['arguments']['breakpoints']]
        respond(message, {{'breakpoints': points}})
    elif command == 'stackTrace':
        respond(message, {{'stackFrames':[{{
            'id': 7, 'line': 2, 'source': {{'path': source}}
        }}]}})
    elif command == 'scopes':
        respond(message, {{'scopes':[{{
            'name':'Locals','variablesReference':100,'expensive':False
        }}]}})
    elif command == 'variables':
        respond(message, {{'variables':[{{
            'name':'x','value':'42','type':'int',
            'variablesReference':0,'evaluateName':'x'
        }}]}})
    elif command == 'evaluate':
        respond(message, {{'result':'42','type':'int'}})
    elif command == 'continue':
        respond(message)
        send({{'type':'event','event':'continued','body':{{}}}})
    elif command == 'disconnect':
        respond(message)
        break
    else:
        respond(message)
"#,
                source_text = source_text
            ),
        )
        .unwrap();

        let workspace = dir.to_string_lossy().into_owned();
        let config = DebuggerConfig {
            adapter: "python3".into(),
            arguments: vec![adapter.to_string_lossy().into_owned()],
            configuration: json::parse(r#"{"request":"launch","program":"sample"}"#).unwrap(),
            start: DebuggerStart::Stdio,
        };
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(normalize_path(&source_text), BTreeSet::from([0usize]));

        let mut manager = Manager::new();
        manager
            .try_start(
                config,
                &source_text,
                &workspace,
                json::parse(r#"{"request":"launch","program":"sample"}"#).unwrap(),
                &breakpoints,
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        let mut breakpoint_verified = false;
        let mut stopped_location = false;
        while Instant::now() < deadline && !(breakpoint_verified && stopped_location) {
            manager.poll();
            for event in manager.take_events() {
                if let Event::Breakpoints { breakpoints, .. } = &event {
                    breakpoint_verified = breakpoints.iter().any(|point| point.verified);
                }
                if matches!(event, Event::StoppedLocation { .. }) {
                    stopped_location = true;
                }
                events.push(event);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            breakpoint_verified,
            "no verified breakpoint event: {events:?}"
        );
        assert!(stopped_location, "no stopped location event: {events:?}");
        assert!(manager.is_initialized());
        assert!(manager.is_stopped());

        assert!(manager.request_debug_variables());
        assert!(manager.request_evaluate("x"));

        let mut scopes = false;
        let mut variables = false;
        let mut watch = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !(scopes && variables && watch) {
            manager.poll();
            for event in manager.take_events() {
                match &event {
                    Event::Scopes { .. } => scopes = true,
                    Event::Variables {
                        scoped_variables, ..
                    } => {
                        variables = scoped_variables
                            .iter()
                            .any(|scope| scope.variables.iter().any(|item| item.value == "42"));
                    }
                    Event::WatchValue { watch: value, .. } => watch = value.value == "42",
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(scopes, "no scopes event");
        assert!(variables, "no variables event");
        assert!(watch, "no watch event");

        let report = manager.build_debug_report();
        assert!(report.contains("Hexal DAP diagnostics"));
        assert!(report.contains("Breakpoints: 1"));
        assert!(report.contains("<--"));
        assert!(report.contains(&format!("{}: 1", normalize_path(&source_text))));

        assert!(manager.continue_debuggee());
        let mut continued = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !continued {
            manager.poll();
            for event in manager.take_events() {
                if matches!(event, Event::Continued { .. }) {
                    continued = true;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(continued, "no continued event");
        manager.stop();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn connects_to_a_tcp_debug_adapter() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut pending = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let read = std::io::Read::read(&mut stream, &mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                pending.extend_from_slice(&buffer[..read]);
                while let Some((body, consumed)) = take_message(&pending) {
                    pending.drain(..consumed);
                    let message = json::parse(&String::from_utf8_lossy(&body)).unwrap();
                    if message.get("type").and_then(Value::as_str) != Some("request") {
                        continue;
                    }
                    let sequence = message.get("seq").and_then(Value::as_i64).unwrap_or(0);
                    let command = message
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let mut response = object(vec![
                        ("seq", number(1)),
                        ("type", string("response")),
                        ("request_seq", number(sequence)),
                        ("success", bool_value(true)),
                        ("command", string(&command)),
                    ]);
                    if command == "initialize" {
                        response.set("body", object(vec![("capabilities", object(Vec::new()))]));
                    } else if command == "setBreakpoints" {
                        response.set(
                            "body",
                            object(vec![(
                                "breakpoints",
                                Value::Array(vec![object(vec![
                                    ("verified", bool_value(true)),
                                    ("line", number(1)),
                                ])]),
                            )]),
                        );
                    }
                    let text = json::to_string(&response, false);
                    write!(stream, "Content-Length: {}\r\n\r\n{}", text.len(), text).unwrap();
                    let _ = stream.flush();
                    if command == "launch" {
                        let event = object(vec![
                            ("seq", number(2)),
                            ("type", string("event")),
                            ("event", string("initialized")),
                            ("body", object(Vec::new())),
                        ]);
                        let text = json::to_string(&event, false);
                        write!(stream, "Content-Length: {}\r\n\r\n{}", text.len(), text).unwrap();
                        let _ = stream.flush();
                    }
                    if command == "disconnect" {
                        return;
                    }
                }
            }
        });

        let directory =
            std::env::temp_dir().join(format!("hexal-dap-tcp-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&directory);
        let source = directory.join("Main.java");
        fs::write(&source, b"class Main {}\n").unwrap();
        let source = source.to_string_lossy().into_owned();
        let workspace = directory.to_string_lossy().into_owned();
        let config = DebuggerConfig {
            adapter: "tcp-test".into(),
            arguments: Vec::new(),
            configuration: json::parse(r#"{"request":"launch"}"#).unwrap(),
            start: DebuggerStart::Stdio,
        };
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(normalize_path(&source), BTreeSet::from([0usize]));

        let mut manager = Manager::new();
        manager
            .try_start_server(
                config,
                "127.0.0.1",
                port,
                &source,
                &workspace,
                json::parse(r#"{"request":"launch"}"#).unwrap(),
                &breakpoints,
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut verified = false;
        while Instant::now() < deadline && !verified {
            manager.poll();
            for event in manager.take_events() {
                if let Event::Breakpoints { breakpoints, .. } = event {
                    verified = breakpoints.iter().any(|point| point.verified);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(verified, "no verified breakpoint over TCP");
        assert!(manager.is_running());
        manager.stop();
        let _ = server.join();
        let _ = fs::remove_dir_all(&directory);
    }
}
