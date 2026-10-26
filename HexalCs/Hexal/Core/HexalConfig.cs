using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Hexal.Util;

namespace Hexal.Core;

public sealed class HexalConfig
{
    public const string DefaultFileName = ".hexcfg";

    public const string DefaultCentralRepositoryUrl = "https://github.com/navid-m/hexal-plugins";

    private const string DefaultContents = """
        {
            "version": 1,
            "centralRepositoryUrl": "https://github.com/navid-m/hexal-plugins",
            "formatOnSave": false,
            "trimTrailingWhitespaceOnSave": false,
            "smartBackspace": true,
            "autoDetectIndentationOnStartup": true,
            "startupBenchmarker": false,
            "syntaxHighlightPickerPreview": false,
            "debugAdapter": true,
            "showDiffLines": true,
            "diagnosticLineHighlight": true,
            "showGitIgnored": false,
            "showModifiedIndicators": true,
            "wordWrap": false,
            "keepSidebarFocusOnOpen": true,
            "confirmQuitWithoutSaving": true,
            "searchMatchCaseEnabled": false
        }

        """;

    public string SourcePath { get; init; } = string.Empty;

    /// <summary>
    /// Git repository used by <c>hexal install-plugin</c>. Plugin folders live
    /// at the repository root.
    /// </summary>
    public string CentralRepositoryUrl { get; init; } = DefaultCentralRepositoryUrl;

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

    /// <summary>
    /// When true, the file picker preview applies the detected language's
    /// syntax highlighting. Disabled by default.
    /// </summary>
    public bool SyntaxHighlightPickerPreview { get; init; }

    /// <summary>
    /// Enables the experimental Debug Adapter Protocol integration,
    /// including the Ctrl+E breakpoint toggle and the :debug commands.
    /// </summary>
    public bool DebugAdapter { get; init; } = true;

    public bool ShowDiffLines { get; init; } = true;

    /// <summary>
    /// When true, lines containing language-server diagnostics get a muted
    /// background tint based on the most severe diagnostic on the line.
    /// </summary>
    public bool DiagnosticLineHighlight { get; init; } = true;

    public bool ShowGitIgnored { get; init; }

    /// <summary>
    /// When true, the sidebar marks Git-modified and renamed files and their
    /// parent folders with an M.
    /// </summary>
    public bool ShowModifiedIndicators { get; init; } = true;

    /// <summary>
    /// When true, long editor lines are softly wrapped at the pane width.
    /// </summary>
    public bool WordWrap { get; init; }

    /// <summary>
    /// When true, opening a file from the sidebar keeps keyboard focus in the
    /// sidebar until it is closed with Ctrl+B. When false, focus moves to the
    /// newly opened buffer.
    /// </summary>
    public bool KeepSidebarFocusOnOpen { get; init; } = true;

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

    public static string AlternatePath => Path.Combine(HexalPaths.HexalHomeDirectory, "hexcfg");

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
        long phaseStarted = StartupTrace.Timestamp();
        string path = ResolvePath();
        StartupTrace.Mark("config: resolve path", phaseStarted, path);

        if (!File.Exists(path))
        {
            error = string.Empty;
            StartupTrace.Note($"config: no config file at '{path}', using defaults");
            return new HexalConfig { SourcePath = path };
        }

        return Load(path, out error);
    }

    public static HexalConfig Load(string path, out string error)
    {
        error = string.Empty;
        long loadStarted = StartupTrace.Timestamp();

        try
        {
            long phaseStarted = StartupTrace.Timestamp();
            string json = File.ReadAllText(path);
            StartupTrace.Mark("config: read file", phaseStarted, $"{json.Length} char(s)");

            phaseStarted = StartupTrace.Timestamp();
            if (!TryParse(json, path, out HexalConfig? config, out error))
            {
                StartupTrace.Mark("config: parse", phaseStarted, error);
                return new HexalConfig { SourcePath = path };
            }
            StartupTrace.Mark("config: parse", phaseStarted);

            return config!;
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
            StartupTrace.Note($"config: load failed: {ex.Message}");
            return new HexalConfig { SourcePath = path };
        }
        finally
        {
            StartupTrace.Mark("config: load total", loadStarted);
        }
    }

    public static bool TryParse(
        string json,
        string sourcePath,
        out HexalConfig? config,
        out string error
    )
    {
        config = null;
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

            string? centralRepositoryUrl =
                ReadString(root, "centralRepositoryUrl")
                ?? ReadString(root, "central_repository_url");
            bool? formatOnSave = ReadBool(root, "formatOnSave") ?? ReadBool(root, "format_on_save");
            bool? trimTrailingWhitespaceOnSave =
                ReadBool(root, "trimTrailingWhitespaceOnSave")
                ?? ReadBool(root, "trim_trailing_whitespace_on_save");
            bool? smartBackspace =
                ReadBool(root, "smartBackspace") ?? ReadBool(root, "smart_backspace");
            bool? autoDetectIndentationOnStartup =
                ReadBool(root, "autoDetectIndentationOnStartup")
                ?? ReadBool(root, "auto_detect_indentation_on_startup");
            bool? startupBenchmarker =
                ReadBool(root, "startupBenchmarker") ?? ReadBool(root, "startup_benchmarker");
            bool? syntaxHighlightPickerPreview =
                ReadBool(root, "syntaxHighlightPickerPreview")
                ?? ReadBool(root, "syntax_highlight_picker_preview");
            bool? debugAdapter = ReadBool(root, "debugAdapter") ?? ReadBool(root, "debug_adapter");
            bool? showDiffLines =
                ReadBool(root, "showDiffLines") ?? ReadBool(root, "show_diff_lines");
            bool? diagnosticLineHighlight =
                ReadBool(root, "diagnosticLineHighlight")
                ?? ReadBool(root, "diagnostic_line_highlight");
            bool? showGitIgnored =
                ReadBool(root, "showGitIgnored") ?? ReadBool(root, "show_git_ignored");
            bool? showModifiedIndicators =
                ReadBool(root, "showModifiedIndicators")
                ?? ReadBool(root, "show_modified_indicators");
            bool? wordWrap = ReadBool(root, "wordWrap") ?? ReadBool(root, "word_wrap");
            bool? keepSidebarFocusOnOpen =
                ReadBool(root, "keepSidebarFocusOnOpen")
                ?? ReadBool(root, "keep_sidebar_focus_on_open");
            bool? confirmQuitWithoutSaving =
                ReadBool(root, "confirmQuitWithoutSaving")
                ?? ReadBool(root, "confirm_quit_without_saving");
            bool? searchMatchCaseEnabled =
                ReadBool(root, "searchMatchCaseEnabled")
                ?? ReadBool(root, "search_match_case_enabled");

            if (
                root.TryGetProperty("editor", out JsonElement editor)
                && editor.ValueKind == JsonValueKind.Object
            )
            {
                centralRepositoryUrl ??=
                    ReadString(editor, "centralRepositoryUrl")
                    ?? ReadString(editor, "central_repository_url");
                formatOnSave ??=
                    ReadBool(editor, "formatOnSave") ?? ReadBool(editor, "format_on_save");
                trimTrailingWhitespaceOnSave ??=
                    ReadBool(editor, "trimTrailingWhitespaceOnSave")
                    ?? ReadBool(editor, "trim_trailing_whitespace_on_save");
                smartBackspace ??=
                    ReadBool(editor, "smartBackspace") ?? ReadBool(editor, "smart_backspace");
                autoDetectIndentationOnStartup ??=
                    ReadBool(editor, "autoDetectIndentationOnStartup")
                    ?? ReadBool(editor, "auto_detect_indentation_on_startup");
                startupBenchmarker ??=
                    ReadBool(editor, "startupBenchmarker")
                    ?? ReadBool(editor, "startup_benchmarker");
                syntaxHighlightPickerPreview ??=
                    ReadBool(editor, "syntaxHighlightPickerPreview")
                    ?? ReadBool(editor, "syntax_highlight_picker_preview");
                debugAdapter ??=
                    ReadBool(editor, "debugAdapter") ?? ReadBool(editor, "debug_adapter");
                showDiffLines ??=
                    ReadBool(editor, "showDiffLines") ?? ReadBool(editor, "show_diff_lines");
                diagnosticLineHighlight ??=
                    ReadBool(editor, "diagnosticLineHighlight")
                    ?? ReadBool(editor, "diagnostic_line_highlight");
                showGitIgnored ??=
                    ReadBool(editor, "showGitIgnored") ?? ReadBool(editor, "show_git_ignored");
                showModifiedIndicators ??=
                    ReadBool(editor, "showModifiedIndicators")
                    ?? ReadBool(editor, "show_modified_indicators");
                wordWrap ??= ReadBool(editor, "wordWrap") ?? ReadBool(editor, "word_wrap");
                keepSidebarFocusOnOpen ??=
                    ReadBool(editor, "keepSidebarFocusOnOpen")
                    ?? ReadBool(editor, "keep_sidebar_focus_on_open");
                confirmQuitWithoutSaving ??=
                    ReadBool(editor, "confirmQuitWithoutSaving")
                    ?? ReadBool(editor, "confirm_quit_without_saving");
                searchMatchCaseEnabled ??=
                    ReadBool(editor, "searchMatchCaseEnabled")
                    ?? ReadBool(editor, "search_match_case_enabled");
            }

            config = new HexalConfig
            {
                SourcePath = sourcePath,
                CentralRepositoryUrl = string.IsNullOrWhiteSpace(centralRepositoryUrl)
                    ? DefaultCentralRepositoryUrl
                    : centralRepositoryUrl,
                FormatOnSave = formatOnSave ?? false,
                TrimTrailingWhitespaceOnSave = trimTrailingWhitespaceOnSave ?? false,
                SmartBackspace = smartBackspace ?? true,
                AutoDetectIndentationOnStartup = autoDetectIndentationOnStartup ?? true,
                StartupBenchmarker = startupBenchmarker ?? false,
                SyntaxHighlightPickerPreview = syntaxHighlightPickerPreview ?? false,
                DebugAdapter = debugAdapter ?? true,
                ShowDiffLines = showDiffLines ?? true,
                DiagnosticLineHighlight = diagnosticLineHighlight ?? true,
                ShowGitIgnored = showGitIgnored ?? false,
                ShowModifiedIndicators = showModifiedIndicators ?? true,
                WordWrap = wordWrap ?? false,
                KeepSidebarFocusOnOpen = keepSidebarFocusOnOpen ?? true,
                ConfirmQuitWithoutSaving = confirmQuitWithoutSaving ?? true,
                SearchMatchCaseEnabled = searchMatchCaseEnabled ?? false,
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

    /// <summary>
    /// Writes the persisted display toggles into the configuration file while
    /// leaving unrelated properties untouched.
    /// </summary>
    public static bool UpdateDisplaySettings(
        string path,
        bool showDiffLines,
        bool showGitIgnored,
        bool searchMatchCaseEnabled,
        out string error
    ) =>
        UpdateSettings(
            path,
            showDiffLines,
            showGitIgnored,
            searchMatchCaseEnabled,
            wordWrap: null,
            out error
        );

    public static bool UpdateWordWrap(string path, bool wordWrap, out string error) =>
        UpdateSettings(path, null, null, null, wordWrap, out error);

    private static bool UpdateSettings(
        string path,
        bool? showDiffLines,
        bool? showGitIgnored,
        bool? searchMatchCaseEnabled,
        bool? wordWrap,
        out string error
    )
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
                JsonNode? node = JsonNode.Parse(
                    json,
                    documentOptions: new JsonDocumentOptions
                    {
                        AllowTrailingCommas = true,
                        CommentHandling = JsonCommentHandling.Skip,
                    }
                );

                if (node is not JsonObject parsed)
                {
                    error = "root must be a JSON object";
                    return false;
                }

                root = parsed;
            }
            else
            {
                root = new JsonObject { ["version"] = 1, ["formatOnSave"] = false };
            }

            if (showDiffLines is { } diffLines)
            {
                root["showDiffLines"] = diffLines;
            }
            if (showGitIgnored is { } gitIgnored)
            {
                root["showGitIgnored"] = gitIgnored;
            }
            if (searchMatchCaseEnabled is { } matchCase)
            {
                root["searchMatchCaseEnabled"] = matchCase;
            }
            if (wordWrap is { } wrap)
            {
                root["wordWrap"] = wrap;
            }

            string output = root.ToJsonString(new JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText(path, output + Environment.NewLine, new UTF8Encoding(false));
            return true;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
                        or JsonException
            )
        {
            error = ex.Message;
            return false;
        }
    }

    private static string? ReadString(JsonElement element, string property)
    {
        if (
            !element.TryGetProperty(property, out JsonElement value)
            || value.ValueKind != JsonValueKind.String
        )
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
            _ => null,
        };
    }
}
