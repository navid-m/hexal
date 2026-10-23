using System.Text.Json.Nodes;

namespace Hexal.Dap;

/// <summary>
/// Optional debug-adapter metadata from a .hexdef language definition.
/// The adapter is a process command; arguments are passed before the DAP
/// framing starts. Configuration is the launch/attach payload sent to the
/// adapter after initialization. Placeholder expansion is performed by the
/// editor before the request is sent.
/// </summary>
public sealed class DebuggerConfig
{
    public string Adapter { get; init; } = string.Empty;

    public IReadOnlyList<string> Arguments { get; init; } = Array.Empty<string>();

    public JsonObject Configuration { get; init; } = new();
}
