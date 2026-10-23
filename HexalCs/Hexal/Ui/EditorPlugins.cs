using Hexal.Platform;
using Hexal.Scripting;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _pluginManagerVisible;
    private int _pluginManagerSelected;
    private int _pluginManagerTop;
    private readonly List<PluginInfo> _pluginManagerItems = new();

    private void OpenPluginManager()
    {
        ClearMessage();
        RefreshPluginManagerItems(keepSelection: false);
        _pluginManagerVisible = true;
        _forceFullRedraw = true;
    }

    private void ClosePluginManager()
    {
        _pluginManagerVisible = false;
        _pluginManagerTop = 0;
        _forceFullRedraw = true;
    }

    private void RefreshPluginManagerItems(bool keepSelection)
    {
        string selectedId =
            keepSelection
            && _pluginManagerSelected >= 0
            && _pluginManagerSelected < _pluginManagerItems.Count
                ? _pluginManagerItems[_pluginManagerSelected].Id
                : string.Empty;

        _pluginManagerItems.Clear();
        if (_plugins is not null)
        {
            _pluginManagerItems.AddRange(_plugins.Plugins);
        }

        _pluginManagerSelected = 0;
        if (selectedId.Length > 0)
        {
            for (int i = 0; i < _pluginManagerItems.Count; ++i)
            {
                if (string.Equals(_pluginManagerItems[i].Id, selectedId, StringComparison.Ordinal))
                {
                    _pluginManagerSelected = i;
                    break;
                }
            }
        }

        _pluginManagerSelected = Math.Clamp(
            _pluginManagerSelected,
            0,
            Math.Max(0, _pluginManagerItems.Count - 1)
        );
        int rows = PluginManagerListRows();
        _pluginManagerTop = Math.Clamp(
            _pluginManagerTop,
            0,
            Math.Max(0, _pluginManagerItems.Count - rows)
        );
    }

    private void HandlePluginManagerInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
                ClosePluginManager();
                return;
            case Key.Up:
            case Key.ScrollUp:
                MovePluginManagerSelection(-1);
                return;
            case Key.Down:
            case Key.ScrollDown:
                MovePluginManagerSelection(1);
                return;
            case Key.PageUp:
                MovePluginManagerSelection(-PluginManagerListRows());
                return;
            case Key.PageDown:
                MovePluginManagerSelection(PluginManagerListRows());
                return;
            case Key.Home:
                _pluginManagerSelected = 0;
                EnsurePluginManagerSelectionVisible();
                return;
            case Key.End:
                _pluginManagerSelected = Math.Max(0, _pluginManagerItems.Count - 1);
                EnsurePluginManagerSelectionVisible();
                return;
            case Key.Enter:
                ToggleSelectedPlugin();
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl || inputEvent.Alt)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
            case 'q':
            case 'Q':
                ClosePluginManager();
                return;
            case 'k':
            case 'K':
                MovePluginManagerSelection(-1);
                return;
            case 'j':
            case 'J':
                MovePluginManagerSelection(1);
                return;
            case ' ':
            case 'e':
            case 'E':
            case 't':
            case 'T':
                ToggleSelectedPlugin();
                return;
            case 's':
            case 'S':
                OpenSelectedPluginSource();
                return;
            case 'r':
                ReloadSelectedPlugin();
                return;
            case 'R':
                ReloadAllPlugins();
                return;
        }
    }

    private PluginInfo? SelectedPluginManagerItem() =>
        _pluginManagerSelected >= 0 && _pluginManagerSelected < _pluginManagerItems.Count
            ? _pluginManagerItems[_pluginManagerSelected]
            : null;

    private void MovePluginManagerSelection(int delta)
    {
        if (_pluginManagerItems.Count == 0)
        {
            return;
        }

        _pluginManagerSelected = Math.Clamp(
            _pluginManagerSelected + delta,
            0,
            _pluginManagerItems.Count - 1
        );
        EnsurePluginManagerSelectionVisible();
    }

    private void EnsurePluginManagerSelectionVisible()
    {
        int rows = PluginManagerListRows();
        if (_pluginManagerSelected < _pluginManagerTop)
        {
            _pluginManagerTop = _pluginManagerSelected;
        }
        else if (_pluginManagerSelected >= _pluginManagerTop + rows)
        {
            _pluginManagerTop = _pluginManagerSelected - rows + 1;
        }

        _pluginManagerTop = Math.Clamp(
            _pluginManagerTop,
            0,
            Math.Max(0, _pluginManagerItems.Count - rows)
        );
    }

    private void ToggleSelectedPlugin()
    {
        if (
            _plugins is null
            || _pluginManagerSelected < 0
            || _pluginManagerSelected >= _pluginManagerItems.Count
        )
        {
            return;
        }

        PluginInfo plugin = _pluginManagerItems[_pluginManagerSelected];
        bool enable = !plugin.Enabled;
        if (!_plugins.SetPluginEnabled(plugin.Id, enable, out string error))
        {
            SetError(error);
            RefreshPluginManagerItems(keepSelection: true);
            _forceFullRedraw = true;
            return;
        }

        if (enable)
        {
            _plugins.FireEvent(
                "editor_started",
                new Dictionary<string, object?> { ["api_version"] = 1 }
            );
            _plugins.FireEvent("buffer_opened", CurrentBufferData());
            _plugins.ActivateLuaLsp();
        }

        SetMessage($"Plugin {plugin.Id} {(enable ? "enabled" : "disabled")}.");
        RefreshPluginManagerItems(keepSelection: true);
        _forceFullRedraw = true;
    }

    private void OpenSelectedPluginSource()
    {
        PluginInfo? plugin = SelectedPluginManagerItem();
        if (plugin is null)
        {
            return;
        }

        string directory = plugin.Directory;
        if (directory.Length == 0 && plugin.EntryPath.Length > 0)
        {
            directory = Path.GetDirectoryName(plugin.EntryPath) ?? string.Empty;
        }

        if (directory.Length == 0)
        {
            SetError($"plugin directory is unavailable for {plugin.Id}");
            return;
        }

        string fullDirectory;
        try
        {
            fullDirectory = Path.GetFullPath(directory);
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            SetError($"invalid plugin directory: {ex.Message}");
            return;
        }

        if (!Directory.Exists(fullDirectory))
        {
            SetError($"plugin directory not found: {fullDirectory}");
            return;
        }

        string entryPath =
            plugin.EntryPath.Length > 0
                ? plugin.EntryPath
                : Path.Combine(fullDirectory, "init.lua");

        if (!MakeDocumentPathsAbsolute())
        {
            return;
        }

        _workspaceRoot = fullDirectory;
        try
        {
            Directory.SetCurrentDirectory(fullDirectory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            SetError($"cannot change working directory: {ex.Message}");
        }

        if (_sidebarVisible)
        {
            ExpandSidebarToPath(entryPath);
            RefreshSidebar(entryPath);
        }

        ClosePluginManager();
        RequestOpen(entryPath, force: false);
        if (_prompt == PromptKind.None)
        {
            SetMessage($"Working directory: {fullDirectory}");
        }
    }

    private bool MakeDocumentPathsAbsolute()
    {
        try
        {
            if (_diffBaselinePath.Length > 0 && !Path.IsPathRooted(_diffBaselinePath))
            {
                _diffBaselinePath = Path.GetFullPath(_diffBaselinePath);
            }

            foreach (EditorPane pane in AllPanes())
            {
                if (pane.Document.Path is { } path && !Path.IsPathRooted(path))
                {
                    pane.Document.ChangePath(Path.GetFullPath(path));
                }

                if (pane.DiffBaselinePath.Length > 0 && !Path.IsPathRooted(pane.DiffBaselinePath))
                {
                    pane.DiffBaselinePath = Path.GetFullPath(pane.DiffBaselinePath);
                }
            }
            return true;
        }
        catch (Exception ex)
            when (ex
                    is ArgumentException
                        or NotSupportedException
                        or PathTooLongException
                        or IOException
                        or UnauthorizedAccessException
            )
        {
            SetError($"cannot preserve document paths: {ex.Message}");
            return false;
        }
    }

    private void ReloadSelectedPlugin()
    {
        PluginInfo? plugin = SelectedPluginManagerItem();
        if (_plugins is null || plugin is null)
        {
            return;
        }

        bool reloaded = _plugins.ReloadPlugin(plugin.Id, out string error);
        RefreshPluginManagerItems(keepSelection: true);
        _forceFullRedraw = true;

        if (!reloaded)
        {
            SetError(error);
            return;
        }

        PluginInfo? current = SelectedPluginManagerItem();
        if (current is not null && current.Running)
        {
            FirePluginStartedAndOpened();
        }
        SetMessage($"Reloaded plugin {plugin.Id}.");
    }

    private void ReloadAllPlugins()
    {
        if (_plugins is null)
        {
            return;
        }

        bool reloaded = _plugins.ReloadAll(out string error);
        RefreshPluginManagerItems(keepSelection: true);
        _forceFullRedraw = true;

        FirePluginStartedAndOpened();
        if (!reloaded)
        {
            SetError(error);
            return;
        }

        SetMessage("Reloaded all plugins.");
    }

    private void FirePluginStartedAndOpened()
    {
        _plugins?.FireEvent(
            "editor_started",
            new Dictionary<string, object?> { ["api_version"] = 1 }
        );
        _plugins?.FireEvent("buffer_opened", CurrentBufferData());
        _plugins?.ActivateLuaLsp();
    }

    private int PluginManagerHeight()
    {
        int maximum = Math.Max(5, _rows - 2);
        return Math.Min(24, maximum);
    }

    private int PluginManagerWidth()
    {
        int maximum = Math.Max(20, _columns - 4);
        return Math.Min(100, maximum);
    }

    private int PluginManagerListRows() => Math.Max(1, PluginManagerHeight() - 3);

    private void RenderPluginManager()
    {
        if (!_pluginManagerVisible)
        {
            return;
        }

        int width = PluginManagerWidth();
        int height = PluginManagerHeight();
        if (width < 24 || height < 5 || width > _columns || height > _rows)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (_rows - height) / 2);
        DrawBox(x, y, width, height, " Plugins ");

        int contentRows = Math.Max(1, height - 3);
        int listWidth = Math.Clamp(width / 3, 14, 34);
        int detailX = x + 2 + listWidth;
        int detailWidth = Math.Max(0, width - listWidth - 4);

        for (int row = 0; row < contentRows; ++row)
        {
            _screen.Put(x + 1 + listWidth, y + 1 + row, (uint)'│', _theme.BorderStyle);
        }

        if (_pluginManagerItems.Count == 0)
        {
            _screen.PutString(
                x + 2,
                y + 1,
                "No plugins discovered.",
                _theme.MutedStyle,
                detailWidth
            );
        }
        else
        {
            _pluginManagerSelected = Math.Clamp(
                _pluginManagerSelected,
                0,
                _pluginManagerItems.Count - 1
            );
            int maxTop = Math.Max(0, _pluginManagerItems.Count - contentRows);
            _pluginManagerTop = Math.Clamp(_pluginManagerTop, 0, maxTop);

            for (int row = 0; row < contentRows; ++row)
            {
                int index = _pluginManagerTop + row;
                if (index >= _pluginManagerItems.Count)
                {
                    break;
                }

                PluginInfo plugin = _pluginManagerItems[index];
                string label = $"{plugin.Name} [{(plugin.Enabled ? "on" : "off")}]";
                Style style =
                    index == _pluginManagerSelected ? _theme.SelectedItemStyle
                    : plugin.Enabled ? _theme.TextStyle
                    : _theme.MutedStyle;
                _screen.PutString(x + 2, y + 1 + row, label, style, Math.Max(0, listWidth - 1));
            }

            PluginInfo selected = _pluginManagerItems[_pluginManagerSelected];
            int detailRow = y + 1;
            int bottom = y + height - 2;

            foreach ((string text, Style style) in PluginDetailLines(selected, detailWidth))
            {
                if (detailRow >= bottom)
                {
                    break;
                }

                _screen.PutString(detailX, detailRow, text, style, detailWidth);
                ++detailRow;
            }
        }

        _screen.PutString(
            x + 2,
            y + height - 2,
            "j/k move · Enter toggle · s source · r reload · R all · Esc close",
            _theme.MutedStyle,
            width - 4
        );
    }

    private IEnumerable<(string Text, Style Style)> PluginDetailLines(PluginInfo plugin, int width)
    {
        var style = _theme.TextStyle;
        yield return ($"Name: {plugin.Name}", _theme.AccentStyle);
        yield return ($"Id: {plugin.Id}", style);
        if (plugin.Version.Length > 0)
        {
            yield return ($"Version: {plugin.Version}", style);
        }

        string status =
            plugin.Deferred ? "Status: enabled (loads when its language is opened)"
            : plugin.Enabled ? "Status: enabled"
            : "Status: disabled";
        yield return (status, plugin.Enabled ? _theme.TextStyle : _theme.MutedStyle);
        if (plugin.Author.Length > 0)
        {
            yield return ($"Author: {plugin.Author}", style);
        }
        if (plugin.License.Length > 0)
        {
            yield return ($"License: {plugin.License}", style);
        }

        if (plugin.Description.Length > 0)
        {
            foreach (
                string line in WrapText($"Description: {plugin.Description}", Math.Max(1, width))
            )
            {
                yield return (line, style);
            }
        }

        string commands = plugin.Commands.Count == 0 ? "none" : string.Join(", ", plugin.Commands);
        foreach (string line in WrapText($"Commands: {commands}", Math.Max(1, width)))
        {
            yield return (line, style);
        }

        if (plugin.Error.Length > 0)
        {
            foreach (string line in WrapText($"Error: {plugin.Error}", Math.Max(1, width)))
            {
                yield return (line, _theme.ErrorStyle);
            }
        }

        if (plugin.EntryPath.Length > 0)
        {
            foreach (string line in WrapText($"Entry: {plugin.EntryPath}", Math.Max(1, width)))
            {
                yield return (line, _theme.MutedStyle);
            }
        }

        if (plugin.ManifestPath.Length > 0)
        {
            foreach (
                string line in WrapText($"Manifest: {plugin.ManifestPath}", Math.Max(1, width))
            )
            {
                yield return (line, _theme.MutedStyle);
            }
        }
    }
}
