using System.Text;
using Hexal.Platform;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _lspDebugVisible;
    private int _lspDebugTop;
    private string _lspDebugStatus = string.Empty;
    private readonly List<string> _lspDebugLines = new();

    private void OpenLspDebug()
    {
        _lspDebugVisible = true;
        _lspDebugTop = 0;
        _lspDebugStatus = string.Empty;
        RefreshLspDebugLines();
        ClearMessage();
        _forceFullRedraw = true;
    }

    private void CloseLspDebug()
    {
        _lspDebugVisible = false;
        _lspDebugTop = 0;
        _lspDebugStatus = string.Empty;
        _lspDebugLines.Clear();
        _forceFullRedraw = true;
    }

    private int LspDebugContentRows() => Math.Max(1, _rows - 3);

    private int LspDebugMaxTop() => Math.Max(0, _lspDebugLines.Count - LspDebugContentRows());

    private void RefreshLspDebugLines()
    {
        int width = Math.Max(10, _columns - 4);
        string report = _lsp.BuildDebugReport();

        _lspDebugLines.Clear();
        foreach (string rawLine in report.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n'))
        {
            if (rawLine.Length == 0)
            {
                _lspDebugLines.Add(string.Empty);
                continue;
            }

            List<string> wrapped = WrapText(rawLine, width);
            if (wrapped.Count == 0)
            {
                _lspDebugLines.Add(string.Empty);
            }
            else
            {
                _lspDebugLines.AddRange(wrapped);
            }
        }
    }

    private void HandleLspDebugInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
            case Key.Enter:
                CloseLspDebug();
                return;
            case Key.Up:
                ScrollLspDebug(-1);
                return;
            case Key.Down:
                ScrollLspDebug(1);
                return;
            case Key.ScrollUp:
                ScrollLspDebug(-MouseScrollLines);
                return;
            case Key.ScrollDown:
                ScrollLspDebug(MouseScrollLines);
                return;
            case Key.PageUp:
                ScrollLspDebug(-LspDebugContentRows());
                return;
            case Key.PageDown:
                ScrollLspDebug(LspDebugContentRows());
                return;
            case Key.Home:
                RefreshLspDebugLines();
                _lspDebugTop = 0;
                return;
            case Key.End:
                RefreshLspDebugLines();
                _lspDebugTop = LspDebugMaxTop();
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl || inputEvent.Alt)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
            case 'q':
            case 'Q':
                CloseLspDebug();
                return;
            case 'j':
            case 'J':
                ScrollLspDebug(1);
                return;
            case 'k':
            case 'K':
                ScrollLspDebug(-1);
                return;
            case 'g':
            case 'G':
                RefreshLspDebugLines();
                _lspDebugTop = inputEvent.Character.Value == 'g' ? 0 : LspDebugMaxTop();
                return;
            case 'y':
            case 'Y':
                CopyLspDebugReport();
                return;
            case 'c':
            case 'C':
                _lsp.ClearLog();
                _lspDebugStatus = "Log cleared.";
                RefreshLspDebugLines();
                _lspDebugTop = 0;
                _forceFullRedraw = true;
                return;
        }
    }

    private void ScrollLspDebug(int delta)
    {
        RefreshLspDebugLines();
        int maxTop = LspDebugMaxTop();
        _lspDebugTop = Math.Clamp(_lspDebugTop + delta, 0, maxTop);
    }

    private void CopyLspDebugReport()
    {
        string report = _lsp.BuildDebugReport();
        if (!Clipboard.TryCopy(_terminal, Encoding.UTF8.GetBytes(report), out string error))
        {
            _lspDebugStatus = $"Copy failed: {error}";
            return;
        }

        _lspDebugStatus = $"Copied {report.Length} characters to the clipboard.";
    }

    private void RenderLspDebug()
    {
        if (!_lspDebugVisible)
        {
            return;
        }

        RefreshLspDebugLines();

        int width = _columns;
        int height = _rows;
        if (width < 4 || height < 3)
        {
            return;
        }

        DrawBox(0, 0, width, height, " LSP Debug ");

        int contentRows = Math.Max(1, height - 3);
        int maxTop = Math.Max(0, _lspDebugLines.Count - contentRows);
        _lspDebugTop = Math.Clamp(_lspDebugTop, 0, maxTop);
        int contentWidth = Math.Max(0, width - 4);

        for (int row = 0; row < contentRows; ++row)
        {
            int index = _lspDebugTop + row;
            if (index >= _lspDebugLines.Count)
            {
                break;
            }

            _screen.PutString(2, 1 + row, _lspDebugLines[index], _theme.TextStyle, contentWidth);
        }

        const string hint = "Esc close  |  j/k/arrows scroll  |  Home/End  |  y copy  |  c clear";
        string footer = _lspDebugStatus.Length > 0 ? $"{_lspDebugStatus}  |  {hint}" : hint;
        _screen.PutString(2, height - 2, footer, _theme.MutedStyle, Math.Max(0, width - 4));
    }
}
