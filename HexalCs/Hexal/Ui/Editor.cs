using System.Text;
using Hexal.Core;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed class Editor
{
    private const int TabWidth = 4;

    private static readonly byte[] Lf = { (byte)'\n' };
    private static readonly byte[] CrLf = { (byte)'\r', (byte)'\n' };

    private enum Mode
    {
        Normal,
        Insert,
        Command
    }

    private enum PromptKind
    {
        None,
        Quit,
        Open
    }

    private readonly ITerminal _terminal;
    private readonly Renderer _renderer = new();
    private readonly Screen _screen = new();

    private Document _document;

    private Mode _mode = Mode.Normal;
    private PromptKind _prompt = PromptKind.None;
    private string _pendingOpen = string.Empty;

    private long _cursor;
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

    public Editor(ITerminal terminal, Document document)
    {
        _terminal = terminal;
        _document = document;
        SnapCursor();
    }

    public int Run()
    {
        RefreshSize();
        EnsureCursorVisible();
        Render();

        while (_running)
        {
            if (PollSave())
            {
                Render();
            }
            if (!_running)
            {
                break;
            }

            // A pending save needs a periodic wake-up so the UI can report
            // completion and :wq can finish. Idle with no save uses a blocking
            // read and consumes no CPU.
            int timeout = _saveOperation is null ? -1 : 50;
            if (!_terminal.ReadEvent(out InputEvent inputEvent, timeout))
            {
                break;
            }

            if (inputEvent.Key == Key.None)
            {
                if (PollSave())
                {
                    Render();
                }
                continue;
            }

            if (!HandleEvent(inputEvent))
            {
                break;
            }

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

        try
        {
            if (_prompt != PromptKind.None)
            {
                HandlePrompt(inputEvent);
            }
            else if (_mode == Mode.Command)
            {
                HandleCommandInput(inputEvent);
            }
            else if (_mode == Mode.Insert)
            {
                HandleInsert(inputEvent);
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
                SetMessage("Use :q to quit.");
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
                    MoveToDocumentStart(recordJump: true);
                }
                else if (character is 'e' or 'E')
                {
                    MoveToDocumentEnd(recordJump: true);
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
                MoveLeft();
                return;
            case Key.Right:
                MoveRight();
                return;
            case Key.Up:
                MoveVertical(-1);
                return;
            case Key.Down:
                MoveVertical(1);
                return;
            case Key.PageUp:
                MovePage(-1);
                return;
            case Key.PageDown:
                MovePage(1);
                return;
            case Key.Home:
                if (inputEvent.Ctrl)
                {
                    MoveToDocumentStart(recordJump: true);
                }
                else
                {
                    MoveTo(LineStartOfCursor(), recordJump: false);
                }
                return;
            case Key.End:
                if (inputEvent.Ctrl)
                {
                    MoveToDocumentEnd(recordJump: true);
                }
                else
                {
                    MoveTo(LineEndOfCursor(), recordJump: false);
                }
                return;
            case Key.Escape:
                _document.BreakUndoGroup();
                ClearMessage();
                return;
            case Key.Delete:
                DeleteAtCursor();
                return;
        }

        if (inputEvent.Key != Key.Character)
        {
            return;
        }

        switch (inputEvent.Character.Value)
        {
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
                return;
            }
            case 'A':
                _cursor = LineEndOfCursor();
                _preferredColumnValid = false;
                EnterInsert();
                return;
            case 'o':
                InsertNewlineBelow();
                return;
            case 'O':
                InsertNewlineAbove();
                return;
            case 'h':
                MoveLeft();
                return;
            case 'l':
                MoveRight();
                return;
            case 'j':
                MoveVertical(1);
                return;
            case 'k':
                MoveVertical(-1);
                return;
            case 'x':
                DeleteAtCursor();
                return;
            case 'u':
                Undo();
                return;
            case 'g':
            case 'G':
                _pendingG = true;
                return;
            case 'V':
                SetMessage("SELECT MODE is planned for Phase 2 and is not in this MVP.");
                return;
            case 'X':
                SetMessage("Line selection is planned for Phase 2 and is not in this MVP.");
                return;
            case 'C':
                SetMessage("Line commenting is planned for Phase 4 and is not in this MVP.");
                return;
            default:
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
                MoveLeft();
                return;
            case Key.Right:
                MoveRight();
                return;
            case Key.Up:
                MoveVertical(-1);
                return;
            case Key.Down:
                MoveVertical(1);
                return;
            case Key.PageUp:
                MovePage(-1);
                return;
            case Key.PageDown:
                MovePage(1);
                return;
            case Key.Home:
                if (inputEvent.Ctrl)
                {
                    MoveToDocumentStart(recordJump: true);
                }
                else
                {
                    MoveTo(LineStartOfCursor(), recordJump: false);
                }
                return;
            case Key.End:
                if (inputEvent.Ctrl)
                {
                    MoveToDocumentEnd(recordJump: true);
                }
                else
                {
                    MoveTo(LineEndOfCursor(), recordJump: false);
                }
                return;
            case Key.Enter:
                InsertNewline();
                return;
            case Key.Backspace:
                Backspace();
                return;
            case Key.Delete:
                DeleteForward();
                return;
            case Key.Tab:
                InsertCharacter(new Rune('\t'));
                return;
        }

        if (inputEvent.Key == Key.Character && !inputEvent.Ctrl && !inputEvent.Alt)
        {
            InsertCharacter(inputEvent.Character);
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
            _pendingOpen = string.Empty;
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
                _pendingOpen = string.Empty;
                OpenPath(path);
            }
            return;
        }

        if (character is 'n' or 'N')
        {
            _prompt = PromptKind.None;
            _pendingOpen = string.Empty;
            SetMessage("Cancelled.");
        }
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

            case "help":
                ClearMessage();
                SetMessage("i insert | Esc normal | :w :wq :q :e | u/Ctrl+R undo | gg/ge/gp");
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
        => name is "fmt" or "cfg" or "rcfg" or "lang" or "sf" or "g" or "sv" or "sh" or
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
                SetMessage($"Wrote {result.Path} ({_document.LineCount} lines)");
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

    private void RequestOpen(string path, bool force)
    {
        if (!force && _document.IsDirty)
        {
            _pendingOpen = path;
            _prompt = PromptKind.Open;
            return;
        }

        OpenPath(path);
    }

    private void OpenPath(string path)
    {
        Document? loaded = Document.Open(path, out string error);
        if (loaded is null)
        {
            SetError(error);
            return;
        }

        _document = loaded;
        _cursor = 0;
        _topLine = 0;
        _leftColumn = 0;
        _preferredColumnValid = false;
        _jumpHistory.Clear();
        _pendingG = false;
        _mode = Mode.Normal;
        _commandLine.Clear();
        _commandCursor = 0;
        SnapCursor();
        SetMessage($"Opened {path}");
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
        SnapCursor();
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
        SnapCursor();
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        ClearMessage();
    }

    // -----------------------------------------------------------------------
    // NAVIGATION
    // -----------------------------------------------------------------------

    private long CursorLine() => _document.LineOfOffset(_cursor);

    private long CursorVisualColumn()
    {
        long line = CursorLine();
        long begin = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        return UnicodeUtil.VisualColumn(_lineBuffer.AsSpan(0, length),
            (int)(_cursor - begin), TabWidth);
    }

    private int TextRows() => _rows > 2 ? _rows - 2 : 0;

    private long LineStartOfCursor() => _document.LineStart(CursorLine());

    private long LineEndOfCursor() => _document.LineEnd(CursorLine());

    private void SnapCursor()
    {
        if (_cursor > _document.Size)
        {
            _cursor = _document.Size;
        }

        long line = _document.LineOfOffset(_cursor);
        long begin = _document.LineStart(line);
        long end = _document.LineEnd(line);

        if (_cursor < begin)
        {
            _cursor = begin;
        }
        else if (_cursor > end)
        {
            _cursor = end;
        }
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
            Mode.Command => "COMMAND",
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
        if (_saveOperation is not null)
        {
            text += " [saving]";
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
        var textStyle = new Style();

        for (int row = 0; row < rows; ++row)
        {
            long line = firstLine + row;
            if (line >= total)
            {
                break;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);

            int offset = 0;
            long visual = 0;

            while (offset < length)
            {
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

    private void RenderCursor(long firstLine, int rows, int gutter)
    {
        long line = CursorLine();
        if (line < firstLine || line >= firstLine + rows)
        {
            return;
        }

        int row = (int)(line - firstLine);
        long column = CursorVisualColumn();
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
            RenderCursor(_topLine, rows, gutter);
        }

        RenderStatusLine();
        RenderBottomLine();

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
