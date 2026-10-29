//! Installs a plugin folder from the configured central Git repository.
//!
//! This mirrors `Hexal.Scripting.PluginInstaller` from the C# editor: the
//! repository is shallow-cloned into a temporary directory, the named plugin
//! folder is copied into place and the temporary checkout is removed.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CLONE_TIMEOUT: Duration = Duration::from_secs(120);
static UNIQUE: AtomicU64 = AtomicU64::new(1);

/// Installs `name` from `repository_url` into `plugin_directory`.
pub fn install(
    name: &str,
    repository_url: &str,
    plugin_directory: &Path,
) -> Result<PathBuf, String> {
    if !is_valid_plugin_name(name) {
        return Err("invalid plugin name".into());
    }
    if repository_url.trim().is_empty() {
        return Err("central repository URL is empty".into());
    }

    fs::create_dir_all(plugin_directory).map_err(|error| error.to_string())?;
    let destination = std::path::absolute(plugin_directory)
        .map_err(|error| error.to_string())?
        .join(name);

    if destination.exists() {
        return Err(format!(
            "plugin '{name}' is already installed at '{}'",
            destination.display()
        ));
    }

    let temporary_root = std::env::temp_dir().join(format!("hexal-plugin-{}", unique_id()));
    let result = install_inner(name, repository_url, &destination, &temporary_root);
    try_delete_directory(&temporary_root);
    result.map(|()| destination)
}

fn install_inner(
    name: &str,
    repository_url: &str,
    destination: &Path,
    temporary_root: &Path,
) -> Result<(), String> {
    fs::create_dir_all(temporary_root).map_err(|error| error.to_string())?;
    let clone_directory = temporary_root.join("repository");
    clone_repository(repository_url, &clone_directory)?;

    let source = clone_directory.join(name);
    if !source.is_dir() {
        return Err(format!(
            "plugin '{name}' was not found in '{repository_url}'"
        ));
    }

    let staging = destination.with_file_name(format!(
        "{}.installing-{}",
        destination
            .file_name()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default(),
        unique_id()
    ));
    if let Err(error) = copy_directory(&source, &staging)
        .and_then(|()| fs::rename(&staging, destination).map_err(|error| error.to_string()))
    {
        try_delete_directory(&staging);
        return Err(error);
    }
    Ok(())
}

fn clone_repository(repository_url: &str, directory: &Path) -> Result<(), String> {
    let mut child = Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", "--"])
        .arg(repository_url)
        .arg(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run git: {error}"))?;

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stream) = stdout.as_mut() {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stream) = stderr.as_mut() {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    });

    let deadline = Instant::now() + CLONE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("cannot run git: {error}")),
        }
    };

    let output = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default())
        .trim()
        .to_string();
    let detail = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default())
        .trim()
        .to_string();

    let Some(status) = status else {
        return Err(format!(
            "git clone timed out after {} ms",
            CLONE_TIMEOUT.as_millis()
        ));
    };
    if status.success() {
        return Ok(());
    }

    Err(if !detail.is_empty() {
        detail
    } else if !output.is_empty() {
        output
    } else {
        format!("git clone exited with code {}", status.code().unwrap_or(-1))
    })
}

fn is_valid_plugin_name(name: &str) -> bool {
    if name.trim().is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains(std::path::MAIN_SEPARATOR) || name.contains('/') || name.contains('\\') {
        return false;
    }
    // Mirrors Path.GetInvalidFileNameChars() on Unix plus NUL on every
    // platform. Rejecting the Windows-only set too keeps installs portable.
    !name
        .chars()
        .any(|character| character == '\0' || character.is_control())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;

    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            if metadata.file_type().is_symlink() {
                continue;
            }
            copy_directory(&path, &target)?;
        } else {
            fs::copy(&path, &target).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn try_delete_directory(path: &Path) {
    if path.exists() {
        let _ = fs::remove_dir_all(path);
    }
}

fn unique_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{:x}{:x}{:x}",
        std::process::id(),
        nanos,
        UNIQUE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Resolves the same plugin directory as the C# editor: the portable
/// `<binary>/data/plugins` directory when present, otherwise
/// `~/.hexal/data/plugins`.
pub fn plugin_directory() -> PathBuf {
    let binary = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let portable = binary.join("data").join("plugins");
    if portable.is_dir() {
        return portable;
    }
    #[cfg(not(test))]
    {
        let local = Path::new("data").join("plugins");
        if local.is_dir() {
            return local;
        }
    }
    crate::core::config::hexal_home_directory()
        .join("data")
        .join("plugins")
}

#[cfg(test)]
mod tests {
    use super::{install, is_valid_plugin_name};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(args: &[&str], directory: &PathBuf) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn validates_plugin_names() {
        assert!(is_valid_plugin_name("demo"));
        assert!(is_valid_plugin_name("demo-2"));
        assert!(!is_valid_plugin_name(""));
        assert!(!is_valid_plugin_name("  "));
        assert!(!is_valid_plugin_name("."));
        assert!(!is_valid_plugin_name(".."));
        assert!(!is_valid_plugin_name("../demo"));
        assert!(!is_valid_plugin_name("a/b"));
        assert!(!is_valid_plugin_name("a\\b"));
    }

    #[test]
    fn installs_from_a_local_repository() {
        if !Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            return;
        }
        let root = std::env::temp_dir().join(format!("hexal-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repository = root.join("repository");
        let plugins = root.join("plugins");
        fs::create_dir_all(repository.join("demo")).unwrap();
        fs::write(
            repository.join("demo/plugin.toml"),
            "name = \"Demo\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        fs::write(
            repository.join("demo/init.lua"),
            "hexal.commands.register(\"demo\", function() end)\n",
        )
        .unwrap();
        assert!(git(&["init", "-q"], &repository));
        assert!(git(
            &["config", "user.email", "tests@example.com"],
            &repository
        ));
        assert!(git(&["config", "user.name", "Hexal Tests"], &repository));
        assert!(git(&["add", "."], &repository));
        assert!(git(&["commit", "-qm", "plugin"], &repository));

        let installed = install("demo", &repository.to_string_lossy(), &plugins).unwrap();
        assert!(installed.join("plugin.toml").is_file());
        assert!(installed.join("init.lua").is_file());
        assert!(!installed.join(".git").exists());
        assert!(install("demo", &repository.to_string_lossy(), &plugins).is_err());
        assert!(install("../demo", &repository.to_string_lossy(), &plugins).is_err());
        assert!(install("missing", &repository.to_string_lossy(), &plugins).is_err());

        let _ = fs::remove_dir_all(root);
    }
}
