using System.Text;
using Hexal.Lsp;
using Hexal.Platform;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private bool TryRequestLspCompletion(string? triggerCharacter)
    {
        if (
            !_lsp.IsRunning
            || !_lsp.IsInitialized
            || _document.Path is null
            || _extraSelections.Count > 0
        )
        {
            return false;
        }

        if (_lspDirty)
        {
            _lsp.DidChange(GetText());
            _lspDirty = false;
        }

        (int line, int character) = CursorLspPosition();
        if (!_lsp.RequestCompletion(_document.Path, line, character, triggerCharacter))
        {
            return false;
        }

        _lspCompletionPending = true;
        return true;
    }

    private void RequestLspCompletionManually()
    {
        if (!EnsureLspReady("completion"))
        {
            return;
        }

        if (TryRequestLspCompletion(null))
        {
            SetMessage("Requesting completions...");
        }
    }

    private void OpenLspCompletion(IReadOnlyList<LspCompletionItem> items)
    {
        _lspCompletionPending = false;
        if (items.Count == 0)
        {
            CloseLspCompletion();
            SetMessage("No completions available.");
            return;
        }

        _lspCompletionAllItems.Clear();
        _lspCompletionAllItems.AddRange(items);
        _lspCompletionFilter = string.Empty;
        ApplyLspCompletionFilter();
        _lspCompletionVisible = true;
        ClearMessage();
    }

    private void CloseLspCompletion()
    {
        _lspCompletionVisible = false;
        _lspCompletionPending = false;
        _lspCompletionAllItems.Clear();
        _lspCompletionItems.Clear();
        _lspCompletionFilter = string.Empty;
        _lspCompletionSelected = 0;
        _lspCompletionTop = 0;
    }

    private bool HandleLspCompletionInput(InputEvent inputEvent)
    {
        switch (inputEvent.Key)
        {
            case Key.Escape:
                CloseLspCompletion();
                return true;
            case Key.Up:
            case Key.ScrollUp:
                MoveLspCompletionSelection(-1);
                return true;
            case Key.Down:
            case Key.ScrollDown:
                MoveLspCompletionSelection(1);
                return true;
            case Key.PageUp:
                MoveLspCompletionSelection(-LspCompletionRows());
                return true;
            case Key.PageDown:
                MoveLspCompletionSelection(LspCompletionRows());
                return true;
            case Key.Home:
                _lspCompletionSelected = 0;
                EnsureLspCompletionSelectionVisible();
                return true;
            case Key.End:
                _lspCompletionSelected = Math.Max(0, _lspCompletionItems.Count - 1);
                EnsureLspCompletionSelectionVisible();
                return true;
            case Key.Enter:
            case Key.Tab:
                // With no matching item there is nothing to accept; let the
                // key through so Enter inserts a newline and Tab indents.
                if (_lspCompletionItems.Count == 0)
                {
                    CloseLspCompletion();
                    return false;
                }

                AcceptLspCompletion();
                return true;
            case Key.Backspace:
                if (_lspCompletionFilter.Length > 0)
                {
                    _lspCompletionFilter = RemoveLastRune(_lspCompletionFilter);
                    ApplyLspCompletionFilter();
                    // Let the backspace edit the buffer so the filter text and
                    // the document stay in sync.
                    return false;
                }

                CloseLspCompletion();
                return false;
            case Key.Character:
                if (
                    !inputEvent.Ctrl
                    && !inputEvent.Alt
                    && IsLspCompletionFilterCharacter(inputEvent.Character)
                )
                {
                    _lspCompletionFilter += inputEvent.Character.ToString();
                    ApplyLspCompletionFilter();
                    // Let the character be inserted after the list is filtered.
                    return false;
                }
                break;
        }

        CloseLspCompletion();
        return false;
    }

    private static string RemoveLastRune(string text)
    {
        int index = text.Length - 1;
        if (index > 0 && char.IsLowSurrogate(text[index]) && char.IsHighSurrogate(text[index - 1]))
        {
            --index;
        }

        return text[..index];
    }

    private static bool IsLspCompletionFilterCharacter(Rune rune)
    {
        if (Rune.IsLetterOrDigit(rune))
        {
            return true;
        }

        return rune.Value is '_' or '$';
    }

    private void ApplyLspCompletionFilter()
    {
        _lspCompletionItems.Clear();
        if (_lspCompletionFilter.Length == 0)
        {
            _lspCompletionItems.AddRange(_lspCompletionAllItems);
        }
        else
        {
            foreach (LspCompletionItem item in _lspCompletionAllItems)
            {
                string candidate = item.FilterText.Length > 0 ? item.FilterText : item.Label;
                if (candidate.StartsWith(_lspCompletionFilter, StringComparison.OrdinalIgnoreCase))
                {
                    _lspCompletionItems.Add(item);
                }
            }
        }

        _lspCompletionSelected = 0;
        _lspCompletionTop = 0;
    }

    private int LspCompletionRows() => Math.Min(10, Math.Max(1, _lspCompletionItems.Count));

    private void MoveLspCompletionSelection(int delta)
    {
        int count = _lspCompletionItems.Count;
        if (count == 0)
        {
            return;
        }

        _lspCompletionSelected = ((_lspCompletionSelected + delta) % count + count) % count;
        EnsureLspCompletionSelectionVisible();
    }

    private void EnsureLspCompletionSelectionVisible()
    {
        int rows = LspCompletionRows();
        if (_lspCompletionSelected < _lspCompletionTop)
        {
            _lspCompletionTop = _lspCompletionSelected;
        }
        else if (_lspCompletionSelected >= _lspCompletionTop + rows)
        {
            _lspCompletionTop = _lspCompletionSelected - rows + 1;
        }

        _lspCompletionTop = Math.Clamp(
            _lspCompletionTop,
            0,
            Math.Max(0, _lspCompletionItems.Count - rows)
        );
    }

    private long LspPositionToDocumentOffset(long line, int character)
    {
        if (line < 0)
        {
            return 0;
        }

        if (line >= _document.LineCount)
        {
            return _document.Size;
        }

        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = LspPositions.Utf16ColumnToByteOffset(
            _lineBuffer.AsSpan(0, length),
            character
        );
        return _document.LineStart(line) + local;
    }

    private void AcceptLspCompletion()
    {        if (
            !_lspCompletionVisible
            || _lspCompletionItems.Count == 0
            || _lspCompletionSelected < 0
            || _lspCompletionSelected >= _lspCompletionItems.Count
        )
        {
            return;
        }

        LspCompletionItem item = _lspCompletionItems[_lspCompletionSelected];
        long start = _cursor;
        long end = _cursor;
        if (item.HasTextEdit && _document.Path is not null)
        {
            start = LspPositionToDocumentOffset(item.StartLine, item.StartCharacter);
            end = LspPositionToDocumentOffset(item.EndLine, item.EndCharacter);
            if (end < start)
            {
                (start, end) = (end, start);
            }
            start = Math.Clamp(start, 0, _document.Size);
            end = Math.Clamp(end, start, _document.Size);

            // Typing after the completion request filters the list, but the
            // server's text edit still ends at the position from that request.
            // Extend the replacement over the characters typed since then.
            if (_lspCompletionFilter.Length > 0 && _cursor >= end)
            {
                end = _cursor;
            }
        }

        string text = item.InsertText;
        int cursorCharacters = text.Length;
        if (item.InsertTextFormat == 2)
        {
            text = ExpandSnippet(text, out cursorCharacters);
        }

        byte[] bytes = Encoding.UTF8.GetBytes(text);
        _document.BreakUndoGroup();
        _document.Replace(start, end - start, bytes);

        int safeCursor = Math.Clamp(cursorCharacters, 0, text.Length);
        _cursor = start + Encoding.UTF8.GetByteCount(text.AsSpan(0, safeCursor));
        _selectionAnchor = _cursor;
        _selectionLinewise = false;
        CloseLspCompletion();
        SnapCursor();
        _preferredColumnValid = false;
        _lspDirty = _lsp.IsRunning;
        _lspDirtySince = Environment.TickCount64;
        ClearMessage();
    }

    private static string ExpandSnippet(string snippet, out int cursorOffset)
    {
        var result = new StringBuilder(snippet.Length);
        cursorOffset = -1;
        int finalOffset = -1;
        int index = 0;

        while (index < snippet.Length)
        {
            char character = snippet[index];
            if (
                character == '\\'
                && index + 1 < snippet.Length
                && snippet[index + 1] is '$' or '}' or '\\'
            )
            {
                result.Append(snippet[index + 1]);
                index += 2;
                continue;
            }

            if (character != '$' || index + 1 >= snippet.Length)
            {
                result.Append(character);
                ++index;
                continue;
            }

            char next = snippet[index + 1];
            if (next is >= '0' and <= '9')
            {
                int end = index + 1;
                while (end < snippet.Length && snippet[end] is >= '0' and <= '9')
                {
                    ++end;
                }

                int number = int.TryParse(
                    snippet.AsSpan(index + 1, end - index - 1),
                    out int parsedNumber
                )
                    ? parsedNumber
                    : 0;
                if (number == 0)
                {
                    if (finalOffset < 0)
                    {
                        finalOffset = result.Length;
                    }
                }
                else if (cursorOffset < 0)
                {
                    cursorOffset = result.Length;
                }

                index = end;
                continue;
            }

            if (next != '{')
            {
                result.Append('$');
                ++index;
                continue;
            }

            int close = FindSnippetClosingBrace(snippet, index + 2);
            if (close < 0)
            {
                result.Append('$');
                ++index;
                continue;
            }

            string inner = snippet[(index + 2)..close];
            int digitsEnd = 0;
            while (digitsEnd < inner.Length && inner[digitsEnd] is >= '0' and <= '9')
            {
                ++digitsEnd;
            }

            if (digitsEnd > 0)
            {
                int number = int.TryParse(inner.AsSpan(0, digitsEnd), out int parsedNumber)
                    ? parsedNumber
                    : 0;
                string rest = inner[digitsEnd..];
                if (rest.StartsWith(':'))
                {
                    result.Append(rest[1..]);
                }
                else if (rest.StartsWith('|'))
                {
                    int choiceEnd = rest.IndexOf('|', 1);
                    string choices = choiceEnd > 1 ? rest[1..choiceEnd] : rest[1..];
                    int comma = choices.IndexOf(',');
                    result.Append(comma >= 0 ? choices[..comma] : choices);
                }

                if (number == 0)
                {
                    if (finalOffset < 0)
                    {
                        finalOffset = result.Length;
                    }
                }
                else if (cursorOffset < 0)
                {
                    cursorOffset = result.Length;
                }
            }

            index = close + 1;
        }

        if (cursorOffset < 0)
        {
            cursorOffset = finalOffset >= 0 ? finalOffset : result.Length;
        }

        return result.ToString();
    }

    private static int FindSnippetClosingBrace(string snippet, int start)
    {
        int depth = 1;
        for (int index = start; index < snippet.Length; ++index)
        {
            if (snippet[index] == '\\')
            {
                ++index;
                continue;
            }

            if (snippet[index] == '{')
            {
                ++depth;
            }
            else if (snippet[index] == '}')
            {
                --depth;
                if (depth == 0)
                {
                    return index;
                }
            }
        }

        return -1;
    }

    private void RenderLspCompletion()
    {
        if (!_lspCompletionVisible || _lspCompletionItems.Count == 0)
        {
            return;
        }

        int rows = LspCompletionRows();
        _lspCompletionSelected = Math.Clamp(
            _lspCompletionSelected,
            0,
            _lspCompletionItems.Count - 1
        );
        EnsureLspCompletionSelectionVisible();

        int width = 24;
        for (int row = 0; row < rows; ++row)
        {
            int index = _lspCompletionTop + row;
            if (index >= _lspCompletionItems.Count)
            {
                break;
            }
            width = Math.Max(width, LspCompletionDisplay(_lspCompletionItems[index]).Length + 4);
        }

        width = Math.Min(width, Math.Max(10, _columns - 4));
        int height = rows + 2;
        PaneBounds bounds = ActivePaneBounds();
        int available = (int)Math.Max(0, StatusRow());
        if (available < height + 1 || width < 10 || bounds.Width <= 0)
        {
            return;
        }

        long cursorLine = CursorLine();
        int gutter = GutterWidth();
        int cursorRow = bounds.Y + (int)(cursorLine - _topLine);
        int cursorColumn = bounds.X + gutter + (int)(CursorVisualColumn() - _leftColumn);

        int y = cursorRow + 1;
        if (y + height > available)
        {
            y = cursorRow - height;
        }
        y = Math.Clamp(y, bounds.Y, Math.Max(bounds.Y, available - height));

        int x = cursorColumn + 1;
        int maxX = _columns - width - 1;
        x = Math.Clamp(x, bounds.X, Math.Max(bounds.X, maxX));

        DrawBox(
            x,
            y,
            width,
            height,
            $" Completions {_lspCompletionSelected + 1}/{_lspCompletionItems.Count} "
        );

        var selectedStyle = _theme.SelectedItemStyle;
        var itemStyle = _theme.TextStyle;
        for (int row = 0; row < rows; ++row)
        {
            int index = _lspCompletionTop + row;
            if (index >= _lspCompletionItems.Count)
            {
                break;
            }

            int screenRow = y + 1 + row;
            Style style = index == _lspCompletionSelected ? selectedStyle : itemStyle;
            _screen.Fill(x + 1, screenRow, width - 2, 1, ' ', style);
            _screen.PutString(
                x + 2,
                screenRow,
                LspCompletionDisplay(_lspCompletionItems[index]),
                style,
                width - 4
            );
        }
    }

    private static string LspCompletionDisplay(LspCompletionItem item)
    {
        if (item.Detail.Length == 0)
        {
            return item.Label;
        }

        return $"{item.Label}  {item.Detail}";
    }
}
