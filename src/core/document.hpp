#pragma once

#include "core/piece_table.hpp"

#include <cstddef>
#include <deque>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace hexal {

// Coordinates used by Hexal:
//   * Storage: UTF-8 byte offsets into the document.
//   * Cursor:  a UTF-8 byte offset on a code point boundary.
//   * Display: terminal cell columns (tabs expanded, wide code points = 2).
// These are deliberately distinct types of value; the UI converts between
// them. Invalid UTF-8 is preserved on disk and displayed as U+FFFD.
class Document {
public:
    Document();
    ~Document();

    Document(Document&&) noexcept;
    Document& operator=(Document&&) noexcept;

    Document(const Document&) = delete;
    Document& operator=(const Document&) = delete;

    // Opens an existing file, or creates an empty document if the path does
    // not exist but its parent directory does.
    static std::optional<Document> open(const std::filesystem::path& path,
                                        std::string& error);

    const std::optional<std::filesystem::path>& path() const noexcept;
    std::string displayName() const;
    bool hasFile() const noexcept;
    bool isDirty() const noexcept;

    const std::string& eol() const noexcept;
    bool hasBom() const noexcept;

    std::size_t size() const noexcept;
    std::size_t lineCount() const noexcept;
    std::size_t lineStart(std::size_t line) const;
    std::size_t lineEnd(std::size_t line) const;
    std::size_t lineOfOffset(std::size_t offset) const;
    std::string lineText(std::size_t line) const;
    std::string range(std::size_t offset, std::size_t length) const;
    std::string text() const;
    char byteAt(std::size_t offset) const;

    // Editing. Adjacent typing / deletion is merged into one undo group until
    // breakUndoGroup() is called (navigation, mode changes, commands, save).
    void insert(std::size_t offset, std::string_view text);
    void erase(std::size_t offset, std::size_t length);
    void replace(std::size_t offset, std::size_t length, std::string_view text);
    void breakUndoGroup();

    // Returns a byte offset that the caller should use as the new cursor.
    std::optional<std::size_t> undo();
    std::optional<std::size_t> redo();
    bool canUndo() const noexcept;
    bool canRedo() const noexcept;

    // Saves to the associated path. The original file is never truncated in
    // place: output is written to a sibling temporary file and renamed.
    bool save(std::string& error);
    bool saveAs(const std::filesystem::path& path, std::string& error);

    // Asynchronous-save support. createSaveSnapshot() is cheap for ordinary
    // editing sessions: it copies piece descriptors and inserted text, and
    // shares the immutable original file buffer. writeSnapshot() is safe to
    // call from a worker while the document continues to be edited.
    struct SaveSnapshot {
        std::filesystem::path path;
        bool bom = false;
        std::size_t pose = 0;
        std::uint64_t revision = 0;
        PieceTable::Snapshot table;
    };

    std::optional<SaveSnapshot> createSaveSnapshot(std::string& error) const;
    std::optional<SaveSnapshot> createSaveSnapshot(const std::filesystem::path& path,
                                                   std::string& error) const;
    static bool writeSnapshot(const SaveSnapshot& snapshot, std::string& error);
    void markSaved(const std::filesystem::path& path, std::size_t pose,
                   std::uint64_t revision);

private:
    enum class GroupKind { Insert, Erase, Mixed };

    struct Change {
        std::size_t offset = 0;
        std::string removed;
        std::string inserted;
    };

    struct Group {
        GroupKind kind = GroupKind::Mixed;
        std::vector<Change> changes;
    };

    static std::size_t groupBytes(const Group& group);
    static std::string detectEol(std::string_view text);

    void prepareForEdit();
    void pushGroup(Group group);
    void trimHistory();
    void applyChangeForward(const Change& change);
    void applyChangeBackward(const Change& change);

    PieceTable table_;
    std::optional<std::filesystem::path> path_;
    std::string eol_ = "\n";
    bool bom_ = false;

    std::deque<Group> undo_;
    std::deque<Group> redo_;
    std::size_t historyBytes_ = 0;

    // Number of applied undo groups at the last save. -1 means the saved
    // state is no longer reachable (history was truncated).
    long long savedPose_ = 0;

    std::uint64_t revision_ = 0;

    bool insertRunOpen_ = false;
    bool eraseRunOpen_ = false;
    bool restoring_ = false;
};

} // namespace hexal
