#pragma once

#include "core/command.hpp"
#include "core/document.hpp"
#include "platform/terminal.hpp"
#include "ui/renderer.hpp"
#include "ui/screen.hpp"

#include <cstddef>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace hexal {

class Editor {
public:
    Editor(Terminal& terminal, Document document);

    // Runs until the user quits or input ends. The terminal must already be
    // initialized. Returns a process exit code.
    int run();

private:
    enum class Mode { Normal, Insert, Command };
    enum class PromptKind { None, Quit, Open };

    static constexpr int kTabWidth = 4;

    // Main loop / input.
    void refreshSize();
    bool handleEvent(const InputEvent& event);
    void handleNormal(const InputEvent& event);
    void handleInsert(const InputEvent& event);
    void handleCommandInput(const InputEvent& event);
    void handlePrompt(const InputEvent& event);

    // Command execution.
    void executeParsedCommand(const ExCommand& command, bool force);
    void openPath(const std::filesystem::path& path);
    void requestQuit(bool force);
    void requestOpen(const std::filesystem::path& path, bool force);
    bool saveCurrent();
    bool saveCurrentAs(const std::filesystem::path& path);
    void undo();
    void redo();

    // Navigation / editing.
    std::size_t cursorLine() const;
    std::size_t cursorVisualColumn() const;
    std::size_t textRows() const;
    std::size_t lineStartOfCursor() const;
    std::size_t lineEndOfCursor() const;
    void snapCursor();
    void moveLeft();
    void moveRight();
    void moveVertical(int delta);
    void movePage(int direction);
    void moveTo(std::size_t offset, bool recordJump);
    void moveToDocumentStart(bool recordJump);
    void moveToDocumentEnd(bool recordJump);
    void moveToPreviousLocation();
    void recordJump(std::size_t from);
    void enterInsert();
    void insertCharacter(char32_t codePoint);
    void insertNewline();
    void insertNewlineBelow();
    void insertNewlineAbove();
    void backspace();
    void deleteForward();
    void deleteAtCursor();
    void ensureCursorVisible();

    // Rendering.
    void render();
    int gutterWidth() const;
    std::size_t statusRow() const;
    std::size_t commandRow() const;
    std::string modeLabel() const;
    std::string statusLeft(std::size_t line, std::size_t column) const;
    std::string statusRight(std::size_t line, std::size_t column) const;
    void renderLineNumbers(std::size_t firstLine, std::size_t rows, int gutter);
    void renderTextLines(std::size_t firstLine, std::size_t rows, int gutter);
    void renderStatusLine();
    void renderBottomLine();
    void renderCursor(std::size_t firstLine, std::size_t rows, int gutter);

    void setMessage(std::string text);
    void setError(std::string text);
    void clearMessage();

    Terminal& terminal_;
    Document document_;
    Renderer renderer_;
    Screen screen_;

    Mode mode_ = Mode::Normal;
    PromptKind prompt_ = PromptKind::None;
    std::filesystem::path pendingOpen_;

    std::size_t cursor_ = 0;
    std::size_t topLine_ = 0;
    std::size_t leftColumn_ = 0;
    std::size_t preferredColumn_ = 0;
    bool preferredColumnValid_ = false;

    std::size_t columns_ = 80;
    std::size_t rows_ = 24;
    bool running_ = true;
    bool forceFullRedraw_ = true;

    std::string message_;
    bool messageIsError_ = false;

    std::string commandLine_;
    std::size_t commandCursor_ = 0;
    std::vector<std::string> commandHistory_;
    int commandHistoryIndex_ = -1;

    std::vector<std::size_t> jumpHistory_;
    bool pendingG_ = false;
};

} // namespace hexal
