//! Persistent `.hexcfg` configuration.
//!
//! The schema, defaults, path resolution and update behavior mirror the C#
//! editor's `HexalConfig`. Unknown properties are preserved when display
//! toggles are written.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::json::{self, Value};

pub const DEFAULT_FILE_NAME: &str = ".hexcfg";
pub const DEFAULT_CENTRAL_REPOSITORY_URL: &str = "https://github.com/navid-m/hexal-plugins";

pub const DEFAULT_CONTENTS: &str = r#"{
    "version": 1,
    "centralRepositoryUrl": "https://github.com/navid-m/hexal-plugins",
    "formatOnSave": false,
    "fsyncOnSave": false,
    "trimTrailingWhitespaceOnSave": false,
    "smartBackspace": true,
    "autoDetectIndentationOnStartup": true,
    "startupBenchmarker": false,
    "syntaxHighlightPickerPreview": false,
    "debugAdapter": true,
    "showDiffLines": true,
    "diagnosticLineHighlight": true,
    "showGitIgnored": false,
    "showModifiedIndicators": true,
    "wordWrap": false,
    "keepSidebarFocusOnOpen": true,
    "confirmQuitWithoutSaving": true,
    "searchMatchCaseEnabled": false,
    "searchSkipFilesOverSizeMb": 4
}
"#;

#[derive(Clone, Debug)]
pub struct Config {
    pub source_path: PathBuf,
    #[allow(dead_code)]
    pub central_repository_url: String,
    pub format_on_save: bool,
    pub fsync_on_save: bool,
    pub trim_trailing_whitespace_on_save: bool,
    pub smart_backspace: bool,
    pub auto_detect_indentation_on_startup: bool,
    pub startup_benchmarker: bool,
    #[allow(dead_code)]
    pub syntax_highlight_picker_preview: bool,
    #[allow(dead_code)]
    pub debug_adapter: bool,
    pub show_diff_lines: bool,
    #[allow(dead_code)]
    pub diagnostic_line_highlight: bool,
    pub show_git_ignored: bool,
    #[allow(dead_code)]
    pub show_modified_indicators: bool,
    #[allow(dead_code)]
    pub word_wrap: bool,
    pub keep_sidebar_focus_on_open: bool,
    pub confirm_quit_without_saving: bool,
    pub search_match_case_enabled: bool,
    pub search_skip_files_over_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source_path: default_path(),
            central_repository_url: DEFAULT_CENTRAL_REPOSITORY_URL.into(),
            format_on_save: false,
            fsync_on_save: false,
            trim_trailing_whitespace_on_save: false,
            smart_backspace: true,
            auto_detect_indentation_on_startup: true,
            startup_benchmarker: false,
            syntax_highlight_picker_preview: false,
            debug_adapter: true,
            show_diff_lines: true,
            diagnostic_line_highlight: true,
            show_git_ignored: false,
            show_modified_indicators: true,
            word_wrap: false,
            keep_sidebar_focus_on_open: true,
            confirm_quit_without_saving: true,
            search_match_case_enabled: false,
            search_skip_files_over_size_mb: 4,
        }
    }
}

impl Config {
    pub fn load_default() -> (Config, String) {
        let path = resolve_path();
        if !path.exists() {
            return (
                Config {
                    source_path: path,
                    ..Config::default()
                },
                String::new(),
            );
        }
        Config::load(&path)
    }

    pub fn load(path: &Path) -> (Config, String) {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                return (
                    Config {
                        source_path: path.to_path_buf(),
                        ..Config::default()
                    },
                    error.to_string(),
                );
            }
        };
        match parse(&text, path) {
            Ok(config) => (config, String::new()),
            Err(error) => (
                Config {
                    source_path: path.to_path_buf(),
                    ..Config::default()
                },
                format!("invalid JSON: {error}"),
            ),
        }
    }

    pub fn ensure_default_file() -> Result<PathBuf, String> {
        let path = resolve_path();
        let directory = hexal_home_directory();
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        if !path.exists() {
            fs::write(&path, DEFAULT_CONTENTS).map_err(|error| error.to_string())?;
        }
        Ok(path)
    }

    pub fn update_display_settings(
        path: &Path,
        show_diff_lines: bool,
        show_git_ignored: bool,
        search_match_case_enabled: bool,
    ) -> Result<(), String> {
        update_settings(
            path,
            Some(show_diff_lines),
            Some(show_git_ignored),
            Some(search_match_case_enabled),
        )
    }

    pub fn update_word_wrap(path: &Path, word_wrap: bool) -> Result<(), String> {
        update_settings(path, None, None, None).and_then(|_| {
            let mut root = read_root(path)?;
            root.set("wordWrap", Value::Bool(word_wrap));
            write_root(path, &root)
        })
    }
}

fn update_settings(
    path: &Path,
    show_diff_lines: Option<bool>,
    show_git_ignored: Option<bool>,
    search_match_case_enabled: Option<bool>,
) -> Result<(), String> {
    if let Some(directory) = path.parent() {
        if !directory.as_os_str().is_empty() {
            fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
    }
    let mut root = read_root(path)?;
    if let Some(value) = show_diff_lines {
        root.set("showDiffLines", Value::Bool(value));
    }
    if let Some(value) = show_git_ignored {
        root.set("showGitIgnored", Value::Bool(value));
    }
    if let Some(value) = search_match_case_enabled {
        root.set("searchMatchCaseEnabled", Value::Bool(value));
    }
    write_root(path, &root)
}

fn read_root(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        let mut root = Value::Object(Vec::new());
        root.set("version", Value::Number(1.0));
        root.set("formatOnSave", Value::Bool(false));
        return Ok(root);
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value = json::parse(&text)?;
    if !matches!(value, Value::Object(_)) {
        return Err("root must be a JSON object".into());
    }
    Ok(value)
}

fn write_root(path: &Path, root: &Value) -> Result<(), String> {
    let mut output = json::to_string(root, true);
    output.push('\n');
    fs::write(path, output).map_err(|error| error.to_string())
}

fn parse(text: &str, path: &Path) -> Result<Config, String> {
    let root = json::parse(text)?;
    if !matches!(root, Value::Object(_)) {
        return Err("root must be a JSON object".into());
    }
    let editor = root.get("editor");
    let read_string = |name: &str, alias: &str| -> Option<String> {
        root.get(name)
            .and_then(Value::as_str)
            .or_else(|| root.get(alias).and_then(Value::as_str))
            .or_else(|| {
                editor.and_then(|editor| {
                    editor
                        .get(name)
                        .and_then(Value::as_str)
                        .or_else(|| editor.get(alias).and_then(Value::as_str))
                })
            })
            .map(str::to_string)
    };
    let read_bool = |name: &str, alias: &str| -> Option<bool> {
        root.get(name)
            .and_then(Value::as_bool)
            .or_else(|| root.get(alias).and_then(Value::as_bool))
            .or_else(|| {
                editor.and_then(|editor| {
                    editor
                        .get(name)
                        .and_then(Value::as_bool)
                        .or_else(|| editor.get(alias).and_then(Value::as_bool))
                })
            })
    };
    let read_size_mb = |name: &str, alias: &str| -> Option<u64> {
        root.get(name)
            .and_then(Value::as_i64)
            .or_else(|| root.get(alias).and_then(Value::as_i64))
            .or_else(|| {
                editor.and_then(|editor| {
                    editor
                        .get(name)
                        .and_then(Value::as_i64)
                        .or_else(|| editor.get(alias).and_then(Value::as_i64))
                })
            })
            .and_then(|value| u64::try_from(value).ok())
    };

    let central_repository_url = read_string("centralRepositoryUrl", "central_repository_url");
    Ok(Config {
        source_path: path.to_path_buf(),
        central_repository_url: central_repository_url
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CENTRAL_REPOSITORY_URL.to_string()),
        format_on_save: read_bool("formatOnSave", "format_on_save").unwrap_or(false),
        fsync_on_save: read_bool("fsyncOnSave", "fsync_on_save").unwrap_or(false),
        trim_trailing_whitespace_on_save: read_bool(
            "trimTrailingWhitespaceOnSave",
            "trim_trailing_whitespace_on_save",
        )
        .unwrap_or(false),
        smart_backspace: read_bool("smartBackspace", "smart_backspace").unwrap_or(true),
        auto_detect_indentation_on_startup: read_bool(
            "autoDetectIndentationOnStartup",
            "auto_detect_indentation_on_startup",
        )
        .unwrap_or(true),
        startup_benchmarker: read_bool("startupBenchmarker", "startup_benchmarker")
            .unwrap_or(false),
        syntax_highlight_picker_preview: read_bool(
            "syntaxHighlightPickerPreview",
            "syntax_highlight_picker_preview",
        )
        .unwrap_or(false),
        debug_adapter: read_bool("debugAdapter", "debug_adapter").unwrap_or(true),
        show_diff_lines: read_bool("showDiffLines", "show_diff_lines").unwrap_or(true),
        diagnostic_line_highlight: read_bool(
            "diagnosticLineHighlight",
            "diagnostic_line_highlight",
        )
        .unwrap_or(true),
        show_git_ignored: read_bool("showGitIgnored", "show_git_ignored").unwrap_or(false),
        show_modified_indicators: read_bool("showModifiedIndicators", "show_modified_indicators")
            .unwrap_or(true),
        word_wrap: read_bool("wordWrap", "word_wrap").unwrap_or(false),
        keep_sidebar_focus_on_open: read_bool(
            "keepSidebarFocusOnOpen",
            "keep_sidebar_focus_on_open",
        )
        .unwrap_or(true),
        confirm_quit_without_saving: read_bool(
            "confirmQuitWithoutSaving",
            "confirm_quit_without_saving",
        )
        .unwrap_or(true),
        search_match_case_enabled: read_bool("searchMatchCaseEnabled", "search_match_case_enabled")
            .unwrap_or(false),
        search_skip_files_over_size_mb: read_size_mb(
            "searchSkipFilesOverSizeMb",
            "search_skip_files_over_size_mb",
        )
        .unwrap_or(4),
    })
}

pub fn hexal_home_directory() -> PathBuf {
    home_directory().join(".hexal")
}

pub fn default_path() -> PathBuf {
    hexal_home_directory().join(DEFAULT_FILE_NAME)
}

pub fn alternate_path() -> PathBuf {
    hexal_home_directory().join("hexcfg")
}

pub fn resolve_path() -> PathBuf {
    let primary = default_path();
    if primary.exists() {
        return primary;
    }
    let alternate = alternate_path();
    if alternate.exists() {
        return alternate;
    }
    primary
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::{Config, parse};
    use std::path::Path;

    #[test]
    fn parses_aliases_and_editor_object() {
        let config = parse(
            r#"{
                "format_on_save": true,
                "fsyncOnSave": true,
                "editor": { "showGitIgnored": true, "search_match_case_enabled": true },
                "wordWrap": true
            }"#,
            Path::new("/tmp/test.hexcfg"),
        )
        .unwrap();
        assert!(config.format_on_save);
        assert!(config.fsync_on_save);
        assert!(config.show_git_ignored);
        assert!(config.search_match_case_enabled);
        assert!(config.word_wrap);
        assert!(config.show_diff_lines);
        assert_eq!(config.search_skip_files_over_size_mb, 4);
    }

    #[test]
    fn parses_search_size_limit_from_editor_object() {
        let config = parse(
            r#"{
                "editor": { "searchSkipFilesOverSizeMb": 12 }
            }"#,
            Path::new("/tmp/test.hexcfg"),
        )
        .unwrap();
        assert_eq!(config.search_skip_files_over_size_mb, 12);

        let unlimited = parse(
            r#"{ "search_skip_files_over_size_mb": 0 }"#,
            Path::new("/tmp/test.hexcfg"),
        )
        .unwrap();
        assert_eq!(unlimited.search_skip_files_over_size_mb, 0);
    }

    #[test]
    fn parses_default_contents() {
        let config = parse(super::DEFAULT_CONTENTS, Path::new("/tmp/test.hexcfg")).unwrap();
        assert!(config.auto_detect_indentation_on_startup);
        assert!(!config.fsync_on_save);
        assert!(config.smart_backspace);
        assert!(config.show_diff_lines);
        assert!(!config.word_wrap);
        assert!(!config.search_match_case_enabled);
        assert_eq!(config.search_skip_files_over_size_mb, 4);
        assert_eq!(
            config.central_repository_url,
            super::DEFAULT_CENTRAL_REPOSITORY_URL
        );
    }

    #[test]
    fn creates_missing_config_when_persisting() {
        let directory =
            std::env::temp_dir().join(format!("hexal-config-create-{}", std::process::id()));
        let path = directory.join(".hexcfg");
        let _ = std::fs::remove_dir_all(&directory);
        Config::update_display_settings(&path, true, true, false).unwrap();
        let parsed = crate::util::json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed
                .get("version")
                .and_then(crate::util::json::Value::as_i64),
            Some(1)
        );
        assert_eq!(
            parsed
                .get("formatOnSave")
                .and_then(crate::util::json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            parsed
                .get("showGitIgnored")
                .and_then(crate::util::json::Value::as_bool),
            Some(true)
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn writes_display_settings_and_preserves_unknown_keys() {
        let path =
            std::env::temp_dir().join(format!("hexal-config-test-{}.hexcfg", std::process::id()));
        std::fs::write(
            &path,
            r#"{"version":1,"unknown":{"a":1},"showDiffLines":true}"#,
        )
        .unwrap();
        Config::update_display_settings(&path, false, true, true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\n  \"version\": 1,\n"));
        assert!(text.ends_with('\n'));
        assert!(text.contains("\"unknown\""));
        let parsed = crate::util::json::parse(&text).unwrap();
        assert_eq!(
            parsed
                .get("showDiffLines")
                .and_then(crate::util::json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            parsed
                .get("showGitIgnored")
                .and_then(crate::util::json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            parsed
                .get("searchMatchCaseEnabled")
                .and_then(crate::util::json::Value::as_bool),
            Some(true)
        );
        let _ = std::fs::remove_file(path);
    }
}
