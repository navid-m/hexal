#include "ui/screen.hpp"

#include "util/unicode.hpp"

#include <algorithm>

namespace hexal {

Screen::Screen(int width, int height) {
    resize(width, height);
}

void Screen::resize(int width, int height) {
    width_ = std::max(0, width);
    height_ = std::max(0, height);
    cells_.assign(static_cast<std::size_t>(width_) * static_cast<std::size_t>(height_),
                  Cell{});
}

void Screen::clear(const Style& style) {
    Cell cell;
    cell.style = style;
    std::fill(cells_.begin(), cells_.end(), cell);
}

int Screen::width() const noexcept {
    return width_;
}

int Screen::height() const noexcept {
    return height_;
}

bool Screen::inBounds(int x, int y) const noexcept {
    return x >= 0 && y >= 0 && x < width_ && y < height_;
}

Cell& Screen::at(int x, int y) {
    return cells_[static_cast<std::size_t>(y) * static_cast<std::size_t>(width_) +
                  static_cast<std::size_t>(x)];
}

const Cell& Screen::at(int x, int y) const {
    return cells_[static_cast<std::size_t>(y) * static_cast<std::size_t>(width_) +
                  static_cast<std::size_t>(x)];
}

void Screen::put(int x, int y, char32_t codePoint, const Style& style, int width) {
    if (width <= 0 || !inBounds(x, y)) {
        return;
    }
    if (width > 1 && x + width > width_) {
        return;
    }

    // Repair any wide code point that we are overwriting from the side.
    if (x > 0 && at(x, y).wideContinuation) {
        at(x - 1, y) = Cell{};
    }
    if (x + 1 < width_ && at(x + 1, y).wideContinuation) {
        at(x + 1, y) = Cell{};
    }

    at(x, y) = Cell{codePoint, style, false};
    for (int offset = 1; offset < width; ++offset) {
        at(x + offset, y) = Cell{U' ', style, true};
    }
}

void Screen::putAscii(int x, int y, std::string_view text, const Style& style,
                      int maxWidth) {
    int column = 0;
    for (char c : text) {
        if (column >= maxWidth || x + column >= width_) {
            break;
        }
        put(x + column, y, static_cast<unsigned char>(c), style, 1);
        ++column;
    }
}

void Screen::putUtf8(int x, int y, std::string_view text, const Style& style,
                     int maxWidth) {
    int column = 0;
    std::size_t offset = 0;
    while (offset < text.size() && column < maxWidth && x + column < width_) {
        char32_t cp = 0;
        std::size_t length = 0;
        unicode::decode(text, offset, cp, length);
        offset += length;

        int width = unicode::displayWidth(cp);
        if (width == 0) {
            // The cell model stores one code point per cell; zero-width
            // combining marks are not rendered separately in the MVP.
            continue;
        }
        if (x + column + width > width_ || column + width > maxWidth) {
            break;
        }
        put(x + column, y, cp, style, width);
        column += width;
    }
}

void Screen::fill(int x, int y, int width, int height, char32_t codePoint,
                  const Style& style) {
    for (int row = 0; row < height; ++row) {
        for (int column = 0; column < width; ++column) {
            if (inBounds(x + column, y + row)) {
                put(x + column, y + row, codePoint, style, 1);
            }
        }
    }
}

void Screen::applyStyle(int x, int y, const Style& style) {
    if (!inBounds(x, y)) {
        return;
    }

    Cell& cell = at(x, y);
    cell.style = style;

    if (cell.wideContinuation && x > 0) {
        at(x - 1, y).style = style;
    } else if (x + 1 < width_ && at(x + 1, y).wideContinuation) {
        at(x + 1, y).style = style;
    }
}

const std::vector<Cell>& Screen::cells() const noexcept {
    return cells_;
}

} // namespace hexal
