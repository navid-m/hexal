using Hexal.Util;

namespace Hexal.Ui;

/// <summary>
/// Produces ANSI escape sequences for changed cells only.
/// </summary>
public sealed class Renderer
{
    private Cell[] _previous = Array.Empty<Cell>();
    private int _previousWidth;
    private int _previousHeight;
    private readonly ByteBuffer _output = new(64 * 1024);

    /// <summary>
    /// Forgets the previously rendered frame so the next render repaints every
    /// cell. Used by the hard-refresh path to clear terminal artefacts.
    /// </summary>
    public void Reset()
    {
        _previous = Array.Empty<Cell>();
        _previousWidth = 0;
        _previousHeight = 0;
        _output.Clear();
    }

    public ReadOnlySpan<byte> Render(Screen screen, bool forceFullRedraw = false)
    {
        _output.Clear();

        int width = screen.Width;
        int height = screen.Height;

        if (width <= 0 || height <= 0)
        {
            _previous = Array.Empty<Cell>();
            _previousWidth = width;
            _previousHeight = height;
            return ReadOnlySpan<byte>.Empty;
        }

        ReadOnlySpan<Cell> cells = screen.Cells;
        bool sizeChanged =
            width != _previousWidth
            || height != _previousHeight
            || _previous.Length != cells.Length;

        if (sizeChanged)
        {
            _output.Append("\x1b[2J"u8);
            forceFullRedraw = true;
        }

        bool wroteAny = sizeChanged;

        for (int y = 0; y < height; ++y)
        {
            int x = 0;
            while (x < width)
            {
                int index = (y * width) + x;
                bool changed =
                    forceFullRedraw
                    || index >= _previous.Length
                    || !cells[index].Equals(_previous[index]);

                if (!changed || cells[index].WideContinuation)
                {
                    ++x;
                    continue;
                }

                int runStart = x;
                while (x < width)
                {
                    int runIndex = (y * width) + x;
                    bool runChanged =
                        forceFullRedraw
                        || runIndex >= _previous.Length
                        || !cells[runIndex].Equals(_previous[runIndex]);
                    if (!runChanged)
                    {
                        break;
                    }
                    ++x;
                }

                wroteAny = true;
                _output.Append("\x1b["u8);
                AppendInt(_output, y + 1);
                _output.Append((byte)';');
                AppendInt(_output, runStart + 1);
                _output.Append((byte)'H');

                Style current = new();
                bool currentValid = false;

                for (int column = runStart; column < x; ++column)
                {
                    Cell cell = cells[(y * width) + column];
                    if (cell.WideContinuation)
                    {
                        continue;
                    }

                    if (!currentValid || cell.Style != current)
                    {
                        AppendStyle(_output, cell.Style);
                        current = cell.Style;
                        currentValid = true;
                    }

                    AppendCodePoint(_output, cell.Ch);
                }
            }
        }

        if (wroteAny)
        {
            _output.Append("\x1b[0m"u8);
        }

        if (_previous.Length != cells.Length)
        {
            _previous = new Cell[cells.Length];
        }
        cells.CopyTo(_previous);

        _previousWidth = width;
        _previousHeight = height;

        return _output.Span;
    }

    private static void AppendStyle(ByteBuffer output, Style style)
    {
        output.Append("\x1b[0"u8);

        if (style.Bold)
        {
            output.Append(";1"u8);
        }
        if (style.Dim)
        {
            output.Append(";2"u8);
        }
        if (style.Reverse)
        {
            output.Append(";7"u8);
        }

        uint fg = style.Fg;
        AppendColor(output, fg, foreground: true);

        uint bg = style.Bg;
        AppendColor(output, bg, foreground: false);

        output.Append((byte)'m');
    }

    private static void AppendColor(ByteBuffer output, uint color, bool foreground)
    {
        if (Theme.IsTrueColor(color))
        {
            output.Append(foreground ? ";38;2;"u8 : ";48;2;"u8);
            AppendInt(output, Theme.Red(color));
            output.Append((byte)';');
            AppendInt(output, Theme.Green(color));
            output.Append((byte)';');
            AppendInt(output, Theme.Blue(color));
            return;
        }

        if (color < 8)
        {
            output.Append(foreground ? ";3"u8 : ";4"u8);
            AppendInt(output, (int)color);
        }
        else if (color < 16)
        {
            output.Append(foreground ? ";9"u8 : ";10"u8);
            AppendInt(output, (int)(color - 8));
        }
        else
        {
            output.Append(foreground ? ";38;5;"u8 : ";48;5;"u8);
            AppendInt(output, (int)(color & 0xFF));
        }
    }

    private static void AppendCodePoint(ByteBuffer output, uint codePoint)
    {
        Span<byte> buffer = stackalloc byte[4];
        int length = UnicodeUtil.Encode(codePoint, buffer);
        output.Append(buffer[..length]);
    }

    private static void AppendInt(ByteBuffer output, int value)
    {
        if (value < 0)
        {
            output.Append((byte)'-');
            value = -value;
        }

        Span<byte> digits = stackalloc byte[12];
        int position = digits.Length;
        do
        {
            digits[--position] = (byte)('0' + (value % 10));
            value /= 10;
        } while (value > 0);

        output.Append(digits[position..]);
    }
}
