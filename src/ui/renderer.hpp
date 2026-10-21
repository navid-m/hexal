#pragma once

#include "ui/screen.hpp"

#include <string>
#include <vector>

namespace hexal {

// Produces ANSI escape sequences for the changed cells only.
class Renderer {
public:
    std::string render(const Screen& screen, bool forceFullRedraw = false);

private:
    static void appendStyle(std::string& out, const Style& style);
    static void appendCodePoint(std::string& out, char32_t codePoint);

    std::vector<Cell> previous_;
    int previousWidth_ = 0;
    int previousHeight_ = 0;
};

} // namespace hexal
