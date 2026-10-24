using System.Globalization;
using System.Text.Json;
using Hexal.Util;

namespace Hexal.Ui;

/// <summary>
/// A named colour palette for the editor. Colours are either an ANSI palette
/// index (0-255) or a packed 24-bit RGB value marked by <see cref="TrueColorFlag"/>.
/// Themes are data files in <c>data/themes</c> so they can be copied, edited and
/// added without rebuilding Hexal.
/// </summary>
public sealed class Theme
{
    public const uint TrueColorFlag = 0x0100_0000;

    public string Id { get; set; } = "hexal-dark";

    public string Name { get; set; } = "Hexal Dark";

    public string Description { get; set; } = "Default Hexal theme.";

    public string Author { get; set; } = string.Empty;

    public string License { get; set; } = string.Empty;

    public uint Foreground { get; set; } = 15;
    public uint Background { get; set; }
    public uint Muted { get; set; } = 8;
    public uint Accent { get; set; } = 14;
    public uint Border { get; set; } = 8;
    public uint Error { get; set; } = 9;
    public uint Warning { get; set; } = 11;
    public uint Success { get; set; } = 10;

    public uint DiffAdded { get; set; } = 2;
    public uint DiffDeleted { get; set; } = 1;
    public uint DiffModified { get; set; } = 4;

    /// <summary>
    /// Marker colour for files and folders modified in the Git working tree.
    /// </summary>
    public uint GitModified { get; set; } = 14;

    public uint AlertForeground { get; set; } = 15;
    public uint AlertBackground { get; set; } = 5;

    public uint SelectionForeground { get; set; } = 15;
    public uint SelectionBackground { get; set; } = 4;

    public uint StatusForeground { get; set; } = 15;
    public uint StatusBackground { get; set; } = 4;

    public uint InsertForeground { get; set; }
    public uint InsertBackground { get; set; } = 2;

    public uint SelectForeground { get; set; }
    public uint SelectBackground { get; set; } = 3;

    public uint PromptForeground { get; set; }
    public uint PromptBackground { get; set; } = 6;

    public uint CommandForeground { get; set; }
    public uint CommandBackground { get; set; } = 6;

    public uint MatchForeground { get; set; }
    public uint MatchBackground { get; set; } = 3;

    public uint CurrentMatchForeground { get; set; }
    public uint CurrentMatchBackground { get; set; } = 11;

    public uint Keyword { get; set; } = 13;
    public uint ControlStatement { get; set; } = 12;
    public uint Type { get; set; } = 14;
    public uint Constant { get; set; } = 11;
    public uint StringLiteral { get; set; } = 11;
    public uint Comment { get; set; } = 10;
    public uint Number { get; set; } = 11;

    public Style BaseStyle => new() { Fg = Foreground, Bg = Background };

    public Style TextStyle => new() { Fg = Foreground, Bg = Background };

    public Style MutedStyle =>
        new()
        {
            Fg = Muted,
            Bg = Background,
            Dim = true,
        };

    public Style AccentStyle =>
        new()
        {
            Fg = Accent,
            Bg = Background,
            Bold = true,
        };

    public Style BorderStyle =>
        new()
        {
            Fg = Border,
            Bg = Background,
            Dim = true,
        };

    public Style ErrorStyle =>
        new()
        {
            Fg = Error,
            Bg = Background,
            Bold = true,
        };

    public Style WarningStyle =>
        new()
        {
            Fg = Warning,
            Bg = Background,
            Bold = true,
        };

    public Style SelectionStyle => new() { Fg = SelectionForeground, Bg = SelectionBackground };

    public Style SelectedItemStyle =>
        new()
        {
            Fg = SelectionForeground,
            Bg = SelectionBackground,
            Bold = true,
        };

    public Style StatusStyle =>
        new()
        {
            Fg = StatusForeground,
            Bg = StatusBackground,
            Bold = true,
        };

    public Style InsertStatusStyle =>
        new()
        {
            Fg = InsertForeground,
            Bg = InsertBackground,
            Bold = true,
        };

    public Style SelectStatusStyle =>
        new()
        {
            Fg = SelectForeground,
            Bg = SelectBackground,
            Bold = true,
        };

    public Style PromptStatusStyle =>
        new()
        {
            Fg = AlertForeground,
            Bg = AlertBackground,
            Bold = true,
        };

    public Style CommandStatusStyle =>
        new()
        {
            Fg = CommandForeground,
            Bg = CommandBackground,
            Bold = true,
        };

    public Style MatchStyle => new() { Fg = MatchForeground, Bg = MatchBackground };

    public Style CurrentMatchStyle =>
        new()
        {
            Fg = CurrentMatchForeground,
            Bg = CurrentMatchBackground,
            Bold = true,
        };

    public Style CurrentLineNumberStyle =>
        new()
        {
            Fg = Foreground,
            Bg = Background,
            Bold = true,
        };

    public Style DiffAddedStyle =>
        new()
        {
            Fg = DiffAdded,
            Bg = Background,
            Bold = true,
        };

    public Style DiffDeletedStyle =>
        new()
        {
            Fg = DiffDeleted,
            Bg = Background,
            Bold = true,
        };

    public Style DiffModifiedStyle =>
        new()
        {
            Fg = DiffModified,
            Bg = Background,
            Bold = true,
        };

    public Style GitModifiedStyle =>
        new()
        {
            Fg = GitModified,
            Bg = Background,
            Bold = true,
        };

    public Style SyntaxStyle(uint foreground) => new() { Fg = foreground, Bg = Background };

    public static Theme CreateDefault() => new();

    public static uint Rgb(byte red, byte green, byte blue) =>
        TrueColorFlag | ((uint)red << 16) | ((uint)green << 8) | blue;

    public static bool IsTrueColor(uint color) => (color & TrueColorFlag) != 0;

    public static byte Red(uint color) => (byte)((color >> 16) & 0xFF);

    public static byte Green(uint color) => (byte)((color >> 8) & 0xFF);

    public static byte Blue(uint color) => (byte)(color & 0xFF);

    public static bool TryParse(string json, string sourcePath, out Theme? theme, out string error)
    {
        theme = null;
        error = string.Empty;

        try
        {
            using JsonDocument document = JsonDocument.Parse(
                json,
                new JsonDocumentOptions
                {
                    AllowTrailingCommas = true,
                    CommentHandling = JsonCommentHandling.Skip,
                }
            );

            JsonElement root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                error = "root must be a JSON object";
                return false;
            }

            string fallbackId = Path.GetFileNameWithoutExtension(sourcePath);
            var result = CreateDefault();
            result.Id = ReadString(root, "id", fallbackId);
            if (result.Id.Length == 0)
            {
                result.Id = "theme";
            }
            result.Name = ReadString(root, "name", result.Id);
            result.Description = ReadString(root, "description", string.Empty);
            result.Author = ReadString(root, "author", string.Empty).Trim();
            result.License = ReadString(root, "license", string.Empty).Trim();

            if (root.TryGetProperty("colors", out JsonElement colors))
            {
                if (colors.ValueKind != JsonValueKind.Object)
                {
                    error = "colors must be a JSON object";
                    return false;
                }

                result.Foreground = ReadColor(colors, "foreground", result.Foreground, ref error);
                result.Background = ReadColor(colors, "background", result.Background, ref error);
                result.Muted = ReadColor(colors, "muted", result.Muted, ref error);
                result.Accent = ReadColor(colors, "accent", result.Accent, ref error);
                result.Border = ReadColor(colors, "border", result.Border, ref error);
                result.Error = ReadColor(colors, "error", result.Error, ref error);
                result.Warning = ReadColor(colors, "warning", result.Warning, ref error);
                result.Success = ReadColor(colors, "success", result.Success, ref error);
                result.GitModified = ReadColor(
                    colors,
                    "gitModified",
                    result.GitModified,
                    ref error
                );

                (result.AlertForeground, result.AlertBackground) = ReadPair(
                    colors,
                    "alert",
                    result.AlertForeground,
                    result.AlertBackground,
                    ref error
                );
                (result.SelectionForeground, result.SelectionBackground) = ReadPair(
                    colors,
                    "selection",
                    result.SelectionForeground,
                    result.SelectionBackground,
                    ref error
                );
                (result.StatusForeground, result.StatusBackground) = ReadPair(
                    colors,
                    "status",
                    result.StatusForeground,
                    result.StatusBackground,
                    ref error
                );
                (result.InsertForeground, result.InsertBackground) = ReadPair(
                    colors,
                    "insert",
                    result.InsertForeground,
                    result.InsertBackground,
                    ref error
                );
                (result.SelectForeground, result.SelectBackground) = ReadPair(
                    colors,
                    "select",
                    result.SelectForeground,
                    result.SelectBackground,
                    ref error
                );
                (result.PromptForeground, result.PromptBackground) = ReadPair(
                    colors,
                    "prompt",
                    result.PromptForeground,
                    result.PromptBackground,
                    ref error
                );
                (result.CommandForeground, result.CommandBackground) = ReadPair(
                    colors,
                    "command",
                    result.CommandForeground,
                    result.CommandBackground,
                    ref error
                );
                (result.MatchForeground, result.MatchBackground) = ReadPair(
                    colors,
                    "match",
                    result.MatchForeground,
                    result.MatchBackground,
                    ref error
                );
                (result.CurrentMatchForeground, result.CurrentMatchBackground) = ReadPair(
                    colors,
                    "currentMatch",
                    result.CurrentMatchForeground,
                    result.CurrentMatchBackground,
                    ref error
                );

                if (colors.TryGetProperty("syntax", out JsonElement syntax))
                {
                    if (syntax.ValueKind != JsonValueKind.Object)
                    {
                        error = "colors.syntax must be a JSON object";
                        return false;
                    }

                    result.Keyword = ReadColor(syntax, "keyword", result.Keyword, ref error);
                    result.ControlStatement = ReadColor(
                        syntax,
                        "control",
                        result.ControlStatement,
                        ref error
                    );
                    result.Type = ReadColor(syntax, "type", result.Type, ref error);
                    result.Constant = ReadColor(syntax, "constant", result.Constant, ref error);
                    result.StringLiteral = ReadColor(
                        syntax,
                        "string",
                        result.StringLiteral,
                        ref error
                    );
                    result.Comment = ReadColor(syntax, "comment", result.Comment, ref error);
                    result.Number = ReadColor(syntax, "number", result.Number, ref error);
                }
            }

            if (error.Length > 0)
            {
                return false;
            }

            theme = result;
            return true;
        }
        catch (JsonException ex)
        {
            error = $"invalid JSON: {ex.Message}";
            return false;
        }
    }

    public static bool TryParseColor(JsonElement element, out uint color)
    {
        color = 0;

        if (element.ValueKind == JsonValueKind.Number)
        {
            if (element.TryGetInt32(out int number) && number is >= 0 and <= 255)
            {
                color = (uint)number;
                return true;
            }
            return false;
        }

        if (element.ValueKind != JsonValueKind.String)
        {
            return false;
        }

        string text = (element.GetString() ?? string.Empty).Trim();
        if (text.Length == 0)
        {
            return false;
        }

        if (text[0] == '#')
        {
            string hex = text[1..];
            if (hex.Length == 3)
            {
                hex = string.Concat(hex[0], hex[0], hex[1], hex[1], hex[2], hex[2]);
            }

            if (
                hex.Length == 6
                && byte.TryParse(
                    hex[..2],
                    NumberStyles.HexNumber,
                    CultureInfo.InvariantCulture,
                    out byte red
                )
                && byte.TryParse(
                    hex[2..4],
                    NumberStyles.HexNumber,
                    CultureInfo.InvariantCulture,
                    out byte green
                )
                && byte.TryParse(
                    hex[4..6],
                    NumberStyles.HexNumber,
                    CultureInfo.InvariantCulture,
                    out byte blue
                )
            )
            {
                color = Rgb(red, green, blue);
                return true;
            }

            return false;
        }

        if (int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out int index))
        {
            if (index is >= 0 and <= 255)
            {
                color = (uint)index;
                return true;
            }
            return false;
        }

        if (NamedColors.TryGetValue(text, out uint named))
        {
            color = named;
            return true;
        }

        return false;
    }

    private static readonly Dictionary<string, uint> NamedColors = new(
        StringComparer.OrdinalIgnoreCase
    )
    {
        ["black"] = 0,
        ["red"] = 1,
        ["green"] = 2,
        ["yellow"] = 3,
        ["blue"] = 4,
        ["magenta"] = 5,
        ["cyan"] = 6,
        ["white"] = 7,
        ["bright-black"] = 8,
        ["bright_black"] = 8,
        ["gray"] = 8,
        ["grey"] = 8,
        ["bright-red"] = 9,
        ["bright_red"] = 9,
        ["bright-green"] = 10,
        ["bright_green"] = 10,
        ["bright-yellow"] = 11,
        ["bright_yellow"] = 11,
        ["bright-blue"] = 12,
        ["bright_blue"] = 12,
        ["bright-magenta"] = 13,
        ["bright_magenta"] = 13,
        ["bright-cyan"] = 14,
        ["bright_cyan"] = 14,
        ["bright-white"] = 15,
        ["bright_white"] = 15,
    };

    private static string ReadString(JsonElement element, string property, string fallback)
    {
        return
            element.TryGetProperty(property, out JsonElement value)
            && value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? fallback
            : fallback;
    }

    private static uint ReadColor(
        JsonElement element,
        string property,
        uint fallback,
        ref string error
    )
    {
        if (!element.TryGetProperty(property, out JsonElement value))
        {
            return fallback;
        }

        if (!TryParseColor(value, out uint color))
        {
            if (error.Length == 0)
            {
                error = $"{property}: invalid color (use 0-255, a name, or #RRGGBB)";
            }
            return fallback;
        }

        return color;
    }

    private static (uint Foreground, uint Background) ReadPair(
        JsonElement colors,
        string name,
        uint foreground,
        uint background,
        ref string error
    )
    {
        if (!colors.TryGetProperty(name, out JsonElement pair))
        {
            return (foreground, background);
        }

        if (pair.ValueKind != JsonValueKind.Object)
        {
            if (error.Length == 0)
            {
                error = $"{name} must be an object with fg and bg";
            }
            return (foreground, background);
        }

        foreground = ReadColor(pair, "fg", foreground, ref error);
        background = ReadColor(pair, "bg", background, ref error);
        return (foreground, background);
    }
}

/// <summary>
/// Loads themes from the portable <c>data/themes</c> directory (or the
/// <c>~/.hexal/data/themes</c> fallback) and remembers the active theme in
/// <c>~/.hexal/theme</c>.
/// </summary>
public sealed class ThemeRegistry
{
    private readonly List<Theme> _themes = new();
    private readonly Dictionary<string, Theme> _byId = new(StringComparer.OrdinalIgnoreCase);
    private readonly List<string> _errors = new();

    public Theme ActiveTheme { get; private set; } = Theme.CreateDefault();

    public IReadOnlyList<Theme> Themes => _themes;

    public IReadOnlyList<string> Errors => _errors;

    public static string DefaultThemeDirectory => HexalPaths.ThemeDirectory;

    public static string ActiveThemePath => HexalPaths.ActiveThemePath;

    public static ThemeRegistry LoadDefault(out string error)
    {
        var registry = new ThemeRegistry();
        string directory = DefaultThemeDirectory;

        try
        {
            Directory.CreateDirectory(directory);
            registry.LoadDirectory(directory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            registry._errors.Add($"cannot create theme directory '{directory}': {ex.Message}");
        }

        if (registry._themes.Count == 0)
        {
            registry.AddTheme(Theme.CreateDefault());
        }

        string activeId = ReadActiveThemeId();
        Theme? active = registry.Find(activeId);
        registry.ActiveTheme = active ?? registry._themes[0];

        error = registry._errors.Count > 0 ? registry._errors[0] : string.Empty;
        HexalPaths.Log(
            $"Themes: {directory} ({registry._themes.Count} loaded, "
                + $"{registry._errors.Count} error(s), active '{registry.ActiveTheme.Id}')"
        );
        return registry;
    }

    public void LoadDirectory(string directory)
    {
        string[] files;
        try
        {
            files = Directory.GetFiles(directory, "*.json");
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            _errors.Add($"cannot read theme directory '{directory}': {ex.Message}");
            return;
        }

        Array.Sort(files, StringComparer.OrdinalIgnoreCase);

        // Theme files are independent; parse them concurrently and merge in
        // sorted file order for stable duplicate handling.
        var parsed = new (Theme? Theme, string? Error)[files.Length];
        Parallel.For(
            0,
            files.Length,
            index =>
            {
                string file = files[index];
                try
                {
                    string text = File.ReadAllText(file);
                    if (Theme.TryParse(text, file, out Theme? theme, out string error))
                    {
                        parsed[index] = (theme, null);
                    }
                    else
                    {
                        parsed[index] = (null, error);
                    }
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or NotSupportedException
                                or ArgumentException
                    )
                {
                    parsed[index] = (null, ex.Message);
                }
            }
        );

        for (int index = 0; index < files.Length; ++index)
        {
            string file = files[index];
            (Theme? theme, string? error) = parsed[index];
            if (theme is null)
            {
                _errors.Add($"{Path.GetFileName(file)}: {error}");
                continue;
            }

            if (_byId.ContainsKey(theme.Id))
            {
                _errors.Add($"{Path.GetFileName(file)}: duplicate theme id '{theme.Id}'");
                continue;
            }

            AddTheme(theme);
        }
    }

    public Theme? Find(string id)
    {
        if (id.Length == 0)
        {
            return null;
        }

        return _byId.TryGetValue(id, out Theme? theme) ? theme : null;
    }

    public void SetActiveTheme(Theme theme)
    {
        ActiveTheme = theme;
    }

    public static string ReadActiveThemeId()
    {
        try
        {
            if (!File.Exists(ActiveThemePath))
            {
                return string.Empty;
            }

            return File.ReadAllText(ActiveThemePath).Trim();
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            return string.Empty;
        }
    }

    public static bool SaveActiveTheme(string id, out string error)
    {
        error = string.Empty;
        try
        {
            Directory.CreateDirectory(HexalPaths.HexalHomeDirectory);
            File.WriteAllText(ActiveThemePath, id + Environment.NewLine);
            return true;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            error = ex.Message;
            return false;
        }
    }

    private void AddTheme(Theme theme)
    {
        _themes.Add(theme);
        _byId[theme.Id] = theme;
    }
}
