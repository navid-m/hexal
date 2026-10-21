#include "util/unicode.hpp"

#include <algorithm>

namespace hexal::unicode {
namespace {

constexpr char32_t kReplacement = 0xFFFD;

bool isContinuation(unsigned char byte) {
    return (byte & 0xC0U) == 0x80U;
}

bool inRange(char32_t value, char32_t low, char32_t high) {
    return value >= low && value <= high;
}

} // namespace

bool decode(std::string_view text, std::size_t offset, char32_t& codePoint,
            std::size_t& length) {
    length = 1;
    codePoint = kReplacement;

    if (offset >= text.size()) {
        return false;
    }

    const auto first = static_cast<unsigned char>(text[offset]);
    if (first < 0x80U) {
        codePoint = first;
        return true;
    }

    std::size_t needed = 0;
    char32_t value = 0;
    char32_t minimum = 0;

    if ((first & 0xE0U) == 0xC0U) {
        needed = 1;
        value = first & 0x1FU;
        minimum = 0x80;
    } else if ((first & 0xF0U) == 0xE0U) {
        needed = 2;
        value = first & 0x0FU;
        minimum = 0x800;
    } else if ((first & 0xF8U) == 0xF0U) {
        needed = 3;
        value = first & 0x07U;
        minimum = 0x10000;
    } else {
        return false;
    }

    if (offset + needed >= text.size()) {
        return false;
    }

    for (std::size_t i = 1; i <= needed; ++i) {
        if (offset + i >= text.size()) {
            return false;
        }
        const auto byte = static_cast<unsigned char>(text[offset + i]);
        if (!isContinuation(byte)) {
            return false;
        }
        value = (value << 6U) | (byte & 0x3FU);
    }

    if (value < minimum || value > 0x10FFFF) {
        return false;
    }
    if (value >= 0xD800 && value <= 0xDFFF) {
        return false;
    }

    codePoint = value;
    length = needed + 1;
    return true;
}

std::size_t encode(char32_t codePoint, char out[4]) {
    if (codePoint > 0x10FFFF || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) {
        codePoint = kReplacement;
    }

    if (codePoint < 0x80) {
        out[0] = static_cast<char>(codePoint);
        return 1;
    }
    if (codePoint < 0x800) {
        out[0] = static_cast<char>(0xC0 | (codePoint >> 6));
        out[1] = static_cast<char>(0x80 | (codePoint & 0x3F));
        return 2;
    }
    if (codePoint < 0x10000) {
        out[0] = static_cast<char>(0xE0 | (codePoint >> 12));
        out[1] = static_cast<char>(0x80 | ((codePoint >> 6) & 0x3F));
        out[2] = static_cast<char>(0x80 | (codePoint & 0x3F));
        return 3;
    }
    out[0] = static_cast<char>(0xF0 | (codePoint >> 18));
    out[1] = static_cast<char>(0x80 | ((codePoint >> 12) & 0x3F));
    out[2] = static_cast<char>(0x80 | ((codePoint >> 6) & 0x3F));
    out[3] = static_cast<char>(0x80 | (codePoint & 0x3F));
    return 4;
}

void append(std::string& out, char32_t codePoint) {
    char buffer[4];
    const std::size_t length = encode(codePoint, buffer);
    out.append(buffer, length);
}

std::size_t previousBoundary(std::string_view text, std::size_t offset) {
    if (offset == 0) {
        return 0;
    }
    offset = std::min(offset, text.size());

    std::size_t candidate = offset - 1;
    std::size_t back = 0;
    while (candidate > 0 && back < 3 &&
           isContinuation(static_cast<unsigned char>(text[candidate]))) {
        --candidate;
        ++back;
    }

    char32_t cp = 0;
    std::size_t length = 0;
    if (decode(text, candidate, cp, length) && candidate + length == offset) {
        return candidate;
    }
    return offset - 1;
}

std::size_t nextBoundary(std::string_view text, std::size_t offset) {
    if (offset >= text.size()) {
        return text.size();
    }
    char32_t cp = 0;
    std::size_t length = 0;
    decode(text, offset, cp, length);
    return std::min(text.size(), offset + length);
}

bool isWide(char32_t cp) {
    return inRange(cp, 0x1100, 0x115F) || inRange(cp, 0x2329, 0x232A) ||
           inRange(cp, 0x2E80, 0x303E) || inRange(cp, 0x3041, 0x33FF) ||
           inRange(cp, 0x3400, 0x4DBF) || inRange(cp, 0x4E00, 0x9FFF) ||
           inRange(cp, 0xA000, 0xA4CF) || inRange(cp, 0xA960, 0xA97F) ||
           inRange(cp, 0xAC00, 0xD7A3) || inRange(cp, 0xF900, 0xFAFF) ||
           inRange(cp, 0xFE10, 0xFE19) || inRange(cp, 0xFE30, 0xFE6F) ||
           inRange(cp, 0xFF00, 0xFF60) || inRange(cp, 0xFFE0, 0xFFE6) ||
           inRange(cp, 0x1F300, 0x1FAFF) || inRange(cp, 0x20000, 0x3FFFD);
}

int displayWidth(char32_t cp) {
    if (cp == 0 || cp < 0x20 || (cp >= 0x7F && cp < 0xA0)) {
        return 0;
    }
    // Combining marks, zero-width joiners and variation selectors.
    if (inRange(cp, 0x0300, 0x036F) || inRange(cp, 0x0483, 0x0489) ||
        inRange(cp, 0x0591, 0x05BD) || inRange(cp, 0x0610, 0x061A) ||
        inRange(cp, 0x064B, 0x065F) || inRange(cp, 0x0670, 0x0670) ||
        inRange(cp, 0x06D6, 0x06DC) || inRange(cp, 0x0E31, 0x0E31) ||
        inRange(cp, 0x0E34, 0x0E3A) || inRange(cp, 0x1AB0, 0x1AFF) ||
        inRange(cp, 0x1DC0, 0x1DFF) || inRange(cp, 0x200B, 0x200F) ||
        inRange(cp, 0x20D0, 0x20FF) || inRange(cp, 0xFE00, 0xFE0F) ||
        inRange(cp, 0xFE20, 0xFE2F) || inRange(cp, 0xE0100, 0xE01EF)) {
        return 0;
    }
    return isWide(cp) ? 2 : 1;
}

std::size_t visualColumn(std::string_view line, std::size_t byteOffset, int tabWidth) {
    tabWidth = std::max(1, tabWidth);
    byteOffset = std::min(byteOffset, line.size());

    std::size_t column = 0;
    std::size_t offset = 0;
    while (offset < byteOffset) {
        char32_t cp = 0;
        std::size_t length = 0;
        decode(line, offset, cp, length);
        offset += length;

        if (cp == '\t') {
            column += static_cast<std::size_t>(tabWidth) -
                      (column % static_cast<std::size_t>(tabWidth));
        } else {
            const int width = displayWidth(cp);
            if (width > 0) {
                column += static_cast<std::size_t>(width);
            }
        }
    }
    return column;
}

std::size_t byteOffsetForVisualColumn(std::string_view line, std::size_t column,
                                      int tabWidth) {
    tabWidth = std::max(1, tabWidth);

    std::size_t visual = 0;
    std::size_t offset = 0;
    while (offset < line.size()) {
        char32_t cp = 0;
        std::size_t length = 0;
        decode(line, offset, cp, length);

        std::size_t next = visual;
        if (cp == '\t') {
            next += static_cast<std::size_t>(tabWidth) -
                    (next % static_cast<std::size_t>(tabWidth));
        } else {
            const int width = displayWidth(cp);
            if (width > 0) {
                next += static_cast<std::size_t>(width);
            }
        }

        if (next > column) {
            return offset;
        }
        visual = next;
        offset += length;
        if (visual == column) {
            return offset;
        }
    }
    return line.size();
}

} // namespace hexal::unicode
