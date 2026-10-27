//! Piece-table text buffer with a line-start index and grouped undo/redo.]
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::{self, Write};
use std::path::Path;

const MAX_UNDO_GROUPS: usize = 4096;
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
        }
    }

    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            Eol::Lf => b"\n",
            Eol::Crlf => b"\r\n",
        }
    }
}

#[derive(Clone, Copy)]
enum Source {
    Original,
    Add,
}

#[derive(Clone, Copy)]
struct Piece {
    source: Source,
    start: usize,
    len: usize,
}

struct PieceTable {
    original: Vec<u8>,
    add: Vec<u8>,
    pieces: Vec<Piece>,
    line_starts: Vec<usize>,
    len: usize,
}

impl PieceTable {
    fn new() -> PieceTable {
        PieceTable {
            original: Vec::new(),
            add: Vec::new(),
            pieces: Vec::new(),
            line_starts: vec![0],
            len: 0,
        }
    }

    fn set_original(&mut self, data: Vec<u8>) {
        self.len = data.len();
        if !data.is_empty() {
            self.pieces.push(Piece {
                source: Source::Original,
                start: 0,
                len: data.len(),
            });
        }
        self.original = data;
        self.rebuild_lines();
    }

    fn rebuild_lines(&mut self) {
        let mut starts = Vec::with_capacity(self.line_starts.capacity().max(1));
        starts.push(0);
        let mut offset = 0usize;
        for piece in &self.pieces {
            let bytes = self.bytes(piece.source);
            for &byte in &bytes[piece.start..piece.start + piece.len] {
                offset += 1;
                if byte == b'\n' {
                    starts.push(offset);
                }
            }
        }
        self.line_starts = starts;
    }

    fn bytes(&self, source: Source) -> &[u8] {
        match source {
            Source::Original => &self.original,
            Source::Add => &self.add,
        }
    }

    fn split_at(&mut self, offset: usize) -> usize {
        let mut position = 0usize;
        let mut index = 0usize;
        while index < self.pieces.len() {
            let piece = self.pieces[index];
            let end = position + piece.len;
            if offset == position {
                return index;
            }
            if offset < end {
                let left = offset - position;
                self.pieces[index].len = left;
                self.pieces.insert(
                    index + 1,
                    Piece {
                        source: piece.source,
                        start: piece.start + left,
                        len: piece.len - left,
                    },
                );
                return index + 1;
            }
            if offset == end {
                return index + 1;
            }
            position = end;
            index += 1;
        }
        self.pieces.len()
    }

    fn insert(&mut self, offset: usize, text: &[u8]) {
        if text.is_empty() {
            return;
        }
        let offset = offset.min(self.len);
        let index = self.split_at(offset);
        let start = self.add.len();
        self.add.extend_from_slice(text);
        self.pieces.insert(
            index,
            Piece {
                source: Source::Add,
                start,
                len: text.len(),
            },
        );
        self.len += text.len();
        self.rebuild_lines();
    }

    fn erase(&mut self, offset: usize, length: usize) {
        if length == 0 || offset >= self.len {
            return;
        }
        let actual = length.min(self.len - offset);
        let start = self.split_at(offset);
        let end = self.split_at(offset + actual);
        if start < end {
            self.pieces.drain(start..end);
            self.len -= actual;
            self.rebuild_lines();
        }
    }

    fn replace(&mut self, offset: usize, length: usize, text: &[u8]) {
        let offset = offset.min(self.len);
        let actual = length.min(self.len - offset);
        if actual == 0 && text.is_empty() {
            return;
        }
        let start = self.split_at(offset);
        let end = self.split_at(offset + actual);
        if start < end {
            self.pieces.drain(start..end);
            self.len -= actual;
        }
        if !text.is_empty() {
            let add_start = self.add.len();
            self.add.extend_from_slice(text);
            self.pieces.insert(
                start,
                Piece {
                    source: Source::Add,
                    start: add_start,
                    len: text.len(),
                },
            );
            self.len += text.len();
        }
        // One rebuild for the whole replacement instead of one per erase and
        // insert. Bulk plugin edits use this path.
        self.rebuild_lines();
    }

    fn range(&self, offset: usize, length: usize) -> Vec<u8> {
        let offset = offset.min(self.len);
        let end = (offset + length).min(self.len);
        let mut result = Vec::with_capacity(end.saturating_sub(offset));
        let mut position = 0usize;
        for piece in &self.pieces {
            let piece_end = position + piece.len;
            let from = offset.max(position);
            let to = end.min(piece_end);
            if from < to {
                let bytes = self.bytes(piece.source);
                let local = from - position;
                result.extend_from_slice(
                    &bytes[piece.start + local..piece.start + local + (to - from)],
                );
            }
            if piece_end >= end {
                break;
            }
            position = piece_end;
        }
        result
    }

    fn all_text(&self) -> Vec<u8> {
        self.range(0, self.len)
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        if offset >= self.len {
            return None;
        }
        let mut position = 0usize;
        for piece in &self.pieces {
            if offset < position + piece.len {
                let bytes = self.bytes(piece.source);
                return Some(bytes[piece.start + (offset - position)]);
            }
            position += piece.len;
        }
        None
    }

    fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        for piece in &self.pieces {
            let bytes = self.bytes(piece.source);
            writer.write_all(&bytes[piece.start..piece.start + piece.len])?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    Insert,
    Erase,
    Mixed,
}

struct Op {
    offset: usize,
    removed: Vec<u8>,
    inserted: Vec<u8>,
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

    pub fn size(&self) -> usize {
        self.table.len
    }

    pub fn line_count(&self) -> usize {
        self.table.line_starts.len()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.table.line_starts[line.min(self.line_count() - 1)]
    }

    pub fn line_end(&self, line: usize) -> usize {
        if line + 1 < self.line_count() {
            let mut end = self.table.line_starts[line + 1];
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
        self.table.range(start, end.saturating_sub(start))
    }

    pub fn line_of_offset(&self, offset: usize) -> usize {
        let offset = offset.min(self.size());
        self.table
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1)
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        self.table.byte_at(offset)
    }

    pub fn all_text(&self) -> Vec<u8> {
        self.table.all_text()
    }

    pub fn range(&self, offset: usize, length: usize) -> Vec<u8> {
        self.table.range(offset, length)
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
        group.ops.push(Op {
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
        let removed = self.table.range(offset, actual);
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
        group.ops.push(Op {
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
        let removed = self.table.range(clamped, actual);
        if self.group_open {
            self.close_group();
        }
        self.start_group(GroupKind::Mixed);
        let group = self.undo.last_mut().expect("open group");
        group.ops.push(Op {
            offset: clamped,
            removed,
            inserted: text.to_vec(),
        });
        self.table.replace(clamped, actual, text);
        self.revision += 1;
        self.close_group();
    }

    pub fn break_undo_group(&mut self) {
        self.close_group();
    }

    pub fn undo(&mut self) -> bool {
        let Some(group) = self.undo.pop() else {
            return false;
        };
        self.close_group();
        for op in group.ops.iter().rev() {
            self.table.erase(op.offset, op.inserted.len());
            self.table.insert(op.offset, &op.removed);
        }
        self.redo.push(group);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        self.close_group();
        for op in &group.ops {
            self.table.erase(op.offset, op.removed.len());
            self.table.insert(op.offset, &op.inserted);
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

        let temporary = format!("{}.hexalrs-tmp-{}", target, std::process::id());
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
}
