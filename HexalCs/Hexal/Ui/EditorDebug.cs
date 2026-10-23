using System.Text.Json.Nodes;
using Hexal.Dap;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private readonly DapManager _dap = new();
    private readonly Dictionary<string, SortedSet<int>> _breakpoints = new(StringComparer.Ordinal);

    private string _dapStatus = string.Empty;
    private string _dapStoppedPath = string.Empty;
    private long _dapStoppedLine = -1;
    private bool _dapStopped;
    private string _debugPathSource = string.Empty;
    private string _debugPath = string.Empty;

    private bool _debugVariablesVisible;
    private int _debugVariablesTop;
    private readonly List<(string Text, Style Style)> _debugVariableLines = new();
    private readonly List<DapScope> _debugScopes = new();
    private readonly List<DapScopeVariables> _debugScopedVariables = new();
    private readonly List<string> _debugWatches = new();
    private readonly Dictionary<string, DapWatchValue> _debugWatchValues = new(
        StringComparer.Ordinal
    );

    private readonly record struct DapLaunchContext(
        string Workspace,
        string File,
        string FileDirectory,
        string FileName,
        string FileNameWithoutExtension,
        string? Executable
    );

    private int EditorMarkerWidth() => _showDiffLines ? 1 : 0;

    private Style? LineHighlightStyle(long line)
    {
        if (IsBreakpointAt(line))
        {
            return LineTint(red: true);
        }

        if (IsCurrentDebugLine(line))
        {
            return LineTint(red: false);
        }

        return null;
    }

    private Style LineTint(bool red)
    {
        bool dark = ThemeBackgroundIsDark();
        uint background;
        if (red)
        {
            // A red tint tuned to the theme's background luminance keeps the
            // breakpoint line visible without washing out syntax colours.
            background = dark ? Theme.Rgb(78, 18, 25) : Theme.Rgb(247, 207, 212);
        }
        else
        {
            background = dark ? Theme.Rgb(56, 44, 14) : Theme.Rgb(250, 235, 190);
        }

        return new Style { Fg = _theme.Foreground, Bg = background };
    }

    private bool ThemeBackgroundIsDark()
    {
        uint background = _theme.Background;
        if (!Theme.IsTrueColor(background))
        {
            // ANSI palette indices 0-6 are dark; 7 is light grey, and 8-15
            // are the bright variants. For the extended palette use a simple
            // cube approximation.
            if (background < 7)
            {
                return true;
            }
            if (background < 16)
            {
                return background < 15;
            }
            if (background < 232)
            {
                int value = (int)background - 16;
                int red = value / 36;
                int green = (value / 6) % 6;
                int blue = value % 6;
                int[] levels = { 0, 95, 135, 175, 215, 255 };
                double luminance =
                    0.2126 * levels[red] + 0.7152 * levels[green] + 0.0722 * levels[blue];
                return luminance < 128;
            }

            return background < 244;
        }

        double trueColorLuminance =
            0.2126 * Theme.Red(background)
            + 0.7152 * Theme.Green(background)
            + 0.0722 * Theme.Blue(background);
        return trueColorLuminance < 128;
    }

    private string DebugStatusText()
    {
        if (!_config.DebugAdapter || _dapStatus.Length == 0)
        {
            return string.Empty;
        }
        return _dapStatus;
    }

    private string DebugPathForCurrentDocument()
    {
        string source = _document.Path ?? string.Empty;
        if (!string.Equals(source, _debugPathSource, StringComparison.Ordinal))
        {
            _debugPathSource = source;
            _debugPath = source.Length == 0 ? string.Empty : NormalizeDebugPath(source);
        }
        return _debugPath;
    }

    private bool IsBreakpointAt(long line)
    {
        if (_breakpoints.Count == 0)
        {
            return false;
        }

        string path = DebugPathForCurrentDocument();
        return path.Length > 0
            && _breakpoints.TryGetValue(path, out SortedSet<int>? lines)
            && lines.Contains((int)line);
    }

    private bool IsCurrentDebugLine(long line)
    {
        if (!_dapStopped || _dapStoppedLine < 0 || _dapStoppedPath.Length == 0)
        {
            return false;
        }

        return _dapStoppedLine == line
            && string.Equals(
                DebugPathForCurrentDocument(),
                _dapStoppedPath,
                StringComparison.Ordinal
            );
    }

    private void ToggleBreakpoint()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }

        if (_language?.Debugger is null)
        {
            SetError(
                _language is null
                    ? "no language is active for breakpoints"
                    : $"no debug adapter is configured for {_language.Name}"
            );
            return;
        }

        if (_document.Path is null)
        {
            SetError("breakpoints require a file-backed buffer");
            return;
        }

        string path = NormalizeDebugPath(_document.Path);
        long line = CursorLine();
        if (line < 0)
        {
            return;
        }

        if (!_breakpoints.TryGetValue(path, out SortedSet<int>? lines))
        {
            lines = new SortedSet<int>();
            _breakpoints[path] = lines;
        }

        bool removed = lines.Remove((int)line);
        if (!removed)
        {
            lines.Add((int)line);
        }

        if (lines.Count == 0)
        {
            _breakpoints.Remove(path);
        }

        SetMessage(
            removed
                ? $"Breakpoint removed at line {line + 1}."
                : $"Breakpoint set at line {line + 1}."
        );

        if (_dap.IsInitialized)
        {
            _dap.SetBreakpoints(path, lines);
        }

        _forceFullRedraw = true;
    }

    private void ToggleDebugVariables()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }

        _debugVariablesVisible = !_debugVariablesVisible;
        _debugVariablesTop = 0;
        if (_debugVariablesVisible)
        {
            RequestDebugState();
            SetMessage("Debug variables shown.");
        }
        else
        {
            SetMessage("Debug variables hidden.");
        }
        _forceFullRedraw = true;
    }

    private void AddDebugWatch(string expression)
    {
        expression = expression.Trim();
        if (expression.Length == 0)
        {
            SetError("usage: :dbg-watch <expression>");
            return;
        }

        if (!_debugWatches.Contains(expression, StringComparer.Ordinal))
        {
            _debugWatches.Add(expression);
        }
        _debugWatchValues.Remove(expression);
        _debugVariablesVisible = true;
        _debugVariablesTop = 0;
        RebuildDebugVariableLines();
        if (_dap.IsStopped)
        {
            _dap.RequestEvaluate(expression);
        }
        SetMessage($"Watching '{expression}'.");
        _forceFullRedraw = true;
    }

    private void RemoveDebugWatch(string expression)
    {
        string value = expression.Trim();
        if (
            _debugWatches.RemoveAll(watch => string.Equals(watch, value, StringComparison.Ordinal))
            == 0
        )
        {
            SetError($"not watching '{value}'");
            return;
        }

        _debugWatchValues.Remove(value);
        RebuildDebugVariableLines();
        SetMessage($"Stopped watching '{value}'.");
        _forceFullRedraw = true;
    }

    private void ClearDebugWatches()
    {
        _debugWatches.Clear();
        _debugWatchValues.Clear();
        RebuildDebugVariableLines();
        SetMessage("Debug watches cleared.");
        _forceFullRedraw = true;
    }

    private void RequestDebugState()
    {
        if (!_debugVariablesVisible && _debugWatches.Count == 0)
        {
            return;
        }

        _debugScopes.Clear();
        _debugScopedVariables.Clear();
        _debugWatchValues.Clear();
        RebuildDebugVariableLines();

        if (!_dap.IsStopped)
        {
            return;
        }

        _dap.RequestDebugVariables();
        foreach (string watch in _debugWatches)
        {
            _dap.RequestEvaluate(watch);
        }
    }

    private void ClearDebugVariableValues()
    {
        _debugScopes.Clear();
        _debugScopedVariables.Clear();
        _debugWatchValues.Clear();
        RebuildDebugVariableLines();
    }

    private void RebuildDebugVariableLines()
    {
        _debugVariableLines.Clear();

        if (!_dap.IsRunning)
        {
            _debugVariableLines.Add(("No debug session.", _theme.MutedStyle));
            return;
        }

        if (!_dap.IsStopped)
        {
            _debugVariableLines.Add(("Program is running.", _theme.MutedStyle));
            if (_debugWatches.Count > 0)
            {
                _debugVariableLines.Add((string.Empty, _theme.TextStyle));
            }
        }

        if (_debugWatches.Count > 0)
        {
            _debugVariableLines.Add(("Watches", _theme.AccentStyle));
            foreach (string expression in _debugWatches)
            {
                if (_debugWatchValues.TryGetValue(expression, out DapWatchValue? watch))
                {
                    if (watch.Error.Length > 0)
                    {
                        _debugVariableLines.Add(
                            ($"  {expression} = <{watch.Error}>", _theme.ErrorStyle)
                        );
                    }
                    else
                    {
                        string type = watch.Type.Length > 0 ? $" ({watch.Type})" : string.Empty;
                        _debugVariableLines.Add(
                            (
                                $"  {expression} = {SanitizeDebugValue(watch.Value)}{type}",
                                _theme.TextStyle
                            )
                        );
                    }
                }
                else
                {
                    _debugVariableLines.Add(($"  {expression} = …", _theme.MutedStyle));
                }
            }
            _debugVariableLines.Add((string.Empty, _theme.TextStyle));
        }

        if (_debugScopes.Count == 0)
        {
            _debugVariableLines.Add(
                (_dap.IsStopped ? "Waiting for scopes…" : "Not stopped.", _theme.MutedStyle)
            );
            return;
        }

        foreach (DapScope scope in _debugScopes)
        {
            _debugVariableLines.Add(($"[{scope.Name}]", _theme.AccentStyle));
            DapScopeVariables? values = _debugScopedVariables.FirstOrDefault(item =>
                string.Equals(item.Name, scope.Name, StringComparison.Ordinal)
            );
            if (values is null || values.Variables.Count == 0)
            {
                _debugVariableLines.Add(("  (no variables)", _theme.MutedStyle));
                continue;
            }

            foreach (DapVariable variable in values.Variables)
            {
                string type = variable.Type.Length > 0 ? $" ({variable.Type})" : string.Empty;
                _debugVariableLines.Add(
                    (
                        $"  {variable.Name} = {SanitizeDebugValue(variable.Value)}{type}",
                        _theme.TextStyle
                    )
                );
            }
        }
    }

    private static string SanitizeDebugValue(string value)
    {
        if (value.Length == 0)
        {
            return "…";
        }

        return value
            .Replace("\r\n", " ", StringComparison.Ordinal)
            .Replace('\n', ' ')
            .Replace('\r', ' ');
    }

    private void StartDebugSession(List<string> arguments)
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }

        if (_language?.Debugger is not { } debugger)
        {
            SetError(
                _language is null
                    ? "no language is active for debugging"
                    : $"no debug adapter is configured for {_language.Name}"
            );
            return;
        }

        if (_document.Path is null)
        {
            SetError("debugging requires a file-backed buffer");
            return;
        }

        if (arguments.Count > 1)
        {
            SetError("usage: :debug [executable]");
            return;
        }

        string? executable =
            arguments.Count == 1
                ? NormalizeDebugPath(arguments[0])
                : FindDefaultDebugExecutable(_document.Path);

        if (executable is null && DebuggerUsesExecutable(debugger.Configuration))
        {
            SetError("debug: no executable found; use :debug <program>");
            return;
        }

        var configuration = (JsonObject)debugger.Configuration.DeepClone();
        if (executable is not null && !configuration.ContainsKey("program"))
        {
            configuration["program"] = executable;
        }

        string sourcePath = NormalizeDebugPath(_document.Path);
        string fileDirectory = Path.GetDirectoryName(sourcePath) ?? _workspaceRoot;
        string fileName = Path.GetFileName(sourcePath);
        string fileNameWithoutExtension = Path.GetFileNameWithoutExtension(sourcePath);
        var context = new DapLaunchContext(
            _workspaceRoot,
            sourcePath,
            fileDirectory,
            fileName,
            fileNameWithoutExtension,
            executable
        );

        if (!TryResolveDapNode(configuration, context, out string resolveError))
        {
            SetError(resolveError);
            return;
        }

        if (
            _dap.TryStart(
                debugger,
                sourcePath,
                _workspaceRoot,
                configuration,
                _breakpoints,
                out string error
            )
        )
        {
            _dapStatus = $"DBG: starting {debugger.Adapter}";
            _dapStopped = false;
            _dapStoppedPath = string.Empty;
            _dapStoppedLine = -1;
            ClearMessage();
            SetMessage($"Debug: starting {debugger.Adapter}.");
            _forceFullRedraw = true;
            return;
        }

        SetError($"debug: {error}");
    }

    private void ContinueDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        if (!_dap.Continue())
        {
            SetError("debugger is not stopped");
            return;
        }
        _dapStatus = "DBG: running";
        _dapStopped = false;
        _dapStoppedPath = string.Empty;
        _dapStoppedLine = -1;
        ClearDebugVariableValues();
        SetMessage("Debug: continued.");
    }

    private void NextDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        if (!_dap.Next())
        {
            SetError("debugger is not stopped");
            return;
        }
        _dapStatus = "DBG: stepping";
        _dapStopped = false;
        ClearDebugVariableValues();
        SetMessage("Debug: step over.");
    }

    private void StepInDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        if (!_dap.StepIn())
        {
            SetError("debugger is not stopped");
            return;
        }
        _dapStatus = "DBG: stepping";
        _dapStopped = false;
        ClearDebugVariableValues();
        SetMessage("Debug: step in.");
    }

    private void StepOutDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        if (!_dap.StepOut())
        {
            SetError("debugger is not stopped");
            return;
        }
        _dapStatus = "DBG: stepping";
        _dapStopped = false;
        ClearDebugVariableValues();
        SetMessage("Debug: step out.");
    }

    private void PauseDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        if (!_dap.Pause())
        {
            SetError("debugger is not running");
            return;
        }
        _dapStatus = "DBG: pausing";
        SetMessage("Debug: pause requested.");
    }

    private void StopDebugSession()
    {
        if (!EnsureDebugAdapterEnabled())
        {
            return;
        }
        _dap.Stop();
        _dapStatus = string.Empty;
        _dapStopped = false;
        _dapStoppedPath = string.Empty;
        _dapStoppedLine = -1;
        ClearDebugVariableValues();
        SetMessage("Debug session stopped.");
        _forceFullRedraw = true;
    }

    private bool EnsureDebugAdapterEnabled()
    {
        if (_config.DebugAdapter)
        {
            return true;
        }

        SetError("debug adapter support is disabled (debugAdapter in .hexcfg)");
        return false;
    }

    private void ApplyDebugAdapterConfiguration()
    {
        if (_config.DebugAdapter)
        {
            return;
        }

        if (_dap.IsRunning)
        {
            _dap.Stop();
        }
        _dapStatus = string.Empty;
        _dapStopped = false;
        _dapStoppedPath = string.Empty;
        _dapStoppedLine = -1;
    }

    private void DisposeDebugger()
    {
        _dap.Dispose();
    }

    private bool PollDap()
    {
        bool redraw = false;
        while (_dap.TryDequeueEvent(out DapEvent dapEvent))
        {
            redraw = true;
            HandleDapEvent(dapEvent);
        }
        return redraw;
    }

    private void HandleDapEvent(DapEvent dapEvent)
    {
        switch (dapEvent.Kind)
        {
            case DapEventKind.Started:
                _dapStatus = $"DBG: starting {dapEvent.AdapterName}";
                break;

            case DapEventKind.Initialized:
                _dapStatus = $"DBG: {dapEvent.AdapterName}";
                break;

            case DapEventKind.Stopped:
                _dapStopped = true;
                _dapStatus = string.IsNullOrEmpty(dapEvent.Reason)
                    ? "DBG: stopped"
                    : $"DBG: stopped ({dapEvent.Reason})";
                if (!string.IsNullOrEmpty(dapEvent.Reason))
                {
                    SetMessage($"Debug: stopped ({dapEvent.Reason}).");
                }
                _forceFullRedraw = true;
                break;

            case DapEventKind.StoppedLocation:
            {
                _dapStopped = true;
                _dapStoppedPath =
                    dapEvent.Path.Length > 0 ? NormalizeDebugPath(dapEvent.Path) : string.Empty;
                _dapStoppedLine = dapEvent.Line;
                string location =
                    dapEvent.Line >= 0 && dapEvent.Path.Length > 0
                        ? $"{Path.GetFileName(dapEvent.Path)}:{dapEvent.Line + 1}"
                        : "an unknown location";
                _dapStatus = $"DBG: stopped {location}";
                SetMessage($"Debug: stopped at {location}.");

                if (dapEvent.Line >= 0 && dapEvent.Path.Length > 0)
                {
                    if (_document.Path is not null && SamePath(_document.Path, dapEvent.Path))
                    {
                        MoveToLineColumn(dapEvent.Line, 0);
                    }
                    else if (!_document.IsDirty && File.Exists(dapEvent.Path))
                    {
                        OpenPathAt(dapEvent.Path, dapEvent.Line, 0);
                    }
                }

                RequestDebugState();
                _forceFullRedraw = true;
                break;
            }

            case DapEventKind.Continued:
                _dapStopped = false;
                _dapStoppedPath = string.Empty;
                _dapStoppedLine = -1;
                _dapStatus = "DBG: running";
                ClearDebugVariableValues();
                SetMessage("Debug: running.");
                _forceFullRedraw = true;
                break;

            case DapEventKind.Exited:
                _dapStopped = false;
                _dapStoppedPath = string.Empty;
                _dapStoppedLine = -1;
                _dapStatus = $"DBG: exited ({dapEvent.ExitCode})";
                ClearDebugVariableValues();
                SetMessage($"Debug: program exited ({dapEvent.ExitCode}).");
                _forceFullRedraw = true;
                break;

            case DapEventKind.Terminated:
                _dapStopped = false;
                _dapStoppedPath = string.Empty;
                _dapStoppedLine = -1;
                _dapStatus = string.Empty;
                ClearDebugVariableValues();
                SetMessage("Debug session terminated.");
                _forceFullRedraw = true;
                break;

            case DapEventKind.Scopes:
                _debugScopes.Clear();
                _debugScopes.AddRange(dapEvent.Scopes);
                RebuildDebugVariableLines();
                _forceFullRedraw = true;
                break;

            case DapEventKind.Variables:
                _debugScopedVariables.Clear();
                _debugScopedVariables.AddRange(dapEvent.ScopedVariables);
                RebuildDebugVariableLines();
                _forceFullRedraw = true;
                break;

            case DapEventKind.WatchValue:
                if (dapEvent.Watch is { } watch)
                {
                    _debugWatchValues[watch.Expression] = watch;
                    RebuildDebugVariableLines();
                    _forceFullRedraw = true;
                }
                break;

            case DapEventKind.Breakpoints:
                foreach (DapBreakpoint breakpoint in dapEvent.Breakpoints)
                {
                    if (!breakpoint.Verified && breakpoint.Message.Length > 0)
                    {
                        SetError($"debug breakpoint {breakpoint.Line}: {breakpoint.Message}");
                    }
                }
                break;

            case DapEventKind.Error:
                SetError($"DAP: {dapEvent.Message}");
                break;
        }
    }

    private static bool DebuggerUsesExecutable(JsonNode? node)
    {
        switch (node)
        {
            case JsonObject obj:
                foreach ((_, JsonNode? value) in obj)
                {
                    if (DebuggerUsesExecutable(value))
                    {
                        return true;
                    }
                }
                return false;
            case JsonArray array:
                foreach (JsonNode? value in array)
                {
                    if (DebuggerUsesExecutable(value))
                    {
                        return true;
                    }
                }
                return false;
            case JsonValue value:
                return value.TryGetValue(out string? text)
                    && text.Contains("${executable}", StringComparison.Ordinal);
            default:
                return false;
        }
    }

    private static bool TryResolveDapNode(
        JsonNode? node,
        DapLaunchContext context,
        out string error
    )
    {
        error = string.Empty;
        switch (node)
        {
            case JsonObject obj:
            {
                foreach (string key in obj.Select(pair => pair.Key).ToList())
                {
                    JsonNode? value = obj[key];
                    if (value is JsonValue jsonValue && jsonValue.TryGetValue(out string? text))
                    {
                        if (!TryResolveDapString(text!, context, out string resolved, out error))
                        {
                            return false;
                        }
                        obj[key] = resolved;
                    }
                    else if (value is not null && !TryResolveDapNode(value, context, out error))
                    {
                        return false;
                    }
                }
                return true;
            }

            case JsonArray array:
            {
                for (int index = 0; index < array.Count; ++index)
                {
                    JsonNode? value = array[index];
                    if (value is JsonValue jsonValue && jsonValue.TryGetValue(out string? text))
                    {
                        if (!TryResolveDapString(text!, context, out string resolved, out error))
                        {
                            return false;
                        }
                        array[index] = resolved;
                    }
                    else if (value is not null && !TryResolveDapNode(value, context, out error))
                    {
                        return false;
                    }
                }
                return true;
            }

            default:
                return true;
        }
    }

    private static bool TryResolveDapString(
        string text,
        DapLaunchContext context,
        out string resolved,
        out string error
    )
    {
        resolved = text.Replace("${workspace}", context.Workspace, StringComparison.Ordinal)
            .Replace("${file}", context.File, StringComparison.Ordinal)
            .Replace("${fileDir}", context.FileDirectory, StringComparison.Ordinal)
            .Replace("${fileBasename}", context.FileName, StringComparison.Ordinal)
            .Replace(
                "${fileBasenameNoExtension}",
                context.FileNameWithoutExtension,
                StringComparison.Ordinal
            );

        error = string.Empty;
        if (!resolved.Contains("${executable}", StringComparison.Ordinal))
        {
            return true;
        }

        if (string.IsNullOrEmpty(context.Executable))
        {
            error =
                "debug: ${executable} is set but no program was provided; "
                + "use :debug <program>";
            return false;
        }

        resolved = resolved.Replace("${executable}", context.Executable, StringComparison.Ordinal);
        return true;
    }

    private string? FindDefaultDebugExecutable(string sourcePath)
    {
        string sourceFull = NormalizeDebugPath(sourcePath);
        string directory = Path.GetDirectoryName(sourceFull) ?? _workspaceRoot;
        string name = Path.GetFileNameWithoutExtension(sourceFull);
        string[] candidates =
        {
            Path.Combine(directory, name),
            Path.Combine(_workspaceRoot, name),
            Path.Combine(_workspaceRoot, "build", name),
            Path.Combine(_workspaceRoot, "bin", name),
            Path.Combine(_workspaceRoot, "target", "debug", name),
            Path.Combine(_workspaceRoot, "target", "release", name),
            Path.Combine(_workspaceRoot, "zig-out", "bin", name),
            Path.Combine(_workspaceRoot, "out", name),
            Path.Combine(directory, "build", name),
            Path.Combine(directory, "bin", name),
        };

        foreach (string candidate in candidates)
        {
            if (File.Exists(candidate))
            {
                return NormalizeDebugPath(candidate);
            }
        }

        return null;
    }

    private static string NormalizeDebugPath(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return path;
        }
    }
}
