using Hexal.Lsp;

namespace Hexal.Scripting;

public readonly record struct PluginCursor(long Line, long Column);

public readonly record struct PluginSelection(
    long StartLine,
    long StartColumn,
    long EndLine,
    long EndColumn,
    string Text,
    bool Linewise
);

public readonly record struct PluginPanelSegment(string Text, string Style);

public sealed class PluginPanelLine
{
    public IReadOnlyList<PluginPanelSegment> Segments { get; init; } =
        Array.Empty<PluginPanelSegment>();

    public bool Selectable { get; init; } = true;
}

public sealed class PluginPanelOptions
{
    public string Title { get; init; } = "Plugin";

    public string Footer { get; init; } = string.Empty;

    public bool Selectable { get; init; } = true;

    public long Selected { get; init; }

    public IReadOnlyList<PluginPanelLine> Lines { get; init; } = Array.Empty<PluginPanelLine>();
}

/// <summary>
/// The narrow surface a script host is allowed to touch. Keeping Lua away from
/// editor internals makes ownership, validation and UI-thread rules explicit.
/// Line and column values are zero-based; columns count Unicode scalars.
/// </summary>
public interface IEditorApi
{
    string CurrentBufferId { get; }

    string? WorkspaceRoot { get; }

    int TerminalColumns { get; }

    int TerminalRows { get; }

    string GetText();

    string? GetLine(long line, out string error);

    PluginCursor GetCursor();

    bool SetCursor(long line, long column, out string error);

    PluginSelection? GetSelection();

    bool InsertText(string text, out string error);

    bool ReplaceRange(object? start, object? finish, string text, out string error);

    void ShowMessage(string text, bool warning);

    void SetPluginStatus(string key, string? text);

    void RequestPrompt(string label, Action<string?> callback);

    void RequestSelect(string title, IReadOnlyList<string> items, Action<int?> callback);

    int OpenPluginPanel(
        PluginPanelOptions options,
        Func<string, long, bool>? onKey,
        string pluginId
    );

    void SetPluginPanelLines(int panelId, IReadOnlyList<PluginPanelLine> lines);

    void SetPluginPanelTitle(int panelId, string title);

    void SetPluginPanelFooter(int panelId, string footer);

    void SetPluginPanelSelected(int panelId, long selected);

    void SetPluginPanelSelectable(int panelId, bool selectable);

    void ClosePluginPanel(int panelId);

    void ClosePluginPanels(string pluginId);

    bool StartLsp(LspServerConfig config, out string error);

    void StopLsp(string? name);

    bool ExecuteCommand(string name, IReadOnlyList<string> args, out string error);

    void ReportPluginError(string pluginId, string message);
}
