using System.Collections.Concurrent;

namespace Hexal.Lsp;

/// <summary>
/// A message-level transport for an in-process Lua language server. The editor
/// still speaks JSON-RPC to the server; only the byte stream is replaced by a
/// pair of queues.
/// </summary>
public interface ILspServerTransport : IDisposable
{
    void Send(string message);

    bool TryReceive(out string message);

    bool IsAlive { get; }

    Action? Disconnected { get; set; }
}

public sealed class LuaLspTransport : ILspServerTransport
{
    private readonly ConcurrentQueue<string> _toClient = new();
    private LuaLspServer? _server;
    private bool _disposed;

    public LuaLspTransport(LuaLspServer server)
    {
        _server = server;
        server.SetTransport(this);
    }

    public bool IsAlive => !_disposed && _server is not null;

    public Action? Disconnected { get; set; }

    public void Send(string message)
    {
        if (_disposed || _server is null)
        {
            return;
        }
        _server.HandleIncoming(message);
    }

    public bool TryReceive(out string message)
    {
        if (_disposed)
        {
            message = string.Empty;
            return false;
        }
        return _toClient.TryDequeue(out message!);
    }

    internal void Enqueue(string message)
    {
        if (!_disposed)
        {
            _toClient.Enqueue(message);
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;

        LuaLspServer? server = _server;
        _server = null;
        server?.OnClientDisconnected();
        Disconnected?.Invoke();
        while (_toClient.TryDequeue(out _))
        {
            // Discard undelivered messages.
        }
    }
}
