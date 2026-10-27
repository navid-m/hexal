//! Loads and indexes the bundled hexdef grammars plus user overrides from
//! `~/.hexal/data/grammars`.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::definition::Language;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/grammars.rs"));
}

#[derive(Default)]
pub struct Registry {
    languages: Vec<Rc<Language>>,
    by_id: HashMap<String, usize>,
    by_extension: HashMap<String, usize>,
    by_filename: HashMap<String, usize>,
    ordered_extensions: Vec<(String, usize)>,
    errors: Vec<String>,
}

impl Registry {
    pub fn load() -> Registry {
        let mut entries: Vec<(String, String)> = embedded::EMBEDDED_GRAMMARS
            .iter()
            .map(|(name, text)| ((*name).to_string(), (*text).to_string()))
            .collect();
        entries.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });

        // User grammars override a bundled grammar with the same id. This is a
        // superset of the reference editor's directory loading and lets a
        // single portable binary still accept local definitions.
        let embedded_text: HashMap<&str, &str> = embedded::EMBEDDED_GRAMMARS
            .iter()
            .map(|(name, text)| (*name, *text))
            .collect();
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
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if embedded_text.get(name.as_str()) == Some(&text.as_str()) {
                    continue;
                }
                entries.push((name, text));
            }
        }

        let mut registry = Registry::default();
        let mut slots: Vec<Option<Rc<Language>>> = Vec::new();
        for (name, text) in entries {
            match Language::parse(&text, &name) {
                Ok(language) => {
                    let language = Rc::new(language);
                    if let Some(index) = registry.by_id.get(&language.id).copied() {
                        slots[index] = Some(language);
                    } else {
                        registry.by_id.insert(language.id.clone(), slots.len());
                        slots.push(Some(language));
                    }
                }
                Err(error) => registry.errors.push(format!("{name}: {error}")),
            }
        }

        registry.languages = slots.into_iter().flatten().collect();
        registry.rebuild_indexes();
        registry
    }

    fn rebuild_indexes(&mut self) {
        self.by_extension.clear();
        self.by_filename.clear();
        self.ordered_extensions.clear();

        for (index, language) in self.languages.iter().enumerate() {
            for file_name in &language.file_names {
                let key = file_name.to_ascii_lowercase();
                self.by_filename.entry(key).or_insert(index);
            }
            for extension in &language.extensions {
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

    pub fn find(&self, id: &str) -> Option<&Language> {
        self.by_id
            .get(&id.to_ascii_lowercase())
            .and_then(|index| self.languages.get(*index))
            .map(|language| language.as_ref())
    }

    pub fn find_shared(&self, id: &str) -> Option<Rc<Language>> {
        self.by_id
            .get(&id.to_ascii_lowercase())
            .and_then(|index| self.languages.get(*index))
            .cloned()
    }

    pub fn detect(&self, path: &str) -> Option<&Language> {
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
            return self.languages.get(*index).map(|language| language.as_ref());
        }

        let lower_name = file_name.to_ascii_lowercase();
        for (extension, index) in &self.ordered_extensions {
            if lower_name.ends_with(extension) {
                return self.languages.get(*index).map(|language| language.as_ref());
            }
        }
        None
    }

    pub fn detect_shared(&self, path: &str) -> Option<Rc<Language>> {
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
            return self.languages.get(*index).cloned();
        }
        let lower_name = file_name.to_ascii_lowercase();
        for (extension, index) in &self.ordered_extensions {
            if lower_name.ends_with(extension) {
                return self.languages.get(*index).cloned();
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn languages(&self) -> &[Rc<Language>] {
        &self.languages
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
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
    use super::Registry;

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
