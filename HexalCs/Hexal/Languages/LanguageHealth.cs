using Hexal.Lsp;
using Hexal.Scripting;

namespace Hexal.Languages;

/// <summary>
/// The health of one language's editor support. A null language server or
/// formatter means that none is configured for the language.
/// </summary>
public sealed record LanguageHealthEntry(
    string Language,
    string? LanguageServer,
    bool LanguageServerAvailable,
    string? Formatter,
    bool FormatterAvailable,
    bool Highlighting);

public sealed class LanguageHealthReport
{
    public LanguageHealthReport(IReadOnlyList<LanguageHealthEntry> entries,
        IReadOnlyList<string> errors)
    {
        Entries = entries;
        Errors = errors;
    }

    public IReadOnlyList<LanguageHealthEntry> Entries { get; }

    public IReadOnlyList<string> Errors { get; }
}

/// <summary>
/// Builds a <c>--health</c> report from the same data-driven grammars and Lua
/// plugins the editor uses. Plugin language-server candidates are collected by
/// firing <c>buffer_opened</c> with a lightweight host, so plugins are inspected
/// but no external process is started.
/// </summary>
public static class LanguageHealth
{
    public static LanguageHealthReport BuildDefault()
    {
        return Build(LanguageRegistry.DefaultGrammarDirectory,
            LuaPluginManager.DefaultPluginDirectory,
            LuaPluginManager.DefaultStatePath);
    }

    public static LanguageHealthReport Build(string grammarDirectory,
        string pluginDirectory, string? pluginStatePath = null)
    {
        var errors = new List<string>();
        var registry = new LanguageRegistry();
        registry.LoadDirectory(grammarDirectory);
        errors.AddRange(registry.Errors);

        List<LanguageDefinition> languages = registry.Languages
            .OrderBy(language => language.Id, StringComparer.OrdinalIgnoreCase)
            .ToList();

        var host = new HealthPluginHost();
        var captured = new List<CapturedServer>();
        var builtIn = new List<CapturedServer>();

        try
        {
            using var plugins = new LuaPluginManager(host, pluginDirectory,
                pluginStatePath ?? string.Empty);
            plugins.LoadAll();
            errors.AddRange(plugins.Errors);

            foreach (LanguageDefinition language in languages)
            {
                host.CurrentLanguage = language.Id;
                plugins.FireEvent("buffer_opened",
                    new Dictionary<string, object?> { ["language"] = language.Id });
            }

            host.CurrentLanguage = null;
            captured.AddRange(host.Captured);

            foreach (LuaLspCoordinator coordinator in plugins.Coordinators)
            {
                foreach (LuaLspServer server in coordinator.Servers)
                {
                    foreach (string language in ResolveServerLanguages(server, registry))
                    {
                        builtIn.Add(new CapturedServer(language, server.Name, true));
                    }
                }
            }
        }
        catch (Exception ex)
        {
            errors.Add($"plugins: {ex.Message}");
        }

        var entries = new List<LanguageHealthEntry>(languages.Count);
        foreach (LanguageDefinition language in languages)
        {
            List<CapturedServer> servers = builtIn
                .Concat(captured)
                .Where(server => string.Equals(server.Language, language.Id,
                    StringComparison.OrdinalIgnoreCase))
                .GroupBy(server => (server.Name, server.Command),
                    StringTupleComparer.Instance)
                .Select(group => group.First())
                .ToList();

            CapturedServer? available = servers.FirstOrDefault(server => server.Available);
            CapturedServer? selected = available ?? servers.FirstOrDefault();

            string? formatter = language.Formatter?.Command;
            bool formatterAvailable = formatter is not null &&
                                      ExecutableFinder.Find(formatter) is not null;

            entries.Add(new LanguageHealthEntry(
                language.Id,
                selected?.Name,
                selected?.Available ?? false,
                formatter,
                formatterAvailable,
                Highlighting: true));
        }

        return new LanguageHealthReport(entries, errors);
    }

    private static HashSet<string> ResolveServerLanguages(LuaLspServer server,
        LanguageRegistry registry)
    {
        var languages = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (string filetype in server.Filetypes)
        {
            if (filetype.Length > 0)
            {
                languages.Add(filetype);
            }
        }

        if (languages.Count == 0)
        {
            foreach (string extension in server.Extensions)
            {
                LanguageDefinition? language = registry.Detect("file" + extension);
                if (language is not null)
                {
                    languages.Add(language.Id);
                }
            }
        }

        return languages;
    }

    private sealed record CapturedServer(string Language, string Name, bool Available,
        string Command = "");

    private sealed class HealthPluginHost : IEditorApi
    {
        public List<CapturedServer> Captured { get; } = new();

        public string? CurrentLanguage { get; set; }

        public string CurrentBufferId => string.Empty;

        public string? WorkspaceRoot => null;

        public int OpenPluginPanel(PluginPanelOptions options,
            Func<string, long, bool>? onKey, string pluginId) => 0;

        public void SetPluginPanelLines(int panelId,
            IReadOnlyList<PluginPanelLine> lines)
        {
        }

        public void SetPluginPanelTitle(int panelId, string title)
        {
        }

        public void SetPluginPanelFooter(int panelId, string footer)
        {
        }

        public void SetPluginPanelSelected(int panelId, long selected)
        {
        }

        public void SetPluginPanelSelectable(int panelId, bool selectable)
        {
        }

        public void ClosePluginPanel(int panelId)
        {
        }

        public void ClosePluginPanels(string pluginId)
        {
        }

        public string GetText() => string.Empty;

        public string? GetLine(long line, out string error)
        {
            error = string.Empty;
            return null;
        }

        public PluginCursor GetCursor() => default;

        public bool SetCursor(long line, long column, out string error)
        {
            error = string.Empty;
            return false;
        }

        public PluginSelection? GetSelection() => null;

        public bool InsertText(string text, out string error)
        {
            error = string.Empty;
            return false;
        }

        public bool ReplaceRange(object? start, object? finish, string text,
            out string error)
        {
            error = string.Empty;
            return false;
        }

        public void ShowMessage(string text, bool warning)
        {
        }

        public void SetPluginStatus(string key, string? text)
        {
        }

        public void RequestPrompt(string label, Action<string?> callback)
        {
        }

        public void RequestSelect(string title, IReadOnlyList<string> items,
            Action<int?> callback)
        {
        }

        public bool StartLsp(LspServerConfig config, out string error)
        {
            bool available = ExecutableFinder.Find(config.Command) is not null;
            string language = !string.IsNullOrEmpty(CurrentLanguage)
                ? CurrentLanguage!
                : config.LanguageId;

            if (language.Length > 0)
            {
                Captured.Add(new CapturedServer(language, config.Name, available,
                    config.Command));
            }

            error = available ? string.Empty : $"'{config.Command}' was not found in PATH";
            return available;
        }

        public void StopLsp(string? name)
        {
        }

        public bool ExecuteCommand(string name, IReadOnlyList<string> args,
            out string error)
        {
            error = string.Empty;
            return false;
        }

        public void ReportPluginError(string pluginId, string message)
        {
        }
    }

    private sealed class StringTupleComparer : IEqualityComparer<(string, string)>
    {
        public static readonly StringTupleComparer Instance = new();

        public bool Equals((string, string) x, (string, string) y)
        {
            return string.Equals(x.Item1, y.Item1, StringComparison.Ordinal) &&
                   string.Equals(x.Item2, y.Item2, StringComparison.Ordinal);
        }

        public int GetHashCode((string, string) obj)
        {
            return HashCode.Combine(
                StringComparer.Ordinal.GetHashCode(obj.Item1),
                StringComparer.Ordinal.GetHashCode(obj.Item2));
        }
    }
}
