using System.Text;
using System.Text.Json;

namespace Hexal.Languages;

public enum SyntaxTokenKind
{
    Default,
    Keyword,
    ControlStatement,
    Type,
    Constant,
    StringLiteral,
    Comment,
    Number
}

public readonly record struct SyntaxToken(int Start, int Length, SyntaxTokenKind Kind)
{
    public int End => Start + Length;
}

/// <summary>
/// A validated .hexdef language definition. The canonical schema is the JSON
/// object documented in README.md. Definitions are data only; adding a grammar
/// never requires recompiling the editor.
/// </summary>
public sealed class LanguageDefinition
{
    private readonly HashSet<string> _keywords;
    private readonly HashSet<string> _controlStatements;
    private readonly HashSet<string> _types;
    private readonly HashSet<string> _constants;

    private LanguageDefinition(string id, string name, List<string> extensions,
        HashSet<string> keywords, HashSet<string> controlStatements,
        HashSet<string> types, HashSet<string> constants, string? lineComment,
        string? blockCommentStart, string? blockCommentEnd,
        List<string> quoteCharacters, string? escapeCharacter, bool highlightNumbers)
    {
        Id = id;
        Name = name;
        Extensions = extensions;
        _keywords = keywords;
        _controlStatements = controlStatements;
        _types = types;
        _constants = constants;

        LineComment = lineComment;
        BlockCommentStart = blockCommentStart;
        BlockCommentEnd = blockCommentEnd;
        QuoteCharacters = quoteCharacters;
        EscapeCharacter = escapeCharacter;
        HighlightNumbers = highlightNumbers;

        LineCommentBytes = Encode(lineComment);
        BlockCommentStartBytes = Encode(blockCommentStart);
        BlockCommentEndBytes = Encode(blockCommentEnd);
        QuoteCharacterBytes = quoteCharacters
            .Select(quote => Encoding.UTF8.GetBytes(quote))
            .ToArray();
        EscapeCharacterBytes = Encode(escapeCharacter);
    }

    public string Id { get; }

    public string Name { get; }

    public IReadOnlyList<string> Extensions { get; }

    public string? LineComment { get; }

    public string? BlockCommentStart { get; }

    public string? BlockCommentEnd { get; }

    public IReadOnlyList<string> QuoteCharacters { get; }

    public string? EscapeCharacter { get; }

    public bool HighlightNumbers { get; }

    internal byte[]? LineCommentBytes { get; }

    internal byte[]? BlockCommentStartBytes { get; }

    internal byte[]? BlockCommentEndBytes { get; }

    internal IReadOnlyList<byte[]> QuoteCharacterBytes { get; }

    internal byte[]? EscapeCharacterBytes { get; }

    public bool HasBlockComments
        => BlockCommentStartBytes is { Length: > 0 } &&
           BlockCommentEndBytes is { Length: > 0 };

    public SyntaxTokenKind ClassifyIdentifier(ReadOnlySpan<byte> identifier)
    {
        if (identifier.IsEmpty)
        {
            return SyntaxTokenKind.Default;
        }

        string text = Encoding.UTF8.GetString(identifier);
        if (_keywords.Contains(text))
        {
            return SyntaxTokenKind.Keyword;
        }
        if (_controlStatements.Contains(text))
        {
            return SyntaxTokenKind.ControlStatement;
        }
        if (_types.Contains(text))
        {
            return SyntaxTokenKind.Type;
        }
        if (_constants.Contains(text))
        {
            return SyntaxTokenKind.Constant;
        }
        return SyntaxTokenKind.Default;
    }

    public static bool TryParse(string json, string sourcePath,
        out LanguageDefinition? definition, out string error)
    {
        definition = null;
        error = string.Empty;

        try
        {
            using JsonDocument document = JsonDocument.Parse(json, new JsonDocumentOptions
            {
                AllowTrailingCommas = true,
                CommentHandling = JsonCommentHandling.Skip
            });

            JsonElement root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                error = "root must be a JSON object";
                return false;
            }

            string fallbackId = Path.GetFileNameWithoutExtension(sourcePath);
            string id = GetString(root, "id")?.Trim() ?? string.Empty;
            if (id.Length == 0)
            {
                id = fallbackId;
            }
            if (id.Length == 0)
            {
                error = "missing language id";
                return false;
            }

            string name = GetString(root, "name")?.Trim() ?? string.Empty;
            if (name.Length == 0)
            {
                name = id;
            }

            var extensions = new List<string>();
            foreach (string extension in ReadStringArray(root, "extensions"))
            {
                string value = extension.Trim();
                if (value.Length == 0)
                {
                    continue;
                }
                if (!value.StartsWith('.'))
                {
                    value = "." + value;
                }
                if (!extensions.Contains(value, StringComparer.OrdinalIgnoreCase))
                {
                    extensions.Add(value.ToLowerInvariant());
                }
            }

            var keywords = ReadSet(root, "keywords");
            var controlStatements = ReadSet(root, "controlStatements");
            var types = ReadSet(root, "types");
            var constants = ReadSet(root, "constants");

            string? lineComment = null;
            string? blockStart = null;
            string? blockEnd = null;

            if (root.TryGetProperty("comments", out JsonElement comments) &&
                comments.ValueKind == JsonValueKind.Object)
            {
                lineComment = GetString(comments, "line");
                blockStart = GetString(comments, "blockStart") ??
                             GetString(comments, "start");
                blockEnd = GetString(comments, "blockEnd") ??
                           GetString(comments, "end");
            }

            // Accept the older illustrative aliases when the canonical
            // "comments" object is absent.
            lineComment ??= GetString(root, "lineComment");
            if (root.TryGetProperty("blockComment", out JsonElement blockComment) &&
                blockComment.ValueKind == JsonValueKind.Object)
            {
                blockStart ??= GetString(blockComment, "start");
                blockEnd ??= GetString(blockComment, "end");
            }

            if ((blockStart is null) != (blockEnd is null))
            {
                error = "comments.blockStart and comments.blockEnd must both be set";
                return false;
            }

            var quoteCharacters = new List<string>();
            string? escapeCharacter = null;
            if (root.TryGetProperty("strings", out JsonElement strings) &&
                strings.ValueKind == JsonValueKind.Object)
            {
                foreach (string quote in ReadStringArray(strings, "quoteCharacters"))
                {
                    if (quote.Length > 0 && !quoteCharacters.Contains(quote, StringComparer.Ordinal))
                    {
                        quoteCharacters.Add(quote);
                    }
                }
                escapeCharacter = GetString(strings, "escapeCharacter");
            }

            bool highlightNumbers = true;
            if (root.TryGetProperty("numbers", out JsonElement numbers))
            {
                if (numbers.ValueKind is JsonValueKind.True or JsonValueKind.False)
                {
                    highlightNumbers = numbers.GetBoolean();
                }
            }

            if (lineComment is { Length: 0 })
            {
                lineComment = null;
            }
            if (blockStart is { Length: 0 })
            {
                blockStart = null;
                blockEnd = null;
            }

            definition = new LanguageDefinition(id, name, extensions, keywords,
                controlStatements, types, constants, lineComment, blockStart,
                blockEnd, quoteCharacters, escapeCharacter, highlightNumbers);
            return true;
        }
        catch (JsonException ex)
        {
            error = $"invalid JSON: {ex.Message}";
            return false;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            error = ex.Message;
            return false;
        }
    }

    private static byte[]? Encode(string? value)
    {
        return string.IsNullOrEmpty(value) ? null : Encoding.UTF8.GetBytes(value);
    }

    private static string? GetString(JsonElement element, string property)
    {
        return element.TryGetProperty(property, out JsonElement value) &&
               value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static IEnumerable<string> ReadStringArray(JsonElement element, string property)
    {
        if (!element.TryGetProperty(property, out JsonElement value))
        {
            yield break;
        }

        if (value.ValueKind == JsonValueKind.String)
        {
            string? single = value.GetString();
            if (single is not null)
            {
                yield return single;
            }
            yield break;
        }

        if (value.ValueKind != JsonValueKind.Array)
        {
            yield break;
        }

        foreach (JsonElement item in value.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.String)
            {
                string? text = item.GetString();
                if (text is not null)
                {
                    yield return text;
                }
            }
        }
    }

    private static HashSet<string> ReadSet(JsonElement element, string property)
    {
        var result = new HashSet<string>(StringComparer.Ordinal);
        foreach (string value in ReadStringArray(element, property))
        {
            if (value.Length > 0)
            {
                result.Add(value);
            }
        }
        return result;
    }
}
