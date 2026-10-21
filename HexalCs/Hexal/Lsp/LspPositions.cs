using System.Text;
using Hexal.Util;

namespace Hexal.Lsp;

/// <summary>
/// Converts between the editor's UTF-8 byte offsets and the LSP position
/// model (zero-based line plus UTF-16 code-unit column). Keeping this in one
/// place makes the emoji/surrogate handling on both sides consistent.
/// </summary>
public static class LspPositions
{
    /// <summary>
    /// Converts a UTF-16 column within a UTF-8 line into a byte offset. Values
    /// past the end of the line clamp to the line length; a column that splits a
    /// surrogate pair rounds up to the end of the scalar.
    /// </summary>
    public static int Utf16ColumnToByteOffset(ReadOnlySpan<byte> utf8Line, int utf16Column)
    {
        if (utf16Column <= 0)
        {
            return 0;
        }

        int offset = 0;
        int units = 0;

        while (offset < utf8Line.Length && units < utf16Column)
        {
            UnicodeUtil.Decode(utf8Line, offset, out Rune rune, out int length);
            int runeUnits = rune.Value > 0xFFFF ? 2 : 1;
            units += runeUnits;
            offset += length;
        }

        return Math.Clamp(offset, 0, utf8Line.Length);
    }

    /// <summary>
    /// Converts a byte offset within a UTF-8 line into a UTF-16 code-unit
    /// column.
    /// </summary>
    public static int ByteOffsetToUtf16Column(ReadOnlySpan<byte> utf8Line, int byteOffset)
    {
        int offset = 0;
        int units = 0;
        int target = Math.Clamp(byteOffset, 0, utf8Line.Length);

        while (offset < target)
        {
            UnicodeUtil.Decode(utf8Line, offset, out Rune rune, out int length);
            offset += length;
            units += rune.Value > 0xFFFF ? 2 : 1;
        }

        return units;
    }
}
