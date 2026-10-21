#include "core/document.hpp"
#include "platform/terminal.hpp"
#include "ui/editor.hpp"
#include "util/filesystem.hpp"

#include <cstddef>
#include <exception>
#include <iostream>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace hexal {
namespace {

constexpr const char* kVersion = "0.1.0";

void printHelp() {
    std::cout
        << "Hexal - modal text editor (MVP)\n"
        << "\n"
        << "Usage:\n"
        << "  hexal [options] [file]\n"
        << "\n"
        << "Options:\n"
        << "  -h, --help       Show this help message and exit\n"
        << "  -v, --version    Show version information and exit\n"
        << "  --               Treat the remaining arguments as file names\n"
        << "\n"
        << "Editor commands:\n"
        << "  :w [file]        Save, optionally as a new file\n"
        << "  :wq              Save and quit\n"
        << "  :q[!]            Quit (asks before discarding changes)\n"
        << "  :e[!] <file>     Open a file\n"
        << "  :help            Show a short key reminder\n"
        << "\n"
        << "Modes and keys:\n"
        << "  i / I            Enter INSERT mode\n"
        << "  Esc              Return to NORMAL mode\n"
        << "  h j k l          Move left / down / up / right\n"
        << "  arrows           Move (Home/End/PageUp/PageDown supported)\n"
        << "  gg / ge          Go to first / last line\n"
        << "  gp               Go to the previous location\n"
        << "  x                Delete the character at the cursor\n"
        << "  u / Ctrl+R       Undo / redo\n"
        << "  :                Enter command mode\n"
        << "\n"
        << "Relative file names are resolved from the current working directory.\n";
}

int runEditor(const std::string& file) {
    std::unique_ptr<Terminal> terminal;

    try {
        Document document;
        if (!file.empty()) {
            std::string error;
            auto loaded = Document::open(fsutil::pathFromUtf8(file), error);
            if (!loaded) {
                std::cerr << "hexal: " << error << '\n';
                return 4;
            }
            document = std::move(*loaded);
        }

        terminal = makeTerminal();
        std::string error;
        if (!terminal->init(error)) {
            std::cerr << "hexal: " << error << '\n';
            return 1;
        }

        Editor editor(*terminal, std::move(document));
        const int result = editor.run();
        terminal->shutdown();
        return result;
    } catch (const std::exception& ex) {
        if (terminal) {
            terminal->shutdown();
        }
        std::cerr << "hexal: fatal error: " << ex.what() << '\n';
        return 1;
    } catch (...) {
        if (terminal) {
            terminal->shutdown();
        }
        std::cerr << "hexal: fatal error\n";
        return 1;
    }
}

} // namespace

int runHexal(const std::vector<std::string>& arguments) {
    std::string file;
    bool optionsEnded = false;

    for (std::size_t i = 1; i < arguments.size(); ++i) {
        const std::string& argument = arguments[i];

        if (!optionsEnded && argument == "--") {
            optionsEnded = true;
            continue;
        }

        if (!optionsEnded && (argument == "-h" || argument == "--help")) {
            printHelp();
            return 0;
        }

        if (!optionsEnded && (argument == "-v" || argument == "--version")) {
            std::cout << "hexal " << kVersion << '\n';
            return 0;
        }

        if (!optionsEnded && argument.size() > 1 && argument[0] == '-') {
            std::cerr << "hexal: unknown option '" << argument << "'\n\n";
            printHelp();
            return 2;
        }

        if (!file.empty()) {
            std::cerr << "hexal: the MVP opens one file at a time; use :e to switch files\n";
            return 2;
        }
        file = argument;
    }

    return runEditor(file);
}

} // namespace hexal

#if defined(_WIN32)

int wmain(int argc, wchar_t** argv) {
    std::vector<std::string> arguments;
    arguments.reserve(static_cast<std::size_t>(argc));
    for (int i = 0; i < argc; ++i) {
        arguments.push_back(hexal::fsutil::wideToUtf8(argv[i]));
    }
    return hexal::runHexal(arguments);
}

#else

int main(int argc, char** argv) {
    std::vector<std::string> arguments;
    arguments.reserve(static_cast<std::size_t>(argc));
    for (int i = 0; i < argc; ++i) {
        arguments.emplace_back(argv[i]);
    }
    return hexal::runHexal(arguments);
}

#endif
