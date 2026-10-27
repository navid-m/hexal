//! Workspace file enumeration and literal cross-file search.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::path::Path;

const IGNORED: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "build",
    "dist",
    "zig-cache",
    ".zig-cache",
    "zig-out",
    "vendor",
    ".cache",
    ".idea",
    ".vscode",
    "out",
    "cmake-build-debug",
    "cmake-build-release",
];

pub struct SearchHit {
    pub file_index: usize,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

pub fn scan(max_files: usize) -> Vec<String> {
    let mut files = Vec::new();
    walk(Path::new("."), "", &mut files, max_files, 0);
    files.sort();
    files
}

fn walk(_root: &Path, relative: &str, files: &mut Vec<String>, max_files: usize, depth: usize) {
    if files.len() >= max_files || depth > 24 {
        return;
    }
    let directory = if relative.is_empty() { "." } else { relative };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if files.len() >= max_files {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if IGNORED.contains(&name.as_str()) || name.starts_with(".zig-cache") {
            continue;
        }
        let child = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            walk(Path::new(&child), &child, files, max_files, depth + 1);
        } else if file_type.is_file() {
            files.push(child);
        }
    }
}

pub fn search(files: &[String], query: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return hits;
    }
    let needle = query.as_bytes();

    for (file_index, path) in files.iter().enumerate() {
        if hits.len() >= max_hits {
            break;
        }
        let data = match fs::read(path) {
            Ok(data) if data.len() <= 4 * 1024 * 1024 => data,
            _ => continue,
        };
        if data.iter().take(8192).any(|&byte| byte == 0) {
            continue;
        }

        let mut line = 0usize;
        let mut line_start = 0usize;
        while line_start <= data.len() {
            let relative_newline = data[line_start..].iter().position(|&byte| byte == b'\n');
            let line_end = relative_newline
                .map(|n| line_start + n)
                .unwrap_or(data.len());
            let line_text = &data[line_start..line_end];

            if let Some(column) = crate::util::index_of_ignore_case(line_text, needle) {
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
