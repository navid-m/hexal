#include "core/command.hpp"
#include "core/document.hpp"
#include "core/piece_table.hpp"
#include "util/filesystem.hpp"
#include "util/unicode.hpp"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <random>
#include <sstream>
#include <string>
#include <string_view>
#include <vector>

namespace {

int gChecks = 0;
int gFailures = 0;

void check(bool condition, const char* expression, const char* file, int line) {
    ++gChecks;
    if (!condition) {
        ++gFailures;
        std::cerr << file << ':' << line << ": CHECK failed: " << expression << '\n';
    }
}

template <typename Actual, typename Expected>
void checkEqual(const Actual& actual, const Expected& expected, const char* actualText,
                const char* expectedText, const char* file, int line) {
    ++gChecks;
    if (!(actual == expected)) {
        ++gFailures;
        std::cerr << file << ':' << line << ": CHECK_EQ failed: " << actualText
                  << " == " << expectedText << "\n  actual:   " << actual
                  << "\n  expected: " << expected << '\n';
    }
}

#define CHECK(expr) check(static_cast<bool>(expr), #expr, __FILE__, __LINE__)
#define CHECK_EQ(actual, expected) \
    checkEqual((actual), (expected), #actual, #expected, __FILE__, __LINE__)

class TempDir {
public:
    TempDir() {
        const auto stamp = std::chrono::steady_clock::now().time_since_epoch().count();
        static unsigned counter = 0;
        directory_ = std::filesystem::temp_directory_path() /
                     ("hexal-tests-" + std::to_string(stamp) + "-" +
                      std::to_string(counter++));
        std::error_code ec;
        std::filesystem::create_directories(directory_, ec);
    }

    ~TempDir() {
        std::error_code ec;
        std::filesystem::remove_all(directory_, ec);
    }

    std::filesystem::path path(std::string_view name) const {
        return directory_ / std::filesystem::path(std::string(name));
    }

private:
    std::filesystem::path directory_;
};

bool writeBytes(const std::filesystem::path& path, std::string_view data) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (!out) {
        return false;
    }
    out.write(data.data(), static_cast<std::streamsize>(data.size()));
    return static_cast<bool>(out);
}

std::string readBytes(const std::filesystem::path& path) {
    std::string contents;
    std::string error;
    if (!hexal::fsutil::readFile(path, contents, error)) {
        return {};
    }
    return contents;
}

std::size_t expectedLineStart(std::string_view text, std::size_t line) {
    if (line == 0) {
        return 0;
    }
    std::size_t seen = 0;
    for (std::size_t i = 0; i < text.size(); ++i) {
        if (text[i] == '\n') {
            ++seen;
            if (seen == line) {
                return i + 1;
            }
        }
    }
    return text.size();
}

void testPieceTableBasics() {
    hexal::PieceTable table;
    table.setOriginal("hello world\nsecond line\n");

    CHECK_EQ(table.size(), std::string("hello world\nsecond line\n").size());
    CHECK_EQ(table.newlineCount(), std::size_t{2});
    CHECK_EQ(table.lineStart(0), std::size_t{0});
    CHECK_EQ(table.lineStart(1), std::size_t{12});
    CHECK_EQ(table.lineStart(2), std::size_t{24});
    CHECK_EQ(table.lineStart(3), std::size_t{24});
    CHECK_EQ(table.lineOfOffset(0), std::size_t{0});
    CHECK_EQ(table.lineOfOffset(11), std::size_t{0});
    CHECK_EQ(table.lineOfOffset(12), std::size_t{1});
    CHECK_EQ(table.lineOfOffset(24), std::size_t{2});

    table.insert(5, ",");
    CHECK_EQ(table.toString(), std::string("hello, world\nsecond line\n"));

    table.erase(0, 5);
    CHECK_EQ(table.toString(), std::string(", world\nsecond line\n"));

    table.insert(0, "well");
    CHECK_EQ(table.toString(), std::string("well, world\nsecond line\n"));

    CHECK_EQ(table.range(6, 5), std::string("world"));
    CHECK_EQ(table.byteAt(0), 'w');
    CHECK_EQ(table.byteAt(table.size()), '\0');

    table.erase(0, table.size());
    CHECK_EQ(table.size(), std::size_t{0});
    CHECK_EQ(table.newlineCount(), std::size_t{0});
    CHECK_EQ(table.lineStart(0), std::size_t{0});
}

void testPieceTableRandomized() {
    std::string reference;
    for (int i = 0; i < 80; ++i) {
        reference += "line " + std::to_string(i) + "\n";
    }

    hexal::PieceTable table;
    table.setOriginal(reference);

    const std::vector<std::string> insertions = {
        "x", "YZ", "\n", "alpha\n", "\t", " ", "\r\n", "123", "longer text here"};

    std::mt19937 rng(0xC0FFEEU);
    for (int step = 0; step < 3000; ++step) {
        const bool doInsert = reference.empty() || (rng() % 3U) != 0U;
        if (doInsert) {
            const std::size_t position = rng() % (reference.size() + 1);
            const std::string& text = insertions[rng() % insertions.size()];
            reference.insert(position, text);
            table.insert(position, text);
        } else {
            const std::size_t position = rng() % reference.size();
            const std::size_t maxLength =
                std::min<std::size_t>(8, reference.size() - position);
            const std::size_t length = 1 + rng() % maxLength;
            reference.erase(position, length);
            table.erase(position, length);
        }

        if (step % 89 == 0) {
            CHECK_EQ(table.toString(), reference);
        }
    }

    CHECK_EQ(table.toString(), reference);
    CHECK_EQ(table.newlineCount(),
              static_cast<std::size_t>(
                  std::count(reference.begin(), reference.end(), '\n')));

    const std::size_t lineCount = table.newlineCount() + 1;
    for (std::size_t line = 0; line < lineCount; line += 17) {
        CHECK_EQ(table.lineStart(line), expectedLineStart(reference, line));
    }
    CHECK_EQ(table.lineStart(lineCount), reference.size());

    for (int i = 0; i < 200; ++i) {
        const std::size_t offset = rng() % (reference.size() + 1);
        const auto expected =
            static_cast<std::size_t>(std::count(reference.begin(),
                                                reference.begin() +
                                                    static_cast<std::ptrdiff_t>(offset),
                                                '\n'));
        CHECK_EQ(table.lineOfOffset(offset), expected);
    }
}

void testSnapshotIsolation() {
    hexal::PieceTable table;
    table.setOriginal("hello\n");
    table.insert(0, "A");

    const hexal::PieceTable::Snapshot snapshot = table.snapshot();
    table.insert(0, "B");
    table.insert(table.size(), "tail");

    std::ostringstream out;
    CHECK(snapshot.writeTo(out));
    CHECK_EQ(out.str(), std::string("Ahello\n"));
    CHECK_EQ(table.toString(), std::string("BAhello\ntail"));

    // Appending after a snapshot must not affect the snapshot, even when the
    // add buffer grows well beyond its previous size.
    const hexal::PieceTable::Snapshot beforeLargeAppend = table.snapshot();
    std::string payload(1500000, 'z');
    table.insert(table.size(), payload);

    std::ostringstream largeOut;
    CHECK(beforeLargeAppend.writeTo(largeOut));
    CHECK_EQ(largeOut.str(), std::string("BAhello\ntail"));

    TempDir directory;
    const auto path = directory.path("snapshot-doc.txt");
    CHECK(writeBytes(path, "base\n"));

    std::string error;
    auto document = hexal::Document::open(path, error);
    CHECK(document.has_value());
    if (!document) {
        return;
    }

    document->insert(0, "x");
    const auto savePath = directory.path("saved-snapshot.txt");
    auto documentSnapshot = document->createSaveSnapshot(savePath, error);
    CHECK(documentSnapshot.has_value());
    if (!documentSnapshot) {
        return;
    }

    document->insert(0, "y");
    CHECK(hexal::Document::writeSnapshot(*documentSnapshot, error));
    CHECK_EQ(readBytes(savePath), std::string("xbase\n"));

    document->markSaved(documentSnapshot->path, documentSnapshot->pose,
                        documentSnapshot->revision);
    CHECK(document->isDirty());
    CHECK_EQ(hexal::fsutil::pathToUtf8(*document->path()),
             hexal::fsutil::pathToUtf8(savePath));

    // Undo plus a new edit can return the undo stack to the same depth with
    // different contents. The revision check must keep the document dirty.
    const auto branchPath = directory.path("branch.txt");
    CHECK(writeBytes(branchPath, "base\n"));
    auto branch = hexal::Document::open(branchPath, error);
    CHECK(branch.has_value());
    if (!branch) {
        return;
    }

    branch->insert(0, "x");
    const auto branchSavePath = directory.path("branch-saved.txt");
    auto branchSnapshot = branch->createSaveSnapshot(branchSavePath, error);
    CHECK(branchSnapshot.has_value());
    if (!branchSnapshot) {
        return;
    }

    CHECK(branch->undo().has_value());
    branch->insert(0, "y");
    branch->markSaved(branchSavePath, branchSnapshot->pose,
                      branchSnapshot->revision);
    CHECK(branch->isDirty());
}

void testLargeLineIndex() {
    std::string large;
    large.reserve(600000);
    for (int i = 0; i < 100000; ++i) {
        large += "line\n";
    }

    hexal::PieceTable table;
    table.setOriginal(large);
    CHECK_EQ(table.newlineCount(), std::size_t{100000});
    CHECK_EQ(table.lineStart(50000), std::size_t{250000});
    CHECK_EQ(table.lineOfOffset(250000), std::size_t{50000});
    CHECK_EQ(table.lineStart(99999), std::size_t{499995});
    CHECK_EQ(table.lineStart(100000), large.size());

    table.insert(250000, "prefix\n");
    CHECK_EQ(table.lineStart(50000), std::size_t{250000});
    CHECK_EQ(table.lineStart(50001), std::size_t{250007});
    CHECK_EQ(table.lineOfOffset(250007), std::size_t{50001});
}

void testUnicode() {
    using namespace hexal::unicode;

    char32_t cp = 0;
    std::size_t length = 0;
    CHECK(decode("\xC3\xA9", 0, cp, length));
    CHECK(cp == char32_t{0xE9});
    CHECK_EQ(length, std::size_t{2});

    CHECK(!decode("\x80", 0, cp, length));
    CHECK(cp == char32_t{0xFFFD});
    CHECK_EQ(length, std::size_t{1});

    CHECK(decode("\xE7\x95\x8C", 0, cp, length));
    CHECK(cp == char32_t{0x754C});
    CHECK_EQ(displayWidth(cp), 2);
    CHECK_EQ(displayWidth(U'a'), 1);

    CHECK_EQ(previousBoundary("\xC3\xA9", 2), std::size_t{0});
    CHECK_EQ(nextBoundary("\xC3\xA9", 0), std::size_t{2});

    const std::string line = "a\t\xE7\x95\x8C";
    CHECK_EQ(visualColumn(line, line.size(), 4), std::size_t{6});
    CHECK_EQ(byteOffsetForVisualColumn(line, 4, 4), std::size_t{2});
    CHECK_EQ(byteOffsetForVisualColumn(line, 5, 4), std::size_t{2});
    CHECK_EQ(byteOffsetForVisualColumn(line, 6, 4), line.size());

    char buffer[4];
    CHECK_EQ(encode(U'A', buffer), std::size_t{1});
    CHECK_EQ(encode(char32_t{0x754C}, buffer), std::size_t{3});
}

void testCommandParsing() {
    auto result = hexal::parseExCommand("e \"src/my file.cpp\"");
    CHECK(result.ok);
    CHECK_EQ(result.command.name, std::string("e"));
    CHECK_EQ(result.command.arguments.size(), std::size_t{1});
    CHECK_EQ(result.command.arguments[0], std::string("src/my file.cpp"));

    result = hexal::parseExCommand("w 'a b.txt'");
    CHECK(result.ok);
    CHECK_EQ(result.command.arguments[0], std::string("a b.txt"));

    result = hexal::parseExCommand("w C:\\tmp\\file.txt");
    CHECK(result.ok);
    CHECK_EQ(result.command.arguments[0], std::string("C:\\tmp\\file.txt"));

    result = hexal::parseExCommand("w \"say \\\"hi\\\"\"");
    CHECK(result.ok);
    CHECK_EQ(result.command.arguments[0], std::string("say \"hi\""));

    result = hexal::parseExCommand(R"(e "\\server\share")");
    CHECK(result.ok);
    CHECK_EQ(result.command.arguments[0], std::string(R"(\\server\share)"));

    result = hexal::parseExCommand("e \"unterminated");
    CHECK(!result.ok);
    CHECK(!result.error.empty());

    result = hexal::parseExCommand("   ");
    CHECK(result.ok);
    CHECK(result.command.name.empty());

    result = hexal::parseExCommand("q!");
    CHECK(result.ok);
    CHECK_EQ(result.command.name, std::string("q!"));
}

void testDocumentLinesAndSaving() {
    TempDir directory;

    const auto crlfPath = directory.path("crlf.txt");
    CHECK(writeBytes(crlfPath, "alpha\r\nbeta\r\ngamma"));

    std::string error;
    auto document = hexal::Document::open(crlfPath, error);
    CHECK(document.has_value());
    if (!document) {
        return;
    }

    CHECK_EQ(document->eol(), std::string("\r\n"));
    CHECK_EQ(document->lineCount(), std::size_t{3});
    CHECK_EQ(document->lineStart(0), std::size_t{0});
    CHECK_EQ(document->lineStart(1), std::size_t{7});
    CHECK_EQ(document->lineStart(2), std::size_t{13});
    CHECK_EQ(document->lineEnd(0), std::size_t{5});
    CHECK_EQ(document->lineEnd(1), std::size_t{11});
    CHECK_EQ(document->lineEnd(2), std::size_t{18});
    CHECK_EQ(document->lineText(0), std::string("alpha"));
    CHECK_EQ(document->lineText(1), std::string("beta"));
    CHECK_EQ(document->lineText(2), std::string("gamma"));
    CHECK_EQ(document->lineOfOffset(6), std::size_t{0});
    CHECK_EQ(document->lineOfOffset(7), std::size_t{1});
    CHECK_EQ(document->lineOfOffset(18), std::size_t{2});
    CHECK(!document->isDirty());

    document->insert(document->lineEnd(0), "\r\n");
    CHECK_EQ(document->lineCount(), std::size_t{4});
    CHECK_EQ(document->lineText(1), std::string(""));
    CHECK_EQ(document->lineText(2), std::string("beta"));

    CHECK(document->save(error));
    CHECK_EQ(readBytes(crlfPath), std::string("alpha\r\n\r\nbeta\r\ngamma"));
    CHECK(!document->isDirty());

    const auto lfPath = directory.path("lf.txt");
    CHECK(writeBytes(lfPath, "x\ny"));
    auto lf = hexal::Document::open(lfPath, error);
    CHECK(lf.has_value());
    if (!lf) {
        return;
    }
    CHECK_EQ(lf->lineCount(), std::size_t{2});
    CHECK_EQ(lf->lineStart(1), std::size_t{2});
    CHECK_EQ(lf->lineEnd(0), std::size_t{1});
    CHECK_EQ(lf->lineEnd(1), std::size_t{3});

    const auto trailingPath = directory.path("trailing.txt");
    CHECK(writeBytes(trailingPath, "x\n"));
    auto trailing = hexal::Document::open(trailingPath, error);
    CHECK(trailing.has_value());
    if (!trailing) {
        return;
    }
    CHECK_EQ(trailing->lineCount(), std::size_t{2});
    CHECK_EQ(trailing->lineStart(1), std::size_t{2});
    CHECK_EQ(trailing->lineEnd(0), std::size_t{1});
    CHECK_EQ(trailing->lineEnd(1), std::size_t{2});
}

void testDocumentOpenNewFileAndBom() {
    TempDir directory;
    std::string error;

    const auto newPath = directory.path("new.txt");
    auto created = hexal::Document::open(newPath, error);
    CHECK(created.has_value());
    if (!created) {
        return;
    }
    CHECK(created->hasFile());
    CHECK_EQ(created->size(), std::size_t{0});
    CHECK_EQ(created->lineCount(), std::size_t{1});
    CHECK(!created->isDirty());

    created->insert(0, "hello\n");
    CHECK(created->isDirty());
    CHECK(created->save(error));
    CHECK(!created->isDirty());
    CHECK_EQ(readBytes(newPath), std::string("hello\n"));

    const auto missingParent = directory.path("missing") / "file.txt";
    auto missing = hexal::Document::open(missingParent, error);
    CHECK(!missing.has_value());

    const auto bomPath = directory.path("bom.txt");
    CHECK(writeBytes(bomPath, "\xEF\xBB\xBFhi\n"));
    auto bom = hexal::Document::open(bomPath, error);
    CHECK(bom.has_value());
    if (!bom) {
        return;
    }
    CHECK(bom->hasBom());
    CHECK_EQ(bom->text(), std::string("hi\n"));
    CHECK(bom->save(error));
    CHECK_EQ(readBytes(bomPath), std::string("\xEF\xBB\xBFhi\n"));
}

void testUndoRedo() {
    TempDir directory;
    const auto path = directory.path("undo.txt");
    CHECK(writeBytes(path, "abc"));

    std::string error;
    auto loaded = hexal::Document::open(path, error);
    CHECK(loaded.has_value());
    if (!loaded) {
        return;
    }
    hexal::Document& document = *loaded;

    document.insert(3, "d");
    document.insert(4, "e");
    document.insert(5, "f");
    CHECK_EQ(document.text(), std::string("abcdef"));
    CHECK(document.isDirty());

    auto hint = document.undo();
    CHECK(hint.has_value());
    CHECK_EQ(*hint, std::size_t{3});
    CHECK_EQ(document.text(), std::string("abc"));
    CHECK(!document.isDirty());

    hint = document.redo();
    CHECK(hint.has_value());
    CHECK_EQ(*hint, std::size_t{6});
    CHECK_EQ(document.text(), std::string("abcdef"));

    document.erase(5, 1);
    document.erase(4, 1);
    CHECK_EQ(document.text(), std::string("abcd"));
    CHECK(document.undo().has_value());
    CHECK_EQ(document.text(), std::string("abcdef"));
    CHECK(document.redo().has_value());
    CHECK_EQ(document.text(), std::string("abcd"));

    CHECK(document.save(error));
    CHECK(!document.isDirty());
    CHECK(document.undo().has_value());
    CHECK_EQ(document.text(), std::string("abcdef"));
    CHECK(document.isDirty());
    CHECK(document.redo().has_value());
    CHECK(!document.isDirty());
    CHECK_EQ(document.text(), std::string("abcd"));

    // A new edit after undo must discard the redo branch.
    TempDir secondDirectory;
    const auto secondPath = secondDirectory.path("undo2.txt");
    CHECK(writeBytes(secondPath, "abc"));
    auto second = hexal::Document::open(secondPath, error);
    CHECK(second.has_value());
    if (!second) {
        return;
    }
    hexal::Document& branch = *second;

    branch.insert(3, "def");
    CHECK_EQ(branch.text(), std::string("abcdef"));
    CHECK(branch.undo().has_value());
    CHECK_EQ(branch.text(), std::string("abc"));
    CHECK(branch.canRedo());

    branch.insert(branch.size(), "Z");
    CHECK_EQ(branch.text(), std::string("abcZ"));
    CHECK(!branch.canRedo());

    branch.breakUndoGroup();
    branch.replace(0, 3, "XYZ");
    CHECK_EQ(branch.text(), std::string("XYZZ"));
    CHECK(branch.undo().has_value());
    CHECK_EQ(branch.text(), std::string("abcZ"));
    CHECK(branch.redo().has_value());
    CHECK_EQ(branch.text(), std::string("XYZZ"));
}

void testSaveFailureLeavesOriginal() {
    TempDir directory;
    const auto path = directory.path("safe.txt");
    CHECK(writeBytes(path, "original\n"));

    std::string error;
    auto loaded = hexal::Document::open(path, error);
    CHECK(loaded.has_value());
    if (!loaded) {
        return;
    }
    hexal::Document& document = *loaded;

    document.insert(0, "changed ");
    CHECK(!document.saveAs(directory.path("missing") / "out.txt", error));
    CHECK(!error.empty());
    CHECK_EQ(readBytes(path), std::string("original\n"));
    CHECK_EQ(document.text(), std::string("changed original\n"));
    CHECK(document.isDirty());

    CHECK(document.save(error));
    CHECK_EQ(readBytes(path), std::string("changed original\n"));
    CHECK(!document.isDirty());
}

void testFilesystemReadWrite() {
    TempDir directory;
    const auto path = directory.path("atomic.txt");
    CHECK(writeBytes(path, "old"));

    std::string error;
    CHECK(hexal::fsutil::atomicWrite(
        path,
        [](std::ostream& out) {
            out << "new";
            return out.good();
        },
        error));
    CHECK_EQ(readBytes(path), std::string("new"));

    CHECK(!hexal::fsutil::atomicWrite(
        directory.path("nope") / "x.txt",
        [](std::ostream& out) {
            out << "x";
            return out.good();
        },
        error));

    std::string contents;
    CHECK(hexal::fsutil::readFile(path, contents, error));
    CHECK_EQ(contents, std::string("new"));
}

} // namespace

int main() {
    testPieceTableBasics();
    testPieceTableRandomized();
    testSnapshotIsolation();
    testLargeLineIndex();
    testUnicode();
    testCommandParsing();
    testDocumentLinesAndSaving();
    testDocumentOpenNewFileAndBom();
    testUndoRedo();
    testSaveFailureLeavesOriginal();
    testFilesystemReadWrite();

    std::cout << (gChecks - gFailures) << " of " << gChecks << " checks passed\n";
    if (gFailures != 0) {
        std::cerr << gFailures << " check(s) failed\n";
        return 1;
    }
    return 0;
}
