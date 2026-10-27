//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    // ---------------------------------------------------------------------
    // Editing
    // ---------------------------------------------------------------------

    pub(super) fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn enter_insert_from_selection(&mut self) {
        self.delete_selection();
        self.enter_insert();
    }

    pub(super) fn insert_newline(&mut self) {
        let line = self.cursor_line();
        let line_start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - line_start).min(text.len());

        let mut indent = 0usize;
        while indent < text.len() && (text[indent] == b' ' || text[indent] == b'\t') {
            indent += 1;
        }
        indent = indent.min(local);

        let before = &text[..local];
        let after = &text[local..];
        let trigger = if self.auto_indent_enabled() {
            self.indent_trigger(before)
        } else {
            None
        };
        let increase = trigger.is_some();
        let add_closing_line = increase
            && trigger
                .as_deref()
                .is_some_and(|trigger| self.starts_with_closing_token(after, trigger));

        let eol = self.doc.eol.as_bytes();
        let mut builder = Vec::new();
        builder.extend_from_slice(eol);
        builder.extend_from_slice(&text[..indent]);
        if increase {
            builder.extend_from_slice(&self.doc.indent_unit());
        }

        if add_closing_line {
            self.doc.insert(self.cursor, &builder);
            self.cursor += builder.len();
            let cursor_on_indented_line = self.cursor;
            let mut closing = Vec::new();
            closing.extend_from_slice(eol);
            closing.extend_from_slice(&text[..indent]);
            self.doc.insert(self.cursor, &closing);
            self.cursor = cursor_on_indented_line;
        } else {
            self.doc.insert(self.cursor, &builder);
            self.cursor += builder.len();
        }

        self.anchor = self.cursor;
        self.preferred_column = None;
        self.mode = Mode::Insert;
        self.snap_cursor();
        self.clear_message();
    }

    fn auto_indent_enabled(&self) -> bool {
        self.active_language()
            .is_some_and(|language| language.auto_indent)
    }

    fn auto_close_enabled(&self) -> bool {
        self.active_language()
            .is_some_and(|language| language.auto_close)
    }

    fn indent_trigger(&self, before: &[u8]) -> Option<String> {
        let language = self.active_language()?;
        let trimmed = trim_end_whitespace(before);
        if language.traditional_markup {
            return parse_open_tag_at_end(trimmed).map(|name| format!("<{name}>"));
        }
        let text = String::from_utf8_lossy(trimmed);
        let mut best: Option<String> = None;
        for candidate in ["{", "[", "("] {
            if text.ends_with(candidate)
                && best
                    .as_ref()
                    .is_none_or(|current| candidate.len() > current.len())
            {
                best = Some(candidate.to_string());
            }
        }
        for candidate in &language.indent_after {
            if text.ends_with(candidate)
                && best
                    .as_ref()
                    .is_none_or(|current| candidate.len() > current.len())
            {
                best = Some(candidate.clone());
            }
        }
        best
    }

    fn starts_with_closing_token(&self, after: &[u8], trigger: &str) -> bool {
        let text = String::from_utf8_lossy(trim_start_whitespace(after));
        if text.is_empty() || trigger.is_empty() {
            return false;
        }
        if trigger.starts_with('<') && trigger.ends_with('>') {
            let name = &trigger[1..trigger.len() - 1];
            return text
                .to_ascii_lowercase()
                .starts_with(&format!("</{}>", name.to_ascii_lowercase()));
        }
        let closing = match trigger {
            "{" => Some("}"),
            "[" => Some("]"),
            "(" => Some(")"),
            _ => None,
        };
        if let Some(closing) = closing {
            return text.starts_with(closing);
        }
        self.active_language().is_some_and(|language| {
            language
                .unindent_before
                .iter()
                .any(|item| text.starts_with(item))
        })
    }

    pub(super) fn insert_character(&mut self, character: char) {
        if character == '\r' {
            return;
        }
        if character == '\n' {
            self.insert_newline();
            return;
        }
        if character < '\u{20}' && character != '\t' {
            return;
        }
        if ('\u{7F}'..'\u{A0}').contains(&character) {
            return;
        }

        if self.try_skip_auto_close(character) || self.try_insert_auto_close_pair(character) {
            return;
        }

        self.completion.pending = false;
        let mut buffer = [0u8; 4];
        let text = character.encode_utf8(&mut buffer);
        self.doc.insert(self.cursor, text.as_bytes());
        self.cursor += text.len();
        self.anchor = self.cursor;

        if character == '>' {
            self.try_insert_auto_close_tag();
        }
        self.try_auto_unindent_keyword();
        self.snap_cursor();
        self.preferred_column = None;
        self.clear_message();

        if character.is_ascii()
            && !character.is_whitespace()
            && self.lsp.is_running()
            && self.lsp.is_initialized()
            && self.lsp.is_completion_trigger(character)
        {
            self.try_request_lsp_completion(Some(character));
        }
    }

    pub(super) fn insert_indent(&mut self) {
        let unit = self.doc.indent_unit();
        self.doc.insert(self.cursor, &unit);
        self.cursor += unit.len();
        self.anchor = self.cursor;
        self.snap_cursor();
        self.preferred_column = None;
        self.clear_message();
    }

    fn try_skip_auto_close(&mut self, character: char) -> bool {
        if !self.auto_close_enabled() || !matches!(character, ')' | ']' | '}') {
            return false;
        }
        if self.cursor >= self.doc.size()
            || self.doc.byte_at(self.cursor) != Some(character as u8)
            || self.is_insert_position_in_string_or_comment()
        {
            return false;
        }
        self.cursor += 1;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
        self.clear_message();
        true
    }

    fn try_insert_auto_close_pair(&mut self, character: char) -> bool {
        if !self.auto_close_enabled() {
            return false;
        }
        let close = match character {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => return false,
        };
        if self.is_insert_position_in_string_or_comment() {
            return false;
        }
        let pair = [character as u8, close as u8];
        self.doc.insert(self.cursor, &pair);
        self.cursor += 1;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
        self.clear_message();
        true
    }

    fn try_insert_auto_close_tag(&mut self) {
        let Some(language) = self.active_language() else {
            return;
        };
        if !language.auto_close || !language.traditional_markup {
            return;
        }
        if self.is_insert_position_in_string_or_comment() {
            return;
        }
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        let Some(name) = parse_open_tag_at_end(&text[..local]) else {
            return;
        };
        let closing = format!("</{name}>");
        self.doc.insert(self.cursor, closing.as_bytes());
        self.snap_cursor();
    }

    fn try_auto_unindent_keyword(&mut self) {
        let language = if let Some(id) = &self.language_override {
            self.languages.find(id)
        } else {
            self.doc
                .path
                .as_deref()
                .and_then(|path| self.languages.detect(path))
        };
        let Some(language) = language else {
            return;
        };
        if !language.auto_indent || language.unindent_before.is_empty() {
            return;
        }
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        let mut content = 0usize;
        while content < local && (text[content] == b' ' || text[content] == b'\t') {
            content += 1;
        }
        if content >= local {
            return;
        }
        let text_after = String::from_utf8_lossy(&text[content..local]);
        if !language.is_unindent_keyword(&text_after) {
            return;
        }
        let remove = self.doc.unindent_length(&text[..content]);
        if remove == 0 {
            return;
        }
        self.doc.erase(start, remove);
        self.cursor = self.cursor.saturating_sub(remove);
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    fn is_insert_position_in_string_or_comment(&mut self) -> bool {
        let language = if let Some(id) = &self.language_override {
            self.languages.find(id)
        } else {
            self.doc
                .path
                .as_deref()
                .and_then(|path| self.languages.detect(path))
        };
        let Some(language) = language else {
            return false;
        };
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        let mut tokens = std::mem::take(&mut self.line_tokens);
        self.highlighter
            .get_line_tokens(&self.doc, language, line, &text, &mut tokens);
        let mut result = false;
        for token in tokens.iter() {
            if token.kind == SyntaxKind::Comment {
                if local > token.start && local <= token.end {
                    result = true;
                    break;
                }
                continue;
            }
            if token.kind != SyntaxKind::String {
                continue;
            }
            if local > token.start && local < token.end {
                result = true;
                break;
            }
            if local == token.end && !is_closed_string_token(token, &text, language) {
                result = true;
                break;
            }
        }
        self.line_tokens = tokens;
        result
    }

    pub(super) fn insert_newline_below(&mut self) {
        self.doc.break_undo_group();
        let line = self.cursor_line();
        let end = self.doc.line_end(line);
        let eol = self.doc.eol.as_bytes();
        self.doc.insert(end, eol);
        self.cursor = self.doc.line_start(line + 1);
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn insert_newline_above(&mut self) {
        self.doc.break_undo_group();
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let eol = self.doc.eol.as_bytes();
        self.doc.insert(start, eol);
        self.cursor = start;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.config.smart_backspace {
            let line = self.cursor_line();
            let line_start = self.doc.line_start(line);
            if self.cursor > line_start {
                let text = self.doc.line_text(line);
                let local = (self.cursor - line_start).min(text.len());
                if text[..local]
                    .iter()
                    .all(|byte| matches!(byte, b' ' | b'\t'))
                {
                    self.doc.erase(line_start, local);
                    self.cursor = line_start;
                    self.anchor = self.cursor;
                    self.preferred_column = None;
                    self.snap_cursor();
                    self.clear_message();
                    return;
                }
            }
        }
        let mut previous = self.previous_scalar(self.cursor);
        if previous > 0
            && self.doc.byte_at(previous) == Some(b'\n')
            && self.doc.byte_at(previous - 1) == Some(b'\r')
        {
            previous -= 1;
        }
        self.doc.erase(previous, self.cursor - previous);
        self.cursor = previous;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn delete_forward(&mut self) {
        if self.cursor >= self.doc.size() {
            return;
        }
        let line_end = self.line_end_of_cursor();
        let mut end = self.next_scalar(self.cursor);
        if self.cursor == line_end
            && self.doc.byte_at(self.cursor) == Some(b'\r')
            && self.doc.byte_at(self.cursor + 1) == Some(b'\n')
        {
            end = self.cursor + 2;
        }
        self.doc.erase(self.cursor, end - self.cursor);
        self.snap_cursor();
    }

    pub(super) fn delete_at_cursor(&mut self) {
        if self.mode == Mode::Select {
            self.delete_selection();
        } else {
            self.delete_forward();
        }
    }

    pub(super) fn delete_selection(&mut self) {
        if self.mode != Mode::Select {
            return;
        }
        let (start, end) = self.selection_range();
        if end > start {
            self.doc.replace(start, end - start, b"");
        }
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.preferred_column = None;
        self.snap_cursor();
    }

    pub(super) fn indent_selection(&mut self, unindent: bool) {
        if self.mode != Mode::Select {
            self.set_message("Select lines before indenting.");
            return;
        }
        let (range_start, range_end) = self.selection_range();
        let first_line = self.doc.line_of_offset(range_start);
        let mut last_line = self.doc.line_of_offset(range_end);
        if last_line > first_line && range_end == self.doc.line_start(last_line) {
            last_line -= 1;
        }
        for line in first_line..=last_line {
            let start = self.doc.line_start(line);
            if unindent {
                let text = self.doc.line_text(line);
                let count = self.doc.unindent_length(&text);
                if count > 0 {
                    self.doc.erase(start, count);
                }
            } else {
                let unit = self.doc.indent_unit();
                self.doc.insert(start, &unit);
            }
        }
        self.doc.break_undo_group();
    }

    // ---------------------------------------------------------------------
    // Selection and clipboard
    // ---------------------------------------------------------------------

    pub(super) fn start_character_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn start_line_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = true;
        self.anchor = self.doc.line_start(self.cursor_line());
        self.cursor = self.anchor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn extend_line_selection(&mut self) {
        if !self.selection_linewise {
            self.selection_linewise = true;
            self.anchor = self.doc.line_start(self.cursor_line());
            self.cursor = self.anchor;
        } else {
            let line = self.cursor_line();
            if line + 1 < self.doc.line_count() {
                self.cursor = self.doc.line_start(line + 1);
            } else {
                self.set_message("Already at the last line.");
            }
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn select_all(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = 0;
        self.cursor = self.doc.size();
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    pub(super) fn cancel_selection(&mut self) {
        self.mode = Mode::Normal;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn selection_range(&self) -> (usize, usize) {
        if self.selection_linewise {
            let anchor_line = self.doc.line_of_offset(self.anchor);
            let cursor_line = self.doc.line_of_offset(self.cursor);
            let first = anchor_line.min(cursor_line);
            let last = anchor_line.max(cursor_line);
            let start = self.doc.line_start(first);
            let end = if last + 1 < self.doc.line_count() {
                self.doc.line_start(last + 1)
            } else {
                self.doc.size()
            };
            return (start, end);
        }

        let mut range_start = self.anchor.min(self.cursor);
        let mut range_end = self.anchor.max(self.cursor);
        if range_end < self.doc.size() {
            let line = self.doc.line_of_offset(range_end);
            let line_end = self.doc.line_end(line);
            if range_end < line_end {
                range_end = self.next_scalar(range_end);
            } else if range_end == line_end && line + 1 < self.doc.line_count() {
                range_end = self.doc.line_start(line + 1);
            }
        }
        if range_start > range_end {
            range_start = range_end;
        }
        (range_start, range_end)
    }

    pub(super) fn yank_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to yank.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        if self.mode == Mode::Select {
            self.cursor = start;
            self.cancel_selection();
        }
        self.set_message(&format!("Yanked {} byte(s).", text.len()));
    }

    pub(super) fn cut_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to cut.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        self.doc.replace(start, end - start, b"");
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.set_message(&format!("Cut {} byte(s).", text.len()));
    }

    pub(super) fn store_clipboard(&mut self, text: &[u8]) {
        self.clipboard = String::from_utf8_lossy(text).into_owned();
        let encoded = util::base64_encode(text);
        self.pending_osc = Some(format!("\x1b]52;c;{encoded}\x07").into_bytes());
    }

    pub(super) fn paste_clipboard(&mut self, new_line: bool) {
        if self.clipboard.is_empty() {
            self.set_error("clipboard is empty");
            return;
        }
        let text = self.clipboard.clone();
        if new_line {
            let line = self.cursor_line();
            let end = self.doc.line_end(line);
            let mut builder = Vec::new();
            builder.extend_from_slice(self.doc.eol.as_bytes());
            builder.extend_from_slice(text.as_bytes());
            self.doc.insert(end, &builder);
            self.cursor = (end + builder.len()).min(self.doc.size());
        } else if self.mode == Mode::Select {
            let (start, end) = self.selection_range();
            self.doc.replace(start, end - start, text.as_bytes());
            self.cursor = (start + text.len()).min(self.doc.size());
        } else {
            self.doc.insert(self.cursor, text.as_bytes());
            self.cursor += text.len();
        }
        self.mode = Mode::Normal;
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.snap_cursor();
        self.set_message(&format!("Pasted {} byte(s).", text.len()));
    }
}

fn trim_start_whitespace(text: &[u8]) -> &[u8] {
    let mut start = 0usize;
    while start < text.len() && matches!(text[start], b' ' | b'\t' | b'\r' | b'\n') {
        start += 1;
    }
    &text[start..]
}

fn trim_end_whitespace(text: &[u8]) -> &[u8] {
    let mut end = text.len();
    while end > 0 && matches!(text[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    &text[..end]
}

fn is_closed_string_token(token: &Token, line: &[u8], language: &Language) -> bool {
    if token.end <= token.start || token.end > line.len() {
        return false;
    }
    let text = &line[token.start..token.end];
    let mut quote_length = 0usize;
    for quote in &language.quote_characters {
        if text.ends_with(quote) {
            quote_length = quote.len();
            break;
        }
    }
    if quote_length == 0 || text.len() <= quote_length {
        return false;
    }
    let escape = &language.escape_character;
    let mut escapes = 0usize;
    let mut index = text.len() - quote_length;
    while !escape.is_empty()
        && index >= escape.len()
        && &text[index - escape.len()..index] == escape
    {
        escapes += 1;
        index -= escape.len();
    }
    escapes % 2 == 0
}

fn parse_open_tag_at_end(text: &[u8]) -> Option<String> {
    if text.len() < 3 || text[text.len() - 1] != b'>' {
        return None;
    }
    let mut start = text.len() - 2;
    while text[start] != b'<' {
        if start == 0 {
            return None;
        }
        start -= 1;
    }
    let content = trim_both(&text[start + 1..text.len() - 1]);
    if content.is_empty()
        || matches!(content[0], b'/' | b'!' | b'?')
        || content[content.len() - 1] == b'/'
    {
        return None;
    }
    let mut name_length = 0usize;
    while name_length < content.len()
        && !matches!(
            content[name_length],
            b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>'
        )
    {
        name_length += 1;
    }
    if name_length == 0 {
        return None;
    }
    for (index, value) in content[..name_length].iter().enumerate() {
        let valid = value.is_ascii_alphabetic()
            || value.is_ascii_digit()
            || matches!(*value, b'_' | b'-' | b':' | b'.');
        if !valid || (index == 0 && !value.is_ascii_alphabetic() && !matches!(*value, b'_' | b':'))
        {
            return None;
        }
    }
    if !has_balanced_quotes_and_angles(content) {
        return None;
    }
    let name = String::from_utf8_lossy(&content[..name_length]).into_owned();
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];
    if VOID.iter().any(|void| void.eq_ignore_ascii_case(&name)) {
        return None;
    }
    Some(name)
}

fn trim_both(text: &[u8]) -> &[u8] {
    trim_end_whitespace(trim_start_whitespace(text))
}

fn has_balanced_quotes_and_angles(content: &[u8]) -> bool {
    let mut in_double = false;
    let mut in_single = false;
    for value in content {
        if in_double {
            if *value == b'"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if *value == b'\'' {
                in_single = false;
            }
            continue;
        }
        match *value {
            b'"' => in_double = true,
            b'\'' => in_single = true,
            b'<' | b'>' => return false,
            _ => {}
        }
    }
    !in_double && !in_single
}
