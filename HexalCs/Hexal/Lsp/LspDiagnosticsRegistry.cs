using System.Text.Json.Nodes;

namespace Hexal.Lsp;

/// <summary>
/// Namespace-aware diagnostics storage. Publishing one namespace does not
/// erase diagnostics owned by another analyzer. The combined set is what is
/// sent to the client.
/// </summary>
public sealed class LspDiagnosticsRegistry
{
    private sealed class Entry
    {
        public JsonArray Items = new();
        public long? Version;
    }

    private readonly Dictionary<string, Dictionary<string, Entry>> _documents = new(
        StringComparer.Ordinal
    );

    public event Action<string, JsonArray>? Changed;

    public bool Set(string uri, string @namespace, JsonArray items, long? version)
    {
        if (!_documents.TryGetValue(uri, out Dictionary<string, Entry>? namespaces))
        {
            namespaces = new Dictionary<string, Entry>(StringComparer.Ordinal);
            _documents[uri] = namespaces;
        }

        if (
            namespaces.TryGetValue(@namespace, out Entry? existing)
            && existing.Version is long oldVersion
            && version is long newVersion
            && newVersion < oldVersion
        )
        {
            // Results computed from an obsolete snapshot are discarded.
            return false;
        }

        var entry = new Entry { Items = CloneArray(items), Version = version };
        namespaces[@namespace] = entry;
        Notify(uri);
        return true;
    }

    public void Publish(string uri, JsonArray items, long? version)
    {
        Set(uri, "default", items, version);
    }

    public bool Clear(string uri)
    {
        if (!_documents.Remove(uri))
        {
            return false;
        }
        Changed?.Invoke(uri, new JsonArray());
        return true;
    }

    public bool Clear(string uri, string @namespace)
    {
        if (
            !_documents.TryGetValue(uri, out Dictionary<string, Entry>? namespaces)
            || !namespaces.Remove(@namespace)
        )
        {
            return false;
        }

        if (namespaces.Count == 0)
        {
            _documents.Remove(uri);
        }
        Notify(uri);
        return true;
    }

    public void ClearAll()
    {
        string[] uris = _documents.Keys.ToArray();
        _documents.Clear();
        foreach (string uri in uris)
        {
            Changed?.Invoke(uri, new JsonArray());
        }
    }

    public JsonArray Get(string uri)
    {
        var result = new JsonArray();
        if (!_documents.TryGetValue(uri, out Dictionary<string, Entry>? namespaces))
        {
            return result;
        }

        foreach (Entry entry in namespaces.Values)
        {
            foreach (JsonNode? item in entry.Items)
            {
                result.Add(item?.DeepClone());
            }
        }
        return result;
    }

    public JsonObject Snapshot()
    {
        var result = new JsonObject();
        foreach ((string uri, Dictionary<string, Entry> namespaces) in _documents)
        {
            var item = new JsonObject();
            foreach ((string name, Entry entry) in namespaces)
            {
                item[name] = CloneArray(entry.Items);
            }
            result[uri] = item;
        }
        return result;
    }

    public int Count(string uri) => Get(uri).Count;

    public int CountAll() =>
        _documents.Values.Sum(namespaces => namespaces.Values.Sum(entry => entry.Items.Count));

    private void Notify(string uri)
    {
        Changed?.Invoke(uri, Get(uri));
    }

    private static JsonArray CloneArray(JsonArray source)
    {
        var result = new JsonArray();
        foreach (JsonNode? item in source)
        {
            result.Add(item?.DeepClone());
        }
        return result;
    }
}
