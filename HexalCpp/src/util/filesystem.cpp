#include "util/filesystem.hpp"

#include <atomic>
#include <cerrno>
#include <cstdio>
#include <exception>
#include <fstream>
#include <system_error>

#if defined(_WIN32)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
#else
#  include <sys/stat.h>
#  include <unistd.h>
#endif

namespace hexal::fsutil {
namespace {

std::string systemErrorMessage(int code) {
#if defined(_WIN32)
    return std::error_code(code, std::system_category()).message();
#else
    return std::error_code(code, std::generic_category()).message();
#endif
}

std::string fileNameForTemp(const std::filesystem::path& target) {
    std::string name = pathToUtf8(target.filename());
    if (name.empty()) {
        name = "hexal";
    }
    return name;
}

unsigned long processId() {
#if defined(_WIN32)
    return static_cast<unsigned long>(GetCurrentProcessId());
#else
    return static_cast<unsigned long>(getpid());
#endif
}

bool replaceFile(const std::filesystem::path& source,
                 const std::filesystem::path& target, std::string& error) {
#if defined(_WIN32)
    if (!MoveFileExW(source.c_str(), target.c_str(),
                     MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
        const DWORD code = GetLastError();
        error = "cannot replace '" + pathToUtf8(target) + "': " +
                systemErrorMessage(static_cast<int>(code));
        return false;
    }
    return true;
#else
    if (std::rename(source.c_str(), target.c_str()) != 0) {
        error = "cannot replace '" + pathToUtf8(target) + "': " +
                systemErrorMessage(errno);
        return false;
    }
    return true;
#endif
}

void preservePermissions(const std::filesystem::path& target,
                         const std::filesystem::path& temp) {
#if !defined(_WIN32)
    struct stat info {};
    if (::stat(target.c_str(), &info) == 0) {
        ::chmod(temp.c_str(), info.st_mode & 07777);
    }
#else
    (void)target;
    (void)temp;
#endif
}

} // namespace

std::filesystem::path pathFromUtf8(std::string_view text) {
#if defined(_WIN32)
    return std::filesystem::path(utf8ToWide(text));
#else
    return std::filesystem::path(std::string(text));
#endif
}

std::string pathToUtf8(const std::filesystem::path& path) {
#if defined(_WIN32)
    return wideToUtf8(path.wstring());
#else
    return path.string();
#endif
}

#if defined(_WIN32)

std::wstring utf8ToWide(std::string_view text) {
    if (text.empty()) {
        return {};
    }
    const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(),
                                         static_cast<int>(text.size()), nullptr, 0);
    if (size <= 0) {
        // Fall back to lenient conversion so that unusual but writable names
        // are still usable.
        const int fallback = MultiByteToWideChar(CP_UTF8, 0, text.data(),
                                                 static_cast<int>(text.size()),
                                                 nullptr, 0);
        if (fallback <= 0) {
            return {};
        }
        std::wstring result(static_cast<std::size_t>(fallback), L'\0');
        MultiByteToWideChar(CP_UTF8, 0, text.data(), static_cast<int>(text.size()),
                            result.data(), fallback);
        return result;
    }
    std::wstring result(static_cast<std::size_t>(size), L'\0');
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(),
                        static_cast<int>(text.size()), result.data(), size);
    return result;
}

std::string wideToUtf8(std::wstring_view text) {
    if (text.empty()) {
        return {};
    }
    const int size = WideCharToMultiByte(CP_UTF8, 0, text.data(),
                                         static_cast<int>(text.size()), nullptr, 0,
                                         nullptr, nullptr);
    if (size <= 0) {
        return {};
    }
    std::string result(static_cast<std::size_t>(size), '\0');
    WideCharToMultiByte(CP_UTF8, 0, text.data(), static_cast<int>(text.size()),
                        result.data(), size, nullptr, nullptr);
    return result;
}

#endif

bool readFile(const std::filesystem::path& path, std::string& contents,
              std::string& error) {
    if (path.empty()) {
        error = "empty file name";
        return false;
    }

    std::error_code ec;
    const auto status = std::filesystem::status(path, ec);
    if (ec) {
        error = "cannot access '" + pathToUtf8(path) + "': " + ec.message();
        return false;
    }
    if (std::filesystem::is_directory(status)) {
        error = "'" + pathToUtf8(path) + "' is a directory";
        return false;
    }

    std::ifstream in(path, std::ios::binary);
    if (!in) {
        error = "cannot open '" + pathToUtf8(path) + "' for reading";
        return false;
    }

    contents.clear();
    const auto fileSize = std::filesystem::file_size(path, ec);
    if (!ec) {
        contents.reserve(static_cast<std::size_t>(fileSize));
    }

    char buffer[64 * 1024];
    while (in) {
        in.read(buffer, sizeof(buffer));
        const std::streamsize count = in.gcount();
        if (count > 0) {
            contents.append(buffer, static_cast<std::size_t>(count));
        }
    }

    if (in.bad()) {
        error = "error while reading '" + pathToUtf8(path) + "'";
        return false;
    }

    return true;
}

bool atomicWrite(const std::filesystem::path& target,
                 const std::function<bool(std::ostream&)>& writer,
                 std::string& error) {
    if (target.empty()) {
        error = "empty file name";
        return false;
    }

    std::error_code ec;
    const auto parent = target.has_parent_path() ? target.parent_path()
                                                 : std::filesystem::path(".");
    if (!std::filesystem::is_directory(parent, ec)) {
        error = "directory does not exist: '" + pathToUtf8(parent) + "'";
        return false;
    }

    if (std::filesystem::is_directory(target, ec)) {
        error = "'" + pathToUtf8(target) + "' is a directory";
        return false;
    }

    static std::atomic<unsigned long> counter{0};

    std::filesystem::path temp;
    for (int attempt = 0; attempt < 100; ++attempt) {
        const std::string suffix = ".hexal-tmp-" + std::to_string(processId()) + "-" +
                                   std::to_string(counter.fetch_add(1));
        temp = parent / (fileNameForTemp(target) + suffix);
        if (!std::filesystem::exists(temp, ec)) {
            break;
        }
    }

    if (std::filesystem::exists(temp, ec)) {
        error = "cannot create a unique temporary file in '" + pathToUtf8(parent) + "'";
        return false;
    }

    {
        std::ofstream out(temp, std::ios::binary | std::ios::trunc);
        if (!out) {
            error = "cannot create temporary file in '" + pathToUtf8(parent) + "'";
            return false;
        }

        bool wrote = false;
        try {
            wrote = writer(out);
        } catch (const std::exception& ex) {
            error = std::string("write failed: ") + ex.what();
        } catch (...) {
            error = "write failed";
        }

        out.flush();
        if (!wrote || !out) {
            if (error.empty()) {
                error = "write failed for '" + pathToUtf8(target) + "'";
            }
            out.close();
            std::filesystem::remove(temp, ec);
            return false;
        }
        out.close();
        if (!out) {
            error = "failed to flush '" + pathToUtf8(target) + "'";
            std::filesystem::remove(temp, ec);
            return false;
        }
    }

    preservePermissions(target, temp);

    if (!replaceFile(temp, target, error)) {
        std::filesystem::remove(temp, ec);
        return false;
    }

    return true;
}

} // namespace hexal::fsutil
