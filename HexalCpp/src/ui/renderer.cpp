#include "ui/renderer.hpp"

#include "util/unicode.hpp"

#include <algorithm>

namespace hexal {
namespace {

void appendNumber(std::string& out, int value) {
    out += std::to_string(value);
}

} // namespace

void Renderer::appendStyle(std::string& out, const Style& style) {
    out += "\x1b[0";

    if (style.bold) {
        out += ";1";
    }
    if (style.dim) {
        out += ";2";
    }
    if (style.reverse) {
        out += ";7";
    }

    const unsigned fg = style.fg & 0x0FU;
    if (fg < 8) {
        out += ";3";
        appendNumber(out, static_cast<int>(fg));
    } else {
        out += ";9";
        appendNumber(out, static_cast<int>(fg - 8));
    }

    const unsigned bg = style.bg & 0x0FU;
    if (bg < 8) {
        out += ";4";
        appendNumber(out, static_cast<int>(bg));
    } else {
        out += ";10";
        appendNumber(out, static_cast<int>(bg - 8));
    }

    out += 'm';
}

void Renderer::appendCodePoint(std::string& out, char32_t codePoint) {
    char buffer[4];
    const std::size_t length = unicode::encode(codePoint, buffer);
    out.append(buffer, length);
}

std::string Renderer::render(const Screen& screen, bool forceFullRedraw) {
    const int width = screen.width();
    const int height = screen.height();

    if (width <= 0 || height <= 0) {
        previous_.clear();
        previousWidth_ = width;
        previousHeight_ = height;
        return {};
    }

    const bool sizeChanged =
        width != previousWidth_ || height != previousHeight_ ||
        previous_.size() != static_cast<std::size_t>(width) *
                                  static_cast<std::size_t>(height);

    std::string out;
    if (sizeChanged) {
        out += "\x1b[2J";
        forceFullRedraw = true;
    }

    bool wroteAny = sizeChanged;

    for (int y = 0; y < height; ++y) {
        int x = 0;
        while (x < width) {
            const std::size_t index =
                static_cast<std::size_t>(y) * static_cast<std::size_t>(width) +
                static_cast<std::size_t>(x);
            const bool changed = forceFullRedraw ||
                                 index >= previous_.size() ||
                                 !(screen.cells()[index] == previous_[index]);

            if (!changed || screen.cells()[index].wideContinuation) {
                ++x;
                continue;
            }

            const int runStart = x;
            while (x < width) {
                const std::size_t runIndex =
                    static_cast<std::size_t>(y) * static_cast<std::size_t>(width) +
                    static_cast<std::size_t>(x);
                const bool runChanged =
                    forceFullRedraw || runIndex >= previous_.size() ||
                    !(screen.cells()[runIndex] == previous_[runIndex]);
                if (!runChanged) {
                    break;
                }
                ++x;
            }

            wroteAny = true;
            out += "\x1b[";
            appendNumber(out, y + 1);
            out += ';';
            appendNumber(out, runStart + 1);
            out += 'H';

            Style current;
            bool currentValid = false;

            for (int column = runStart; column < x; ++column) {
                const std::size_t cellIndex =
                    static_cast<std::size_t>(y) * static_cast<std::size_t>(width) +
                    static_cast<std::size_t>(column);
                const Cell& cell = screen.cells()[cellIndex];
                if (cell.wideContinuation) {
                    continue;
                }

                if (!currentValid || !(cell.style == current)) {
                    appendStyle(out, cell.style);
                    current = cell.style;
                    currentValid = true;
                }
                appendCodePoint(out, cell.ch);
            }
        }
    }

    if (wroteAny) {
        out += "\x1b[0m";
    }

    previous_.assign(screen.cells().begin(), screen.cells().end());
    previousWidth_ = width;
    previousHeight_ = height;

    return out;
}

} // namespace hexal
