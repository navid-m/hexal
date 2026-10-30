//! Named colour palettes and named styles.
//!
//! A colour is either an ANSI palette index (0-255) or a packed 24-bit RGB
//! value marked by [`TRUE_COLOR_FLAG`]. Themes are JSON files in
//! `data/themes`, matching the C# editor's schema. The active theme id is
//! remembered in `~/.hexal/theme`.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::json::{self, Value};

pub const TRUE_COLOR_FLAG: u32 = 0x0100_0000;

pub fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    TRUE_COLOR_FLAG | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

pub fn is_true_color(color: u32) -> bool {
    color & TRUE_COLOR_FLAG != 0
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Style {
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub dim: bool,
    pub reverse: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fg: 15,
            bg: 0,
            bold: false,
            dim: false,
            reverse: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyntaxKind {
    Default,
    Keyword,
    Control,
    Type,
    Constant,
    String,
    Comment,
    Number,
    Added,
    Deleted,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub license: String,

    pub foreground: u32,
    pub background: u32,
    pub muted: u32,
    pub dimmed: u32,
    pub accent: u32,
    pub border: u32,
    pub error: u32,
    pub warning: u32,
    pub success: u32,

    pub diff_added: u32,
    pub diff_deleted: u32,
    pub diff_modified: u32,
    pub git_modified: u32,

    pub alert_fg: u32,
    pub alert_bg: u32,
    pub selection_fg: u32,
    pub selection_bg: u32,
    pub status_fg: u32,
    pub status_bg: u32,
    pub insert_fg: u32,
    pub insert_bg: u32,
    pub select_fg: u32,
    pub select_bg: u32,
    pub prompt_fg: u32,
    pub prompt_bg: u32,
    pub command_fg: u32,
    pub command_bg: u32,
    pub match_fg: u32,
    pub match_bg: u32,
    pub current_match_fg: u32,
    pub current_match_bg: u32,

    pub keyword: u32,
    pub control: u32,
    pub type_color: u32,
    pub constant: u32,
    pub string: u32,
    pub comment: u32,
    pub number: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            id: "hexal-dark".into(),
            name: "Hexal Dark".into(),
            description: "Default Hexal theme.".into(),
            author: String::new(),
            license: String::new(),
            foreground: 15,
            background: 0,
            muted: 8,
            dimmed: 8,
            accent: 14,
            border: 8,
            error: 9,
            warning: 11,
            success: 10,
            diff_added: 2,
            diff_deleted: 1,
            diff_modified: 4,
            git_modified: 14,
            alert_fg: 15,
            alert_bg: 5,
            selection_fg: 15,
            selection_bg: 4,
            status_fg: 15,
            status_bg: 4,
            insert_fg: 0,
            insert_bg: 2,
            select_fg: 0,
            select_bg: 3,
            prompt_fg: 0,
            prompt_bg: 6,
            command_fg: 0,
            command_bg: 6,
            match_fg: 0,
            match_bg: 3,
            current_match_fg: 0,
            current_match_bg: 11,
            keyword: 13,
            control: 12,
            type_color: 14,
            constant: 11,
            string: 11,
            comment: 10,
            number: 11,
        }
    }
}

impl Theme {
    pub fn base(&self) -> Style {
        Style {
            fg: self.foreground,
            bg: self.background,
            ..Style::default()
        }
    }

    pub fn text(&self) -> Style {
        self.base()
    }

    pub fn muted_style(&self) -> Style {
        Style {
            fg: self.muted,
            bg: self.background,
            dim: true,
            ..Style::default()
        }
    }

    pub fn accent_style(&self) -> Style {
        Style {
            fg: self.accent,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn border_style(&self) -> Style {
        Style {
            fg: self.border,
            bg: self.background,
            dim: true,
            ..Style::default()
        }
    }

    pub fn error_style(&self) -> Style {
        Style {
            fg: self.error,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn warning_style(&self) -> Style {
        Style {
            fg: self.warning,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    #[allow(dead_code)]
    pub fn success_style(&self) -> Style {
        Style {
            fg: self.success,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn selection_style(&self) -> Style {
        Style {
            fg: self.selection_fg,
            bg: self.selection_bg,
            ..Style::default()
        }
    }

    pub fn selected_item_style(&self) -> Style {
        Style {
            fg: self.selection_fg,
            bg: self.selection_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn status_style(&self) -> Style {
        Style {
            fg: self.status_fg,
            bg: self.status_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn insert_style(&self) -> Style {
        Style {
            fg: self.insert_fg,
            bg: self.insert_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn select_style(&self) -> Style {
        Style {
            fg: self.select_fg,
            bg: self.select_bg,
            bold: true,
            ..Style::default()
        }
    }

    #[allow(dead_code)]
    pub fn prompt_style(&self) -> Style {
        Style {
            fg: self.prompt_fg,
            bg: self.prompt_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn alert_style(&self) -> Style {
        Style {
            fg: self.alert_fg,
            bg: self.alert_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn command_style(&self) -> Style {
        Style {
            fg: self.command_fg,
            bg: self.command_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn match_style(&self) -> Style {
        Style {
            fg: self.match_fg,
            bg: self.match_bg,
            ..Style::default()
        }
    }

    pub fn current_match_style(&self) -> Style {
        Style {
            fg: self.current_match_fg,
            bg: self.current_match_bg,
            bold: true,
            ..Style::default()
        }
    }

    pub fn current_line_number_style(&self) -> Style {
        Style {
            fg: self.foreground,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    #[allow(dead_code)]
    pub fn diff_modified_style(&self) -> Style {
        Style {
            fg: self.diff_modified,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn git_modified_style(&self) -> Style {
        Style {
            fg: self.git_modified,
            bg: self.background,
            bold: true,
            ..Style::default()
        }
    }

    pub fn syntax_style(&self, kind: SyntaxKind) -> Style {
        let fg = match kind {
            SyntaxKind::Keyword => self.keyword,
            SyntaxKind::Control => self.control,
            SyntaxKind::Type => self.type_color,
            SyntaxKind::Constant => self.constant,
            SyntaxKind::String => self.string,
            SyntaxKind::Comment => self.comment,
            SyntaxKind::Number => self.number,
            SyntaxKind::Added => self.diff_added,
            SyntaxKind::Deleted => self.diff_deleted,
            SyntaxKind::Default => self.foreground,
        };
        Style {
            fg,
            bg: self.background,
            ..Style::default()
        }
    }
}

const BUILTIN_THEMES: &[&str] = &[
    include_str!("../../data/themes/hexal-dark.json"),
    include_str!("../../data/themes/gruvbox.json"),
    include_str!("../../data/themes/mono.json"),
    include_str!("../../data/themes/nord.json"),
    include_str!("../../data/themes/solarized-light.json"),
];

pub struct ThemeRegistry {
    themes: Vec<Theme>,
    active: Theme,
    errors: Vec<String>,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeRegistry {
    pub fn new() -> ThemeRegistry {
        ThemeRegistry {
            themes: Vec::new(),
            active: Theme::default(),
            errors: Vec::new(),
        }
    }

    pub fn active(&self) -> &Theme {
        &self.active
    }

    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn find(&self, id: &str) -> Option<&Theme> {
        if id.is_empty() {
            return None;
        }
        self.themes
            .iter()
            .find(|theme| theme.id.eq_ignore_ascii_case(id))
    }

    pub fn set_active(&mut self, id: &str) -> bool {
        if let Some(theme) = self.find(id).cloned() {
            self.active = theme;
            true
        } else {
            false
        }
    }

    /// Load only the active theme. This keeps startup free of a directory scan
    /// for installations that do not use themes.
    pub fn load_default() -> ThemeRegistry {
        let mut registry = ThemeRegistry::new();
        let directory = theme_directory();
        let active_id = read_active_theme_id();
        let wanted = if active_id.is_empty() {
            "hexal-dark"
        } else {
            active_id.as_str()
        };

        if let Some(directory) = directory {
            let direct = directory.join(format!("{wanted}.json"));
            if direct.is_file() {
                registry.load_file(&direct);
            } else if let Ok(entries) = fs::read_dir(&directory) {
                let mut files: Vec<PathBuf> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                    .collect();
                files.sort();
                for file in files {
                    registry.load_file(&file);
                    if registry.find(wanted).is_some() {
                        break;
                    }
                }
            }
        }

        registry.load_builtins();
        if registry.themes.is_empty() {
            registry.themes.push(Theme::default());
        }
        registry.active = registry
            .find(wanted)
            .cloned()
            .or_else(|| registry.themes.first().cloned())
            .unwrap_or_default();
        registry
    }

    fn load_builtins(&mut self) {
        for text in BUILTIN_THEMES {
            if let Ok(theme) = Theme::parse(text, Path::new("builtin.json")) {
                if self.find(&theme.id).is_none() {
                    self.themes.push(theme);
                }
            }
        }
    }

    /// Load every theme, for the theme picker and `:rcfg`.
    pub fn load_all() -> ThemeRegistry {
        let mut registry = ThemeRegistry::new();
        let directory = theme_directory();
        let active_id = read_active_theme_id();
        if let Some(directory) = directory {
            if let Ok(entries) = fs::read_dir(&directory) {
                let mut files: Vec<PathBuf> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                    .collect();
                files.sort();
                for file in files {
                    registry.load_file(&file);
                }
            }
        }
        registry.load_builtins();
        if registry.themes.is_empty() {
            registry.themes.push(Theme::default());
        }
        let wanted = if active_id.is_empty() {
            "hexal-dark"
        } else {
            active_id.as_str()
        };
        registry.active = registry
            .find(wanted)
            .cloned()
            .or_else(|| registry.themes.first().cloned())
            .unwrap_or_default();
        registry
    }

    fn load_file(&mut self, path: &Path) {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.errors.push(format!("{}: {error}", path.display()));
                return;
            }
        };
        match Theme::parse(&text, path) {
            Ok(theme) => {
                if self.find(&theme.id).is_some() {
                    self.errors.push(format!(
                        "{}: duplicate theme id '{}'",
                        path.display(),
                        theme.id
                    ));
                } else {
                    self.themes.push(theme);
                }
            }
            Err(error) => self.errors.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
}

impl Theme {
    pub fn parse(text: &str, source: &Path) -> Result<Theme, String> {
        let root = json::parse(text)?;
        let Value::Object(_) = root else {
            return Err("root must be a JSON object".into());
        };

        let fallback_id = source
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "theme".into());
        let mut theme = Theme::default();
        theme.id = string_field(&root, "id", &fallback_id);
        if theme.id.is_empty() {
            theme.id = "theme".into();
        }
        theme.name = string_field(&root, "name", &theme.id);
        theme.description = string_field(&root, "description", "");
        theme.author = string_field(&root, "author", "").trim().to_string();
        theme.license = string_field(&root, "license", "").trim().to_string();

        if let Some(colors) = root.get("colors") {
            if colors.as_object().is_none() {
                return Err("colors must be a JSON object".into());
            }
            theme.foreground = color_field(colors, "foreground", theme.foreground)?;
            theme.background = color_field(colors, "background", theme.background)?;
            theme.muted = color_field(colors, "muted", theme.muted)?;
            theme.dimmed = color_field(colors, "dimmed", theme.muted)?;
            theme.accent = color_field(colors, "accent", theme.accent)?;
            theme.border = color_field(colors, "border", theme.border)?;
            theme.error = color_field(colors, "error", theme.error)?;
            theme.warning = color_field(colors, "warning", theme.warning)?;
            theme.success = color_field(colors, "success", theme.success)?;
            theme.git_modified = color_field(colors, "gitModified", theme.git_modified)?;

            (theme.alert_fg, theme.alert_bg) =
                pair_field(colors, "alert", theme.alert_fg, theme.alert_bg)?;
            (theme.selection_fg, theme.selection_bg) =
                pair_field(colors, "selection", theme.selection_fg, theme.selection_bg)?;
            (theme.status_fg, theme.status_bg) =
                pair_field(colors, "status", theme.status_fg, theme.status_bg)?;
            (theme.insert_fg, theme.insert_bg) =
                pair_field(colors, "insert", theme.insert_fg, theme.insert_bg)?;
            (theme.select_fg, theme.select_bg) =
                pair_field(colors, "select", theme.select_fg, theme.select_bg)?;
            (theme.prompt_fg, theme.prompt_bg) =
                pair_field(colors, "prompt", theme.prompt_fg, theme.prompt_bg)?;
            (theme.command_fg, theme.command_bg) =
                pair_field(colors, "command", theme.command_fg, theme.command_bg)?;
            (theme.match_fg, theme.match_bg) =
                pair_field(colors, "match", theme.match_fg, theme.match_bg)?;
            (theme.current_match_fg, theme.current_match_bg) = pair_field(
                colors,
                "currentMatch",
                theme.current_match_fg,
                theme.current_match_bg,
            )?;

            if let Some(syntax) = colors.get("syntax") {
                if syntax.as_object().is_none() {
                    return Err("colors.syntax must be a JSON object".into());
                }
                theme.keyword = color_field(syntax, "keyword", theme.keyword)?;
                theme.control = color_field(syntax, "control", theme.control)?;
                theme.type_color = color_field(syntax, "type", theme.type_color)?;
                theme.constant = color_field(syntax, "constant", theme.constant)?;
                theme.string = color_field(syntax, "string", theme.string)?;
                theme.comment = color_field(syntax, "comment", theme.comment)?;
                theme.number = color_field(syntax, "number", theme.number)?;
            }
        }

        Ok(theme)
    }
}

fn string_field(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn pair_field(colors: &Value, key: &str, fg: u32, bg: u32) -> Result<(u32, u32), String> {
    let Some(pair) = colors.get(key) else {
        return Ok((fg, bg));
    };
    if pair.as_object().is_none() {
        return Err(format!("{key} must be an object with fg and bg"));
    }
    Ok((color_field(pair, "fg", fg)?, color_field(pair, "bg", bg)?))
}

fn color_field(value: &Value, key: &str, fallback: u32) -> Result<u32, String> {
    match value.get(key) {
        Some(color) => parse_color(color)
            .ok_or_else(|| format!("{key}: invalid color (use 0-255, a name, or #RRGGBB)")),
        None => Ok(fallback),
    }
}

pub fn parse_color(value: &Value) -> Option<u32> {
    if let Some(number) = value.as_i64() {
        return (0..=255).contains(&number).then_some(number as u32);
    }
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(hex) = text.strip_prefix('#') {
        let expanded;
        let hex = if hex.len() == 3 {
            expanded = format!(
                "{}{}{}{}{}{}",
                &hex[0..1],
                &hex[0..1],
                &hex[1..2],
                &hex[1..2],
                &hex[2..3],
                &hex[2..3]
            );
            expanded.as_str()
        } else {
            hex
        };
        if hex.len() != 6 {
            return None;
        }
        let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(rgb(red, green, blue));
    }
    if let Ok(index) = text.parse::<i64>() {
        return (0..=255).contains(&index).then_some(index as u32);
    }
    named_color(text)
}

fn named_color(text: &str) -> Option<u32> {
    let name = text.to_ascii_lowercase().replace('_', "-");
    let index = match name.as_str() {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "bright-black" | "gray" | "grey" => 8,
        "bright-red" => 9,
        "bright-green" => 10,
        "bright-yellow" => 11,
        "bright-blue" => 12,
        "bright-magenta" => 13,
        "bright-cyan" => 14,
        "bright-white" => 15,
        _ => return None,
    };
    Some(index)
}

pub fn theme_directory() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let portable = parent.join("data/themes");
            if portable.is_dir() {
                return Some(portable);
            }
        }
    }
    let local = Path::new("data/themes");
    if local.is_dir() {
        return Some(local.to_path_buf());
    }
    home_directory().map(|home| home.join(".hexal/data/themes"))
}

pub fn home_directory() -> Option<PathBuf> {
    crate::util::home_directory()
}

pub fn active_theme_path() -> Option<PathBuf> {
    home_directory().map(|home| home.join(".hexal/theme"))
}

pub fn read_active_theme_id() -> String {
    let Some(path) = active_theme_path() else {
        return String::new();
    };
    fs::read_to_string(path)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

pub fn save_active_theme(id: &str) -> std::io::Result<()> {
    let Some(path) = active_theme_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no home directory",
        ));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{id}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_the_csharp_schema() {
        let text = r##"{
            "id": "gruvbox",
            "name": "Gruvbox",
            "colors": {
                "foreground": "#ebdbb2",
                "background": "#282828",
                "selection": { "fg": "#fbf1c7", "bg": "#458588" },
                "syntax": { "keyword": "#fb4934", "type": "yellow" }
            }
        }"##;
        let theme = Theme::parse(text, &PathBuf::from("gruvbox.json")).unwrap();
        assert_eq!(theme.id, "gruvbox");
        assert_eq!(theme.foreground, rgb(0xeb, 0xdb, 0xb2));
        assert_eq!(theme.selection_bg, rgb(0x45, 0x85, 0x88));
        assert_eq!(theme.keyword, rgb(0xfb, 0x49, 0x34));
        assert_eq!(theme.type_color, 3);
    }

    #[test]
    fn rejects_invalid_color() {
        let text = r#"{"colors":{"foreground":"not-a-color"}}"#;
        assert!(Theme::parse(text, &PathBuf::from("x.json")).is_err());
    }
}
