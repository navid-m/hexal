using System.Globalization;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _benchmarkVisible;
    private int _benchmarkTop;

    private void OpenStartupBenchmark()
    {
        if (_benchmark is null)
        {
            return;
        }

        ClearMessage();
        _benchmarkVisible = true;
        _benchmarkTop = 0;
        _forceFullRedraw = true;
    }

    private void CloseStartupBenchmark()
    {
        _benchmarkVisible = false;
        _benchmarkTop = 0;
        _forceFullRedraw = true;
    }

    private void HandleStartupBenchmarkInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
                CloseStartupBenchmark();
                return;
            case Key.Up:
            case Key.ScrollUp:
                _benchmarkTop = Math.Max(
                    0,
                    _benchmarkTop - (inputEvent.Key == Key.ScrollUp ? MouseScrollLines : 1)
                );
                return;
            case Key.Down:
            case Key.ScrollDown:
                _benchmarkTop = Math.Min(
                    BenchmarkMaxTop(),
                    _benchmarkTop + (inputEvent.Key == Key.ScrollDown ? MouseScrollLines : 1)
                );
                return;
            case Key.PageUp:
                _benchmarkTop = Math.Max(0, _benchmarkTop - BenchmarkContentRows());
                return;
            case Key.PageDown:
                _benchmarkTop = Math.Min(BenchmarkMaxTop(), _benchmarkTop + BenchmarkContentRows());
                return;
            case Key.Home:
                _benchmarkTop = 0;
                return;
            case Key.End:
                _benchmarkTop = BenchmarkMaxTop();
                return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Character.Value is 'q' or 'Q')
        {
            CloseStartupBenchmark();
        }
    }

    private int BenchmarkContentRows()
    {
        int available = (int)Math.Max(0, StatusRow());
        int height = BenchmarkHeight(available);
        return Math.Max(1, height - 4);
    }

    private int BenchmarkHeight(int available)
    {
        int entries = _benchmark?.Entries.Count ?? 0;
        int desired = Math.Max(8, entries + 4);
        return Math.Min(available, desired);
    }

    private int BenchmarkMaxTop()
    {
        int entries = _benchmark?.Entries.Count ?? 0;
        return Math.Max(0, entries - BenchmarkContentRows());
    }

    private void RenderStartupBenchmark()
    {
        if (!_benchmarkVisible || _benchmark is null)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        if (available < 6 || _columns < 32)
        {
            return;
        }

        int height = BenchmarkHeight(available);
        int width = Math.Min(_columns - 2, 120);
        if (width < 32 || height < 6)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (available - height) / 2);
        DrawBox(x, y, width, height, " Startup benchmark ");

        IReadOnlyList<StartupBenchmarkEntry> entries = _benchmark.SortedByDuration();
        int contentRows = Math.Max(1, height - 4);
        int contentWidth = Math.Max(0, width - 4);
        _benchmarkTop = Math.Clamp(_benchmarkTop, 0, Math.Max(0, entries.Count - contentRows));

        _screen.PutString(x + 2, y + 1, "Duration", _theme.AccentStyle, contentWidth);
        _screen.PutString(
            x + 13,
            y + 1,
            "Component",
            _theme.AccentStyle,
            Math.Max(0, contentWidth - 11)
        );

        int durationWidth = 10;
        for (int row = 0; row < contentRows; ++row)
        {
            int index = _benchmarkTop + row;
            if (index >= entries.Count)
            {
                break;
            }

            StartupBenchmarkEntry entry = entries[index];
            int screenRow = y + 2 + row;
            string duration =
                entry.Milliseconds.ToString("0.00", CultureInfo.InvariantCulture) + " ms";
            Style durationStyle =
                entry.Milliseconds >= 100 ? _theme.WarningStyle
                : entry.Milliseconds >= 25 ? _theme.AccentStyle
                : _theme.TextStyle;
            _screen.PutString(
                x + 2,
                screenRow,
                duration.PadLeft(durationWidth),
                durationStyle,
                contentWidth
            );

            int column = durationWidth + 1;
            string label = $"{entry.Category}: {entry.Name}";
            _screen.PutString(
                x + 2 + column,
                screenRow,
                label,
                _theme.TextStyle,
                Math.Max(0, contentWidth - column)
            );
            column += (int)UnicodeUtil.ColumnWidth(label);

            if (entry.Detail.Length > 0 && column + 2 < contentWidth)
            {
                _screen.PutString(
                    x + 2 + column,
                    screenRow,
                    "  " + entry.Detail,
                    _theme.MutedStyle,
                    Math.Max(0, contentWidth - column - 2)
                );
            }
        }

        string footer =
            $"{entries.Count} measurement"
            + $"{(entries.Count == 1 ? string.Empty : "s")} · "
            + "slowest first · Esc/q close";
        _screen.PutString(x + 2, y + height - 2, footer, _theme.MutedStyle, width - 4);
    }
}
