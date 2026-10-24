using Hexal.Core;
using Hexal.Languages;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    /// <summary>
    /// Reloads every file-backed buffer from disk while preserving the open
    /// pane layout. Modified buffers are kept in memory unless
    /// <paramref name="force"/> is set. The renderer and screen are reset so
    /// the next frame repaints the whole interface, which also clears terminal
    /// artefacts left behind by binary content.
    /// </summary>
    private void ReloadAll(bool force)
    {
        if (_paneRoot is not null)
        {
            CapturePaneState(_activePane);
        }

        List<EditorPane> panes = AllPanes();
        var seen = new HashSet<Document>(ReferenceEqualityComparer.Instance);
        var replacements = new Dictionary<Document, Document>(
            ReferenceEqualityComparer.Instance
        );
        var positions =
            new Dictionary<
                EditorPane,
                (
                    long CursorLine,
                    int CursorByte,
                    long AnchorLine,
                    int AnchorByte,
                    long TopLine,
                    long LeftColumn
                )
            >();
        int reloaded = 0;
        int keptDirty = 0;
        string firstError = string.Empty;

        foreach (EditorPane pane in panes)
        {
            positions[pane] = (
                LineForOffset(pane.Document, pane.Cursor),
                ByteColumnForOffset(pane.Document, pane.Cursor),
                LineForOffset(pane.Document, pane.SelectionAnchor),
                ByteColumnForOffset(pane.Document, pane.SelectionAnchor),
                pane.TopLine,
                pane.LeftColumn
            );

            Document document = pane.Document;
            if (document.Path is null || !seen.Add(document))
            {
                continue;
            }

            if (!force && document.IsDirty)
            {
                ++keptDirty;
                continue;
            }

            if (!File.Exists(document.Path))
            {
                if (firstError.Length == 0)
                {
                    firstError = $"'{document.Path}' no longer exists";
                }
                continue;
            }

            Document? loaded = Document.Open(document.Path, out string error);
            if (loaded is null)
            {
                if (firstError.Length == 0)
                {
                    firstError = error;
                }
                continue;
            }

            CopyIndentationSettings(document, loaded);
            replacements[document] = loaded;
            ++reloaded;
        }

        foreach (EditorPane pane in panes)
        {
            if (!replacements.TryGetValue(pane.Document, out Document? loaded))
            {
                continue;
            }

            pane.Document = loaded;
            pane.Language = loaded.Path is null ? null : _languages.Detect(loaded.Path);
            pane.Syntax = new SyntaxHighlighter();
            pane.Syntax.SetDocument(loaded, pane.Language);
            pane.FileStamp = default;
            if (loaded.Path is not null && TryCaptureFileStamp(loaded.Path, out FileStamp stamp))
            {
                pane.FileStamp = stamp;
            }
            pane.ExternalCheckPending = false;
            pane.Mode = Mode.Normal;
            pane.PendingG = false;
            pane.FindActive = false;
            pane.FindEditing = false;
            pane.FindMatches.Clear();
            pane.FindMatchIndex = -1;
            pane.FindScope.Clear();
            pane.ExtraSelections.Clear();
            pane.PreferredColumnValid = false;
            pane.DiffLines = new Dictionary<long, GitDiffLineKind>();
            pane.DiffBaseline = null;
            pane.DiffBaselinePath = string.Empty;
            pane.DiffRevision = -1;

            (
                long cursorLine,
                int cursorByte,
                long anchorLine,
                int anchorByte,
                long topLine,
                long leftColumn
            ) = positions[pane];
            pane.Cursor = OffsetForLineByte(loaded, cursorLine, cursorByte);
            pane.SelectionAnchor = OffsetForLineByte(loaded, anchorLine, anchorByte);
            pane.TopLine = Math.Clamp(topLine, 0, Math.Max(0, loaded.LineCount - 1));
            pane.LeftColumn = Math.Max(0, leftColumn);
        }

        if (_paneRoot is not null)
        {
            LoadPaneState(_activePane);
        }

        RefreshAllPaneDiffLines();
        ResetLsp();
        FirePluginEvent("buffer_opened", CurrentBufferData());
        RefreshFileStamp();
        InvalidateFileListCaches();
        if (_sidebarVisible)
        {
            RefreshSidebar(SelectedSidebarNode?.Path);
        }

        _renderer.Reset();
        _screen = new Screen();
        _terminal.Write("\x1b[0m\x1b[r\x1b[2J\x1b[H"u8);
        _forceFullRedraw = true;
        UpdateEditorLeft();
        SnapCursor();
        EnsureCursorVisible();

        if (firstError.Length > 0)
        {
            SetError($"Reloaded {reloaded} file(s); {firstError}.");
        }
        else if (reloaded == 0 && keptDirty > 0)
        {
            SetMessage($"Kept {keptDirty} modified file(s); nothing reloaded.");
        }
        else if (keptDirty > 0)
        {
            SetMessage($"Reloaded {reloaded} file(s); kept {keptDirty} modified.");
        }
        else if (reloaded == 0)
        {
            SetMessage("No file-backed buffers to reload.");
        }
        else
        {
            SetMessage($"Reloaded {reloaded} file(s).");
        }
    }

    private static long LineForOffset(Document document, long offset)
    {
        return document.LineOfOffset(Math.Clamp(offset, 0, document.Size));
    }

    private static int ByteColumnForOffset(Document document, long offset)
    {
        offset = Math.Clamp(offset, 0, document.Size);
        long line = document.LineOfOffset(offset);
        long start = document.LineStart(line);
        return (int)Math.Clamp(offset - start, 0, int.MaxValue);
    }

    private static long OffsetForLineByte(Document document, long line, int byteColumn)
    {
        line = Math.Clamp(line, 0, Math.Max(0, document.LineCount - 1));
        long start = document.LineStart(line);
        long end = document.LineEnd(line);
        return Math.Clamp(start + byteColumn, start, end);
    }

    private static void CopyIndentationSettings(Document source, Document target)
    {
        if (source.UseTabs)
        {
            target.SetIndentTabs(source.TabWidth);
            target.IndentWidth = source.IndentWidth;
            return;
        }

        target.TabWidth = source.TabWidth;
        target.SetIndentSpaces(source.IndentWidth);
    }
}
