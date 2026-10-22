using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Hexal.Util;
using NLua;
using NLua.Exceptions;

namespace Hexal.Lsp;

/// <summary>
/// The host-side contract used by Lua language servers. The editor implements
/// this interface so that a server started by a plugin can be connected to the
/// active editor buffer without exposing editor internals to Lua.
/// </summary>
public interface ILuaLspHost
{
    string? CurrentDocumentPath { get; }

    string? CurrentLanguageId { get; }

    string? CurrentText { get; }

    string? WorkspaceRoot { get; }

    bool StartLuaLsp(LuaLspServer server);

    void StopLuaLsp(string name);
}

/// <summary>
/// An in-process LSP server implemented by Lua. It owns JSON-RPC routing,
/// lifecycle, request correlation, cancellation, document synchronisation and
/// capability negotiation. Lua owns all language-specific behaviour.
/// </summary>
public sealed class LuaLspServer : IDisposable
{
    private sealed class PendingClientRequest
    {
        public long Id;
        public string Method = string.Empty;
        public LuaFunction? Callback;
        public readonly List<LuaFunction> CompletionCallbacks = new();
        public long? TimerId;
        public bool Completed;
        public bool Cancelled;
        public JsonNode? Result;
        public LspProtocolError? Error;
    }

    private readonly LuaLspCoordinator _coordinator;
    private readonly Lua _lua;
    private readonly Dictionary<string, LuaFunction> _requestHandlers =
        new(StringComparer.Ordinal);
    private readonly Dictionary<string, LuaFunction> _notificationHandlers =
        new(StringComparer.Ordinal);
    private readonly Dictionary<string, List<LuaFunction>> _eventHandlers =
        new(StringComparer.Ordinal);
    private readonly Dictionary<string, CancellationTokenSource> _requestCancellations =
        new(StringComparer.Ordinal);
    private readonly Dictionary<long, PendingClientRequest> _pendingClientRequests =
        new();
    private readonly Dictionary<string, (string ResultId, JsonArray Data)> _tokenCache =
        new(StringComparer.Ordinal);

    private LuaLspTransport? _transport;
    private Action<string>? _outgoing;
    private int _socketGeneration;
    private long _nextClientRequestId = 1;
    private long _nextProgressId = 1;
    private long _nextTokenResultId = 1;
    private bool _initializeReceived;
    private bool _initialized;
    private bool _shuttingDown;
    private bool _disposed;
    private string _trace = "off";

    private LuaLspServer(LuaLspCoordinator coordinator, Lua lua, string name,
        string version)
    {
        _coordinator = coordinator;
        _lua = lua;
        Name = name;
        Version = version;
        State = "created";
        Settings = new JsonObject();
        InitializationOptions = new JsonObject();
        ClientCapabilities = new JsonObject();
        ServerCapabilities = new JsonObject();
        Limits = new JsonObject();
        SettingsTable = coordinator.NewTable();
        InitializationOptionsTable = coordinator.NewTable();
        ServerTable = CreateServerTable();
        Workspace = coordinator.Workspace;
        WorkspaceTable = coordinator.CreateWorkspaceTable(this);
    }

    public string Name { get; }

    public string Version { get; }

    public string State { get; private set; }

    public bool IsRunning => State is "running" or "initializing" or "created";

    public List<string> Filetypes { get; } = new();

    public List<string> Extensions { get; } = new();

    public List<string> RootMarkers { get; } = new();

    public JsonObject InitializationOptions { get; private set; }

    public JsonObject Settings { get; private set; }

    public JsonObject ClientCapabilities { get; private set; }

    public JsonObject Limits { get; private set; }

    public JsonObject ServerCapabilities { get; private set; }

    public LspWorkspace Workspace { get; private set; }

    public LuaTable ServerTable { get; }

    public LuaTable WorkspaceTable { get; private set; }

    public LuaTable SettingsTable { get; private set; }

    public LuaTable InitializationOptionsTable { get; private set; }

    public JsonObject? InitializeResult { get; private set; }

    public static LuaLspServer? Create(LuaLspCoordinator coordinator, LuaTable options)
    {
        string name = options["name"]?.ToString()?.Trim() ?? string.Empty;
        if (name.Length == 0)
        {
            coordinator.Report("lsp.create_server requires a name");
            return null;
        }

        string version = options["version"]?.ToString() ?? string.Empty;
        var server = new LuaLspServer(coordinator, coordinator.Lua, name, version);
        server.ReadOptions(options);
        coordinator.AddServer(server);
        return server;
    }

    private void ReadOptions(LuaTable options)
    {
        foreach (string filetype in ReadStringArray(options["filetypes"]))
        {
            Filetypes.Add(filetype);
        }
        foreach (string extension in ReadStringArray(options["extensions"]))
        {
            Extensions.Add(extension.StartsWith('.') ? extension : "." + extension);
        }
        foreach (string marker in ReadStringArray(options["root_markers"]))
        {
            RootMarkers.Add(marker);
        }

        if (options["initialization_options"] is LuaTable initializationOptions)
        {
            InitializationOptions = _coordinator.AsObject(initializationOptions) ??
                                    new JsonObject();
        }
        else if (options["initializationOptions"] is LuaTable initializationOptionsCamel)
        {
            InitializationOptions = _coordinator.AsObject(initializationOptionsCamel) ??
                                    new JsonObject();
        }

        if (options["settings"] is LuaTable settings)
        {
            Settings = _coordinator.AsObject(settings) ?? new JsonObject();
        }

        InitializationOptionsTable = _coordinator.ToLua(InitializationOptions) as LuaTable ??
                                     _coordinator.NewTable();
        SettingsTable = _coordinator.ToLua(Settings) as LuaTable ?? _coordinator.NewTable();
        if (!ReferenceEquals(InitializationOptionsTable, _coordinator.NullSentinel))
        {
            // Keep the documented table fields available even for empty options.
        }

        if (options["limits"] is LuaTable limits)
        {
            Limits = _coordinator.AsObject(limits) ?? new JsonObject();
        }

        string encoding = options["encoding"]?.ToString() ?? "utf-16";
        _coordinator.Documents.Encoding = LspEncodingUtil.Parse(encoding);
        _trace = options["trace"]?.ToString() ?? "off";

        if (options["permissions"] is LuaTable permissions)
        {
            ReadPermissions(permissions);
        }

        ConfigureWorkspace();

        if (options["capabilities"] is LuaTable capabilities)
        {
            ServerCapabilities = LspCapabilities.Translate(_coordinator.NullSentinel,
                capabilities, _coordinator.SemanticLegend);
        }

        if (options["semantic_tokens"] is LuaTable tokens)
        {
            ReadSemanticLegend(tokens);
        }
    }

    private void ReadPermissions(LuaTable permissions)
    {
        object? filesystem = permissions["filesystem"] ?? permissions["fs"];
        if (filesystem is bool fsEnabled)
        {
            _coordinator.Permissions.ReadFiles = fsEnabled;
            _coordinator.Permissions.WriteFiles = fsEnabled;
        }
        else if (filesystem is string fsMode)
        {
            string mode = fsMode.ToLowerInvariant();
            _coordinator.Permissions.ReadFiles = mode is "read" or "readwrite" or "write";
            _coordinator.Permissions.WriteFiles = mode is "write" or "readwrite";
        }

        if (permissions["read"] is not null)
        {
            _coordinator.Permissions.ReadFiles = LspJson.ToBool(permissions["read"], true);
        }
        if (permissions["write"] is not null)
        {
            _coordinator.Permissions.WriteFiles = LspJson.ToBool(permissions["write"], true);
        }
        if (permissions["process"] is not null)
        {
            _coordinator.Permissions.CreateProcesses =
                LspJson.ToBool(permissions["process"]);
        }
        if (permissions["network"] is not null)
        {
            _coordinator.Permissions.Network = LspJson.ToBool(permissions["network"]);
        }
        if (permissions["workspace_only"] is not null)
        {
            _coordinator.Permissions.WorkspaceOnly =
                LspJson.ToBool(permissions["workspace_only"]);
        }
        if (permissions["workspaceOnly"] is not null)
        {
            _coordinator.Permissions.WorkspaceOnly =
                LspJson.ToBool(permissions["workspaceOnly"]);
        }
        foreach (string root in ReadStringArray(permissions["roots"]))
        {
            _coordinator.Permissions.AllowedRoots.Add(root);
        }
    }

    private void ConfigureWorkspace()
    {
        string? start = _coordinator.Host?.CurrentDocumentPath;
        if (string.IsNullOrEmpty(start))
        {
            string current = _coordinator.Host?.CurrentDocumentPath ?? string.Empty;
            if (current.Length == 0)
            {
                start = _coordinator.PluginDirectory;
            }
        }

        string? root = DiscoverRoot(start);
        if (string.IsNullOrEmpty(root))
        {
            root = _coordinator.Host?.WorkspaceRoot;
        }
        if (string.IsNullOrEmpty(root))
        {
            root = start is not null && File.Exists(start)
                ? Path.GetDirectoryName(start)
                : start;
        }

        Workspace = new LspWorkspace(_coordinator.Documents, _coordinator.Permissions,
            root);
        if (!string.IsNullOrEmpty(root))
        {
            _coordinator.Permissions.AllowedRoots.Add(root);
        }
        WorkspaceTable = _coordinator.CreateWorkspaceTable(this);
    }

    private string? DiscoverRoot(string? start)
    {
        if (string.IsNullOrEmpty(start) || RootMarkers.Count == 0)
        {
            return null;
        }

        string? directory = File.Exists(start) ? Path.GetDirectoryName(start) : start;
        while (!string.IsNullOrEmpty(directory))
        {
            foreach (string marker in RootMarkers)
            {
                string candidate = Path.Combine(directory, marker);
                if (File.Exists(candidate) || Directory.Exists(candidate))
                {
                    return directory;
                }
            }

            DirectoryInfo? parent = Directory.GetParent(directory);
            if (parent is null)
            {
                break;
            }
            directory = parent.FullName;
        }
        return null;
    }

    private static IEnumerable<string> ReadStringArray(object? value)
    {
        if (value is not LuaTable table)
        {
            yield break;
        }

        int index = 1;
        while (true)
        {
            object? item = table[index];
            if (item is null)
            {
                yield break;
            }
            string text = item.ToString() ?? string.Empty;
            if (text.Length > 0)
            {
                yield return text;
            }
            ++index;
        }
    }

    private void ReadSemanticLegend(LuaTable options)
    {
        if (options["legend"] is LuaTable legendTable)
        {
            JsonObject? legend = _coordinator.AsObject(legendTable);
            if (legend is not null)
            {
                ReplaceLegend(legend);
            }
        }
        else if (options["token_types"] is not null || options["tokenTypes"] is not null)
        {
            var legend = new JsonObject
            {
                ["tokenTypes"] = _coordinator.AsArray(
                    options["token_types"] ?? options["tokenTypes"]),
                ["tokenModifiers"] = _coordinator.AsArray(
                    options["token_modifiers"] ?? options["tokenModifiers"])
            };
            ReplaceLegend(legend);
        }
    }

    private void ReplaceLegend(JsonObject legend)
    {
        if (legend["tokenTypes"] is JsonArray types)
        {
            _coordinator.SemanticLegend["tokenTypes"] = types.DeepClone();
        }
        if (legend["tokenModifiers"] is JsonArray modifiers)
        {
            _coordinator.SemanticLegend["tokenModifiers"] = modifiers.DeepClone();
        }
    }

    // -----------------------------------------------------------------------
    // Lua-facing server table
    // -----------------------------------------------------------------------

    private LuaTable CreateServerTable()
    {
        LuaTable table = _coordinator.NewTable();
        table["__hexal_server_name"] = Name;
        table["name"] = new Func<string>(() => Name);
        table["version"] = new Func<string>(() => Version);
        table["start"] = new Func<bool>(Start);
        table["stop"] = new Func<bool>(Stop);
        table["exit"] = new Func<object?, bool>(code =>
        {
            Exit(LspJson.ToInt32(code) ?? 0);
            return true;
        });
        table["state"] = new Func<string>(() => State);
        table["client"] = new Func<LuaTable>(CreateClientTable);
        table["capabilities"] = new Func<object?, bool>(options =>
        {
            if (options is not LuaTable capabilities)
            {
                return false;
            }
            JsonObject translated = LspCapabilities.Translate(_coordinator.NullSentinel,
                capabilities, _coordinator.SemanticLegend);
            MergeCapabilities(translated);
            return true;
        });
        table["advertised_capabilities"] = new Func<LuaTable>(() =>
            _coordinator.ToLua(ServerCapabilities) as LuaTable ?? _coordinator.NewTable());
        table["register"] = new Func<string, LuaFunction, object?, bool>(
            (method, handler, options) => RegisterRequest(method, handler, options));
        table["notify"] = new Func<string, LuaFunction, object?, bool>(
            (method, handler, options) => RegisterNotification(method, handler, options));
        table["on"] = new Func<string, LuaFunction, bool>(On);
        table["off"] = new Func<string, LuaFunction, bool>(Off);
        table["send_request"] = new Func<string, object?, object?, object?, LuaTable>(
            (method, parameters, callback, options) =>
                SendRequest(method, parameters, callback as LuaFunction,
                    options as LuaTable));
        table["send_notification"] = new Func<string, object?, bool>(
            SendNotification);
        table["documents"] = new Func<LuaTable>(() =>
            _coordinator.DocumentsModule ?? _coordinator.NewTable());
        table["workspace"] = new Func<LuaTable>(() => WorkspaceTable);
        _coordinator.WrapTable(table);
        _coordinator.PadTable(table, "exit", 1);
        _coordinator.PadTable(table, "capabilities", 1);
        _coordinator.PadTable(table, "register", 3);
        _coordinator.PadTable(table, "notify", 3);
        _coordinator.PadTable(table, "on", 2);
        _coordinator.PadTable(table, "off", 2);
        _coordinator.PadTable(table, "send_request", 4);
        _coordinator.PadTable(table, "send_notification", 2);
        return table;
    }

    private void MergeCapabilities(JsonObject capabilities)
    {
        foreach ((string key, JsonNode? value) in capabilities)
        {
            ServerCapabilities[key] = value?.DeepClone();
        }
    }

    public IEnumerable<string> RegisteredRequestMethods => _requestHandlers.Keys;

    public IEnumerable<string> RegisteredNotificationMethods => _notificationHandlers.Keys;

    public IEnumerable<long> PendingRequestIds => _pendingClientRequests.Keys;

    public LuaTable CreateClientTable()
    {
        LuaTable table = _coordinator.NewTable();
        table["name"] = "Hexal";
        table["capabilities"] = new Func<LuaTable>(() =>
            _coordinator.ToLua(ClientCapabilities) as LuaTable ?? _coordinator.NewTable());
        table["supports"] = new Func<string, bool>(path =>
            ClientSupports(path));
        table["send_request"] = new Func<string, object?, object?, object?, LuaTable>(
            (method, parameters, callback, options) =>
                SendRequest(method, parameters, callback as LuaFunction,
                    options as LuaTable));
        table["send_notification"] = new Func<string, object?, bool>(
            SendNotification);
        table["register_capability"] = new Func<object?, LuaTable>(options =>
        {
            var parameters = new JsonObject
            {
                ["registrations"] = _coordinator.AsArray(options)
            };
            return SendRequest("client/registerCapability", parameters, null, null);
        });
        table["unregister_capability"] = new Func<object?, LuaTable>(options =>
        {
            var parameters = new JsonObject
            {
                ["unregisterations"] = _coordinator.AsArray(options)
            };
            return SendRequest("client/unregisterCapability", parameters, null, null);
        });
        _coordinator.WrapTable(table);
        _coordinator.PadTable(table, "send_request", 4);
        _coordinator.PadTable(table, "send_notification", 2);
        _coordinator.PadTable(table, "register_capability", 1);
        _coordinator.PadTable(table, "unregister_capability", 1);
        return table;
    }

    public bool Matches(string? documentPath, string? languageId)
    {
        if (Filetypes.Count > 0 && !string.IsNullOrEmpty(languageId))
        {
            foreach (string filetype in Filetypes)
            {
                if (string.Equals(filetype, languageId,
                        StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }
        }

        if (Extensions.Count > 0 && !string.IsNullOrEmpty(documentPath))
        {
            string extension = Path.GetExtension(documentPath);
            foreach (string candidate in Extensions)
            {
                if (string.Equals(candidate, extension,
                        StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }
        }

        return Filetypes.Count == 0 && Extensions.Count == 0;
    }

    public bool ClientSupports(string path)
    {
        JsonNode? current = ClientCapabilities;
        foreach (string part in path.Split('.', StringSplitOptions.RemoveEmptyEntries))
        {
            if (current is not JsonObject obj)
            {
                return false;
            }
            current = obj[part];
        }
        return current is JsonValue value && value.TryGetValue(out bool supported) && supported;
    }

    public bool Start()
    {
        if (_disposed)
        {
            return false;
        }
        if (State == "running" || State == "initializing")
        {
            return true;
        }
        if (State is "shutting_down" or "stopped" or "failed")
        {
            State = "created";
        }

        State = "running";
        _coordinator.AddServer(this);
        _coordinator.Host?.StartLuaLsp(this);
        return true;
    }

    public bool Stop()
    {
        if (_disposed)
        {
            return false;
        }
        if (State == "stopped")
        {
            return true;
        }

        State = "stopped";
        _initialized = false;
        _initializeReceived = false;
        CancelPendingRequests("server stopped");
        _coordinator.Host?.StopLuaLsp(Name);
        return true;
    }

    public void Exit(int code)
    {
        FireEvent("exit", code,
            _coordinator.CreateContext(this, "exit", null, CancellationToken.None));
        State = "stopped";
        _initialized = false;
        _initializeReceived = false;
        CancelPendingRequests("server exited");
        _coordinator.Host?.StopLuaLsp(Name);
    }

    public bool RegisterRequest(string method, LuaFunction handler, object? options)
    {
        if (string.IsNullOrWhiteSpace(method) || handler is null)
        {
            return false;
        }

        bool replace = OptionsRequestReplace(options);
        if (_requestHandlers.ContainsKey(method) && !replace)
        {
            _coordinator.Report($"duplicate request handler for '{method}' rejected");
            return false;
        }
        _requestHandlers[method] = handler;
        return true;
    }

    public bool RegisterNotification(string method, LuaFunction handler, object? options)
    {
        if (string.IsNullOrWhiteSpace(method) || handler is null)
        {
            return false;
        }

        bool replace = OptionsRequestReplace(options);
        if (_notificationHandlers.ContainsKey(method) && !replace)
        {
            _coordinator.Report($"duplicate notification handler for '{method}' rejected");
            return false;
        }
        _notificationHandlers[method] = handler;
        return true;
    }

    private static bool OptionsRequestReplace(object? options)
    {
        return options switch
        {
            bool flag => flag,
            LuaTable table => LspJson.ToBool(table["replace"]),
            _ => false
        };
    }

    public bool On(string eventName, LuaFunction handler)
    {
        if (string.IsNullOrWhiteSpace(eventName) || handler is null)
        {
            return false;
        }
        if (!_eventHandlers.TryGetValue(eventName, out List<LuaFunction>? handlers))
        {
            handlers = new List<LuaFunction>();
            _eventHandlers[eventName] = handlers;
        }
        handlers.Add(handler);
        return true;
    }

    public bool Off(string eventName, LuaFunction handler)
    {
        if (_eventHandlers.TryGetValue(eventName, out List<LuaFunction>? handlers))
        {
            return handlers.Remove(handler);
        }
        return false;
    }

    public LuaTable? FireEvent(string eventName, object? argument, LuaTable? context = null)
    {
        if (!_eventHandlers.TryGetValue(eventName, out List<LuaFunction>? handlers))
        {
            return null;
        }

        LuaTable? result = null;
        foreach (LuaFunction handler in handlers.ToArray())
        {
            try
            {
                object[] values = context is null
                    ? new[] { argument! }
                    : new[] { argument!, context };
                object[] returned = handler.Call(values);
                if (returned.Length > 0 && returned[0] is LuaTable table)
                {
                    result = table;
                }
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                       ArgumentException)
            {
                _coordinator.Report($"event '{eventName}' failed: {ex.Message}");
            }
        }
        return result;
    }

    public void FireWorkspaceChanged(JsonObject item)
    {
        LuaTable context = _coordinator.CreateContext(this, "workspace_changed", null,
            CancellationToken.None);
        FireEvent("workspace_changed", _coordinator.ToLua(item), context);
    }

    // -----------------------------------------------------------------------
    // JSON-RPC input
    // -----------------------------------------------------------------------

    public void SetTransport(LuaLspTransport transport)
    {
        _transport = transport;
        _outgoing = null;
    }

    public void SetOutgoing(Action<string> outgoing)
    {
        _outgoing = outgoing;
        _transport = null;
    }

    private void SendRaw(JsonObject message)
    {
        string text = message.ToJsonString();
        try
        {
            if (_outgoing is not null)
            {
                _outgoing(text);
            }
            else
            {
                _transport?.Enqueue(text);
            }
        }
        catch (Exception ex) when (ex is InvalidOperationException or ObjectDisposedException or
                                   JsonException)
        {
            _coordinator.Report($"cannot send LSP message: {ex.Message}");
        }
    }

    private bool HasClientConnection => _outgoing is not null || _transport is not null;

    public void HandleIncoming(string message)
    {
        if (_disposed)
        {
            return;
        }

        JsonNode? parsed;
        try
        {
            parsed = JsonNode.Parse(message);
        }
        catch (JsonException ex)
        {
            SendError(null, new LspProtocolError(LspErrorCodes.ParseError, ex.Message));
            return;
        }

        if (parsed is not JsonObject root)
        {
            SendError(null, new LspProtocolError(LspErrorCodes.InvalidRequest,
                "JSON-RPC message must be an object"));
            return;
        }

        JsonNode? id = root["id"];
        string method = LspJson.GetString(root["method"]) ?? string.Empty;
        JsonNode? parameters = root["params"];

        if (method.Length > 0)
        {
            if (id is not null)
            {
                HandleRequest(id.DeepClone(), method, parameters);
            }
            else
            {
                HandleNotification(method, parameters);
            }
            return;
        }

        if (id is not null)
        {
            HandleResponse(id, root);
        }
    }

    private void HandleRequest(JsonNode id, string method, JsonNode? parameters)
    {
        if (method == "initialize")
        {
            HandleInitialize(id, parameters);
            return;
        }

        if (method == "shutdown")
        {
            HandleShutdown(id);
            return;
        }

        if (!_initializeReceived)
        {
            SendError(id, new LspProtocolError(LspErrorCodes.ServerNotInitialized,
                "server has not been initialized"));
            return;
        }

        if (_shuttingDown)
        {
            SendError(id, new LspProtocolError(LspErrorCodes.InvalidRequest,
                "server is shutting down"));
            return;
        }

        if (!_requestHandlers.TryGetValue(method, out LuaFunction? handler))
        {
            SendError(id, new LspProtocolError(LspErrorCodes.MethodNotFound,
                $"method not found: {method}"));
            return;
        }

        string key = id.ToJsonString();
        var cancellation = new CancellationTokenSource();
        _requestCancellations[key] = cancellation;
        CancellationToken previous = _coordinator.CurrentCancellation;
        _coordinator.CurrentCancellation = cancellation.Token;
        try
        {
            JsonObject? paramsObject = parameters as JsonObject;
            object? parametersValue = parameters is null
                ? _coordinator.NullSentinel
                : _coordinator.ToLua(parameters);
            LuaTable context = _coordinator.CreateContext(this, method, id,
                cancellation.Token);
            object[] result;
            try
            {
                result = handler.Call(parametersValue, context);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                       ArgumentException)
            {
                if (cancellation.IsCancellationRequested)
                {
                    SendError(id, new LspProtocolError(LspErrorCodes.RequestCancelled,
                        "request cancelled"));
                }
                else
                {
                    _coordinator.Report($"handler '{method}' failed: {ex}");
                    SendError(id, new LspProtocolError(LspErrorCodes.InternalError,
                        ex.Message));
                }
                return;
            }

            if (cancellation.IsCancellationRequested)
            {
                SendError(id, new LspProtocolError(LspErrorCodes.RequestCancelled,
                    "request cancelled"));
                return;
            }

            object? value = result.Length > 0 ? result[0] : null;
            if (LuaLspCoordinator.IsErrorTable(value))
            {
                SendError(id, _coordinator.ToProtocolError((LuaTable)value!));
                return;
            }
            if (value is null && result.Length > 1 &&
                LuaLspCoordinator.IsErrorTable(result[1]))
            {
                SendError(id, _coordinator.ToProtocolError((LuaTable)result[1]!));
                return;
            }

            JsonNode? resultNode = value is null ? null : _coordinator.FromLua(value);
            try
            {
                resultNode = NormalizeResult(method, resultNode, paramsObject);
            }
            catch (LspProtocolException ex)
            {
                SendError(id, new LspProtocolError(LspErrorCodes.InvalidParams,
                    ex.Message));
                return;
            }
            SendResult(id, resultNode);
        }
        finally
        {
            _coordinator.CurrentCancellation = previous;
            _requestCancellations.Remove(key);
            cancellation.Dispose();
        }
    }

    private void HandleNotification(string method, JsonNode? parameters)
    {
        if (method == "$/cancelRequest")
        {
            JsonNode? id = parameters?["id"];
            if (id is not null &&
                _requestCancellations.TryGetValue(id.ToJsonString(),
                    out CancellationTokenSource? cancellation))
            {
                cancellation.Cancel();
            }
            return;
        }

        if (method == "initialized")
        {
            _initialized = true;
            _initializeReceived = true;
            State = "running";
            LuaTable context = _coordinator.CreateContext(this, method, null,
                CancellationToken.None);
            FireEvent("initialized", parameters is null
                ? _coordinator.NullSentinel
                : _coordinator.ToLua(parameters), context);
            return;
        }

        if (method == "exit")
        {
            int code = LspJson.GetInt32(parameters?["code"]) ?? 0;
            Exit(code);
            return;
        }

        if (method == "$/setTrace")
        {
            _trace = LspJson.GetString(parameters?["value"]) ?? "off";
            return;
        }

        if (method == "textDocument/didOpen")
        {
            HandleDidOpen(parameters);
            return;
        }
        if (method == "textDocument/didChange")
        {
            HandleDidChange(parameters);
            return;
        }
        if (method == "textDocument/didSave")
        {
            HandleDidSave(parameters);
            return;
        }
        if (method == "textDocument/didClose")
        {
            HandleDidClose(parameters);
            return;
        }

        if (method == "workspace/didChangeConfiguration")
        {
            Settings = parameters?["settings"] as JsonObject ??
                       parameters?.DeepClone() as JsonObject ?? new JsonObject();
            SettingsTable = _coordinator.ToLua(Settings) as LuaTable ??
                            _coordinator.NewTable();
            LuaTable context = _coordinator.CreateContext(this,
                "configuration_changed", null, CancellationToken.None);
            FireEvent("configuration_changed", SettingsTable, context);
            return;
        }

        if (method == "workspace/didChangeWorkspaceFolders")
        {
            JsonNode? added = parameters?["event"]?["added"];
            JsonNode? removed = parameters?["event"]?["removed"];
            if (added is JsonArray addedArray)
            {
                foreach (JsonNode? item in addedArray)
                {
                    if (item is JsonObject folder)
                    {
                        Workspace.AddFolder(LspJson.GetString(folder["uri"]) ?? string.Empty,
                            LspJson.GetString(folder["name"]) ?? string.Empty);
                    }
                }
            }
            if (removed is JsonArray removedArray)
            {
                foreach (JsonNode? item in removedArray)
                {
                    if (item is JsonObject folder)
                    {
                        Workspace.RemoveFolder(LspJson.GetString(folder["uri"]) ??
                                               string.Empty);
                    }
                }
            }
            FireEvent("workspace_changed", _coordinator.ToLua(parameters),
                _coordinator.CreateContext(this, "workspace_changed", null,
                    CancellationToken.None));
            return;
        }

        if (method == "workspace/didChangeWatchedFiles")
        {
            FireEvent("workspace_changed", _coordinator.ToLua(parameters),
                _coordinator.CreateContext(this, "workspace_changed", null,
                    CancellationToken.None));
            return;
        }

        if (_notificationHandlers.TryGetValue(method, out LuaFunction? handler))
        {
            LuaTable context = _coordinator.CreateContext(this, method, null,
                CancellationToken.None);
            try
            {
                handler.Call(parameters is null
                    ? _coordinator.NullSentinel
                    : _coordinator.ToLua(parameters), context);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                       ArgumentException)
            {
                _coordinator.Report($"notification '{method}' failed: {ex.Message}");
            }
        }
    }

    private void HandleInitialize(JsonNode id, JsonNode? parameters)
    {
        try
        {
            JsonObject paramsObject = parameters as JsonObject ?? new JsonObject();
            ClientCapabilities = paramsObject["capabilities"] as JsonObject ??
                                 new JsonObject();
            if (paramsObject["initializationOptions"] is JsonObject initOptions)
            {
                InitializationOptions = initOptions.DeepClone().AsObject();
            }
            if (paramsObject["settings"] is JsonObject settings)
            {
                Settings = settings.DeepClone().AsObject();
            }

            InitializationOptionsTable = _coordinator.ToLua(InitializationOptions)
                as LuaTable ?? _coordinator.NewTable();
            SettingsTable = _coordinator.ToLua(Settings) as LuaTable ??
                            _coordinator.NewTable();

            JsonNode? folders = paramsObject["workspaceFolders"];
            if (folders is JsonArray folderArray)
            {
                Workspace.ClearFolders();
                foreach (JsonNode? item in folderArray)
                {
                    if (item is JsonObject folder)
                    {
                        Workspace.AddFolder(LspJson.GetString(folder["uri"]) ?? string.Empty,
                            LspJson.GetString(folder["name"]) ?? string.Empty);
                    }
                }
            }
            else if (paramsObject["rootUri"] is JsonValue rootUri &&
                     rootUri.TryGetValue(out string? root))
            {
                string? path = LspUri.ToPath(root);
                if (!string.IsNullOrEmpty(path))
                {
                    Workspace.ReplaceRoot(path);
                }
            }

            if (paramsObject["trace"] is JsonValue traceValue &&
                traceValue.TryGetValue(out string? trace))
            {
                _trace = trace;
            }

            JsonNode? encodings = ClientCapabilities["general"]?["positionEncodings"];
            if (encodings is JsonArray encodingArray)
            {
                foreach (JsonNode? encodingNode in encodingArray)
                {
                    string? name = LspJson.GetString(encodingNode);
                    if (name is not null)
                    {
                        _coordinator.Documents.Encoding = LspEncodingUtil.Parse(name);
                        break;
                    }
                }
            }

            _initializeReceived = true;
            State = "initializing";

            LuaTable context = _coordinator.CreateContext(this, "initialize", id,
                CancellationToken.None);
            LuaTable? handlerResult = FireEvent("initialize",
                _coordinator.ToLua(paramsObject), context);

            JsonObject result;
            if (handlerResult is not null)
            {
                result = _coordinator.AsObject(handlerResult) ?? new JsonObject();
                if (result["capabilities"] is null)
                {
                    result["capabilities"] = ServerCapabilities.DeepClone();
                }
                if (result["serverInfo"] is null)
                {
                    result["serverInfo"] = new JsonObject
                    {
                        ["name"] = Name,
                        ["version"] = Version
                    };
                }
            }
            else
            {
                result = new JsonObject
                {
                    ["capabilities"] = ServerCapabilities.DeepClone(),
                    ["serverInfo"] = new JsonObject
                    {
                        ["name"] = Name,
                        ["version"] = Version
                    }
                };
            }

            InitializeResult = result;
            if (result["capabilities"] is JsonObject advertised)
            {
                MergeCapabilities(advertised);
            }
            SendResult(id, result);
        }
        catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                   ArgumentException or JsonException)
        {
            SendError(id, new LspProtocolError(LspErrorCodes.InternalError, ex.Message));
        }
    }

    private void HandleShutdown(JsonNode id)
    {
        LuaTable context = _coordinator.CreateContext(this, "shutdown", id,
            CancellationToken.None);
        FireEvent("shutdown", _coordinator.NullSentinel, context);
        _shuttingDown = true;
        State = "shutting_down";
        SendResult(id, null);
    }

    private static JsonObject? DocumentParameters(JsonNode? parameters)
    {
        return parameters?["textDocument"] as JsonObject;
    }

    private void HandleDidOpen(JsonNode? parameters)
    {
        JsonObject? textDocument = DocumentParameters(parameters);
        string uri = LspJson.GetString(textDocument?["uri"]) ?? string.Empty;
        if (uri.Length == 0)
        {
            return;
        }

        LspDocument document = _coordinator.Documents.Open(uri,
            LspJson.GetString(textDocument?["languageId"]) ?? string.Empty,
            LspJson.GetInt64(textDocument?["version"]) ?? 0,
            LspJson.GetString(textDocument?["text"]) ?? string.Empty);
        FireDocumentEvent("document_opened", document, parameters);
    }

    private void HandleDidChange(JsonNode? parameters)
    {
        JsonObject? textDocument = DocumentParameters(parameters);
        string uri = LspJson.GetString(textDocument?["uri"]) ?? string.Empty;
        if (uri.Length == 0)
        {
            return;
        }

        long version = LspJson.GetInt64(textDocument?["version"]) ??
                       (_coordinator.Documents.Get(uri)?.Version ?? 0) + 1;
        JsonArray? changes = parameters?["contentChanges"] as JsonArray;
        if (!_coordinator.Documents.Change(uri, changes, version, out string error))
        {
            _coordinator.Report($"didChange rejected for {uri}: {error}");
            return;
        }

        LspDocument? document = _coordinator.Documents.Get(uri);
        if (document is not null)
        {
            FireDocumentEvent("document_changed", document, parameters);
        }
    }

    private void HandleDidSave(JsonNode? parameters)
    {
        JsonObject? textDocument = DocumentParameters(parameters);
        string uri = LspJson.GetString(textDocument?["uri"]) ?? string.Empty;
        if (uri.Length == 0)
        {
            return;
        }

        string? text = LspJson.GetString(parameters?["text"]);
        _coordinator.Documents.Save(uri, text, null);
        LspDocument? document = _coordinator.Documents.Get(uri);
        if (document is not null)
        {
            FireDocumentEvent("document_saved", document, parameters);
        }
    }

    private void HandleDidClose(JsonNode? parameters)
    {
        JsonObject? textDocument = DocumentParameters(parameters);
        string uri = LspJson.GetString(textDocument?["uri"]) ?? string.Empty;
        if (uri.Length == 0)
        {
            return;
        }

        LspDocument? document = _coordinator.Documents.Get(uri);
        _coordinator.Documents.Close(uri);
        _coordinator.Diagnostics.Clear(uri);
        if (document is not null)
        {
            document.Close();
            FireDocumentEvent("document_closed", document, parameters);
        }
    }

    private void FireDocumentEvent(string eventName, LspDocument document,
        JsonNode? parameters)
    {
        LuaTable context = _coordinator.CreateContext(this, eventName, null,
            CancellationToken.None);
        FireEvent(eventName, _coordinator.CreateDocumentTable(document), context);
    }

    private void HandleResponse(JsonNode id, JsonObject root)
    {
        long? numeric = id is JsonValue value &&
                        value.TryGetValue(out long parsedId)
            ? parsedId
            : (long?)null;
        if (numeric is null ||
            !_pendingClientRequests.TryGetValue(numeric.Value,
                out PendingClientRequest? pending))
        {
            return;
        }

        if (root["error"] is JsonObject error)
        {
            int code = LspJson.GetInt32(error["code"]) ?? LspErrorCodes.RequestFailed;
            string message = LspJson.GetString(error["message"]) ?? "request failed";
            CompleteClientRequest(pending, null, new LspProtocolError(code, message,
                error["data"]?.DeepClone()));
        }
        else
        {
            CompleteClientRequest(pending, root["result"]?.DeepClone(), null);
        }
    }

    private JsonNode? NormalizeResult(string method, JsonNode? result,
        JsonObject? parameters)
    {
        if (method is "textDocument/semanticTokens/full" or
            "textDocument/semanticTokens/full/delta" or
            "textDocument/semanticTokens/range")
        {
            return NormalizeSemanticTokens(method, result, parameters);
        }
        return result;
    }

    private JsonNode? NormalizeSemanticTokens(string method, JsonNode? result,
        JsonObject? parameters)
    {
        string uri = LspJson.GetString(parameters?["textDocument"]?["uri"]) ?? string.Empty;
        JsonArray? structured = result as JsonArray;
        if (result is JsonObject obj && obj["data"] is JsonArray data)
        {
            structured = data;
        }

        bool looksStructured = structured is { Count: > 0 } &&
                               structured[0] is JsonObject;
        JsonArray encoded;
        string resultId = string.Empty;
        if (looksStructured && structured is not null)
        {
            if (!TryEncodeSemanticTokens(structured, out encoded, out string error))
            {
                throw new LspProtocolException(error);
            }
            resultId = (_nextTokenResultId++).ToString(CultureInfo.InvariantCulture);
        }
        else if (result is JsonObject existing && existing["data"] is JsonArray existingData)
        {
            encoded = existingData.DeepClone().AsArray();
            resultId = LspJson.GetString(existing["resultId"]) ?? string.Empty;
        }
        else
        {
            return result;
        }

        if (resultId.Length == 0)
        {
            resultId = (_nextTokenResultId++).ToString(CultureInfo.InvariantCulture);
        }

        if (method == "textDocument/semanticTokens/full")
        {
            _tokenCache[uri] = (resultId, encoded.DeepClone().AsArray());
            return new JsonObject
            {
                ["resultId"] = resultId,
                ["data"] = encoded
            };
        }

        if (method == "textDocument/semanticTokens/full/delta")
        {
            if (_tokenCache.TryGetValue(uri, out (string ResultId, JsonArray Data) cache))
            {
                JsonArray edits = BuildTokenDelta(cache.Data, encoded);
                _tokenCache[uri] = (resultId, encoded.DeepClone().AsArray());
                return new JsonObject
                {
                    ["resultId"] = resultId,
                    ["edits"] = edits
                };
            }

            _tokenCache[uri] = (resultId, encoded.DeepClone().AsArray());
            return new JsonObject
            {
                ["resultId"] = resultId,
                ["data"] = encoded
            };
        }

        return new JsonObject
        {
            ["data"] = encoded
        };
    }

    public bool TryEncodeSemanticTokens(JsonArray items, out JsonArray data,
        out string error)
    {
        error = string.Empty;
        data = new JsonArray();
        var tokenTypes = _coordinator.SemanticLegend["tokenTypes"] as JsonArray ??
                         new JsonArray();
        var tokenModifiers = _coordinator.SemanticLegend["tokenModifiers"] as JsonArray ??
                             new JsonArray();

        var parsed = new List<(int Line, int Character, int Length, int Type,
            int Modifiers)>();
        foreach (JsonNode? node in items)
        {
            if (node is not JsonObject item)
            {
                error = "semantic token must be an object";
                return false;
            }

            int line = LspJson.GetInt32(item["line"]) ??
                       LspJson.GetInt32(item["startLine"]) ?? 0;
            int character = LspJson.GetInt32(item["character"]) ??
                            LspJson.GetInt32(item["startCharacter"]) ??
                            LspJson.GetInt32(item["start"]?["character"]) ?? 0;
            int length = LspJson.GetInt32(item["length"]) ?? 0;
            if (length <= 0)
            {
                continue;
            }

            string type = LspJson.GetString(item["type"]) ??
                          LspJson.GetString(item["tokenType"]) ??
                          LspJson.GetString(item["token_type"]) ?? "variable";
            int typeIndex = IndexOf(tokenTypes, type, StringComparison.Ordinal);
            if (typeIndex < 0)
            {
                error = $"unknown semantic token type '{type}'";
                return false;
            }

            int modifiers = 0;
            JsonArray? modifierArray = item["modifiers"] as JsonArray;
            if (modifierArray is not null)
            {
                foreach (JsonNode? modifierNode in modifierArray)
                {
                    string modifier = LspJson.GetString(modifierNode) ?? string.Empty;
                    int modifierIndex = IndexOf(tokenModifiers, modifier,
                        StringComparison.Ordinal);
                    if (modifierIndex < 0)
                    {
                        error = $"unknown semantic token modifier '{modifier}'";
                        return false;
                    }
                    modifiers |= 1 << modifierIndex;
                }
            }

            parsed.Add((Math.Max(0, line), Math.Max(0, character), length, typeIndex,
                modifiers));
        }

        parsed.Sort((left, right) => left.Line == right.Line
            ? left.Character.CompareTo(right.Character)
            : left.Line.CompareTo(right.Line));

        int previousLine = 0;
        int previousCharacter = 0;
        foreach ((int line, int character, int length, int type, int modifiers) in parsed)
        {
            int deltaLine = line - previousLine;
            int deltaCharacter = deltaLine == 0 ? character - previousCharacter : character;
            LspJson.AddNode(data, JsonValue.Create(deltaLine));
            LspJson.AddNode(data, JsonValue.Create(deltaCharacter));
            LspJson.AddNode(data, JsonValue.Create(length));
            LspJson.AddNode(data, JsonValue.Create(type));
            LspJson.AddNode(data, JsonValue.Create(modifiers));
            previousLine = line;
            previousCharacter = character;
        }
        return true;
    }

    private static int IndexOf(JsonArray array, string value,
        StringComparison comparison)
    {
        for (int index = 0; index < array.Count; ++index)
        {
            string? candidate = LspJson.GetString(array[index]);
            if (candidate is not null && string.Equals(candidate, value, comparison))
            {
                return index;
            }
        }
        return -1;
    }

    private static JsonArray BuildTokenDelta(JsonArray previous, JsonArray current)
    {
        int prefix = 0;
        int limit = Math.Min(previous.Count, current.Count);
        while (prefix < limit && JsonNode.DeepEquals(previous[prefix], current[prefix]))
        {
            ++prefix;
        }

        int suffix = 0;
        while (suffix < limit - prefix &&
               JsonNode.DeepEquals(previous[previous.Count - 1 - suffix],
                   current[current.Count - 1 - suffix]))
        {
            ++suffix;
        }

        var replacement = new JsonArray();
        for (int index = prefix; index < current.Count - suffix; ++index)
        {
            LspJson.AddNode(replacement, current[index]?.DeepClone());
        }

        var edits = new JsonArray();
        LspJson.AddNode(edits, new JsonObject
        {
            ["start"] = prefix,
            ["deleteCount"] = previous.Count - prefix - suffix,
            ["data"] = replacement
        });
        return edits;
    }

    public void InvalidateSemanticTokens(string uri)
    {
        if (string.IsNullOrEmpty(uri))
        {
            _tokenCache.Clear();
        }
        else
        {
            _tokenCache.Remove(uri);
        }
    }

    // -----------------------------------------------------------------------
    // Outgoing messages
    // -----------------------------------------------------------------------

    public bool SendNotification(string method, object? parameters)
    {
        if (string.IsNullOrWhiteSpace(method))
        {
            return false;
        }

        JsonNode? paramsNode = parameters is null
            ? null
            : ReferenceEquals(parameters, _coordinator.NullSentinel)
                ? null
                : _coordinator.FromLua(parameters);

        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["method"] = method
        };
        if (paramsNode is not null)
        {
            message["params"] = paramsNode;
        }
        SendRaw(message);
        return true;
    }

    private void SendDiagnosticsNotification(string uri, JsonArray diagnostics)
    {
        LspDocument? document = _coordinator.Documents.Get(uri);
        var parameters = new JsonObject
        {
            ["uri"] = uri,
            ["diagnostics"] = diagnostics.DeepClone()
        };
        if (document is not null)
        {
            parameters["version"] = document.Version;
        }
        SendNotification("textDocument/publishDiagnostics", parameters);
    }

    public void PublishDiagnostics(string uri, JsonArray diagnostics)
    {
        SendDiagnosticsNotification(uri, diagnostics);
    }

    public LuaTable SendRequest(string method, object? parameters,
        LuaFunction? callback, LuaTable? options)
    {
        long id = _nextClientRequestId++;
        var pending = new PendingClientRequest
        {
            Id = id,
            Method = method,
            Callback = callback
        };
        _pendingClientRequests[id] = pending;

        if (!HasClientConnection)
        {
            LuaTable disconnectedHandle = CreateRequestHandle(pending);
            CompleteClientRequest(pending, null, new LspProtocolError(
                LspErrorCodes.RequestFailed, "no LSP client is connected"));
            return disconnectedHandle;
        }

        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id,
            ["method"] = method
        };
        JsonNode? paramsNode = parameters is null
            ? null
            : _coordinator.FromLua(parameters);
        if (paramsNode is not null)
        {
            message["params"] = paramsNode;
        }
        SendRaw(message);

        long timeout = ReadTimeout(options);
        if (timeout > 0)
        {
            pending.TimerId = _coordinator.ScheduleAction(timeout,
                () => HandleRequestTimeout(id));
        }

        return CreateRequestHandle(pending);
    }

    private static long ReadTimeout(LuaTable? options)
    {
        if (options is null)
        {
            return 0;
        }
        return LspJson.ToInt64(options["timeout"] ?? options["timeout_ms"] ??
                               options["timeoutMs"]) ?? 0;
    }

    private LuaTable CreateRequestHandle(PendingClientRequest pending)
    {
        LuaTable table = _coordinator.NewTable();
        table["id"] = new Func<long>(() => pending.Id);
        table["cancel"] = new Func<bool>(() =>
        {
            if (pending.Completed)
            {
                return false;
            }
            pending.Cancelled = true;
            SendNotification("$/cancelRequest", new JsonObject { ["id"] = pending.Id });
            CompleteClientRequest(pending, null, new LspProtocolError(
                LspErrorCodes.RequestCancelled, "request cancelled"));
            return true;
        });
        table["is_pending"] = new Func<bool>(() => !pending.Completed);
        table["result"] = new Func<object?>(() => pending.Result is null
            ? null
            : _coordinator.ToLua(pending.Result));
        table["on_complete"] = new Func<LuaFunction, bool>(callback =>
        {
            if (pending.Completed)
            {
                callback.Call(_coordinator.ToLua(pending.Result),
                    pending.Error is null
                        ? null
                        : _coordinator.CreateErrorTable(pending.Error.Code,
                            pending.Error.Message, pending.Error.Data));
            }
            else
            {
                pending.CompletionCallbacks.Add(callback);
            }
            return true;
        });
        _coordinator.WrapTable(table);
        _coordinator.PadTable(table, "on_complete", 1);
        return table;
    }

    private void CompleteClientRequest(PendingClientRequest pending, JsonNode? result,
        LspProtocolError? error)
    {
        if (pending.Completed)
        {
            return;
        }
        pending.Completed = true;
        pending.Result = result;
        pending.Error = error;
        _pendingClientRequests.Remove(pending.Id);
        if (pending.TimerId is long timer)
        {
            _coordinator.CancelTimer(timer);
        }

        LuaTable? errorTable = error is null
            ? null
            : _coordinator.CreateErrorTable(error.Code, error.Message, error.Data);

        if (pending.Callback is not null)
        {
            try
            {
                pending.Callback.Call(_coordinator.ToLua(result), errorTable);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                       ArgumentException)
            {
                _coordinator.Report($"request callback failed: {ex.Message}");
            }
        }

        foreach (LuaFunction callback in pending.CompletionCallbacks.ToArray())
        {
            try
            {
                callback.Call(_coordinator.ToLua(result), errorTable);
            }
            catch (Exception ex) when (ex is LuaScriptException or InvalidOperationException or
                                       ArgumentException)
            {
                _coordinator.Report($"request completion callback failed: {ex.Message}");
            }
        }
        pending.CompletionCallbacks.Clear();
    }

    private void CancelPendingRequests(string reason)
    {
        foreach (PendingClientRequest pending in _pendingClientRequests.Values.ToArray())
        {
            CompleteClientRequest(pending, null,
                new LspProtocolError(LspErrorCodes.RequestCancelled, reason));
        }
    }

    public void HandleRequestTimeout(long id)
    {
        if (_pendingClientRequests.TryGetValue(id, out PendingClientRequest? pending))
        {
            CompleteClientRequest(pending, null,
                new LspProtocolError(LspErrorCodes.RequestFailed,
                    $"request '{pending.Method}' timed out"));
        }
    }

    private void SendResult(JsonNode id, JsonNode? result)
    {
        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id.DeepClone(),
            ["result"] = result
        };
        SendRaw(message);
    }

    private void SendError(JsonNode? id, LspProtocolError error)
    {
        var errorObject = new JsonObject
        {
            ["code"] = error.Code,
            ["message"] = error.Message
        };
        if (error.Data is not null)
        {
            errorObject["data"] = error.Data.DeepClone();
        }

        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id?.DeepClone(),
            ["error"] = errorObject
        };
        SendRaw(message);
    }

    // -----------------------------------------------------------------------
    // Progress
    // -----------------------------------------------------------------------

    public string NextProgressToken()
    {
        return $"hexal-progress-{Name}-{_nextProgressId++}";
    }

    public LuaTable ProgressForRequest(JsonNode? requestId)
    {
        return _coordinator.CreateProgress(this, NextProgressToken());
    }

    public void BeginProgress(string token, string title, LuaTable? options)
    {
        if (HasClientConnection && _initialized &&
            ClientSupports("window.workDoneProgress"))
        {
            SendRequest("window/workDoneProgress/create",
                new JsonObject { ["token"] = token }, null, null);
        }

        var value = new JsonObject
        {
            ["kind"] = "begin",
            ["title"] = title ?? string.Empty
        };
        ApplyProgressOptions(value, options);
        SendNotification("$/progress", new JsonObject
        {
            ["token"] = token,
            ["value"] = value
        });
    }

    public void ReportProgress(string token, LuaTable? value)
    {
        var report = new JsonObject
        {
            ["kind"] = "report"
        };
        ApplyProgressOptions(report, value);
        SendNotification("$/progress", new JsonObject
        {
            ["token"] = token,
            ["value"] = report
        });
    }

    public void FinishProgress(string token, string? message)
    {
        var value = new JsonObject
        {
            ["kind"] = "end"
        };
        if (!string.IsNullOrEmpty(message))
        {
            value["message"] = message;
        }
        SendNotification("$/progress", new JsonObject
        {
            ["token"] = token,
            ["value"] = value
        });
    }

    private void ApplyProgressOptions(JsonObject target, LuaTable? options)
    {
        if (options is null)
        {
            return;
        }
        JsonObject? source = _coordinator.AsObject(options);
        if (source is null)
        {
            return;
        }
        foreach ((string key, JsonNode? value) in source)
        {
            if (key is "kind" or "token")
            {
                continue;
            }
            target[key] = value?.DeepClone();
        }
    }

    public void OnClientDisconnected()
    {
        _socketGeneration++;
        _initialized = false;
        _initializeReceived = false;
        _shuttingDown = false;
        CancelPendingRequests("client disconnected");
        if (State is "running" or "initializing" or "shutting_down")
        {
            State = "created";
        }
    }

    public void SetTransportAfterReconnect(LuaLspTransport transport)
    {
        _transport = transport;
        _outgoing = null;
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        CancelPendingRequests("server disposed");
        _transport?.Dispose();
        _transport = null;
        _outgoing = null;
    }
}

public sealed class LspProtocolException : Exception
{
    public LspProtocolException(string message) : base(message)
    {
    }
}
