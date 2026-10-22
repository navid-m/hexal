using System.Globalization;
using System.Text;
using Hexal.Lsp;
using Hexal.Platform;
using Hexal.Util;
using NLua;
using NLua.Exceptions;

namespace Hexal.Scripting;

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
        public bool Running;
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

    private readonly IEditorApi _host;
    private readonly string _pluginsDirectory;
    private readonly List<LoadedPlugin> _plugins = new();
    private readonly Dictionary<string, PluginCommand> _commands =
        new(StringComparer.OrdinalIgnoreCase);
    private readonly List<PluginKeyBinding> _keyBindings = new();
    private readonly Dictionary<long, EventSubscription> _subscriptions = new();
    private readonly List<string> _errors = new();
    private readonly List<string> _pendingKeys = new();
    private long _nextSubscriptionId = 1;
    private string _pendingMode = string.Empty;
    private long _pendingSince;
    private bool _inEvent;
    private bool _disposed;

    public LuaPluginManager(IEditorApi host)
        : this(host, DefaultPluginDirectory)
    {
    }

    public LuaPluginManager(IEditorApi host, string pluginDirectory)
    {
        _host = host;
        _pluginsDirectory = pluginDirectory;
    }

    public IReadOnlyList<string> Errors => _errors;

    public int PluginCount => _plugins.Count;

    public IReadOnlyList<LuaLspCoordinator> Coordinators =>
        _plugins.Where(plugin => plugin.Lsp is not null)
            .Select(plugin => plugin.Lsp!)
            .ToArray();

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
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            _errors.Add($"cannot create plugin directory '{_pluginsDirectory}': {ex.Message}");
            return;
        }

        string[] directories;
        try
        {
            directories = Directory.GetDirectories(_pluginsDirectory);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            _errors.Add($"cannot read plugin directory '{_pluginsDirectory}': {ex.Message}");
            return;
        }

        Array.Sort(directories, StringComparer.OrdinalIgnoreCase);
        foreach (string directory in directories)
        {
            LoadPlugin(directory);
        }

        HexalPaths.Log(
            $"Plugins: {_pluginsDirectory} ({_plugins.Count} loaded, {_errors.Count} error(s))");
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

    public bool TryExecuteCommand(string name, IReadOnlyList<string> args,
        out string error)
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
            if (string.Equals(subscription.Event, eventName,
                    StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }
        return false;
    }

    public void FireEvent(string eventName,
        IReadOnlyDictionary<string, object?>? data = null)
    {
        if (_inEvent || _disposed)
        {
            return;
        }

        List<EventSubscription>? matching = null;
        foreach (EventSubscription subscription in _subscriptions.Values)
        {
            if (string.Equals(subscription.Event, eventName,
                    StringComparison.OrdinalIgnoreCase))
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
                    _host.ReportPluginError(subscription.PluginId,
                        $"event '{eventName}' failed: {ex.Message}");
                }
            }
        }
        finally
        {
            _inEvent = false;
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

        if (_pendingKeys.Count > 0 &&
            (Environment.TickCount64 - _pendingSince > PendingKeyTimeoutMs ||
             !string.Equals(_pendingMode, mode, StringComparison.OrdinalIgnoreCase)))
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
            if (!string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase) ||
                binding.Keys.Length <= index ||
                !string.Equals(binding.Keys[index], token, StringComparison.Ordinal))
            {
                continue;
            }

            bool matchesPrefix = true;
            for (int i = 0; i < index; ++i)
            {
                if (!string.Equals(binding.Keys[i], _pendingKeys[i],
                        StringComparison.Ordinal))
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
                if (!TryExecuteCommand(binding.Command, Array.Empty<string>(),
                        out string error) && error.Length == 0)
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
                    binding.Callback.Call();
                }
                catch (Exception ex)
                {
                    _host.ReportPluginError(binding.PluginId,
                        $"keymap '{string.Join(' ', keys)}' failed: {ex.Message}");
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
        _commands.Clear();
        _keyBindings.Clear();
        _subscriptions.Clear();
        _pendingKeys.Clear();
    }

    private void LoadPlugin(string directory)
    {
        string id = Path.GetFileName(directory);
        string manifestPath = Path.Combine(directory, "plugin.toml");
        Dictionary<string, string> manifest = new(StringComparer.OrdinalIgnoreCase);

        if (File.Exists(manifestPath))
        {
            if (!TryParseManifest(manifestPath, manifest, out string manifestError))
            {
                _errors.Add($"{id}: {manifestError}");
                return;
            }
        }

        if (manifest.TryGetValue("id", out string? manifestId) &&
            !string.IsNullOrWhiteSpace(manifestId))
        {
            id = manifestId.Trim();
        }

        if (id.Length == 0 || id.Contains('/') || id.Contains('\\') ||
            id is "." or "..")
        {
            _errors.Add($"{Path.GetFileName(directory)}: invalid plugin id");
            return;
        }

        if (manifest.TryGetValue("enabled", out string? enabled) &&
            string.Equals(enabled, "false", StringComparison.OrdinalIgnoreCase))
        {
            return;
        }

        string name = manifest.TryGetValue("name", out string? manifestName) &&
                      !string.IsNullOrWhiteSpace(manifestName)
            ? manifestName.Trim()
            : id;

        string entry = manifest.TryGetValue("entry", out string? manifestEntry) &&
                       !string.IsNullOrWhiteSpace(manifestEntry)
            ? manifestEntry.Trim()
            : "init.lua";

        string entryPath = Path.GetFullPath(Path.Combine(directory, entry));
        string fullDirectory = Path.GetFullPath(directory);
        if (!entryPath.StartsWith(fullDirectory + Path.DirectorySeparatorChar,
                StringComparison.Ordinal) &&
            !string.Equals(entryPath, fullDirectory, StringComparison.Ordinal))
        {
            _errors.Add($"{id}: entry point escapes the plugin directory");
            return;
        }

        if (!File.Exists(entryPath))
        {
            _errors.Add($"{id}: entry point '{entry}' not found");
            return;
        }

        var plugin = new LoadedPlugin
        {
            Id = id,
            Name = name,
            Directory = directory,
            EntryPath = entryPath
        };

        string configDirectory = DefaultPluginConfigDirectory;
        string configPath = Path.Combine(configDirectory,
            SanitizeFileName(id) + ".json");
        plugin.Config = new PluginConfigStore(configPath);
        if (!plugin.Config.TryLoad(out string configError))
        {
            _errors.Add($"{id}: config: {configError}");
        }

        try
        {
            plugin.Lua = new Lua();
            plugin.Lua.NewTable("hexal");
            plugin.Hexal = (LuaTable)plugin.Lua["hexal"];
            plugin.Hexal["api_version"] = 1;

            plugin.Lua["__hexal_plugin_dir"] = directory.Replace('\\', '/');
            plugin.Lua.DoString(
                "package.path = __hexal_plugin_dir .. '/?.lua;' .. " +
                "__hexal_plugin_dir .. '/?/init.lua;' .. package.path");

            RegisterModules(plugin);
            plugin.Lua.DoFile(entryPath);
            plugin.Running = true;
            _plugins.Add(plugin);
        }
        catch (Exception ex)
        {
            string message = ex is LuaScriptException luaError
                ? luaError.Message
                : ex.Message;
            _errors.Add($"{id}: {message}");
            CleanupPlugin(id);
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

    private void RegisterModules(LoadedPlugin plugin)
    {
        Lua lua = plugin.Lua;
        LuaTable root = plugin.Hexal;
        LuaTable internalApi = NewTable(lua);
        lua["__hexal_host_api"] = internalApi;

        LuaTable editor = NewModule(lua, root, "editor");
        editor["get_current_buffer"] = new Func<string>(() => _host.CurrentBufferId);
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
        editor["set_cursor"] = new Func<long, long, bool>((line, column) =>
        {
            if (_host.SetCursor(line, column, out string error))
            {
                return true;
            }
            _host.ShowMessage(error, true);
            return false;
        });
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
            });

        LuaTable commands = NewModule(lua, root, "commands");
        internalApi["commands_register"] =
            new Func<string, LuaFunction, string, bool>((name, callback, description) =>
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
                    Callback = callback
                };
                return true;
            });
        internalApi["commands_unregister"] = new Func<string, bool>(name =>
        {
            if (_commands.TryGetValue(name, out PluginCommand? command) &&
                string.Equals(command.PluginId, plugin.Id, StringComparison.Ordinal))
            {
                _commands.Remove(name);
                return true;
            }
            return false;
        });
        internalApi["commands_execute"] =
            new Func<string, LuaTable, bool>((name, arguments) =>
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

                _host.ShowMessage($"unknown plugin command '{name}'", true);
                return false;
            });

        LuaTable keymap = NewModule(lua, root, "keymap");
        internalApi["keymap_map"] =
            new Func<string, string, string, bool>((keys, command, mode) =>
                AddBinding(plugin, keys, command, null, mode));
        internalApi["keymap_map_callback"] =
            new Func<string, LuaFunction, string, bool>((keys, callback, mode) =>
                AddBinding(plugin, keys, null, callback, mode));
        internalApi["keymap_unmap"] =
            new Func<string, string, bool>((keys, mode) =>
            {
                string[] normalized = NormalizeKeySequence(keys);
                if (normalized.Length == 0)
                {
                    return false;
                }

                return _keyBindings.RemoveAll(binding =>
                    string.Equals(binding.PluginId, plugin.Id, StringComparison.Ordinal) &&
                    string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase) &&
                    binding.Keys.SequenceEqual(normalized)) > 0;
            });

        LuaTable ui = NewModule(lua, root, "ui");
        ui["message"] = new Action<string>(text => _host.ShowMessage(text, false));
        ui["warn"] = new Action<string>(text => _host.ShowMessage(text, true));
        ui["prompt"] = new Action<string, LuaFunction>((label, callback) =>
        {
            _host.RequestPrompt(label, value =>
            {
                if (value is null)
                {
                    InvokeSafely(plugin, callback);
                }
                else
                {
                    InvokeSafely(plugin, callback, value);
                }
            });
        });
        ui["select"] = new Action<string, LuaTable, LuaFunction>((title, items, callback) =>
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

            _host.RequestSelect(title, values, index =>
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
            });
        });
        ui["set_status"] = new Func<string, object?, bool>((key, text) =>
        {
            if (string.IsNullOrEmpty(key))
            {
                return false;
            }
            _host.SetPluginStatus(key, text?.ToString());
            return true;
        });

        LuaTable events = NewModule(lua, root, "events");
        events["on"] = new Func<string, LuaFunction, long>((eventName, callback) =>
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
                Callback = callback
            };
            return id;
        });
        events["off"] = new Func<long, bool>(id =>
        {
            if (_subscriptions.TryGetValue(id, out EventSubscription? subscription) &&
                string.Equals(subscription.PluginId, plugin.Id, StringComparison.Ordinal))
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
                LanguageId = languageId
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
            });
        lsp["notify"] = new Func<string, object?, bool>((method, parameters) =>
        {
            _host.ShowMessage("custom LSP notifications are not supported by this host yet", true);
            return false;
        });
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
                Callback = callback
            };
            return id;
        });
        lsp["go_to_definition"] = new Action(() =>
            _host.ShowMessage("go-to-definition is not implemented yet", true));
        lsp["find_references"] = new Action(() =>
            _host.ShowMessage("find-references is not implemented yet", true));

        LuaTable config = NewModule(lua, root, "config");
        internalApi["config_get"] = new Func<string, object?, object?>((key, fallback) =>
        {
            object? value = plugin.Config.Get(key, fallback);
            return ToLua(plugin, value);
        });
        internalApi["config_set"] = new Func<string, object?, bool>((key, value) =>
        {
            if (string.IsNullOrEmpty(key))
            {
                return false;
            }
            plugin.Config.Set(key, FromLua(value));
            return true;
        });
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
            ToLua(plugin, plugin.Config.Snapshot) as LuaTable ?? NewTable(lua));

        _ = commands;
        _ = keymap;
        _ = config;

        lua.DoString(Prelude);

        var coordinator = new LuaLspCoordinator(lua, plugin.Id, plugin.Directory)
        {
            ReportError = message => _host.ReportPluginError(plugin.Id, message),
            Host = _host as ILuaLspHost
        };
        plugin.Lsp = coordinator;
        LuaLspModuleBinder.Bind(lua, plugin.Hexal, coordinator);
    }

    private bool AddBinding(LoadedPlugin plugin, string keys, string? command,
        LuaFunction? callback, string mode)
    {
        string[] normalized = NormalizeKeySequence(keys);
        if (normalized.Length == 0 || (command is null && callback is null))
        {
            return false;
        }

        mode = string.IsNullOrWhiteSpace(mode) ? "normal" : mode.Trim().ToLowerInvariant();

        // User/plugin bindings override earlier bindings with the same keys.
        _keyBindings.RemoveAll(binding =>
            string.Equals(binding.Mode, mode, StringComparison.OrdinalIgnoreCase) &&
            binding.Keys.SequenceEqual(normalized));

        _keyBindings.Add(new PluginKeyBinding
        {
            PluginId = plugin.Id,
            Mode = mode,
            Keys = normalized,
            Command = command,
            Callback = callback
        });
        return true;
    }

    private void CleanupPlugin(string pluginId)
    {
        foreach (string name in _commands
                     .Where(pair => string.Equals(pair.Value.PluginId, pluginId,
                         StringComparison.Ordinal))
                     .Select(pair => pair.Key)
                     .ToArray())
        {
            _commands.Remove(name);
        }

        _keyBindings.RemoveAll(binding =>
            string.Equals(binding.PluginId, pluginId, StringComparison.Ordinal));

        foreach (long id in _subscriptions
                     .Where(pair => string.Equals(pair.Value.PluginId, pluginId,
                         StringComparison.Ordinal))
                     .Select(pair => pair.Key)
                     .ToArray())
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

    private static bool TryParseManifest(string path,
        Dictionary<string, string> values, out string error)
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

                if (value.Length >= 2 &&
                    ((value[0] == '"' && value[^1] == '"') ||
                     (value[0] == '\'' && value[^1] == '\'')))
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
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
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
            builder.Append(character is '/' or '\\' or ':' or '*' or '?' or '"' or '<' or
                '>' or '|' ? '_' : character);
        }
        return builder.Length == 0 ? "plugin" : builder.ToString();
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

    private void InvokeSafely(LoadedPlugin plugin, LuaFunction callback,
        params object?[] arguments)
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
            _host.ReportPluginError(plugin.Id,
                $"callback failed: {ex.Message}");
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
                    map[key.ToString() ?? string.Empty] =
                        FromLua(table[key], depth + 1);
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
            case byte or sbyte or short or ushort or int or uint or long or ulong or
                 float or double or decimal:
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
            _ => string.Empty
        };
    }

    private static string[] NormalizeKeySequence(string keys)
    {
        if (string.IsNullOrWhiteSpace(keys))
        {
            return Array.Empty<string>();
        }

        string[] parts = keys.Split((char[]?)null,
            StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
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
                    _ => part
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
