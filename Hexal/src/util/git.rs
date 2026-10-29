//! Gitignore matching and workspace Git status for the sidebar.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_GITIGNORE_BYTES: u64 = 1024 * 1024;

const BUILT_IN_IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vs",
    ".vscode",
    "bin",
    "obj",
    "build",
    "dist",
    "target",
    "node_modules",
    "vendor",
];

pub fn is_built_in_ignored_directory(name: &str) -> bool {
    BUILT_IN_IGNORED_DIRECTORIES
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(name))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatternKind {
    Literal,
    /// `*suffix`
    Suffix,
    /// `prefix*`
    Prefix,
    /// `prefix*suffix` with no other wildcards
    Affix,
    Glob,
}

fn classify_pattern(pattern: &str) -> PatternKind {
    if !pattern.is_ascii() {
        return PatternKind::Glob;
    }
    let bytes = pattern.as_bytes();
    let mut star = None;
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'*' => {
                if star.is_some() {
                    return PatternKind::Glob;
                }
                star = Some(index);
            }
            b'?' | b'[' | b'\\' => return PatternKind::Glob,
            _ => {}
        }
    }
    match star {
        None => PatternKind::Literal,
        Some(0) => PatternKind::Suffix,
        Some(index) if index + 1 == bytes.len() => PatternKind::Prefix,
        Some(_) => PatternKind::Affix,
    }
}

#[derive(Clone)]
struct Rule {
    /// `base` relative to the ignore root, precomputed so matching a path does
    /// not rebuild strings for every rule.
    base_relative: String,
    pattern: String,
    /// True when `pattern` is ASCII and has no character classes, allowing
    /// the allocation-free byte matcher.
    simple_glob: bool,
    /// Fast-path classification for the common single-`*` shapes.
    kind: PatternKind,
    anchored: bool,
    directory_only: bool,
    negated: bool,
}

pub struct GitIgnore {
    root: PathBuf,
    rules: Vec<Rule>,
}

impl GitIgnore {
    pub fn new(root: &Path) -> GitIgnore {
        let root = absolute(root);
        let mut ignore = GitIgnore {
            root: root.clone(),
            rules: Vec::new(),
        };
        ignore.load_file(&root.join(".git/info/exclude"), &root);
        ignore.load_file(&root.join(".gitignore"), &root);
        ignore
    }

    /// Add a nested `.gitignore` while walking a directory. Rules from deeper
    /// files are appended so they override their parents.
    pub fn with_directory(&self, directory: &Path) -> GitIgnore {
        let mut copy = GitIgnore {
            root: self.root.clone(),
            rules: self.rules.clone(),
        };
        copy.load_file(&directory.join(".gitignore"), directory);
        copy
    }

    fn load_file(&mut self, path: &Path, base: &Path) {
        let Ok(metadata) = fs::metadata(path) else {
            return;
        };
        if metadata.len() > MAX_GITIGNORE_BYTES {
            return;
        }
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        for line in text.lines() {
            if let Some(rule) = parse_rule(line, base, &self.root) {
                self.rules.push(rule);
            }
        }
    }

    pub fn is_ignored(&self, path: &Path, is_directory: bool) -> bool {
        let full = absolute(path);
        if full == self.root {
            return false;
        }
        let Ok(relative) = relative_slash_path(&self.root, &full) else {
            return false;
        };
        self.is_ignored_relative(&relative, is_directory)
    }

    /// Same as [`GitIgnore::is_ignored`] but takes a path relative to the
    /// ignore root, avoiding a `getcwd` call and path allocations per entry.
    pub fn is_ignored_relative(&self, relative: &str, is_directory: bool) -> bool {
        if relative.is_empty() || relative == "." {
            return false;
        }

        let mut ignored = false;
        for rule in &self.rules {
            let base_relative = if rule.base_relative.is_empty() {
                relative
            } else {
                // The candidate must live below the directory that owns the
                // rule; anything else cannot match it.
                let base = rule.base_relative.as_str();
                if relative.len() <= base.len() || !relative.starts_with(base) {
                    continue;
                }
                if relative.as_bytes()[base.len()] != b'/' {
                    continue;
                }
                &relative[base.len() + 1..]
            };
            if base_relative.is_empty() {
                continue;
            }
            let matched = if rule.anchored {
                matches_anchored(rule, base_relative, is_directory, rule.directory_only)
            } else {
                matches_component(rule, base_relative, is_directory, rule.directory_only)
            };
            if matched {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

fn parse_rule(line: &str, base: &Path, root: &Path) -> Option<Rule> {
    let mut line = line.trim_end_matches('\r').to_string();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // Unescaped trailing spaces are not part of the pattern.
    while line.ends_with(' ') && !line.ends_with("\\ ") {
        line.pop();
    }
    if line.is_empty() {
        return None;
    }
    let negated = line.starts_with('!');
    if negated {
        line.remove(0);
    }
    if line.is_empty() {
        return None;
    }
    let directory_only = line.ends_with('/');
    if directory_only {
        line.pop();
    }
    let anchored = line.starts_with('/');
    if anchored {
        line.remove(0);
    }
    // A slash anywhere except the trailing one anchors the rule to its base.
    let anchored = anchored || line.contains('/');
    let base = absolute(base);
    let base_relative = relative_slash_path(root, &base).unwrap_or_default();
    let pattern = line.replace('\\', "/");
    let simple_glob = pattern.is_ascii() && !pattern.as_bytes().contains(&b'[');
    let kind = classify_pattern(&pattern);
    Some(Rule {
        base_relative,
        pattern,
        simple_glob,
        kind,
        anchored,
        directory_only,
        negated,
    })
}

fn rule_matches_glob(rule: &Rule, text: &str) -> bool {
    let pattern = rule.pattern.as_bytes();
    match rule.kind {
        PatternKind::Literal => text == rule.pattern,
        PatternKind::Suffix => {
            let suffix = &pattern[1..];
            text.len() >= suffix.len()
                && text.as_bytes().ends_with(suffix)
                && !text.as_bytes()[..text.len() - suffix.len()].contains(&b'/')
        }
        PatternKind::Prefix => {
            let prefix = &pattern[..pattern.len() - 1];
            text.len() >= prefix.len()
                && text.as_bytes().starts_with(prefix)
                && !text.as_bytes()[prefix.len()..].contains(&b'/')
        }
        PatternKind::Affix => {
            let star = pattern.iter().position(|&byte| byte == b'*').unwrap_or(0);
            let prefix = &pattern[..star];
            let suffix = &pattern[star + 1..];
            text.len() >= prefix.len() + suffix.len()
                && text.as_bytes().starts_with(prefix)
                && text.as_bytes().ends_with(suffix)
                && !text.as_bytes()[prefix.len()..text.len() - suffix.len()].contains(&b'/')
        }
        PatternKind::Glob => {
            if rule.simple_glob && text.is_ascii() {
                glob_ascii(pattern, text.as_bytes())
            } else {
                glob_match(&rule.pattern, text)
            }
        }
    }
}

fn matches_anchored(rule: &Rule, relative: &str, is_directory: bool, directory_only: bool) -> bool {
    if directory_only && !is_directory {
        return false;
    }
    rule_matches_glob(rule, relative)
        || relative
            .match_indices('/')
            .any(|(index, _)| rule_matches_glob(rule, &relative[..index]))
}

fn matches_component(
    rule: &Rule,
    relative: &str,
    is_directory: bool,
    directory_only: bool,
) -> bool {
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        let component_is_directory = is_directory || components.peek().is_some();
        if directory_only && !component_is_directory {
            continue;
        }
        if rule_matches_glob(rule, component) {
            return true;
        }
    }
    false
}

/// Glob matcher used by `.gitignore`. Supports `*`, `?`, `**` and character
/// classes. `*` never matches `/`; `**` does.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern.is_ascii() && text.is_ascii() && !pattern.as_bytes().contains(&b'[') {
        return glob_ascii(pattern.as_bytes(), text.as_bytes());
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob_inner(&pattern, &text)
}

fn glob_ascii(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            if pattern.get(1) == Some(&b'*') {
                let rest = if pattern.get(2) == Some(&b'/') {
                    &pattern[3..]
                } else {
                    &pattern[2..]
                };
                if rest.is_empty() {
                    return true;
                }
                for index in 0..=text.len() {
                    if glob_ascii(rest, &text[index..]) {
                        return true;
                    }
                }
                return false;
            }
            for index in 0..=text.len() {
                if glob_ascii(&pattern[1..], &text[index..]) {
                    return true;
                }
                if index < text.len() && text[index] == b'/' {
                    break;
                }
            }
            false
        }
        b'?' => !text.is_empty() && text[0] != b'/' && glob_ascii(&pattern[1..], &text[1..]),
        b'\\' => {
            if pattern.len() > 1 {
                !text.is_empty() && text[0] == pattern[1] && glob_ascii(&pattern[2..], &text[1..])
            } else {
                !text.is_empty() && text[0] == b'\\' && glob_ascii(&pattern[1..], &text[1..])
            }
        }
        value => !text.is_empty() && text[0] == value && glob_ascii(&pattern[1..], &text[1..]),
    }
}

fn glob_inner(pattern: &[char], text: &[char]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        '*' => {
            if pattern.get(1) == Some(&'*') {
                let rest = if pattern.get(2) == Some(&'/') {
                    &pattern[3..]
                } else {
                    &pattern[2..]
                };
                if rest.is_empty() {
                    return true;
                }
                for index in 0..=text.len() {
                    if glob_inner(rest, &text[index..]) {
                        return true;
                    }
                }
                return false;
            }
            for index in 0..=text.len() {
                if glob_inner(&pattern[1..], &text[index..]) {
                    return true;
                }
                if index < text.len() && text[index] == '/' {
                    break;
                }
            }
            false
        }
        '?' => !text.is_empty() && text[0] != '/' && glob_inner(&pattern[1..], &text[1..]),
        '[' => {
            if text.is_empty() {
                return false;
            }
            let mut index = 1;
            let negate = pattern.get(index) == Some(&'!') || pattern.get(index) == Some(&'^');
            if negate {
                index += 1;
            }
            let mut matched = false;
            let mut previous = '\0';
            while index < pattern.len() && pattern[index] != ']' {
                let current = pattern[index];
                if current == '-' && previous != '\0' && index + 1 < pattern.len() {
                    let end = pattern[index + 1];
                    if text[0] >= previous && text[0] <= end {
                        matched = true;
                    }
                    index += 2;
                    previous = '\0';
                    continue;
                }
                if current == text[0] {
                    matched = true;
                }
                previous = current;
                index += 1;
            }
            if index >= pattern.len() {
                return false;
            }
            if matched != negate {
                glob_inner(&pattern[index + 1..], &text[1..])
            } else {
                false
            }
        }
        '\\' => {
            if pattern.len() > 1 {
                !text.is_empty() && text[0] == pattern[1] && glob_inner(&pattern[2..], &text[1..])
            } else {
                !text.is_empty() && text[0] == '\\' && glob_inner(&pattern[1..], &text[1..])
            }
        }
        value => !text.is_empty() && text[0] == value && glob_inner(&pattern[1..], &text[1..]),
    }
}

#[derive(Default)]
pub struct GitStatus {
    pub modified_files: HashSet<PathBuf>,
    pub modified_directories: HashSet<PathBuf>,
    pub untracked_files: HashSet<PathBuf>,
    pub untracked_directories: HashSet<PathBuf>,
}

pub fn load_git_status(root: &Path) -> GitStatus {
    let mut result = GitStatus::default();
    let root = absolute(root);
    if !has_git_directory(&root) {
        return result;
    }
    let Some(repository_root) = git_output(&root, &["rev-parse", "--show-toplevel"]) else {
        return result;
    };
    let repository_root = absolute(Path::new(repository_root.trim()));
    let Some(status) = git_output(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) else {
        return result;
    };

    let bytes = status.as_bytes();
    let mut position = 0usize;
    while position + 3 <= bytes.len() {
        let index_status = bytes[position] as char;
        let worktree_status = bytes[position + 1] as char;
        position += 3;
        let Some(end) = bytes[position..].iter().position(|&byte| byte == 0) else {
            break;
        };
        let path_text = String::from_utf8_lossy(&bytes[position..position + end]).into_owned();
        position += end + 1;
        if matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C') {
            if let Some(end) = bytes[position..].iter().position(|&byte| byte == 0) {
                position += end + 1;
            } else {
                break;
            }
        }

        let untracked = index_status == '?' && worktree_status == '?';
        let modified = matches!(index_status, 'M' | 'T' | 'R' | 'C')
            || matches!(worktree_status, 'M' | 'T' | 'R' | 'C');
        if !untracked && !modified {
            continue;
        }
        let full =
            absolute(&repository_root.join(path_text.replace('/', std::path::MAIN_SEPARATOR_STR)));
        if !is_inside(&root, &full) && full != root {
            continue;
        }
        if untracked {
            result.untracked_files.insert(full.clone());
        }
        if modified {
            result.modified_files.insert(full.clone());
        }
        add_parent_directories(&root, &full, &mut result.modified_directories);
    }
    result
        .modified_directories
        .retain(|path| !result.untracked_directories.contains(path));
    for file in &result.untracked_files {
        add_parent_directories(&root, file, &mut result.untracked_directories);
    }
    result
}

fn add_parent_directories(root: &Path, path: &Path, output: &mut HashSet<PathBuf>) {
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == root || !is_inside(root, directory) {
            break;
        }
        output.insert(directory.to_path_buf());
        parent = directory.parent();
    }
}

fn has_git_directory(path: &Path) -> bool {
    let mut current = Some(path);
    while let Some(directory) = current {
        if directory.join(".git").exists() {
            return true;
        }
        current = directory.parent();
    }
    false
}

fn git_output(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn project_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidate = absolute(&cwd);
    loop {
        if candidate.join(".git").exists() {
            return candidate;
        }
        let Some(parent) = candidate.parent() else {
            break;
        };
        if parent == candidate {
            break;
        }
        candidate = parent.to_path_buf();
    }
    absolute(&cwd)
}

pub fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    lexical_normalize(&joined)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

pub fn is_inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

pub fn relative_slash_path(base: &Path, path: &Path) -> Result<String, ()> {
    let relative = path.strip_prefix(base).map_err(|_| ())?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    absolute(left) == absolute(right)
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn display_relative(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(relative) => {
            let text = relative.to_string_lossy().replace('\\', "/");
            if text.is_empty() { "./".into() } else { text }
        }
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitDiffLineKind {
    Deleted,
    Added,
    Modified,
}

/// Reads zero-context Git diff hunks for a file and maps them onto the
/// current buffer's zero-based line numbers. Uses HEAD so staged and unstaged
/// changes are both included, then falls back to the index for files without
/// HEAD history.
pub fn load_diff_lines(path: &Path, line_count: usize) -> BTreeMap<usize, GitDiffLineKind> {
    let mut result = BTreeMap::new();
    if line_count == 0 {
        return result;
    }
    let full = absolute(path);
    let Some(directory) = full.parent() else {
        return result;
    };
    let Some(file_name) = full.file_name() else {
        return result;
    };
    let file_name = file_name.to_string_lossy().into_owned();
    let output =
        run_diff(directory, &file_name, true).or_else(|| run_diff(directory, &file_name, false));
    if let Some(output) = output {
        parse_diff_lines(&output, line_count, &mut result);
    }
    result
}

fn run_diff(directory: &Path, file_name: &str, use_head: bool) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(directory).args([
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--unified=0",
    ]);
    if use_head {
        command.arg("HEAD");
    }
    command
        .arg("--")
        .arg(format!(":(literal){file_name}"))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_diff_lines(
    output: &str,
    line_count: usize,
    result: &mut BTreeMap<usize, GitDiffLineKind>,
) {
    for line in output.lines() {
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some(plus) = rest.find(" +") else {
            continue;
        };
        let old = &rest[1..plus];
        let new = rest[plus + 2..]
            .split([' ', '@'])
            .next()
            .unwrap_or_default();
        let (_, old_count) = parse_range(old);
        let (new_start, new_count) = parse_range(new);
        if new_count == 0 {
            let target = new_start
                .saturating_sub(1)
                .min(line_count.saturating_sub(1));
            result.insert(target, GitDiffLineKind::Deleted);
            continue;
        }
        let kind = if old_count == 0 {
            GitDiffLineKind::Added
        } else {
            GitDiffLineKind::Modified
        };
        for offset in 0..new_count {
            let target = new_start.saturating_sub(1) + offset;
            if target < line_count {
                result.insert(target, kind);
            }
        }
    }
}

fn parse_range(value: &str) -> (usize, usize) {
    if let Some((start, count)) = value.split_once(',') {
        (start.parse().unwrap_or(0), count.parse().unwrap_or(1))
    } else {
        (value.parse().unwrap_or(0), 1)
    }
}

/// Write bytes atomically, preserving the previous permission bits.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!(
        "{}.hexal-tmp-{}",
        path.extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    fs::write(&temporary, data)?;
    if let Ok(metadata) = fs::metadata(path) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

/// Create a unique copy path such as `name copy.rs`, `name copy 2.rs`.
pub fn unique_copy_path(source: &Path, is_directory: bool) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = if is_directory {
        file_name(source)
    } else {
        source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_name(source))
    };
    let extension = if is_directory {
        String::new()
    } else {
        source
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default()
    };
    for copy in 1..10_000 {
        let name = if copy == 1 {
            format!("{stem} copy{extension}")
        } else {
            format!("{stem} copy {copy}{extension}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} copy{extension}"))
}

pub fn copy_path(source: &Path, target: &Path, is_directory: bool) -> Result<(), String> {
    if is_directory {
        if same_path(source, target) || is_inside(source, target) {
            return Err("Cannot copy a folder inside itself.".into());
        }
        if target.exists() {
            return Err(format!(
                "Destination already exists: '{}'",
                target.display()
            ));
        }
        copy_directory(source, target)
    } else {
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                return Err(format!(
                    "Destination folder does not exist: '{}'",
                    parent.display()
                ));
            }
        }
        if target.exists() {
            return Err(format!(
                "Destination already exists: '{}'",
                target.display()
            ));
        }
        fs::copy(source, target)
            .map(|_| ())
            .map_err(|error| format!("Copy failed: {error}"))
    }
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir(target).map_err(|error| format!("Copy failed: {error}"))?;
    let result = (|| -> Result<(), String> {
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let child_target = target.join(entry.file_name());
            if file_type.is_dir() {
                copy_directory(&entry.path(), &child_target)?;
            } else {
                fs::copy(entry.path(), child_target).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    result
}

pub fn valid_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    !name.contains('/') && !name.contains('\\') && !name.contains('\0')
}

pub fn trim_utf8_bom(data: &[u8]) -> &[u8] {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &data[3..]
    } else {
        data
    }
}

pub fn has_utf8_bom(data: &[u8]) -> bool {
    data.starts_with(&[0xEF, 0xBB, 0xBF])
}

/// Locate an executable named by a formatter, language server or debug
/// adapter. Bare names are searched for in `PATH`; relative paths are resolved
/// against the editor's working directory. Symbolic links must resolve to a
/// real file so a dangling link cannot be mistaken for a usable command.
pub fn command_path(command: &str) -> Option<PathBuf> {
    if command.trim().is_empty() {
        return None;
    }
    if command.contains('/') || command.contains('\\') {
        let path = Path::new(command);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        return is_usable_executable(&candidate).then_some(candidate);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter_map(|directory| {
            let directory = if directory.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                directory
            };
            let candidate = directory.join(command);
            if candidate.is_absolute() {
                Some(candidate)
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(candidate))
            }
        })
        .find(|candidate| is_usable_executable(candidate))
}

pub fn command_exists(command: &str) -> bool {
    command_path(command).is_some()
}

fn is_usable_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
pub fn line_text(data: &[u8], line: usize) -> Option<String> {
    let mut current = 0;
    let mut start = 0;
    for (index, &byte) in data.iter().enumerate() {
        if byte == b'\n' {
            if current == line {
                let end = if index > start && data[index - 1] == b'\r' {
                    index - 1
                } else {
                    index
                };
                return Some(String::from_utf8_lossy(&data[start..end]).into_owned());
            }
            current += 1;
            start = index + 1;
        }
    }
    if current == line {
        return Some(String::from_utf8_lossy(&data[start..]).into_owned());
    }
    None
}

#[allow(dead_code)]
pub fn preview_string(data: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(&data[..data.len().min(limit)]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zero_context_diff_hunks() {
        let output = "@@ -1,2 +1,3 @@\n@@ -10 +11,0 @@\n@@ -20,0 +21,2 @@\n";
        let mut lines = BTreeMap::new();
        parse_diff_lines(output, 30, &mut lines);
        assert_eq!(lines.get(&0), Some(&GitDiffLineKind::Modified));
        assert_eq!(lines.get(&1), Some(&GitDiffLineKind::Modified));
        assert_eq!(lines.get(&2), Some(&GitDiffLineKind::Modified));
        assert_eq!(lines.get(&10), Some(&GitDiffLineKind::Deleted));
        assert_eq!(lines.get(&20), Some(&GitDiffLineKind::Added));
        assert_eq!(lines.get(&21), Some(&GitDiffLineKind::Added));
    }

    #[test]
    fn loads_line_markers_from_a_git_repository() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("hexal-git-diff-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let run = |arguments: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(arguments)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "Hexal Test"]);
        run(&["config", "user.email", "hexal@example.invalid"]);
        let file = root.join("sample.txt");
        fs::write(&file, b"one\ntwo\nthree\n").unwrap();
        run(&["add", "sample.txt"]);
        run(&["commit", "-qm", "initial"]);
        fs::write(&file, b"one\nchanged\nthree\n").unwrap();

        let lines = load_diff_lines(&file, 3);
        assert_eq!(lines.get(&1), Some(&GitDiffLineKind::Modified));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn glob_patterns() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("src/**", "src/a/b.rs"));
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file10.txt"));
    }

    #[test]
    fn rule_matching() {
        let root = PathBuf::from("/tmp/project");
        let mut ignore = GitIgnore {
            root: root.clone(),
            rules: Vec::new(),
        };
        ignore
            .rules
            .push(parse_rule("*.log", &root, &root).expect("rule"));
        assert!(ignore.is_ignored(Path::new("/tmp/project/a.log"), false));
        assert!(ignore.is_ignored(Path::new("/tmp/project/sub/a.log"), false));
        assert!(!ignore.is_ignored(Path::new("/tmp/project/a.rs"), false));
    }
}
