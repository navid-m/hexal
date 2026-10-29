//! Grammar-driven byte lexer with cached line-start block-comment states.
//!
//! Only visible lines are tokenized. The cache mirrors the reference editor:
//! before trusting a cached predecessor its line signature is revalidated, so
//! an edit never leaves a stale block-comment state behind.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::HashMap;

use crate::core::document::Document;
use crate::ui::theme::SyntaxKind;

use super::definition::Language;

pub const MAX_BACKWARD_STATE_SEARCH: usize = 512;
const MAX_CACHE_ENTRIES: usize = 50_000;

#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: SyntaxKind,
}

#[derive(Clone, Copy)]
struct CachedLine {
    signature: u64,
    end_in_block_comment: bool,
}

#[derive(Clone, Default)]
pub struct Highlighter {
    cache: HashMap<usize, CachedLine>,
    line_buffer: Vec<u8>,
}

impl Highlighter {
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn get_line_tokens(
        &mut self,
        document: &Document,
        language: &Language,
        line: usize,
        text: &[u8],
        tokens: &mut Vec<Token>,
    ) {
        tokens.clear();
        let start_in_block = self.resolve_line_start(document, language, line);
        let end_in_block = tokenize_line(language, text, start_in_block, tokens);

        if self.cache.len() >= MAX_CACHE_ENTRIES {
            self.cache.clear();
        }
        self.cache.insert(
            line,
            CachedLine {
                signature: signature(text),
                end_in_block_comment: end_in_block,
            },
        );
    }

    fn resolve_line_start(
        &mut self,
        document: &Document,
        language: &Language,
        line: usize,
    ) -> bool {
        if line == 0 {
            return false;
        }

        self.cache.remove(&line);

        let mut found_line = None;
        let mut state = false;
        let mut search = line.saturating_sub(1);
        let mut steps = 0usize;
        while steps < MAX_BACKWARD_STATE_SEARCH {
            if let Some(previous) = self.cache.get(&search).copied() {
                document.copy_line(search, &mut self.line_buffer);
                if previous.signature == signature(&self.line_buffer) {
                    state = previous.end_in_block_comment;
                    found_line = Some(search);
                    break;
                }
                self.cache.remove(&search);
            }
            if search == 0 {
                break;
            }
            search -= 1;
            steps += 1;
        }

        let from_line = match found_line {
            Some(found) => found + 1,
            None if line <= 1024 => 0,
            // A viewport far from any cached state starts in the default
            // state; jumps into huge files stay proportional to the viewport.
            None => line,
        };

        let mut current = from_line;
        while current < line {
            document.copy_line(current, &mut self.line_buffer);
            let end = tokenize_line(language, &self.line_buffer, state, &mut Vec::new());
            self.cache.insert(
                current,
                CachedLine {
                    signature: signature(&self.line_buffer),
                    end_in_block_comment: end,
                },
            );
            state = end;
            current += 1;
        }

        state
    }
}

fn signature(text: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

fn starts_with(text: &[u8], offset: usize, marker: &[u8]) -> bool {
    !marker.is_empty()
        && marker.len() <= text.len()
        && offset <= text.len() - marker.len()
        && &text[offset..offset + marker.len()] == marker
}

fn find_bytes(text: &[u8], offset: usize, marker: &[u8]) -> Option<usize> {
    if marker.is_empty() || offset >= text.len() {
        return None;
    }
    text[offset..]
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|index| offset + index)
}

fn add_token(tokens: &mut Vec<Token>, start: usize, end: usize, kind: SyntaxKind) {
    if end > start && kind != SyntaxKind::Default {
        tokens.push(Token { start, end, kind });
    }
}

fn match_quote(text: &[u8], offset: usize, quotes: &[Vec<u8>]) -> usize {
    for quote in quotes {
        if starts_with(text, offset, quote) {
            return quote.len();
        }
    }
    0
}

fn skip_rune(text: &[u8], offset: &mut usize) {
    if *offset >= text.len() {
        return;
    }
    let (_, length) = crate::util::decode_at(text, *offset);
    *offset += length.max(1).min(text.len() - *offset);
}

fn consume_number(text: &[u8], offset: &mut usize) {
    let mut hexadecimal = false;
    let mut binary = false;
    if *offset + 1 < text.len() && text[*offset] == b'0' {
        let prefix = text[*offset + 1];
        hexadecimal = matches!(prefix, b'x' | b'X');
        binary = matches!(prefix, b'b' | b'B');
    }
    if hexadecimal || binary {
        *offset += 2;
        while *offset < text.len() {
            let value = text[*offset];
            if value.is_ascii_digit() || (hexadecimal && value.is_ascii_hexdigit()) || value == b'_'
            {
                *offset += 1;
                continue;
            }
            break;
        }
        return;
    }

    while *offset < text.len() {
        let value = text[*offset];
        if value.is_ascii_digit() || value == b'.' || value == b'_' {
            *offset += 1;
            continue;
        }
        if matches!(value, b'e' | b'E') {
            *offset += 1;
            if *offset < text.len() && matches!(text[*offset], b'+' | b'-') {
                *offset += 1;
            }
            continue;
        }
        break;
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_identifier_part(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}

/// Tokenizes one line. Returns true when the line ends inside a block
/// comment.
pub fn tokenize_line(
    language: &Language,
    text: &[u8],
    start_in_block: bool,
    tokens: &mut Vec<Token>,
) -> bool {
    let mut in_block = start_in_block;
    let mut offset = 0usize;
    let line_highlight = if !in_block && !language.line_highlights.is_empty() {
        match_line_highlight(language, text)
    } else {
        None
    };

    let block_start = language.block_comment_start.as_ref().map(String::as_bytes);
    let block_end = language.block_comment_end.as_ref().map(String::as_bytes);
    let line_comment = language.line_comment.as_ref().map(String::as_bytes);

    while offset < text.len() {
        if in_block {
            let Some(end_marker) = block_end else {
                add_token(tokens, offset, text.len(), SyntaxKind::Comment);
                break;
            };
            let Some(end) = find_bytes(text, offset, end_marker) else {
                add_token(tokens, offset, text.len(), SyntaxKind::Comment);
                break;
            };
            let token_end = end + end_marker.len();
            add_token(tokens, offset, token_end, SyntaxKind::Comment);
            offset = token_end;
            in_block = false;
            continue;
        }

        // Block comments are checked first: Lua's --[[ ... ]] starts with its
        // line-comment marker but must still carry state across lines.
        if let (Some(start_marker), Some(end_marker)) = (block_start, block_end) {
            if starts_with(text, offset, start_marker) {
                let content_start = offset + start_marker.len();
                if let Some(end) = find_bytes(text, content_start, end_marker) {
                    let token_end = end + end_marker.len();
                    add_token(tokens, offset, token_end, SyntaxKind::Comment);
                    offset = token_end;
                } else {
                    add_token(tokens, offset, text.len(), SyntaxKind::Comment);
                    in_block = true;
                    break;
                }
                continue;
            }
        }

        if let Some(marker) = line_comment {
            if starts_with(text, offset, marker) {
                add_token(tokens, offset, text.len(), SyntaxKind::Comment);
                break;
            }
        }

        let quote_length = match_quote(text, offset, &language.quote_characters);
        if quote_length > 0 {
            let start = offset;
            offset += quote_length;
            while offset < text.len() {
                if !language.escape_character.is_empty()
                    && starts_with(text, offset, &language.escape_character)
                {
                    offset += language.escape_character.len();
                    skip_rune(text, &mut offset);
                    continue;
                }
                let closing = match_quote(text, offset, &language.quote_characters);
                if closing > 0 {
                    offset += closing;
                    break;
                }
                skip_rune(text, &mut offset);
            }
            add_token(tokens, start, offset, SyntaxKind::String);
            continue;
        }

        if language.highlight_numbers && text[offset].is_ascii_digit() {
            let start = offset;
            consume_number(text, &mut offset);
            add_token(tokens, start, offset, SyntaxKind::Number);
            continue;
        }

        if is_identifier_start(text[offset]) {
            let start = offset;
            offset += 1;
            while offset < text.len() && is_identifier_part(text[offset]) {
                offset += 1;
            }
            let kind = language.classify_identifier(&text[start..offset]);
            add_token(tokens, start, offset, kind);
            continue;
        }

        skip_rune(text, &mut offset);
    }

    if let Some((highlight_start, highlight_end, kind)) = line_highlight {
        merge_line_highlight(tokens, highlight_start, highlight_end, kind);
    }

    in_block
}

fn match_line_highlight(language: &Language, text: &[u8]) -> Option<(usize, usize, SyntaxKind)> {
    if text.is_empty() {
        return None;
    }
    for rule in &language.line_highlights {
        if let Some((start, end)) = rule.pattern.find(text) {
            if end > start {
                return Some((start, end, rule.kind));
            }
        }
    }
    None
}

fn merge_line_highlight(
    tokens: &mut Vec<Token>,
    highlight_start: usize,
    highlight_end: usize,
    kind: SyntaxKind,
) {
    let mut merged = Vec::with_capacity(tokens.len() + 1);
    let mut index = 0usize;
    while index < tokens.len() && tokens[index].end <= highlight_start {
        merged.push(tokens[index]);
        index += 1;
    }
    while index < tokens.len() && tokens[index].start < highlight_end {
        index += 1;
    }
    merged.push(Token {
        start: highlight_start,
        end: highlight_end,
        kind,
    });
    while index < tokens.len() {
        merged.push(tokens[index]);
        index += 1;
    }
    *tokens = merged;
}

#[cfg(test)]
mod tests {
    use super::{Highlighter, tokenize_line};
    use crate::core::document::Document;
    use crate::languages::Registry;
    use crate::ui::theme::SyntaxKind;

    #[test]
    fn carries_block_comment_state() {
        let registry = Registry::load();
        let language = registry.find("c").unwrap();
        let document = Document::from_bytes(None, b"/* start\nmiddle\n*/ int x;\n".to_vec());
        let mut highlighter = Highlighter::default();
        let mut tokens = Vec::new();

        highlighter.get_line_tokens(&document, language, 0, b"/* start", &mut tokens);
        assert!(tokens.iter().any(|token| token.kind == SyntaxKind::Comment));
        highlighter.get_line_tokens(&document, language, 1, b"middle", &mut tokens);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxKind::Comment);
        highlighter.get_line_tokens(&document, language, 2, b"*/ int x;", &mut tokens);
        assert_eq!(tokens[0].kind, SyntaxKind::Comment);
        assert!(tokens.iter().any(|token| token.kind == SyntaxKind::Type));
    }

    #[test]
    fn handles_lines_shorter_than_comment_markers() {
        let registry = Registry::load();
        let language = registry.find("rust").unwrap();
        let mut tokens = Vec::new();
        tokenize_line(language, b"}", false, &mut tokens);
        assert!(tokens.is_empty());
        tokenize_line(language, b"{", false, &mut tokens);
        assert!(tokens.is_empty());
        let c = registry.find("c").unwrap();
        tokenize_line(c, b"}", false, &mut tokens);
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenizes_short_lines_for_every_grammar() {
        let registry = Registry::load();
        let lines: &[&[u8]] = &[
            b"", b"}", b"{", b"+", b"-", b"/", b"*", b"#", b"--", b"/*", b"*/", b"\"", b"'", b"<",
            b">", b"=", b"1", b"x",
        ];
        for language in registry.languages() {
            let mut tokens = Vec::new();
            for line in lines {
                tokenize_line(&language, line, false, &mut tokens);
                tokenize_line(&language, line, true, &mut tokens);
            }
        }
    }

    #[test]
    fn applies_line_patterns() {
        let registry = Registry::load();
        let language = registry.find("diff").unwrap();
        let mut tokens = Vec::new();
        tokenize_line(language, b"+ added", false, &mut tokens);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxKind::Added);
        tokenize_line(language, b"- removed", false, &mut tokens);
        assert_eq!(tokens[0].kind, SyntaxKind::Deleted);
    }
}
