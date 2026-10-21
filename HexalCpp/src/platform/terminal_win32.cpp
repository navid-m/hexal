#include "platform/terminal.hpp"

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>

#ifndef ENABLE_VIRTUAL_TERMINAL_PROCESSING
#  define ENABLE_VIRTUAL_TERMINAL_PROCESSING 0x0004
#endif
#ifndef DISABLE_NEWLINE_AUTO_RETURN
#  define DISABLE_NEWLINE_AUTO_RETURN 0x0008
#endif

#include <algorithm>
#include <optional>
#include <string>

namespace hexal {
namespace {

class WindowsTerminal final : public Terminal {
public:
    WindowsTerminal() = default;
    ~WindowsTerminal() override {
        shutdown();
    }

    bool init(std::string& error) override {
        input_ = GetStdHandle(STD_INPUT_HANDLE);
        output_ = GetStdHandle(STD_OUTPUT_HANDLE);
        if (input_ == INVALID_HANDLE_VALUE || output_ == INVALID_HANDLE_VALUE) {
            error = "cannot access the console";
            return false;
        }

        if (!GetConsoleMode(input_, &originalInputMode_) ||
            !GetConsoleMode(output_, &originalOutputMode_)) {
            error = "standard input/output is not an interactive console";
            return false;
        }

        DWORD outputMode = originalOutputMode_ | ENABLE_VIRTUAL_TERMINAL_PROCESSING |
                           DISABLE_NEWLINE_AUTO_RETURN;
        if (!SetConsoleMode(output_, outputMode)) {
            error = "this console does not support virtual terminal sequences "
                    "(Windows 10 or newer is required)";
            return false;
        }

        const DWORD inputMode = ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS;
        if (!SetConsoleMode(input_, inputMode)) {
            SetConsoleMode(output_, originalOutputMode_);
            error = "cannot configure console input";
            return false;
        }

        originalOutputCodePage_ = GetConsoleOutputCP();
        originalInputCodePage_ = GetConsoleCP();
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);

        initialized_ = true;
        write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        return true;
    }

    void shutdown() override {
        if (!initialized_) {
            return;
        }

        write("\x1b[0m\x1b[?25h\x1b[?1049l");

        SetConsoleMode(input_, originalInputMode_);
        SetConsoleMode(output_, originalOutputMode_);
        SetConsoleOutputCP(originalOutputCodePage_);
        SetConsoleCP(originalInputCodePage_);
        initialized_ = false;
    }

    bool readEvent(InputEvent& event, int timeoutMilliseconds) override {
        event = InputEvent{};

        while (true) {
            if (timeoutMilliseconds >= 0) {
                const DWORD wait = WaitForSingleObject(
                    input_, static_cast<DWORD>(timeoutMilliseconds));
                if (wait == WAIT_TIMEOUT) {
                    event.key = Key::None;
                    return true;
                }
                if (wait == WAIT_FAILED) {
                    return false;
                }
            }

            INPUT_RECORD record {};
            DWORD count = 0;
            if (!ReadConsoleInputW(input_, &record, 1, &count)) {
                return false;
            }
            if (count == 0) {
                continue;
            }

            if (record.EventType == WINDOW_BUFFER_SIZE_EVENT) {
                event.key = Key::Resize;
                return true;
            }
            if (record.EventType != KEY_EVENT) {
                continue;
            }

            const KEY_EVENT_RECORD& key = record.Event.KeyEvent;
            if (!key.bKeyDown) {
                continue;
            }

            if (translateKey(key, event)) {
                return true;
            }
        }
    }

    void write(std::string_view text) override {
        std::size_t offset = 0;
        while (offset < text.size()) {
            DWORD written = 0;
            const DWORD chunk = static_cast<DWORD>(
                std::min<std::size_t>(text.size() - offset, 64 * 1024));
            if (!WriteFile(output_, text.data() + offset, chunk, &written, nullptr) ||
                written == 0) {
                break;
            }
            offset += written;
        }
    }

    void getSize(int& columns, int& rows) const override {
        CONSOLE_SCREEN_BUFFER_INFO info {};
        if (!GetConsoleScreenBufferInfo(output_, &info)) {
            columns = 80;
            rows = 24;
            return;
        }

        columns = info.srWindow.Right - info.srWindow.Left + 1;
        rows = info.srWindow.Bottom - info.srWindow.Top + 1;
        if (columns <= 0 || rows <= 0) {
            columns = 80;
            rows = 24;
        }
    }

private:
    bool translateKey(const KEY_EVENT_RECORD& key, InputEvent& event) {
        const bool ctrl = (key.dwControlKeyState & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED)) != 0;
        const bool alt = (key.dwControlKeyState & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED)) != 0;
        const bool shift = (key.dwControlKeyState & SHIFT_PRESSED) != 0;

        event.ctrl = ctrl;
        event.alt = alt;
        event.shift = shift;

        switch (key.wVirtualKeyCode) {
        case VK_RETURN: event.key = Key::Enter; return true;
        case VK_ESCAPE: event.key = Key::Escape; return true;
        case VK_BACK: event.key = Key::Backspace; return true;
        case VK_DELETE: event.key = Key::Delete; return true;
        case VK_TAB: event.key = shift ? Key::BackTab : Key::Tab; return true;
        case VK_UP: event.key = Key::Up; return true;
        case VK_DOWN: event.key = Key::Down; return true;
        case VK_LEFT: event.key = Key::Left; return true;
        case VK_RIGHT: event.key = Key::Right; return true;
        case VK_HOME: event.key = Key::Home; return true;
        case VK_END: event.key = Key::End; return true;
        case VK_PRIOR: event.key = Key::PageUp; return true;
        case VK_NEXT: event.key = Key::PageDown; return true;
        case VK_INSERT: event.key = Key::Insert; return true;
        default:
            break;
        }

        if (key.wVirtualKeyCode >= VK_F1 && key.wVirtualKeyCode <= VK_F12) {
            event.key = static_cast<Key>(static_cast<int>(Key::F1) +
                                         (key.wVirtualKeyCode - VK_F1));
            return true;
        }

        // Ctrl+letter often arrives with a control character or no character
        // at all. Translate it consistently with the POSIX implementation.
        if (ctrl && !alt && key.wVirtualKeyCode >= 'A' && key.wVirtualKeyCode <= 'Z') {
            event.key = Key::Character;
            event.character = static_cast<char32_t>('a' + (key.wVirtualKeyCode - 'A'));
            return true;
        }

        wchar_t wide = key.UnicodeChar;

        if (pendingHighSurrogate_) {
            if (wide >= 0xDC00 && wide <= 0xDFFF) {
                const char32_t high = static_cast<char32_t>(*pendingHighSurrogate_);
                const char32_t low = static_cast<char32_t>(wide);
                pendingHighSurrogate_.reset();
                event.key = Key::Character;
                event.character =
                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                return true;
            }
            pendingHighSurrogate_.reset();
        }

        if (wide >= 0xD800 && wide <= 0xDBFF) {
            pendingHighSurrogate_ = wide;
            return false;
        }

        if (wide == 0) {
            return false;
        }

        // AltGr is reported as Ctrl+Alt, but it produces ordinary characters.
        if (ctrl && alt && wide >= 0x20) {
            event.ctrl = false;
            event.alt = false;
            event.key = Key::Character;
            event.character = static_cast<char32_t>(wide);
            return true;
        }

        if (wide < 0x20) {
            if (wide >= 0x01 && wide <= 0x1A) {
                event.key = Key::Character;
                event.character = static_cast<char32_t>('a' + (wide - 1));
                event.ctrl = true;
                return true;
            }
            return false;
        }

        if (wide == 0x7F) {
            event.key = Key::Backspace;
            return true;
        }

        event.key = Key::Character;
        event.character = static_cast<char32_t>(wide);
        return true;
    }

    HANDLE input_ = INVALID_HANDLE_VALUE;
    HANDLE output_ = INVALID_HANDLE_VALUE;
    DWORD originalInputMode_ = 0;
    DWORD originalOutputMode_ = 0;
    UINT originalOutputCodePage_ = 0;
    UINT originalInputCodePage_ = 0;
    std::optional<wchar_t> pendingHighSurrogate_;
    bool initialized_ = false;
};

} // namespace

std::unique_ptr<Terminal> makeTerminal() {
    return std::make_unique<WindowsTerminal>();
}

} // namespace hexal
