using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;

namespace Hexal.Lsp;

public enum LspPositionEncoding
{
    Utf8,
    Utf16,
    Utf32,
}

public static class LspEncodingUtil
{
    public static LspPositionEncoding Parse(string? value)
    {
        return (value ?? string.Empty).Trim().ToLowerInvariant() switch
        {
            "utf-8" or "utf8" => LspPositionEncoding.Utf8,
            "utf-32" or "utf32" => LspPositionEncoding.Utf32,
            _ => LspPositionEncoding.Utf16,
        };
    }

    public static string ToProtocolName(LspPositionEncoding encoding)
    {
        return encoding switch
        {
            LspPositionEncoding.Utf8 => "utf-8",
            LspPositionEncoding.Utf32 => "utf-32",
            _ => "utf-16",
        };
    }

    /// <summary>
    /// Converts an LSP character column to a UTF-16 index within a line.
    /// Columns beyond the line clamp to the end; a column inside a surrogate
    /// pair rounds forward to the end of the scalar.
    /// </summary>
    public static int ColumnToCharIndex(string line, int column, LspPositionEncoding encoding)
    {
        if (column <= 0)
        {
            return 0;
        }

        if (encoding == LspPositionEncoding.Utf16)
        {
            int index = Math.Min(column, line.Length);
            if (
                index > 0
                && index < line.Length
                && char.IsLowSurrogate(line[index])
                && char.IsHighSurrogate(line[index - 1])
            )
            {
                ++index;
            }
            return Math.Min(index, line.Length);
        }

        int current = 0;
        int units = 0;
        while (current < line.Length && units < column)
        {
            int scalarLength =
                char.IsHighSurrogate(line[current])
                && current + 1 < line.Length
                && char.IsLowSurrogate(line[current + 1])
                    ? 2
                    : 1;

            if (encoding == LspPositionEncoding.Utf8)
            {
                int codePoint = char.ConvertToUtf32(line, current);
                units += Encoding.UTF8.GetByteCount(char.ConvertFromUtf32(codePoint));
            }
            else
            {
                ++units;
            }

            current += scalarLength;
        }

        return current;
    }

    /// <summary>
    /// Converts a UTF-16 index within a line to an LSP character column.
    /// </summary>
    public static int CharIndexToColumn(string line, int charIndex, LspPositionEncoding encoding)
    {
        charIndex = Math.Clamp(charIndex, 0, line.Length);
        if (encoding == LspPositionEncoding.Utf16)
        {
            return charIndex;
        }

        int units = 0;
        int current = 0;
        while (current < charIndex)
        {
            int scalarLength =
                char.IsHighSurrogate(line[current])
                && current + 1 < line.Length
                && char.IsLowSurrogate(line[current + 1])
                    ? 2
                    : 1;

            // Only count whole scalars. If the requested index splits a
            // surrogate pair, round down to the start of the scalar.
            if (current + scalarLength > charIndex)
            {
                break;
            }

            if (encoding == LspPositionEncoding.Utf8)
            {
                int codePoint = char.ConvertToUtf32(line, current);
                units += Encoding.UTF8.GetByteCount(char.ConvertFromUtf32(codePoint));
            }
            else
            {
                ++units;
            }

            current += scalarLength;
        }

        return units;
    }
}

public sealed class LspSnapshot
{
    public LspSnapshot(string uri, long version, string text, LspPositionEncoding encoding)
    {
        Uri = uri;
        Version = version;
        Text = text;
        Encoding = encoding;
        LineStarts = LspTextIndex.BuildLineStarts(text);
    }

    public string Uri { get; }

    public long Version { get; }

    public string Text { get; }

    public LspPositionEncoding Encoding { get; }

    public IReadOnlyList<int> LineStarts { get; }

    public long LineCount => LineStarts.Count;
}

public static class LspTextIndex
{
    public static List<int> BuildLineStarts(string text)
    {
        var starts = new List<int> { 0 };
        for (int index = 0; index < text.Length; ++index)
        {
            if (text[index] == '\n' && index + 1 <= text.Length)
            {
                starts.Add(index + 1);
            }
        }
        return starts;
    }

    public static string LineText(string text, IReadOnlyList<int> starts, int line)
    {
        if (line < 0 || line >= starts.Count)
        {
            return string.Empty;
        }

        int start = starts[line];
        int end = line + 1 < starts.Count ? starts[line + 1] : text.Length;
        while (end > start && (text[end - 1] == '\n' || text[end - 1] == '\r'))
        {
            --end;
        }
        return text.Substring(start, end - start);
    }

    public static int PositionToCharOffset(
        string text,
        IReadOnlyList<int> starts,
        int line,
        int character,
        LspPositionEncoding encoding
    )
    {
        if (line < 0)
        {
            return 0;
        }
        if (line >= starts.Count)
        {
            return text.Length;
        }

        string lineText = LineText(text, starts, line);
        int local = LspEncodingUtil.ColumnToCharIndex(lineText, character, encoding);
        return Math.Clamp(starts[line] + local, 0, text.Length);
    }

    public static (int Line, int Character) CharOffsetToPosition(
        string text,
        IReadOnlyList<int> starts,
        int offset,
        LspPositionEncoding encoding
    )
    {
        offset = Math.Clamp(offset, 0, text.Length);

        int low = 0;
        int high = starts.Count - 1;
        while (low < high)
        {
            int middle = (low + high + 1) / 2;
            if (starts[middle] <= offset)
            {
                low = middle;
            }
            else
            {
                high = middle - 1;
            }
        }

        int line = low;
        int start = starts[line];
        string lineText = LineText(text, starts, line);
        int local = Math.Clamp(offset - start, 0, lineText.Length);
        return (line, LspEncodingUtil.CharIndexToColumn(lineText, local, encoding));
    }
}

/// <summary>
/// A mutable document tracked by the Lua language-server runtime. Documents
/// remain in the store after they are closed so workspace diagnostics and
/// snapshots can still be served, but <see cref="IsOpen"/> becomes false.
/// </summary>
public sealed class LspDocument
{
    private readonly List<LspDocumentChange> _changes = new();

    public LspDocument(
        string uri,
        string languageId,
        long version,
        string text,
        LspPositionEncoding encoding
    )
    {
        Uri = uri;
        LanguageId = languageId;
        Version = version;
        Text = text;
        Encoding = encoding;
        LineStarts = LspTextIndex.BuildLineStarts(text);
    }

    public string Uri { get; }

    public string LanguageId { get; set; }

    public long Version { get; private set; }

    public string Text { get; private set; }

    public LspPositionEncoding Encoding { get; set; }

    public bool IsOpen { get; private set; }

    public bool IsDirty { get; private set; }

    public bool IsVirtual { get; private set; }

    public List<int> LineStarts { get; private set; }

    public long LineCount => LineStarts.Count;

    public IReadOnlyList<LspDocumentChange> Changes => _changes;

    public void Open(string languageId, long version, string text)
    {
        LanguageId = languageId;
        Version = version;
        Text = text;
        LineStarts = LspTextIndex.BuildLineStarts(text);
        IsOpen = true;
        IsDirty = false;
    }

    public void CreateVirtual(string languageId, long version, string text)
    {
        IsVirtual = true;
        Open(languageId, version, text);
        IsDirty = true;
    }

    public void SetText(long version, string text)
    {
        Version = version;
        Text = text;
        LineStarts = LspTextIndex.BuildLineStarts(text);
        IsDirty = true;
    }

    public void MarkSaved(string? text, long? version)
    {
        if (text is not null)
        {
            Text = text;
            LineStarts = LspTextIndex.BuildLineStarts(text);
        }
        if (version is not null)
        {
            Version = version.Value;
        }
        IsDirty = false;
    }

    public void Close()
    {
        IsOpen = false;
        IsDirty = false;
    }

    public void Reopen() => IsOpen = true;

    public LspSnapshot Snapshot()
    {
        return new LspSnapshot(Uri, Version, Text, Encoding);
    }

    public int CharOffset(int line, int character)
    {
        return LspTextIndex.PositionToCharOffset(Text, LineStarts, line, character, Encoding);
    }

    public (int Line, int Character) Position(int charOffset)
    {
        return LspTextIndex.CharOffsetToPosition(Text, LineStarts, charOffset, Encoding);
    }

    public string GetLine(int line)
    {
        return LspTextIndex.LineText(Text, LineStarts, line);
    }

    public string GetRange(JsonObject? range)
    {
        if (range is null)
        {
            return string.Empty;
        }
        (int startLine, int startCharacter) = ReadPosition(range["start"]);
        (int endLine, int endCharacter) = ReadPosition(range["end"]);
        int start = CharOffset(startLine, startCharacter);
        int end = CharOffset(endLine, endCharacter);
        if (end < start)
        {
            (start, end) = (end, start);
        }
        return Text[start..end];
    }

    public bool TryApplyChanges(JsonArray? changes, long version, out string error)
    {
        error = string.Empty;
        if (changes is null)
        {
            Version = version;
            return true;
        }

        string working = Text;
        List<int> starts = LineStarts;
        var recorded = new List<LspDocumentChange>();

        foreach (JsonNode? node in changes)
        {
            if (node is not JsonObject change)
            {
                error = "content change must be an object";
                return false;
            }

            string newText = LspJson.GetString(change["text"]) ?? string.Empty;
            if (change["range"] is not JsonObject range)
            {
                // A change without a range replaces the whole document.
                recorded.Add(
                    new LspDocumentChange(null, working.Length, newText) { Version = version }
                );
                working = newText;
                starts = LspTextIndex.BuildLineStarts(working);
                continue;
            }

            (int startLine, int startCharacter) = ReadPosition(range["start"]);
            (int endLine, int endCharacter) = ReadPosition(range["end"]);
            int start = LspTextIndex.PositionToCharOffset(
                working,
                starts,
                startLine,
                startCharacter,
                Encoding
            );
            int end = LspTextIndex.PositionToCharOffset(
                working,
                starts,
                endLine,
                endCharacter,
                Encoding
            );
            if (end < start)
            {
                error = "content change has an inverted range";
                return false;
            }

            int rangeLength = end - start;
            if (
                change["rangeLength"] is JsonValue lengthValue
                && lengthValue.TryGetValue(out int expectedLength)
                && expectedLength != rangeLength
            )
            {
                // Some clients count UTF-16 units rather than characters. A
                // mismatch is harmless as long as the range itself is valid.
                _ = expectedLength;
            }

            recorded.Add(
                new LspDocumentChange(range.DeepClone().AsObject(), rangeLength, newText)
                {
                    Version = version,
                }
            );
            working = string.Concat(working.AsSpan(0, start), newText, working.AsSpan(end));
            starts = LspTextIndex.BuildLineStarts(working);
        }

        Text = working;
        LineStarts = starts;
        Version = version;
        IsDirty = true;
        if (_changes.Count > 512)
        {
            _changes.RemoveRange(0, _changes.Count - 512);
        }
        _changes.AddRange(recorded);
        return true;
    }

    public bool ApplyTextEdits(JsonArray edits, long? version, out string error)
    {
        error = string.Empty;
        var resolved = new List<(int Start, int End, string Text)>();
        List<int> starts = LineStarts;

        foreach (JsonNode? node in edits)
        {
            if (node is not JsonObject edit || edit["range"] is not JsonObject range)
            {
                error = "text edit is missing a range";
                return false;
            }

            int startLine = LspJson.GetInt32(range["start"]?["line"]) ?? 0;
            int startCharacter = LspJson.GetInt32(range["start"]?["character"]) ?? 0;
            int endLine = LspJson.GetInt32(range["end"]?["line"]) ?? 0;
            int endCharacter = LspJson.GetInt32(range["end"]?["character"]) ?? 0;
            if (
                startLine < 0
                || endLine < 0
                || startLine >= starts.Count
                || endLine >= starts.Count
            )
            {
                error = "text edit references a line outside the document";
                return false;
            }

            int start = LspTextIndex.PositionToCharOffset(
                Text,
                starts,
                startLine,
                startCharacter,
                Encoding
            );
            int end = LspTextIndex.PositionToCharOffset(
                Text,
                starts,
                endLine,
                endCharacter,
                Encoding
            );
            if (end < start)
            {
                error = "text edit has an inverted range";
                return false;
            }

            resolved.Add((start, end, LspJson.GetString(edit["newText"]) ?? string.Empty));
        }

        resolved.Sort((left, right) => right.Start.CompareTo(left.Start));
        int lastStart = int.MaxValue;
        foreach ((int start, int end, string _) in resolved)
        {
            if (end > lastStart)
            {
                error = "text edits overlap";
                return false;
            }
            lastStart = start;
        }

        string working = Text;
        foreach ((int start, int end, string text) in resolved)
        {
            working = string.Concat(working.AsSpan(0, start), text, working.AsSpan(end));
        }

        Text = working;
        LineStarts = LspTextIndex.BuildLineStarts(working);
        if (version is not null)
        {
            Version = version.Value;
        }
        IsDirty = true;
        return true;
    }

    public JsonArray ChangesSince(long version)
    {
        var array = new JsonArray();
        foreach (LspDocumentChange change in _changes)
        {
            if (change.Version < version)
            {
                continue;
            }
            var item = new JsonObject { ["text"] = change.Text };
            if (change.Range is not null)
            {
                item["range"] = change.Range.DeepClone();
                item["rangeLength"] = change.RangeLength;
            }
            LspJson.AddNode(array, item);
        }
        return array;
    }

    private static (int Line, int Character) ReadPosition(JsonNode? node)
    {
        if (node is not JsonObject position)
        {
            return (0, 0);
        }
        int line = LspJson.GetInt32(position["line"]) ?? 0;
        int character = LspJson.GetInt32(position["character"]) ?? 0;
        return (Math.Max(0, line), Math.Max(0, character));
    }
}

public sealed record LspDocumentChange(JsonObject? Range, int RangeLength, string Text)
{
    public long Version { get; init; }
}

public sealed class LspDocumentStore
{
    private readonly Dictionary<string, LspDocument> _documents = new(StringComparer.Ordinal);

    public LspPositionEncoding Encoding { get; set; } = LspPositionEncoding.Utf16;

    public LspDocument? Get(string uri)
    {
        return uri is not null && _documents.TryGetValue(uri, out LspDocument? document)
            ? document
            : null;
    }

    public LspDocument? GetOpen(string uri)
    {
        LspDocument? document = Get(uri);
        return document is { IsOpen: true } ? document : null;
    }

    public bool IsOpen(string uri) => GetOpen(uri) is not null;

    public IReadOnlyCollection<LspDocument> All => _documents.Values;

    public IReadOnlyList<LspDocument> OpenDocuments =>
        _documents.Values.Where(document => document.IsOpen).ToArray();

    public LspDocument Open(string uri, string languageId, long version, string text)
    {
        if (!_documents.TryGetValue(uri, out LspDocument? document))
        {
            document = new LspDocument(uri, languageId, version, text, Encoding);
            _documents[uri] = document;
        }
        document.Encoding = Encoding;
        document.Open(languageId, version, text);
        return document;
    }

    public LspDocument CreateVirtual(string uri, string languageId, long version, string text)
    {
        if (!_documents.TryGetValue(uri, out LspDocument? document))
        {
            document = new LspDocument(uri, languageId, version, text, Encoding);
            _documents[uri] = document;
        }
        document.Encoding = Encoding;
        document.CreateVirtual(languageId, version, text);
        return document;
    }

    public bool Change(string uri, JsonArray? changes, long version, out string error)
    {
        error = string.Empty;
        LspDocument? document = Get(uri);
        if (document is null)
        {
            error = "document is not open";
            return false;
        }
        return document.TryApplyChanges(changes, version, out error);
    }

    public bool Save(string uri, string? text, long? version)
    {
        LspDocument? document = Get(uri);
        if (document is null)
        {
            return false;
        }
        document.MarkSaved(text, version);
        return true;
    }

    public bool Close(string uri)
    {
        LspDocument? document = Get(uri);
        if (document is null)
        {
            return false;
        }
        document.Close();
        return true;
    }

    public bool Remove(string uri) => _documents.Remove(uri);

    public LspSnapshot? Snapshot(string uri) => Get(uri)?.Snapshot();

    public void Clear()
    {
        _documents.Clear();
    }
}
