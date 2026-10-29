//! Workspace file enumeration and literal cross-file search.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::Read;
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

fn search_worker_count(file_count: usize) -> usize {
    if file_count < 64 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(2, 8)
        .min(file_count)
}

pub fn search(
    root: &Path,
    files: &[String],
    query: &str,
    max_hits: usize,
    match_case: bool,
    cancel: &AtomicBool,
    max_file_bytes: u64,
) -> Vec<SearchHit> {
    if query.is_empty() || max_hits == 0 || files.is_empty() {
        return Vec::new();
    }

    let workers = search_worker_count(files.len());
    if workers <= 1 {
        return search_range(
            root,
            files,
            query,
            max_hits,
            match_case,
            cancel,
            max_file_bytes,
            0,
            files.len(),
        );
    }

    let chunk = files.len().div_ceil(workers);
    let mut hits = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..workers {
            let start = worker * chunk;
            if start >= files.len() {
                break;
            }
            let end = (start + chunk).min(files.len());
            handles.push(scope.spawn(move || {
                search_range(
                    root,
                    files,
                    query,
                    max_hits,
                    match_case,
                    cancel,
                    max_file_bytes,
                    start,
                    end,
                )
            }));
        }
        let mut hits = Vec::new();
        for handle in handles {
            hits.extend(handle.join().unwrap_or_default());
        }
        hits
    });

    hits.sort_by(|left, right| {
        (left.file_index, left.line, left.column).cmp(&(right.file_index, right.line, right.column))
    });
    hits.truncate(max_hits);
    hits
}

#[allow(clippy::too_many_arguments)]
fn search_range(
    root: &Path,
    files: &[String],
    query: &str,
    max_hits: usize,
    match_case: bool,
    cancel: &AtomicBool,
    max_file_bytes: u64,
    start: usize,
    end: usize,
) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let needle = query.as_bytes();

    for (file_index, path) in files.iter().enumerate().take(end).skip(start) {
        if hits.len() >= max_hits || cancel.load(Ordering::Relaxed) {
            break;
        }
        let full_path = root.join(path);
        let Ok(metadata) = fs::metadata(&full_path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > max_file_bytes {
            continue;
        }
        let mut file = match fs::File::open(&full_path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let mut prefix = [0u8; 8192];
        let prefix_len = match file.read(&mut prefix) {
            Ok(length) => length,
            Err(_) => continue,
        };
        if prefix[..prefix_len].contains(&0) {
            continue;
        }
        let mut data = Vec::with_capacity(metadata.len() as usize);
        data.extend_from_slice(&prefix[..prefix_len]);
        let remaining = max_file_bytes.saturating_sub(prefix_len as u64);
        if file.take(remaining).read_to_end(&mut data).is_err() {
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
    fn search_skips_binary_and_oversized_files() {
        let root =
            std::env::temp_dir().join(format!("hexal-search-limits-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("text.txt"), b"before needle after\n").unwrap();
        fs::write(root.join("binary.bin"), b"needle\0binary\n").unwrap();
        let oversized = fs::File::create(root.join("oversized.txt")).unwrap();
        oversized.set_len(4 * 1024 * 1024 + 1).unwrap();
        drop(oversized);

        let hits = search(
            &root,
            &[
                "text.txt".to_string(),
                "binary.bin".to_string(),
                "oversized.txt".to_string(),
            ],
            "needle",
            10,
            true,
            &AtomicBool::new(false),
            4 * 1024 * 1024,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_index, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_respects_the_configured_size_limit() {
        let root =
            std::env::temp_dir().join(format!("hexal-search-size-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("text.txt"), b"needle\n").unwrap();

        let files = ["text.txt".to_string()];
        let skipped = search(
            &root,
            &files,
            "needle",
            10,
            true,
            &AtomicBool::new(false),
            3,
        );
        assert!(skipped.is_empty());

        let searched = search(
            &root,
            &files,
            "needle",
            10,
            true,
            &AtomicBool::new(false),
            100,
        );
        assert_eq!(searched.len(), 1);

        let _ = fs::remove_dir_all(root);
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
            4 * 1024 * 1024,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 0);

        let _ = fs::remove_dir_all(root);
    }
}
