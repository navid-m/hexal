//! Loads and indexes the bundled hexdef grammars plus user overrides from
//! `~/.hexal/data/grammars` and `~/.hexal/grammars`. A local grammar with the
//! same id is merged onto its bundled definition as a field-level patch.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::definition::{GrammarMetadata, Language};
use crate::util::json::{self, Value};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/grammars.rs"));
}

struct LanguageEntry {
    id: String,
    extensions: Vec<String>,
    file_names: Vec<String>,
    source_name: String,
    text: String,
    language: OnceCell<Rc<Language>>,
}

impl LanguageEntry {
    fn language(&self) -> Option<&Rc<Language>> {
        if self.language.get().is_none() {
            let language = Language::parse(&self.text, &self.source_name).ok()?;
            let _ = self.language.set(Rc::new(language));
        }
        self.language.get()
    }
}

#[derive(Default)]
pub struct Registry {
    entries: Vec<LanguageEntry>,
    by_id: HashMap<String, usize>,
    by_extension: HashMap<String, usize>,
    by_filename: HashMap<String, usize>,
    ordered_extensions: Vec<(String, usize)>,
    errors: Vec<String>,
}

impl Registry {
    pub fn load() -> Registry {
        let mut sources: Vec<(String, String)> = embedded::EMBEDDED_GRAMMARS
            .iter()
            .map(|(name, text)| ((*name).to_string(), (*text).to_string()))
            .collect();
        sources.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });

        let embedded_text: HashMap<&str, &str> = embedded::EMBEDDED_GRAMMARS
            .iter()
            .map(|(name, text)| (*name, *text))
            .collect();
        let mut embedded_by_id: Option<HashMap<String, &'static str>> = None;
        for directory in grammar_directories() {
            let mut files: Vec<PathBuf> = match fs::read_dir(&directory) {
                Ok(read) => read
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension().is_some_and(|extension| {
                            extension.to_string_lossy().eq_ignore_ascii_case("hexdef")
                        })
                    })
                    .collect(),
                Err(_) => continue,
            };
            files.sort_by(|left, right| {
                left.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_ascii_lowercase(),
                    )
            });
            for path in files {
                let Ok(mut text) = fs::read_to_string(&path) else {
                    continue;
                };
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if embedded_text.get(name.as_str()) == Some(&text.as_str()) {
                    continue;
                }
                let Some(local_id) = GrammarMetadata::parse(&text, &name)
                    .ok()
                    .map(|metadata| metadata.id)
                else {
                    sources.push((name, text));
                    continue;
                };

                // Fast path for the common case where the override keeps the
                // bundled file name.
                let base = embedded_text.get(name.as_str()).copied().and_then(|base| {
                    GrammarMetadata::parse(base, &name)
                        .ok()
                        .filter(|metadata| metadata.id.eq_ignore_ascii_case(&local_id))
                        .map(|_| base)
                });
                let base = base.or_else(|| {
                    embedded_by_id
                        .get_or_insert_with(embedded_grammars_by_id)
                        .get(&local_id.to_ascii_lowercase())
                        .copied()
                });
                if let Some(base) = base {
                    if let Some(merged) = merge_grammar(base, &text) {
                        text = merged;
                    }
                }
                sources.push((name, text));
            }
        }

        let mut registry = Registry::default();
        let mut slots: Vec<Option<LanguageEntry>> = Vec::new();
        for (source_name, text) in sources {
            // Indexing only needs the language id, extensions and file names.
            // Parsing the full definition (keywords, regexes, formatters) is
            // deferred until that language is actually used.
            let metadata = match GrammarMetadata::parse(&text, &source_name) {
                Ok(metadata) => metadata,
                Err(metadata_error) => match Language::parse(&text, &source_name) {
                    Ok(language) => {
                        let entry = LanguageEntry {
                            id: language.id.clone(),
                            extensions: language.extensions.clone(),
                            file_names: language.file_names.clone(),
                            source_name,
                            text,
                            language: OnceCell::new(),
                        };
                        entry.language.set(Rc::new(language)).ok();
                        registry.insert_entry(entry, &mut slots);
                        continue;
                    }
                    Err(parse_error) => {
                        registry
                            .errors
                            .push(format!("{source_name}: {parse_error} ({metadata_error})"));
                        continue;
                    }
                },
            };
            let entry = LanguageEntry {
                id: metadata.id,
                extensions: metadata.extensions,
                file_names: metadata.file_names,
                source_name,
                text,
                language: OnceCell::new(),
            };
            registry.insert_entry(entry, &mut slots);
        }

        registry.entries = slots.into_iter().flatten().collect();
        registry.rebuild_indexes();
        registry
    }

    fn insert_entry(&mut self, entry: LanguageEntry, slots: &mut Vec<Option<LanguageEntry>>) {
        let key = entry.id.to_ascii_lowercase();
        if let Some(index) = self.by_id.get(&key).copied() {
            slots[index] = Some(entry);
        } else {
            self.by_id.insert(key, slots.len());
            slots.push(Some(entry));
        }
    }

    fn rebuild_indexes(&mut self) {
        self.by_id.clear();
        self.by_extension.clear();
        self.by_filename.clear();
        self.ordered_extensions.clear();

        for (index, entry) in self.entries.iter().enumerate() {
            self.by_id.insert(entry.id.to_ascii_lowercase(), index);
            for file_name in &entry.file_names {
                let key = file_name.to_ascii_lowercase();
                self.by_filename.entry(key).or_insert(index);
            }
            for extension in &entry.extensions {
                let key = extension.to_ascii_lowercase();
                if self.by_extension.insert(key.clone(), index).is_none() {
                    self.ordered_extensions.push((key, index));
                }
            }
        }

        // Compound extensions such as .blade.php must win over .php.
        self.ordered_extensions
            .sort_by(|left, right| right.0.len().cmp(&left.0.len()));
    }

    fn entry_language(&self, index: usize) -> Option<&Language> {
        self.entries
            .get(index)
            .and_then(LanguageEntry::language)
            .map(Rc::as_ref)
    }

    pub fn find(&self, id: &str) -> Option<&Language> {
        self.by_id
            .get(&id.to_ascii_lowercase())
            .and_then(|index| self.entry_language(*index))
    }

    pub fn find_shared(&self, id: &str) -> Option<Rc<Language>> {
        self.by_id
            .get(&id.to_ascii_lowercase())
            .and_then(|index| self.entries.get(*index))
            .and_then(LanguageEntry::language)
            .cloned()
    }

    pub fn detect(&self, path: &str) -> Option<&Language> {
        self.detect_index(path)
            .and_then(|index| self.entry_language(index))
    }

    pub fn detect_shared(&self, path: &str) -> Option<Rc<Language>> {
        self.detect_index(path)
            .and_then(|index| self.entries.get(index))
            .and_then(LanguageEntry::language)
            .cloned()
    }

    fn detect_index(&self, path: &str) -> Option<usize> {
        if path.is_empty() {
            return None;
        }
        let file_name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_name.is_empty() {
            return None;
        }
        if let Some(index) = self.by_filename.get(&file_name.to_ascii_lowercase()) {
            return Some(*index);
        }

        let lower_name = file_name.to_ascii_lowercase();
        for (extension, index) in &self.ordered_extensions {
            if lower_name.ends_with(extension) {
                return Some(*index);
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn languages(&self) -> Vec<Rc<Language>> {
        self.entries
            .iter()
            .filter_map(LanguageEntry::language)
            .cloned()
            .collect()
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

fn embedded_grammars_by_id() -> HashMap<String, &'static str> {
    let mut result = HashMap::new();
    for (name, text) in embedded::EMBEDDED_GRAMMARS.iter() {
        if let Ok(metadata) = GrammarMetadata::parse(text, name) {
            result
                .entry(metadata.id.to_ascii_lowercase())
                .or_insert(*text);
        }
    }
    result
}

/// Patches a local grammar onto its bundled definition. Objects merge
/// recursively while every other value from the local file replaces the
/// bundled value. The user file itself is never modified.
fn merge_grammar(base: &str, overlay: &str) -> Option<String> {
    match (json::parse(base).ok()?, json::parse(overlay).ok()?) {
        (Value::Object(base), Value::Object(overlay)) => {
            let merged = merge_values(Value::Object(base), Value::Object(overlay));
            Some(json::to_string(&merged, true))
        }
        _ => None,
    }
}

fn merge_values(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(index) = base.iter().position(|(name, _)| name == &key) {
                    let existing = std::mem::replace(&mut base[index].1, Value::Null);
                    base[index].1 = merge_values(existing, value);
                } else {
                    base.push((key, value));
                }
            }
            Value::Object(base)
        }
        (_, overlay) => overlay,
    }
}

fn grammar_directories() -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            result.push(directory.join("data/grammars"));
        }
    }
    result.push(PathBuf::from("data/grammars"));
    if let Some(home) = home_directory() {
        result.push(home.join(".hexal/data/grammars"));
        result.push(home.join(".hexal/grammars"));
    }
    result
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{Registry, Value, json, merge_grammar};

    #[test]
    fn lazy_metadata_matches_the_parsed_definitions() {
        let registry = Registry::load();
        for entry in &registry.entries {
            let language = entry.language().expect("grammar should parse");
            assert_eq!(entry.id, language.id, "{}", entry.source_name);
            assert_eq!(
                entry.extensions, language.extensions,
                "{}",
                entry.source_name
            );
            assert_eq!(
                entry.file_names, language.file_names,
                "{}",
                entry.source_name
            );
        }
    }

    #[test]
    fn loads_bundled_grammars() {
        let registry = Registry::load();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        assert!(registry.languages().len() >= 100);
        assert_eq!(
            registry.find("Rust").map(|language| language.id.as_str()),
            Some("rust")
        );
        assert_eq!(
            registry
                .detect("src/main.rs")
                .map(|language| language.id.as_str()),
            Some("rust")
        );
        assert_eq!(
            registry
                .detect("Cargo.toml")
                .map(|language| language.id.as_str()),
            Some("toml")
        );
        let rust = registry.find("rust").unwrap();
        let debugger = rust.debugger.as_ref().unwrap();
        assert_eq!(debugger.adapter, "lldb-dap");
        assert_eq!(
            debugger
                .configuration
                .get("program")
                .and_then(crate::util::json::Value::as_str),
            Some("${executable}")
        );
    }

    #[test]
    fn merges_local_grammar_patches_over_bundled_definition() {
        let bundled = r#"{
            "id": "csharp",
            "keywords": ["class"],
            "comments": { "line": "//", "blockStart": "/*", "blockEnd": "*/" },
            "formatter": { "command": "csharpier" },
            "debugger": { "adapter": "netcoredbg" }
        }"#;
        let local = r#"{
            "id": "csharp",
            "keywords": ["record"],
            "comments": { "line": "//" },
            "debugger": { "adapter": "custom-debugger" }
        }"#;

        let merged = merge_grammar(bundled, local).unwrap();
        let value = json::parse(&merged).unwrap();

        let keywords = value.get("keywords").and_then(Value::as_array).unwrap();
        assert_eq!(keywords.len(), 1);
        assert_eq!(keywords[0].as_str(), Some("record"));

        assert_eq!(
            value
                .get("comments")
                .and_then(|comments| comments.get("blockStart"))
                .and_then(Value::as_str),
            Some("/*")
        );
        assert_eq!(
            value
                .get("formatter")
                .and_then(|formatter| formatter.get("command"))
                .and_then(Value::as_str),
            Some("csharpier")
        );
        assert_eq!(
            value
                .get("debugger")
                .and_then(|debugger| debugger.get("adapter"))
                .and_then(Value::as_str),
            Some("custom-debugger")
        );
    }

    #[test]
    fn detects_compound_extensions() {
        let registry = Registry::load();
        assert_eq!(
            registry
                .detect("view.blade.php")
                .map(|language| language.id.as_str()),
            Some("blade")
        );
    }
}
