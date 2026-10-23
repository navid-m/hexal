using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool _wordWrap;

    private readonly record struct WrapSegment(
        int StartByte,
        int LengthBytes,
        long VisualStart,
        long VisualWidth
    );

    private readonly record struct WrapRow(long Line, WrapSegment Segment, bool FirstSegment);

    private void ToggleWordWrap()
    {
        _wordWrap = !_wordWrap;
        if (_wordWrap)
        {
            _leftColumn = 0;
        }

        _forceFullRedraw = true;
        EnsureCursorVisible();

        if (HexalConfig.UpdateWordWrap(_config.SourcePath, _wordWrap, out string error))
        {
            SetMessage(_wordWrap ? "Word wrap enabled." : "Word wrap disabled.");
        }
        else
        {
            SetError($"config: {error}");
        }
    }

    private int RuneCellWidth(Rune rune, long visualColumn)
    {
        int cp = rune.Value;
        if (cp == '\t')
        {
            return TabWidth - (int)(visualColumn % TabWidth);
        }

        int width = UnicodeUtil.DisplayWidth((uint)cp);
        if (width == 0 && (cp < 0x20 || cp == 0x7F))
        {
            width = 1;
        }
        return width;
    }

    private List<WrapSegment> WrapSegmentsForLine(long line, int width)
    {
        var segments = new List<WrapSegment>();
        width = Math.Max(1, width);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        if (length == 0)
        {
            segments.Add(new WrapSegment(0, 0, 0, 0));
            return segments;
        }

        ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
        int offset = 0;
        int segmentStart = 0;
        int lastBreak = -1;
        long visual = 0;
        long segmentVisualStart = 0;
        long lastBreakVisual = 0;

        while (offset < length)
        {
            int runeStart = offset;
            UnicodeUtil.Decode(text, offset, out Rune rune, out int byteLength);
            offset += byteLength;
            int cellWidth = RuneCellWidth(rune, visual);
            bool isSpace = rune.Value is ' ' or '\t';

            if (visual + cellWidth - segmentVisualStart > width && runeStart > segmentStart)
            {
                int breakAt = lastBreak > segmentStart ? lastBreak : runeStart;
                long endVisual = lastBreak > segmentStart ? lastBreakVisual : visual;
                segments.Add(
                    new WrapSegment(
                        segmentStart,
                        breakAt - segmentStart,
                        segmentVisualStart,
                        endVisual - segmentVisualStart
                    )
                );
                segmentStart = breakAt;
                segmentVisualStart = endVisual;
                lastBreak = -1;
                offset = breakAt;
                visual = endVisual;
                continue;
            }

            visual += cellWidth;
            if (isSpace)
            {
                lastBreak = offset;
                lastBreakVisual = visual;
            }
        }

        segments.Add(
            new WrapSegment(
                segmentStart,
                length - segmentStart,
                segmentVisualStart,
                visual - segmentVisualStart
            )
        );
        return segments;
    }

    private List<WrapRow> BuildWrapRows(long firstLine, int rows, int width)
    {
        var result = new List<WrapRow>();
        long total = _document.LineCount;
        for (long line = firstLine; line < total && result.Count < rows; ++line)
        {
            List<WrapSegment> segments = WrapSegmentsForLine(line, width);
            for (int index = 0; index < segments.Count && result.Count < rows; ++index)
            {
                result.Add(new WrapRow(line, segments[index], index == 0));
            }
        }
        return result;
    }

    private static int WrapSegmentIndexForVisual(
        IReadOnlyList<WrapSegment> segments,
        long visualColumn
    )
    {
        for (int index = 0; index < segments.Count; ++index)
        {
            WrapSegment segment = segments[index];
            if (
                visualColumn < segment.VisualStart + segment.VisualWidth
                || index == segments.Count - 1
            )
            {
                return index;
            }
        }
        return 0;
    }

    private int WrapRowIndexInViewport(
        long firstLine,
        long targetLine,
        long targetVisual,
        int width,
        out int segmentIndex
    )
    {
        int rowIndex = 0;
        long total = _document.LineCount;
        for (long line = firstLine; line < targetLine && line < total; ++line)
        {
            rowIndex += WrapSegmentsForLine(line, width).Count;
        }

        List<WrapSegment> segments = WrapSegmentsForLine(targetLine, width);
        segmentIndex = WrapSegmentIndexForVisual(segments, targetVisual);
        return rowIndex + segmentIndex;
    }

    private void EnsureWrappedCursorVisible(long line, int rows, int paneWidth)
    {
        int gutter = GutterWidthFor(paneWidth);
        int textWidth = Math.Max(1, paneWidth - gutter);
        long column = CursorVisualColumn();

        if (line < _topLine)
        {
            _topLine = line;
        }
        else
        {
            int cursorRow = WrapRowIndexInViewport(
                _topLine,
                line,
                column,
                textWidth,
                out int segmentIndex
            );
            if (cursorRow >= rows)
            {
                long candidate = line;
                int rowsIncluded = segmentIndex + 1;
                while (candidate > 0 && rowsIncluded < rows)
                {
                    --candidate;
                    rowsIncluded += WrapSegmentsForLine(candidate, textWidth).Count;
                }
                _topLine = candidate;
            }
        }

        long maxTop = Math.Max(0, _document.LineCount - 1);
        _topLine = Math.Clamp(_topLine, 0, maxTop);
        _leftColumn = 0;
    }

    private bool TryOffsetAtScreenWrapped(
        int localX,
        int localY,
        int paneWidth,
        int paneHeight,
        out long offset
    )
    {
        offset = 0;
        int gutter = GutterWidthFor(paneWidth);
        int textWidth = Math.Max(1, paneWidth - gutter);
        List<WrapRow> rows = BuildWrapRows(_topLine, paneHeight, textWidth);
        if (localY < 0)
        {
            return false;
        }

        if (localY >= rows.Count)
        {
            // Clicking the empty space below the last wrapped row moves the
            // cursor to the end of the document instead of doing nothing.
            offset = _document.Size;
            return true;
        }

        WrapRow row = rows[localY];
        int length = _document.CopyLineText(row.Line, ref _lineBuffer, out _);
        long visual =
            localX > gutter ? row.Segment.VisualStart + (localX - gutter) : row.Segment.VisualStart;
        int local = UnicodeUtil.ByteOffsetForVisualColumn(
            _lineBuffer.AsSpan(0, length),
            visual,
            TabWidth
        );
        long start = _document.LineStart(row.Line);
        long end = _document.LineEnd(row.Line);
        offset = Math.Clamp(start + local, start, end);
        return true;
    }

    private void RenderWrappedLineNumbers(long firstLine, int rows, int gutter)
    {
        if (gutter <= 0)
        {
            return;
        }

        int textWidth = Math.Max(1, _columns - _editorLeft - gutter);
        List<WrapRow> wrapRows = BuildWrapRows(firstLine, rows, textWidth);
        int markerWidth = EditorMarkerWidth();
        int digits = gutter - 1 - markerWidth;
        long current = CursorLine();
        long total = _document.LineCount;

        for (int row = 0; row < wrapRows.Count; ++row)
        {
            WrapRow wrapped = wrapRows[row];
            if (wrapped.Line >= total)
            {
                break;
            }

            Style? highlight = LineHighlightStyle(wrapped.Line);
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

            if (!wrapped.FirstSegment)
            {
                continue;
            }

            if (
                markerWidth > 0
                && TryGetDiffMarker(wrapped.Line, out char marker, out Style markerStyle)
            )
            {
                if (highlight is { } markerHighlight)
                {
                    markerStyle.Bg = markerHighlight.Bg;
                }
                _screen.Put(_editorLeft, row, marker, markerStyle);
            }

            long number = wrapped.Line + 1;
            int x = digits - DigitCount(number);
            if (x < 0)
            {
                x = 0;
            }

            Style style =
                wrapped.Line == current ? _theme.CurrentLineNumberStyle : _theme.MutedStyle;
            if (highlight is { } highlightStyle)
            {
                style.Bg = highlightStyle.Bg;
            }

            _screen.PutInt(_editorLeft + markerWidth + x, row, number, style, digits);
        }
    }

    private void RenderWrappedTextLines(long firstLine, int rows, int gutter)
    {
        if (rows == 0 || _columns <= _editorLeft + gutter)
        {
            return;
        }

        int textWidth = _columns - _editorLeft - gutter;
        List<WrapRow> wrapRows = BuildWrapRows(firstLine, rows, textWidth);
        for (int row = 0; row < wrapRows.Count; ++row)
        {
            WrapRow wrapped = wrapRows[row];
            int length = _document.CopyLineText(wrapped.Line, ref _lineBuffer, out _);
            ReadOnlySpan<byte> text = _lineBuffer.AsSpan(0, length);
            _syntax.GetLineTokens(wrapped.Line, text, _lineTokens);
            Style? lineHighlight = LineHighlightStyle(wrapped.Line);
            if (lineHighlight is { } fillStyle)
            {
                _screen.Fill(_editorLeft + gutter, row, textWidth, 1, ' ', fillStyle);
            }

            int start = Math.Clamp(wrapped.Segment.StartByte, 0, length);
            int end = Math.Clamp(
                wrapped.Segment.StartByte + wrapped.Segment.LengthBytes,
                start,
                length
            );
            int tokenIndex = 0;
            while (tokenIndex < _lineTokens.Count && _lineTokens[tokenIndex].End <= start)
            {
                ++tokenIndex;
            }

            int offset = start;
            long visual = wrapped.Segment.VisualStart;
            while (offset < end)
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
                int width = RuneCellWidth(rune, visual);
                if (width == 0)
                {
                    if (cp < 0x20 || cp == 0x7F)
                    {
                        cp = 0xFFFD;
                        width = 1;
                    }
                    else
                    {
                        continue;
                    }
                }

                long screenX = _editorLeft + gutter + (visual - wrapped.Segment.VisualStart);
                int available = _columns - (int)screenX;
                if (available > 0)
                {
                    if (cp == '\t')
                    {
                        int tabWidth = Math.Min(width, available);
                        for (int i = 0; i < tabWidth; ++i)
                        {
                            _screen.Put((int)screenX + i, row, ' ', textStyle);
                        }
                    }
                    else
                    {
                        int drawn = Math.Min(width, available);
                        _screen.Put((int)screenX, row, cp, textStyle, drawn);
                    }
                }

                visual += width;
            }
        }
    }

    private void RenderWrappedSelectionRange(
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
        int textWidth = Math.Max(1, _columns - _editorLeft - gutter);
        List<WrapRow> wrapRows = BuildWrapRows(firstLine, rows, textWidth);

        for (int row = 0; row < wrapRows.Count; ++row)
        {
            WrapRow wrapped = wrapRows[row];
            long line = wrapped.Line;
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
                if (
                    wrapped.FirstSegment
                    && selectionStart == lineStart
                    && selectionEnd > lineStart
                    && lineEnd == lineStart
                )
                {
                    if (_editorLeft + gutter < _columns)
                    {
                        _screen.ApplyStyle(_editorLeft + gutter, row, style);
                    }
                }
                continue;
            }

            int length = _document.CopyLineText(line, ref _lineBuffer, out _);
            long segmentStart = lineStart + wrapped.Segment.StartByte;
            long segmentEnd = Math.Min(lineEnd, segmentStart + wrapped.Segment.LengthBytes);
            long fromSegment = Math.Max(from, segmentStart);
            long toSegment = Math.Min(to, segmentEnd);
            if (toSegment <= fromSegment)
            {
                continue;
            }

            int localFrom = (int)Math.Clamp(fromSegment - lineStart, 0, length);
            int localTo = (int)Math.Clamp(toSegment - lineStart, 0, length);
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
                _editorLeft + gutter + visualFrom - wrapped.Segment.VisualStart
            );
            long lastX = Math.Min(
                _columns,
                _editorLeft + gutter + visualTo - wrapped.Segment.VisualStart
            );
            for (long x = firstX; x < lastX; ++x)
            {
                _screen.ApplyStyle((int)x, row, style);
            }
        }
    }

    private void RenderWrappedFindMatches(long firstLine, int rows, int gutter)
    {
        if (!_findActive || _findMatches.Count == 0 || rows <= 0)
        {
            return;
        }

        var matchStyle = _theme.MatchStyle;
        var currentStyle = _theme.CurrentMatchStyle;
        int textWidth = Math.Max(1, _columns - _editorLeft - gutter);
        List<WrapRow> wrapRows = BuildWrapRows(firstLine, rows, textWidth);

        for (int matchIndex = 0; matchIndex < _findMatches.Count; ++matchIndex)
        {
            FindMatch match = _findMatches[matchIndex];
            for (int row = 0; row < wrapRows.Count; ++row)
            {
                WrapRow wrapped = wrapRows[row];
                if (wrapped.Line != match.Line)
                {
                    continue;
                }

                long lineStart = _document.LineStart(match.Line);
                int length = _document.CopyLineText(match.Line, ref _lineBuffer, out _);
                long segmentStart = lineStart + wrapped.Segment.StartByte;
                long segmentEnd = segmentStart + wrapped.Segment.LengthBytes;
                long from = Math.Max(match.Start, segmentStart);
                long to = Math.Min(match.End, segmentEnd);
                if (to <= from)
                {
                    continue;
                }

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
                    _editorLeft + gutter + visualFrom - wrapped.Segment.VisualStart
                );
                long lastX = Math.Min(
                    _columns,
                    _editorLeft + gutter + visualTo - wrapped.Segment.VisualStart
                );

                Style style = matchIndex == _findMatchIndex ? currentStyle : matchStyle;
                for (long x = firstX; x < lastX; ++x)
                {
                    _screen.ApplyStyle((int)x, row, style);
                }
            }
        }
    }

    private void RenderWrappedCursorAt(long cursor, long firstLine, int rows, int gutter)
    {
        long line = _document.LineOfOffset(cursor);
        if (line < firstLine)
        {
            return;
        }

        long column = VisualColumnOfOffset(cursor);
        int textWidth = Math.Max(1, _columns - _editorLeft - gutter);
        List<WrapRow> wrapRows = BuildWrapRows(firstLine, rows, textWidth);
        int candidate = -1;
        for (int row = 0; row < wrapRows.Count; ++row)
        {
            WrapRow wrapped = wrapRows[row];
            if (wrapped.Line != line)
            {
                continue;
            }

            candidate = row;
            long segmentEnd = wrapped.Segment.VisualStart + wrapped.Segment.VisualWidth;
            if (column < segmentEnd)
            {
                long x = _editorLeft + gutter + (column - wrapped.Segment.VisualStart);
                if (x >= _editorLeft + gutter && x < _columns)
                {
                    Style style = _screen.At((int)x, row).Style;
                    style.Reverse = true;
                    _screen.ApplyStyle((int)x, row, style);
                }
                return;
            }
        }

        if (candidate >= 0)
        {
            WrapRow wrapped = wrapRows[candidate];
            long x = _editorLeft + gutter + (column - wrapped.Segment.VisualStart);
            x = Math.Clamp(x, _editorLeft + gutter, _columns - 1);
            Style style = _screen.At((int)x, candidate).Style;
            style.Reverse = true;
            _screen.ApplyStyle((int)x, candidate, style);
        }
    }
}
