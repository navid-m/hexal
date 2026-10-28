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

fn absolute_document_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn working_directory(document_path: Option<&Path>) -> Option<PathBuf> {
    let document_path = document_path?;
    let parent = absolute_document_path(document_path)
        .parent()
        .map(Path::to_path_buf)?;
    // `Path::parent` returns an empty path for a bare relative name. Passing
    // that to `Command::current_dir` makes spawning fail with ENOENT even when
    // the formatter itself is present in PATH.
    if parent.as_os_str().is_empty() || !parent.is_dir() {
        return None;
    }
    Some(parent)
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
        .map(|path| absolute_document_path(path).to_string_lossy().into_owned())
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

        let program = crate::util::git::command_path(&formatter.command)
            .unwrap_or_else(|| PathBuf::from(&formatter.command));
        let mut command = Command::new(&program);
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
                        if let Some(writer) = writer {
                            let _ = writer.join();
                        }
                        if let Some(reader) = stdout_reader {
                            let _ = reader.join();
                        }
                        if let Some(reader) = stderr_reader {
                            let _ = reader.join();
                        }
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
                    if let Some(writer) = writer {
                        let _ = writer.join();
                    }
                    if let Some(reader) = stdout_reader {
                        let _ = reader.join();
                    }
                    if let Some(reader) = stderr_reader {
                        let _ = reader.join();
                    }
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
    use std::path::Path;

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

    fn shell(arguments: Vec<&str>) -> FormatterConfig {
        FormatterConfig {
            command: "sh".into(),
            arguments: arguments.into_iter().map(str::to_string).collect(),
            input: "stdin".into(),
            output: "stdout".into(),
            timeout_ms: 5000,
        }
    }

    #[test]
    fn runs_stdin_file_formatter() {
        let mut formatter = shell(vec!["-c", "cat > \"$1\"", "sh", "{tmp}"]);
        formatter.output = "file".into();
        let output = run(&formatter, b"from stdin\n", None).unwrap();
        assert_eq!(output, b"from stdin\n");
    }

    #[test]
    fn runs_file_stdout_formatter() {
        let mut formatter = shell(vec!["-c", "cat \"$1\"", "sh", "{file}"]);
        formatter.input = "file".into();
        let output = run(&formatter, b"from file\n", None).unwrap();
        assert_eq!(output, b"from file\n");
    }

    #[test]
    fn runs_with_a_bare_relative_document_path() {
        // A bare file name has an empty parent path. The formatter must still
        // run from the current directory instead of failing to spawn.
        let formatter = shell(vec!["-c", "cat"]);
        let output = run(&formatter, b"bare\n", Some(Path::new("bare.zig"))).unwrap();
        assert_eq!(output, b"bare\n");
    }

    #[test]
    fn substitutes_an_absolute_file_path_for_relative_documents() {
        let formatter = shell(vec!["-c", "printf '%s' \"$1\"", "sh", "{file}"]);
        let output = run(&formatter, b"", Some(Path::new("relative.zig"))).unwrap();
        let expected = std::env::current_dir().unwrap().join("relative.zig");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            expected.to_string_lossy()
        );
    }

    #[test]
    fn strips_a_utf8_bom_from_formatter_output() {
        let formatter = shell(vec!["-c", "printf '\\357\\273\\277x'"]);
        let output = run(&formatter, b"ignored", None).unwrap();
        assert_eq!(output, b"x");
    }

    #[test]
    fn preserves_binary_formatter_output() {
        let formatter = shell(vec!["-c", "printf '\\000\\001\\377'"]);
        let output = run(&formatter, b"ignored", None).unwrap();
        assert_eq!(output, &[0, 1, 255]);
    }

    #[test]
    fn reports_formatter_timeout() {
        let mut formatter = shell(vec!["-c", "sleep 2"]);
        formatter.timeout_ms = 50;
        let error = run(&formatter, b"x", None).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn reports_formatter_stderr_and_exit_code() {
        let formatter = shell(vec!["-c", "echo boom >&2; exit 7"]);
        let error = run(&formatter, b"x", None).unwrap_err();
        assert!(error.contains("code 7"), "{error}");
        assert!(error.contains("boom"), "{error}");
    }

    #[test]
    fn file_formatter_must_create_the_output_file() {
        let mut formatter = shell(vec!["-c", "cat > /dev/null"]);
        formatter.output = "file".into();
        let error = run(&formatter, b"x", None).unwrap_err();
        assert!(error.contains("did not produce"), "{error}");
    }

    #[test]
    fn handles_large_formatter_output() {
        let formatter = shell(vec!["-c", "yes x | head -c 1000000"]);
        let output = run(&formatter, b"ignored", None).unwrap();
        assert_eq!(output.len(), 1_000_000);
    }

    #[test]
    fn handles_large_formatter_input() {
        let formatter = shell(vec!["-c", "cat"]);
        let input = vec![b'a'; 1_000_000];
        let output = run(&formatter, &input, None).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn formatter_that_closes_stdin_early_does_not_panic() {
        let formatter = shell(vec!["-c", "exit 0"]);
        let output = run(&formatter, &vec![b'a'; 100_000], None).unwrap();
        assert!(output.is_empty());
    }
}
