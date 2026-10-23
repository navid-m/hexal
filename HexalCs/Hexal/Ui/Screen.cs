using System.Globalization;
using Hexal.Util;

namespace Hexal.Ui;

public struct Style : IEquatable<Style>
{
    public uint Fg;
    public uint Bg;
    public bool Bold;
    public bool Dim;
    public bool Reverse;

    public Style()
    {
        Fg = 7;
        Bg = 0;
    }

    public readonly bool Equals(Style other) =>
        Fg == other.Fg
        && Bg == other.Bg
        && Bold == other.Bold
        && Dim == other.Dim
        && Reverse == other.Reverse;

    public override readonly bool Equals(object? obj) => obj is Style other && Equals(other);

    public override readonly int GetHashCode() => HashCode.Combine(Fg, Bg, Bold, Dim, Reverse);

    public static bool operator ==(Style left, Style right) => left.Equals(right);

    public static bool operator !=(Style left, Style right) => !left.Equals(right);
}

public struct Cell : IEquatable<Cell>
{
    public uint Ch;
    public Style Style;
    public bool WideContinuation;

    public Cell()
    {
        Ch = ' ';
        Style = new Style();
        WideContinuation = false;
    }

    public readonly bool Equals(Cell other) =>
        Ch == other.Ch && Style == other.Style && WideContinuation == other.WideContinuation;

    public override readonly bool Equals(object? obj) => obj is Cell other && Equals(other);

    public override readonly int GetHashCode() => HashCode.Combine(Ch, Style, WideContinuation);

    public static bool operator ==(Cell left, Cell right) => left.Equals(right);

    public static bool operator !=(Cell left, Cell right) => !left.Equals(right);
}

/// <summary>
/// A double-buffered cell grid. The renderer diffs two grids so only changed
/// cells are written to the terminal.
/// </summary>
public sealed class Screen
{
    private int _width;
    private int _height;
    private Cell[] _cells = Array.Empty<Cell>();

    public Screen()
    {
        Resize(0, 0);
    }

    public Screen(int width, int height)
    {
        Resize(width, height);
    }

    public int Width => _width;

    public int Height => _height;

    public ReadOnlySpan<Cell> Cells => _cells.AsSpan(0, _width * _height);

    public void Resize(int width, int height)
    {
        width = Math.Max(0, width);
        height = Math.Max(0, height);

        if (width == _width && height == _height)
        {
            return;
        }

        _width = width;
        _height = height;
        _cells = new Cell[width * height];
        Clear();
    }

    public void Clear(Style style = default)
    {
        // C++ default member initialisers give Style a foreground of 7. A C#
        // default(Style) is all zeroes, so restore the intended default when
        // the caller did not supply a style.
        if (style == default)
        {
            style = new Style();
        }

        Cell blank = new()
        {
            Ch = ' ',
            Style = style,
            WideContinuation = false,
        };
        Array.Fill(_cells, blank);
    }

    public bool InBounds(int x, int y) => x >= 0 && y >= 0 && x < _width && y < _height;

    public ref Cell At(int x, int y) => ref _cells[(y * _width) + x];

    public void Put(int x, int y, uint codePoint, Style style, int width = 1)
    {
        if (width <= 0 || !InBounds(x, y))
        {
            return;
        }
        if (width > 1 && x + width > _width)
        {
            return;
        }

        int index = (y * _width) + x;

        // Repair any wide scalar that we are overwriting from the side.
        if (x > 0 && _cells[index].WideContinuation)
        {
            _cells[index - 1] = new Cell { Ch = ' ', Style = _cells[index - 1].Style };
        }
        if (x + 1 < _width && _cells[index + 1].WideContinuation)
        {
            _cells[index + 1] = new Cell { Ch = ' ', Style = _cells[index + 1].Style };
        }

        _cells[index] = new Cell
        {
            Ch = codePoint,
            Style = style,
            WideContinuation = false,
        };
        for (int offset = 1; offset < width; ++offset)
        {
            _cells[index + offset] = new Cell
            {
                Ch = ' ',
                Style = style,
                WideContinuation = true,
            };
        }
    }

    public void PutAscii(
        int x,
        int y,
        ReadOnlySpan<char> text,
        Style style,
        int maxWidth = int.MaxValue
    )
    {
        int column = 0;
        foreach (char c in text)
        {
            if (column >= maxWidth || x + column >= _width)
            {
                break;
            }
            Put(x + column, y, c, style);
            ++column;
        }
    }

    public void PutString(
        int x,
        int y,
        ReadOnlySpan<char> text,
        Style style,
        int maxWidth = int.MaxValue
    )
    {
        int column = 0;
        int offset = 0;

        while (offset < text.Length && column < maxWidth && x + column < _width)
        {
            uint codePoint = UnicodeUtil.DecodeChar(text, ref offset);
            int width = UnicodeUtil.DisplayWidth(codePoint);
            if (width == 0)
            {
                // The cell model stores one scalar per cell; combining marks are
                // not rendered separately in the MVP.
                continue;
            }
            if (x + column + width > _width || column + width > maxWidth)
            {
                break;
            }

            Put(x + column, y, codePoint, style, width);
            column += width;
        }
    }

    public void PutUtf8(
        int x,
        int y,
        ReadOnlySpan<byte> text,
        Style style,
        int maxWidth = int.MaxValue
    )
    {
        int column = 0;
        int offset = 0;

        while (offset < text.Length && column < maxWidth && x + column < _width)
        {
            UnicodeUtil.Decode(text, offset, out System.Text.Rune rune, out int length);
            offset += length;

            int width = UnicodeUtil.DisplayWidth((uint)rune.Value);
            if (width == 0)
            {
                continue;
            }
            if (x + column + width > _width || column + width > maxWidth)
            {
                break;
            }

            Put(x + column, y, (uint)rune.Value, style, width);
            column += width;
        }
    }

    public void PutInt(int x, int y, long value, Style style, int maxWidth = int.MaxValue)
    {
        Span<char> buffer = stackalloc char[24];
        if (!value.TryFormat(buffer, out int written, default, CultureInfo.InvariantCulture))
        {
            return;
        }
        PutAscii(x, y, buffer[..written], style, maxWidth);
    }

    public void Fill(int x, int y, int width, int height, uint codePoint, Style style)
    {
        for (int row = 0; row < height; ++row)
        {
            for (int column = 0; column < width; ++column)
            {
                if (InBounds(x + column, y + row))
                {
                    Put(x + column, y + row, codePoint, style);
                }
            }
        }
    }

    /// <summary>
    /// Copies another screen into this one at the given origin. Used to compose
    /// pane content rendered in an isolated grid onto the terminal screen.
    /// </summary>
    public void Blit(Screen source, int targetX, int targetY)
    {
        int width = Math.Min(source._width, _width - targetX);
        int height = Math.Min(source._height, _height - targetY);
        if (width <= 0 || height <= 0)
        {
            return;
        }

        for (int row = 0; row < height; ++row)
        {
            Array.Copy(
                source._cells,
                row * source._width,
                _cells,
                (targetY + row) * _width + targetX,
                width
            );
        }
    }

    /// <summary>
    /// Applies a style to a cell, keeping both halves of a wide scalar in sync.
    /// Used for the block cursor.
    /// </summary>
    public void ApplyStyle(int x, int y, Style style)
    {
        if (!InBounds(x, y))
        {
            return;
        }

        int index = (y * _width) + x;
        _cells[index].Style = style;

        if (_cells[index].WideContinuation && x > 0)
        {
            _cells[index - 1].Style = style;
        }
        else if (x + 1 < _width && _cells[index + 1].WideContinuation)
        {
            _cells[index + 1].Style = style;
        }
    }
}
