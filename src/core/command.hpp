#pragma once

#include <string>
#include <string_view>
#include <vector>

namespace hexal {

struct ExCommand {
    std::string name;
    std::vector<std::string> arguments;
};

struct ExParseResult {
    bool ok = false;
    ExCommand command;
    std::string error;
};

// Parses the text after the leading ':' of an Ex command. Supports single and
// double quoted arguments (filenames with spaces). Inside double quotes only
// \" is an escape; every other backslash is literal so Windows and UNC paths
// are not mangled. Backslashes outside quotes are literal as well.
ExParseResult parseExCommand(std::string_view input);

} // namespace hexal
