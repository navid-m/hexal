//! Document model: piece table, line helpers, grouped undo/redo and file I/O.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::core::piece_table::PieceTable;

const MAX_UNDO_GROUPS: usize = 4096;
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

pub use crate::core::piece_table::Eol;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    Insert,
    Erase,
    Mixed,
}

enum Op {
    Edit {
        offset: usize,
        removed: Vec<u8>,
        inserted: Vec<u8>,
    },
    /// A whole-buffer replacement. `stored` is the text that is *not* currently
    /// installed in the piece table, so undo/redo can swap it without copying.
    ReplaceAll { stored: Vec<u8> },
}

struct Group {
    ops: Vec<Op>,
}

pub struct Document {
    table: PieceTable,
    pub path: Option<String>,
    pub eol: Eol,
    pub bom: bool,
    undo: Vec<Group>,
    redo: Vec<Group>,
    group_open: bool,
    group_kind: GroupKind,
    group_last_offset: usize,
    revision: u64,
    saved_revision: u64,
    use_tabs: bool,
    indent_width: usize,
    tab_width: usize,
}

impl Document {
    pub fn new() -> Document {
        Document {
            table: PieceTable::new(),
            path: None,
            eol: Eol::Lf,
            bom: false,
            undo: Vec::new(),
            redo: Vec::new(),
            group_open: false,
            group_kind: GroupKind::Mixed,
            group_last_offset: 0,
            revision: 0,
            saved_revision: 0,
            use_tabs: false,
            indent_width: 4,
            tab_width: 4,
        }
    }

    pub fn open(path: &str) -> io::Result<Document> {
        let raw = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };

        let bom = raw.len() >= 3 && raw[..3] == BOM;
        let data = if bom { raw[3..].to_vec() } else { raw };
        let eol = detect_eol(&data);

        let mut doc = Document::new();
        doc.table.set_original(data);
        doc.path = Some(path.to_string());
        doc.eol = eol;
        doc.bom = bom;
        Ok(doc)
    }

    /// Build a scratch document from owned bytes, for example a sidebar diff.
    pub fn from_bytes(path: Option<String>, mut data: Vec<u8>) -> Document {
        let bom = data.len() >= 3 && data[..3] == BOM;
        if bom {
            data.drain(..3);
        }
        let eol = detect_eol(&data);
        let mut doc = Document::new();
        doc.table.set_original(data);
        doc.path = path;
        doc.eol = eol;
        doc.bom = bom;
        doc.saved_revision = doc.revision;
        doc
    }

    pub fn display_name(&self) -> String {
        match &self.path {
            Some(path) => {
                let name = Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    path.clone()
                } else {
                    name
                }
            }
            None => "[No Name]".to_string(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn use_tabs(&self) -> bool {
        self.use_tabs
    }

    pub fn indent_width(&self) -> usize {
        self.indent_width
    }

    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    pub fn set_indent_spaces(&mut self, width: usize) {
        self.indent_width = width.clamp(1, 16);
        self.use_tabs = false;
    }

    pub fn set_indent_tabs(&mut self, width: usize) {
        let width = width.clamp(1, 16);
        self.tab_width = width;
        self.indent_width = width;
        self.use_tabs = true;
    }

    pub fn set_indentation(&mut self, use_tabs: bool, indent_width: usize, tab_width: usize) {
        self.tab_width = tab_width.clamp(1, 16);
        self.indent_width = indent_width.clamp(1, 16);
        self.use_tabs = use_tabs;
    }

    pub fn indent_unit(&self) -> Vec<u8> {
        if self.use_tabs {
            vec![b'\t']
        } else {
            vec![b' '; self.indent_width]
        }
    }

    /// Number of leading whitespace bytes that form one indentation level.
    pub fn unindent_length(&self, text: &[u8]) -> usize {
        let mut offset = 0usize;
        let mut columns = 0usize;
        while offset < text.len() && columns < self.indent_width {
            match text[offset] {
                b'\t' => {
                    offset += 1;
                    if self.use_tabs {
                        break;
                    }
                    columns += self.tab_width - (columns % self.tab_width);
                }
                b' ' => {
                    offset += 1;
                    columns += 1;
                }
                _ => break,
            }
        }
        offset
    }

    pub fn size(&self) -> usize {
        self.table.size()
    }

    pub fn line_count(&self) -> usize {
        self.table.line_count()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.table.line_start(line)
    }

    pub fn line_end(&self, line: usize) -> usize {
        if line + 1 < self.line_count() {
            let mut end = self.table.line_start(line + 1);
            if end > 0 && self.table.byte_at(end - 1) == Some(b'\n') {
                end -= 1;
                if end > 0 && self.table.byte_at(end - 1) == Some(b'\r') {
                    end -= 1;
                }
            }
            end
        } else {
            self.size()
        }
    }

    pub fn line_text(&self, line: usize) -> Vec<u8> {
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.table.range_alloc(start, end.saturating_sub(start))
    }

    /// Copy a line into `buffer`, avoiding a fresh allocation in the renderer.
    pub fn copy_line(&self, line: usize, buffer: &mut Vec<u8>) {
        let start = self.line_start(line);
        let end = self.line_end(line);
        let length = end.saturating_sub(start);
        buffer.clear();
        buffer.resize(length, 0);
        self.table.copy_range(start, length, buffer);
    }

    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.table.line_of_offset(offset.min(self.size()))
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        self.table.byte_at(offset)
    }

    pub fn all_text(&self) -> Vec<u8> {
        self.table.range_alloc(0, self.size())
    }

    pub fn borrowed_text(&self) -> Option<(*const u8, usize)> {
        self.table.borrowed_view()
    }

    pub fn range(&self, offset: usize, length: usize) -> Vec<u8> {
        self.table.range_alloc(offset, length)
    }

    pub fn insert(&mut self, offset: usize, text: &[u8]) {
        if text.is_empty() {
            return;
        }
        let clamped = offset.min(self.size());
        if !self.group_open
            || self.group_kind != GroupKind::Insert
            || self.group_last_offset != clamped
        {
            if self.group_open {
                self.close_group();
            }
            self.start_group(GroupKind::Insert);
        }
        let group = self.undo.last_mut().expect("open group");
        group.ops.push(Op::Edit {
            offset: clamped,
            removed: Vec::new(),
            inserted: text.to_vec(),
        });
        self.table.insert(clamped, text);
        self.group_last_offset = clamped + text.len();
        self.revision += 1;
    }

    pub fn erase(&mut self, offset: usize, length: usize) {
        if length == 0 || offset >= self.size() {
            return;
        }
        let actual = length.min(self.size() - offset);
        let removed = self.table.range_alloc(offset, actual);
        let continuous = self.group_open
            && self.group_kind == GroupKind::Erase
            && (self.group_last_offset == offset || self.group_last_offset == offset + actual);
        if !continuous {
            if self.group_open {
                self.close_group();
            }
            self.start_group(GroupKind::Erase);
        }
        let group = self.undo.last_mut().expect("open group");
        group.ops.push(Op::Edit {
            offset,
            removed,
            inserted: Vec::new(),
        });
        self.table.erase(offset, actual);
        self.group_last_offset = offset;
        self.revision += 1;
    }

    pub fn replace(&mut self, offset: usize, length: usize, text: &[u8]) {
        let clamped = offset.min(self.size());
        let actual = length.min(self.size() - clamped);
        if clamped == 0 && actual == self.size() {
            self.replace_all_owned(text.to_vec());
            return;
        }

        let removed = self.table.range_alloc(clamped, actual);
        if self.group_open {
            self.close_group();
        }
        self.start_group(GroupKind::Mixed);
        let group = self.undo.last_mut().expect("open group");
        group.ops.push(Op::Edit {
            offset: clamped,
            removed,
            inserted: text.to_vec(),
        });
        self.table.erase(clamped, actual);
        self.table.insert(clamped, text);
        self.revision += 1;
        self.close_group();
    }

    /// Replace the whole buffer while moving the new text into the piece table
    /// instead of copying it through the add buffer. Used by bulk plugin edits.
    pub fn replace_all_owned(&mut self, data: Vec<u8>) {
        if self.group_open {
            self.close_group();
        }
        self.start_group(GroupKind::Mixed);
        let removed = self.table.replace_all_owned(data);
        let group = self.undo.last_mut().expect("open group");
        group.ops.push(Op::ReplaceAll { stored: removed });
        self.revision += 1;
        self.close_group();
    }

    pub fn break_undo_group(&mut self) {
        self.close_group();
    }

    pub fn undo(&mut self) -> bool {
        let Some(mut group) = self.undo.pop() else {
            return false;
        };
        self.close_group();
        for op in group.ops.iter_mut().rev() {
            match op {
                Op::Edit {
                    offset,
                    removed,
                    inserted,
                } => {
                    self.table.erase(*offset, inserted.len());
                    self.table.insert(*offset, removed);
                }
                Op::ReplaceAll { stored } => {
                    let current = self.table.replace_all_owned(std::mem::take(stored));
                    *stored = current;
                }
            }
        }
        self.redo.push(group);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(mut group) = self.redo.pop() else {
            return false;
        };
        self.close_group();
        for op in group.ops.iter_mut() {
            match op {
                Op::Edit {
                    offset,
                    removed,
                    inserted,
                } => {
                    self.table.erase(*offset, removed.len());
                    self.table.insert(*offset, inserted);
                }
                Op::ReplaceAll { stored } => {
                    let current = self.table.replace_all_owned(std::mem::take(stored));
                    *stored = current;
                }
            }
        }
        self.undo.push(group);
        self.revision += 1;
        true
    }

    pub fn save(&mut self, save_path: Option<&str>) -> io::Result<()> {
        let target = save_path
            .map(str::to_string)
            .or_else(|| self.path.clone())
            .filter(|path| !path.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;

        let temporary = format!("{}.hexal-tmp-{}", target, std::process::id());
        let result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            if self.bom {
                file.write_all(&BOM)?;
            }
            self.table.write_to(&mut file)?;
            file.sync_all()?;
            drop(file);
            if let Ok(metadata) = fs::metadata(&target) {
                fs::set_permissions(&temporary, metadata.permissions())?;
            }
            fs::rename(&temporary, &target)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;

        self.path = Some(target);
        self.saved_revision = self.revision;
        self.close_group();
        Ok(())
    }

    fn start_group(&mut self, kind: GroupKind) {
        if self.undo.len() >= MAX_UNDO_GROUPS {
            self.undo.remove(0);
        }
        self.undo.push(Group { ops: Vec::new() });
        self.group_open = true;
        self.group_kind = kind;
        self.group_last_offset = 0;
        self.redo.clear();
    }

    fn close_group(&mut self) {
        self.group_open = false;
        self.group_kind = GroupKind::Mixed;
    }
}

fn detect_eol(data: &[u8]) -> Eol {
    match data.iter().position(|&byte| byte == b'\n') {
        Some(index) if index > 0 && data[index - 1] == b'\r' => Eol::Crlf,
        _ => Eol::Lf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::piece_table::count_newlines;

    #[test]
    fn edits_and_undo() {
        let mut doc = Document::new();
        doc.insert(0, b"hello");
        doc.break_undo_group();
        doc.insert(5, b" world");
        assert_eq!(doc.all_text(), b"hello world");
        assert!(doc.undo());
        assert_eq!(doc.all_text(), b"hello");
        assert!(doc.redo());
        assert_eq!(doc.all_text(), b"hello world");
        doc.replace(6, 5, b"rust");
        assert_eq!(doc.all_text(), b"hello rust");
        assert_eq!(doc.line_count(), 1);
    }

    #[test]
    fn lines() {
        let mut doc = Document::new();
        doc.insert(0, b"one\ntwo\r\nthree");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line_text(1), b"two");
        assert_eq!(doc.line_of_offset(5), 1);
        assert_eq!(doc.line_start(2), 9);
    }

    #[test]
    fn bulk_replace_undo_redo() {
        let mut doc = Document::new();
        doc.insert(0, b"alpha\nbeta");
        doc.replace_all_owned(b"first\nbuffer".to_vec());
        assert_eq!(doc.all_text(), b"first\nbuffer");
        doc.replace_all_owned(b"second\nbuffer".to_vec());
        assert_eq!(doc.all_text(), b"second\nbuffer");
        assert!(doc.undo());
        assert_eq!(doc.all_text(), b"first\nbuffer");
        assert!(doc.undo());
        assert_eq!(doc.all_text(), b"alpha\nbeta");
        assert!(doc.redo());
        assert_eq!(doc.all_text(), b"first\nbuffer");
        assert!(doc.redo());
        assert_eq!(doc.all_text(), b"second\nbuffer");
    }

    #[test]
    fn randomized_edits_match_vec() {
        let mut doc = Document::new();
        let mut reference: Vec<u8> = Vec::new();
        let mut state = 0x1234_5678u32;
        for _ in 0..800 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let offset = if reference.is_empty() {
                0
            } else {
                (state as usize) % (reference.len() + 1)
            };
            let text = b"abc\nxyz";
            let text_length = 1 + (state as usize >> 8) % text.len();
            match state % 3 {
                0 => {
                    doc.insert(offset, &text[..text_length]);
                    reference.splice(offset..offset, text[..text_length].iter().copied());
                }
                1 if !reference.is_empty() => {
                    let offset = offset.min(reference.len() - 1);
                    let length = 1 + (state as usize >> 8) % (reference.len() - offset);
                    doc.erase(offset, length);
                    reference.drain(offset..offset + length);
                }
                _ => {
                    let offset = offset.min(reference.len());
                    let length = (state as usize >> 8) % (reference.len() - offset + 1);
                    doc.replace(offset, length, &text[..text_length]);
                    reference.splice(offset..offset + length, text[..text_length].iter().copied());
                }
            }
            assert_eq!(doc.all_text(), reference);
            let expected_lines = reference.iter().filter(|&&byte| byte == b'\n').count() + 1;
            assert_eq!(doc.line_count(), expected_lines);
            assert_eq!(doc.line_of_offset(reference.len()), expected_lines - 1);
        }
    }

    #[test]
    fn line_index_scales_without_rebuild() {
        let mut doc = Document::new();
        doc.insert(0, &vec![b'x'; 1_000_000]);
        for _ in 0..1000 {
            let offset = doc.size() / 2;
            doc.insert(offset, b"y");
            doc.erase(offset, 1);
        }
        assert_eq!(doc.size(), 1_000_000);
        assert_eq!(count_newlines(&doc.all_text()), 0);
    }
}
