using Hexal.Core;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const char DiffMarkerCharacter = '\u258e';

    private bool _showDiffLines = true;
    private Dictionary<long, GitDiffLineKind> _diffLines = new();
    private string _diffLinesPath = string.Empty;
    private ulong[]? _diffBaseline;
    private string _diffBaselinePath = string.Empty;
    private long _diffRevision = -1;
    private DiffLoadOperation? _diffLoadOperation;
    private long _diffLoadGeneration;

    private sealed class DiffLoadOperation
    {
        public long Generation;
        public Document Document = null!;
        public string Path = string.Empty;
        public Task<DiffLoadResult> Task = null!;
    }

    private sealed class DiffLoadResult
    {
        public ulong[]? Baseline;
        public Dictionary<long, GitDiffLineKind> Diff = new();
    }

    /// <summary>
    /// Starts reading Git state in the background. Git can be slow on
    /// machines with endpoint protection or network-backed home directories,
    /// so it must never run on the render/input thread.
    /// </summary>
    private void RefreshDiffLines()
    {
        ++_diffLoadGeneration;
        _diffLoadOperation = null;
        string oldPath = _diffLinesPath;
        _diffBaseline = null;
        _diffBaselinePath = string.Empty;
        _diffRevision = _document.Revision;

        if (!_showDiffLines || _document.Path is null)
        {
            _diffLines.Clear();
            _diffLinesPath = string.Empty;
            return;
        }

        Document document = _document;
        string path = document.Path;
        long lineCount = document.LineCount;
        long generation = _diffLoadGeneration;
        bool live = GitDiff.CanComputeLive(lineCount);

        // Keep the previous markers while an unchanged file is reloaded (for
        // example after a save) so the gutter does not flicker. A different
        // buffer must not inherit stale markers.
        if (!SamePath(oldPath, path))
        {
            _diffLines.Clear();
        }
        _diffLinesPath = path;

        Task<DiffLoadResult> task = Task.Run(() =>
        {
            if (live)
            {
                ulong[]? baseline = GitDiff.LoadBaseline(path);
                if (baseline is not null)
                {
                    return new DiffLoadResult { Baseline = baseline };
                }
            }

            return new DiffLoadResult { Diff = GitDiff.Load(path, lineCount) };
        });

        _diffLoadOperation = new DiffLoadOperation
        {
            Generation = generation,
            Document = document,
            Path = path,
            Task = task,
        };
    }

    private bool PollDiffLoad()
    {
        DiffLoadOperation? operation = _diffLoadOperation;
        if (operation is null || !operation.Task.IsCompleted)
        {
            return false;
        }

        _diffLoadOperation = null;
        if (operation.Generation != _diffLoadGeneration)
        {
            return false;
        }

        DiffLoadResult result;
        try
        {
            result = operation.Task.GetAwaiter().GetResult();
        }
        catch
        {
            return false;
        }

        bool applied = false;
        ulong[]? baseline = result.Baseline;
        bool live = baseline is not null && GitDiff.CanComputeLive(operation.Document.LineCount);
        Dictionary<long, GitDiffLineKind> diff = live
            ? GitDiff.Compute(baseline!, DocumentLineHashes(operation.Document))
            : result.Diff;

        foreach (EditorPane pane in AllPanes())
        {
            Document paneDocument = pane.Document;
            if (
                !ReferenceEquals(paneDocument, operation.Document)
                || paneDocument.Path is null
                || !SamePath(paneDocument.Path, operation.Path)
            )
            {
                continue;
            }

            bool active = ReferenceEquals(pane, _activePane);
            pane.DiffBaseline = live ? baseline : null;
            pane.DiffBaselinePath = live ? operation.Path : string.Empty;
            pane.DiffRevision = paneDocument.Revision;
            pane.DiffLines = active
                ? diff
                : new Dictionary<long, GitDiffLineKind>(diff);

            if (active)
            {
                _diffBaseline = pane.DiffBaseline;
                _diffBaselinePath = pane.DiffBaselinePath;
                _diffRevision = pane.DiffRevision;
                _diffLines = pane.DiffLines;
                _diffLinesPath = operation.Path;
            }

            applied = true;
        }

        return applied;
    }

    /// <summary>
    /// Synchronous variant used when display settings change and every pane
    /// must be updated as part of one multi-pane state swap.
    /// </summary>
    private void RefreshDiffLinesImmediate()
    {
        ++_diffLoadGeneration;
        _diffLoadOperation = null;
        _diffBaseline = null;
        _diffBaselinePath = string.Empty;
        _diffRevision = _document.Revision;

        if (!_showDiffLines || _document.Path is null)
        {
            _diffLines.Clear();
            _diffLinesPath = string.Empty;
            return;
        }

        _diffLinesPath = _document.Path;
        if (GitDiff.CanComputeLive(_document.LineCount))
        {
            ulong[]? baseline = GitDiff.LoadBaseline(_document.Path);
            if (baseline is not null)
            {
                _diffBaseline = baseline;
                _diffBaselinePath = _document.Path;
                _diffLines = ComputeLiveDiffLines();
                return;
            }
        }

        _diffLines = GitDiff.Load(_document.Path, _document.LineCount);
    }

    private void UpdateDiffLinesForBufferChange()
    {
        if (
            !_showDiffLines
            || _diffBaseline is null
            || _document.Path is null
            || !SamePath(_document.Path, _diffBaselinePath)
            || _document.Revision == _diffRevision
            || !GitDiff.CanComputeLive(_document.LineCount)
        )
        {
            return;
        }

        _diffRevision = _document.Revision;
        _diffLines = ComputeLiveDiffLines();
    }

    private Dictionary<long, GitDiffLineKind> ComputeLiveDiffLines()
    {
        if (_diffBaseline is null)
        {
            return new Dictionary<long, GitDiffLineKind>();
        }

        return GitDiff.Compute(_diffBaseline, DocumentLineHashes());
    }

    private ulong[] DocumentLineHashes()
    {
        long count = _document.LineCount;
        var hashes = new ulong[count];
        for (long line = 0; line < count; ++line)
        {
            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            hashes[line] = GitDiff.HashLine(_lineBuffer.AsSpan(0, length));
        }

        return hashes;
    }

    private static ulong[] DocumentLineHashes(Document document)
    {
        long count = document.LineCount;
        var hashes = new ulong[count];
        byte[] buffer = Array.Empty<byte>();
        for (long line = 0; line < count; ++line)
        {
            int length = document.CopyLineText(line, ref buffer, out _);
            hashes[line] = GitDiff.HashLine(buffer.AsSpan(0, length));
        }

        return hashes;
    }

    private void RefreshAllPaneDiffLines()
    {
        if (_paneRoot is null)
        {
            RefreshDiffLinesImmediate();
            return;
        }

        EditorPane active = _activePane;
        CapturePaneState(active);
        foreach (EditorPane pane in AllPanes())
        {
            LoadPaneState(pane);
            RefreshDiffLinesImmediate();
            CapturePaneState(pane);
        }

        LoadPaneState(active);
    }

    private void ToggleDiffLines()
    {
        _showDiffLines = !_showDiffLines;
        RefreshAllPaneDiffLines();
        PersistDisplaySettings();
        _forceFullRedraw = true;
        SetMessage(_showDiffLines ? "Showing git diff lines." : "Hiding git diff lines.");
    }

    private void JumpToNextDiffLine() => JumpToDiffLine(forward: true);

    private void JumpToPreviousDiffLine() => JumpToDiffLine(forward: false);

    private void JumpToDiffLine(bool forward)
    {
        UpdateDiffLinesForBufferChange();

        if (!_showDiffLines)
        {
            SetMessage("Git diff lines are hidden.");
            return;
        }

        if (_diffLines.Count == 0)
        {
            SetMessage("No git diff lines.");
            return;
        }

        long currentLine = _document.LineOfOffset(_cursor);
        long target = -1;
        foreach (long line in _diffLines.Keys)
        {
            if (forward)
            {
                if (line > currentLine && (target < 0 || line < target))
                {
                    target = line;
                }
            }
            else if (line < currentLine && line > target)
            {
                target = line;
            }
        }

        if (target < 0)
        {
            SetMessage(forward ? "No next git diff line." : "No previous git diff line.");
            return;
        }

        target = Math.Clamp(target, 0, _document.LineCount - 1);
        _cursor = _document.LineStart(target);
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        _extraSelections.Clear();
        _preferredColumnValid = false;
        SnapCursor();
        EnsureCursorVisible();
        SetMessage($"Git diff line {target + 1}");
    }

    private bool TryGetDiffMarker(long line, out char marker, out Style style)
    {
        if (!_showDiffLines || !_diffLines.TryGetValue(line, out GitDiffLineKind kind))
        {
            marker = '\0';
            style = default;
            return false;
        }

        marker = DiffMarkerCharacter;
        style = kind switch
        {
            GitDiffLineKind.Added => _theme.DiffAddedStyle,
            GitDiffLineKind.Deleted => _theme.DiffDeletedStyle,
            _ => _theme.DiffModifiedStyle,
        };
        return true;
    }

    private void PersistDisplaySettings()
    {
        if (
            HexalConfig.UpdateDisplaySettings(
                _config.SourcePath,
                _showDiffLines,
                _showGitIgnored,
                _searchMatchCase,
                out string error
            )
        )
        {
            return;
        }

        SetError($"config: {error}");
    }
}
