//! Piece table over immutable original text and an append-only add buffer.
//!
//! This is a faithful Rust port of the Zig (and C#) implementation. The piece
//! sequence lives in an implicit treap stored in one array; every node is
//! augmented with the byte and newline counts of its subtree. That gives
//! expected `O(log p)` insertion, deletion, offset-to-line and line-to-offset
//! queries, with pieces split on roughly 8 KiB newline boundaries to bound
//! newline scans.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const MAX_PIECE_SIZE: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
struct Node {
    left: i32,
    right: i32,
    from_add: bool,
    offset: usize,
    length: usize,
    piece_newlines: usize,
    bytes: usize,
    newlines: usize,
    priority: u64,
}

impl Node {
    const EMPTY: Node = Node {
        left: -1,
        right: -1,
        from_add: false,
        offset: 0,
        length: 0,
        piece_newlines: 0,
        bytes: 0,
        newlines: 0,
        priority: 0,
    };
}

pub struct PieceTable {
    original: Vec<u8>,
    /// False only while the table is reading an undo/redo snapshot that it does
    /// not own. Incremental edits materialize a private copy first.
    original_owned: bool,
    added: Vec<u8>,
    nodes: Vec<Node>,
    free_head: i32,
    root: i32,
    random_state: u64,
    /// True while the visible text is exactly `original` (no add-buffer
    /// pieces). Bulk replacements can then move the old Vec into undo instead
    /// of copying it.
    text_is_original: bool,
    pub eol: Eol,
}

impl Default for PieceTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PieceTable {
    pub fn new() -> PieceTable {
        PieceTable {
            original: Vec::new(),
            original_owned: true,
            added: Vec::new(),
            nodes: Vec::new(),
            free_head: -1,
            root: -1,
            random_state: 0x9E37_79B9_7F4A_7C15,
            text_is_original: true,
            eol: Eol::Lf,
        }
    }

    pub fn size(&self) -> usize {
        self.bytes_of(self.root)
    }

    /// A stable pointer to the whole visible text when it is a single owned
    /// original buffer. The Lua host uses this to read plugin snapshots
    /// without copying after a bulk write-back.
    pub fn borrowed_view(&self) -> Option<(*const u8, usize)> {
        if self.text_is_original && self.size() == self.original.len() {
            Some((self.original.as_ptr(), self.original.len()))
        } else {
            None
        }
    }

    pub fn newline_count(&self) -> usize {
        self.newlines_of(self.root)
    }

    pub fn line_count(&self) -> usize {
        self.newline_count() + 1
    }

    /// Replace the entire buffer, taking ownership of `data`.
    pub fn set_original(&mut self, data: Vec<u8>) {
        if self.original_owned && !self.original.is_empty() {
            // `data` is about to replace the old backing store; dropping the
            // Vec frees it.
        }
        self.original = data;
        self.original_owned = true;
        self.text_is_original = true;
        self.added.clear();
        self.nodes.clear();
        self.free_head = -1;
        self.root = -1;
        self.eol = detect_eol(&self.original);
        self.rebuild_all();
    }

    fn rebuild_all(&mut self) {
        let mut position = 0usize;
        while position < self.original.len() {
            let end = choose_end(&self.original, position, self.original.len());
            let node = self.make_node(false, position, end - position);
            self.root = self.merge(self.root, node);
            position = end;
        }
    }

    /// Materialize borrowed text before an incremental edit.
    fn ensure_owned(&mut self) {
        if self.original_owned {
            return;
        }
        let copy = self.original.clone();
        self.original = copy;
        self.original_owned = true;
    }

    /// Replace the complete buffer with `data` without copying it into the add
    /// buffer. Old text is returned so the caller can keep it for undo.
    pub fn replace_all_owned(&mut self, data: Vec<u8>) -> Vec<u8> {
        let old = if self.text_is_original {
            std::mem::take(&mut self.original)
        } else {
            self.all_text()
        };
        self.original = data;
        self.original_owned = true;
        self.text_is_original = true;
        self.added.clear();
        self.nodes.clear();
        self.free_head = -1;
        self.root = -1;
        self.eol = detect_eol(&self.original);
        self.rebuild_all();
        old
    }

    /// Borrow the current text only for an undo/redo snapshot. The caller must
    /// keep it alive while the table uses it.
    pub fn all_text(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(self.size());
        self.append_all(&mut output);
        output
    }

    pub fn insert(&mut self, offset: usize, text: &[u8]) {
        if text.is_empty() {
            return;
        }
        self.ensure_owned();
        self.text_is_original = false;
        let clamped = offset.min(self.size());
        let start = self.added.len();
        self.added.extend_from_slice(text);
        let end = self.added.len();

        let mut inserted_root = -1i32;
        let mut position = start;
        while position < end {
            let piece_end = choose_end(&self.added, position, end);
            let node = self.make_node(true, position, piece_end - position);
            inserted_root = self.merge(inserted_root, node);
            position = piece_end;
        }

        let parts = self.split(self.root, clamped);
        let left = self.merge(parts.0, inserted_root);
        self.root = self.merge(left, parts.1);
    }

    pub fn erase(&mut self, offset: usize, length: usize) {
        if length == 0 || offset >= self.size() {
            return;
        }
        self.ensure_owned();
        self.text_is_original = false;
        let actual = length.min(self.size() - offset);
        let first = self.split(self.root, offset);
        let second = self.split(first.1, actual);
        self.free_tree(second.0);
        self.root = self.merge(first.0, second.1);
    }

    pub fn copy_range(&self, offset: usize, length: usize, destination: &mut [u8]) -> usize {
        if length == 0 || offset >= self.size() {
            return 0;
        }
        let actual = length.min(self.size() - offset);
        let mut written = 0usize;
        self.collect_range(
            self.root,
            0,
            offset,
            offset + actual,
            &mut destination[..actual],
            &mut written,
        );
        written
    }

    pub fn range_alloc(&self, offset: usize, length: usize) -> Vec<u8> {
        let actual = if offset >= self.size() {
            0
        } else {
            length.min(self.size() - offset)
        };
        let mut result = vec![0u8; actual];
        self.copy_range(offset, actual, &mut result);
        result
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        if offset >= self.size() {
            return None;
        }
        let mut base = 0usize;
        let mut node_index = self.root;
        while node_index >= 0 {
            let node = self.nodes[node_index as usize];
            let left_bytes = self.bytes_of(node.left);
            let node_start = base + left_bytes;
            if offset < node_start {
                node_index = node.left;
                continue;
            }
            if offset < node_start + node.length {
                return Some(self.source_slice(node)[offset - node_start]);
            }
            base = node_start + node.length;
            node_index = node.right;
        }
        None
    }

    /// Byte offset of the first character of a zero-based line.
    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 || self.root < 0 {
            return 0;
        }

        let mut remaining = line;
        let mut base = 0usize;
        let mut node_index = self.root;
        while node_index >= 0 {
            let node = self.nodes[node_index as usize];
            let left_newlines = self.newlines_of(node.left);
            if remaining <= left_newlines {
                node_index = node.left;
                continue;
            }
            remaining -= left_newlines;

            if remaining <= node.piece_newlines {
                let index = self.newline_offset_in_piece(node, remaining);
                return base + self.bytes_of(node.left) + index + 1;
            }
            remaining -= node.piece_newlines;
            base += self.bytes_of(node.left) + node.length;
            node_index = node.right;
        }
        self.size()
    }

    /// Zero-based line containing a byte offset.
    pub fn line_of_offset(&self, offset: usize) -> usize {
        if offset >= self.size() {
            return self.newline_count();
        }

        let mut line = 0usize;
        let mut base = 0usize;
        let mut node_index = self.root;
        while node_index >= 0 {
            let node = self.nodes[node_index as usize];
            let left_bytes = self.bytes_of(node.left);
            let node_start = base + left_bytes;
            if offset < node_start {
                node_index = node.left;
                continue;
            }
            if offset < node_start + node.length {
                let source = self.source_slice(node);
                let local = offset - node_start;
                return line + self.newlines_of(node.left) + count_newlines(&source[..local]);
            }
            line += self.newlines_of(node.left) + node.piece_newlines;
            base = node_start + node.length;
            node_index = node.right;
        }
        line
    }

    /// Append the complete text to `output`. Used by the diff/Lua helpers.
    pub fn append_all(&self, output: &mut Vec<u8>) {
        self.append_node(self.root, output);
    }

    fn append_node(&self, node_index: i32, output: &mut Vec<u8>) {
        if node_index < 0 {
            return;
        }
        let node = self.nodes[node_index as usize];
        self.append_node(node.left, output);
        output.extend_from_slice(self.source_slice(node));
        self.append_node(node.right, output);
    }

    pub fn write_to(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        self.write_node(self.root, writer)
    }

    fn write_node(&self, node_index: i32, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        if node_index < 0 {
            return Ok(());
        }
        let node = self.nodes[node_index as usize];
        self.write_node(node.left, writer)?;
        writer.write_all(self.source_slice(node))?;
        self.write_node(node.right, writer)
    }

    fn source_slice(&self, node: Node) -> &[u8] {
        let source = if node.from_add {
            &self.added
        } else {
            &self.original
        };
        &source[node.offset..node.offset + node.length]
    }

    fn bytes_of(&self, node_index: i32) -> usize {
        if node_index < 0 {
            0
        } else {
            self.nodes[node_index as usize].bytes
        }
    }

    fn newlines_of(&self, node_index: i32) -> usize {
        if node_index < 0 {
            0
        } else {
            self.nodes[node_index as usize].newlines
        }
    }

    fn make_node(&mut self, from_add: bool, offset: usize, length: usize) -> i32 {
        let source = if from_add {
            &self.added
        } else {
            &self.original
        };
        let newlines = count_newlines(&source[offset..offset + length]);
        let index = self.alloc_node();
        self.nodes[index as usize] = Node {
            left: -1,
            right: -1,
            from_add,
            offset,
            length,
            piece_newlines: newlines,
            bytes: 0,
            newlines: 0,
            priority: self.next_priority(),
        };
        self.update(index);
        index
    }

    fn alloc_node(&mut self) -> i32 {
        if self.free_head >= 0 {
            let index = self.free_head;
            self.free_head = self.nodes[index as usize].left;
            self.nodes[index as usize] = Node::EMPTY;
            index
        } else {
            let index = self.nodes.len() as i32;
            self.nodes.push(Node::EMPTY);
            index
        }
    }

    fn free_node(&mut self, index: i32) {
        self.nodes[index as usize].left = self.free_head;
        self.free_head = index;
    }

    fn free_tree(&mut self, root: i32) {
        if root < 0 {
            return;
        }
        let node = self.nodes[root as usize];
        self.free_tree(node.left);
        self.free_tree(node.right);
        self.free_node(root);
    }

    fn update(&mut self, index: i32) {
        let node = self.nodes[index as usize];
        let bytes = self.bytes_of(node.left) + node.length + self.bytes_of(node.right);
        let newlines =
            self.newlines_of(node.left) + node.piece_newlines + self.newlines_of(node.right);
        self.nodes[index as usize].bytes = bytes;
        self.nodes[index as usize].newlines = newlines;
    }

    fn merge(&mut self, left: i32, right: i32) -> i32 {
        if left < 0 {
            return right;
        }
        if right < 0 {
            return left;
        }

        if self.nodes[left as usize].priority >= self.nodes[right as usize].priority {
            let old_right = self.nodes[left as usize].right;
            let merged = self.merge(old_right, right);
            self.nodes[left as usize].right = merged;
            self.update(left);
            left
        } else {
            let old_left = self.nodes[right as usize].left;
            let merged = self.merge(left, old_left);
            self.nodes[right as usize].left = merged;
            self.update(right);
            right
        }
    }

    fn split(&mut self, tree: i32, offset: usize) -> (i32, i32) {
        if tree < 0 {
            return (-1, -1);
        }

        let node = self.nodes[tree as usize];
        let left_bytes = self.bytes_of(node.left);
        let node_length = node.length;

        if offset < left_bytes {
            let old_left = node.left;
            let parts = self.split(old_left, offset);
            self.nodes[tree as usize].left = parts.1;
            self.update(tree);
            return (parts.0, tree);
        }

        if offset > left_bytes + node_length {
            let old_right = node.right;
            let parts = self.split(old_right, offset - left_bytes - node_length);
            self.nodes[tree as usize].right = parts.0;
            self.update(tree);
            return (tree, parts.1);
        }

        if offset == left_bytes {
            let old_left = node.left;
            self.nodes[tree as usize].left = -1;
            self.update(tree);
            return (old_left, tree);
        }

        if offset == left_bytes + node_length {
            let old_right = node.right;
            self.nodes[tree as usize].right = -1;
            self.update(tree);
            return (tree, old_right);
        }

        // The split point is inside the piece. Both halves keep the original
        // priority so the heap invariant relative to existing children holds;
        // no text is copied.
        let local_offset = offset - left_bytes;
        let source = self.source_slice(node);
        let left_newlines = count_newlines(&source[..local_offset]);

        let left_half = self.alloc_node();
        self.nodes[left_half as usize] = Node {
            left: node.left,
            right: -1,
            from_add: node.from_add,
            offset: node.offset,
            length: local_offset,
            piece_newlines: left_newlines,
            bytes: 0,
            newlines: 0,
            priority: node.priority,
        };
        self.update(left_half);

        let right_half = self.alloc_node();
        self.nodes[right_half as usize] = Node {
            left: -1,
            right: node.right,
            from_add: node.from_add,
            offset: node.offset + local_offset,
            length: node.length - local_offset,
            piece_newlines: node.piece_newlines - left_newlines,
            bytes: 0,
            newlines: 0,
            priority: node.priority,
        };
        self.update(right_half);

        self.free_node(tree);
        (left_half, right_half)
    }

    fn collect_range(
        &self,
        node_index: i32,
        base: usize,
        begin: usize,
        end: usize,
        destination: &mut [u8],
        written: &mut usize,
    ) {
        if node_index < 0 || begin >= end {
            return;
        }
        let node = self.nodes[node_index as usize];
        let left_bytes = self.bytes_of(node.left);
        let node_start = base + left_bytes;
        let node_end = node_start + node.length;

        if begin < node_start {
            self.collect_range(
                node.left,
                base,
                begin,
                end.min(node_start),
                destination,
                written,
            );
        }
        if begin < node_end && end > node_start {
            let from = begin.max(node_start) - node_start;
            let to = end.min(node_end) - node_start;
            let source = self.source_slice(node);
            destination[*written..*written + (to - from)].copy_from_slice(&source[from..to]);
            *written += to - from;
        }
        if end > node_end {
            self.collect_range(
                node.right,
                node_end,
                begin.max(node_end),
                end,
                destination,
                written,
            );
        }
    }

    fn newline_offset_in_piece(&self, node: Node, nth: usize) -> usize {
        let source = self.source_slice(node);
        let mut seen = 0usize;
        for (i, &byte) in source.iter().enumerate() {
            if byte == b'\n' {
                seen += 1;
                if seen == nth {
                    return i;
                }
            }
        }
        node.length
    }

    fn next_priority(&mut self) -> u64 {
        self.random_state = self.random_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.random_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn choose_end(source: &[u8], start: usize, limit: usize) -> usize {
    if start >= limit {
        return limit;
    }
    let max_end = limit.min(start + MAX_PIECE_SIZE);
    if max_end < limit {
        if let Some(relative) = source[start..max_end]
            .iter()
            .rposition(|&byte| byte == b'\n')
        {
            return start + relative + 1;
        }
    }
    max_end
}

pub fn count_newlines(source: &[u8]) -> usize {
    let mut count = 0usize;
    let mut position = 0usize;
    while position < source.len() {
        let found = unsafe {
            libc::memchr(
                source[position..].as_ptr() as *const std::os::raw::c_void,
                b'\n' as std::os::raw::c_int,
                source.len() - position,
            )
        };
        if found.is_null() {
            break;
        }
        count += 1;
        position = found as usize - source.as_ptr() as usize + 1;
    }
    count
}

fn detect_eol(source: &[u8]) -> Eol {
    match source.iter().position(|&byte| byte == b'\n') {
        Some(index) if index > 0 && source[index - 1] == b'\r' => Eol::Crlf,
        _ => Eol::Lf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_deletion_and_line_indexing() {
        let mut table = PieceTable::new();
        table.set_original(b"one\ntwo\nthree".to_vec());
        assert_eq!(table.size(), 13);
        assert_eq!(table.line_count(), 3);
        assert_eq!(table.line_start(1), 4);
        assert_eq!(table.line_start(2), 8);
        assert_eq!(table.line_of_offset(6), 1);

        table.insert(3, b"!");
        assert_eq!(table.range_alloc(0, table.size()), b"one!\ntwo\nthree");

        table.erase(4, 4);
        assert_eq!(table.range_alloc(0, table.size()), b"one!\nthree");
    }

    #[test]
    fn large_randomized_consistency() {
        let mut table = PieceTable::new();
        table.set_original(b"abcdef\nghijkl\n".to_vec());

        table.insert(7, b"XYZ");
        table.erase(2, 3);
        assert_eq!(table.range_alloc(0, table.size()), b"abf\nXYZghijkl\n");

        let mut reference: Vec<u8> = b"abf\nXYZghijkl\n".to_vec();
        let mut state = 0x1234_5678u32;
        for _ in 0..2000 {
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
                    table.insert(offset, &text[..text_length]);
                    reference.splice(offset..offset, text[..text_length].iter().copied());
                }
                1 if !reference.is_empty() => {
                    let offset = offset.min(reference.len() - 1);
                    let length = 1 + (state as usize >> 8) % (reference.len() - offset);
                    table.erase(offset, length);
                    reference.drain(offset..offset + length);
                }
                _ => {
                    let offset = offset.min(reference.len());
                    let length = (state as usize >> 8) % (reference.len() - offset + 1);
                    table.erase(offset, length);
                    table.insert(offset, &text[..text_length]);
                    reference.splice(offset..offset + length, text[..text_length].iter().copied());
                }
            }
            assert_eq!(table.range_alloc(0, table.size()), reference);
            let expected_lines = reference.iter().filter(|&&byte| byte == b'\n').count() + 1;
            assert_eq!(table.line_count(), expected_lines);
            assert_eq!(table.line_of_offset(reference.len()), expected_lines - 1);
        }
    }

    #[test]
    fn many_line_queries_match_scan() {
        let mut table = PieceTable::new();
        let mut text = Vec::new();
        for line in 0..1000 {
            text.extend_from_slice(format!("line {line} has some text\n").as_bytes());
        }
        text.extend_from_slice(b"tail");
        table.set_original(text.clone());

        let mut starts = vec![0usize];
        for (index, &byte) in text.iter().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        assert_eq!(table.line_count(), starts.len());
        for (line, &start) in starts.iter().enumerate() {
            assert_eq!(table.line_start(line), start);
            assert_eq!(table.line_of_offset(start), line);
        }
    }
}
