//! Small UTF-8, width and path helpers shared by the editor and renderer.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

pub const REPLACEMENT: char = '\u{FFFD}';

/// Decode one UTF-8 scalar at `offset`. Invalid input consumes one byte and
/// yields U+FFFD so unsaved bytes are preserved on disk but never emitted raw.
pub fn decode_at(text: &[u8], offset: usize) -> (char, usize) {
    if offset >= text.len() {
        return (REPLACEMENT, 0);
    }
    let first = text[offset];
    let len = match first {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return (REPLACEMENT, 1),
    };
    if offset + len > text.len() {
        return (REPLACEMENT, 1);
    }
    match std::str::from_utf8(&text[offset..offset + len]) {
        Ok(s) => (s.chars().next().unwrap_or(REPLACEMENT), len),
        Err(_) => (REPLACEMENT, 1),
    }
}

pub fn next_boundary(text: &[u8], offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    let (_, len) = decode_at(text, offset);
    (offset + len.max(1)).min(text.len())
}

pub fn previous_boundary(text: &[u8], offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut pos = offset.min(text.len());
    pos -= 1;
    while pos > 0 && (text[pos] & 0xC0) == 0x80 {
        pos -= 1;
    }
    pos
}

pub fn is_combining(cp: char) -> bool {
    matches!(cp as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD |
        0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED |
        0x0711 | 0x0730..=0x074A | 0x07A6..=0x07B0 | 0x07EB..=0x07F3 |
        0x0816..=0x082D | 0x0859..=0x085B | 0x08D3..=0x0902 |
        0x093A | 0x0941..=0x0948 | 0x094D | 0x0951..=0x0957 |
        0x0962..=0x0963 | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E |
        0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF |
        0xFE20..=0xFE2F)
}

/// Terminal display width in cells. Tabs are handled by the caller because
/// their width depends on the current column.
pub fn display_width(cp: char) -> usize {
    let n = cp as u32;
    if cp == '\t' {
        return 1;
    }
    if is_combining(cp) || n == 0x200B || n == 0x200C || n == 0x200D {
        return 0;
    }
    if n < 0x20 || (0x7F..0xA0).contains(&n) {
        return 1;
    }
    if (0x1100..=0x115F).contains(&n)
        || (0x2E80..=0x303E).contains(&n)
        || (0x3041..=0x33FF).contains(&n)
        || (0x3400..=0x4DBF).contains(&n)
        || (0x4E00..=0x9FFF).contains(&n)
        || (0xA000..=0xA4CF).contains(&n)
        || (0xA960..=0xA97F).contains(&n)
        || (0xAC00..=0xD7A3).contains(&n)
        || (0xF900..=0xFAFF).contains(&n)
        || (0xFE10..=0xFE19).contains(&n)
        || (0xFE30..=0xFE6F).contains(&n)
        || (0xFF00..=0xFF60).contains(&n)
        || (0xFFE0..=0xFFE6).contains(&n)
        || (0x1F300..=0x1FAFF).contains(&n)
        || (0x20000..=0x3FFFD).contains(&n)
    {
        2
    } else {
        1
    }
}

pub fn text_width(text: &str) -> usize {
    text.chars().map(display_width).sum()
}

pub fn visual_column(text: &[u8], offset: usize, tab_width: usize) -> usize {
    let mut column = 0;
    let mut pos = 0;
    while pos < offset && pos < text.len() {
        let (cp, len) = decode_at(text, pos);
        if len == 0 {
            break;
        }
        if cp == '\t' {
            column += tab_width - (column % tab_width);
        } else {
            column += display_width(cp);
        }
        pos += len;
    }
    column
}

pub fn byte_offset_for_visual_column(text: &[u8], target: usize, tab_width: usize) -> usize {
    let mut column = 0;
    let mut pos = 0;
    while pos < text.len() {
        let (cp, len) = decode_at(text, pos);
        if len == 0 {
            break;
        }
        let width = if cp == '\t' {
            tab_width - (column % tab_width)
        } else {
            display_width(cp)
        };
        if column + width > target {
            return pos;
        }
        column += width;
        pos += len;
    }
    text.len()
}

pub fn digit_count(value: usize) -> usize {
    let mut n = value.max(1);
    let mut digits = 0;
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits
}

pub fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// ASCII-only literal substring search. Searching is case-insensitive unless
/// `match_case` is set.
pub fn index_of(haystack: &[u8], needle: &[u8], match_case: bool) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| {
        if match_case {
            window == needle
        } else {
            window.eq_ignore_ascii_case(needle)
        }
    })
}

/// ASCII-only case-insensitive substring search.
pub fn index_of_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    index_of(haystack, needle, false)
}

pub fn trim_trailing_whitespace(text: &[u8]) -> &[u8] {
    let mut end = text.len();
    while end > 0 && (text[end - 1] == b' ' || text[end - 1] == b'\t' || text[end - 1] == b'\r') {
        end -= 1;
    }
    &text[..end]
}

pub fn expand_home(path: &str) -> String {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().into_owned();
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut result = home.to_string_lossy().into_owned();
            result.push('/');
            result.push_str(rest);
            return result;
        }
    }
    path.to_string()
}

pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b & 0x0F) << 2) | (c >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(c & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
