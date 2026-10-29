//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

const std = @import("std");

pub const replacement: u21 = 0xFFFD;

pub const Decoded = struct {
    codepoint: u21,
    len: usize,
};

/// Decode one UTF-8 scalar at `offset`. Invalid input is reported as U+FFFD
/// while still consuming one byte, matching Hexal's documented policy of
/// preserving invalid bytes on disk and drawing U+FFFD.
pub fn decodeAt(text: []const u8, offset: usize) Decoded {
    if (offset >= text.len) {
        return .{ .codepoint = replacement, .len = 0 };
    }

    const first = text[offset];
    const seq_len = std.unicode.utf8ByteSequenceLength(first) catch {
        return .{ .codepoint = replacement, .len = 1 };
    };
    const len: usize = seq_len;
    if (offset + len > text.len) {
        return .{ .codepoint = replacement, .len = 1 };
    }
    const cp = std.unicode.utf8Decode(text[offset .. offset + len]) catch {
        return .{ .codepoint = replacement, .len = 1 };
    };
    return .{ .codepoint = cp, .len = len };
}

pub fn nextBoundary(text: []const u8, offset: usize) usize {
    if (offset >= text.len) return text.len;
    const decoded = decodeAt(text, offset);
    const step = if (decoded.len == 0) 1 else decoded.len;
    return @min(text.len, offset + step);
}

pub fn previousBoundary(text: []const u8, offset: usize) usize {
    if (offset == 0) return 0;
    var pos = @min(offset, text.len);
    pos -= 1;
    while (pos > 0 and (text[pos] & 0xC0) == 0x80) {
        pos -= 1;
    }
    return pos;
}

pub fn isCombining(cp: u21) bool {
    return (cp >= 0x0300 and cp <= 0x036F) or
        (cp >= 0x0483 and cp <= 0x0489) or
        (cp >= 0x0591 and cp <= 0x05BD) or
        (cp >= 0x0610 and cp <= 0x061A) or
        (cp >= 0x064B and cp <= 0x065F) or
        (cp >= 0x0670 and cp <= 0x0670) or
        (cp >= 0x06D6 and cp <= 0x06ED) or
        (cp >= 0x0711 and cp <= 0x0711) or
        (cp >= 0x0730 and cp <= 0x074A) or
        (cp >= 0x07A6 and cp <= 0x07B0) or
        (cp >= 0x07EB and cp <= 0x07F3) or
        (cp >= 0x0816 and cp <= 0x082D) or
        (cp >= 0x0859 and cp <= 0x085B) or
        (cp >= 0x08D3 and cp <= 0x0902) or
        (cp >= 0x093A and cp <= 0x093A) or
        (cp >= 0x0941 and cp <= 0x0948) or
        (cp >= 0x094D and cp <= 0x094D) or
        (cp >= 0x0951 and cp <= 0x0957) or
        (cp >= 0x0962 and cp <= 0x0963) or
        (cp >= 0x0981 and cp <= 0x0981) or
        (cp >= 0x09BC and cp <= 0x09BC) or
        (cp >= 0x09C1 and cp <= 0x09C4) or
        (cp >= 0x09CD and cp <= 0x09CD) or
        (cp >= 0x09E2 and cp <= 0x09E3) or
        (cp >= 0x0A01 and cp <= 0x0A02) or
        (cp >= 0x0A3C and cp <= 0x0A3C) or
        (cp >= 0x0A41 and cp <= 0x0A42) or
        (cp >= 0x0A47 and cp <= 0x0A48) or
        (cp >= 0x0A4B and cp <= 0x0A4D) or
        (cp >= 0x0A51 and cp <= 0x0A51) or
        (cp >= 0x0A70 and cp <= 0x0A71) or
        (cp >= 0x0A75 and cp <= 0x0A75) or
        (cp >= 0x0A81 and cp <= 0x0A82) or
        (cp >= 0x0ABC and cp <= 0x0ABC) or
        (cp >= 0x0AC1 and cp <= 0x0AC5) or
        (cp >= 0x0AC7 and cp <= 0x0AC8) or
        (cp >= 0x0ACD and cp <= 0x0ACD) or
        (cp >= 0x0AE2 and cp <= 0x0AE3) or
        (cp >= 0x0B01 and cp <= 0x0B01) or
        (cp >= 0x0B3C and cp <= 0x0B3C) or
        (cp >= 0x0B3F and cp <= 0x0B3F) or
        (cp >= 0x0B41 and cp <= 0x0B44) or
        (cp >= 0x0B4D and cp <= 0x0B4D) or
        (cp >= 0x0B56 and cp <= 0x0B56) or
        (cp >= 0x0B62 and cp <= 0x0B63) or
        (cp >= 0x0B82 and cp <= 0x0B82) or
        (cp >= 0x0BC0 and cp <= 0x0BC0) or
        (cp >= 0x0BCD and cp <= 0x0BCD) or
        (cp >= 0x0C00 and cp <= 0x0C00) or
        (cp >= 0x0C3E and cp <= 0x0C40) or
        (cp >= 0x0C46 and cp <= 0x0C48) or
        (cp >= 0x0C4A and cp <= 0x0C4D) or
        (cp >= 0x0C55 and cp <= 0x0C56) or
        (cp >= 0x0C62 and cp <= 0x0C63) or
        (cp >= 0x0C81 and cp <= 0x0C81) or
        (cp >= 0x0CBC and cp <= 0x0CBC) or
        (cp >= 0x0CBF and cp <= 0x0CBF) or
        (cp >= 0x0CC6 and cp <= 0x0CC6) or
        (cp >= 0x0CCC and cp <= 0x0CCD) or
        (cp >= 0x0CE2 and cp <= 0x0CE3) or
        (cp >= 0x0D00 and cp <= 0x0D01) or
        (cp >= 0x0D3B and cp <= 0x0D3C) or
        (cp >= 0x0D41 and cp <= 0x0D44) or
        (cp >= 0x0D4D and cp <= 0x0D4D) or
        (cp >= 0x0D62 and cp <= 0x0D63) or
        (cp >= 0x0DCA and cp <= 0x0DCA) or
        (cp >= 0x0DD2 and cp <= 0x0DD4) or
        (cp >= 0x0DD6 and cp <= 0x0DD6) or
        (cp >= 0x0E31 and cp <= 0x0E31) or
        (cp >= 0x0E34 and cp <= 0x0E3A) or
        (cp >= 0x0E47 and cp <= 0x0E4E) or
        (cp >= 0x0EB1 and cp <= 0x0EB1) or
        (cp >= 0x0EB4 and cp <= 0x0EB9) or
        (cp >= 0x0EBB and cp <= 0x0EBC) or
        (cp >= 0x0EC8 and cp <= 0x0ECD) or
        (cp >= 0x0F18 and cp <= 0x0F19) or
        (cp >= 0x0F35 and cp <= 0x0F35) or
        (cp >= 0x0F37 and cp <= 0x0F37) or
        (cp >= 0x0F39 and cp <= 0x0F39) or
        (cp >= 0x0F71 and cp <= 0x0F7E) or
        (cp >= 0x0F80 and cp <= 0x0F84) or
        (cp >= 0x0F86 and cp <= 0x0F87) or
        (cp >= 0x0F8D and cp <= 0x0F97) or
        (cp >= 0x0F99 and cp <= 0x0FBC) or
        (cp >= 0x0FC6 and cp <= 0x0FC6) or
        (cp >= 0x102D and cp <= 0x1030) or
        (cp >= 0x1032 and cp <= 0x1037) or
        (cp >= 0x1039 and cp <= 0x103A) or
        (cp >= 0x103D and cp <= 0x103E) or
        (cp >= 0x1058 and cp <= 0x1059) or
        (cp >= 0x105E and cp <= 0x1060) or
        (cp >= 0x1071 and cp <= 0x1074) or
        (cp >= 0x1082 and cp <= 0x1082) or
        (cp >= 0x1085 and cp <= 0x1086) or
        (cp >= 0x108D and cp <= 0x108D) or
        (cp >= 0x109D and cp <= 0x109D) or
        (cp >= 0x135D and cp <= 0x135F) or
        (cp >= 0x1712 and cp <= 0x1714) or
        (cp >= 0x1732 and cp <= 0x1734) or
        (cp >= 0x1752 and cp <= 0x1753) or
        (cp >= 0x1772 and cp <= 0x1773) or
        (cp >= 0x17B4 and cp <= 0x17B5) or
        (cp >= 0x17B7 and cp <= 0x17BD) or
        (cp >= 0x17C6 and cp <= 0x17C6) or
        (cp >= 0x17C9 and cp <= 0x17D3) or
        (cp >= 0x17DD and cp <= 0x17DD) or
        (cp >= 0x180B and cp <= 0x180D) or
        (cp >= 0x1885 and cp <= 0x1886) or
        (cp >= 0x18A9 and cp <= 0x18A9) or
        (cp >= 0x1920 and cp <= 0x1922) or
        (cp >= 0x1927 and cp <= 0x1928) or
        (cp >= 0x1932 and cp <= 0x1932) or
        (cp >= 0x1939 and cp <= 0x193B) or
        (cp >= 0x1A17 and cp <= 0x1A18) or
        (cp >= 0x1A1B and cp <= 0x1A1B) or
        (cp >= 0x1A56 and cp <= 0x1A56) or
        (cp >= 0x1A58 and cp <= 0x1A5E) or
        (cp >= 0x1A60 and cp <= 0x1A60) or
        (cp >= 0x1A62 and cp <= 0x1A62) or
        (cp >= 0x1A65 and cp <= 0x1A6C) or
        (cp >= 0x1A73 and cp <= 0x1A7C) or
        (cp >= 0x1A7F and cp <= 0x1A7F) or
        (cp >= 0x1AB0 and cp <= 0x1AFF) or
        (cp >= 0x1B00 and cp <= 0x1B03) or
        (cp >= 0x1B34 and cp <= 0x1B34) or
        (cp >= 0x1B36 and cp <= 0x1B3A) or
        (cp >= 0x1B3C and cp <= 0x1B3C) or
        (cp >= 0x1B42 and cp <= 0x1B42) or
        (cp >= 0x1B6B and cp <= 0x1B73) or
        (cp >= 0x1B80 and cp <= 0x1B81) or
        (cp >= 0x1BA2 and cp <= 0x1BA5) or
        (cp >= 0x1BA8 and cp <= 0x1BA9) or
        (cp >= 0x1BAB and cp <= 0x1BAD) or
        (cp >= 0x1BE6 and cp <= 0x1BE6) or
        (cp >= 0x1BE8 and cp <= 0x1BE9) or
        (cp >= 0x1BED and cp <= 0x1BED) or
        (cp >= 0x1BEF and cp <= 0x1BF1) or
        (cp >= 0x1C2C and cp <= 0x1C33) or
        (cp >= 0x1C36 and cp <= 0x1C37) or
        (cp >= 0x1CD0 and cp <= 0x1CD2) or
        (cp >= 0x1CD4 and cp <= 0x1CE0) or
        (cp >= 0x1CE2 and cp <= 0x1CE8) or
        (cp >= 0x1CED and cp <= 0x1CED) or
        (cp >= 0x1CF4 and cp <= 0x1CF4) or
        (cp >= 0x1CF8 and cp <= 0x1CF9) or
        (cp >= 0x1DC0 and cp <= 0x1DFF) or
        (cp >= 0x20D0 and cp <= 0x20FF) or
        (cp >= 0x2CEF and cp <= 0x2CF1) or
        (cp >= 0x2D7F and cp <= 0x2D7F) or
        (cp >= 0x2DE0 and cp <= 0x2DFF) or
        (cp >= 0x302A and cp <= 0x302D) or
        (cp >= 0x3099 and cp <= 0x309A) or
        (cp >= 0xA66F and cp <= 0xA672) or
        (cp >= 0xA674 and cp <= 0xA67D) or
        (cp >= 0xA69E and cp <= 0xA69F) or
        (cp >= 0xA6F0 and cp <= 0xA6F1) or
        (cp >= 0xA802 and cp <= 0xA802) or
        (cp >= 0xA806 and cp <= 0xA806) or
        (cp >= 0xA80B and cp <= 0xA80B) or
        (cp >= 0xA825 and cp <= 0xA826) or
        (cp >= 0xA8C4 and cp <= 0xA8C5) or
        (cp >= 0xA8E0 and cp <= 0xA8F1) or
        (cp >= 0xA926 and cp <= 0xA92D) or
        (cp >= 0xA947 and cp <= 0xA951) or
        (cp >= 0xA980 and cp <= 0xA982) or
        (cp >= 0xA9B3 and cp <= 0xA9B3) or
        (cp >= 0xA9B6 and cp <= 0xA9B9) or
        (cp >= 0xA9BC and cp <= 0xA9BD) or
        (cp >= 0xA9E5 and cp <= 0xA9E5) or
        (cp >= 0xAA29 and cp <= 0xAA2E) or
        (cp >= 0xAA31 and cp <= 0xAA32) or
        (cp >= 0xAA35 and cp <= 0xAA36) or
        (cp >= 0xAA43 and cp <= 0xAA43) or
        (cp >= 0xAA4C and cp <= 0xAA4C) or
        (cp >= 0xAA7C and cp <= 0xAA7C) or
        (cp >= 0xAAB0 and cp <= 0xAAB0) or
        (cp >= 0xAAB2 and cp <= 0xAAB4) or
        (cp >= 0xAAB7 and cp <= 0xAAB8) or
        (cp >= 0xAABE and cp <= 0xAABF) or
        (cp >= 0xAAC1 and cp <= 0xAAC1) or
        (cp >= 0xAAEC and cp <= 0xAAED) or
        (cp >= 0xAAF6 and cp <= 0xAAF6) or
        (cp >= 0xABE5 and cp <= 0xABE5) or
        (cp >= 0xABE8 and cp <= 0xABE8) or
        (cp >= 0xABED and cp <= 0xABED) or
        (cp >= 0xFB1E and cp <= 0xFB1E) or
        (cp >= 0xFE00 and cp <= 0xFE0F) or
        (cp >= 0xFE20 and cp <= 0xFE2F) or
        (cp >= 0x101FD and cp <= 0x101FD) or
        (cp >= 0x102E0 and cp <= 0x102E0) or
        (cp >= 0x10376 and cp <= 0x1037A) or
        (cp >= 0x10A01 and cp <= 0x10A03) or
        (cp >= 0x10A05 and cp <= 0x10A06) or
        (cp >= 0x10A0C and cp <= 0x10A0F) or
        (cp >= 0x10A38 and cp <= 0x10A3A) or
        (cp >= 0x10A3F and cp <= 0x10A3F) or
        (cp >= 0x10AE5 and cp <= 0x10AE6) or
        (cp >= 0x11001 and cp <= 0x11001) or
        (cp >= 0x11038 and cp <= 0x11046) or
        (cp >= 0x1107F and cp <= 0x11081) or
        (cp >= 0x110B3 and cp <= 0x110B6) or
        (cp >= 0x110B9 and cp <= 0x110BA) or
        (cp >= 0x11100 and cp <= 0x11102) or
        (cp >= 0x11127 and cp <= 0x1112B) or
        (cp >= 0x1112D and cp <= 0x11134) or
        (cp >= 0x11173 and cp <= 0x11173) or
        (cp >= 0x11180 and cp <= 0x11181) or
        (cp >= 0x111B6 and cp <= 0x111BE) or
        (cp >= 0x111CA and cp <= 0x111CC) or
        (cp >= 0x1122F and cp <= 0x11231) or
        (cp >= 0x11234 and cp <= 0x11234) or
        (cp >= 0x11236 and cp <= 0x11237) or
        (cp >= 0x1123E and cp <= 0x1123E) or
        (cp >= 0x112DF and cp <= 0x112DF) or
        (cp >= 0x112E3 and cp <= 0x112EA) or
        (cp >= 0x11300 and cp <= 0x11301) or
        (cp >= 0x1133C and cp <= 0x1133C) or
        (cp >= 0x11340 and cp <= 0x11340) or
        (cp >= 0x11366 and cp <= 0x1136C) or
        (cp >= 0x11370 and cp <= 0x11374) or
        (cp >= 0x11438 and cp <= 0x1143F) or
        (cp >= 0x11442 and cp <= 0x11444) or
        (cp >= 0x11446 and cp <= 0x11446) or
        (cp >= 0x114B3 and cp <= 0x114B8) or
        (cp >= 0x114BA and cp <= 0x114BA) or
        (cp >= 0x114BF and cp <= 0x114C0) or
        (cp >= 0x114C2 and cp <= 0x114C3) or
        (cp >= 0x115B2 and cp <= 0x115B5) or
        (cp >= 0x115BC and cp <= 0x115BD) or
        (cp >= 0x115BF and cp <= 0x115C0) or
        (cp >= 0x115DC and cp <= 0x115DD) or
        (cp >= 0x11633 and cp <= 0x1163A) or
        (cp >= 0x1163D and cp <= 0x1163D) or
        (cp >= 0x1163F and cp <= 0x11640) or
        (cp >= 0x116AB and cp <= 0x116AB) or
        (cp >= 0x116AD and cp <= 0x116AD) or
        (cp >= 0x116B0 and cp <= 0x116B5) or
        (cp >= 0x116B7 and cp <= 0x116B7) or
        (cp >= 0x1171D and cp <= 0x1171F) or
        (cp >= 0x11722 and cp <= 0x11725) or
        (cp >= 0x11727 and cp <= 0x1172B) or
        (cp >= 0x11C30 and cp <= 0x11C36) or
        (cp >= 0x11C38 and cp <= 0x11C3D) or
        (cp >= 0x11C3F and cp <= 0x11C3F) or
        (cp >= 0x11C92 and cp <= 0x11CA7) or
        (cp >= 0x11CAA and cp <= 0x11CB0) or
        (cp >= 0x11CB2 and cp <= 0x11CB3) or
        (cp >= 0x11CB5 and cp <= 0x11CB6) or
        (cp >= 0x16AF0 and cp <= 0x16AF4) or
        (cp >= 0x16B30 and cp <= 0x16B36) or
        (cp >= 0x16F8F and cp <= 0x16F92) or
        (cp >= 0x1BC9D and cp <= 0x1BC9E) or
        (cp >= 0x1D167 and cp <= 0x1D169) or
        (cp >= 0x1D17B and cp <= 0x1D182) or
        (cp >= 0x1D185 and cp <= 0x1D18B) or
        (cp >= 0x1D1AA and cp <= 0x1D1AD) or
        (cp >= 0x1D242 and cp <= 0x1D244) or
        (cp >= 0x1DA00 and cp <= 0x1DA36) or
        (cp >= 0x1DA3B and cp <= 0x1DA6C) or
        (cp >= 0x1DA75 and cp <= 0x1DA75) or
        (cp >= 0x1DA84 and cp <= 0x1DA84) or
        (cp >= 0x1DA9B and cp <= 0x1DA9F) or
        (cp >= 0x1DAA1 and cp <= 0x1DAAF) or
        (cp >= 0x1E000 and cp <= 0x1E006) or
        (cp >= 0x1E008 and cp <= 0x1E018) or
        (cp >= 0x1E01B and cp <= 0x1E021) or
        (cp >= 0x1E023 and cp <= 0x1E024) or
        (cp >= 0x1E026 and cp <= 0x1E02A) or
        (cp >= 0x1E8D0 and cp <= 0x1E8D6) or
        (cp >= 0x1E944 and cp <= 0x1E94A) or
        (cp >= 0xE0100 and cp <= 0xE01EF);
}

pub fn isWide(cp: u21) bool {
    return cp >= 0x1100 and
        (cp <= 0x115F or
            cp == 0x2329 or cp == 0x232A or
            (cp >= 0x2E80 and cp <= 0x303E) or
            (cp >= 0x3041 and cp <= 0x33FF) or
            (cp >= 0x3400 and cp <= 0x4DBF) or
            (cp >= 0x4E00 and cp <= 0x9FFF) or
            (cp >= 0xA000 and cp <= 0xA4CF) or
            (cp >= 0xA960 and cp <= 0xA97F) or
            (cp >= 0xAC00 and cp <= 0xD7A3) or
            (cp >= 0xF900 and cp <= 0xFAFF) or
            (cp >= 0xFE10 and cp <= 0xFE19) or
            (cp >= 0xFE30 and cp <= 0xFE6F) or
            (cp >= 0xFF00 and cp <= 0xFF60) or
            (cp >= 0xFFE0 and cp <= 0xFFE6) or
            (cp >= 0x1B000 and cp <= 0x1B001) or
            (cp >= 0x1F004 and cp <= 0x1F004) or
            (cp >= 0x1F0CF and cp <= 0x1F0CF) or
            (cp >= 0x1F18E and cp <= 0x1F18E) or
            (cp >= 0x1F191 and cp <= 0x1F19A) or
            (cp >= 0x1F200 and cp <= 0x1F320) or
            (cp >= 0x1F32D and cp <= 0x1F335) or
            (cp >= 0x1F337 and cp <= 0x1F37C) or
            (cp >= 0x1F37E and cp <= 0x1F393) or
            (cp >= 0x1F3A0 and cp <= 0x1F3CA) or
            (cp >= 0x1F3CF and cp <= 0x1F3D3) or
            (cp >= 0x1F3E0 and cp <= 0x1F3F0) or
            (cp >= 0x1F3F4 and cp <= 0x1F3F4) or
            (cp >= 0x1F3F8 and cp <= 0x1F43E) or
            (cp >= 0x1F440 and cp <= 0x1F440) or
            (cp >= 0x1F442 and cp <= 0x1F4FC) or
            (cp >= 0x1F4FF and cp <= 0x1F53D) or
            (cp >= 0x1F54B and cp <= 0x1F54E) or
            (cp >= 0x1F550 and cp <= 0x1F567) or
            (cp >= 0x1F57A and cp <= 0x1F57A) or
            (cp >= 0x1F595 and cp <= 0x1F596) or
            (cp >= 0x1F5A4 and cp <= 0x1F5A4) or
            (cp >= 0x1F5FB and cp <= 0x1F64F) or
            (cp >= 0x1F680 and cp <= 0x1F6C5) or
            (cp >= 0x1F6CC and cp <= 0x1F6CC) or
            (cp >= 0x1F6D0 and cp <= 0x1F6D2) or
            (cp >= 0x1F6D5 and cp <= 0x1F6D7) or
            (cp >= 0x1F6EB and cp <= 0x1F6EC) or
            (cp >= 0x1F6F4 and cp <= 0x1F6FC) or
            (cp >= 0x1F7E0 and cp <= 0x1F7EB) or
            (cp >= 0x1F90C and cp <= 0x1F93A) or
            (cp >= 0x1F93C and cp <= 0x1F945) or
            (cp >= 0x1F947 and cp <= 0x1F9FF) or
            (cp >= 0x1FA70 and cp <= 0x1FAFF) or
            (cp >= 0x20000 and cp <= 0x3FFFD));
}

pub fn displayWidth(cp: u21) usize {
    if (cp == 0) return 0;
    if (cp < 0x20 or (cp >= 0x7F and cp < 0xA0)) return 1;
    if (isCombining(cp)) return 0;
    if (isWide(cp)) return 2;
    return 1;
}

pub fn visualColumn(text: []const u8, byte_offset: usize, tab_width: usize) usize {
    var offset: usize = 0;
    var column: usize = 0;
    const stop = @min(byte_offset, text.len);
    while (offset < stop) {
        const decoded = decodeAt(text, offset);
        if (decoded.len == 0) break;
        if (decoded.codepoint == '\t') {
            column += tab_width - (column % tab_width);
        } else if (decoded.codepoint < 0x20 or decoded.codepoint == 0x7F) {
            column += 1;
        } else {
            column += displayWidth(decoded.codepoint);
        }
        offset += decoded.len;
    }
    return column;
}

pub fn byteOffsetForVisualColumn(text: []const u8, target: usize, tab_width: usize) usize {
    var offset: usize = 0;
    var column: usize = 0;
    while (offset < text.len) {
        const decoded = decodeAt(text, offset);
        if (decoded.len == 0) break;
        const width = if (decoded.codepoint == '\t')
            tab_width - (column % tab_width)
        else if (decoded.codepoint < 0x20 or decoded.codepoint == 0x7F)
            1
        else
            displayWidth(decoded.codepoint);

        if (column + width > target) {
            return offset;
        }
        column += width;
        offset += decoded.len;
        if (column == target) {
            return offset;
        }
    }
    return text.len;
}

pub fn digitCount(value: usize) usize {
    var v = value;
    var digits: usize = 1;
    while (v >= 10) : (v /= 10) digits += 1;
    return digits;
}

/// Finds the first byte equal to either `a` or `b` at or after `start_index`.
/// The two cases are compared in a single vectorized pass, which keeps
/// case-insensitive search close to a plain `memchr`.
fn findEitherPos(haystack: []const u8, start_index: usize, a: u8, b: u8) ?usize {
    if (start_index >= haystack.len) return null;
    if (a == b) return std.mem.findScalarPos(u8, haystack, start_index, a);

    var index = start_index;
    if (std.simd.suggestVectorLength(u8)) |block_len| {
        const Block = @Vector(block_len, u8);
        if (index + 2 * block_len <= haystack.len) {
            const first: Block = @splat(a);
            const second: Block = @splat(b);
            while (true) {
                inline for (0..2) |_| {
                    const block: Block = haystack[index..][0..block_len].*;
                    const matches = (block == first) | (block == second);
                    if (@reduce(.Or, matches)) {
                        return index + std.simd.firstTrue(matches).?;
                    }
                    index += block_len;
                }
                if (index + 2 * block_len > haystack.len) break;
            }
        }
    }
    while (index < haystack.len) : (index += 1) {
        if (haystack[index] == a or haystack[index] == b) return index;
    }
    return null;
}

pub fn indexOfIgnoreCase(haystack: []const u8, needle: []const u8) ?usize {
    if (needle.len == 0) return 0;
    if (needle.len > haystack.len) return null;

    const lower = std.ascii.toLower(needle[0]);
    const upper = std.ascii.toUpper(needle[0]);
    var position: usize = 0;
    while (position + needle.len <= haystack.len) {
        const index = findEitherPos(haystack, position, lower, upper) orelse return null;
        if (index + needle.len > haystack.len) return null;
        if (std.ascii.eqlIgnoreCase(haystack[index .. index + needle.len], needle)) {
            return index;
        }
        position = index + 1;
    }
    return null;
}

pub fn trimTrailingWhitespace(line: []const u8) []const u8 {
    var end = line.len;
    while (end > 0 and (line[end - 1] == ' ' or line[end - 1] == '\t' or line[end - 1] == '\r')) {
        end -= 1;
    }
    return line[0..end];
}

pub fn isWordByte(byte: u8) bool {
    return std.ascii.isAlphanumeric(byte) or byte == '_' or byte >= 0x80;
}

pub fn expandHome(allocator: std.mem.Allocator, path: []const u8) ![]u8 {
    // Zig 0.17's Io/process API no longer exposes a global `getenv`. The core
    // editor therefore accepts paths relative to the workspace and absolute
    // paths; `~` expansion is intentionally left to the shell.
    return allocator.dupe(u8, path);
}

test "unicode boundaries" {
    const text = "a\xC3\xA9z";
    try std.testing.expectEqual(@as(usize, 1), nextBoundary(text, 0));
    try std.testing.expectEqual(@as(usize, 3), nextBoundary(text, 1));
    try std.testing.expectEqual(@as(usize, 1), previousBoundary(text, 3));
    try std.testing.expectEqual(@as(usize, 0), previousBoundary(text, 1));
    try std.testing.expectEqual(@as(usize, 2), visualColumn(text, 3, 4));
}

test "tab visual columns" {
    const text = "a\tb";
    try std.testing.expectEqual(@as(usize, 4), visualColumn(text, 2, 4));
    try std.testing.expectEqual(@as(usize, 2), byteOffsetForVisualColumn(text, 4, 4));
}
