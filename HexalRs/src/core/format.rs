//! External formatter discovery and execution.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::languages::FormatterConfig;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn substitute(argument: &str, file: &str, temp: &str) -> String {
    argument
        .replace("{file}", file)
        .replace("{path}", file)
        .replace("{tmp}", temp)
        .replace("{temp}", temp)
        .replace("{output}", temp)
}

fn create_temp_path(document_path: Option<&Path>) -> PathBuf {
    let extension = document_path
        .and_then(|path| path.extension())
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .filter(|extension| extension.len() > 1)
        .unwrap_or_else(|| ".tmp".into());
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "hexal-format-{}-{stamp}-{sequence}{extension}",
        std::process::id()
    ))
}

fn working_directory(document_path: Option<&Path>) -> Option<PathBuf> {
    document_path
        .and_then(Path::parent)
        .map(|parent| parent.to_path_buf())
}

pub fn run(
    formatter: &FormatterConfig,
    input: &[u8],
    document_path: Option<&Path>,
) -> Result<Vec<u8>, String> {
    if formatter.command.trim().is_empty() {
        return Err("no formatter command configured".into());
    }

    let uses_stdin = formatter.uses_stdin();
    let writes_file = formatter.writes_file();
    let temp_path = if !uses_stdin || writes_file {
        Some(create_temp_path(document_path))
    } else {
        None
    };
    let file = document_path
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| {
            temp_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let temp = temp_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.clone());

    let result = (|| {
        if !uses_stdin {
            if let Some(path) = &temp_path {
                fs::write(path, input).map_err(|error| format!("formatter failed: {error}"))?;
            }
        }

        let mut command = Command::new(&formatter.command);
        for argument in &formatter.arguments {
            command.arg(substitute(argument, &file, &temp));
        }
        command
            .stdin(if uses_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(if writes_file {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stderr(Stdio::piped());
        if let Some(directory) = working_directory(document_path) {
            command.current_dir(directory);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start formatter '{}': {error}", formatter.command))?;

        let writer = if uses_stdin {
            child.stdin.take().map(|mut stdin| {
                let input = input.to_vec();
                std::thread::spawn(move || {
                    let _ = stdin.write_all(&input);
                    let _ = stdin.flush();
                })
            })
        } else {
            None
        };

        let stdout_reader = child.stdout.take().map(|mut stdout| {
            std::thread::spawn(move || {
                let mut output = Vec::new();
                let _ = std::io::Read::read_to_end(&mut stdout, &mut output);
                output
            })
        });
        let stderr_reader = child.stderr.take().map(|mut stderr| {
            std::thread::spawn(move || {
                let mut output = Vec::new();
                let _ = std::io::Read::read_to_end(&mut stderr, &mut output);
                output
            })
        });

        let timeout = Duration::from_millis(if formatter.timeout_ms > 0 {
            formatter.timeout_ms
        } else {
            15000
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "formatter timed out after {} ms",
                            timeout.as_millis()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("formatter failed: {error}"));
                }
            }
        };

        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let stdout = stdout_reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        let stderr = stderr_reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();

        if !status.success() {
            let detail = String::from_utf8_lossy(&stderr).trim().to_string();
            return Err(if detail.is_empty() {
                format!("formatter exited with code {}", status.code().unwrap_or(-1))
            } else {
                format!(
                    "formatter exited with code {}: {detail}",
                    status.code().unwrap_or(-1)
                )
            });
        }

        let mut output = if writes_file {
            let Some(path) = &temp_path else {
                return Err("formatter did not produce an output file".into());
            };
            if !path.exists() {
                return Err("formatter did not produce an output file".into());
            }
            fs::read(path).map_err(|error| format!("formatter failed: {error}"))?
        } else {
            stdout
        };
        if output.starts_with(&[0xEF, 0xBB, 0xBF]) {
            output.drain(..3);
        }
        Ok(output)
    })();

    if let Some(path) = temp_path {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::languages::FormatterConfig;

    #[test]
    fn runs_stdin_stdout_formatter() {
        let formatter = FormatterConfig {
            command: "cat".into(),
            arguments: Vec::new(),
            input: "stdin".into(),
            output: "stdout".into(),
            timeout_ms: 5000,
        };
        let output = run(&formatter, b"fn main() {}\n", None).unwrap();
        assert_eq!(output, b"fn main() {}\n");
    }

    #[test]
    fn reports_missing_formatter() {
        let formatter = FormatterConfig {
            command: "hexal-definitely-missing-formatter".into(),
            arguments: Vec::new(),
            input: "stdin".into(),
            output: "stdout".into(),
            timeout_ms: 5000,
        };
        assert!(run(&formatter, b"x", None).is_err());
    }
}
