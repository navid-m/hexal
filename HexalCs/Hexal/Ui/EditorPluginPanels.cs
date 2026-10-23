using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _pluginPanelVisible;
    private int _pluginPanelId;
    private int _nextPluginPanelId;
    private string _pluginPanelPluginId = string.Empty;
    private string _pluginPanelTitle = string.Empty;
    private string _pluginPanelFooter = string.Empty;
    private readonly List<PluginPanelLine> _pluginPanelLines = new();
    private long _pluginPanelSelected;
    private int _pluginPanelTop;
    private bool _pluginPanelSelectable = true;
    private Func<string, long, bool>? _pluginPanelOnKey;

    public int OpenPluginPanel(PluginPanelOptions options,
        Func<string, long, bool>? onKey, string pluginId)
    {
        _pluginPanelId = ++_nextPluginPanelId;
        _pluginPanelPluginId = pluginId;
        _pluginPanelTitle = options.Title;
        _pluginPanelFooter = options.Footer;
        _pluginPanelLines.Clear();
        _pluginPanelLines.AddRange(options.Lines);
        _pluginPanelSelectable = options.Selectable;
        _pluginPanelSelected = FindSelectableIndex(options.Selected);
        _pluginPanelTop = 0;
        _pluginPanelOnKey = onKey;
        _pluginPanelVisible = true;
        EnsurePluginPanelSelectionVisible();
        _forceFullRedraw = true;
        return _pluginPanelId;
    }

    public void SetPluginPanelLines(int panelId,
        IReadOnlyList<PluginPanelLine> lines)
    {
        if (!PluginPanelMatches(panelId))
        {
            return;
        }

        _pluginPanelLines.Clear();
        _pluginPanelLines.AddRange(lines);
        _pluginPanelTop = 0;
        _pluginPanelSelected = FindSelectableIndex(_pluginPanelSelected);
        EnsurePluginPanelSelectionVisible();
        _forceFullRedraw = true;
    }

    public void SetPluginPanelTitle(int panelId, string title)
    {
        if (!PluginPanelMatches(panelId))
        {
            return;
        }

        _pluginPanelTitle = title;
        _forceFullRedraw = true;
    }

    public void SetPluginPanelFooter(int panelId, string footer)
    {
        if (!PluginPanelMatches(panelId))
        {
            return;
        }

        _pluginPanelFooter = footer;
        _forceFullRedraw = true;
    }

    public void SetPluginPanelSelected(int panelId, long selected)
    {
        if (!PluginPanelMatches(panelId))
        {
            return;
        }

        _pluginPanelSelected = FindSelectableIndex(selected);
        EnsurePluginPanelSelectionVisible();
        _forceFullRedraw = true;
    }

    public void SetPluginPanelSelectable(int panelId, bool selectable)
    {
        if (!PluginPanelMatches(panelId))
        {
            return;
        }

        _pluginPanelSelectable = selectable;
        if (selectable)
        {
            _pluginPanelSelected =
                FindSelectableIndex(_pluginPanelSelected);
            EnsurePluginPanelSelectionVisible();
        }
        _forceFullRedraw = true;
    }

    public void ClosePluginPanel(int panelId)
    {
        if (PluginPanelMatches(panelId))
        {
            ClosePluginPanel();
        }
    }

    public void ClosePluginPanels(string pluginId)
    {
        if (_pluginPanelVisible &&
            string.Equals(_pluginPanelPluginId, pluginId, StringComparison.Ordinal))
        {
            ClosePluginPanel();
        }
    }

    private bool PluginPanelMatches(int panelId)
        => _pluginPanelVisible && panelId == _pluginPanelId;

    private void ClosePluginPanel()
    {
        _pluginPanelVisible = false;
        _pluginPanelPluginId = string.Empty;
        _pluginPanelTitle = string.Empty;
        _pluginPanelFooter = string.Empty;
        _pluginPanelLines.Clear();
        _pluginPanelSelected = 0;
        _pluginPanelTop = 0;
        _pluginPanelSelectable = true;
        _pluginPanelOnKey = null;
        _forceFullRedraw = true;
    }

    private bool PluginPanelLineSelectable(int index)
        => index >= 0 && index < _pluginPanelLines.Count &&
           _pluginPanelLines[index].Selectable;

    private long FindSelectableIndex(long preferred)
    {
        int count = _pluginPanelLines.Count;
        if (count == 0)
        {
            return 0;
        }

        int start = (int)Math.Clamp(preferred, 0, count - 1);
        if (!_pluginPanelSelectable)
        {
            return start;
        }

        for (int index = start; index < count; ++index)
        {
            if (_pluginPanelLines[index].Selectable)
            {
                return index;
            }
        }
        for (int index = start - 1; index >= 0; --index)
        {
            if (_pluginPanelLines[index].Selectable)
            {
                return index;
            }
        }
        return start;
    }

    private void HandlePluginPanelInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
                HandlePluginPanelKey("esc", closeIfUnhandled: true);
                return;
            case Key.Enter:
                HandlePluginPanelKey("enter");
                return;
            case Key.Backspace:
                HandlePluginPanelKey("backspace", closeIfUnhandled: true);
                return;
            case Key.Tab:
                HandlePluginPanelKey("tab");
                return;
            case Key.Up:
            case Key.ScrollUp:
                MovePluginPanelSelection(inputEvent.Key == Key.ScrollUp
                    ? -MouseScrollLines
                    : -1);
                return;
            case Key.Down:
            case Key.ScrollDown:
                MovePluginPanelSelection(inputEvent.Key == Key.ScrollDown
                    ? MouseScrollLines
                    : 1);
                return;
            case Key.PageUp:
                MovePluginPanelSelection(-PluginPanelContentRows());
                return;
            case Key.PageDown:
                MovePluginPanelSelection(PluginPanelContentRows());
                return;
            case Key.Home:
                MovePluginPanelToEdge(first: true);
                return;
            case Key.End:
                MovePluginPanelToEdge(first: false);
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl || inputEvent.Alt)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
            case 'j':
            case 'J':
                MovePluginPanelSelection(1);
                return;
            case 'k':
            case 'K':
                MovePluginPanelSelection(-1);
                return;
            case 'g':
                MovePluginPanelToEdge(first: true);
                return;
            case 'G':
                MovePluginPanelToEdge(first: false);
                return;
            case 'q':
            case 'Q':
                HandlePluginPanelKey("q", closeIfUnhandled: true);
                return;
        }

        HandlePluginPanelKey(inputEvent.Character.ToString());
    }

    private void HandlePluginPanelKey(string key, bool closeIfUnhandled = false)
    {
        bool handled = false;
        if (_pluginPanelOnKey is not null)
        {
            try
            {
                handled = _pluginPanelOnKey(key, _pluginPanelSelected);
            }
            catch (Exception ex)
            {
                SetError($"plugin panel: {ex.Message}");
                return;
            }
        }

        if (!_pluginPanelVisible)
        {
            return;
        }

        if (closeIfUnhandled && !handled)
        {
            ClosePluginPanel();
        }
    }

    private void MovePluginPanelSelection(int delta)
    {
        if (_pluginPanelLines.Count == 0 || delta == 0)
        {
            return;
        }

        if (!_pluginPanelSelectable)
        {
            ScrollPluginPanel(delta);
            return;
        }

        int step = Math.Sign(delta);
        int remaining = Math.Abs(delta);
        int index = (int)_pluginPanelSelected;
        while (remaining > 0)
        {
            int next = index + step;
            if (next < 0 || next >= _pluginPanelLines.Count)
            {
                break;
            }

            index = next;
            if (_pluginPanelLines[index].Selectable)
            {
                --remaining;
            }
        }

        _pluginPanelSelected = index;
        EnsurePluginPanelSelectionVisible();
    }

    private void ScrollPluginPanel(int delta)
    {
        int rows = PluginPanelContentRows();
        int maximum = Math.Max(0, _pluginPanelLines.Count - rows);
        _pluginPanelTop = Math.Clamp(_pluginPanelTop + delta, 0, maximum);
    }

    private void MovePluginPanelToEdge(bool first)
    {
        if (_pluginPanelLines.Count == 0)
        {
            return;
        }

        if (!_pluginPanelSelectable)
        {
            _pluginPanelTop = first
                ? 0
                : Math.Max(0, _pluginPanelLines.Count -
                               PluginPanelContentRows());
            return;
        }

        int index = first ? 0 : _pluginPanelLines.Count - 1;
        int direction = first ? 1 : -1;
        while (index >= 0 && index < _pluginPanelLines.Count)
        {
            if (_pluginPanelLines[index].Selectable)
            {
                _pluginPanelSelected = index;
                EnsurePluginPanelSelectionVisible();
                return;
            }
            index += direction;
        }
    }

    private void EnsurePluginPanelSelectionVisible()
    {
        int rows = PluginPanelContentRows();
        if (_pluginPanelSelected < _pluginPanelTop)
        {
            _pluginPanelTop = (int)_pluginPanelSelected;
        }
        else if (_pluginPanelSelected >= _pluginPanelTop + rows)
        {
            _pluginPanelTop = (int)_pluginPanelSelected - rows + 1;
        }

        _pluginPanelTop = Math.Clamp(_pluginPanelTop, 0,
            Math.Max(0, _pluginPanelLines.Count - rows));
    }

    private int PluginPanelContentRows()
    {
        int available = (int)Math.Max(0, StatusRow());
        int height = PluginPanelHeight(available);
        return Math.Max(1, height - 3);
    }

    private int PluginPanelHeight(int available)
    {
        if (available < 5)
        {
            return available;
        }

        int desired = Math.Max(5, _pluginPanelLines.Count + 3);
        return Math.Min(available, desired);
    }

    private void RenderPluginPanel()
    {
        if (!_pluginPanelVisible)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        if (available < 5 || _columns < 20)
        {
            return;
        }

        int height = PluginPanelHeight(available);
        int width = Math.Min(_columns - 2, 160);
        if (width < 20 || height < 5)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, (available - height) / 2);
        DrawBox(x, y, width, height, _pluginPanelTitle);

        int contentRows = Math.Max(1, height - 3);
        int contentWidth = Math.Max(0, width - 4);
        _pluginPanelTop = Math.Clamp(_pluginPanelTop, 0,
            Math.Max(0, _pluginPanelLines.Count - contentRows));

        for (int row = 0; row < contentRows; ++row)
        {
            int index = _pluginPanelTop + row;
            if (index >= _pluginPanelLines.Count)
            {
                break;
            }

            PluginPanelLine line = _pluginPanelLines[index];
            bool selected = _pluginPanelSelectable && line.Selectable &&
                            index == _pluginPanelSelected;
            int column = 0;
            foreach (PluginPanelSegment segment in line.Segments)
            {
                if (column >= contentWidth)
                {
                    break;
                }

                Style style = PluginPanelStyle(segment.Style);
                if (selected)
                {
                    style.Bg = _theme.SelectionBackground;
                    style.Bold = true;
                    style.Dim = false;
                }

                _screen.PutString(x + 2 + column, y + 1 + row, segment.Text,
                    style, contentWidth - column);
                column += (int)UnicodeUtil.ColumnWidth(segment.Text);
            }
        }

        if (_pluginPanelFooter.Length > 0)
        {
            _screen.PutString(x + 2, y + height - 2, _pluginPanelFooter,
                _theme.MutedStyle, width - 4);
        }
    }

    private Style PluginPanelStyle(string name)
    {
        return name.ToLowerInvariant() switch
        {
            "accent" => _theme.AccentStyle,
            "muted" => _theme.MutedStyle,
            "error" => _theme.ErrorStyle,
            "warning" => _theme.WarningStyle,
            "success" => new Style
            {
                Fg = _theme.Success,
                Bg = _theme.Background,
                Bold = true
            },
            "diff_added" => _theme.DiffAddedStyle,
            "diff_deleted" => _theme.DiffDeletedStyle,
            "diff_modified" => _theme.DiffModifiedStyle,
            _ => _theme.TextStyle
        };
    }
}
