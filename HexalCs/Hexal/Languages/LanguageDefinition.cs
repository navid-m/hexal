using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Hexal.Core;
using Hexal.Dap;

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
    Number,
    Added,
    Deleted
}

public readonly record struct SyntaxToken(int Start, int Length, SyntaxTokenKind Kind)
{
    public int End => Start + Length;
}

public readonly record struct LineHighlightPattern(
    Regex Pattern, SyntaxTokenKind Kind);

/// <summary>
/// Default indentation preferences declared by a language definition. The
/// editor applies these when a buffer is associated with the language.
/// </summary>
public readonly record struct IndentationSettings(
    bool UseTabs,
    int IndentWidth,
    int TabWidth);

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
        List<string> fileNames, List<LineHighlightPattern> lineHighlights,
        HashSet<string> keywords, HashSet<string> controlStatements,
        HashSet<string> types, HashSet<string> constants, string? lineComment,
        string? blockCommentStart, string? blockCommentEnd,
        List<string> quoteCharacters, string? escapeCharacter, bool highlightNumbers,
        FormatterConfig? formatter, bool? formatOnSave,
        IndentationSettings? indentation, bool autoIndent, bool autoClose,
        bool traditionalMarkupLanguage, List<string> indentAfter,
        List<string> unindentBefore, DebuggerConfig? debugger)
    {
        Id = id;
        Name = name;
        Extensions = extensions;
        FileNames = fileNames;
        LineHighlights = lineHighlights;
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
        Formatter = formatter;
        FormatOnSave = formatOnSave;
        Indentation = indentation;
        AutoIndent = autoIndent;
        AutoClose = autoClose;
        TraditionalMarkupLanguage = traditionalMarkupLanguage;
        IndentAfter = indentAfter;
        UnindentBefore = unindentBefore;
        Debugger = debugger;

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

    /// <summary>
    /// Exact file names (for example <c>meson.build</c> or
    /// <c>COMMIT_EDITMSG</c>) that select this language regardless of
    /// extension.
    /// </summary>
    public IReadOnlyList<string> FileNames { get; }

    /// <summary>
    /// Optional regular expressions applied to the start of each line before
    /// the normal lexer. The first matching pattern highlights its match with
    /// the configured token kind.
    /// </summary>
    public IReadOnlyList<LineHighlightPattern> LineHighlights { get; }

    public string? LineComment { get; }

    public string? BlockCommentStart { get; }

    public string? BlockCommentEnd { get; }

    public IReadOnlyList<string> QuoteCharacters { get; }

    public string? EscapeCharacter { get; }

    public bool HighlightNumbers { get; }

    public FormatterConfig? Formatter { get; }

    public bool? FormatOnSave { get; }

    /// <summary>
    /// Optional default indentation style from the grammar's
    /// <c>indentation</c> object. Null leaves the buffer's current settings
    /// untouched.
    /// </summary>
    public IndentationSettings? Indentation { get; }

    /// <summary>
    /// When true, pressing Enter can add or remove an indentation level based
    /// on the grammar's indentation triggers. Defaults to true.
    /// </summary>
    public bool AutoIndent { get; }

    /// <summary>
    /// When true, typing an opening bracket or markup tag inserts its closing
    /// counterpart. Defaults to true.
    /// </summary>
    public bool AutoClose { get; }

    /// <summary>
    /// Marks tag-based markup languages such as XML and HTML, enabling
    /// automatic closing-tag insertion when <see cref="AutoClose"/> is true.
    /// </summary>
    public bool TraditionalMarkupLanguage { get; }

    /// <summary>
    /// Extra literal line suffixes that increase indentation on Enter, on top
    /// of the built-in bracket rules.
    /// </summary>
    public IReadOnlyList<string> IndentAfter { get; }

    /// <summary>
    /// Extra literal line prefixes that cause an automatically inserted
    /// closing line to unindent.
    /// </summary>
    public IReadOnlyList<string> UnindentBefore { get; }

    /// <summary>
    /// Optional Debug Adapter Protocol configuration for this language.
    /// </summary>
    public DebuggerConfig? Debugger { get; }

    /// <summary>
    /// True when a line's leading text exactly matches one of the grammar's
    /// auto-unindent keywords.
    /// </summary>
    public bool IsUnindentKeyword(string text)
    {
        foreach (string keyword in UnindentBefore)
        {
            if (string.Equals(keyword, text, StringComparison.Ordinal))
            {
                return true;
            }
        }
        return false;
    }

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

            var fileNames = new List<string>();
            foreach (string fileName in ReadStringArray(root, "fileNames")
                         .Concat(ReadStringArray(root, "file_names")))
            {
                string value = fileName.Trim();
                if (value.Length > 0 &&
                    !fileNames.Contains(value, StringComparer.OrdinalIgnoreCase))
                {
                    fileNames.Add(value);
                }
            }

            var lineHighlights = new List<LineHighlightPattern>();
            if (root.TryGetProperty("linePatterns", out JsonElement linePatterns))
            {
                if (linePatterns.ValueKind != JsonValueKind.Array)
                {
                    error = "linePatterns must be an array";
                    return false;
                }

                int index = 0;
                foreach (JsonElement linePattern in linePatterns.EnumerateArray())
                {
                    if (linePattern.ValueKind != JsonValueKind.Object)
                    {
                        error = $"linePatterns[{index}] must be a JSON object";
                        return false;
                    }

                    string pattern = GetString(linePattern, "pattern")?.Trim() ??
                                     string.Empty;
                    if (pattern.Length == 0)
                    {
                        error = $"linePatterns[{index}].pattern is required";
                        return false;
                    }

                    string kindName = GetString(linePattern, "kind")?.Trim() ??
                                      string.Empty;
                    if (!TryReadTokenKind(kindName, out SyntaxTokenKind kind))
                    {
                        error = $"linePatterns[{index}].kind is invalid";
                        return false;
                    }

                    Regex regex;
                    try
                    {
                        regex = new Regex(pattern,
                            RegexOptions.CultureInvariant);
                    }
                    catch (ArgumentException ex)
                    {
                        error = $"linePatterns[{index}].pattern is invalid: {ex.Message}";
                        return false;
                    }

                    lineHighlights.Add(new LineHighlightPattern(regex, kind));
                    ++index;
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

            bool? formatOnSave = ReadBool(root, "formatOnSave") ??
                                 ReadBool(root, "format_on_save");
            FormatterConfig? formatter = null;
            if (root.TryGetProperty("formatter", out JsonElement formatterElement) &&
                formatterElement.ValueKind == JsonValueKind.Object)
            {
                string command = GetString(formatterElement, "command")?.Trim() ??
                                 string.Empty;
                if (command.Length == 0)
                {
                    error = "formatter.command is required";
                    return false;
                }

                string input = (GetString(formatterElement, "input") ?? "stdin")
                    .Trim().ToLowerInvariant();
                if (input is not ("stdin" or "file"))
                {
                    error = "formatter.input must be 'stdin' or 'file'";
                    return false;
                }

                string output = (GetString(formatterElement, "output") ?? "stdout")
                    .Trim().ToLowerInvariant();
                if (output is not ("stdout" or "file"))
                {
                    error = "formatter.output must be 'stdout' or 'file'";
                    return false;
                }

                int timeoutMs = 15000;
                if (formatterElement.TryGetProperty("timeout", out JsonElement timeout) &&
                    timeout.ValueKind == JsonValueKind.Number &&
                    timeout.TryGetInt32(out int configuredTimeout) &&
                    configuredTimeout > 0)
                {
                    timeoutMs = configuredTimeout;
                }

                formatOnSave ??= ReadBool(formatterElement, "formatOnSave") ??
                                  ReadBool(formatterElement, "format_on_save");

                formatter = new FormatterConfig
                {
                    Command = command,
                    Arguments = ReadStringArray(formatterElement, "arguments").ToList(),
                    Input = input,
                    Output = output,
                    TimeoutMs = timeoutMs
                };
            }

            DebuggerConfig? debugger = null;
            if (root.TryGetProperty("debugger", out JsonElement debuggerElement))
            {
                if (debuggerElement.ValueKind != JsonValueKind.Object)
                {
                    error = "debugger must be a JSON object";
                    return false;
                }

                string adapter = (GetString(debuggerElement, "adapter") ?? string.Empty)
                    .Trim();
                if (adapter.Length == 0)
                {
                    error = "debugger.adapter is required";
                    return false;
                }

                var debuggerConfiguration = new JsonObject();
                if (debuggerElement.TryGetProperty("configuration",
                        out JsonElement configurationElement))
                {
                    if (configurationElement.ValueKind != JsonValueKind.Object)
                    {
                        error = "debugger.configuration must be a JSON object";
                        return false;
                    }

                    debuggerConfiguration =
                        JsonNode.Parse(configurationElement.GetRawText()) as JsonObject ??
                        new JsonObject();
                }

                debugger = new DebuggerConfig
                {
                    Adapter = adapter,
                    Arguments = ReadStringArray(debuggerElement, "arguments").ToList(),
                    Configuration = debuggerConfiguration
                };
            }

            IndentationSettings? indentation = null;
            if (root.TryGetProperty("indentation", out JsonElement indentationElement))
            {
                if (indentationElement.ValueKind != JsonValueKind.Object)
                {
                    error = "indentation must be a JSON object";
                    return false;
                }

                string style = (GetString(indentationElement, "style") ?? "spaces")
                    .Trim().ToLowerInvariant();
                if (style is not ("spaces" or "tabs"))
                {
                    error = "indentation.style must be 'spaces' or 'tabs'";
                    return false;
                }

                bool useTabs = style == "tabs";
                int indentWidth = useTabs
                    ? Document.DefaultTabWidth
                    : Document.DefaultIndentWidth;
                if (indentationElement.TryGetProperty("width", out JsonElement widthValue) &&
                    (widthValue.ValueKind != JsonValueKind.Number ||
                     !widthValue.TryGetInt32(out indentWidth) ||
                     indentWidth is < 1 or > 16))
                {
                    error = "indentation.width must be an integer between 1 and 16";
                    return false;
                }

                int tabWidth = useTabs ? indentWidth : Document.DefaultTabWidth;
                if (indentationElement.TryGetProperty("tabWidth",
                        out JsonElement tabWidthValue) &&
                    (tabWidthValue.ValueKind != JsonValueKind.Number ||
                     !tabWidthValue.TryGetInt32(out tabWidth) ||
                     tabWidth is < 1 or > 16))
                {
                    error = "indentation.tabWidth must be an integer between 1 and 16";
                    return false;
                }

                indentation = new IndentationSettings(useTabs, indentWidth, tabWidth);
            }

            bool autoIndent = ReadBool(root, "autoIndent") ??
                              ReadBool(root, "auto_indent") ?? true;
            bool autoClose = ReadBool(root, "autoClose") ??
                             ReadBool(root, "auto_close") ?? true;
            bool traditionalMarkupLanguage =
                ReadBool(root, "traditionalMarkupLanguage") ??
                ReadBool(root, "traditional_markup_language") ?? false;

            List<string> indentAfter = ReadStringArray(root, "indentAfter")
                .Concat(ReadStringArray(root, "indent_after"))
                .Where(value => value.Length > 0)
                .Distinct(StringComparer.Ordinal)
                .ToList();
            List<string> unindentBefore = ReadStringArray(root, "unindentBefore")
                .Concat(ReadStringArray(root, "unindent_before"))
                .Where(value => value.Length > 0)
                .Distinct(StringComparer.Ordinal)
                .ToList();

            if (lineComment is { Length: 0 })
            {
                lineComment = null;
            }
            if (blockStart is { Length: 0 })
            {
                blockStart = null;
                blockEnd = null;
            }

            definition = new LanguageDefinition(id, name, extensions, fileNames,
                lineHighlights, keywords, controlStatements, types, constants,
                lineComment, blockStart, blockEnd, quoteCharacters,
                escapeCharacter, highlightNumbers, formatter, formatOnSave,
                indentation, autoIndent, autoClose, traditionalMarkupLanguage,
                indentAfter, unindentBefore, debugger);
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

    private static bool TryReadTokenKind(string name, out SyntaxTokenKind kind)
    {
        switch (name.Trim().ToLowerInvariant())
        {
            case "keyword":
                kind = SyntaxTokenKind.Keyword;
                return true;
            case "control":
            case "controlstatement":
            case "control_statement":
                kind = SyntaxTokenKind.ControlStatement;
                return true;
            case "type":
                kind = SyntaxTokenKind.Type;
                return true;
            case "constant":
                kind = SyntaxTokenKind.Constant;
                return true;
            case "string":
            case "stringliteral":
            case "string_literal":
                kind = SyntaxTokenKind.StringLiteral;
                return true;
            case "comment":
                kind = SyntaxTokenKind.Comment;
                return true;
            case "number":
                kind = SyntaxTokenKind.Number;
                return true;
            case "added":
                kind = SyntaxTokenKind.Added;
                return true;
            case "deleted":
                kind = SyntaxTokenKind.Deleted;
                return true;
            default:
                kind = SyntaxTokenKind.Default;
                return false;
        }
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

    private static bool? ReadBool(JsonElement element, string property)
    {
        if (!element.TryGetProperty(property, out JsonElement value))
        {
            return null;
        }

        return value.ValueKind switch
        {
            JsonValueKind.True => true,
            JsonValueKind.False => false,
            _ => null
        };
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
