using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using NLua;

namespace Hexal.Lsp;

/// <summary>
/// Standard JSON-RPC and Language Server Protocol error codes.
/// Plugins receive these values through hexal.lsp.protocol.error_codes.
/// </summary>
public static class LspErrorCodes
{
    public const int ParseError = -32700;
    public const int InvalidRequest = -32600;
    public const int MethodNotFound = -32601;
    public const int InvalidParams = -32602;
    public const int InternalError = -32603;
    public const int ServerNotInitialized = -32002;
    public const int RequestCancelled = -32800;
    public const int ContentModified = -32801;
    public const int RequestFailed = -32803;
    public const int ServerCancelled = -32802;
}

/// <summary>
/// A protocol-level error returned by a Lua handler (or produced by the
/// runtime). It is deliberately distinct from a nil result and from a Lua
/// exception.
/// </summary>
public sealed class LspProtocolError
{
    public LspProtocolError(int code, string message, JsonNode? data = null)
    {
        Code = code;
        Message = message ?? string.Empty;
        Data = data;
    }

    public int Code { get; }

    public string Message { get; }

    public JsonNode? Data { get; }
}

/// <summary>
/// Converts between NLua values and System.Text.Json nodes. JSON null is
/// represented by a unique Lua table (exposed as hexal.json.null) so that null
/// entries survive decoding inside arrays.
/// </summary>
public static class LspJson
{
    public static JsonNode? FromLua(LuaTable? nullSentinel, object? value, int depth = 0)
    {
        if (depth > 64)
        {
            return null;
        }

        switch (value)
        {
            case null:
                return null;
            case LuaTable table when nullSentinel is not null &&
                                      ReferenceEquals(table, nullSentinel):
                return null;
            case LuaTable table:
                return FromLuaTable(nullSentinel, table, depth);
            case string text:
                return JsonValue.Create(text);
            case bool flag:
                return JsonValue.Create(flag);
            case char character:
                return JsonValue.Create(character.ToString());
            case byte or sbyte or short or ushort or int or uint or long or ulong:
                return JsonValue.Create(Convert.ToInt64(value, CultureInfo.InvariantCulture));
            case float or double or decimal:
                return JsonValue.Create(Convert.ToDouble(value, CultureInfo.InvariantCulture));
            case JsonNode node:
                return node.DeepClone();
            case System.Collections.IDictionary dictionary:
            {
                var result = new JsonObject();
                foreach (System.Collections.DictionaryEntry entry in dictionary)
                {
                    result[entry.Key?.ToString() ?? string.Empty] =
                        FromLua(nullSentinel, entry.Value, depth + 1);
                }
                return result;
            }
            case System.Collections.IEnumerable sequence when value is not string:
            {
                var result = new JsonArray();
                foreach (object? item in sequence)
                {
                    result.Add(FromLua(nullSentinel, item, depth + 1));
                }
                return result;
            }
            default:
                return JsonValue.Create(value.ToString());
        }
    }

    private static JsonNode FromLuaTable(LuaTable? nullSentinel, LuaTable table,
        int depth)
    {
        var keys = new List<object?>();
        foreach (object? key in table.Keys)
        {
            if (key is not null)
            {
                keys.Add(key);
            }
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
            var array = new JsonArray();
            for (int index = 1; index <= keys.Count; ++index)
            {
                array.Add(FromLua(nullSentinel, table[index], depth + 1));
            }
            return array;
        }

        var obj = new JsonObject();
        foreach (object? key in keys)
        {
            string name = key switch
            {
                null => string.Empty,
                string text => text,
                _ => Convert.ToString(key, CultureInfo.InvariantCulture) ?? string.Empty
            };
            obj[name] = FromLua(nullSentinel, table[key!], depth + 1);
        }
        return obj;
    }

    public static object? ToLuaValue(Lua lua, LuaTable nullSentinel, JsonNode? value)
    {
        return ToLuaValue(lua, nullSentinel, value, 0);
    }

    private static object? ToLuaValue(Lua lua, LuaTable nullSentinel, JsonNode? value,
        int depth)
    {
        if (depth > 64 || value is null)
        {
            return nullSentinel;
        }

        switch (value)
        {
            case JsonObject obj:
            {
                LuaTable table = NewTable(lua);
                foreach ((string key, JsonNode? item) in obj)
                {
                    table[key] = ToLuaValue(lua, nullSentinel, item, depth + 1);
                }
                return table;
            }
            case JsonArray array:
            {
                LuaTable table = NewTable(lua);
                for (int index = 0; index < array.Count; ++index)
                {
                    JsonNode? item = array[index];
                    table[index + 1] = item is null
                        ? nullSentinel
                        : ToLuaValue(lua, nullSentinel, item, depth + 1);
                }
                return table;
            }
            case JsonValue jsonValue:
            {
                switch (jsonValue.GetValueKind())
                {
                    case JsonValueKind.Null:
                    case JsonValueKind.Undefined:
                        return nullSentinel;
                    case JsonValueKind.True:
                        return true;
                    case JsonValueKind.False:
                        return false;
                    case JsonValueKind.String:
                        return jsonValue.TryGetValue(out string? text)
                            ? text ?? string.Empty
                            : string.Empty;
                    case JsonValueKind.Number:
                        if (GetInt64(jsonValue) is long integer)
                        {
                            return integer;
                        }
                        return jsonValue.TryGetValue(out double number) ? number : 0d;
                    default:
                        return jsonValue.ToString();
                }
            }
            default:
                return nullSentinel;
        }
    }

    public static void AddNode(JsonArray array, JsonNode? value)
    {
        array.Add(value);
    }

    public static JsonArray StringArray(params string[] values)
    {
        var result = new JsonArray();
        foreach (string value in values)
        {
            AddNode(result, JsonValue.Create(value));
        }
        return result;
    }

    public static JsonArray NumberArray(params long[] values)
    {
        var result = new JsonArray();
        foreach (long value in values)
        {
            AddNode(result, JsonValue.Create(value));
        }
        return result;
    }

    public static LuaTable NewTable(Lua lua)
    {
        return (LuaTable)lua.DoString("return {}")[0]!;
    }

    public static LuaTable NewArray(Lua lua, IEnumerable<object?> values)
    {
        LuaTable table = NewTable(lua);
        int index = 1;
        foreach (object? value in values)
        {
            table[index++] = value;
        }
        return table;
    }

    public static string Encode(JsonNode? value)
    {
        return value?.ToJsonString() ?? "null";
    }

    public static JsonNode? Parse(string text)
    {
        return JsonNode.Parse(text);
    }

    public static bool IsValid(string text)
    {
        try
        {
            using JsonDocument document = JsonDocument.Parse(text);
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    public static int? GetInt32(JsonNode? node)
    {
        long? value = GetInt64(node);
        if (value is null || value < int.MinValue || value > int.MaxValue)
        {
            return null;
        }
        return (int)value.Value;
    }

    public static long? GetInt64(JsonNode? node)
    {
        if (node is not JsonValue value)
        {
            return null;
        }

        JsonValueKind kind = value.GetValueKind();
        if (kind is JsonValueKind.Null or JsonValueKind.Undefined)
        {
            return null;
        }

        if (value.TryGetValue(out int integer))
        {
            return integer;
        }
        if (value.TryGetValue(out long longValue))
        {
            return longValue;
        }
        if (value.TryGetValue(out double number))
        {
            return (long)number;
        }
        if (value.TryGetValue(out string? text) &&
            long.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture,
                out long parsed))
        {
            return parsed;
        }
        return null;
    }

    public static bool? GetBool(JsonNode? node)
    {
        if (node is not JsonValue value)
        {
            return null;
        }
        if (value.TryGetValue(out bool flag))
        {
            return flag;
        }
        long? number = GetInt64(node);
        return number is null ? null : number.Value != 0;
    }

    public static string? GetString(JsonNode? node)
    {
        if (node is not JsonValue value)
        {
            return null;
        }
        return value.TryGetValue(out string? text) ? text : value.ToString();
    }

    public static bool TryNumericKey(object? key, out double value)
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

    public static long? ToInt64(object? value)
    {
        switch (value)
        {
            case null:
                return null;
            case long integer:
                return integer;
            case int integer:
                return integer;
            case double number when number >= long.MinValue && number <= long.MaxValue:
                return (long)number;
            case float number when number >= long.MinValue && number <= long.MaxValue:
                return (long)number;
            case string text when long.TryParse(text, NumberStyles.Integer,
                CultureInfo.InvariantCulture, out long parsed):
                return parsed;
            default:
                try
                {
                    return Convert.ToInt64(value, CultureInfo.InvariantCulture);
                }
                catch (Exception ex) when (ex is FormatException or InvalidCastException or
                                           OverflowException)
                {
                    return null;
                }
        }
    }

    public static int? ToInt32(object? value)
    {
        long? integer = ToInt64(value);
        if (integer is null || integer < int.MinValue || integer > int.MaxValue)
        {
            return null;
        }
        return (int)integer.Value;
    }

    public static bool ToBool(object? value, bool fallback = false)
    {
        return value switch
        {
            null => fallback,
            bool flag => flag,
            long integer => integer != 0,
            int integer => integer != 0,
            double number => number != 0,
            string text when bool.TryParse(text, out bool parsed) => parsed,
            _ => fallback
        };
    }

    public static List<object?> ToList(LuaTable table)
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

    public static JsonObject ToObject(LuaTable? nullSentinel, LuaTable table)
    {
        return FromLua(nullSentinel, table) as JsonObject ?? new JsonObject();
    }
}
