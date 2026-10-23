using Hexal.Core;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const char DiffMarkerCharacter = '\u258e';

    private bool _showDiffLines = true;
    private Dictionary<long, GitDiffLineKind> _diffLines = new();
    private ulong[]? _diffBaseline;
    private string _diffBaselinePath = string.Empty;
    private long _diffRevision = -1;

    private void RefreshDiffLines()
    {
        _diffBaseline = null;
        _diffBaselinePath = string.Empty;
        _diffRevision = _document.Revision;

        if (!_showDiffLines || _document.Path is null)
        {
            _diffLines.Clear();
            return;
        }

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

    private void RefreshAllPaneDiffLines()
    {
        if (_paneRoot is null)
        {
            RefreshDiffLines();
            return;
        }

        EditorPane active = _activePane;
        CapturePaneState(active);
        foreach (EditorPane pane in AllPanes())
        {
            LoadPaneState(pane);
            RefreshDiffLines();
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
