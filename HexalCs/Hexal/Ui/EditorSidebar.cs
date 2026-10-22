using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private static readonly StringComparer SidebarPathComparison =
        OperatingSystem.IsWindows()
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;

    private static readonly string[] SidebarFileMenuItems =
    {
        "Open", "Rename", "Duplicate", "Copy to...", "Delete"
    };

    private static readonly string[] SidebarFolderMenuItems =
    {
        "Expand/Collapse", "Rename", "Duplicate", "Copy to...", "Delete"
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
    private int _sidebarSelected;
    private int _sidebarTop;
    private int _sidebarMenuSelected;
    private int _editorLeft;
    private readonly List<SidebarNode> _sidebarNodes = new();
    private readonly HashSet<string> _sidebarExpanded =
        new(SidebarPathComparison);
    private string _pendingSidebarDeletePath = string.Empty;
    private bool _pendingSidebarDeleteDirectory;

    private SidebarNode? SelectedSidebarNode
        => _sidebarSelected >= 0 && _sidebarSelected < _sidebarNodes.Count
            ? _sidebarNodes[_sidebarSelected]
            : null;

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
                MoveSidebarSelection(-1, wrap: false);
                return;
            case Key.Down:
                MoveSidebarSelection(1, wrap: false);
                return;
            case Key.PageUp:
                MoveSidebarSelection(-SidebarListRows(), wrap: false);
                return;
            case Key.PageDown:
                MoveSidebarSelection(SidebarListRows(), wrap: false);
                return;
            case Key.Home:
                if (_sidebarNodes.Count > 0)
                {
                    _sidebarSelected = 0;
                    EnsureSidebarSelectionVisible();
                }
                return;
            case Key.End:
                if (_sidebarNodes.Count > 0)
                {
                    _sidebarSelected = _sidebarNodes.Count - 1;
                    EnsureSidebarSelectionVisible();
                }
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

    private void MoveSidebarSelection(int delta, bool wrap)
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

        _sidebarSelected = index;
        EnsureSidebarSelectionVisible();
    }

    private void EnsureSidebarSelectionVisible()
    {
        if (_sidebarNodes.Count == 0)
        {
            _sidebarSelected = 0;
            _sidebarTop = 0;
            return;
        }

        _sidebarSelected = Math.Clamp(_sidebarSelected, 0,
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

        if (_sidebarExpanded.Contains(node.Path))
        {
            _sidebarExpanded.Remove(node.Path);
        }
        else
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
            ++_sidebarSelected;
            EnsureSidebarSelectionVisible();
        }
    }

    private void OpenSidebarFile(SidebarNode node)
    {
        _sidebarMenuVisible = false;
        _sidebarFocused = false;
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

    private static string[] SidebarMenuItemsFor(SidebarNode node)
        => node.IsDirectory ? SidebarFolderMenuItems : SidebarFileMenuItems;

    private void HandleSidebarMenu(InputEvent inputEvent)
    {
        string[] items = SelectedSidebarNode is { } node
            ? SidebarMenuItemsFor(node)
            : Array.Empty<string>();

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

        if (inputEvent.Key == Key.Character && !inputEvent.Ctrl &&
            !inputEvent.Alt && inputEvent.Character.Value is 'c' or 'C')
        {
            _sidebarMenuVisible = false;
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
        SidebarNode? node = SelectedSidebarNode;
        if (node is null)
        {
            _sidebarMenuVisible = false;
            return;
        }

        int index = _sidebarMenuSelected;
        _sidebarMenuVisible = false;
        _sidebarFocused = true;

        switch (index)
        {
            case 0:
                if (node.IsDirectory)
                {
                    ToggleSidebarDirectory(node);
                }
                else
                {
                    OpenSidebarFile(node);
                }
                return;
            case 1:
                BeginSidebarRename(node);
                return;
            case 2:
                DuplicateSidebarNode(node);
                return;
            case 3:
                BeginSidebarCopy(node);
                return;
            case 4:
                RequestDeleteSidebarNode(node);
                return;
        }
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

        if (name.IndexOf(Path.DirectorySeparatorChar) >= 0 ||
            name.IndexOf(Path.AltDirectorySeparatorChar) >= 0)
        {
            return false;
        }

        return name.IndexOfAny(Path.GetInvalidFileNameChars()) < 0;
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

    private void DuplicateSidebarNode(SidebarNode node)
    {
        _sidebarFocused = _sidebarVisible;
        string target = UniqueSidebarCopyPath(node.Path, node.IsDirectory);
        if (!CopySidebarPath(node.Path, target, node.IsDirectory, out string error))
        {
            SetError(error);
            RefreshSidebar(node.Path);
            return;
        }

        ExpandSidebarToPath(target);
        RefreshSidebar(target);
        SetMessage($"Duplicated as {Path.GetFileName(target)}");
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

    private void RequestDeleteSidebarNode(SidebarNode node)
    {
        if (SidebarPathContainsOpenDocument(node.Path, node.IsDirectory))
        {
            SetError("Cannot delete the open file or its parent folder.");
            _sidebarFocused = _sidebarVisible;
            return;
        }

        _pendingSidebarDeletePath = node.Path;
        _pendingSidebarDeleteDirectory = node.IsDirectory;
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

    private void DeleteSidebarPath(string path, bool isDirectory)
    {
        _sidebarFocused = _sidebarVisible;
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
            SetError($"Delete failed: {ex.Message}");
            RefreshSidebar(path);
            return;
        }

        RefreshSidebar(null);
        SetMessage($"Deleted {Path.GetFileName(path)}");
    }

    private void ClearPendingSidebarDelete()
    {
        _pendingSidebarDeletePath = string.Empty;
        _pendingSidebarDeleteDirectory = false;
    }

    private void RefreshSidebar(string? selectPath)
    {
        string? selectedPath = selectPath ?? SelectedSidebarNode?.Path;
        _sidebarNodes.Clear();
        AddSidebarDirectory(ProjectRoot(), 0, root: true);

        if (_sidebarNodes.Count == 0)
        {
            _sidebarSelected = 0;
            _sidebarTop = 0;
            return;
        }

        if (selectedPath is not null)
        {
            SelectSidebarPath(selectedPath);
        }

        EnsureSidebarSelectionVisible();
    }

    private void AddSidebarDirectory(string directory, int depth, bool root)
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
                AddSidebarDirectory(child, depth + 1, root: false);
            }
        }

        foreach (string child in files)
        {
            string name = Path.GetFileName(child);
            if (name.Length == 0)
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
        _sidebarSelected = index;
        _sidebarMenuVisible = false;
        EnsureSidebarSelectionVisible();
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

        var background = new Style();
        var headerStyle = new Style
        {
            Fg = _sidebarFocused ? (byte)15 : (byte)8,
            Bold = _sidebarFocused
        };
        var borderStyle = new Style { Fg = 8, Dim = true };
        _screen.Fill(0, 0, width, rows, ' ', background);

        _screen.Fill(0, 0, width, 1, ' ', headerStyle);
        _screen.PutString(1, 0, "EXPLORER", headerStyle, width - 2);

        for (int row = 0; row < rows; ++row)
        {
            _screen.Put(width - 1, row, '\u2502', borderStyle);
        }

        if (_sidebarNodes.Count == 0)
        {
            var emptyStyle = new Style { Fg = 8, Dim = true };
            _screen.PutString(1, 1, "No files", emptyStyle, width - 2);
            return;
        }

        EnsureSidebarSelectionVisible();
        int listRows = SidebarListRows();
        var selectedStyle = new Style { Fg = 15, Bg = 4, Bold = true };
        var folderStyle = new Style { Fg = 14 };
        var fileStyle = new Style { Fg = 15 };

        for (int row = 0; row < listRows; ++row)
        {
            int index = _sidebarTop + row;
            if (index >= _sidebarNodes.Count)
            {
                break;
            }

            SidebarNode node = _sidebarNodes[index];
            int screenRow = 1 + row;
            bool selected = index == _sidebarSelected;
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

        string[] items = SidebarMenuItemsFor(node);
        int width = 4;
        foreach (string item in items)
        {
            width = Math.Max(width, item.Length + 4);
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

        DrawBox(x, y, width, height, node.IsDirectory ? "Folder" : "File");

        var selectedStyle = new Style { Fg = 15, Bg = 4, Bold = true };
        var itemStyle = new Style { Fg = 15 };

        for (int index = 0; index < items.Length; ++index)
        {
            int screenRow = y + 1 + index;
            Style style = index == _sidebarMenuSelected
                ? selectedStyle
                : itemStyle;
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(x + 2, screenRow, items[index], style,
                width - 4);
        }
    }
}
