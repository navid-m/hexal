using System.Globalization;
using System.Text;
using Hexal.Core;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Util;
using NLua;

namespace Hexal.Ui;

public sealed partial class Editor : IEditorApi, IDisposable
{
    private LuaPluginManager? _plugins;
    private readonly LspManager _lsp = new();
    private readonly Dictionary<string, string> _pluginStatus =
        new(StringComparer.Ordinal);
    private bool _lspDirty;
    private long _lspDirtySince;

    private string _pluginPromptLabel = string.Empty;
    private readonly StringBuilder _pluginPromptText = new();
    private int _pluginPromptCursor;
    private Action<string?>? _pluginPromptCallback;

    private string _pluginSelectTitle = string.Empty;
    private readonly List<string> _pluginSelectItems = new();
    private Action<int?>? _pluginSelectCallback;

    private void LoadPlugins()
    {
        if (_plugins is not null)
        {
            return;
        }

        _plugins = new LuaPluginManager(this);
        _plugins.LoadAll();

        if (_plugins.Errors.Count > 0)
        {
            SetError($"plugin: {_plugins.Errors[0]}");
        }
        else if (_plugins.PluginCount > 0)
        {
            SetMessage($"{_plugins.PluginCount} plugin{(_plugins.PluginCount == 1 ? string.Empty : "s")} loaded");
        }

        _plugins.FireEvent("editor_started", new Dictionary<string, object?>
        {
            ["api_version"] = 1
        });
        _plugins.FireEvent("buffer_opened", CurrentBufferData());
    }

    public void Dispose()
    {
        UnloadPlugins();
    }

    private void UnloadPlugins()
    {
        if (_plugins is null)
        {
            return;
        }

        _plugins.FireEvent("editor_shutdown");
        _plugins.Dispose();
        _plugins = null;
        _lsp.Stop();
        _pluginStatus.Clear();
        _lspDirty = false;
    }

    private void FirePluginEvent(string eventName,
        IReadOnlyDictionary<string, object?>? data = null)
    {
        _plugins?.FireEvent(eventName, data);
    }

    private void EmitInputEvents(Document documentBefore, long revisionBefore,
        long cursorBefore)
    {
        if (!ReferenceEquals(documentBefore, _document))
        {
            return;
        }

        if (_document.Revision != revisionBefore)
        {
            _lspDirty = _lsp.IsRunning;
            _lspDirtySince = Environment.TickCount64;

            if (_plugins is not null && _plugins.HasSubscribers("buffer_changed"))
            {
                FirePluginEvent("buffer_changed", CurrentBufferData());
            }
        }

        if (_plugins is null)
        {
            return;
        }

        if (_cursor != cursorBefore && _plugins.HasSubscribers("cursor_moved"))
        {
            PluginCursor cursor = GetCursor();
            FirePluginEvent("cursor_moved", new Dictionary<string, object?>
            {
                ["line"] = cursor.Line,
                ["column"] = cursor.Column
            });
        }
    }

    private Dictionary<string, object?> CurrentBufferData()
    {
        return new Dictionary<string, object?>
        {
            ["id"] = CurrentBufferId,
            ["path"] = _document.Path,
            ["modified"] = _document.IsDirty,
            ["language"] = _language?.Id
        };
    }

    private string PluginStatusText()
    {
        if (_pluginStatus.Count == 0)
        {
            return string.Empty;
        }
        return string.Join(" | ", _pluginStatus.Values);
    }

    private void HandlePluginPrompt(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Escape)
        {
            FinishPluginPrompt(null);
            return;
        }

        if (inputEvent.Key == Key.Character && inputEvent.Ctrl)
        {
            return;
        }

        switch (inputEvent.Key)
        {
            case Key.Enter:
                FinishPluginPrompt(_pluginPromptText.ToString());
                return;
            case Key.Backspace:
                if (_pluginPromptCursor > 0)
                {
                    string text = _pluginPromptText.ToString();
                    int previous = UnicodeUtil.PreviousBoundary(text, _pluginPromptCursor);
                    _pluginPromptText.Remove(previous, _pluginPromptCursor - previous);
                    _pluginPromptCursor = previous;
                }
                return;
            case Key.Delete:
                if (_pluginPromptCursor < _pluginPromptText.Length)
                {
                    string text = _pluginPromptText.ToString();
                    int next = UnicodeUtil.NextBoundary(text, _pluginPromptCursor);
                    _pluginPromptText.Remove(_pluginPromptCursor, next - _pluginPromptCursor);
                }
                return;
            case Key.Left:
                _pluginPromptCursor = UnicodeUtil.PreviousBoundary(
                    _pluginPromptText.ToString(), _pluginPromptCursor);
                return;
            case Key.Right:
                _pluginPromptCursor = UnicodeUtil.NextBoundary(
                    _pluginPromptText.ToString(), _pluginPromptCursor);
                return;
            case Key.Home:
                _pluginPromptCursor = 0;
                return;
            case Key.End:
                _pluginPromptCursor = _pluginPromptText.Length;
                return;
        }

        if (inputEvent.Key != Key.Character || inputEvent.Alt)
        {
            return;
        }

        uint codePoint = (uint)inputEvent.Character.Value;
        if (codePoint < 0x20 || codePoint == 0x7F)
        {
            return;
        }

        string inserted = inputEvent.Character.ToString();
        _pluginPromptText.Insert(_pluginPromptCursor, inserted);
        _pluginPromptCursor += inserted.Length;
    }

    private void FinishPluginPrompt(string? value)
    {
        Action<string?>? callback = _pluginPromptCallback;
        _pluginPromptCallback = null;
        _pluginPromptText.Clear();
        _pluginPromptCursor = 0;
        _pluginPromptLabel = string.Empty;
        _mode = Mode.Normal;
        ClearMessage();
        callback?.Invoke(value);
    }

    private void RenderPluginPrompt(int row, Style baseStyle)
    {
        var promptStyle = new Style { Fg = 14, Bold = true };
        string label = _pluginPromptLabel;
        _screen.PutString(0, row, label, promptStyle, _columns);

        long labelWidth = UnicodeUtil.ColumnWidth(label);
        string text = _pluginPromptText.ToString();
        int remaining = _columns - (int)labelWidth;
        if (remaining > 0)
        {
            _screen.PutString((int)labelWidth, row, text, baseStyle, remaining);
        }

        long cursorColumn = UnicodeUtil.VisualColumn(text, _pluginPromptCursor, 1);
        long cursorX = labelWidth + cursorColumn;
        if (cursorX >= 0 && cursorX < _columns)
        {
            Style cursorStyle = _screen.At((int)cursorX, row).Style;
            cursorStyle.Reverse = true;
            _screen.ApplyStyle((int)cursorX, row, cursorStyle);
        }
    }

    private void OpenPluginPicker(string title, IReadOnlyList<string> items,
        Action<int?> callback)
    {
        _pluginSelectTitle = title;
        _pluginSelectItems.Clear();
        _pluginSelectItems.AddRange(items);
        _pluginSelectCallback = callback;
        OpenPicker(PickerKind.Plugin);
    }

    private bool PollLsp()
    {
        bool redraw = false;

        while (_lsp.TryDequeueEvent(out LspEvent lspEvent))
        {
            redraw = true;
            switch (lspEvent.Kind)
            {
                case LspEventKind.Started:
                    _pluginStatus["lsp"] = $"LSP: starting {lspEvent.ServerName}";
                    break;
                case LspEventKind.Initialized:
                    _pluginStatus["lsp"] = $"LSP: {lspEvent.ServerName}";
                    break;
                case LspEventKind.Diagnostics:
                {
                    int count = lspEvent.Diagnostics.Count;
                    _pluginStatus["lsp"] = count == 0
                        ? $"LSP: {lspEvent.ServerName}"
                        : $"LSP: {lspEvent.ServerName} ({count})";

                    if (_plugins is not null)
                    {
                        var diagnostics = new List<object?>(count);
                        foreach (LspDiagnostic diagnostic in lspEvent.Diagnostics)
                        {
                            diagnostics.Add(new Dictionary<string, object?>
                            {
                                ["line"] = diagnostic.Line,
                                ["column"] = diagnostic.Column,
                                ["severity"] = diagnostic.Severity,
                                ["message"] = diagnostic.Message,
                                ["source"] = diagnostic.Source
                            });
                        }

                        _plugins.FireEvent("__lsp_diagnostics",
                            new Dictionary<string, object?>
                            {
                                ["server"] = lspEvent.ServerName,
                                ["uri"] = lspEvent.Uri,
                                ["diagnostics"] = diagnostics
                            });
                    }
                    break;
                }
                case LspEventKind.Error:
                    SetError($"LSP {lspEvent.ServerName}: {lspEvent.Message}");
                    break;
                case LspEventKind.Stopped:
                    _pluginStatus.Remove("lsp");
                    _lspDirty = false;
                    break;
            }
        }

        if (_lspDirty && _lsp.IsInitialized &&
            Environment.TickCount64 - _lspDirtySince >= 150)
        {
            _lsp.DidChange(GetText());
            _lspDirty = false;
        }

        return redraw;
    }

    private void ResetLsp()
    {
        _lsp.Stop();
        _pluginStatus.Remove("lsp");
        _lspDirty = false;
    }

    private void NotifyLspSaved()
    {
        if (_lsp.IsInitialized)
        {
            _lsp.DidSave(GetText());
        }
    }

    // -----------------------------------------------------------------------
    // IEditorApi
    // -----------------------------------------------------------------------

    public string CurrentBufferId => _document.Path ?? "[No Name]";

    public string GetText()
    {
        return Encoding.UTF8.GetString(_document.Text());
    }

    public string? GetLine(long line, out string error)
    {
        error = string.Empty;
        if (line < 0 || line >= _document.LineCount)
        {
            error = $"line {line} is out of range";
            return null;
        }

        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        return Encoding.UTF8.GetString(_lineBuffer, 0, length);
    }

    public PluginCursor GetCursor()
    {
        long line = CursorLine();
        long lineStart = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        long column = ScalarColumn(_lineBuffer.AsSpan(0, length),
            (int)(_cursor - lineStart));
        return new PluginCursor(line, column);
    }

    public bool SetCursor(long line, long column, out string error)
    {
        error = string.Empty;
        if (line < 0 || line >= _document.LineCount)
        {
            error = $"line {line} is out of range";
            return false;
        }
        if (column < 0)
        {
            error = "column must not be negative";
            return false;
        }

        long lineStart = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int local = ByteOffsetForScalarColumn(_lineBuffer.AsSpan(0, length), column);
        _cursor = Math.Clamp(lineStart + local, lineStart, _document.LineEnd(line));
        _preferredColumnValid = false;
        _document.BreakUndoGroup();
        SnapCursor();
        ClearMessage();
        return true;
    }

    public PluginSelection? GetSelection()
    {
        if (_mode != Mode.Select)
        {
            return null;
        }

        (long start, long end) = SelectionRange();
        long startLine = _document.LineOfOffset(start);
        long endLine = _document.LineOfOffset(end);
        long startLineStart = _document.LineStart(startLine);
        long endLineStart = _document.LineStart(endLine);

        int startLength = _document.CopyLineText(startLine, ref _lineBuffer, out _);
        long startColumn = ScalarColumn(_lineBuffer.AsSpan(0, startLength),
            (int)Math.Clamp(start - startLineStart, 0, startLength));

        int endLength = _document.CopyLineText(endLine, ref _lineBuffer, out _);
        long endColumn = ScalarColumn(_lineBuffer.AsSpan(0, endLength),
            (int)Math.Clamp(end - endLineStart, 0, endLength));

        string text = Encoding.UTF8.GetString(_document.Range(start, end - start));
        return new PluginSelection(startLine, startColumn, endLine, endColumn,
            text, _selectionLinewise);
    }

    public bool InsertText(string text, out string error)
    {
        error = string.Empty;
        if (string.IsNullOrEmpty(text))
        {
            return true;
        }

        byte[] bytes = Encoding.UTF8.GetBytes(text);
        _document.BreakUndoGroup();
        _document.Insert(_cursor, bytes);
        _cursor = Math.Min(_cursor + bytes.Length, _document.Size);
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
        return true;
    }

    public bool ReplaceRange(object? start, object? finish, string text,
        out string error)
    {
        error = string.Empty;
        if (!TryPluginOffset(start, out long startOffset, out error) ||
            !TryPluginOffset(finish, out long endOffset, out error))
        {
            return false;
        }

        if (startOffset > endOffset)
        {
            error = "start must not be after finish";
            return false;
        }
        if (startOffset < 0 || endOffset > _document.Size)
        {
            error = "range is outside the document";
            return false;
        }

        byte[] bytes = Encoding.UTF8.GetBytes(text ?? string.Empty);
        _document.BreakUndoGroup();
        _document.Replace(startOffset, endOffset - startOffset, bytes);
        _cursor = Math.Clamp(startOffset + bytes.Length, 0, _document.Size);
        _preferredColumnValid = false;
        SnapCursor();
        ClearMessage();
        return true;
    }

    public void ShowMessage(string text, bool warning)
    {
        SetMessage(warning ? $"Warning: {text}" : text);
    }

    public void SetPluginStatus(string key, string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            _pluginStatus.Remove(key);
        }
        else
        {
            _pluginStatus[key] = text;
        }
    }

    public void RequestPrompt(string label, Action<string?> callback)
    {
        Action<string?>? previous = _pluginPromptCallback;
        _pluginPromptCallback = null;
        previous?.Invoke(null);

        _pluginPromptLabel = label ?? string.Empty;
        _pluginPromptText.Clear();
        _pluginPromptCursor = 0;
        _pluginPromptCallback = callback;
        _mode = Mode.PluginPrompt;
        ClearMessage();
    }

    public void RequestSelect(string title, IReadOnlyList<string> items,
        Action<int?> callback)
    {
        OpenPluginPicker(title ?? "Select", items, callback);
    }

    public bool StartLsp(LspServerConfig config, out string error)
    {
        error = string.Empty;
        if (_document.Path is null)
        {
            error = "language servers require a file-backed buffer";
            return false;
        }

        _lspDirty = false;
        return _lsp.TryStart(config, _document.Path, GetText(), out error);
    }

    public void StopLsp(string? name)
    {
        if (string.IsNullOrEmpty(name) ||
            string.Equals(_lsp.ServerName, name, StringComparison.OrdinalIgnoreCase))
        {
            _lsp.Stop();
            _pluginStatus.Remove("lsp");
            _lspDirty = false;
        }
    }

    public bool ExecuteCommand(string name, IReadOnlyList<string> args,
        out string error)
    {
        error = string.Empty;
        if (_plugins is not null && _plugins.HasCommand(name))
        {
            return _plugins.TryExecuteCommand(name, args, out error);
        }

        var command = new ExCommand { Name = name };
        command.Arguments.AddRange(args);
        ExecuteParsedCommand(command, force: false);
        return true;
    }

    public void ReportPluginError(string pluginId, string message)
    {
        SetError($"plugin {pluginId}: {message}");
    }

    private bool TryPluginOffset(object? value, out long offset, out string error)
    {
        offset = 0;
        error = string.Empty;

        switch (value)
        {
            case null:
                error = "position is required";
                return false;
            case byte or sbyte or short or ushort or int or uint or long or ulong or
                 float or double or decimal:
                offset = Convert.ToInt64(value, CultureInfo.InvariantCulture);
                return true;
            case LuaTable table:
            {
                object? lineValue = table["line"];
                object? columnValue = table["column"];
                if (lineValue is null || columnValue is null)
                {
                    error = "position table needs line and column fields";
                    return false;
                }

                long line = Convert.ToInt64(lineValue, CultureInfo.InvariantCulture);
                long column = Convert.ToInt64(columnValue, CultureInfo.InvariantCulture);
                if (line < 0 || line >= _document.LineCount || column < 0)
                {
                    error = "position is outside the document";
                    return false;
                }

                long lineStart = _document.LineStart(line);
                int length = _document.CopyLineText(line, ref _lineBuffer, out _);
                int local = ByteOffsetForScalarColumn(_lineBuffer.AsSpan(0, length),
                    column);
                offset = Math.Clamp(lineStart + local, lineStart, _document.LineEnd(line));
                return true;
            }
            default:
                error = "position must be a byte offset or a {line, column} table";
                return false;
        }
    }

    private static long ScalarColumn(ReadOnlySpan<byte> text, int byteOffset)
    {
        int offset = 0;
        long column = 0;
        int target = Math.Clamp(byteOffset, 0, text.Length);

        while (offset < target)
        {
            UnicodeUtil.Decode(text, offset, out _, out int length);
            if (length <= 0)
            {
                break;
            }
            offset += length;
            ++column;
        }

        return column;
    }

    private static int ByteOffsetForScalarColumn(ReadOnlySpan<byte> text, long column)
    {
        int offset = 0;
        for (long i = 0; i < column && offset < text.Length; ++i)
        {
            offset = UnicodeUtil.NextBoundary(text, offset);
        }
        return offset;
    }
}
