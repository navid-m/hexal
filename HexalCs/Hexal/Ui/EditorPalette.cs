using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private sealed record PaletteCommand(string Title, string Description, Action Action);

    private readonly List<PaletteCommand> _paletteCommands = new();

    private void OpenCommandPalette()
    {
        RebuildPaletteCommands();
        OpenPicker(PickerKind.Palette);
    }

    private void RebuildPaletteCommands()
    {
        _paletteCommands.Clear();

        AddPaletteCommand(
            "Open file picker",
            "Search the workspace and open a file.",
            () => OpenPicker(PickerKind.Files)
        );
        AddPaletteCommand(
            "Search across files",
            "Search file contents in the workspace.",
            () => OpenPicker(PickerKind.Search)
        );
        AddPaletteCommand(
            "Open active .hexcfg",
            "Open the active Hexal configuration, creating it with the default schema when missing.",
            OpenConfigFile
        );
        AddPaletteCommand(
            "Open Hexal log",
            "Open ~/.hexal/hexal.log to inspect startup, plugin, LSP and formatter messages.",
            OpenLogFile
        );
        AddPaletteCommand(
            "Open ~/.hexal directory",
            "Browse the Hexal home directory in the file picker.",
            OpenHexalDirectory
        );
        AddPaletteCommand(
            "Open installation directory",
            "Browse the directory containing the Hexal executable.",
            OpenInstallationDirectory
        );
        AddPaletteCommand(
            "Open themes directory",
            "Browse the active theme directory.",
            OpenThemesDirectory
        );
        AddPaletteCommand("Toggle sidebar", "Show or hide the file sidebar.", ToggleSidebar);
        AddPaletteCommand(
            "Toggle git-ignored files",
            "Show or hide files matched by .gitignore in the sidebar and pickers.",
            ToggleShowGitIgnored
        );
        AddPaletteCommand(
            "Toggle git diff lines",
            "Show or hide added, deleted and modified line markers in the gutter.",
            ToggleDiffLines
        );
        AddPaletteCommand(
            "Toggle search match case",
            "Make all searches case-sensitive or case-insensitive.",
            ToggleSearchMatchCase
        );
        AddPaletteCommand(
            "Split pane vertically",
            "Create a new pane to the right of the focused pane.",
            () => SplitPane(PaneSplitOrientation.Vertical)
        );
        AddPaletteCommand(
            "Split pane horizontally",
            "Create a new pane below the focused pane.",
            () => SplitPane(PaneSplitOrientation.Horizontal)
        );
        AddPaletteCommand(
            "Close current buffer pane",
            "Close the focused pane; use :q or :q! when only one remains.",
            CloseCurrentBuffer
        );
        AddPaletteCommand(
            "Detect document indentation",
            "Infer tabs or spaces and the indent width from the current buffer.",
            () => AutoDetectIndentation()
        );
        AddPaletteCommand(
            "Open theme selector",
            "Search and preview installed themes.",
            () => OpenPicker(PickerKind.Themes)
        );
        AddPaletteCommand(
            "Reload configuration",
            "Reload .hexcfg, grammars, themes and language servers.",
            ReloadConfiguration
        );
        AddPaletteCommand(
            "Manage plugins",
            "Enable or disable installed plugins and inspect their details.",
            OpenPluginManager
        );
        if (_benchmark is not null)
        {
            AddPaletteCommand(
                "Show startup benchmark",
                "Inspect startup and plugin load/unload timings.",
                OpenStartupBenchmark
            );
        }
        AddPaletteCommand(
            "Format document",
            "Format the current buffer with its language formatter.",
            FormatCurrentDocument
        );
        AddPaletteCommand(
            "Clean trailing whitespace",
            "Remove trailing spaces and tabs from every line.",
            CleanWhitespace
        );
        AddPaletteCommand(
            "Save file",
            "Write the current buffer to disk.",
            () => StartSave(path: null, quitAfter: false)
        );
        AddPaletteCommand(
            "Save and quit",
            "Write the current buffer, then quit.",
            () => StartSave(path: null, quitAfter: true)
        );
        AddPaletteCommand(
            "Show key help",
            "Show a short reminder of the available keys.",
            ShowHelpReminder
        );
        AddPaletteCommand(
            "About Hexal",
            "Show the Hexal title, author and ASCII branding.",
            OpenAbout
        );
        AddPaletteCommand("Show LSP hover", "Show hover information at the cursor.", ShowHover);
        AddPaletteCommand(
            "Show code actions",
            "List language-server code actions.",
            ShowCodeActions
        );
        AddPaletteCommand(
            "Show document diagnostics",
            "List diagnostics for the current file.",
            () => ShowProblems(workspace: false)
        );
        AddPaletteCommand(
            "Show workspace diagnostics",
            "List diagnostics for the whole workspace.",
            () => ShowProblems(workspace: true)
        );
        AddPaletteCommand(
            "Show LSP debug log",
            "Inspect recent language-server traffic and client state.",
            OpenLspDebug
        );
        AddPaletteCommand(
            "Show debug variables",
            "Show scopes, local/global variables and watch expressions while stopped.",
            ToggleDebugVariables
        );
        AddPaletteCommand(
            "Quit",
            "Quit Hexal; unsaved changes are confirmed first.",
            () => RequestQuit(force: false)
        );

        if (_plugins is null)
        {
            return;
        }

        foreach (
            string name in _plugins.CommandNames.OrderBy(
                command => command,
                StringComparer.OrdinalIgnoreCase
            )
        )
        {
            string description = _plugins.TryGetCommandDescription(name, out string text)
                ? text
                : $"Run the plugin command :{name}";
            string command = name;
            AddPaletteCommand(
                $"Plugin: {name}",
                description,
                () => ExecutePluginPaletteCommand(command)
            );
        }
    }

    private void AddPaletteCommand(string title, string description, Action action)
    {
        _paletteCommands.Add(new PaletteCommand(title, description, action));
    }

    private static string ThemeDetail(Theme theme)
    {
        var parts = new List<string>();
        if (theme.Description.Length > 0)
        {
            parts.Add(theme.Description);
        }
        if (theme.Author.Length > 0)
        {
            parts.Add($"by {theme.Author}");
        }
        if (theme.License.Length > 0)
        {
            parts.Add(theme.License);
        }
        return string.Join("  ·  ", parts);
    }

    private void ExecutePluginPaletteCommand(string name)
    {
        if (_plugins is null || !_plugins.HasCommand(name))
        {
            SetError($"plugin command ':{name}' is not available");
            return;
        }

        if (!_plugins.TryExecuteCommand(name, new List<string>(), out string error))
        {
            SetError($"plugin command ':{name}' failed");
        }
        else if (error.Length > 0)
        {
            SetError(error);
        }
    }

    private void PopulatePaletteItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        var scored = new List<(int Score, PickerItem Item)>();
        for (int i = 0; i < _paletteCommands.Count; ++i)
        {
            PaletteCommand command = _paletteCommands[i];
            string searchText = $"{command.Title} {command.Description}";
            int score = FuzzyScore(searchText, query);
            if (score == int.MaxValue)
            {
                continue;
            }

            scored.Add(
                (
                    score,
                    new PickerItem
                    {
                        Display = command.Title,
                        Detail = command.Description,
                        Value = i,
                    }
                )
            );
        }

        scored.Sort(
            (left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(
                        left.Item.Display,
                        right.Item.Display,
                        StringComparison.OrdinalIgnoreCase
                    );
            }
        );

        foreach ((int _, PickerItem item) in scored)
        {
            _pickerItems.Add(item);
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
                ? "No matching commands"
                : $"{_pickerItems.Count} command{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        UpdatePickerDetailStatus();
    }

    private void PopulateThemeItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        var scored = new List<(int Score, PickerItem Item)>();
        for (int i = 0; i < _themes.Themes.Count; ++i)
        {
            Theme theme = _themes.Themes[i];
            string detail = ThemeDetail(theme);
            string searchText = $"{theme.Name} {theme.Description} {theme.Author} " + theme.License;
            int score = FuzzyScore(searchText, query);
            if (score == int.MaxValue)
            {
                continue;
            }

            scored.Add(
                (
                    score,
                    new PickerItem
                    {
                        Display = theme.Name,
                        Detail = detail,
                        Key = theme.Id,
                    }
                )
            );
        }

        scored.Sort(
            (left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(
                        left.Item.Display,
                        right.Item.Display,
                        StringComparison.OrdinalIgnoreCase
                    );
            }
        );

        foreach ((int _, PickerItem item) in scored)
        {
            _pickerItems.Add(item);
        }

        _pickerSelected = 0;
        _pickerTop = 0;

        if (_pickerQuery.Length == 0 && _themeBeforePicker.Length > 0)
        {
            for (int i = 0; i < _pickerItems.Count; ++i)
            {
                if (
                    string.Equals(
                        _pickerItems[i].Key,
                        _themeBeforePicker,
                        StringComparison.OrdinalIgnoreCase
                    )
                )
                {
                    _pickerSelected = i;
                    break;
                }
            }
        }

        int rows = PickerLayoutRows();
        if (_pickerSelected >= rows)
        {
            _pickerTop = _pickerSelected - rows + 1;
        }

        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
                ? "No matching themes"
                : $"{_pickerItems.Count} theme{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        UpdatePickerDetailStatus();
        PreviewSelectedTheme();
    }

    private void SetActiveTheme(string id, bool persist, bool announce)
    {
        Theme? theme = _themes.Find(id);
        if (theme is null)
        {
            if (announce)
            {
                SetError($"unknown theme '{id}'");
            }
            return;
        }

        _theme = theme;
        _themes.SetActiveTheme(theme);
        _forceFullRedraw = true;

        if (persist && !ThemeRegistry.SaveActiveTheme(theme.Id, out string error))
        {
            SetError($"theme: {error}");
        }
        else if (announce)
        {
            SetMessage($"Theme: {theme.Name}");
        }
    }

    private void OpenInstallationDirectory()
    {
        string directory = HexalPaths.BinaryDirectory;
        if (!Directory.Exists(directory))
        {
            SetError($"installation directory not found: {directory}");
            return;
        }

        OpenPicker(PickerKind.Files, directory, "Open installation directory");
    }

    private void OpenHexalDirectory()
    {
        try
        {
            Directory.CreateDirectory(HexalPaths.HexalHomeDirectory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            SetError($"cannot open ~/.hexal: {ex.Message}");
            return;
        }

        OpenPicker(PickerKind.Files, HexalPaths.HexalHomeDirectory, "Open ~/.hexal");
    }

    private void OpenThemesDirectory()
    {
        string directory = HexalPaths.ThemeDirectory;
        try
        {
            Directory.CreateDirectory(directory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            SetError($"cannot open themes directory: {ex.Message}");
            return;
        }

        OpenPicker(PickerKind.Files, directory, "Open themes");
    }

    private void ShowHelpReminder() => OpenHelp();
}
