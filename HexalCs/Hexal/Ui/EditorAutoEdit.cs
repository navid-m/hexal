using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private static readonly string[] DefaultIndentTriggers = { "{", "[", "(" };

    private static readonly HashSet<string> VoidHtmlElements =
        new(StringComparer.OrdinalIgnoreCase)
        {
            "area", "base", "br", "col", "embed", "hr", "img", "input",
            "link", "meta", "param", "source", "track", "wbr"
        };

    private bool AutoIndentEnabled => _language?.AutoIndent == true;

    private bool AutoCloseEnabled => _language?.AutoClose == true;

    private bool IsTraditionalMarkup => _language?.TraditionalMarkupLanguage == true;

    private bool TrySkipAutoClose(Rune rune)
    {
        if (!AutoCloseEnabled)
        {
            return false;
        }

        char character = (char)rune.Value;
        if (character is not (')' or ']' or '}'))
        {
            return false;
        }

        if (_cursor >= _document.Size ||
            _document.ByteAt(_cursor) != (byte)character ||
            IsInsertPositionInStringOrComment())
        {
            return false;
        }

        ++_cursor;
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
        return true;
    }

    private bool TryInsertAutoClosePair(Rune rune)
    {
        if (!AutoCloseEnabled)
        {
            return false;
        }

        char open = (char)rune.Value;
        char close = open switch
        {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => '\0'
        };
        if (close == '\0' || IsInsertPositionInStringOrComment())
        {
            return false;
        }

        Span<byte> pair = stackalloc byte[2];
        pair[0] = (byte)open;
        pair[1] = (byte)close;
        _document.Insert(_cursor, pair);
        ++_cursor;
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
        return true;
    }

    private void TryInsertAutoCloseTag()
    {
        if (!AutoCloseEnabled || !IsTraditionalMarkup ||
            IsInsertPositionInStringOrComment())
        {
            return;
        }

        long line = CursorLine();
        long start = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(_cursor - start, 0, length);
        if (!TryParseOpenTagAtEnd(_lineBuffer.AsSpan(0, local), out string name))
        {
            return;
        }

        byte[] closing = Encoding.UTF8.GetBytes($"</{name}>");
        _document.Insert(_cursor, closing);
        SnapCursor();
    }

    private bool IsInsertPositionInStringOrComment()
    {
        if (_language is null)
        {
            return false;
        }

        long line = CursorLine();
        long start = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(_cursor - start, 0, length);
        _syntax.GetLineTokens(line, _lineBuffer.AsSpan(0, length), _lineTokens);

        foreach (SyntaxToken token in _lineTokens)
        {
            if (token.Kind == SyntaxTokenKind.Comment)
            {
                if (local > token.Start && local <= token.End)
                {
                    return true;
                }
                continue;
            }

            if (token.Kind != SyntaxTokenKind.StringLiteral)
            {
                continue;
            }

            if (local > token.Start && local < token.End)
            {
                return true;
            }

            // The cursor immediately after a closed string is outside the
            // string. Only unterminated strings extend to the end of the line.
            if (local == token.End &&
                !IsClosedStringToken(token, _lineBuffer.AsSpan(0, length)))
            {
                return true;
            }
        }

        return false;
    }

    private bool IsClosedStringToken(SyntaxToken token,
        ReadOnlySpan<byte> lineText)
    {
        if (token.Length == 0 || token.Start < 0 || token.End > lineText.Length)
        {
            return false;
        }

        ReadOnlySpan<byte> text = lineText[token.Start..token.End];
        int quoteLength = 0;
        foreach (byte[] quote in _language?.QuoteCharacterBytes ??
                 Array.Empty<byte[]>())
        {
            if (text.EndsWith(quote))
            {
                quoteLength = quote.Length;
                break;
            }
        }

        if (quoteLength == 0 || text.Length <= quoteLength)
        {
            return false;
        }

        ReadOnlySpan<byte> escape = _language?.EscapeCharacterBytes ??
                                    ReadOnlySpan<byte>.Empty;
        int escapes = 0;
        int index = text.Length - quoteLength;
        while (escape.Length > 0 && index >= escape.Length &&
               text.Slice(index - escape.Length, escape.Length)
                   .SequenceEqual(escape))
        {
            ++escapes;
            index -= escape.Length;
        }

        return escapes % 2 == 0;
    }

    private static bool TryParseOpenTagAtEnd(ReadOnlySpan<byte> text,
        out string name)
    {
        name = string.Empty;
        if (text.Length < 3 || text[^1] != (byte)'>')
        {
            return false;
        }

        int start = text.Length - 2;
        while (start >= 0 && text[start] != (byte)'<')
        {
            --start;
        }
        if (start < 0)
        {
            return false;
        }

        ReadOnlySpan<byte> content = text[(start + 1)..^1];
        content = Trim(content);
        if (content.IsEmpty ||
            content[0] is (byte)'/' or (byte)'!' or (byte)'?' ||
            content[^1] == (byte)'/')
        {
            return false;
        }

        int nameLength = 0;
        while (nameLength < content.Length &&
               !IsTagNameTerminator(content[nameLength]))
        {
            ++nameLength;
        }
        if (nameLength == 0)
        {
            return false;
        }

        for (int index = 0; index < nameLength; ++index)
        {
            byte value = content[index];
            bool valid = IsAsciiLetter(value) || IsAsciiDigit(value) ||
                         value is (byte)'_' or (byte)'-' or (byte)':' or (byte)'.';
            if (!valid || (index == 0 && !IsAsciiLetter(value) &&
                           value is not ((byte)'_' or (byte)':')))
            {
                return false;
            }
        }

        if (!HasBalancedQuotesAndAngles(content))
        {
            return false;
        }

        string candidate = Encoding.UTF8.GetString(content[..nameLength]);
        if (VoidHtmlElements.Contains(candidate))
        {
            return false;
        }

        name = candidate;
        return true;
    }

    private static bool HasBalancedQuotesAndAngles(ReadOnlySpan<byte> content)
    {
        bool inDouble = false;
        bool inSingle = false;
        foreach (byte value in content)
        {
            if (inDouble)
            {
                if (value == (byte)'"')
                {
                    inDouble = false;
                }
                continue;
            }

            if (inSingle)
            {
                if (value == (byte)'\'')
                {
                    inSingle = false;
                }
                continue;
            }

            if (value == (byte)'"')
            {
                inDouble = true;
            }
            else if (value == (byte)'\'')
            {
                inSingle = true;
            }
            else if (value is (byte)'<' or (byte)'>')
            {
                return false;
            }
        }

        return !inDouble && !inSingle;
    }

    private static bool IsTagNameTerminator(byte value)
        => value is (byte)' ' or (byte)'\t' or (byte)'\r' or (byte)'\n' or
           (byte)'/' or (byte)'>';

    private static bool IsAsciiLetter(byte value)
        => value is >= (byte)'A' and <= (byte)'Z' or >= (byte)'a' and <= (byte)'z';

    private static bool IsAsciiDigit(byte value)
        => value is >= (byte)'0' and <= (byte)'9';

    private void InsertNewlineWithAutoIndent()
    {
        long line = CursorLine();
        long lineStart = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(_cursor - lineStart, 0, length);
        int indent = 0;
        while (indent < length &&
               (_lineBuffer[indent] == (byte)' ' || _lineBuffer[indent] == (byte)'\t'))
        {
            ++indent;
        }

        indent = Math.Min(indent, local);

        ReadOnlySpan<byte> before = _lineBuffer.AsSpan(0, local);
        ReadOnlySpan<byte> after = _lineBuffer.AsSpan(local, length - local);
        string trigger = string.Empty;
        bool increase = AutoIndentEnabled &&
                        TryGetIndentTrigger(before, out trigger);
        bool addClosingLine = increase &&
                              StartsWithClosingToken(after, trigger);

        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        _document.Insert(_cursor, eol);
        _cursor += eol.Length;

        if (indent > 0)
        {
            _document.Insert(_cursor, _lineBuffer.AsSpan(0, indent));
            _cursor += indent;
        }

        if (increase)
        {
            byte[] unit = _document.IndentUnit();
            _document.Insert(_cursor, unit);
            _cursor += unit.Length;
        }

        if (addClosingLine)
        {
            long cursorOnIndentedLine = _cursor;
            _document.Insert(_cursor, eol);
            _cursor += eol.Length;
            if (indent > 0)
            {
                _document.Insert(_cursor, _lineBuffer.AsSpan(0, indent));
                _cursor += indent;
            }

            _cursor = cursorOnIndentedLine;
        }

        SnapCursor();
        _preferredColumnValid = false;
        ClearMessage();
    }

    private bool TryGetIndentTrigger(ReadOnlySpan<byte> before, out string trigger)
    {
        trigger = string.Empty;
        if (_language is null)
        {
            return false;
        }

        ReadOnlySpan<byte> trimmed = TrimEnd(before);
        if (IsTraditionalMarkup)
        {
            if (TryParseOpenTagAtEnd(trimmed, out string name))
            {
                trigger = $"<{name}>";
                return true;
            }

            return false;
        }

        string text = Encoding.UTF8.GetString(trimmed);
        string? best = null;
        foreach (string candidate in DefaultIndentTriggers)
        {
            if (text.EndsWith(candidate, StringComparison.Ordinal) &&
                (best is null || candidate.Length > best.Length))
            {
                best = candidate;
            }
        }

        foreach (string candidate in _language.IndentAfter)
        {
            if (text.EndsWith(candidate, StringComparison.Ordinal) &&
                (best is null || candidate.Length > best.Length))
            {
                best = candidate;
            }
        }

        if (best is null)
        {
            return false;
        }

        trigger = best;
        return true;
    }

    private bool TryAutoUnindentKeyword()
    {
        if (!AutoIndentEnabled || _language is null ||
            _language.UnindentBefore.Count == 0)
        {
            return false;
        }

        long line = CursorLine();
        long start = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(_cursor - start, 0, length);
        int content = 0;
        while (content < local &&
               (_lineBuffer[content] == (byte)' ' ||
                _lineBuffer[content] == (byte)'\t'))
        {
            ++content;
        }

        if (content >= local)
        {
            return false;
        }

        string text = Encoding.UTF8.GetString(_lineBuffer, content,
            local - content);
        if (!_language.IsUnindentKeyword(text))
        {
            return false;
        }

        int remove = _document.UnindentLength(
            _lineBuffer.AsSpan(0, content));
        if (remove == 0)
        {
            return false;
        }

        _document.Erase(start, remove);
        _cursor -= remove;
        _preferredColumnValid = false;
        SnapCursor();
        return true;
    }

    private bool StartsWithClosingToken(ReadOnlySpan<byte> after, string trigger)
    {
        string text = Encoding.UTF8.GetString(TrimStart(after));
        if (text.Length == 0 || trigger.Length == 0)
        {
            return false;
        }

        if (trigger[0] == '<' && trigger[^1] == '>')
        {
            string name = trigger[1..^1];
            return text.StartsWith($"</{name}>", StringComparison.OrdinalIgnoreCase);
        }

        string? closing = trigger switch
        {
            "{" => "}",
            "[" => "]",
            "(" => ")",
            _ => null
        };
        if (closing is not null)
        {
            return text.StartsWith(closing, StringComparison.Ordinal);
        }

        foreach (string candidate in _language?.UnindentBefore ?? Array.Empty<string>())
        {
            if (text.StartsWith(candidate, StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    private static ReadOnlySpan<byte> Trim(ReadOnlySpan<byte> value)
        => TrimEnd(TrimStart(value));

    private static ReadOnlySpan<byte> TrimStart(ReadOnlySpan<byte> value)
    {
        int start = 0;
        while (start < value.Length && IsWhitespace(value[start]))
        {
            ++start;
        }
        return value[start..];
    }

    private static ReadOnlySpan<byte> TrimEnd(ReadOnlySpan<byte> value)
    {
        int end = value.Length;
        while (end > 0 && IsWhitespace(value[end - 1]))
        {
            --end;
        }
        return value[..end];
    }

    private static bool IsWhitespace(byte value)
        => value is (byte)' ' or (byte)'\t' or (byte)'\r' or (byte)'\n';

    private void AutoDetectIndentation(bool announce = true)
    {
        (bool useTabs, int width) = DetectDocumentIndentation(_document);
        if (useTabs)
        {
            _document.SetIndentTabs(width);
            if (announce)
            {
                SetMessage(
                    $"Detected tabs; document indentation set to tabs (width {width}).");
            }
        }
        else
        {
            _document.SetIndentSpaces(width);
            if (announce)
            {
                SetMessage(
                    $"Detected spaces; document indentation set to {width} spaces.");
            }
        }
    }

    private void AutoDetectIndentationOnOpen()
    {
        if (!_config.AutoDetectIndentationOnStartup || _document.Size == 0)
        {
            return;
        }

        AutoDetectIndentation(announce: false);
    }

    private static (bool UseTabs, int Width) DetectDocumentIndentation(
        Document document)
    {
        const int maxLines = 20_000;
        int lineCount = (int)Math.Min(document.LineCount, maxLines);
        byte[] buffer = Array.Empty<byte>();
        int tabLines = 0;
        int spaceLines = 0;
        var spaceCounts = new Dictionary<int, int>();

        for (int line = 0; line < lineCount; ++line)
        {
            int length = document.CopyLineText(line, ref buffer, out _);
            int tabs = 0;
            int spaces = 0;
            int offset = 0;
            while (offset < length)
            {
                if (buffer[offset] == (byte)'\t')
                {
                    ++tabs;
                    ++offset;
                }
                else if (buffer[offset] == (byte)' ')
                {
                    ++spaces;
                    ++offset;
                }
                else
                {
                    break;
                }
            }

            if (offset >= length)
            {
                continue;
            }

            if (tabs > 0)
            {
                ++tabLines;
            }
            else if (spaces > 0)
            {
                ++spaceLines;
                spaceCounts[spaces] = spaceCounts.TryGetValue(spaces,
                    out int count) ? count + 1 : 1;
            }
        }

        if (tabLines > 0 && tabLines >= spaceLines)
        {
            int width = document.TabWidth > 0
                ? document.TabWidth
                : Document.DefaultTabWidth;
            return (true, width);
        }

        if (spaceLines > 0)
        {
            int width = DetectSpaceWidth(spaceCounts, document.IndentWidth);
            return (false, width);
        }

        return (document.UseTabs,
            document.UseTabs ? document.TabWidth : document.IndentWidth);
    }

    private static int DetectSpaceWidth(Dictionary<int, int> counts,
        int fallback)
    {
        if (counts.Count == 0)
        {
            return fallback;
        }

        int gcd = 0;
        foreach (int count in counts.Keys)
        {
            gcd = GreatestCommonDivisor(gcd, count);
        }

        if (gcd is >= 2 and <= 16)
        {
            return gcd;
        }

        int best = 0;
        int bestFrequency = 0;
        foreach ((int width, int frequency) in counts)
        {
            if ((width is >= 1 and <= 16) &&
                (frequency > bestFrequency ||
                 (frequency == bestFrequency && width < best)))
            {
                best = width;
                bestFrequency = frequency;
            }
        }

        return best > 0 ? best : fallback;
    }

    private static int GreatestCommonDivisor(int left, int right)
    {
        while (right != 0)
        {
            (left, right) = (right, left % right);
        }

        return left;
    }
}
