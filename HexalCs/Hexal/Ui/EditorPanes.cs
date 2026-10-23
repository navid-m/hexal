using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private enum PaneSplitOrientation
    {
        Vertical,
        Horizontal,
    }

    private enum PaneDirection
    {
        Left,
        Right,
        Up,
        Down,
    }

    private abstract class PaneNode
    {
        public PaneSplit? Parent;
    }

    private sealed class PaneLeaf : PaneNode
    {
        public EditorPane Pane = null!;
    }

    private sealed class PaneSplit : PaneNode
    {
        public PaneSplitOrientation Orientation;
        public double Ratio = 0.5;
        public PaneNode First = null!;
        public PaneNode Second = null!;
    }

    private sealed class EditorPane
    {
        public Document Document = null!;
        public LanguageDefinition? Language;
        public SyntaxHighlighter Syntax = new();
        public FileStamp FileStamp;
        public bool ExternalCheckPending;
        public Mode Mode = Mode.Normal;
        public long Cursor;
        public long SelectionAnchor;
        public bool SelectionLinewise;
        public List<Selection> ExtraSelections = new();
        public bool FindActive;
        public bool FindEditing;
        public bool FindMultiCursor;
        public List<Selection>? FindSavedSelections;
        public StringBuilder FindQuery = new();
        public int FindQueryCursor;
        public List<(long Start, long End)> FindScope = new();
        public List<FindMatch> FindMatches = new();
        public int FindMatchIndex = -1;
        public long TopLine;
        public long LeftColumn;
        public long PreferredColumn;
        public bool PreferredColumnValid;
        public bool FreeScroll;
        public bool MouseDragging;
        public long MouseDragAnchor;
        public List<JumpLocation> JumpHistory = new();
        public List<JumpLocation> JumpForward = new();
        public bool PendingG;
        public Dictionary<long, GitDiffLineKind> DiffLines = new();
        public ulong[]? DiffBaseline;
        public string DiffBaselinePath = string.Empty;
        public long DiffRevision = -1;
    }

    private readonly record struct PaneView(EditorPane Pane, int X, int Y, int Width, int Height);

    private readonly record struct PaneBounds(int X, int Y, int Width, int Height);

    private readonly record struct PaneDivider(int X, int Y, int Length, bool Vertical);

    private const int MinPaneSplitWidth = 12;
    private const int MinPaneSplitHeight = 3;
    private const double PaneResizeStep = 0.05;

    private readonly Screen _paneScreen = new();
    private readonly List<PaneView> _paneViews = new();
    private readonly List<PaneDivider> _paneDividers = new();
    private PaneNode? _paneRoot;
    private EditorPane _activePane = null!;
    private bool _pendingWindow;

    private void InitializePanes()
    {
        var pane = new EditorPane();
        CapturePaneState(pane);
        _paneRoot = new PaneLeaf { Pane = pane };
        _activePane = pane;
    }

    private void CapturePaneState(EditorPane pane)
    {
        pane.Document = _document;
        pane.Language = _language;
        pane.Syntax = _syntax;
        pane.FileStamp = _fileStamp;
        pane.ExternalCheckPending = _externalCheckPending;
        pane.Mode = _mode;
        pane.Cursor = _cursor;
        pane.SelectionAnchor = _selectionAnchor;
        pane.SelectionLinewise = _selectionLinewise;
        pane.ExtraSelections = _extraSelections;
        pane.FindActive = _findActive;
        pane.FindEditing = _findEditing;
        pane.FindMultiCursor = _findMultiCursor;
        pane.FindSavedSelections = _findSavedSelections;
        pane.FindQuery = _findQuery;
        pane.FindQueryCursor = _findQueryCursor;
        pane.FindScope = _findScope;
        pane.FindMatches = _findMatches;
        pane.FindMatchIndex = _findMatchIndex;
        pane.TopLine = _topLine;
        pane.LeftColumn = _leftColumn;
        pane.PreferredColumn = _preferredColumn;
        pane.PreferredColumnValid = _preferredColumnValid;
        pane.FreeScroll = _freeScroll;
        pane.MouseDragging = _mouseDragging;
        pane.MouseDragAnchor = _mouseDragAnchor;
        pane.JumpHistory = _jumpHistory;
        pane.JumpForward = _jumpForward;
        pane.PendingG = _pendingG;
        pane.DiffLines = _diffLines;
        pane.DiffBaseline = _diffBaseline;
        pane.DiffBaselinePath = _diffBaselinePath;
        pane.DiffRevision = _diffRevision;
    }

    private void LoadPaneState(EditorPane pane)
    {
        _document = pane.Document;
        _language = pane.Language;
        _syntax = pane.Syntax;
        _fileStamp = pane.FileStamp;
        _externalCheckPending = pane.ExternalCheckPending;
        _mode = pane.Mode;
        _cursor = pane.Cursor;
        _selectionAnchor = pane.SelectionAnchor;
        _selectionLinewise = pane.SelectionLinewise;
        _extraSelections = pane.ExtraSelections;
        _findActive = pane.FindActive;
        _findEditing = pane.FindEditing;
        _findMultiCursor = pane.FindMultiCursor;
        _findSavedSelections = pane.FindSavedSelections;
        _findQuery = pane.FindQuery;
        _findQueryCursor = pane.FindQueryCursor;
        _findScope = pane.FindScope;
        _findMatches = pane.FindMatches;
        _findMatchIndex = pane.FindMatchIndex;
        _topLine = pane.TopLine;
        _leftColumn = pane.LeftColumn;
        _preferredColumn = pane.PreferredColumn;
        _preferredColumnValid = pane.PreferredColumnValid;
        _freeScroll = pane.FreeScroll;
        _mouseDragging = pane.MouseDragging;
        _mouseDragAnchor = pane.MouseDragAnchor;
        _jumpHistory = pane.JumpHistory;
        _jumpForward = pane.JumpForward;
        _pendingG = pane.PendingG;
        _diffLines = pane.DiffLines;
        _diffBaseline = pane.DiffBaseline;
        _diffBaselinePath = pane.DiffBaselinePath;
        _diffRevision = pane.DiffRevision;
    }

    private static EditorPane ClonePane(EditorPane source)
    {
        var pane = new EditorPane
        {
            Document = source.Document,
            Language = source.Language,
            Syntax = new SyntaxHighlighter(),
            FileStamp = source.FileStamp,
            ExternalCheckPending = source.ExternalCheckPending,
            Mode = Mode.Normal,
            Cursor = source.Cursor,
            SelectionAnchor = source.SelectionAnchor,
            SelectionLinewise = source.SelectionLinewise,
            ExtraSelections = new List<Selection>(source.ExtraSelections),
            FindQuery = new StringBuilder(source.FindQuery.ToString()),
            FindQueryCursor = source.FindQueryCursor,
            FindScope = new List<(long Start, long End)>(source.FindScope),
            FindMatches = new List<FindMatch>(source.FindMatches),
            FindMatchIndex = source.FindMatchIndex,
            TopLine = source.TopLine,
            LeftColumn = source.LeftColumn,
            PreferredColumn = source.PreferredColumn,
            PreferredColumnValid = source.PreferredColumnValid,
            FreeScroll = source.FreeScroll,
            JumpHistory = new List<JumpLocation>(source.JumpHistory),
            JumpForward = new List<JumpLocation>(source.JumpForward),
            PendingG = false,
            DiffLines = new Dictionary<long, GitDiffLineKind>(source.DiffLines),
            DiffBaseline = source.DiffBaseline,
            DiffBaselinePath = source.DiffBaselinePath,
            DiffRevision = source.DiffRevision,
        };
        pane.Syntax.SetDocument(pane.Document, pane.Language);
        return pane;
    }

    private static PaneLeaf? FindPaneLeaf(PaneNode? node, EditorPane pane)
    {
        return node switch
        {
            PaneLeaf leaf when ReferenceEquals(leaf.Pane, pane) => leaf,
            PaneSplit split => FindPaneLeaf(split.First, pane) ?? FindPaneLeaf(split.Second, pane),
            _ => null,
        };
    }

    private static bool PaneNodeContains(PaneNode? node, EditorPane pane)
    {
        return node switch
        {
            PaneLeaf leaf => ReferenceEquals(leaf.Pane, pane),
            PaneSplit split => PaneNodeContains(split.First, pane)
                || PaneNodeContains(split.Second, pane),
            _ => false,
        };
    }

    private static void CollectPanes(PaneNode? node, List<EditorPane> panes)
    {
        switch (node)
        {
            case PaneLeaf leaf:
                panes.Add(leaf.Pane);
                break;
            case PaneSplit split:
                CollectPanes(split.First, panes);
                CollectPanes(split.Second, panes);
                break;
        }
    }

    private List<EditorPane> AllPanes()
    {
        var panes = new List<EditorPane>();
        CollectPanes(_paneRoot, panes);
        return panes;
    }

    private void ReplacePaneNode(PaneNode oldNode, PaneNode newNode)
    {
        PaneSplit? parent = oldNode.Parent;
        if (parent is null)
        {
            _paneRoot = newNode;
            newNode.Parent = null;
            return;
        }

        if (ReferenceEquals(parent.First, oldNode))
        {
            parent.First = newNode;
        }
        else
        {
            parent.Second = newNode;
        }

        newNode.Parent = parent;
    }

    private void SplitPane(PaneSplitOrientation orientation)
    {
        ComputePaneLayout();
        PaneBounds bounds = ActivePaneBounds();
        if (
            orientation == PaneSplitOrientation.Vertical
            && bounds.Width < MinPaneSplitWidth * 2 + 1
        )
        {
            SetError("Pane is too narrow to split vertically.");
            return;
        }

        if (
            orientation == PaneSplitOrientation.Horizontal
            && bounds.Height < MinPaneSplitHeight * 2 + 1
        )
        {
            SetError("Pane is too short to split horizontally.");
            return;
        }

        PaneLeaf? leaf = FindPaneLeaf(_paneRoot, _activePane);
        if (leaf is null)
        {
            SetError("Cannot split the active pane.");
            return;
        }

        CapturePaneState(_activePane);
        EditorPane created = ClonePane(_activePane);
        var createdLeaf = new PaneLeaf { Pane = created };
        var split = new PaneSplit
        {
            Orientation = orientation,
            Ratio = 0.5,
            First = leaf,
            Second = createdLeaf,
        };

        ReplacePaneNode(leaf, split);
        leaf.Parent = split;
        createdLeaf.Parent = split;

        _pendingG = false;
        SwitchToPane(created);
        _forceFullRedraw = true;
        SetMessage(
            orientation == PaneSplitOrientation.Vertical
                ? "Split pane vertically."
                : "Split pane horizontally."
        );
    }

    private void CloseCurrentBuffer()
    {
        if (AllPanes().Count <= 1)
        {
            SetMessage("Only one buffer is open; use :q or :q! to quit Hexal.");
            return;
        }

        CloseActivePane();
    }

    private void CloseActivePane()
    {
        PaneLeaf? leaf = FindPaneLeaf(_paneRoot, _activePane);
        if (leaf is null || leaf.Parent is null)
        {
            SetError("Cannot close the last pane.");
            return;
        }

        List<EditorPane> panes = AllPanes();
        if (panes.Count <= 1)
        {
            SetError("Cannot close the last pane.");
            return;
        }

        Document document = _document;
        bool displayedElsewhere = panes.Any(pane =>
            !ReferenceEquals(pane, _activePane) && ReferenceEquals(pane.Document, document)
        );
        if (document.IsDirty && !displayedElsewhere)
        {
            SetError("Save the buffer before closing its last pane.");
            return;
        }

        PaneSplit split = leaf.Parent;
        PaneNode sibling = ReferenceEquals(split.First, leaf) ? split.Second : split.First;

        ReplacePaneNode(split, sibling);
        split.Parent = null;
        leaf.Parent = null;

        EditorPane? target = null;
        foreach (EditorPane pane in AllPanes())
        {
            if (ReferenceEquals(pane.Document, document))
            {
                target = pane;
                break;
            }
        }

        target ??= AllPanes().FirstOrDefault();
        if (target is not null)
        {
            SwitchToPane(target);
        }

        _forceFullRedraw = true;
        SetMessage("Closed pane.");
    }

    private void SwitchToPane(EditorPane pane)
    {
        if (ReferenceEquals(pane, _activePane) || _paneRoot is null)
        {
            return;
        }

        Document previous = _document;
        CapturePaneState(_activePane);
        LoadPaneState(pane);
        _activePane = pane;
        _pendingWindow = false;
        _hoverVisible = false;
        _hoverLines.Clear();

        if (!ReferenceEquals(previous, _document))
        {
            ResetLsp();
            FirePluginEvent("buffer_opened", CurrentBufferData());
        }

        SnapCursor();
        EnsureCursorVisible();
    }

    private void FocusPaneDirection(PaneDirection direction)
    {
        ComputePaneLayout();
        PaneView? current = null;
        foreach (PaneView view in _paneViews)
        {
            if (ReferenceEquals(view.Pane, _activePane))
            {
                current = view;
                break;
            }
        }

        if (current is null)
        {
            return;
        }

        PaneView choice = current.Value;
        int bestOverlap = 0;
        int bestDistance = int.MaxValue;
        foreach (PaneView candidate in _paneViews)
        {
            if (ReferenceEquals(candidate.Pane, _activePane))
            {
                continue;
            }

            int overlap = Overlap(current.Value, candidate, direction);
            if (overlap <= 0)
            {
                continue;
            }

            int distance = Distance(current.Value, candidate, direction);
            if (distance < 0)
            {
                continue;
            }

            if (overlap > bestOverlap || (overlap == bestOverlap && distance < bestDistance))
            {
                choice = candidate;
                bestOverlap = overlap;
                bestDistance = distance;
            }
        }

        if (!ReferenceEquals(choice.Pane, _activePane))
        {
            SwitchToPane(choice.Pane);
            _forceFullRedraw = true;
            ClearMessage();
        }
    }

    private static int Overlap(PaneView current, PaneView candidate, PaneDirection direction)
    {
        return direction is PaneDirection.Left or PaneDirection.Right
            ? Math.Max(
                0,
                Math.Min(current.Y + current.Height, candidate.Y + candidate.Height)
                    - Math.Max(current.Y, candidate.Y)
            )
            : Math.Max(
                0,
                Math.Min(current.X + current.Width, candidate.X + candidate.Width)
                    - Math.Max(current.X, candidate.X)
            );
    }

    private static int Distance(PaneView current, PaneView candidate, PaneDirection direction)
    {
        return direction switch
        {
            PaneDirection.Left when candidate.X + candidate.Width <= current.X => current.X
                - (candidate.X + candidate.Width),
            PaneDirection.Right when candidate.X >= current.X + current.Width => candidate.X
                - (current.X + current.Width),
            PaneDirection.Up when candidate.Y + candidate.Height <= current.Y => current.Y
                - (candidate.Y + candidate.Height),
            PaneDirection.Down when candidate.Y >= current.Y + current.Height => candidate.Y
                - (current.Y + current.Height),
            _ => -1,
        };
    }

    private void ResizeActivePane(bool grow)
    {
        PaneLeaf? leaf = FindPaneLeaf(_paneRoot, _activePane);
        if (leaf?.Parent is not PaneSplit split)
        {
            SetMessage("Cannot resize the last pane.");
            return;
        }

        bool first = ReferenceEquals(split.First, leaf);
        double delta = first ? PaneResizeStep : -PaneResizeStep;
        if (!grow)
        {
            delta = -delta;
        }

        split.Ratio = Math.Clamp(split.Ratio + delta, 0.1, 0.9);
        _forceFullRedraw = true;
        SetMessage($"Pane ratio {split.Ratio:P0}.");
    }

    private void HandlePaneCommand(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            ClearMessage();
            return;
        }

        PaneDirection direction;
        if (inputEvent.Key == Key.Left)
        {
            direction = PaneDirection.Left;
        }
        else if (inputEvent.Key == Key.Right)
        {
            direction = PaneDirection.Right;
        }
        else if (inputEvent.Key == Key.Up)
        {
            direction = PaneDirection.Up;
        }
        else if (inputEvent.Key == Key.Down)
        {
            direction = PaneDirection.Down;
        }
        else if (inputEvent.Key == Key.Character && !inputEvent.Ctrl && !inputEvent.Alt)
        {
            char key = (char)inputEvent.Character.Value;
            switch (key)
            {
                case 'h':
                case 'H':
                    direction = PaneDirection.Left;
                    break;
                case 'l':
                case 'L':
                    direction = PaneDirection.Right;
                    break;
                case 'k':
                case 'K':
                    direction = PaneDirection.Up;
                    break;
                case 'j':
                case 'J':
                    direction = PaneDirection.Down;
                    break;
                case '+':
                case '=':
                    ResizeActivePane(grow: true);
                    return;
                case '-':
                case '_':
                    ResizeActivePane(grow: false);
                    return;
                case 'q':
                case 'Q':
                case 'x':
                case 'X':
                    CloseActivePane();
                    return;
                default:
                    ClearMessage();
                    return;
            }
        }
        else
        {
            return;
        }

        FocusPaneDirection(direction);
    }

    private PaneBounds ActivePaneBounds()
    {
        ComputePaneLayout();
        foreach (PaneView view in _paneViews)
        {
            if (ReferenceEquals(view.Pane, _activePane))
            {
                return new PaneBounds(view.X, view.Y, view.Width, view.Height);
            }
        }

        int width = Math.Max(1, _columns - _editorLeft);
        int height = Math.Max(0, TextRows());
        return new PaneBounds(_editorLeft, 0, width, height);
    }

    private int ActivePaneRows() => Math.Max(0, ActivePaneBounds().Height);

    private void ComputePaneLayout()
    {
        _paneViews.Clear();
        _paneDividers.Clear();
        if (_paneRoot is null)
        {
            return;
        }

        int width = _columns - _editorLeft;
        int height = TextRows();
        if (width <= 0 || height <= 0)
        {
            return;
        }

        LayoutPaneNode(_paneRoot, _editorLeft, 0, width, height);
    }

    private void LayoutPaneNode(PaneNode node, int x, int y, int width, int height)
    {
        if (width <= 0 || height <= 0)
        {
            return;
        }

        if (node is PaneLeaf leaf)
        {
            _paneViews.Add(new PaneView(leaf.Pane, x, y, width, height));
            return;
        }

        if (node is not PaneSplit split)
        {
            return;
        }

        bool vertical = split.Orientation == PaneSplitOrientation.Vertical;
        int span = vertical ? width : height;
        if (span < 3)
        {
            PaneNode visible = PaneNodeContains(split.First, _activePane)
                ? split.First
                : split.Second;
            LayoutPaneNode(visible, x, y, width, height);
            return;
        }

        int available = span - 1;
        int first = (int)Math.Round(available * split.Ratio);
        first = Math.Clamp(first, 1, available - 1);
        int second = available - first;

        if (vertical)
        {
            LayoutPaneNode(split.First, x, y, first, height);
            LayoutPaneNode(split.Second, x + first + 1, y, second, height);
            _paneDividers.Add(new PaneDivider(x + first, y, height, Vertical: true));
        }
        else
        {
            LayoutPaneNode(split.First, x, y, width, first);
            LayoutPaneNode(split.Second, x, y + first + 1, width, second);
            _paneDividers.Add(new PaneDivider(x, y + first, width, Vertical: false));
        }
    }

    private void RenderPanes()
    {
        if (_paneRoot is null)
        {
            return;
        }

        ComputePaneLayout();
        if (_paneViews.Count == 0)
        {
            return;
        }

        EditorPane active = _activePane;
        CapturePaneState(active);
        foreach (PaneView view in _paneViews)
        {
            LoadPaneState(view.Pane);
            UpdateDiffLinesForBufferChange();
            RenderPaneContent(view, ReferenceEquals(view.Pane, active));
            CapturePaneState(view.Pane);
        }

        LoadPaneState(active);
        DrawPaneDividers();
    }

    private void RenderPaneContent(PaneView view, bool focused)
    {
        if (view.Width <= 0 || view.Height <= 0)
        {
            return;
        }

        _paneScreen.Resize(view.Width, view.Height);
        _paneScreen.Clear(_theme.BaseStyle);

        int visibleRows = VisiblePaneRows(view.Y, view.Height, focused);

        Screen previousScreen = _screen;
        int previousColumns = _columns;
        int previousRows = _rows;
        int previousLeft = _editorLeft;

        _screen = _paneScreen;
        _columns = view.Width;
        _rows = view.Height;
        _editorLeft = 0;

        try
        {
            long total = _document.LineCount;
            long maxTop = total > visibleRows ? total - visibleRows : 0;
            if (_topLine > maxTop)
            {
                _topLine = maxTop;
            }

            int gutter = GutterWidth();
            RenderLineNumbers(_topLine, view.Height, gutter);
            RenderTextLines(_topLine, view.Height, gutter);
            RenderSelection(_topLine, view.Height, gutter);
            RenderFindMatches(_topLine, view.Height, gutter);
            if (focused)
            {
                RenderCursor(_topLine, view.Height, gutter);
            }
        }
        finally
        {
            _screen = previousScreen;
            _columns = previousColumns;
            _rows = previousRows;
            _editorLeft = previousLeft;
        }

        _screen.Blit(_paneScreen, view.X, view.Y);
    }

    private void DrawPaneDividers()
    {
        if (_paneViews.Count <= 1)
        {
            return;
        }

        foreach (PaneDivider divider in _paneDividers)
        {
            if (divider.Vertical)
            {
                for (int row = 0; row < divider.Length; ++row)
                {
                    _screen.Put(divider.X, divider.Y + row, '\u2502', _theme.BorderStyle);
                }
            }
            else
            {
                for (int column = 0; column < divider.Length; ++column)
                {
                    _screen.Put(divider.X + column, divider.Y, '\u2500', _theme.BorderStyle);
                }
            }
        }
    }

    private bool TryFindPaneAt(int x, int y, out EditorPane pane)
    {
        pane = null!;
        ComputePaneLayout();
        foreach (PaneView view in _paneViews)
        {
            if (x >= view.X && x < view.X + view.Width && y >= view.Y && y < view.Y + view.Height)
            {
                pane = view.Pane;
                return true;
            }
        }

        return false;
    }

    private int ActivePaneOrdinal()
    {
        ComputePaneLayout();
        for (int index = 0; index < _paneViews.Count; ++index)
        {
            if (ReferenceEquals(_paneViews[index].Pane, _activePane))
            {
                return index + 1;
            }
        }

        return 1;
    }

    private bool AnyPaneDocumentDirty()
    {
        var seen = new HashSet<Document>(ReferenceEqualityComparer.Instance);
        foreach (EditorPane pane in AllPanes())
        {
            Document document = ReferenceEquals(pane, _activePane) ? _document : pane.Document;
            if (document.IsDirty && seen.Add(document))
            {
                return true;
            }
        }

        return false;
    }
}
