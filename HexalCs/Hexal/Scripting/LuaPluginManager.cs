using System.Diagnostics;
using System.Globalization;
using System.Text;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Util;
using NLua;
using NLua.Exceptions;

namespace Hexal.Scripting;

/// <summary>
/// A snapshot of one discovered plugin manifest. It is also available for
/// plugins that are currently disabled, so the plugin manager can show all
/// installed plugins and their metadata.
/// </summary>
public sealed class PluginInfo
{
    public string Id { get; init; } = string.Empty;
    public string Name { get; init; } = string.Empty;
    public string Version { get; init; } = string.Empty;
    public string Description { get; init; } = string.Empty;
    public string Author { get; init; } = string.Empty;
    public string License { get; init; } = string.Empty;
    public string Directory { get; init; } = string.Empty;
    public string EntryPath { get; init; } = string.Empty;
    public string ManifestPath { get; init; } = string.Empty;
    public bool Enabled { get; init; }
    public bool Running { get; init; }
    public bool Deferred { get; init; }
    public string Error { get; init; } = string.Empty;
    public IReadOnlyList<string> Commands { get; init; } = Array.Empty<string>();
}

/// <summary>
/// Loads Lua plugins into isolated NLua states and exposes only the documented
/// hexal.* modules. The editor talks to this class on the UI thread; Lua never
/// receives editor objects or the CLR package.
/// </summary>
public sealed class LuaPluginManager : IDisposable
{
    private const int PendingKeyTimeoutMs = 800;

    private sealed class LoadedPlugin
    {
        public string Id = string.Empty;
        public string Name = string.Empty;
        public string Directory = string.Empty;
        public string EntryPath = string.Empty;
        public Lua Lua = null!;
        public LuaTable Hexal = null!;
        public PluginConfigStore Config = null!;
        public LuaLspCoordinator? Lsp;
        public bool LspBinding;
        public bool Running;
    }

    private sealed class PluginEntry
    {
        public string Id = string.Empty;
        public string Name = string.Empty;
        public string Version = string.Empty;
        public string Description = string.Empty;
        public string Author = string.Empty;
        public string License = string.Empty;
        public string Directory = string.Empty;
        public string EntryPath = string.Empty;
        public string ManifestPath = string.Empty;
        public string Error = string.Empty;
        public string LazyLanguage = string.Empty;
        public bool Enabled;
        public bool Deferred;
        public LoadedPlugin? Plugin;
    }

    private sealed class PluginCommand
    {
        public string Name = string.Empty;
        public string Description = string.Empty;
        public string PluginId = string.Empty;
        public LuaFunction Callback = null!;
    }

    private sealed class PluginKeyBinding
    {
        public string PluginId = string.Empty;
        public string Mode = "normal";
        public string[] Keys = Array.Empty<string>();
        public string? Command;
        public LuaFunction? Callback;
    }

    private sealed class EventSubscription
    {
        public long Id;
        public string Event = string.Empty;
        public string PluginId = string.Empty;
        public LuaFunction Callback = null!;
    }

    private static readonly object WarmupLock = new();
    private static Task? s_warmupTask;

    private readonly IEditorApi _host;
    private readonly string _pluginsDirectory;
    private readonly string _statePath;
    private readonly List<PluginEntry> _entries = new();
    private readonly HashSet<string> _disabledPlugins = new(StringComparer.Ordinal);
    private readonly List<LoadedPlugin> _plugins = new();
    private readonly Dictionary<string, PluginCommand> _commands = new(
        StringComparer.OrdinalIgnoreCase
    );
    private readonly List<PluginKeyBinding> _keyBindings = new();
    private readonly Dictionary<long, EventSubscription> _subscriptions = new();
    private readonly List<string> _errors = new();
    private readonly List<string> _pendingKeys = new();
    private long _nextSubscriptionId = 1;
    private string _pendingMode = string.Empty;
    private long _pendingSince;
    private bool _inEvent;
    private bool _disposed;
    private StartupBenchmark? _benchmark;

    public LuaPluginManager(IEditorApi host)
        : this(host, DefaultPluginDirectory, DefaultStatePath) { }

    public LuaPluginManager(IEditorApi host, string pluginDirectory)
        : this(host, pluginDirectory, string.Empty) { }

    public LuaPluginManager(IEditorApi host, string pluginDirectory, string statePath)
    {
        _host = host;
        _pluginsDirectory = pluginDirectory;
        _statePath = statePath;
    }

    /// <summary>
    /// Starts loading the native Lua runtime on a background thread. The
    /// first Lua state initialization includes one-time native library and
    /// runtime setup; overlapping it with grammar/theme loading removes that
    /// cost from the plugin-load phase without changing plugin isolation.
    /// </summary>
    public static void BeginWarmup()
    {
        lock (WarmupLock)
        {
            s_warmupTask ??= Task.Run(static () => WarmupRuntime());
        }
    }

    private static void WarmupRuntime()
    {
        try
        {
            using var lua = new Lua();
            lua.State.Encoding = Encoding.UTF8;
            lua.NewTable("hexal");
            var plugin = new LoadedPlugin
            {
                Id = "warmup",
                Lua = lua,
                Hexal = (LuaTable)lua["hexal"],
                Config = new PluginConfigStore(string.Empty),
            };

            // The first plugin load pays the one-time JIT cost of building the
            // complete hexal.* module surface. Do it here, concurrently with
            // grammar and theme loading, using a throwaway manager/state.
            var manager = new LuaPluginManager(null!, string.Empty, string.Empty);
            manager.RegisterModules(plugin);
        }
        catch
        {
            // Warmup is best effort; real plugin loading reports errors.
        }
    }

    public IReadOnlyList<string> Errors => _errors;

    public int PluginCount => _plugins.Count;

    /// <summary>
    /// Number of enabled plugins whose Lua state is created on demand when a
    /// buffer of their declared language is first opened.
    /// </summary>
    public int DeferredPluginCount =>
        _entries.Count(entry => entry.Deferred && entry.Enabled && entry.Plugin is null);

    public void SetBenchmark(StartupBenchmark? benchmark)
    {
        _benchmark = benchmark;
    }

    public static string DefaultStatePath =>
        Path.Combine(HexalPaths.HexalHomeDirectory, "disabled-plugins");

    /// <summary>
    /// Returns a snapshot of every discovered plugin, including plugins that
    /// are currently disabled or failed to load.
    /// </summary>
    public IReadOnlyList<PluginInfo> Plugins
    {
        get
        {
            var result = new List<PluginInfo>(_entries.Count);
            foreach (PluginEntry entry in _entries)
            {
                result.Add(
                    new PluginInfo
                    {
                        Id = entry.Id,
                        Name = entry.Name,
                        Version = entry.Version,
                        Description = entry.Description,
                        Author = entry.Author,
                        License = entry.License,
                        Directory = entry.Directory,
                        EntryPath = entry.EntryPath,
                        ManifestPath = entry.ManifestPath,
                        Enabled = entry.Enabled,
                        Running = entry.Plugin?.Running ?? false,
                        Deferred = entry.Deferred && entry.Enabled && entry.Plugin is null,
                        Error = entry.Error,
                        Commands = CommandsForPlugin(entry.Id),
                    }
                );
            }
            return result;
        }
    }

    /// <summary>
    /// Enables or disables a discovered plugin. Enabling loads the plugin into
    /// a fresh Lua state; disabling unregisters its commands, keymaps, events
    /// and language servers before releasing the state.
    /// </summary>
    public bool SetPluginEnabled(string id, bool enabled, out string error)
    {
        error = string.Empty;
        PluginEntry? entry = FindEntry(id);
        if (entry is null)
        {
            error = $"unknown plugin '{id}'";
            return false;
        }

        if (enabled)
        {
            if (entry.Plugin is null)
            {
                StartPlugin(entry);
            }

            entry.Enabled = entry.Plugin is not null;
            if (!entry.Enabled)
            {
                error = entry.Error.Length > 0 ? entry.Error : $"could not enable plugin '{id}'";
                return false;
            }

            _disabledPlugins.Remove(entry.Id);
        }
        else
        {
            StopPlugin(entry);
            entry.Enabled = false;
            _disabledPlugins.Add(entry.Id);
        }

        SaveState();
        return true;
    }

    /// <summary>
    /// Re-reads a plugin manifest and entry point from disk, replacing the
    /// running instance. Disabled plugins stay disabled.
    /// </summary>
    public bool ReloadPlugin(string id, out string error)
    {
        error = string.Empty;
        PluginEntry? entry = FindEntry(id);
        if (entry is null)
        {
            error = $"unknown plugin '{id}'";
            return false;
        }

        PluginEntry reloaded = ReloadEntry(entry);
        if (reloaded.Error.Length > 0)
        {
            error = $"{reloaded.Id}: {reloaded.Error}";
            return false;
        }

        return true;
    }

    /// <summary>
    /// Re-scans the plugin directory and restarts every enabled plugin. This
    /// also discovers plugins that were added after startup.
    /// </summary>
    public bool ReloadAll(out string error)
    {
        error = string.Empty;

        foreach (PluginEntry entry in _entries.ToArray())
        {
            StopPlugin(entry);
        }

        _entries.Clear();
        _errors.Clear();
        _commands.Clear();
        _keyBindings.Clear();
        _subscriptions.Clear();
        _pendingKeys.Clear();
        LoadAll();

        if (_errors.Count > 0)
        {
            error = _errors[0];
            return false;
        }

        return true;
    }

    private PluginEntry ReloadEntry(PluginEntry entry)
    {
        int index = _entries.IndexOf(entry);
        string directory = entry.Directory;
        string oldId = entry.Id;

        StopPlugin(entry);
        _entries.Remove(entry);
        _errors.RemoveAll(error => error.StartsWith(oldId + ": ", StringComparison.Ordinal));

        DiscoverPlugin(directory, _disabledPlugins);

        PluginEntry reloaded = _entries[^1];
        int last = _entries.Count - 1;
        if (index >= 0 && index < last)
        {
            _entries.RemoveAt(last);
            _entries.Insert(index, reloaded);
        }

        return reloaded;
    }

    private List<string> CommandsForPlugin(string pluginId)
    {
        var names = new List<string>();
        foreach ((string name, PluginCommand command) in _commands)
        {
            if (string.Equals(command.PluginId, pluginId, StringComparison.Ordinal))
            {
                names.Add(name);
            }
        }
        names.Sort(StringComparer.OrdinalIgnoreCase);
        return names;
    }

    public IReadOnlyList<LuaLspCoordinator> Coordinators =>
        _plugins.Where(plugin => plugin.Lsp is not null).Select(plugin => plugin.Lsp!).ToArray();

    public void Tick()
    {
        if (_disposed)
        {
            return;
        }
        foreach (LoadedPlugin plugin in _plugins)
        {
            plugin.Lsp?.Tick();
        }
    }

    public void ActivateLuaLsp()
    {
        if (_disposed)
        {
            return;
        }
        foreach (LoadedPlugin plugin in _plugins)
        {
            plugin.Lsp?.ActivateServers();
        }
    }

    public static string DefaultPluginDirectory => HexalPaths.PluginDirectory;

    public void LoadAll()
    {
        try
        {
            Directory.CreateDirectory(_pluginsDirectory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            _errors.Add($"cannot create plugin directory '{_pluginsDirectory}': {ex.Message}");
            return;
        }

        string[] directories;
        try
        {
            directories = Directory.GetDirectories(_pluginsDirectory);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            _errors.Add($"cannot read plugin directory '{_pluginsDirectory}': {ex.Message}");
            return;
        }

        HashSet<string> disabled = LoadDisabledPlugins();

        Array.Sort(directories, StringComparer.OrdinalIgnoreCase);
        foreach (string directory in directories)
        {
            DiscoverPlugin(directory, disabled);
        }

        HexalPaths.Log(
            $"Plugins: {_pluginsDirectory} ({_plugins.Count} loaded, {_errors.Count} error(s))"
        );
    }

    public bool HasCommand(string name) => _commands.ContainsKey(name);

    public IEnumerable<string> CommandNames => _commands.Keys;

    public bool TryGetCommandDescription(string name, out string description)
    {
        if (_commands.TryGetValue(name, out PluginCommand? command))
        {
            description = command.Description;
            return true;
        }

        description = string.Empty;
        return false;
    }

    public bool TryExecuteCommand(string name, IReadOnlyList<string> args, out string error)
    {
        error = string.Empty;
        if (!_commands.TryGetValue(name, out PluginCommand? command))
        {
            return false;
        }

        try
        {
            command.Callback.Call(args.Cast<object>().ToArray());
        }
        catch (Exception ex)
        {
            error = $"plugin command '{name}' failed: {ex.Message}";
            _host.ReportPluginError(command.PluginId, error);
        }

        return true;
    }

    public bool HasSubscribers(string eventName)
    {
        foreach (EventSubscription subscription in _subscriptions.Values)
        {
            if (string.Equals(subscription.Event, eventName, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }
        return false;
    }

    public void FireEvent(string eventName, IReadOnlyDictionary<string, object?>? data = null)
    {
        if (_inEvent || _disposed)
        {
            return;
        }

        if (
            string.Equals(eventName, "buffer_opened", StringComparison.OrdinalIgnoreCase)
            && data is not null
            && data.TryGetValue("language", out object? languageValue)
            && languageValue?.ToString() is { Length: > 0 } language
        )
        {
            LoadDeferredPlugins(language);
        }

        List<EventSubscription>? matching = null;
        foreach (EventSubscription subscription in _subscriptions.Values)
        {
            if (string.Equals(subscription.Event, eventName, StringComparison.OrdinalIgnoreCase))
            {
                matching ??= new List<EventSubscription>();
                matching.Add(subscription);
            }
        }

        if (matching is null)
        {
            return;
        }

        _inEvent = true;
        try
        {
            foreach (EventSubscription subscription in matching)
            {
                LoadedPlugin? plugin = FindPlugin(subscription.PluginId);
                if (plugin is null || !plugin.Running)
                {
                    continue;
                }

                try
                {
                    LuaTable arguments = data is null
                        ? NewTable(plugin.Lua)
                        : (LuaTable)ToLua(plugin, data)!;
                    subscription.Callback.Call(arguments);
                }
                catch (Exception ex)
                {
                    _host.ReportPluginError(
                        subscription.PluginId,
                        $"event '{eventName}' failed: {ex.Message}"
                    );
                }
            }
        }
        finally
        {
            _inEvent = false;
        }
    }

    private void LoadDeferredPlugins(string language)
    {
        foreach (PluginEntry entry in _entries.ToArray())
        {
            if (
                !entry.Deferred
                || !entry.Enabled
                || entry.Plugin is not null
                || !string.Equals(entry.LazyLanguage, language, StringComparison.OrdinalIgnoreCase)
            )
            {
                continue;
            }

            StartPlugin(entry);
        }
    }

    /// <summary>
    /// Feeds a key to plugin keymaps. Returns true when the key was consumed
    /// (executed or is part of a pending multi-key sequence).
    /// </summary>
    public bool TryHandleKey(InputEvent inputEvent, string mode, out bool handled)
    {
        handled = false;
        if (_keyBindings.Count == 0)
        {
            return false;
        }

        if (
            _pendingKeys.Count > 0
            && (
                Environment.TickCount64 - _pendingSince > PendingKeyTimeoutMs
                || !string.Equals(_pendingMode, mode, StringComparison.OrdinalIgnoreCase)
            )
        )
        {
            _pendingKeys.Clear();
        }

        string token = KeyToken(inputEvent);
        if (token.Length == 0)
        {
            return false;
        }

        int index = _pendingKeys.Count;
        var candidates = new List<PluginKeyBinding>();
        foreach (PluginKeyBinding binding in _keyBindings)
        {
            if (
                !string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase)
                || binding.Keys.Length <= index
                || !string.Equals(binding.Keys[index], token, StringComparison.Ordinal)
            )
            {
                continue;
            }

            bool matchesPrefix = true;
            for (int i = 0; i < index; ++i)
            {
                if (!string.Equals(binding.Keys[i], _pendingKeys[i], StringComparison.Ordinal))
                {
                    matchesPrefix = false;
                    break;
                }
            }

            if (matchesPrefix)
            {
                candidates.Add(binding);
            }
        }

        if (candidates.Count == 0)
        {
            if (_pendingKeys.Count > 0)
            {
                _pendingKeys.Clear();
                return false;
            }
            return false;
        }

        _pendingKeys.Add(token);
        _pendingMode = mode;
        _pendingSince = Environment.TickCount64;

        foreach (PluginKeyBinding binding in candidates)
        {
            if (binding.Keys.Length != _pendingKeys.Count)
            {
                continue;
            }

            string[] keys = _pendingKeys.ToArray();
            _pendingKeys.Clear();

            if (binding.Command is not null)
            {
                if (
                    !TryExecuteCommand(binding.Command, Array.Empty<string>(), out string error)
                    && error.Length == 0
                )
                {
                    _host.ShowMessage($"unknown plugin command '{binding.Command}'", true);
                }
                else if (error.Length > 0)
                {
                    _host.ShowMessage(error, true);
                }
            }
            else if (binding.Callback is not null)
            {
                try
                {
                    object[] result = binding.Callback.Call();
                    if (result.Length > 0 && result[0] is bool passThrough && !passThrough)
                    {
                        // A callback can decline the binding and let the editor
                        // handle the key normally. Anything else consumes it.
                        handled = false;
                        return false;
                    }
                }
                catch (Exception ex)
                {
                    _host.ReportPluginError(
                        binding.PluginId,
                        $"keymap '{string.Join(' ', keys)}' failed: {ex.Message}"
                    );
                }
            }

            handled = true;
            return true;
        }

        handled = true;
        return true;
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;

        foreach (LoadedPlugin plugin in _plugins)
        {
            try
            {
                plugin.Lsp?.Dispose();
            }
            catch
            {
                // Coordinator teardown is best effort.
            }

            try
            {
                plugin.Lua.Dispose();
            }
            catch
            {
                // Plugin teardown is best effort.
            }
        }

        _plugins.Clear();
        _entries.Clear();
        _disabledPlugins.Clear();
        _commands.Clear();
        _keyBindings.Clear();
        _subscriptions.Clear();
        _pendingKeys.Clear();
    }

    /// <summary>
    /// Reads a plugin manifest and records it even when the plugin is disabled
    /// or cannot be loaded, so it still appears in the plugin manager.
    /// </summary>
    private void DiscoverPlugin(string directory, HashSet<string> disabled)
    {
        string fallbackId = Path.GetFileName(directory);
        string manifestPath = Path.Combine(directory, "plugin.toml");
        Dictionary<string, string> manifest = new(StringComparer.OrdinalIgnoreCase);

        var entry = new PluginEntry
        {
            Id = fallbackId,
            Name = fallbackId,
            Directory = directory,
            ManifestPath = manifestPath,
        };
        _entries.Add(entry);

        if (
            File.Exists(manifestPath)
            && !TryParseManifest(manifestPath, manifest, out string manifestError)
        )
        {
            RecordDiscoveryError(entry, manifestError);
            return;
        }

        if (
            manifest.TryGetValue("id", out string? manifestId)
            && !string.IsNullOrWhiteSpace(manifestId)
        )
        {
            entry.Id = manifestId.Trim();
        }

        if (
            entry.Id.Length == 0
            || entry.Id.Contains('/')
            || entry.Id.Contains('\\')
            || entry.Id is "." or ".."
        )
        {
            entry.Id = fallbackId;
            RecordDiscoveryError(entry, "invalid plugin id");
            return;
        }

        entry.Name =
            manifest.TryGetValue("name", out string? manifestName)
            && !string.IsNullOrWhiteSpace(manifestName)
                ? manifestName.Trim()
                : entry.Id;
        entry.Version = ReadManifestValue(manifest, "version");
        entry.Description = ReadManifestValue(manifest, "description");
        entry.Author = ReadManifestValue(manifest, "author");
        entry.License = ReadManifestValue(manifest, "license");

        string entryFile =
            manifest.TryGetValue("entry", out string? manifestEntry)
            && !string.IsNullOrWhiteSpace(manifestEntry)
                ? manifestEntry.Trim()
                : "init.lua";

        string entryPath;
        try
        {
            entryPath = Path.GetFullPath(Path.Combine(directory, entryFile));
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            RecordDiscoveryError(entry, $"invalid entry point: {ex.Message}");
            return;
        }

        string fullDirectory = Path.GetFullPath(directory);
        if (
            !entryPath.StartsWith(
                fullDirectory + Path.DirectorySeparatorChar,
                StringComparison.Ordinal
            ) && !string.Equals(entryPath, fullDirectory, StringComparison.Ordinal)
        )
        {
            RecordDiscoveryError(entry, "entry point escapes the plugin directory");
            return;
        }

        entry.EntryPath = entryPath;
        if (!File.Exists(entryPath))
        {
            RecordDiscoveryError(entry, $"entry point '{entryFile}' not found");
            return;
        }

        bool manifestEnabled =
            !manifest.TryGetValue("enabled", out string? enabled)
            || !string.Equals(enabled, "false", StringComparison.OrdinalIgnoreCase);
        entry.Enabled = manifestEnabled && !disabled.Contains(entry.Id);

        bool lazy =
            manifest.TryGetValue("lazy", out string? lazyValue)
            && string.Equals(lazyValue, "true", StringComparison.OrdinalIgnoreCase);
        entry.LazyLanguage = ReadManifestValue(manifest, "language");
        if (entry.Enabled && lazy && entry.LazyLanguage.Length > 0)
        {
            entry.Deferred = true;
            return;
        }

        if (entry.Enabled)
        {
            StartPlugin(entry);
        }
    }

    private static string ReadManifestValue(Dictionary<string, string> manifest, string key) =>
        manifest.TryGetValue(key, out string? value) && !string.IsNullOrWhiteSpace(value)
            ? value.Trim()
            : string.Empty;

    private void RecordDiscoveryError(PluginEntry entry, string message)
    {
        entry.Error = message;
        entry.Enabled = false;
        _errors.Add($"{entry.Id}: {message}");
    }

    /// <summary>
    /// Creates a fresh Lua state for a discovered plugin. Failure leaves the
    /// entry in the list with an error so the plugin manager can show it and
    /// the user can retry after fixing the plugin.
    /// </summary>
    private void StartPlugin(PluginEntry entry)
    {
        if (_benchmark is null)
        {
            StartPluginCore(entry);
            return;
        }

        long started = Stopwatch.GetTimestamp();
        try
        {
            StartPluginCore(entry);
        }
        finally
        {
            _benchmark.Add(
                "Plugins",
                entry.Id,
                Stopwatch.GetElapsedTime(started).TotalMilliseconds,
                entry.Enabled ? "started" : entry.Error
            );
        }
    }

    private void StartPluginCore(PluginEntry entry)
    {
        if (entry.Plugin is not null)
        {
            entry.Enabled = true;
            return;
        }

        _errors.RemoveAll(error => error.StartsWith(entry.Id + ": ", StringComparison.Ordinal));

        var plugin = new LoadedPlugin
        {
            Id = entry.Id,
            Name = entry.Name,
            Directory = entry.Directory,
            EntryPath = entry.EntryPath,
        };

        string configDirectory = DefaultPluginConfigDirectory;
        string configPath = Path.Combine(configDirectory, SanitizeFileName(entry.Id) + ".json");
        plugin.Config = new PluginConfigStore(configPath);
        if (!plugin.Config.TryLoad(out string configError))
        {
            _errors.Add($"{entry.Id}: config: {configError}");
        }

        try
        {
            plugin.Lua = new Lua();
            plugin.Lua.State.Encoding = Encoding.UTF8;
            plugin.Lua.NewTable("hexal");
            plugin.Hexal = (LuaTable)plugin.Lua["hexal"];
            plugin.Hexal["api_version"] = 1;

            plugin.Lua["__hexal_plugin_dir"] = entry.Directory.Replace('\\', '/');
            plugin.Lua.DoString(
                "package.path = __hexal_plugin_dir .. '/?.lua;' .. "
                    + "__hexal_plugin_dir .. '/?/init.lua;' .. package.path"
            );

            RegisterModules(plugin);
            plugin.Lua.DoFile(entry.EntryPath);
            plugin.Running = true;
            _plugins.Add(plugin);
            entry.Plugin = plugin;
            entry.Error = string.Empty;
            entry.Enabled = true;
            entry.Deferred = false;
        }
        catch (Exception ex)
        {
            string message = ex is LuaScriptException luaError ? luaError.Message : ex.Message;
            entry.Error = message;
            entry.Enabled = false;
            _errors.Add($"{entry.Id}: {message}");
            CleanupPlugin(entry.Id);
            try
            {
                plugin.Lsp?.Dispose();
            }
            catch
            {
                // Ignore coordinator teardown for a failed plugin.
            }
            try
            {
                plugin.Lua?.Dispose();
            }
            catch
            {
                // Ignore teardown failures for a failed plugin.
            }
        }
    }

    /// <summary>
    /// Unregisters a running plugin's resources and releases its Lua state.
    /// </summary>
    private void StopPlugin(PluginEntry entry)
    {
        if (_benchmark is null)
        {
            StopPluginCore(entry);
            return;
        }

        long started = Stopwatch.GetTimestamp();
        try
        {
            StopPluginCore(entry);
        }
        finally
        {
            _benchmark.Add(
                "Plugins (stop)",
                entry.Id,
                Stopwatch.GetElapsedTime(started).TotalMilliseconds
            );
        }
    }

    private void StopPluginCore(PluginEntry entry)
    {
        LoadedPlugin? plugin = entry.Plugin;
        if (plugin is null)
        {
            return;
        }

        CleanupPlugin(entry.Id);
        try
        {
            plugin.Lsp?.Dispose();
        }
        catch
        {
            // Coordinator teardown is best effort.
        }
        try
        {
            plugin.Lua.Dispose();
        }
        catch
        {
            // Plugin teardown is best effort.
        }

        _plugins.Remove(plugin);
        entry.Plugin = null;
    }

    private void RegisterModules(LoadedPlugin plugin)
    {
        Lua lua = plugin.Lua;
        LuaTable root = plugin.Hexal;
        LuaTable internalApi = NewTable(lua);
        lua["__hexal_host_api"] = internalApi;

        LuaTable editor = NewModule(lua, root, "editor");
        editor["get_current_buffer"] = new Func<string>(() => _host.CurrentBufferId);
        editor["get_workspace_root"] = new Func<string?>(() => _host.WorkspaceRoot);
        editor["get_text"] = new Func<string>(_host.GetText);
        editor["get_line"] = new Func<long, object?>(line =>
        {
            string? text = _host.GetLine(line, out string error);
            if (error.Length > 0)
            {
                _host.ShowMessage(error, true);
                return null;
            }
            return text;
        });
        editor["get_cursor"] = new Func<LuaTable>(() =>
        {
            PluginCursor cursor = _host.GetCursor();
            LuaTable table = NewTable(lua);
            table["line"] = cursor.Line;
            table["column"] = cursor.Column;
            return table;
        });
        editor["set_cursor"] = new Func<long, long, bool>(
            (line, column) =>
            {
                if (_host.SetCursor(line, column, out string error))
                {
                    return true;
                }
                _host.ShowMessage(error, true);
                return false;
            }
        );
        editor["get_selection"] = new Func<object?>(() =>
        {
            PluginSelection? selection = _host.GetSelection();
            if (selection is null)
            {
                return null;
            }

            PluginSelection value = selection.Value;
            LuaTable table = NewTable(lua);
            table["start_line"] = value.StartLine;
            table["start_column"] = value.StartColumn;
            table["end_line"] = value.EndLine;
            table["end_column"] = value.EndColumn;
            table["text"] = value.Text;
            table["linewise"] = value.Linewise;
            return table;
        });
        editor["insert_text"] = new Action<string>(text =>
        {
            if (!_host.InsertText(text, out string error))
            {
                _host.ShowMessage(error, true);
            }
        });
        editor["replace_range"] = new Func<object?, object?, string, bool>(
            (start, finish, text) =>
            {
                if (_host.ReplaceRange(start, finish, text, out string error))
                {
                    return true;
                }
                _host.ShowMessage(error, true);
                return false;
            }
        );

        LuaTable commands = NewModule(lua, root, "commands");
        internalApi["commands_register"] = new Func<string, LuaFunction, string, bool>(
            (name, callback, description) =>
            {
                if (string.IsNullOrWhiteSpace(name) || callback is null)
                {
                    _host.ShowMessage("commands.register requires a name and callback", true);
                    return false;
                }

                _commands[name] = new PluginCommand
                {
                    Name = name,
                    Description = description ?? string.Empty,
                    PluginId = plugin.Id,
                    Callback = callback,
                };
                return true;
            }
        );
        internalApi["commands_unregister"] = new Func<string, bool>(name =>
        {
            if (
                _commands.TryGetValue(name, out PluginCommand? command)
                && string.Equals(command.PluginId, plugin.Id, StringComparison.Ordinal)
            )
            {
                _commands.Remove(name);
                return true;
            }
            return false;
        });
        internalApi["commands_execute"] = new Func<string, LuaTable, bool>(
            (name, arguments) =>
            {
                List<string> values = LuaArguments(arguments);
                if (TryExecuteCommand(name, values, out string error))
                {
                    if (error.Length > 0)
                    {
                        _host.ShowMessage(error, true);
                    }
                    return error.Length == 0;
                }

                // Plugins may also invoke built-in Ex commands, for example
                // hexal.commands.execute("e", { path }).
                if (_host.ExecuteCommand(name, values, out error))
                {
                    if (error.Length > 0)
                    {
                        _host.ShowMessage(error, true);
                    }
                    return error.Length == 0;
                }

                _host.ShowMessage($"unknown command '{name}'", true);
                return false;
            }
        );

        LuaTable keymap = NewModule(lua, root, "keymap");
        internalApi["keymap_map"] = new Func<string, string, string, bool>(
            (keys, command, mode) => AddBinding(plugin, keys, command, null, mode)
        );
        internalApi["keymap_map_callback"] = new Func<string, LuaFunction, string, bool>(
            (keys, callback, mode) => AddBinding(plugin, keys, null, callback, mode)
        );
        internalApi["keymap_unmap"] = new Func<string, string, bool>(
            (keys, mode) =>
            {
                string[] normalized = NormalizeKeySequence(keys);
                if (normalized.Length == 0)
                {
                    return false;
                }

                return _keyBindings.RemoveAll(binding =>
                        string.Equals(binding.PluginId, plugin.Id, StringComparison.Ordinal)
                        && string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase)
                        && binding.Keys.SequenceEqual(normalized)
                    ) > 0;
            }
        );

        LuaTable ui = NewModule(lua, root, "ui");
        ui["message"] = new Action<string>(text => _host.ShowMessage(text, false));
        ui["warn"] = new Action<string>(text => _host.ShowMessage(text, true));
        ui["copy"] = new Func<string, bool>(text =>
        {
            if (_host.CopyToClipboard(text ?? string.Empty, out string error))
            {
                return true;
            }

            if (error.Length > 0)
            {
                _host.ShowMessage(error, true);
            }
            return false;
        });
        ui["terminal_size"] = new Func<LuaTable>(() =>
        {
            LuaTable table = NewTable(lua);
            table["columns"] = _host.TerminalColumns;
            table["rows"] = _host.TerminalRows;
            return table;
        });
        ui["prompt"] = new Action<string, LuaFunction>(
            (label, callback) =>
            {
                _host.RequestPrompt(
                    label,
                    value =>
                    {
                        if (value is null)
                        {
                            InvokeSafely(plugin, callback);
                        }
                        else
                        {
                            InvokeSafely(plugin, callback, value);
                        }
                    }
                );
            }
        );
        ui["select"] = new Action<string, LuaTable, LuaFunction>(
            (title, items, callback) =>
            {
                var values = new List<string>();
                foreach (object? item in LuaArray(items))
                {
                    values.Add(item?.ToString() ?? string.Empty);
                }

                if (values.Count == 0)
                {
                    InvokeSafely(plugin, callback, new object?[] { null });
                    return;
                }

                _host.RequestSelect(
                    title,
                    values,
                    index =>
                    {
                        if (index is null)
                        {
                            InvokeSafely(plugin, callback, new object?[] { null });
                        }
                        else
                        {
                            // Lua convention: selection indexes are one-based.
                            InvokeSafely(plugin, callback, index.Value + 1);
                        }
                    }
                );
            }
        );
        ui["set_status"] = new Func<string, object?, bool>(
            (key, text) =>
            {
                if (string.IsNullOrEmpty(key))
                {
                    return false;
                }
                _host.SetPluginStatus(key, text?.ToString());
                return true;
            }
        );
        ui["panel"] = new Func<LuaTable, LuaFunction?, LuaTable>(
            (options, onKey) =>
            {
                PluginPanelOptions parsed = ParsePanelOptions(options);
                int panelId = _host.OpenPluginPanel(
                    parsed,
                    (key, selected) =>
                    {
                        if (onKey is null)
                        {
                            return false;
                        }

                        try
                        {
                            object[] result = onKey.Call(key, selected + 1);
                            return result.Length > 0 && result[0] is bool handled && handled;
                        }
                        catch (Exception ex)
                        {
                            _host.ReportPluginError(
                                plugin.Id,
                                $"panel key handler failed: {ex.Message}"
                            );
                            return false;
                        }
                    },
                    plugin.Id
                );

                LuaTable panel = NewTable(lua);
                panel["id"] = panelId;
                panel["set_lines"] = new Action<object?, LuaTable>(
                    (_, lines) => _host.SetPluginPanelLines(panelId, ParsePanelLines(lines))
                );
                panel["set_title"] = new Action<object?, string>(
                    (_, title) => _host.SetPluginPanelTitle(panelId, title ?? string.Empty)
                );
                panel["set_footer"] = new Action<object?, string>(
                    (_, footer) => _host.SetPluginPanelFooter(panelId, footer ?? string.Empty)
                );
                panel["set_selected"] = new Action<object?, object?>(
                    (_, selected) => _host.SetPluginPanelSelected(panelId, ToLong(selected, 1) - 1)
                );
                panel["set_selectable"] = new Action<object?, object?>(
                    (_, selectable) =>
                        _host.SetPluginPanelSelectable(
                            panelId,
                            selectable is bool flag ? flag : true
                        )
                );
                panel["close"] = new Action<object?>(_ => _host.ClosePluginPanel(panelId));
                return panel;
            }
        );

        LuaTable process = NewModule(lua, root, "exec");
        internalApi["process_run"] = new Func<string, LuaTable?, LuaTable?, LuaTable>(
            (command, args, options) => RunProcess(plugin, command, args, options)
        );

        LuaTable events = NewModule(lua, root, "events");
        events["on"] = new Func<string, LuaFunction, long>(
            (eventName, callback) =>
            {
                if (string.IsNullOrWhiteSpace(eventName) || callback is null)
                {
                    return 0;
                }

                long id = _nextSubscriptionId++;
                _subscriptions[id] = new EventSubscription
                {
                    Id = id,
                    Event = eventName,
                    PluginId = plugin.Id,
                    Callback = callback,
                };
                return id;
            }
        );
        events["off"] = new Func<long, bool>(id =>
        {
            if (
                _subscriptions.TryGetValue(id, out EventSubscription? subscription)
                && string.Equals(subscription.PluginId, plugin.Id, StringComparison.Ordinal)
            )
            {
                _subscriptions.Remove(id);
                return true;
            }
            return false;
        });

        LuaTable lsp = NewModule(lua, root, "lsp");
        lsp["start"] = new Func<object?, bool>(configValue =>
        {
            if (configValue is not LuaTable config)
            {
                _host.ShowMessage("lsp.start expects a server configuration table", true);
                return false;
            }

            string? command = config["command"]?.ToString();
            if (string.IsNullOrWhiteSpace(command))
            {
                _host.ShowMessage("lsp.start requires a command", true);
                return false;
            }

            string name = config["name"]?.ToString() ?? command;
            string languageId = config["language_id"]?.ToString() ?? string.Empty;
            var arguments = new List<string>();
            if (config["args"] is LuaTable args)
            {
                foreach (object? argument in LuaArray(args))
                {
                    arguments.Add(argument?.ToString() ?? string.Empty);
                }
            }

            var serverConfig = new LspServerConfig
            {
                Name = name,
                Command = command,
                Arguments = arguments,
                LanguageId = languageId,
            };

            if (_host.StartLsp(serverConfig, out string error))
            {
                return true;
            }

            HexalPaths.Log($"LSP unavailable for plugin {plugin.Id}: {error}");
            return false;
        });
        lsp["stop"] = new Func<object?, bool>(name =>
        {
            _host.StopLsp(name?.ToString());
            return true;
        });
        lsp["request"] = new Func<string, object?, LuaFunction?, bool>(
            (method, parameters, callback) =>
            {
                const string message = "custom LSP requests are not supported by this host yet";
                _host.ShowMessage(message, true);
                if (callback is not null)
                {
                    InvokeSafely(plugin, callback, new object?[] { null, message });
                }
                return false;
            }
        );
        lsp["notify"] = new Func<string, object?, bool>(
            (method, parameters) =>
            {
                _host.ShowMessage(
                    "custom LSP notifications are not supported by this host yet",
                    true
                );
                return false;
            }
        );
        lsp["on_diagnostics"] = new Func<LuaFunction, long>(callback =>
        {
            if (callback is null)
            {
                return 0;
            }
            long id = _nextSubscriptionId++;
            _subscriptions[id] = new EventSubscription
            {
                Id = id,
                Event = "__lsp_diagnostics",
                PluginId = plugin.Id,
                Callback = callback,
            };
            return id;
        });
        lsp["go_to_definition"] = new Action(() =>
            _host.ShowMessage("go-to-definition is not implemented yet", true)
        );
        lsp["find_references"] = new Action(() =>
            _host.ShowMessage("find-references is not implemented yet", true)
        );

        LuaTable config = NewModule(lua, root, "config");
        internalApi["config_get"] = new Func<string, object?, object?>(
            (key, fallback) =>
            {
                object? value = plugin.Config.Get(key, fallback);
                return ToLua(plugin, value);
            }
        );
        internalApi["config_set"] = new Func<string, object?, bool>(
            (key, value) =>
            {
                if (string.IsNullOrEmpty(key))
                {
                    return false;
                }
                plugin.Config.Set(key, FromLua(value));
                return true;
            }
        );
        internalApi["config_save"] = new Func<bool>(() =>
        {
            if (plugin.Config.Save(out string error))
            {
                return true;
            }
            _host.ShowMessage($"config save failed: {error}", true);
            return false;
        });
        internalApi["config_get_all"] = new Func<LuaTable>(() =>
            ToLua(plugin, plugin.Config.Snapshot) as LuaTable ?? NewTable(lua)
        );

        _ = commands;
        _ = keymap;
        _ = config;

        internalApi["lsp_ensure"] = new Func<bool>(() => EnsureLsp(plugin) is not null);
        ConfigureLazyLspModules(plugin);

        lua.DoString(Prelude);
    }

    private LuaLspCoordinator? EnsureLsp(LoadedPlugin plugin)
    {
        if (plugin.Lsp is not null)
        {
            return plugin.Lsp;
        }
        if (plugin.LspBinding)
        {
            return null;
        }

        plugin.LspBinding = true;
        try
        {
            var coordinator = new LuaLspCoordinator(plugin.Lua, plugin.Id, plugin.Directory)
            {
                ReportError = message => _host.ReportPluginError(plugin.Id, message),
                Host = _host as ILuaLspHost,
            };
            plugin.Lsp = coordinator;
            LuaLspModuleBinder.Bind(plugin.Lua, plugin.Hexal, coordinator);
            return coordinator;
        }
        catch (Exception ex)
        {
            plugin.Lsp = null;
            _host.ReportPluginError(plugin.Id, $"LSP module initialization failed: {ex.Message}");
            return null;
        }
        finally
        {
            plugin.LspBinding = false;
        }
    }

    private static void ConfigureLazyLspModules(LoadedPlugin plugin)
    {
        Lua lua = plugin.Lua;
        LuaTable root = plugin.Hexal;

        foreach (
            string name in new[]
            {
                "async",
                "json",
                "fs",
                "log",
                "index",
                "process",
                "debug",
                "testing",
            }
        )
        {
            if (root[name] is not LuaTable)
            {
                LuaTable module = NewTable(lua);
                root[name] = module;
            }
        }

        LuaTable meta = (LuaTable)
            lua.DoString(
                "return { __index = function(t, k) "
                    + "if __hexal_host_api.lsp_ensure() then return rawget(t, k) end end }"
            )[0]!;

        LuaFunction setMetatable = (LuaFunction)lua["setmetatable"];
        var lazyTables = new List<LuaTable>();
        if (root["lsp"] is LuaTable lsp)
        {
            lazyTables.Add(lsp);
        }
        foreach (
            string name in new[]
            {
                "async",
                "json",
                "fs",
                "log",
                "index",
                "process",
                "debug",
                "testing",
            }
        )
        {
            if (root[name] is LuaTable module)
            {
                lazyTables.Add(module);
            }
        }

        foreach (LuaTable table in lazyTables)
        {
            setMetatable.Call(table, meta);
        }
    }

    private bool AddBinding(
        LoadedPlugin plugin,
        string keys,
        string? command,
        LuaFunction? callback,
        string mode
    )
    {
        string[] normalized = NormalizeKeySequence(keys);
        if (normalized.Length == 0 || (command is null && callback is null))
        {
            return false;
        }

        mode = string.IsNullOrWhiteSpace(mode) ? "normal" : mode.Trim().ToLowerInvariant();

        // User/plugin bindings override earlier bindings with the same keys.
        _keyBindings.RemoveAll(binding =>
            string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase)
            && binding.Keys.SequenceEqual(normalized)
        );

        _keyBindings.Add(
            new PluginKeyBinding
            {
                PluginId = plugin.Id,
                Mode = mode,
                Keys = normalized,
                Command = command,
                Callback = callback,
            }
        );
        return true;
    }

    private void CleanupPlugin(string pluginId)
    {
        _host.ClosePluginPanels(pluginId);

        foreach (
            string name in _commands
                .Where(pair =>
                    string.Equals(pair.Value.PluginId, pluginId, StringComparison.Ordinal)
                )
                .Select(pair => pair.Key)
                .ToArray()
        )
        {
            _commands.Remove(name);
        }

        _keyBindings.RemoveAll(binding =>
            string.Equals(binding.PluginId, pluginId, StringComparison.Ordinal)
        );

        foreach (
            long id in _subscriptions
                .Where(pair =>
                    string.Equals(pair.Value.PluginId, pluginId, StringComparison.Ordinal)
                )
                .Select(pair => pair.Key)
                .ToArray()
        )
        {
            _subscriptions.Remove(id);
        }
    }

    private LoadedPlugin? FindPlugin(string id)
    {
        foreach (LoadedPlugin plugin in _plugins)
        {
            if (string.Equals(plugin.Id, id, StringComparison.Ordinal))
            {
                return plugin;
            }
        }
        return null;
    }

    private PluginEntry? FindEntry(string id)
    {
        foreach (PluginEntry entry in _entries)
        {
            if (string.Equals(entry.Id, id, StringComparison.Ordinal))
            {
                return entry;
            }
        }
        return null;
    }

    private HashSet<string> LoadDisabledPlugins()
    {
        _disabledPlugins.Clear();
        if (_statePath.Length == 0 || !File.Exists(_statePath))
        {
            return _disabledPlugins;
        }

        try
        {
            foreach (string line in File.ReadLines(_statePath))
            {
                string id = line.Trim();
                if (id.Length > 0)
                {
                    _disabledPlugins.Add(id);
                }
            }
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            _errors.Add($"plugin state: {ex.Message}");
        }

        return _disabledPlugins;
    }

    private void SaveState()
    {
        if (_statePath.Length == 0)
        {
            return;
        }

        try
        {
            string? directory = Path.GetDirectoryName(_statePath);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            string[] ids = _disabledPlugins
                .OrderBy(id => id, StringComparer.OrdinalIgnoreCase)
                .ToArray();
            File.WriteAllLines(_statePath, ids);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            _errors.Add($"plugin state: {ex.Message}");
        }
    }

    private static bool TryParseManifest(
        string path,
        Dictionary<string, string> values,
        out string error
    )
    {
        error = string.Empty;

        try
        {
            foreach (string rawLine in File.ReadLines(path))
            {
                string line = rawLine.Trim();
                if (line.Length == 0 || line.StartsWith('#') || line.StartsWith('['))
                {
                    continue;
                }

                int separator = line.IndexOf('=');
                if (separator <= 0)
                {
                    continue;
                }

                string key = line[..separator].Trim();
                string value = line[(separator + 1)..].Trim();

                if (
                    value.Length >= 2
                    && (
                        (value[0] == '"' && value[^1] == '"')
                        || (value[0] == '\'' && value[^1] == '\'')
                    )
                )
                {
                    value = value[1..^1];
                }

                if (key.Length > 0)
                {
                    values[key] = value;
                }
            }
            return true;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            error = ex.Message;
            return false;
        }
    }

    private static string DefaultPluginConfigDirectory =>
        Path.Combine(HexalPaths.HexalHomeDirectory, "plugin-config");

    private static string SanitizeFileName(string id)
    {
        var builder = new StringBuilder(id.Length);
        foreach (char character in id)
        {
            builder.Append(
                character is '/' or '\\' or ':' or '*' or '?' or '"' or '<' or '>' or '|'
                    ? '_'
                    : character
            );
        }
        return builder.Length == 0 ? "plugin" : builder.ToString();
    }

    private static PluginPanelOptions ParsePanelOptions(LuaTable options)
    {
        string title = options["title"]?.ToString() ?? "Plugin";
        string footer = options["footer"]?.ToString() ?? string.Empty;
        bool selectable = options["selectable"] is bool flag ? flag : true;
        long selected = ToLong(options["selected"], 1) - 1;
        IReadOnlyList<PluginPanelLine> lines = options["lines"] is LuaTable table
            ? ParsePanelLines(table)
            : Array.Empty<PluginPanelLine>();

        return new PluginPanelOptions
        {
            Title = title,
            Footer = footer,
            Selectable = selectable,
            Selected = Math.Max(0, selected),
            Lines = lines,
        };
    }

    private static List<PluginPanelLine> ParsePanelLines(LuaTable lines)
    {
        var result = new List<PluginPanelLine>();
        foreach (object? item in LuaArray(lines))
        {
            result.Add(ParsePanelLine(item));
        }
        return result;
    }

    private static PluginPanelLine ParsePanelLine(object? item)
    {
        if (item is string text)
        {
            return new PluginPanelLine
            {
                Segments = new[] { new PluginPanelSegment(text, "text") },
            };
        }

        if (item is not LuaTable table)
        {
            return new PluginPanelLine
            {
                Segments = new[]
                {
                    new PluginPanelSegment(item?.ToString() ?? string.Empty, "text"),
                },
            };
        }

        bool selectable = table["selectable"] is bool flag ? flag : true;
        var segments = new List<PluginPanelSegment>();
        if (table["segments"] is LuaTable segmentTable)
        {
            AppendPanelSegments(segments, segmentTable);
        }
        else if (table["text"] is not null)
        {
            AppendPanelSegment(segments, table);
        }
        else
        {
            AppendPanelSegments(segments, table);
        }

        return new PluginPanelLine { Segments = segments, Selectable = selectable };
    }

    private static void AppendPanelSegments(List<PluginPanelSegment> segments, LuaTable table)
    {
        foreach (object? item in LuaArray(table))
        {
            AppendPanelSegment(segments, item);
        }
    }

    private static void AppendPanelSegment(List<PluginPanelSegment> segments, object? item)
    {
        string text;
        string style = "text";
        if (item is LuaTable table)
        {
            text = table["text"]?.ToString() ?? string.Empty;
            style = table["style"]?.ToString() ?? "text";
        }
        else
        {
            text = item?.ToString() ?? string.Empty;
        }

        if (text.Length > 0)
        {
            segments.Add(new PluginPanelSegment(text, style));
        }
    }

    private LuaTable RunProcess(
        LoadedPlugin plugin,
        string command,
        LuaTable? args,
        LuaTable? options
    )
    {
        LuaTable result = NewTable(plugin.Lua);
        result["ok"] = false;
        result["code"] = 0L;
        result["stdout"] = string.Empty;
        result["stderr"] = string.Empty;
        result["error"] = string.Empty;

        if (string.IsNullOrWhiteSpace(command))
        {
            result["error"] = "empty command";
            return result;
        }

        var startInfo = new ProcessStartInfo
        {
            FileName = command,
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
        };

        if (args is not null)
        {
            foreach (object? argument in LuaArray(args))
            {
                startInfo.ArgumentList.Add(argument?.ToString() ?? string.Empty);
            }
        }

        string? directory = options?["cwd"]?.ToString();
        if (string.IsNullOrWhiteSpace(directory))
        {
            directory = _host.WorkspaceRoot;
        }

        if (!string.IsNullOrWhiteSpace(directory))
        {
            if (!Directory.Exists(directory))
            {
                result["error"] = $"working directory not found: {directory}";
                return result;
            }
            startInfo.WorkingDirectory = directory;
        }

        int timeoutMs = (int)Math.Clamp(ToLong(options?["timeout"], 15000), 100, 120_000);

        try
        {
            using var process = new Process { StartInfo = startInfo };
            if (!process.Start())
            {
                result["error"] = $"cannot start '{command}'";
                return result;
            }

            Task<string> stdoutTask = process.StandardOutput.ReadToEndAsync();
            Task<string> stderrTask = process.StandardError.ReadToEndAsync();

            using var cancellation = new CancellationTokenSource(timeoutMs);
            try
            {
                process.WaitForExitAsync(cancellation.Token).GetAwaiter().GetResult();
            }
            catch (OperationCanceledException)
            {
                try
                {
                    if (!process.HasExited)
                    {
                        process.Kill(entireProcessTree: true);
                    }
                }
                catch
                {
                    // Process teardown is best effort.
                }

                result["error"] = $"process timed out after {timeoutMs} ms";
                return result;
            }

            string stdout = stdoutTask.GetAwaiter().GetResult();
            string stderr = stderrTask.GetAwaiter().GetResult();
            result["ok"] = process.ExitCode == 0;
            result["code"] = (long)process.ExitCode;
            result["stdout"] = stdout;
            result["stderr"] = stderr;
            if (process.ExitCode != 0)
            {
                string detail = stderr.Trim();
                result["error"] =
                    detail.Length > 0 ? detail : $"process exited with code {process.ExitCode}";
            }
            return result;
        }
        catch (Exception ex)
            when (ex
                    is System.ComponentModel.Win32Exception
                        or InvalidOperationException
                        or IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            result["error"] = ex.Message;
            return result;
        }
    }

    private static long ToLong(object? value, long fallback = 0)
    {
        if (value is null)
        {
            return fallback;
        }

        try
        {
            return Convert.ToInt64(value, CultureInfo.InvariantCulture);
        }
        catch (Exception ex)
            when (ex is FormatException or InvalidCastException or OverflowException)
        {
            return fallback;
        }
    }

    private static LuaTable NewTable(Lua lua)
    {
        return (LuaTable)lua.DoString("return {}")[0]!;
    }

    private static LuaTable NewModule(Lua lua, LuaTable root, string name)
    {
        LuaTable module = NewTable(lua);
        root[name] = module;
        return module;
    }

    private void InvokeSafely(LoadedPlugin plugin, LuaFunction callback, params object?[] arguments)
    {
        try
        {
            object[] values = new object[arguments.Length];
            for (int i = 0; i < arguments.Length; ++i)
            {
                values[i] = arguments[i]!;
            }
            callback.Call(values);
        }
        catch (Exception ex)
        {
            _host.ReportPluginError(plugin.Id, $"callback failed: {ex.Message}");
        }
    }

    private static List<string> LuaArguments(LuaTable table)
    {
        var result = new List<string>();
        foreach (object? value in LuaArray(table))
        {
            result.Add(value?.ToString() ?? string.Empty);
        }
        return result;
    }

    private static List<object?> LuaArray(LuaTable table)
    {
        var result = new List<object?>();
        int index = 1;
        while (true)
        {
            object? value = table[index];
            if (value is null)
            {
                break;
            }
            result.Add(value);
            ++index;
        }
        return result;
    }

    private static object? FromLua(object? value)
    {
        return FromLua(value, 0);
    }

    private static object? FromLua(object? value, int depth)
    {
        if (depth > 32)
        {
            return null;
        }

        switch (value)
        {
            case null:
                return null;
            case string text:
                return text;
            case bool flag:
                return flag;
            case byte or sbyte or short or ushort or int or uint or long or ulong:
                return Convert.ToInt64(value, CultureInfo.InvariantCulture);
            case float or double or decimal:
                return Convert.ToDouble(value, CultureInfo.InvariantCulture);
            case LuaTable table:
            {
                var keys = new List<object?>();
                foreach (object? key in table.Keys)
                {
                    keys.Add(key);
                }

                bool isArray = keys.Count > 0;
                int expected = 1;
                foreach (object? key in keys)
                {
                    if (!TryNumericKey(key, out double numeric) || numeric != expected)
                    {
                        isArray = false;
                        break;
                    }
                    ++expected;
                }

                if (isArray)
                {
                    var list = new List<object?>();
                    for (int i = 1; i <= keys.Count; ++i)
                    {
                        list.Add(FromLua(table[i], depth + 1));
                    }
                    return list;
                }

                var map = new Dictionary<string, object?>(StringComparer.Ordinal);
                foreach (object? key in keys)
                {
                    if (key is null)
                    {
                        continue;
                    }
                    map[key.ToString() ?? string.Empty] = FromLua(table[key], depth + 1);
                }
                return map;
            }
            default:
                return value.ToString();
        }
    }

    private static bool TryNumericKey(object? key, out double value)
    {
        switch (key)
        {
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
                value = Convert.ToDouble(key, CultureInfo.InvariantCulture);
                return true;
            default:
                value = 0;
                return false;
        }
    }

    private static object? ToLua(LoadedPlugin plugin, object? value)
    {
        return ToLua(plugin, value, 0);
    }

    private static object? ToLua(LoadedPlugin plugin, object? value, int depth)
    {
        if (depth > 32)
        {
            return null;
        }

        switch (value)
        {
            case null:
                return null;
            case LuaTable table:
                return table;
            case string or bool:
                return value;
            case byte or sbyte or short or ushort or int or uint or long or ulong:
                return Convert.ToDouble(value, CultureInfo.InvariantCulture);
            case float or double or decimal:
                return Convert.ToDouble(value, CultureInfo.InvariantCulture);
            case IReadOnlyDictionary<string, object?> readOnlyMap:
            {
                LuaTable table = NewTable(plugin.Lua);
                foreach ((string key, object? item) in readOnlyMap)
                {
                    table[key] = ToLua(plugin, item, depth + 1);
                }
                return table;
            }
            case IDictionary<string, object?> map:
            {
                LuaTable table = NewTable(plugin.Lua);
                foreach ((string key, object? item) in map)
                {
                    table[key] = ToLua(plugin, item, depth + 1);
                }
                return table;
            }
            case System.Collections.IEnumerable sequence when value is not string:
            {
                LuaTable table = NewTable(plugin.Lua);
                int index = 1;
                foreach (object? item in sequence)
                {
                    table[index++] = ToLua(plugin, item, depth + 1);
                }
                return table;
            }
            default:
                return value.ToString();
        }
    }

    private static string KeyToken(InputEvent inputEvent)
    {
        if (inputEvent.Key == Key.Character)
        {
            string character = inputEvent.Character.ToString();
            if (character.Length == 0)
            {
                return string.Empty;
            }

            if (character == " ")
            {
                character = "space";
            }

            if (inputEvent.Ctrl)
            {
                return "ctrl+" + character.ToLowerInvariant();
            }
            if (inputEvent.Alt)
            {
                return "alt+" + character.ToLowerInvariant();
            }
            return character;
        }

        return inputEvent.Key switch
        {
            Key.Escape => "esc",
            Key.Enter => "enter",
            Key.Backspace => "backspace",
            Key.Delete => "delete",
            Key.Tab => "tab",
            Key.BackTab => "shift+tab",
            Key.Up => "up",
            Key.Down => "down",
            Key.Left => "left",
            Key.Right => "right",
            Key.Home => "home",
            Key.End => "end",
            Key.PageUp => "pageup",
            Key.PageDown => "pagedown",
            Key.Insert => "insert",
            Key.F1 => "f1",
            Key.F2 => "f2",
            Key.F3 => "f3",
            Key.F4 => "f4",
            Key.F5 => "f5",
            Key.F6 => "f6",
            Key.F7 => "f7",
            Key.F8 => "f8",
            Key.F9 => "f9",
            Key.F10 => "f10",
            Key.F11 => "f11",
            Key.F12 => "f12",
            _ => string.Empty,
        };
    }

    private static string[] NormalizeKeySequence(string keys)
    {
        if (string.IsNullOrWhiteSpace(keys))
        {
            return Array.Empty<string>();
        }

        string[] parts = keys.Split(
            (char[]?)null,
            StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries
        );
        var result = new string[parts.Length];

        for (int i = 0; i < parts.Length; ++i)
        {
            string part = parts[i];
            if (part.Length > 1 && !part.Contains('+'))
            {
                part = part.ToLowerInvariant() switch
                {
                    "escape" => "esc",
                    "space" => "space",
                    "return" => "enter",
                    "backspace" => "backspace",
                    "delete" => "delete",
                    "tab" => "tab",
                    "up" => "up",
                    "down" => "down",
                    "left" => "left",
                    "right" => "right",
                    "home" => "home",
                    "end" => "end",
                    "pageup" => "pageup",
                    "pagedown" => "pagedown",
                    "insert" => "insert",
                    _ => part,
                };
            }
            else if (part.StartsWith("ctrl+", StringComparison.OrdinalIgnoreCase))
            {
                part = "ctrl+" + part[5..].ToLowerInvariant();
            }
            else if (part.StartsWith("alt+", StringComparison.OrdinalIgnoreCase))
            {
                part = "alt+" + part[4..].ToLowerInvariant();
            }

            result[i] = part;
        }

        return result;
    }

    private const string Prelude = """
        (function()
            local H = __hexal_host_api

            local searchers = package.searchers or package.loaders
            if searchers then
                local additional = {
                    "hexal.async", "hexal.json", "hexal.fs", "hexal.log",
                    "hexal.index", "hexal.process", "hexal.debug",
                    "hexal.testing",
                }
                table.insert(searchers, 2, function(name)
                    local wanted = name == "hexal" or
                        name == "hexal.lsp" or
                        name:match("^hexal%.lsp%.") or
                        name:match("^hexal%.testing%.")
                    if not wanted then
                        for _, module_name in ipairs(additional) do
                            if name == module_name then
                                wanted = true
                                break
                            end
                        end
                    end
                    if wanted then
                        H.lsp_ensure()
                        local loaded = package.loaded[name]
                        if loaded ~= nil then
                            return function() return loaded end
                        end
                        local loader = package.preload[name]
                        if type(loader) == "function" then
                            return loader
                        end
                    end
                    return "\n\tno lazy hexal module '" .. name .. "'"
                end)
            end

            local C = hexal.commands
            C.register = function(name, callback, description)
                return H.commands_register(name, callback, description or "")
            end
            C.unregister = function(name)
                return H.commands_unregister(name)
            end
            C.execute = function(name, ...)
                return H.commands_execute(name, { ... })
            end

            local K = hexal.keymap
            K.map = function(keys, command, mode)
                return H.keymap_map(keys, command, mode or "normal")
            end
            K.map_callback = function(keys, callback, mode)
                return H.keymap_map_callback(keys, callback, mode or "normal")
            end
            K.unmap = function(keys, mode)
                return H.keymap_unmap(keys, mode or "normal")
            end

            local P = hexal.exec
            P.run = function(command, args, options)
                return H.process_run(command, args or {}, options or {})
            end

            local CFG = hexal.config
            CFG.get = function(key, default_value)
                return H.config_get(key, default_value)
            end
            CFG.set = function(key, value)
                return H.config_set(key, value)
            end
            CFG.save = function()
                return H.config_save()
            end
            CFG.get_all = function()
                return H.config_get_all()
            end
        end)()
        """;
}
