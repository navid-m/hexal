using Hexal.Platform;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _binaryFilePromptVisible;
    private string _binaryFilePath = string.Empty;
    private long _binaryFileLine = -1;
    private long _binaryFileColumn = -1;
    private bool _binaryFileForce;

    private void ShowBinaryFilePrompt(string path, bool force, long line, long column)
    {
        _binaryFilePromptVisible = true;
        _binaryFilePath = path;
        _binaryFileLine = line;
        _binaryFileColumn = column;
        _binaryFileForce = force;
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void HandleBinaryFilePrompt(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            ClearBinaryFilePrompt();
            SetMessage("Cancelled.");
            return;
        }

        if (inputEvent.Key != Key.Enter)
        {
            return;
        }

        string path = _binaryFilePath;
        long line = _binaryFileLine;
        long column = _binaryFileColumn;
        bool force = _binaryFileForce;
        ClearBinaryFilePrompt();
        OpenPathWithDirtyCheck(path, force, line, column);
    }

    private void ClearBinaryFilePrompt()
    {
        _binaryFilePromptVisible = false;
        _binaryFilePath = string.Empty;
        _binaryFileLine = -1;
        _binaryFileColumn = -1;
        _binaryFileForce = false;
    }

    private void RenderBinaryFilePrompt()
    {
        if (!_binaryFilePromptVisible)
        {
            return;
        }

        const string label = "<Binary File>";
        const string hint = "Enter open  ·  Esc cancel";

        int width = Math.Min(Math.Max(label.Length + 6, 52), Math.Max(1, _columns - 2));
        int height = Math.Min(6, Math.Max(1, _rows - 2));
        if (width < 2 || height < 2)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (_rows - height) / 2);
        DrawBox(x, y, width, height, "Binary File");

        if (height < 3)
        {
            return;
        }

        int contentTop = y + 1;
        int contentRows = height - 2;
        int labelRow = contentRows >= 4 ? contentTop + 1 : contentTop;
        int labelX = x + Math.Max(0, (width - label.Length) / 2);
        _screen.PutString(labelX, labelRow, label, _theme.WarningStyle, Math.Max(0, width - 2));

        if (contentRows >= 3)
        {
            string path = _binaryFilePath;
            if (path.Length > width - 4)
            {
                path = "…" + path[^Math.Max(0, width - 5)..];
            }
            _screen.PutString(
                x + 2,
                labelRow + 1,
                path,
                _theme.MutedStyle,
                Math.Max(0, width - 4)
            );
        }

        if (contentRows >= 2)
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
