#include "core/command.hpp"

#include <cctype>

namespace hexal {
namespace {

bool isSpace(char c) {
    return std::isspace(static_cast<unsigned char>(c)) != 0;
}

} // namespace

ExParseResult parseExCommand(std::string_view input) {
    ExParseResult result;
    result.ok = true;

    std::size_t pos = 0;
    while (pos < input.size() && isSpace(input[pos])) {
        ++pos;
    }

    if (pos >= input.size()) {
        return result;
    }

    const std::size_t nameStart = pos;
    while (pos < input.size() && !isSpace(input[pos])) {
        ++pos;
    }
    result.command.name = std::string(input.substr(nameStart, pos - nameStart));

    while (true) {
        while (pos < input.size() && isSpace(input[pos])) {
            ++pos;
        }
        if (pos >= input.size()) {
            break;
        }

        const char first = input[pos];
        if (first == '"' || first == '\'') {
            const char quote = first;
            ++pos;
            std::string argument;

            while (pos < input.size() && input[pos] != quote) {
                if (quote == '"' && input[pos] == '\\' && pos + 1 < input.size() &&
                    input[pos + 1] == '"') {
                    argument.push_back('"');
                    pos += 2;
                    continue;
                }
                argument.push_back(input[pos]);
                ++pos;
            }

            if (pos >= input.size()) {
                result.ok = false;
                result.error = quote == '"' ? "unterminated double quote"
                                            : "unterminated single quote";
                result.command.arguments.clear();
                return result;
            }

            ++pos; // closing quote
            result.command.arguments.push_back(std::move(argument));
            continue;
        }

        const std::size_t argumentStart = pos;
        while (pos < input.size() && !isSpace(input[pos])) {
            ++pos;
        }
        result.command.arguments.emplace_back(input.substr(argumentStart, pos - argumentStart));
    }

    return result;
}

} // namespace hexal
