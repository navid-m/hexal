//! Linux desktop launcher installation.
//!
//! Mirrors the C# `LinuxDesktopEntry` implementation: a `.desktop` file is
//! written to `$XDG_DATA_HOME/applications` (normally
//! `~/.local/share/applications`) and falls back to the executable directory
//! when that location cannot be used. `Terminal=true` makes desktop
//! environments launch the TUI inside a terminal.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DESKTOP_FILE_NAME: &str = "hexal.desktop";
const DESKTOP_FILE_MODE: u32 = 0o755;
const UPDATE_DATABASE_TIMEOUT: Duration = Duration::from_millis(2000);

static CHECKED: AtomicBool = AtomicBool::new(false);

/// Installs or refreshes the desktop launcher. Safe to call from a background
/// thread; it only performs work once per process.
pub fn ensure_launcher() {
    if !cfg!(target_os = "linux") || CHECKED.swap(true, Ordering::SeqCst) {
        return;
    }

    let Some(executable) = std::env::current_exe().ok() else {
        log_failure("could not resolve the executable path");
        return;
    };
    let executable_name = executable
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if executable_name == "dotnet" || executable_name == "dotnet.exe" {
        return;
    }
    let Some(executable_directory) = executable.parent() else {
        log_failure(&format!(
            "could not resolve the directory for '{}'",
            executable.display()
        ));
        return;
    };
    let executable = executable.to_string_lossy().into_owned();
    if executable.contains('\r') || executable.contains('\n') {
        log_failure("the executable path contains a line break");
        return;
    }

    let contents = build_contents(&executable);
    if let Some(directory) = applications_directory() {
        let path = directory.join(DESKTOP_FILE_NAME);
        match try_install_launcher(&path, &contents, true) {
            Ok(true) => {
                crate::util::log::write(&format!("Installed desktop launcher: {}", path.display()));
                try_update_desktop_database(&directory);
                return;
            }
            Ok(false) => return,
            Err(error) => {
                crate::util::log::write(&format!(
                    "Could not install desktop launcher in '{}': {error}; falling back to the executable directory",
                    directory.display()
                ));
            }
        }
    }

    let path = executable_directory.join(DESKTOP_FILE_NAME);
    match try_install_launcher(&path, &contents, false) {
        Ok(true) => {
            crate::util::log::write(&format!("Created desktop launcher: {}", path.display()));
        }
        Ok(false) => {}
        Err(error) => log_failure(&error),
    }
}

fn applications_directory() -> Option<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && !path.as_os_str().is_empty());
    let data_home = match data_home {
        Some(path) => path,
        None => home_directory()?.join(".local").join("share"),
    };
    Some(data_home.join("applications"))
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn try_install_launcher(
    path: &Path,
    contents: &str,
    create_directory: bool,
) -> Result<bool, String> {
    if create_directory {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)
                .map_err(|error| format!("cannot create '{}': {error}", directory.display()))?;
        }
    }

    if let Ok(existing) = fs::read_to_string(path) {
        if existing == contents {
            return Ok(false);
        }
    }

    fs::write(path, contents.as_bytes())
        .map_err(|error| format!("cannot write '{}': {error}", path.display()))?;
    set_desktop_file_mode(path);
    Ok(true)
}

fn set_desktop_file_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(DESKTOP_FILE_MODE));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn try_update_desktop_database(directory: &Path) {
    let Ok(mut child) = Command::new("update-desktop-database")
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() < UPDATE_DATABASE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

fn build_contents(executable: &str) -> String {
    let quoted = quote_exec_argument(executable);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Hexal\n\
         GenericName=Text Editor\n\
         Comment=Modal Text Editing Redefined\n\
         Exec={quoted} %f\n\
         Terminal=true\n\
         Categories=Utility;TextEditor;\n\
         MimeType=text/plain;\n\
         Keywords=text;editor;terminal;modal;\n\
         StartupNotify=false\n\
         Icon=utilities-terminal\n"
    )
}

fn quote_exec_argument(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 8);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '`' => quoted.push_str("\\`"),
            '$' => quoted.push_str("\\$"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn log_failure(message: &str) {
    crate::util::log::write(&format!("Could not create desktop launcher: {message}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "hexal-desktop-test-{}-{sequence}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn builds_the_expected_launcher_contents() {
        let contents = build_contents("/usr/bin/hexal");
        assert!(contents.starts_with("[Desktop Entry]\n"));
        assert!(contents.contains("Name=Hexal\n"));
        assert!(contents.contains("Exec=\"/usr/bin/hexal\" %f\n"));
        assert!(contents.contains("Terminal=true\n"));
        assert!(contents.ends_with("Icon=utilities-terminal\n"));
    }

    #[test]
    fn quotes_exec_arguments_like_the_reference_editor() {
        assert_eq!(quote_exec_argument("/usr/bin/hexal"), "\"/usr/bin/hexal\"");
        assert_eq!(
            quote_exec_argument("/tmp/a b/$x/`y`/\"z\"/100%"),
            "\"/tmp/a b/\\$x/\\`y\\`/\\\"z\\\"/100%%\""
        );
        assert_eq!(
            quote_exec_argument("C:\\Tools\\hexal"),
            "\"C:\\\\Tools\\\\hexal\""
        );
    }

    #[test]
    fn installs_the_launcher_with_the_expected_mode() {
        let path = temp_path("hexal.desktop");
        let contents = build_contents("/usr/bin/hexal");
        assert!(try_install_launcher(&path, &contents, true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        assert!(!fs::read(&path).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                DESKTOP_FILE_MODE
            );
        }
        // Installing identical contents again is a no-op.
        assert!(!try_install_launcher(&path, &contents, true).unwrap());
        let _ = fs::remove_file(path);
    }
}
