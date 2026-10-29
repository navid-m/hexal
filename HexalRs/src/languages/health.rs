//! Builds and renders the `--health` language support report.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::{self, IsTerminal, Write};

use super::Registry;
use crate::util::git::command_exists;

pub struct LanguageHealthEntry {
    pub language: String,
    pub server: Option<String>,
    pub server_available: bool,
    pub formatter: Option<String>,
    pub formatter_available: bool,
    pub debug_adapter: Option<String>,
    pub debug_adapter_available: bool,
    pub highlighting: bool,
}

pub fn build() -> (Vec<LanguageHealthEntry>, Vec<String>) {
    let registry = Registry::load();
    let errors = registry.errors().to_vec();
    let mut languages = registry.languages();
    languages.sort_by(|left, right| {
        left.id
            .to_ascii_lowercase()
            .cmp(&right.id.to_ascii_lowercase())
    });

    let plugin_servers = PluginServerIndex::load();
    let mut entries = Vec::new();
    for language in languages {
        let mut servers = plugin_servers.servers_for(&language.id);
        if let Some(fallback) = &language.fallback_lsp {
            servers.push((
                fallback.name.clone(),
                fallback.command.clone(),
                command_exists(&fallback.command),
            ));
        }
        let selected = servers
            .iter()
            .find(|(_, _, available)| *available)
            .or_else(|| servers.first())
            .cloned();

        let formatter = language.formatter.as_ref().and_then(|formatter| {
            if command_exists(&formatter.command) {
                Some((formatter.command.clone(), true))
            } else if let Some(fallback) = &language.fallback_formatter {
                if command_exists(&fallback.command) {
                    Some((fallback.command.clone(), true))
                } else {
                    Some((formatter.command.clone(), false))
                }
            } else {
                Some((formatter.command.clone(), false))
            }
        });

        let debug_adapter = language.debugger.as_ref().map(|debugger| {
            (
                debugger.adapter.clone(),
                crate::dap::Manager::find_adapter(&debugger.adapter).is_some(),
            )
        });

        entries.push(LanguageHealthEntry {
            language: language.id.clone(),
            server: selected.as_ref().map(|(name, _, _)| name.clone()),
            server_available: selected
                .as_ref()
                .is_some_and(|(_, _, available)| *available),
            formatter: formatter.as_ref().map(|(command, _)| command.clone()),
            formatter_available: formatter.as_ref().is_some_and(|(_, available)| *available),
            debug_adapter: debug_adapter.as_ref().map(|(adapter, _)| adapter.clone()),
            debug_adapter_available: debug_adapter
                .as_ref()
                .is_some_and(|(_, available)| *available),
            highlighting: true,
        });
    }
    (entries, errors)
}

pub fn print_report(language: Option<&str>) -> io::Result<()> {
    let (mut entries, errors) = build();
    if let Some(language) = language {
        let wanted = language.to_ascii_lowercase();
        entries.retain(|entry| entry.language.eq_ignore_ascii_case(&wanted));
        if entries.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown language '{language}'"),
            ));
        }
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    let color = output.is_terminal();
    write_table(&mut output, &entries, color)?;
    for error in errors {
        writeln!(output, "error: {error}")?;
    }
    Ok(())
}

const LANGUAGE_WIDTH: usize = 14;
const SERVER_WIDTH: usize = 32;
const FORMATTER_WIDTH: usize = 18;
const DEBUG_ADAPTER_WIDTH: usize = 18;

const RESET: &str = "\x1b[0m";
const WHITE: &str = "\x1b[37m";
const HEADER: &str = "\x1b[1;37m";
const CYAN: &str = "\x1b[36m";
const LIME: &str = "\x1b[38;5;118m";
const RED: &str = "\x1b[31m";

fn write_table(
    writer: &mut dyn Write,
    entries: &[LanguageHealthEntry],
    use_color: bool,
) -> io::Result<()> {
    write_cell(writer, "Language", LANGUAGE_WIDTH, HEADER, use_color)?;
    writer.write_all(b" ")?;
    write_cell(writer, "Language server", SERVER_WIDTH, HEADER, use_color)?;
    writer.write_all(b" ")?;
    write_cell(writer, "Formatter", FORMATTER_WIDTH, HEADER, use_color)?;
    writer.write_all(b" ")?;
    write_cell(
        writer,
        "Debug adapter",
        DEBUG_ADAPTER_WIDTH,
        HEADER,
        use_color,
    )?;
    writer.write_all(b" ")?;
    write_text(writer, "Highlight", HEADER, use_color)?;
    writer.write_all(b"\n")?;

    for entry in entries {
        write_cell(writer, &entry.language, LANGUAGE_WIDTH, WHITE, use_color)?;
        writer.write_all(b" ")?;
        write_status_cell(
            writer,
            entry.server.as_deref(),
            entry.server_available,
            SERVER_WIDTH,
            use_color,
        )?;
        writer.write_all(b" ")?;
        write_status_cell(
            writer,
            entry.formatter.as_deref(),
            entry.formatter_available,
            FORMATTER_WIDTH,
            use_color,
        )?;
        writer.write_all(b" ")?;
        write_status_cell(
            writer,
            entry.debug_adapter.as_deref(),
            entry.debug_adapter_available,
            DEBUG_ADAPTER_WIDTH,
            use_color,
        )?;
        writer.write_all(b" ")?;
        write_text(
            writer,
            if entry.highlighting { "✓" } else { "✘" },
            if entry.highlighting { LIME } else { RED },
            use_color,
        )?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn write_status_cell(
    writer: &mut dyn Write,
    value: Option<&str>,
    available: bool,
    width: usize,
    use_color: bool,
) -> io::Result<()> {
    match value {
        None => write_cell(writer, "None", width, CYAN, use_color),
        Some(value) => {
            let text = format!("{} {value}", if available { "✓" } else { "✘" });
            write_cell(
                writer,
                &text,
                width,
                if available { LIME } else { RED },
                use_color,
            )
        }
    }
}

fn write_cell(
    writer: &mut dyn Write,
    value: &str,
    width: usize,
    color: &str,
    use_color: bool,
) -> io::Result<()> {
    let mut value = value.to_string();
    let length = value.chars().count();
    if length > width {
        value = if width <= 1 {
            "…".to_string()
        } else {
            let mut truncated: String = value.chars().take(width - 1).collect();
            truncated.push('…');
            truncated
        };
    }
    let text = value.trim_end().to_string();
    write_text(writer, &text, color, use_color)?;
    let padding = width.saturating_sub(text.chars().count());
    for _ in 0..padding {
        writer.write_all(b" ")?;
    }
    Ok(())
}

fn write_text(writer: &mut dyn Write, text: &str, color: &str, use_color: bool) -> io::Result<()> {
    if use_color {
        writer.write_all(color.as_bytes())?;
    }
    writer.write_all(text.as_bytes())?;
    if use_color {
        writer.write_all(RESET.as_bytes())?;
    }
    Ok(())
}

struct PluginServerIndex {
    servers: Vec<(String, String, String, bool)>,
}

impl PluginServerIndex {
    fn load() -> PluginServerIndex {
        let mut servers = Vec::new();
        let directory = crate::util::plugin_install::plugin_directory();
        let Ok(read) = fs::read_dir(&directory) else {
            return PluginServerIndex { servers };
        };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(manifest) = fs::read_to_string(path.join("plugin.toml")) else {
                continue;
            };
            let Some(language) = manifest_value(&manifest, "language") else {
                continue;
            };
            let entry_name =
                manifest_value(&manifest, "entry").unwrap_or_else(|| "init.lua".into());
            let Ok(script) = fs::read_to_string(path.join(entry_name)) else {
                continue;
            };
            let mut current_name: Option<String> = None;
            for line in script.lines() {
                if let Some(name) = quoted_value(line, "name") {
                    current_name = Some(name);
                }
                if let Some(command) = quoted_value(line, "command") {
                    let available = command_exists(&command);
                    let name = current_name.take().unwrap_or_else(|| command.clone());
                    servers.push((language.to_ascii_lowercase(), name, command, available));
                }
            }
        }
        PluginServerIndex { servers }
    }

    fn servers_for(&self, language: &str) -> Vec<(String, String, bool)> {
        let wanted = language.to_ascii_lowercase();
        let mut result: Vec<(String, String, bool)> = Vec::new();
        for (language, name, command, available) in &self.servers {
            if language != &wanted {
                continue;
            }
            if !result.iter().any(|(existing, existing_command, _)| {
                existing == name && existing_command == command
            }) {
                result.push((name.clone(), command.clone(), *available));
            }
        }
        result
    }
}

fn manifest_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                return unquote(rest.trim());
            }
        }
    }
    None
}

fn quoted_value(line: &str, key: &str) -> Option<String> {
    let index = line.find(key)?;
    let rest = line[index + key.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    unquote(rest)
}

fn unquote(value: &str) -> Option<String> {
    let value = value.trim_end_matches(|character| character == ',' || character == '}');
    let value = value.trim();
    let (quote, rest) = match value.chars().next()? {
        '"' => ('"', &value[1..]),
        '\'' => ('\'', &value[1..]),
        _ => return None,
    };
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}
