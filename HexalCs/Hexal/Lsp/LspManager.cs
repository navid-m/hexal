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
    private readonly object _stateLock = new();
    private readonly object _writeLock = new();
    private readonly ConcurrentQueue<LspEvent> _events = new();

    private Process? _process;
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
    private bool _disposed;

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

    public bool TryStart(LspServerConfig config, string documentPath, string text,
        out string error)
    {
        error = string.Empty;
        if (string.IsNullOrWhiteSpace(config.Command))
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
            if (_process is not null)
            {
                if (_config is not null &&
                    string.Equals(_config.Command, config.Command,
                        StringComparison.OrdinalIgnoreCase) &&
                    string.Equals(_documentUri, PathToUri(documentPath),
                        StringComparison.Ordinal))
                {
                    return true;
                }
                StopLocked();
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

            string workingDirectory = Path.GetDirectoryName(fullPath) ?? ".";
            _rootPath = workingDirectory;
            _rootUri = PathToUri(workingDirectory, directory: true);
            _documentUri = PathToUri(fullPath);
            _config = config;
            _currentText = text;
            _version = 1;
            _initialized = false;
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

            try
            {
                _process = Process.Start(startInfo);
                if (_process is null)
                {
                    error = $"failed to start language server: {config.Command}";
                    return false;
                }

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

    public void Stop()
    {
        lock (_stateLock)
        {
            StopLocked();
        }
    }

    public bool TryDequeueEvent(out LspEvent lspEvent)
        => _events.TryDequeue(out lspEvent!);

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
            byte[] body = Encoding.UTF8.GetBytes(message.ToJsonString());
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
                    break;
                }

                HandleMessage(body);
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or
                                   InvalidOperationException)
        {
            if (!token.IsCancellationRequested)
            {
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
            responseId.TryGetInt64(out long id) && id == _initializeId)
        {
            lock (_stateLock)
            {
                if (_process is null)
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
                        Message = message
                    });
                    return;
                }

                SendInitializedAndOpenLocked();
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
                    if (item.TryGetProperty("range", out JsonElement range) &&
                        range.TryGetProperty("start", out JsonElement start))
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
                            : string.Empty));
                }
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
            HexalPaths.Log($"LSP [{_config?.Name}]: {message}");
        }
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
            if (separator <= 0)
            {
                continue;
            }

            string name = line[..separator].Trim();
            if (!string.Equals(name, "Content-Length", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            if (int.TryParse(line[(separator + 1)..].Trim(), out int length))
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
        if (_process is null)
        {
            return;
        }

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
        _initialized = false;
        _currentText = string.Empty;
        _documentUri = string.Empty;

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
}
