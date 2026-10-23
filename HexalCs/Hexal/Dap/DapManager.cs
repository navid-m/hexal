using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Hexal.Lsp;
using Hexal.Util;

namespace Hexal.Dap;

public enum DapEventKind
{
    Started,
    Initialized,
    Stopped,
    StoppedLocation,
    Continued,
    Exited,
    Terminated,
    Output,
    Breakpoints,
    Scopes,
    Variables,
    WatchValue,
    Error
}

public sealed class DapEvent
{
    public DapEventKind Kind { get; init; }
    public string AdapterName { get; init; } = string.Empty;
    public string Message { get; init; } = string.Empty;
    public string Category { get; init; } = string.Empty;
    public string Output { get; init; } = string.Empty;
    public string Path { get; init; } = string.Empty;
    public int Line { get; init; } = -1;
    public int ThreadId { get; init; }
    public int ExitCode { get; init; }
    public string Reason { get; init; } = string.Empty;
    public IReadOnlyList<DapBreakpoint> Breakpoints { get; init; } =
        Array.Empty<DapBreakpoint>();
    public IReadOnlyList<DapScope> Scopes { get; init; } =
        Array.Empty<DapScope>();
    public IReadOnlyList<DapScopeVariables> ScopedVariables { get; init; } =
        Array.Empty<DapScopeVariables>();
    public DapWatchValue? Watch { get; init; }
}

public sealed record DapBreakpoint(int Line, bool Verified, string Message);

/// <summary>
/// Minimal Debug Adapter Protocol client. It starts an adapter over stdio,
/// performs the initialize/launch/configurationDone handshake and forwards
/// stopped/exited/output notifications to the editor. This is intentionally
/// small and experimental: it supports launch/attach, breakpoints, continue,
/// pause and the three stepping commands.
/// </summary>
public sealed class DapManager : IDisposable
{
    private const int MaxLogEntries = 2000;
    private const int MaxLogEntryLength = 16 * 1024;

    private readonly object _stateLock = new();
    private readonly object _writeLock = new();
    private readonly object _logLock = new();
    private readonly ConcurrentQueue<DapEvent> _events = new();
    private readonly Dictionary<long, PendingRequest> _pendingRequests = new();
    private readonly Dictionary<string, SortedSet<int>> _breakpoints =
        new(StringComparer.Ordinal);
    private readonly List<DapScope> _currentScopes = new();
    private readonly Dictionary<int, List<DapVariable>> _variablesByReference = new();
    private readonly Queue<string> _log = new();

    private Process? _process;
    private Stream? _input;
    private CancellationTokenSource? _cancellation;
    private Task? _readerTask;
    private Task? _stderrTask;

    private DebuggerConfig? _config;
    private JsonObject? _launchConfiguration;
    private string _adapterName = string.Empty;
    private string _adapterPath = string.Empty;
    private string _sourcePath = string.Empty;
    private string _workspaceRoot = string.Empty;
    private string _stoppedPath = string.Empty;
    private string _stopReason = string.Empty;
    private long _nextSequence;
    private long _stoppedFrameId;
    private int _stoppedThreadId;
    private int _stoppedLine = -1;
    private int _variablesGeneration;
    private int _pendingVariablesResponses;
    private bool _initialized;
    private bool _adapterInitialized;
    private bool _launchSent;
    private bool _configurationDone;
    private bool _stopped;
    private bool _disposed;

    private sealed record PendingRequest(
        string Command,
        int Generation = 0,
        int VariablesReference = 0,
        string ScopeName = "",
        string Expression = "");

    public bool IsRunning
    {
        get
        {
            lock (_stateLock)
            {
                return _process is not null;
            }
        }
    }

    public bool IsInitialized
    {
        get
        {
            lock (_stateLock)
            {
                return _initialized;
            }
        }
    }

    public bool IsStopped
    {
        get
        {
            lock (_stateLock)
            {
                return _stopped;
            }
        }
    }

    public string AdapterName
    {
        get
        {
            lock (_stateLock)
            {
                return _adapterName;
            }
        }
    }

    public bool TryStart(DebuggerConfig config, string sourcePath,
        string workspaceRoot, JsonObject launchConfiguration,
        IReadOnlyDictionary<string, SortedSet<int>> breakpoints,
        out string error)
    {
        error = string.Empty;

        if (string.IsNullOrWhiteSpace(config.Adapter))
        {
            error = "debug adapter is empty";
            return false;
        }

        string? adapter = FindAdapter(config.Adapter);
        if (adapter is null)
        {
            error = $"debug adapter not found in PATH: {config.Adapter}";
            return false;
        }

        string sourceFull;
        string workspaceFull;
        try
        {
            sourceFull = Path.GetFullPath(sourcePath);
            workspaceFull = !string.IsNullOrWhiteSpace(workspaceRoot) &&
                            Directory.Exists(workspaceRoot)
                ? Path.GetFullPath(workspaceRoot)
                : Path.GetDirectoryName(sourceFull) ?? ".";
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            error = ex.Message;
            return false;
        }

        lock (_stateLock)
        {
            StopLocked();

            _config = config;
            _adapterName = config.Adapter;
            _adapterPath = adapter;
            _sourcePath = sourceFull;
            _workspaceRoot = workspaceFull;
            _launchConfiguration =
                (JsonObject)launchConfiguration.DeepClone();
            _breakpoints.Clear();
            foreach ((string path, SortedSet<int> lines) in breakpoints)
            {
                _breakpoints[path] = new SortedSet<int>(lines);
            }

            _pendingRequests.Clear();
            _nextSequence = 0;
            _initialized = false;
            _adapterInitialized = false;
            _launchSent = false;
            _configurationDone = false;
            _stopped = false;
            _stoppedThreadId = 0;
            _stoppedPath = string.Empty;
            _stoppedLine = -1;
            _stopReason = string.Empty;
            ClearDebugStateLocked();

            var startInfo = new ProcessStartInfo
            {
                FileName = adapter,
                WorkingDirectory = workspaceFull,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                StandardErrorEncoding = Encoding.UTF8
            };
            foreach (string argument in config.Arguments)
            {
                startInfo.ArgumentList.Add(argument);
            }

            string arguments = string.Join(' ', config.Arguments);
            LogInfo($"starting {config.Adapter}: {adapter}" +
                    (arguments.Length > 0 ? $" {arguments}" : string.Empty) +
                    $" (cwd {workspaceFull})");

            try
            {
                _process = Process.Start(startInfo);
                if (_process is null)
                {
                    error = $"failed to start debug adapter: {config.Adapter}";
                    StopLocked();
                    return false;
                }

                int processId = -1;
                try
                {
                    processId = _process.Id;
                }
                catch (Exception ex) when (ex is InvalidOperationException or
                                           NotSupportedException)
                {
                    processId = -1;
                }

                LogInfo(processId > 0
                    ? $"adapter process started (pid {processId})"
                    : "adapter process started");

                _input = _process.StandardInput.BaseStream;
                _cancellation = new CancellationTokenSource();
                CancellationToken token = _cancellation.Token;
                _readerTask = Task.Run(() =>
                    ReadLoop(_process.StandardOutput.BaseStream, token));
                _stderrTask = Task.Run(() =>
                    ReadStandardError(_process.StandardError, token));

                SendInitializeLocked();
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Started,
                    AdapterName = _adapterName
                });
                HexalPaths.Log(
                    $"DAP started: {config.Adapter} ({adapter}) for {sourceFull}");
                return true;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       InvalidOperationException or NotSupportedException or
                                       System.ComponentModel.Win32Exception)
            {
                error = $"cannot start debug adapter '{config.Adapter}': {ex.Message}";
                StopLocked();
                return false;
            }
        }
    }

    public void SetBreakpoints(string path, IEnumerable<int> lines)
    {
        string fullPath = NormalizePath(path);
        lock (_stateLock)
        {
            var set = new SortedSet<int>();
            foreach (int line in lines)
            {
                if (line >= 0)
                {
                    set.Add(line);
                }
            }

            if (set.Count == 0)
            {
                _breakpoints.Remove(fullPath);
            }
            else
            {
                _breakpoints[fullPath] = set;
            }

            if (_process is not null && _initialized && _configurationDone)
            {
                SendBreakpointsLocked(fullPath);
            }
        }
    }

    public bool Continue() => SendThreadRequest("continue", clearStopped: true);

    public bool Next() => SendThreadRequest("next", clearStopped: true);

    public bool StepIn() => SendThreadRequest("stepIn", clearStopped: true);

    public bool StepOut() => SendThreadRequest("stepOut", clearStopped: true);

    public bool Pause() => SendThreadRequest("pause", clearStopped: false);

    /// <summary>
    /// Requests the scopes and variables for the current stopped frame. The
    /// manager follows up with one variables request per scope and emits a
    /// combined Variables event when every response has arrived.
    /// </summary>
    public bool RequestDebugVariables()
    {
        lock (_stateLock)
        {
            if (_process is null || !_initialized || !_stopped ||
                _stoppedFrameId <= 0)
            {
                return false;
            }

            int generation = ++_variablesGeneration;
            _currentScopes.Clear();
            _variablesByReference.Clear();
            _pendingVariablesResponses = 0;
            SendRequestLocked(new PendingRequest("scopes", generation),
                new JsonObject
                {
                    ["frameId"] = _stoppedFrameId
                });
            return true;
        }
    }

    public bool RequestEvaluate(string expression)
    {
        if (string.IsNullOrWhiteSpace(expression))
        {
            return false;
        }

        lock (_stateLock)
        {
            if (_process is null || !_initialized || !_stopped ||
                _stoppedFrameId <= 0)
            {
                return false;
            }

            SendRequestLocked(new PendingRequest("evaluate", _variablesGeneration,
                Expression: expression), new JsonObject
                {
                    ["expression"] = expression,
                    ["frameId"] = _stoppedFrameId,
                    ["context"] = "watch"
                });
            return true;
        }
    }

    public void Stop()
    {
        lock (_stateLock)
        {
            if (_process is not null && _initialized)
            {
                SendRequestLocked("disconnect", new JsonObject
                {
                    ["terminateDebuggee"] = true
                });
            }
            StopLocked();
        }
    }

    public bool TryDequeueEvent(out DapEvent dapEvent)
        => _events.TryDequeue(out dapEvent!);

    public string BuildDebugReport()
    {
        string adapterName;
        string adapterPath;
        string arguments;
        string workspace;
        string source;
        bool running;
        bool initialized;
        bool configurationDone;
        bool stopped;
        int stoppedThread;
        string stoppedPath;
        int stoppedLine;
        int pendingCount;
        string pendingMethods;
        int processId = -1;

        lock (_stateLock)
        {
            adapterName = _adapterName;
            adapterPath = _adapterPath;
            arguments = _config is null
                ? string.Empty
                : string.Join(' ', _config.Arguments);
            workspace = _workspaceRoot;
            source = _sourcePath;
            running = _process is not null;
            initialized = _initialized;
            configurationDone = _configurationDone;
            stopped = _stopped;
            stoppedThread = _stoppedThreadId;
            stoppedPath = _stoppedPath;
            stoppedLine = _stoppedLine;
            pendingCount = _pendingRequests.Count;
            pendingMethods = string.Join(", ", _pendingRequests.Values
                .Select(request => request.Command)
                .Distinct(StringComparer.Ordinal));

            if (_process is not null)
            {
                try
                {
                    processId = _process.Id;
                }
                catch (Exception ex) when (ex is InvalidOperationException or
                                           NotSupportedException)
                {
                    processId = -1;
                }
            }
        }

        var builder = new StringBuilder();
        builder.AppendLine("Hexal DAP diagnostics");
        builder.Append("Generated: ").AppendLine(
            DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff"));
        builder.Append("Adapter: ").AppendLine(adapterName.Length > 0
            ? adapterName
            : "(none)");
        builder.Append("Command: ").AppendLine(adapterPath.Length > 0
            ? adapterPath
            : "(none)");
        builder.Append("Arguments: ").AppendLine(arguments.Length > 0
            ? arguments
            : "(none)");
        builder.Append("State: ")
            .Append(running ? "running" : "stopped")
            .Append(initialized ? ", initialized" : ", not initialized")
            .Append(configurationDone ? ", configured" : ", not configured")
            .Append(stopped ? ", paused" : ", running")
            .AppendLine();
        builder.Append("Process ID: ").AppendLine(processId > 0
            ? processId.ToString()
            : "(none)");
        builder.Append("Workspace: ").AppendLine(workspace);
        builder.Append("Source: ").AppendLine(source);
        builder.Append("Stopped thread: ").AppendLine(stoppedThread.ToString());
        builder.Append("Stop location: ").AppendLine(stoppedLine >= 0
            ? $"{stoppedPath}:{stoppedLine + 1}"
            : "(none)");
        builder.Append("Pending requests: ").Append(pendingCount);
        if (pendingMethods.Length > 0)
        {
            builder.Append("  ").Append(pendingMethods);
        }
        builder.AppendLine();
        builder.Append("Breakpoints: ").AppendLine(_breakpoints.Count.ToString());
        foreach ((string path, SortedSet<int> lines) in _breakpoints)
        {
            builder.Append("  ").Append(path).Append(": ")
                .AppendLine(string.Join(", ", lines.Select(line => line + 1)));
        }
        builder.Append("File log: ").AppendLine(HexalPaths.LogPath);
        builder.AppendLine();

        lock (_logLock)
        {
            builder.Append("Log entries: ").AppendLine(_log.Count.ToString());
            builder.AppendLine("----- DAP log (oldest first) -----");
            foreach (string entry in _log)
            {
                builder.AppendLine(entry);
            }
        }

        return builder.ToString();
    }

    public void ClearLog()
    {
        lock (_logLock)
        {
            _log.Clear();
        }
        LogInfo("log cleared");
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        lock (_stateLock)
        {
            StopLocked();
        }
    }

    /// <summary>
    /// Resolves a configured adapter name to an executable. This includes the
    /// version-suffixed LLVM fallback used by lldb-dap installations.
    /// </summary>
    public static string? FindAdapter(string adapter)
    {
        string? path = ExecutableFinder.Find(adapter);
        if (path is not null)
        {
            return path;
        }

        // LLVM installs version-suffixed DAP binaries on several distributions
        // (for example lldb-dap-18). Prefer the newest one we can find.
        if (string.Equals(adapter, "lldb-dap", StringComparison.OrdinalIgnoreCase))
        {
            for (int version = 20; version >= 14; --version)
            {
                path = ExecutableFinder.Find($"lldb-dap-{version}");
                if (path is not null)
                {
                    return path;
                }
            }
        }

        return null;
    }

    private void SendInitializeLocked()
    {
        SendRequestLocked("initialize", new JsonObject
            {
                ["clientID"] = "hexal",
                ["clientName"] = "Hexal",
                ["adapterID"] = _adapterName,
                ["pathFormat"] = "path",
                ["linesStartAt1"] = true,
                ["columnsStartAt1"] = true,
                ["supportsVariableType"] = true,
                ["supportsVariablePaging"] = true,
                ["supportsRunInTerminalRequest"] = false,
                ["locale"] = "en-US"
            });
    }

    private void HandleInitializeResponseLocked()
    {
        _initialized = true;
        LogInfo("initialize response received");

        JsonObject launch = _launchConfiguration is null
            ? new JsonObject()
            : (JsonObject)_launchConfiguration.DeepClone();
        string request = "launch";
        if (launch["request"] is JsonValue requestValue &&
            requestValue.TryGetValue(out string? configuredRequest) &&
            !string.IsNullOrWhiteSpace(configuredRequest))
        {
            request = configuredRequest!;
        }
        if (!launch.ContainsKey("request"))
        {
            launch["request"] = request;
        }

        _launchSent = true;
        SendRequestLocked(request, launch);
        ConfigureAdapterIfReadyLocked();
    }

    private void HandleInitializedEventLocked()
    {
        _adapterInitialized = true;
        Enqueue(new DapEvent
        {
            Kind = DapEventKind.Initialized,
            AdapterName = _adapterName
        });
        ConfigureAdapterIfReadyLocked();
    }

    private void ConfigureAdapterIfReadyLocked()
    {
        if (!_initialized || !_launchSent || !_adapterInitialized ||
            _configurationDone)
        {
            return;
        }

        foreach (string path in _breakpoints.Keys.ToList())
        {
            SendBreakpointsLocked(path);
        }

        SendRequestLocked("configurationDone", new JsonObject());
        _configurationDone = true;
        LogInfo("configurationDone sent");
    }

    private void SendBreakpointsLocked(string path)
    {
        var breakpoints = new JsonArray();
        if (_breakpoints.TryGetValue(path, out SortedSet<int>? lines))
        {
            foreach (int line in lines)
            {
                breakpoints.Add((JsonNode)new JsonObject
                {
                    ["line"] = line + 1
                });
            }
        }

        SendRequestLocked("setBreakpoints", new JsonObject
        {
            ["source"] = new JsonObject
            {
                ["path"] = path
            },
            ["breakpoints"] = breakpoints,
            ["linesStartAt1"] = true,
            ["sourceModified"] = false
        });
    }

    private bool SendThreadRequest(string command, bool clearStopped)
    {
        lock (_stateLock)
        {
            if (_process is null || !_initialized)
            {
                EnqueueError("debugger is not running");
                return false;
            }

            if (_stoppedThreadId <= 0)
            {
                EnqueueError("the program is not stopped");
                return false;
            }

            SendRequestLocked(command, new JsonObject
            {
                ["threadId"] = _stoppedThreadId
            });

            if (clearStopped)
            {
                _stopped = false;
                _stoppedPath = string.Empty;
                _stoppedLine = -1;
                ClearDebugStateLocked();
            }
            return true;
        }
    }

    private long SendRequestLocked(string command, JsonObject arguments)
        => SendRequestLocked(new PendingRequest(command), arguments);

    private long SendRequestLocked(PendingRequest request, JsonObject arguments)
    {
        long sequence = Interlocked.Increment(ref _nextSequence);
        _pendingRequests[sequence] = request;

        SendMessage(new JsonObject
        {
            ["seq"] = sequence,
            ["type"] = "request",
            ["command"] = request.Command,
            ["arguments"] = arguments
        });
        return sequence;
    }

    private void SendMessage(JsonObject message)
    {
        Stream? input;
        lock (_stateLock)
        {
            input = _input;
        }

        if (input is null)
        {
            return;
        }

        try
        {
            string json = message.ToJsonString();
            LogOutbound(message, json);
            byte[] body = Encoding.UTF8.GetBytes(json);
            byte[] header = Encoding.ASCII.GetBytes(
                $"Content-Length: {body.Length}\r\n\r\n");

            lock (_writeLock)
            {
                input.Write(header);
                input.Write(body);
                input.Flush();
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                   InvalidOperationException)
        {
            LogInfo($"send failed: {ex.Message}");
            EnqueueError(ex.Message);
        }
    }

    private void ReadLoop(Stream output, CancellationToken token)
    {
        try
        {
            while (!token.IsCancellationRequested)
            {
                int contentLength = ReadHeaders(output);
                if (contentLength < 0)
                {
                    break;
                }
                if (contentLength == 0)
                {
                    continue;
                }

                byte[] body = ReadBody(output, contentLength);
                if (body.Length != contentLength)
                {
                    LogInfo($"incomplete message body ({body.Length}/{contentLength} bytes)");
                    break;
                }

                LogTraffic("<--", Encoding.UTF8.GetString(body));
                try
                {
                    HandleMessage(body);
                }
                catch (Exception ex) when (ex is JsonException or
                                           InvalidOperationException or
                                           ArgumentException)
                {
                    LogInfo($"invalid DAP message: {ex.Message}");
                }
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                   InvalidOperationException)
        {
            if (!token.IsCancellationRequested)
            {
                LogInfo($"read failed: {ex.Message}");
                EnqueueError(ex.Message);
            }
        }
        finally
        {
            if (!token.IsCancellationRequested)
            {
                LogInfo("adapter output closed");
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Terminated,
                    AdapterName = _adapterName,
                    Message = "adapter output closed"
                });
            }
        }
    }

    private void ReadStandardError(StreamReader reader, CancellationToken token)
    {
        try
        {
            while (!token.IsCancellationRequested)
            {
                string? line = reader.ReadLine();
                if (line is null)
                {
                    break;
                }

                if (line.Length > 0)
                {
                    LogTraffic("stderr", line);
                    HexalPaths.Log($"DAP stderr [{_adapterName}]: {line}");
                }
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                   InvalidOperationException)
        {
            // Stderr is diagnostic only.
        }
    }

    private void HandleMessage(byte[] body)
    {
        using JsonDocument document = JsonDocument.Parse(body);
        JsonElement root = document.RootElement;
        string type = ReadString(root, "type");

        if (type == "event")
        {
            string eventName = ReadString(root, "event");
            root.TryGetProperty("body", out JsonElement eventBody);
            HandleEvent(eventName, eventBody);
            return;
        }

        if (type == "response")
        {
            long requestSequence = ReadLong(root, "request_seq");
            string command = ReadString(root, "command");
            bool success = root.TryGetProperty("success", out JsonElement successValue) &&
                           successValue.ValueKind == JsonValueKind.True;

            PendingRequest pending;
            lock (_stateLock)
            {
                if (!_pendingRequests.Remove(requestSequence, out pending!))
                {
                    pending = new PendingRequest(command);
                }
            }

            HandleResponse(pending, root, success);
            return;
        }

        if (type == "request")
        {
            HandleServerRequest(ReadString(root, "command"), ReadLong(root, "seq"));
        }
    }

    private void HandleServerRequest(string command, long requestSequence)
    {
        // This experimental client does not create terminals or child debug
        // sessions. Reply with a protocol error so strict adapters do not wait
        // forever for a response.
        long sequence = Interlocked.Increment(ref _nextSequence);
        SendMessage(new JsonObject
        {
            ["seq"] = sequence,
            ["type"] = "response",
            ["request_seq"] = requestSequence,
            ["success"] = false,
            ["command"] = command,
            ["message"] = $"unsupported DAP request: {command}"
        });
    }

    private void HandleEvent(string eventName, JsonElement body)
    {
        switch (eventName)
        {
            case "initialized":
                lock (_stateLock)
                {
                    HandleInitializedEventLocked();
                }
                return;

            case "stopped":
            {
                int threadId = ReadInt(body, "threadId");
                string reason = ReadString(body, "reason");
                lock (_stateLock)
                {
                    _stopped = true;
                    _stoppedThreadId = threadId;
                    _stopReason = reason;
                    ClearDebugStateLocked();
                    Enqueue(new DapEvent
                    {
                        Kind = DapEventKind.Stopped,
                        AdapterName = _adapterName,
                        ThreadId = threadId,
                        Reason = reason
                    });

                    if (threadId > 0)
                    {
                        SendRequestLocked("stackTrace", new JsonObject
                        {
                            ["threadId"] = threadId,
                            ["startFrame"] = 0,
                            ["levels"] = 1
                        });
                    }
                }
                return;
            }

            case "continued":
                lock (_stateLock)
                {
                    _stopped = false;
                    _stoppedThreadId = 0;
                    _stoppedPath = string.Empty;
                    _stoppedLine = -1;
                    ClearDebugStateLocked();
                }
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Continued,
                    AdapterName = _adapterName
                });
                return;

            case "exited":
            {
                int exitCode = ReadInt(body, "exitCode");
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Exited,
                    AdapterName = _adapterName,
                    ExitCode = exitCode,
                    Message = $"exit code {exitCode}"
                });
                return;
            }

            case "terminated":
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Terminated,
                    AdapterName = _adapterName,
                    Message = "debug session terminated"
                });
                lock (_stateLock)
                {
                    StopLocked();
                }
                return;

            case "output":
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Output,
                    AdapterName = _adapterName,
                    Category = ReadString(body, "category"),
                    Output = ReadString(body, "output")
                });
                return;

            case "breakpoint":
            {
                DapBreakpoint? breakpoint = ParseBreakpoint(body);
                if (breakpoint is not null)
                {
                    Enqueue(new DapEvent
                    {
                        Kind = DapEventKind.Breakpoints,
                        AdapterName = _adapterName,
                        Breakpoints = new[] { breakpoint }
                    });
                }
                return;
            }
        }
    }

    private void HandleResponse(PendingRequest request, JsonElement root,
        bool success)
    {
        string command = request.Command;
        if (!success)
        {
            string message = ReadString(root, "message");
            if (message.Length == 0 &&
                root.TryGetProperty("body", out JsonElement errorBody))
            {
                message = ReadString(errorBody, "error");
            }
            if (message.Length == 0)
            {
                message = $"{command} failed";
            }

            if (command == "variables")
            {
                CompleteVariablesRequest(request, new List<DapVariable>());
                return;
            }
            if (command == "evaluate")
            {
                lock (_stateLock)
                {
                    if (request.Generation != _variablesGeneration)
                    {
                        return;
                    }
                }
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.WatchValue,
                    AdapterName = _adapterName,
                    Watch = new DapWatchValue(request.Expression, string.Empty,
                        string.Empty, message)
                });
                return;
            }

            Enqueue(new DapEvent
            {
                Kind = DapEventKind.Error,
                AdapterName = _adapterName,
                Message = message
            });
            return;
        }

        root.TryGetProperty("body", out JsonElement result);

        switch (command)
        {
            case "initialize":
                lock (_stateLock)
                {
                    HandleInitializeResponseLocked();
                }
                return;

            case "launch":
            case "attach":
                lock (_stateLock)
                {
                    ConfigureAdapterIfReadyLocked();
                }
                return;

            case "setBreakpoints":
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.Breakpoints,
                    AdapterName = _adapterName,
                    Breakpoints = ParseBreakpoints(result)
                });
                return;

            case "stackTrace":
            {
                string path = string.Empty;
                int line = -1;
                int frameId = 0;
                if (result.ValueKind == JsonValueKind.Object &&
                    result.TryGetProperty("stackFrames", out JsonElement frames) &&
                    frames.ValueKind == JsonValueKind.Array &&
                    frames.GetArrayLength() > 0)
                {
                    JsonElement frame = frames[0];
                    frameId = ReadInt(frame, "id");
                    if (frame.TryGetProperty("source", out JsonElement source) &&
                        source.ValueKind == JsonValueKind.Object)
                    {
                        path = ReadString(source, "path");
                        if (path.Length == 0)
                        {
                            path = ReadString(source, "name");
                        }
                    }

                    int oneBasedLine = ReadInt(frame, "line");
                    if (oneBasedLine > 0)
                    {
                        line = oneBasedLine - 1;
                    }
                }

                lock (_stateLock)
                {
                    _stoppedFrameId = frameId;
                    _stoppedPath = path;
                    _stoppedLine = line;
                }

                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.StoppedLocation,
                    AdapterName = _adapterName,
                    Path = path,
                    Line = line,
                    Reason = _stopReason
                });
                return;
            }

            case "scopes":
            {
                lock (_stateLock)
                {
                    if (request.Generation != _variablesGeneration)
                    {
                        return;
                    }

                    List<DapScope> scopes = ParseScopes(result);
                    _currentScopes.Clear();
                    _currentScopes.AddRange(scopes);
                    _variablesByReference.Clear();
                    _pendingVariablesResponses = 0;

                    foreach (DapScope scope in scopes)
                    {
                        if (scope.VariablesReference > 0)
                        {
                            ++_pendingVariablesResponses;
                        }
                    }

                    Enqueue(new DapEvent
                    {
                        Kind = DapEventKind.Scopes,
                        AdapterName = _adapterName,
                        Scopes = scopes
                    });

                    if (_pendingVariablesResponses == 0)
                    {
                        EmitVariablesLocked();
                        return;
                    }

                    foreach (DapScope scope in scopes)
                    {
                        if (scope.VariablesReference <= 0)
                        {
                            continue;
                        }

                        SendRequestLocked(new PendingRequest("variables",
                                request.Generation, scope.VariablesReference,
                                scope.Name),
                            new JsonObject
                            {
                                ["variablesReference"] = scope.VariablesReference
                            });
                    }
                }
                return;
            }

            case "variables":
            {
                CompleteVariablesRequest(request, ParseVariables(result));
                return;
            }

            case "evaluate":
            {
                lock (_stateLock)
                {
                    if (request.Generation != _variablesGeneration)
                    {
                        return;
                    }
                }
                Enqueue(new DapEvent
                {
                    Kind = DapEventKind.WatchValue,
                    AdapterName = _adapterName,
                    Watch = new DapWatchValue(request.Expression,
                        ReadString(result, "result"),
                        ReadString(result, "type"),
                        string.Empty)
                });
                return;
            }
        }
    }

    private void CompleteVariablesRequest(PendingRequest request,
        List<DapVariable> variables)
    {
        lock (_stateLock)
        {
            if (request.Generation != _variablesGeneration)
            {
                return;
            }

            _variablesByReference[request.VariablesReference] = variables;
            if (_pendingVariablesResponses > 0)
            {
                --_pendingVariablesResponses;
            }

            if (_pendingVariablesResponses == 0)
            {
                EmitVariablesLocked();
            }
        }
    }

    private void EmitVariablesLocked()
    {
        var scoped = new List<DapScopeVariables>(_currentScopes.Count);
        foreach (DapScope scope in _currentScopes)
        {
            List<DapVariable> variables = scope.VariablesReference > 0 &&
                                          _variablesByReference.TryGetValue(
                                              scope.VariablesReference,
                                              out List<DapVariable>? found)
                ? found
                : new List<DapVariable>();
            scoped.Add(new DapScopeVariables(scope.Name, variables));
        }

        Enqueue(new DapEvent
        {
            Kind = DapEventKind.Variables,
            AdapterName = _adapterName,
            ScopedVariables = scoped
        });
    }

    private static List<DapScope> ParseScopes(JsonElement result)
    {
        var scopes = new List<DapScope>();
        if (result.ValueKind != JsonValueKind.Object ||
            !result.TryGetProperty("scopes", out JsonElement array) ||
            array.ValueKind != JsonValueKind.Array)
        {
            return scopes;
        }

        foreach (JsonElement item in array.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object)
            {
                continue;
            }

            scopes.Add(new DapScope(
                ReadString(item, "name"),
                ReadInt(item, "variablesReference"),
                item.TryGetProperty("expensive", out JsonElement expensive) &&
                expensive.ValueKind == JsonValueKind.True));
        }
        return scopes;
    }

    private static List<DapVariable> ParseVariables(JsonElement result)
    {
        var variables = new List<DapVariable>();
        if (result.ValueKind != JsonValueKind.Object ||
            !result.TryGetProperty("variables", out JsonElement array) ||
            array.ValueKind != JsonValueKind.Array)
        {
            return variables;
        }

        foreach (JsonElement item in array.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object)
            {
                continue;
            }

            int reference = ReadInt(item, "variablesReference");
            variables.Add(new DapVariable(
                ReadString(item, "name"),
                ReadString(item, "value"),
                ReadString(item, "type"),
                reference,
                ReadString(item, "evaluateName")));
        }
        return variables;
    }

    private static List<DapBreakpoint> ParseBreakpoints(JsonElement result)
    {
        var breakpoints = new List<DapBreakpoint>();
        if (result.ValueKind != JsonValueKind.Object ||
            !result.TryGetProperty("breakpoints", out JsonElement array) ||
            array.ValueKind != JsonValueKind.Array)
        {
            return breakpoints;
        }

        foreach (JsonElement item in array.EnumerateArray())
        {
            DapBreakpoint? breakpoint = ParseBreakpoint(item);
            if (breakpoint is not null)
            {
                breakpoints.Add(breakpoint);
            }
        }
        return breakpoints;
    }

    private static DapBreakpoint? ParseBreakpoint(JsonElement element)
    {
        if (element.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        int line = ReadInt(element, "line");
        bool verified = element.TryGetProperty("verified",
                            out JsonElement verifiedElement) &&
                        verifiedElement.ValueKind == JsonValueKind.True;
        string message = ReadString(element, "message");
        return line > 0 ? new DapBreakpoint(line, verified, message) : null;
    }

    private static int ReadInt(JsonElement element, string property)
    {
        return element.TryGetProperty(property, out JsonElement value) &&
               value.ValueKind == JsonValueKind.Number &&
               value.TryGetInt32(out int number)
            ? number
            : 0;
    }

    private static long ReadLong(JsonElement element, string property)
    {
        return element.TryGetProperty(property, out JsonElement value) &&
               value.ValueKind == JsonValueKind.Number &&
               value.TryGetInt64(out long number)
            ? number
            : 0;
    }

    private static string ReadString(JsonElement element, string property)
    {
        return element.TryGetProperty(property, out JsonElement value) &&
               value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? string.Empty
            : string.Empty;
    }

    private static string NormalizePath(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return path;
        }
    }

    private static int ReadHeaders(Stream stream)
    {
        int contentLength = -1;
        while (true)
        {
            string? line = ReadAsciiLine(stream);
            if (line is null)
            {
                return -1;
            }
            if (line.Length == 0)
            {
                return contentLength;
            }

            int separator = line.IndexOf(':');
            string name = separator > 0 ? line[..separator].Trim() : string.Empty;
            if (string.Equals(name, "Content-Length",
                    StringComparison.OrdinalIgnoreCase) &&
                int.TryParse(line[(separator + 1)..].Trim(), out int length))
            {
                contentLength = length;
            }
        }
    }

    private static string? ReadAsciiLine(Stream stream)
    {
        Span<byte> buffer = stackalloc byte[1];
        var line = new StringBuilder(64);
        while (true)
        {
            int read;
            try
            {
                read = stream.Read(buffer);
            }
            catch (Exception ex) when (ex is IOException or ObjectDisposedException)
            {
                return null;
            }

            if (read == 0)
            {
                return null;
            }

            byte value = buffer[0];
            if (value == (byte)'\n')
            {
                if (line.Length > 0 && line[^1] == '\r')
                {
                    line.Length -= 1;
                }
                return line.ToString();
            }

            if (value != (byte)'\r')
            {
                line.Append((char)value);
            }
        }
    }

    private static byte[] ReadBody(Stream stream, int length)
    {
        byte[] body = new byte[length];
        int offset = 0;
        while (offset < length)
        {
            int read = stream.Read(body, offset, length - offset);
            if (read <= 0)
            {
                break;
            }
            offset += read;
        }
        return offset == length ? body : body[..offset];
    }

    private void StopLocked()
    {
        if (_process is null)
        {
            return;
        }

        LogInfo($"stopping {_adapterName}");
        _cancellation?.Cancel();

        try
        {
            if (!_process.HasExited)
            {
                _process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception ex) when (ex is InvalidOperationException or
                                   System.ComponentModel.Win32Exception or
                                   NotSupportedException)
        {
            // Best effort process cleanup.
        }

        try
        {
            _process.Dispose();
        }
        catch
        {
            // Ignore teardown failures.
        }

        _cancellation?.Dispose();
        _cancellation = null;
        _process = null;
        _input = null;
        _config = null;
        _launchConfiguration = null;
        _initialized = false;
        _adapterInitialized = false;
        _launchSent = false;
        _configurationDone = false;
        _stopped = false;
        _stoppedThreadId = 0;
        _stoppedPath = string.Empty;
        _stoppedLine = -1;
        _stopReason = string.Empty;
        _pendingRequests.Clear();
        ClearDebugStateLocked();
    }

    private void ClearDebugStateLocked()
    {
        _stoppedFrameId = 0;
        ++_variablesGeneration;
        _currentScopes.Clear();
        _variablesByReference.Clear();
        _pendingVariablesResponses = 0;
    }

    private void Enqueue(DapEvent dapEvent) => _events.Enqueue(dapEvent);

    private void EnqueueError(string message)
    {
        Enqueue(new DapEvent
        {
            Kind = DapEventKind.Error,
            AdapterName = _adapterName,
            Message = message
        });
    }

    private void LogOutbound(JsonObject message, string json)
    {
        if (message["command"] is JsonValue commandValue &&
            commandValue.TryGetValue(out string? command) &&
            command is "stackTrace")
        {
            LogTraffic("-->", "stackTrace");
            return;
        }
        LogTraffic("-->", json);
    }

    private void LogInfo(string text) => Log("info", text);

    private void LogTraffic(string channel, string text) => Log(channel, text);

    private void Log(string channel, string text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return;
        }
        if (text.Length > MaxLogEntryLength)
        {
            text = text[..MaxLogEntryLength] +
                   $"\n… truncated ({text.Length} chars total)";
        }

        lock (_logLock)
        {
            _log.Enqueue(
                $"{DateTime.Now:HH:mm:ss.fff} {channel} {text}");
            while (_log.Count > MaxLogEntries)
            {
                _log.Dequeue();
            }
        }
    }
}
