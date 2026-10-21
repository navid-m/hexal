#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>

namespace hexal::unicode {

// Decodes one UTF-8 code point starting at `offset`.
// Invalid sequences are reported as U+FFFD and consume exactly one byte.
bool decode(std::string_view text, std::size_t offset, char32_t& codePoint,
            std::size_t& length);

std::size_t encode(char32_t codePoint, char out[4]);
void append(std::string& out, char32_t codePoint);

// Byte offset of the previous / next code point boundary. These never move
// across the supplied view's boundaries.
std::size_t previousBoundary(std::string_view text, std::size_t offset);
std::size_t nextBoundary(std::string_view text, std::size_t offset);

// Terminal display width in cells. Tabs are handled by the callers because
// their width depends on the current column.
int displayWidth(char32_t codePoint);
bool isWide(char32_t codePoint);

// Converts between byte offsets and visual columns inside one line.
std::size_t visualColumn(std::string_view line, std::size_t byteOffset, int tabWidth);
std::size_t byteOffsetForVisualColumn(std::string_view line, std::size_t column,
                                      int tabWidth);

} // namespace hexal::unicode
