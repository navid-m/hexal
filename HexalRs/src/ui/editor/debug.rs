//! Breakpoints, debug sessions, variables and the DAP diagnostics modal.
//!
//! This mirrors the C# editor's `EditorDebug` and `EditorDapDebug` partial
//! classes, including the exact command names, status messages and launch
//! configuration placeholder expansion.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::lsp_ui::wrap_text;
use super::*;
use crate::dap::Event as DapEvent;
use crate::ui::theme::{is_true_color, rgb, Style};

const MOUSE_SCROLL_LINES: usize = 3;

struct DapLaunchContext {
    workspace: String,
    file: String,
    file_directory: String,
    file_name: String,
    file_name_without_extension: String,
    executable: Option<String>,
}

impl Editor {
    // ---------------------------------------------------------------------
    // Line decorations
    // ---------------------------------------------------------------------

    pub(super) fn line_highlight_style(&mut self, line: usize) -> Option<Style> {
        if self.is_breakpoint_at(line) {
            return Some(self.line_tint(true));
        }
        if self.is_current_debug_line(line) {
            return Some(self.line_tint(false));
        }
        None
    }

    fn line_tint(&self, red: bool) -> Style {
        let dark = self.theme_background_is_dark();
        let background = if red {
            // A red tint tuned to the theme's background luminance keeps the
            // breakpoint line visible without washing out syntax colours.
            if dark {
                rgb(78, 18, 25)
            } else {
                rgb(247, 207, 212)
            }
        } else if dark {
            rgb(56, 44, 14)
        } else {
            rgb(250, 235, 190)
        };
        Style {
            fg: self.theme.foreground,
            bg: background,
            ..Style::default()
        }
    }

    fn theme_background_is_dark(&self) -> bool {
        let background = self.theme.background;
        if !is_true_color(background) {
            // ANSI palette indices 0-6 are dark; 7 is light grey, and 8-15
            // are the bright variants. For the extended palette use a simple
            // cube approximation.
            if background < 7 {
                return true;
            }
            if background < 16 {
                return background < 15;
            }
            if background < 232 {
                let value = background as i64 - 16;
                let levels = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
                let red = levels[(value / 36) as usize];
                let green = levels[((value / 6) % 6) as usize];
                let blue = levels[(value % 6) as usize];
                let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
                return luminance < 128.0;
            }
            return background < 244;
        }

        let luminance = 0.2126 * color_red(background) as f64
            + 0.7152 * color_green(background) as f64
            + 0.0722 * color_blue(background) as f64;
        luminance < 128.0
    }

    fn update_debug_path(&mut self) {
        let source = self.doc.path.as_deref().unwrap_or("");
        if source != self.debug_path_source {
            self.debug_path_source = source.to_string();
            self.debug_path = if source.is_empty() {
                String::new()
            } else {
                normalize_debug_path(source)
            };
        }
    }

    fn is_breakpoint_at(&mut self, line: usize) -> bool {
        if self.breakpoints.is_empty() {
            return false;
        }
        self.update_debug_path();
        if self.debug_path.is_empty() {
            return false;
        }
        self.breakpoints
            .get(&self.debug_path)
            .is_some_and(|lines| lines.contains(&line))
    }

    fn is_current_debug_line(&mut self, line: usize) -> bool {
        if !self.dap_stopped || self.dap_stopped_line < 0 || self.dap_stopped_path.is_empty() {
            return false;
        }
        if self.dap_stopped_line != line as i64 {
            return false;
        }
        self.update_debug_path();
        self.debug_path == self.dap_stopped_path
    }

    // ---------------------------------------------------------------------
    // Breakpoints and watches
    // ---------------------------------------------------------------------

    pub(super) fn toggle_breakpoint(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }

        let language = self.active_language();
        let Some(language) = language else {
            self.set_error("no language is active for breakpoints");
            return;
        };
        if language.debugger.is_none() {
            self.set_error(&format!(
                "no debug adapter is configured for {}",
                language.name
            ));
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            self.set_error("breakpoints require a file-backed buffer");
            return;
        };

        let path = normalize_debug_path(&path);
        let line = self.cursor_line();
        let lines = self.breakpoints.entry(path.clone()).or_default();
        let removed = lines.remove(&line);
        if !removed {
            lines.insert(line);
        }
        if lines.is_empty() {
            self.breakpoints.remove(&path);
        }
        let breakpoints = self.breakpoints.get(&path).cloned().unwrap_or_default();

        if removed {
            self.set_message(&format!("Breakpoint removed at line {}.", line + 1));
        } else {
            self.set_message(&format!("Breakpoint set at line {}.", line + 1));
        }

        if self.dap.is_initialized() {
            self.dap.set_breakpoints(&path, &breakpoints);
        }
        self.force_full_redraw = true;
    }

    pub(super) fn toggle_debug_variables(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }

        self.debug_variables_visible = !self.debug_variables_visible;
        self.debug_variables_top = 0;
        if self.debug_variables_visible {
            self.request_debug_state();
            self.set_message("Debug variables shown.");
        } else {
            self.set_message("Debug variables hidden.");
        }
        self.force_full_redraw = true;
    }

    pub(super) fn add_debug_watch(&mut self, expression: &str) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            self.set_error("usage: :dbg-watch <expression>");
            return;
        }

        if !self.debug_watches.contains(&expression) {
            self.debug_watches.push(expression.clone());
        }
        self.debug_watch_values.remove(&expression);
        self.debug_variables_visible = true;
        self.debug_variables_top = 0;
        self.rebuild_debug_variable_lines();
        if self.dap.is_stopped() {
            self.dap.request_evaluate(&expression);
        }
        self.set_message(&format!("Watching '{expression}'."));
        self.force_full_redraw = true;
    }

    pub(super) fn remove_debug_watch(&mut self, expression: &str) {
        let value = expression.trim();
        let before = self.debug_watches.len();
        self.debug_watches.retain(|watch| watch != value);
        if self.debug_watches.len() == before {
            self.set_error(&format!("not watching '{value}'"));
            return;
        }

        self.debug_watch_values.remove(value);
        self.rebuild_debug_variable_lines();
        self.set_message(&format!("Stopped watching '{value}'."));
        self.force_full_redraw = true;
    }

    pub(super) fn clear_debug_watches(&mut self) {
        self.debug_watches.clear();
        self.debug_watch_values.clear();
        self.rebuild_debug_variable_lines();
        self.set_message("Debug watches cleared.");
        self.force_full_redraw = true;
    }

    fn request_debug_state(&mut self) {
        if !self.debug_variables_visible && self.debug_watches.is_empty() {
            return;
        }

        self.debug_scopes.clear();
        self.debug_scoped_variables.clear();
        self.debug_watch_values.clear();
        self.rebuild_debug_variable_lines();

        if !self.dap.is_stopped() {
            return;
        }

        self.dap.request_debug_variables();
        for watch in self.debug_watches.clone() {
            self.dap.request_evaluate(&watch);
        }
    }

    fn clear_debug_variable_values(&mut self) {
        self.debug_scopes.clear();
        self.debug_scoped_variables.clear();
        self.debug_watch_values.clear();
        self.rebuild_debug_variable_lines();
    }

    fn rebuild_debug_variable_lines(&mut self) {
        let text_style = self.theme.text();
        let muted_style = self.theme.muted_style();
        let accent_style = self.theme.accent_style();
        let error_style = self.theme.error_style();

        self.debug_variable_lines.clear();

        if !self.dap.is_running() {
            self.debug_variable_lines
                .push(("No debug session.".into(), muted_style));
            return;
        }

        if !self.dap.is_stopped() {
            self.debug_variable_lines
                .push(("Program is running.".into(), muted_style));
            if !self.debug_watches.is_empty() {
                self.debug_variable_lines.push((String::new(), text_style));
            }
        }

        if !self.debug_watches.is_empty() {
            self.debug_variable_lines
                .push(("Watches".into(), accent_style));
            for expression in &self.debug_watches {
                if let Some(watch) = self.debug_watch_values.get(expression) {
                    if !watch.error.is_empty() {
                        self.debug_variable_lines
                            .push((format!("  {expression} = <{}>", watch.error), error_style));
                    } else {
                        let type_text = if watch.type_name.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", watch.type_name)
                        };
                        self.debug_variable_lines.push((
                            format!(
                                "  {expression} = {}{type_text}",
                                sanitize_debug_value(&watch.value)
                            ),
                            text_style,
                        ));
                    }
                } else {
                    self.debug_variable_lines
                        .push((format!("  {expression} = …"), muted_style));
                }
            }
            self.debug_variable_lines.push((String::new(), text_style));
        }

        if self.debug_scopes.is_empty() {
            self.debug_variable_lines.push((
                if self.dap.is_stopped() {
                    "Waiting for scopes…".into()
                } else {
                    "Not stopped.".into()
                },
                muted_style,
            ));
            return;
        }

        for scope in &self.debug_scopes {
            self.debug_variable_lines
                .push((format!("[{}]", scope.name), accent_style));
            let values = self
                .debug_scoped_variables
                .iter()
                .find(|item| item.name == scope.name);
            let Some(values) = values else {
                self.debug_variable_lines
                    .push(("  (no variables)".into(), muted_style));
                continue;
            };
            if values.variables.is_empty() {
                self.debug_variable_lines
                    .push(("  (no variables)".into(), muted_style));
                continue;
            }

            for variable in &values.variables {
                let type_text = if variable.type_name.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", variable.type_name)
                };
                self.debug_variable_lines.push((
                    format!(
                        "  {} = {}{type_text}",
                        variable.name,
                        sanitize_debug_value(&variable.value)
                    ),
                    text_style,
                ));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Session control
    // ---------------------------------------------------------------------

    pub(super) fn start_debug_session(&mut self, arguments: &[String]) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }

        let language = self.active_language();
        let Some(language) = language else {
            self.set_error("no language is active for debugging");
            return;
        };
        let Some(debugger) = language.debugger.clone() else {
            self.set_error(&format!(
                "no debug adapter is configured for {}",
                language.name
            ));
            return;
        };

        let Some(document_path) = self.doc.path.clone() else {
            self.set_error("debugging requires a file-backed buffer");
            return;
        };

        if arguments.len() > 1 {
            self.set_error("usage: :debug [executable]");
            return;
        }

        let executable = if arguments.len() == 1 {
            Some(normalize_debug_path(&arguments[0]))
        } else {
            self.find_default_debug_executable(&document_path)
        };

        if executable.is_none() && debugger_uses_executable(&debugger.configuration) {
            self.set_error("debug: no executable found; use :debug <program>");
            return;
        }

        let mut configuration = debugger.configuration.clone();
        if let Some(executable) = &executable {
            if configuration.get("program").is_none() {
                configuration.set(
                    "program",
                    crate::util::json::Value::String(executable.clone()),
                );
            }
        }

        let source_path = normalize_debug_path(&document_path);
        let source = Path::new(&source_path);
        let workspace = self.debug_workspace_root();
        let file_directory = source
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| workspace.clone());
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_name_without_extension = source
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let context = DapLaunchContext {
            workspace: workspace.clone(),
            file: source_path.clone(),
            file_directory,
            file_name,
            file_name_without_extension,
            executable,
        };

        if let Err(error) = try_resolve_dap_node(&mut configuration, &context) {
            self.set_error(&error);
            return;
        }

        match self.dap.try_start(
            debugger.clone(),
            &source_path,
            &workspace,
            configuration,
            &self.breakpoints,
        ) {
            Ok(()) => {
                self.dap_status = format!("DBG: starting {}", debugger.adapter);
                self.dap_stopped = false;
                self.dap_stopped_path.clear();
                self.dap_stopped_line = -1;
                self.clear_message();
                self.set_message(&format!("Debug: starting {}.", debugger.adapter));
                self.force_full_redraw = true;
            }
            Err(error) => self.set_error(&format!("debug: {error}")),
        }
    }

    pub(super) fn continue_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        if !self.dap.continue_debuggee() {
            self.set_error("debugger is not stopped");
            return;
        }
        self.dap_status = "DBG: running".into();
        self.dap_stopped = false;
        self.dap_stopped_path.clear();
        self.dap_stopped_line = -1;
        self.clear_debug_variable_values();
        self.set_message("Debug: continued.");
    }

    pub(super) fn next_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        if !self.dap.next() {
            self.set_error("debugger is not stopped");
            return;
        }
        self.dap_status = "DBG: stepping".into();
        self.dap_stopped = false;
        self.clear_debug_variable_values();
        self.set_message("Debug: step over.");
    }

    pub(super) fn step_in_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        if !self.dap.step_in() {
            self.set_error("debugger is not stopped");
            return;
        }
        self.dap_status = "DBG: stepping".into();
        self.dap_stopped = false;
        self.clear_debug_variable_values();
        self.set_message("Debug: step in.");
    }

    pub(super) fn step_out_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        if !self.dap.step_out() {
            self.set_error("debugger is not stopped");
            return;
        }
        self.dap_status = "DBG: stepping".into();
        self.dap_stopped = false;
        self.clear_debug_variable_values();
        self.set_message("Debug: step out.");
    }

    pub(super) fn pause_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        if !self.dap.pause() {
            self.set_error("debugger is not running");
            return;
        }
        self.dap_status = "DBG: pausing".into();
        self.set_message("Debug: pause requested.");
    }

    pub(super) fn stop_debug_session(&mut self) {
        if !self.ensure_debug_adapter_enabled() {
            return;
        }
        self.dap.stop();
        self.dap_status.clear();
        self.dap_stopped = false;
        self.dap_stopped_path.clear();
        self.dap_stopped_line = -1;
        self.clear_debug_variable_values();
        self.set_message("Debug session stopped.");
        self.force_full_redraw = true;
    }

    fn ensure_debug_adapter_enabled(&mut self) -> bool {
        if self.config.debug_adapter {
            return true;
        }
        self.set_error("debug adapter support is disabled (debugAdapter in .hexcfg)");
        false
    }

    pub(super) fn apply_debug_adapter_configuration(&mut self) {
        if self.config.debug_adapter {
            return;
        }

        if self.dap.is_running() {
            self.dap.stop();
        }
        self.dap_status.clear();
        self.dap_stopped = false;
        self.dap_stopped_path.clear();
        self.dap_stopped_line = -1;
    }

    pub(super) fn poll_dap(&mut self) {
        self.dap.poll();
        let events = self.dap.take_events();
        for event in events {
            self.handle_dap_event(event);
        }
        if self.dap_debug.visible {
            self.refresh_dap_debug_lines();
        }
    }

    fn handle_dap_event(&mut self, event: DapEvent) {
        match event {
            DapEvent::Started { adapter } => {
                self.dap_status = format!("DBG: starting {adapter}");
            }
            DapEvent::Initialized { adapter } => {
                self.dap_status = format!("DBG: {adapter}");
            }
            DapEvent::Stopped {
                thread_id: _,
                reason,
                ..
            } => {
                self.dap_stopped = true;
                self.dap_status = if reason.is_empty() {
                    "DBG: stopped".into()
                } else {
                    format!("DBG: stopped ({reason})")
                };
                if !reason.is_empty() {
                    self.set_message(&format!("Debug: stopped ({reason})."));
                }
                self.force_full_redraw = true;
            }
            DapEvent::StoppedLocation {
                path,
                line,
                reason: _,
                ..
            } => {
                self.dap_stopped = true;
                self.dap_stopped_path = if path.is_empty() {
                    String::new()
                } else {
                    normalize_debug_path(&path)
                };
                self.dap_stopped_line = line;
                let location = if line >= 0 && !path.is_empty() {
                    format!(
                        "{}:{}",
                        Path::new(&path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone()),
                        line + 1
                    )
                } else {
                    "an unknown location".into()
                };
                self.dap_status = format!("DBG: stopped {location}");
                self.set_message(&format!("Debug: stopped at {location}."));

                if line >= 0 && !path.is_empty() {
                    let same = self.doc.path.as_deref().is_some_and(|current| {
                        git::same_path(Path::new(current), Path::new(&path))
                    });
                    if same {
                        self.move_to_line_character(line, 0);
                    } else if !self.doc.is_dirty() && Path::new(&path).is_file() {
                        self.request_open(&path, line.max(0) as usize, 0, false);
                    }
                }

                self.request_debug_state();
                self.force_full_redraw = true;
            }
            DapEvent::Continued { .. } => {
                self.dap_stopped = false;
                self.dap_stopped_path.clear();
                self.dap_stopped_line = -1;
                self.dap_status = "DBG: running".into();
                self.clear_debug_variable_values();
                self.set_message("Debug: running.");
                self.force_full_redraw = true;
            }
            DapEvent::Exited { exit_code, .. } => {
                self.dap_stopped = false;
                self.dap_stopped_path.clear();
                self.dap_stopped_line = -1;
                self.dap_status = format!("DBG: exited ({exit_code})");
                self.clear_debug_variable_values();
                self.set_message(&format!("Debug: program exited ({exit_code})."));
                self.force_full_redraw = true;
            }
            DapEvent::Terminated { .. } => {
                self.dap_stopped = false;
                self.dap_stopped_path.clear();
                self.dap_stopped_line = -1;
                self.dap_status.clear();
                self.clear_debug_variable_values();
                self.set_message("Debug session terminated.");
                self.force_full_redraw = true;
            }
            DapEvent::Scopes { scopes, .. } => {
                self.debug_scopes = scopes;
                self.rebuild_debug_variable_lines();
                self.force_full_redraw = true;
            }
            DapEvent::Variables {
                scoped_variables, ..
            } => {
                self.debug_scoped_variables = scoped_variables;
                self.rebuild_debug_variable_lines();
                self.force_full_redraw = true;
            }
            DapEvent::WatchValue { watch, .. } => {
                self.debug_watch_values
                    .insert(watch.expression.clone(), watch);
                self.rebuild_debug_variable_lines();
                self.force_full_redraw = true;
            }
            DapEvent::Breakpoints { breakpoints, .. } => {
                for breakpoint in breakpoints {
                    if !breakpoint.verified && !breakpoint.message.is_empty() {
                        self.set_error(&format!(
                            "debug breakpoint {}: {}",
                            breakpoint.line, breakpoint.message
                        ));
                    }
                }
            }
            DapEvent::Error { message, .. } => {
                self.set_error(&format!("DAP: {message}"));
            }
            DapEvent::Output { .. } => {}
        }
    }

    pub(super) fn debug_status_text(&self) -> &str {
        if !self.config.debug_adapter || self.dap_status.is_empty() {
            return "";
        }
        &self.dap_status
    }

    // ---------------------------------------------------------------------
    // Launch configuration helpers
    // ---------------------------------------------------------------------

    fn debug_workspace_root(&self) -> String {
        std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| self.sidebar.root.to_string_lossy().into_owned())
    }

    fn find_default_debug_executable(&self, source_path: &str) -> Option<String> {
        let source_full = normalize_debug_path(source_path);
        let source = Path::new(&source_full);
        let workspace = self.debug_workspace_root();
        let root = Path::new(&workspace);
        let directory = source
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| root.to_path_buf());
        let name = source
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return None;
        }
        let candidates = [
            directory.join(&name),
            root.join(&name),
            root.join("build").join(&name),
            root.join("bin").join(&name),
            root.join("target").join("debug").join(&name),
            root.join("target").join("release").join(&name),
            root.join("zig-out").join("bin").join(&name),
            root.join("out").join(&name),
            directory.join("build").join(&name),
            directory.join("bin").join(&name),
        ];

        candidates
            .iter()
            .find(|candidate| candidate.is_file())
            .map(|candidate| normalize_debug_path(&candidate.to_string_lossy()))
    }

    /// Parses the text after an Ex command name with the same quoting rules
    /// as the C# editor: quote whole arguments, `\"` inside double quotes.
    pub(super) fn parse_debug_command_arguments(input: &str) -> Result<Vec<String>, String> {
        let chars: Vec<char> = input.chars().collect();
        let mut arguments = Vec::new();
        let mut position = 0usize;
        while position < chars.len() {
            while position < chars.len() && chars[position].is_whitespace() {
                position += 1;
            }
            if position >= chars.len() {
                break;
            }

            let first = chars[position];
            if first == '"' || first == '\'' {
                let quote = first;
                position += 1;
                let mut argument = String::new();
                while position < chars.len() && chars[position] != quote {
                    if quote == '"'
                        && chars[position] == '\\'
                        && position + 1 < chars.len()
                        && chars[position + 1] == '"'
                    {
                        argument.push('"');
                        position += 2;
                        continue;
                    }
                    argument.push(chars[position]);
                    position += 1;
                }
                if position >= chars.len() {
                    return Err(if quote == '"' {
                        "unterminated double quote".into()
                    } else {
                        "unterminated single quote".into()
                    });
                }
                position += 1;
                arguments.push(argument);
                continue;
            }

            let start = position;
            while position < chars.len() && !chars[position].is_whitespace() {
                position += 1;
            }
            arguments.push(chars[start..position].iter().collect());
        }
        Ok(arguments)
    }

    // ---------------------------------------------------------------------
    // Debug variable panel
    // ---------------------------------------------------------------------

    pub(super) fn render_debug_variables(&mut self) {
        if !self.debug_variables_visible {
            return;
        }

        let height = self.status_row();
        if height < 4 || self.cols < 20 {
            return;
        }

        let mut width = (self.cols / 3).clamp(24, 56);
        if width > self.cols.saturating_sub(4) {
            width = self.cols.saturating_sub(4).max(20);
        }
        let x = self.cols.saturating_sub(width);
        self.draw_box(x, 0, width, height, " Debug variables ");

        let content_rows = height.saturating_sub(3).max(1);
        let content_width = width.saturating_sub(4);
        let max_top = self.debug_variable_lines.len().saturating_sub(content_rows);
        self.debug_variables_top = self.debug_variables_top.min(max_top);

        for row in 0..content_rows {
            let index = self.debug_variables_top + row;
            if index >= self.debug_variable_lines.len() {
                break;
            }
            let (text, style) = &self.debug_variable_lines[index];
            self.screen.put_str(
                (x + 2) as i64,
                (1 + row) as i64,
                text,
                *style,
                content_width,
            );
        }

        self.screen.put_str(
            (x + 2) as i64,
            (height - 2) as i64,
            ":dbg-sv close  :dbg-watch <expr>",
            self.theme.muted_style(),
            width.saturating_sub(4),
        );
    }

    // ---------------------------------------------------------------------
    // DAP debug modal
    // ---------------------------------------------------------------------

    pub(super) fn open_dap_debug(&mut self) {
        self.dap_debug.visible = true;
        self.dap_debug.top = 0;
        self.dap_debug.status.clear();
        self.refresh_dap_debug_lines();
        self.clear_message();
        self.force_full_redraw = true;
    }

    fn close_dap_debug(&mut self) {
        self.dap_debug.visible = false;
        self.dap_debug.top = 0;
        self.dap_debug.status.clear();
        self.dap_debug_lines.clear();
        self.force_full_redraw = true;
    }

    fn dap_debug_rows(&self) -> usize {
        self.rows.saturating_sub(3).max(1)
    }

    fn dap_debug_max_top(&self) -> usize {
        self.dap_debug_lines
            .len()
            .saturating_sub(self.dap_debug_rows())
    }

    fn refresh_dap_debug_lines(&mut self) {
        let width = self.cols.saturating_sub(4).max(10);
        let report = self.dap.build_debug_report();
        self.dap_debug_lines.clear();
        for raw_line in report.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
            if raw_line.is_empty() {
                self.dap_debug_lines.push(String::new());
                continue;
            }
            let wrapped = wrap_text(raw_line, width);
            if wrapped.is_empty() {
                self.dap_debug_lines.push(String::new());
            } else {
                self.dap_debug_lines.extend(wrapped);
            }
        }
    }

    fn scroll_dap_debug(&mut self, delta: isize) {
        self.refresh_dap_debug_lines();
        let max_top = self.dap_debug_max_top();
        let top = self.dap_debug.top as isize + delta;
        self.dap_debug.top = top.clamp(0, max_top as isize) as usize;
    }

    fn copy_dap_debug_report(&mut self) {
        let report = self.dap.build_debug_report();
        if self.store_clipboard(report.as_bytes()) {
            self.dap_debug.status = format!(
                "Copied {} characters to the clipboard.",
                report.encode_utf16().count()
            );
        } else {
            self.dap_debug.status = "Copy failed.".to_string();
        }
    }

    pub(super) fn handle_dap_debug_input(&mut self, event: Event) {
        match event.key {
            Key::Escape | Key::Enter => {
                self.close_dap_debug();
                return;
            }
            Key::Up => {
                self.scroll_dap_debug(-1);
                return;
            }
            Key::Down => {
                self.scroll_dap_debug(1);
                return;
            }
            Key::ScrollUp => {
                self.scroll_dap_debug(-(MOUSE_SCROLL_LINES as isize));
                return;
            }
            Key::ScrollDown => {
                self.scroll_dap_debug(MOUSE_SCROLL_LINES as isize);
                return;
            }
            Key::PageUp => {
                let rows = self.dap_debug_rows() as isize;
                self.scroll_dap_debug(-rows);
                return;
            }
            Key::PageDown => {
                let rows = self.dap_debug_rows() as isize;
                self.scroll_dap_debug(rows);
                return;
            }
            Key::Home => {
                self.refresh_dap_debug_lines();
                self.dap_debug.top = 0;
                return;
            }
            Key::End => {
                self.refresh_dap_debug_lines();
                self.dap_debug.top = self.dap_debug_max_top();
                return;
            }
            _ => {}
        }

        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }

        match event.ch {
            'q' | 'Q' => self.close_dap_debug(),
            'j' | 'J' => self.scroll_dap_debug(1),
            'k' | 'K' => self.scroll_dap_debug(-1),
            'g' | 'G' => {
                self.refresh_dap_debug_lines();
                self.dap_debug.top = if event.ch == 'g' {
                    0
                } else {
                    self.dap_debug_max_top()
                };
            }
            'y' | 'Y' => self.copy_dap_debug_report(),
            'c' | 'C' => {
                self.dap.clear_log();
                self.dap_debug.status = "Log cleared.".into();
                self.refresh_dap_debug_lines();
                self.dap_debug.top = 0;
                self.force_full_redraw = true;
            }
            _ => {}
        }
    }

    pub(super) fn render_dap_debug(&mut self) {
        if !self.dap_debug.visible {
            return;
        }

        self.refresh_dap_debug_lines();

        let width = self.cols;
        let height = self.rows;
        if width < 4 || height < 3 {
            return;
        }

        self.screen.clear(self.theme.base());
        self.draw_box(0, 0, width, height, " DAP Debug ");
        let rows = self.dap_debug_rows();
        let max_top = self.dap_debug_lines.len().saturating_sub(rows);
        self.dap_debug.top = self.dap_debug.top.min(max_top);
        let content_width = width.saturating_sub(4);

        for row in 0..rows {
            let index = self.dap_debug.top + row;
            if index >= self.dap_debug_lines.len() {
                break;
            }
            self.screen.put_str(
                2,
                (1 + row) as i64,
                &self.dap_debug_lines[index],
                self.theme.text(),
                content_width,
            );
        }

        let hint = "Esc close  |  j/k/arrows scroll  |  Home/End  |  y copy  |  c clear";
        let footer = if self.dap_debug.status.is_empty() {
            hint.to_string()
        } else {
            format!("{}  |  {hint}", self.dap_debug.status)
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
// Launch configuration helpers
// ---------------------------------------------------------------------------

fn debugger_uses_executable(node: &crate::util::json::Value) -> bool {
    use crate::util::json::Value;
    match node {
        Value::Object(entries) => entries
            .iter()
            .any(|(_, value)| debugger_uses_executable(value)),
        Value::Array(values) => values.iter().any(debugger_uses_executable),
        Value::String(text) => text.contains("${executable}"),
        _ => false,
    }
}

fn try_resolve_dap_node(
    node: &mut crate::util::json::Value,
    context: &DapLaunchContext,
) -> Result<(), String> {
    use crate::util::json::Value;
    match node {
        Value::Object(entries) => {
            for (_, value) in entries.iter_mut() {
                match value {
                    Value::String(text) => {
                        *text = resolve_dap_string(text, context)?;
                    }
                    _ => try_resolve_dap_node(value, context)?,
                }
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values.iter_mut() {
                match value {
                    Value::String(text) => {
                        *text = resolve_dap_string(text, context)?;
                    }
                    _ => try_resolve_dap_node(value, context)?,
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn resolve_dap_string(text: &str, context: &DapLaunchContext) -> Result<String, String> {
    let mut resolved = text
        .replace("${workspace}", &context.workspace)
        .replace("${file}", &context.file)
        .replace("${fileDir}", &context.file_directory)
        .replace("${fileBasename}", &context.file_name)
        .replace(
            "${fileBasenameNoExtension}",
            &context.file_name_without_extension,
        );

    if !resolved.contains("${executable}") {
        return Ok(resolved);
    }

    let Some(executable) = &context.executable else {
        return Err(
            "debug: ${executable} is set but no program was provided; use :debug <program>".into(),
        );
    };
    resolved = resolved.replace("${executable}", executable);
    Ok(resolved)
}

fn normalize_debug_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    git::absolute(Path::new(path))
        .to_string_lossy()
        .into_owned()
}

fn sanitize_debug_value(value: &str) -> String {
    if value.is_empty() {
        return "…".into();
    }
    value
        .replace("\r\n", " ")
        .replace('\n', " ")
        .replace('\r', " ")
}

fn color_red(color: u32) -> u8 {
    ((color >> 16) & 0xFF) as u8
}

fn color_green(color: u32) -> u8 {
    ((color >> 8) & 0xFF) as u8
}

fn color_blue(color: u32) -> u8 {
    (color & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::json::{self, Value};

    #[test]
    fn parses_debug_command_arguments_like_the_ex_parser() {
        assert_eq!(
            Editor::parse_debug_command_arguments(" \"a b\" c\\ d 'e' ").unwrap(),
            vec![
                "a b".to_string(),
                "c\\".to_string(),
                "d".to_string(),
                "e".to_string()
            ]
        );
        assert_eq!(
            Editor::parse_debug_command_arguments(r#""a \"quoted\" value""#).unwrap(),
            vec!["a \"quoted\" value".to_string()]
        );
        assert_eq!(
            Editor::parse_debug_command_arguments("\"unterminated").unwrap_err(),
            "unterminated double quote"
        );
    }

    #[test]
    fn expands_launch_placeholders_and_requires_an_executable() {
        let mut configuration = json::parse(
            r#"{
                "request": "launch",
                "program": "${executable}",
                "cwd": "${workspace}",
                "source": "${file}",
                "directory": "${fileDir}",
                "name": "${fileBasename}",
                "stem": "${fileBasenameNoExtension}"
            }"#,
        )
        .unwrap();
        let context = DapLaunchContext {
            workspace: "/work".into(),
            file: "/work/src/main.rs".into(),
            file_directory: "/work/src".into(),
            file_name: "main.rs".into(),
            file_name_without_extension: "main".into(),
            executable: Some("/work/target/debug/app".into()),
        };
        let mut unresolved = configuration.clone();
        try_resolve_dap_node(&mut configuration, &context).unwrap();
        assert_eq!(
            configuration.get("program").and_then(Value::as_str),
            Some("/work/target/debug/app")
        );
        assert_eq!(
            configuration.get("source").and_then(Value::as_str),
            Some("/work/src/main.rs")
        );
        assert_eq!(
            configuration.get("stem").and_then(Value::as_str),
            Some("main")
        );

        let context = DapLaunchContext {
            executable: None,
            ..context
        };
        let error = try_resolve_dap_node(&mut unresolved, &context).unwrap_err();
        assert!(error.contains("use :debug <program>"));
    }
}
