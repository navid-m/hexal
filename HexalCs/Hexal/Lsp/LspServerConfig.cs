using System.Text.Json.Nodes;

namespace Hexal.Lsp;

public sealed class LspServerConfig
{
    public string Name { get; init; } = string.Empty;
    public string Command { get; init; } = string.Empty;
    public IReadOnlyList<string> Arguments { get; init; } = Array.Empty<string>();
    public string LanguageId { get; init; } = string.Empty;
    public ILspServerTransport? Transport { get; init; }
}

public sealed record LspDiagnostic(
    int Line,
    int Column,
    int Severity,
    string Message,
    string Source,
    int EndLine = -1,
    int EndColumn = -1
);

/// <summary>
/// A single textual edit from an LSP <c>WorkspaceEdit</c>. Positions use the
/// LSP coordinate system: zero-based lines and UTF-16 code units.
/// </summary>
public sealed record LspTextEdit(
    string Uri,
    int StartLine,
    int StartCharacter,
    int EndLine,
    int EndCharacter,
    string NewText
);

/// <summary>
/// A code action offered by a language server. <see cref="Raw"/> keeps the
/// original JSON object so that data-only actions can be resolved later.
/// </summary>
public sealed class LspCodeAction
{
    public string Title { get; init; } = string.Empty;
    public string Kind { get; init; } = string.Empty;
    public IReadOnlyList<LspTextEdit> Edits { get; init; } = Array.Empty<LspTextEdit>();
    public string Command { get; init; } = string.Empty;
    public JsonArray? CommandArguments { get; init; }
    public bool NeedsResolve { get; init; }
    public JsonObject? Raw { get; init; }
}

public sealed record LspDocumentDiagnostics(string Uri, IReadOnlyList<LspDiagnostic> Diagnostics);

/// <summary>
/// A completion returned by <c>textDocument/completion</c>. Text edit
/// positions use the LSP coordinate system.
/// </summary>
public sealed class LspCompletionItem
{
    public string Label { get; init; } = string.Empty;
    public string InsertText { get; init; } = string.Empty;
    public int InsertTextFormat { get; init; } = 1;
    public int Kind { get; init; }
    public string Detail { get; init; } = string.Empty;
    public string Documentation { get; init; } = string.Empty;
    public string FilterText { get; init; } = string.Empty;
    public string SortText { get; init; } = string.Empty;
    public bool HasTextEdit { get; init; }
    public int StartLine { get; init; }
    public int StartCharacter { get; init; }
    public int EndLine { get; init; }
    public int EndCharacter { get; init; }
}

/// <summary>
/// A target returned by a definition-like request. Positions use the LSP
/// coordinate system: zero-based lines and UTF-16 code units.
/// </summary>
public sealed record LspLocation(string Uri, int Line, int Character);

/// <summary>
/// A symbol returned by <c>textDocument/documentSymbol</c>. Positions use the
/// LSP coordinate system: zero-based lines and UTF-16 code units.
/// </summary>
public sealed record LspDocumentSymbol(
    string Name,
    string Detail,
    int Kind,
    string Uri,
    int Line,
    int Character,
    int Depth
);

public enum LspEventKind
{
    Started,
    Initialized,
    Diagnostics,
    Error,
    Stopped,
    Hover,
    Definition,
    CodeActions,
    CodeActionResolved,
    WorkspaceDiagnostics,
    DocumentSymbols,
    Completions,
}

public sealed class LspEvent
{
    public LspEventKind Kind { get; init; }
    public string ServerName { get; init; } = string.Empty;
    public string Message { get; init; } = string.Empty;
    public string Uri { get; init; } = string.Empty;
    public string Method { get; init; } = string.Empty;
    public string Text { get; init; } = string.Empty;
    public IReadOnlyList<LspDiagnostic> Diagnostics { get; init; } = Array.Empty<LspDiagnostic>();
    public IReadOnlyList<LspLocation> Locations { get; init; } = Array.Empty<LspLocation>();
    public IReadOnlyList<LspCodeAction> CodeActions { get; init; } = Array.Empty<LspCodeAction>();
    public IReadOnlyList<LspDocumentDiagnostics> DocumentDiagnostics { get; init; } =
        Array.Empty<LspDocumentDiagnostics>();
    public IReadOnlyList<LspDocumentSymbol> DocumentSymbols { get; init; } =
        Array.Empty<LspDocumentSymbol>();
    public IReadOnlyList<LspCompletionItem> Completions { get; init; } =
        Array.Empty<LspCompletionItem>();
}

public static class ExecutableFinder
{
    public static string? Find(string command)
    {
        if (string.IsNullOrWhiteSpace(command))
        {
            return null;
        }

        if (Path.IsPathRooted(command))
        {
            return File.Exists(command) ? command : null;
        }

        string path = Environment.GetEnvironmentVariable("PATH") ?? string.Empty;
        string[] extensions = OperatingSystem.IsWindows()
            ? (Environment.GetEnvironmentVariable("PATHEXT") ?? ".EXE;.CMD;.BAT").Split(
                ';',
                StringSplitOptions.RemoveEmptyEntries
            )
            : new[] { string.Empty };

        foreach (
            string directory in path.Split(
                Path.PathSeparator,
                StringSplitOptions.RemoveEmptyEntries
            )
        )
        {
            foreach (string extension in extensions)
            {
                string candidate;
                try
                {
                    candidate = Path.Combine(directory, command + extension);
                }
                catch (Exception ex) when (ex is ArgumentException or NotSupportedException)
                {
                    continue;
                }

                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        return null;
    }
}
