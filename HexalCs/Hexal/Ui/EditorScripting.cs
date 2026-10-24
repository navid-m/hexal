using System.Diagnostics;
using System.Globalization;
using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Util;
using NLua;

namespace Hexal.Ui;

public sealed partial class Editor : IEditorApi, ILuaLspHost, IDisposable
{
    private LuaPluginManager? _plugins;
    private readonly LspManager _lsp = new();
    private readonly Dictionary<string, string> _pluginStatus = new(StringComparer.Ordinal);
    private bool _lspDirty;
    private bool _lspFallbackBlocked;
    private long _lspDirtySince;
    private readonly Dictionary<long, List<(int Start, int End)>> _unnecessaryDiagnostics = new();
    private readonly List<(int Start, int End)> _unnecessaryByteRanges = new();

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
        if (_benchmark is not null)
        {
            _plugins.SetBenchmark(_benchmark);
        }

        long phaseStart = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
        _plugins.LoadAll();
        _benchmark?.AddSince(
            "Plugins",
            "Load all",
            phaseStart,
            $"{_plugins.PluginCount} running, "
                + $"{_plugins.DeferredPluginCount} deferred, "
                + $"{_plugins.Errors.Count} errors"
        );

        if (_plugins.Errors.Count > 0)
        {
            SetError($"plugin: {_plugins.Errors[0]}");
        }
        else if (_plugins.PluginCount > 0 || _plugins.DeferredPluginCount > 0)
        {
            int total = _plugins.PluginCount + _plugins.DeferredPluginCount;
            string message = $"{total} plugin{(total == 1 ? string.Empty : "s")} loaded";
            if (_plugins.DeferredPluginCount > 0)
            {
                message += $" ({_plugins.DeferredPluginCount} deferred)";
            }
            SetMessage(message);
        }

        _plugins.FireEvent(
            "editor_started",
            new Dictionary<string, object?> { ["api_version"] = 1 }
        );
        _plugins.FireEvent("buffer_opened", CurrentBufferData());
        _plugins.ActivateLuaLsp();
    }

    public void Dispose()
    {
        UnloadPlugins();
    }

    private void UnloadPlugins()
    {
        DisposeDebugger();
        if (_plugins is null)
        {
            return;
        }

        long phaseStart = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
        int pluginCount = _plugins.PluginCount;
        _plugins.FireEvent("editor_shutdown");
        _plugins.Dispose();
        _benchmark?.AddSince(
            "Plugins (shutdown)",
            "Unload all",
            phaseStart,
            $"{pluginCount} plugins"
        );
        _plugins = null;
        _lsp.Stop();
        _pluginStatus.Clear();
        _lspDirty = false;
    }

    private void FirePluginEvent(
        string eventName,
        IReadOnlyDictionary<string, object?>? data = null
    )
    {
        if (string.Equals(eventName, "buffer_opened", StringComparison.Ordinal))
        {
            _lspFallbackBlocked = false;
        }

        _plugins?.FireEvent(eventName, data);
        if (string.Equals(eventName, "buffer_opened", StringComparison.Ordinal))
        {
            _plugins?.ActivateLuaLsp();
        }
    }

    private void EmitInputEvents(Document documentBefore, long revisionBefore, long cursorBefore)
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
            FirePluginEvent(
                "cursor_moved",
                new Dictionary<string, object?>
                {
                    ["line"] = cursor.Line,
                    ["column"] = cursor.Column,
                }
            );
        }
    }

    private Dictionary<string, object?> CurrentBufferData()
    {
        return new Dictionary<string, object?>
        {
            ["id"] = CurrentBufferId,
            ["path"] = _document.Path,
            ["modified"] = _document.IsDirty,
            ["language"] = _language?.Id,
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
                    _pluginPromptText.ToString(),
                    _pluginPromptCursor
                );
                return;
            case Key.Right:
                _pluginPromptCursor = UnicodeUtil.NextBoundary(
                    _pluginPromptText.ToString(),
                    _pluginPromptCursor
                );
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
        var promptStyle = _theme.AccentStyle;
        string label = _pluginPromptLabel;
        _screen.PutString(BottomLinePadding, row, label, promptStyle, _columns - BottomLinePadding);

        long labelWidth = UnicodeUtil.ColumnWidth(label);
        string text = _pluginPromptText.ToString();
        int remaining = _columns - BottomLinePadding - (int)labelWidth;
        if (remaining > 0)
        {
            _screen.PutString(BottomLinePadding + (int)labelWidth, row, text, baseStyle, remaining);
        }

        long cursorColumn = UnicodeUtil.VisualColumn(text, _pluginPromptCursor, 1);
        long cursorX = BottomLinePadding + labelWidth + cursorColumn;
        if (cursorX >= 0 && cursorX < _columns)
        {
            Style cursorStyle = _screen.At((int)cursorX, row).Style;
            cursorStyle.Reverse = true;
            _screen.ApplyStyle((int)cursorX, row, cursorStyle);
        }
    }

    private void OpenPluginPicker(string title, IReadOnlyList<string> items, Action<int?> callback)
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
                    UpdateUnnecessaryDiagnostics(lspEvent.Uri, lspEvent.Diagnostics);
                    _pluginStatus["lsp"] =
                        count == 0
                            ? $"LSP: {lspEvent.ServerName}"
                            : $"LSP: {lspEvent.ServerName} ({count})";

                    if (_pickerKind == PickerKind.Problems)
                    {
                        PopulateProblemItems();
                    }

                    if (_plugins is not null)
                    {
                        var diagnostics = new List<object?>(count);
                        foreach (LspDiagnostic diagnostic in lspEvent.Diagnostics)
                        {
                            diagnostics.Add(
                                new Dictionary<string, object?>
                                {
                                    ["line"] = diagnostic.Line,
                                    ["column"] = diagnostic.Column,
                                    ["severity"] = diagnostic.Severity,
                                    ["message"] = diagnostic.Message,
                                    ["source"] = diagnostic.Source,
                                    ["tags"] = diagnostic.Tags,
                                }
                            );
                        }

                        _plugins.FireEvent(
                            "__lsp_diagnostics",
                            new Dictionary<string, object?>
                            {
                                ["server"] = lspEvent.ServerName,
                                ["uri"] = lspEvent.Uri,
                                ["diagnostics"] = diagnostics,
                            }
                        );
                    }
                    break;
                }
                case LspEventKind.Hover:
                    if (!_hoverRequestPending)
                    {
                        break;
                    }
                    _hoverRequestPending = false;
                    if (lspEvent.Text.Length == 0)
                    {
                        SetMessage("No hover information available.");
                    }
                    else
                    {
                        ShowHoverPopup(lspEvent.Text);
                    }
                    break;
                case LspEventKind.Definition:
                    if (!_definitionRequestPending)
                    {
                        break;
                    }
                    _definitionRequestPending = false;
                    if (lspEvent.Locations.Count == 0)
                    {
                        SetMessage("No definition found.");
                    }
                    else
                    {
                        NavigateToDefinition(lspEvent.Locations[0]);
                    }
                    break;
                case LspEventKind.CodeActions:
                    if (!_codeActionRequestPending)
                    {
                        break;
                    }
                    _codeActionRequestPending = false;
                    if (lspEvent.CodeActions.Count == 0)
                    {
                        SetMessage("No code actions available.");
                    }
                    else
                    {
                        _codeActions.Clear();
                        _codeActions.AddRange(lspEvent.CodeActions);
                        _pendingCodeActionResolve = false;
                        OpenPicker(PickerKind.CodeActions);
                    }
                    break;
                case LspEventKind.CodeActionResolved:
                    if (_pendingCodeActionResolve)
                    {
                        _pendingCodeActionResolve = false;
                        if (lspEvent.CodeActions.Count > 0)
                        {
                            ApplyCodeAction(lspEvent.CodeActions[0]);
                        }
                        else
                        {
                            SetMessage(
                                "The language server returned no changes for this code action."
                            );
                        }
                    }
                    break;
                case LspEventKind.DocumentSymbols:
                    if (!_symbolRequestPending)
                    {
                        break;
                    }
                    _symbolRequestPending = false;
                    OpenDocumentSymbolPicker(lspEvent.DocumentSymbols);
                    break;
                case LspEventKind.Completions:
                    if (!_lspCompletionPending)
                    {
                        break;
                    }
                    OpenLspCompletion(lspEvent.Completions);
                    break;
                case LspEventKind.WorkspaceDiagnostics:
                    if (_pickerKind == PickerKind.Problems && _problemsWorkspace)
                    {
                        PopulateProblemItems();
                    }
                    break;
                case LspEventKind.Error:
                    _pendingCodeActionResolve = false;
                    SetError($"LSP {lspEvent.ServerName}: {lspEvent.Message}");
                    break;
                case LspEventKind.Stopped:
                    _pluginStatus.Remove("lsp");
                    _lspDirty = false;
                    break;
            }
        }

        if (_lspDirty && _lsp.IsInitialized && Environment.TickCount64 - _lspDirtySince >= 150)
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
        _unnecessaryDiagnostics.Clear();
        _unnecessaryByteRanges.Clear();
        _hoverRequestPending = false;
        _definitionRequestPending = false;
        _codeActionRequestPending = false;
        _symbolRequestPending = false;
        CloseLspCompletion();
    }

    private void UpdateUnnecessaryDiagnostics(
        string uri,
        IReadOnlyList<LspDiagnostic> diagnostics
    )
    {
        if (_document.Path is null)
        {
            return;
        }

        string path = LspManager.UriToPath(uri);
        try
        {
            if (!PathsEqual(Path.GetFullPath(path), Path.GetFullPath(_document.Path)))
            {
                return;
            }
        }
        catch (Exception ex)
            when (ex
                    is ArgumentException
                        or NotSupportedException
                        or PathTooLongException
                        or UnauthorizedAccessException
                        or IOException
            )
        {
            return;
        }

        _unnecessaryDiagnostics.Clear();
        if (diagnostics.Count == 0)
        {
            return;
        }

        long lastLine = _document.LineCount - 1;
        foreach (LspDiagnostic diagnostic in diagnostics)
        {
            if (!diagnostic.HasTag(LspDiagnosticTag.Unnecessary))
            {
                continue;
            }
            if (diagnostic.Line < 0 || diagnostic.Line > lastLine)
            {
                continue;
            }

            int endLine = diagnostic.EndLine >= 0 ? diagnostic.EndLine : diagnostic.Line;
            int endColumn =
                diagnostic.EndColumn >= 0 ? diagnostic.EndColumn : diagnostic.Column + 1;
            if (endLine < diagnostic.Line)
            {
                endLine = diagnostic.Line;
                endColumn = diagnostic.Column + 1;
            }
            endLine = (int)Math.Min(endLine, lastLine);

            for (long line = diagnostic.Line; line <= endLine; ++line)
            {
                int start = line == diagnostic.Line ? Math.Max(0, diagnostic.Column) : 0;
                int end = line == endLine ? Math.Max(start, endColumn) : int.MaxValue;
                if (end <= start)
                {
                    continue;
                }

                if (!_unnecessaryDiagnostics.TryGetValue(line, out List<(int Start, int End)>? ranges))
                {
                    ranges = new List<(int Start, int End)>(1);
                    _unnecessaryDiagnostics[line] = ranges;
                }
                ranges.Add((start, end));
            }
        }
    }

    private void NotifyLspSaved()
    {
        if (_lsp.IsInitialized)
        {
            _lsp.DidSave(GetText());
        }
    }

    // -----------------------------------------------------------------------
    // LSP ACTIONS
    // -----------------------------------------------------------------------

    private (int Line, int Character) CursorLspPosition()
    {
        long line = CursorLine();
        long start = _document.LineStart(line);
        int length = _document.CopyLineText(line, ref _lineBuffer, out _);
        int byteOffset = (int)Math.Clamp(_cursor - start, 0, length);
        return (
            (int)line,
            LspPositions.ByteOffsetToUtf16Column(_lineBuffer.AsSpan(0, length), byteOffset)
        );
    }

    private bool EnsureLspReady(string feature)
    {
        if (!_lsp.IsRunning && _language is not null && _plugins is not null)
        {
            // Some plugin language servers only register themselves on
            // buffer_opened. Retry that event lazily so the first LSP action
            // can start the server even when the open event was missed.
            FirePluginEvent("buffer_opened", CurrentBufferData());
        }

        if (!_lsp.IsRunning)
        {
            SetError("No LSP configured");
            return false;
        }

        if (!_lsp.IsInitialized)
        {
            SetError($"LSP is still starting; {feature} is not available yet");
            return false;
        }

        if (_document.Path is null)
        {
            SetError($"{feature} requires a file-backed buffer");
            return false;
        }

        return true;
    }

    private void ShowHover()
    {
        if (!EnsureLspReady("hover"))
        {
            return;
        }

        (int line, int character) = CursorLspPosition();
        if (!_lsp.RequestHover(_document.Path!, line, character))
        {
            SetError("No LSP configured");
            return;
        }

        _hoverRequestPending = true;
        SetMessage("Requesting hover information...");
    }

    private void GoToDefinition()
    {
        if (!EnsureLspReady("go to definition"))
        {
            return;
        }

        (int line, int character) = CursorLspPosition();
        if (!_lsp.RequestDefinition(_document.Path!, line, character))
        {
            SetError("No LSP configured");
            return;
        }

        _definitionRequestPending = true;
        SetMessage("Requesting definition...");
    }

    private void NavigateToDefinition(LspLocation location)
    {
        string path = LspManager.UriToPath(location.Uri);
        if (string.IsNullOrEmpty(path))
        {
            SetError("The language server returned an invalid definition location.");
            return;
        }

        long column = LspColumnToByteOffset(path, location.Line, location.Character);

        if (_document.Path is not null && SamePath(_document.Path, path))
        {
            RecordJump(_cursor);
            MoveToLineColumn(location.Line, column);
        }
        else
        {
            RecordJump(_cursor);
            RequestOpen(path, force: false, location.Line, column);
        }
    }

    private void ShowCodeActions()
    {
        if (!EnsureLspReady("code actions"))
        {
            return;
        }

        (int line, int character) = CursorLspPosition();
        List<LspDiagnostic> diagnostics = _lsp.SnapshotDiagnosticsForPath(_document.Path!);
        _pendingCodeActionResolve = false;

        if (!_lsp.RequestCodeActions(_document.Path!, line, character, diagnostics))
        {
            SetError("No LSP configured");
            return;
        }

        _codeActionRequestPending = true;
        SetMessage("Requesting code actions...");
    }

    private void ShowDocumentSymbols()
    {
        if (!EnsureLspReady("document symbols"))
        {
            return;
        }

        if (!_lsp.RequestDocumentSymbols(_document.Path!))
        {
            SetError("No LSP configured");
            return;
        }

        _symbolRequestPending = true;
        SetMessage("Requesting document symbols...");
    }

    private void OpenDocumentSymbolPicker(IReadOnlyList<LspDocumentSymbol> symbols)
    {
        if (symbols.Count == 0)
        {
            SetMessage("No symbols found.");
            return;
        }

        var labels = new List<string>(symbols.Count);
        foreach (LspDocumentSymbol symbol in symbols)
        {
            int indent = Math.Min(symbol.Depth, 12) * 2;
            string label = new string(' ', indent) + $"{SymbolKindName(symbol.Kind)} {symbol.Name}";
            if (symbol.Detail.Length > 0)
            {
                label += $"  {symbol.Detail}";
            }
            labels.Add(label);
        }

        OpenPluginPicker(
            "Symbols",
            labels,
            index =>
            {
                if (index is null || index.Value < 0 || index.Value >= symbols.Count)
                {
                    SetMessage("Symbol picker cancelled.");
                    return;
                }

                LspDocumentSymbol symbol = symbols[index.Value];
                string path = LspManager.UriToPath(symbol.Uri);
                if (string.IsNullOrEmpty(path))
                {
                    SetError("The language server returned an invalid symbol location.");
                    return;
                }

                long column = LspColumnToByteOffset(path, symbol.Line, symbol.Character);
                ClearMessage();
                if (_document.Path is not null && SamePath(_document.Path, path))
                {
                    RecordJump(_cursor);
                    MoveToLineColumn(symbol.Line, column);
                }
                else
                {
                    RecordJump(_cursor);
                    RequestOpen(path, force: false, symbol.Line, column);
                }
            }
        );
    }

    private static string SymbolKindName(int kind)
    {
        return kind switch
        {
            1 => "File",
            2 => "Module",
            3 => "Namespace",
            4 => "Package",
            5 => "Class",
            6 => "Method",
            7 => "Property",
            8 => "Field",
            9 => "Constructor",
            10 => "Enum",
            11 => "Interface",
            12 => "Function",
            13 => "Variable",
            14 => "Constant",
            15 => "String",
            16 => "Number",
            17 => "Boolean",
            18 => "Array",
            19 => "Object",
            20 => "Key",
            21 => "Null",
            22 => "EnumMember",
            23 => "Struct",
            24 => "Event",
            25 => "Operator",
            26 => "TypeParameter",
            _ => "Symbol",
        };
    }

    private void ShowProblems(bool workspace)
    {
        _problemsWorkspace = workspace;

        if (!_lsp.IsRunning)
        {
            SetError("No LSP configured");
            return;
        }

        if (!workspace && _document.Path is null)
        {
            SetError("Document diagnostics require a file-backed buffer");
            return;
        }

        // Servers that support pull diagnostics answer with a complete
        // workspace report. Published diagnostics are always shown as well, so
        // this is useful even when the request is unsupported.
        _lsp.RequestWorkspaceDiagnostics();
        OpenPicker(PickerKind.Problems);
    }

    private void ShowHoverPopup(string text)
    {
        _hoverLines.Clear();

        int width = Math.Max(20, Math.Min(_columns - 6, 88));
        int contentWidth = Math.Max(1, width - 4);
        string? codeInfo = null;
        char fenceChar = '\0';
        int fenceLength = 0;
        bool inCode = false;
        bool codeStartInBlock = false;

        foreach (string rawLine in text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n'))
        {
            bool isFence = TryHoverFence(
                rawLine,
                out char currentFence,
                out int currentLength,
                out string info
            );

            if (inCode)
            {
                if (
                    isFence
                    && currentFence == fenceChar
                    && currentLength >= fenceLength
                    && info.Length == 0
                )
                {
                    inCode = false;
                    continue;
                }

                AddHoverCodeLine(
                    rawLine,
                    ResolveHoverLanguage(codeInfo),
                    contentWidth,
                    _hoverLines,
                    ref codeStartInBlock
                );
                continue;
            }

            if (isFence)
            {
                inCode = true;
                fenceChar = currentFence;
                fenceLength = currentLength;
                codeInfo = info;
                codeStartInBlock = false;
                continue;
            }

            if (rawLine.Length == 0)
            {
                _hoverLines.Add(new HoverLine());
                continue;
            }

            WrapHoverLine(rawLine, contentWidth, _hoverLines);
        }

        if (_hoverLines.Count == 0)
        {
            SetMessage("No hover information available.");
            return;
        }

        _hoverTitle = "Hover";
        _hoverTop = 0;
        _hoverVisible = true;
        ClearMessage();
    }

    private static bool TryHoverFence(string line, out char fence, out int count, out string info)
    {
        fence = '\0';
        count = 0;
        info = string.Empty;

        int offset = 0;
        while (offset < line.Length && char.IsWhiteSpace(line[offset]))
        {
            ++offset;
        }

        if (offset >= line.Length || (line[offset] != '`' && line[offset] != '~'))
        {
            return false;
        }

        char marker = line[offset];
        while (offset < line.Length && line[offset] == marker)
        {
            ++count;
            ++offset;
        }

        if (count < 3)
        {
            count = 0;
            return false;
        }

        fence = marker;
        info = line[offset..].Trim();
        return true;
    }

    private LanguageDefinition? ResolveHoverLanguage(string? info)
    {
        if (string.IsNullOrWhiteSpace(info))
        {
            return null;
        }

        string id = info.Trim();
        if (id.Length >= 2 && id[0] == '{' && id[^1] == '}')
        {
            id = id[1..^1].Trim();
        }

        int separator = -1;
        for (int index = 0; index < id.Length; ++index)
        {
            if (id[index] is ' ' or '\t' or ',')
            {
                separator = index;
                break;
            }
        }
        if (separator >= 0)
        {
            id = id[..separator];
        }

        id = id.Trim('.').ToLowerInvariant();
        id = id switch
        {
            "cs" or "c#" => "csharp",
            "c++" or "cxx" or "cc" or "hpp" => "cpp",
            "py" => "python",
            "golang" => "go",
            "pyx" => "cython",
            _ => id,
        };

        return _languages.Find(id);
    }

    private void AddHoverCodeLine(
        string line,
        LanguageDefinition? language,
        int width,
        List<HoverLine> destination,
        ref bool startInBlock
    )
    {
        if (line.Length == 0)
        {
            destination.Add(new HoverLine());
            return;
        }

        byte[] text = Encoding.UTF8.GetBytes(line);
        var tokens = new List<SyntaxToken>();
        bool endInBlock = startInBlock;
        if (language is not null)
        {
            SyntaxHighlighter.GetLineTokens(language, text, startInBlock, tokens, out endInBlock);
        }
        startInBlock = endInBlock;

        var current = new HoverLine();
        int offset = 0;
        int tokenIndex = 0;
        int rowWidth = 0;
        int visual = 0;

        while (offset < text.Length)
        {
            int runeStart = offset;
            while (tokenIndex < tokens.Count && tokens[tokenIndex].End <= runeStart)
            {
                ++tokenIndex;
            }

            SyntaxTokenKind kind = SyntaxTokenKind.Default;
            if (tokenIndex < tokens.Count && tokens[tokenIndex].Start <= runeStart)
            {
                kind = tokens[tokenIndex].Kind;
            }

            UnicodeUtil.Decode(text, offset, out Rune rune, out int byteLength);
            offset += byteLength;

            uint codePoint = (uint)rune.Value;
            string output;
            int cellWidth;

            if (codePoint == '\t')
            {
                cellWidth = TabWidth - (visual % TabWidth);
                output = new string(' ', cellWidth);
            }
            else
            {
                cellWidth = UnicodeUtil.DisplayWidth(codePoint);
                if (cellWidth == 0)
                {
                    if (codePoint < 0x20 || codePoint == 0x7F)
                    {
                        output = "\uFFFD";
                        cellWidth = 1;
                    }
                    else
                    {
                        continue;
                    }
                }
                else
                {
                    output = rune.ToString();
                }
            }

            if (rowWidth > 0 && rowWidth + cellWidth > width)
            {
                destination.Add(current);
                current = new HoverLine();
                rowWidth = 0;
                visual = 0;
            }

            AddHoverSegment(current, output, kind);
            rowWidth += cellWidth;
            visual += cellWidth;
        }

        if (current.Segments.Count > 0)
        {
            destination.Add(current);
        }
    }

    private static void AddHoverSegment(HoverLine line, string text, SyntaxTokenKind kind)
    {
        if (text.Length == 0)
        {
            return;
        }

        if (line.Segments.Count > 0 && line.Segments[^1].Kind == kind)
        {
            HoverSegment previous = line.Segments[^1];
            line.Segments[^1] = new HoverSegment(previous.Text + text, kind);
            return;
        }

        line.Segments.Add(new HoverSegment(text, kind));
    }

    private static void WrapHoverLine(string line, int width, List<HoverLine> destination)
    {
        width = Math.Max(1, width);
        int offset = 0;

        while (offset < line.Length)
        {
            int take = Math.Min(width, line.Length - offset);
            if (offset + take < line.Length && take > 0)
            {
                int space = line.LastIndexOf(' ', offset + take - 1, take);
                if (space > offset)
                {
                    take = space - offset;
                }
            }

            if (take <= 0)
            {
                take = 1;
            }

            var hoverLine = new HoverLine();
            hoverLine.Segments.Add(
                new HoverSegment(line.Substring(offset, take).TrimEnd(), SyntaxTokenKind.Default)
            );
            destination.Add(hoverLine);
            offset += take;
            while (offset < line.Length && line[offset] == ' ')
            {
                ++offset;
            }
        }
    }

    private void CloseHover()
    {
        _hoverVisible = false;
        _hoverLines.Clear();
        _hoverTop = 0;
        _hoverRequestPending = false;
    }

    private void ApplyCodeAction(LspCodeAction action)
    {
        _pendingCodeActionResolve = false;

        if (
            action.NeedsResolve
            || (action.Edits.Count == 0 && action.Command.Length == 0 && action.Raw is not null)
        )
        {
            if (_lsp.ResolveCodeAction(action))
            {
                _pendingCodeActionResolve = true;
                SetMessage($"Resolving code action: {action.Title}");
                return;
            }
        }

        bool changedCurrent = false;
        int changedFiles = 0;
        var failures = new List<string>();

        foreach (
            IGrouping<string, LspTextEdit> group in action.Edits.GroupBy(
                edit => edit.Uri,
                StringComparer.Ordinal
            )
        )
        {
            string path;
            try
            {
                path = Path.GetFullPath(LspManager.UriToPath(group.Key));
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                failures.Add(ex.Message);
                continue;
            }

            bool isCurrent =
                _document.Path is not null && PathsEqual(Path.GetFullPath(_document.Path), path);

            if (isCurrent)
            {
                if (!TryApplyTextEdits(_document, group.ToList(), out string error))
                {
                    failures.Add(error);
                    continue;
                }

                changedCurrent = true;
                ++changedFiles;
                continue;
            }

            Document? document = Document.Open(path, out string openError);
            if (document is null)
            {
                failures.Add(openError);
                continue;
            }

            if (!TryApplyTextEdits(document, group.ToList(), out string editError))
            {
                failures.Add(editError);
                continue;
            }

            if (!document.Save(out string saveError))
            {
                failures.Add(saveError);
                continue;
            }

            ++changedFiles;
        }

        if (changedCurrent)
        {
            _cursor = Math.Clamp(_cursor, 0, _document.Size);
            _preferredColumnValid = false;
            _document.BreakUndoGroup();
            SnapCursor();
            _lspDirty = _lsp.IsRunning;
            _lspDirtySince = Environment.TickCount64;
        }

        if (action.Command.Length > 0)
        {
            _lsp.ExecuteCommand(action.Command, action.CommandArguments);
        }

        if (failures.Count > 0)
        {
            SetError($"code action failed: {failures[0]}");
        }
        else if (changedFiles > 0)
        {
            string plural = changedFiles == 1 ? "file" : "files";
            SetMessage($"Applied code action: {action.Title} ({changedFiles} {plural})");
        }
        else if (action.Command.Length > 0)
        {
            SetMessage($"Ran code action: {action.Title}");
        }
        else
        {
            SetMessage($"Code action '{action.Title}' made no changes.");
        }
    }

    /// <summary>
    /// Applies LSP edits to a document. Edits are converted from UTF-16
    /// positions using the pre-edit line index and applied from the end of the
    /// document backwards so earlier offsets remain valid.
    /// </summary>
    private static bool TryApplyTextEdits(
        Document document,
        List<LspTextEdit> edits,
        out string error
    )
    {
        error = string.Empty;
        var resolved = new List<(long Start, long End, byte[] Text)>(edits.Count);

        foreach (LspTextEdit edit in edits)
        {
            if (
                edit.StartLine < 0
                || edit.EndLine < 0
                || edit.StartLine >= document.LineCount
                || edit.EndLine >= document.LineCount
            )
            {
                error = "code action references a line outside the document";
                return false;
            }

            long start = LinePositionToOffset(document, edit.StartLine, edit.StartCharacter);
            long end = LinePositionToOffset(document, edit.EndLine, edit.EndCharacter);
            if (end < start)
            {
                error = "code action has an inverted edit range";
                return false;
            }

            resolved.Add((start, end, Encoding.UTF8.GetBytes(edit.NewText)));
        }

        resolved.Sort((left, right) => right.Start.CompareTo(left.Start));

        long lastStart = long.MaxValue;
        foreach ((long start, long end, byte[] text) in resolved)
        {
            if (end > lastStart)
            {
                error = "code action contains overlapping edits";
                return false;
            }

            lastStart = start;
        }

        document.BreakUndoGroup();
        foreach ((long start, long end, byte[] text) in resolved)
        {
            document.Replace(start, end - start, text);
        }
        document.BreakUndoGroup();
        return true;
    }

    private static long LinePositionToOffset(Document document, int line, int utf16Character)
    {
        line = (int)Math.Clamp(line, 0, document.LineCount - 1);
        long start = document.LineStart(line);
        long end = document.LineEnd(line);
        byte[] text = document.LineText(line);
        int column = LspPositions.Utf16ColumnToByteOffset(text, utf16Character);
        return Math.Clamp(start + column, start, end);
    }

    // -----------------------------------------------------------------------
    // ILuaLspHost
    // -----------------------------------------------------------------------

    public string? CurrentDocumentPath => _document.Path;

    public string? CurrentLanguageId => _language?.Id;

    public string? CurrentText => GetText();

    public string? WorkspaceRoot => _workspaceRoot;

    public bool StartLuaLsp(LuaLspServer server)
    {
        if (_document.Path is null || !server.Matches(_document.Path, _language?.Id))
        {
            return false;
        }

        if (_lsp.IsRunning && string.Equals(_lsp.ServerName, server.Name, StringComparison.Ordinal))
        {
            return true;
        }

        var transport = new LuaLspTransport(server);
        var config = new LspServerConfig
        {
            Name = server.Name,
            LanguageId = _language?.Id ?? server.Filetypes.FirstOrDefault() ?? string.Empty,
            Transport = transport,
        };

        _lspDirty = false;
        if (_lsp.TryStart(config, _document.Path, GetText(), _workspaceRoot, out string error))
        {
            return true;
        }

        HexalPaths.Log($"Lua LSP '{server.Name}' could not start: {error}");
        return false;
    }

    public void StopLuaLsp(string name)
    {
        if (
            _lsp.IsRunning
            && (
                string.IsNullOrEmpty(name)
                || string.Equals(_lsp.ServerName, name, StringComparison.Ordinal)
            )
        )
        {
            _lsp.Stop();
            _pluginStatus.Remove("lsp");
            _lspDirty = false;
        }
    }

    // -----------------------------------------------------------------------
    // IEditorApi
    // -----------------------------------------------------------------------

    public string CurrentBufferId => _document.Path ?? "[No Name]";

    public int TerminalColumns => _columns;

    public int TerminalRows => _rows;

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
        long column = ScalarColumn(_lineBuffer.AsSpan(0, length), (int)(_cursor - lineStart));
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
        long startColumn = ScalarColumn(
            _lineBuffer.AsSpan(0, startLength),
            (int)Math.Clamp(start - startLineStart, 0, startLength)
        );

        int endLength = _document.CopyLineText(endLine, ref _lineBuffer, out _);
        long endColumn = ScalarColumn(
            _lineBuffer.AsSpan(0, endLength),
            (int)Math.Clamp(end - endLineStart, 0, endLength)
        );

        string text = Encoding.UTF8.GetString(_document.Range(start, end - start));
        return new PluginSelection(
            startLine,
            startColumn,
            endLine,
            endColumn,
            text,
            _selectionLinewise
        );
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

    public bool ReplaceRange(object? start, object? finish, string text, out string error)
    {
        error = string.Empty;
        if (
            !TryPluginOffset(start, out long startOffset, out error)
            || !TryPluginOffset(finish, out long endOffset, out error)
        )
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

    public void RequestSelect(string title, IReadOnlyList<string> items, Action<int?> callback)
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

        LspServerConfig selected = config;
        bool primaryPresent = ExecutableFinder.Find(config.Command) is not null;
        bool usingFallback = false;
        LanguageDefinition? language = _languages.Find(config.LanguageId) ?? _language;
        if (
            !primaryPresent
            && !_lspFallbackBlocked
            && language?.FallbackLsp is { } fallback
            && ExecutableFinder.Find(fallback.Command) is not null
        )
        {
            selected = new LspServerConfig
            {
                Name = fallback.Name,
                Command = fallback.Command,
                Arguments = fallback.Arguments,
                LanguageId =
                    config.LanguageId.Length > 0 ? config.LanguageId : fallback.LanguageId,
            };
            usingFallback = true;
        }

        _lspDirty = false;
        bool started = _lsp.TryStart(
            selected,
            _document.Path,
            GetText(),
            _workspaceRoot,
            out error
        );
        if (!started && (primaryPresent || usingFallback))
        {
            // A present executable that fails is a separate error; never mask
            // it by silently switching to the fallback afterwards.
            _lspFallbackBlocked = true;
        }
        return started;
    }

    public void StopLsp(string? name)
    {
        if (
            string.IsNullOrEmpty(name)
            || string.Equals(_lsp.ServerName, name, StringComparison.OrdinalIgnoreCase)
        )
        {
            _lsp.Stop();
            _pluginStatus.Remove("lsp");
            _lspDirty = false;
        }
    }

    public bool ExecuteCommand(string name, IReadOnlyList<string> args, out string error)
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
            case byte
            or sbyte
            or short
            or ushort
            or int
            or uint
            or long
            or ulong
            or float
            or double
            or decimal:
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
                int local = ByteOffsetForScalarColumn(_lineBuffer.AsSpan(0, length), column);
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
