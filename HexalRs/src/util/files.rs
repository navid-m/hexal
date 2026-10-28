//! Workspace file enumeration and literal cross-file search.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::util::git::{self, GitIgnore};

pub struct SearchHit {
    pub file_index: usize,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

/// Enumerates project files with the default policy of hiding ignored paths.
#[allow(dead_code)]
pub fn scan(max_files: usize) -> Vec<String> {
    scan_with_ignored(max_files, false)
}

/// Enumerates project files, matching the C# editor's `FileSearch`: ignored
/// files and build/VCS directories are skipped unless `show_ignored` is set.
/// The flag is the state of `:gi` / `showGitIgnored` from `.hexcfg`.
pub fn scan_with_ignored(max_files: usize, show_ignored: bool) -> Vec<String> {
    scan_root_with_ignored(Path::new("."), max_files, show_ignored)
}

/// Enumerates files below an explicit root and returns paths relative to it.
pub fn scan_root_with_ignored(root: &Path, max_files: usize, show_ignored: bool) -> Vec<String> {
    scan_root_with_cancel(root, max_files, show_ignored, None)
}

/// Cancellable variant used by the asynchronous picker file scan.
pub fn scan_root_with_cancel(
    root: &Path,
    max_files: usize,
    show_ignored: bool,
    cancel: Option<&AtomicBool>,
) -> Vec<String> {
    let root = git::absolute(root);
    let mut files = Vec::new();
    let ignore = if show_ignored {
        None
    } else {
        Some(GitIgnore::new(&root))
    };
    walk(
        &root,
        &root,
        "",
        &mut files,
        max_files,
        show_ignored,
        0,
        ignore.as_ref(),
        cancel,
    );
    files.sort();
    files
}

#[allow(clippy::too_many_arguments)]
fn walk(
    root: &Path,
    directory: &Path,
    relative: &str,
    files: &mut Vec<String>,
    max_files: usize,
    show_ignored: bool,
    depth: usize,
    parent_ignore: Option<&GitIgnore>,
    cancel: Option<&AtomicBool>,
) {
    if files.len() >= max_files
        || depth > 24
        || cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    {
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // Nested .gitignore files apply to the directory that contains them and
    // everything below it, so each directory gets its own accumulated rules.
    let ignore = if show_ignored || relative.is_empty() {
        None
    } else {
        Some(match parent_ignore {
            Some(parent) => parent.with_directory(directory),
            None => GitIgnore::new(directory),
        })
    };
    let ignore = ignore.as_ref().or(parent_ignore);

    for entry in entries.flatten() {
        if files.len() >= max_files || cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_relative = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        let child_path = directory.join(&name);
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            if !show_ignored {
                if git::is_built_in_ignored_directory(&name) {
                    continue;
                }
                if ignore.is_some_and(|ignore| ignore.is_ignored(&child_path, true)) {
                    continue;
                }
            }
            walk(
                root,
                &child_path,
                &child_relative,
                files,
                max_files,
                show_ignored,
                depth + 1,
                ignore,
                cancel,
            );
        } else if file_type.is_file() {
            if !show_ignored && ignore.is_some_and(|ignore| ignore.is_ignored(&child_path, false)) {
                continue;
            }
            files.push(child_relative);
        }
    }
}

pub fn search(
    root: &Path,
    files: &[String],
    query: &str,
    max_hits: usize,
    match_case: bool,
    cancel: &AtomicBool,
) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    let needle = query.as_bytes();

    for (file_index, path) in files.iter().enumerate() {
        if hits.len() >= max_hits || cancel.load(Ordering::Relaxed) {
            break;
        }
        let data = match fs::read(root.join(path)) {
            Ok(data) if data.len() <= 4 * 1024 * 1024 => data,
            _ => continue,
        };
        if data.iter().take(8192).any(|&byte| byte == 0) {
            continue;
        }

        let mut line = 0usize;
        let mut line_start = 0usize;
        while line_start <= data.len() {
            if line & 0x3F == 0 && cancel.load(Ordering::Relaxed) {
                return hits;
            }
            let relative_newline = data[line_start..].iter().position(|&byte| byte == b'\n');
            let line_end = relative_newline
                .map(|n| line_start + n)
                .unwrap_or(data.len());
            let line_text = &data[line_start..line_end];

            if let Some(column) = crate::util::text::index_of(line_text, needle, match_case) {
                let mut preview_end = line_text.len();
                while preview_end > 0 && matches!(line_text[preview_end - 1], b' ' | b'\t' | b'\r')
                {
                    preview_end -= 1;
                }
                preview_end = preview_end.min(180);
                hits.push(SearchHit {
                    file_index,
                    line,
                    column,
                    preview: String::from_utf8_lossy(&line_text[..preview_end]).into_owned(),
                });
                if hits.len() >= max_hits {
                    break;
                }
            }

            let Some(newline) = relative_newline else {
                break;
            };
            line_start = line_start + newline + 1;
            line += 1;
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::{scan_root_with_ignored, search};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn ignores_gitignored_and_builtin_files_unless_requested() {
        let root = std::env::temp_dir().join(format!("hexal-files-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join(".gitignore"), b"secret.txt\nnested/*.log\n").unwrap();
        fs::write(root.join("visible.txt"), b"visible\n").unwrap();
        fs::write(root.join("secret.txt"), b"secret\n").unwrap();
        fs::write(root.join("nested/hidden.log"), b"hidden\n").unwrap();
        fs::write(root.join("target/debug/app"), b"binary\n").unwrap();

        let hidden = scan_root_with_ignored(&root, 100, false);
        assert!(hidden.contains(&"visible.txt".to_string()));
        assert!(!hidden.contains(&"secret.txt".to_string()));
        assert!(!hidden.contains(&"nested/hidden.log".to_string()));
        assert!(!hidden.contains(&"target/debug/app".to_string()));

        let shown = scan_root_with_ignored(&root, 100, true);
        assert!(shown.contains(&"visible.txt".to_string()));
        assert!(shown.contains(&"secret.txt".to_string()));
        assert!(shown.contains(&"nested/hidden.log".to_string()));
        assert!(shown.contains(&"target/debug/app".to_string()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn search_reads_files_relative_to_the_workspace_root() {
        let root =
            std::env::temp_dir().join(format!("hexal-search-root-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/file.txt"), b"before needle after\n").unwrap();

        let hits = search(
            &root,
            &["sub/file.txt".to_string()],
            "needle",
            10,
            true,
            &AtomicBool::new(false),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 0);

        let _ = fs::remove_dir_all(root);
    }
}
