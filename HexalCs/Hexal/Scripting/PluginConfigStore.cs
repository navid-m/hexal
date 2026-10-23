using System.Text.Json;

namespace Hexal.Scripting;

/// <summary>
/// Per-plugin JSON configuration. Plugin data lives under
/// ~/.hexal/plugin-config/&lt;plugin-id&gt;.json and never mixes with the main
/// editor configuration.
/// </summary>
public sealed class PluginConfigStore
{
    private readonly string _path;
    private readonly Dictionary<string, object?> _values = new(StringComparer.Ordinal);

    public PluginConfigStore(string path)
    {
        _path = path;
    }

    public string Path => _path;

    public bool TryLoad(out string error)
    {
        error = string.Empty;
        _values.Clear();

        if (!File.Exists(_path))
        {
            return true;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(File.ReadAllText(_path));
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                error = "configuration root must be a JSON object";
                return false;
            }

            foreach (JsonProperty property in document.RootElement.EnumerateObject())
            {
                _values[property.Name] = FromJson(property.Value);
            }
            return true;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or JsonException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            error = ex.Message;
            return false;
        }
    }

    public object? Get(string key, object? defaultValue = null)
    {
        if (_values.TryGetValue(key, out object? value))
        {
            return value;
        }

        // Support dotted keys for nested tables without exposing the backing
        // dictionary itself.
        string[] parts = key.Split('.', StringSplitOptions.RemoveEmptyEntries);
        object? current = _values;
        foreach (string part in parts)
        {
            if (
                current is not Dictionary<string, object?> table
                || !table.TryGetValue(part, out current)
            )
            {
                return defaultValue;
            }
        }
        return current;
    }

    public void Set(string key, object? value)
    {
        if (key.Length == 0)
        {
            return;
        }

        string[] parts = key.Split('.', StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length <= 1)
        {
            _values[key] = value;
            return;
        }

        Dictionary<string, object?> table = _values;
        for (int i = 0; i < parts.Length - 1; ++i)
        {
            if (
                !table.TryGetValue(parts[i], out object? child)
                || child is not Dictionary<string, object?> childTable
            )
            {
                childTable = new Dictionary<string, object?>(StringComparer.Ordinal);
                table[parts[i]] = childTable;
            }
            table = childTable;
        }
        table[parts[^1]] = value;
    }

    public IReadOnlyDictionary<string, object?> Snapshot => _values;

    public bool Save(out string error)
    {
        error = string.Empty;

        try
        {
            string? directory = System.IO.Path.GetDirectoryName(_path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            using var stream = new FileStream(
                _path,
                FileMode.Create,
                FileAccess.Write,
                FileShare.None
            );
            using var writer = new Utf8JsonWriter(
                stream,
                new JsonWriterOptions { Indented = true }
            );

            writer.WriteStartObject();
            WriteObjectContents(writer, _values);
            writer.WriteEndObject();
            writer.Flush();
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

    private static object? FromJson(JsonElement element)
    {
        switch (element.ValueKind)
        {
            case JsonValueKind.Object:
            {
                var table = new Dictionary<string, object?>(StringComparer.Ordinal);
                foreach (JsonProperty property in element.EnumerateObject())
                {
                    table[property.Name] = FromJson(property.Value);
                }
                return table;
            }
            case JsonValueKind.Array:
            {
                var list = new List<object?>();
                foreach (JsonElement item in element.EnumerateArray())
                {
                    list.Add(FromJson(item));
                }
                return list;
            }
            case JsonValueKind.String:
                return element.GetString();
            case JsonValueKind.Number:
                return element.GetDouble();
            case JsonValueKind.True:
                return true;
            case JsonValueKind.False:
                return false;
            default:
                return null;
        }
    }

    private static void WriteObjectContents(
        Utf8JsonWriter writer,
        Dictionary<string, object?> table
    )
    {
        foreach ((string key, object? value) in table)
        {
            writer.WritePropertyName(key);
            WriteValue(writer, value);
        }
    }

    private static void WriteValue(Utf8JsonWriter writer, object? value)
    {
        switch (value)
        {
            case null:
                writer.WriteNullValue();
                break;
            case string text:
                writer.WriteStringValue(text);
                break;
            case bool flag:
                writer.WriteBooleanValue(flag);
                break;
            case byte or sbyte or short or ushort or int or uint or long or ulong:
                writer.WriteNumberValue(
                    Convert.ToInt64(value, System.Globalization.CultureInfo.InvariantCulture)
                );
                break;
            case float or double or decimal:
                writer.WriteNumberValue(
                    Convert.ToDouble(value, System.Globalization.CultureInfo.InvariantCulture)
                );
                break;
            case Dictionary<string, object?> table:
                writer.WriteStartObject();
                WriteObjectContents(writer, table);
                writer.WriteEndObject();
                break;
            case List<object?> list:
                writer.WriteStartArray();
                foreach (object? item in list)
                {
                    WriteValue(writer, item);
                }
                writer.WriteEndArray();
                break;
            default:
                writer.WriteStringValue(value.ToString());
                break;
        }
    }
}
