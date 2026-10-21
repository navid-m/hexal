using Hexal.Util;

namespace Hexal.Languages;

/// <summary>
/// Loads data-driven .hexdef grammars from ~/.hexal/grammars (or an explicit
/// directory for tests and embedders). Invalid definitions are reported but do
/// not prevent the editor from starting.
/// </summary>
public sealed class LanguageRegistry
{
    private readonly Dictionary<string, LanguageDefinition> _byId =
        new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, LanguageDefinition> _byExtension =
        new(StringComparer.OrdinalIgnoreCase);
    private readonly List<string> _errors = new();

    public IReadOnlyList<string> Errors => _errors;

    public IReadOnlyCollection<LanguageDefinition> Languages => _byId.Values;

    public static string DefaultGrammarDirectory => HexalPaths.GrammarDirectory;

    public static LanguageRegistry LoadUserGrammars()
    {
        var registry = new LanguageRegistry();
        string directory = DefaultGrammarDirectory;

        try
        {
            Directory.CreateDirectory(directory);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            registry._errors.Add($"cannot create grammar directory '{directory}': {ex.Message}");
            return registry;
        }

        registry.LoadDirectory(directory);
        HexalPaths.Log(
            $"Grammars: {directory} ({registry._byId.Count} loaded, {registry._errors.Count} error(s))");
        return registry;
    }

    public void LoadDirectory(string directory)
    {
        string[] files;
        try
        {
            files = Directory.GetFiles(directory, "*.hexdef");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            _errors.Add($"cannot read grammar directory '{directory}': {ex.Message}");
            return;
        }

        Array.Sort(files, StringComparer.OrdinalIgnoreCase);

        foreach (string file in files)
        {
            string text;
            try
            {
                text = File.ReadAllText(file);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                _errors.Add($"{Path.GetFileName(file)}: {ex.Message}");
                continue;
            }

            if (!LanguageDefinition.TryParse(text, file, out LanguageDefinition? definition,
                    out string error))
            {
                _errors.Add($"{Path.GetFileName(file)}: {error}");
                continue;
            }

            LanguageDefinition language = definition!;
            if (_byId.ContainsKey(language.Id))
            {
                _errors.Add($"{Path.GetFileName(file)}: duplicate language id '{language.Id}'");
                continue;
            }

            _byId.Add(language.Id, language);

            foreach (string extension in language.Extensions)
            {
                if (_byExtension.TryGetValue(extension, out LanguageDefinition? existing))
                {
                    _errors.Add(
                        $"{Path.GetFileName(file)}: extension '{extension}' already belongs to '{existing.Id}'");
                    continue;
                }
                _byExtension.Add(extension, language);
            }
        }
    }

    public LanguageDefinition? Find(string id)
    {
        return _byId.TryGetValue(id, out LanguageDefinition? language) ? language : null;
    }

    public LanguageDefinition? Detect(string? path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return null;
        }

        string extension;
        try
        {
            extension = Path.GetExtension(path);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException)
        {
            return null;
        }

        if (extension.Length == 0)
        {
            return null;
        }

        return _byExtension.TryGetValue(extension, out LanguageDefinition? language)
            ? language
            : null;
    }
}
