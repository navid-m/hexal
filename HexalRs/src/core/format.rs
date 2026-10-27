//! External formatter discovery and execution.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Formatter {
    pub command: &'static str,
    pub arguments: Vec<String>,
    pub uses_stdin: bool,
}

/// Return the preferred formatter for a language id, trying fallbacks in the
/// same order the C# language definitions do.
pub fn for_language(id: &str) -> Option<Formatter> {
    let candidates: &[(&str, &[&str])] = match id {
        "rust" => &[("rustfmt", &["--edition", "2021", "--emit", "stdout"])],
        "zig" => &[("zig", &["fmt", "--stdin"])],
        "go" => &[("gofmt", &[]), ("goimports", &[])],
        "python" => &[
            ("ruff", &["format", "-"]),
            ("black", &["--quiet", "-"]),
        ],
        "javascript" | "typescript" | "tsx" | "jsx" | "json" | "yaml" | "markdown" => &[
            ("prettier", &["--stdin-filepath", "{file}"]),
        ],
        "c" | "cpp" | "csharp" | "java" => &[("clang-format", &[]), ("astyle", &[])],
        "lua" => &[("stylua", &["-"])],
        "shell" => &[("shfmt", &[])],
        "toml" => &[("taplo", &["fmt", "-"])],
        _ => &[],
    };
    for (command, arguments) in candidates {
        if crate::util::git::command_exists(command) {
            return Some(Formatter {
                command,
                arguments: arguments.iter().map(|argument| argument.to_string()).collect(),
                uses_stdin: true,
            });
        }
    }
    None
}

pub fn run(formatter: &Formatter, input: &[u8], path: Option<&Path>) -> Result<Vec<u8>, String> {
    let file = path
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "buffer".into());
    let mut command = Command::new(formatter.command);
    for argument in &formatter.arguments {
        command.arg(argument.replace("{file}", &file).replace("{path}", &file));
    }
    command
        .stdin(if formatter.uses_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = path.and_then(Path::parent) {
        command.current_dir(parent);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start formatter '{}': {error}", formatter.command))?;
    if formatter.uses_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            let input = input.to_vec();
            let writer = std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
            // Join after wait so a formatter that ignores stdin cannot block
            // startup forever on a full pipe.
            let output = child
                .wait_with_output()
                .map_err(|error| format!("formatter failed: {error}"))?;
            let _ = writer.join();
            return finish(output);
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("formatter failed: {error}"))?;
    finish(output)
}

fn finish(output: std::process::Output) -> Result<Vec<u8>, String> {
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("formatter exited with code {}", output.status.code().unwrap_or(-1))
        } else {
            format!(
                "formatter exited with code {}: {detail}",
                output.status.code().unwrap_or(-1)
            )
        });
    }
    let mut result = output.stdout;
    if result.starts_with(&[0xEF, 0xBB, 0xBF]) {
        result.drain(..3);
    }
    Ok(result)
}
