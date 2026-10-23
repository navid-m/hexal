using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private static readonly StringComparer SidebarPathComparison =
        OperatingSystem.IsWindows()
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;

    private enum SidebarMenuAction
    {
        Open,
        NewFolder,
        NewFile,
        Rename,
        Duplicate,
        Copy,
        Move,
        Delete
    }

    private readonly record struct SidebarMenuItem(
        string Label, char Hotkey, SidebarMenuAction Action);

    private static readonly SidebarMenuItem[] SidebarFileMenuItems =
    {
        new("Open", 'O', SidebarMenuAction.Open),
        new("New Folder", 'X', SidebarMenuAction.NewFolder),
        new("New File", 'E', SidebarMenuAction.NewFile),
        new("Rename", 'R', SidebarMenuAction.Rename),
        new("Duplicate", 'D', SidebarMenuAction.Duplicate),
        new("Copy to...", 'T', SidebarMenuAction.Copy),
        new("Move to...", 'M', SidebarMenuAction.Move),
        new("Delete", 'd', SidebarMenuAction.Delete)
    };

    private static readonly SidebarMenuItem[] SidebarFolderMenuItems =
    {
        new("Expand/Collapse", 'O', SidebarMenuAction.Open),
        new("New Folder", 'X', SidebarMenuAction.NewFolder),
        new("New File", 'E', SidebarMenuAction.NewFile),
        new("Rename", 'R', SidebarMenuAction.Rename),
        new("Duplicate", 'D', SidebarMenuAction.Duplicate),
        new("Copy to...", 'T', SidebarMenuAction.Copy),
        new("Move to...", 'M', SidebarMenuAction.Move),
        new("Delete", 'd', SidebarMenuAction.Delete)
    };

    private static readonly SidebarMenuItem[] SidebarMultiMenuItems =
    {
        new("Duplicate", 'D', SidebarMenuAction.Duplicate),
        new("Copy to...", 'T', SidebarMenuAction.Copy),
        new("Move to...", 'M', SidebarMenuAction.Move),
        new("Delete", 'd', SidebarMenuAction.Delete)
    };

    private sealed class SidebarNode
    {
        public string Path = string.Empty;
        public string Name = string.Empty;
        public int Depth;
        public bool IsDirectory;
        public bool Expanded;
    }

    private bool _sidebarVisible;
    private bool _sidebarFocused;
    private bool _sidebarMenuVisible;
    private bool _showGitIgnored;
    private int _sidebarSelected;
    private int _sidebarSelectionAnchor;
    private int _sidebarTop;
    private int _sidebarMenuSelected;
    private int _editorLeft;
    private readonly List<SidebarNode> _sidebarNodes = new();
    private readonly HashSet<string> _sidebarExpanded =
        new(SidebarPathComparison);
    private readonly List<(string Path, bool IsDirectory)> _pendingSidebarDeletes =
        new();

    private SidebarNode? SelectedSidebarNode
        => _sidebarSelected >= 0 && _sidebarSelected < _sidebarNodes.Count
            ? _sidebarNodes[_sidebarSelected]
            : null;

    private int SidebarSelectionStart
        => Math.Min(_sidebarSelected, _sidebarSelectionAnchor);

    private int SidebarSelectionEnd
        => Math.Max(_sidebarSelected, _sidebarSelectionAnchor);

    private int SidebarSelectionCount
        => _sidebarNodes.Count == 0
            ? 0
            : Math.Abs(_sidebarSelected - _sidebarSelectionAnchor) + 1;

    private int SidebarWidth()
    {
        if (!_sidebarVisible || _columns < 36)
        {
            return 0;
        }

        int width = Math.Clamp(_columns / 3, 20, 40);
        return Math.Min(width, _columns - 16);
    }

    private void UpdateEditorLeft()
    {
        _editorLeft = SidebarWidth();
    }

    private static string SidebarFullPath(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return path;
        }
    }

    private void FocusSidebarFromMouse()
    {
        _sidebarFocused = true;
        _mode = Mode.Normal;
        _selectionLinewise = false;
        _selectionAnchor = _cursor;
        _document.BreakUndoGroup();
    }

    private int SidebarListRows()
    {
        return Math.Max(1, TextRows() - 1);
    }

    private void ToggleSidebar()
    {
        if (!_sidebarVisible)
        {
            _sidebarVisible = true;
            UpdateEditorLeft();
            if (_editorLeft <= 0)
            {
                _sidebarVisible = false;
                UpdateEditorLeft();
                SetError("Terminal is too narrow for the sidebar.");
                return;
            }

            _sidebarFocused = true;
            _sidebarMenuVisible = false;
            _mode = Mode.Normal;
            _selectionLinewise = false;
            _selectionAnchor = _cursor;
            _document.BreakUndoGroup();
            ExpandSidebarToPath(_document.Path);
            RefreshSidebar(_document.Path);
            SetMessage("Sidebar focused (Ctrl+B to close)");
        }
        else if (!_sidebarFocused)
        {
            _sidebarFocused = true;
            _sidebarMenuVisible = false;
            _mode = Mode.Normal;
            _selectionLinewise = false;
            _selectionAnchor = _cursor;
            _document.BreakUndoGroup();
            ExpandSidebarToPath(_document.Path);
            RefreshSidebar(_document.Path);
            SetMessage("Sidebar focused (Ctrl+B to close)");
        }
        else
        {
            _sidebarVisible = false;
            _sidebarFocused = false;
            _sidebarMenuVisible = false;
            UpdateEditorLeft();
            ClearMessage();
        }

        _forceFullRedraw = true;
        EnsureCursorVisible();
    }

    private void HandleSidebar(InputEvent inputEvent)
    {
        if (_sidebarMenuVisible)
        {
            HandleSidebarMenu(inputEvent);
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Escape:
                if (SidebarSelectionCount > 1)
                {
                    _sidebarSelectionAnchor = _sidebarSelected;
                    return;
                }

                _sidebarFocused = false;
                ClearMessage();
                return;
            case Key.Tab:
                MoveSidebarSelection(1, wrap: true);
                return;
            case Key.BackTab:
                MoveSidebarSelection(-1, wrap: true);
                return;
            case Key.Up:
                MoveSidebarSelection(-1, wrap: false, extend: inputEvent.Shift);
                return;
            case Key.Down:
                MoveSidebarSelection(1, wrap: false, extend: inputEvent.Shift);
                return;
            case Key.PageUp:
                MoveSidebarSelection(-SidebarListRows(), wrap: false,
                    extend: inputEvent.Shift);
                return;
            case Key.PageDown:
                MoveSidebarSelection(SidebarListRows(), wrap: false,
                    extend: inputEvent.Shift);
                return;
            case Key.Home:
                SetSidebarSelection(0, extend: inputEvent.Shift);
                return;
            case Key.End:
                SetSidebarSelection(_sidebarNodes.Count - 1,
                    extend: inputEvent.Shift);
                return;
            case Key.Left:
                CollapseSidebarOrSelectParent();
                return;
            case Key.Right:
                ExpandSidebarOrSelectChild();
                return;
            case Key.Enter:
                ActivateSelectedSidebarNode();
                return;
        }

        if (inputEvent.Key == Key.Character && !inputEvent.Ctrl &&
            !inputEvent.Alt)
        {
            uint character = (uint)inputEvent.Character.Value;
            if (character is 'c' or 'C')
            {
                OpenSidebarContextMenu();
            }
            else if (character == ' ')
            {
                ActivateSelectedSidebarNode();
            }
        }
    }

    private void MoveSidebarSelection(int delta, bool wrap, bool extend = false)
    {
        int count = _sidebarNodes.Count;
        if (count == 0)
        {
            return;
        }

        int index = _sidebarSelected + delta;
        if (wrap)
        {
            index = ((index % count) + count) % count;
        }
        else
        {
            index = Math.Clamp(index, 0, count - 1);
        }

        SetSidebarSelection(index, extend);
    }

    private void SetSidebarSelection(int index, bool extend = false)
    {
        if (_sidebarNodes.Count == 0)
        {
            _sidebarSelected = 0;
            _sidebarSelectionAnchor = 0;
            return;
        }

        _sidebarSelected = Math.Clamp(index, 0, _sidebarNodes.Count - 1);
        if (!extend)
        {
            _sidebarSelectionAnchor = _sidebarSelected;
        }

        EnsureSidebarSelectionVisible();
    }

    private void EnsureSidebarSelectionVisible()
    {
        if (_sidebarNodes.Count == 0)
        {
            _sidebarSelected = 0;
            _sidebarSelectionAnchor = 0;
            _sidebarTop = 0;
            return;
        }

        _sidebarSelected = Math.Clamp(_sidebarSelected, 0,
            _sidebarNodes.Count - 1);
        _sidebarSelectionAnchor = Math.Clamp(_sidebarSelectionAnchor, 0,
            _sidebarNodes.Count - 1);
        int rows = SidebarListRows();
        if (_sidebarSelected < _sidebarTop)
        {
            _sidebarTop = _sidebarSelected;
        }
        else if (_sidebarSelected >= _sidebarTop + rows)
        {
            _sidebarTop = _sidebarSelected - rows + 1;
        }

        _sidebarTop = Math.Clamp(_sidebarTop, 0,
            Math.Max(0, _sidebarNodes.Count - rows));
    }

    private void ActivateSelectedSidebarNode()
    {
        SidebarNode? node = SelectedSidebarNode;
        if (node is null)
        {
            return;
        }

        if (node.IsDirectory)
        {
            ToggleSidebarDirectory(node);
        }
        else
        {
            OpenSidebarFile(node);
        }
    }

    private void ToggleSidebarDirectory(SidebarNode node)
    {
        if (!node.IsDirectory)
        {
            return;
        }

        if (!_sidebarExpanded.Remove(node.Path))
        {
            _sidebarExpanded.Add(node.Path);
        }

        RefreshSidebar(node.Path);
    }

    private void CollapseSidebarOrSelectParent()
    {
        SidebarNode? node = SelectedSidebarNode;
        if (node is null)
        {
            return;
        }

        if (node.IsDirectory && node.Expanded)
        {
            _sidebarExpanded.Remove(node.Path);
            RefreshSidebar(node.Path);
            return;
        }

        string? parent = Path.GetDirectoryName(node.Path);
        if (parent is not null &&
            !SamePath(parent, ProjectRoot()) &&
            IsPathInside(ProjectRoot(), parent))
        {
            SelectSidebarPath(parent);
        }
    }

    private void ExpandSidebarOrSelectChild()
    {
        SidebarNode? node = SelectedSidebarNode;
        if (node is null || !node.IsDirectory)
        {
            return;
        }

        if (!node.Expanded)
        {
            _sidebarExpanded.Add(node.Path);
            RefreshSidebar(node.Path);
            return;
        }

        if (_sidebarSelected + 1 < _sidebarNodes.Count &&
            _sidebarNodes[_sidebarSelected + 1].Depth > node.Depth)
        {
            SetSidebarSelection(_sidebarSelected + 1);
        }
    }

    private void OpenSidebarFile(SidebarNode node)
    {
        _sidebarMenuVisible = false;
        _sidebarFocused = _config.KeepSidebarFocusOnOpen && _sidebarVisible;
        RequestOpen(node.Path, force: false);
    }

    private void OpenSidebarContextMenu()
    {
        if (SelectedSidebarNode is null)
        {
            return;
        }

        _sidebarMenuVisible = true;
        _sidebarMenuSelected = 0;
        _sidebarFocused = true;
        ClearMessage();
    }

    private SidebarMenuItem[] SidebarMenuItemsForSelection()
    {
        if (SidebarSelectionCount > 1)
        {
            return SidebarMultiMenuItems;
        }

        return SelectedSidebarNode is { IsDirectory: true }
            ? SidebarFolderMenuItems
            : SidebarFileMenuItems;
    }

    private List<SidebarNode> SelectedSidebarNodes()
    {
        var result = new List<SidebarNode>();
        if (_sidebarNodes.Count == 0)
        {
            return result;
        }

        int start = Math.Clamp(SidebarSelectionStart, 0, _sidebarNodes.Count - 1);
        int end = Math.Clamp(SidebarSelectionEnd, 0, _sidebarNodes.Count - 1);
        for (int index = start; index <= end; ++index)
        {
            result.Add(_sidebarNodes[index]);
        }

        return result;
    }

    private List<SidebarNode> EffectiveSidebarSelection()
    {
        List<SidebarNode> selected = SelectedSidebarNodes();
        if (selected.Count < 2)
        {
            return selected;
        }

        var selectedDirectories = new List<string>();
        foreach (SidebarNode node in selected)
        {
            if (node.IsDirectory)
            {
                selectedDirectories.Add(SidebarFullPath(node.Path));
            }
        }

        if (selectedDirectories.Count == 0)
        {
            return selected;
        }

        var result = new List<SidebarNode>();
        foreach (SidebarNode node in selected)
        {
            string path = SidebarFullPath(node.Path);
            bool covered = false;
            foreach (string directory in selectedDirectories)
            {
                if (!SamePath(path, directory) && IsPathInside(directory, path))
                {
                    covered = true;
                    break;
                }
            }

            if (!covered)
            {
                result.Add(node);
            }
        }

        return result;
    }

    private static bool SidebarHotkeyMatches(char hotkey, char character)
    {
        if (hotkey == character)
        {
            return true;
        }

        return hotkey != 'd' &&
               char.ToUpperInvariant(character) == hotkey;
    }

    private void HandleSidebarMenu(InputEvent inputEvent)
    {
        SidebarMenuItem[] items = SidebarMenuItemsForSelection();

        switch (inputEvent.Key)
        {
            case Key.Escape:
                _sidebarMenuVisible = false;
                return;
            case Key.Tab:
                MoveSidebarMenuSelection(1, items.Length, wrap: true);
                return;
            case Key.BackTab:
                MoveSidebarMenuSelection(-1, items.Length, wrap: true);
                return;
            case Key.Up:
                MoveSidebarMenuSelection(-1, items.Length, wrap: false);
                return;
            case Key.Down:
                MoveSidebarMenuSelection(1, items.Length, wrap: false);
                return;
            case Key.Enter:
                ActivateSidebarMenuItem();
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl ||
            inputEvent.Alt)
        {
            return;
        }

        char character = (char)inputEvent.Character.Value;
        if (character is 'c' or 'C')
        {
            _sidebarMenuVisible = false;
            return;
        }

        int hotkeyIndex = Array.FindIndex(items,
            item => SidebarHotkeyMatches(item.Hotkey, character));
        if (hotkeyIndex >= 0)
        {
            _sidebarMenuSelected = hotkeyIndex;
            ActivateSidebarMenuItem();
        }
    }

    private void MoveSidebarMenuSelection(int delta, int count, bool wrap)
    {
        if (count == 0)
        {
            _sidebarMenuSelected = 0;
            return;
        }

        int index = _sidebarMenuSelected + delta;
        if (wrap)
        {
            index = ((index % count) + count) % count;
        }
        else
        {
            index = Math.Clamp(index, 0, count - 1);
        }

        _sidebarMenuSelected = index;
    }

    private void ActivateSidebarMenuItem()
    {
        SidebarMenuItem[] items = SidebarMenuItemsForSelection();
        SidebarNode? node = SelectedSidebarNode;
        if (node is null || _sidebarMenuSelected < 0 ||
            _sidebarMenuSelected >= items.Length)
        {
            _sidebarMenuVisible = false;
            return;
        }

        SidebarMenuItem item = items[_sidebarMenuSelected];
        _sidebarMenuVisible = false;
        _sidebarFocused = true;

        List<SidebarNode> nodes = EffectiveSidebarSelection();
        switch (item.Action)
        {
            case SidebarMenuAction.Open:
                if (node.IsDirectory)
                {
                    ToggleSidebarDirectory(node);
                }
                else
                {
                    OpenSidebarFile(node);
                }
                return;
            case SidebarMenuAction.NewFolder:
                if (nodes.Count == 1)
                {
                    BeginSidebarNewFolder(nodes[0]);
                }
                return;
            case SidebarMenuAction.NewFile:
                if (nodes.Count == 1)
                {
                    BeginSidebarNewFile(nodes[0]);
                }
                return;
            case SidebarMenuAction.Rename:
                if (nodes.Count == 1)
                {
                    BeginSidebarRename(nodes[0]);
                }
                return;
            case SidebarMenuAction.Duplicate:
                DuplicateSidebarNodes(nodes);
                return;
            case SidebarMenuAction.Copy:
                BeginSidebarCopy(nodes);
                return;
            case SidebarMenuAction.Move:
                BeginSidebarMove(nodes);
                return;
            case SidebarMenuAction.Delete:
                RequestDeleteSidebarNodes(nodes);
                return;
        }
    }

    private string SidebarParentForNewEntry(SidebarNode node)
        => node.IsDirectory
            ? node.Path
            : Path.GetDirectoryName(node.Path) ?? ProjectRoot();

    private void BeginSidebarNewFolder(SidebarNode node)
    {
        _sidebarFocused = _sidebarVisible;
        string parent = SidebarParentForNewEntry(node);
        RequestPrompt("New folder name: ", value =>
            CompleteSidebarNewFolder(parent, value));
    }

    private void CompleteSidebarNewFolder(string parent, string? value)
    {
        _sidebarFocused = _sidebarVisible;
        if (string.IsNullOrWhiteSpace(value))
        {
            SetMessage("New folder cancelled.");
            RefreshSidebar(null);
            return;
        }

        string name = value.Trim();
        if (!IsValidSidebarName(name))
        {
            SetError("Invalid folder name.");
            RefreshSidebar(null);
            return;
        }

        string target = Path.Combine(parent, name);
        if (File.Exists(target) || Directory.Exists(target))
        {
            SetError($"'{name}' already exists.");
            RefreshSidebar(null);
            return;
        }

        try
        {
            Directory.CreateDirectory(target);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            SetError($"Create folder failed: {ex.Message}");
            RefreshSidebar(null);
            return;
        }

        ExpandSidebarToPath(target);
        RefreshSidebar(target);
        SetMessage($"Created folder {name}");
    }

    private void BeginSidebarNewFile(SidebarNode node)
    {
        _sidebarFocused = _sidebarVisible;
        string parent = SidebarParentForNewEntry(node);
        RequestPrompt("New file name: ", value =>
            CompleteSidebarNewFile(parent, value));
    }

    private void CompleteSidebarNewFile(string parent, string? value)
    {
        _sidebarFocused = _sidebarVisible;
        if (string.IsNullOrWhiteSpace(value))
        {
            SetMessage("New file cancelled.");
            RefreshSidebar(null);
            return;
        }

        string name = value.Trim();
        if (!IsValidSidebarName(name))
        {
            SetError("Invalid file name.");
            RefreshSidebar(null);
            return;
        }

        string target = Path.Combine(parent, name);
        if (File.Exists(target) || Directory.Exists(target))
        {
            SetError($"'{name}' already exists.");
            RefreshSidebar(null);
            return;
        }

        try
        {
            using var stream = new FileStream(target, FileMode.CreateNew,
                FileAccess.Write, FileShare.None);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            SetError($"Create file failed: {ex.Message}");
            RefreshSidebar(null);
            return;
        }

        ExpandSidebarToPath(target);
        RefreshSidebar(target);
        SetMessage($"Created file {name}");
    }

    private void BeginSidebarRename(SidebarNode node)
    {
        string oldPath = node.Path;
        bool isDirectory = node.IsDirectory;
        string initial = node.Name;

        RequestPrompt($"Rename {node.Name}: ", value =>
            CompleteSidebarRename(oldPath, isDirectory, value));
        _pluginPromptText.Append(initial);
        _pluginPromptCursor = _pluginPromptText.Length;
    }

    private void CompleteSidebarRename(string oldPath, bool isDirectory,
        string? value)
    {
        _sidebarFocused = _sidebarVisible;
        if (string.IsNullOrWhiteSpace(value))
        {
            SetMessage("Rename cancelled.");
            RefreshSidebar(oldPath);
            return;
        }

        string name = value.Trim();
        if (!IsValidSidebarName(name))
        {
            SetError("Invalid file or folder name.");
            RefreshSidebar(oldPath);
            return;
        }

        string parent = Path.GetDirectoryName(oldPath) ?? ProjectRoot();
        string target = Path.Combine(parent, name);
        if (SamePath(target, oldPath))
        {
            SetMessage("Name unchanged.");
            RefreshSidebar(oldPath);
            return;
        }

        if (File.Exists(target) || Directory.Exists(target))
        {
            SetError($"'{name}' already exists.");
            RefreshSidebar(oldPath);
            return;
        }

        try
        {
            if (isDirectory)
            {
                Directory.Move(oldPath, target);
                UpdateSidebarExpandedAfterRename(oldPath, target);
            }
            else
            {
                File.Move(oldPath, target);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            SetError($"Rename failed: {ex.Message}");
            RefreshSidebar(oldPath);
            return;
        }

        UpdateOpenDocumentPathAfterRename(oldPath, target, isDirectory);
        ExpandSidebarToPath(target);
        RefreshSidebar(target);
        SetMessage($"Renamed to {name}");
    }

    private static bool IsValidSidebarName(string name)
    {
        if (name.Length == 0 || name is "." or "..")
        {
            return false;
        }

        if (name.Contains(Path.DirectorySeparatorChar) ||
            name.Contains(Path.AltDirectorySeparatorChar))
        {
            return false;
        }

        return name.IndexOfAny(Path.GetInvalidFileNameChars()) < 0;
    }

    private void BeginSidebarCopy(IReadOnlyList<SidebarNode> nodes)
    {
        if (nodes.Count == 0)
        {
            return;
        }

        if (nodes.Count == 1)
        {
            BeginSidebarCopy(nodes[0]);
            return;
        }

        _sidebarFocused = _sidebarVisible;
        List<string> folders = CollectSidebarMoveFolders(nodes);
        List<string> labels = SidebarFolderLabels(folders);
        OpenPluginPicker("Copy to folder", labels, index =>
        {
            _sidebarFocused = _sidebarVisible;
            if (index is null)
            {
                SetMessage("Copy cancelled.");
                return;
            }

            if (index.Value < 0 || index.Value >= folders.Count)
            {
                return;
            }

            CompleteSidebarCopy(nodes, folders[index.Value]);
        });
        SetMessage($"Copying {nodes.Count} items...");
    }

    private void BeginSidebarCopy(SidebarNode node)
    {
        string source = node.Path;
        bool isDirectory = node.IsDirectory;
        string initial = UniqueSidebarCopyPath(source, isDirectory);

        RequestPrompt("Copy to: ", value =>
            CompleteSidebarCopy(source, isDirectory, value));
        _pluginPromptText.Append(initial);
        _pluginPromptCursor = _pluginPromptText.Length;
    }

    private void CompleteSidebarCopy(string source, bool isDirectory,
        string? value)
    {
        _sidebarFocused = _sidebarVisible;
        if (string.IsNullOrWhiteSpace(value))
        {
            SetMessage("Copy cancelled.");
            RefreshSidebar(source);
            return;
        }

        string destination = value.Trim();
        try
        {
            if (!Path.IsPathRooted(destination))
            {
                destination = Path.Combine(ProjectRoot(), destination);
            }
            destination = Path.GetFullPath(destination);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            SetError($"Invalid destination: {ex.Message}");
            RefreshSidebar(source);
            return;
        }

        if (Directory.Exists(destination))
        {
            destination = Path.Combine(destination, Path.GetFileName(source));
        }

        if (!CopySidebarPath(source, destination, isDirectory, out string error))
        {
            SetError(error);
            RefreshSidebar(source);
            return;
        }

        ExpandSidebarToPath(destination);
        RefreshSidebar(destination);
        SetMessage($"Copied to {Path.GetFileName(destination)}");
    }

    private void CompleteSidebarCopy(IReadOnlyList<SidebarNode> nodes,
        string destinationFolder)
    {
        _sidebarFocused = _sidebarVisible;
        string folderFull;
        try
        {
            folderFull = Path.GetFullPath(destinationFolder);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            SetError($"Invalid destination: {ex.Message}");
            RefreshSidebar(null);
            return;
        }

        if (!Directory.Exists(folderFull))
        {
            SetError($"Destination folder does not exist: '{folderFull}'");
            RefreshSidebar(null);
            return;
        }

        var targets = new HashSet<string>(SidebarPathComparison);
        foreach (SidebarNode node in nodes)
        {
            string target = SidebarFullPath(Path.Combine(folderFull, node.Name));
            if (!targets.Add(target))
            {
                SetError($"Multiple items would be copied to '{node.Name}'.");
                RefreshSidebar(null);
                return;
            }
        }

        int copied = 0;
        string? firstError = null;
        string? lastTarget = null;
        foreach (SidebarNode node in nodes)
        {
            string target = Path.Combine(folderFull, node.Name);
            if (!CopySidebarPath(node.Path, target, node.IsDirectory,
                    out string error))
            {
                firstError ??= error;
                continue;
            }

            ++copied;
            lastTarget = target;
        }

        if (lastTarget is not null)
        {
            ExpandSidebarToPath(lastTarget);
            RefreshSidebar(lastTarget);
        }
        else
        {
            RefreshSidebar(null);
        }

        if (firstError is not null)
        {
            SetError(copied == 0
                ? firstError
                : $"Copied {copied} of {nodes.Count} items; {firstError}");
            return;
        }

        SetMessage($"Copied {copied} items to {folderFull}");
    }

    private void BeginSidebarMove(IReadOnlyList<SidebarNode> nodes)
    {
        if (nodes.Count == 0)
        {
            return;
        }

        _sidebarFocused = _sidebarVisible;
        List<string> folders = CollectSidebarMoveFolders(nodes);
        List<string> labels = SidebarFolderLabels(folders);
        OpenPluginPicker("Move to folder", labels, index =>
        {
            _sidebarFocused = _sidebarVisible;
            if (index is null)
            {
                SetMessage("Move cancelled.");
                return;
            }

            if (index.Value < 0 || index.Value >= folders.Count)
            {
                return;
            }

            CompleteSidebarMove(nodes, folders[index.Value]);
        });
        SetMessage(nodes.Count == 1
            ? $"Moving {Path.GetFileName(nodes[0].Path)}..."
            : $"Moving {nodes.Count} items...");
    }

    private List<string> SidebarFolderLabels(List<string> folders)
    {
        string root = SidebarFullPath(ProjectRoot());
        var labels = new List<string>(folders.Count);
        foreach (string folder in folders)
        {
            string relative = Path.GetRelativePath(root, folder);
            labels.Add(relative == "." ? "./ (project root)" : relative +
                Path.DirectorySeparatorChar);
        }

        return labels;
    }

    private List<string> CollectSidebarMoveFolders(
        IReadOnlyList<SidebarNode> sources)
    {
        string root = SidebarFullPath(ProjectRoot());
        var result = new List<string> { root };
        GitIgnore? gitIgnore = _showGitIgnored ? null : new GitIgnore(root);
        AddSidebarMoveFolders(root, sources, result, gitIgnore);
        return result;
    }

    private static void AddSidebarMoveFolders(string directory,
        IReadOnlyList<SidebarNode> sources, List<string> folders,
        GitIgnore? gitIgnore)
    {
        string[] directories;
        try
        {
            directories = Directory.GetDirectories(directory);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            return;
        }

        Array.Sort(directories, SidebarPathComparison);
        foreach (string child in directories)
        {
            try
            {
                FileAttributes attributes = File.GetAttributes(child);
                if ((attributes & FileAttributes.ReparsePoint) != 0)
                {
                    continue;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                continue;
            }

            string name = Path.GetFileName(child);
            if (name.Length == 0)
            {
                continue;
            }

            if (gitIgnore is not null &&
                (GitIgnore.IsBuiltInIgnoredDirectory(name) ||
                 gitIgnore.IsIgnored(child, isDirectory: true)))
            {
                continue;
            }

            if (IsSelectedSidebarDirectoryOrChild(child, sources))
            {
                continue;
            }

            folders.Add(child);
            AddSidebarMoveFolders(child, sources, folders, gitIgnore);
        }
    }

    private static bool IsSelectedSidebarDirectoryOrChild(string path,
        IReadOnlyList<SidebarNode> sources)
    {
        foreach (SidebarNode source in sources)
        {
            if (source.IsDirectory &&
                (SamePath(path, source.Path) || IsPathInside(source.Path, path)))
            {
                return true;
            }
        }

        return false;
    }

    private void CompleteSidebarMove(IReadOnlyList<SidebarNode> nodes,
        string destinationFolder)
    {
        _sidebarFocused = _sidebarVisible;
        string folderFull;
        try
        {
            folderFull = Path.GetFullPath(destinationFolder);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            SetError($"Invalid destination: {ex.Message}");
            RefreshSidebar(null);
            return;
        }

        if (!Directory.Exists(folderFull))
        {
            SetError($"Destination folder does not exist: '{folderFull}'");
            RefreshSidebar(null);
            return;
        }

        var moves = new List<(SidebarNode Node, string Target)>();
        var targets = new HashSet<string>(SidebarPathComparison);
        foreach (SidebarNode node in nodes)
        {
            string sourceFull = SidebarFullPath(node.Path);
            if (node.IsDirectory &&
                (SamePath(folderFull, sourceFull) ||
                 IsPathInside(sourceFull, folderFull)))
            {
                SetError("Cannot move a folder inside itself.");
                RefreshSidebar(null);
                return;
            }

            string target = Path.Combine(folderFull, node.Name);
            if (SamePath(target, sourceFull))
            {
                continue;
            }

            if (!targets.Add(SidebarFullPath(target)))
            {
                SetError($"Multiple items would be moved to '{node.Name}'.");
                RefreshSidebar(null);
                return;
            }

            if (File.Exists(target) || Directory.Exists(target))
            {
                SetError($"'{node.Name}' already exists in that folder.");
                RefreshSidebar(null);
                return;
            }

            moves.Add((node, target));
        }

        if (moves.Count == 0)
        {
            SetMessage("Already in that folder.");
            return;
        }

        int moved = 0;
        string? firstError = null;
        string? lastTarget = null;
        foreach ((SidebarNode node, string target) in moves)
        {
            try
            {
                if (node.IsDirectory)
                {
                    Directory.Move(node.Path, target);
                    UpdateSidebarExpandedAfterRename(node.Path, target);
                }
                else
                {
                    File.Move(node.Path, target);
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException or
                                       PathTooLongException)
            {
                firstError ??= $"Move failed: {ex.Message}";
                continue;
            }

            UpdateOpenDocumentPathAfterRename(node.Path, target,
                node.IsDirectory);
            ++moved;
            lastTarget = target;
        }

        if (lastTarget is not null)
        {
            ExpandSidebarToPath(lastTarget);
            RefreshSidebar(lastTarget);
        }
        else
        {
            RefreshSidebar(null);
        }

        if (firstError is not null)
        {
            SetError(moved == 0
                ? firstError
                : $"Moved {moved} of {moves.Count} items; {firstError}");
            return;
        }

        SetMessage(moves.Count == 1
            ? $"Moved {Path.GetFileName(moves[0].Node.Path)} to {folderFull}"
            : $"Moved {moved} items to {folderFull}");
    }

    private void DuplicateSidebarNodes(IReadOnlyList<SidebarNode> nodes)
    {
        if (nodes.Count == 0)
        {
            return;
        }

        _sidebarFocused = _sidebarVisible;
        int duplicated = 0;
        string? firstError = null;
        string? lastTarget = null;
        foreach (SidebarNode node in nodes)
        {
            string target = UniqueSidebarCopyPath(node.Path, node.IsDirectory);
            if (!CopySidebarPath(node.Path, target, node.IsDirectory,
                    out string error))
            {
                firstError ??= error;
                continue;
            }

            ++duplicated;
            lastTarget = target;
        }

        if (lastTarget is not null)
        {
            ExpandSidebarToPath(lastTarget);
            RefreshSidebar(lastTarget);
        }
        else
        {
            RefreshSidebar(null);
        }

        if (firstError is not null)
        {
            SetError(duplicated == 0
                ? firstError
                : $"Duplicated {duplicated} of {nodes.Count} items; {firstError}");
            return;
        }

        SetMessage(duplicated == 1
            ? $"Duplicated as {Path.GetFileName(lastTarget)}"
            : $"Duplicated {duplicated} items");
    }

    private static string UniqueSidebarCopyPath(string source, bool isDirectory)
    {
        string parent = Path.GetDirectoryName(source) ?? ".";
        string name = Path.GetFileName(source);
        string stem = isDirectory ? name : Path.GetFileNameWithoutExtension(name);
        string extension = isDirectory ? string.Empty : Path.GetExtension(name);

        for (int copy = 1; copy < 10_000; ++copy)
        {
            string candidateName = copy == 1
                ? $"{stem} copy{extension}"
                : $"{stem} copy {copy}{extension}";
            string candidate = Path.Combine(parent, candidateName);
            if (!File.Exists(candidate) && !Directory.Exists(candidate))
            {
                return candidate;
            }
        }

        return Path.Combine(parent, $"{stem} copy{extension}");
    }

    private static bool CopySidebarPath(string source, string target,
        bool isDirectory, out string error)
    {
        error = string.Empty;

        try
        {
            if (isDirectory)
            {
                if (SamePath(source, target) || IsPathInside(source, target))
                {
                    error = "Cannot copy a folder inside itself.";
                    return false;
                }

                string? parent = Path.GetDirectoryName(target);
                if (!string.IsNullOrEmpty(parent) && !Directory.Exists(parent))
                {
                    error = $"Destination folder does not exist: '{parent}'";
                    return false;
                }

                if (Directory.Exists(target) || File.Exists(target))
                {
                    error = $"Destination already exists: '{target}'";
                    return false;
                }

                CopySidebarDirectory(source, target);
                return true;
            }

            string? fileParent = Path.GetDirectoryName(target);
            if (!string.IsNullOrEmpty(fileParent) && !Directory.Exists(fileParent))
            {
                error = $"Destination folder does not exist: '{fileParent}'";
                return false;
            }

            if (File.Exists(target) || Directory.Exists(target))
            {
                error = $"Destination already exists: '{target}'";
                return false;
            }

            File.Copy(source, target, overwrite: false);
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            error = $"Copy failed: {ex.Message}";
            if (isDirectory && Directory.Exists(target))
            {
                try
                {
                    Directory.Delete(target, recursive: true);
                }
                catch
                {
                    // Best-effort cleanup only.
                }
            }
            return false;
        }
    }

    private static void CopySidebarDirectory(string source, string target)
    {
        Directory.CreateDirectory(target);

        foreach (string file in Directory.GetFiles(source))
        {
            File.Copy(file, Path.Combine(target, Path.GetFileName(file)),
                overwrite: false);
        }

        foreach (string directory in Directory.GetDirectories(source))
        {
            FileAttributes attributes = File.GetAttributes(directory);
            if ((attributes & FileAttributes.ReparsePoint) != 0)
            {
                continue;
            }

            CopySidebarDirectory(directory,
                Path.Combine(target, Path.GetFileName(directory)));
        }
    }

    private void RequestDeleteSidebarNodes(IReadOnlyList<SidebarNode> nodes)
    {
        if (nodes.Count == 0)
        {
            return;
        }

        foreach (SidebarNode node in nodes)
        {
            if (SidebarPathContainsOpenDocument(node.Path, node.IsDirectory))
            {
                SetError("Cannot delete the open file or its parent folder.");
                _sidebarFocused = _sidebarVisible;
                return;
            }
        }

        _pendingSidebarDeletes.Clear();
        foreach (SidebarNode node in nodes)
        {
            _pendingSidebarDeletes.Add((node.Path, node.IsDirectory));
        }

        _prompt = PromptKind.SidebarDelete;
        ClearMessage();
    }

    private bool SidebarPathContainsOpenDocument(string path, bool isDirectory)
    {
        if (_document.Path is null)
        {
            return false;
        }

        string documentPath = SidebarFullPath(_document.Path);
        path = SidebarFullPath(path);
        if (SamePath(documentPath, path))
        {
            return true;
        }

        return isDirectory && IsPathInside(path, documentPath);
    }

    private void DeleteSidebarPaths()
    {
        _sidebarFocused = _sidebarVisible;
        var items = new List<(string Path, bool IsDirectory)>(
            _pendingSidebarDeletes);
        ClearPendingSidebarDelete();

        int deleted = 0;
        string? firstError = null;
        foreach ((string path, bool isDirectory) in items)
        {
            try
            {
                if (isDirectory)
                {
                    Directory.Delete(path, recursive: true);
                    RemoveSidebarExpandedUnder(path);
                }
                else
                {
                    File.Delete(path);
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException or
                                       PathTooLongException)
            {
                firstError ??=
                    $"Delete failed for '{Path.GetFileName(path)}': {ex.Message}";
                continue;
            }

            ++deleted;
        }

        RefreshSidebar(null);

        if (firstError is not null)
        {
            SetError(deleted == 0
                ? firstError
                : $"Deleted {deleted} of {items.Count} items; {firstError}");
            return;
        }

        SetMessage(items.Count == 1
            ? $"Deleted {Path.GetFileName(items[0].Path)}"
            : $"Deleted {deleted} items");
    }

    private void ClearPendingSidebarDelete()
    {
        _pendingSidebarDeletes.Clear();
    }

    private void RefreshSidebar(string? selectPath)
    {
        string? selectedPath = selectPath ?? SelectedSidebarNode?.Path;
        _sidebarNodes.Clear();
        GitIgnore? gitIgnore = _showGitIgnored ? null : new GitIgnore(ProjectRoot());
        AddSidebarDirectory(ProjectRoot(), 0, root: true, gitIgnore);

        if (_sidebarNodes.Count == 0)
        {
            _sidebarSelected = 0;
            _sidebarSelectionAnchor = 0;
            _sidebarTop = 0;
            return;
        }

        if (selectedPath is not null)
        {
            SelectSidebarPath(selectedPath);
        }

        _sidebarSelectionAnchor = _sidebarSelected;
        EnsureSidebarSelectionVisible();
    }

    private void AddSidebarDirectory(string directory, int depth, bool root,
        GitIgnore? gitIgnore)
    {
        string[] directories;
        string[] files;
        try
        {
            directories = Directory.GetDirectories(directory);
            files = Directory.GetFiles(directory);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            return;
        }

        Array.Sort(directories, SidebarPathComparison);
        Array.Sort(files, SidebarPathComparison);

        foreach (string child in directories)
        {
            try
            {
                FileAttributes attributes = File.GetAttributes(child);
                if ((attributes & FileAttributes.ReparsePoint) != 0)
                {
                    continue;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                continue;
            }

            string name = Path.GetFileName(child);
            if (name.Length == 0)
            {
                continue;
            }

            if (gitIgnore is not null &&
                (GitIgnore.IsBuiltInIgnoredDirectory(name) ||
                 gitIgnore.IsIgnored(child, isDirectory: true)))
            {
                continue;
            }

            bool expanded = _sidebarExpanded.Contains(child);
            _sidebarNodes.Add(new SidebarNode
            {
                Path = child,
                Name = name,
                Depth = depth,
                IsDirectory = true,
                Expanded = expanded
            });

            if (expanded)
            {
                AddSidebarDirectory(child, depth + 1, root: false, gitIgnore);
            }
        }

        foreach (string child in files)
        {
            string name = Path.GetFileName(child);
            if (name.Length == 0)
            {
                continue;
            }

            if (gitIgnore is not null &&
                gitIgnore.IsIgnored(child, isDirectory: false))
            {
                continue;
            }

            _sidebarNodes.Add(new SidebarNode
            {
                Path = child,
                Name = name,
                Depth = depth,
                IsDirectory = false
            });
        }
    }

    private void SelectSidebarPath(string path)
    {
        for (int index = 0; index < _sidebarNodes.Count; ++index)
        {
            if (SamePath(_sidebarNodes[index].Path, path))
            {
                _sidebarSelected = index;
                _sidebarSelectionAnchor = index;
                EnsureSidebarSelectionVisible();
                return;
            }
        }
    }

    private void ExpandSidebarToPath(string? path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return;
        }

        path = SidebarFullPath(path);
        string root = SidebarFullPath(ProjectRoot());
        string? directory = Path.GetDirectoryName(path);
        while (!string.IsNullOrEmpty(directory) &&
               !SamePath(directory, root) &&
               IsPathInside(root, directory))
        {
            _sidebarExpanded.Add(directory);
            directory = Path.GetDirectoryName(directory);
        }
    }

    private void UpdateSidebarExpandedAfterRename(string oldPath,
        string newPath)
    {
        var updated = new List<string>();
        foreach (string path in _sidebarExpanded)
        {
            if (SamePath(path, oldPath))
            {
                updated.Add(newPath);
            }
            else if (IsPathInside(oldPath, path))
            {
                updated.Add(Path.Combine(newPath,
                    Path.GetRelativePath(oldPath, path)));
            }
            else
            {
                updated.Add(path);
            }
        }

        _sidebarExpanded.Clear();
        foreach (string path in updated)
        {
            _sidebarExpanded.Add(path);
        }
    }

    private void RemoveSidebarExpandedUnder(string path)
    {
        var remove = new List<string>();
        foreach (string expanded in _sidebarExpanded)
        {
            if (SamePath(expanded, path) || IsPathInside(path, expanded))
            {
                remove.Add(expanded);
            }
        }

        foreach (string expanded in remove)
        {
            _sidebarExpanded.Remove(expanded);
        }
    }

    private void UpdateOpenDocumentPathAfterRename(string oldPath,
        string newPath, bool isDirectory)
    {
        if (_document.Path is null)
        {
            return;
        }

        string documentPath = SidebarFullPath(_document.Path);
        oldPath = SidebarFullPath(oldPath);
        newPath = SidebarFullPath(newPath);
        string? updated = null;
        if (SamePath(documentPath, oldPath))
        {
            updated = newPath;
        }
        else if (isDirectory && IsPathInside(oldPath, documentPath))
        {
            updated = Path.Combine(newPath,
                Path.GetRelativePath(oldPath, documentPath));
        }

        if (updated is null)
        {
            return;
        }

        _document.ChangePath(updated);
        RefreshLanguage();
        RefreshFileStamp();
        ResetLsp();
        FirePluginEvent("buffer_opened", CurrentBufferData());
    }

    private void HandleSidebarMouse(InputEvent inputEvent)
    {
        int width = SidebarWidth();
        if (width <= 0 || inputEvent.MouseX < 0 || inputEvent.MouseX >= width)
        {
            return;
        }

        if (inputEvent.Key == Key.ScrollUp)
        {
            FocusSidebarFromMouse();
            MoveSidebarSelection(-MouseScrollLines, wrap: false);
            return;
        }

        if (inputEvent.Key == Key.ScrollDown)
        {
            FocusSidebarFromMouse();
            MoveSidebarSelection(MouseScrollLines, wrap: false);
            return;
        }

        if (inputEvent.Key != Key.MouseDown ||
            inputEvent.MouseButton != MouseButton.Left)
        {
            return;
        }

        int row = inputEvent.MouseY - 1;
        if (row < 0)
        {
            return;
        }

        int index = _sidebarTop + row;
        if (index < 0 || index >= _sidebarNodes.Count)
        {
            return;
        }

        FocusSidebarFromMouse();
        _sidebarMenuVisible = false;
        SetSidebarSelection(index, extend: inputEvent.Shift);
        ClearMessage();
    }

    private void RenderSidebar()
    {
        if (!_sidebarVisible)
        {
            return;
        }

        int width = SidebarWidth();
        int rows = TextRows();
        if (width <= 0 || rows <= 0)
        {
            return;
        }

        var background = _theme.BaseStyle;
        var headerStyle = _sidebarFocused
            ? _theme.AccentStyle
            : _theme.MutedStyle;
        var borderStyle = _theme.BorderStyle;
        _screen.Fill(0, 0, width, rows, ' ', background);

        _screen.Fill(0, 0, width, 1, ' ', headerStyle);

        for (int row = 0; row < rows; ++row)
        {
            _screen.Put(width - 1, row, '\u2502', borderStyle);
        }

        if (_sidebarNodes.Count == 0)
        {
            var emptyStyle = _theme.MutedStyle;
            _screen.PutString(1, 1, "No files", emptyStyle, width - 2);
            return;
        }

        EnsureSidebarSelectionVisible();
        if (SidebarSelectionCount > 1 && width > 2)
        {
            _screen.PutString(1, 0, $"{SidebarSelectionCount} selected",
                headerStyle, width - 2);
        }

        int listRows = SidebarListRows();
        var selectedStyle = _theme.SelectedItemStyle;
        var folderStyle = _theme.SyntaxStyle(_theme.Accent);
        var fileStyle = _theme.TextStyle;

        for (int row = 0; row < listRows; ++row)
        {
            int index = _sidebarTop + row;
            if (index >= _sidebarNodes.Count)
            {
                break;
            }

            SidebarNode node = _sidebarNodes[index];
            int screenRow = 1 + row;
            bool selected = index >= SidebarSelectionStart &&
                            index <= SidebarSelectionEnd;
            Style style = selected
                ? selectedStyle
                : node.IsDirectory ? folderStyle : fileStyle;

            if (selected)
            {
                _screen.Fill(0, screenRow, width - 1, 1, ' ', style);
            }

            int indent = 1 + node.Depth * 2;
            string marker = node.IsDirectory ? (node.Expanded ? "\u25be" : "\u25b8") : " ";
            _screen.PutString(indent, screenRow, marker, style, width - indent - 1);

            int nameX = indent + 2;
            if (nameX < width - 1)
            {
                _screen.PutString(nameX, screenRow, node.Name, style,
                    width - nameX - 1);
            }
        }
    }

    private void RenderSidebarContextMenu()
    {
        if (!_sidebarMenuVisible || SelectedSidebarNode is not { } node)
        {
            return;
        }

        SidebarMenuItem[] items = SidebarMenuItemsForSelection();
        int width = 4;
        foreach (SidebarMenuItem item in items)
        {
            width = Math.Max(width, item.Label.Length + 7);
        }

        int rows = TextRows();
        int height = items.Length + 2;
        if (rows < 3 || _columns < 10)
        {
            return;
        }

        width = Math.Min(width, _columns - 2);
        int anchorRow = 1 + (_sidebarSelected - _sidebarTop);
        int y = Math.Clamp(anchorRow, 0, Math.Max(0, rows - height));
        int x = 1;

        string title = SidebarSelectionCount > 1
            ? $"{SidebarSelectionCount} selected"
            : node.IsDirectory ? "Folder" : "File";
        DrawBox(x, y, width, height, title);

        var selectedStyle = _theme.SelectedItemStyle;
        var itemStyle = _theme.TextStyle;

        for (int index = 0; index < items.Length; ++index)
        {
            int screenRow = y + 1 + index;
            Style style = index == _sidebarMenuSelected
                ? selectedStyle
                : itemStyle;
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(x + 2, screenRow, items[index].Label, style,
                width - 7);
            _screen.PutString(x + width - 3, screenRow,
                items[index].Hotkey.ToString(), style, 2);
        }
    }
}
