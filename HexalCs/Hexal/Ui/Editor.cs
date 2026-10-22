using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const int TabWidth = 4;

    private static readonly byte[] Lf = { (byte)'\n' };
    private static readonly byte[] CrLf = { (byte)'\r', (byte)'\n' };

    private enum Mode
    {
        Normal,
        Insert,
        Select,
        Command,
        PluginPrompt
    }

    private enum PromptKind
    {
        None,
        Quit,
        Open
    }

    private enum PickerKind
    {
        None,
        Files,
        Search,
        Plugin,
        CodeActions,
        Problems
    }

    private static readonly (char Key, string Label)[] SpaceMenuItems =
    {
        ('/', "Search across files"),
        ('f', "Open file picker"),
        ('K', "LSP hover"),
        ('A', "LSP code actions"),
        ('D', "Workspace diagnostics"),
        ('d', "Document diagnostics")
    };

    private readonly ITerminal _terminal;
    private readonly Renderer _renderer = new();
    private readonly Screen _screen = new();

    private Document _document;
    private readonly string _workspaceRoot;
    private readonly bool _openFilePickerOnStart;

    private struct FileStamp
    {
        public bool Valid;
        public bool Exists;
        public long Length;
        public long LastWriteTicks;
        public long CreationTicks;
    }

    private FileStamp _fileStamp;
    private bool _externalCheckPending;

    private Mode _mode = Mode.Normal;
    private PromptKind _prompt = PromptKind.None;
    private string _pendingOpen = string.Empty;
    private long _pendingOpenLine = -1;
    private long _pendingOpenColumn = -1;

    private bool _spaceMenuOpen;

    private PickerKind _pickerKind = PickerKind.None;
    private readonly StringBuilder _pickerQuery = new();
    private int _pickerQueryCursor;
    private readonly List<PickerItem> _pickerItems = new();
    private int _pickerSelected;
    private int _pickerTop;
    private bool _pickerLoading;
    private string _pickerStatus = string.Empty;
    private string _pickerRoot = string.Empty;
    private List<string> _pickerFiles = new();
    private bool _pickerFilesLoaded;
    private CancellationTokenSource? _pickerCancellation;
    private Task<PickerScanResult>? _pickerTask;
    private long _pickerGeneration;

    private readonly List<string> _pickerPreviewLines = new();
    private string _pickerPreviewPath = string.Empty;
    private string _pickerPreviewStatus = string.Empty;
    private bool _pickerPreviewBinary;
    private bool _pickerPreviewLoading;
    private CancellationTokenSource? _pickerPreviewCancellation;
    private Task<PickerPreviewResult>? _pickerPreviewTask;
    private long _pickerPreviewGeneration;

    private readonly List<LspCodeAction> _codeActions = new();
    private bool _pendingCodeActionResolve;
    private bool _problemsWorkspace;

    private bool _hoverVisible;
    private readonly List<string> _hoverLines = new();
    private string _hoverTitle = "Hover";
    private int _hoverTop;
    private bool _hoverRequestPending;
    private bool _codeActionRequestPending;

    private sealed class Selection
    {
        public long Cursor;
        public long Anchor;
        public bool Linewise;
        public long PreferredColumn;
        public bool PreferredColumnValid;

        public Selection Clone() => new()
        {
            Cursor = Cursor,
            Anchor = Anchor,
            Linewise = Linewise,
            PreferredColumn = PreferredColumn,
            PreferredColumnValid = PreferredColumnValid
        };
    }

    private long _cursor;
    private long _selectionAnchor;
    private bool _selectionLinewise;
    private readonly List<Selection> _extraSelections = new();
    private long _topLine;
    private long _leftColumn;
    private long _preferredColumn;
    private bool _preferredColumnValid;

    private int _columns = 80;
    private int _rows = 24;
    private bool _running = true;
    private bool _forceFullRedraw = true;

    private string _message = string.Empty;
    private bool _messageIsError;

    private readonly StringBuilder _commandLine = new();
    private int _commandCursor;
    private readonly List<string> _commandHistory = new();
    private int _commandHistoryIndex = -1;

    private readonly List<long> _jumpHistory = new();
    private bool _pendingG;

    private byte[] _lineBuffer = Array.Empty<byte>();

    private readonly LanguageRegistry _languages;
    private readonly SyntaxHighlighter _syntax = new();
    private readonly List<SyntaxToken> _lineTokens = new();
    private LanguageDefinition? _language;

    private sealed class PickerItem
    {
        public string Display = string.Empty;
        public string Path = string.Empty;
        public long Line = -1;
        public long Column;
        public int Value;
    }

    private sealed class PickerScanResult
    {
        public long Generation;
        public bool FileList;
        public List<string> Files = new();
        public List<FileSearchMatch> Matches = new();
        public string Error = string.Empty;
    }

    private sealed class PickerPreviewResult
    {
        public long Generation;
        public string Path = string.Empty;
        public FilePreview Preview = new();
    }

    private sealed class SaveOperation
    {
        public Task<SaveResult> Task = null!;
        public Document Document = null!;
        public Document.SaveSnapshot Snapshot = null!;
    }

    private sealed class SaveResult
    {
        public bool Ok;
        public string Error = string.Empty;
        public string Path = string.Empty;
        public long Pose;
        public long Revision;
    }

    private SaveOperation? _saveOperation;
    private bool _quitAfterSave;
    private bool _resaveRequested;
    private bool _waitThenQuit;
    private bool _waitThenQuitForce;

    public Editor(ITerminal terminal, Document document,
        string? workspaceRoot = null, bool openFilePicker = false)
    {
        _terminal = terminal;
        _document = document;
        _workspaceRoot = ResolveWorkspaceRoot(workspaceRoot);
        _openFilePickerOnStart = openFilePicker;
        _languages = LanguageRegistry.LoadUserGrammars();
        RefreshLanguage();
        RefreshFileStamp();
        SnapCursor();

        if (_languages.Errors.Count > 0)
        {
            SetError($"grammar: {_languages.Errors[0]}");
        }
    }

    private static string ResolveWorkspaceRoot(string? workspaceRoot)
    {
        if (!string.IsNullOrWhiteSpace(workspaceRoot))
        {
            try
            {
                if (Directory.Exists(workspaceRoot))
                {
                    return Path.GetFullPath(workspaceRoot);
                }
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                       PathTooLongException or UnauthorizedAccessException or
                                       IOException)
            {
                // Fall back to the current directory below.
            }
        }

        try
        {
            return Directory.GetCurrentDirectory();
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            return ".";
        }
    }

    public int Run()
    {
        LoadPlugins();
        RefreshSize();
        EnsureCursorVisible();
        if (_openFilePickerOnStart)
        {
            OpenPicker(PickerKind.Files);
        }
        Render();

        while (_running)
        {
            if (PollBackgroundWork())
            {
                Render();
            }
            if (!_running)
            {
                break;
            }

            // Pending background work needs a periodic wake-up so the UI can
            // report completion. Idle with no work uses a blocking read and
            // consumes no CPU.
            bool busy = _saveOperation is not null || _pickerTask is not null ||
                        _lsp.IsRunning || _lspDirty;
            int timeout = busy ? 50 : -1;
            if (!_terminal.ReadEvent(out InputEvent inputEvent, timeout))
            {
                break;
            }

            if (inputEvent.Key == Key.None)
            {
                if (PollBackgroundWork())
                {
                    Render();
                }
                continue;
            }

            Document documentBefore = _document;
            long revisionBefore = _document.Revision;
            long cursorBefore = _cursor;

            if (!HandleEvent(inputEvent))
            {
                break;
            }

            EmitInputEvents(documentBefore, revisionBefore, cursorBefore);
            EnsureCursorVisible();
            Render();
        }

        if (_saveOperation is not null)
        {
            _quitAfterSave = false;
            _resaveRequested = false;
            _waitThenQuit = false;
            _waitThenQuitForce = false;
            SetMessage("Waiting for save to finish...");
            Render();

            while (_saveOperation is not null)
            {
                Thread.Sleep(20);
                PollSave();
            }
        }

        UnloadPlugins();
        return 0;
    }

    private void RefreshSize()
    {
        _terminal.GetSize(out int columns, out int rows);
        _columns = Math.Max(1, columns);
        _rows = Math.Max(1, rows);
    }

    private bool HandleEvent(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Resize)
        {
            RefreshSize();
            _forceFullRedraw = true;
            EnsureCursorVisible();
            return _running;
        }

        if (inputEvent.Key == Key.FocusIn)
        {
            HandleFocusGained();
            return _running;
        }

        if (inputEvent.Key == Key.FocusOut)
        {
            return _running;
        }

        if (_hoverVisible)
        {
            HandleHoverInput(inputEvent);
            return _running;
        }

        if (_plugins is not null &&
            _prompt == PromptKind.None &&
            _pickerKind == PickerKind.None &&
            !_spaceMenuOpen &&
            _mode is Mode.Normal or Mode.Insert or Mode.Select)
        {
            string keymapMode = _mode switch
            {
                Mode.Insert => "insert",
                Mode.Select => "select",
                _ => "normal"
            };

            if (_plugins.TryHandleKey(inputEvent, keymapMode, out bool handled) && handled)
            {
                return _running;
            }
        }

        try
        {
            if (_pickerKind != PickerKind.None)
            {
                HandlePicker(inputEvent);
            }
            else if (_spaceMenuOpen)
            {
                HandleSpaceMenu(inputEvent);
            }
            else if (_prompt != PromptKind.None)
            {
                HandlePrompt(inputEvent);
            }
            else if (_mode == Mode.PluginPrompt)
            {
                HandlePluginPrompt(inputEvent);
            }
            else if (_mode == Mode.Command)
            {
                HandleCommandInput(inputEvent);
            }
            else if (_mode == Mode.Insert)
            {
                HandleInsert(inputEvent);
            }
            else if (_mode == Mode.Select)
            {
                HandleSelect(inputEvent);
            }
            else
            {
                HandleNormal(inputEvent);
            }
        }
        catch (Exception ex)
        {
            // Keep the UI alive on allocation or I/O exceptions. Exiting
            // unexpectedly would risk losing more user work.
            SetError($"internal error: {ex.Message}");
        }

        return _running;
    }

    // -----------------------------------------------------------------------
    // EXTERNAL FILE CHANGES
    // -----------------------------------------------------------------------

    private void HandleFocusGained()
    {
        // A save can be writing the file while focus changes. Defer the stat
        // until the writer has finished so our own output is not mistaken for
        // an external modification.
        if (_saveOperation is not null)
        {
            _externalCheckPending = true;
            return;
        }

        CheckExternalFile();
    }

    /// <summary>
    /// Checks a single stat record captured on focus. The common case is one
    /// metadata lookup and no file I/O at all.
    /// </summary>
    private bool CheckExternalFile()
    {
        if (_document.Path is null)
        {
            return false;
        }

        if (!_fileStamp.Valid)
        {
            RefreshFileStamp();
            return false;
        }

        if (!TryCaptureFileStamp(_document.Path, out FileStamp current))
        {
            return false;
        }

        if (current.Exists == _fileStamp.Exists &&
            current.Length == _fileStamp.Length &&
            current.LastWriteTicks == _fileStamp.LastWriteTicks &&
            current.CreationTicks == _fileStamp.CreationTicks)
        {
            return false;
        }

        return ReloadFromDisk(_document.Path, current);
    }

    private bool ReloadFromDisk(string path, FileStamp stamp)
    {
        long oldLine = _document.LineOfOffset(_cursor);
        long oldColumn = CursorVisualColumn();
        long oldTopOffset = oldLine - _topLine;
        bool wasDirty = _document.IsDirty;

        Document? loaded = Document.Open(path, out string error);
        if (loaded is null)
        {
            SetError($"external change: {error}");
            return false;
        }

        ResetLsp();
        _document = loaded;
        RefreshLanguage();

        if (_mode == Mode.Select)
        {
            _mode = Mode.Normal;
            _selectionLinewise = false;
        }

        _pendingG = false;
        _jumpHistory.Clear();
        _extraSelections.Clear();

        long line = Math.Clamp(oldLine, 0, _document.LineCount - 1);
        long lineStart = _document.LineStart(line);
        long lineEnd = _document.LineEnd(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        long column = UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, length), oldColumn, TabWidth);

        _cursor = Math.Clamp(lineStart + column, lineStart, lineEnd);
        _selectionAnchor = _cursor;
        _preferredColumnValid = false;
        _topLine = Math.Max(0, line - Math.Max(0, oldTopOffset));
        SnapCursor();
        _fileStamp = stamp;

        if (wasDirty)
        {
            SetMessage($"Reloaded {_document.DisplayName} (external change; unsaved edits replaced)");
        }
        else
        {
            SetMessage($"Reloaded {_document.DisplayName} (changed on disk)");
        }

        FirePluginEvent("buffer_opened", new Dictionary<string, object?>
        {
            ["path"] = path,
            ["id"] = CurrentBufferId,
            ["reloaded"] = true
        });
        return true;
    }

    private void RefreshFileStamp()
    {
        _fileStamp = default;
        if (_document.Path is null)
        {
            return;
        }

        if (TryCaptureFileStamp(_document.Path, out FileStamp stamp))
        {
            _fileStamp = stamp;
        }
    }

    private static bool TryCaptureFileStamp(string path, out FileStamp stamp)
    {
        stamp = default;

        try
        {
            var info = new FileInfo(path);
            info.Refresh();

            stamp.Valid = true;
            stamp.Exists = info.Exists;
            if (!info.Exists)
            {
                return true;
            }

            stamp.Length = info.Length;
            stamp.LastWriteTicks = info.LastWriteTimeUtc.Ticks;
            stamp.CreationTicks = info.CreationTimeUtc.Ticks;
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            return false;
        }
    }

    // -----------------------------------------------------------------------
    // NORMAL MODE
    // -----------------------------------------------------------------------

    private void HandleNormal(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            _pendingG = false;
            uint character = (uint)inputEvent.Character.Value;
            if (character == 'r')
            {
                Redo();
            }
            else if (character == 'c')
            {
                ToggleComment();
            }
            else if (character == 'w')
            {
                SetMessage("Pane navigation is not part of the MVP.");
            }
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Alt)
        {
            _pendingG = false;
            return;
        }

        if (_pendingG)
        {
            _pendingG = false;
            if (inputEvent.Key == Key.Character)
            {
                uint character = (uint)inputEvent.Character.Value;
                if (character is 'g' or 'G')
                {
                    ForEachSelection(() => MoveToDocumentStart(recordJump: true));
                }
                else if (character is 'e' or 'E')
                {
                    ForEachSelection(() => MoveToDocumentEnd(recordJump: true));
                }
                else if (character is 'p' or 'P')
                {
                    MoveToPreviousLocation();
                }
            }
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Left:
                ForEachSelection(MoveLeft);
                return;
            case Key.Right:
                ForEachSelection(MoveRight);
                return;
            case Key.Up:
                ForEachSelection(() => MoveVertical(-1));
                return;
            case Key.Down:
                ForEachSelection(() => MoveVertical(1));
                return;
            case Key.PageUp:
                ForEachSelection(() => MovePage(-1));
                return;
            case Key.PageDown:
                ForEachSelection(() => MovePage(1));
                return;
            case Key.Home:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentStart(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineStartOfCursor(), recordJump: false));
                }
                return;
            case Key.End:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentEnd(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineEndOfCursor(), recordJump: false));
                }
                return;
            case Key.Escape:
                _document.BreakUndoGroup();
                _hoverRequestPending = false;
                _codeActionRequestPending = false;
                ClearMessage();
                return;
            case Key.Delete:
                ForEachSelection(DeleteAtCursor, editing: true);
                return;
        }

        if (inputEvent.Key != Key.Character)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
            case ' ':
                OpenSpaceMenu();
                return;
            case ':':
                _mode = Mode.Command;
                _commandLine.Clear();
                _commandCursor = 0;
                _commandHistoryIndex = -1;
                _document.BreakUndoGroup();
                ClearMessage();
                return;
            case 'i':
            case 'I':
                EnterInsert();
                return;
            case 'a':
                ForEachSelection(() =>
                {
                    long end = LineEndOfCursor();
                    if (_cursor < end)
                    {
                        long line = CursorLine();
                        long begin = _document.LineStart(line);
                        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
                        int local = (int)(_cursor - begin);
                        _cursor = begin +
                                  UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
                    }
                    EnterInsert();
                });
                return;
            case 'A':
                ForEachSelection(() =>
                {
                    _cursor = LineEndOfCursor();
                    _preferredColumnValid = false;
                    EnterInsert();
                });
                return;
            case 'o':
                ForEachSelection(InsertNewlineBelow, editing: true);
                return;
            case 'O':
                ForEachSelection(InsertNewlineAbove, editing: true);
                return;
            case 'h':
                ForEachSelection(MoveLeft);
                return;
            case 'l':
                ForEachSelection(MoveRight);
                return;
            case 'j':
                ForEachSelection(() => MoveVertical(1));
                return;
            case 'k':
                ForEachSelection(() => MoveVertical(-1));
                return;
            case 'd':
            case 'D':
                ForEachSelection(DeleteAtCursor, editing: true);
                return;
            case 'u':
                Undo();
                return;
            case 'g':
            case 'G':
                _pendingG = true;
                return;
            case 'v':
            case 'V':
                ForEachSelection(StartCharacterSelection);
                return;
            case 'x':
            case 'X':
                ForEachSelection(StartLineSelection);
                return;
            case 'C':
                AddCursorBelow();
                return;
            default:
                return;
        }
    }

    // -----------------------------------------------------------------------
    // SELECT MODE
    // -----------------------------------------------------------------------

    private void HandleSelect(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            _pendingG = false;
            if (inputEvent.Character.Value == 'r')
            {
                CancelSelection();
                Redo();
            }
            else if (inputEvent.Character.Value == 'c')
            {
                ToggleComment();
            }
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Alt)
        {
            _pendingG = false;
            return;
        }

        if (_pendingG)
        {
            _pendingG = false;
            if (inputEvent.Key == Key.Character)
            {
                uint character = (uint)inputEvent.Character.Value;
                if (character is 'g' or 'G')
                {
                    ForEachSelection(() => MoveToDocumentStart(recordJump: true));
                }
                else if (character is 'e' or 'E')
                {
                    ForEachSelection(() => MoveToDocumentEnd(recordJump: true));
                }
                else if (character is 'p' or 'P')
                {
                    MoveToPreviousLocation();
                }
            }
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Left:
                ForEachSelection(MoveLeft);
                return;
            case Key.Right:
                ForEachSelection(MoveRight);
                return;
            case Key.Up:
                ForEachSelection(() => MoveVertical(-1));
                return;
            case Key.Down:
                ForEachSelection(() => MoveVertical(1));
                return;
            case Key.PageUp:
                ForEachSelection(() => MovePage(-1));
                return;
            case Key.PageDown:
                ForEachSelection(() => MovePage(1));
                return;
            case Key.Home:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentStart(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineStartOfCursor(), recordJump: false));
                }
                return;
            case Key.End:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentEnd(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineEndOfCursor(), recordJump: false));
                }
                return;
            case Key.Escape:
                CancelSelection();
                return;
            case Key.Delete:
                ForEachSelection(DeleteSelection, editing: true);
                return;
        }

        if (inputEvent.Key != Key.Character)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
            case 'x':
            case 'X':
                ForEachSelection(ExtendLineSelection);
                return;
            case 'd':
            case 'D':
                ForEachSelection(DeleteSelection, editing: true);
                return;
            case 'h':
                ForEachSelection(MoveLeft);
                return;
            case 'l':
                ForEachSelection(MoveRight);
                return;
            case 'j':
                ForEachSelection(() => MoveVertical(1));
                return;
            case 'k':
                ForEachSelection(() => MoveVertical(-1));
                return;
            case 'g':
            case 'G':
                _pendingG = true;
                return;
            case 'C':
                AddCursorBelow();
                return;
            case 'u':
                CancelSelection();
                Undo();
                return;
        }
    }

    // -----------------------------------------------------------------------
    // INSERT MODE
    // -----------------------------------------------------------------------

    private void HandleInsert(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            _mode = Mode.Normal;
            _pendingG = false;
            _document.BreakUndoGroup();
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            if (inputEvent.Character.Value == 'r')
            {
                Redo();
            }
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Left:
                ForEachSelection(MoveLeft);
                return;
            case Key.Right:
                ForEachSelection(MoveRight);
                return;
            case Key.Up:
                ForEachSelection(() => MoveVertical(-1));
                return;
            case Key.Down:
                ForEachSelection(() => MoveVertical(1));
                return;
            case Key.PageUp:
                ForEachSelection(() => MovePage(-1));
                return;
            case Key.PageDown:
                ForEachSelection(() => MovePage(1));
                return;
            case Key.Home:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentStart(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineStartOfCursor(), recordJump: false));
                }
                return;
            case Key.End:
                if (inputEvent.Ctrl)
                {
                    ForEachSelection(() => MoveToDocumentEnd(recordJump: true));
                }
                else
                {
                    ForEachSelection(() => MoveTo(LineEndOfCursor(), recordJump: false));
                }
                return;
            case Key.Enter:
                ForEachSelection(InsertNewline, editing: true);
                return;
            case Key.Backspace:
                ForEachSelection(Backspace, editing: true);
                return;
            case Key.Delete:
                ForEachSelection(DeleteForward, editing: true);
                return;
            case Key.Tab:
                ForEachSelection(() => InsertCharacter(new Rune('\t')), editing: true);
                return;
        }

        if (inputEvent.Key == Key.Character && !inputEvent.Ctrl && !inputEvent.Alt)
        {
            ForEachSelection(() => InsertCharacter(inputEvent.Character), editing: true);
        }
    }

    // -----------------------------------------------------------------------
    // COMMAND MODE
    // -----------------------------------------------------------------------

    private void HandleCommandInput(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Escape:
                _mode = Mode.Normal;
                _commandLine.Clear();
                _commandCursor = 0;
                _commandHistoryIndex = -1;
                return;
            case Key.Enter:
            {
                string line = _commandLine.ToString();
                _mode = Mode.Normal;
                _commandLine.Clear();
                _commandCursor = 0;
                _commandHistoryIndex = -1;

                if (line.Length > 0 &&
                    (_commandHistory.Count == 0 || _commandHistory[^1] != line))
                {
                    _commandHistory.Add(line);
                    if (_commandHistory.Count > 100)
                    {
                        _commandHistory.RemoveAt(0);
                    }
                }

                ExParseResult parsed = ExCommandParser.Parse(line);
                if (!parsed.Ok)
                {
                    SetError(parsed.Error);
                    return;
                }
                if (parsed.Command.Name.Length == 0)
                {
                    return;
                }

                ExecuteParsedCommand(parsed.Command, force: false);
                return;
            }
            case Key.Backspace:
                if (_commandCursor > 0)
                {
                    string text = _commandLine.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, _commandCursor);
                    _commandLine.Remove(previous, _commandCursor - previous);
                    _commandCursor = previous;
                }
                return;
            case Key.Delete:
                if (_commandCursor < _commandLine.Length)
                {
                    string text = _commandLine.ToString();
                    int next = UnicodeUtil.NextBoundary(text, _commandCursor);
                    _commandLine.Remove(_commandCursor, next - _commandCursor);
                }
                return;
            case Key.Left:
            {
                string text = _commandLine.ToString();
                _commandCursor = UnicodeUtil.PreviousBoundary(text, _commandCursor);
                return;
            }
            case Key.Right:
            {
                string text = _commandLine.ToString();
                _commandCursor = UnicodeUtil.NextBoundary(text, _commandCursor);
                return;
            }
            case Key.Home:
                _commandCursor = 0;
                return;
            case Key.End:
                _commandCursor = _commandLine.Length;
                return;
            case Key.Up:
            case Key.Down:
                if (_commandHistory.Count == 0)
                {
                    return;
                }

                if (inputEvent.Key == Key.Up)
                {
                    if (_commandHistoryIndex < 0)
                    {
                        _commandHistoryIndex = _commandHistory.Count - 1;
                    }
                    else if (_commandHistoryIndex > 0)
                    {
                        --_commandHistoryIndex;
                    }

                    _commandLine.Clear();
                    _commandLine.Append(_commandHistory[_commandHistoryIndex]);
                }
                else
                {
                    if (_commandHistoryIndex < 0)
                    {
                        return;
                    }

                    ++_commandHistoryIndex;
                    if (_commandHistoryIndex >= _commandHistory.Count)
                    {
                        _commandHistoryIndex = -1;
                        _commandLine.Clear();
                    }
                    else
                    {
                        _commandLine.Clear();
                        _commandLine.Append(_commandHistory[_commandHistoryIndex]);
                    }
                }

                _commandCursor = _commandLine.Length;
                return;
        }

        if (inputEvent.Key == Key.Character && !inputEvent.Alt)
        {
            string inserted = inputEvent.Character.ToString();
            _commandLine.Insert(_commandCursor, inserted);
            _commandCursor += inserted.Length;
        }
    }

    private void HandlePrompt(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            _prompt = PromptKind.None;
            ClearPendingOpen();
            SetMessage("Cancelled.");
            return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl || inputEvent.Alt)
        {
            return;
        }

        uint character = (uint)inputEvent.Character.Value;
        if (character is 'y' or 'Y')
        {
            PromptKind kind = _prompt;
            _prompt = PromptKind.None;

            if (kind == PromptKind.Quit)
            {
                _running = false;
            }
            else if (kind == PromptKind.Open)
            {
                string path = _pendingOpen;
                long line = _pendingOpenLine;
                long column = _pendingOpenColumn;
                ClearPendingOpen();
                OpenPathAt(path, line, column);
            }
            return;
        }

        if (character is 'n' or 'N')
        {
            _prompt = PromptKind.None;
            ClearPendingOpen();
            SetMessage("Cancelled.");
        }
    }

    private void ClearPendingOpen()
    {
        _pendingOpen = string.Empty;
        _pendingOpenLine = -1;
        _pendingOpenColumn = -1;
    }

    // -----------------------------------------------------------------------
    // SPACE MENU AND PICKERS
    // -----------------------------------------------------------------------

    private void OpenSpaceMenu()
    {
        _spaceMenuOpen = true;
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void HandleSpaceMenu(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            _spaceMenuOpen = false;
            return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Ctrl || inputEvent.Alt)
        {
            return;
        }

        _spaceMenuOpen = false;
        switch (inputEvent.Character.Value)
        {
            case '/':
                OpenPicker(PickerKind.Search);
                return;
            case 'f':
            case 'F':
                OpenPicker(PickerKind.Files);
                return;
            case 'K':
            case 'k':
                ShowHover();
                return;
            case 'A':
            case 'a':
                ShowCodeActions();
                return;
            case 'D':
                ShowProblems(workspace: true);
                return;
            case 'd':
                ShowProblems(workspace: false);
                return;
            default:
                // Helix closes the which-key menu when an unbound key is
                // pressed; the key does not fall through to NORMAL mode.
                return;
        }
    }

    private void OpenPicker(PickerKind kind)
    {
        CancelPickerPreview();
        _pickerKind = kind;
        _pickerQuery.Clear();
        _pickerQueryCursor = 0;
        _pickerItems.Clear();
        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerStatus = string.Empty;
        _pickerLoading = true;
        _pickerRoot = ProjectRoot();
        _pickerPreviewLines.Clear();
        _pickerPreviewPath = string.Empty;
        _pickerPreviewStatus = string.Empty;
        _pickerPreviewBinary = false;
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();

        if (kind == PickerKind.Plugin || kind == PickerKind.CodeActions ||
            kind == PickerKind.Problems)
        {
            BeginPickerQuery();
        }
        else if (!_pickerFilesLoaded)
        {
            StartPickerFileScan();
        }
        else
        {
            BeginPickerQuery();
        }
    }

    private void ClosePicker(bool cancelled = true)
    {
        PickerKind oldKind = _pickerKind;
        Action<int?>? pluginCallback = oldKind == PickerKind.Plugin
            ? _pluginSelectCallback
            : null;
        _pluginSelectCallback = null;

        CancelPickerWork();
        CancelPickerPreview();
        _pickerKind = PickerKind.None;
        _pickerQuery.Clear();
        _pickerQueryCursor = 0;
        _pickerItems.Clear();
        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = string.Empty;
        _pickerPreviewLines.Clear();
        _pickerPreviewPath = string.Empty;
        _pickerPreviewStatus = string.Empty;
        _codeActions.Clear();
        _pendingCodeActionResolve = false;
        _pluginSelectTitle = string.Empty;
        _pluginSelectItems.Clear();

        if (cancelled && pluginCallback is not null)
        {
            pluginCallback(null);
        }
    }

    private void StartPickerFileScan()
    {
        CancelPickerWork();

        long generation = ++_pickerGeneration;
        var cancellation = new CancellationTokenSource();
        _pickerCancellation = cancellation;
        string root = _pickerRoot;

        _pickerTask = Task.Run(() =>
        {
            var result = new PickerScanResult
            {
                Generation = generation,
                FileList = true
            };

            try
            {
                result.Files = FileSearch.EnumerateFiles(root, FileSearch.DefaultMaxFiles,
                    cancellation.Token);
            }
            catch (OperationCanceledException)
            {
                // Superseded by a newer picker operation.
            }
            catch (Exception ex)
            {
                result.Error = ex.Message;
            }

            return result;
        }, cancellation.Token);
    }

    private void BeginPickerQuery()
    {
        switch (_pickerKind)
        {
            case PickerKind.Files:
                PopulateFileItems();
                return;
            case PickerKind.Plugin:
                PopulatePluginItems();
                return;
            case PickerKind.CodeActions:
                PopulateCodeActionItems();
                return;
            case PickerKind.Problems:
                PopulateProblemItems();
                return;
            case PickerKind.Search:
                if (_pickerQuery.Length == 0)
                {
                    _pickerItems.Clear();
                    _pickerSelected = 0;
                    _pickerTop = 0;
                    _pickerLoading = false;
                    _pickerStatus = "Type to search";
                    return;
                }

                StartPickerSearch();
                return;
        }
    }

    private void PopulateFileItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        if (query.Length == 0)
        {
            foreach (string path in _pickerFiles)
            {
                _pickerItems.Add(new PickerItem
                {
                    Display = path,
                    Path = Path.GetFullPath(Path.Combine(_pickerRoot, path))
                });
            }
        }
        else
        {
            var scored = new List<(int Score, PickerItem Item)>();
            foreach (string path in _pickerFiles)
            {
                int score = FuzzyScore(path, query);
                if (score < int.MaxValue)
                {
                    scored.Add((score, new PickerItem
                    {
                        Display = path,
                        Path = Path.GetFullPath(Path.Combine(_pickerRoot, path))
                    }));
                }
            }

            scored.Sort((left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(left.Item.Display, right.Item.Display,
                        StringComparison.OrdinalIgnoreCase);
            });

            foreach ((int _, PickerItem item) in scored)
            {
                _pickerItems.Add(item);
            }
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = _pickerItems.Count == 0
            ? "No matching files"
            : $"{_pickerItems.Count} file{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        UpdatePickerPreview();
    }

    private void PopulatePluginItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        if (query.Length == 0)
        {
            for (int i = 0; i < _pluginSelectItems.Count; ++i)
            {
                _pickerItems.Add(new PickerItem
                {
                    Display = _pluginSelectItems[i],
                    Value = i
                });
            }
        }
        else
        {
            var scored = new List<(int Score, PickerItem Item)>();
            for (int i = 0; i < _pluginSelectItems.Count; ++i)
            {
                string item = _pluginSelectItems[i];
                int score = FuzzyScore(item, query);
                if (score < int.MaxValue)
                {
                    scored.Add((score, new PickerItem
                    {
                        Display = item,
                        Value = i
                    }));
                }
            }

            scored.Sort((left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(left.Item.Display, right.Item.Display,
                        StringComparison.OrdinalIgnoreCase);
            });

            foreach ((int _, PickerItem item) in scored)
            {
                _pickerItems.Add(item);
            }
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = _pickerItems.Count == 0
            ? "No matching items"
            : $"{_pickerItems.Count} item{(_pickerItems.Count == 1 ? string.Empty : "s")}";
    }

    private void PopulateCodeActionItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        for (int i = 0; i < _codeActions.Count; ++i)
        {
            LspCodeAction action = _codeActions[i];
            string label = action.Kind.Length > 0
                ? $"[{action.Kind}] {action.Title}"
                : action.Title;

            if (query.Length > 0 && FuzzyScore(label, query) == int.MaxValue)
            {
                continue;
            }

            _pickerItems.Add(new PickerItem
            {
                Display = label,
                Value = i
            });
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = _pickerItems.Count == 0
            ? "No matching code actions"
            : $"{_pickerItems.Count} code action{(_pickerItems.Count == 1 ? string.Empty : "s")}";
    }

    private void PopulateProblemItems()
    {
        _pickerItems.Clear();
        string query = _pickerQuery.ToString();

        foreach ((string path, LspDiagnostic diagnostic) in CollectProblems())
        {
            string relative;
            try
            {
                relative = Path.GetRelativePath(_pickerRoot, path);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException)
            {
                relative = path;
            }

            string severity = diagnostic.Severity switch
            {
                1 => "E",
                2 => "W",
                3 => "I",
                _ => "H"
            };

            string label =
                $"{severity} {relative}:{diagnostic.Line + 1}:{diagnostic.Column + 1}  {diagnostic.Message}";
            if (query.Length > 0 && FuzzyScore(label, query) == int.MaxValue)
            {
                continue;
            }

            _pickerItems.Add(new PickerItem
            {
                Display = label,
                Path = path,
                Line = diagnostic.Line,
                Column = diagnostic.Column,
                Value = diagnostic.Severity
            });
        }

        _pickerSelected = Math.Clamp(_pickerSelected, 0,
            Math.Max(0, _pickerItems.Count - 1));
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = _pickerItems.Count == 0
            ? "No problems reported"
            : $"{_pickerItems.Count} problem{(_pickerItems.Count == 1 ? string.Empty : "s")}";
    }

    private List<(string Path, LspDiagnostic Diagnostic)> CollectProblems()
    {
        var result = new List<(string Path, LspDiagnostic Diagnostic)>();
        string root = Path.GetFullPath(_pickerRoot);
        string currentPath = _document.Path is null
            ? string.Empty
            : Path.GetFullPath(_document.Path);

        foreach (LspDocumentDiagnostics report in _lsp.SnapshotDiagnostics())
        {
            string path;
            try
            {
                path = Path.GetFullPath(LspManager.UriToPath(report.Uri));
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                       PathTooLongException)
            {
                continue;
            }

            if (!_problemsWorkspace)
            {
                if (currentPath.Length == 0 ||
                    !PathsEqual(path, currentPath))
                {
                    continue;
                }
            }
            else if (!IsPathInside(root, path))
            {
                continue;
            }

            foreach (LspDiagnostic diagnostic in report.Diagnostics)
            {
                result.Add((path, diagnostic));
            }
        }

        result.Sort((left, right) =>
        {
            int comparison = string.Compare(left.Path, right.Path,
                StringComparison.OrdinalIgnoreCase);
            if (comparison != 0)
            {
                return comparison;
            }
            return left.Diagnostic.Line.CompareTo(right.Diagnostic.Line);
        });

        return result;
    }

    private static bool PathsEqual(string left, string right)
    {
        return string.Equals(left, right, OperatingSystem.IsWindows()
            ? StringComparison.OrdinalIgnoreCase
            : StringComparison.Ordinal);
    }

    private static bool IsPathInside(string root, string path)
    {
        string normalizedRoot = root.TrimEnd(Path.DirectorySeparatorChar,
            Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;
        string comparisonPath = path;
        if (!comparisonPath.EndsWith(Path.DirectorySeparatorChar))
        {
            comparisonPath += Path.DirectorySeparatorChar;
        }

        return comparisonPath.StartsWith(normalizedRoot,
            OperatingSystem.IsWindows()
                ? StringComparison.OrdinalIgnoreCase
                : StringComparison.Ordinal);
    }

    private void UpdatePickerPreview()
    {
        if (_pickerKind != PickerKind.Files)
        {
            CancelPickerPreview();
            return;
        }

        if (_pickerSelected < 0 || _pickerSelected >= _pickerItems.Count)
        {
            CancelPickerPreview();
            _pickerPreviewLines.Clear();
            _pickerPreviewPath = string.Empty;
            _pickerPreviewStatus = "No file selected";
            return;
        }

        PickerItem item = _pickerItems[_pickerSelected];
        if (item.Path.Length == 0)
        {
            return;
        }

        if (PathsEqual(_pickerPreviewPath, item.Path) &&
            (_pickerPreviewLoading || _pickerPreviewStatus.Length > 0 ||
             _pickerPreviewLines.Count > 0))
        {
            return;
        }

        StartPickerPreviewLoad(item.Path);
    }

    private void StartPickerPreviewLoad(string path)
    {
        CancelPickerPreview();

        long generation = ++_pickerPreviewGeneration;
        var cancellation = new CancellationTokenSource();
        _pickerPreviewCancellation = cancellation;
        _pickerPreviewPath = path;
        _pickerPreviewLines.Clear();
        _pickerPreviewBinary = false;
        _pickerPreviewStatus = string.Empty;
        _pickerPreviewLoading = true;

        _pickerPreviewTask = Task.Run(() => new PickerPreviewResult
        {
            Generation = generation,
            Path = path,
            Preview = FileSearch.ReadPreview(path)
        }, cancellation.Token);
    }

    private void CancelPickerPreview()
    {
        _pickerPreviewCancellation?.Cancel();
        _pickerPreviewCancellation?.Dispose();
        _pickerPreviewCancellation = null;
        _pickerPreviewTask = null;
        _pickerPreviewLoading = false;
    }

    private bool PollPickerPreview()
    {
        Task<PickerPreviewResult>? task = _pickerPreviewTask;
        if (task is null || !task.IsCompleted)
        {
            return false;
        }

        _pickerPreviewTask = null;
        _pickerPreviewCancellation?.Dispose();
        _pickerPreviewCancellation = null;

        PickerPreviewResult result;
        try
        {
            result = task.GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            return false;
        }
        catch (Exception ex)
        {
            _pickerPreviewLoading = false;
            _pickerPreviewStatus = $"E: {ex.Message}";
            return true;
        }

        if (result.Generation != _pickerPreviewGeneration)
        {
            return false;
        }

        _pickerPreviewLoading = false;
        _pickerPreviewPath = result.Path;
        _pickerPreviewLines.Clear();
        _pickerPreviewLines.AddRange(result.Preview.Lines);
        _pickerPreviewBinary = result.Preview.Binary;

        if (result.Preview.Error.Length > 0)
        {
            _pickerPreviewStatus = $"E: {result.Preview.Error}";
        }
        else if (result.Preview.Binary)
        {
            _pickerPreviewStatus = "Binary file";
        }
        else if (result.Preview.Lines.Count == 0)
        {
            _pickerPreviewStatus = "Empty file";
        }
        else
        {
            _pickerPreviewStatus = result.Preview.Truncated
                ? $"First {result.Preview.Lines.Count} lines (truncated)"
                : $"First {result.Preview.Lines.Count} line{(result.Preview.Lines.Count == 1 ? string.Empty : "s")}";
        }

        return true;
    }

    private void StartPickerSearch()
    {
        CancelPickerWork();

        long generation = ++_pickerGeneration;
        var cancellation = new CancellationTokenSource();
        _pickerCancellation = cancellation;
        string root = _pickerRoot;
        string query = _pickerQuery.ToString();
        List<string> files = _pickerFiles;

        _pickerLoading = true;
        _pickerStatus = "Searching...";

        _pickerTask = Task.Run(() =>
        {
            var result = new PickerScanResult
            {
                Generation = generation
            };

            try
            {
                result.Matches = FileSearch.Search(root, files, query,
                    FileSearch.DefaultMaxResults, cancellation.Token);
            }
            catch (OperationCanceledException)
            {
                // Superseded by a newer query.
            }
            catch (Exception ex)
            {
                result.Error = ex.Message;
            }

            return result;
        }, cancellation.Token);
    }

    private bool PollPicker()
    {
        Task<PickerScanResult>? task = _pickerTask;
        if (task is null || !task.IsCompleted)
        {
            return false;
        }

        _pickerTask = null;
        _pickerCancellation?.Dispose();
        _pickerCancellation = null;

        PickerScanResult result;
        try
        {
            result = task.GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            return false;
        }
        catch (Exception ex)
        {
            _pickerLoading = false;
            _pickerStatus = $"E: {ex.Message}";
            return true;
        }

        if (result.Generation != _pickerGeneration)
        {
            return false;
        }

        if (result.FileList)
        {
            if (result.Error.Length > 0)
            {
                _pickerFilesLoaded = false;
                _pickerLoading = false;
                _pickerStatus = $"E: {result.Error}";
                return true;
            }

            _pickerFiles = result.Files;
            _pickerFilesLoaded = true;
            if (_pickerKind != PickerKind.None)
            {
                BeginPickerQuery();
            }
            return true;
        }

        if (_pickerKind != PickerKind.Search)
        {
            return false;
        }

        _pickerItems.Clear();
        if (result.Error.Length > 0)
        {
            _pickerStatus = $"E: {result.Error}";
        }
        else
        {
            foreach (FileSearchMatch match in result.Matches)
            {
                _pickerItems.Add(new PickerItem
                {
                    Display = $"{match.Path}:{match.Line + 1}  {match.Preview}",
                    Path = Path.GetFullPath(Path.Combine(_pickerRoot, match.Path)),
                    Line = match.Line,
                    Column = match.Column
                });
            }

            _pickerStatus = _pickerItems.Count == 0
                ? "No matches"
                : $"{_pickerItems.Count} match{(_pickerItems.Count == 1 ? string.Empty : "es")}";
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        return true;
    }

    private string ProjectRoot()
    {
        return _workspaceRoot;
    }

    private void HandlePicker(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            bool pluginPicker = _pickerKind == PickerKind.Plugin;
            ClosePicker();
            if (!pluginPicker)
            {
                SetMessage("Cancelled.");
            }
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Enter:
                ActivatePickerItem();
                return;
            case Key.Up:
                MovePickerSelection(-1);
                return;
            case Key.Down:
                MovePickerSelection(1);
                return;
            case Key.PageUp:
                MovePickerSelection(-PickerLayoutRows());
                return;
            case Key.PageDown:
                MovePickerSelection(PickerLayoutRows());
                return;
            case Key.Backspace:
                if (_pickerQueryCursor > 0)
                {
                    string text = _pickerQuery.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, _pickerQueryCursor);
                    _pickerQuery.Remove(previous, _pickerQueryCursor - previous);
                    _pickerQueryCursor = previous;
                    BeginPickerQuery();
                }
                return;
            case Key.Delete:
                if (_pickerQueryCursor < _pickerQuery.Length)
                {
                    string text = _pickerQuery.ToString();
                    int next = UnicodeUtil.NextBoundary(text, _pickerQueryCursor);
                    _pickerQuery.Remove(_pickerQueryCursor, next - _pickerQueryCursor);
                    BeginPickerQuery();
                }
                return;
            case Key.Left:
                _pickerQueryCursor = UnicodeUtil.PreviousBoundary(_pickerQuery.ToString(),
                    _pickerQueryCursor);
                return;
            case Key.Right:
                _pickerQueryCursor = UnicodeUtil.NextBoundary(_pickerQuery.ToString(),
                    _pickerQueryCursor);
                return;
            case Key.Home:
                _pickerQueryCursor = 0;
                return;
            case Key.End:
                _pickerQueryCursor = _pickerQuery.Length;
                return;
        }

        if (inputEvent.Key == Key.Character && !inputEvent.Alt)
        {
            uint codePoint = (uint)inputEvent.Character.Value;
            if (codePoint < 0x20 || codePoint == 0x7F)
            {
                return;
            }

            string inserted = inputEvent.Character.ToString();
            _pickerQuery.Insert(_pickerQueryCursor, inserted);
            _pickerQueryCursor += inserted.Length;
            BeginPickerQuery();
        }
    }

    private void MovePickerSelection(int delta)
    {
        if (_pickerItems.Count == 0)
        {
            return;
        }

        _pickerSelected = Math.Clamp(_pickerSelected + delta, 0, _pickerItems.Count - 1);
        int rows = PickerLayoutRows();
        if (_pickerSelected < _pickerTop)
        {
            _pickerTop = _pickerSelected;
        }
        else if (_pickerSelected >= _pickerTop + rows)
        {
            _pickerTop = _pickerSelected - rows + 1;
        }

        int maxTop = Math.Max(0, _pickerItems.Count - rows);
        _pickerTop = Math.Clamp(_pickerTop, 0, maxTop);

        if (_pickerKind == PickerKind.Files)
        {
            UpdatePickerPreview();
        }
    }

    private void ActivatePickerItem()
    {
        if (_pickerLoading || _pickerSelected < 0 || _pickerSelected >= _pickerItems.Count)
        {
            return;
        }

        PickerItem item = _pickerItems[_pickerSelected];
        if (_pickerKind == PickerKind.Plugin)
        {
            int value = item.Value;
            Action<int?>? callback = _pluginSelectCallback;
            ClosePicker(cancelled: false);
            callback?.Invoke(value);
            return;
        }

        if (_pickerKind == PickerKind.CodeActions)
        {
            if (item.Value < 0 || item.Value >= _codeActions.Count)
            {
                return;
            }

            LspCodeAction action = _codeActions[item.Value];
            ClosePicker(cancelled: false);
            ApplyCodeAction(action);
            return;
        }

        if (_pickerKind == PickerKind.Problems)
        {
            string problemPath = item.Path;
            long problemLine = item.Line;
            long problemColumn = LspColumnToByteOffset(problemPath, item.Line, (int)item.Column);
            bool sameFile = _document.Path is not null &&
                            PathsEqual(Path.GetFullPath(_document.Path),
                                Path.GetFullPath(problemPath));

            ClosePicker(cancelled: false);
            if (sameFile)
            {
                if (problemLine >= 0 && problemLine < _document.LineCount)
                {
                    long start = _document.LineStart(problemLine);
                    long end = _document.LineEnd(problemLine);
                    MoveTo(Math.Clamp(start + problemColumn, start, end), recordJump: true);
                }
            }
            else
            {
                RequestOpen(problemPath, force: false, problemLine, problemColumn);
            }
            return;
        }

        string path = item.Path;
        long line = item.Line;
        long column = item.Column;

        ClosePicker();
        RequestOpen(path, force: false, line, column);
    }

    /// <summary>
    /// Converts an LSP UTF-16 column into a UTF-8 byte offset within a line of
    /// an arbitrary file. The current buffer is read in place; other files are
    /// read lazily up to the requested line.
    /// </summary>
    private long LspColumnToByteOffset(string path, long line, int character)
    {
        if (line < 0)
        {
            return 0;
        }

        try
        {
            if (_document.Path is not null &&
                PathsEqual(Path.GetFullPath(_document.Path), Path.GetFullPath(path)) &&
                line < _document.LineCount)
            {
                int length = _document.CopyLineText(line, ref _lineBuffer, out _);
                return LspPositions.Utf16ColumnToByteOffset(
                    _lineBuffer.AsSpan(0, length), character);
            }

            foreach (string text in File.ReadLines(path))
            {
                if (line == 0)
                {
                    byte[] bytes = Encoding.UTF8.GetBytes(text);
                    return LspPositions.Utf16ColumnToByteOffset(bytes, character);
                }
                --line;
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            // Fall back to the raw column below.
        }

        return Math.Max(0, character);
    }

    private void CancelPickerWork()
    {
        _pickerCancellation?.Cancel();
        _pickerCancellation?.Dispose();
        _pickerCancellation = null;
        _pickerTask = null;
    }

    /// <summary>
    /// Returns a lower score for better matches, or int.MaxValue when the
    /// query does not match. Contiguous matches rank ahead of subsequences.
    /// </summary>
    private static int FuzzyScore(string text, string query)
    {
        if (query.Length == 0)
        {
            return 0;
        }

        int contiguous = text.IndexOf(query, StringComparison.OrdinalIgnoreCase);
        if (contiguous >= 0)
        {
            return contiguous;
        }

        int textIndex = 0;
        int gapScore = 0;
        int lastMatch = -2;

        foreach (char queryChar in query)
        {
            char wanted = char.ToLowerInvariant(queryChar);
            bool found = false;

            while (textIndex < text.Length)
            {
                char candidate = char.ToLowerInvariant(text[textIndex]);
                if (candidate == wanted)
                {
                    gapScore += textIndex - lastMatch - 1;
                    lastMatch = textIndex;
                    ++textIndex;
                    found = true;
                    break;
                }

                ++textIndex;
            }

            if (!found)
            {
                return int.MaxValue;
            }
        }

        return 10_000 + gapScore + Math.Min(text.Length, 1_000);
    }

    // -----------------------------------------------------------------------
    // COMMANDS
    // -----------------------------------------------------------------------

    private void ExecuteParsedCommand(ExCommand command, bool force)
    {
        string name = command.Name;
        if (name.EndsWith('!'))
        {
            force = true;
            name = name[..^1];
        }

        List<string> arguments = command.Arguments;

        switch (name)
        {
            case "w":
                if (arguments.Count > 1)
                {
                    SetError("usage: :w [file]");
                    return;
                }
                StartSave(arguments.Count == 0 ? null : arguments[0], quitAfter: false);
                return;

            case "wq":
                if (arguments.Count != 0)
                {
                    SetError("usage: :wq");
                    return;
                }
                StartSave(path: null, quitAfter: true);
                return;

            case "q":
                if (arguments.Count != 0)
                {
                    SetError("usage: :q[!]");
                    return;
                }
                RequestQuit(force);
                return;

            case "e":
                if (arguments.Count != 1)
                {
                    SetError("usage: :e[!] <file>");
                    return;
                }
                RequestOpen(arguments[0], force);
                return;

            case "lang":
                if (arguments.Count != 1)
                {
                    SetError("usage: :lang <id>");
                    return;
                }
                if (!TrySetLanguage(arguments[0], out string languageError))
                {
                    SetError(languageError);
                    return;
                }
                SetMessage($"Language set to {arguments[0]}");
                ResetLsp();
                FirePluginEvent("buffer_opened", CurrentBufferData());
                return;

            case "help":
                ClearMessage();
                SetMessage("i insert | Esc normal | Ctrl+C comment | Shift+C add cursor | :w :wq :q :e :lang | u/Ctrl+R undo");
                return;
        }

        if (_plugins is not null && _plugins.HasCommand(name))
        {
            if (!_plugins.TryExecuteCommand(name, arguments, out string pluginError))
            {
                SetError($"plugin command ':{name}' failed");
            }
            else if (pluginError.Length > 0)
            {
                SetError(pluginError);
            }
            return;
        }

        if (IsFutureCommand(name))
        {
            SetError($"command ':{name}' is not implemented in this MVP");
            return;
        }

        SetError($"unknown command ':{command.Name}' (try :help)");
    }

    private static bool IsFutureCommand(string name)
        => name is "fmt" or "cfg" or "rcfg" or "sf" or "g" or "sv" or "sh" or
            "setup";

    private void StartSave(string? path, bool quitAfter)
    {
        if (_saveOperation is not null)
        {
            if (quitAfter)
            {
                _quitAfterSave = true;
            }
            _resaveRequested = true;
            SetMessage("Save already in progress; queued another save.");
            return;
        }

        Document.SaveSnapshot? snapshot = _document.CreateSaveSnapshot(path, out string error);
        if (snapshot is null)
        {
            SetError(error);
            if (quitAfter)
            {
                _quitAfterSave = false;
            }
            return;
        }

        Document document = _document;
        Task<SaveResult> task = Task.Factory.StartNew(() =>
        {
            bool ok = snapshot.WriteTo(out string writeError);
            return new SaveResult
            {
                Ok = ok,
                Error = writeError,
                Path = snapshot.Path,
                Pose = snapshot.Pose,
                Revision = snapshot.Revision
            };
        }, CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default);

        _saveOperation = new SaveOperation
        {
            Task = task,
            Document = document,
            Snapshot = snapshot
        };

        if (quitAfter)
        {
            _quitAfterSave = true;
        }

        SetMessage($"Writing {snapshot.Path} ...");
    }

    /// <summary>
    /// Polls every background task that can change what is on screen.
    /// </summary>
    private bool PollBackgroundWork()
    {
        bool redraw = PollSave();
        if (PollPicker())
        {
            redraw = true;
        }
        if (PollPickerPreview())
        {
            redraw = true;
        }

        if (PollLsp())
        {
            redraw = true;
        }

        if (_externalCheckPending && _saveOperation is null)
        {
            _externalCheckPending = false;
            if (CheckExternalFile())
            {
                redraw = true;
            }
        }

        return redraw;
    }

    /// <summary>
    /// Called from the UI loop. Returns true when the save state changed and a
    /// redraw is warranted.
    /// </summary>
    private bool PollSave()
    {
        if (_saveOperation is null || !_saveOperation.Task.IsCompleted)
        {
            return false;
        }

        SaveOperation operation = _saveOperation;
        _saveOperation = null;

        SaveResult result;
        try
        {
            result = operation.Task.GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            result = new SaveResult
            {
                Ok = false,
                Error = ex.Message,
                Path = operation.Snapshot.Path,
                Pose = operation.Snapshot.Pose,
                Revision = operation.Snapshot.Revision
            };
        }

        bool sameDocument = ReferenceEquals(operation.Document, _document);

        if (result.Ok)
        {
            if (sameDocument)
            {
                operation.Document.MarkSaved(result.Path, result.Pose, result.Revision);
                RefreshFileStamp();
                SetMessage($"Wrote {result.Path} ({_document.LineCount} lines)");
                FirePluginEvent("buffer_saved", new Dictionary<string, object?>
                {
                    ["path"] = result.Path,
                    ["id"] = CurrentBufferId
                });
                NotifyLspSaved();
            }
            else
            {
                SetMessage($"Wrote {result.Path}");
            }

            if (_waitThenQuit)
            {
                _waitThenQuit = false;
                _quitAfterSave = false;
                _resaveRequested = false;

                if (!sameDocument)
                {
                    SetMessage("Save finished; current buffer was not quit.");
                }
                else if (_waitThenQuitForce || !_document.IsDirty)
                {
                    _running = false;
                }
                else
                {
                    SetMessage("Save finished; newer edits remain unsaved.");
                }
            }
            else if (_quitAfterSave)
            {
                if (sameDocument && !_document.IsDirty)
                {
                    _running = false;
                }
                else if (sameDocument)
                {
                    // The user edited while the snapshot was being written.
                    // Save the newer state before quitting.
                    StartSave(path: null, quitAfter: true);
                }
                else
                {
                    _quitAfterSave = false;
                    _resaveRequested = false;
                    SetMessage("Saved the previous buffer; current buffer was not quit.");
                }
            }
            else if (_resaveRequested)
            {
                _resaveRequested = false;
                StartSave(path: null, quitAfter: false);
            }
        }
        else
        {
            SetError(result.Error);
            _quitAfterSave = false;
            _resaveRequested = false;
            _waitThenQuit = false;
            _waitThenQuitForce = false;
        }

        return true;
    }

    private void RequestQuit(bool force)
    {
        if (_saveOperation is not null)
        {
            if (!force && _document.IsDirty)
            {
                _prompt = PromptKind.Quit;
                return;
            }

            _waitThenQuit = true;
            _waitThenQuitForce = force;
            SetMessage("Will quit when the save finishes.");
            return;
        }

        if (!force && _document.IsDirty)
        {
            _prompt = PromptKind.Quit;
            return;
        }

        _running = false;
    }

    private void RequestOpen(string path, bool force, long line = -1, long column = -1)
    {
        if (!force && _document.IsDirty)
        {
            _pendingOpen = path;
            _pendingOpenLine = line;
            _pendingOpenColumn = column;
            _prompt = PromptKind.Open;
            return;
        }

        OpenPathAt(path, line, column);
    }

    private void OpenPathAt(string path, long line, long column)
    {
        Document? loaded = Document.Open(path, out string error);
        if (loaded is null)
        {
            SetError(error);
            return;
        }

        ResetLsp();
        _document = loaded;
        RefreshLanguage();
        _cursor = 0;
        _topLine = 0;
        _leftColumn = 0;
        _preferredColumnValid = false;
        _selectionLinewise = false;
        _extraSelections.Clear();
        _jumpHistory.Clear();
        _pendingG = false;
        _mode = Mode.Normal;
        _commandLine.Clear();
        _commandCursor = 0;

        if (line >= 0 && _document.LineCount > 0)
        {
            long targetLine = Math.Clamp(line, 0, _document.LineCount - 1);
            long start = _document.LineStart(targetLine);
            long end = _document.LineEnd(targetLine);
            int length = _document.CopyLineText(targetLine, ref _lineBuffer, out _);

            int local = (int)Math.Clamp(column, 0, length);
            // A search match comes from decoded text, so its byte offset can
            // land inside a scalar in a file containing invalid UTF-8. Move
            // forward to the next scalar boundary before placing the cursor.
            while (local < length && (_lineBuffer[local] & 0xC0) == 0x80)
            {
                ++local;
            }

            _cursor = Math.Clamp(start + local, start, end);
            int rows = TextRows();
            _topLine = Math.Max(0, targetLine - Math.Max(0, rows / 2));
        }

        SnapCursor();
        _selectionAnchor = _cursor;
        RefreshFileStamp();
        SetMessage(line >= 0
            ? $"Opened {path}:{line + 1}"
            : $"Opened {path}");

        FirePluginEvent("buffer_opened", new Dictionary<string, object?>
        {
            ["path"] = path,
            ["id"] = CurrentBufferId,
            ["line"] = line
        });
    }

    private void Undo()
    {
        _document.BreakUndoGroup();
        long? hint = _document.Undo();
        if (hint is null)
        {
            SetMessage("Already at oldest change.");
            return;
        }

        _cursor = hint.Value;
        _extraSelections.Clear();
        SnapCursor();
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void Redo()
    {
        _document.BreakUndoGroup();
        long? hint = _document.Redo();
        if (hint is null)
        {
            SetMessage("Already at newest change.");
            return;
        }

        _cursor = hint.Value;
        _extraSelections.Clear();
        SnapCursor();
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    // -----------------------------------------------------------------------
    // NAVIGATION
    // -----------------------------------------------------------------------

    private Selection CapturePrimarySelection() => new()
    {
        Cursor = _cursor,
        Anchor = _selectionAnchor,
        Linewise = _selectionLinewise,
        PreferredColumn = _preferredColumn,
        PreferredColumnValid = _preferredColumnValid
    };

    private void LoadSelection(Selection selection)
    {
        _cursor = selection.Cursor;
        _selectionAnchor = selection.Anchor;
        _selectionLinewise = selection.Linewise;
        _preferredColumn = selection.PreferredColumn;
        _preferredColumnValid = selection.PreferredColumnValid;
    }

    private List<Selection> CaptureAllSelections()
    {
        var selections = new List<Selection>(_extraSelections.Count + 1)
        {
            CapturePrimarySelection()
        };
        foreach (Selection selection in _extraSelections)
        {
            selections.Add(selection.Clone());
        }
        return selections;
    }

    private void SetSelections(IReadOnlyList<Selection> selections)
    {
        if (selections.Count == 0)
        {
            return;
        }

        Selection primary = selections[0];
        _cursor = SnapOffset(primary.Cursor);
        _selectionAnchor = SnapOffset(primary.Anchor);
        _selectionLinewise = primary.Linewise;
        _preferredColumn = primary.PreferredColumn;
        _preferredColumnValid = primary.PreferredColumnValid;

        _extraSelections.Clear();
        var seen = new HashSet<long> { _cursor };
        for (int i = 1; i < selections.Count; ++i)
        {
            Selection source = selections[i];
            var selection = new Selection
            {
                Cursor = SnapOffset(source.Cursor),
                Anchor = SnapOffset(source.Anchor),
                Linewise = source.Linewise,
                PreferredColumn = source.PreferredColumn,
                PreferredColumnValid = source.PreferredColumnValid
            };

            if (seen.Add(selection.Cursor))
            {
                _extraSelections.Add(selection);
            }
        }
    }

    /// <summary>
    /// Runs an action once for every selection, restoring the primary and
    /// secondary selections afterwards. Edits are applied from the highest
    /// offset to the lowest so earlier edits remain valid.
    /// </summary>
    private void ForEachSelection(Action action, bool editing = false)
    {
        List<Selection> selections = CaptureAllSelections();
        if (selections.Count == 1)
        {
            LoadSelection(selections[0]);
            action();
            return;
        }

        var results = new Selection[selections.Count];
        int[] order = new int[selections.Count];
        for (int i = 0; i < order.Length; ++i)
        {
            order[i] = i;
        }

        Mode modeBefore = _mode;
        if (editing)
        {
            Array.Sort(order, (left, right) =>
                selections[right].Cursor.CompareTo(selections[left].Cursor));
            _document.BeginCompositeEdit();
        }

        try
        {
            foreach (int index in order)
            {
                LoadSelection(selections[index]);
                _mode = modeBefore;
                long revisionBefore = _document.Revision;
                action();

                if (editing && _document.Revision != revisionBefore)
                {
                    // This edit is at a lower offset than the selections
                    // already processed, so shift their saved positions.
                    long offset = _document.LastChangeOffset;
                    long removed = _document.LastChangeRemoved;
                    long inserted = _document.LastChangeInserted;
                    for (int i = 0; i < results.Length; ++i)
                    {
                        Selection? result = results[i];
                        if (result is null)
                        {
                            continue;
                        }

                        result.Cursor = AdjustOffset(result.Cursor, offset,
                            removed, inserted);
                        result.Anchor = AdjustOffset(result.Anchor, offset,
                            removed, inserted);
                    }
                }

                results[index] = CapturePrimarySelection();
            }
        }
        finally
        {
            if (editing)
            {
                _document.EndCompositeEdit();
            }
        }

        SetSelections(results);
    }

    private long SnapOffset(long offset)
    {
        if (offset > _document.Size)
        {
            offset = _document.Size;
        }

        long line = _document.LineOfOffset(offset);
        long begin = _document.LineStart(line);
        long end = _document.LineEnd(line);

        if (offset < begin)
        {
            offset = begin;
        }
        else if (offset > end)
        {
            offset = end;
        }

        return offset;
    }

    private long VisualColumnOfOffset(long offset)
    {
        long line = _document.LineOfOffset(offset);
        long begin = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        return UnicodeUtil.VisualColumn(_lineBuffer.AsSpan(0, length),
            (int)(offset - begin), TabWidth);
    }

    private long CursorLine() => _document.LineOfOffset(_cursor);

    private long CursorVisualColumn() => VisualColumnOfOffset(_cursor);

    private int TextRows() => _rows > 2 ? _rows - 2 : 0;

    private long LineStartOfCursor() => _document.LineStart(CursorLine());

    private long LineEndOfCursor() => _document.LineEnd(CursorLine());

    private void SnapCursor()
    {
        _cursor = SnapOffset(_cursor);
    }

    private void MoveLeft()
    {
        long line = CursorLine();
        long begin = _document.LineStart(line);

        if (_cursor > begin)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int local = (int)(_cursor - begin);
            _cursor = begin + UnicodeUtil.PreviousBoundary(_lineBuffer.AsSpan(0, length), local);
        }
        else if (line > 0)
        {
            _cursor = _document.LineEnd(line - 1);
        }

        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void MoveRight()
    {
        long line = CursorLine();
        long end = _document.LineEnd(line);

        if (_cursor < end)
        {
            long begin = _document.LineStart(line);
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int local = (int)(_cursor - begin);
            _cursor = begin + UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
        }
        else if (line + 1 < _document.LineCount)
        {
            _cursor = _document.LineStart(line + 1);
        }

        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void MoveVertical(int delta)
    {
        long line = CursorLine();

        if (!_preferredColumnValid)
        {
            _preferredColumn = CursorVisualColumn();
            _preferredColumnValid = true;
        }

        long target = line + delta;
        if (target < 0)
        {
            target = 0;
        }

        long lastLine = _document.LineCount - 1;
        if (target > lastLine)
        {
            target = lastLine;
        }

        if (target == line)
        {
            return;
        }

        long start = _document.LineStart(target);
        int length = _document.CopyLineText(target, ref _lineBuffer, out _);
        _cursor = start + UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, length), _preferredColumn, TabWidth);

        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void MovePage(int direction)
    {
        long before = _cursor;
        int rows = TextRows();
        int amount = Math.Max(1, rows > 1 ? rows - 1 : 1);
        MoveVertical(direction * amount);

        if (_cursor != before)
        {
            RecordJump(before);
        }
    }

    private void MoveTo(long offset, bool recordJump)
    {
        if (recordJump)
        {
            RecordJump(_cursor);
        }

        _cursor = Math.Clamp(offset, 0, _document.Size);
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void MoveToDocumentStart(bool recordJump)
        => MoveTo(0, recordJump);

    private void MoveToDocumentEnd(bool recordJump)
        => MoveTo(_document.Size, recordJump);

    private void MoveToPreviousLocation()
    {
        if (_jumpHistory.Count == 0)
        {
            SetMessage("No previous location.");
            return;
        }

        _cursor = _jumpHistory[^1];
        _jumpHistory.RemoveAt(_jumpHistory.Count - 1);
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void RecordJump(long from)
    {
        if (from == _cursor)
        {
            return;
        }
        if (_jumpHistory.Count > 0 && _jumpHistory[^1] == from)
        {
            return;
        }

        _jumpHistory.Add(from);
        if (_jumpHistory.Count > 200)
        {
            _jumpHistory.RemoveAt(0);
        }
    }

    private void EnterInsert()
    {
        _mode = Mode.Insert;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
    }

    private void InsertCharacter(Rune rune)
    {
        uint codePoint = (uint)rune.Value;
        if (codePoint == '\r')
        {
            return;
        }
        if (codePoint == '\n')
        {
            InsertNewline();
            return;
        }
        if (codePoint < 0x20 && codePoint != '\t')
        {
            return;
        }
        if (codePoint >= 0x7F && codePoint < 0xA0)
        {
            return;
        }

        Span<byte> text = stackalloc byte[4];
        int length = UnicodeUtil.Encode(codePoint, text);
        _document.Insert(_cursor, text[..length]);
        _cursor += length;
        SnapCursor();
        _preferredColumnValid = false;
        ClearMessage();
    }

    private void InsertNewline()
    {
        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        _document.Insert(_cursor, eol);
        _cursor = Math.Min(_cursor + eol.Length, _document.Size);
        SnapCursor();
        _preferredColumnValid = false;
        ClearMessage();
    }

    private void InsertNewlineBelow()
    {
        _document.BreakUndoGroup();
        long line = CursorLine();
        long end = _document.LineEnd(line);
        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        _document.Insert(end, eol);
        _cursor = _document.LineStart(line + 1);
        _preferredColumnValid = false;
        SnapCursor();
        EnterInsert();
    }

    private void InsertNewlineAbove()
    {
        _document.BreakUndoGroup();
        long line = CursorLine();
        long start = _document.LineStart(line);
        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        _document.Insert(start, eol);
        _cursor = start;
        _preferredColumnValid = false;
        SnapCursor();
        EnterInsert();
    }

    private void Backspace()
    {
        long line = CursorLine();
        long begin = _document.LineStart(line);

        if (_cursor > begin)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int local = (int)(_cursor - begin);
            int previous = UnicodeUtil.PreviousBoundary(_lineBuffer.AsSpan(0, length), local);
            long target = begin + previous;
            _document.Erase(target, _cursor - target);
            _cursor = target;
        }
        else if (line > 0)
        {
            long previousEnd = _document.LineEnd(line - 1);
            _document.Erase(previousEnd, _cursor - previousEnd);
            _cursor = previousEnd;
        }

        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
    }

    private void DeleteForward()
    {
        long line = CursorLine();
        long end = _document.LineEnd(line);

        if (_cursor < end)
        {
            long begin = _document.LineStart(line);
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int local = (int)(_cursor - begin);
            int next = UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
            _document.Erase(_cursor, next - local);
        }
        else if (line + 1 < _document.LineCount)
        {
            long nextStart = _document.LineStart(line + 1);
            _document.Erase(_cursor, nextStart - _cursor);
        }

        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
    }

    private void DeleteAtCursor()
    {
        long line = CursorLine();
        long end = _document.LineEnd(line);
        if (_cursor >= end)
        {
            return;
        }

        _document.BreakUndoGroup();
        long begin = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)(_cursor - begin);
        int next = UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
        _document.Erase(_cursor, next - local);
        _preferredColumnValid = false;
        SnapCursor();
    }

    private void StartCharacterSelection()
    {
        _mode = Mode.Select;
        _selectionLinewise = false;
        _selectionAnchor = _cursor;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void StartLineSelection()
    {
        _mode = Mode.Select;
        _selectionLinewise = true;
        _selectionAnchor = _document.LineStart(CursorLine());
        _cursor = _selectionAnchor;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void ExtendLineSelection()
    {
        if (!_selectionLinewise)
        {
            long line = CursorLine();
            _selectionAnchor = _document.LineStart(line);
            _cursor = _selectionAnchor;
            _selectionLinewise = true;
        }
        else
        {
            long line = CursorLine();
            if (line + 1 < _document.LineCount)
            {
                _cursor = _document.LineStart(line + 1);
            }
            else
            {
                SetMessage("Already at the last line.");
            }
        }

        _preferredColumnValid = false;
        _document.BreakUndoGroup();
    }

    private void CancelSelection()
    {
        _mode = Mode.Normal;
        _selectionLinewise = false;
        _selectionAnchor = _cursor;
        foreach (Selection selection in _extraSelections)
        {
            selection.Linewise = false;
            selection.Anchor = selection.Cursor;
        }
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void DeleteSelection()
    {
        if (_mode != Mode.Select)
        {
            return;
        }

        (long start, long end) = SelectionRange();
        _document.BreakUndoGroup();
        if (end > start)
        {
            _document.Erase(start, end - start);
        }

        _cursor = Math.Clamp(start, 0, _document.Size);
        _mode = Mode.Normal;
        _selectionLinewise = false;
        _selectionAnchor = _cursor;
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
    }

    /// <summary>
    /// Returns the selected byte range as a half-open interval. Character
    /// selections include the scalar under the cursor; line selections include
    /// the final line's terminator when one exists.
    /// </summary>
    private (long Start, long End) SelectionRange()
        => SelectionRange(_selectionAnchor, _cursor, _selectionLinewise);

    private (long Start, long End) SelectionRange(long anchor, long cursor,
        bool linewise)
    {
        if (linewise)
        {
            long anchorLine = _document.LineOfOffset(anchor);
            long cursorLine = _document.LineOfOffset(cursor);
            long firstLine = Math.Min(anchorLine, cursorLine);
            long lastLine = Math.Max(anchorLine, cursorLine);
            long start = _document.LineStart(firstLine);
            long end = lastLine + 1 < _document.LineCount
                ? _document.LineStart(lastLine + 1)
                : _document.Size;
            return (start, end);
        }

        long rangeStart = Math.Min(anchor, cursor);
        long rangeEnd = Math.Max(anchor, cursor);

        if (rangeEnd < _document.Size)
        {
            long line = _document.LineOfOffset(rangeEnd);
            long lineStart = _document.LineStart(line);
            long lineEnd = _document.LineEnd(line);

            if (rangeEnd < lineEnd)
            {
                int length = _document.CopyLineText(line, ref _lineBuffer, out _);
                int local = (int)(rangeEnd - lineStart);
                rangeEnd = lineStart +
                           UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
            }
            else if (rangeEnd == lineEnd && line + 1 < _document.LineCount)
            {
                // The cursor sits just past the last scalar; include the
                // line's terminator so selecting to end-of-line deletes the
                // line break as expected.
                rangeEnd = _document.LineStart(line + 1);
            }
        }

        return (rangeStart, rangeEnd);
    }

    // -----------------------------------------------------------------------
    // MULTI-CURSOR AND COMMENTS
    // -----------------------------------------------------------------------

    /// <summary>
    /// Adds a cursor on the next line at the same visual column, mirroring
    /// Helix's "copy selection on next line". Repeated presses therefore add
    /// one cursor per line below the existing block of cursors.
    /// </summary>
    private void AddCursorBelow()
    {
        List<Selection> selections = CaptureAllSelections();
        var results = new List<Selection>(selections);
        bool added = false;

        foreach (Selection source in selections)
        {
            if (TryCreateSelectionBelow(source, out Selection created))
            {
                results.Add(created);
                added = true;
            }
        }

        if (!added)
        {
            SetMessage("Already at the last line.");
            return;
        }

        SetSelections(results);
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private bool TryCreateSelectionBelow(Selection source, out Selection created)
    {
        created = null!;

        long sourceLine = _document.LineOfOffset(source.Cursor);
        if (sourceLine + 1 >= _document.LineCount)
        {
            return false;
        }

        long targetLine = sourceLine + 1;
        long targetStart = _document.LineStart(targetLine);
        int targetLength = _document.CopyLineText(targetLine, ref _lineBuffer, out _);
        long cursorColumn = VisualColumnOfOffset(source.Cursor);

        long targetCursor = targetStart + UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, targetLength), cursorColumn, TabWidth);
        long targetAnchor = targetCursor;

        if (_mode == Mode.Select && source.Anchor != source.Cursor &&
            _document.LineOfOffset(source.Anchor) == sourceLine)
        {
            long anchorColumn = VisualColumnOfOffset(source.Anchor);
            targetAnchor = targetStart + UnicodeUtil.ByteOffsetForVisualColumn(
                _lineBuffer.AsSpan(0, targetLength), anchorColumn, TabWidth);
        }

        created = new Selection
        {
            Cursor = targetCursor,
            Anchor = targetAnchor,
            Linewise = source.Linewise,
            PreferredColumn = cursorColumn,
            PreferredColumnValid = true
        };
        return true;
    }

    private void ToggleComment()
    {
        string? lineComment = _language?.LineComment;
        string blockStart = _language?.BlockCommentStart ?? string.Empty;
        string blockEnd = _language?.BlockCommentEnd ?? string.Empty;

        if (string.IsNullOrEmpty(lineComment) &&
            (blockStart.Length == 0 || blockEnd.Length == 0))
        {
            SetError("No comment syntax is available for this language.");
            return;
        }

        var lines = new SortedSet<long>();
        bool selecting = _mode == Mode.Select;
        foreach (Selection selection in CaptureAllSelections())
        {
            AddTouchedLines(lines, selection, selecting);
        }

        if (lines.Count == 0)
        {
            return;
        }

        _document.BreakUndoGroup();
        _document.BeginCompositeEdit();
        try
        {
            if (!string.IsNullOrEmpty(lineComment))
            {
                ToggleLineComments(lines, lineComment);
            }
            else
            {
                ToggleBlockComments(lines, blockStart, blockEnd);
            }
        }
        finally
        {
            _document.EndCompositeEdit();
        }

        ClearMessage();
    }

    private void AddTouchedLines(SortedSet<long> lines, Selection selection,
        bool selecting)
    {
        if (!selecting || selection.Anchor == selection.Cursor)
        {
            lines.Add(_document.LineOfOffset(selection.Cursor));
            return;
        }

        long start = Math.Min(selection.Anchor, selection.Cursor);
        long end = Math.Max(selection.Anchor, selection.Cursor);
        long firstLine;
        long lastLine;

        if (selection.Linewise)
        {
            firstLine = Math.Min(
                _document.LineOfOffset(selection.Anchor),
                _document.LineOfOffset(selection.Cursor));
            lastLine = Math.Max(
                _document.LineOfOffset(selection.Anchor),
                _document.LineOfOffset(selection.Cursor));
        }
        else
        {
            firstLine = _document.LineOfOffset(start);
            lastLine = end > start
                ? _document.LineOfOffset(end - 1)
                : firstLine;
        }

        for (long line = firstLine; line <= lastLine; ++line)
        {
            lines.Add(line);
        }
    }

    private static int IndentLength(ReadOnlySpan<byte> text)
    {
        int offset = 0;
        while (offset < text.Length &&
               (text[offset] == (byte)' ' || text[offset] == (byte)'\t'))
        {
            ++offset;
        }
        return offset;
    }

    private static int TrimTrailingWhitespace(ReadOnlySpan<byte> text, int minimum)
    {
        int end = text.Length;
        while (end > minimum &&
               (text[end - 1] == (byte)' ' || text[end - 1] == (byte)'\t'))
        {
            --end;
        }
        return end;
    }

    private static bool StartsWith(ReadOnlySpan<byte> text,
        ReadOnlySpan<byte> prefix)
        => prefix.Length > 0 && text.Length >= prefix.Length &&
           text[..prefix.Length].SequenceEqual(prefix);

    private static bool EndsWith(ReadOnlySpan<byte> text,
        ReadOnlySpan<byte> suffix)
        => suffix.Length > 0 && text.Length >= suffix.Length &&
           text[^suffix.Length..].SequenceEqual(suffix);

    private void ToggleLineComments(SortedSet<long> lines, string marker)
    {
        byte[] markerBytes = Encoding.UTF8.GetBytes(marker);
        bool allCommented = true;

        foreach (long line in lines)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
            int indent = IndentLength(text);
            if (!StartsWith(text[indent..], markerBytes))
            {
                allCommented = false;
                break;
            }
        }

        var changes = new List<(long Offset, long Remove, byte[] Insert)>();
        foreach (long line in lines)
        {
            long lineStart = _document.LineStart(line);
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int indent = IndentLength(_lineBuffer.AsSpan(0, length));

            if (allCommented)
            {
                int remove = markerBytes.Length;
                int after = indent + remove;
                if (after < length && _lineBuffer[after] == (byte)' ')
                {
                    ++remove;
                }
                changes.Add((lineStart + indent, remove, Array.Empty<byte>()));
            }
            else
            {
                var insert = new List<byte>(markerBytes);
                if (indent < length)
                {
                    insert.Add((byte)' ');
                }
                changes.Add((lineStart + indent, 0, insert.ToArray()));
            }
        }

        ApplyCommentChanges(changes);
    }

    private void ToggleBlockComments(SortedSet<long> lines, string start,
        string end)
    {
        byte[] startBytes = Encoding.UTF8.GetBytes(start);
        byte[] endBytes = Encoding.UTF8.GetBytes(end);
        bool allBlocked = true;

        foreach (long line in lines)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
            int indent = IndentLength(text);
            int contentEnd = TrimTrailingWhitespace(text, indent);
            if (contentEnd <= indent ||
                !StartsWith(text[indent..contentEnd], startBytes) ||
                !EndsWith(text[indent..contentEnd], endBytes))
            {
                allBlocked = false;
                break;
            }
        }

        var changes = new List<(long Offset, long Remove, byte[] Insert)>();
        foreach (long line in lines)
        {
            long lineStart = _document.LineStart(line);
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
            int indent = IndentLength(text);
            int contentEnd = TrimTrailingWhitespace(text, indent);

            if (allBlocked)
            {
                int removeStart = startBytes.Length;
                int afterStart = indent + removeStart;
                if (afterStart < contentEnd && text[afterStart] == (byte)' ')
                {
                    ++removeStart;
                }
                changes.Add((lineStart + indent, removeStart, Array.Empty<byte>()));

                long endOffset = lineStart + contentEnd - endBytes.Length;
                int removeEnd = endBytes.Length;
                if (endOffset > lineStart + indent + removeStart &&
                    text[contentEnd - endBytes.Length - 1] == (byte)' ')
                {
                    --endOffset;
                    ++removeEnd;
                }
                changes.Add((endOffset, removeEnd, Array.Empty<byte>()));
            }
            else if (contentEnd <= indent)
            {
                byte[] insert = Encoding.UTF8.GetBytes(start + " " + end);
                changes.Add((lineStart + indent, 0, insert));
            }
            else
            {
                changes.Add((lineStart + indent, 0,
                    Encoding.UTF8.GetBytes(start + " ")));
                changes.Add((lineStart + contentEnd, 0,
                    Encoding.UTF8.GetBytes(" " + end)));
            }
        }

        ApplyCommentChanges(changes);
    }

    private void ApplyCommentChanges(
        List<(long Offset, long Remove, byte[] Insert)> changes)
    {
        if (changes.Count == 0)
        {
            return;
        }

        changes.Sort((left, right) => right.Offset.CompareTo(left.Offset));
        List<Selection> selections = CaptureAllSelections();

        foreach ((long offset, long remove, byte[] insert) in changes)
        {
            if (remove == 0 && insert.Length == 0)
            {
                continue;
            }

            foreach (Selection selection in selections)
            {
                selection.Cursor = AdjustOffset(selection.Cursor, offset, remove,
                    insert.Length);
                selection.Anchor = AdjustOffset(selection.Anchor, offset, remove,
                    insert.Length);
            }

            _document.Replace(offset, remove, insert);
        }

        SetSelections(selections);
        _document.BreakUndoGroup();
    }

    private static long AdjustOffset(long position, long offset, long removed,
        long inserted)
    {
        if (position < offset)
        {
            return position;
        }
        if (position >= offset + removed)
        {
            return position + inserted - removed;
        }
        return offset + Math.Min(position - offset, inserted);
    }

    private void EnsureCursorVisible()
    {
        long line = CursorLine();
        int rows = TextRows();

        if (rows == 0)
        {
            _topLine = line;
        }
        else
        {
            if (line < _topLine)
            {
                _topLine = line;
            }
            else if (line >= _topLine + rows)
            {
                _topLine = line - rows + 1;
            }

            long maxTop = _document.LineCount > rows ? _document.LineCount - rows : 0;
            if (_topLine > maxTop)
            {
                _topLine = maxTop;
            }
        }

        int gutter = GutterWidth();
        int textWidth = _columns > gutter ? _columns - gutter : 1;
        long column = CursorVisualColumn();

        if (column < _leftColumn)
        {
            _leftColumn = column;
        }
        else if (column >= _leftColumn + textWidth)
        {
            _leftColumn = column - textWidth + 1;
        }
        if (_leftColumn > column)
        {
            _leftColumn = column;
        }
    }

    // -----------------------------------------------------------------------
    // RENDERING
    // -----------------------------------------------------------------------

    private int GutterWidth()
    {
        int digits = DigitCount(_document.LineCount);
        int gutter = digits + 1;
        if (_columns < gutter + 10)
        {
            return 0;
        }
        return gutter;
    }

    private static int DigitCount(long value)
    {
        int digits = 1;
        while (value >= 10)
        {
            value /= 10;
            ++digits;
        }
        return digits;
    }

    private long StatusRow() => _rows >= 2 ? _rows - 2 : _rows - 1;

    private long CommandRow() => _rows - 1;

    private string ModeLabel()
    {
        if (_prompt != PromptKind.None)
        {
            return "CONFIRM";
        }

        return _mode switch
        {
            Mode.Insert => "INSERT",
            Mode.Select => "SELECT",
            Mode.Command => "COMMAND",
            Mode.PluginPrompt => "PROMPT",
            _ => "NORMAL"
        };
    }

    private string StatusLeft(long line, long column)
    {
        _ = line;
        _ = column;
        string text = $" {ModeLabel()}  {_document.DisplayName}";
        if (_document.IsDirty)
        {
            text += " [+]";
        }
        if (_language is not null)
        {
            text += $"  [{_language.Name}]";
        }
        if (_extraSelections.Count > 0)
        {
            text += $"  [{_extraSelections.Count + 1} cursors]";
        }
        if (_saveOperation is not null)
        {
            text += " [saving]";
        }

        string pluginStatus = PluginStatusText();
        if (pluginStatus.Length > 0)
        {
            text += "  " + pluginStatus;
        }
        return text;
    }

    private string StatusRight(long line, long column)
    {
        string text = $"{line + 1}:{column + 1}  {line + 1}/{_document.LineCount}  ";
        text += _document.Eol == "\r\n" ? "CRLF" : "LF";
        if (_document.HasBom)
        {
            text += " BOM";
        }
        return text;
    }

    private void RenderLineNumbers(long firstLine, int rows, int gutter)
    {
        if (gutter <= 0)
        {
            return;
        }

        int digits = gutter - 1;
        long current = CursorLine();
        long total = _document.LineCount;

        for (int row = 0; row < rows; ++row)
        {
            long line = firstLine + row;
            if (line >= total)
            {
                break;
            }

            long number = line + 1;
            int x = digits - DigitCount(number);
            if (x < 0)
            {
                x = 0;
            }

            var style = new Style();
            if (line == current)
            {
                style.Fg = 15;
                style.Bold = true;
            }
            else
            {
                style.Fg = 8;
                style.Dim = true;
            }

            _screen.PutInt(x, row, number, style, digits);
        }
    }

    private void RenderTextLines(long firstLine, int rows, int gutter)
    {
        if (rows == 0 || _columns <= gutter)
        {
            return;
        }

        long total = _document.LineCount;
        int textWidth = _columns - gutter;

        for (int row = 0; row < rows; ++row)
        {
            long line = firstLine + row;
            if (line >= total)
            {
                break;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
            _syntax.GetLineTokens(line, text, _lineTokens);
            int tokenIndex = 0;

            int offset = 0;
            long visual = 0;

            while (offset < length)
            {
                int runeStart = offset;
                while (tokenIndex < _lineTokens.Count &&
                       _lineTokens[tokenIndex].End <= runeStart)
                {
                    ++tokenIndex;
                }

                SyntaxTokenKind tokenKind = SyntaxTokenKind.Default;
                if (tokenIndex < _lineTokens.Count &&
                    _lineTokens[tokenIndex].Start <= runeStart)
                {
                    tokenKind = _lineTokens[tokenIndex].Kind;
                }
                Style textStyle = TokenStyle(tokenKind);

                UnicodeUtil.Decode(text, offset, out Rune rune, out int byteLength);
                offset += byteLength;

                uint cp = (uint)rune.Value;
                int width;
                if (cp == '\t')
                {
                    width = TabWidth - (int)(visual % TabWidth);
                }
                else
                {
                    width = UnicodeUtil.DisplayWidth(cp);
                    if (width == 0)
                    {
                        if (cp < 0x20 || cp == 0x7F)
                        {
                            cp = 0xFFFD;
                            width = 1;
                        }
                        else
                        {
                            continue; // combining mark
                        }
                    }
                }

                long nextVisual = visual + width;

                if (nextVisual <= _leftColumn)
                {
                    visual = nextVisual;
                    continue;
                }

                if (visual < _leftColumn)
                {
                    long visible = nextVisual - _leftColumn;
                    for (long i = 0; i < visible && i < textWidth; ++i)
                    {
                        _screen.Put(gutter + (int)i, row, ' ', textStyle);
                    }
                    visual = nextVisual;
                    continue;
                }

                long screenX = gutter + (visual - _leftColumn);
                if (screenX >= _columns)
                {
                    break;
                }

                int available = _columns - (int)screenX;
                if (width <= available)
                {
                    if (cp == '\t')
                    {
                        for (int i = 0; i < width; ++i)
                        {
                            _screen.Put((int)screenX + i, row, ' ', textStyle);
                        }
                    }
                    else
                    {
                        _screen.Put((int)screenX, row, cp, textStyle, width);
                    }
                }
                else
                {
                    for (int i = 0; i < available; ++i)
                    {
                        _screen.Put((int)screenX + i, row, ' ', textStyle);
                    }
                }

                visual = nextVisual;
                if (visual - _leftColumn >= textWidth)
                {
                    break;
                }
            }
        }
    }

    private void RenderSelection(long firstLine, int rows, int gutter)
    {
        if (_mode != Mode.Select || rows <= 0 || _columns <= gutter)
        {
            return;
        }

        foreach (Selection selection in CaptureAllSelections())
        {
            (long selectionStart, long selectionEnd) = SelectionRange(
                selection.Anchor, selection.Cursor, selection.Linewise);
            if (selectionEnd > selectionStart)
            {
                RenderSelectionRange(selectionStart, selectionEnd,
                    selection.Linewise, firstLine, rows, gutter);
            }
        }
    }

    private void RenderSelectionRange(long selectionStart, long selectionEnd,
        bool linewise, long firstLine, int rows, int gutter)
    {
        var style = new Style { Fg = 15, Bg = 4 };
        long total = _document.LineCount;

        for (int row = 0; row < rows; ++row)
        {
            long line = firstLine + row;
            if (line >= total)
            {
                break;
            }

            long lineStart = _document.LineStart(line);
            long lineEnd = _document.LineEnd(line);
            long nextLineStart = line + 1 < total
                ? _document.LineStart(line + 1)
                : _document.Size;

            if (linewise)
            {
                if (selectionStart < nextLineStart && selectionEnd > lineStart)
                {
                    for (int x = gutter; x < _columns; ++x)
                    {
                        _screen.ApplyStyle(x, row, style);
                    }
                }
                continue;
            }

            long from = Math.Max(selectionStart, lineStart);
            long to = Math.Min(selectionEnd, lineEnd);
            if (to <= from)
            {
                // A selection that ends exactly on a line break still needs a
                // visible cell when the line itself is empty.
                if (selectionStart == lineStart && selectionEnd > lineStart &&
                    lineEnd == lineStart)
                {
                    if (gutter < _columns)
                    {
                        _screen.ApplyStyle(gutter, row, style);
                    }
                }
                continue;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int localFrom = (int)Math.Clamp(from - lineStart, 0, length);
            int localTo = (int)Math.Clamp(to - lineStart, 0, length);

            long visualFrom = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length), localFrom, TabWidth);
            long visualTo = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length), localTo, TabWidth);

            long firstX = Math.Max(gutter, gutter + visualFrom - _leftColumn);
            long lastX = Math.Min(_columns, gutter + visualTo - _leftColumn);

            for (long x = firstX; x < lastX; ++x)
            {
                _screen.ApplyStyle((int)x, row, style);
            }
        }
    }

    private void RenderCursor(long firstLine, int rows, int gutter)
    {
        foreach (Selection selection in CaptureAllSelections())
        {
            RenderCursorAt(selection.Cursor, firstLine, rows, gutter);
        }
    }

    private void RenderCursorAt(long cursor, long firstLine, int rows, int gutter)
    {
        long line = _document.LineOfOffset(cursor);
        if (line < firstLine || line >= firstLine + rows)
        {
            return;
        }

        int row = (int)(line - firstLine);
        long column = VisualColumnOfOffset(cursor);
        if (column < _leftColumn)
        {
            return;
        }

        long x = gutter + (column - _leftColumn);
        if (x >= _columns || row >= _rows)
        {
            return;
        }

        Style style = _screen.At((int)x, row).Style;
        style.Reverse = true;
        _screen.ApplyStyle((int)x, row, style);
    }

    private void RenderStatusLine()
    {
        long y = StatusRow();
        if (y >= _rows)
        {
            return;
        }

        var style = new Style();
        if (_prompt != PromptKind.None)
        {
            style.Fg = 15;
            style.Bg = 5;
            style.Bold = true;
        }
        else if (_mode == Mode.Insert)
        {
            style.Fg = 0;
            style.Bg = 2;
            style.Bold = true;
        }
        else if (_mode == Mode.Select)
        {
            style.Fg = 0;
            style.Bg = 3;
            style.Bold = true;
        }
        else if (_mode == Mode.PluginPrompt)
        {
            style.Fg = 0;
            style.Bg = 6;
            style.Bold = true;
        }
        else if (_mode == Mode.Command)
        {
            style.Fg = 0;
            style.Bg = 6;
            style.Bold = true;
        }
        else
        {
            style.Fg = 15;
            style.Bg = 4;
            style.Bold = true;
        }

        _screen.Fill(0, (int)y, _columns, 1, ' ', style);

        long line = CursorLine();
        long column = CursorVisualColumn();
        string left = StatusLeft(line, column);
        string right = StatusRight(line, column);

        int rightWidth = (int)UnicodeUtil.ColumnWidth(right);
        if (rightWidth + 3 > _columns)
        {
            right = $"{line + 1}:{column + 1}";
            rightWidth = (int)UnicodeUtil.ColumnWidth(right);
        }

        int rightX = _columns - rightWidth - 1;
        if (rightX < 1)
        {
            rightX = 1;
        }

        int leftMax = rightX - 1;
        if (leftMax > 0)
        {
            _screen.PutString(1, (int)y, left, style, leftMax);
        }

        int rightMax = _columns - rightX;
        if (rightMax > 0 && rightX < _columns)
        {
            _screen.PutString(rightX, (int)y, right, style, rightMax);
        }
    }

    private void RenderBottomLine()
    {
        long y = CommandRow();
        if (y >= _rows)
        {
            return;
        }

        var baseStyle = new Style();
        _screen.Fill(0, (int)y, _columns, 1, ' ', baseStyle);

        if (_mode == Mode.PluginPrompt)
        {
            RenderPluginPrompt((int)y, baseStyle);
            return;
        }

        if (_mode == Mode.Command)
        {
            var promptStyle = new Style { Fg = 14, Bold = true };
            _screen.Put(0, (int)y, ':', promptStyle);

            string line = _commandLine.ToString();
            _screen.PutString(1, (int)y, line, baseStyle, _columns - 1);

            long visual = UnicodeUtil.VisualColumn(line, _commandCursor, 1);
            long x = 1 + visual;
            if (x < _columns)
            {
                Style cursorStyle = _screen.At((int)x, (int)y).Style;
                cursorStyle.Reverse = true;
                _screen.ApplyStyle((int)x, (int)y, cursorStyle);
            }
            return;
        }

        if (_prompt != PromptKind.None)
        {
            string text = _prompt == PromptKind.Quit
                ? "Unsaved changes. Quit without saving? (y/N)"
                : $"Unsaved changes. Discard and open '{_pendingOpen}'? (y/N)";

            var style = new Style { Fg = 11, Bold = true };
            _screen.PutString(0, (int)y, text, style, _columns);
            return;
        }

        if (_message.Length > 0)
        {
            var style = new Style();
            if (_messageIsError)
            {
                style.Fg = 9;
                style.Bold = true;
            }
            _screen.PutString(0, (int)y, _message, style, _columns);
        }
    }

    private int PickerHeight()
    {
        int available = (int)Math.Max(0, StatusRow());
        return Math.Max(0, Math.Min(18, available - 1));
    }

    private int PickerListRows()
    {
        return Math.Max(1, PickerHeight() - 3);
    }

    private int PickerPreviewHeight(int available)
    {
        if (_pickerKind != PickerKind.Files || available < 10)
        {
            return 0;
        }

        int preview = Math.Min(12, Math.Max(4, Math.Min(12, available - 5)));
        return available - preview - 1 >= 4 ? preview : 0;
    }

    private int PickerDialogHeight(int available)
    {
        int preview = PickerPreviewHeight(available);
        if (preview <= 0)
        {
            return PickerHeight();
        }

        return Math.Max(4, Math.Min(18, available - preview - 1));
    }

    private int PickerLayoutRows()
    {
        int available = (int)Math.Max(0, StatusRow());
        return Math.Max(1, PickerDialogHeight(available) - 3);
    }

    private void DrawBox(int x, int y, int width, int height, string title)
    {
        if (width < 2 || height < 2)
        {
            return;
        }

        var fillStyle = new Style();
        var borderStyle = new Style { Fg = 8 };
        var titleStyle = new Style { Fg = 14, Bold = true };

        _screen.Fill(x, y, width, height, ' ', fillStyle);

        for (int column = 1; column < width - 1; ++column)
        {
            _screen.Put(x + column, y, (uint)'─', borderStyle);
            _screen.Put(x + column, y + height - 1, (uint)'─', borderStyle);
        }

        for (int row = 1; row < height - 1; ++row)
        {
            _screen.Put(x, y + row, (uint)'│', borderStyle);
            _screen.Put(x + width - 1, y + row, (uint)'│', borderStyle);
        }

        _screen.Put(x, y, (uint)'┌', borderStyle);
        _screen.Put(x + width - 1, y, (uint)'┐', borderStyle);
        _screen.Put(x, y + height - 1, (uint)'└', borderStyle);
        _screen.Put(x + width - 1, y + height - 1, (uint)'┘', borderStyle);

        if (title.Length > 0 && width > 6)
        {
            _screen.PutString(x + 2, y, $" {title} ", titleStyle, width - 4);
        }
    }

    private void RenderSpaceMenu()
    {
        if (!_spaceMenuOpen)
        {
            return;
        }

        int contentWidth = 0;
        foreach ((char _, string label) in SpaceMenuItems)
        {
            contentWidth = Math.Max(contentWidth, label.Length + 4);
        }

        int width = Math.Max(14, contentWidth + 2);
        int height = SpaceMenuItems.Length + 2;
        int statusRow = (int)StatusRow();
        if (_columns < 10 || statusRow < height + 1)
        {
            return;
        }

        if (width > _columns - 2)
        {
            width = _columns - 2;
        }

        int x = _columns - width - 1;
        int y = statusRow - height;
        if (x < 0)
        {
            x = 0;
        }
        if (y < 0)
        {
            y = 0;
        }

        DrawBox(x, y, width, height, "SPACE");

        var keyStyle = new Style { Fg = 14, Bold = true };
        var labelStyle = new Style { Fg = 15 };

        for (int i = 0; i < SpaceMenuItems.Length; ++i)
        {
            (char key, string label) = SpaceMenuItems[i];
            int row = y + 1 + i;
            _screen.PutString(x + 2, row, key.ToString(), keyStyle, width - 3);
            _screen.PutString(x + 4, row, label, labelStyle, width - 5);
        }
    }

    private void RenderPicker()
    {
        if (_pickerKind == PickerKind.None)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        int previewHeight = PickerPreviewHeight(available);
        int height = PickerDialogHeight(available);

        if (height < 4 || _columns < 20)
        {
            return;
        }

        int width = Math.Min(_columns - 2, 88);
        if (width < 20)
        {
            return;
        }

        int totalHeight = height + (previewHeight > 0 ? previewHeight + 1 : 0);
        int x = (_columns - width) / 2;
        int y = Math.Max(0, (available - totalHeight) / 2);
        string title = _pickerKind switch
        {
            PickerKind.Files => "Open file",
            PickerKind.Plugin => _pluginSelectTitle.Length > 0
                ? _pluginSelectTitle
                : "Select",
            PickerKind.CodeActions => "Code actions",
            PickerKind.Problems => _problemsWorkspace
                ? "Workspace diagnostics"
                : "Document diagnostics",
            _ => "Search across files"
        };

        DrawBox(x, y, width, height, title);

        var promptStyle = new Style { Fg = 14, Bold = true };
        var queryStyle = new Style { Fg = 15 };
        var selectedStyle = new Style { Fg = 15, Bold = true, Reverse = true };

        _screen.PutString(x + 2, y + 1, ">", promptStyle, width - 4);

        string query = _pickerQuery.ToString();
        int queryWidth = Math.Max(0, width - 5);
        _screen.PutString(x + 4, y + 1, query, queryStyle, queryWidth);

        long queryColumn = UnicodeUtil.VisualColumn(query, _pickerQueryCursor, 1);
        if (queryColumn < queryWidth)
        {
            int cursorX = x + 4 + (int)queryColumn;
            if (cursorX < x + width - 1)
            {
                Style cursorStyle = _screen.At(cursorX, y + 1).Style;
                cursorStyle.Reverse = true;
                _screen.ApplyStyle(cursorX, y + 1, cursorStyle);
            }
        }

        int listRows = height - 3;
        for (int row = 0; row < listRows; ++row)
        {
            int index = _pickerTop + row;
            if (index >= _pickerItems.Count)
            {
                break;
            }

            int screenRow = y + 2 + row;
            Style style = index == _pickerSelected ? selectedStyle : new Style();
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(x + 2, screenRow, _pickerItems[index].Display, style,
                width - 4);
        }

        string status = _pickerLoading
            ? (_pickerKind == PickerKind.Files ? "Loading files..." : "Searching...")
            : _pickerStatus;
        if (status.Length == 0)
        {
            status = _pickerItems.Count == 0
                ? "No items"
                : $"{_pickerItems.Count} item{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        }

        var statusStyle = new Style { Fg = 14 };
        _screen.PutString(x + 2, y + height - 1, $" {status} ", statusStyle, width - 4);

        if (previewHeight > 0)
        {
            RenderPickerPreview(x, y + height + 1, width, previewHeight);
        }
    }

    private void RenderPickerPreview(int x, int y, int width, int height)
    {
        string relativePath = _pickerPreviewPath;
        if (relativePath.Length > 0)
        {
            try
            {
                relativePath = Path.GetRelativePath(_pickerRoot, relativePath);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException)
            {
                // Keep the absolute path.
            }
        }

        string title = relativePath.Length > 0
            ? $"Preview: {relativePath}"
            : "Preview";
        if (_pickerPreviewStatus.Length > 0)
        {
            title += $" — {_pickerPreviewStatus}";
        }

        DrawBox(x, y, width, height, title);

        var style = new Style { Fg = 15 };
        var dimStyle = new Style { Fg = 8, Dim = true };
        var errorStyle = new Style { Fg = 9, Bold = true };
        int contentRows = Math.Max(0, height - 2);
        int contentWidth = Math.Max(0, width - 4);

        if (_pickerPreviewLoading)
        {
            _screen.PutString(x + 2, y + 1, "Loading preview...", dimStyle, contentWidth);
            return;
        }

        if (_pickerPreviewStatus.StartsWith("E: ", StringComparison.Ordinal))
        {
            _screen.PutString(x + 2, y + 1, _pickerPreviewStatus, errorStyle,
                contentWidth);
            return;
        }

        if (_pickerPreviewBinary)
        {
            _screen.PutString(x + 2, y + 1, "Binary file", dimStyle, contentWidth);
            return;
        }

        if (_pickerPreviewLines.Count == 0)
        {
            _screen.PutString(x + 2, y + 1,
                _pickerPreviewStatus.Length > 0 ? _pickerPreviewStatus : "No preview",
                dimStyle, contentWidth);
            return;
        }

        for (int row = 0; row < contentRows && row < _pickerPreviewLines.Count; ++row)
        {
            _screen.PutString(x + 2, y + 1 + row,
                ExpandPreviewLine(_pickerPreviewLines[row]), style, contentWidth);
        }
    }

    private static string ExpandPreviewLine(string line)
    {
        var builder = new StringBuilder(line.Length + 8);
        int column = 0;

        foreach (char character in line)
        {
            if (character == '\t')
            {
                int spaces = TabWidth - (column % TabWidth);
                builder.Append(' ', spaces);
                column += spaces;
            }
            else if (character < 0x20 || character == 0x7F)
            {
                builder.Append(' ');
                ++column;
            }
            else
            {
                builder.Append(character);
                ++column;
            }
        }

        return builder.ToString();
    }

    private void HandleHoverInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
            case Key.Enter:
                CloseHover();
                return;
            case Key.Up:
                if (_hoverTop > 0)
                {
                    --_hoverTop;
                }
                return;
            case Key.Down:
                if (_hoverTop + HoverContentRows() < _hoverLines.Count)
                {
                    ++_hoverTop;
                }
                return;
            case Key.PageUp:
                _hoverTop = Math.Max(0, _hoverTop - Math.Max(1, HoverContentRows()));
                return;
            case Key.PageDown:
                _hoverTop = Math.Min(
                    Math.Max(0, _hoverLines.Count - HoverContentRows()),
                    _hoverTop + Math.Max(1, HoverContentRows()));
                return;
        }

        if (inputEvent.Key == Key.Character &&
            (inputEvent.Character.Value == 'q' || inputEvent.Character.Value == 'Q'))
        {
            CloseHover();
        }
    }

    private int HoverContentRows()
    {
        int available = (int)Math.Max(0, StatusRow());
        int height = Math.Min(available - 1, _hoverLines.Count + 2);
        return Math.Max(1, height - 2);
    }

    private void RenderHover()
    {
        if (!_hoverVisible || _hoverLines.Count == 0)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        if (available < 5 || _columns < 20)
        {
            return;
        }

        int height = Math.Min(available - 1, _hoverLines.Count + 2);
        height = Math.Max(3, height);
        int width = Math.Min(_columns - 2, 92);
        if (width < 20)
        {
            return;
        }

        int x = (_columns - width) / 2;
        int y = Math.Max(0, (available - height) / 2);
        DrawBox(x, y, width, height, $"{_hoverTitle} (Esc to close)");

        var style = new Style { Fg = 15 };
        int contentRows = Math.Max(0, height - 2);
        int contentWidth = Math.Max(0, width - 4);

        for (int row = 0; row < contentRows; ++row)
        {
            int index = _hoverTop + row;
            if (index >= _hoverLines.Count)
            {
                break;
            }

            _screen.PutString(x + 2, y + 1 + row, _hoverLines[index], style,
                contentWidth);
        }
    }

    private void Render()
    {
        _screen.Resize(_columns, _rows);
        _screen.Clear();

        EnsureCursorVisible();

        int rows = TextRows();
        long total = _document.LineCount;

        if (rows > 0)
        {
            long maxTop = total > rows ? total - rows : 0;
            if (_topLine > maxTop)
            {
                _topLine = maxTop;
            }
        }
        else
        {
            _topLine = CursorLine();
        }

        int gutter = GutterWidth();
        if (rows > 0)
        {
            RenderLineNumbers(_topLine, rows, gutter);
            RenderTextLines(_topLine, rows, gutter);
            RenderSelection(_topLine, rows, gutter);
            RenderCursor(_topLine, rows, gutter);
        }

        RenderStatusLine();
        RenderBottomLine();
        RenderSpaceMenu();
        RenderPicker();
        RenderHover();

        ReadOnlySpan<byte> output = _renderer.Render(_screen, _forceFullRedraw);
        if (!output.IsEmpty)
        {
            _terminal.Write(output);
        }
        _forceFullRedraw = false;
    }

    private void SetMessage(string text)
    {
        _message = text;
        _messageIsError = false;
    }

    private void SetError(string text)
    {
        _message = text.StartsWith("E: ", StringComparison.Ordinal) ? text : $"E: {text}";
        _messageIsError = true;
    }

    private void ClearMessage()
    {
        _message = string.Empty;
        _messageIsError = false;
    }
}
