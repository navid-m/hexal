//! Data-driven language definitions loaded from `.hexdef` grammars.
//!
//! The schema and defaults mirror the C# editor's `LanguageDefinition`
//! exactly, including the legacy aliases accepted by its JSON reader.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::ui::theme::SyntaxKind;
use crate::util::json::{self, Value};

use super::regex::Regex;

#[derive(Clone, Debug)]
pub struct IndentationSettings {
    pub use_tabs: bool,
    pub indent_width: usize,
    pub tab_width: usize,
}

#[derive(Clone, Debug)]
pub struct LspFallback {
    pub name: String,
    pub command: String,
    pub arguments: Vec<String>,
    pub language_id: String,
}

#[derive(Clone, Debug)]
pub struct FormatterConfig {
    pub command: String,
    pub arguments: Vec<String>,
    pub input: String,
    pub output: String,
    pub timeout_ms: u64,
}

impl FormatterConfig {
    pub fn uses_stdin(&self) -> bool {
        !self.input.eq_ignore_ascii_case("file")
    }

    pub fn writes_file(&self) -> bool {
        self.output.eq_ignore_ascii_case("file")
    }
}

#[derive(Clone, Debug)]
pub struct LineHighlight {
    pub pattern: Regex,
    pub kind: SyntaxKind,
}

/// A fast deterministic hasher for keyword lookup. Grammar keyword tables are
/// built once at startup and queried for every identifier on screen.
#[derive(Default)]
pub struct FnvHasher(u64);

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        self.0 = hash;
    }
}

pub type FnvBuildHasher = BuildHasherDefault<FnvHasher>;
pub type ClassificationMap = HashMap<Box<[u8]>, SyntaxKind, FnvBuildHasher>;

#[derive(Clone, Debug)]
pub struct Language {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub file_names: Vec<String>,
    pub line_highlights: Vec<LineHighlight>,

    classifications: ClassificationMap,
    case_insensitive: bool,

    pub line_comment: Option<String>,
    pub block_comment_start: Option<String>,
    pub block_comment_end: Option<String>,
    pub quote_characters: Vec<Vec<u8>>,
    pub escape_character: Vec<u8>,
    pub highlight_numbers: bool,

    pub formatter: Option<FormatterConfig>,
    pub fallback_formatter: Option<FormatterConfig>,
    pub fallback_lsp: Option<LspFallback>,
    pub format_on_save: Option<bool>,
    pub indentation: Option<IndentationSettings>,
    pub auto_indent: bool,
    pub auto_close: bool,
    pub traditional_markup: bool,
    pub indent_after: Vec<String>,
    pub unindent_before: Vec<String>,
}

impl Language {
    pub fn classify_identifier(&self, identifier: &[u8]) -> SyntaxKind {
        if identifier.is_empty() {
            return SyntaxKind::Default;
        }
        if self.case_insensitive {
            let mut lower = Vec::with_capacity(identifier.len());
            lower.extend_from_slice(identifier);
            lower.make_ascii_lowercase();
            self.classifications
                .get(lower.as_slice())
                .copied()
                .unwrap_or(SyntaxKind::Default)
        } else {
            self.classifications
                .get(identifier)
                .copied()
                .unwrap_or(SyntaxKind::Default)
        }
    }

    pub fn is_unindent_keyword(&self, text: &str) -> bool {
        self.unindent_before.iter().any(|keyword| keyword == text)
    }

    /// Returns the primary formatter when its executable exists in PATH,
    /// otherwise a fallback whose executable exists. If neither exists the
    /// primary is still returned so the normal error path can report it.
    pub fn resolve_formatter(&self) -> Option<&FormatterConfig> {
        let formatter = self.formatter.as_ref()?;
        if crate::util::git::command_exists(&formatter.command) {
            return Some(formatter);
        }
        if let Some(fallback) = &self.fallback_formatter {
            if crate::util::git::command_exists(&fallback.command) {
                return Some(fallback);
            }
        }
        Some(formatter)
    }
}

struct Builder {
    classifications: ClassificationMap,
    case_insensitive: bool,
}

impl Builder {
    fn new(case_insensitive: bool) -> Builder {
        Builder {
            classifications: ClassificationMap::default(),
            case_insensitive,
        }
    }

    fn add(&mut self, value: &str, kind: SyntaxKind) {
        if value.is_empty() {
            return;
        }
        let bytes = if self.case_insensitive {
            value.to_ascii_lowercase().into_bytes()
        } else {
            value.as_bytes().to_vec()
        };
        self.classifications
            .entry(bytes.into_boxed_slice())
            .or_insert(kind);
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn bool_value(element: &Value, key: &str) -> Option<bool> {
    match element.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn string_array(element: &Value, key: &str) -> Vec<String> {
    let Some(value) = element.get(key) else {
        return Vec::new();
    };
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn read_set(element: &Value, key: &str, builder: &mut Builder, kind: SyntaxKind) {
    for value in string_array(element, key) {
        builder.add(&value, kind);
    }
}

fn parse_formatter(
    element: &Value,
    field: &str,
) -> Result<(Option<FormatterConfig>, Option<bool>), String> {
    if element.get(field).is_none() {
        return Ok((None, None));
    }
    let value = element.get(field).unwrap();
    if !matches!(value, Value::Object(_)) {
        return Err(format!("{field} must be a JSON object"));
    }
    let command = string_value(value.get("command"))
        .unwrap_or_default()
        .trim()
        .to_string();
    if command.is_empty() {
        return Err(format!("{field}.command is required"));
    }
    let input = string_value(value.get("input"))
        .unwrap_or_else(|| "stdin".into())
        .trim()
        .to_ascii_lowercase();
    if input != "stdin" && input != "file" {
        return Err(format!("{field}.input must be 'stdin' or 'file'"));
    }
    let output = string_value(value.get("output"))
        .unwrap_or_else(|| "stdout".into())
        .trim()
        .to_ascii_lowercase();
    if output != "stdout" && output != "file" {
        return Err(format!("{field}.output must be 'stdout' or 'file'"));
    }
    let mut timeout_ms = 15000u64;
    if let Some(Value::Number(value)) = value.get("timeout") {
        let configured = *value as i64;
        if configured > 0 {
            timeout_ms = configured as u64;
        }
    }
    Ok((
        Some(FormatterConfig {
            command,
            arguments: string_array(value, "arguments"),
            input,
            output,
            timeout_ms,
        }),
        bool_value(value, "formatOnSave").or_else(|| bool_value(value, "format_on_save")),
    ))
}

fn parse_fallback_formatter(element: &Value) -> Result<Option<FormatterConfig>, String> {
    let key = if element.get("fallbackFormatter").is_some() {
        "fallbackFormatter"
    } else if element.get("fallback_formatter").is_some() {
        "fallback_formatter"
    } else {
        return Ok(None);
    };
    Ok(parse_formatter(element, key)?.0)
}

fn parse_fallback_lsp(element: &Value, id: &str) -> Result<Option<LspFallback>, String> {
    let value = if let Some(value) = element.get("fallbackLsp") {
        value
    } else if let Some(value) = element.get("fallback_lsp") {
        value
    } else {
        return Ok(None);
    };
    if !matches!(value, Value::Object(_)) {
        return Err("fallbackLsp must be a JSON object".into());
    }
    let command = string_value(value.get("command"))
        .unwrap_or_default()
        .trim()
        .to_string();
    if command.is_empty() {
        return Err("fallbackLsp.command is required".into());
    }
    let mut name = string_value(value.get("name"))
        .unwrap_or_default()
        .trim()
        .to_string();
    if name.is_empty() {
        name = command.clone();
    }
    let mut arguments = string_array(value, "arguments");
    if arguments.is_empty() {
        arguments = string_array(value, "args");
    }
    Ok(Some(LspFallback {
        name,
        command,
        arguments,
        language_id: string_value(value.get("languageId"))
            .or_else(|| string_value(value.get("language_id")))
            .unwrap_or_else(|| id.to_string()),
    }))
}

fn parse_indentation(element: &Value) -> Result<Option<IndentationSettings>, String> {
    let Some(value) = element.get("indentation") else {
        return Ok(None);
    };
    if !matches!(value, Value::Object(_)) {
        return Err("indentation must be a JSON object".into());
    }
    let style = string_value(value.get("style"))
        .unwrap_or_else(|| "spaces".into())
        .trim()
        .to_ascii_lowercase();
    if style != "spaces" && style != "tabs" {
        return Err("indentation.style must be 'spaces' or 'tabs'".into());
    }
    let use_tabs = style == "tabs";
    let mut indent_width = if use_tabs { 4 } else { 4 };
    if let Some(Value::Number(value)) = value.get("width") {
        let configured = *value as i64;
        if !(1..=16).contains(&configured) {
            return Err("indentation.width must be an integer between 1 and 16".into());
        }
        indent_width = configured as usize;
    }
    let mut tab_width = if use_tabs { indent_width } else { 4 };
    if let Some(Value::Number(value)) = value.get("tabWidth") {
        let configured = *value as i64;
        if !(1..=16).contains(&configured) {
            return Err("indentation.tabWidth must be an integer between 1 and 16".into());
        }
        tab_width = configured as usize;
    }
    Ok(Some(IndentationSettings {
        use_tabs,
        indent_width,
        tab_width,
    }))
}

fn token_kind(name: &str) -> Option<SyntaxKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "keyword" => Some(SyntaxKind::Keyword),
        "control" | "controlstatement" | "control_statement" => Some(SyntaxKind::Control),
        "type" => Some(SyntaxKind::Type),
        "constant" => Some(SyntaxKind::Constant),
        "string" | "stringliteral" | "string_literal" => Some(SyntaxKind::String),
        "comment" => Some(SyntaxKind::Comment),
        "number" => Some(SyntaxKind::Number),
        "added" => Some(SyntaxKind::Added),
        "deleted" => Some(SyntaxKind::Deleted),
        _ => None,
    }
}

impl Language {
    pub fn parse(json_text: &str, source_path: &str) -> Result<Language, String> {
        let root = json::parse(json_text)?;
        if !matches!(root, Value::Object(_)) {
            return Err("root must be a JSON object".into());
        }

        let fallback_id = std::path::Path::new(source_path)
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut id = string_value(root.get("id"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if id.is_empty() {
            id = fallback_id;
        }
        if id.is_empty() {
            return Err("missing language id".into());
        }
        let mut name = string_value(root.get("name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            name = id.clone();
        }

        let mut extensions = Vec::new();
        for extension in string_array(&root, "extensions") {
            let mut value = extension.trim().to_string();
            if value.is_empty() {
                continue;
            }
            if !value.starts_with('.') {
                value.insert(0, '.');
            }
            let lower = value.to_ascii_lowercase();
            if !extensions.contains(&lower) {
                extensions.push(lower);
            }
        }

        let mut file_names = Vec::new();
        for file_name in string_array(&root, "fileNames")
            .into_iter()
            .chain(string_array(&root, "file_names"))
        {
            let value = file_name.trim().to_string();
            if !value.is_empty()
                && !file_names
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
            {
                file_names.push(value);
            }
        }

        let mut line_highlights = Vec::new();
        if let Some(value) = root.get("linePatterns") {
            let Value::Array(patterns) = value else {
                return Err("linePatterns must be an array".into());
            };
            for (index, pattern) in patterns.iter().enumerate() {
                if !matches!(pattern, Value::Object(_)) {
                    return Err(format!("linePatterns[{index}] must be a JSON object"));
                }
                let text = string_value(pattern.get("pattern"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if text.is_empty() {
                    return Err(format!("linePatterns[{index}].pattern is required"));
                }
                let kind_name = string_value(pattern.get("kind")).unwrap_or_default();
                let Some(kind) = token_kind(&kind_name) else {
                    return Err(format!("linePatterns[{index}].kind is invalid"));
                };
                let compiled = Regex::new(&text).map_err(|error| {
                    format!("linePatterns[{index}].pattern is invalid: {error}")
                })?;
                line_highlights.push(LineHighlight {
                    pattern: compiled,
                    kind,
                });
            }
        }

        let case_insensitive = matches!(root.get("caseInsensitive"), Some(Value::Bool(true)));
        let mut builder = Builder::new(case_insensitive);
        read_set(&root, "keywords", &mut builder, SyntaxKind::Keyword);
        read_set(
            &root,
            "controlStatements",
            &mut builder,
            SyntaxKind::Control,
        );
        read_set(&root, "types", &mut builder, SyntaxKind::Type);
        read_set(&root, "constants", &mut builder, SyntaxKind::Constant);

        let mut line_comment = None;
        let mut block_start = None;
        let mut block_end = None;
        if let Some(comments) = root.get("comments") {
            if matches!(comments, Value::Object(_)) {
                line_comment = string_value(comments.get("line"));
                block_start = string_value(comments.get("blockStart"))
                    .or_else(|| string_value(comments.get("start")));
                block_end = string_value(comments.get("blockEnd"))
                    .or_else(|| string_value(comments.get("end")));
            }
        }
        if line_comment.is_none() {
            line_comment = string_value(root.get("lineComment"));
        }
        if let Some(block) = root.get("blockComment") {
            if matches!(block, Value::Object(_)) {
                if block_start.is_none() {
                    block_start = string_value(block.get("start"));
                }
                if block_end.is_none() {
                    block_end = string_value(block.get("end"));
                }
            }
        }
        if block_start.is_some() != block_end.is_some() {
            return Err("comments.blockStart and comments.blockEnd must both be set".into());
        }

        let mut quote_characters = Vec::new();
        let mut escape_character = Vec::new();
        if let Some(strings) = root.get("strings") {
            if matches!(strings, Value::Object(_)) {
                for quote in string_array(strings, "quoteCharacters") {
                    if !quote.is_empty()
                        && !quote_characters
                            .iter()
                            .any(|existing: &Vec<u8>| existing.as_slice() == quote.as_bytes())
                    {
                        quote_characters.push(quote.into_bytes());
                    }
                }
                if let Some(escape) = string_value(strings.get("escapeCharacter")) {
                    escape_character = escape.into_bytes();
                }
            }
        }

        let highlight_numbers = match root.get("numbers") {
            Some(Value::Bool(value)) => *value,
            _ => true,
        };

        let mut format_on_save =
            bool_value(&root, "formatOnSave").or_else(|| bool_value(&root, "format_on_save"));
        let (formatter, formatter_format_on_save) = parse_formatter(&root, "formatter")?;
        if format_on_save.is_none() {
            format_on_save = formatter_format_on_save;
        }
        let fallback_formatter = parse_fallback_formatter(&root)?;
        let fallback_lsp = parse_fallback_lsp(&root, &id)?;
        let indentation = parse_indentation(&root)?;

        if line_comment.as_deref() == Some("") {
            line_comment = None;
        }
        if block_start.as_deref() == Some("") {
            block_start = None;
            block_end = None;
        }

        let auto_indent = bool_value(&root, "autoIndent")
            .or_else(|| bool_value(&root, "auto_indent"))
            .unwrap_or(true);
        let auto_close = bool_value(&root, "autoClose")
            .or_else(|| bool_value(&root, "auto_close"))
            .unwrap_or(true);
        let traditional_markup = bool_value(&root, "traditionalMarkupLanguage")
            .or_else(|| bool_value(&root, "traditional_markup_language"))
            .unwrap_or(false);

        let mut indent_after = string_array(&root, "indentAfter");
        indent_after.extend(string_array(&root, "indent_after"));
        indent_after.retain(|value| !value.is_empty());
        indent_after.sort();
        indent_after.dedup();

        let mut unindent_before = string_array(&root, "unindentBefore");
        unindent_before.extend(string_array(&root, "unindent_before"));
        unindent_before.retain(|value| !value.is_empty());
        unindent_before.sort();
        unindent_before.dedup();

        Ok(Language {
            id,
            name,
            extensions,
            file_names,
            line_highlights,
            classifications: builder.classifications,
            case_insensitive,
            line_comment,
            block_comment_start: block_start,
            block_comment_end: block_end,
            quote_characters,
            escape_character,
            highlight_numbers,
            formatter,
            fallback_formatter,
            fallback_lsp,
            format_on_save,
            indentation,
            auto_indent,
            auto_close,
            traditional_markup,
            indent_after,
            unindent_before,
        })
    }
}
