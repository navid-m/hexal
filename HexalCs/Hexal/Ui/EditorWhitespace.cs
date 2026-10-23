namespace Hexal.Ui;

public sealed partial class Editor
{
    private int TrimTrailingWhitespaceInBuffer()
    {
        List<FormatSelectionSnapshot> selections = CaptureFormatSelections();
        int lines = _document.TrimTrailingWhitespace();
        if (lines == 0)
        {
            return 0;
        }

        SetSelections(RestoreFormatSelections(selections));
        _lspDirty = _lsp.IsRunning;
        _lspDirtySince = Environment.TickCount64;
        return lines;
    }

    private void CleanWhitespace()
    {
        int lines = TrimTrailingWhitespaceInBuffer();
        if (lines == 0)
        {
            SetMessage("No trailing whitespace.");
            return;
        }

        SetMessage(
            $"Removed trailing whitespace from {lines} "
                + $"line{(lines == 1 ? string.Empty : "s")}."
        );
    }

    private bool BackspaceIndentation(long lineStart)
    {
        if (!_config.SmartBackspace || _cursor <= lineStart)
        {
            return false;
        }

        for (long offset = lineStart; offset < _cursor; ++offset)
        {
            byte value = _document.ByteAt(offset);
            if (value != (byte)' ' && value != (byte)'\t')
            {
                return false;
            }
        }

        _document.Erase(lineStart, _cursor - lineStart);
        _cursor = lineStart;
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
        return true;
    }
}
