//! File-picker preview state and rendering.
//!
//! The picker reads the leading lines of the highlighted file synchronously,
//! matching the C# editor: reading ten short lines from a file that was just
//! enumerated is cheaper than scheduling work and re-rendering later, and it
//! keeps the preview exactly in step with the selection. Results are cached
//! in a small LRU so scrolling back over files does not touch the disk again.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::languages::highlight::tokenize_line;
use std::fs::File;
use std::io::{BufRead, BufReader};

pub(super) const PREVIEW_LINES: usize = 10;
const PREVIEW_MAX_LINE_LENGTH: usize = 400;
const PREVIEW_MAX_BYTES: u64 = 4 * 1024 * 1024;
const PREVIEW_CACHE_ENTRIES: usize = 128;

#[derive(Clone)]
pub(super) struct FilePreview {
    pub lines: Vec<String>,
    pub binary: bool,
    pub truncated: bool,
    pub error: String,
}

fn read_preview(path: &Path) -> FilePreview {
    let mut preview = FilePreview {
        lines: Vec::new(),
        binary: false,
        truncated: false,
        error: String::new(),
    };

    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            preview.error = error.to_string();
            return preview;
        }
    };
    match file.metadata() {
        Ok(metadata) if metadata.len() > PREVIEW_MAX_BYTES => {
            preview.error = "file is too large to preview".into();
            return preview;
        }
        Ok(_) => {}
        Err(error) => {
            preview.error = error.to_string();
            return preview;
        }
    }

    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    while preview.lines.len() < PREVIEW_LINES {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {
                if bytes.last() == Some(&b'\n') {
                    bytes.pop();
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                }
                if bytes.contains(&0) {
                    preview.binary = true;
                    break;
                }
                let mut line = String::from_utf8_lossy(&bytes).into_owned();
                if line.chars().count() > PREVIEW_MAX_LINE_LENGTH {
                    line = line.chars().take(PREVIEW_MAX_LINE_LENGTH).collect();
                    line.push('…');
                    preview.truncated = true;
                }
                preview.lines.push(line);
            }
            Err(error) => {
                preview.error = error.to_string();
                return preview;
            }
        }
    }

    if !preview.binary && preview.lines.len() >= PREVIEW_LINES {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(read) if read > 0 => preview.truncated = true,
            Ok(_) => {}
            Err(_) => {}
        }
    }

    preview
}

impl Editor {
    pub(super) fn picker_preview_item_path(&self) -> Option<String> {
        if self.picker.kind != PickerKind::Files {
            return None;
        }
        let item = self.picker.items.get(self.picker.selected)?;
        let path = self
            .picker
            .files
            .get(item.file_index)
            .map(|relative| self.picker_root().join(relative))
            .unwrap_or_else(|| PathBuf::from(&item.display));
        Some(path.to_string_lossy().into_owned())
    }

    pub(super) fn update_picker_preview(&mut self) {
        if self.picker.kind != PickerKind::Files {
            return;
        }
        let Some(path) = self.picker_preview_item_path() else {
            self.picker_preview_path.clear();
            self.picker_preview_lines.clear();
            self.picker_preview_status = "No file selected".into();
            self.picker_preview_binary = false;
            return;
        };
        if path == self.picker_preview_path {
            return;
        }

        let preview = if let Some(preview) = self.picker_preview_cache.get(&path) {
            preview.clone()
        } else {
            let preview = read_preview(Path::new(&path));
            self.cache_picker_preview(&path, preview.clone());
            preview
        };
        self.apply_picker_preview(&path, preview);
    }

    fn apply_picker_preview(&mut self, path: &str, preview: FilePreview) {
        self.picker_preview_path = path.to_string();
        self.picker_preview_lines = preview.lines;
        self.picker_preview_binary = preview.binary;
        self.picker_preview_status = if !preview.error.is_empty() {
            format!("E: {}", preview.error)
        } else if preview.binary {
            "Binary file".into()
        } else if self.picker_preview_lines.is_empty() {
            "Empty file".into()
        } else if preview.truncated {
            format!(
                "First {} lines (truncated)",
                self.picker_preview_lines.len()
            )
        } else {
            format!(
                "First {} line{}",
                self.picker_preview_lines.len(),
                if self.picker_preview_lines.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )
        };
    }

    fn cache_picker_preview(&mut self, path: &str, preview: FilePreview) {
        if self.picker_preview_cache.contains_key(path) {
            return;
        }
        while self.picker_preview_cache.len() >= PREVIEW_CACHE_ENTRIES
            && !self.picker_preview_order.is_empty()
        {
            if let Some(oldest) = self.picker_preview_order.pop_front() {
                self.picker_preview_cache.remove(&oldest);
            }
        }
        self.picker_preview_cache.insert(path.to_string(), preview);
        self.picker_preview_order.push_back(path.to_string());
    }

    pub(super) fn picker_preview_height(&self, available: usize) -> usize {
        if self.picker.kind != PickerKind::Files || available < 10 {
            return 0;
        }
        let preview = available.saturating_sub(5).clamp(4, 12);
        if available.saturating_sub(preview).saturating_sub(1) >= 4 {
            preview
        } else {
            0
        }
    }

    pub(super) fn picker_dialog_height(&self, available: usize) -> usize {
        let preview = self.picker_preview_height(available);
        if preview == 0 {
            return self.picker_height();
        }
        available
            .saturating_sub(preview)
            .saturating_sub(1)
            .clamp(4, 18)
    }

    fn expand_preview_line(line: &str) -> String {
        let mut output = String::with_capacity(line.len() + 8);
        let mut column = 0usize;
        for ch in line.chars() {
            if ch == '\t' {
                let spaces = 4 - (column % 4);
                for _ in 0..spaces {
                    output.push(' ');
                }
                column += spaces;
            } else if ch.is_control() {
                output.push(' ');
                column += 1;
            } else {
                output.push(ch);
                column += 1;
            }
        }
        output
    }

    pub(super) fn render_picker_preview(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) {
        let mut relative = self.picker_preview_path.clone();
        if !relative.is_empty() {
            let path = Path::new(&relative);
            if let Ok(stripped) = path.strip_prefix(self.picker_root()) {
                relative = stripped.to_string_lossy().into_owned();
            }
        }
        let mut title = if relative.is_empty() {
            "Preview".to_string()
        } else {
            format!("Preview: {relative}")
        };
        if !self.picker_preview_status.is_empty() {
            title.push_str(" — ");
            title.push_str(&self.picker_preview_status);
        }
        self.draw_box(x, y, width, height, &title);

        let content_rows = height.saturating_sub(2);
        let content_width = width.saturating_sub(4);
        if self.picker_preview_status.starts_with("E: ") {
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1) as i64,
                &self.picker_preview_status.clone(),
                self.theme.error_style(),
                content_width,
            );
            return;
        }
        if self.picker_preview_binary {
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1) as i64,
                "Binary file",
                self.theme.muted_style(),
                content_width,
            );
            return;
        }
        if self.picker_preview_lines.is_empty() {
            let status = if self.picker_preview_status.is_empty() {
                "No preview"
            } else {
                &self.picker_preview_status
            };
            let status = status.to_string();
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1) as i64,
                &status,
                self.theme.muted_style(),
                content_width,
            );
            return;
        }

        let language = if self.config.syntax_highlight_picker_preview {
            self.languages.detect(&self.picker_preview_path).cloned()
        } else {
            None
        };
        let lines = self.picker_preview_lines.clone();
        let mut in_block = false;
        for (row, line) in lines.iter().take(content_rows).enumerate() {
            let line = Self::expand_preview_line(line);
            if let Some(language) = language.as_ref() {
                let mut tokens = Vec::new();
                in_block = tokenize_line(language, line.as_bytes(), in_block, &mut tokens);
                self.render_preview_highlighted_line(
                    x + 2,
                    y + 1 + row,
                    &line,
                    &tokens,
                    content_width,
                );
            } else {
                self.screen.put_str(
                    (x + 2) as i64,
                    (y + 1 + row) as i64,
                    &line,
                    self.theme.text(),
                    content_width,
                );
            }
        }
    }

    fn render_preview_highlighted_line(
        &mut self,
        x: usize,
        y: usize,
        line: &str,
        tokens: &[Token],
        width: usize,
    ) {
        if width == 0 {
            return;
        }
        let mut column = 0usize;
        let mut offset = 0usize;
        let emit = |screen: &mut Screen,
                    column: &mut usize,
                    start: usize,
                    end: usize,
                    style: crate::ui::theme::Style| {
            if end <= start || *column >= width {
                return;
            }
            let Ok(text) = std::str::from_utf8(&line.as_bytes()[start..end]) else {
                return;
            };
            screen.put_str(
                (x + *column) as i64,
                y as i64,
                text,
                style,
                width.saturating_sub(*column),
            );
            *column += util::text_width(text);
        };

        for token in tokens {
            if token.start > offset {
                emit(
                    &mut self.screen,
                    &mut column,
                    offset,
                    token.start,
                    self.theme.text(),
                );
            }
            if token.start >= offset {
                emit(
                    &mut self.screen,
                    &mut column,
                    token.start,
                    token.end,
                    self.theme.syntax_style(token.kind),
                );
                offset = token.end;
            }
        }
        if offset < line.len() {
            emit(
                &mut self.screen,
                &mut column,
                offset,
                line.len(),
                self.theme.text(),
            );
        }
    }
}
