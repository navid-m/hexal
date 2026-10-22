using Hexal.Util;

namespace Hexal.Core;

/// <summary>
/// Coordinates used by Hexal:
///   * Storage: UTF-8 byte offsets into the document.
///   * Cursor:  a UTF-8 byte offset on a Unicode scalar boundary.
///   * Display: terminal cell columns (tabs expanded, wide scalars = 2).
/// These are deliberately distinct; the UI converts between them. Invalid
/// UTF-8 is preserved on disk and displayed as U+FFFD.
/// </summary>
public sealed class Document
{
    private const int MaxUndoGroups = 4096;
    private const long MaxUndoBytes = 64L * 1024 * 1024;
    private const long Unreachable = -1;

    private static readonly byte[] BomBytes = { 0xEF, 0xBB, 0xBF };
    private static readonly byte[] TabBytes = { (byte)'\t' };

    public const int DefaultIndentWidth = 4;
    public const int DefaultTabWidth = 4;
    private const int MaxIndentWidth = 16;
    private const int MaxTabWidth = 16;

    private enum GroupKind
    {
        Insert,
        Erase,
        Mixed
    }

    private sealed class Change
    {
        public long Offset;
        public readonly ByteBuffer Removed = new();
        public readonly ByteBuffer Inserted = new();
    }

    private sealed class Group
    {
        public GroupKind Kind;
        public readonly List<Change> Changes = new();
    }

    private readonly PieceTable _table = new();
    private readonly List<Group> _undo = new();
    private readonly List<Group> _redo = new();

    private string? _path;
    private string _eol = "\n";
    private bool _bom;
    private long _historyBytes;
    private long _savedPose;
    private bool _insertRunOpen;
    private bool _eraseRunOpen;
    private bool _restoring;
    private int _compositeDepth;
    private Group? _compositeGroup;
    private long _revision;
    private int _indentWidth = DefaultIndentWidth;
    private int _tabWidth = DefaultTabWidth;

    public string? Path => _path;

    public bool HasFile => _path is not null;

    /// <summary>
    /// Updates the file associated with this buffer after an external rename.
    /// The buffer contents and undo history are kept intact.
    /// </summary>
    public void ChangePath(string path)
    {
        _path = path;
    }

    public bool IsDirty => _savedPose < 0 || _savedPose != _undo.Count;

    public string Eol => _eol;

    public bool HasBom => _bom;

    /// <summary>
    /// The in-memory indentation style for this buffer. These settings are not
    /// written to disk; they decide what the indent commands (<c>&gt;</c> and
    /// <c>&lt;</c>) and the Tab key insert. See <see cref="SetIndentSpaces"/>
    /// and <see cref="SetIndentTabs"/>.
    /// </summary>
    public bool UseTabs { get; private set; }

    public int IndentWidth
    {
        get => _indentWidth;
        set => _indentWidth = Math.Clamp(value, 1, MaxIndentWidth);
    }

    public int TabWidth
    {
        get => _tabWidth;
        set => _tabWidth = Math.Clamp(value, 1, MaxTabWidth);
    }

    /// <summary>
    /// Marks the buffer as space-indented and sets the number of spaces per
    /// indent level.
    /// </summary>
    public void SetIndentSpaces(int width)
    {
        IndentWidth = width;
        UseTabs = false;
    }

    /// <summary>
    /// Marks the buffer as tab-indented. <paramref name="width"/> is the visual
    /// width of a tab and also the number of columns removed by an unindent.
    /// </summary>
    public void SetIndentTabs(int width)
    {
        TabWidth = width;
        IndentWidth = width;
        UseTabs = true;
    }

    /// <summary>
    /// The bytes inserted for one indent level: one tab or
    /// <see cref="IndentWidth"/> spaces.
    /// </summary>
    public byte[] IndentUnit()
    {
        if (UseTabs)
        {
            return (byte[])TabBytes.Clone();
        }

        byte[] spaces = new byte[_indentWidth];
        spaces.AsSpan().Fill((byte)' ');
        return spaces;
    }

    /// <summary>
    /// Returns how many bytes of leading whitespace form one indent level.
    /// Tabs advance to the next tab stop and are consumed before any following
    /// spaces, so a tab-indented line loses exactly one tab per unindent.
    /// </summary>
    public int UnindentLength(ReadOnlySpan<byte> text)
    {
        int offset = 0;
        int columns = 0;

        while (offset < text.Length && columns < _indentWidth)
        {
            if (text[offset] == (byte)'\t')
            {
                ++offset;
                if (UseTabs)
                {
                    break;
                }

                columns += _tabWidth - (columns % _tabWidth);
            }
            else if (text[offset] == (byte)' ')
            {
                ++offset;
                ++columns;
            }
            else
            {
                break;
            }
        }

        return offset;
    }

    public long Size => _table.Size;

    public long LineCount => _table.NewlineCount + 1;

    /// <summary>
    /// Monotonically increasing edit counter. Syntax caches and plugins use it
    /// to detect changes without comparing document text.
    /// </summary>
    public long Revision => _revision;

    /// <summary>
    /// Details of the most recent single edit. Callers that batch edits with
    /// <see cref="BeginCompositeEdit"/> use this to keep cursor positions in
    /// sync after each edit.
    /// </summary>
    public bool HasLastChange { get; private set; }

    public long LastChangeOffset { get; private set; }

    public long LastChangeRemoved { get; private set; }

    public long LastChangeInserted { get; private set; }

    public static Document? Open(string path, out string error)
    {
        error = string.Empty;

        if (string.IsNullOrEmpty(path))
        {
            error = "empty file name";
            return null;
        }

        FileAttributes attributes;
        try
        {
            attributes = File.GetAttributes(path);
        }
        catch (FileNotFoundException)
        {
            return OpenNew(path, out error);
        }
        catch (DirectoryNotFoundException)
        {
            error = $"parent directory does not exist: '{FileSystemUtil.ParentDirectory(path)}'";
            return null;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = $"cannot access '{path}': {ex.Message}";
            return null;
        }

        if ((attributes & FileAttributes.Directory) != 0)
        {
            error = $"'{path}' is a directory";
            return null;
        }

        var document = new Document { _path = path };

        try
        {
            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read,
                FileShare.Read, 1, FileOptions.SequentialScan);

            Span<byte> bom = stackalloc byte[3];
            int bomLength = 0;
            while (bomLength < bom.Length)
            {
                int read = stream.Read(bom[bomLength..]);
                if (read == 0)
                {
                    break;
                }
                bomLength += read;
            }

            if (bomLength == 3 && bom[0] == 0xEF && bom[1] == 0xBB && bom[2] == 0xBF)
            {
                document._bom = true;
            }
            else
            {
                stream.Position = 0;
            }

            document._table.SetOriginal(stream);
            document._eol = document._table.DetectedEol;
            return document;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = $"cannot open '{path}' for reading: {ex.Message}";
            return null;
        }
    }

    private static Document? OpenNew(string path, out string error)
    {
        error = string.Empty;

        if (Directory.Exists(path))
        {
            error = $"'{path}' is a directory";
            return null;
        }

        string parent = FileSystemUtil.ParentDirectory(path);
        if (!Directory.Exists(parent))
        {
            error = $"parent directory does not exist: '{parent}'";
            return null;
        }

        return new Document { _path = path };
    }

    public string DisplayName
    {
        get
        {
            if (_path is null)
            {
                return "[No Name]";
            }

            string name = System.IO.Path.GetFileName(_path);
            return name.Length == 0 ? _path : name;
        }
    }

    public long LineStart(long line) => _table.LineStart(line);

    public long LineEnd(long line)
    {
        long count = LineCount;
        if (line >= count)
        {
            return Size;
        }

        if (line + 1 >= count)
        {
            return Size;
        }

        long end = _table.LineStart(line + 1);
        if (end > 0 && _table.ByteAt(end - 1) == (byte)'\n')
        {
            --end;
            if (end > 0 && _table.ByteAt(end - 1) == (byte)'\r')
            {
                --end;
            }
        }
        return end;
    }

    public long LineOfOffset(long offset) => _table.LineOfOffset(offset);

    public byte[] LineText(long line)
    {
        long begin = LineStart(line);
        long end = LineEnd(line);
        return end <= begin ? Array.Empty<byte>() : _table.Range(begin, end - begin);
    }

    public int CopyLineText(long line, ref byte[] buffer, out int length)
    {
        long begin = LineStart(line);
        long end = LineEnd(line);
        if (end <= begin)
        {
            length = 0;
            return 0;
        }

        length = _table.CopyRange(begin, end - begin, ref buffer);
        return length;
    }

    public byte[] Range(long offset, long length) => _table.Range(offset, length);

    public byte[] Text() => _table.ToArray();

    public byte ByteAt(long offset) => _table.ByteAt(offset);

    public void Insert(long offset, ReadOnlySpan<byte> text)
    {
        if (text.IsEmpty)
        {
            return;
        }

        offset = Math.Clamp(offset, 0, Size);
        PrepareForEdit();
        _table.Insert(offset, text);
        ++_revision;
        RecordLastChange(offset, 0, text.Length);

        if (_compositeDepth == 0 && _insertRunOpen && _undo.Count > 0 &&
            !ContainsLineBreak(text))
        {
            Group group = _undo[^1];
            if (group.Kind == GroupKind.Insert && group.Changes.Count == 1)
            {
                Change change = group.Changes[0];
                if (offset == change.Offset + change.Inserted.Length)
                {
                    change.Inserted.Append(text);
                    _historyBytes += text.Length;
                    TrimHistory();
                    _eraseRunOpen = false;
                    return;
                }
            }
        }

        var newGroup = new Group { Kind = GroupKind.Insert };
        var newChange = new Change { Offset = offset };
        newChange.Inserted.Append(text);
        newGroup.Changes.Add(newChange);

        _insertRunOpen = !ContainsLineBreak(text);
        _eraseRunOpen = false;
        PushGroup(newGroup);
    }

    public void Erase(long offset, long length)
    {
        if (length <= 0 || offset >= Size)
        {
            return;
        }

        offset = Math.Clamp(offset, 0, Size);
        length = Math.Min(length, Size - offset);

        byte[] removed = _table.Range(offset, length);
        _table.Erase(offset, length);
        ++_revision;
        RecordLastChange(offset, length, 0);

        if (_restoring)
        {
            return;
        }

        PrepareForEdit();

        if (_compositeDepth == 0 && _eraseRunOpen && _undo.Count > 0 &&
            !ContainsLineBreak(removed))
        {
            Group group = _undo[^1];
            if (group.Kind == GroupKind.Erase && group.Changes.Count == 1)
            {
                Change change = group.Changes[0];
                if (offset + length == change.Offset)
                {
                    // Backspace: extend the deletion to the left.
                    change.Offset = offset;
                    change.Removed.Prepend(removed);
                    _historyBytes += removed.Length;
                    TrimHistory();
                    _insertRunOpen = false;
                    return;
                }

                if (offset == change.Offset)
                {
                    // Delete: extend the deletion to the right.
                    change.Removed.Append(removed);
                    _historyBytes += removed.Length;
                    TrimHistory();
                    _insertRunOpen = false;
                    return;
                }
            }
        }

        var newGroup = new Group { Kind = GroupKind.Erase };
        var newChange = new Change { Offset = offset };
        newChange.Removed.Append(removed);
        newGroup.Changes.Add(newChange);

        _eraseRunOpen = !ContainsLineBreak(removed);
        _insertRunOpen = false;
        PushGroup(newGroup);
    }

    public void Replace(long offset, long length, ReadOnlySpan<byte> text)
    {
        offset = Math.Clamp(offset, 0, Size);
        length = Math.Min(length, Size - offset);

        byte[] removed = _table.Range(offset, length);
        if (removed.AsSpan().SequenceEqual(text))
        {
            return;
        }

        _table.Erase(offset, length);
        if (!text.IsEmpty)
        {
            _table.Insert(offset, text);
        }
        ++_revision;
        RecordLastChange(offset, length, text.Length);

        if (_restoring)
        {
            return;
        }

        PrepareForEdit();

        var group = new Group { Kind = GroupKind.Mixed };
        var change = new Change { Offset = offset };
        change.Removed.Append(removed);
        change.Inserted.Append(text);
        group.Changes.Add(change);

        _insertRunOpen = false;
        _eraseRunOpen = false;
        PushGroup(group);
    }

    public void BreakUndoGroup()
    {
        _insertRunOpen = false;
        _eraseRunOpen = false;
    }

    /// <summary>
    /// Groups all edits made until <see cref="EndCompositeEdit"/> into a single
    /// undo step. Composite edits may be nested. This is used by multi-cursor
    /// operations and comment toggling so one user action can be undone at once.
    /// </summary>
    public void BeginCompositeEdit()
    {
        if (_compositeDepth++ == 0)
        {
            _compositeGroup = new Group { Kind = GroupKind.Mixed };
        }

        BreakUndoGroup();
    }

    public void EndCompositeEdit()
    {
        if (_compositeDepth == 0)
        {
            return;
        }

        if (--_compositeDepth != 0)
        {
            return;
        }

        Group? group = _compositeGroup;
        _compositeGroup = null;
        if (group is not null && group.Changes.Count > 0)
        {
            PushGroup(group);
        }
    }

    public long? Undo()
    {
        BreakUndoGroup();
        if (_undo.Count == 0)
        {
            return null;
        }

        Group group = _undo[^1];
        _undo.RemoveAt(_undo.Count - 1);

        _restoring = true;
        for (int i = group.Changes.Count - 1; i >= 0; --i)
        {
            ApplyChangeBackward(group.Changes[i]);
        }
        _restoring = false;
        ++_revision;

        long hint = group.Changes[0].Offset;
        if (group.Changes[0].Inserted.Length == 0)
        {
            hint += group.Changes[0].Removed.Length;
        }

        _redo.Add(group);
        TrimHistory();
        return hint;
    }

    public long? Redo()
    {
        BreakUndoGroup();
        if (_redo.Count == 0)
        {
            return null;
        }

        Group group = _redo[^1];
        _redo.RemoveAt(_redo.Count - 1);

        _restoring = true;
        foreach (Change change in group.Changes)
        {
            ApplyChangeForward(change);
        }
        _restoring = false;
        ++_revision;

        Change last = group.Changes[^1];
        long hint = last.Inserted.Length == 0
            ? last.Offset
            : last.Offset + last.Inserted.Length;

        _undo.Add(group);
        TrimHistory();
        return hint;
    }

    public bool CanUndo => _undo.Count > 0;

    public bool CanRedo => _redo.Count > 0;

    /// <summary>
    /// An immutable view of everything needed to write one version of the
    /// document. It is safe to consume from a worker thread while the live
    /// document continues to be edited.
    /// </summary>
    public sealed class SaveSnapshot
    {
        public string Path { get; }
        public bool Bom { get; }
        public long Pose { get; }
        public long Revision { get; }

        private readonly PieceTable.Snapshot _table;

        internal SaveSnapshot(string path, bool bom, long pose, long revision,
            PieceTable.Snapshot table)
        {
            Path = path;
            Bom = bom;
            Pose = pose;
            Revision = revision;
            _table = table;
        }

        public bool WriteTo(out string error)
        {
            return FileSystemUtil.AtomicWrite(Path, stream =>
            {
                if (Bom)
                {
                    stream.Write(BomBytes);
                }
                _table.WriteTo(stream);
            }, out error);
        }
    }

    /// <summary>
    /// Captures a save snapshot. Pass a non-null path for :w &lt;file&gt;;
    /// otherwise the document's associated path is used.
    /// </summary>
    public SaveSnapshot? CreateSaveSnapshot(string? path, out string error)
    {
        error = string.Empty;

        string target = path ?? _path ?? string.Empty;
        if (target.Length == 0)
        {
            error = path is null
                ? "no file name assigned (use :w <file>)"
                : "empty file name";
            return null;
        }

        return new SaveSnapshot(target, _bom, _undo.Count, _revision,
            _table.CreateSnapshot());
    }

    /// <summary>
    /// Applies the result of a completed background save. If the buffer was
    /// edited after the snapshot was taken, the document stays dirty; edits
    /// made during the save are never silently marked as persisted. The
    /// revision check catches the case where an undo plus a new edit returns
    /// the history to the same group count with different contents.
    /// </summary>
    public void MarkSaved(string path, long pose, long revision)
    {
        _path = path;
        if (_revision == revision)
        {
            _savedPose = pose;
        }
        BreakUndoGroup();
    }

    public bool Save(out string error)
    {
        SaveSnapshot? snapshot = CreateSaveSnapshot(null, out error);
        if (snapshot is null)
        {
            return false;
        }

        if (!snapshot.WriteTo(out error))
        {
            return false;
        }

        MarkSaved(snapshot.Path, snapshot.Pose, snapshot.Revision);
        return true;
    }

    public bool SaveAs(string path, out string error)
    {
        SaveSnapshot? snapshot = CreateSaveSnapshot(path, out error);
        if (snapshot is null)
        {
            return false;
        }

        if (!snapshot.WriteTo(out error))
        {
            return false;
        }

        MarkSaved(snapshot.Path, snapshot.Pose, snapshot.Revision);
        return true;
    }

    private void RecordLastChange(long offset, long removed, long inserted)
    {
        HasLastChange = true;
        LastChangeOffset = offset;
        LastChangeRemoved = removed;
        LastChangeInserted = inserted;
    }

    private void PrepareForEdit()
    {
        if (_redo.Count == 0)
        {
            return;
        }

        if (_savedPose > _undo.Count)
        {
            _savedPose = Unreachable;
        }

        foreach (Group group in _redo)
        {
            _historyBytes -= GroupBytes(group);
        }
        _redo.Clear();
    }

    private void PushGroup(Group group)
    {
        if (_compositeDepth > 0)
        {
            _compositeGroup?.Changes.AddRange(group.Changes);
            return;
        }

        _historyBytes += GroupBytes(group);
        _undo.Add(group);
        TrimHistory();
    }

    private void TrimHistory()
    {
        long dropped = 0;
        while (_undo.Count > 1 &&
               (_undo.Count > MaxUndoGroups || _historyBytes > MaxUndoBytes))
        {
            _historyBytes -= GroupBytes(_undo[0]);
            _undo.RemoveAt(0);
            ++dropped;
        }

        if (dropped > 0 && _savedPose >= 0)
        {
            if (_savedPose >= dropped)
            {
                _savedPose -= dropped;
            }
            else
            {
                _savedPose = Unreachable;
            }
        }

        while (_redo.Count > 0 &&
               (_redo.Count > MaxUndoGroups || _historyBytes > MaxUndoBytes))
        {
            if (_savedPose > _undo.Count)
            {
                _savedPose = Unreachable;
            }
            _historyBytes -= GroupBytes(_redo[0]);
            _redo.RemoveAt(0);
        }
    }

    private static long GroupBytes(Group group)
    {
        long total = 32;
        foreach (Change change in group.Changes)
        {
            total += change.Removed.Length + change.Inserted.Length;
        }
        return total;
    }

    private void ApplyChangeForward(Change change)
    {
        if (change.Removed.Length > 0)
        {
            _table.Erase(change.Offset, change.Removed.Length);
        }
        if (change.Inserted.Length > 0)
        {
            _table.Insert(change.Offset, change.Inserted.Span);
        }
    }

    private void ApplyChangeBackward(Change change)
    {
        if (change.Inserted.Length > 0)
        {
            _table.Erase(change.Offset, change.Inserted.Length);
        }
        if (change.Removed.Length > 0)
        {
            _table.Insert(change.Offset, change.Removed.Span);
        }
    }

    private static bool ContainsLineBreak(ReadOnlySpan<byte> text)
        => text.IndexOf((byte)'\n') >= 0 || text.IndexOf((byte)'\r') >= 0;
}
