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

        // Parsing is CPU-bound and each definition is independent. Parse in
        // parallel, then merge in file order so duplicate handling stays
        // deterministic.
        var parsed = new (LanguageDefinition? Definition, string? Error)[files.Length];
        Parallel.For(0, files.Length, index =>
        {
            string file = files[index];
            try
            {
                string text = File.ReadAllText(file);
                if (LanguageDefinition.TryParse(text, file,
                        out LanguageDefinition? definition, out string error))
                {
                    parsed[index] = (definition, null);
                }
                else
                {
                    parsed[index] = (null, error);
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                parsed[index] = (null, ex.Message);
            }
        });

        for (int index = 0; index < files.Length; ++index)
        {
            string file = files[index];
            (LanguageDefinition? definition, string? error) = parsed[index];
            if (definition is null)
            {
                _errors.Add($"{Path.GetFileName(file)}: {error}");
                continue;
            }

            LanguageDefinition language = definition;
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
                    // Several grammars can legitimately share an extension
                    // (.h is both C and C++). Keep the deterministic first
                    // match; users can still select the other with :lang.
                    HexalPaths.Log(
                        $"Grammar warning: extension '{extension}' belongs to " +
                        $"'{existing.Id}'; ignoring duplicate from '{language.Id}'");
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
