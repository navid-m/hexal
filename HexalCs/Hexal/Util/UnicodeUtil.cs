using System.Buffers;
using System.Globalization;
using System.Text;

namespace Hexal.Util;

/// <summary>
/// UTF-8 and terminal-cell helpers. Three coordinate systems are kept distinct:
/// byte offsets, Unicode scalar values, and terminal display columns.
///
/// Invalid UTF-8 is preserved on disk. When it must be interpreted for the UI,
/// each malformed byte is treated as U+FFFD and consumes exactly one byte.
/// </summary>
public static class UnicodeUtil
{
    public static bool Decode(ReadOnlySpan<byte> text, int offset, out Rune rune, out int length)
    {
        rune = Rune.ReplacementChar;
        length = 1;

        if ((uint)offset >= (uint)text.Length)
        {
            return false;
        }

        OperationStatus status = Rune.DecodeFromUtf8(text[offset..], out rune, out int consumed);
        if (status == OperationStatus.Done && consumed > 0 && offset + consumed <= text.Length)
        {
            length = consumed;
            return true;
        }

        rune = Rune.ReplacementChar;
        length = 1;
        return false;
    }

    public static int Encode(uint codePoint, Span<byte> destination)
    {
        if (!Rune.TryCreate((int)codePoint, out Rune rune))
        {
            rune = Rune.ReplacementChar;
        }

        return rune.EncodeToUtf8(destination);
    }

    public static void Append(ByteBuffer buffer, uint codePoint)
    {
        Span<byte> encoded = stackalloc byte[4];
        int length = Encode(codePoint, encoded);
        buffer.Append(encoded[..length]);
    }

    public static int PreviousBoundary(ReadOnlySpan<byte> text, int offset)
    {
        if (offset <= 0)
        {
            return 0;
        }

        offset = Math.Min(offset, text.Length);
        int candidate = offset - 1;
        int back = 0;
        while (candidate > 0 && back < 3 && IsContinuation(text[candidate]))
        {
            --candidate;
            ++back;
        }

        if (Decode(text, candidate, out _, out int length) && candidate + length == offset)
        {
            return candidate;
        }

        return offset - 1;
    }

    public static int NextBoundary(ReadOnlySpan<byte> text, int offset)
    {
        if (offset >= text.Length)
        {
            return text.Length;
        }

        Decode(text, offset, out _, out int length);
        return Math.Min(text.Length, offset + length);
    }

    public static int PreviousBoundary(ReadOnlySpan<char> text, int offset)
    {
        if (offset <= 0)
        {
            return 0;
        }

        offset = Math.Min(offset, text.Length);
        int candidate = offset - 1;
        if (
            candidate > 0
            && char.IsLowSurrogate(text[candidate])
            && char.IsHighSurrogate(text[candidate - 1])
        )
        {
            return candidate - 1;
        }

        return candidate;
    }

    public static int NextBoundary(ReadOnlySpan<char> text, int offset)
    {
        if (offset >= text.Length)
        {
            return text.Length;
        }

        if (
            char.IsHighSurrogate(text[offset])
            && offset + 1 < text.Length
            && char.IsLowSurrogate(text[offset + 1])
        )
        {
            return offset + 2;
        }

        return offset + 1;
    }

    public static bool IsWide(uint cp)
    {
        return InRange(cp, 0x1100, 0x115F)
            || InRange(cp, 0x2329, 0x232A)
            || InRange(cp, 0x2E80, 0x303E)
            || InRange(cp, 0x3041, 0x33FF)
            || InRange(cp, 0x3400, 0x4DBF)
            || InRange(cp, 0x4E00, 0x9FFF)
            || InRange(cp, 0xA000, 0xA4CF)
            || InRange(cp, 0xA960, 0xA97F)
            || InRange(cp, 0xAC00, 0xD7A3)
            || InRange(cp, 0xF900, 0xFAFF)
            || InRange(cp, 0xFE10, 0xFE19)
            || InRange(cp, 0xFE30, 0xFE6F)
            || InRange(cp, 0xFF00, 0xFF60)
            || InRange(cp, 0xFFE0, 0xFFE6)
            || InRange(cp, 0x1F300, 0x1FAFF)
            || InRange(cp, 0x20000, 0x3FFFD);
    }

    public static int DisplayWidth(uint cp)
    {
        if (cp == 0 || cp < 0x20 || (cp >= 0x7F && cp < 0xA0))
        {
            return 0;
        }

        if (
            InRange(cp, 0x0300, 0x036F)
            || InRange(cp, 0x0483, 0x0489)
            || InRange(cp, 0x0591, 0x05BD)
            || InRange(cp, 0x0610, 0x061A)
            || InRange(cp, 0x064B, 0x065F)
            || InRange(cp, 0x0670, 0x0670)
            || InRange(cp, 0x06D6, 0x06DC)
            || InRange(cp, 0x0E31, 0x0E31)
            || InRange(cp, 0x0E34, 0x0E3A)
            || InRange(cp, 0x1AB0, 0x1AFF)
            || InRange(cp, 0x1DC0, 0x1DFF)
            || InRange(cp, 0x200B, 0x200F)
            || InRange(cp, 0x20D0, 0x20FF)
            || InRange(cp, 0xFE00, 0xFE0F)
            || InRange(cp, 0xFE20, 0xFE2F)
            || InRange(cp, 0xE0100, 0xE01EF)
        )
        {
            return 0;
        }

        return IsWide(cp) ? 2 : 1;
    }

    /// <summary>
    /// Formatting and separator code points that terminals may interpret as
    /// bidirectional overrides or otherwise render with an unexpected width.
    /// Callers that draw one scalar per cell should replace these with U+FFFD.
    /// </summary>
    public static bool IsUnsafeFormatCharacter(uint cp)
    {
        if (cp < 0x80)
        {
            return false;
        }

        UnicodeCategory category = Rune.GetUnicodeCategory(new Rune((int)cp));
        return category
            is UnicodeCategory.Format
                or UnicodeCategory.Surrogate
                or UnicodeCategory.LineSeparator
                or UnicodeCategory.ParagraphSeparator
                or UnicodeCategory.OtherNotAssigned;
    }

    public static long VisualColumn(ReadOnlySpan<byte> line, int byteOffset, int tabWidth)
    {
        tabWidth = Math.Max(1, tabWidth);
        byteOffset = Math.Clamp(byteOffset, 0, line.Length);

        long column = 0;
        int offset = 0;
        while (offset < byteOffset)
        {
            Decode(line, offset, out Rune rune, out int length);
            offset += length;

            if (rune.Value == '\t')
            {
                column += tabWidth - (column % tabWidth);
            }
            else
            {
                int width = DisplayWidth((uint)rune.Value);
                if (width > 0)
                {
                    column += width;
                }
            }
        }

        return column;
    }

    public static long VisualColumn(ReadOnlySpan<char> text, int charOffset, int tabWidth)
    {
        tabWidth = Math.Max(1, tabWidth);
        charOffset = Math.Clamp(charOffset, 0, text.Length);

        long column = 0;
        int offset = 0;
        while (offset < charOffset)
        {
            uint cp = DecodeChar(text, ref offset);
            if (cp == '\t')
            {
                column += tabWidth - (column % tabWidth);
            }
            else
            {
                int width = DisplayWidth(cp);
                if (width > 0)
                {
                    column += width;
                }
            }
        }

        return column;
    }

    public static long ColumnWidth(ReadOnlySpan<char> text) => VisualColumn(text, text.Length, 1);

    public static int ByteOffsetForVisualColumn(ReadOnlySpan<byte> line, long column, int tabWidth)
    {
        tabWidth = Math.Max(1, tabWidth);
        long visual = 0;
        int offset = 0;

        while (offset < line.Length)
        {
            Decode(line, offset, out Rune rune, out int length);

            long next = visual;
            if (rune.Value == '\t')
            {
                next += tabWidth - (next % tabWidth);
            }
            else
            {
                int width = DisplayWidth((uint)rune.Value);
                if (width > 0)
                {
                    next += width;
                }
            }

            if (next > column)
            {
                return offset;
            }

            visual = next;
            offset += length;
            if (visual == column)
            {
                return offset;
            }
        }

        return line.Length;
    }

    public static uint DecodeChar(ReadOnlySpan<char> text, ref int offset)
    {
        if ((uint)offset >= (uint)text.Length)
        {
            return 0xFFFD;
        }

        char first = text[offset];
        if (
            char.IsHighSurrogate(first)
            && offset + 1 < text.Length
            && char.IsLowSurrogate(text[offset + 1])
        )
        {
            uint value = (uint)char.ConvertToUtf32(first, text[offset + 1]);
            offset += 2;
            return value;
        }

        if (char.IsSurrogate(first))
        {
            ++offset;
            return 0xFFFD;
        }

        ++offset;
        return first;
    }

    public static string FormatLong(long value) => value.ToString(CultureInfo.InvariantCulture);

    private static bool InRange(uint value, uint low, uint high) => value >= low && value <= high;

    private static bool IsContinuation(byte value) => (value & 0xC0) == 0x80;
}
