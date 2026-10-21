namespace Hexal.Lsp;

public sealed class LspServerConfig
{
    public string Name { get; init; } = string.Empty;
    public string Command { get; init; } = string.Empty;
    public IReadOnlyList<string> Arguments { get; init; } = Array.Empty<string>();
    public string LanguageId { get; init; } = string.Empty;
}

public sealed record LspDiagnostic(
    int Line,
    int Column,
    int Severity,
    string Message,
    string Source);

public enum LspEventKind
{
    Started,
    Initialized,
    Diagnostics,
    Error,
    Stopped
}

public sealed class LspEvent
{
    public LspEventKind Kind { get; init; }
    public string ServerName { get; init; } = string.Empty;
    public string Message { get; init; } = string.Empty;
    public string Uri { get; init; } = string.Empty;
    public IReadOnlyList<LspDiagnostic> Diagnostics { get; init; } =
        Array.Empty<LspDiagnostic>();
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
            ? (Environment.GetEnvironmentVariable("PATHEXT") ?? ".EXE;.CMD;.BAT")
                .Split(';', StringSplitOptions.RemoveEmptyEntries)
            : new[] { string.Empty };

        foreach (string directory in path.Split(Path.PathSeparator,
                     StringSplitOptions.RemoveEmptyEntries))
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
