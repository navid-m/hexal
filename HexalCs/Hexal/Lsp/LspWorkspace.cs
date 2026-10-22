using System.Collections.Concurrent;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Hexal.Util;

namespace Hexal.Lsp;

public sealed class LspPermissionPolicy
{
    public bool ReadFiles { get; set; } = true;

    public bool WriteFiles { get; set; } = true;

    public bool CreateProcesses { get; set; }

    public bool Network { get; set; }

    public bool WorkspaceOnly { get; set; }

    public List<string> AllowedRoots { get; } = new();

    public void AuthorizeRead(string path)
    {
        if (!ReadFiles)
        {
            throw new UnauthorizedAccessException(
                $"filesystem read permission is required for '{path}'");
        }
        AuthorizeRoot(path);
    }

    public void AuthorizeWrite(string path)
    {
        if (!WriteFiles)
        {
            throw new UnauthorizedAccessException(
                $"filesystem write permission is required for '{path}'");
        }
        AuthorizeRoot(path);
    }

    private void AuthorizeRoot(string path)
    {
        if (!WorkspaceOnly || AllowedRoots.Count == 0)
        {
            return;
        }

        foreach (string root in AllowedRoots)
        {
            if (LspUri.IsWithin(path, root))
            {
                return;
            }
        }

        throw new UnauthorizedAccessException(
            $"'{path}' is outside the authorized workspace roots");
    }
}

public sealed class LspTaskResult
{
    public bool IsDone { get; private set; }

    public bool IsCancelled { get; private set; }

    public bool IsFailed { get; private set; }

    public string Error { get; private set; } = string.Empty;

    public object? Result { get; private set; }

    private readonly List<Action<object?>> _callbacks = new();

    public void Complete(object? result)
    {
        if (IsDone)
        {
            return;
        }
        IsDone = true;
        Result = result;
        InvokeCallbacks();
    }

    public void Fail(string error)
    {
        if (IsDone)
        {
            return;
        }
        IsDone = true;
        IsFailed = true;
        Error = error ?? string.Empty;
        InvokeCallbacks();
    }

    public void Cancel()
    {
        if (IsDone)
        {
            return;
        }
        IsDone = true;
        IsCancelled = true;
        InvokeCallbacks();
    }

    public void OnComplete(Action<object?> callback)
    {
        if (IsDone)
        {
            callback(Result);
            return;
        }
        _callbacks.Add(callback);
    }

    private void InvokeCallbacks()
    {
        foreach (Action<object?> callback in _callbacks.ToArray())
        {
            try
            {
                callback(Result);
            }
            catch
            {
                // Callback failures are reported by the Lua binder.
            }
        }
        _callbacks.Clear();
    }
}

public sealed class LspWorkspace
{
    private readonly LspDocumentStore _documents;
    private readonly LspPermissionPolicy _permissions;
    private readonly string? _workspaceRoot;
    private readonly List<(string Uri, string Name)> _folders = new();
    private readonly List<LspWatchRegistration> _watches = new();
    private readonly Dictionary<string, string> _configuration = new(StringComparer.Ordinal);

    public LspWorkspace(LspDocumentStore documents, LspPermissionPolicy permissions,
        string? workspaceRoot, IEnumerable<(string Uri, string Name)>? folders = null)
    {
        _documents = documents;
        _permissions = permissions;
        _workspaceRoot = workspaceRoot;
        if (folders is not null)
        {
            _folders.AddRange(folders);
        }
        else if (!string.IsNullOrEmpty(workspaceRoot))
        {
            _folders.Add((LspUri.FromPath(workspaceRoot),
                Path.GetFileName(workspaceRoot.TrimEnd(Path.DirectorySeparatorChar))));
        }
    }

    public string RootUri => _folders.Count > 0
        ? _folders[0].Uri
        : string.IsNullOrEmpty(_workspaceRoot) ? string.Empty : LspUri.FromPath(_workspaceRoot);

    public string? RootPath => _folders.Count > 0
        ? LspUri.ToPath(_folders[0].Uri)
        : _workspaceRoot;

    public IReadOnlyList<(string Uri, string Name)> Folders => _folders;

    public LspPermissionPolicy Permissions => _permissions;

    public LspDocumentStore Documents => _documents;

    public void ClearFolders()
    {
        _folders.Clear();
    }

    public void ReplaceRoot(string path)
    {
        string full = Path.GetFullPath(path);
        _folders.Clear();
        _folders.Add((LspUri.FromPath(full),
            Path.GetFileName(full.TrimEnd(Path.DirectorySeparatorChar))));
        _permissions.AllowedRoots.Add(full);
    }

    public void AddFolder(string uri, string name)
    {
        if (!_folders.Any(folder => string.Equals(folder.Uri, uri,
                StringComparison.Ordinal)))
        {
            _folders.Add((uri, name));
            _permissions.AllowedRoots.Add(LspUri.ToPath(uri));
        }
    }

    public void RemoveFolder(string uri)
    {
        _folders.RemoveAll(folder => string.Equals(folder.Uri, uri, StringComparison.Ordinal));
    }

    public JsonObject ConfigurationObject(string? section)
    {
        var result = new JsonObject();
        if (string.IsNullOrEmpty(section))
        {
            return result;
        }

        string[] parts = section.Split('.', StringSplitOptions.RemoveEmptyEntries);
        JsonNode? current = BuildConfigurationNode();
        foreach (string part in parts)
        {
            current = current is JsonObject obj ? obj[part] : null;
            if (current is null)
            {
                return result;
            }
        }

        return current as JsonObject ?? result;
    }

    public JsonNode? ConfigurationValue(string? section)
    {
        if (string.IsNullOrEmpty(section))
        {
            return BuildConfigurationNode();
        }

        JsonNode? current = BuildConfigurationNode();
        foreach (string part in section.Split('.', StringSplitOptions.RemoveEmptyEntries))
        {
            current = current is JsonObject obj ? obj[part] : null;
            if (current is null)
            {
                return null;
            }
        }
        return current;
    }

    public void SetConfiguration(string section, string value)
    {
        _configuration[section] = value;
    }

    private JsonNode BuildConfigurationNode()
    {
        var result = new JsonObject();
        foreach ((string key, string value) in _configuration)
        {
            result[key] = value;
        }
        return result;
    }

    public string ResolvePath(string pathOrUri)
    {
        string value = pathOrUri;
        if (value.StartsWith("file:", StringComparison.OrdinalIgnoreCase))
        {
            value = LspUri.ToPath(value);
        }

        if (!Path.IsPathRooted(value))
        {
            string? root = RootPath;
            value = string.IsNullOrEmpty(root) ? Path.GetFullPath(value) : Path.Combine(root, value);
        }

        return Path.GetFullPath(value);
    }

    public string ReadText(string pathOrUri)
    {
        string path = ResolvePath(pathOrUri);
        _permissions.AuthorizeRead(path);
        return File.ReadAllText(path, Encoding.UTF8);
    }

    public byte[] ReadBytes(string pathOrUri)
    {
        string path = ResolvePath(pathOrUri);
        _permissions.AuthorizeRead(path);
        return File.ReadAllBytes(path);
    }

    public void WriteText(string pathOrUri, string contents, bool createDirectories = false)
    {
        string path = ResolvePath(pathOrUri);
        _permissions.AuthorizeWrite(path);
        if (createDirectories)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path) ?? ".");
        }
        File.WriteAllText(path, contents, new UTF8Encoding(encoderShouldEmitUTF8Identifier: false));
    }

    public JsonObject Stat(string pathOrUri)
    {
        string path = ResolvePath(pathOrUri);
        _permissions.AuthorizeRead(path);
        var result = new JsonObject
        {
            ["path"] = path,
            ["uri"] = LspUri.FromPath(path),
            ["exists"] = false,
            ["directory"] = false,
            ["file"] = false,
            ["size"] = 0L,
            ["mtime"] = 0L
        };

        try
        {
            var info = new FileInfo(path);
            if (info.Exists)
            {
                result["exists"] = true;
                result["file"] = true;
                result["size"] = info.Length;
                result["mtime"] = new DateTimeOffset(info.LastWriteTimeUtc).ToUnixTimeMilliseconds();
                return result;
            }

            var directory = new DirectoryInfo(path);
            if (directory.Exists)
            {
                result["exists"] = true;
                result["directory"] = true;
                result["mtime"] = new DateTimeOffset(directory.LastWriteTimeUtc)
                    .ToUnixTimeMilliseconds();
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            result["error"] = ex.Message;
        }

        return result;
    }

    public IReadOnlyList<string> FindFiles(string glob, JsonObject? options = null)
    {
        string root = RootPath ?? Directory.GetCurrentDirectory();
        int maxResults = LspJson.GetInt32(options?["max_results"]) ??
                         LspJson.GetInt32(options?["maxResults"]) ?? 10_000;
        bool includeDirectories = LspJson.GetBool(options?["include_directories"]) ??
                                  LspJson.GetBool(options?["includeDirectories"]) ?? false;
        var results = new List<string>();
        if (!Directory.Exists(root) || maxResults <= 0)
        {
            return results;
        }

        string pattern = string.IsNullOrEmpty(glob) ? "**/*" : glob.Replace('\\', '/');
        Regex regex;
        try
        {
            regex = new Regex(LspUri.GlobToRegex(pattern),
                OperatingSystem.IsWindows()
                    ? RegexOptions.IgnoreCase | RegexOptions.CultureInvariant
                    : RegexOptions.CultureInvariant,
                TimeSpan.FromSeconds(1));
        }
        catch (ArgumentException)
        {
            return results;
        }

        try
        {
            foreach (string path in Directory.EnumerateFileSystemEntries(root, "*",
                         SearchOption.AllDirectories))
            {
                string relative = Path.GetRelativePath(root, path).Replace('\\', '/');
                if (!includeDirectories && Directory.Exists(path))
                {
                    continue;
                }

                if (!regex.IsMatch(relative) && !regex.IsMatch("/" + relative))
                {
                    continue;
                }

                results.Add(LspUri.FromPath(path));
                if (results.Count >= maxResults)
                {
                    break;
                }
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            _ = ex;
        }

        results.Sort(StringComparer.Ordinal);
        return results;
    }

    public LspWatchRegistration Watch(string glob, string? root = null)
    {
        var watch = new LspWatchRegistration(glob, root ?? RootPath ??
            Directory.GetCurrentDirectory());
        watch.Reset();
        _watches.Add(watch);
        return watch;
    }

    public void RemoveWatch(LspWatchRegistration watch)
    {
        watch.Cancel();
        _watches.Remove(watch);
    }

    public void PollWatches(List<(LspWatchRegistration Watch, JsonObject Event)> events)
    {
        for (int index = _watches.Count - 1; index >= 0; --index)
        {
            LspWatchRegistration watch = _watches[index];
            if (watch.IsCancelled)
            {
                _watches.RemoveAt(index);
                continue;
            }

            foreach (JsonObject item in watch.Poll())
            {
                events.Add((watch, item));
            }
        }
    }

    public void ClearWatches()
    {
        foreach (LspWatchRegistration watch in _watches)
        {
            watch.Cancel();
        }
        _watches.Clear();
    }

    public LspWorkspaceIndex CreateIndex(JsonObject? options)
    {
        return new LspWorkspaceIndex(options);
    }
}

public sealed class LspWatchRegistration
{
    private readonly string _glob;
    private readonly string _root;
    private readonly Regex? _regex;
    private readonly Dictionary<string, (long Length, long Mtime)> _known =
        new(StringComparer.Ordinal);

    public LspWatchRegistration(string glob, string root)
    {
        _glob = glob;
        _root = root;
        IsCancelled = false;
        string pattern = string.IsNullOrEmpty(glob) ? "**/*" : glob.Replace('\\', '/');
        try
        {
            _regex = new Regex(LspUri.GlobToRegex(pattern),
                OperatingSystem.IsWindows()
                    ? RegexOptions.IgnoreCase | RegexOptions.CultureInvariant
                    : RegexOptions.CultureInvariant,
                TimeSpan.FromSeconds(1));
        }
        catch (ArgumentException)
        {
            _regex = null;
        }
    }

    public bool IsCancelled { get; private set; }

    public string Glob => _glob;

    public NLua.LuaFunction? Callback { get; set; }

    public void Cancel() => IsCancelled = true;

    public void Reset()
    {
        _known.Clear();
        foreach ((string path, (long Length, long Mtime) stamp) in Snapshot())
        {
            _known[path] = stamp;
        }
    }

    public List<JsonObject> Poll()
    {
        var events = new List<JsonObject>();
        if (IsCancelled || !Directory.Exists(_root))
        {
            return events;
        }

        Dictionary<string, (long Length, long Mtime)> current = Snapshot();
        foreach ((string path, (long Length, long Mtime) stamp) in current)
        {
            if (!_known.TryGetValue(path, out (long Length, long Mtime) previous))
            {
                events.Add(WatchEvent(1, path));
            }
            else if (previous.Length != stamp.Length || previous.Mtime != stamp.Mtime)
            {
                events.Add(WatchEvent(2, path));
            }
        }

        foreach (string path in _known.Keys)
        {
            if (!current.ContainsKey(path))
            {
                events.Add(WatchEvent(3, path));
            }
        }

        _known.Clear();
        foreach ((string path, (long Length, long Mtime) stamp) in current)
        {
            _known[path] = stamp;
        }
        return events;
    }

    private Dictionary<string, (long Length, long Mtime)> Snapshot()
    {
        var result = new Dictionary<string, (long Length, long Mtime)>(StringComparer.Ordinal);
        try
        {
            foreach (string path in Directory.EnumerateFiles(_root, "*",
                         SearchOption.AllDirectories))
            {
                string relative = Path.GetRelativePath(_root, path).Replace('\\', '/');
                if (_regex is not null && !_regex.IsMatch(relative) &&
                    !_regex.IsMatch("/" + relative))
                {
                    continue;
                }

                try
                {
                    var info = new FileInfo(path);
                    result[path] = (info.Length, info.LastWriteTimeUtc.Ticks);
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                           NotSupportedException)
                {
                    // File disappeared during enumeration.
                }
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            _ = ex;
        }
        return result;
    }

    private JsonObject WatchEvent(int type, string path)
    {
        return new JsonObject
        {
            ["type"] = type,
            ["path"] = path,
            ["uri"] = LspUri.FromPath(path)
        };
    }
}

public sealed class LspWorkspaceIndex
{
    private readonly Dictionary<string, List<JsonObject>> _symbols =
        new(StringComparer.Ordinal);
    private readonly Dictionary<string, long> _versions = new(StringComparer.Ordinal);

    public LspWorkspaceIndex(JsonObject? options)
    {
        Options = options?.DeepClone().AsObject() ?? new JsonObject();
    }

    public JsonObject Options { get; }

    public int FileCount => _symbols.Count;

    public int SymbolCount => _symbols.Values.Sum(list => list.Count);

    public void AddFile(string uri, JsonArray symbols)
    {
        _symbols[uri] = symbols.OfType<JsonObject>()
            .Select(symbol => symbol.DeepClone().AsObject())
            .ToList();
        _versions[uri] = 0;
    }

    public void UpdateFile(string uri, JsonArray symbols)
    {
        AddFile(uri, symbols);
        _versions[uri] = _versions.TryGetValue(uri, out long version) ? version + 1 : 1;
    }

    public bool RemoveFile(string uri)
    {
        _versions.Remove(uri);
        return _symbols.Remove(uri);
    }

    public void Invalidate(string uri)
    {
        _versions.Remove(uri);
    }

    public void Clear()
    {
        _symbols.Clear();
        _versions.Clear();
    }

    public JsonArray QuerySymbols(string query, JsonObject? options)
    {
        int limit = LspJson.GetInt32(options?["max_results"]) ??
                    LspJson.GetInt32(options?["maxResults"]) ?? 256;
        bool exact = LspJson.GetBool(options?["exact"]) ?? false;
        var result = new JsonArray();

        foreach ((string uri, List<JsonObject> symbols) in _symbols)
        {
            foreach (JsonObject symbol in symbols)
            {
                string name = LspJson.GetString(symbol["name"]) ?? string.Empty;
                bool match = exact
                    ? string.Equals(name, query, StringComparison.OrdinalIgnoreCase)
                    : name.Contains(query, StringComparison.OrdinalIgnoreCase);
                if (!match)
                {
                    continue;
                }

                var item = symbol.DeepClone().AsObject();
                item["uri"] = uri;
                item["location"] = new JsonObject
                {
                    ["uri"] = uri,
                    ["range"] = symbol["range"]?.DeepClone()
                };
                LspJson.AddNode(result, item);
                if (result.Count >= limit)
                {
                    return result;
                }
            }
        }

        return result;
    }

    public JsonArray QueryReferences(JsonObject symbol, JsonObject? options)
    {
        string name = LspJson.GetString(symbol["name"]) ?? string.Empty;
        int limit = LspJson.GetInt32(options?["max_results"]) ??
                    LspJson.GetInt32(options?["maxResults"]) ?? 1024;
        var result = new JsonArray();
        foreach ((string uri, List<JsonObject> symbols) in _symbols)
        {
            foreach (JsonObject item in symbols)
            {
                if (!string.Equals(LspJson.GetString(item["name"]), name,
                        StringComparison.Ordinal))
                {
                    continue;
                }

                LspJson.AddNode(result, new JsonObject
                {
                    ["uri"] = uri,
                    ["range"] = item["range"]?.DeepClone()
                });
                if (result.Count >= limit)
                {
                    return result;
                }
            }
        }
        return result;
    }
}
