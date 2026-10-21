#include "ui/editor.hpp"

#include "util/filesystem.hpp"
#include "util/unicode.hpp"

#include <algorithm>
#include <exception>
#include <string>
#include <utility>

namespace hexal {
namespace {

int digitCount(std::size_t value) {
    int digits = 1;
    while (value >= 10) {
        value /= 10;
        ++digits;
    }
    return digits;
}

int utf8Columns(std::string_view text) {
    int columns = 0;
    std::size_t offset = 0;
    while (offset < text.size()) {
        char32_t cp = 0;
        std::size_t length = 0;
        unicode::decode(text, offset, cp, length);
        offset += length;
        if (cp == '\t') {
            ++columns;
        } else {
            const int width = unicode::displayWidth(cp);
            if (width > 0) {
                columns += width;
            }
        }
    }
    return columns;
}

bool isFutureCommand(const std::string& name) {
    return name == "fmt" || name == "cfg" || name == "rcfg" || name == "lang" ||
           name == "sf" || name == "g" || name == "sv" || name == "sh" ||
           name == "setup";
}

} // namespace

Editor::Editor(Terminal& terminal, Document document)
    : terminal_(terminal), document_(std::move(document)) {
    snapCursor();
}

int Editor::run() {
    refreshSize();
    ensureCursorVisible();
    render();

    while (running_) {
        InputEvent event;
        if (!terminal_.readEvent(event)) {
            break;
        }

        if (!handleEvent(event)) {
            break;
        }

        ensureCursorVisible();
        render();
    }

    return 0;
}

void Editor::refreshSize() {
    int columns = 80;
    int rows = 24;
    terminal_.getSize(columns, rows);
    columns_ = static_cast<std::size_t>(std::max(1, columns));
    rows_ = static_cast<std::size_t>(std::max(1, rows));
}

bool Editor::handleEvent(const InputEvent& event) {
    if (event.key == Key::Resize) {
        refreshSize();
        forceFullRedraw_ = true;
        ensureCursorVisible();
        return running_;
    }

    try {
        if (prompt_ != PromptKind::None) {
            handlePrompt(event);
        } else if (mode_ == Mode::Command) {
            handleCommandInput(event);
        } else if (mode_ == Mode::Insert) {
            handleInsert(event);
        } else {
            handleNormal(event);
        }
    } catch (const std::exception& ex) {
        // Keep the UI alive on allocation or I/O exceptions. The buffer may
        // have a partially applied edit only in the rare out-of-memory case,
        // but exiting unexpectedly would risk losing more user work.
        setError(std::string("internal error: ") + ex.what());
    } catch (...) {
        setError("internal error");
    }

    return running_;
}

// ---------------------------------------------------------------------------
// NORMAL MODE
// ---------------------------------------------------------------------------

void Editor::handleNormal(const InputEvent& event) {
    if (event.key == Key::Character && event.ctrl) {
        pendingG_ = false;
        if (event.character == 'r') {
            redo();
        } else if (event.character == 'c') {
            setMessage("Use :q to quit.");
        } else if (event.character == 'w') {
            setMessage("Pane navigation is not part of the MVP.");
        }
        return;
    }

    if (event.key == Key::Character && event.alt) {
        pendingG_ = false;
        return;
    }

    if (pendingG_) {
        pendingG_ = false;
        if (event.key == Key::Character) {
            if (event.character == 'g' || event.character == 'G') {
                moveToDocumentStart(true);
            } else if (event.character == 'e' || event.character == 'E') {
                moveToDocumentEnd(true);
            } else if (event.character == 'p' || event.character == 'P') {
                moveToPreviousLocation();
            }
        }
        return;
    }

    switch (event.key) {
    case Key::Left: moveLeft(); return;
    case Key::Right: moveRight(); return;
    case Key::Up: moveVertical(-1); return;
    case Key::Down: moveVertical(1); return;
    case Key::PageUp: movePage(-1); return;
    case Key::PageDown: movePage(1); return;
    case Key::Home:
        if (event.ctrl) {
            moveToDocumentStart(true);
        } else {
            moveTo(lineStartOfCursor(), false);
        }
        return;
    case Key::End:
        if (event.ctrl) {
            moveToDocumentEnd(true);
        } else {
            moveTo(lineEndOfCursor(), false);
        }
        return;
    case Key::Escape:
        document_.breakUndoGroup();
        clearMessage();
        return;
    case Key::Delete:
        deleteAtCursor();
        return;
    default:
        break;
    }

    if (event.key != Key::Character) {
        return;
    }

    switch (event.character) {
    case ':':
        mode_ = Mode::Command;
        commandLine_.clear();
        commandCursor_ = 0;
        commandHistoryIndex_ = -1;
        document_.breakUndoGroup();
        clearMessage();
        return;
    case 'i':
    case 'I':
        enterInsert();
        return;
    case 'a': {
        const std::size_t end = lineEndOfCursor();
        if (cursor_ < end) {
            const std::string text = document_.lineText(cursorLine());
            const std::size_t local = cursor_ - lineStartOfCursor();
            cursor_ = lineStartOfCursor() + unicode::nextBoundary(text, local);
        }
        enterInsert();
        return;
    }
    case 'A':
        cursor_ = lineEndOfCursor();
        preferredColumnValid_ = false;
        enterInsert();
        return;
    case 'o':
        insertNewlineBelow();
        return;
    case 'O':
        insertNewlineAbove();
        return;
    case 'h': moveLeft(); return;
    case 'l': moveRight(); return;
    case 'j': moveVertical(1); return;
    case 'k': moveVertical(-1); return;
    case 'x': deleteAtCursor(); return;
    case 'u': undo(); return;
    case 'g':
    case 'G':
        pendingG_ = true;
        return;
    case 'V':
        setMessage("SELECT MODE is planned for Phase 2 and is not in this MVP.");
        return;
    case 'X':
        setMessage("Line selection is planned for Phase 2 and is not in this MVP.");
        return;
    case 'C':
        setMessage("Line commenting is planned for Phase 4 and is not in this MVP.");
        return;
    default:
        return;
    }
}

// ---------------------------------------------------------------------------
// INSERT MODE
// ---------------------------------------------------------------------------

void Editor::handleInsert(const InputEvent& event) {
    if (event.key == Key::Escape) {
        mode_ = Mode::Normal;
        pendingG_ = false;
        document_.breakUndoGroup();
        return;
    }

    if (event.key == Key::Character && event.ctrl) {
        if (event.character == 'r') {
            redo();
        }
        return;
    }

    switch (event.key) {
    case Key::Left: moveLeft(); return;
    case Key::Right: moveRight(); return;
    case Key::Up: moveVertical(-1); return;
    case Key::Down: moveVertical(1); return;
    case Key::PageUp: movePage(-1); return;
    case Key::PageDown: movePage(1); return;
    case Key::Home:
        if (event.ctrl) {
            moveToDocumentStart(true);
        } else {
            moveTo(lineStartOfCursor(), false);
        }
        return;
    case Key::End:
        if (event.ctrl) {
            moveToDocumentEnd(true);
        } else {
            moveTo(lineEndOfCursor(), false);
        }
        return;
    case Key::Enter:
        insertNewline();
        return;
    case Key::Backspace:
        backspace();
        return;
    case Key::Delete:
        deleteForward();
        return;
    case Key::Tab:
        insertCharacter(U'\t');
        return;
    default:
        break;
    }

    if (event.key == Key::Character && !event.ctrl && !event.alt) {
        insertCharacter(event.character);
    }
}

// ---------------------------------------------------------------------------
// COMMAND MODE
// ---------------------------------------------------------------------------

void Editor::handleCommandInput(const InputEvent& event) {
    if (event.key == Key::Character && event.ctrl) {
        return;
    }

    switch (event.key) {
    case Key::Escape:
        mode_ = Mode::Normal;
        commandLine_.clear();
        commandCursor_ = 0;
        commandHistoryIndex_ = -1;
        return;
    case Key::Enter: {
        const std::string line = commandLine_;
        mode_ = Mode::Normal;
        commandLine_.clear();
        commandCursor_ = 0;
        commandHistoryIndex_ = -1;

        if (!line.empty() &&
            (commandHistory_.empty() || commandHistory_.back() != line)) {
            commandHistory_.push_back(line);
            if (commandHistory_.size() > 100) {
                commandHistory_.erase(commandHistory_.begin());
            }
        }

        const ExParseResult parsed = parseExCommand(line);
        if (!parsed.ok) {
            setError(parsed.error);
            return;
        }
        if (parsed.command.name.empty()) {
            return;
        }
        executeParsedCommand(parsed.command, false);
        return;
    }
    case Key::Backspace:
        if (commandCursor_ > 0) {
            const std::size_t previous =
                unicode::previousBoundary(commandLine_, commandCursor_);
            commandLine_.erase(previous, commandCursor_ - previous);
            commandCursor_ = previous;
        }
        return;
    case Key::Delete:
        if (commandCursor_ < commandLine_.size()) {
            const std::size_t next =
                unicode::nextBoundary(commandLine_, commandCursor_);
            commandLine_.erase(commandCursor_, next - commandCursor_);
        }
        return;
    case Key::Left:
        commandCursor_ = unicode::previousBoundary(commandLine_, commandCursor_);
        return;
    case Key::Right:
        commandCursor_ = unicode::nextBoundary(commandLine_, commandCursor_);
        return;
    case Key::Home:
        commandCursor_ = 0;
        return;
    case Key::End:
        commandCursor_ = commandLine_.size();
        return;
    case Key::Up:
    case Key::Down:
        if (commandHistory_.empty()) {
            return;
        }
        if (event.key == Key::Up) {
            if (commandHistoryIndex_ < 0) {
                commandHistoryIndex_ =
                    static_cast<int>(commandHistory_.size()) - 1;
            } else if (commandHistoryIndex_ > 0) {
                --commandHistoryIndex_;
            }
            commandLine_ = commandHistory_[static_cast<std::size_t>(commandHistoryIndex_)];
        } else {
            if (commandHistoryIndex_ < 0) {
                return;
            }
            ++commandHistoryIndex_;
            if (commandHistoryIndex_ >= static_cast<int>(commandHistory_.size())) {
                commandHistoryIndex_ = -1;
                commandLine_.clear();
            } else {
                commandLine_ =
                    commandHistory_[static_cast<std::size_t>(commandHistoryIndex_)];
            }
        }
        commandCursor_ = commandLine_.size();
        return;
    default:
        break;
    }

    if (event.key == Key::Character && !event.alt) {
        char buffer[4];
        const std::size_t length = unicode::encode(event.character, buffer);
        commandLine_.insert(commandCursor_, buffer, length);
        commandCursor_ += length;
    }
}

void Editor::handlePrompt(const InputEvent& event) {
    if (event.key == Key::Escape) {
        prompt_ = PromptKind::None;
        pendingOpen_.clear();
        setMessage("Cancelled.");
        return;
    }

    if (event.key != Key::Character || event.ctrl || event.alt) {
        return;
    }

    if (event.character == 'y' || event.character == 'Y') {
        const PromptKind kind = prompt_;
        prompt_ = PromptKind::None;

        if (kind == PromptKind::Quit) {
            running_ = false;
        } else if (kind == PromptKind::Open) {
            const std::filesystem::path path = pendingOpen_;
            pendingOpen_.clear();
            openPath(path);
        }
        return;
    }

    if (event.character == 'n' || event.character == 'N') {
        prompt_ = PromptKind::None;
        pendingOpen_.clear();
        setMessage("Cancelled.");
    }
}

// ---------------------------------------------------------------------------
// COMMANDS
// ---------------------------------------------------------------------------

void Editor::executeParsedCommand(const ExCommand& command, bool force) {
    std::string name = command.name;
    if (!name.empty() && name.back() == '!') {
        force = true;
        name.pop_back();
    }

    const auto& arguments = command.arguments;

    if (name == "w") {
        if (arguments.size() > 1) {
            setError("usage: :w [file]");
            return;
        }
        if (arguments.empty()) {
            saveCurrent();
        } else {
            saveCurrentAs(fsutil::pathFromUtf8(arguments[0]));
        }
        return;
    }

    if (name == "wq") {
        if (!arguments.empty()) {
            setError("usage: :wq");
            return;
        }
        if (saveCurrent()) {
            running_ = false;
        }
        return;
    }

    if (name == "q") {
        if (!arguments.empty()) {
            setError("usage: :q[!]");
            return;
        }
        requestQuit(force);
        return;
    }

    if (name == "e") {
        if (arguments.size() != 1) {
            setError("usage: :e[!] <file>");
            return;
        }
        requestOpen(fsutil::pathFromUtf8(arguments[0]), force);
        return;
    }

    if (name == "help") {
        clearMessage();
        setMessage("i insert | Esc normal | :w :wq :q :e | u/Ctrl+R undo | gg/ge/gp");
        return;
    }

    if (isFutureCommand(name)) {
        setError("command ':" + name + "' is not implemented in this MVP");
        return;
    }

    setError("unknown command ':" + command.name + "' (try :help)");
}

bool Editor::saveCurrent() {
    std::string error;
    if (!document_.save(error)) {
        setError(error);
        return false;
    }

    setMessage("Wrote " + fsutil::pathToUtf8(document_.path().value()) + " (" +
               std::to_string(document_.lineCount()) + " lines)");
    return true;
}

bool Editor::saveCurrentAs(const std::filesystem::path& path) {
    std::string error;
    if (!document_.saveAs(path, error)) {
        setError(error);
        return false;
    }

    setMessage("Wrote " + fsutil::pathToUtf8(path) + " (" +
               std::to_string(document_.lineCount()) + " lines)");
    return true;
}

void Editor::requestQuit(bool force) {
    if (!force && document_.isDirty()) {
        prompt_ = PromptKind::Quit;
        return;
    }
    running_ = false;
}

void Editor::requestOpen(const std::filesystem::path& path, bool force) {
    if (!force && document_.isDirty()) {
        pendingOpen_ = path;
        prompt_ = PromptKind::Open;
        return;
    }
    openPath(path);
}

void Editor::openPath(const std::filesystem::path& path) {
    std::string error;
    auto loaded = Document::open(path, error);
    if (!loaded) {
        setError(error);
        return;
    }

    document_ = std::move(*loaded);
    cursor_ = 0;
    topLine_ = 0;
    leftColumn_ = 0;
    preferredColumnValid_ = false;
    jumpHistory_.clear();
    pendingG_ = false;
    mode_ = Mode::Normal;
    commandLine_.clear();
    commandCursor_ = 0;
    snapCursor();
    setMessage("Opened " + fsutil::pathToUtf8(path));
}

void Editor::undo() {
    document_.breakUndoGroup();
    const auto hint = document_.undo();
    if (!hint) {
        setMessage("Already at oldest change.");
        return;
    }

    cursor_ = *hint;
    snapCursor();
    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    clearMessage();
}

void Editor::redo() {
    document_.breakUndoGroup();
    const auto hint = document_.redo();
    if (!hint) {
        setMessage("Already at newest change.");
        return;
    }

    cursor_ = *hint;
    snapCursor();
    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    clearMessage();
}

// ---------------------------------------------------------------------------
// NAVIGATION
// ---------------------------------------------------------------------------

std::size_t Editor::cursorLine() const {
    return document_.lineOfOffset(cursor_);
}

std::size_t Editor::cursorVisualColumn() const {
    const std::size_t line = cursorLine();
    const std::size_t begin = document_.lineStart(line);
    const std::string text = document_.lineText(line);
    return unicode::visualColumn(text, cursor_ - begin, kTabWidth);
}

std::size_t Editor::textRows() const {
    return rows_ > 2 ? rows_ - 2 : 0;
}

std::size_t Editor::lineStartOfCursor() const {
    return document_.lineStart(cursorLine());
}

std::size_t Editor::lineEndOfCursor() const {
    return document_.lineEnd(cursorLine());
}

void Editor::snapCursor() {
    if (cursor_ > document_.size()) {
        cursor_ = document_.size();
    }

    const std::size_t line = document_.lineOfOffset(cursor_);
    const std::size_t begin = document_.lineStart(line);
    const std::size_t end = document_.lineEnd(line);

    if (cursor_ < begin) {
        cursor_ = begin;
    } else if (cursor_ > end) {
        cursor_ = end;
    }
}

void Editor::moveLeft() {
    const std::size_t line = cursorLine();
    const std::size_t begin = document_.lineStart(line);

    if (cursor_ > begin) {
        const std::string text = document_.lineText(line);
        cursor_ = begin + unicode::previousBoundary(text, cursor_ - begin);
    } else if (line > 0) {
        cursor_ = document_.lineEnd(line - 1);
    }

    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    snapCursor();
}

void Editor::moveRight() {
    const std::size_t line = cursorLine();
    const std::size_t end = document_.lineEnd(line);

    if (cursor_ < end) {
        const std::string text = document_.lineText(line);
        const std::size_t begin = document_.lineStart(line);
        cursor_ = begin + unicode::nextBoundary(text, cursor_ - begin);
    } else if (line + 1 < document_.lineCount()) {
        cursor_ = document_.lineStart(line + 1);
    }

    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    snapCursor();
}

void Editor::moveVertical(int delta) {
    const std::size_t line = cursorLine();

    if (!preferredColumnValid_) {
        preferredColumn_ = cursorVisualColumn();
        preferredColumnValid_ = true;
    }

    long long target = static_cast<long long>(line) + delta;
    if (target < 0) {
        target = 0;
    }
    const long long lastLine =
        static_cast<long long>(document_.lineCount()) - 1;
    if (target > lastLine) {
        target = lastLine;
    }

    if (static_cast<std::size_t>(target) == line) {
        return;
    }

    const std::size_t start = document_.lineStart(static_cast<std::size_t>(target));
    const std::string text = document_.lineText(static_cast<std::size_t>(target));
    cursor_ = start + unicode::byteOffsetForVisualColumn(text, preferredColumn_, kTabWidth);

    document_.breakUndoGroup();
    snapCursor();
}

void Editor::movePage(int direction) {
    const std::size_t before = cursor_;
    const std::size_t rows = textRows();
    const int amount = static_cast<int>(std::max<std::size_t>(1, rows > 1 ? rows - 1 : 1));
    moveVertical(direction * amount);
    if (cursor_ != before) {
        recordJump(before);
    }
}

void Editor::moveTo(std::size_t offset, bool recordJumpRequested) {
    if (recordJumpRequested) {
        recordJump(cursor_);
    }
    cursor_ = std::min(offset, document_.size());
    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    snapCursor();
}

void Editor::moveToDocumentStart(bool recordJumpRequested) {
    moveTo(0, recordJumpRequested);
}

void Editor::moveToDocumentEnd(bool recordJumpRequested) {
    moveTo(document_.size(), recordJumpRequested);
}

void Editor::moveToPreviousLocation() {
    if (jumpHistory_.empty()) {
        setMessage("No previous location.");
        return;
    }

    cursor_ = jumpHistory_.back();
    jumpHistory_.pop_back();
    preferredColumnValid_ = false;
    document_.breakUndoGroup();
    snapCursor();
}

void Editor::recordJump(std::size_t from) {
    if (from == cursor_) {
        return;
    }
    if (!jumpHistory_.empty() && jumpHistory_.back() == from) {
        return;
    }

    jumpHistory_.push_back(from);
    if (jumpHistory_.size() > 200) {
        jumpHistory_.erase(jumpHistory_.begin());
    }
}

void Editor::enterInsert() {
    mode_ = Mode::Insert;
    preferredColumnValid_ = false;
    document_.breakUndoGroup();
}

void Editor::insertCharacter(char32_t codePoint) {
    if (codePoint == '\r') {
        return;
    }
    if (codePoint == '\n') {
        insertNewline();
        return;
    }
    if (codePoint < 0x20 && codePoint != '\t') {
        return;
    }
    if (codePoint >= 0x7F && codePoint < 0xA0) {
        return;
    }

    std::string text;
    unicode::append(text, codePoint);
    document_.insert(cursor_, text);
    cursor_ += text.size();
    snapCursor();
    preferredColumnValid_ = false;
    clearMessage();
}

void Editor::insertNewline() {
    const std::string& eol = document_.eol();
    document_.insert(cursor_, eol);
    cursor_ = std::min(cursor_ + eol.size(), document_.size());
    snapCursor();
    preferredColumnValid_ = false;
    clearMessage();
}

void Editor::insertNewlineBelow() {
    document_.breakUndoGroup();
    const std::size_t line = cursorLine();
    const std::size_t end = document_.lineEnd(line);
    const std::string& eol = document_.eol();
    document_.insert(end, eol);
    cursor_ = document_.lineStart(line + 1);
    preferredColumnValid_ = false;
    snapCursor();
    enterInsert();
}

void Editor::insertNewlineAbove() {
    document_.breakUndoGroup();
    const std::size_t line = cursorLine();
    const std::size_t start = document_.lineStart(line);
    document_.insert(start, document_.eol());
    cursor_ = start;
    preferredColumnValid_ = false;
    snapCursor();
    enterInsert();
}

void Editor::backspace() {
    const std::size_t line = cursorLine();
    const std::size_t begin = document_.lineStart(line);

    if (cursor_ > begin) {
        const std::string text = document_.lineText(line);
        const std::size_t previous =
            begin + unicode::previousBoundary(text, cursor_ - begin);
        document_.erase(previous, cursor_ - previous);
        cursor_ = previous;
    } else if (line > 0) {
        const std::size_t previousEnd = document_.lineEnd(line - 1);
        document_.erase(previousEnd, cursor_ - previousEnd);
        cursor_ = previousEnd;
    }

    preferredColumnValid_ = false;
    snapCursor();
    clearMessage();
}

void Editor::deleteForward() {
    const std::size_t line = cursorLine();
    const std::size_t end = document_.lineEnd(line);

    if (cursor_ < end) {
        const std::string text = document_.lineText(line);
        const std::size_t begin = document_.lineStart(line);
        const std::size_t next =
            begin + unicode::nextBoundary(text, cursor_ - begin);
        document_.erase(cursor_, next - cursor_);
    } else if (line + 1 < document_.lineCount()) {
        const std::size_t nextStart = document_.lineStart(line + 1);
        document_.erase(cursor_, nextStart - cursor_);
    }

    preferredColumnValid_ = false;
    snapCursor();
    clearMessage();
}

void Editor::deleteAtCursor() {
    const std::size_t line = cursorLine();
    const std::size_t end = document_.lineEnd(line);

    if (cursor_ >= end) {
        return;
    }

    document_.breakUndoGroup();
    const std::string text = document_.lineText(line);
    const std::size_t begin = document_.lineStart(line);
    const std::size_t next = begin + unicode::nextBoundary(text, cursor_ - begin);
    document_.erase(cursor_, next - cursor_);
    preferredColumnValid_ = false;
    snapCursor();
}

void Editor::ensureCursorVisible() {
    const std::size_t line = cursorLine();
    const std::size_t rows = textRows();

    if (rows == 0) {
        topLine_ = line;
    } else {
        if (line < topLine_) {
            topLine_ = line;
        } else if (line >= topLine_ + rows) {
            topLine_ = line - rows + 1;
        }

        const std::size_t maxTop =
            document_.lineCount() > rows ? document_.lineCount() - rows : 0;
        if (topLine_ > maxTop) {
            topLine_ = maxTop;
        }
    }

    const int gutter = gutterWidth();
    const std::size_t textWidth =
        columns_ > static_cast<std::size_t>(gutter)
            ? columns_ - static_cast<std::size_t>(gutter)
            : 1;
    const std::size_t column = cursorVisualColumn();

    if (column < leftColumn_) {
        leftColumn_ = column;
    } else if (column >= leftColumn_ + textWidth) {
        leftColumn_ = column - textWidth + 1;
    }
    if (leftColumn_ > column) {
        leftColumn_ = column;
    }
}

// ---------------------------------------------------------------------------
// RENDERING
// ---------------------------------------------------------------------------

int Editor::gutterWidth() const {
    const int digits = digitCount(document_.lineCount());
    const int gutter = digits + 1;
    if (columns_ < static_cast<std::size_t>(gutter + 10)) {
        return 0;
    }
    return gutter;
}

std::size_t Editor::statusRow() const {
    return rows_ >= 2 ? rows_ - 2 : rows_ - 1;
}

std::size_t Editor::commandRow() const {
    return rows_ - 1;
}

std::string Editor::modeLabel() const {
    if (prompt_ != PromptKind::None) {
        return "CONFIRM";
    }
    switch (mode_) {
    case Mode::Normal: return "NORMAL";
    case Mode::Insert: return "INSERT";
    case Mode::Command: return "COMMAND";
    }
    return "NORMAL";
}

std::string Editor::statusLeft(std::size_t line, std::size_t column) const {
    (void)line;
    (void)column;
    std::string text = " " + modeLabel() + "  " + document_.displayName();
    if (document_.isDirty()) {
        text += " [+]";
    }
    return text;
}

std::string Editor::statusRight(std::size_t line, std::size_t column) const {
    std::string text = std::to_string(line + 1) + ":" + std::to_string(column + 1) +
                       "  " + std::to_string(line + 1) + "/" +
                       std::to_string(document_.lineCount()) + "  ";
    text += document_.eol() == "\r\n" ? "CRLF" : "LF";
    if (document_.hasBom()) {
        text += " BOM";
    }
    return text;
}

void Editor::renderLineNumbers(std::size_t firstLine, std::size_t rows, int gutter) {
    if (gutter <= 0) {
        return;
    }

    const int digits = gutter - 1;
    const std::size_t current = cursorLine();
    const std::size_t total = document_.lineCount();

    for (std::size_t row = 0; row < rows; ++row) {
        const std::size_t line = firstLine + row;
        if (line >= total) {
            break;
        }

        const std::string number = std::to_string(line + 1);
        int x = digits - static_cast<int>(number.size());
        if (x < 0) {
            x = 0;
        }

        Style style;
        if (line == current) {
            style.fg = 15;
            style.bold = true;
        } else {
            style.fg = 8;
            style.dim = true;
        }

        screen_.putAscii(x, static_cast<int>(row), number, style, digits);
    }
}

void Editor::renderTextLines(std::size_t firstLine, std::size_t rows, int gutter) {
    if (rows == 0 || columns_ <= static_cast<std::size_t>(gutter)) {
        return;
    }

    const std::size_t total = document_.lineCount();
    const std::size_t textWidth = columns_ - static_cast<std::size_t>(gutter);
    const Style textStyle;

    for (std::size_t row = 0; row < rows; ++row) {
        const std::size_t line = firstLine + row;
        if (line >= total) {
            break;
        }

        const std::string text = document_.lineText(line);
        std::size_t offset = 0;
        std::size_t visual = 0;

        while (offset < text.size()) {
            char32_t cp = 0;
            std::size_t length = 0;
            unicode::decode(text, offset, cp, length);
            offset += length;

            int width = 0;
            if (cp == '\t') {
                const std::size_t tab = static_cast<std::size_t>(kTabWidth);
                width = static_cast<int>(tab - (visual % tab));
            } else {
                width = unicode::displayWidth(cp);
                if (width == 0) {
                    if (cp < 0x20 || cp == 0x7F) {
                        cp = 0xFFFD;
                        width = 1;
                    } else {
                        continue; // combining mark
                    }
                }
            }

            const std::size_t nextVisual = visual + static_cast<std::size_t>(width);

            if (nextVisual <= leftColumn_) {
                visual = nextVisual;
                continue;
            }

            if (visual < leftColumn_) {
                const std::size_t visible = nextVisual - leftColumn_;
                for (std::size_t i = 0; i < visible && i < textWidth; ++i) {
                    screen_.put(gutter + static_cast<int>(i), static_cast<int>(row),
                                U' ', textStyle, 1);
                }
                visual = nextVisual;
                continue;
            }

            const std::size_t screenX =
                static_cast<std::size_t>(gutter) + (visual - leftColumn_);
            if (screenX >= columns_) {
                break;
            }

            const std::size_t available = columns_ - screenX;
            if (static_cast<std::size_t>(width) <= available) {
                if (cp == '\t') {
                    for (int i = 0; i < width; ++i) {
                        screen_.put(static_cast<int>(screenX) + i,
                                    static_cast<int>(row), U' ', textStyle, 1);
                    }
                } else {
                    screen_.put(static_cast<int>(screenX), static_cast<int>(row), cp,
                                textStyle, width);
                }
            } else {
                for (std::size_t i = 0; i < available; ++i) {
                    screen_.put(static_cast<int>(screenX + i),
                                static_cast<int>(row), U' ', textStyle, 1);
                }
            }

            visual = nextVisual;
            if (visual - leftColumn_ >= textWidth) {
                break;
            }
        }
    }
}

void Editor::renderCursor(std::size_t firstLine, std::size_t rows, int gutter) {
    const std::size_t line = cursorLine();
    if (line < firstLine || line >= firstLine + rows) {
        return;
    }

    const std::size_t row = line - firstLine;
    const std::size_t column = cursorVisualColumn();
    if (column < leftColumn_) {
        return;
    }

    const std::size_t x =
        static_cast<std::size_t>(gutter) + (column - leftColumn_);
    if (x >= columns_ || row >= rows_) {
        return;
    }

    Style style = screen_.at(static_cast<int>(x), static_cast<int>(row)).style;
    style.reverse = true;
    screen_.applyStyle(static_cast<int>(x), static_cast<int>(row), style);
}

void Editor::renderStatusLine() {
    const std::size_t y = statusRow();
    if (y >= rows_) {
        return;
    }

    Style style;
    if (prompt_ != PromptKind::None) {
        style.fg = 15;
        style.bg = 5;
        style.bold = true;
    } else if (mode_ == Mode::Insert) {
        style.fg = 0;
        style.bg = 2;
        style.bold = true;
    } else if (mode_ == Mode::Command) {
        style.fg = 0;
        style.bg = 6;
        style.bold = true;
    } else {
        style.fg = 15;
        style.bg = 4;
        style.bold = true;
    }

    screen_.fill(0, static_cast<int>(y), static_cast<int>(columns_), 1, U' ', style);

    const std::size_t line = cursorLine();
    const std::size_t column = cursorVisualColumn();
    const std::string left = statusLeft(line, column);
    std::string right = statusRight(line, column);

    int rightWidth = utf8Columns(right);
    if (static_cast<std::size_t>(rightWidth + 3) > columns_) {
        right = std::to_string(line + 1) + ":" + std::to_string(column + 1);
        rightWidth = utf8Columns(right);
    }

    int rightX = static_cast<int>(columns_) - rightWidth - 1;
    if (rightX < 1) {
        rightX = 1;
    }

    const int leftMax = rightX - 1;
    if (leftMax > 0) {
        screen_.putUtf8(1, static_cast<int>(y), left, style, leftMax);
    }
    const int rightMax = static_cast<int>(columns_) - rightX;
    if (rightMax > 0 && rightX < static_cast<int>(columns_)) {
        screen_.putUtf8(rightX, static_cast<int>(y), right, style, rightMax);
    }
}

void Editor::renderBottomLine() {
    const std::size_t y = commandRow();
    if (y >= rows_) {
        return;
    }

    const Style base;
    screen_.fill(0, static_cast<int>(y), static_cast<int>(columns_), 1, U' ', base);

    if (mode_ == Mode::Command) {
        Style promptStyle;
        promptStyle.fg = 14;
        promptStyle.bold = true;
        screen_.put(0, static_cast<int>(y), U':', promptStyle, 1);

        screen_.putUtf8(1, static_cast<int>(y), commandLine_, base,
                        static_cast<int>(columns_) - 1);

        const std::size_t visual =
            unicode::visualColumn(commandLine_, commandCursor_, 1);
        const std::size_t x = 1 + visual;
        if (x < columns_) {
            Style cursorStyle =
                screen_.at(static_cast<int>(x), static_cast<int>(y)).style;
            cursorStyle.reverse = true;
            screen_.applyStyle(static_cast<int>(x), static_cast<int>(y), cursorStyle);
        }
        return;
    }

    if (prompt_ != PromptKind::None) {
        std::string text;
        if (prompt_ == PromptKind::Quit) {
            text = "Unsaved changes. Quit without saving? (y/N)";
        } else {
            text = "Unsaved changes. Discard and open '" +
                   fsutil::pathToUtf8(pendingOpen_) + "'? (y/N)";
        }

        Style style;
        style.fg = 11;
        style.bold = true;
        screen_.putUtf8(0, static_cast<int>(y), text, style,
                        static_cast<int>(columns_));
        return;
    }

    if (!message_.empty()) {
        Style style;
        if (messageIsError_) {
            style.fg = 9;
            style.bold = true;
        }
        screen_.putUtf8(0, static_cast<int>(y), message_, style,
                        static_cast<int>(columns_));
    }
}

void Editor::render() {
    screen_.resize(static_cast<int>(columns_), static_cast<int>(rows_));
    screen_.clear();

    ensureCursorVisible();

    const std::size_t rows = textRows();
    const std::size_t total = document_.lineCount();

    if (rows > 0) {
        const std::size_t maxTop = total > rows ? total - rows : 0;
        if (topLine_ > maxTop) {
            topLine_ = maxTop;
        }
    } else {
        topLine_ = cursorLine();
    }

    const int gutter = gutterWidth();
    if (rows > 0) {
        renderLineNumbers(topLine_, rows, gutter);
        renderTextLines(topLine_, rows, gutter);
        renderCursor(topLine_, rows, gutter);
    }

    renderStatusLine();
    renderBottomLine();

    const std::string output = renderer_.render(screen_, forceFullRedraw_);
    if (!output.empty()) {
        terminal_.write(output);
    }
    forceFullRedraw_ = false;
}

void Editor::setMessage(std::string text) {
    message_ = std::move(text);
    messageIsError_ = false;
}

void Editor::setError(std::string text) {
    if (text.rfind("E: ", 0) == 0) {
        message_ = std::move(text);
    } else {
        message_ = "E: " + text;
    }
    messageIsError_ = true;
}

void Editor::clearMessage() {
    message_.clear();
    messageIsError_ = false;
}

} // namespace hexal
