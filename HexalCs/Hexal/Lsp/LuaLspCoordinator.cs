using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Hexal.Util;
using NLua;
using NLua.Exceptions;

namespace Hexal.Lsp;

/// <summary>
/// Per-plugin state shared by every Lua language server created by that
/// plugin: documents, diagnostics, workspace access, timers and async tasks.
/// Lua code is only ever touched on the editor thread.
/// </summary>
public sealed class LuaLspCoordinator : IDisposable
{
    private sealed class TimerRegistration
    {
        public long Id;
        public long DueMs;
        public LuaFunction? Callback;
        public Action? Action;
        public bool Cancelled;
    }

    private readonly List<TimerRegistration> _timers = new();
    private readonly Dictionary<long, LuaFunction> _diagnosticHandlers = new();
    private long _nextTimerId = 1;
    private long _nextDiagnosticHandlerId = 1;
    private long _nextProgressId = 1;
    private bool _disposed;

    public LuaLspCoordinator(Lua lua, string pluginId, string pluginDirectory)
    {
        Lua = lua;
        PluginId = pluginId;
        PluginDirectory = pluginDirectory;
        NullSentinel = LspJson.NewTable(lua);
        JsonNull = NullSentinel;
        Documents = new LspDocumentStore();
        Diagnostics = new LspDiagnosticsRegistry();
        Permissions = new LspPermissionPolicy();
        Workspace = new LspWorkspace(
            Documents,
            Permissions,
            Path.GetDirectoryName(pluginDirectory) is { Length: > 0 } parent
                ? parent
                : pluginDirectory
        );
        Diagnostics.Changed += OnDiagnosticsChanged;
    }

    public Lua Lua { get; }

    public string PluginId { get; }

    public string PluginDirectory { get; }

    public LuaTable NullSentinel { get; }

    public LuaTable JsonNull { get; }

    public LspDocumentStore Documents { get; }

    public LspDiagnosticsRegistry Diagnostics { get; }

    public LspWorkspace Workspace { get; }

    public LspPermissionPolicy Permissions { get; }

    public List<LuaLspServer> Servers { get; } = new();

    public Dictionary<string, LuaTable> Modules { get; } = new(StringComparer.Ordinal);

    public Dictionary<string, LuaTable> ModuleAliases { get; } = new(StringComparer.Ordinal);

    public Action<string>? ReportError { get; set; }

    public ILuaLspHost? Host { get; set; }

    public LuaFunction? WrapFunction { get; set; }

    public LuaFunction? PadFunction { get; set; }

    public LuaTable? DocumentsModule { get; set; }

    public LuaTable? DiagnosticsModule { get; set; }

    public LuaTable? WorkspaceModule { get; set; }

    public bool TraceEnabled { get; set; }

    public CancellationToken CurrentCancellation { get; set; }

    public JsonObject SemanticLegend { get; } = CreateDefaultLegend();

    public LuaLspServer? ActiveServer
    {
        get
        {
            foreach (LuaLspServer server in Servers)
            {
                if (server.IsRunning)
                {
                    return server;
                }
            }
            return Servers.Count > 0 ? Servers[0] : null;
        }
    }

    public LuaTable WrapTable(LuaTable table)
    {
        if (WrapFunction is not null)
        {
            try
            {
                WrapFunction.Call(table);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException)
            {
                Report($"table wrapper failed: {ex.Message}");
            }
        }
        return table;
    }

    public LuaTable PadTable(LuaTable table, string name, int count)
    {
        if (PadFunction is not null)
        {
            try
            {
                PadFunction.Call(table, name, count);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException)
            {
                Report($"table padding failed for '{name}': {ex.Message}");
            }
        }
        return table;
    }

    public LuaTable NewTable() => LspJson.NewTable(Lua);

    public LuaTable NewArray(IEnumerable<object?> values) => LspJson.NewArray(Lua, values);

    public object? ToLua(JsonNode? value) => LspJson.ToLuaValue(Lua, NullSentinel, value);

    public JsonNode? FromLua(object? value) => LspJson.FromLua(NullSentinel, value);

    public void Report(string message)
    {
        ReportError?.Invoke(message);
        HexalPaths.Log($"lua-lsp [{PluginId}]: {message}");
    }

    public void AddServer(LuaLspServer server)
    {
        if (!Servers.Contains(server))
        {
            Servers.Add(server);
        }
    }

    public void RemoveServer(LuaLspServer server)
    {
        Servers.Remove(server);
    }

    public LuaLspServer? FindServer(string name)
    {
        foreach (LuaLspServer server in Servers)
        {
            if (string.Equals(server.Name, name, StringComparison.Ordinal))
            {
                return server;
            }
        }
        return null;
    }

    public void ActivateServers()
    {
        if (Host is null)
        {
            return;
        }

        foreach (LuaLspServer server in Servers.ToArray())
        {
            if (server.IsRunning)
            {
                Host.StartLuaLsp(server);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Timers, async scheduling and file watching
    // -----------------------------------------------------------------------

    public long Schedule(long delayMs, LuaFunction callback)
    {
        if (_disposed || callback is null)
        {
            return 0;
        }

        long id = _nextTimerId++;
        _timers.Add(
            new TimerRegistration
            {
                Id = id,
                DueMs = Environment.TickCount64 + Math.Max(0, delayMs),
                Callback = callback,
            }
        );
        return id;
    }

    public long ScheduleAction(long delayMs, Action action)
    {
        if (_disposed || action is null)
        {
            return 0;
        }

        long id = _nextTimerId++;
        _timers.Add(
            new TimerRegistration
            {
                Id = id,
                DueMs = Environment.TickCount64 + Math.Max(0, delayMs),
                Action = action,
            }
        );
        return id;
    }

    public bool CancelTimer(long id)
    {
        for (int index = 0; index < _timers.Count; ++index)
        {
            if (_timers[index].Id == id)
            {
                _timers[index].Cancelled = true;
                return true;
            }
        }
        return false;
    }

    public void Tick()
    {
        if (_disposed)
        {
            return;
        }

        long now = Environment.TickCount64;
        var due = new List<TimerRegistration>();
        for (int index = _timers.Count - 1; index >= 0; --index)
        {
            TimerRegistration timer = _timers[index];
            if (timer.Cancelled)
            {
                _timers.RemoveAt(index);
                continue;
            }
            if (timer.DueMs <= now)
            {
                _timers.RemoveAt(index);
                due.Add(timer);
            }
        }

        foreach (TimerRegistration timer in due.OrderBy(item => item.DueMs))
        {
            try
            {
                if (timer.Action is not null)
                {
                    timer.Action();
                }
                else
                {
                    timer.Callback?.Call();
                }
            }
            catch (Exception ex)
                when (ex is LuaScriptException or InvalidOperationException or ArgumentException)
            {
                Report($"async callback failed: {ex.Message}");
            }
        }

        var watchEvents = new List<(LspWatchRegistration Watch, JsonObject Event)>();
        Workspace.PollWatches(watchEvents);
        foreach ((LspWatchRegistration watch, JsonObject item) in watchEvents)
        {
            LuaFunction? callback = watch.Callback;
            if (callback is not null && !watch.IsCancelled)
            {
                try
                {
                    callback.Call(ToLua(item));
                }
                catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException)
                {
                    Report($"watch callback failed: {ex.Message}");
                }
            }

            foreach (LuaLspServer server in Servers)
            {
                if (server.IsRunning)
                {
                    server.FireWorkspaceChanged(item);
                }
            }
        }
    }

    public LuaTable CreateTask(LspTaskResult task)
    {
        LuaTable table = NewTable();
        table["is_done"] = new Func<bool>(() => task.IsDone);
        table["is_cancelled"] = new Func<bool>(() => task.IsCancelled);
        table["result"] = new Func<object?>(() => ToLuaTaskResult(task));
        table["error"] = new Func<string>(() => task.Error);
        table["cancel"] = new Func<bool>(() =>
        {
            task.Cancel();
            return true;
        });
        table["on_complete"] = new Func<LuaFunction, bool>(callback =>
        {
            task.OnComplete(result =>
            {
                try
                {
                    callback.Call(ToLuaTaskResult(task), task.Error);
                }
                catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException)
                {
                    Report($"task callback failed: {ex.Message}");
                }
            });
            return true;
        });
        WrapTable(table);
        PadTable(table, "on_complete", 1);
        return table;
    }

    private object? ToLuaTaskResult(LspTaskResult task)
    {
        if (task.IsFailed)
        {
            return null;
        }
        return task.Result switch
        {
            JsonNode node => ToLua(node),
            null => null,
            _ => task.Result,
        };
    }

    public LuaTable CreateWatch(LspWatchRegistration watch, LuaFunction callback)
    {
        watch.Callback = callback;
        LuaTable table = NewTable();
        table["cancel"] = new Func<bool>(() =>
        {
            Workspace.RemoveWatch(watch);
            return true;
        });
        table["is_active"] = new Func<bool>(() => !watch.IsCancelled);
        table["is_cancelled"] = new Func<bool>(() => watch.IsCancelled);
        table["glob"] = watch.Glob;
        WrapTable(table);
        return table;
    }

    public LuaTable CreateProgress(LuaLspServer? server, string token)
    {
        LuaTable table = NewTable();
        table["token"] = token;
        table["begin"] = new Func<string, object?, bool>(
            (nextTitle, options) =>
            {
                if (server is not null)
                {
                    server.BeginProgress(token, nextTitle ?? string.Empty, options as LuaTable);
                }
                return true;
            }
        );
        table["report"] = new Func<object?, bool>(value =>
        {
            if (server is not null)
            {
                server.ReportProgress(token, value as LuaTable);
            }
            return true;
        });
        table["finish"] = new Func<object?, bool>(message =>
        {
            if (server is not null)
            {
                server.FinishProgress(token, message?.ToString());
            }
            return true;
        });
        WrapTable(table);
        PadTable(table, "begin", 2);
        PadTable(table, "report", 1);
        PadTable(table, "finish", 1);
        return table;
    }

    public LuaTable CreateWorkspaceTable(LuaLspServer? server)
    {
        LspWorkspace workspace = server?.Workspace ?? Workspace;
        LuaTable table = NewTable();
        table["root_uri"] = new Func<object?>(() => workspace.RootUri);
        table["folders"] = new Func<LuaTable>(() =>
        {
            var values = new List<object?>();
            foreach ((string uri, string name) in workspace.Folders)
            {
                LuaTable folder = NewTable();
                folder["uri"] = uri;
                folder["name"] = name;
                values.Add(folder);
            }
            return NewArray(values);
        });
        table["configuration"] = new Func<string?, object?>(section =>
            ToLua(workspace.ConfigurationValue(section))
        );
        table["find_files"] = new Func<string, object?, LuaTable>(
            (glob, options) =>
            {
                var task = new LspTaskResult();
                try
                {
                    IReadOnlyList<string> files = workspace.FindFiles(
                        glob,
                        options as LuaTable is null
                            ? null
                            : LspJson.FromLua(NullSentinel, options) as JsonObject
                    );
                    task.Complete(NewArray(files.Cast<object?>()));
                }
                catch (Exception ex)
                {
                    task.Fail(ex.Message);
                }
                return CreateTask(task);
            }
        );
        table["read_file"] = new Func<string, LuaTable>(uri =>
        {
            var task = new LspTaskResult();
            try
            {
                if (Documents.Get(uri) is LspDocument document && document.IsOpen)
                {
                    task.Complete(document.Text);
                }
                else
                {
                    task.Complete(workspace.ReadText(uri));
                }
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return CreateTask(task);
        });
        table["stat"] = new Func<string, LuaTable>(uri =>
        {
            var task = new LspTaskResult();
            try
            {
                task.Complete(ToLua(workspace.Stat(uri)));
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return CreateTask(task);
        });
        table["watch"] = new Func<string, LuaFunction, object?>(
            (glob, callback) =>
            {
                if (callback is null)
                {
                    return null;
                }
                LspWatchRegistration watch = workspace.Watch(glob);
                return CreateWatch(watch, callback);
            }
        );
        table["index"] = new Func<object?, LuaTable>(options =>
        {
            var task = new LspTaskResult();
            LspWorkspaceIndex index = LspWorkspace.CreateIndex(
                options is LuaTable table
                    ? LspJson.FromLua(NullSentinel, table) as JsonObject
                    : null
            );
            task.Complete(CreateIndexTable(index));
            return CreateTask(task);
        });
        table["progress"] = new Func<string, LuaFunction?, LuaTable>(
            (title, callback) =>
            {
                var task = new LspTaskResult();
                string token =
                    server?.NextProgressToken() ?? $"hexal-progress-{PluginId}-{_nextProgressId++}";
                LuaTable progress = CreateProgress(server, token);
                if (server is not null)
                {
                    server.BeginProgress(token, title, null);
                }
                if (callback is not null)
                {
                    try
                    {
                        callback.Call(progress);
                    }
                    catch (Exception ex)
                        when (ex is LuaScriptException or InvalidOperationException)
                    {
                        task.Fail(ex.Message);
                        return CreateTask(task);
                    }
                }
                task.Complete(progress);
                return CreateTask(task);
            }
        );
        WrapTable(table);
        PadTable(table, "configuration", 1);
        PadTable(table, "find_files", 2);
        PadTable(table, "read_file", 1);
        PadTable(table, "stat", 1);
        PadTable(table, "watch", 2);
        PadTable(table, "index", 1);
        PadTable(table, "progress", 2);
        return table;
    }

    public LuaTable CreateIndexTable(LspWorkspaceIndex index)
    {
        LuaTable table = NewTable();
        table["add_file"] = new Func<string, object?, bool>(
            (uri, symbols) =>
            {
                index.AddFile(uri, AsArray(symbols));
                return true;
            }
        );
        table["update_file"] = new Func<string, object?, bool>(
            (uri, symbols) =>
            {
                index.UpdateFile(uri, AsArray(symbols));
                return true;
            }
        );
        table["remove_file"] = new Func<string, bool>(uri => index.RemoveFile(uri));
        table["invalidate"] = new Func<string, bool>(uri =>
        {
            index.Invalidate(uri);
            return true;
        });
        table["clear"] = new Func<bool>(() =>
        {
            index.Clear();
            return true;
        });
        table["query_symbols"] = new Func<string, object?, LuaTable>(
            (query, options) =>
                NewArray(
                    (
                        index.QuerySymbols(query, AsObject(options)) as JsonArray ?? new JsonArray()
                    ).Select(item => (object?)ToLua(item))
                )
        );
        table["query_references"] = new Func<object?, object?, LuaTable>(
            (symbol, options) =>
            {
                JsonObject value = AsObject(symbol) ?? new JsonObject();
                return NewArray(
                    (
                        index.QueryReferences(value, AsObject(options)) as JsonArray
                        ?? new JsonArray()
                    ).Select(item => (object?)ToLua(item))
                );
            }
        );
        table["statistics"] = new Func<LuaTable>(() =>
        {
            LuaTable stats = NewTable();
            stats["files"] = index.FileCount;
            stats["symbols"] = index.SymbolCount;
            return stats;
        });
        WrapTable(table);
        PadTable(table, "add_file", 2);
        PadTable(table, "update_file", 2);
        PadTable(table, "query_symbols", 2);
        PadTable(table, "query_references", 2);
        return table;
    }

    public JsonArray AsArray(object? value)
    {
        JsonNode? node = LspJson.FromLua(NullSentinel, value);
        return node as JsonArray ?? new JsonArray();
    }

    public JsonObject? AsObject(object? value)
    {
        return LspJson.FromLua(NullSentinel, value) as JsonObject;
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

    private void OnDiagnosticsChanged(string uri, JsonArray items)
    {
        LuaLspServer? server = ActiveServer;
        server?.PublishDiagnostics(uri, items);

        foreach (LuaFunction callback in _diagnosticHandlers.Values.ToArray())
        {
            try
            {
                callback.Call(uri, ToLua(items));
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException)
            {
                Report($"diagnostics callback failed: {ex.Message}");
            }
        }
    }

    public long AddDiagnosticsHandler(LuaFunction callback)
    {
        long id = _nextDiagnosticHandlerId++;
        _diagnosticHandlers[id] = callback;
        return id;
    }

    public bool RemoveDiagnosticsHandler(long id) => _diagnosticHandlers.Remove(id);

    // -----------------------------------------------------------------------
    // Document and snapshot tables
    // -----------------------------------------------------------------------

    public LuaTable CreateDocumentTable(LspDocument document)
    {
        LuaTable table = NewTable();
        table["uri"] = document.Uri;
        table["language_id"] = document.LanguageId;
        table["languageId"] = document.LanguageId;
        table["version"] = document.Version;
        table["text"] = document.Text;
        table["line_count"] = document.LineCount;
        table["lineCount"] = document.LineCount;
        table["is_open"] = document.IsOpen;
        table["isOpen"] = document.IsOpen;
        table["is_dirty"] = document.IsDirty;
        table["isDirty"] = document.IsDirty;
        table["encoding"] = LspEncodingUtil.ToProtocolName(document.Encoding);

        table["get_text"] = new Func<string>(() => document.Text);
        table["get_line"] = new Func<object?, object?>(
            (lineValue) =>
            {
                long? line = LspJson.ToInt64(lineValue);
                if (line is null || line < 0 || line >= document.LineCount)
                {
                    return null;
                }
                return document.GetLine((int)line.Value);
            }
        );
        table["get_range"] = new Func<object?, string>(range =>
            document.GetRange(range is LuaTable table ? AsObject(table) : null)
        );
        table["word_at"] = new Func<object?, object?>(
            (positionValue) =>
            {
                JsonObject? position = positionValue is LuaTable table ? AsObject(table) : null;
                return WordAt(document, position);
            }
        );
        table["word_range_at"] = new Func<object?, object?>(
            (positionValue) =>
            {
                JsonObject? position = positionValue is LuaTable table ? AsObject(table) : null;
                return WordRangeAt(document, position);
            }
        );
        table["position_to_offset"] = new Func<object?, long>(
            (positionValue) =>
            {
                JsonObject? position = positionValue is LuaTable table ? AsObject(table) : null;
                int line = LspJson.GetInt32(position?["line"]) ?? 0;
                int character = LspJson.GetInt32(position?["character"]) ?? 0;
                return Encoding.UTF8.GetByteCount(
                    document.Text.AsSpan(0, document.CharOffset(line, character))
                );
            }
        );
        table["offset_to_position"] = new Func<object?, LuaTable>(
            (offsetValue) =>
            {
                long offset = LspJson.ToInt64(offsetValue) ?? 0;
                (int line, int character) = ByteOffsetToPosition(document, offset);
                LuaTable position = NewTable();
                position["line"] = line;
                position["character"] = character;
                return position;
            }
        );
        table["get_changes_since"] = new Func<object?, LuaTable>(
            (versionValue) =>
                NewArray(
                    (
                        document.ChangesSince(LspJson.ToInt64(versionValue) ?? 0) as JsonArray
                        ?? new JsonArray()
                    ).Select(item => (object?)ToLua(item))
                )
        );
        table["snapshot"] = new Func<LuaTable>(() => CreateSnapshotTable(document.Snapshot()));
        table["is_open"] = new Func<bool>(() => document.IsOpen);
        table["is_dirty"] = new Func<bool>(() => document.IsDirty);
        WrapTable(table);
        PadTable(table, "get_line", 1);
        PadTable(table, "get_range", 1);
        PadTable(table, "word_at", 1);
        PadTable(table, "word_range_at", 1);
        PadTable(table, "position_to_offset", 1);
        PadTable(table, "offset_to_position", 1);
        PadTable(table, "get_changes_since", 1);
        return table;
    }

    public LuaTable CreateSnapshotTable(LspSnapshot snapshot)
    {
        LuaTable table = NewTable();
        table["uri"] = snapshot.Uri;
        table["version"] = snapshot.Version;
        table["text"] = snapshot.Text;
        table["line_count"] = snapshot.LineCount;
        table["lineCount"] = snapshot.LineCount;
        table["encoding"] = LspEncodingUtil.ToProtocolName(snapshot.Encoding);
        table["get_text"] = new Func<string>(() => snapshot.Text);
        table["get_line"] = new Func<object?, object?>(
            (lineValue) =>
            {
                long? line = LspJson.ToInt64(lineValue);
                if (line is null || line < 0 || line >= snapshot.LineCount)
                {
                    return null;
                }
                return LspTextIndex.LineText(snapshot.Text, snapshot.LineStarts, (int)line.Value);
            }
        );
        table["get_range"] = new Func<object?, string>(range =>
            SnapshotRange(snapshot, range is LuaTable table ? AsObject(table) : null)
        );
        WrapTable(table);
        PadTable(table, "get_line", 1);
        PadTable(table, "get_range", 1);
        return table;
    }

    private static string SnapshotRange(LspSnapshot snapshot, JsonObject? range)
    {
        if (range is null)
        {
            return string.Empty;
        }
        int startLine = LspJson.GetInt32(range["start"]?["line"]) ?? 0;
        int startCharacter = LspJson.GetInt32(range["start"]?["character"]) ?? 0;
        int endLine = LspJson.GetInt32(range["end"]?["line"]) ?? 0;
        int endCharacter = LspJson.GetInt32(range["end"]?["character"]) ?? 0;
        int start = LspTextIndex.PositionToCharOffset(
            snapshot.Text,
            snapshot.LineStarts,
            startLine,
            startCharacter,
            snapshot.Encoding
        );
        int end = LspTextIndex.PositionToCharOffset(
            snapshot.Text,
            snapshot.LineStarts,
            endLine,
            endCharacter,
            snapshot.Encoding
        );
        if (end < start)
        {
            (start, end) = (end, start);
        }
        return snapshot.Text[start..end];
    }

    private static (int Line, int Character) ByteOffsetToPosition(
        LspDocument document,
        long byteOffset
    )
    {
        byte[] bytes = Encoding.UTF8.GetBytes(document.Text);
        int offset = (int)Math.Clamp(byteOffset, 0, bytes.Length);
        while (
            offset > 0
            && (bytes[offset - 1] & 0xC0) == 0x80
            && (offset >= bytes.Length || (bytes[offset] & 0xC0) != 0x80)
        )
        {
            --offset;
        }
        int charOffset = Encoding.UTF8.GetCharCount(bytes, 0, offset);
        return document.Position(charOffset);
    }

    private static string? WordAt(LspDocument document, JsonObject? position)
    {
        if (position is null)
        {
            return null;
        }
        int line = LspJson.GetInt32(position["line"]) ?? 0;
        int character = LspJson.GetInt32(position["character"]) ?? 0;
        string text = document.GetLine(line);
        int index = LspEncodingUtil.ColumnToCharIndex(text, character, document.Encoding);
        (int start, int end) = WordBounds(text, index);
        return end > start ? text[start..end] : null;
    }

    private LuaTable? WordRangeAt(LspDocument document, JsonObject? position)
    {
        if (position is null)
        {
            return null;
        }
        int line = LspJson.GetInt32(position["line"]) ?? 0;
        int character = LspJson.GetInt32(position["character"]) ?? 0;
        string text = document.GetLine(line);
        int index = LspEncodingUtil.ColumnToCharIndex(text, character, document.Encoding);
        (int start, int end) = WordBounds(text, index);
        if (end <= start)
        {
            return null;
        }

        LuaTable range = NewTable();
        LuaTable startPosition = NewTable();
        startPosition["line"] = line;
        startPosition["character"] = LspEncodingUtil.CharIndexToColumn(
            text,
            start,
            document.Encoding
        );
        LuaTable endPosition = NewTable();
        endPosition["line"] = line;
        endPosition["character"] = LspEncodingUtil.CharIndexToColumn(text, end, document.Encoding);
        range["start"] = startPosition;
        range["end"] = endPosition;
        return range;
    }

    private static (int Start, int End) WordBounds(string text, int index)
    {
        int start = Math.Clamp(index, 0, text.Length);
        int end = start;
        while (start > 0 && IsWordCharacter(text[start - 1]))
        {
            --start;
        }
        while (end < text.Length && IsWordCharacter(text[end]))
        {
            ++end;
        }
        return (start, end);
    }

    private static bool IsWordCharacter(char character)
    {
        return char.IsLetterOrDigit(character) || character == '_' || character == '$';
    }

    // -----------------------------------------------------------------------
    // Context and errors
    // -----------------------------------------------------------------------

    public LuaTable CreateErrorTable(int code, string message, JsonNode? data)
    {
        LuaTable table = NewTable();
        table["__hexal_lsp_error"] = true;
        table["code"] = code;
        table["message"] = message;
        table["data"] = ToLua(data);
        return table;
    }

    public static bool IsErrorTable(object? value)
    {
        return value is LuaTable table && LspJson.ToBool(table["__hexal_lsp_error"]);
    }

    public LspProtocolError ToProtocolError(LuaTable table)
    {
        int code = LspJson.ToInt32(table["code"]) ?? LspErrorCodes.InternalError;
        string message = table["message"]?.ToString() ?? string.Empty;
        JsonNode? data = FromLua(table["data"]);
        return new LspProtocolError(code, message, data);
    }

    public LuaTable CreateContext(
        LuaLspServer server,
        string method,
        JsonNode? requestId,
        CancellationToken cancellationToken
    )
    {
        LuaTable context = NewTable();
        context["server"] = server.ServerTable;
        context["client"] = server.CreateClientTable();
        context["documents"] = DocumentsModule;
        context["workspace"] = server.WorkspaceTable;
        context["method"] = method;
        if (requestId is not null)
        {
            context["request_id"] = ToLua(requestId);
        }
        context["initialization_options"] = ToLua(server.InitializationOptions);
        context["settings"] = ToLua(server.Settings);
        context["cancelled"] = new Func<bool>(() => cancellationToken.IsCancellationRequested);
        context["is_cancelled"] = context["cancelled"];
        context["progress"] = server.ProgressForRequest(requestId);
        context["reject"] = new Func<int, string, object?, LuaTable>(
            (code, message, data) =>
                CreateErrorTable(
                    code,
                    message ?? string.Empty,
                    data is LuaTable table ? FromLua(table) : FromLua(data)
                )
        );
        WrapTable(context);
        PadTable(context, "reject", 3);
        return context;
    }

    private static JsonObject CreateDefaultLegend()
    {
        return new JsonObject
        {
            ["tokenTypes"] = LspJson.StringArray(
                "namespace",
                "type",
                "class",
                "enum",
                "interface",
                "struct",
                "typeParameter",
                "parameter",
                "variable",
                "property",
                "enumMember",
                "event",
                "function",
                "method",
                "macro",
                "keyword",
                "modifier",
                "comment",
                "string",
                "number",
                "regexp",
                "operator",
                "decorator"
            ),
            ["tokenModifiers"] = LspJson.StringArray(
                "declaration",
                "definition",
                "readonly",
                "static",
                "deprecated",
                "abstract",
                "async",
                "modification",
                "documentation",
                "defaultLibrary"
            ),
        };
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        Diagnostics.Changed -= OnDiagnosticsChanged;
        foreach (LuaLspServer server in Servers.ToArray())
        {
            server.Dispose();
        }
        Servers.Clear();
        _timers.Clear();
        Workspace.ClearWatches();
    }
}
