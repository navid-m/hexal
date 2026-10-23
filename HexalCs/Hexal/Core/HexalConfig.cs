using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Hexal.Util;

namespace Hexal.Core;

public sealed class HexalConfig
{
    public const string DefaultFileName = ".hexcfg";

    public const string DefaultCentralRepositoryUrl =
        "https://github.com/navid-m/hexal-plugins";

    private const string DefaultContents = """
        {
            "version": 1,
            "centralRepositoryUrl": "https://github.com/navid-m/hexal-plugins",
            "formatOnSave": false,
            "trimTrailingWhitespaceOnSave": false,
            "smartBackspace": true,
            "autoDetectIndentationOnStartup": true,
            "startupBenchmarker": false,
            "showDiffLines": true,
            "showGitIgnored": false,
            "confirmQuitWithoutSaving": true,
            "searchMatchCaseEnabled": false
        }

        """;

    public string SourcePath { get; init; } = string.Empty;

    /// <summary>
    /// Git repository used by <c>hexal install-plugin</c>. Plugin folders live
    /// at the repository root.
    /// </summary>
    public string CentralRepositoryUrl { get; init; } =
        DefaultCentralRepositoryUrl;

    public bool FormatOnSave { get; init; }

    /// <summary>
    /// When true, trailing spaces and tabs are removed from every line before
    /// a save writes the buffer to disk.
    /// </summary>
    public bool TrimTrailingWhitespaceOnSave { get; init; }

    /// <summary>
    /// When true, Backspace removes the whole run of indentation when only
    /// whitespace precedes the cursor on the line.
    /// </summary>
    public bool SmartBackspace { get; init; } = true;

    /// <summary>
    /// When true, opening a non-empty buffer infers its indentation style and
    /// width instead of relying on the language defaults.
    /// </summary>
    public bool AutoDetectIndentationOnStartup { get; init; } = true;

    /// <summary>
    /// When true, records startup, plugin and shutdown timings and exposes the
    /// benchmark modal through the command palette.
    /// </summary>
    public bool StartupBenchmarker { get; init; }

    public bool ShowDiffLines { get; init; } = true;

    public bool ShowGitIgnored { get; init; }

    /// <summary>
    /// When true, file-name filtering, in-file find and cross-file search all
    /// match case exactly. When false, matching is case-insensitive.
    /// </summary>
    public bool SearchMatchCaseEnabled { get; init; }

    /// <summary>
    /// When true, quitting with unsaved changes asks for confirmation. When
    /// false, the quit is refused and the user must run <c>:q!</c>.
    /// </summary>
    public bool ConfirmQuitWithoutSaving { get; init; } = true;

    public static string DefaultPath =>
        Path.Combine(HexalPaths.HexalHomeDirectory, DefaultFileName);

    public static string AlternatePath =>
        Path.Combine(HexalPaths.HexalHomeDirectory, "hexcfg");

    public static string ResolvePath()
    {
        string primary = DefaultPath;
        if (File.Exists(primary))
        {
            return primary;
        }

        string alternate = AlternatePath;
        return File.Exists(alternate) ? alternate : primary;
    }

    public static HexalConfig LoadDefault(out string error)
    {
        string path = ResolvePath();
        if (!File.Exists(path))
        {
            error = string.Empty;
            return new HexalConfig { SourcePath = path };
        }

        return Load(path, out error);
    }

    public static HexalConfig Load(string path, out string error)
    {
        error = string.Empty;

        try
        {
            string json = File.ReadAllText(path);
            if (!TryParse(json, path, out HexalConfig? config, out error))
            {
                return new HexalConfig { SourcePath = path };
            }

            return config!;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = ex.Message;
            return new HexalConfig { SourcePath = path };
        }
    }

    public static bool TryParse(string json, string sourcePath,
        out HexalConfig? config, out string error)
    {
        config = null;
        error = string.Empty;

        try
        {
            using JsonDocument document = JsonDocument.Parse(json,
                new JsonDocumentOptions
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

            string? centralRepositoryUrl =
                ReadString(root, "centralRepositoryUrl") ??
                ReadString(root, "central_repository_url");
            bool? formatOnSave = ReadBool(root, "formatOnSave") ??
                                 ReadBool(root, "format_on_save");
            bool? trimTrailingWhitespaceOnSave =
                ReadBool(root, "trimTrailingWhitespaceOnSave") ??
                ReadBool(root, "trim_trailing_whitespace_on_save");
            bool? smartBackspace = ReadBool(root, "smartBackspace") ??
                                   ReadBool(root, "smart_backspace");
            bool? autoDetectIndentationOnStartup =
                ReadBool(root, "autoDetectIndentationOnStartup") ??
                ReadBool(root, "auto_detect_indentation_on_startup");
            bool? startupBenchmarker = ReadBool(root, "startupBenchmarker") ??
                                       ReadBool(root, "startup_benchmarker");
            bool? showDiffLines = ReadBool(root, "showDiffLines") ??
                                  ReadBool(root, "show_diff_lines");
            bool? showGitIgnored = ReadBool(root, "showGitIgnored") ??
                                   ReadBool(root, "show_git_ignored");
            bool? confirmQuitWithoutSaving =
                ReadBool(root, "confirmQuitWithoutSaving") ??
                ReadBool(root, "confirm_quit_without_saving");
            bool? searchMatchCaseEnabled =
                ReadBool(root, "searchMatchCaseEnabled") ??
                ReadBool(root, "search_match_case_enabled");

            if (root.TryGetProperty("editor", out JsonElement editor) &&
                editor.ValueKind == JsonValueKind.Object)
            {
                centralRepositoryUrl ??=
                    ReadString(editor, "centralRepositoryUrl") ??
                    ReadString(editor, "central_repository_url");
                formatOnSave ??= ReadBool(editor, "formatOnSave") ??
                                 ReadBool(editor, "format_on_save");
                trimTrailingWhitespaceOnSave ??=
                    ReadBool(editor, "trimTrailingWhitespaceOnSave") ??
                    ReadBool(editor, "trim_trailing_whitespace_on_save");
                smartBackspace ??= ReadBool(editor, "smartBackspace") ??
                                   ReadBool(editor, "smart_backspace");
                autoDetectIndentationOnStartup ??=
                    ReadBool(editor, "autoDetectIndentationOnStartup") ??
                    ReadBool(editor, "auto_detect_indentation_on_startup");
                startupBenchmarker ??=
                    ReadBool(editor, "startupBenchmarker") ??
                    ReadBool(editor, "startup_benchmarker");
                showDiffLines ??= ReadBool(editor, "showDiffLines") ??
                                  ReadBool(editor, "show_diff_lines");
                showGitIgnored ??= ReadBool(editor, "showGitIgnored") ??
                                   ReadBool(editor, "show_git_ignored");
                confirmQuitWithoutSaving ??=
                    ReadBool(editor, "confirmQuitWithoutSaving") ??
                    ReadBool(editor, "confirm_quit_without_saving");
                searchMatchCaseEnabled ??=
                    ReadBool(editor, "searchMatchCaseEnabled") ??
                    ReadBool(editor, "search_match_case_enabled");
            }

            config = new HexalConfig
            {
                SourcePath = sourcePath,
                CentralRepositoryUrl = string.IsNullOrWhiteSpace(centralRepositoryUrl)
                    ? DefaultCentralRepositoryUrl
                    : centralRepositoryUrl,
                FormatOnSave = formatOnSave ?? false,
                TrimTrailingWhitespaceOnSave =
                    trimTrailingWhitespaceOnSave ?? false,
                SmartBackspace = smartBackspace ?? true,
                AutoDetectIndentationOnStartup =
                    autoDetectIndentationOnStartup ?? true,
                StartupBenchmarker = startupBenchmarker ?? false,
                ShowDiffLines = showDiffLines ?? true,
                ShowGitIgnored = showGitIgnored ?? false,
                ConfirmQuitWithoutSaving = confirmQuitWithoutSaving ?? true,
                SearchMatchCaseEnabled = searchMatchCaseEnabled ?? false
            };
            return true;
        }
        catch (JsonException ex)
        {
            error = $"invalid JSON: {ex.Message}";
            return false;
        }
    }

    public static bool EnsureDefaultFile(out string path, out string error)
    {
        path = ResolvePath();
        error = string.Empty;

        try
        {
            Directory.CreateDirectory(HexalPaths.HexalHomeDirectory);
            if (!File.Exists(path))
            {
                File.WriteAllText(path, DefaultContents, new UTF8Encoding(false));
            }
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = ex.Message;
            return false;
        }
    }

    /// <summary>
    /// Writes the persisted display toggles into the configuration file while
    /// leaving unrelated properties untouched.
    /// </summary>
    public static bool UpdateDisplaySettings(string path, bool showDiffLines,
        bool showGitIgnored, bool searchMatchCaseEnabled, out string error)
    {
        error = string.Empty;

        try
        {
            string? directory = Path.GetDirectoryName(path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            JsonObject root;
            if (File.Exists(path))
            {
                string json = File.ReadAllText(path);
                JsonNode? node = JsonNode.Parse(json, documentOptions: new JsonDocumentOptions
                {
                    AllowTrailingCommas = true,
                    CommentHandling = JsonCommentHandling.Skip
                });

                if (node is not JsonObject parsed)
                {
                    error = "root must be a JSON object";
                    return false;
                }

                root = parsed;
            }
            else
            {
                root = new JsonObject
                {
                    ["version"] = 1,
                    ["formatOnSave"] = false
                };
            }

            root["showDiffLines"] = showDiffLines;
            root["showGitIgnored"] = showGitIgnored;
            root["searchMatchCaseEnabled"] = searchMatchCaseEnabled;

            string output = root.ToJsonString(new JsonSerializerOptions
            {
                WriteIndented = true
            });
            File.WriteAllText(path, output + Environment.NewLine,
                new UTF8Encoding(false));
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   JsonException)
        {
            error = ex.Message;
            return false;
        }
    }

    private static string? ReadString(JsonElement element, string property)
    {
        if (!element.TryGetProperty(property, out JsonElement value) ||
            value.ValueKind != JsonValueKind.String)
        {
            return null;
        }

        return value.GetString();
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
}
