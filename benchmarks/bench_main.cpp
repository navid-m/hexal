#include "core/document.hpp"
#include "core/piece_table.hpp"
#include "ui/renderer.hpp"
#include "ui/screen.hpp"
#include "util/unicode.hpp"

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <exception>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace {

volatile std::uint64_t g_sink = 0;

struct Rng {
    std::uint64_t state;

    std::uint64_t next() {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        return state * 2685821657736338717ULL;
    }
};

struct Options {
    std::string file;
    std::size_t lines = 100000;
    int reps = 3;
    std::uint64_t seed = 0x9E3779B97F4A7C15ULL;
};

std::string makeLines(std::size_t count) {
    std::string text;
    text.reserve(count * 48);
    for (std::size_t i = 0; i < count; ++i) {
        text += "This is test line ";
        text += std::to_string(i + 1);
        text += ": Hexal performance testing.\n";
    }
    return text;
}

double median(std::vector<double> values) {
    std::sort(values.begin(), values.end());
    return values[values.size() / 2];
}

template <typename Setup, typename Body>
void measure(const char* name, std::uint64_t operations, int reps, Setup setup,
             Body body) {
    setup();
    body(); // warm-up

    std::vector<double> times;
    times.reserve(static_cast<std::size_t>(reps));
    for (int i = 0; i < reps; ++i) {
        setup();
        const auto start = std::chrono::steady_clock::now();
        body();
        const auto end = std::chrono::steady_clock::now();
        times.push_back(std::chrono::duration<double>(end - start).count());
    }

    std::cout << "METRIC\t" << name << '\t' << std::setprecision(9)
              << median(std::move(times)) << '\t' << operations << '\n';
}

Options parseOptions(int argc, char** argv) {
    Options options;
    for (int i = 1; i < argc; ++i) {
        const std::string_view argument = argv[i];
        if (argument == "--file" && i + 1 < argc) {
            options.file = argv[++i];
        } else if (argument == "--lines" && i + 1 < argc) {
            options.lines = static_cast<std::size_t>(std::stoull(argv[++i]));
        } else if (argument == "--reps" && i + 1 < argc) {
            options.reps = std::max(1, std::atoi(argv[++i]));
        } else if (argument == "--seed" && i + 1 < argc) {
            options.seed = std::stoull(argv[++i]);
        }
    }
    return options;
}

void runPieceBenchmarks(const std::string& initial, const Options& options) {
    Rng rng {options.seed};
    const std::size_t insertOps = options.lines >= 1000000 ? 100000 : 200000;
    const std::size_t eraseOps = insertOps / 2;
    const std::size_t lookupOps = options.lines >= 1000000 ? 300000 : 500000;
    const std::size_t extractOps = options.lines >= 1000000 ? 100000 : 200000;

    hexal::PieceTable insertTable;
    auto insertSetup = [&]() {
        insertTable = hexal::PieceTable{};
        insertTable.setOriginal(initial);
    };
    auto insertBody = [&]() {
        for (std::size_t i = 0; i < insertOps; ++i) {
            const std::size_t position = rng.next() %
                static_cast<std::size_t>(insertTable.size() + 1);
            insertTable.insert(position, "x");
        }
        g_sink += insertTable.size();
    };
    measure("piece_insert", insertOps, options.reps, insertSetup, insertBody);

    hexal::PieceTable eraseTable;
    auto eraseSetup = [&]() {
        eraseTable = hexal::PieceTable{};
        eraseTable.setOriginal(initial);
    };
    auto eraseBody = [&]() {
        for (std::size_t i = 0; i < eraseOps && eraseTable.size() > 8; ++i) {
            const std::size_t position = rng.next() %
                static_cast<std::size_t>(eraseTable.size() - 1);
            const std::size_t length = 1 + (rng.next() % 8);
            eraseTable.erase(position, length);
        }
        g_sink += eraseTable.size();
    };
    measure("piece_erase", eraseOps, options.reps, eraseSetup, eraseBody);

    hexal::PieceTable lookupTable;
    lookupTable.setOriginal(initial);
    auto noSetup = []() {};
    auto lookupBody = [&]() {
        for (std::size_t i = 0; i < lookupOps; ++i) {
            const std::size_t offset = rng.next() %
                static_cast<std::size_t>(lookupTable.size() + 1);
            const std::size_t line = lookupTable.lineOfOffset(offset);
            const std::size_t start = lookupTable.lineStart(line);
            g_sink += line + start;
        }
    };
    measure("line_lookup", lookupOps, options.reps, noSetup, lookupBody);

    const std::size_t lineCount = lookupTable.newlineCount();
    auto extractBody = [&]() {
        for (std::size_t i = 0; i < extractOps; ++i) {
            const std::size_t line = rng.next() % lineCount;
            const std::size_t start = lookupTable.lineStart(line);
            const std::size_t end = lookupTable.lineStart(line + 1);
            const std::size_t length = end > start ? end - start - 1 : 0;
            const std::string text = lookupTable.range(start, length);
            g_sink += text.size();
        }
    };
    measure("line_extract", extractOps, options.reps, noSetup, extractBody);
}

void runUnicodeBenchmark(const Options& options) {
    const std::string text = "ASCII αβγ 世界 😀 Привет 0123456789\n";
    const std::size_t iterations = 20000;
    const std::uint64_t operations =
        static_cast<std::uint64_t>(text.size()) * iterations;
    auto noSetup = []() {};
    auto body = [&]() {
        for (std::size_t i = 0; i < iterations; ++i) {
            std::size_t offset = 0;
            while (offset < text.size()) {
                char32_t codePoint = 0;
                std::size_t length = 0;
                hexal::unicode::decode(text, offset, codePoint, length);
                offset += length;
                g_sink += static_cast<std::uint64_t>(codePoint) +
                          static_cast<std::uint64_t>(
                              hexal::unicode::displayWidth(codePoint));
            }
        }
    };
    measure("unicode_decode", operations, options.reps, noSetup, body);
}

void runDocumentBenchmarks(const Options& options) {
    if (options.file.empty()) {
        return;
    }

    std::string error;
    auto opened = hexal::Document::open(options.file, error);
    if (!opened) {
        throw std::runtime_error("cannot open benchmark file: " + error);
    }

    auto noSetup = []() {};
    auto openBody = [&]() {
        std::string openError;
        auto document = hexal::Document::open(options.file, openError);
        if (!document) {
            throw std::runtime_error(openError);
        }
        g_sink += static_cast<std::uint64_t>(document->size());
    };
    measure("doc_open", 1, options.reps, noSetup, openBody);

    const std::filesystem::path savePath = options.file + ".bench-save";
    const int saveReps = options.lines >= 1000000 ? 2 : 3;
    auto saveBody = [&]() {
        std::string saveError;
        auto snapshot = opened->createSaveSnapshot(savePath, saveError);
        if (!snapshot) {
            throw std::runtime_error(saveError);
        }
        if (!hexal::Document::writeSnapshot(*snapshot, saveError)) {
            throw std::runtime_error(saveError);
        }
    };
    measure("doc_save", 1, saveReps, noSetup, saveBody);
    std::error_code ec;
    std::filesystem::remove(savePath, ec);

    const int columns = 120;
    const int rows = 40;
    const int textRows = rows - 2;
    const std::size_t frameCount = options.lines >= 1000000 ? 600 : 1200;
    const std::size_t lineCount = opened->lineCount();
    hexal::Screen screen(columns, rows);
    hexal::Renderer renderer;

    auto renderSetup = [&]() {
        screen.resize(columns, rows);
        screen.clear();
    };
    auto renderBody = [&]() {
        const std::size_t maxFirst =
            lineCount > static_cast<std::size_t>(textRows)
                ? lineCount - static_cast<std::size_t>(textRows)
                : 0;
        for (std::size_t frame = 0; frame < frameCount; ++frame) {
            screen.clear();
            const std::size_t first = maxFirst == 0 ? 0 : (frame * 37) % maxFirst;

            for (int row = 0; row < textRows; ++row) {
                const std::size_t line = first + static_cast<std::size_t>(row);
                if (line >= lineCount) {
                    break;
                }

                char number[24];
                const auto formatted = std::to_chars(number, number + sizeof(number),
                                                     static_cast<unsigned long long>(line + 1));
                screen.putAscii(0, row,
                                std::string_view(number, static_cast<std::size_t>(
                                    formatted.ptr - number)),
                                hexal::Style{}, 4);
                const std::string text = opened->lineText(line);
                screen.putUtf8(5, row, text, hexal::Style{}, columns - 5);
            }

            hexal::Style status;
            status.fg = 15;
            status.bg = 4;
            status.bold = true;
            screen.fill(0, rows - 2, columns, 1, U' ', status);
            screen.putAscii(0, rows - 2, "NORMAL  benchmark  file.txt", status,
                            columns);

            const std::string output = renderer.render(screen, frame == 0);
            g_sink += output.size();
        }
    };
    measure("render_viewport", frameCount, options.reps, renderSetup, renderBody);
}

} // namespace

int main(int argc, char** argv) {
    try {
        const Options options = parseOptions(argc, argv);
        std::cout << "IMPL\tcpp\n";
        std::cout << "LINES\t" << options.lines << '\n';
        std::cout << "FILE\t" << options.file << '\n';

        const std::string initial = makeLines(options.lines);
        runPieceBenchmarks(initial, options);
        runUnicodeBenchmark(options);
        runDocumentBenchmarks(options);

        std::cout << "SINK\t" << g_sink << '\n';
        return 0;
    } catch (const std::exception& ex) {
        std::cerr << "cpp benchmark failed: " << ex.what() << '\n';
        return 1;
    }
}
