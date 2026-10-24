using Hexal.Platform;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private enum HelpLineKind
    {
        Section,
        Entry,
        Text,
        Blank,
    }

    private readonly record struct HelpLine(HelpLineKind Kind, string Left, string Right);

    private static readonly HelpLine[] HelpLines = BuildHelpLines();

    private bool _helpVisible;
    private int _helpTop;

    private static HelpLine[] BuildHelpLines()
    {
        var lines = new List<HelpLine>();

        void Section(string title) => lines.Add(new HelpLine(HelpLineKind.Section, title, ""));

        void Entry(string keys, string description) =>
            lines.Add(new HelpLine(HelpLineKind.Entry, keys, description));

        void Text(string text) => lines.Add(new HelpLine(HelpLineKind.Text, text, ""));

        void Blank() => lines.Add(new HelpLine(HelpLineKind.Blank, "", ""));

        Section("Fundamentals");
        Entry("i / I", "Insert before the cursor / at the start of the line");
        Entry("a / A", "Append after the cursor / at the end of the line");
        Entry("o / O", "Open a new line below / above and enter INSERT");
        Entry("Esc", "Return to NORMAL, cancel SELECT, or dismiss a prompt");
        Entry(":", "Open the Ex command line");
        Entry("Ctrl+P", "Open the command palette");
        Entry(":w / :wq / :q", "Write, write and quit, or quit");
        Entry(":q!", "Quit and discard unsaved changes");
        Entry("Ctrl+B", "Toggle and focus the file sidebar");
        Blank();

        Section("Movement");
        Entry("h j k l / arrows", "Move left, down, up, right");
        Entry("w / b / e", "Next word / previous word / end of word");
        Entry("0 / $  Home / End", "Start / end of line");
        Entry("gg / ge", "First / last line");
        Entry("PageUp / PageDown", "Move one screen up / down");
        Entry("Ctrl+Home / Ctrl+End", "Start / end of document");
        Entry("g p", "Jump to the previous location");
        Blank();

        Section("Selection and editing");
        Entry("v / V", "Start character / line selection");
        Entry("x / X", "Select current line / extend line selection");
        Entry("d / D / Delete", "Delete the character or selection");
        Entry("u / Ctrl+R", "Undo / redo");
        Entry("Ctrl+C", "Toggle comment on the selection or line");
        Entry("< / >", "Unindent / indent the selection or current line");
        Entry("Space y", "Copy the selection to the system clipboard");
        Entry("Space X", "Cut the selection to the system clipboard");
        Entry("Space p / Space P", "Paste at the cursor / on a new line");
        Entry("Ctrl+Space", "Request language-server completions in INSERT");
        Blank();

        Section("Search");
        Entry("/", "Literal search in the current file");
        Entry("\\", "Regular-expression search in the current file");
        Entry("'", "Regular-expression search and replace");
        Entry(".", "Select every match for multi-cursor editing");
        Entry("Space /", "Search across all project files");
        Entry("Space .", "Find and replace across matching files");
        Entry("Space \\", "Regex find and replace across matching files");
        Entry("Space f", "Open the file picker");
        Blank();

        Section("Git");
        Entry(":gi", "Toggle git-ignored files in the sidebar and pickers");
        Entry(":gdl", "Toggle added/deleted/modified gutter markers");
        Entry(":gj / :gjp", "Jump to the next / previous Git diff line");
        Blank();

        Section("Sidebar");
        Entry("C", "Open the context menu for the selected item(s)");
        Entry("Ctrl+Left / Ctrl+Right", "Shrink / grow the sidebar");
        Entry("Shift+Up / Shift+Down", "Extend the multi-selection");
        Entry("Enter / Space", "Open a file or expand/collapse a folder");
        Entry("O X E", "Menu: open, new folder, new file");
        Entry("R D F", "Menu: rename, duplicate, format");
        Entry("I", "Menu: diff (exactly two files selected)");
        Entry("T M d", "Menu: copy to, move to, delete");
        Blank();

        Section("Panes");
        Entry("Ctrl+W h j k l", "Focus an adjacent split pane");
        Entry("Ctrl+W + / -", "Resize the current pane");
        Entry("Ctrl+W q", "Close the current pane");
        Entry(":vs / :hs", "Split vertically / horizontally");
        Blank();

        Section("Formatting and configuration");
        Entry(":fmt", "Format the current buffer with its language formatter");
        Entry(":fmta <ext>", "Format all non-ignored workspace files with an extension");
        Entry(":ww", "Toggle word wrap (persisted to .hexcfg)");
        Entry(":cw", "Trim trailing whitespace from every line");
        Entry(":cfg / :rcfg", "Open .hexcfg / reload configuration");
        Entry(":path", "Show the current file's absolute path");
        Entry(":lang <id>", "Override the language for the current buffer");
        Entry(":adi", "Detect and apply the buffer's indentation");
        Blank();

        Section("Debugging");
        Entry("Ctrl+E", "Toggle a breakpoint on the current line");
        Entry(":debug [exe]", "Start a debug session");
        Entry(":dbg-cont", "Continue execution");
        Entry(":dbg-next / :dbg-step", "Step over / step into");
        Entry(":dbg-out / :dbg-pause", "Step out / pause");
        Entry(":dbg-sv", "Show scopes, variables and watches");
        Entry(":dbg-stop", "Stop the debug session");
        Blank();

        Section("Plugins, themes and reload");
        Entry(":rcfg", "Reload grammars, themes, plugins and language servers");
        Entry("Ctrl+P then type", "Search every command, including plugin commands");
        Entry(":help", "Open this help window");
        Blank();

        Text("Tip: most commands accept lowercase or uppercase names, but arguments");
        Text("keep their exact spelling (for example :E MyFile.cs).");
        return lines.ToArray();
    }

    private void OpenHelp()
    {
        _helpVisible = true;
        _helpTop = 0;
        ClearMessage();
        _forceFullRedraw = true;
    }

    private void CloseHelp()
    {
        _helpVisible = false;
        _helpTop = 0;
        _forceFullRedraw = true;
    }

    private (int Width, int Height, int X, int Y, int Rows) HelpLayout()
    {
        int width = Math.Clamp(_columns - 6, 20, 110);
        int height = Math.Clamp(_rows - 4, 5, 38);
        width = Math.Min(width, Math.Max(10, _columns - 2));
        height = Math.Min(height, Math.Max(3, _rows - 2));
        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (_rows - height) / 2);
        return (width, height, x, y, Math.Max(1, height - 2));
    }

    private int HelpMaxTop()
    {
        (_, _, _, _, int rows) = HelpLayout();
        return Math.Max(0, HelpLines.Length - rows);
    }

    private void HandleHelpInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
            case Key.Enter:
                CloseHelp();
                return;
            case Key.Up:
            case Key.ScrollUp:
                _helpTop = Math.Max(
                    0,
                    _helpTop - (inputEvent.Key == Key.ScrollUp ? MouseScrollLines : 1)
                );
                return;
            case Key.Down:
            case Key.ScrollDown:
                _helpTop = Math.Min(
                    HelpMaxTop(),
                    _helpTop + (inputEvent.Key == Key.ScrollDown ? MouseScrollLines : 1)
                );
                return;
            case Key.PageUp:
            {
                (_, _, _, _, int rows) = HelpLayout();
                _helpTop = Math.Max(0, _helpTop - rows);
                return;
            }
            case Key.PageDown:
            {
                (_, _, _, _, int rows) = HelpLayout();
                _helpTop = Math.Min(HelpMaxTop(), _helpTop + rows);
                return;
            }
            case Key.Home:
                _helpTop = 0;
                return;
            case Key.End:
                _helpTop = HelpMaxTop();
                return;
        }

        if (
            inputEvent.Key == Key.Character
            && !inputEvent.Ctrl
            && !inputEvent.Alt
            && inputEvent.Character.Value is 'q' or 'Q'
        )
        {
            CloseHelp();
        }
    }

    private void RenderHelp()
    {
        if (!_helpVisible)
        {
            return;
        }

        _screen.Fill(0, 0, _columns, _rows, ' ', _theme.BaseStyle);
        (int width, int height, int x, int y, int rows) = HelpLayout();
        DrawBox(x, y, width, height, " Help ");

        _helpTop = Math.Clamp(_helpTop, 0, HelpMaxTop());
        var sectionStyle = _theme.AccentStyle;
        var keyStyle = _theme.SyntaxStyle(_theme.Accent);
        var textStyle = _theme.TextStyle;
        var mutedStyle = _theme.MutedStyle;
        int keyWidth = Math.Clamp(width / 3, 8, 30);
        keyWidth = Math.Min(keyWidth, Math.Max(4, width - 8));

        for (int row = 0; row < rows; ++row)
        {
            int index = _helpTop + row;
            if (index >= HelpLines.Length)
            {
                break;
            }

            HelpLine line = HelpLines[index];
            int screenRow = y + 1 + row;
            switch (line.Kind)
            {
                case HelpLineKind.Section:
                    _screen.PutString(
                        x + 2,
                        screenRow,
                        line.Left,
                        sectionStyle,
                        Math.Max(0, width - 4)
                    );
                    break;
                case HelpLineKind.Entry:
                    _screen.PutString(x + 2, screenRow, line.Left, keyStyle, Math.Max(0, keyWidth));
                    _screen.PutString(
                        x + 2 + keyWidth,
                        screenRow,
                        line.Right,
                        textStyle,
                        Math.Max(0, width - keyWidth - 4)
                    );
                    break;
                case HelpLineKind.Text:
                    _screen.PutString(
                        x + 2,
                        screenRow,
                        line.Left,
                        mutedStyle,
                        Math.Max(0, width - 4)
                    );
                    break;
            }
        }

        int maxTop = HelpMaxTop();
        if (maxTop > 0 && height > 2)
        {
            int trackHeight = Math.Max(1, height - 2);
            int thumbHeight = Math.Max(
                1,
                (int)Math.Round(trackHeight * (double)rows / HelpLines.Length)
            );
            int thumbTop =
                y + 1 + (int)Math.Round((trackHeight - thumbHeight) * (double)_helpTop / maxTop);
            for (int row = 0; row < trackHeight; ++row)
            {
                bool thumb = row >= thumbTop - (y + 1) && row < thumbTop - (y + 1) + thumbHeight;
                _screen.Put(
                    x + width - 2,
                    y + 1 + row,
                    thumb ? '\u2588' : '\u2502',
                    thumb ? keyStyle : mutedStyle
                );
            }
        }

        int hintRow = y + height;
        if (hintRow < _rows)
        {
            DrawCenteredLine(
                hintRow,
                "Esc / q close    Up/Down scroll    PgUp/PgDn page    Home/End",
                mutedStyle
            );
        }
    }
}
