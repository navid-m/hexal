using System.Text.Json.Nodes;

namespace Hexal.Lsp;

public static class LspSemanticTokens
{
    public static bool Encode(
        JsonArray items,
        JsonObject legend,
        out JsonArray data,
        out string error
    )
    {
        error = string.Empty;
        data = new JsonArray();
        var tokenTypes = legend["tokenTypes"] as JsonArray ?? new JsonArray();
        var tokenModifiers = legend["tokenModifiers"] as JsonArray ?? new JsonArray();

        var parsed = new List<(int Line, int Character, int Length, int Type, int Modifiers)>();
        foreach (JsonNode? node in items)
        {
            if (node is not JsonObject item)
            {
                error = "semantic token must be an object";
                return false;
            }

            int line = LspJson.GetInt32(item["line"]) ?? LspJson.GetInt32(item["startLine"]) ?? 0;
            int character =
                LspJson.GetInt32(item["character"])
                ?? LspJson.GetInt32(item["startCharacter"])
                ?? LspJson.GetInt32(item["start"]?["character"])
                ?? 0;
            int length = LspJson.GetInt32(item["length"]) ?? 0;
            if (length <= 0)
            {
                continue;
            }

            string type =
                LspJson.GetString(item["type"])
                ?? LspJson.GetString(item["tokenType"])
                ?? LspJson.GetString(item["token_type"])
                ?? "variable";
            int typeIndex = IndexOf(tokenTypes, type, StringComparison.Ordinal);
            if (typeIndex < 0)
            {
                error = $"unknown semantic token type '{type}'";
                return false;
            }

            int modifiers = 0;
            if (item["modifiers"] is JsonArray modifierArray)
            {
                foreach (JsonNode? modifierNode in modifierArray)
                {
                    string modifier = LspJson.GetString(modifierNode) ?? string.Empty;
                    int modifierIndex = IndexOf(tokenModifiers, modifier, StringComparison.Ordinal);
                    if (modifierIndex < 0)
                    {
                        error = $"unknown semantic token modifier '{modifier}'";
                        return false;
                    }
                    modifiers |= 1 << modifierIndex;
                }
            }

            parsed.Add((Math.Max(0, line), Math.Max(0, character), length, typeIndex, modifiers));
        }

        parsed.Sort(
            (left, right) =>
                left.Line == right.Line
                    ? left.Character.CompareTo(right.Character)
                    : left.Line.CompareTo(right.Line)
        );

        int previousLine = 0;
        int previousCharacter = 0;
        foreach ((int line, int character, int length, int type, int modifiers) in parsed)
        {
            int deltaLine = line - previousLine;
            int deltaCharacter = deltaLine == 0 ? character - previousCharacter : character;
            LspJson.AddNode(data, JsonValue.Create(deltaLine));
            LspJson.AddNode(data, JsonValue.Create(deltaCharacter));
            LspJson.AddNode(data, JsonValue.Create(length));
            LspJson.AddNode(data, JsonValue.Create(type));
            LspJson.AddNode(data, JsonValue.Create(modifiers));
            previousLine = line;
            previousCharacter = character;
        }
        return true;
    }

    private static int IndexOf(JsonArray array, string value, StringComparison comparison)
    {
        for (int index = 0; index < array.Count; ++index)
        {
            string? candidate = LspJson.GetString(array[index]);
            if (candidate is not null && string.Equals(candidate, value, comparison))
            {
                return index;
            }
        }
        return -1;
    }
}
