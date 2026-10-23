using System.Text;
using System.Text.RegularExpressions;
using Hexal.Core;

namespace Hexal.Languages;

/// <summary>
/// Byte-oriented lexer driven entirely by a <see cref="LanguageDefinition"/>.
/// Only visible lines are tokenized. Line-start lexer states are cached so
/// block comments remain correct while scrolling, without rescanning the
/// document on every frame or after every edit.
/// </summary>
public sealed class SyntaxHighlighter
{
    private const int MaxBackwardStateSearch = 512;
    private const int MaxCacheEntries = 50_000;

    private sealed class CachedLine
    {
        public ulong Signature;
        public bool StartInBlockComment;
        public bool EndInBlockComment;
    }

    private readonly Dictionary<long, CachedLine> _cache = new();
    private byte[] _lineBuffer = Array.Empty<byte>();

    private Document? _document;
    private LanguageDefinition? _language;

    public LanguageDefinition? Language => _language;

    public void SetDocument(Document? document, LanguageDefinition? language)
    {
        _document = document;
        _language = language;
        _cache.Clear();
    }

    public void Invalidate()
    {
        _cache.Clear();
    }

    public void GetLineTokens(long line, ReadOnlySpan<byte> text,
        List<SyntaxToken> tokens)
    {
        tokens.Clear();
        if (_language is null || _document is null)
        {
            return;
        }

        bool startInBlock = ResolveLineStart(line);
        TokenizeLine(_language, text, startInBlock, tokens, out bool endInBlock);

        if (_cache.Count >= MaxCacheEntries)
        {
            _cache.Clear();
        }

        _cache[line] = new CachedLine
        {
            Signature = Signature(text),
            StartInBlockComment = startInBlock,
            EndInBlockComment = endInBlock
        };
    }

    private bool ResolveLineStart(long line)
    {
        if (line <= 0 || _document is null)
        {
            return false;
        }

        // Do not trust the target line's own cached start state: an edit on an
        // earlier line can change whether this line begins inside a block
        // comment. Validate backwards from a nearby line whose text still
        // matches its cached signature instead.
        _cache.Remove(line);

        long foundLine = -1;
        bool state = false;
        long search = line - 1;
        int steps = 0;

        while (search >= 0 && steps < MaxBackwardStateSearch)
        {
            if (_cache.TryGetValue(search, out CachedLine? previous))
            {
                int length = _document.CopyLineText(search, ref _lineBuffer, out _);
                if (previous.Signature == Signature(_lineBuffer.AsSpan(0, length)))
                {
                    state = previous.EndInBlockComment;
                    foundLine = search;
                    break;
                }
                _cache.Remove(search);
            }

            --search;
            ++steps;
        }

        long fromLine;
        if (foundLine >= 0)
        {
            fromLine = foundLine + 1;
        }
        else if (line <= 1024)
        {
            state = false;
            fromLine = 0;
        }
        else
        {
            // A viewport far from any cached state starts in the default state.
            // This keeps jumps into multi-million-line files O(visible lines).
            state = false;
            fromLine = line;
        }

        for (long current = fromLine; current < line; ++current)
        {
            int length = _document.CopyLineText(current, ref _lineBuffer, out _);
            ReadOnlySpan<byte> currentText = _lineBuffer.AsSpan(0, length);
            TokenizeLine(_language!, currentText, state, null, out bool endInBlock);
            _cache[current] = new CachedLine
            {
                Signature = Signature(currentText),
                StartInBlockComment = state,
                EndInBlockComment = endInBlock
            };
            state = endInBlock;
        }

        return state;
    }

    /// <summary>
    /// Tokenizes one line of arbitrary text using a grammar. This is used for
    /// content that is not backed by a <see cref="Document"/>, such as code
    /// blocks in hover popups. Callers must pass the previous line's end state
    /// so multi-line constructs remain correct.
    /// </summary>
    public static void GetLineTokens(LanguageDefinition language,
        ReadOnlySpan<byte> text, bool startInBlock, List<SyntaxToken> tokens,
        out bool endInBlock)
    {
        TokenizeLine(language, text, startInBlock, tokens, out endInBlock);
    }

    private static void TokenizeLine(LanguageDefinition language,
        ReadOnlySpan<byte> text, bool startInBlock,
        List<SyntaxToken>? tokens, out bool endInBlock)
    {
        bool inBlock = startInBlock;
        int offset = 0;

        if (!inBlock && language.LineHighlights.Count > 0 &&
            TryTokenizeLineHighlights(language, text, tokens))
        {
            endInBlock = false;
            return;
        }

        while (offset < text.Length)
        {
            if (inBlock)
            {
                int end = IndexOf(text, offset, language.BlockCommentEndBytes!);
                if (end < 0)
                {
                    AddToken(tokens, offset, text.Length - offset,
                        SyntaxTokenKind.Comment);
                    offset = text.Length;
                    break;
                }

                int tokenEnd = end + language.BlockCommentEndBytes!.Length;
                AddToken(tokens, offset, tokenEnd - offset, SyntaxTokenKind.Comment);
                offset = tokenEnd;
                inBlock = false;
                continue;
            }

            // Check block comments first: Lua's --[[ ... ]] starts with its
            // line-comment marker, and must still carry state across lines.
            if (language.HasBlockComments &&
                StartsWith(text, offset, language.BlockCommentStartBytes!))
            {
                int contentStart = offset + language.BlockCommentStartBytes!.Length;
                int end = IndexOf(text, contentStart, language.BlockCommentEndBytes!);
                if (end < 0)
                {
                    AddToken(tokens, offset, text.Length - offset,
                        SyntaxTokenKind.Comment);
                    offset = text.Length;
                    inBlock = true;
                    break;
                }

                int tokenEnd = end + language.BlockCommentEndBytes!.Length;
                AddToken(tokens, offset, tokenEnd - offset, SyntaxTokenKind.Comment);
                offset = tokenEnd;
                continue;
            }

            if (language.LineCommentBytes is { Length: > 0 } lineComment &&
                StartsWith(text, offset, lineComment))
            {
                AddToken(tokens, offset, text.Length - offset,
                    SyntaxTokenKind.Comment);
                offset = text.Length;
                break;
            }

            int quoteLength = MatchQuote(text, offset, language.QuoteCharacterBytes);
            if (quoteLength > 0)
            {
                int start = offset;
                offset += quoteLength;

                while (offset < text.Length)
                {
                    if (language.EscapeCharacterBytes is { Length: > 0 } escape &&
                        StartsWith(text, offset, escape))
                    {
                        offset += escape.Length;
                        SkipRune(text, ref offset);
                        continue;
                    }

                    int closingLength = MatchQuote(text, offset,
                        language.QuoteCharacterBytes);
                    if (closingLength > 0)
                    {
                        offset += closingLength;
                        break;
                    }

                    SkipRune(text, ref offset);
                }

                AddToken(tokens, start, offset - start, SyntaxTokenKind.StringLiteral);
                continue;
            }

            if (language.HighlightNumbers && IsAsciiDigit(text[offset]))
            {
                int start = offset;
                ConsumeNumber(text, ref offset);
                AddToken(tokens, start, offset - start, SyntaxTokenKind.Number);
                continue;
            }

            if (IsIdentifierStart(text[offset]))
            {
                int start = offset;
                ++offset;
                while (offset < text.Length && IsIdentifierPart(text[offset]))
                {
                    ++offset;
                }

                AddToken(tokens, start, offset - start,
                    language.ClassifyIdentifier(text[start..offset]));
                continue;
            }

            SkipRune(text, ref offset);
        }

        endInBlock = inBlock;
    }

    private static bool TryTokenizeLineHighlights(
        LanguageDefinition language, ReadOnlySpan<byte> text,
        List<SyntaxToken>? tokens)
    {
        if (text.IsEmpty)
        {
            return false;
        }

        string line = Encoding.UTF8.GetString(text);
        foreach (LineHighlightPattern rule in language.LineHighlights)
        {
            Match match = rule.Pattern.Match(line);
            if (!match.Success || match.Length == 0)
            {
                continue;
            }

            int start = Encoding.UTF8.GetByteCount(
                line.AsSpan(0, match.Index));
            int length = Encoding.UTF8.GetByteCount(
                line.AsSpan(match.Index, match.Length));
            AddToken(tokens, start, length, rule.Kind);
            return true;
        }

        return false;
    }

    private static void AddToken(List<SyntaxToken>? tokens, int start, int length,
        SyntaxTokenKind kind)
    {
        if (tokens is null || length <= 0 || kind == SyntaxTokenKind.Default)
        {
            return;
        }
        tokens.Add(new SyntaxToken(start, length, kind));
    }

    private static int MatchQuote(ReadOnlySpan<byte> text, int offset,
        IReadOnlyList<byte[]> quotes)
    {
        foreach (byte[] quote in quotes)
        {
            if (StartsWith(text, offset, quote))
            {
                return quote.Length;
            }
        }
        return 0;
    }

    private static bool StartsWith(ReadOnlySpan<byte> text, int offset,
        ReadOnlySpan<byte> marker)
    {
        return marker.Length > 0 &&
               offset <= text.Length - marker.Length &&
               text.Slice(offset, marker.Length).SequenceEqual(marker);
    }

    private static int IndexOf(ReadOnlySpan<byte> text, int offset,
        ReadOnlySpan<byte> marker)
    {
        if (marker.IsEmpty || offset >= text.Length)
        {
            return -1;
        }
        int index = text[offset..].IndexOf(marker);
        return index < 0 ? -1 : offset + index;
    }

    private static bool IsAsciiDigit(byte value) => value is >= (byte)'0' and <= (byte)'9';

    private static bool IsIdentifierStart(byte value)
    {
        return value is >= (byte)'a' and <= (byte)'z' or
               >= (byte)'A' and <= (byte)'Z' or
               (byte)'_' or >= 0x80;
    }

    private static bool IsIdentifierPart(byte value)
    {
        return IsIdentifierStart(value) || IsAsciiDigit(value);
    }

    private static void ConsumeNumber(ReadOnlySpan<byte> text, ref int offset)
    {
        bool hexadecimal = false;
        bool binary = false;

        if (offset + 1 < text.Length && text[offset] == (byte)'0')
        {
            byte prefix = text[offset + 1];
            hexadecimal = prefix is (byte)'x' or (byte)'X';
            binary = prefix is (byte)'b' or (byte)'B';
        }

        if (hexadecimal || binary)
        {
            offset += 2;
            while (offset < text.Length)
            {
                byte value = text[offset];
                if (IsAsciiDigit(value) ||
                    (hexadecimal && value is >= (byte)'a' and <= (byte)'f' or
                                      >= (byte)'A' and <= (byte)'F') ||
                    value == (byte)'_')
                {
                    ++offset;
                    continue;
                }
                break;
            }
            return;
        }

        while (offset < text.Length)
        {
            byte value = text[offset];
            if (IsAsciiDigit(value) || value is (byte)'.' or (byte)'_')
            {
                ++offset;
                continue;
            }
            if (value is (byte)'e' or (byte)'E')
            {
                ++offset;
                if (offset < text.Length && text[offset] is (byte)'+' or (byte)'-')
                {
                    ++offset;
                }
                continue;
            }
            break;
        }
    }

    private static void SkipRune(ReadOnlySpan<byte> text, ref int offset)
    {
        if (offset >= text.Length)
        {
            return;
        }

        byte lead = text[offset];
        int length = 1;
        if ((lead & 0xE0) == 0xC0)
        {
            length = 2;
        }
        else if ((lead & 0xF0) == 0xE0)
        {
            length = 3;
        }
        else if ((lead & 0xF8) == 0xF0)
        {
            length = 4;
        }

        offset += Math.Min(length, text.Length - offset);
    }

    private static ulong Signature(ReadOnlySpan<byte> text)
    {
        const ulong offsetBasis = 14695981039346656037UL;
        const ulong prime = 1099511628211UL;

        ulong hash = offsetBasis;
        foreach (byte value in text)
        {
            hash ^= value;
            hash *= prime;
        }
        return hash;
    }
}
