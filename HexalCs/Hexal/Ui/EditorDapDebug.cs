using System.Text;
using Hexal.Platform;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _dapDebugVisible;
    private int _dapDebugTop;
    private string _dapDebugStatus = string.Empty;
    private readonly List<string> _dapDebugLines = new();

    private void OpenDapDebug()
    {
        _dapDebugVisible = true;
        _dapDebugTop = 0;
        _dapDebugStatus = string.Empty;
        RefreshDapDebugLines();
        ClearMessage();
        _forceFullRedraw = true;
    }

    private void CloseDapDebug()
    {
        _dapDebugVisible = false;
        _dapDebugTop = 0;
        _dapDebugStatus = string.Empty;
        _dapDebugLines.Clear();
        _forceFullRedraw = true;
    }

    private int DapDebugContentRows() => Math.Max(1, _rows - 3);

    private int DapDebugMaxTop()
        => Math.Max(0, _dapDebugLines.Count - DapDebugContentRows());

    private void RefreshDapDebugLines()
    {
        int width = Math.Max(10, _columns - 4);
        string report = _dap.BuildDebugReport();

        _dapDebugLines.Clear();
        foreach (string rawLine in report
                     .Replace("\r\n", "\n")
                     .Replace('\r', '\n')
                     .Split('\n'))
        {
            if (rawLine.Length == 0)
            {
                _dapDebugLines.Add(string.Empty);
                continue;
            }

            List<string> wrapped = WrapText(rawLine, width);
            if (wrapped.Count == 0)
            {
                _dapDebugLines.Add(string.Empty);
            }
            else
            {
                _dapDebugLines.AddRange(wrapped);
            }
        }
    }

    private void HandleDapDebugInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
            case Key.Enter:
                CloseDapDebug();
                return;
            case Key.Up:
                ScrollDapDebug(-1);
                return;
            case Key.Down:
                ScrollDapDebug(1);
                return;
            case Key.ScrollUp:
                ScrollDapDebug(-MouseScrollLines);
                return;
            case Key.ScrollDown:
                ScrollDapDebug(MouseScrollLines);
                return;
            case Key.PageUp:
                ScrollDapDebug(-DapDebugContentRows());
                return;
            case Key.PageDown:
                ScrollDapDebug(DapDebugContentRows());
                return;
            case Key.Home:
                RefreshDapDebugLines();
                _dapDebugTop = 0;
                return;
            case Key.End:
                RefreshDapDebugLines();
                _dapDebugTop = DapDebugMaxTop();
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
                CloseDapDebug();
                return;
            case 'j':
            case 'J':
                ScrollDapDebug(1);
                return;
            case 'k':
            case 'K':
                ScrollDapDebug(-1);
                return;
            case 'g':
            case 'G':
                RefreshDapDebugLines();
                _dapDebugTop = inputEvent.Character.Value == 'g'
                    ? 0
                    : DapDebugMaxTop();
                return;
            case 'y':
            case 'Y':
                CopyDapDebugReport();
                return;
            case 'c':
            case 'C':
                _dap.ClearLog();
                _dapDebugStatus = "Log cleared.";
                RefreshDapDebugLines();
                _dapDebugTop = 0;
                _forceFullRedraw = true;
                return;
        }
    }

    private void ScrollDapDebug(int delta)
    {
        RefreshDapDebugLines();
        int maxTop = DapDebugMaxTop();
        _dapDebugTop = Math.Clamp(_dapDebugTop + delta, 0, maxTop);
    }

    private void CopyDapDebugReport()
    {
        string report = _dap.BuildDebugReport();
        if (!Clipboard.TryCopy(_terminal, Encoding.UTF8.GetBytes(report),
                out string error))
        {
            _dapDebugStatus = $"Copy failed: {error}";
            return;
        }

        _dapDebugStatus = $"Copied {report.Length} characters to the clipboard.";
    }

    private void RenderDapDebug()
    {
        if (!_dapDebugVisible)
        {
            return;
        }

        RefreshDapDebugLines();

        int width = _columns;
        int height = _rows;
        if (width < 4 || height < 3)
        {
            return;
        }

        DrawBox(0, 0, width, height, " DAP Debug ");

        int contentRows = Math.Max(1, height - 3);
        int maxTop = Math.Max(0, _dapDebugLines.Count - contentRows);
        _dapDebugTop = Math.Clamp(_dapDebugTop, 0, maxTop);
        int contentWidth = Math.Max(0, width - 4);

        for (int row = 0; row < contentRows; ++row)
        {
            int index = _dapDebugTop + row;
            if (index >= _dapDebugLines.Count)
            {
                break;
            }

            _screen.PutString(2, 1 + row, _dapDebugLines[index],
                _theme.TextStyle, contentWidth);
        }

        const string hint =
            "Esc close  |  j/k/arrows scroll  |  Home/End  |  y copy  |  c clear";
        string footer = _dapDebugStatus.Length > 0
            ? $"{_dapDebugStatus}  |  {hint}"
            : hint;
        _screen.PutString(2, height - 2, footer, _theme.MutedStyle,
            Math.Max(0, width - 4));
    }
}
