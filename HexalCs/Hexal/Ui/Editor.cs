using System.Diagnostics;
using System.Text;
using System.Text.RegularExpressions;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const int BottomLinePadding = 1;
    private const int MouseScrollLines = 3;
    private const int MaxFindMatches = 50000;
    private const int MaxPickerPreviewCacheEntries = 128;

    private static readonly byte[] Lf = { (byte)'\n' };
    private static readonly byte[] CrLf = { (byte)'\r', (byte)'\n' };

    private enum Mode
    {
        Normal,
        Insert,
        Select,
        Command,
        PluginPrompt,
    }

    private enum PromptKind
    {
        None,
        Quit,
        Open,
        SidebarDelete,
        SidebarDiff,
    }

    private enum PickerKind
    {
        None,
        Files,
        Search,
        Plugin,
        CodeActions,
        Problems,
        Palette,
        Themes,
    }

    private enum CompletionKind
    {
        None,
        Commands,
        Arguments,
        Files,
    }

    private static readonly (char Key, string Label)[] SpaceMenuItems =
    {
        ('/', "Search across files"),
        ('f', "Open file picker"),
        ('y', "Yank selection to clipboard"),
        ('X', "Cut selection to clipboard"),
        ('p', "Paste on the current line"),
        ('P', "Paste on a new line"),
        ('K', "LSP hover"),
        ('A', "LSP code actions"),
        ('S', "Document symbols"),
        ('D', "Workspace diagnostics"),
        ('d', "Document diagnostics"),
        ('b', "Toggle breakpoint"),
    };

    private static readonly string[] BuiltinCommands =
    {
        "e",
        "w",
        "wq",
        "q",
        "fmt",
        "cw",
        "cfg",
        "rcfg",
        "lang",
        "help",
        "about",
        "a",
        "debug-lsp",
        "dl",
        "debug",
        "dbg",
        "dbg-break",
        "dbg-cont",
        "dbg-next",
        "dbg-step",
        "dbg-out",
        "dbg-pause",
        "dbg-stop",
        "dbg-sv",
        "dbg-vars",
        "dbg-watch",
        "dbg-unwatch",
        "dbg-clear-watches",
        "debug-dap",
        "dd",
        "s",
        "t",
        "gi",
        "gdl",
        "smc",
        "vs",
        "hs",
        "cb",
        "adi",
    };

    private static readonly Dictionary<string, string> BuiltinCommandDescriptions = new(
        StringComparer.OrdinalIgnoreCase
    )
    {
        ["e"] = "Open a file in the current buffer.",
        ["w"] = "Write the current buffer to disk, optionally to a new path.",
        ["wq"] = "Write the current buffer, then quit when saving finishes.",
        ["q"] = "Quit the editor; use :q! to discard unsaved changes.",
        ["fmt"] = "Format the current buffer with its language formatter.",
        ["cw"] = "Remove trailing spaces and tabs from every line.",
        ["cfg"] = "Open (or create) the Hexal configuration file.",
        ["rcfg"] = "Reload configuration, grammars, themes and language servers.",
        ["lang"] = "Set the language grammar for the current buffer.",
        ["help"] = "Show a short reminder of the available keys.",
        ["about"] = "Show information about Hexal itself.",
        ["a"] = "Alias for :about.",
        ["debug-lsp"] = "Show live language-server traffic and diagnostics.",
        ["dl"] = "Alias for :debug-lsp.",
        ["debug"] = "Start a debug session, optionally with an executable.",
        ["dbg"] = "Alias for :debug.",
        ["dbg-break"] = "Toggle a breakpoint on the current line.",
        ["dbg-cont"] = "Continue the stopped debuggee.",
        ["dbg-next"] = "Step over the current source line.",
        ["dbg-step"] = "Step into the current call.",
        ["dbg-out"] = "Step out of the current function.",
        ["dbg-pause"] = "Pause the running debuggee.",
        ["dbg-stop"] = "Stop the debug session.",
        ["dbg-sv"] = "Show or hide scopes, variables and watches.",
        ["dbg-vars"] = "Alias for :dbg-sv.",
        ["dbg-watch"] = "Watch a variable or expression while debugging.",
        ["dbg-unwatch"] = "Remove a debug watch expression.",
        ["dbg-clear-watches"] = "Remove all debug watch expressions.",
        ["debug-dap"] = "Show live debug-adapter traffic and state.",
        ["dd"] = "Alias for :debug-dap.",
        ["s"] = "Indent with spaces; :s <width> also sets the indent width.",
        ["t"] = "Indent with tabs and set the visual tab width (:t <width>).",
        ["gi"] = "Toggle showing git-ignored files in the sidebar and pickers.",
        ["gdl"] = "Toggle git diff markers in the line-number gutter.",
        ["smc"] = "Toggle case-sensitive matching for all searches.",
        ["vs"] = "Split the current pane vertically (side by side).",
        ["hs"] = "Split the current pane horizontally (stacked).",
        ["cb"] = "Close the current buffer pane; use :q to quit the editor.",
        ["adi"] = "Detect the document indentation and apply it to this buffer.",
    };

    private readonly ITerminal _terminal;
    private readonly Renderer _renderer = new();
    private Screen _screen = new();

    private Document _document;
    private string _workspaceRoot;
    private readonly bool _openFilePickerOnStart;
    private StartupBenchmark? _benchmark;

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
    private string _pickerTitle = string.Empty;
    private List<string> _pickerFiles = new();
    private string _pickerFilesRoot = string.Empty;
    private bool _pickerFilesLoaded;
    private string _themeBeforePicker = string.Empty;
    private CancellationTokenSource? _pickerCancellation;
    private Task<PickerScanResult>? _pickerTask;
    private long _pickerGeneration;

    private readonly List<string> _pickerPreviewLines = new();
    private string _pickerPreviewPath = string.Empty;
    private string _pickerPreviewStatus = string.Empty;
    private bool _pickerPreviewBinary;
    private readonly Dictionary<string, FilePreview> _pickerPreviewCache = new(
        StringComparer.Ordinal
    );
    private readonly Queue<string> _pickerPreviewCacheOrder = new();

    private readonly List<LspCodeAction> _codeActions = new();
    private bool _pendingCodeActionResolve;
    private bool _problemsWorkspace;

    private bool _hoverVisible;
    private readonly List<HoverLine> _hoverLines = new();
    private string _hoverTitle = "Hover";
    private int _hoverTop;
    private bool _hoverRequestPending;
    private bool _definitionRequestPending;
    private bool _codeActionRequestPending;
    private bool _symbolRequestPending;

    private sealed class Selection
    {
        public long Cursor;
        public long Anchor;
        public bool Linewise;
        public long PreferredColumn;
        public bool PreferredColumnValid;

        public Selection Clone() =>
            new()
            {
                Cursor = Cursor,
                Anchor = Anchor,
                Linewise = Linewise,
                PreferredColumn = PreferredColumn,
                PreferredColumnValid = PreferredColumnValid,
            };
    }

    private readonly record struct JumpLocation(string? Path, long Line, long Column);

    private readonly record struct FindMatch(
        long Start,
        long End,
        long Line,
        string Replacement = ""
    );

    private readonly struct HoverSegment
    {
        public HoverSegment(string text, SyntaxTokenKind kind)
        {
            Text = text;
            Kind = kind;
        }

        public string Text { get; }

        public SyntaxTokenKind Kind { get; }
    }

    private sealed class HoverLine
    {
        public readonly List<HoverSegment> Segments = new();
    }

    private long _cursor;
    private long _selectionAnchor;
    private bool _selectionLinewise;
    private List<Selection> _extraSelections = new();
    private bool _findActive;
    private bool _findEditing;
    private bool _findMultiCursor;
    private bool _findRegex;
    private bool _findReplace;
    private bool _findEditingReplacement;
    private string _findError = string.Empty;
    private List<Selection>? _findSavedSelections;
    private StringBuilder _findQuery = new();
    private int _findQueryCursor;
    private StringBuilder _findReplacement = new();
    private int _findReplacementCursor;
    private List<(long Start, long End)> _findScope = new();
    private List<FindMatch> _findMatches = new();
    private int _findMatchIndex = -1;
    private long _topLine;
    private long _leftColumn;
    private long _preferredColumn;
    private bool _preferredColumnValid;
    private bool _freeScroll;
    private bool _mouseDragging;
    private long _mouseDragAnchor;

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

    private bool _completionVisible;
    private CompletionKind _completionKind;
    private readonly List<string> _completionItems = new();
    private bool _lspCompletionVisible;
    private bool _lspCompletionPending;
    private readonly List<LspCompletionItem> _lspCompletionItems = new();
    private int _lspCompletionSelected;
    private int _lspCompletionTop;
    private int _completionSelected;
    private int _completionTop;
    private string _completionPrefix = string.Empty;
    private string _completionSuffix = string.Empty;
    private char _completionQuote;
    private string _completionStatus = string.Empty;
    private List<string>? _completionFiles;
    private string _completionFilesRoot = string.Empty;

    private List<JumpLocation> _jumpHistory = new();
    private List<JumpLocation> _jumpForward = new();
    private bool _pendingG;

    private byte[] _lineBuffer = Array.Empty<byte>();

    private LanguageRegistry _languages;
    private HexalConfig _config = new();
    private bool _searchMatchCase;
    private ThemeRegistry _themes = new();
    private Theme _theme = Theme.CreateDefault();
    private SyntaxHighlighter _syntax = new();
    private readonly List<SyntaxToken> _lineTokens = new();
    private LanguageDefinition? _language;

    private sealed class PickerItem
    {
        public string Display = string.Empty;
        public string Detail = string.Empty;
        public string Path = string.Empty;
        public string Key = string.Empty;
        public long Line = -1;
        public long Column;
        public int Value;
    }

    private sealed class PickerScanResult
    {
        public long Generation;
        public bool FileList;
        public string Root = string.Empty;
        public List<string> Files = new();
        public List<FileSearchMatch> Matches = new();
        public string Error = string.Empty;
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

    private readonly record struct FormatSelectionSnapshot(
        long CursorLine,
        long CursorColumn,
        long AnchorLine,
        long AnchorColumn,
        bool Linewise
    );

    private sealed class FormatOperation
    {
        public Task<FormatResult> Task = null!;
        public Document Document = null!;
        public long Revision;
        public string? SavePath;
        public bool SaveAfter;
        public bool QuitAfter;
        public List<FormatSelectionSnapshot> Selections = new();
    }

    private SaveOperation? _saveOperation;
    private FormatOperation? _formatOperation;
    private CancellationTokenSource? _formatCancellation;
    private bool _quitAfterSave;
    private bool _resaveRequested;
    private bool _waitThenQuit;
    private bool _waitThenQuitForce;

    public Editor(
        ITerminal terminal,
        Document document,
        string? workspaceRoot = null,
        bool openFilePicker = false
    )
    {
        LuaPluginManager.BeginWarmup();
        _terminal = terminal;
        _document = document;
        _workspaceRoot = ResolveWorkspaceRoot(workspaceRoot);
        _openFilePickerOnStart = openFilePicker;
        _config = HexalConfig.LoadDefault(out string configError);
        if (_config.StartupBenchmarker)
        {
            _benchmark = new StartupBenchmark();
        }

        _showDiffLines = _config.ShowDiffLines;
        _showGitIgnored = _config.ShowGitIgnored;
        _searchMatchCase = _config.SearchMatchCaseEnabled;

        // Grammar parsing and Lua warm-up are independent of the initial
        // frame. Start them in the background so the terminal can be painted
        // immediately; the registry is applied as soon as it is ready.
        _languages = new LanguageRegistry();
        _startupGrammarStarted = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
        _startupGrammarTask = Task.Run(LanguageRegistry.LoadUserGrammars);

        long phaseStart = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
        _themes = ThemeRegistry.LoadDefault(out string themeError);
        _benchmark?.AddSince(
            "Internal",
            "Theme registry",
            phaseStart,
            $"{_themes.Themes.Count} themes"
        );
        _theme = _themes.ActiveTheme;

        phaseStart = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
        RefreshLanguage();
        RefreshFileStamp();
        SnapCursor();
        InitializePanes();
        _benchmark?.AddSince("Internal", "Editor initialization", phaseStart);

        if (themeError.Length > 0)
        {
            SetError($"theme: {themeError}");
        }
        else if (configError.Length > 0)
        {
            SetError($"config: {configError}");
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
            catch (Exception ex)
                when (ex
                        is ArgumentException
                            or NotSupportedException
                            or PathTooLongException
                            or UnauthorizedAccessException
                            or IOException
                )
            {
                // Fall back to the current directory below.
            }
        }

        try
        {
            return Directory.GetCurrentDirectory();
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            return ".";
        }
    }

    public int Run()
    {
        RefreshSize();
        EnsureCursorVisible();
        if (_openFilePickerOnStart)
        {
            OpenPicker(PickerKind.Files);
        }

        // Paint the editor before plugin loading, git diff inspection and
        // other startup work. This removes the blank-terminal flash on slow
        // filesystems and leaves the UI responsive during the remaining work.
        Render();
        _benchmark?.AddSince("Startup", "UI first render", _editorStartTimestamp);

        PollStartupWork();
        LoadPlugins();
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
            bool busy =
                _saveOperation is not null
                || _formatOperation is not null
                || _sidebarFormatOperation is not null
                || _pickerTask is not null
                || _lsp.IsRunning
                || _lspDirty
                || _dap.IsRunning
                || _startupGrammarTask is not null;
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
            if (!_freeScroll)
            {
                EnsureCursorVisible();
            }
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
            UpdateEditorLeft();
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

        if (_pluginPanelVisible)
        {
            HandlePluginPanelInput(inputEvent);
            return _running;
        }

        if (_benchmarkVisible)
        {
            HandleStartupBenchmarkInput(inputEvent);
            return _running;
        }

        if (_aboutVisible)
        {
            HandleAboutInput(inputEvent);
            return _running;
        }

        if (_pluginManagerVisible)
        {
            HandlePluginManagerInput(inputEvent);
            return _running;
        }

        if (_lspDebugVisible)
        {
            HandleLspDebugInput(inputEvent);
            return _running;
        }

        if (_dapDebugVisible)
        {
            HandleDapDebugInput(inputEvent);
            return _running;
        }

        if (inputEvent.Key is not (Key.ScrollUp or Key.ScrollDown))
        {
            _freeScroll = false;
        }

        if (
            inputEvent.Key
            is Key.MouseDown
                or Key.MouseUp
                or Key.MouseMove
                or Key.ScrollUp
                or Key.ScrollDown
        )
        {
            if (
                !_hoverVisible
                && !_spaceMenuOpen
                && !_findActive
                && _prompt == PromptKind.None
                && _pickerKind == PickerKind.None
                && _mode is Mode.Normal or Mode.Insert or Mode.Select
            )
            {
                if (_sidebarVisible && inputEvent.MouseX < SidebarWidth())
                {
                    HandleSidebarMouse(inputEvent);
                }
                else
                {
                    HandleEditorMouse(inputEvent);
                }
            }
            return _running;
        }

        if (
            inputEvent.Key == Key.Character
            && inputEvent.Ctrl
            && inputEvent.Character.Value is 'b' or 'B'
            && _pickerKind == PickerKind.None
            && _prompt == PromptKind.None
            && !_spaceMenuOpen
            && !_findActive
            && _mode is Mode.Normal or Mode.Insert or Mode.Select
        )
        {
            ToggleSidebar();
            return _running;
        }

        if (
            inputEvent.Key == Key.Character
            && inputEvent.Ctrl
            && inputEvent.Character.Value is 'e' or 'E'
            && _pickerKind == PickerKind.None
            && _prompt == PromptKind.None
            && !_spaceMenuOpen
            && !_findActive
            && _mode is Mode.Normal or Mode.Insert or Mode.Select
        )
        {
            ToggleBreakpoint();
            return _running;
        }

        if (
            inputEvent.Key == Key.Character
            && inputEvent.Ctrl
            && inputEvent.Character.Value is 'p' or 'P'
            && _pickerKind == PickerKind.None
            && _prompt == PromptKind.None
            && !_spaceMenuOpen
            && !_findActive
            && _mode is Mode.Normal or Mode.Insert or Mode.Select
        )
        {
            OpenCommandPalette();
            return _running;
        }

        if (_pendingWindow)
        {
            _pendingWindow = false;
            HandlePaneCommand(inputEvent);
            return _running;
        }

        if (
            inputEvent.Key == Key.Character
            && inputEvent.Ctrl
            && inputEvent.Character.Value is 'w' or 'W'
            && _pickerKind == PickerKind.None
            && _prompt == PromptKind.None
            && !_spaceMenuOpen
            && !_findActive
            && _mode is Mode.Normal or Mode.Insert or Mode.Select
        )
        {
            _pendingWindow = true;
            SetMessage("Pane: arrows/hjkl focus, +/- resize, q close, Esc cancel");
            return _running;
        }

        if (_hoverVisible)
        {
            HandleHoverInput(inputEvent);
            return _running;
        }

        if (
            _plugins is not null
            && _prompt == PromptKind.None
            && _pickerKind == PickerKind.None
            && !_spaceMenuOpen
            && !_findActive
            && _mode is Mode.Normal or Mode.Insert or Mode.Select
        )
        {
            string keymapMode = _mode switch
            {
                Mode.Insert => "insert",
                Mode.Select => "select",
                _ => "normal",
            };

            if (_plugins.TryHandleKey(inputEvent, keymapMode, out bool handled) && handled)
            {
                return _running;
            }
        }

        try
        {
            if (_findActive)
            {
                HandleFind(inputEvent);
            }
            else if (_pickerKind != PickerKind.None)
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
            else if (_sidebarFocused && _sidebarVisible)
            {
                HandleSidebar(inputEvent);
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

        if (!CheckExternalFile())
        {
            RefreshDiffLines();
        }
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

        if (
            current.Exists == _fileStamp.Exists
            && current.Length == _fileStamp.Length
            && current.LastWriteTicks == _fileStamp.LastWriteTicks
            && current.CreationTicks == _fileStamp.CreationTicks
        )
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
        AutoDetectIndentationOnOpen();

        if (_mode == Mode.Select)
        {
            _mode = Mode.Normal;
            _selectionLinewise = false;
        }

        _pendingG = false;
        _extraSelections.Clear();

        long line = Math.Clamp(oldLine, 0, _document.LineCount - 1);
        long lineStart = _document.LineStart(line);
        long lineEnd = _document.LineEnd(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        long column = UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, length),
            oldColumn,
            TabWidth
        );

        _cursor = Math.Clamp(lineStart + column, lineStart, lineEnd);
        _selectionAnchor = _cursor;
        _preferredColumnValid = false;
        _topLine = Math.Max(0, line - Math.Max(0, oldTopOffset));
        SnapCursor();
        _fileStamp = stamp;
        RefreshDiffLines();

        if (_findActive)
        {
            RebuildFindMatches();
        }

        if (wasDirty)
        {
            SetMessage(
                $"Reloaded {_document.DisplayName} (external change; unsaved edits replaced)"
            );
        }
        else
        {
            SetMessage($"Reloaded {_document.DisplayName} (changed on disk)");
        }

        Dictionary<string, object?> opened = CurrentBufferData();
        opened["reloaded"] = true;
        FirePluginEvent("buffer_opened", opened);
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
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
                        or PathTooLongException
            )
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
                else if (character is 'n' or 'N')
                {
                    MoveToNextLocation();
                }
                else if (character is 'd' or 'D')
                {
                    GoToDefinition();
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
                _symbolRequestPending = false;
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
            case '/':
                OpenFind();
                return;
            case '\\':
                OpenFind(regex: true);
                return;
            case '\'':
                OpenFind(regex: true, replace: true);
                return;
            case '.':
                OpenFind(multiCursor: true);
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
                        _cursor =
                            begin + UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
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
            case '%':
                SelectAll();
                return;
            case 'x':
            case 'X':
                ForEachSelection(StartLineSelection);
                return;
            case '<':
                IndentSelection(unindent: true);
                return;
            case '>':
                IndentSelection(unindent: false);
                return;
            case 'C':
                AddCursorBelow();
                return;
            case ',':
                KeepPrimarySelection();
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
                else if (character is 'n' or 'N')
                {
                    MoveToNextLocation();
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
            case ' ':
                OpenSpaceMenu();
                return;
            case '%':
                SelectAll();
                return;
            case '.':
                OpenFind(multiCursor: true);
                return;
            case 'x':
            case 'X':
                ForEachSelection(ExtendLineSelection);
                return;
            case 'd':
            case 'D':
                ForEachSelection(DeleteSelection, editing: true);
                return;
            case '<':
                IndentSelection(unindent: true);
                return;
            case '>':
                IndentSelection(unindent: false);
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
            case ',':
                KeepPrimarySelection();
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
        if (_lspCompletionVisible && HandleLspCompletionInput(inputEvent))
        {
            return;
        }

        _lspCompletionPending = false;

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
            else if (inputEvent.Character.Value == ' ')
            {
                RequestLspCompletionManually();
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
                ForEachSelection(InsertIndent, editing: true);
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
                if (_completionVisible)
                {
                    CloseCompletion();
                    return;
                }
                _mode = Mode.Normal;
                CloseCompletion();
                _commandLine.Clear();
                _commandCursor = 0;
                _commandHistoryIndex = -1;
                return;
            case Key.Enter:
                if (_completionVisible && _completionItems.Count > 0)
                {
                    AcceptCompletion();
                    return;
                }
                SubmitCommandLine();
                return;
            case Key.Tab:
                if (_completionVisible)
                {
                    MoveCompletionSelection(1, wrap: true);
                    return;
                }
                TryOpenCompletion();
                return;
            case Key.BackTab:
                if (_completionVisible)
                {
                    MoveCompletionSelection(-1, wrap: true);
                }
                return;
            case Key.Backspace:
                CloseCompletion();
                if (_commandCursor > 0)
                {
                    string text = _commandLine.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, _commandCursor);
                    _commandLine.Remove(previous, _commandCursor - previous);
                    _commandCursor = previous;
                }
                return;
            case Key.Delete:
                CloseCompletion();
                if (_commandCursor < _commandLine.Length)
                {
                    string text = _commandLine.ToString();
                    int next = UnicodeUtil.NextBoundary(text, _commandCursor);
                    _commandLine.Remove(_commandCursor, next - _commandCursor);
                }
                return;
            case Key.Left:
            {
                CloseCompletion();
                string text = _commandLine.ToString();
                _commandCursor = UnicodeUtil.PreviousBoundary(text, _commandCursor);
                return;
            }
            case Key.Right:
            {
                CloseCompletion();
                string text = _commandLine.ToString();
                _commandCursor = UnicodeUtil.NextBoundary(text, _commandCursor);
                return;
            }
            case Key.Home:
                CloseCompletion();
                _commandCursor = 0;
                return;
            case Key.End:
                CloseCompletion();
                _commandCursor = _commandLine.Length;
                return;
            case Key.Up:
            case Key.Down:
                if (_completionVisible)
                {
                    MoveCompletionSelection(inputEvent.Key == Key.Up ? -1 : 1, wrap: false);
                    return;
                }

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
            CloseCompletion();
            string inserted = inputEvent.Character.ToString();
            _commandLine.Insert(_commandCursor, inserted);
            _commandCursor += inserted.Length;
        }
    }

    private void SubmitCommandLine()
    {
        CloseCompletion();

        string line = _commandLine.ToString();
        _mode = Mode.Normal;
        _commandLine.Clear();
        _commandCursor = 0;
        _commandHistoryIndex = -1;

        if (line.Length > 0 && (_commandHistory.Count == 0 || _commandHistory[^1] != line))
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
    }

    private bool TryOpenCompletion()
    {
        string line = _commandLine.ToString();
        int cursor = Math.Clamp(_commandCursor, 0, line.Length);

        int firstStart = 0;
        while (firstStart < line.Length && char.IsWhiteSpace(line[firstStart]))
        {
            ++firstStart;
        }

        int firstEnd = firstStart;
        while (firstEnd < line.Length && !char.IsWhiteSpace(line[firstEnd]))
        {
            ++firstEnd;
        }

        string command = line[firstStart..firstEnd];
        bool fileCommand = IsFileCompletionCommand(command);
        bool atOrInsideCommand = cursor <= firstEnd;
        bool commandHasArguments = firstEnd < line.Length && line[firstEnd..].Trim().Length > 0;

        if (atOrInsideCommand && (!fileCommand || cursor < firstEnd || commandHasArguments))
        {
            int filterEnd = Math.Clamp(cursor, firstStart, firstEnd);
            string filter = line[firstStart..filterEnd];
            List<string> items = FilterCommands(filter);
            return OpenCompletion(
                CompletionKind.Commands,
                items,
                line[..firstStart],
                line[firstEnd..],
                '\0',
                filter
            );
        }

        if (fileCommand)
        {
            int argumentStart = firstEnd;
            while (argumentStart < line.Length && char.IsWhiteSpace(line[argumentStart]))
            {
                ++argumentStart;
            }

            if (cursor < argumentStart)
            {
                cursor = argumentStart;
            }

            char quote = '\0';
            int tokenStart = argumentStart;
            int tokenEnd = cursor;

            if (
                argumentStart < line.Length
                && (line[argumentStart] == '"' || line[argumentStart] == '\'')
            )
            {
                quote = line[argumentStart];
                tokenStart = argumentStart + 1;
                int closing = line.IndexOf(quote, tokenStart);
                tokenEnd = closing >= 0 ? closing : Math.Max(tokenStart, cursor);
            }

            tokenStart = Math.Clamp(tokenStart, 0, line.Length);
            tokenEnd = Math.Clamp(tokenEnd, tokenStart, line.Length);

            string filter = line[tokenStart..tokenEnd];
            List<string> items = FilterFiles(CompletionFiles(), filter);
            string prefix = line[..tokenStart];

            // ":e" has no separator yet; add one when the completion is
            // accepted so the result is ":e <path>" rather than ":e<path>".
            if (tokenStart == firstEnd && firstEnd == line.Length)
            {
                prefix += ' ';
            }

            return OpenCompletion(
                CompletionKind.Files,
                items,
                prefix,
                line[tokenEnd..],
                quote,
                filter
            );
        }

        if (command is "lang")
        {
            int argumentStart = firstEnd;
            while (argumentStart < line.Length && char.IsWhiteSpace(line[argumentStart]))
            {
                ++argumentStart;
            }

            if (cursor < argumentStart)
            {
                cursor = argumentStart;
            }

            int tokenStart = Math.Clamp(argumentStart, 0, line.Length);
            int tokenEnd = Math.Clamp(cursor, tokenStart, line.Length);
            string filter = line[tokenStart..tokenEnd];

            List<string> languages = _languages
                .Languages.Select(language => language.Id)
                .OrderBy(id => id, StringComparer.OrdinalIgnoreCase)
                .ToList();

            return OpenCompletion(
                CompletionKind.Arguments,
                FilterFiles(languages, filter),
                line[..tokenStart],
                line[tokenEnd..],
                '\0',
                filter
            );
        }

        SetMessage("No completions available.");
        return true;
    }

    private bool OpenCompletion(
        CompletionKind kind,
        List<string> items,
        string prefix,
        string suffix,
        char quote,
        string filter
    )
    {
        _completionVisible = true;
        _completionKind = kind;
        _completionItems.Clear();
        _completionItems.AddRange(items);
        _completionSelected = 0;
        _completionTop = 0;
        _completionPrefix = prefix;
        _completionSuffix = suffix;
        _completionQuote = quote;
        _completionStatus =
            items.Count > 0 ? string.Empty
            : filter.Length == 0 ? "No completions available"
            : $"No matches for '{filter}'";
        ClearMessage();
        return true;
    }

    private void CloseCompletion()
    {
        _completionVisible = false;
        _completionKind = CompletionKind.None;
        _completionItems.Clear();
        _completionSelected = 0;
        _completionTop = 0;
        _completionPrefix = string.Empty;
        _completionSuffix = string.Empty;
        _completionQuote = '\0';
        _completionStatus = string.Empty;
    }

    private void MoveCompletionSelection(int delta, bool wrap)
    {
        int count = _completionItems.Count;
        if (count == 0)
        {
            return;
        }

        int index = _completionSelected + delta;
        if (wrap)
        {
            index = ((index % count) + count) % count;
        }
        else
        {
            index = Math.Clamp(index, 0, count - 1);
        }

        _completionSelected = index;
        int rows = CompletionMaxRows();
        if (_completionSelected < _completionTop)
        {
            _completionTop = _completionSelected;
        }
        else if (_completionSelected >= _completionTop + rows)
        {
            _completionTop = _completionSelected - rows + 1;
        }

        int maxTop = Math.Max(0, count - rows);
        _completionTop = Math.Clamp(_completionTop, 0, maxTop);
    }

    private void AcceptCompletion()
    {
        if (
            !_completionVisible
            || _completionItems.Count == 0
            || _completionSelected < 0
            || _completionSelected >= _completionItems.Count
        )
        {
            return;
        }

        string selected = _completionItems[_completionSelected];
        CompletionKind kind = _completionKind;
        string inserted =
            kind == CompletionKind.Files ? CompletionPathForCommand(selected) : selected;

        if (kind == CompletionKind.Files)
        {
            if (_completionQuote == '\0')
            {
                if (NeedsPathQuotes(inserted))
                {
                    inserted = QuoteCompletionPath(inserted);
                }
            }
            else
            {
                if (_completionQuote == '"')
                {
                    inserted = inserted.Replace("\"", "\\\"");
                }

                if (_completionSuffix.Length == 0 || _completionSuffix[0] != _completionQuote)
                {
                    inserted += _completionQuote;
                }
            }
        }

        string line = _completionPrefix + inserted + _completionSuffix;
        int cursor = _completionPrefix.Length + inserted.Length;
        CloseCompletion();
        _commandLine.Clear();
        _commandLine.Append(line);
        _commandCursor = Math.Clamp(cursor, 0, _commandLine.Length);

        // File completion is usually the last part of the command, so accept
        // and run it in one step. Command names stay in command mode so the
        // user can add arguments (for example, :e <file>).
        if (kind == CompletionKind.Files)
        {
            SubmitCommandLine();
        }
    }

    private static bool IsFileCompletionCommand(string command) =>
        command is "e" or "w" or "debug" or "dbg";

    private string CompletionPathForCommand(string relativePath)
    {
        try
        {
            string fullPath = Path.GetFullPath(Path.Combine(ProjectRoot(), relativePath));
            return Path.GetRelativePath(Directory.GetCurrentDirectory(), fullPath);
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
            return relativePath;
        }
    }

    private static bool NeedsPathQuotes(string path)
    {
        foreach (char character in path)
        {
            if (char.IsWhiteSpace(character))
            {
                return true;
            }
        }
        return false;
    }

    private static string QuoteCompletionPath(string path) =>
        "\"" + path.Replace("\"", "\\\"") + "\"";

    private string CommandDescription(string name)
    {
        if (BuiltinCommandDescriptions.TryGetValue(name, out string? description))
        {
            return description;
        }

        if (_plugins is not null && _plugins.TryGetCommandDescription(name, out description))
        {
            return description.Length > 0 ? description : "Plugin command.";
        }

        return string.Empty;
    }

    private static List<string> WrapText(string text, int width)
    {
        var lines = new List<string>();
        if (width <= 0 || text.Length == 0)
        {
            return lines;
        }

        var current = new StringBuilder();
        foreach (string word in text.Split(' ', StringSplitOptions.RemoveEmptyEntries))
        {
            string remaining = word;
            while (remaining.Length > width)
            {
                if (current.Length > 0)
                {
                    lines.Add(current.ToString());
                    current.Clear();
                }

                lines.Add(remaining[..width]);
                remaining = remaining[width..];
            }

            if (remaining.Length == 0)
            {
                continue;
            }

            if (current.Length == 0)
            {
                current.Append(remaining);
            }
            else if (current.Length + 1 + remaining.Length <= width)
            {
                current.Append(' ').Append(remaining);
            }
            else
            {
                lines.Add(current.ToString());
                current.Clear();
                current.Append(remaining);
            }
        }

        if (current.Length > 0)
        {
            lines.Add(current.ToString());
        }

        return lines;
    }

    private List<string> FilterCommands(string filter)
    {
        var names = new List<string>(BuiltinCommands);
        if (_plugins is not null)
        {
            names.AddRange(_plugins.CommandNames);
        }

        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var matches = new List<(int Score, string Name)>();
        foreach (string name in names)
        {
            if (name.Length == 0 || !seen.Add(name))
            {
                continue;
            }

            int score = FuzzyScore(name, filter);
            if (score < int.MaxValue)
            {
                matches.Add((score, name));
            }
        }

        matches.Sort(
            (left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(left.Name, right.Name, StringComparison.OrdinalIgnoreCase);
            }
        );

        var result = new List<string>(matches.Count);
        foreach ((int _, string name) in matches)
        {
            result.Add(name);
        }
        return result;
    }

    private List<string> FilterFiles(IReadOnlyList<string> files, string filter)
    {
        if (filter.Length == 0)
        {
            return new List<string>(files);
        }

        var matches = new List<(int Score, string Path)>();
        foreach (string path in files)
        {
            int score = FuzzyScore(path, filter);
            if (score < int.MaxValue)
            {
                matches.Add((score, path));
            }
        }

        matches.Sort(
            (left, right) =>
            {
                int comparison = left.Score.CompareTo(right.Score);
                return comparison != 0
                    ? comparison
                    : string.Compare(left.Path, right.Path, StringComparison.OrdinalIgnoreCase);
            }
        );

        var result = new List<string>(matches.Count);
        foreach ((int _, string path) in matches)
        {
            result.Add(path);
        }
        return result;
    }

    private List<string> CompletionFiles()
    {
        string root = ProjectRoot();
        if (_pickerFilesLoaded && _pickerRoot.Length > 0 && SamePath(_pickerRoot, root))
        {
            return _pickerFiles;
        }

        if (
            _completionFiles is not null
            && _completionFilesRoot.Length > 0
            && SamePath(_completionFilesRoot, root)
        )
        {
            return _completionFiles;
        }

        _completionFiles = FileSearch.EnumerateFiles(
            root,
            FileSearch.DefaultMaxFiles,
            _showGitIgnored,
            CancellationToken.None
        );
        _completionFilesRoot = root;
        return _completionFiles;
    }

    private int CompletionMaxRows()
    {
        int available = (int)Math.Max(0, CommandRow());
        return Math.Max(1, Math.Min(9, available - 2));
    }

    private void HandleEditorMouse(InputEvent inputEvent)
    {
        if (
            inputEvent.Key == Key.MouseDown
            && inputEvent.MouseButton == MouseButton.Left
            && TryFindPaneAt(inputEvent.MouseX, inputEvent.MouseY, out EditorPane clickedPane)
            && !ReferenceEquals(clickedPane, _activePane)
        )
        {
            SwitchToPane(clickedPane);
            _forceFullRedraw = true;
        }

        switch (inputEvent.Key)
        {
            case Key.ScrollUp:
                ScrollDocument(-MouseScrollLines);
                return;
            case Key.ScrollDown:
                ScrollDocument(MouseScrollLines);
                return;
            case Key.MouseDown:
                _mouseDragging = false;
                if (
                    inputEvent.MouseButton == MouseButton.Left
                    && MoveCursorToScreen(inputEvent.MouseX, inputEvent.MouseY)
                )
                {
                    _mouseDragging = true;
                    _mouseDragAnchor = _cursor;
                }
                return;
            case Key.MouseMove:
                if (_mouseDragging && inputEvent.MouseButton == MouseButton.Left)
                {
                    ExtendMouseSelection(inputEvent.MouseX, inputEvent.MouseY);
                }
                return;
            case Key.MouseUp:
                if (_mouseDragging && inputEvent.MouseButton == MouseButton.Left)
                {
                    bool extend = _mode == Mode.Select;
                    if (
                        !extend
                        && TryOffsetAtScreen(
                            inputEvent.MouseX,
                            inputEvent.MouseY,
                            out long releaseOffset
                        )
                    )
                    {
                        extend = releaseOffset != _mouseDragAnchor;
                    }

                    if (extend)
                    {
                        ExtendMouseSelection(inputEvent.MouseX, inputEvent.MouseY);
                        if (_mode == Mode.Select && _selectionAnchor == _cursor)
                        {
                            _mode = Mode.Normal;
                            _selectionLinewise = false;
                        }
                    }

                    _mouseDragging = false;
                }
                return;
        }
    }

    private void ScrollDocument(int delta)
    {
        int rows = ActivePaneRows();
        if (rows <= 0)
        {
            return;
        }

        long maxTop = Math.Max(0, _document.LineCount - rows);
        long top = Math.Clamp(_topLine + delta, 0, maxTop);
        if (top == _topLine)
        {
            return;
        }

        _topLine = top;
        _freeScroll = true;
        ClearMessage();
    }

    private bool MoveCursorToScreen(int x, int y)
    {
        if (!TryOffsetAtScreen(x, y, out long offset))
        {
            return false;
        }

        _cursor = offset;

        if (_mode == Mode.Select)
        {
            _mode = Mode.Normal;
            _selectionLinewise = false;
        }

        _selectionAnchor = _cursor;
        _extraSelections.Clear();
        _preferredColumnValid = false;
        _freeScroll = false;
        _document.BreakUndoGroup();
        SnapCursor();
        ClearMessage();
        return true;
    }

    private void ExtendMouseSelection(int x, int y)
    {
        if (!TryOffsetAtScreen(x, y, out long offset))
        {
            return;
        }

        if (_mode != Mode.Select)
        {
            _mode = Mode.Select;
            _selectionLinewise = false;
            _extraSelections.Clear();
        }

        _selectionAnchor = _mouseDragAnchor;
        _cursor = offset;
        _preferredColumnValid = false;
        _freeScroll = false;
        _document.BreakUndoGroup();
        SnapCursor();
        ClearMessage();
    }

    private bool TryOffsetAtScreen(int x, int y, out long offset)
    {
        offset = 0;
        PaneBounds bounds = ActivePaneBounds();
        int localX = x - bounds.X;
        int localY = y - bounds.Y;
        if (
            localX < 0
            || localY < 0
            || bounds.Height <= 0
            || localX >= bounds.Width
            || localY >= bounds.Height
        )
        {
            return false;
        }

        long line = _topLine + localY;
        if (line < 0 || line >= _document.LineCount)
        {
            return false;
        }

        int gutter = GutterWidthFor(bounds.Width);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        long visual = localX > gutter ? _leftColumn + (localX - gutter) : 0;
        int local = UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, length),
            visual,
            TabWidth
        );
        long start = _document.LineStart(line);
        long end = _document.LineEnd(line);
        offset = Math.Clamp(start + local, start, end);
        return true;
    }

    private void HandlePrompt(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            _prompt = PromptKind.None;
            ClearPendingOpen();
            ClearPendingSidebarDelete();
            ClearPendingSidebarDiff();
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
            else if (kind == PromptKind.SidebarDelete)
            {
                DeleteSidebarPaths();
            }
            else if (kind == PromptKind.SidebarDiff)
            {
                string diff = _pendingSidebarDiff;
                string summary = _pendingSidebarDiffLabel;
                ClearPendingSidebarDiff();
                ShowSidebarDiff(diff, summary);
            }
            return;
        }

        if (character is 'n' or 'N')
        {
            _prompt = PromptKind.None;
            ClearPendingOpen();
            ClearPendingSidebarDelete();
            ClearPendingSidebarDiff();
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
            case 'y':
            case 'Y':
                YankSelection();
                return;
            case 'x':
            case 'X':
                CutSelection();
                return;
            case 'p':
                PasteClipboard(newLine: false);
                return;
            case 'P':
                PasteClipboard(newLine: true);
                return;
            case 'K':
            case 'k':
                ShowHover();
                return;
            case 'A':
            case 'a':
                ShowCodeActions();
                return;
            case 'S':
            case 's':
                ShowDocumentSymbols();
                return;
            case 'D':
                ShowProblems(workspace: true);
                return;
            case 'd':
                ShowProblems(workspace: false);
                return;
            case 'b':
            case 'B':
                ToggleBreakpoint();
                return;
            default:
                // Helix closes the which-key menu when an unbound key is
                // pressed; the key does not fall through to NORMAL mode.
                return;
        }
    }

    private void OpenFind(bool multiCursor = false, bool regex = false, bool replace = false)
    {
        _findActive = true;
        _findEditing = true;
        _findMultiCursor = multiCursor;
        _findRegex = regex;
        _findReplace = replace && regex;
        _findEditingReplacement = false;
        _findError = string.Empty;
        _findQuery.Clear();
        _findQueryCursor = 0;
        _findReplacement.Clear();
        _findReplacementCursor = 0;
        _findMatches.Clear();
        _findMatchIndex = -1;
        _findScope.Clear();
        _findSavedSelections = null;

        // A multi-cursor search started from SELECT mode only considers
        // matches fully contained in the active selection, so replacing with
        // Delete/I cannot affect text outside it. Keep the original selection
        // so cancelling the dialog can restore it.
        if (multiCursor && _mode == Mode.Select)
        {
            _findSavedSelections = CaptureAllSelections();
            CaptureFindScope(_findSavedSelections);
        }

        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    private void CloseFind()
    {
        List<Selection>? savedSelections = _findSavedSelections;

        _findActive = false;
        _findEditing = false;
        _findMultiCursor = false;
        _findRegex = false;
        _findReplace = false;
        _findEditingReplacement = false;
        _findError = string.Empty;
        _findSavedSelections = null;
        _findQuery.Clear();
        _findQueryCursor = 0;
        _findReplacement.Clear();
        _findReplacementCursor = 0;
        _findScope.Clear();
        _findMatches.Clear();
        _findMatchIndex = -1;

        if (savedSelections is not null)
        {
            SetSelections(savedSelections);
            _mode = Mode.Select;
        }

        _document.BreakUndoGroup();
    }

    private void CaptureFindScope(IReadOnlyList<Selection> selections)
    {
        var ranges = new List<(long Start, long End)>();
        foreach (Selection selection in selections)
        {
            (long start, long end) = SelectionRange(
                selection.Anchor,
                selection.Cursor,
                selection.Linewise
            );
            if (end > start)
            {
                ranges.Add((start, end));
            }
        }

        if (ranges.Count == 0)
        {
            return;
        }

        ranges.Sort((left, right) => left.Start.CompareTo(right.Start));
        long mergedStart = ranges[0].Start;
        long mergedEnd = ranges[0].End;
        for (int i = 1; i < ranges.Count; ++i)
        {
            (long start, long end) = ranges[i];
            if (start <= mergedEnd)
            {
                mergedEnd = Math.Max(mergedEnd, end);
                continue;
            }

            _findScope.Add((mergedStart, mergedEnd));
            mergedStart = start;
            mergedEnd = end;
        }

        _findScope.Add((mergedStart, mergedEnd));
    }

    private bool IsInFindScope(long start, long end)
    {
        if (_findScope.Count == 0)
        {
            return true;
        }

        int low = 0;
        int high = _findScope.Count - 1;
        int candidate = -1;
        while (low <= high)
        {
            int middle = low + ((high - low) / 2);
            if (_findScope[middle].Start <= start)
            {
                candidate = middle;
                low = middle + 1;
            }
            else
            {
                high = middle - 1;
            }
        }

        return candidate >= 0 && end <= _findScope[candidate].End;
    }

    private StringBuilder ActiveFindField =>
        _findReplace && _findEditingReplacement ? _findReplacement : _findQuery;

    private int ActiveFindCursor
    {
        get => _findReplace && _findEditingReplacement ? _findReplacementCursor : _findQueryCursor;
        set
        {
            if (_findReplace && _findEditingReplacement)
            {
                _findReplacementCursor = value;
            }
            else
            {
                _findQueryCursor = value;
            }
        }
    }

    private void HandleFind(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            CloseFind();
            return;
        }

        if (!_findEditing)
        {
            switch (inputEvent.Key)
            {
                case Key.Enter:
                    _findEditing = true;
                    return;
                case Key.Up:
                    FindPrevious();
                    return;
                case Key.Down:
                    FindNext();
                    return;
            }

            if (inputEvent.Key == Key.Character && !inputEvent.Ctrl && !inputEvent.Alt)
            {
                switch (inputEvent.Character.Value)
                {
                    case 'n':
                    case 'N':
                        FindNext();
                        return;
                    case 'p':
                    case 'P':
                        FindPrevious();
                        return;
                    case '/':
                        _findEditing = true;
                        return;
                }
            }
            return;
        }

        if ((inputEvent.Key is Key.Tab or Key.BackTab) && _findReplace)
        {
            _findEditingReplacement = !_findEditingReplacement;
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Enter:
                if (_findReplace)
                {
                    if (!_findEditingReplacement)
                    {
                        _findEditingReplacement = true;
                        return;
                    }

                    ReplaceCurrentFindMatch();
                    return;
                }

                if (_findMultiCursor)
                {
                    ApplyMultiCursorFind();
                    return;
                }
                _findEditing = false;
                return;
            case Key.Up:
                FindPrevious();
                return;
            case Key.Down:
                FindNext();
                return;
            case Key.Backspace:
            {
                StringBuilder buffer = ActiveFindField;
                int cursor = ActiveFindCursor;
                if (cursor > 0)
                {
                    string text = buffer.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, cursor);
                    buffer.Remove(previous, cursor - previous);
                    ActiveFindCursor = previous;
                    RebuildFindMatches();
                }
                return;
            }
            case Key.Delete:
            {
                StringBuilder buffer = ActiveFindField;
                int cursor = ActiveFindCursor;
                if (cursor < buffer.Length)
                {
                    string text = buffer.ToString();
                    int next = UnicodeUtil.NextBoundary(text, cursor);
                    buffer.Remove(cursor, next - cursor);
                    RebuildFindMatches();
                }
                return;
            }
            case Key.Left:
                ActiveFindCursor = UnicodeUtil.PreviousBoundary(
                    ActiveFindField.ToString(),
                    ActiveFindCursor
                );
                return;
            case Key.Right:
                ActiveFindCursor = UnicodeUtil.NextBoundary(
                    ActiveFindField.ToString(),
                    ActiveFindCursor
                );
                return;
            case Key.Home:
                ActiveFindCursor = 0;
                return;
            case Key.End:
                ActiveFindCursor = ActiveFindField.Length;
                return;
        }

        if (inputEvent.Key != Key.Character)
        {
            return;
        }

        uint codePoint = (uint)inputEvent.Character.Value;
        if (inputEvent.Ctrl)
        {
            if (codePoint is 'n' or 'N')
            {
                FindNext();
            }
            else if (codePoint is 'p' or 'P')
            {
                FindPrevious();
            }
            else if (_findReplace && codePoint is 'a' or 'A')
            {
                ReplaceAllFindMatches();
            }
            return;
        }

        if (inputEvent.Alt || codePoint < 0x20 || codePoint == 0x7F)
        {
            return;
        }

        string inserted = inputEvent.Character.ToString();
        StringBuilder field = ActiveFindField;
        int fieldCursor = ActiveFindCursor;
        field.Insert(fieldCursor, inserted);
        ActiveFindCursor = fieldCursor + inserted.Length;
        RebuildFindMatches();
    }

    private void ReplaceCurrentFindMatch()
    {
        if (_findError.Length > 0)
        {
            return;
        }

        if (_findMatches.Count == 0)
        {
            SetMessage("No matches to replace.");
            return;
        }

        int index =
            _findMatchIndex >= 0 && _findMatchIndex < _findMatches.Count ? _findMatchIndex : 0;
        FindMatch match = _findMatches[index];
        byte[] replacement = Encoding.UTF8.GetBytes(match.Replacement);

        _document.BeginCompositeEdit();
        _document.Replace(match.Start, match.End - match.Start, replacement);
        _document.EndCompositeEdit();
        _document.BreakUndoGroup();

        _cursor = match.Start + replacement.Length;
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _preferredColumnValid = false;
        SnapCursor();
        RebuildFindMatches();
        SetMessage($"Replaced 1 match ({_findMatches.Count} remaining).");
    }

    private void ReplaceAllFindMatches()
    {
        if (_findError.Length > 0)
        {
            return;
        }

        if (_findMatches.Count == 0)
        {
            SetMessage("No matches to replace.");
            return;
        }

        int count = _findMatches.Count;
        _document.BeginCompositeEdit();
        for (int i = _findMatches.Count - 1; i >= 0; --i)
        {
            FindMatch match = _findMatches[i];
            byte[] replacement = Encoding.UTF8.GetBytes(match.Replacement);
            _document.Replace(match.Start, match.End - match.Start, replacement);
        }
        _document.EndCompositeEdit();
        _document.BreakUndoGroup();

        _cursor = _findMatches[0].Start;
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _preferredColumnValid = false;
        SnapCursor();
        RebuildFindMatches();
        SetMessage($"Replaced {count} match{(count == 1 ? string.Empty : "es")}.");
    }

    private void RebuildFindMatches()
    {
        _findMatches.Clear();
        _findMatchIndex = -1;
        _findError = string.Empty;

        string query = _findQuery.ToString();
        if (query.Length == 0)
        {
            return;
        }

        if (_findRegex)
        {
            RebuildRegexFindMatches(query);
        }
        else
        {
            RebuildLiteralFindMatches(query);
        }

        if (_findMatches.Count == 0)
        {
            return;
        }

        int selected = 0;
        for (int i = 0; i < _findMatches.Count; ++i)
        {
            if (_findMatches[i].Start >= _cursor)
            {
                selected = i;
                break;
            }
        }

        SelectFindMatch(selected);
    }

    private void RebuildLiteralFindMatches(string query)
    {
        long total = _document.LineCount;
        for (long line = 0; line < total; ++line)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            if (length == 0)
            {
                continue;
            }

            string text = Encoding.UTF8.GetString(_lineBuffer, 0, length);
            int searchStart = 0;
            StringComparison comparison = _searchMatchCase
                ? StringComparison.Ordinal
                : StringComparison.OrdinalIgnoreCase;
            while (searchStart < text.Length)
            {
                int index = text.IndexOf(query, searchStart, comparison);
                if (index < 0)
                {
                    break;
                }

                int matchLength = Math.Min(query.Length, text.Length - index);
                long lineStart = _document.LineStart(line);
                long start = lineStart + Encoding.UTF8.GetByteCount(text.AsSpan(0, index));
                long end = start + Encoding.UTF8.GetByteCount(text.AsSpan(index, matchLength));

                if (IsInFindScope(start, end))
                {
                    _findMatches.Add(new FindMatch(start, end, line));
                    if (_findMatches.Count >= MaxFindMatches)
                    {
                        break;
                    }
                }

                searchStart = index + Math.Max(1, matchLength);
            }

            if (_findMatches.Count >= MaxFindMatches)
            {
                break;
            }
        }
    }

    private void RebuildRegexFindMatches(string pattern)
    {
        Regex regex;
        try
        {
            RegexOptions options = RegexOptions.CultureInvariant;
            if (!_searchMatchCase)
            {
                options |= RegexOptions.IgnoreCase;
            }
            regex = new Regex(pattern, options, TimeSpan.FromMilliseconds(100));
        }
        catch (ArgumentException ex)
        {
            _findError = ex.Message;
            return;
        }

        string replacementPattern = _findReplacement.ToString();
        long total = _document.LineCount;
        for (long line = 0; line < total; ++line)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            string text = Encoding.UTF8.GetString(_lineBuffer, 0, length);
            long lineStart = _document.LineStart(line);

            try
            {
                for (Match match = regex.Match(text); match.Success; match = match.NextMatch())
                {
                    long start =
                        lineStart + Encoding.UTF8.GetByteCount(text.AsSpan(0, match.Index));
                    long end =
                        start + Encoding.UTF8.GetByteCount(text.AsSpan(match.Index, match.Length));

                    string replacement = string.Empty;
                    if (_findReplace)
                    {
                        try
                        {
                            replacement = match.Result(replacementPattern);
                        }
                        catch (ArgumentException ex)
                        {
                            _findError = $"invalid replacement: {ex.Message}";
                            _findMatches.Clear();
                            return;
                        }
                    }

                    if (!IsInFindScope(start, end))
                    {
                        continue;
                    }

                    _findMatches.Add(new FindMatch(start, end, line, replacement));
                    if (_findMatches.Count >= MaxFindMatches)
                    {
                        return;
                    }
                }
            }
            catch (RegexMatchTimeoutException)
            {
                _findError = "regular expression timed out";
                _findMatches.Clear();
                return;
            }
        }
    }

    /// <summary>
    /// Turns every current find match into a selection and leaves FIND mode.
    /// The match nearest the cursor becomes the primary selection so the
    /// viewport stays where the user was searching. Leaving SELECT mode active
    /// lets the normal selection commands (Delete/d, I, indent, ...) operate on
    /// every match at once.
    /// </summary>
    private void ApplyMultiCursorFind()
    {
        if (_findMatches.Count == 0)
        {
            string query = _findQuery.ToString();
            CloseFind();
            SetMessage(query.Length == 0 ? "No search query." : $"No matches for '{query}'.");
            return;
        }

        int primaryIndex =
            _findMatchIndex >= 0 && _findMatchIndex < _findMatches.Count ? _findMatchIndex : 0;

        var selections = new List<Selection>(_findMatches.Count);
        selections.Add(CreateFindSelection(_findMatches[primaryIndex]));
        for (int i = 0; i < _findMatches.Count; ++i)
        {
            if (i != primaryIndex)
            {
                selections.Add(CreateFindSelection(_findMatches[i]));
            }
        }

        int count = selections.Count;
        CloseFind();
        SetSelections(selections);
        _mode = Mode.Select;
        _selectionLinewise = false;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SetMessage($"{count} matching selections.");
    }

    /// <summary>
    /// Builds a character-wise selection covering exactly the match. A
    /// selection includes the scalar under its cursor, so the cursor is placed
    /// at the start of the match's final scalar rather than at its end.
    /// </summary>
    private Selection CreateFindSelection(FindMatch match)
    {
        long cursor = PreviousScalarOffset(match.End);
        return new Selection
        {
            Cursor = cursor,
            Anchor = match.Start,
            Linewise = false,
            PreferredColumn = 0,
            PreferredColumnValid = false,
        };
    }

    private long PreviousScalarOffset(long offset)
    {
        if (offset <= 0)
        {
            return 0;
        }

        long line = _document.LineOfOffset(offset);
        long lineStart = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(offset - lineStart, 0, length);
        return lineStart + UnicodeUtil.PreviousBoundary(_lineBuffer.AsSpan(0, length), local);
    }

    private void SelectFindMatch(int index)
    {
        if (index < 0 || index >= _findMatches.Count)
        {
            _findMatchIndex = -1;
            return;
        }

        _findMatchIndex = index;
        FindMatch match = _findMatches[index];
        _cursor = match.Start;
        _selectionAnchor = _cursor;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void FindNext()
    {
        if (_findMatches.Count == 0)
        {
            RebuildFindMatches();
            return;
        }

        int index = _findMatchIndex < 0 ? 0 : (_findMatchIndex + 1) % _findMatches.Count;
        SelectFindMatch(index);
    }

    private void FindPrevious()
    {
        if (_findMatches.Count == 0)
        {
            RebuildFindMatches();
            return;
        }

        int index =
            _findMatchIndex < 0
                ? _findMatches.Count - 1
                : (_findMatchIndex - 1 + _findMatches.Count) % _findMatches.Count;
        SelectFindMatch(index);
    }

    private void OpenPicker(PickerKind kind, string? root = null, string? title = null)
    {
        _pickerPreviewCache.Clear();
        _pickerPreviewCacheOrder.Clear();
        _pickerKind = kind;
        _pickerQuery.Clear();
        _pickerQueryCursor = 0;
        _pickerItems.Clear();
        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerStatus = string.Empty;
        _pickerLoading = true;
        _pickerRoot = string.IsNullOrEmpty(root) ? ProjectRoot() : root;
        _pickerTitle = title ?? string.Empty;
        _pickerPreviewLines.Clear();
        _pickerPreviewPath = string.Empty;
        _pickerPreviewStatus = string.Empty;
        _pickerPreviewBinary = false;
        _pendingG = false;
        _document.BreakUndoGroup();
        ClearMessage();

        if (kind == PickerKind.Themes)
        {
            _themeBeforePicker = _theme.Id;
        }

        if (
            kind
            is PickerKind.Plugin
                or PickerKind.CodeActions
                or PickerKind.Problems
                or PickerKind.Palette
                or PickerKind.Themes
        )
        {
            BeginPickerQuery();
        }
        else if (!_pickerFilesLoaded || !SamePath(_pickerRoot, _pickerFilesRoot))
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
        Action<int?>? pluginCallback = oldKind == PickerKind.Plugin ? _pluginSelectCallback : null;
        _pluginSelectCallback = null;

        CancelPickerWork();
        _pickerKind = PickerKind.None;
        _pickerQuery.Clear();
        _pickerQueryCursor = 0;
        _pickerItems.Clear();
        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus = string.Empty;
        _pickerTitle = string.Empty;
        _pickerPreviewLines.Clear();
        _pickerPreviewPath = string.Empty;
        _pickerPreviewStatus = string.Empty;
        _codeActions.Clear();
        _pendingCodeActionResolve = false;
        _pluginSelectTitle = string.Empty;
        _pluginSelectItems.Clear();

        if (cancelled && oldKind == PickerKind.Themes && _themeBeforePicker.Length > 0)
        {
            SetActiveTheme(_themeBeforePicker, persist: false, announce: false);
        }
        _themeBeforePicker = string.Empty;

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
        bool showGitIgnored = _showGitIgnored;

        _pickerTask = Task.Run(
            () =>
            {
                var result = new PickerScanResult
                {
                    Generation = generation,
                    FileList = true,
                    Root = root,
                };

                try
                {
                    result.Files = FileSearch.EnumerateFiles(
                        root,
                        FileSearch.DefaultMaxFiles,
                        showGitIgnored,
                        cancellation.Token
                    );
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
            },
            cancellation.Token
        );
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
            case PickerKind.Palette:
                PopulatePaletteItems();
                return;
            case PickerKind.Themes:
                PopulateThemeItems();
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
                _pickerItems.Add(
                    new PickerItem
                    {
                        Display = path,
                        Path = Path.GetFullPath(Path.Combine(_pickerRoot, path)),
                    }
                );
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
                    scored.Add(
                        (
                            score,
                            new PickerItem
                            {
                                Display = path,
                                Path = Path.GetFullPath(Path.Combine(_pickerRoot, path)),
                            }
                        )
                    );
                }
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
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
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
                _pickerItems.Add(new PickerItem { Display = _pluginSelectItems[i], Value = i });
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
                    scored.Add((score, new PickerItem { Display = item, Value = i }));
                }
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
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
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
            string label =
                action.Kind.Length > 0 ? $"[{action.Kind}] {action.Title}" : action.Title;

            if (query.Length > 0 && FuzzyScore(label, query) == int.MaxValue)
            {
                continue;
            }

            _pickerItems.Add(new PickerItem { Display = label, Value = i });
        }

        _pickerSelected = 0;
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
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
                _ => "H",
            };

            string label =
                $"{severity} {relative}:{diagnostic.Line + 1}:{diagnostic.Column + 1}  {diagnostic.Message}";
            if (query.Length > 0 && FuzzyScore(label, query) == int.MaxValue)
            {
                continue;
            }

            _pickerItems.Add(
                new PickerItem
                {
                    Display = label,
                    Path = path,
                    Line = diagnostic.Line,
                    Column = diagnostic.Column,
                    Value = diagnostic.Severity,
                }
            );
        }

        _pickerSelected = Math.Clamp(_pickerSelected, 0, Math.Max(0, _pickerItems.Count - 1));
        _pickerTop = 0;
        _pickerLoading = false;
        _pickerStatus =
            _pickerItems.Count == 0
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
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                continue;
            }

            if (!_problemsWorkspace)
            {
                if (currentPath.Length == 0 || !PathsEqual(path, currentPath))
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

        result.Sort(
            (left, right) =>
            {
                int comparison = string.Compare(
                    left.Path,
                    right.Path,
                    StringComparison.OrdinalIgnoreCase
                );
                if (comparison != 0)
                {
                    return comparison;
                }
                return left.Diagnostic.Line.CompareTo(right.Diagnostic.Line);
            }
        );

        return result;
    }

    private static bool PathsEqual(string left, string right)
    {
        return string.Equals(
            left,
            right,
            OperatingSystem.IsWindows()
                ? StringComparison.OrdinalIgnoreCase
                : StringComparison.Ordinal
        );
    }

    private static bool IsPathInside(string root, string path)
    {
        string normalizedRoot =
            root.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
            + Path.DirectorySeparatorChar;
        string comparisonPath = path;
        if (!comparisonPath.EndsWith(Path.DirectorySeparatorChar))
        {
            comparisonPath += Path.DirectorySeparatorChar;
        }

        return comparisonPath.StartsWith(
            normalizedRoot,
            OperatingSystem.IsWindows()
                ? StringComparison.OrdinalIgnoreCase
                : StringComparison.Ordinal
        );
    }

    private void UpdatePickerPreview()
    {
        if (_pickerKind != PickerKind.Files)
        {
            return;
        }

        if (_pickerSelected < 0 || _pickerSelected >= _pickerItems.Count)
        {
            _pickerPreviewLines.Clear();
            _pickerPreviewPath = string.Empty;
            _pickerPreviewStatus = "No file selected";
            _pickerPreviewBinary = false;
            return;
        }

        PickerItem item = _pickerItems[_pickerSelected];
        if (item.Path.Length == 0 || PathsEqual(_pickerPreviewPath, item.Path))
        {
            return;
        }

        if (!_pickerPreviewCache.TryGetValue(item.Path, out FilePreview? preview))
        {
            preview = FileSearch.ReadPreview(item.Path);
            CachePickerPreview(item.Path, preview);
        }

        ApplyPickerPreview(item.Path, preview);
    }

    private void ApplyPickerPreview(string path, FilePreview preview)
    {
        _pickerPreviewPath = path;
        _pickerPreviewLines.Clear();
        _pickerPreviewLines.AddRange(preview.Lines);
        _pickerPreviewBinary = preview.Binary;

        if (preview.Error.Length > 0)
        {
            _pickerPreviewStatus = $"E: {preview.Error}";
        }
        else if (preview.Binary)
        {
            _pickerPreviewStatus = "Binary file";
        }
        else if (preview.Lines.Count == 0)
        {
            _pickerPreviewStatus = "Empty file";
        }
        else
        {
            _pickerPreviewStatus = preview.Truncated
                ? $"First {preview.Lines.Count} lines (truncated)"
                : $"First {preview.Lines.Count} line{(preview.Lines.Count == 1 ? string.Empty : "s")}";
        }
    }

    private void CachePickerPreview(string path, FilePreview preview)
    {
        if (_pickerPreviewCache.ContainsKey(path))
        {
            return;
        }

        while (
            _pickerPreviewCache.Count >= MaxPickerPreviewCacheEntries
            && _pickerPreviewCacheOrder.Count > 0
        )
        {
            string oldest = _pickerPreviewCacheOrder.Dequeue();
            _pickerPreviewCache.Remove(oldest);
        }

        _pickerPreviewCache[path] = preview;
        _pickerPreviewCacheOrder.Enqueue(path);
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
        bool matchCase = _searchMatchCase;

        _pickerLoading = true;
        _pickerStatus = "Searching...";

        _pickerTask = Task.Run(
            () =>
            {
                var result = new PickerScanResult { Generation = generation };

                try
                {
                    result.Matches = FileSearch.Search(
                        root,
                        files,
                        query,
                        FileSearch.DefaultMaxResults,
                        cancellation.Token,
                        matchCase
                    );
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
            },
            cancellation.Token
        );
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
            _pickerFilesRoot = result.Root;
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
                _pickerItems.Add(
                    new PickerItem
                    {
                        Display = $"{match.Path}:{match.Line + 1}  {match.Preview}",
                        Path = Path.GetFullPath(Path.Combine(_pickerRoot, match.Path)),
                        Line = match.Line,
                        Column = match.Column,
                    }
                );
            }

            _pickerStatus =
                _pickerItems.Count == 0
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
                _pickerQueryCursor = UnicodeUtil.PreviousBoundary(
                    _pickerQuery.ToString(),
                    _pickerQueryCursor
                );
                return;
            case Key.Right:
                _pickerQueryCursor = UnicodeUtil.NextBoundary(
                    _pickerQuery.ToString(),
                    _pickerQueryCursor
                );
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
        else if (_pickerKind == PickerKind.Themes)
        {
            PreviewSelectedTheme();
        }

        if (_pickerKind is PickerKind.Palette or PickerKind.Themes)
        {
            UpdatePickerDetailStatus();
        }
    }

    private void UpdatePickerDetailStatus()
    {
        if (_pickerSelected < 0 || _pickerSelected >= _pickerItems.Count)
        {
            return;
        }

        PickerItem item = _pickerItems[_pickerSelected];
        if (item.Detail.Length == 0)
        {
            return;
        }

        string count =
            _pickerKind == PickerKind.Themes
                ? $"{_pickerItems.Count} theme{(_pickerItems.Count == 1 ? string.Empty : "s")}"
                : $"{_pickerItems.Count} command{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        _pickerStatus = $"{count}  ·  {item.Detail}";
    }

    private void PreviewSelectedTheme()
    {
        if (_pickerSelected < 0 || _pickerSelected >= _pickerItems.Count)
        {
            return;
        }

        string id = _pickerItems[_pickerSelected].Key;
        if (id.Length > 0)
        {
            SetActiveTheme(id, persist: false, announce: false);
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

        if (_pickerKind == PickerKind.Palette)
        {
            int index = item.Value;
            Action? action =
                index >= 0 && index < _paletteCommands.Count
                    ? _paletteCommands[index].Action
                    : null;
            ClosePicker(cancelled: false);
            action?.Invoke();
            return;
        }

        if (_pickerKind == PickerKind.Themes)
        {
            string themeId = item.Key;
            ClosePicker(cancelled: false);
            SetActiveTheme(themeId, persist: true, announce: true);
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
            bool sameFile =
                _document.Path is not null
                && PathsEqual(Path.GetFullPath(_document.Path), Path.GetFullPath(problemPath));

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
            if (
                _document.Path is not null
                && PathsEqual(Path.GetFullPath(_document.Path), Path.GetFullPath(path))
                && line < _document.LineCount
            )
            {
                int length = _document.CopyLineText(line, ref _lineBuffer, out _);
                return LspPositions.Utf16ColumnToByteOffset(
                    _lineBuffer.AsSpan(0, length),
                    character
                );
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
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
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
    private int FuzzyScore(string text, string query)
    {
        if (query.Length == 0)
        {
            return 0;
        }

        StringComparison comparison = _searchMatchCase
            ? StringComparison.Ordinal
            : StringComparison.OrdinalIgnoreCase;
        int contiguous = text.IndexOf(query, comparison);
        if (contiguous >= 0)
        {
            return contiguous;
        }

        int textIndex = 0;
        int gapScore = 0;
        int lastMatch = -2;

        foreach (char queryChar in query)
        {
            char wanted = _searchMatchCase ? queryChar : char.ToLowerInvariant(queryChar);
            bool found = false;

            while (textIndex < text.Length)
            {
                char candidate = _searchMatchCase
                    ? text[textIndex]
                    : char.ToLowerInvariant(text[textIndex]);
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

        name = name.ToLowerInvariant();

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

            case "fmt":
                if (arguments.Count != 0)
                {
                    SetError("usage: :fmt");
                    return;
                }
                FormatCurrentDocument();
                return;

            case "cw":
                if (arguments.Count != 0)
                {
                    SetError("usage: :cw");
                    return;
                }
                CleanWhitespace();
                return;

            case "cfg":
                if (arguments.Count != 0)
                {
                    SetError("usage: :cfg");
                    return;
                }
                OpenConfigFile();
                return;

            case "rcfg":
                if (arguments.Count != 0)
                {
                    SetError("usage: :rcfg");
                    return;
                }
                ReloadConfiguration();
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

            case "s":
                SetIndentStyle(useTabs: false, arguments);
                return;

            case "t":
                SetIndentStyle(useTabs: true, arguments);
                return;

            case "gi":
                if (arguments.Count != 0)
                {
                    SetError("usage: :gi");
                    return;
                }
                ToggleShowGitIgnored();
                return;

            case "gdl":
                if (arguments.Count != 0)
                {
                    SetError("usage: :gdl");
                    return;
                }
                ToggleDiffLines();
                return;

            case "smc":
                if (arguments.Count != 0)
                {
                    SetError("usage: :smc");
                    return;
                }
                ToggleSearchMatchCase();
                return;

            case "vs":
                if (arguments.Count != 0)
                {
                    SetError("usage: :vs");
                    return;
                }
                SplitPane(PaneSplitOrientation.Vertical);
                return;

            case "hs":
                if (arguments.Count != 0)
                {
                    SetError("usage: :hs");
                    return;
                }
                SplitPane(PaneSplitOrientation.Horizontal);
                return;

            case "cb":
                if (arguments.Count != 0)
                {
                    SetError("usage: :cb");
                    return;
                }
                CloseCurrentBuffer();
                return;

            case "adi":
                if (arguments.Count != 0)
                {
                    SetError("usage: :adi");
                    return;
                }
                AutoDetectIndentation();
                return;

            case "help":
                ShowHelpReminder();
                return;

            case "a":
            case "about":
                if (arguments.Count != 0)
                {
                    SetError("usage: :about");
                    return;
                }
                OpenAbout();
                return;

            case "debug-lsp":
            case "dl":
                if (arguments.Count != 0)
                {
                    SetError($"usage: :{name}");
                    return;
                }
                OpenLspDebug();
                return;

            case "debug":
            case "dbg":
                StartDebugSession(arguments);
                return;

            case "dbg-break":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-break");
                    return;
                }
                ToggleBreakpoint();
                return;

            case "dbg-cont":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-cont");
                    return;
                }
                ContinueDebugSession();
                return;

            case "dbg-next":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-next");
                    return;
                }
                NextDebugSession();
                return;

            case "dbg-step":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-step");
                    return;
                }
                StepInDebugSession();
                return;

            case "dbg-out":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-out");
                    return;
                }
                StepOutDebugSession();
                return;

            case "dbg-pause":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-pause");
                    return;
                }
                PauseDebugSession();
                return;

            case "dbg-stop":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-stop");
                    return;
                }
                StopDebugSession();
                return;

            case "dbg-sv":
            case "dbg-vars":
                if (arguments.Count != 0)
                {
                    SetError($"usage: :{name}");
                    return;
                }
                ToggleDebugVariables();
                return;

            case "dbg-watch":
                if (arguments.Count == 0)
                {
                    SetError("usage: :dbg-watch <expression>");
                    return;
                }
                AddDebugWatch(string.Join(' ', arguments));
                return;

            case "dbg-unwatch":
                if (arguments.Count == 0)
                {
                    SetError("usage: :dbg-unwatch <expression>");
                    return;
                }
                RemoveDebugWatch(string.Join(' ', arguments));
                return;

            case "dbg-clear-watches":
                if (arguments.Count != 0)
                {
                    SetError("usage: :dbg-clear-watches");
                    return;
                }
                ClearDebugWatches();
                return;

            case "debug-dap":
            case "dd":
                if (arguments.Count != 0)
                {
                    SetError($"usage: :{name}");
                    return;
                }
                OpenDapDebug();
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

    private void ToggleShowGitIgnored()
    {
        _showGitIgnored = !_showGitIgnored;
        _completionFiles = null;
        _completionFilesRoot = string.Empty;
        _pickerFilesLoaded = false;
        _pickerFilesRoot = string.Empty;
        _pickerFiles.Clear();

        if (_sidebarVisible)
        {
            RefreshSidebar(SelectedSidebarNode?.Path);
        }

        if (_pickerKind is PickerKind.Files or PickerKind.Search)
        {
            _pickerLoading = true;
            _pickerStatus = "Loading files...";
            StartPickerFileScan();
        }

        PersistDisplaySettings();
        SetMessage(_showGitIgnored ? "Showing git-ignored files." : "Hiding git-ignored files.");
    }

    private void ToggleSearchMatchCase()
    {
        _searchMatchCase = !_searchMatchCase;
        PersistDisplaySettings();

        if (_findActive)
        {
            RebuildFindMatches();
        }

        if (_pickerKind != PickerKind.None)
        {
            BeginPickerQuery();
        }

        _forceFullRedraw = true;
        SetMessage(_searchMatchCase ? "Search is case-sensitive." : "Search is case-insensitive.");
    }

    /// <summary>
    /// Applies an indentation-style command. <c>:s [width]</c> registers the
    /// buffer as space-indented; <c>:t [width]</c> registers it as
    /// tab-indented and sets both the visual tab width and indent width.
    /// </summary>
    private void SetIndentStyle(bool useTabs, List<string> arguments)
    {
        string name = useTabs ? "t" : "s";
        if (arguments.Count > 1)
        {
            SetError($"usage: :{name} [width]");
            return;
        }

        int width = useTabs ? _document.TabWidth : _document.IndentWidth;
        if (
            arguments.Count == 1
            && (!int.TryParse(arguments[0], out width) || width < 1 || width > 16)
        )
        {
            SetError($"invalid indent width '{arguments[0]}' (expected 1-16)");
            return;
        }

        if (useTabs)
        {
            _document.SetIndentTabs(width);
            SetMessage($"Indenting with tabs; tab width {_document.TabWidth}.");
        }
        else
        {
            _document.SetIndentSpaces(width);
            SetMessage($"Indenting with {_document.IndentWidth} spaces.");
        }
    }

    private static bool IsFutureCommand(string name) => name is "sf" or "g" or "setup";

    private void FormatCurrentDocument() => FormatCurrentDocument(saveAfter: false);

    private void FormatCurrentDocument(bool saveAfter)
    {
        if (_formatOperation is not null)
        {
            SetMessage("Formatting is already in progress.");
            return;
        }

        if (_language is null)
        {
            SetError("No language is active for this buffer.");
            return;
        }

        if (_language.Formatter is null)
        {
            SetError($"No formatter configured for {_language.Name}.");
            return;
        }

        StartFormat(_language.Formatter, savePath: null, quitAfter: false, saveAfter);
    }

    private bool ShouldFormatOnSave() => _language?.FormatOnSave ?? _config.FormatOnSave;

    private bool TryGetFormatter(out FormatterConfig formatter)
    {
        formatter = _language?.Formatter!;
        return formatter is not null;
    }

    private void StartFormat(
        FormatterConfig formatter,
        string? savePath,
        bool quitAfter,
        bool saveAfter
    )
    {
        if (_formatOperation is not null)
        {
            SetMessage("Formatting is already in progress.");
            return;
        }

        string? path = savePath ?? _document.Path;
        byte[] input = _document.Text();
        Document document = _document;
        long revision = document.Revision;
        List<FormatSelectionSnapshot> selections = CaptureFormatSelections();
        var cancellation = new CancellationTokenSource();
        _formatCancellation = cancellation;

        Task<FormatResult> task = Task.Factory.StartNew(
            () => FormatterRunner.Run(formatter, input, path, cancellation.Token),
            CancellationToken.None,
            TaskCreationOptions.LongRunning,
            TaskScheduler.Default
        );

        _formatOperation = new FormatOperation
        {
            Task = task,
            Document = document,
            Revision = revision,
            SavePath = savePath,
            SaveAfter = saveAfter,
            QuitAfter = quitAfter,
            Selections = selections,
        };

        string name = Path.GetFileName(formatter.Command);
        if (name.Length == 0)
        {
            name = formatter.Command;
        }
        SetMessage($"Formatting with {name} ...");
    }

    private List<FormatSelectionSnapshot> CaptureFormatSelections()
    {
        List<Selection> selections = CaptureAllSelections();
        var result = new List<FormatSelectionSnapshot>(selections.Count);
        foreach (Selection selection in selections)
        {
            result.Add(
                new FormatSelectionSnapshot(
                    _document.LineOfOffset(selection.Cursor),
                    VisualColumnOfOffset(selection.Cursor),
                    _document.LineOfOffset(selection.Anchor),
                    VisualColumnOfOffset(selection.Anchor),
                    selection.Linewise
                )
            );
        }
        return result;
    }

    private List<Selection> RestoreFormatSelections(
        IReadOnlyList<FormatSelectionSnapshot> snapshots
    )
    {
        var result = new List<Selection>(snapshots.Count);
        long lastLine = _document.LineCount - 1;

        foreach (FormatSelectionSnapshot snapshot in snapshots)
        {
            long cursorLine = Math.Clamp(snapshot.CursorLine, 0, lastLine);
            int cursorLength = _document.CopyLineText(cursorLine, ref _lineBuffer, out _);
            long cursorStart = _document.LineStart(cursorLine);
            long cursor =
                cursorStart
                + UnicodeUtil.ByteOffsetForVisualColumn(
                    _lineBuffer.AsSpan(0, cursorLength),
                    snapshot.CursorColumn,
                    TabWidth
                );

            long anchorLine = Math.Clamp(snapshot.AnchorLine, 0, lastLine);
            int anchorLength = _document.CopyLineText(anchorLine, ref _lineBuffer, out _);
            long anchorStart = _document.LineStart(anchorLine);
            long anchor =
                anchorStart
                + UnicodeUtil.ByteOffsetForVisualColumn(
                    _lineBuffer.AsSpan(0, anchorLength),
                    snapshot.AnchorColumn,
                    TabWidth
                );

            result.Add(
                new Selection
                {
                    Cursor = cursor,
                    Anchor = anchor,
                    Linewise = snapshot.Linewise,
                    PreferredColumn = snapshot.CursorColumn,
                    PreferredColumnValid = true,
                }
            );
        }

        return result;
    }

    private bool PollFormat()
    {
        FormatOperation? operation = _formatOperation;
        if (operation is null || !operation.Task.IsCompleted)
        {
            return false;
        }

        _formatOperation = null;
        _formatCancellation?.Dispose();
        _formatCancellation = null;

        FormatResult result;
        try
        {
            result = operation.Task.GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            result = FormatResult.Failure(ex.Message);
        }

        bool sameDocument = ReferenceEquals(operation.Document, _document);
        bool valid = sameDocument && operation.Revision == _document.Revision;

        if (!result.Ok || !valid)
        {
            if (!result.Ok)
            {
                SetError(result.Error);
            }
            else
            {
                SetError("Buffer changed during formatting; result discarded.");
            }

            if (_resaveRequested)
            {
                bool quitAfter = _quitAfterSave;
                _resaveRequested = false;
                _quitAfterSave = false;
                StartSave(path: null, quitAfter, skipFormat: true);
            }
            else if (operation.SaveAfter)
            {
                _quitAfterSave = false;
            }

            return true;
        }

        ApplyFormattedOutput(operation, result.Output);

        if (operation.SaveAfter)
        {
            bool quitAfter = operation.QuitAfter || _quitAfterSave;
            _resaveRequested = false;
            _quitAfterSave = false;
            StartSave(operation.SavePath, quitAfter, skipFormat: true);
        }
        else if (_resaveRequested)
        {
            bool quitAfter = _quitAfterSave;
            _resaveRequested = false;
            _quitAfterSave = false;
            StartSave(path: null, quitAfter, skipFormat: true);
        }

        return true;
    }

    private void ApplyFormattedOutput(FormatOperation operation, byte[] output)
    {
        List<Selection> selections = RestoreFormatSelections(operation.Selections);

        _document.BeginCompositeEdit();
        _document.Replace(0, _document.Size, output);
        _document.EndCompositeEdit();
        SetSelections(selections);
        _document.BreakUndoGroup();

        _lspDirty = _lsp.IsRunning;
        _lspDirtySince = Environment.TickCount64;

        if (_plugins is not null && _plugins.HasSubscribers("buffer_changed"))
        {
            FirePluginEvent("buffer_changed", CurrentBufferData());
        }

        SetMessage($"Formatted {_document.DisplayName}");
    }

    private void OpenConfigFile()
    {
        if (!HexalConfig.EnsureDefaultFile(out string path, out string error))
        {
            SetError($"config: {error}");
            return;
        }

        RequestOpen(path, force: false);
    }

    private void ReloadConfiguration()
    {
        HexalConfig config = HexalConfig.LoadDefault(out string error);
        if (error.Length > 0)
        {
            SetError($"config: {error}");
            return;
        }

        _config = config;
        ApplyDebugAdapterConfiguration();
        if (config.StartupBenchmarker && _benchmark is null)
        {
            _benchmark = new StartupBenchmark();
            _plugins?.SetBenchmark(_benchmark);
        }
        else if (!config.StartupBenchmarker && _benchmark is not null)
        {
            _plugins?.SetBenchmark(null);
            _benchmark = null;
            _benchmarkVisible = false;
        }

        _showDiffLines = config.ShowDiffLines;
        _showGitIgnored = config.ShowGitIgnored;
        _searchMatchCase = config.SearchMatchCaseEnabled;
        _startupGrammarTask = null;
        _startupGrammarStarted = 0;
        _languages = LanguageRegistry.LoadUserGrammars();

        string activeThemeId = _theme.Id;
        _themes = ThemeRegistry.LoadDefault(out string themeError);
        Theme? activeTheme = _themes.Find(activeThemeId);
        _theme = activeTheme ?? _themes.ActiveTheme;
        _themes.SetActiveTheme(_theme);
        _forceFullRedraw = true;

        RefreshAllPaneDiffLines();
        _completionFiles = null;
        _completionFilesRoot = string.Empty;
        _pickerFilesLoaded = false;
        _pickerFilesRoot = string.Empty;
        if (_sidebarVisible)
        {
            RefreshSidebar(SelectedSidebarNode?.Path);
        }

        ResetLsp();
        RefreshLanguage();
        FirePluginEvent("buffer_opened", CurrentBufferData());

        if (_languages.Errors.Count > 0)
        {
            SetError($"grammar: {_languages.Errors[0]}");
        }
        else if (themeError.Length > 0)
        {
            SetError($"theme: {themeError}");
        }
        else
        {
            SetMessage("Configuration reloaded.");
        }
    }

    private void StartSave(string? path, bool quitAfter, bool skipFormat = false)
    {
        if (_formatOperation is not null)
        {
            if (quitAfter)
            {
                _quitAfterSave = true;
            }
            _resaveRequested = true;
            SetMessage("Formatting in progress; queued another save.");
            return;
        }

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

        if (!skipFormat && ShouldFormatOnSave() && TryGetFormatter(out FormatterConfig formatter))
        {
            StartFormat(formatter, path, quitAfter, saveAfter: true);
            return;
        }

        if (_config.TrimTrailingWhitespaceOnSave)
        {
            TrimTrailingWhitespaceInBuffer();
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
        Task<SaveResult> task = Task.Factory.StartNew(
            () =>
            {
                bool ok = snapshot.WriteTo(out string writeError);
                return new SaveResult
                {
                    Ok = ok,
                    Error = writeError,
                    Path = snapshot.Path,
                    Pose = snapshot.Pose,
                    Revision = snapshot.Revision,
                };
            },
            CancellationToken.None,
            TaskCreationOptions.LongRunning,
            TaskScheduler.Default
        );

        _saveOperation = new SaveOperation
        {
            Task = task,
            Document = document,
            Snapshot = snapshot,
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
        _plugins?.Tick();
        _lsp.Pump();
        bool redraw = PollStartupWork();
        if (PollSave())
        {
            redraw = true;
        }
        if (PollFormat())
        {
            redraw = true;
        }
        if (PollSidebarFormat())
        {
            redraw = true;
        }
        if (PollPicker())
        {
            redraw = true;
        }
        if (PollLsp())
        {
            redraw = true;
        }

        if (PollDap())
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
                Revision = operation.Snapshot.Revision,
            };
        }

        bool sameDocument = ReferenceEquals(operation.Document, _document);

        if (result.Ok)
        {
            if (sameDocument)
            {
                string? previousPath = operation.Document.Path;
                operation.Document.MarkSaved(result.Path, result.Pose, result.Revision);
                bool pathChanged = !string.Equals(
                    previousPath,
                    result.Path,
                    StringComparison.Ordinal
                );
                if (pathChanged)
                {
                    ResetLsp();
                    RefreshLanguage();
                }

                RefreshFileStamp();
                RefreshDiffLines();
                SetMessage($"Wrote {result.Path} ({_document.LineCount} lines)");
                FirePluginEvent(
                    "buffer_saved",
                    new Dictionary<string, object?>
                    {
                        ["path"] = result.Path,
                        ["id"] = CurrentBufferId,
                    }
                );
                NotifyLspSaved();

                // Saving an unnamed buffer assigns a path and therefore a
                // language. Start (or restart) its server just as opening a
                // file would. Also retry on later saves when a server was not
                // available the first time.
                if (pathChanged || (!_lsp.IsRunning && _language is not null))
                {
                    FirePluginEvent("buffer_opened", CurrentBufferData());
                }
            }
            else
            {
                operation.Document.MarkSaved(result.Path, result.Pose, result.Revision);
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
            if (!force && AnyPaneDocumentDirty())
            {
                if (!_config.ConfirmQuitWithoutSaving)
                {
                    SetError("Unsaved changes; use :q! to quit without saving.");
                    return;
                }

                _prompt = PromptKind.Quit;
                return;
            }

            _waitThenQuit = true;
            _waitThenQuitForce = force;
            SetMessage("Will quit when the save finishes.");
            return;
        }

        if (!force && AnyPaneDocumentDirty())
        {
            if (!_config.ConfirmQuitWithoutSaving)
            {
                SetError("Unsaved changes; use :q! to quit without saving.");
                return;
            }

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
        AutoDetectIndentationOnOpen();
        _cursor = 0;
        _topLine = 0;
        _leftColumn = 0;
        _preferredColumnValid = false;
        _selectionLinewise = false;
        _extraSelections.Clear();
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
            int rows = ActivePaneRows();
            _topLine = Math.Max(0, targetLine - Math.Max(0, rows / 2));
        }

        SnapCursor();
        _selectionAnchor = _cursor;
        RefreshFileStamp();
        RefreshDiffLines();
        SetMessage(line >= 0 ? $"Opened {path}:{line + 1}" : $"Opened {path}");

        Dictionary<string, object?> opened = CurrentBufferData();
        opened["line"] = line;
        FirePluginEvent("buffer_opened", opened);
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

    private Selection CapturePrimarySelection() =>
        new()
        {
            Cursor = _cursor,
            Anchor = _selectionAnchor,
            Linewise = _selectionLinewise,
            PreferredColumn = _preferredColumn,
            PreferredColumnValid = _preferredColumnValid,
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
            CapturePrimarySelection(),
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
                PreferredColumnValid = source.PreferredColumnValid,
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
            Array.Sort(
                order,
                (left, right) => selections[right].Cursor.CompareTo(selections[left].Cursor)
            );
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

                        result.Cursor = AdjustOffset(result.Cursor, offset, removed, inserted);
                        result.Anchor = AdjustOffset(result.Anchor, offset, removed, inserted);
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
        return UnicodeUtil.VisualColumn(
            _lineBuffer.AsSpan(0, length),
            (int)(offset - begin),
            TabWidth
        );
    }

    private long CursorLine() => _document.LineOfOffset(_cursor);

    private long CursorVisualColumn() => VisualColumnOfOffset(_cursor);

    /// <summary>
    /// Visual width of a tab in the current buffer. This is a document setting
    /// so <c>:t &lt;width&gt;</c> can change it at runtime.
    /// </summary>
    private int TabWidth => _document.TabWidth;

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
        _cursor =
            start
            + UnicodeUtil.ByteOffsetForVisualColumn(
                _lineBuffer.AsSpan(0, length),
                _preferredColumn,
                TabWidth
            );

        _document.BreakUndoGroup();
        SnapCursor();
    }

    private void MovePage(int direction)
    {
        long before = _cursor;
        int rows = ActivePaneRows();
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

    private void MoveToDocumentStart(bool recordJump) => MoveTo(0, recordJump);

    private void MoveToDocumentEnd(bool recordJump) => MoveTo(_document.Size, recordJump);

    private void MoveToPreviousLocation()
    {
        if (_jumpHistory.Count == 0)
        {
            SetMessage("No previous location.");
            return;
        }

        JumpLocation target = _jumpHistory[^1];
        _jumpHistory.RemoveAt(_jumpHistory.Count - 1);
        _jumpForward.Add(CurrentJumpLocation());
        NavigateToLocation(target);
    }

    private void MoveToNextLocation()
    {
        if (_jumpForward.Count == 0)
        {
            SetMessage("No next location.");
            return;
        }

        JumpLocation target = _jumpForward[^1];
        _jumpForward.RemoveAt(_jumpForward.Count - 1);
        _jumpHistory.Add(CurrentJumpLocation());
        NavigateToLocation(target);
    }

    private JumpLocation CurrentJumpLocation()
    {
        long line = _document.LineOfOffset(_cursor);
        long start = _document.LineStart(line);
        return new JumpLocation(_document.Path, line, _cursor - start);
    }

    private void NavigateToLocation(JumpLocation location)
    {
        if (location.Path is null)
        {
            if (_document.Path is not null)
            {
                SetError("The previous location is no longer open.");
                return;
            }

            MoveToLineColumn(location.Line, location.Column);
            return;
        }

        if (_document.Path is not null && SamePath(_document.Path, location.Path))
        {
            MoveToLineColumn(location.Line, location.Column);
            return;
        }

        // Opening another file reuses the editor's single-buffer machinery.
        // RequestOpen asks before discarding unsaved changes.
        RequestOpen(location.Path, force: false, location.Line, location.Column);
    }

    private void MoveToLineColumn(long line, long column)
    {
        if (_document.LineCount <= 0)
        {
            return;
        }

        line = Math.Clamp(line, 0, _document.LineCount - 1);
        long start = _document.LineStart(line);
        long end = _document.LineEnd(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)Math.Clamp(column, 0, length);

        // Positions can come from an older revision or from another file, so
        // make sure the cursor lands on a UTF-8 scalar boundary.
        while (local < length && (_lineBuffer[local] & 0xC0) == 0x80)
        {
            ++local;
        }

        _cursor = Math.Clamp(start + local, start, end);
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
    }

    private static bool SamePath(string left, string right)
    {
        try
        {
            return PathsEqual(Path.GetFullPath(left), Path.GetFullPath(right));
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return PathsEqual(left, right);
        }
    }

    private void RecordJump(long from)
    {
        if (from == _cursor)
        {
            return;
        }

        long line = _document.LineOfOffset(from);
        long start = _document.LineStart(line);
        var location = new JumpLocation(_document.Path, line, from - start);

        if (_jumpHistory.Count > 0 && _jumpHistory[^1] == location)
        {
            return;
        }

        _jumpHistory.Add(location);
        _jumpForward.Clear();
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

        if (TrySkipAutoClose(rune) || TryInsertAutoClosePair(rune))
        {
            return;
        }

        _lspCompletionPending = false;
        Span<byte> text = stackalloc byte[4];
        int length = UnicodeUtil.Encode(codePoint, text);
        _document.Insert(_cursor, text[..length]);
        _cursor += length;
        if (codePoint == '>')
        {
            TryInsertAutoCloseTag();
        }

        TryAutoUnindentKeyword();
        SnapCursor();
        _preferredColumnValid = false;
        ClearMessage();

        if (
            _extraSelections.Count == 0
            && codePoint < 0x80
            && !char.IsWhiteSpace((char)codePoint)
            && _lsp.IsRunning
            && _lsp.IsInitialized
            && _lsp.IsCompletionTriggerCharacter((char)codePoint)
        )
        {
            TryRequestLspCompletion(((char)codePoint).ToString());
        }
    }

    /// <summary>
    /// Inserts one indent level using the buffer's registered style: a tab for
    /// tab-indented buffers, or <see cref="Document.IndentWidth"/> spaces.
    /// </summary>
    private void InsertIndent()
    {
        byte[] unit = _document.IndentUnit();
        _document.Insert(_cursor, unit);
        _cursor += unit.Length;
        SnapCursor();
        _preferredColumnValid = false;
        ClearMessage();
    }

    private void InsertNewline()
    {
        InsertNewlineWithAutoIndent();
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

        if (_cursor > begin && BackspaceIndentation(begin))
        {
            return;
        }

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
        long begin = _document.LineStart(line);
        long end = _document.LineEnd(line);
        if (_cursor >= end)
        {
            if (end == begin)
            {
                DeleteEmptyLine(line);
            }
            return;
        }

        _document.BreakUndoGroup();
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = (int)(_cursor - begin);
        int next = UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
        _document.Erase(_cursor, next - local);
        _preferredColumnValid = false;
        SnapCursor();
    }

    private void DeleteEmptyLine(long line)
    {
        long start = _document.LineStart(line);
        _document.BreakUndoGroup();

        if (line + 1 < _document.LineCount)
        {
            long nextStart = _document.LineStart(line + 1);
            _document.Erase(start, nextStart - start);
            _cursor = start;
        }
        else if (line > 0)
        {
            long previousEnd = _document.LineEnd(line - 1);
            _document.Erase(previousEnd, start - previousEnd);
            _cursor = previousEnd;
        }
        else
        {
            return;
        }

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

    /// <summary>
    /// Selects the whole buffer, entering SELECT mode if necessary. The cursor
    /// moves to the end of the document so extending or shrinking the whole
    /// buffer selection behaves like any other character-wise selection.
    /// </summary>
    private void SelectAll()
    {
        _mode = Mode.Select;
        _selectionLinewise = false;
        _selectionAnchor = 0;
        _cursor = _document.Size;
        _extraSelections.Clear();
        _preferredColumnValid = false;
        _pendingG = false;
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
    /// Copies the active SELECT-mode range to the system clipboard and leaves
    /// SELECT mode, placing the cursor at the start of the yanked range.
    /// </summary>
    private void YankSelection()
    {
        if (_mode != Mode.Select)
        {
            SetMessage("Select text before yanking.");
            return;
        }

        (long start, long end) = SelectionRange();
        if (end <= start)
        {
            SetMessage("Nothing selected.");
            return;
        }

        byte[] text = _document.Range(start, end - start);
        if (!Clipboard.TryCopy(_terminal, text, out string error))
        {
            SetError(error);
            return;
        }

        _cursor = start;
        _selectionAnchor = start;
        _selectionLinewise = false;
        _mode = Mode.Normal;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
        SetMessage("Yanked selection to the system clipboard.");
    }

    /// <summary>
    /// Copies the active SELECT-mode range to the system clipboard, then
    /// deletes it. The text is only removed after the copy succeeds so a
    /// failed clipboard write cannot lose the selection.
    /// </summary>
    private void CutSelection()
    {
        if (_mode != Mode.Select)
        {
            SetMessage("Select text before cutting.");
            return;
        }

        (long start, long end) = SelectionRange();
        if (end <= start)
        {
            SetMessage("Nothing selected.");
            return;
        }

        byte[] text = _document.Range(start, end - start);
        if (!Clipboard.TryCopy(_terminal, text, out string error))
        {
            SetError(error);
            return;
        }

        _document.BreakUndoGroup();
        _document.Erase(start, end - start);
        _cursor = Math.Clamp(start, 0, _document.Size);
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _mode = Mode.Normal;
        _preferredColumnValid = false;
        SnapCursor();
        SetMessage("Cut selection to the system clipboard.");
    }

    /// <summary>
    /// Inserts the system clipboard at each cursor. When a SELECT-mode range
    /// is active the pasted text replaces it, matching normal editor paste
    /// behaviour, and the cursor ends after the inserted text. With
    /// <paramref name="newLine"/> the paste is placed on one or more new lines
    /// below the cursor's line instead of inline.
    /// </summary>
    private void PasteClipboard(bool newLine)
    {
        if (!Clipboard.TryPaste(out byte[] clipboardText, out string error))
        {
            SetError(error);
            return;
        }

        if (clipboardText.Length == 0)
        {
            SetMessage("The system clipboard is empty.");
            return;
        }

        byte[] text = NormalizePastedText(clipboardText);
        if (newLine)
        {
            ForEachSelection(() => PasteTextOnNewLine(text), editing: true);
        }
        else
        {
            ForEachSelection(() => PasteText(text), editing: true);
        }
    }

    private void PasteText(byte[] text)
    {
        long start;
        if (_mode == Mode.Select)
        {
            (start, long end) = SelectionRange();
            _document.Replace(start, end - start, text);
        }
        else
        {
            start = _cursor;
            _document.Insert(start, text);
        }

        _cursor = start + text.Length;
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _mode = Mode.Normal;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
        ClearMessage();
    }

    /// <summary>
    /// Inserts the clipboard on one or more new lines below the cursor's line,
    /// leaving the current line intact. When the cursor is already on an empty
    /// line the text fills that line instead of creating a blank line above it.
    /// </summary>
    private void PasteTextOnNewLine(byte[] text)
    {
        long line = _document.LineOfOffset(_cursor);
        long start = _document.LineStart(line);
        long end = _document.LineEnd(line);
        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        ReadOnlySpan<byte> content = text;

        long insertAt;
        if (end > start)
        {
            if (line + 1 < _document.LineCount)
            {
                insertAt = _document.LineStart(line + 1);
            }
            else
            {
                insertAt = end;
            }
        }
        else
        {
            insertAt = start;
            if (insertAt < _document.Size && text[^1] == (byte)'\n')
            {
                // The empty line's own terminator already ends the pasted
                // text, so drop one from the clipboard to avoid a blank line.
                content = text.AsSpan(0, text.Length - eol.Length);
            }
        }

        bool lastLine = line + 1 >= _document.LineCount;
        bool currentLineHasText = end > start;
        var payload = new ByteBuffer(text.Length + eol.Length * 2);

        if (currentLineHasText && lastLine)
        {
            payload.Append(eol);
            payload.Append(content);
        }
        else
        {
            payload.Append(content);

            if (
                currentLineHasText
                && insertAt < _document.Size
                && (content.Length == 0 || content[^1] != (byte)'\n')
            )
            {
                payload.Append(eol);
            }
        }

        _document.Insert(insertAt, payload.ToArray());
        _cursor = insertAt + (currentLineHasText && lastLine ? eol.Length : 0);
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _mode = Mode.Normal;
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
        ClearMessage();
    }

    /// <summary>
    /// Converts clipboard line endings to the buffer's EOL convention so a
    /// paste from Windows does not introduce mixed line endings into an LF
    /// file (or vice versa).
    /// </summary>
    private byte[] NormalizePastedText(byte[] text)
    {
        byte[] eol = _document.Eol == "\r\n" ? CrLf : Lf;
        var normalized = new ByteBuffer(text.Length + eol.Length);

        for (int i = 0; i < text.Length; ++i)
        {
            byte value = text[i];
            if (value == (byte)'\r')
            {
                if (i + 1 < text.Length && text[i + 1] == (byte)'\n')
                {
                    ++i;
                }
                normalized.Append(eol);
            }
            else if (value == (byte)'\n')
            {
                normalized.Append(eol);
            }
            else
            {
                normalized.Append(value);
            }
        }

        return normalized.ToArray();
    }

    /// <summary>
    /// Returns the selected byte range as a half-open interval. Character
    /// selections include the scalar under the cursor; line selections include
    /// the final line's terminator when one exists.
    /// </summary>
    private (long Start, long End) SelectionRange() =>
        SelectionRange(_selectionAnchor, _cursor, _selectionLinewise);

    private (long Start, long End) SelectionRange(long anchor, long cursor, bool linewise)
    {
        if (linewise)
        {
            long anchorLine = _document.LineOfOffset(anchor);
            long cursorLine = _document.LineOfOffset(cursor);
            long firstLine = Math.Min(anchorLine, cursorLine);
            long lastLine = Math.Max(anchorLine, cursorLine);
            long start = _document.LineStart(firstLine);
            long end =
                lastLine + 1 < _document.LineCount
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
                rangeEnd =
                    lineStart + UnicodeUtil.NextBoundary(_lineBuffer.AsSpan(0, length), local);
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

    private void KeepPrimarySelection()
    {
        _extraSelections.Clear();
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

        long targetCursor =
            targetStart
            + UnicodeUtil.ByteOffsetForVisualColumn(
                _lineBuffer.AsSpan(0, targetLength),
                cursorColumn,
                TabWidth
            );
        long targetAnchor = targetCursor;

        if (
            _mode == Mode.Select
            && source.Anchor != source.Cursor
            && _document.LineOfOffset(source.Anchor) == sourceLine
        )
        {
            long anchorColumn = VisualColumnOfOffset(source.Anchor);
            targetAnchor =
                targetStart
                + UnicodeUtil.ByteOffsetForVisualColumn(
                    _lineBuffer.AsSpan(0, targetLength),
                    anchorColumn,
                    TabWidth
                );
        }

        created = new Selection
        {
            Cursor = targetCursor,
            Anchor = targetAnchor,
            Linewise = source.Linewise,
            PreferredColumn = cursorColumn,
            PreferredColumnValid = true,
        };
        return true;
    }

    /// <summary>
    /// Indents or unindents every line touched by the active selection, or the
    /// current line when nothing is selected. One indent level is a tab or the
    /// buffer's configured number of spaces. All affected lines form a single
    /// undo step and the selection is preserved.
    /// </summary>
    private void IndentSelection(bool unindent)
    {
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

        var changes = new List<(long Offset, long Remove, byte[] Insert)>();
        byte[] unit = _document.IndentUnit();

        foreach (long line in lines)
        {
            long lineStart = _document.LineStart(line);
            if (!unindent)
            {
                changes.Add((lineStart, 0, unit));
                continue;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int remove = _document.UnindentLength(_lineBuffer.AsSpan(0, length));
            if (remove > 0)
            {
                changes.Add((lineStart, remove, Array.Empty<byte>()));
            }
        }

        if (changes.Count == 0)
        {
            SetMessage("Already at the left margin.");
            return;
        }

        _document.BreakUndoGroup();
        _document.BeginCompositeEdit();
        try
        {
            ApplyCommentChanges(changes);
        }
        finally
        {
            _document.EndCompositeEdit();
        }

        ClearMessage();
    }

    private void ToggleComment()
    {
        string? lineComment = _language?.LineComment;
        string blockStart = _language?.BlockCommentStart ?? string.Empty;
        string blockEnd = _language?.BlockCommentEnd ?? string.Empty;

        if (string.IsNullOrEmpty(lineComment) && (blockStart.Length == 0 || blockEnd.Length == 0))
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

    private void AddTouchedLines(SortedSet<long> lines, Selection selection, bool selecting)
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
                _document.LineOfOffset(selection.Cursor)
            );
            lastLine = Math.Max(
                _document.LineOfOffset(selection.Anchor),
                _document.LineOfOffset(selection.Cursor)
            );
        }
        else
        {
            firstLine = _document.LineOfOffset(start);
            lastLine = end > start ? _document.LineOfOffset(end - 1) : firstLine;
        }

        for (long line = firstLine; line <= lastLine; ++line)
        {
            lines.Add(line);
        }
    }

    private static int IndentLength(ReadOnlySpan<byte> text)
    {
        int offset = 0;
        while (offset < text.Length && (text[offset] == (byte)' ' || text[offset] == (byte)'\t'))
        {
            ++offset;
        }
        return offset;
    }

    private static int TrimTrailingWhitespace(ReadOnlySpan<byte> text, int minimum)
    {
        int end = text.Length;
        while (end > minimum && (text[end - 1] == (byte)' ' || text[end - 1] == (byte)'\t'))
        {
            --end;
        }
        return end;
    }

    private static bool StartsWith(ReadOnlySpan<byte> text, ReadOnlySpan<byte> prefix) =>
        prefix.Length > 0
        && text.Length >= prefix.Length
        && text[..prefix.Length].SequenceEqual(prefix);

    private static bool EndsWith(ReadOnlySpan<byte> text, ReadOnlySpan<byte> suffix) =>
        suffix.Length > 0
        && text.Length >= suffix.Length
        && text[^suffix.Length..].SequenceEqual(suffix);

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

    private void ToggleBlockComments(SortedSet<long> lines, string start, string end)
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
            if (
                contentEnd <= indent
                || !StartsWith(text[indent..contentEnd], startBytes)
                || !EndsWith(text[indent..contentEnd], endBytes)
            )
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
                if (
                    endOffset > lineStart + indent + removeStart
                    && text[contentEnd - endBytes.Length - 1] == (byte)' '
                )
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
                changes.Add((lineStart + indent, 0, Encoding.UTF8.GetBytes(start + " ")));
                changes.Add((lineStart + contentEnd, 0, Encoding.UTF8.GetBytes(" " + end)));
            }
        }

        ApplyCommentChanges(changes);
    }

    private void ApplyCommentChanges(List<(long Offset, long Remove, byte[] Insert)> changes)
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
                selection.Cursor = AdjustOffset(selection.Cursor, offset, remove, insert.Length);
                selection.Anchor = AdjustOffset(selection.Anchor, offset, remove, insert.Length);
            }

            _document.Replace(offset, remove, insert);
        }

        SetSelections(selections);
        _document.BreakUndoGroup();
    }

    private static long AdjustOffset(long position, long offset, long removed, long inserted)
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

    private int FindDialogHeight() => _findReplace ? 6 : 4;

    private int FindDialogWidth() => Math.Min(_columns - 2, 72);

    private bool FindDialogFits()
    {
        int available = (int)Math.Max(0, StatusRow());
        return available >= 5
            && _columns >= 20
            && FindDialogWidth() >= 20
            && available >= FindDialogHeight() + 1;
    }

    private int VisiblePaneRows(int paneY, int paneHeight, bool activePane)
    {
        int rows = Math.Max(0, paneHeight);
        if (!activePane || !_findActive || !FindDialogFits())
        {
            return rows;
        }

        int dialogTop = Math.Max(0, (int)StatusRow() - FindDialogHeight());
        return Math.Clamp(dialogTop - paneY, 0, rows);
    }

    private void EnsureCursorVisible()
    {
        PaneBounds bounds = ActivePaneBounds();
        long line = CursorLine();
        int rows = VisiblePaneRows(bounds.Y, bounds.Height, activePane: true);

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

        int gutter = GutterWidthFor(bounds.Width);
        int textWidth = bounds.Width > gutter ? bounds.Width - gutter : 1;
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

    private int GutterWidth() => GutterWidthFor(_columns - _editorLeft);

    private int GutterWidthFor(int width)
    {
        int digits = DigitCount(_document.LineCount);
        int marker = EditorMarkerWidth();
        int gutter = digits + 1 + marker;
        if (width < gutter + 10)
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

        if (_findActive)
        {
            return "FIND";
        }

        if (_sidebarFocused && _sidebarVisible && _mode == Mode.Normal)
        {
            return "SIDEBAR";
        }

        return _mode switch
        {
            Mode.Insert => "INSERT",
            Mode.Select => "SELECT",
            Mode.Command => "COMMAND",
            Mode.PluginPrompt => "PROMPT",
            _ => "NORMAL",
        };
    }

    private string StatusLeft(long line, long column)
    {
        _ = line;
        _ = column;
        string text = $"{ModeLabel()}    {_document.DisplayName}";
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
        if (_paneViews.Count > 1)
        {
            text += $"  [pane {ActivePaneOrdinal()}/{_paneViews.Count}]";
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

        string debugStatus = DebugStatusText();
        if (debugStatus.Length > 0)
        {
            text += "  " + debugStatus;
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

        int markerWidth = EditorMarkerWidth();
        int digits = gutter - 1 - markerWidth;
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

            Style? highlight = LineHighlightStyle(line);
            if (highlight is { } rowStyle)
            {
                _screen.Fill(
                    _editorLeft,
                    row,
                    Math.Max(0, _columns - _editorLeft),
                    1,
                    ' ',
                    rowStyle
                );
            }

            if (markerWidth > 0 && TryGetDiffMarker(line, out char marker, out Style markerStyle))
            {
                if (highlight is { } markerHighlight)
                {
                    markerStyle.Bg = markerHighlight.Bg;
                }
                _screen.Put(_editorLeft, row, marker, markerStyle);
            }

            Style style = line == current ? _theme.CurrentLineNumberStyle : _theme.MutedStyle;
            if (highlight is { } highlightStyle)
            {
                style.Bg = highlightStyle.Bg;
            }

            _screen.PutInt(_editorLeft + markerWidth + x, row, number, style, digits);
        }
    }

    private void RenderTextLines(long firstLine, int rows, int gutter)
    {
        if (rows == 0 || _columns <= _editorLeft + gutter)
        {
            return;
        }

        long total = _document.LineCount;
        int textWidth = _columns - _editorLeft - gutter;

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
            Style? lineHighlight = LineHighlightStyle(line);
            if (lineHighlight is { } fillStyle)
            {
                _screen.Fill(_editorLeft + gutter, row, textWidth, 1, ' ', fillStyle);
            }

            int offset = 0;
            long visual = 0;

            while (offset < length)
            {
                int runeStart = offset;
                while (tokenIndex < _lineTokens.Count && _lineTokens[tokenIndex].End <= runeStart)
                {
                    ++tokenIndex;
                }

                SyntaxTokenKind tokenKind = SyntaxTokenKind.Default;
                if (tokenIndex < _lineTokens.Count && _lineTokens[tokenIndex].Start <= runeStart)
                {
                    tokenKind = _lineTokens[tokenIndex].Kind;
                }
                Style textStyle = TokenStyle(tokenKind);
                if (lineHighlight is { } highlightStyle)
                {
                    textStyle.Bg = highlightStyle.Bg;
                }

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
                        _screen.Put(_editorLeft + gutter + (int)i, row, ' ', textStyle);
                    }
                    visual = nextVisual;
                    continue;
                }

                long screenX = _editorLeft + gutter + (visual - _leftColumn);
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
        if (_mode != Mode.Select || rows <= 0 || _columns <= _editorLeft + gutter)
        {
            return;
        }

        foreach (Selection selection in CaptureAllSelections())
        {
            (long selectionStart, long selectionEnd) = SelectionRange(
                selection.Anchor,
                selection.Cursor,
                selection.Linewise
            );
            if (selectionEnd > selectionStart)
            {
                RenderSelectionRange(
                    selectionStart,
                    selectionEnd,
                    selection.Linewise,
                    firstLine,
                    rows,
                    gutter
                );
            }
        }
    }

    private void RenderSelectionRange(
        long selectionStart,
        long selectionEnd,
        bool linewise,
        long firstLine,
        int rows,
        int gutter
    )
    {
        var style = _theme.SelectionStyle;
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
            long nextLineStart = line + 1 < total ? _document.LineStart(line + 1) : _document.Size;

            if (linewise)
            {
                if (selectionStart < nextLineStart && selectionEnd > lineStart)
                {
                    for (int x = _editorLeft + gutter; x < _columns; ++x)
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
                if (selectionStart == lineStart && selectionEnd > lineStart && lineEnd == lineStart)
                {
                    if (_editorLeft + gutter < _columns)
                    {
                        _screen.ApplyStyle(_editorLeft + gutter, row, style);
                    }
                }
                continue;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            int localFrom = (int)Math.Clamp(from - lineStart, 0, length);
            int localTo = (int)Math.Clamp(to - lineStart, 0, length);

            long visualFrom = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length),
                localFrom,
                TabWidth
            );
            long visualTo = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length),
                localTo,
                TabWidth
            );

            long firstX = Math.Max(
                _editorLeft + gutter,
                _editorLeft + gutter + visualFrom - _leftColumn
            );
            long lastX = Math.Min(_columns, _editorLeft + gutter + visualTo - _leftColumn);

            for (long x = firstX; x < lastX; ++x)
            {
                _screen.ApplyStyle((int)x, row, style);
            }
        }
    }

    private void RenderFindMatches(long firstLine, int rows, int gutter)
    {
        if (
            !_findActive
            || _findMatches.Count == 0
            || rows <= 0
            || _columns <= _editorLeft + gutter
        )
        {
            return;
        }

        var matchStyle = _theme.MatchStyle;
        var currentStyle = _theme.CurrentMatchStyle;

        for (int i = 0; i < _findMatches.Count; ++i)
        {
            FindMatch match = _findMatches[i];
            if (match.Line < firstLine || match.Line >= firstLine + rows)
            {
                continue;
            }

            long lineStart = _document.LineStart(match.Line);
            int length = _document.CopyLineText(match.Line, ref _lineBuffer, out _);
            int localFrom = (int)Math.Clamp(match.Start - lineStart, 0, length);
            int localTo = (int)Math.Clamp(match.End - lineStart, 0, length);
            if (localTo <= localFrom)
            {
                continue;
            }

            long visualFrom = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length),
                localFrom,
                TabWidth
            );
            long visualTo = UnicodeUtil.VisualColumn(
                _lineBuffer.AsSpan(0, length),
                localTo,
                TabWidth
            );

            int row = (int)(match.Line - firstLine);
            long firstX = Math.Max(
                _editorLeft + gutter,
                _editorLeft + gutter + visualFrom - _leftColumn
            );
            long lastX = Math.Min(_columns, _editorLeft + gutter + visualTo - _leftColumn);

            Style style = i == _findMatchIndex ? currentStyle : matchStyle;
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

        long x = _editorLeft + gutter + (column - _leftColumn);
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

        Style style;
        if (_prompt != PromptKind.None)
        {
            style = _theme.PromptStatusStyle;
        }
        else if (_mode == Mode.Insert)
        {
            style = _theme.InsertStatusStyle;
        }
        else if (_mode == Mode.Select)
        {
            style = _theme.SelectStatusStyle;
        }
        else if (_mode == Mode.PluginPrompt)
        {
            style = _theme.CommandStatusStyle;
        }
        else if (_mode == Mode.Command)
        {
            style = _theme.CommandStatusStyle;
        }
        else
        {
            style = _theme.StatusStyle;
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

        string diagnostic = string.Empty;
        int diagnosticWidth = 0;
        int diagnosticX = rightX;
        int diagnosticCount = _document.Path is null
            ? 0
            : _lsp.CountDiagnosticsForPath(_document.Path);
        if (diagnosticCount > 0)
        {
            diagnostic = $"\u25CF {diagnosticCount}";
            diagnosticWidth = (int)UnicodeUtil.ColumnWidth(diagnostic);
            diagnosticX = rightX - diagnosticWidth - 1;
            if (diagnosticX < 1)
            {
                diagnostic = string.Empty;
                diagnosticWidth = 0;
                diagnosticX = rightX;
            }
        }

        int leftMax = (diagnosticWidth > 0 ? diagnosticX : rightX) - 1;
        if (leftMax > 0)
        {
            _screen.PutString(1, (int)y, left, style, leftMax);
        }

        if (diagnosticWidth > 0)
        {
            var diagnosticStyle = style;
            diagnosticStyle.Fg = _theme.Error;
            diagnosticStyle.Bold = true;
            _screen.PutString(diagnosticX, (int)y, diagnostic, diagnosticStyle, diagnosticWidth);
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

        var baseStyle = _theme.BaseStyle;
        _screen.Fill(0, (int)y, _columns, 1, ' ', baseStyle);

        if (_mode == Mode.PluginPrompt)
        {
            RenderPluginPrompt((int)y, baseStyle);
            return;
        }

        if (_mode == Mode.Command)
        {
            var promptStyle = _theme.AccentStyle;
            _screen.Put(BottomLinePadding, (int)y, ':', promptStyle);

            string line = _commandLine.ToString();
            _screen.PutString(
                BottomLinePadding + 1,
                (int)y,
                line,
                baseStyle,
                _columns - BottomLinePadding - 1
            );

            long visual = UnicodeUtil.VisualColumn(line, _commandCursor, 1);
            long x = BottomLinePadding + 1 + visual;
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
            string text = _prompt switch
            {
                PromptKind.Quit => "Unsaved changes. Quit without saving? (y/N)",
                PromptKind.Open => $"Unsaved changes. Discard and open '{_pendingOpen}'? (y/N)",
                PromptKind.SidebarDelete => _pendingSidebarDeletes.Count == 1
                    ? $"Delete '{Path.GetFileName(_pendingSidebarDeletes[0].Path)}'? (y/N)"
                    : $"Delete {_pendingSidebarDeletes.Count} selected items? (y/N)",
                PromptKind.SidebarDiff => "Unsaved changes. Discard and show diff? (y/N)",
                _ => string.Empty,
            };

            var style = _theme.WarningStyle;
            _screen.PutString(BottomLinePadding, (int)y, text, style, _columns - BottomLinePadding);
            return;
        }

        if (_message.Length > 0)
        {
            var style = _messageIsError ? _theme.ErrorStyle : _theme.TextStyle;
            _screen.PutString(
                BottomLinePadding,
                (int)y,
                _message,
                style,
                _columns - BottomLinePadding
            );
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

        var fillStyle = _theme.BaseStyle;
        var borderStyle = _theme.BorderStyle;
        var titleStyle = _theme.AccentStyle;

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

    private void RenderCompletion()
    {
        if (!_completionVisible)
        {
            return;
        }

        int available = (int)Math.Max(0, CommandRow());
        if (available < 3 || _columns < 10)
        {
            return;
        }

        int rows =
            _completionItems.Count == 0 ? 1 : Math.Min(_completionItems.Count, CompletionMaxRows());
        if (rows < 1)
        {
            return;
        }

        if (_completionSelected < _completionTop)
        {
            _completionTop = _completionSelected;
        }
        else if (_completionSelected >= _completionTop + rows)
        {
            _completionTop = _completionSelected - rows + 1;
        }

        int maxTop = Math.Max(0, _completionItems.Count - rows);
        _completionTop = Math.Clamp(_completionTop, 0, maxTop);

        string title = _completionKind switch
        {
            CompletionKind.Files =>
                $"Files {(_completionItems.Count == 0 ? 0 : _completionSelected + 1)}/{_completionItems.Count}",
            CompletionKind.Arguments =>
                $"Values {(_completionItems.Count == 0 ? 0 : _completionSelected + 1)}/{_completionItems.Count}",
            _ =>
                $"Commands {(_completionItems.Count == 0 ? 0 : _completionSelected + 1)}/{_completionItems.Count}",
        };

        int desiredWidth = Math.Max(title.Length + 6, _completionStatus.Length + 6);
        for (int row = 0; row < rows; ++row)
        {
            int index = _completionTop + row;
            if (index >= _completionItems.Count)
            {
                break;
            }
            desiredWidth = Math.Max(desiredWidth, _completionItems[index].Length + 4);
        }

        string description = string.Empty;
        string descriptionTitle = "Description";
        if (_completionKind == CompletionKind.Commands && _completionItems.Count > 0)
        {
            int selectedIndex = Math.Clamp(_completionSelected, 0, _completionItems.Count - 1);
            string commandName = _completionItems[selectedIndex];
            description = CommandDescription(commandName);
            if (description.Length == 0)
            {
                description = "No description available.";
            }
            descriptionTitle = $" :{commandName} ";
        }

        int x = BottomLinePadding;
        int width = Math.Min(_columns - 2, Math.Max(16, desiredWidth));
        if (description.Length > 0 && _columns - 2 >= 16 + 1 + 20)
        {
            int maxWidthWithDescription = _columns - 2 - 21;
            width = Math.Min(width, maxWidthWithDescription);
        }
        if (width < 10)
        {
            return;
        }
        if (x + width >= _columns)
        {
            x = Math.Max(0, _columns - width - 1);
        }

        int height = rows + 2;
        int y = Math.Max(0, available - height);
        DrawBox(x, y, width, height, title);

        if (_completionItems.Count == 0)
        {
            var emptyStyle = _theme.WarningStyle;
            _screen.PutString(x + 2, y + 1, _completionStatus, emptyStyle, width - 4);
            return;
        }

        var selectedStyle = _theme.SelectedItemStyle;
        var itemStyle = _theme.TextStyle;

        for (int row = 0; row < rows; ++row)
        {
            int index = _completionTop + row;
            if (index >= _completionItems.Count)
            {
                break;
            }

            int screenRow = y + 1 + row;
            Style style = index == _completionSelected ? selectedStyle : itemStyle;
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(x + 2, screenRow, _completionItems[index], style, width - 4);
        }

        if (description.Length > 0)
        {
            int descriptionX = x + width + 1;
            int descriptionWidth = Math.Min(56, _columns - descriptionX - 1);
            if (descriptionWidth >= 16)
            {
                List<string> descriptionLines = WrapText(description, descriptionWidth - 4);
                int maxDescriptionRows = Math.Max(1, available - 2);
                if (descriptionLines.Count > maxDescriptionRows)
                {
                    descriptionLines.RemoveRange(
                        maxDescriptionRows,
                        descriptionLines.Count - maxDescriptionRows
                    );
                }

                int descriptionHeight = descriptionLines.Count + 2;
                int descriptionY = Math.Max(0, available - descriptionHeight);
                DrawBox(
                    descriptionX,
                    descriptionY,
                    descriptionWidth,
                    descriptionHeight,
                    descriptionTitle
                );

                var descriptionStyle = _theme.TextStyle;
                for (int row = 0; row < descriptionLines.Count; ++row)
                {
                    _screen.PutString(
                        descriptionX + 2,
                        descriptionY + 1 + row,
                        descriptionLines[row],
                        descriptionStyle,
                        descriptionWidth - 4
                    );
                }
            }
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

        var keyStyle = _theme.AccentStyle;
        var labelStyle = _theme.TextStyle;

        for (int i = 0; i < SpaceMenuItems.Length; ++i)
        {
            (char key, string label) = SpaceMenuItems[i];
            int row = y + 1 + i;
            _screen.PutString(x + 2, row, key.ToString(), keyStyle, width - 3);
            _screen.PutString(x + 4, row, label, labelStyle, width - 5);
        }
    }

    private void RenderFind()
    {
        if (!_findActive)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        if (!FindDialogFits())
        {
            return;
        }

        int height = FindDialogHeight();
        int width = FindDialogWidth();
        if (width < 20 || available < height + 1)
        {
            return;
        }

        int x = (_columns - width) / 2;
        int y = Math.Max(0, available - height);
        string title;
        if (_findReplace)
        {
            title = "Regex replace";
        }
        else if (_findRegex)
        {
            title = "Regex find";
        }
        else if (_findMultiCursor)
        {
            title =
                _findScope.Count > 0 ? "Find in selection (multi-cursor)" : "Find (multi-cursor)";
        }
        else
        {
            title = "Find";
        }
        DrawBox(x, y, width, height, title);

        var promptStyle = _theme.AccentStyle;
        var mutedStyle = _theme.MutedStyle;
        var queryStyle = _theme.TextStyle;
        var statusStyle = _theme.AccentStyle;

        string query = _findQuery.ToString();
        string replacement = _findReplacement.ToString();

        if (!_findReplace)
        {
            _screen.PutString(x + 2, y + 1, ">", promptStyle, width - 4);
            int queryWidth = Math.Max(0, width - 5);
            _screen.PutString(x + 4, y + 1, query, queryStyle, queryWidth);
            if (_findEditing)
            {
                RenderFindCursor(x + 4, y + 1, query, _findQueryCursor, queryWidth);
            }
        }
        else
        {
            const int labelWidth = 9;
            int fieldX = x + 2 + labelWidth;
            int fieldWidth = Math.Max(0, width - labelWidth - 4);

            Style findLabelStyle = _findEditingReplacement ? mutedStyle : promptStyle;
            Style replaceLabelStyle = _findEditingReplacement ? promptStyle : mutedStyle;

            _screen.PutString(x + 2, y + 1, "Find", findLabelStyle, labelWidth);
            _screen.PutString(fieldX, y + 1, query, queryStyle, fieldWidth);
            _screen.PutString(x + 2, y + 2, "Replace", replaceLabelStyle, labelWidth);
            _screen.PutString(fieldX, y + 2, replacement, queryStyle, fieldWidth);

            if (_findEditing)
            {
                if (_findEditingReplacement)
                {
                    RenderFindCursor(
                        fieldX,
                        y + 2,
                        replacement,
                        _findReplacementCursor,
                        fieldWidth
                    );
                }
                else
                {
                    RenderFindCursor(fieldX, y + 1, query, _findQueryCursor, fieldWidth);
                }
            }
        }

        string status;
        if (_findError.Length > 0)
        {
            status = _findError;
            statusStyle = _theme.ErrorStyle;
        }
        else if (query.Length == 0)
        {
            status = _findRegex ? "Type a regular expression" : "Type to search";
        }
        else if (_findMatches.Count == 0)
        {
            status = "No matches";
        }
        else
        {
            int position = Math.Max(0, _findMatchIndex) + 1;
            if (_findReplace)
            {
                status = $"{position}/{_findMatches.Count} matches";
            }
            else
            {
                string enterHint = _findMultiCursor ? "Enter: select all" : "Enter: navigate";
                status = _findEditing
                    ? $"{position}/{_findMatches.Count} matches  {enterHint}  Esc: cancel"
                    : $"{position}/{_findMatches.Count} matches  n: next  p: previous  Esc: exit";
            }
        }

        if (_findReplace)
        {
            _screen.PutString(x + 2, y + 3, status, statusStyle, width - 4);
            string hint =
                _findError.Length > 0
                    ? "Esc: close"
                    : "Tab: switch  Enter: replace  Ctrl+A: replace all  Esc: close";
            _screen.PutString(x + 2, y + 4, hint, mutedStyle, width - 4);
        }
        else
        {
            _screen.PutString(x + 2, y + 2, status, statusStyle, width - 4);
        }
    }

    private void RenderFindCursor(int x, int y, string text, int cursor, int width)
    {
        if (width <= 0)
        {
            return;
        }

        long column = UnicodeUtil.VisualColumn(text, cursor, 1);
        if (column >= width)
        {
            return;
        }

        int cursorX = x + (int)column;
        if (cursorX >= x + width || cursorX >= _columns)
        {
            return;
        }

        Style cursorStyle = _screen.At(cursorX, y).Style;
        cursorStyle.Reverse = true;
        _screen.ApplyStyle(cursorX, y, cursorStyle);
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
            PickerKind.Files => _pickerTitle.Length > 0 ? _pickerTitle : "Open file",
            PickerKind.Plugin => _pluginSelectTitle.Length > 0 ? _pluginSelectTitle : "Select",
            PickerKind.CodeActions => "Code actions",
            PickerKind.Problems => _problemsWorkspace
                ? "Workspace diagnostics"
                : "Document diagnostics",
            PickerKind.Palette => "Command palette",
            PickerKind.Themes => "Select theme",
            _ => "Search across files",
        };

        DrawBox(x, y, width, height, title);

        var promptStyle = _theme.AccentStyle;
        var queryStyle = _theme.TextStyle;
        var selectedStyle = _theme.SelectedItemStyle;

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
            Style style = index == _pickerSelected ? selectedStyle : _theme.TextStyle;
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(x + 2, screenRow, _pickerItems[index].Display, style, width - 4);
        }

        string status = _pickerLoading
            ? (_pickerKind == PickerKind.Files ? "Loading files..." : "Searching...")
            : _pickerStatus;
        if (status.Length == 0)
        {
            status =
                _pickerItems.Count == 0
                    ? "No items"
                    : $"{_pickerItems.Count} item{(_pickerItems.Count == 1 ? string.Empty : "s")}";
        }

        var statusStyle = _theme.AccentStyle;
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

        string title = relativePath.Length > 0 ? $"Preview: {relativePath}" : "Preview";
        if (_pickerPreviewStatus.Length > 0)
        {
            title += $" — {_pickerPreviewStatus}";
        }

        DrawBox(x, y, width, height, title);

        var style = _theme.TextStyle;
        var dimStyle = _theme.MutedStyle;
        var errorStyle = _theme.ErrorStyle;
        int contentRows = Math.Max(0, height - 2);
        int contentWidth = Math.Max(0, width - 4);

        if (_pickerPreviewStatus.StartsWith("E: ", StringComparison.Ordinal))
        {
            _screen.PutString(x + 2, y + 1, _pickerPreviewStatus, errorStyle, contentWidth);
            return;
        }

        if (_pickerPreviewBinary)
        {
            _screen.PutString(x + 2, y + 1, "Binary file", dimStyle, contentWidth);
            return;
        }

        if (_pickerPreviewLines.Count == 0)
        {
            _screen.PutString(
                x + 2,
                y + 1,
                _pickerPreviewStatus.Length > 0 ? _pickerPreviewStatus : "No preview",
                dimStyle,
                contentWidth
            );
            return;
        }

        LanguageDefinition? language = _config.SyntaxHighlightPickerPreview
            ? _languages.Detect(_pickerPreviewPath)
            : null;
        bool inBlock = false;
        for (int row = 0; row < contentRows && row < _pickerPreviewLines.Count; ++row)
        {
            string line = ExpandPreviewLine(_pickerPreviewLines[row]);
            if (language is null)
            {
                _screen.PutString(x + 2, y + 1 + row, line, style, contentWidth);
                continue;
            }

            RenderPreviewHighlightedLine(
                x + 2,
                y + 1 + row,
                line,
                language,
                ref inBlock,
                contentWidth
            );
        }
    }

    private void RenderPreviewHighlightedLine(
        int x,
        int y,
        string line,
        LanguageDefinition language,
        ref bool inBlock,
        int width
    )
    {
        if (width <= 0)
        {
            return;
        }

        byte[] bytes = Encoding.UTF8.GetBytes(line);
        _lineTokens.Clear();
        SyntaxHighlighter.GetLineTokens(language, bytes, inBlock, _lineTokens, out inBlock);

        var textStyle = _theme.TextStyle;
        int column = 0;
        int offset = 0;

        void Emit(int start, int length, Style style)
        {
            if (length <= 0 || column >= width)
            {
                return;
            }

            string text = Encoding.UTF8.GetString(bytes, start, length);
            _screen.PutString(x + column, y, text, style, width - column);
            column += (int)UnicodeUtil.ColumnWidth(text);
        }

        foreach (SyntaxToken token in _lineTokens)
        {
            if (token.Start > offset)
            {
                Emit(offset, token.Start - offset, textStyle);
            }

            if (token.Start >= offset)
            {
                Emit(token.Start, token.Length, TokenStyle(token.Kind));
                offset = token.Start + token.Length;
            }
        }

        if (offset < bytes.Length)
        {
            Emit(offset, bytes.Length - offset, textStyle);
        }
    }

    private string ExpandPreviewLine(string line)
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
                    _hoverTop + Math.Max(1, HoverContentRows())
                );
                return;
        }

        if (
            inputEvent.Key == Key.Character
            && (inputEvent.Character.Value == 'q' || inputEvent.Character.Value == 'Q')
        )
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
        DrawBox(x, y, width, height, $"{_hoverTitle}");

        var style = _theme.TextStyle;
        int contentRows = Math.Max(0, height - 2);
        int contentWidth = Math.Max(0, width - 4);

        for (int row = 0; row < contentRows; ++row)
        {
            int index = _hoverTop + row;
            if (index >= _hoverLines.Count)
            {
                break;
            }

            int column = 0;
            foreach (HoverSegment segment in _hoverLines[index].Segments)
            {
                if (column >= contentWidth)
                {
                    break;
                }

                Style segmentStyle =
                    segment.Kind == SyntaxTokenKind.Default ? style : TokenStyle(segment.Kind);
                _screen.PutString(
                    x + 2 + column,
                    y + 1 + row,
                    segment.Text,
                    segmentStyle,
                    contentWidth - column
                );
                column += (int)UnicodeUtil.ColumnWidth(segment.Text);
            }
        }
    }

    private void Render()
    {
        _screen.Resize(_columns, _rows);
        _screen.Clear(_theme.BaseStyle);

        UpdateEditorLeft();
        if (!_freeScroll)
        {
            EnsureCursorVisible();
        }

        RenderPanes();
        RenderLspCompletion();

        RenderSidebar();
        RenderSidebarContextMenu();
        RenderStatusLine();
        RenderBottomLine();
        RenderCompletion();
        RenderSpaceMenu();
        RenderPicker();
        RenderFind();
        RenderHover();
        RenderDebugVariables();
        RenderAbout();
        RenderPluginManager();
        RenderLspDebug();
        RenderDapDebug();
        RenderPluginPanel();
        RenderStartupBenchmark();

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
