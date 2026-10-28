//! System clipboard integration.
//!
//! Mirrors the C# editor: native Windows clipboard access first, then
//! platform helper programs, with an OSC 52 fallback for copying over SSH.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
// Clipboard writers such as wl-copy intentionally stay alive to own the
// selection. Give them just enough time to report an immediate startup
// failure, then let them continue in the background instead of blocking the
// editor's input loop.
const WRITE_GRACE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CopyOutcome {
    System,
    Osc52,
}

pub fn copy(text: &[u8]) -> Result<CopyOutcome, String> {
    if text.is_empty() {
        return Err("nothing selected".into());
    }

    #[cfg(windows)]
    if windows::copy(text).is_ok() {
        return Ok(CopyOutcome::System);
    }

    for (program, arguments) in copy_commands() {
        if run_writer(program, &arguments, text).is_ok() {
            return Ok(CopyOutcome::System);
        }
    }

    Ok(CopyOutcome::Osc52)
}

pub fn paste() -> Result<Vec<u8>, String> {
    let mut last_error = String::new();

    #[cfg(windows)]
    match windows::paste() {
        Ok(text) => return Ok(text),
        Err(error) => last_error = error,
    }

    for (program, arguments) in paste_commands() {
        match run_reader(program, &arguments) {
            Ok(text) => return Ok(text),
            Err(error) => {
                if !error.is_empty() {
                    last_error = error;
                }
            }
        }
    }

    Err(if last_error.is_empty() {
        "cannot read the system clipboard: no clipboard helper found".into()
    } else {
        format!("cannot read the system clipboard: {last_error}")
    })
}

fn copy_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    #[cfg(windows)]
    {
        const POWERSHELL_COPY: &str = concat!(
            "[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false); ",
            "$text = [Console]::In.ReadToEnd(); Set-Clipboard -Value $text"
        );
        return vec![
            (
                "pwsh.exe",
                vec!["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_COPY],
            ),
            (
                "powershell.exe",
                vec!["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_COPY],
            ),
        ];
    }

    #[cfg(target_os = "macos")]
    {
        return vec![("pbcopy", Vec::new())];
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut commands = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
            commands.push(("wl-copy", Vec::new()));
        }
        commands.push(("xclip", vec!["-selection", "clipboard", "-in"]));
        commands.push(("xsel", vec!["--clipboard", "--input"]));
        return commands;
    }

    #[allow(unreachable_code)]
    Vec::new()
}

fn paste_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    #[cfg(windows)]
    {
        const POWERSHELL_PASTE: &str = concat!(
            "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); ",
            "$text = Get-Clipboard -Raw; if ($null -ne $text) { [Console]::Out.Write($text) }"
        );
        return vec![
            (
                "pwsh.exe",
                vec![
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    POWERSHELL_PASTE,
                ],
            ),
            (
                "powershell.exe",
                vec![
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    POWERSHELL_PASTE,
                ],
            ),
        ];
    }

    #[cfg(target_os = "macos")]
    {
        return vec![("pbpaste", Vec::new())];
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut commands = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
            commands.push(("wl-paste", vec!["--no-newline"]));
        }
        commands.push(("xclip", vec!["-selection", "clipboard", "-out"]));
        commands.push(("xsel", vec!["--clipboard", "--output"]));
        return commands;
    }

    #[allow(unreachable_code)]
    Vec::new()
}

fn run_writer(program: &str, arguments: &[&str], text: &[u8]) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    let input = text.to_vec();
    let writer = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
            let _ = stdin.flush();
        })
    });
    let stderr_reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stderr.read_to_end(&mut output);
            output
        })
    });

    let deadline = Instant::now() + WRITE_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(writer) = writer {
                    if writer.is_finished() {
                        let _ = writer.join();
                    }
                }
                if status.success() {
                    // Do not wait for stderr here: X11 helpers may fork a
                    // background process that inherits the pipe and keeps it
                    // open for as long as the selection is owned.
                    return Ok(());
                }
                let stderr = stderr_reader
                    .filter(|reader| reader.is_finished())
                    .and_then(|reader| reader.join().ok())
                    .unwrap_or_default();
                let detail = String::from_utf8_lossy(&stderr).trim().to_string();
                return Err(if detail.is_empty() {
                    format!("{program} exited with code {}", status.code().unwrap_or(-1))
                } else {
                    detail
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                // X11 clipboard helpers may intentionally stay alive to own
                // the selection, so a timeout is not a copy failure. Do not
                // wait for a writer that may still be blocked on its pipe.
                if let Some(writer) = writer {
                    if writer.is_finished() {
                        let _ = writer.join();
                    }
                }
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
}

fn run_reader(program: &str, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    let stdout_reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stdout.read_to_end(&mut output);
            output
        })
    });
    let stderr_reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stderr.read_to_end(&mut output);
            output
        })
    });

    let deadline = Instant::now() + READ_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };

    let output = stdout_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    if status.success() {
        Ok(output)
    } else {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        Err(if detail.is_empty() {
            format!("{program} exited with code {}", status.code().unwrap_or(-1))
        } else {
            detail
        })
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;
    use std::thread;
    use std::time::Duration;

    type Handle = *mut c_void;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;
    const GMEM_ZEROINIT: u32 = 0x0040;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(memory: Handle) -> *mut c_void;
        fn GlobalUnlock(memory: Handle) -> i32;
        fn GlobalFree(memory: Handle) -> Handle;
    }

    pub fn copy(text: &[u8]) -> Result<(), String> {
        let text = String::from_utf8_lossy(text);
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        let bytes = wide.len() * std::mem::size_of::<u16>();
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) };
        if memory.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            unsafe {
                GlobalFree(memory);
            }
            return Err(io::Error::last_os_error().to_string());
        }
        unsafe {
            ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, pointer as *mut u8, bytes);
            GlobalUnlock(memory);
        }

        if let Err(error) = open() {
            unsafe {
                GlobalFree(memory);
            }
            return Err(error);
        }

        let mut owned = memory;
        let result = (|| {
            if unsafe { EmptyClipboard() } == 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            if unsafe { SetClipboardData(CF_UNICODETEXT, memory) }.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            owned = ptr::null_mut();
            Ok(())
        })();
        unsafe {
            CloseClipboard();
            if !owned.is_null() {
                GlobalFree(owned);
            }
        }
        result
    }

    pub fn paste() -> Result<Vec<u8>, String> {
        open()?;
        let result = (|| {
            if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
                return Err("the clipboard does not contain text".to_string());
            }
            let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
            if handle.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            let pointer = unsafe { GlobalLock(handle) };
            if pointer.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            let wide = unsafe { pointer as *const u16 };
            let mut length = 0usize;
            while unsafe { *wide.add(length) } != 0 {
                length += 1;
            }
            let text = unsafe { std::slice::from_raw_parts(wide, length) };
            let text = String::from_utf16_lossy(text).into_bytes();
            unsafe {
                GlobalUnlock(handle);
            }
            Ok(text)
        })();
        unsafe {
            CloseClipboard();
        }
        result
    }

    fn open() -> Result<(), String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::last_os_error().to_string())
    }
}
