#pragma once

#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace hexal {

struct Style {
    std::uint8_t fg = 7;
    std::uint8_t bg = 0;
    bool bold = false;
    bool dim = false;
    bool reverse = false;

    bool operator==(const Style&) const = default;
};

struct Cell {
    char32_t ch = U' ';
    Style style;
    bool wideContinuation = false;

    bool operator==(const Cell&) const = default;
};

// A double-buffered cell grid. The renderer diffs two grids so only changed
// cells are written to the terminal.
class Screen {
public:
    Screen() = default;
    Screen(int width, int height);

    void resize(int width, int height);
    void clear(const Style& style = {});

    int width() const noexcept;
    int height() const noexcept;

    bool inBounds(int x, int y) const noexcept;
    Cell& at(int x, int y);
    const Cell& at(int x, int y) const;

    void put(int x, int y, char32_t codePoint, const Style& style, int width = 1);
    void putAscii(int x, int y, std::string_view text, const Style& style,
                  int maxWidth = 1 << 30);
    void putUtf8(int x, int y, std::string_view text, const Style& style,
                 int maxWidth = 1 << 30);
    void fill(int x, int y, int width, int height, char32_t codePoint,
              const Style& style);

    // Applies a style to a cell, keeping both halves of a wide code point in
    // sync (used for the block cursor).
    void applyStyle(int x, int y, const Style& style);

    const std::vector<Cell>& cells() const noexcept;

private:
    int width_ = 0;
    int height_ = 0;
    std::vector<Cell> cells_;
};

} // namespace hexal
