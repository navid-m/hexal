using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Hexal.Util;

namespace Hexal.Lsp;

/// <summary>
/// Minimal asynchronous LSP client: JSON-RPC over stdio, initialization,
/// full-document synchronization and diagnostics. The UI never waits for a
/// language server; process output is parsed on a worker and surfaced through
/// a lock-free event queue.
/// </summary>
public sealed class LspManager : IDisposable
{
    private const int MaxLogEntries = 4000;
    private const int MaxLogEntryLength = 16 * 1024;

    private readonly object _stateLock = new();
    private readonly object _writeLock = new();
    private readonly object _logLock = new();
    private readonly ConcurrentQueue<LspEvent> _events = new();
    private readonly Dictionary<long, string> _pendingRequests = new();
    private readonly Dictionary<string, List<LspDiagnostic>> _diagnostics =
        new(StringComparer.Ordinal);
    private readonly Queue<LspLogEntry> _log = new();
    private long _droppedLogEntries;

    private Process? _process;
    private ILspServerTransport? _transport;
    private Stream? _input;
    private CancellationTokenSource? _cancellation;
    private Task? _readerTask;
    private Task? _stderrTask;
    private LspServerConfig? _config;
    private string _documentUri = string.Empty;
    private string _rootUri = string.Empty;
    private string _rootPath = string.Empty;
    private string _currentText = string.Empty;
    private long _version;
    private long _nextRequestId;
    private long _initializeId;
    private bool _initialized;
    private bool _workspaceDiagnosticsSupported;
    private bool _disposed;

    private sealed record LspLogEntry(
        DateTime Timestamp,
        string Channel,
        string Text);

    public bool IsRunning
    {
        get
        {
            lock (_stateLock)
            {
                return _process is not null || _transport is not null;
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

    public string ServerName
    {
        get
        {
            lock (_stateLock)
            {
                return _config?.Name ?? string.Empty;
            }
        }
    }

    /// <summary>
    /// Builds a self-contained diagnostic report for the LSP debug modal. The
    /// report includes the client state, the server command line and the
    /// recent JSON-RPC/stderr traffic captured by this manager.
    /// </summary>
    public string BuildDebugReport()
    {
        string serverName;
        string command;
        string arguments;
        string transportKind;
        string rootPath;
        string rootUri;
        string documentUri;
        bool running;
        bool initialized;
        bool workspaceDiagnostics;
        long version;
        int processId = -1;
        int pendingCount;
        string pendingMethods;
        int diagnosticDocuments;

        lock (_stateLock)
        {
            serverName = _config?.Name ?? string.Empty;
            command = _config?.Command ?? string.Empty;
            arguments = _config is null
                ? string.Empty
                : string.Join(' ', _config.Arguments);
            transportKind = _transport is not null
                ? "in-process"
                : _process is not null ? "stdio" : "none";
            rootPath = _rootPath;
            rootUri = _rootUri;
            documentUri = _documentUri;
            running = _process is not null || _transport is not null;
            initialized = _initialized;
            workspaceDiagnostics = _workspaceDiagnosticsSupported;
            version = _version;
            pendingCount = _pendingRequests.Count;
            pendingMethods = string.Join(", ", _pendingRequests.Values
                .Distinct(StringComparer.Ordinal));
            diagnosticDocuments = _diagnostics.Count;

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
        builder.AppendLine("Hexal LSP diagnostics");
        builder.Append("Generated: ").AppendLine(
            DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff"));
        builder.Append("Server: ").AppendLine(serverName.Length > 0
            ? serverName
            : "(none)");
        builder.Append("Command: ").AppendLine(command.Length > 0
            ? command
            : "(none)");
        builder.Append("Arguments: ").AppendLine(arguments.Length > 0
            ? arguments
            : "(none)");
        builder.Append("Transport: ").AppendLine(transportKind);
        builder.Append("State: ")
            .Append(running ? "running" : "stopped")
            .Append(initialized ? ", initialized" : ", not initialized")
            .AppendLine();
        builder.Append("Process ID: ").AppendLine(processId > 0
            ? processId.ToString()
            : "(none)");
        builder.Append("Document: ").AppendLine(documentUri.Length > 0
            ? documentUri
            : "(none)");
        builder.Append("Root: ").AppendLine(rootPath.Length > 0
            ? $"{rootPath}  ({rootUri})"
            : "(none)");
        builder.Append("Document version: ").AppendLine(version.ToString());
        builder.Append("Pending requests: ").Append(pendingCount);
        if (pendingMethods.Length > 0)
        {
            builder.Append("  ").Append(pendingMethods);
        }
        builder.AppendLine();
        builder.Append("Diagnostic documents: ").AppendLine(
            diagnosticDocuments.ToString());
        builder.Append("Workspace diagnostics: ").AppendLine(
            workspaceDiagnostics ? "supported" : "not supported");
        builder.Append("File log: ").AppendLine(HexalPaths.LogPath);
        builder.AppendLine();

        lock (_logLock)
        {
            builder.Append("Log entries: ").Append(_log.Count);
            if (_droppedLogEntries > 0)
            {
                builder.Append(" (").Append(_droppedLogEntries)
                    .Append(" oldest dropped)");
            }
            builder.AppendLine();
            builder.AppendLine("----- LSP log (oldest first) -----");
            foreach (LspLogEntry entry in _log)
            {
                builder.Append(entry.Timestamp.ToString("HH:mm:ss.fff"))
                    .Append(' ')
                    .Append(entry.Channel)
                    .Append(' ')
                    .AppendLine(entry.Text);
            }
        }

        return builder.ToString();
    }

    public void ClearLog()
    {
        lock (_logLock)
        {
            _log.Clear();
            _droppedLogEntries = 0;
        }

        LogInfo("log cleared");
    }

    private void LogInfo(string text) => Log("info", text);

    private void LogTraffic(string channel, string text) => Log(channel, text);

    private void LogOutbound(JsonObject message, string json)
    {
        // Full-document didChange messages are sent on every burst of typing.
        // Logging their payload would bury the protocol traffic being debugged.
        if (message["method"] is JsonValue methodValue &&
            methodValue.TryGetValue(out string? method) &&
            method == "textDocument/didChange")
        {
            int characters = 0;
            if (message["params"] is JsonObject parameters &&
                parameters["contentChanges"] is JsonArray changes)
            {
                foreach (JsonNode? change in changes)
                {
                    if (change is JsonObject item &&
                        item["text"] is JsonValue textValue &&
                        textValue.TryGetValue(out string? text))
                    {
                        characters += text?.Length ?? 0;
                    }
                }
            }

            LogTraffic("-->", $"textDocument/didChange (full document, " +
                              $"{characters} characters)");
            return;
        }

        LogTraffic("-->", json);
    }

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
            _log.Enqueue(new LspLogEntry(DateTime.Now, channel, text));
            while (_log.Count > MaxLogEntries)
            {
                _log.Dequeue();
                ++_droppedLogEntries;
            }
        }
    }

    public bool TryStart(LspServerConfig config, string documentPath, string text,
        string? workspaceRoot, out string error)
    {
        error = string.Empty;
        if (string.IsNullOrWhiteSpace(config.Command) && config.Transport is null)
        {
            error = "language server command is empty";
            return false;
        }

        if (string.IsNullOrEmpty(documentPath) || !File.Exists(documentPath))
        {
            error = "language servers require a file-backed buffer";
            return false;
        }

        lock (_stateLock)
        {
            if (_process is not null || _transport is not null)
            {
                bool sameTransport = config.Transport is null
                    ? _transport is null
                    : ReferenceEquals(_transport, config.Transport);
                if (_config is not null && sameTransport &&
                    string.Equals(_config.Command, config.Command,
                        StringComparison.OrdinalIgnoreCase) &&
                    string.Equals(_documentUri, PathToUri(documentPath),
                        StringComparison.Ordinal))
                {
                    return true;
                }
                StopLocked();
            }

            if (config.Transport is not null)
            {
                string transportPath;
                try
                {
                    transportPath = Path.GetFullPath(documentPath);
                }
                catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                           PathTooLongException)
                {
                    error = ex.Message;
                    return false;
                }

                string transportRoot = !string.IsNullOrWhiteSpace(workspaceRoot) &&
                                       Directory.Exists(workspaceRoot)
                    ? Path.GetFullPath(workspaceRoot)
                    : Path.GetDirectoryName(transportPath) ?? ".";
                _rootPath = transportRoot;
                _rootUri = PathToUri(transportRoot, directory: true);
                _documentUri = PathToUri(transportPath);
                _config = config;
                _currentText = text;
                _version = 1;
                _initialized = false;
                _workspaceDiagnosticsSupported = false;
                _pendingRequests.Clear();
                _diagnostics.Clear();
                _initializeId = Interlocked.Increment(ref _nextRequestId);
                _transport = config.Transport;
                _transport.Disconnected = () =>
                {
                    Enqueue(new LspEvent
                    {
                        Kind = LspEventKind.Stopped,
                        ServerName = config.Name
                    });
                };

                LogInfo($"starting in-process LSP: {config.Name} for {transportPath}");
                SendInitializeLocked();
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Started,
                    ServerName = config.Name
                });
                HexalPaths.Log($"Lua LSP started: {config.Name} for {transportPath}");
                return true;
            }

            string? executable = ExecutableFinder.Find(config.Command);
            if (executable is null)
            {
                error = $"language server not found in PATH: {config.Command}";
                return false;
            }

            string fullPath;
            try
            {
                fullPath = Path.GetFullPath(documentPath);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                       PathTooLongException)
            {
                error = ex.Message;
                return false;
            }

            string workingDirectory = !string.IsNullOrWhiteSpace(workspaceRoot) &&
                                      Directory.Exists(workspaceRoot)
                ? Path.GetFullPath(workspaceRoot)
                : Path.GetDirectoryName(fullPath) ?? ".";
            _rootPath = workingDirectory;
            _rootUri = PathToUri(workingDirectory, directory: true);
            _documentUri = PathToUri(fullPath);
            _config = config;
            _currentText = text;
            _version = 1;
            _initialized = false;
            _workspaceDiagnosticsSupported = false;
            _pendingRequests.Clear();
            _diagnostics.Clear();
            _initializeId = Interlocked.Increment(ref _nextRequestId);

            var startInfo = new ProcessStartInfo
            {
                FileName = executable,
                WorkingDirectory = workingDirectory,
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
            LogInfo($"starting {config.Name}: {executable}" +
                    (arguments.Length > 0 ? $" {arguments}" : string.Empty) +
                    $" (cwd {workingDirectory})");

            try
            {
                _process = Process.Start(startInfo);
                if (_process is null)
                {
                    error = $"failed to start language server: {config.Command}";
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
                    ? $"process started (pid {processId})"
                    : "process started");
                _input = _process.StandardInput.BaseStream;
                _cancellation = new CancellationTokenSource();
                CancellationToken token = _cancellation.Token;

                _readerTask = Task.Run(() =>
                    ReadLoop(_process.StandardOutput.BaseStream, token));
                _stderrTask = Task.Run(() =>
                    ReadStandardError(_process.StandardError, token));

                SendInitializeLocked();
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Started,
                    ServerName = config.Name
                });
                HexalPaths.Log(
                    $"LSP started: {config.Name} ({executable}) for {fullPath}");
                return true;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       InvalidOperationException or NotSupportedException or
                                       System.ComponentModel.Win32Exception)
            {
                error = $"cannot start language server '{config.Command}': {ex.Message}";
                StopLocked();
                return false;
            }
        }
    }

    public void DidChange(string text)
    {
        lock (_stateLock)
        {
            if (_process is null || !_initialized || _input is null)
            {
                _currentText = text;
                return;
            }

            _currentText = text;
            ++_version;

            var contentChanges = new JsonArray();
            contentChanges.Add((JsonNode)new JsonObject { ["text"] = text });

            var parameters = new JsonObject
            {
                ["textDocument"] = new JsonObject
                {
                    ["uri"] = _documentUri,
                    ["version"] = _version
                },
                ["contentChanges"] = contentChanges
            };

            SendNotificationLocked("textDocument/didChange", parameters);
        }
    }

    public void DidSave(string text)
    {
        lock (_stateLock)
        {
            if (_process is null || !_initialized)
            {
                return;
            }

            var parameters = new JsonObject
            {
                ["textDocument"] = new JsonObject
                {
                    ["uri"] = _documentUri
                },
                ["text"] = text
            };
            SendNotificationLocked("textDocument/didSave", parameters);
        }
    }

    /// <summary>
    /// Requests hover information at a zero-based line and UTF-16 column.
    /// Returns false when no initialized server owns the active document.
    /// </summary>
    public bool RequestHover(string documentPath, int line, int character)
    {
        lock (_stateLock)
        {
            if (!CanRequestForPathLocked(documentPath))
            {
                return false;
            }

            var parameters = new JsonObject
            {
                ["textDocument"] = new JsonObject
                {
                    ["uri"] = _documentUri
                },
                ["position"] = new JsonObject
                {
                    ["line"] = line,
                    ["character"] = character
                }
            };

            SendRequestLocked("textDocument/hover", parameters);
            return true;
        }
    }

    /// <summary>
    /// Requests the definition of the symbol at a zero-based line and UTF-16
    /// column. The result may live in another file.
    /// </summary>
    public bool RequestDefinition(string documentPath, int line, int character)
    {
        lock (_stateLock)
        {
            if (!CanRequestForPathLocked(documentPath))
            {
                return false;
            }

            var parameters = new JsonObject
            {
                ["textDocument"] = new JsonObject
                {
                    ["uri"] = _documentUri
                },
                ["position"] = new JsonObject
                {
                    ["line"] = line,
                    ["character"] = character
                }
            };

            SendRequestLocked("textDocument/definition", parameters);
            return true;
        }
    }

    /// <summary>
    /// Requests code actions for the cursor position. Diagnostics for the
    /// current line are passed as context, as required by the specification.
    /// </summary>
    public bool RequestCodeActions(string documentPath, int line, int character,
        IReadOnlyList<LspDiagnostic> diagnostics)
    {
        lock (_stateLock)
        {
            if (!CanRequestForPathLocked(documentPath))
            {
                return false;
            }

            var contextDiagnostics = new JsonArray();
            foreach (LspDiagnostic diagnostic in diagnostics)
            {
                if (diagnostic.Line != line)
                {
                    continue;
                }

                int endLine = diagnostic.EndLine >= 0 ? diagnostic.EndLine : diagnostic.Line;
                int endColumn = diagnostic.EndColumn >= 0
                    ? diagnostic.EndColumn
                    : diagnostic.Column + 1;

                contextDiagnostics.Add((JsonNode)new JsonObject
                {
                    ["range"] = new JsonObject
                    {
                        ["start"] = new JsonObject
                        {
                            ["line"] = diagnostic.Line,
                            ["character"] = diagnostic.Column
                        },
                        ["end"] = new JsonObject
                        {
                            ["line"] = endLine,
                            ["character"] = endColumn
                        }
                    },
                    ["severity"] = diagnostic.Severity,
                    ["source"] = diagnostic.Source,
                    ["message"] = diagnostic.Message
                });
            }

            var parameters = new JsonObject
            {
                ["textDocument"] = new JsonObject
                {
                    ["uri"] = _documentUri
                },
                ["range"] = new JsonObject
                {
                    ["start"] = new JsonObject
                    {
                        ["line"] = line,
                        ["character"] = character
                    },
                    ["end"] = new JsonObject
                    {
                        ["line"] = line,
                        ["character"] = character
                    }
                },
                ["context"] = new JsonObject
                {
                    ["diagnostics"] = contextDiagnostics
                }
            };

            SendRequestLocked("textDocument/codeAction", parameters);
            return true;
        }
    }

    /// <summary>
    /// Resolves a code action that the server returned without an edit or
    /// command (the "data-only" form used by some servers).
    /// </summary>
    public bool ResolveCodeAction(LspCodeAction action)
    {
        if (action.Raw is null)
        {
            return false;
        }

        lock (_stateLock)
        {
            if (_process is null || !_initialized)
            {
                return false;
            }

            var parameters = (JsonObject)action.Raw.DeepClone();
            SendRequestLocked("codeAction/resolve", parameters);
            return true;
        }
    }

    /// <summary>
    /// Asks the server for diagnostics for the whole workspace. Returns false
    /// when the server did not advertise workspace diagnostic support.
    /// </summary>
    public bool RequestWorkspaceDiagnostics()
    {
        lock (_stateLock)
        {
            if (_process is null || !_initialized || !_workspaceDiagnosticsSupported)
            {
                return false;
            }

            var parameters = new JsonObject
            {
                ["previousResultIds"] = new JsonArray()
            };
            SendRequestLocked("workspace/diagnostic", parameters);
            return true;
        }
    }

    public void ExecuteCommand(string command, JsonArray? arguments)
    {
        if (string.IsNullOrEmpty(command))
        {
            return;
        }

        lock (_stateLock)
        {
            if (_process is null || !_initialized)
            {
                return;
            }

            SendRequestLocked("workspace/executeCommand", new JsonObject
            {
                ["command"] = command,
                ["arguments"] = arguments?.DeepClone() ?? new JsonArray()
            });
        }
    }

    /// <summary>
    /// Returns a snapshot of every diagnostic set the server has published.
    /// </summary>
    public List<LspDocumentDiagnostics> SnapshotDiagnostics()
    {
        lock (_stateLock)
        {
            var result = new List<LspDocumentDiagnostics>(_diagnostics.Count);
            foreach ((string uri, List<LspDiagnostic> diagnostics) in _diagnostics)
            {
                result.Add(new LspDocumentDiagnostics(uri, diagnostics.ToArray()));
            }
            return result;
        }
    }

    public List<LspDiagnostic> SnapshotDiagnosticsForPath(string documentPath)
    {
        string uri;
        try
        {
            uri = PathToUri(documentPath);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return new List<LspDiagnostic>();
        }

        lock (_stateLock)
        {
            return _diagnostics.TryGetValue(uri, out List<LspDiagnostic>? diagnostics)
                ? new List<LspDiagnostic>(diagnostics)
                : new List<LspDiagnostic>();
        }
    }

    /// <summary>
    /// Returns the number of diagnostics currently published for one document.
    /// Used by the status bar without allocating a full snapshot on every frame.
    /// </summary>
    public int CountDiagnosticsForPath(string documentPath)
    {
        if (string.IsNullOrEmpty(documentPath))
        {
            return 0;
        }

        string uri;
        try
        {
            uri = PathToUri(documentPath);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return 0;
        }

        lock (_stateLock)
        {
            return _diagnostics.TryGetValue(uri, out List<LspDiagnostic>? diagnostics)
                ? diagnostics.Count
                : 0;
        }
    }

    public void Stop()
    {
        lock (_stateLock)
        {
            StopLocked();
        }
    }

    public bool TryDequeueEvent(out LspEvent lspEvent)
        => _events.TryDequeue(out lspEvent!);

    /// <summary>
    /// Delivers messages from an in-process Lua server transport on the editor
    /// thread. Lua code is never invoked from a background thread.
    /// </summary>
    public void Pump()
    {
        while (true)
        {
            ILspServerTransport? transport;
            lock (_stateLock)
            {
                transport = _transport;
            }
            if (transport is null || !transport.TryReceive(out string message))
            {
                break;
            }

            LogTraffic("<--", message);
            try
            {
                HandleMessage(Encoding.UTF8.GetBytes(message));
            }
            catch (Exception ex) when (ex is System.Text.Json.JsonException or
                                       InvalidOperationException or
                                       ArgumentException)
            {
                LogInfo($"invalid LSP message: {ex.Message}");
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Error,
                    ServerName = ServerName,
                    Message = ex.Message
                });
            }
        }
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

    private void SendInitializeLocked()
    {
        var workspaceFolders = new JsonArray();
        workspaceFolders.Add((JsonNode)new JsonObject
        {
            ["uri"] = _rootUri,
            ["name"] = Path.GetFileName(_rootPath)
        });

        var parameters = new JsonObject
        {
            ["processId"] = Environment.ProcessId,
            ["clientInfo"] = new JsonObject
            {
                ["name"] = "Hexal"
            },
            ["rootUri"] = _rootUri,
            ["rootPath"] = _rootPath,
            ["workspaceFolders"] = workspaceFolders,
            ["capabilities"] = new JsonObject
            {
                ["workspace"] = new JsonObject
                {
                    ["workspaceFolders"] = true,
                    ["configuration"] = true
                },
                ["textDocument"] = new JsonObject
                {
                    ["synchronization"] = new JsonObject
                    {
                        ["dynamicRegistration"] = false,
                        ["didSave"] = true
                    },
                    ["publishDiagnostics"] = new JsonObject
                    {
                        ["relatedInformation"] = true
                    },
                    ["definition"] = new JsonObject
                    {
                        ["linkSupport"] = true
                    }
                }
            }
        };

        _pendingRequests[_initializeId] = "initialize";

        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = _initializeId,
            ["method"] = "initialize",
            ["params"] = parameters
        };
        SendMessage(message);
    }

    private void SendInitializedAndOpenLocked()
    {
        _initialized = true;

        SendNotificationLocked("initialized", new JsonObject());

        var open = new JsonObject
        {
            ["textDocument"] = new JsonObject
            {
                ["uri"] = _documentUri,
                ["languageId"] = _config?.LanguageId ?? string.Empty,
                ["version"] = _version,
                ["text"] = _currentText
            }
        };
        SendNotificationLocked("textDocument/didOpen", open);

        Enqueue(new LspEvent
        {
            Kind = LspEventKind.Initialized,
            ServerName = _config?.Name ?? string.Empty
        });
    }

    private void SendNotificationLocked(string method, JsonObject parameters)
    {
        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["method"] = method,
            ["params"] = parameters
        };
        SendMessage(message);
    }

    private bool CanRequestForPathLocked(string documentPath)
    {
        if ((_process is null && _transport is null) || !_initialized)
        {
            return false;
        }

        try
        {
            return string.Equals(_documentUri, PathToUri(documentPath),
                StringComparison.Ordinal);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return false;
        }
    }

    private void SendRequestLocked(string method, JsonObject parameters)
    {
        long id = Interlocked.Increment(ref _nextRequestId);
        _pendingRequests[id] = method;

        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id,
            ["method"] = method,
            ["params"] = parameters
        };
        SendMessage(message);
    }

    private void SendResponse(JsonElement id, JsonNode? result)
    {
        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = CloneId(id),
            ["result"] = result
        };
        SendMessage(message);
    }

    private void SendMessage(JsonObject message)
    {
        ILspServerTransport? transport;
        Stream? input;
        lock (_stateLock)
        {
            transport = _transport;
            input = _input;
        }

        if (transport is not null)
        {
            try
            {
                string json = message.ToJsonString();
                LogOutbound(message, json);
                transport.Send(json);
            }
            catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                       InvalidOperationException or
                                       System.Text.Json.JsonException)
            {
                LogInfo($"send failed: {ex.Message}");
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Error,
                    ServerName = ServerName,
                    Message = ex.Message
                });
            }
            return;
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
            Enqueue(new LspEvent
            {
                Kind = LspEventKind.Error,
                ServerName = ServerName,
                Message = ex.Message
            });
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
                    LogInfo($"invalid LSP message: {ex.Message}");
                }
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                   InvalidOperationException)
        {
            if (!token.IsCancellationRequested)
            {
                LogInfo($"read failed: {ex.Message}");
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Error,
                    ServerName = ServerName,
                    Message = ex.Message
                });
            }
        }
        finally
        {
            if (!token.IsCancellationRequested)
            {
                LogInfo("server output closed");
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Stopped,
                    ServerName = ServerName
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
                    HexalPaths.Log($"LSP stderr [{ServerName}]: {line}");
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

        if (root.TryGetProperty("method", out JsonElement methodElement) &&
            methodElement.ValueKind == JsonValueKind.String)
        {
            string method = methodElement.GetString() ?? string.Empty;

            if (root.TryGetProperty("id", out JsonElement requestId))
            {
                HandleServerRequest(method, requestId, root);
            }
            else
            {
                HandleNotification(method, root);
            }
            return;
        }

        if (root.TryGetProperty("id", out JsonElement responseId) &&
            responseId.ValueKind == JsonValueKind.Number &&
            responseId.TryGetInt64(out long id))
        {
            if (id == _initializeId)
            {
                HandleInitializeResponse(root);
                return;
            }

            string method;
            lock (_stateLock)
            {
                _pendingRequests.Remove(id, out method!);
            }

            if (!string.IsNullOrEmpty(method))
            {
                HandleResponse(method, root);
            }
        }
    }

    private void HandleInitializeResponse(JsonElement root)
    {
        lock (_stateLock)
        {
            _pendingRequests.Remove(_initializeId);

            if (_process is null && _transport is null)
            {
                return;
            }

            if (root.TryGetProperty("error", out JsonElement error))
            {
                string message = error.TryGetProperty("message", out JsonElement text)
                    ? text.GetString() ?? "initialize failed"
                    : "initialize failed";
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Error,
                    ServerName = _config?.Name ?? string.Empty,
                    Method = "initialize",
                    Message = message
                });
                return;
            }

            if (root.TryGetProperty("result", out JsonElement result) &&
                result.TryGetProperty("capabilities", out JsonElement capabilities) &&
                capabilities.TryGetProperty("diagnosticProvider", out JsonElement provider) &&
                provider.ValueKind == JsonValueKind.Object &&
                provider.TryGetProperty("workspaceDiagnostics", out JsonElement workspace) &&
                workspace.ValueKind == JsonValueKind.True)
            {
                _workspaceDiagnosticsSupported = true;
            }

            LogInfo("initialize response received");
            SendInitializedAndOpenLocked();
        }
    }

    private void HandleResponse(string method, JsonElement root)
    {
        string serverName = ServerName;

        if (root.TryGetProperty("error", out JsonElement error))
        {
            string message = error.TryGetProperty("message", out JsonElement text)
                ? text.GetString() ?? $"{method} failed"
                : $"{method} failed";

            // Workspace diagnostics are opt-in; a server rejecting the pull
            // request should not spam the status line.
            if (method != "workspace/diagnostic")
            {
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Error,
                    ServerName = serverName,
                    Method = method,
                    Message = message
                });
            }
            return;
        }

        root.TryGetProperty("result", out JsonElement result);

        switch (method)
        {
            case "textDocument/hover":
            {
                string text = result.ValueKind == JsonValueKind.Object
                    ? ExtractHoverText(result)
                    : string.Empty;
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Hover,
                    ServerName = serverName,
                    Method = method,
                    Text = text
                });
                return;
            }

            case "textDocument/definition":
            case "textDocument/declaration":
            case "textDocument/typeDefinition":
            {
                List<LspLocation> locations = ParseLocations(result);
                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.Definition,
                    ServerName = serverName,
                    Method = method,
                    Locations = locations
                });
                return;
            }

            case "textDocument/codeAction":
            case "codeAction/resolve":
            {
                List<LspCodeAction> actions = ParseCodeActions(result);
                Enqueue(new LspEvent
                {
                    Kind = method == "codeAction/resolve"
                        ? LspEventKind.CodeActionResolved
                        : LspEventKind.CodeActions,
                    ServerName = serverName,
                    Method = method,
                    CodeActions = actions
                });
                return;
            }

            case "workspace/diagnostic":
            {
                List<LspDocumentDiagnostics> reports = ParseWorkspaceDiagnostics(result);
                lock (_stateLock)
                {
                    foreach (LspDocumentDiagnostics report in reports)
                    {
                        _diagnostics[report.Uri] = new List<LspDiagnostic>(report.Diagnostics);
                    }
                }

                Enqueue(new LspEvent
                {
                    Kind = LspEventKind.WorkspaceDiagnostics,
                    ServerName = serverName,
                    Method = method,
                    DocumentDiagnostics = reports
                });
                return;
            }
        }
    }

    private void HandleNotification(string method, JsonElement root)
    {
        if (!root.TryGetProperty("params", out JsonElement parameters))
        {
            return;
        }

        if (method == "textDocument/publishDiagnostics")
        {
            string uri = parameters.TryGetProperty("textDocument", out JsonElement textDocument)
                ? textDocument.TryGetProperty("uri", out JsonElement uriElement)
                    ? uriElement.GetString() ?? string.Empty
                    : string.Empty
                : parameters.TryGetProperty("uri", out JsonElement directUri)
                    ? directUri.GetString() ?? string.Empty
                    : string.Empty;

            var diagnostics = new List<LspDiagnostic>();
            if (parameters.TryGetProperty("diagnostics", out JsonElement array) &&
                array.ValueKind == JsonValueKind.Array)
            {
                foreach (JsonElement item in array.EnumerateArray())
                {
                    int line = 0;
                    int column = 0;
                    int endLine = -1;
                    int endColumn = -1;
                    if (item.TryGetProperty("range", out JsonElement range))
                    {
                        if (range.TryGetProperty("start", out JsonElement start))
                        {
                            if (start.TryGetProperty("line", out JsonElement lineElement))
                            {
                                line = lineElement.GetInt32();
                            }
                            if (start.TryGetProperty("character", out JsonElement columnElement))
                            {
                                column = columnElement.GetInt32();
                            }
                        }
                        if (range.TryGetProperty("end", out JsonElement end))
                        {
                            if (end.TryGetProperty("line", out JsonElement endLineElement))
                            {
                                endLine = endLineElement.GetInt32();
                            }
                            if (end.TryGetProperty("character", out JsonElement endColumnElement))
                            {
                                endColumn = endColumnElement.GetInt32();
                            }
                        }
                    }

                    diagnostics.Add(new LspDiagnostic(
                        line,
                        column,
                        item.TryGetProperty("severity", out JsonElement severity)
                            ? severity.GetInt32()
                            : 1,
                        item.TryGetProperty("message", out JsonElement message)
                            ? message.GetString() ?? string.Empty
                            : string.Empty,
                        item.TryGetProperty("source", out JsonElement source)
                            ? source.GetString() ?? string.Empty
                            : string.Empty,
                        endLine,
                        endColumn));
                }
            }

            lock (_stateLock)
            {
                _diagnostics[uri] = new List<LspDiagnostic>(diagnostics);
            }

            Enqueue(new LspEvent
            {
                Kind = LspEventKind.Diagnostics,
                ServerName = _config?.Name ?? string.Empty,
                Uri = uri,
                Diagnostics = diagnostics
            });
        }
        else if (method == "window/showMessage")
        {
            string message = parameters.TryGetProperty("message", out JsonElement text)
                ? text.GetString() ?? string.Empty
                : string.Empty;
            LogTraffic("server", message);
            Enqueue(new LspEvent
            {
                Kind = LspEventKind.Error,
                ServerName = _config?.Name ?? string.Empty,
                Message = message
            });
        }
        else if (method == "window/logMessage")
        {
            string message = parameters.TryGetProperty("message", out JsonElement text)
                ? text.GetString() ?? string.Empty
                : string.Empty;
            LogTraffic("server", message);
            HexalPaths.Log($"LSP [{_config?.Name}]: {message}");
        }
    }

    private static string ExtractHoverText(JsonElement hover)
    {
        if (!hover.TryGetProperty("contents", out JsonElement contents))
        {
            return string.Empty;
        }

        var parts = new List<string>();
        AppendHoverContents(contents, parts);
        return string.Join("\n", parts);
    }

    private static void AppendHoverContents(JsonElement contents, List<string> parts)
    {
        switch (contents.ValueKind)
        {
            case JsonValueKind.String:
            {
                string? text = contents.GetString();
                if (!string.IsNullOrEmpty(text))
                {
                    parts.Add(text);
                }
                break;
            }

            case JsonValueKind.Array:
                foreach (JsonElement item in contents.EnumerateArray())
                {
                    AppendHoverContents(item, parts);
                }
                break;

            case JsonValueKind.Object:
                if (contents.TryGetProperty("value", out JsonElement value) &&
                    value.ValueKind == JsonValueKind.String)
                {
                    string? text = value.GetString();
                    if (!string.IsNullOrEmpty(text))
                    {
                        parts.Add(text);
                    }
                }
                break;
        }
    }

    private static List<LspLocation> ParseLocations(JsonElement result)
    {
        var locations = new List<LspLocation>();
        ParseLocations(result, locations);
        return locations;
    }

    private static void ParseLocations(JsonElement result, List<LspLocation> locations)
    {
        if (result.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement item in result.EnumerateArray())
            {
                ParseLocations(item, locations);
            }
            return;
        }

        if (result.ValueKind != JsonValueKind.Object)
        {
            return;
        }

        // LocationLink uses targetUri/targetRange, while Location uses
        // uri/range. Servers may return either shape (or an array of both).
        string uri = string.Empty;
        if (result.TryGetProperty("uri", out JsonElement uriElement) &&
            uriElement.ValueKind == JsonValueKind.String)
        {
            uri = uriElement.GetString() ?? string.Empty;
        }
        else if (result.TryGetProperty("targetUri", out JsonElement targetUri) &&
                 targetUri.ValueKind == JsonValueKind.String)
        {
            uri = targetUri.GetString() ?? string.Empty;
        }

        if (uri.Length == 0)
        {
            return;
        }

        JsonElement range;
        if (!result.TryGetProperty("range", out range) &&
            !result.TryGetProperty("targetSelectionRange", out range) &&
            !result.TryGetProperty("targetRange", out range))
        {
            return;
        }

        if (range.ValueKind != JsonValueKind.Object ||
            !range.TryGetProperty("start", out JsonElement start) ||
            start.ValueKind != JsonValueKind.Object)
        {
            return;
        }

        int line = start.TryGetProperty("line", out JsonElement lineElement)
            ? lineElement.GetInt32()
            : 0;
        int character = start.TryGetProperty("character", out JsonElement characterElement)
            ? characterElement.GetInt32()
            : 0;

        locations.Add(new LspLocation(uri, line, character));
    }

    private static List<LspCodeAction> ParseCodeActions(JsonElement result)
    {
        var actions = new List<LspCodeAction>();
        if (result.ValueKind != JsonValueKind.Array)
        {
            return actions;
        }

        foreach (JsonElement item in result.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object)
            {
                continue;
            }

            List<LspTextEdit> edits = ParseWorkspaceEdit(item, "edit");

            string command = string.Empty;
            JsonArray? arguments = null;
            if (item.TryGetProperty("command", out JsonElement commandElement) &&
                commandElement.ValueKind == JsonValueKind.Object)
            {
                if (commandElement.TryGetProperty("command", out JsonElement name) &&
                    name.ValueKind == JsonValueKind.String)
                {
                    command = name.GetString() ?? string.Empty;
                }

                if (commandElement.TryGetProperty("arguments", out JsonElement args) &&
                    args.ValueKind == JsonValueKind.Array &&
                    JsonNode.Parse(args.GetRawText()) is JsonArray array)
                {
                    arguments = array;
                }
            }

            bool hasData = item.TryGetProperty("data", out JsonElement data) &&
                           data.ValueKind != JsonValueKind.Null &&
                           data.ValueKind != JsonValueKind.Undefined;

            JsonObject? raw = JsonNode.Parse(item.GetRawText()) as JsonObject;

            actions.Add(new LspCodeAction
            {
                Title = item.TryGetProperty("title", out JsonElement title)
                    ? title.GetString() ?? string.Empty
                    : string.Empty,
                Kind = item.TryGetProperty("kind", out JsonElement kind)
                    ? kind.GetString() ?? string.Empty
                    : string.Empty,
                Edits = edits,
                Command = command,
                CommandArguments = arguments,
                NeedsResolve = edits.Count == 0 && command.Length == 0 && hasData,
                Raw = raw
            });
        }

        return actions;
    }

    private static List<LspTextEdit> ParseWorkspaceEdit(JsonElement owner, string property)
    {
        var edits = new List<LspTextEdit>();
        if (!owner.TryGetProperty(property, out JsonElement edit) ||
            edit.ValueKind != JsonValueKind.Object)
        {
            return edits;
        }

        if (edit.TryGetProperty("changes", out JsonElement changes) &&
            changes.ValueKind == JsonValueKind.Object)
        {
            foreach (JsonProperty file in changes.EnumerateObject())
            {
                if (file.Value.ValueKind == JsonValueKind.Array)
                {
                    ParseTextEdits(file.Name, file.Value, edits);
                }
            }
        }

        if (edit.TryGetProperty("documentChanges", out JsonElement documentChanges) &&
            documentChanges.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement change in documentChanges.EnumerateArray())
            {
                if (change.ValueKind != JsonValueKind.Object ||
                    !change.TryGetProperty("textDocument", out JsonElement textDocument) ||
                    !textDocument.TryGetProperty("uri", out JsonElement uriElement))
                {
                    // Create/rename/delete file operations are not supported.
                    continue;
                }

                if (change.TryGetProperty("edits", out JsonElement textEdits) &&
                    textEdits.ValueKind == JsonValueKind.Array)
                {
                    ParseTextEdits(uriElement.GetString() ?? string.Empty, textEdits, edits);
                }
            }
        }

        return edits;
    }

    private static void ParseTextEdits(string uri, JsonElement array,
        List<LspTextEdit> edits)
    {
        foreach (JsonElement item in array.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object ||
                !item.TryGetProperty("range", out JsonElement range) ||
                !range.TryGetProperty("start", out JsonElement start) ||
                !range.TryGetProperty("end", out JsonElement end))
            {
                continue;
            }

            int startLine = start.TryGetProperty("line", out JsonElement startLineElement)
                ? startLineElement.GetInt32()
                : 0;
            int startCharacter = start.TryGetProperty("character",
                out JsonElement startCharacterElement)
                ? startCharacterElement.GetInt32()
                : 0;
            int endLine = end.TryGetProperty("line", out JsonElement endLineElement)
                ? endLineElement.GetInt32()
                : startLine;
            int endCharacter = end.TryGetProperty("character",
                out JsonElement endCharacterElement)
                ? endCharacterElement.GetInt32()
                : startCharacter;

            string newText = item.TryGetProperty("newText", out JsonElement newTextElement)
                ? newTextElement.GetString() ?? string.Empty
                : string.Empty;

            edits.Add(new LspTextEdit(uri, startLine, startCharacter, endLine,
                endCharacter, newText));
        }
    }

    private static List<LspDocumentDiagnostics> ParseWorkspaceDiagnostics(JsonElement result)
    {
        var reports = new List<LspDocumentDiagnostics>();
        if (result.ValueKind != JsonValueKind.Object ||
            !result.TryGetProperty("items", out JsonElement items) ||
            items.ValueKind != JsonValueKind.Array)
        {
            return reports;
        }

        foreach (JsonElement item in items.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object ||
                !item.TryGetProperty("uri", out JsonElement uri) ||
                uri.ValueKind != JsonValueKind.String)
            {
                continue;
            }

            var diagnostics = new List<LspDiagnostic>();
            if (item.TryGetProperty("diagnostics", out JsonElement array) &&
                array.ValueKind == JsonValueKind.Array)
            {
                foreach (JsonElement diagnostic in array.EnumerateArray())
                {
                    diagnostics.Add(ParseDiagnostic(diagnostic));
                }
            }

            reports.Add(new LspDocumentDiagnostics(uri.GetString() ?? string.Empty,
                diagnostics));
        }

        return reports;
    }

    private static LspDiagnostic ParseDiagnostic(JsonElement item)
    {
        int line = 0;
        int column = 0;
        int endLine = -1;
        int endColumn = -1;

        if (item.ValueKind == JsonValueKind.Object &&
            item.TryGetProperty("range", out JsonElement range))
        {
            if (range.TryGetProperty("start", out JsonElement start))
            {
                if (start.TryGetProperty("line", out JsonElement lineElement))
                {
                    line = lineElement.GetInt32();
                }
                if (start.TryGetProperty("character", out JsonElement columnElement))
                {
                    column = columnElement.GetInt32();
                }
            }
            if (range.TryGetProperty("end", out JsonElement end))
            {
                if (end.TryGetProperty("line", out JsonElement endLineElement))
                {
                    endLine = endLineElement.GetInt32();
                }
                if (end.TryGetProperty("character", out JsonElement endColumnElement))
                {
                    endColumn = endColumnElement.GetInt32();
                }
            }
        }

        return new LspDiagnostic(
            line,
            column,
            item.ValueKind == JsonValueKind.Object &&
            item.TryGetProperty("severity", out JsonElement severity)
                ? severity.GetInt32()
                : 1,
            item.ValueKind == JsonValueKind.Object &&
            item.TryGetProperty("message", out JsonElement message)
                ? message.GetString() ?? string.Empty
                : string.Empty,
            item.ValueKind == JsonValueKind.Object &&
            item.TryGetProperty("source", out JsonElement source)
                ? source.GetString() ?? string.Empty
                : string.Empty,
            endLine,
            endColumn);
    }

    private void HandleServerRequest(string method, JsonElement id, JsonElement root)
    {
        JsonNode? result = null;

        if (method == "workspace/configuration")
        {
            int count = 0;
            if (root.TryGetProperty("params", out JsonElement parameters) &&
                parameters.TryGetProperty("items", out JsonElement items) &&
                items.ValueKind == JsonValueKind.Array)
            {
                count = items.GetArrayLength();
            }

            var array = new JsonArray();
            for (int i = 0; i < count; ++i)
            {
                array.Add((JsonNode?)null);
            }
            result = array;
        }
        else if (method == "workspace/workspaceFolders")
        {
            var folders = new JsonArray();
            folders.Add((JsonNode)new JsonObject
            {
                ["uri"] = _rootUri,
                ["name"] = Path.GetFileName(_rootPath)
            });
            result = folders;
        }

        SendResponse(id, result);
    }

    private static JsonValue? CloneId(JsonElement id)
    {
        return id.ValueKind switch
        {
            JsonValueKind.Number when id.TryGetInt64(out long number) =>
                JsonValue.Create(number),
            JsonValueKind.String => JsonValue.Create(id.GetString()),
            _ => null
        };
    }

    private int ReadHeaders(Stream stream)
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
                    StringComparison.OrdinalIgnoreCase))
            {
                if (int.TryParse(line[(separator + 1)..].Trim(), out int length))
                {
                    contentLength = length;
                }
                continue;
            }

            if (string.Equals(name, "Content-Type", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            // Servers invoked without their stdio/LSP flag often emit plain
            // newline-delimited logs instead of framed JSON-RPC. Surface those
            // lines in the debug log instead of silently waiting for a header.
            LogTraffic("raw", line);
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

            if (line.Length >= 8192)
            {
                return null;
            }
            line.Append((char)value);
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
        if (_process is null && _transport is null)
        {
            return;
        }

        LogInfo($"stopping {_config?.Name ?? "language server"}");
        _cancellation?.Cancel();

        if (_process is not null)
        {
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
        }

        _cancellation?.Dispose();
        _cancellation = null;
        _process = null;
        _input = null;

        ILspServerTransport? transport = _transport;
        _transport = null;
        if (transport is not null)
        {
            try
            {
                transport.Dispose();
            }
            catch
            {
                // Transport teardown is best effort.
            }
        }

        _config = null;
        _initialized = false;
        _workspaceDiagnosticsSupported = false;
        _currentText = string.Empty;
        _documentUri = string.Empty;
        _pendingRequests.Clear();
        _diagnostics.Clear();

        Enqueue(new LspEvent
        {
            Kind = LspEventKind.Stopped
        });
    }

    private void Enqueue(LspEvent lspEvent)
    {
        _events.Enqueue(lspEvent);
    }

    private static string PathToUri(string path, bool directory = false)
    {
        string full = Path.GetFullPath(path);
        if (directory && !full.EndsWith(Path.DirectorySeparatorChar))
        {
            full += Path.DirectorySeparatorChar;
        }
        return new Uri(full).AbsoluteUri;
    }

    /// <summary>
    /// Converts a file URI from a language server back into a local path.
    /// </summary>
    public static string UriToPath(string uri)
    {
        try
        {
            return new Uri(uri).LocalPath;
        }
        catch (Exception ex) when (ex is UriFormatException or ArgumentException or
                                   NotSupportedException)
        {
            return uri;
        }
    }
}
