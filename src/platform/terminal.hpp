#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <string_view>

namespace hexal {

enum class Key {
    Character,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Resize,
    Unknown
};

struct InputEvent {
    Key key = Key::Unknown;
    char32_t character = 0;
    bool ctrl = false;
    bool alt = false;
    bool shift = false;
};

// Platform terminal abstraction. The POSIX implementation uses termios and
// ANSI sequences; the Windows implementation uses the console API with
// ENABLE_VIRTUAL_TERMINAL_PROCESSING for output and ReadConsoleInputW for
// keyboard events.
class Terminal {
public:
    virtual ~Terminal() = default;

    virtual bool init(std::string& error) = 0;
    virtual void shutdown() = 0;
    virtual bool readEvent(InputEvent& event) = 0;
    virtual void write(std::string_view text) = 0;
    virtual void getSize(int& columns, int& rows) const = 0;
};

std::unique_ptr<Terminal> makeTerminal();

} // namespace hexal
