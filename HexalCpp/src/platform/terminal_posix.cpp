#include "platform/terminal.hpp"

#include "util/unicode.hpp"

#include <cerrno>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <string>
#include <termios.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include <poll.h>

namespace hexal {
namespace {

volatile std::sig_atomic_t gResizeRequested = 0;

extern "C" void handleWindowChange(int) {
    gResizeRequested = 1;
}

int readByte(int timeoutMilliseconds) {
    struct pollfd descriptor {};
    descriptor.fd = STDIN_FILENO;
    descriptor.events = POLLIN;

    const int ready = ::poll(&descriptor, 1, timeoutMilliseconds);
    if (ready < 0) {
        if (errno == EINTR) {
            return -2; // interrupted, possibly by SIGWINCH
        }
        return -1;
    }
    if (ready == 0) {
        return -3; // timed out (a caller that wants to block loops)
    }

    unsigned char byte = 0;
    while (true) {
        const ssize_t count = ::read(STDIN_FILENO, &byte, 1);
        if (count == 1) {
            return static_cast<int>(byte);
        }
        if (count < 0 && errno == EINTR) {
            return -2;
        }
        return -1;
    }
}

class PosixTerminal final : public Terminal {
public:
    PosixTerminal() = default;
    ~PosixTerminal() override {
        shutdown();
    }

    bool init(std::string& error) override {
        if (!::isatty(STDIN_FILENO) || !::isatty(STDOUT_FILENO)) {
            error = "standard input/output is not an interactive terminal";
            return false;
        }

        if (::tcgetattr(STDIN_FILENO, &original_) != 0) {
            error = std::string("cannot read terminal attributes: ") + std::strerror(errno);
            return false;
        }

        struct termios raw = original_;
        raw.c_iflag &= static_cast<tcflag_t>(~(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR |
                                               IGNCR | ICRNL | IXON));
        raw.c_oflag &= static_cast<tcflag_t>(~OPOST);
        raw.c_lflag &= static_cast<tcflag_t>(~(ECHO | ECHONL | ICANON | ISIG | IEXTEN));
        raw.c_cflag &= static_cast<tcflag_t>(~(CSIZE | PARENB));
        raw.c_cflag |= CS8;
        raw.c_cc[VMIN] = 1;
        raw.c_cc[VTIME] = 0;

        if (::tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw) != 0) {
            error = std::string("cannot configure terminal: ") + std::strerror(errno);
            return false;
        }
        rawMode_ = true;

        struct sigaction action {};
        action.sa_handler = handleWindowChange;
        sigemptyset(&action.sa_mask);
        action.sa_flags = 0; // do not restart blocking reads
        if (::sigaction(SIGWINCH, &action, &oldWindowAction_) != 0) {
            error = std::string("cannot install resize handler: ") + std::strerror(errno);
            shutdown();
            return false;
        }
        windowHandlerInstalled_ = true;

        write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        initialized_ = true;
        return true;
    }

    void shutdown() override {
        if (!initialized_ && !rawMode_) {
            return;
        }

        if (initialized_) {
            write("\x1b[0m\x1b[?25h\x1b[?1049l");
        }

        if (rawMode_) {
            ::tcsetattr(STDIN_FILENO, TCSAFLUSH, &original_);
            rawMode_ = false;
        }

        if (windowHandlerInstalled_) {
            ::sigaction(SIGWINCH, &oldWindowAction_, nullptr);
            windowHandlerInstalled_ = false;
        }

        initialized_ = false;
    }

    bool readEvent(InputEvent& event, int timeoutMilliseconds) override {
        event = InputEvent{};

        while (true) {
            if (gResizeRequested != 0) {
                gResizeRequested = 0;
                event.key = Key::Resize;
                return true;
            }

            int byte = pendingByte_;
            if (byte >= 0) {
                pendingByte_ = -1;
            } else {
                byte = readByte(timeoutMilliseconds < 0 ? -1 : timeoutMilliseconds);
                if (byte == -2) {
                    continue;
                }
                if (byte == -3) {
                    event.key = Key::None;
                    return true;
                }
                if (byte < 0) {
                    return false;
                }
            }

            if (byte == 0x1B) {
                return readEscape(event);
            }

            return decodeInputByte(byte, event);
        }
    }

    void write(std::string_view text) override {
        std::size_t offset = 0;
        while (offset < text.size()) {
            const ssize_t count =
                ::write(STDOUT_FILENO, text.data() + offset, text.size() - offset);
            if (count > 0) {
                offset += static_cast<std::size_t>(count);
                continue;
            }
            if (count < 0 && errno == EINTR) {
                continue;
            }
            break;
        }
    }

    void getSize(int& columns, int& rows) const override {
        struct winsize size {};
        if (::ioctl(STDOUT_FILENO, TIOCGWINSZ, &size) != 0 ||
            size.ws_col == 0 || size.ws_row == 0) {
            columns = 80;
            rows = 24;
            return;
        }
        columns = static_cast<int>(size.ws_col);
        rows = static_cast<int>(size.ws_row);
    }

private:
    bool decodeInputByte(int byte, InputEvent& event) {
        if (byte == '\r' || byte == '\n') {
            event.key = Key::Enter;
            return true;
        }
        if (byte == 0x7F || byte == 0x08) {
            event.key = Key::Backspace;
            return true;
        }
        if (byte == 0x09) {
            event.key = Key::Tab;
            return true;
        }
        if (byte >= 0x01 && byte <= 0x1A) {
            event.key = Key::Character;
            event.character = static_cast<char32_t>('a' + (byte - 1));
            event.ctrl = true;
            return true;
        }
        if (byte < 0x20) {
            event.key = Key::Unknown;
            return true;
        }
        if (byte < 0x80) {
            event.key = Key::Character;
            event.character = static_cast<char32_t>(byte);
            return true;
        }

        // Multi-byte UTF-8 input.
        std::string bytes;
        bytes.push_back(static_cast<char>(byte));
        std::size_t needed = 1;
        if ((byte & 0xE0) == 0xC0) {
            needed = 2;
        } else if ((byte & 0xF0) == 0xE0) {
            needed = 3;
        } else if ((byte & 0xF8) == 0xF0) {
            needed = 4;
        }

        while (bytes.size() < needed) {
            const int next = readByte(50);
            if (next < 0) {
                break;
            }
            bytes.push_back(static_cast<char>(next));
        }

        char32_t cp = 0;
        std::size_t length = 0;
        if (!unicode::decode(bytes, 0, cp, length) || length != bytes.size()) {
            cp = 0xFFFD;
        }

        event.key = Key::Character;
        event.character = cp;
        return true;
    }

    bool readEscape(InputEvent& event) {
        const int next = readByte(25);
        if (next == -2) {
            if (gResizeRequested != 0) {
                gResizeRequested = 0;
            }
            event.key = Key::Resize;
            return true;
        }
        if (next < 0) {
            event.key = Key::Escape;
            return true;
        }

        if (next != '[' && next != 'O') {
            // A bare ESC followed quickly by another key. Treat it as Escape
            // and process the following byte as a separate key on the next
            // call. This makes "Esc :" reliable when typed quickly; Alt+key
            // shortcuts are not part of the MVP.
            pendingByte_ = next;
            event.key = Key::Escape;
            return true;
        }

        const bool csi = next == '[';
        std::string parameters;
        int final = 0;

        for (int i = 0; i < 32; ++i) {
            const int value = readByte(25);
            if (value < 0) {
                event.key = Key::Escape;
                return true;
            }
            if (value >= 0x40 && value <= 0x7E) {
                final = value;
                break;
            }
            if (parameters.size() < 30) {
                parameters.push_back(static_cast<char>(value));
            }
        }

        if (final == 0) {
            event.key = Key::Unknown;
            return true;
        }

        if (!csi) {
            switch (final) {
            case 'A': event.key = Key::Up; return true;
            case 'B': event.key = Key::Down; return true;
            case 'C': event.key = Key::Right; return true;
            case 'D': event.key = Key::Left; return true;
            case 'H': event.key = Key::Home; return true;
            case 'F': event.key = Key::End; return true;
            case 'P': event.key = Key::F1; return true;
            case 'Q': event.key = Key::F2; return true;
            case 'R': event.key = Key::F3; return true;
            case 'S': event.key = Key::F4; return true;
            default: event.key = Key::Unknown; return true;
            }
        }

        applyModifiers(parameters, event);

        switch (final) {
        case 'A': event.key = Key::Up; return true;
        case 'B': event.key = Key::Down; return true;
        case 'C': event.key = Key::Right; return true;
        case 'D': event.key = Key::Left; return true;
        case 'H': event.key = Key::Home; return true;
        case 'F': event.key = Key::End; return true;
        case 'Z': event.key = Key::BackTab; return true;
        case '~':
            return decodeTildeSequence(parameters, event);
        default:
            event.key = Key::Unknown;
            return true;
        }
    }

    static void applyModifiers(const std::string& parameters, InputEvent& event) {
        const std::size_t semicolon = parameters.rfind(';');
        if (semicolon == std::string::npos) {
            return;
        }

        int modifier = 0;
        for (std::size_t i = semicolon + 1; i < parameters.size(); ++i) {
            const char c = parameters[i];
            if (c < '0' || c > '9') {
                break;
            }
            modifier = modifier * 10 + (c - '0');
        }

        // xterm modifier encoding: 1 + shift(1) + alt(2) + ctrl(4).
        const int mask = modifier - 1;
        if (mask > 0) {
            event.shift = (mask & 1) != 0;
            event.alt = (mask & 2) != 0;
            event.ctrl = (mask & 4) != 0;
        }
    }

    static bool decodeTildeSequence(const std::string& parameters, InputEvent& event) {
        int code = 0;
        std::size_t i = 0;
        while (i < parameters.size() && parameters[i] >= '0' && parameters[i] <= '9') {
            code = code * 10 + (parameters[i] - '0');
            ++i;
        }

        switch (code) {
        case 1:
        case 7: event.key = Key::Home; return true;
        case 4:
        case 8: event.key = Key::End; return true;
        case 2: event.key = Key::Insert; return true;
        case 3: event.key = Key::Delete; return true;
        case 5: event.key = Key::PageUp; return true;
        case 6: event.key = Key::PageDown; return true;
        case 11: event.key = Key::F1; return true;
        case 12: event.key = Key::F2; return true;
        case 13: event.key = Key::F3; return true;
        case 14: event.key = Key::F4; return true;
        case 15: event.key = Key::F5; return true;
        case 17: event.key = Key::F6; return true;
        case 18: event.key = Key::F7; return true;
        case 19: event.key = Key::F8; return true;
        case 20: event.key = Key::F9; return true;
        case 21: event.key = Key::F10; return true;
        case 23: event.key = Key::F11; return true;
        case 24: event.key = Key::F12; return true;
        default: event.key = Key::Unknown; return true;
        }
    }

    struct termios original_ {};
    struct sigaction oldWindowAction_ {};
    bool rawMode_ = false;
    bool initialized_ = false;
    bool windowHandlerInstalled_ = false;
    int pendingByte_ = -1;
};

} // namespace

std::unique_ptr<Terminal> makeTerminal() {
    return std::make_unique<PosixTerminal>();
}

} // namespace hexal
