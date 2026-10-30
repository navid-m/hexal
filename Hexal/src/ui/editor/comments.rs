//! Grammar-aware comment toggling and comment removal.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::BTreeSet;

use super::*;

impl Editor {
    pub(super) fn toggle_comment(&mut self) {
        let language = {
            if let Some(id) = &self.language_override {
                self.languages.find(id)
            } else {
                self.doc
                    .path
                    .as_deref()
                    .and_then(|path| self.languages.detect(path))
            }
        };
        let Some(language) = language else {
            self.set_error("No comment syntax is available for this language.");
            return;
        };
        let line_comments: Vec<String> = language.line_comments().map(str::to_string).collect();
        let block_start = language.block_comment_start.clone();
        let block_end = language.block_comment_end.clone();
        if line_comments.is_empty()
            && (block_start.as_deref().unwrap_or("").is_empty()
                || block_end.as_deref().unwrap_or("").is_empty())
        {
            self.set_error("No comment syntax is available for this language.");
            return;
        }

        let selecting = self.mode == Mode::Select;
        let mut touched = BTreeSet::new();
        for selection in self.capture_all_selections() {
            self.add_touched_lines(&mut touched, selection, selecting);
        }
        if touched.is_empty() {
            return;
        }

        self.doc.break_undo_group();
        if !line_comments.is_empty() {
            self.toggle_line_comments(&touched, &line_comments);
        } else {
            self.toggle_block_comments(
                &touched,
                block_start.as_deref().unwrap_or(""),
                block_end.as_deref().unwrap_or(""),
            );
        }
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn add_touched_lines(
        &self,
        lines: &mut BTreeSet<usize>,
        selection: Selection,
        selecting: bool,
    ) {
        if !selecting || selection.anchor == selection.cursor {
            lines.insert(self.doc.line_of_offset(selection.cursor));
            return;
        }

        let start = selection.anchor.min(selection.cursor);
        let end = selection.anchor.max(selection.cursor);
        let (first, last) = if selection.linewise {
            (
                self.doc
                    .line_of_offset(selection.anchor)
                    .min(self.doc.line_of_offset(selection.cursor)),
                self.doc
                    .line_of_offset(selection.anchor)
                    .max(self.doc.line_of_offset(selection.cursor)),
            )
        } else {
            (
                self.doc.line_of_offset(start),
                if end > start {
                    self.doc.line_of_offset(end - 1)
                } else {
                    self.doc.line_of_offset(start)
                },
            )
        };
        for line in first..=last {
            lines.insert(line);
        }
    }

    fn toggle_line_comments(&mut self, lines: &BTreeSet<usize>, markers: &[String]) {
        let Some(primary) = markers.iter().find(|marker| !marker.is_empty()) else {
            return;
        };
        let primary = primary.as_bytes();
        let mut all_commented = true;
        for line in lines {
            let text = self.doc.line_text(*line);
            let indent = indent_length(&text);
            if matching_line_marker(&text[indent..], markers).is_none() {
                all_commented = false;
                break;
            }
        }

        let mut changes = Vec::with_capacity(lines.len());
        for line in lines {
            let line_start = self.doc.line_start(*line);
            let text = self.doc.line_text(*line);
            let indent = indent_length(&text);
            if all_commented {
                let Some(marker) = matching_line_marker(&text[indent..], markers) else {
                    continue;
                };
                let mut remove = marker.len();
                let after = indent + remove;
                if after < text.len() && text[after] == b' ' {
                    remove += 1;
                }
                changes.push((line_start + indent, remove, Vec::new()));
            } else if matching_line_marker(&text[indent..], markers).is_none() {
                let mut insert = primary.to_vec();
                if indent < text.len() {
                    insert.push(b' ');
                }
                changes.push((line_start + indent, 0, insert));
            }
        }
        self.apply_comment_changes(changes);
    }

    fn toggle_block_comments(&mut self, lines: &BTreeSet<usize>, start: &str, end: &str) {
        let start_bytes = start.as_bytes();
        let end_bytes = end.as_bytes();
        let mut all_blocked = true;
        for line in lines {
            let text = self.doc.line_text(*line);
            let indent = indent_length(&text);
            let content_end = trim_trailing_whitespace(&text, indent);
            if content_end <= indent
                || !starts_with(&text[indent..content_end], start_bytes)
                || !ends_with(&text[indent..content_end], end_bytes)
            {
                all_blocked = false;
                break;
            }
        }

        let mut changes = Vec::new();
        for line in lines {
            let line_start = self.doc.line_start(*line);
            let text = self.doc.line_text(*line);
            let indent = indent_length(&text);
            let content_end = trim_trailing_whitespace(&text, indent);

            if all_blocked {
                let mut remove_start = start_bytes.len();
                let after_start = indent + remove_start;
                if after_start < content_end && text[after_start] == b' ' {
                    remove_start += 1;
                }
                changes.push((line_start + indent, remove_start, Vec::new()));

                let mut end_offset = line_start + content_end - end_bytes.len();
                let mut remove_end = end_bytes.len();
                if end_offset > line_start + indent + remove_start
                    && text[content_end - end_bytes.len() - 1] == b' '
                {
                    end_offset -= 1;
                    remove_end += 1;
                }
                changes.push((end_offset, remove_end, Vec::new()));
            } else if content_end <= indent {
                let insert = format!("{start} {end}");
                changes.push((line_start + indent, 0, insert.into_bytes()));
            } else {
                changes.push((line_start + indent, 0, format!("{start} ").into_bytes()));
                changes.push((line_start + content_end, 0, format!(" {end}").into_bytes()));
            }
        }
        self.apply_comment_changes(changes);
    }

    fn apply_comment_changes(&mut self, mut changes: Vec<(usize, usize, Vec<u8>)>) {
        if changes.is_empty() {
            return;
        }
        changes.sort_by(|left, right| right.0.cmp(&left.0));
        let mut selections = self.capture_all_selections();
        self.doc.begin_composite_edit();
        for (offset, remove, insert) in changes {
            if remove == 0 && insert.is_empty() {
                continue;
            }
            for selection in &mut selections {
                selection.cursor = super::multicursor::adjust_offset(
                    selection.cursor,
                    offset,
                    remove,
                    insert.len(),
                );
                selection.anchor = super::multicursor::adjust_offset(
                    selection.anchor,
                    offset,
                    remove,
                    insert.len(),
                );
            }
            self.doc.replace(offset, remove, &insert);
        }
        self.doc.end_composite_edit();
        self.set_selections(selections);
        self.snap_cursor();
        self.doc.break_undo_group();
    }

    pub(super) fn remove_comments_from_selection(&mut self) {
        if self.mode != Mode::Select {
            self.set_error("Select text before removing comments.");
            return;
        }
        let language = {
            if let Some(id) = &self.language_override {
                self.languages.find(id)
            } else {
                self.doc
                    .path
                    .as_deref()
                    .and_then(|path| self.languages.detect(path))
            }
        };
        let Some(language) = language else {
            self.set_error("No language is active for this buffer.");
            return;
        };
        let line_comments: Vec<String> = language.line_comments().map(str::to_string).collect();
        let block_start = language.block_comment_start.clone().unwrap_or_default();
        let block_end = language.block_comment_end.clone().unwrap_or_default();
        if line_comments.is_empty() && (block_start.is_empty() || block_end.is_empty()) {
            self.set_error(&format!(
                "No comment syntax is configured for {}.",
                language.name
            ));
            return;
        }

        let (start, end) = self.selection_range();
        if end <= start {
            self.set_error("Select text before removing comments.");
            return;
        }
        let selection = self.doc.range(start, end - start);
        let line_start = self.doc.line_start(self.doc.line_of_offset(start));
        let (replacement, removed) = strip_comments(
            &selection,
            start,
            line_start,
            &line_comments,
            &block_start,
            &block_end,
            &language.quote_characters,
            &language.escape_character,
        );
        if !removed {
            self.set_message("No comments found in the selection.");
            return;
        }

        self.doc.break_undo_group();
        self.doc.replace(start, end - start, &replacement);
        self.anchor = start;
        self.cursor = start + replacement.len();
        self.selection_linewise = false;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
        self.set_message("Removed comments from the selection.");
    }
}

fn indent_length(text: &[u8]) -> usize {
    let mut offset = 0usize;
    while offset < text.len() && matches!(text[offset], b' ' | b'\t') {
        offset += 1;
    }
    offset
}

fn trim_trailing_whitespace(text: &[u8], minimum: usize) -> usize {
    let mut end = text.len();
    while end > minimum && matches!(text[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    end
}

fn starts_with(text: &[u8], prefix: &[u8]) -> bool {
    !prefix.is_empty() && text.len() >= prefix.len() && &text[..prefix.len()] == prefix
}

/// Returns the longest configured marker at the start of `text`, so that an
/// overlapping marker such as `//` is preferred over `/` when both are valid.
fn matching_line_marker<'a>(text: &[u8], markers: &'a [String]) -> Option<&'a str> {
    markers
        .iter()
        .filter(|marker| starts_with(text, marker.as_bytes()))
        .max_by_key(|marker| marker.len())
        .map(String::as_str)
}

fn ends_with(text: &[u8], suffix: &[u8]) -> bool {
    !suffix.is_empty() && text.len() >= suffix.len() && &text[text.len() - suffix.len()..] == suffix
}

#[allow(clippy::too_many_arguments)]
fn strip_comments(
    text: &[u8],
    selection_start: usize,
    document_line_start: usize,
    line_comments: &[String],
    block_start: &str,
    block_end: &str,
    quote_characters: &[Vec<u8>],
    escape: &[u8],
) -> (Vec<u8>, bool) {
    let mut line_markers: Vec<&[u8]> = line_comments
        .iter()
        .map(String::as_bytes)
        .filter(|marker| !marker.is_empty())
        .collect();
    line_markers.sort_by(|left, right| right.len().cmp(&left.len()));
    let block_start_bytes = block_start.as_bytes();
    let block_end_bytes = block_end.as_bytes();
    let mut quote_tokens: Vec<&[u8]> = quote_characters
        .iter()
        .map(Vec::as_slice)
        .filter(|quote| !quote.is_empty())
        .collect();
    quote_tokens.sort_by(|left, right| right.len().cmp(&left.len()));

    let mut output = Vec::with_capacity(text.len());
    let mut index = 0usize;
    let mut line_start = 0usize;
    let mut line_output_start = 0usize;
    let mut line_fully_selected = selection_start == document_line_start;
    let mut removed = false;

    while index < text.len() {
        let mut quote_token: Option<&[u8]> = None;
        for candidate in &quote_tokens {
            if starts_with(&text[index..], candidate) {
                quote_token = Some(candidate);
                break;
            }
        }

        if let Some(quote_token) = quote_token {
            let quote_start = index;
            index += quote_token.len();
            while index < text.len() {
                if !escape.is_empty() && starts_with(&text[index..], escape) {
                    index += escape.len();
                    if index < text.len() {
                        index = crate::util::next_boundary(text, index);
                    }
                    continue;
                }
                if starts_with(&text[index..], quote_token) {
                    index += quote_token.len();
                    break;
                }
                index = crate::util::next_boundary(text, index);
            }
            for offset in quote_start..index {
                let value = text[offset];
                output.push(value);
                if value == b'\n' {
                    line_start = offset + 1;
                    line_output_start = output.len();
                    line_fully_selected = true;
                }
            }
            continue;
        }

        if !block_start_bytes.is_empty()
            && !block_end_bytes.is_empty()
            && starts_with(&text[index..], block_start_bytes)
        {
            removed = true;
            let mut comment_end = text.len();
            let mut search = index + block_start_bytes.len();
            while search < text.len() {
                if starts_with(&text[search..], block_end_bytes) {
                    comment_end = search + block_end_bytes.len();
                    break;
                }
                search = crate::util::next_boundary(text, search);
            }

            let line_end = find_line_end(text, comment_end);
            let content_end = if line_end > 0 && text[line_end - 1] == b'\r' {
                line_end - 1
            } else {
                line_end
            };
            let only_whitespace_before = text[line_start..index]
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t'));
            let only_whitespace_after = text[comment_end..content_end]
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t'));

            if line_fully_selected && only_whitespace_before && only_whitespace_after {
                output.truncate(line_output_start);
                if line_end < text.len() {
                    index = line_end + 1;
                    line_start = index;
                    line_output_start = output.len();
                    line_fully_selected = true;
                } else {
                    index = text.len();
                }
                continue;
            }

            if only_whitespace_after {
                while output.len() > line_output_start
                    && matches!(output.last(), Some(b' ' | b'\t'))
                {
                    output.pop();
                }
                index = content_end;
                continue;
            }

            let mut last_newline = None;
            for offset in index..comment_end {
                if text[offset] == b'\n' {
                    last_newline = Some(offset);
                }
            }
            index = comment_end;
            if let Some(newline) = last_newline {
                line_start = newline + 1;
                line_output_start = output.len();
                line_fully_selected = true;
            }
            continue;
        }

        if line_markers
            .iter()
            .any(|marker| starts_with(&text[index..], marker))
        {
            removed = true;
            let line_end = find_line_end(text, index);
            let content_end = if line_end > index && text[line_end - 1] == b'\r' {
                line_end - 1
            } else {
                line_end
            };
            let only_whitespace_before = text[line_start..index]
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t'));
            if line_fully_selected && only_whitespace_before {
                output.truncate(line_output_start);
                if line_end < text.len() {
                    index = line_end + 1;
                    line_start = index;
                    line_output_start = output.len();
                    line_fully_selected = true;
                } else {
                    index = text.len();
                }
                continue;
            }

            while output.len() > line_output_start && matches!(output.last(), Some(b' ' | b'\t')) {
                output.pop();
            }
            index = content_end;
            continue;
        }

        let next = text[index];
        output.push(next);
        index += 1;
        if next == b'\n' {
            line_start = index;
            line_output_start = output.len();
            line_fully_selected = true;
        }
    }

    (output, removed)
}

fn find_line_end(text: &[u8], offset: usize) -> usize {
    text[offset..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|index| offset + index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(path: &str, text: &[u8]) -> Editor {
        Editor::new(
            Document::from_bytes(Some(path.to_string()), text.to_vec()),
            80,
            24,
        )
    }

    #[test]
    fn toggles_line_comments() {
        let mut editor = editor("test.rs", b"    let x = 1;\n");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"    // let x = 1;\n");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"    let x = 1;\n");
    }

    #[test]
    fn toggles_selected_lines() {
        let mut editor = editor("test.rs", b"one\ntwo\nthree\n");
        editor.mode = Mode::Select;
        editor.selection_linewise = true;
        editor.anchor = 0;
        editor.cursor = editor.doc.line_start(2);
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"// one\n// two\n// three\n");
    }

    #[test]
    fn removes_line_and_block_comments() {
        let mut editor = editor(
            "test.rs",
            b"// whole line\nlet x = 1; // inline\n/* block */\nlet y = 2; /* trailing */\n",
        );
        editor.mode = Mode::Select;
        editor.anchor = 0;
        editor.cursor = editor.doc.size();
        editor.remove_comments_from_selection();
        assert_eq!(editor.doc.all_text(), b"let x = 1;\nlet y = 2;\n");
    }

    #[test]
    fn removes_comments_after_opening_command_from_a_selection() {
        let mut editor = editor("test.rs", b"// one\n// two\nlet x = 1;\n");
        editor.mode = Mode::Select;
        editor.anchor = 0;
        editor.cursor = editor.doc.size();
        editor.command_selection = true;
        editor.mode = Mode::Command;
        editor.command_line = "rc".into();
        editor.command_cursor = 2;
        editor.submit_command();
        assert_eq!(editor.doc.all_text(), b"let x = 1;\n");
        assert!(editor.mode == Mode::Select);
    }

    #[test]
    fn uses_grammar_line_comment_marker() {
        let mut editor = editor("test.lua", b"--[[ note ]]\n");
        // Lua prefers line comments; this verifies the marker used is the
        // grammar's line comment rather than a hard-coded default.
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"[[ note ]]\n");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"-- [[ note ]]\n");
    }

    #[test]
    fn recognizes_alternative_line_comment_markers() {
        let mut editor = editor("test.ini", b"; note\n");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"note\n");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"# note\n");
    }

    #[test]
    fn mixed_alternative_markers_are_recognized() {
        let mut editor = editor("test.ini", b"; one\n# two\nthree");
        editor.mode = Mode::Select;
        editor.selection_linewise = true;
        editor.anchor = 0;
        editor.cursor = editor.doc.size();
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"; one\n# two\n# three");
        editor.toggle_comment();
        assert_eq!(editor.doc.all_text(), b"one\ntwo\nthree");
    }

    #[test]
    fn removes_alternative_line_comments() {
        let mut editor = editor("test.ini", b"; one\n# two\nlet x = 1");
        editor.mode = Mode::Select;
        editor.anchor = 0;
        editor.cursor = editor.doc.size();
        editor.remove_comments_from_selection();
        assert_eq!(editor.doc.all_text(), b"let x = 1");
    }
}
