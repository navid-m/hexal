#include "core/document.hpp"

#include "util/filesystem.hpp"

#include <algorithm>
#include <ostream>
#include <system_error>

namespace hexal {
namespace {

constexpr std::size_t kMaxUndoGroups = 4096;
constexpr std::size_t kMaxUndoBytes = 64U * 1024U * 1024U;
constexpr long long kUnreachable = -1;

bool containsLineBreak(std::string_view text) {
    return text.find('\n') != std::string_view::npos ||
           text.find('\r') != std::string_view::npos;
}

} // namespace

Document::Document() = default;
Document::~Document() = default;
Document::Document(Document&&) noexcept = default;
Document& Document::operator=(Document&&) noexcept = default;

std::string Document::detectEol(std::string_view text) {
    const std::size_t newline = text.find('\n');
    if (newline == std::string_view::npos) {
        return "\n";
    }
    if (newline > 0 && text[newline - 1] == '\r') {
        return "\r\n";
    }
    return "\n";
}

std::optional<Document> Document::open(const std::filesystem::path& path,
                                       std::string& error) {
    if (path.empty()) {
        error = "empty file name";
        return std::nullopt;
    }

    std::error_code ec;
    const auto status = std::filesystem::status(path, ec);
    const bool statusKnown = !ec;
    const bool exists = statusKnown && std::filesystem::exists(status);

    if (ec && ec != std::errc::no_such_file_or_directory &&
        ec != std::errc::not_a_directory) {
        error = "cannot access '" + fsutil::pathToUtf8(path) + "': " + ec.message();
        return std::nullopt;
    }

    Document document;
    document.path_ = path;

    if (exists) {
        if (std::filesystem::is_directory(status)) {
            error = "'" + fsutil::pathToUtf8(path) + "' is a directory";
            return std::nullopt;
        }
        if (!std::filesystem::is_regular_file(status)) {
            error = "'" + fsutil::pathToUtf8(path) + "' is not a regular file";
            return std::nullopt;
        }

        std::string contents;
        if (!fsutil::readFile(path, contents, error)) {
            return std::nullopt;
        }

        if (contents.size() >= 3 &&
            static_cast<unsigned char>(contents[0]) == 0xEF &&
            static_cast<unsigned char>(contents[1]) == 0xBB &&
            static_cast<unsigned char>(contents[2]) == 0xBF) {
            contents.erase(0, 3);
            document.bom_ = true;
        }

        document.eol_ = detectEol(contents);
        document.table_.setOriginal(std::move(contents));
        return document;
    }

    // A new file is allowed only when its parent directory is usable. This
    // catches typos early without requiring the file itself to exist.
    const auto parent =
        path.has_parent_path() ? path.parent_path() : std::filesystem::path(".");
    if (!std::filesystem::is_directory(parent, ec)) {
        error = "parent directory does not exist: '" + fsutil::pathToUtf8(parent) + "'";
        return std::nullopt;
    }

    document.table_.setOriginal({});
    return document;
}

const std::optional<std::filesystem::path>& Document::path() const noexcept {
    return path_;
}

std::string Document::displayName() const {
    if (!path_) {
        return "[No Name]";
    }
    const auto name = path_->filename();
    return name.empty() ? fsutil::pathToUtf8(*path_) : fsutil::pathToUtf8(name);
}

bool Document::hasFile() const noexcept {
    return path_.has_value();
}

bool Document::isDirty() const noexcept {
    return savedPose_ < 0 || savedPose_ != static_cast<long long>(undo_.size());
}

const std::string& Document::eol() const noexcept {
    return eol_;
}

bool Document::hasBom() const noexcept {
    return bom_;
}

std::size_t Document::size() const noexcept {
    return table_.size();
}

std::size_t Document::lineCount() const noexcept {
    return table_.newlineCount() + 1;
}

std::size_t Document::lineStart(std::size_t line) const {
    return table_.lineStart(line);
}

std::size_t Document::lineOfOffset(std::size_t offset) const {
    return table_.lineOfOffset(offset);
}

std::size_t Document::lineEnd(std::size_t line) const {
    const std::size_t count = lineCount();
    if (line >= count) {
        return size();
    }

    if (line + 1 >= count) {
        return size();
    }

    std::size_t end = table_.lineStart(line + 1);
    if (end > 0 && table_.byteAt(end - 1) == '\n') {
        --end;
        if (end > 0 && table_.byteAt(end - 1) == '\r') {
            --end;
        }
    }
    return end;
}

std::string Document::lineText(std::size_t line) const {
    const std::size_t begin = lineStart(line);
    const std::size_t end = lineEnd(line);
    if (end <= begin) {
        return {};
    }
    return table_.range(begin, end - begin);
}

std::string Document::range(std::size_t offset, std::size_t length) const {
    return table_.range(offset, length);
}

std::string Document::text() const {
    return table_.toString();
}

char Document::byteAt(std::size_t offset) const {
    return table_.byteAt(offset);
}

std::size_t Document::groupBytes(const Group& group) {
    std::size_t total = 32;
    for (const Change& change : group.changes) {
        total += change.removed.size() + change.inserted.size();
    }
    return total;
}

void Document::prepareForEdit() {
    if (redo_.empty()) {
        return;
    }

    if (savedPose_ > static_cast<long long>(undo_.size())) {
        // The saved state depended on an operation that is about to be
        // dropped from the redo stack.
        savedPose_ = kUnreachable;
    }

    for (const Group& group : redo_) {
        historyBytes_ -= groupBytes(group);
    }
    redo_.clear();
}

void Document::pushGroup(Group group) {
    historyBytes_ += groupBytes(group);
    undo_.push_back(std::move(group));
    trimHistory();
}

void Document::trimHistory() {
    // Drop the oldest applied groups first.
    std::size_t dropped = 0;
    while (undo_.size() > 1 &&
           (undo_.size() > kMaxUndoGroups || historyBytes_ > kMaxUndoBytes)) {
        historyBytes_ -= groupBytes(undo_.front());
        undo_.pop_front();
        ++dropped;
    }

    if (dropped > 0 && savedPose_ >= 0) {
        if (savedPose_ >= static_cast<long long>(dropped)) {
            savedPose_ -= static_cast<long long>(dropped);
        } else {
            savedPose_ = kUnreachable;
        }
    }

    // Redo entries are the oldest operations that can be reapplied. Discard
    // them only when the memory budget still requires it.
    while (!redo_.empty() &&
           (redo_.size() > kMaxUndoGroups || historyBytes_ > kMaxUndoBytes)) {
        if (savedPose_ > static_cast<long long>(undo_.size())) {
            savedPose_ = kUnreachable;
        }
        historyBytes_ -= groupBytes(redo_.front());
        redo_.pop_front();
    }
}

void Document::insert(std::size_t offset, std::string_view text) {
    if (text.empty()) {
        return;
    }

    offset = std::min(offset, size());
    prepareForEdit();
    table_.insert(offset, text);

    if (insertRunOpen_ && !undo_.empty() && !containsLineBreak(text)) {
        Group& group = undo_.back();
        if (group.kind == GroupKind::Insert && group.changes.size() == 1) {
            Change& change = group.changes[0];
            if (offset == change.offset + change.inserted.size()) {
                change.inserted.append(text.data(), text.size());
                historyBytes_ += text.size();
                trimHistory();
                eraseRunOpen_ = false;
                return;
            }
        }
    }

    Group group;
    group.kind = GroupKind::Insert;
    Change change;
    change.offset = offset;
    change.inserted.assign(text.data(), text.size());
    group.changes.push_back(std::move(change));

    insertRunOpen_ = !containsLineBreak(text);
    eraseRunOpen_ = false;
    pushGroup(std::move(group));
}

void Document::erase(std::size_t offset, std::size_t length) {
    if (length == 0 || offset >= size()) {
        return;
    }

    offset = std::min(offset, size());
    length = std::min(length, size() - offset);

    const std::string removed = table_.range(offset, length);
    table_.erase(offset, length);

    if (restoring_) {
        return;
    }

    prepareForEdit();

    if (eraseRunOpen_ && !undo_.empty() && !containsLineBreak(removed)) {
        Group& group = undo_.back();
        if (group.kind == GroupKind::Erase && group.changes.size() == 1) {
            Change& change = group.changes[0];
            if (offset + length == change.offset) {
                // Backspace: extend the deletion to the left.
                change.offset = offset;
                change.removed.insert(0, removed);
                historyBytes_ += removed.size();
                trimHistory();
                insertRunOpen_ = false;
                return;
            }
            if (offset == change.offset) {
                // Delete: extend the deletion to the right.
                change.removed.append(removed);
                historyBytes_ += removed.size();
                trimHistory();
                insertRunOpen_ = false;
                return;
            }
        }
    }

    Group group;
    group.kind = GroupKind::Erase;
    Change change;
    change.offset = offset;
    change.removed = removed;
    group.changes.push_back(std::move(change));

    eraseRunOpen_ = !containsLineBreak(removed);
    insertRunOpen_ = false;
    pushGroup(std::move(group));
}

void Document::replace(std::size_t offset, std::size_t length, std::string_view text) {
    offset = std::min(offset, size());
    length = std::min(length, size() - offset);

    const std::string removed = table_.range(offset, length);
    if (std::string_view(removed) == text) {
        return;
    }

    table_.erase(offset, length);
    if (!text.empty()) {
        table_.insert(offset, text);
    }

    if (restoring_) {
        return;
    }

    prepareForEdit();

    Group group;
    group.kind = GroupKind::Mixed;
    Change change;
    change.offset = offset;
    change.removed = removed;
    if (!text.empty()) {
        change.inserted.assign(text.data(), text.size());
    }
    group.changes.push_back(std::move(change));

    insertRunOpen_ = false;
    eraseRunOpen_ = false;
    pushGroup(std::move(group));
}

void Document::breakUndoGroup() {
    insertRunOpen_ = false;
    eraseRunOpen_ = false;
}

void Document::applyChangeForward(const Change& change) {
    if (!change.removed.empty()) {
        table_.erase(change.offset, change.removed.size());
    }
    if (!change.inserted.empty()) {
        table_.insert(change.offset, change.inserted);
    }
}

void Document::applyChangeBackward(const Change& change) {
    if (!change.inserted.empty()) {
        table_.erase(change.offset, change.inserted.size());
    }
    if (!change.removed.empty()) {
        table_.insert(change.offset, change.removed);
    }
}

std::optional<std::size_t> Document::undo() {
    breakUndoGroup();
    if (undo_.empty()) {
        return std::nullopt;
    }

    Group group = std::move(undo_.back());
    undo_.pop_back();

    restoring_ = true;
    for (auto it = group.changes.rbegin(); it != group.changes.rend(); ++it) {
        applyChangeBackward(*it);
    }
    restoring_ = false;

    std::size_t hint = group.changes.front().offset;
    if (group.changes.front().inserted.empty()) {
        hint += group.changes.front().removed.size();
    }

    redo_.push_back(std::move(group));
    trimHistory();
    return hint;
}

std::optional<std::size_t> Document::redo() {
    breakUndoGroup();
    if (redo_.empty()) {
        return std::nullopt;
    }

    Group group = std::move(redo_.back());
    redo_.pop_back();

    restoring_ = true;
    for (const Change& change : group.changes) {
        applyChangeForward(change);
    }
    restoring_ = false;

    const Change& last = group.changes.back();
    const std::size_t hint =
        last.inserted.empty() ? last.offset : last.offset + last.inserted.size();

    undo_.push_back(std::move(group));
    trimHistory();
    return hint;
}

bool Document::canUndo() const noexcept {
    return !undo_.empty();
}

bool Document::canRedo() const noexcept {
    return !redo_.empty();
}

bool Document::writeToPath(const std::filesystem::path& path, std::string& error) {
    error.clear();

    const bool ok = fsutil::atomicWrite(
        path,
        [this](std::ostream& out) {
            if (bom_) {
                out.write("\xEF\xBB\xBF", 3);
            }
            return table_.writeTo(out);
        },
        error);

    if (!ok) {
        return false;
    }

    path_ = path;
    savedPose_ = static_cast<long long>(undo_.size());
    breakUndoGroup();
    return true;
}

bool Document::save(std::string& error) {
    if (!path_) {
        error = "no file name assigned (use :w <file>)";
        return false;
    }
    return writeToPath(*path_, error);
}

bool Document::saveAs(const std::filesystem::path& path, std::string& error) {
    if (path.empty()) {
        error = "empty file name";
        return false;
    }
    return writeToPath(path, error);
}

} // namespace hexal
