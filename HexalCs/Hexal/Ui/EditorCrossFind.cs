using System.Text;
using System.Text.RegularExpressions;
using Hexal.Core;
using Hexal.Platform;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private const int CrossFindDialogWidth = 80;
    private const int CrossFindDialogHeight = 7;

    private bool _crossFindActive;
    private bool _crossFindRegex;
    private int _crossFindField;
    private readonly StringBuilder _crossFindPattern = new();
    private int _crossFindPatternCursor;
    private readonly StringBuilder _crossFindQuery = new();
    private int _crossFindQueryCursor;
    private readonly StringBuilder _crossFindReplacement = new();
    private int _crossFindReplacementCursor;
    private int _crossFindMatchedFiles;
    private string _crossFindPatternError = string.Empty;
    private string _crossFindQueryError = string.Empty;

    private void OpenCrossFind(bool regex)
    {
        if (_crossFindActive)
        {
            CloseCrossFind();
        }

        _crossFindActive = true;
        _crossFindRegex = regex;
        _crossFindField = 0;
        _crossFindPattern.Clear();
        _crossFindPattern.Append('*');
        _crossFindPatternCursor = 1;
        _crossFindQuery.Clear();
        _crossFindQueryCursor = 0;
        _crossFindReplacement.Clear();
        _crossFindReplacementCursor = 0;
        _crossFindPatternError = string.Empty;
        _crossFindQueryError = string.Empty;
        _crossFindMatchedFiles = 0;
        _pendingG = false;
        _document.BreakUndoGroup();
        RefreshCrossFindFileCount();
        ClearMessage();
    }

    private void CloseCrossFind()
    {
        _crossFindActive = false;
        _crossFindRegex = false;
        _crossFindField = 0;
        _crossFindPattern.Clear();
        _crossFindPatternCursor = 0;
        _crossFindQuery.Clear();
        _crossFindQueryCursor = 0;
        _crossFindReplacement.Clear();
        _crossFindReplacementCursor = 0;
        _crossFindMatchedFiles = 0;
        _crossFindPatternError = string.Empty;
        _crossFindQueryError = string.Empty;
        _document.BreakUndoGroup();
    }

    private StringBuilder CrossFindBuffer() =>
        _crossFindField switch
        {
            0 => _crossFindPattern,
            1 => _crossFindQuery,
            _ => _crossFindReplacement,
        };

    private int CrossFindCursor()
    {
        return _crossFindField switch
        {
            0 => _crossFindPatternCursor,
            1 => _crossFindQueryCursor,
            _ => _crossFindReplacementCursor,
        };
    }

    private void SetCrossFindCursor(int value)
    {
        switch (_crossFindField)
        {
            case 0:
                _crossFindPatternCursor = value;
                break;
            case 1:
                _crossFindQueryCursor = value;
                break;
            default:
                _crossFindReplacementCursor = value;
                break;
        }
    }

    private void HandleCrossFind(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            CloseCrossFind();
            return;
        }

        if (inputEvent.Key is Key.Tab or Key.BackTab)
        {
            int direction = inputEvent.Key == Key.Tab ? 1 : -1;
            _crossFindField = ((_crossFindField + direction) % 3 + 3) % 3;
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Up:
                _crossFindField = (_crossFindField + 2) % 3;
                return;
            case Key.Down:
                _crossFindField = (_crossFindField + 1) % 3;
                return;
        }

        if (inputEvent.Key == Key.Enter)
        {
            if (_crossFindField < 2)
            {
                ++_crossFindField;
                return;
            }

            ApplyCrossFind();
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Backspace:
            {
                StringBuilder buffer = CrossFindBuffer();
                int cursor = CrossFindCursor();
                if (cursor > 0)
                {
                    string text = buffer.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, cursor);
                    buffer.Remove(previous, cursor - previous);
                    SetCrossFindCursor(previous);
                    RefreshCrossFindValidation();
                }
                return;
            }
            case Key.Delete:
            {
                StringBuilder buffer = CrossFindBuffer();
                int cursor = CrossFindCursor();
                if (cursor < buffer.Length)
                {
                    string text = buffer.ToString();
                    int next = UnicodeUtil.NextBoundary(text, cursor);
                    buffer.Remove(cursor, next - cursor);
                    RefreshCrossFindValidation();
                }
                return;
            }
            case Key.Left:
                SetCrossFindCursor(
                    UnicodeUtil.PreviousBoundary(CrossFindBuffer().ToString(), CrossFindCursor())
                );
                return;
            case Key.Right:
                SetCrossFindCursor(
                    UnicodeUtil.NextBoundary(CrossFindBuffer().ToString(), CrossFindCursor())
                );
                return;
            case Key.Home:
                SetCrossFindCursor(0);
                return;
            case Key.End:
                SetCrossFindCursor(CrossFindBuffer().Length);
                return;
        }

        if (inputEvent.Key != Key.Character)
        {
            return;
        }

        uint codePoint = (uint)inputEvent.Character.Value;
        if (inputEvent.Ctrl)
        {
            if (codePoint is 'a' or 'A')
            {
                ApplyCrossFind();
            }
            return;
        }

        if (inputEvent.Alt || codePoint < 0x20 || codePoint == 0x7F)
        {
            return;
        }

        string inserted = inputEvent.Character.ToString();
        StringBuilder field = CrossFindBuffer();
        int fieldCursor = CrossFindCursor();
        field.Insert(fieldCursor, inserted);
        SetCrossFindCursor(fieldCursor + inserted.Length);
        RefreshCrossFindValidation();
    }

    private void RefreshCrossFindValidation()
    {
        _crossFindQueryError = string.Empty;
        if (_crossFindRegex && _crossFindQuery.Length > 0)
        {
            try
            {
                _ = CreateCrossFindRegex(_crossFindQuery.ToString());
            }
            catch (ArgumentException ex)
            {
                _crossFindQueryError = ex.Message;
            }
        }

        if (_crossFindField == 0)
        {
            RefreshCrossFindFileCount();
        }
    }

    private Regex CreateCrossFindRegex(string pattern)
    {
        RegexOptions options = RegexOptions.CultureInvariant;
        if (!_searchMatchCase)
        {
            options |= RegexOptions.IgnoreCase;
        }

        return new Regex(pattern, options, TimeSpan.FromMilliseconds(100));
    }

    private static Regex BuildCrossFindGlob(
        string pattern,
        out bool basenameOnly,
        out string error
    )
    {
        error = string.Empty;
        string value = pattern.Trim();
        if (value.Length == 0)
        {
            value = "*";
        }

        value = value.Replace('\\', '/');
        while (value.StartsWith("./", StringComparison.Ordinal))
        {
            value = value[2..];
        }

        basenameOnly = !value.Contains('/');
        var builder = new StringBuilder("^");
        for (int index = 0; index < value.Length; ++index)
        {
            char character = value[index];
            if (character == '*')
            {
                // Wildcards are recursive here, matching the documented
                // behaviour that "*" covers every subdirectory.
                if (index + 1 < value.Length && value[index + 1] == '*')
                {
                    ++index;
                }
                builder.Append(".*");
            }
            else if (character == '?')
            {
                builder.Append('.');
            }
            else
            {
                builder.Append(Regex.Escape(character.ToString()));
            }
        }
        builder.Append('$');

        RegexOptions options = RegexOptions.CultureInvariant;
        if (OperatingSystem.IsWindows() || OperatingSystem.IsMacOS())
        {
            options |= RegexOptions.IgnoreCase;
        }

        try
        {
            return new Regex(builder.ToString(), options, TimeSpan.FromMilliseconds(100));
        }
        catch (ArgumentException ex)
        {
            error = ex.Message;
            return new Regex("$^", options, TimeSpan.FromMilliseconds(100));
        }
    }

    private List<string> CrossFindFiles(string pattern, out string error)
    {
        Regex glob = BuildCrossFindGlob(pattern, out bool basenameOnly, out error);
        var files = new List<string>();
        if (error.Length > 0)
        {
            return files;
        }

        string root = ProjectRoot();
        foreach (string relative in CompletionFiles())
        {
            string normalized = relative.Replace('\\', '/');
            string candidate = basenameOnly
                ? normalized[(normalized.LastIndexOf('/') + 1)..]
                : normalized;
            if (!glob.IsMatch(candidate))
            {
                continue;
            }

            files.Add(Path.Combine(root, relative));
        }

        return files;
    }

    private void RefreshCrossFindFileCount()
    {
        _crossFindMatchedFiles = 0;
        _crossFindPatternError = string.Empty;
        List<string> files = CrossFindFiles(_crossFindPattern.ToString(), out string error);
        if (error.Length > 0)
        {
            _crossFindPatternError = $"pattern: {error}";
            return;
        }

        _crossFindMatchedFiles = files.Count;
    }

    private string CrossFindError =>
        _crossFindPatternError.Length > 0 ? _crossFindPatternError : _crossFindQueryError;

    private void ApplyCrossFind()
    {
        string pattern = _crossFindPattern.ToString();
        string query = _crossFindQuery.ToString();
        string replacement = _crossFindReplacement.ToString();
        if (query.Length == 0)
        {
            _crossFindQueryError = "find text is required";
            return;
        }

        Regex? regex = null;
        if (_crossFindRegex)
        {
            try
            {
                regex = CreateCrossFindRegex(query);
                _ = regex.Replace(string.Empty, replacement);
            }
            catch (ArgumentException ex)
            {
                _crossFindQueryError = $"invalid replacement: {ex.Message}";
                return;
            }
        }

        List<string> files = CrossFindFiles(pattern, out string globError);
        if (globError.Length > 0)
        {
            _crossFindPatternError = $"pattern: {globError}";
            return;
        }
        if (files.Count == 0)
        {
            _crossFindPatternError = "no files match the pattern";
            return;
        }

        var openDocuments = new Dictionary<string, Document>(PathComparer());
        foreach (EditorPane pane in AllPanes())
        {
            Document document = ReferenceEquals(pane, _activePane) ? _document : pane.Document;
            if (document.Path is null)
            {
                continue;
            }
            try
            {
                openDocuments[Path.GetFullPath(document.Path)] = document;
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException) { }
        }

        int replaced = 0;
        int changedFiles = 0;
        int failed = 0;
        int skippedOpen = 0;
        string firstError = string.Empty;
        bool activeChanged = false;

        foreach (string rawPath in files)
        {
            string path;
            try
            {
                path = Path.GetFullPath(rawPath);
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                ++failed;
                firstError ??= ex.Message;
                continue;
            }

            Document document;
            bool isActive;
            bool isOpen;
            if (openDocuments.TryGetValue(path, out Document? openDocument))
            {
                isOpen = true;
                isActive = ReferenceEquals(openDocument, _document);
                if (!isActive)
                {
                    ++skippedOpen;
                    continue;
                }
                document = openDocument;
            }
            else
            {
                isOpen = false;
                isActive = false;
                Document? loaded = Document.Open(path, out string openError);
                if (loaded is null)
                {
                    ++failed;
                    firstError ??= $"{Path.GetFileName(path)}: {openError}";
                    continue;
                }
                document = loaded;
            }

            List<FindMatch> matches = ScanCrossFindMatches(
                document,
                query,
                regex,
                replacement,
                _searchMatchCase,
                MaxFindMatches,
                out string scanError
            );
            if (scanError.Length > 0)
            {
                ++failed;
                firstError ??= $"{Path.GetFileName(path)}: {scanError}";
                continue;
            }
            if (matches.Count == 0)
            {
                continue;
            }

            ApplyCrossFindMatches(document, matches);
            replaced += matches.Count;
            ++changedFiles;
            if (isActive)
            {
                activeChanged = true;
            }
            else if (isOpen)
            {
                // Should not happen, but keep the branch explicit.
                ++skippedOpen;
            }
            else if (!document.Save(out string saveError))
            {
                ++failed;
                firstError ??= $"{Path.GetFileName(path)}: {saveError}";
            }
        }

        CloseCrossFind();

        if (activeChanged)
        {
            _cursor = Math.Clamp(_cursor, 0, _document.Size);
            _selectionAnchor = _cursor;
            _selectionLinewise = false;
            _extraSelections.Clear();
            _preferredColumnValid = false;
            SnapCursor();
            RefreshDiffLines();
            EnsureCursorVisible();
        }

        if (failed > 0)
        {
            string detail = firstError.Length > 0 ? $": {firstError}" : string.Empty;
            SetError(
                $"Replaced {replaced} match{(replaced == 1 ? string.Empty : "es")} "
                    + $"in {changedFiles} file{(changedFiles == 1 ? string.Empty : "s")}, "
                    + $"{failed} failed{detail}"
            );
            return;
        }

        string summary =
            $"Replaced {replaced} match{(replaced == 1 ? string.Empty : "es")} "
            + $"in {changedFiles} file{(changedFiles == 1 ? string.Empty : "s")}";
        if (skippedOpen > 0)
        {
            summary += $", {skippedOpen} open buffer{(skippedOpen == 1 ? string.Empty : "s")} skipped";
        }
        SetMessage(summary);
    }

    private static List<FindMatch> ScanCrossFindMatches(
        Document document,
        string query,
        Regex? regex,
        string replacementPattern,
        bool matchCase,
        int maxMatches,
        out string error
    )
    {
        error = string.Empty;
        var matches = new List<FindMatch>();
        if (query.Length == 0)
        {
            return matches;
        }

        byte[] lineBuffer = Array.Empty<byte>();
        long total = document.LineCount;
        for (long line = 0; line < total; ++line)
        {
            int length = document.CopyLineText(line, ref lineBuffer, out _);
            string text = Encoding.UTF8.GetString(lineBuffer, 0, length);
            long lineStart = document.LineStart(line);

            if (regex is not null)
            {
                try
                {
                    for (Match match = regex.Match(text); match.Success; match = match.NextMatch())
                    {
                        long start =
                            lineStart + Encoding.UTF8.GetByteCount(text.AsSpan(0, match.Index));
                        long end =
                            start
                            + Encoding.UTF8.GetByteCount(
                                text.AsSpan(match.Index, match.Length)
                            );

                        string replacement;
                        try
                        {
                            replacement = match.Result(replacementPattern);
                        }
                        catch (ArgumentException ex)
                        {
                            error = $"invalid replacement: {ex.Message}";
                            matches.Clear();
                            return matches;
                        }

                        matches.Add(new FindMatch(start, end, line, replacement));
                        if (matches.Count >= maxMatches)
                        {
                            return matches;
                        }
                    }
                }
                catch (RegexMatchTimeoutException)
                {
                    error = "regular expression timed out";
                    matches.Clear();
                    return matches;
                }
                continue;
            }

            if (length == 0)
            {
                continue;
            }

            StringComparison comparison = matchCase
                ? StringComparison.Ordinal
                : StringComparison.OrdinalIgnoreCase;
            int searchStart = 0;
            while (searchStart < text.Length)
            {
                int index = text.IndexOf(query, searchStart, comparison);
                if (index < 0)
                {
                    break;
                }

                int matchLength = Math.Min(query.Length, text.Length - index);
                long start = lineStart + Encoding.UTF8.GetByteCount(text.AsSpan(0, index));
                long end = start + Encoding.UTF8.GetByteCount(text.AsSpan(index, matchLength));
                matches.Add(new FindMatch(start, end, line, replacementPattern));
                if (matches.Count >= maxMatches)
                {
                    return matches;
                }

                searchStart = index + Math.Max(1, matchLength);
            }
        }

        return matches;
    }

    private static void ApplyCrossFindMatches(Document document, List<FindMatch> matches)
    {
        document.BeginCompositeEdit();
        for (int index = matches.Count - 1; index >= 0; --index)
        {
            FindMatch match = matches[index];
            document.Replace(
                match.Start,
                match.End - match.Start,
                Encoding.UTF8.GetBytes(match.Replacement)
            );
        }
        document.EndCompositeEdit();
        document.BreakUndoGroup();
    }

    private static StringComparer PathComparer() =>
        OperatingSystem.IsWindows() || OperatingSystem.IsMacOS()
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;

    private void RenderCrossFind()
    {
        if (!_crossFindActive)
        {
            return;
        }

        int available = (int)Math.Max(0, StatusRow());
        int width = Math.Min(_columns - 2, CrossFindDialogWidth);
        int height = CrossFindDialogHeight;
        if (available < height + 1 || width < 24)
        {
            return;
        }

        int x = Math.Max(0, (_columns - width) / 2);
        int y = Math.Max(0, available - height);
        string title = _crossFindRegex
            ? " Regex replace across files "
            : " Replace across files ";
        DrawBox(x, y, width, height, title);

        const int labelWidth = 9;
        int fieldX = x + 2 + labelWidth;
        int fieldWidth = Math.Max(0, width - labelWidth - 4);
        var promptStyle = _theme.AccentStyle;
        var mutedStyle = _theme.MutedStyle;
        var queryStyle = _theme.TextStyle;

        string[] labels = { "Files", "Find", "Replace" };
        StringBuilder[] buffers = { _crossFindPattern, _crossFindQuery, _crossFindReplacement };
        int[] cursors = { _crossFindPatternCursor, _crossFindQueryCursor, _crossFindReplacementCursor };
        for (int field = 0; field < labels.Length; ++field)
        {
            bool active = field == _crossFindField;
            Style labelStyle = active ? promptStyle : mutedStyle;
            _screen.PutString(x + 2, y + 1 + field, labels[field], labelStyle, labelWidth);
            string text = buffers[field].ToString();
            _screen.PutString(fieldX, y + 1 + field, text, queryStyle, fieldWidth);
            if (active)
            {
                RenderFindCursor(fieldX, y + 1 + field, text, cursors[field], fieldWidth);
            }
        }

        string error = CrossFindError;
        string status;
        var statusStyle = _theme.AccentStyle;
        if (error.Length > 0)
        {
            status = error;
            statusStyle = _theme.ErrorStyle;
        }
        else if (_crossFindQuery.Length == 0)
        {
            string verb = _crossFindMatchedFiles == 1 ? "matches" : "match";
            status =
                $"{_crossFindMatchedFiles} file{(_crossFindMatchedFiles == 1 ? string.Empty : "s")} {verb} the pattern  ·  type to find";
        }
        else
        {
            string verb = _crossFindMatchedFiles == 1 ? "matches" : "match";
            status =
                $"{_crossFindMatchedFiles} file{(_crossFindMatchedFiles == 1 ? string.Empty : "s")} {verb} the pattern  ·  Enter to replace all";
        }

        _screen.PutString(x + 2, y + 4, status, statusStyle, width - 4);
        _screen.PutString(
            x + 2,
            y + 5,
            "Tab/Up/Down: switch  ·  Enter: next/apply  ·  Ctrl+A: all  ·  Esc: cancel",
            mutedStyle,
            width - 4
        );
    }
}
