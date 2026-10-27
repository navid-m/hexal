using System.Globalization;
using System.Text;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _gotoLineVisible;
    private readonly StringBuilder _gotoLineText = new();
    private int _gotoLineCursor;
    private string _gotoLineError = string.Empty;

    private void OpenGoToLine()
    {
        _gotoLineVisible = true;
        _gotoLineText.Clear();
        _gotoLineCursor = 0;
        _gotoLineError = string.Empty;
        _spaceMenuOpen = false;
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void ClearGoToLine()
    {
        _gotoLineVisible = false;
        _gotoLineText.Clear();
        _gotoLineCursor = 0;
        _gotoLineError = string.Empty;
    }

    private void HandleGoToLine(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            ClearGoToLine();
            SetMessage("Cancelled.");
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Enter:
                GoToTypedLine();
                return;
            case Key.Backspace:
                if (_gotoLineCursor > 0)
                {
                    string text = _gotoLineText.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, _gotoLineCursor);
                    _gotoLineText.Remove(previous, _gotoLineCursor - previous);
                    _gotoLineCursor = previous;
                    _gotoLineError = string.Empty;
                }
                return;
            case Key.Delete:
                if (_gotoLineCursor < _gotoLineText.Length)
                {
                    string text = _gotoLineText.ToString();
                    int next = UnicodeUtil.NextBoundary(text, _gotoLineCursor);
                    _gotoLineText.Remove(_gotoLineCursor, next - _gotoLineCursor);
                    _gotoLineError = string.Empty;
                }
                return;
            case Key.Left:
                _gotoLineCursor = UnicodeUtil.PreviousBoundary(
                    _gotoLineText.ToString(),
                    _gotoLineCursor
                );
                return;
            case Key.Right:
                _gotoLineCursor = UnicodeUtil.NextBoundary(
                    _gotoLineText.ToString(),
                    _gotoLineCursor
                );
                return;
            case Key.Home:
                _gotoLineCursor = 0;
                return;
            case Key.End:
                _gotoLineCursor = _gotoLineText.Length;
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Alt)
        {
            return;
        }

        // The field only accepts an ASCII line number, so anything else
        // (including pasted text) is ignored instead of producing an error.
        if (inputEvent.Character.Value is < 0x30 or > 0x39)
        {
            return;
        }

        _gotoLineText.Insert(_gotoLineCursor, inputEvent.Character.ToString());
        _gotoLineCursor += 1;
        _gotoLineError = string.Empty;
    }

    private void GoToTypedLine()
    {
        string text = _gotoLineText.ToString();
        if (text.Length == 0)
        {
            ClearGoToLine();
            SetMessage("Cancelled.");
            return;
        }

        if (
            !int.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out int lineNumber)
            || lineNumber < 1
            || lineNumber > _document.LineCount
        )
        {
            _gotoLineError = $"Line must be between 1 and {_document.LineCount}";
            return;
        }

        long target = lineNumber - 1;
        ClearGoToLine();

        long previous = _cursor;
        MoveToLineColumn(target, 0);
        if (_cursor != previous)
        {
            RecordJump(previous);
        }

        // A jump should not keep a selection or extra cursors alive at the
        // old location.
        _mode = Mode.Normal;
        _selectionLinewise = false;
        _selectionAnchor = _cursor;
        _extraSelections.Clear();

        _freeScroll = false;
        int rows = ActivePaneRows();
        _topLine = Math.Max(0, target - Math.Max(0, rows / 2));
        EnsureCursorVisible();

        SetMessage($"Line {lineNumber}");
    }

    private void RenderGoToLine()
    {
        if (!_gotoLineVisible)
        {
            return;
        }

        const string prompt = "Line: ";
        const string hint = "Enter jump  ·  Esc cancel";

        int width = Math.Min(Math.Max(hint.Length + 6, 44), Math.Max(1, _columns - 2));
        int height = Math.Min(5, Math.Max(1, _rows - 2));
        if (width < 2 || height < 2)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (_rows - height) / 2);
        DrawBox(x, y, width, height, "Go to...");

        if (height < 3)
        {
            return;
        }

        int contentTop = y + 1;
        int promptX = x + 2;
        _screen.PutString(promptX, contentTop, prompt, _theme.AccentStyle, width - 4);

        string text = _gotoLineText.ToString();
        int inputX = promptX + prompt.Length;
        int inputWidth = width - 4 - prompt.Length;
        if (inputWidth > 0)
        {
            _screen.PutString(inputX, contentTop, text, _theme.TextStyle, inputWidth);

            long column = UnicodeUtil.VisualColumn(text, _gotoLineCursor, 1);
            int cursorX = inputX + (int)column;
            if (cursorX >= inputX && cursorX < x + width - 1 && cursorX < _columns)
            {
                Style cursorStyle = _screen.At(cursorX, contentTop).Style;
                cursorStyle.Reverse = true;
                _screen.ApplyStyle(cursorX, contentTop, cursorStyle);
            }
        }

        if (height < 4)
        {
            return;
        }

        bool hasError = _gotoLineError.Length > 0;
        string status = hasError ? _gotoLineError : $"1-{_document.LineCount}";
        Style statusStyle = hasError ? _theme.ErrorStyle : _theme.MutedStyle;
        _screen.PutString(x + 2, contentTop + 1, status, statusStyle, width - 4);

        if (height >= 5)
        {
            int hintX = x + Math.Max(0, (width - hint.Length) / 2);
            _screen.PutString(
                hintX,
                y + height - 2,
                hint,
                _theme.MutedStyle,
                Math.Max(0, width - 2)
            );
        }
    }
}
