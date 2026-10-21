#pragma once

#include <filesystem>
#include <functional>
#include <iosfwd>
#include <string>
#include <string_view>

namespace hexal::fsutil {

std::filesystem::path pathFromUtf8(std::string_view text);
std::string pathToUtf8(const std::filesystem::path& path);

#if defined(_WIN32)
std::string wideToUtf8(std::wstring_view text);
std::wstring utf8ToWide(std::string_view text);
#endif

// Reads an entire file in binary mode. Reports I/O and permission errors.
bool readFile(const std::filesystem::path& path, std::string& contents,
              std::string& error);

// Writes the target atomically: data is streamed to a temporary file in the
// same directory, flushed, and then renamed over the target. The original file
// is left untouched if writing or renaming fails.
bool atomicWrite(const std::filesystem::path& target,
                 const std::function<bool(std::ostream&)>& writer,
                 std::string& error);

} // namespace hexal::fsutil
