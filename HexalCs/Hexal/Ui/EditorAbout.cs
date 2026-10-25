using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const int AboutHeaderRows = 4;
    private const int AboutHintRows = 1;

    private static readonly string[] AboutBannerLines = Branding.Banner.Split('\n');
    private static readonly int AboutBannerWidth = AboutBannerLines.Max(line => line.Length);

    private bool _aboutVisible;
    private int _aboutTop;
    private int _aboutLeft;

    private void OpenAbout()
    {
        ClearMessage();
        _aboutVisible = true;
        _aboutTop = 0;
        _aboutLeft = 0;
        CenterAbout();
        _forceFullRedraw = true;
    }

    private void CloseAbout()
    {
        _aboutVisible = false;
        _aboutTop = 0;
        _aboutLeft = 0;
        _forceFullRedraw = true;
    }

    private void CenterAbout()
    {
        int viewportRows = AboutViewportRows();
        _aboutTop = Math.Max(0, (AboutBannerLines.Length - viewportRows) / 2);
        _aboutLeft = Math.Max(0, (AboutBannerWidth - _columns) / 2);
    }

    private int AboutViewportRows()
    {
        int available = Math.Max(1, _rows - AboutHintRows);
        int headerRows = Math.Min(AboutHeaderRows, available);
        return Math.Max(1, available - headerRows);
    }

    private int AboutMaxTop() => Math.Max(0, AboutBannerLines.Length - AboutViewportRows());

    private int AboutMaxLeft() => Math.Max(0, AboutBannerWidth - _columns);

    private void HandleAboutInput(InputEvent inputEvent)
    {
        int maxTop = AboutMaxTop();
        int maxLeft = AboutMaxLeft();

        switch (inputEvent.Key)
        {
            case Key.Escape:
            case Key.Enter:
                CloseAbout();
                return;
            case Key.Up:
            case Key.ScrollUp:
                _aboutTop = Math.Max(
                    0,
                    _aboutTop - (inputEvent.Key == Key.ScrollUp ? MouseScrollLines : 1)
                );
                return;
            case Key.Down:
            case Key.ScrollDown:
                _aboutTop = Math.Min(
                    maxTop,
                    _aboutTop + (inputEvent.Key == Key.ScrollDown ? MouseScrollLines : 1)
                );
                return;
            case Key.Left:
                _aboutLeft = Math.Max(0, _aboutLeft - 1);
                return;
            case Key.Right:
                _aboutLeft = Math.Min(maxLeft, _aboutLeft + 1);
                return;
            case Key.PageUp:
                _aboutTop = Math.Max(0, _aboutTop - AboutViewportRows());
                return;
            case Key.PageDown:
                _aboutTop = Math.Min(maxTop, _aboutTop + AboutViewportRows());
                return;
            case Key.Home:
                _aboutTop = 0;
                _aboutLeft = 0;
                return;
            case Key.End:
                _aboutTop = maxTop;
                _aboutLeft = maxLeft;
                return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Character.Value is 'q' or 'Q')
        {
            CloseAbout();
        }
    }

    private void RenderAbout()
    {
        if (!_aboutVisible)
        {
            return;
        }

        _screen.Fill(0, 0, _columns, _rows, ' ', _theme.BaseStyle);

        int available = Math.Max(1, _rows - AboutHintRows);
        int viewportRows = AboutViewportRows();
        _aboutTop = Math.Clamp(_aboutTop, 0, AboutMaxTop());

        int contentHeight = AboutHeaderRows + AboutBannerLines.Length;
        int headerY = contentHeight <= available ? Math.Max(0, (available - contentHeight) / 2) : 0;

        DrawCenteredLine(headerY, Branding.Title, _theme.AccentStyle);
        DrawCenteredLine(headerY + 1, Branding.Author, _theme.MutedStyle);
        DrawCenteredLine(headerY + 2, Branding.Commit, _theme.MutedStyle);

        _aboutLeft = Math.Clamp(_aboutLeft, 0, AboutMaxLeft());
        int artX = AboutBannerWidth <= _columns ? (_columns - AboutBannerWidth) / 2 : 0;
        int artY = headerY + AboutHeaderRows;

        for (int row = 0; row < viewportRows; ++row)
        {
            int index = _aboutTop + row;
            if (index >= AboutBannerLines.Length)
            {
                break;
            }

            string line = AboutBannerLines[index];
            int column = Math.Min(_aboutLeft, line.Length);
            _screen.PutString(
                artX,
                artY + row,
                line.AsSpan(column),
                _theme.TextStyle,
                Math.Max(0, _columns - artX)
            );
        }

        DrawCenteredLine(_rows - 1, AboutHint(), _theme.MutedStyle);
    }

    private string AboutHint()
    {
        var parts = new List<string> { "Esc close" };
        if (AboutBannerLines.Length > AboutViewportRows())
        {
            parts.Add("Up/Down scroll");
        }
        if (AboutBannerWidth > _columns)
        {
            parts.Add("Left/Right pan");
        }
        return string.Join("  |  ", parts);
    }

    private void DrawCenteredLine(int y, string text, Style style)
    {
        if (y < 0 || y >= _rows)
        {
            return;
        }

        int width = (int)UnicodeUtil.ColumnWidth(text);
        int x = Math.Max(0, (_columns - width) / 2);
        _screen.PutString(x, y, text, style, Math.Max(0, _columns - x));
    }
}
