using System.Text.Json.Nodes;
using NLua;

namespace Hexal.Lsp;

/// <summary>
/// Translates the convenience capability names accepted by Lua servers into
/// standard LSP ServerCapabilities structures. Unknown keys are preserved as
/// raw capability entries.
/// </summary>
public static class LspCapabilities
{
    public static JsonObject Translate(LuaTable? nullSentinel, LuaTable table,
        JsonObject? semanticLegend)
    {
        JsonObject source = LspJson.ToObject(nullSentinel, table);
        return Translate(source, semanticLegend);
    }

    public static JsonObject Translate(JsonObject source, JsonObject? semanticLegend)
    {
        var result = new JsonObject();
        foreach ((string key, JsonNode? value) in source)
        {
            switch (key)
            {
                case "text_document_sync":
                case "textDocumentSync":
                    result["textDocumentSync"] = TranslateSync(value);
                    break;

                case "hover":
                    result["hoverProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "completion":
                    result["completionProvider"] = Provider(value, true,
                        "triggerCharacters", "allCommitCharacters", "resolveProvider",
                        "completionItem");
                    break;
                case "definition":
                    result["definitionProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "declaration":
                    result["declarationProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "type_definition":
                case "typeDefinition":
                    result["typeDefinitionProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "implementation":
                    result["implementationProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "references":
                    result["referencesProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "document_symbols":
                case "documentSymbols":
                    result["documentSymbolProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "workspace_symbols":
                case "workspaceSymbols":
                    result["workspaceSymbolProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "rename":
                    result["renameProvider"] = Provider(value, true, "prepareProvider");
                    break;
                case "code_actions":
                case "codeActions":
                    result["codeActionProvider"] = Provider(value, true,
                        "codeActionKinds", "resolveProvider");
                    break;
                case "formatting":
                    result["documentFormattingProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "range_formatting":
                case "rangeFormatting":
                    result["documentRangeFormattingProvider"] =
                        value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "on_type_formatting":
                case "onTypeFormatting":
                    result["documentOnTypeFormattingProvider"] = value?.DeepClone() ??
                        (JsonNode)true;
                    break;
                case "signature_help":
                case "signatureHelp":
                    result["signatureHelpProvider"] = Provider(value, true,
                        "triggerCharacters", "retriggerCharacters");
                    break;
                case "semantic_tokens":
                case "semanticTokens":
                    result["semanticTokensProvider"] = SemanticTokens(value,
                        semanticLegend);
                    break;
                case "inlay_hints":
                case "inlayHints":
                    result["inlayHintProvider"] = Provider(value, true,
                        "resolveProvider");
                    break;
                case "folding_ranges":
                case "foldingRanges":
                    result["foldingRangeProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "code_lens":
                case "codeLens":
                    result["codeLensProvider"] = Provider(value, true,
                        "resolveProvider");
                    break;
                case "document_links":
                case "documentLinks":
                    result["documentLinkProvider"] = Provider(value, true,
                        "resolveProvider");
                    break;
                case "selection_ranges":
                case "selectionRanges":
                    result["selectionRangeProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "linked_editing":
                case "linkedEditing":
                    result["linkedEditingRangeProvider"] =
                        value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "call_hierarchy":
                case "callHierarchy":
                    result["callHierarchyProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "type_hierarchy":
                case "typeHierarchy":
                    result["typeHierarchyProvider"] = value?.DeepClone() ?? (JsonNode)true;
                    break;
                case "execute_command":
                case "executeCommand":
                    result["executeCommandProvider"] = Provider(value, true,
                        "commands");
                    break;
                case "workspace":
                    result["workspace"] = value?.DeepClone();
                    break;
                case "experimental":
                    result["experimental"] = value?.DeepClone();
                    break;
                default:
                    // Raw structures remain available.
                    result[key] = value?.DeepClone();
                    break;
            }
        }
        return result;
    }

    private static JsonNode TranslateSync(JsonNode? value)
    {
        if (value is not JsonObject sync)
        {
            return value?.DeepClone() ?? (JsonNode)true;
        }

        var result = new JsonObject();
        if (sync["open_close"] is JsonValue openClose)
        {
            result["openClose"] = LspJson.GetBool(openClose);
        }
        else if (sync["openClose"] is JsonValue openCloseCamel)
        {
            result["openClose"] = LspJson.GetBool(openCloseCamel);
        }

        JsonNode? change = sync["change"];
        if (change is JsonValue changeValue)
        {
            if (changeValue.TryGetValue(out string? text))
            {
                result["change"] = text switch
                {
                    "incremental" => 2,
                    "none" => 0,
                    _ => 1
                };
            }
            else if (changeValue.TryGetValue(out int numeric))
            {
                result["change"] = numeric;
            }
        }

        if (sync["save"] is JsonValue saveValue)
        {
            if (saveValue.TryGetValue(out bool include) && include)
            {
                result["save"] = new JsonObject { ["includeText"] = true };
            }
            else
            {
                result["save"] = LspJson.GetBool(saveValue);
            }
        }
        else if (sync["save"] is JsonObject save)
        {
            bool includeText = LspJson.GetBool(save["include_text"]) ??
                               LspJson.GetBool(save["includeText"]) ?? false;
            result["save"] = new JsonObject { ["includeText"] = includeText };
        }

        if (LspJson.GetBool(sync["will_save"]) == true ||
            LspJson.GetBool(sync["willSave"]) == true)
        {
            result["willSave"] = true;
        }
        if (LspJson.GetBool(sync["will_save_wait_until"]) == true ||
            LspJson.GetBool(sync["willSaveWaitUntil"]) == true)
        {
            result["willSaveWaitUntil"] = true;
        }

        return result;
    }

    private static JsonNode Provider(JsonNode? value, bool resolveDefault,
        params string[] properties)
    {
        if (value is not JsonObject options)
        {
            return value is JsonValue flag && flag.TryGetValue(out bool enabled) && enabled
                ? new JsonObject { ["resolveProvider"] = resolveDefault }
                : value?.DeepClone() ?? new JsonObject { ["resolveProvider"] = resolveDefault };
        }

        var result = options.DeepClone().AsObject();
        bool hasResolve = properties.Contains("resolveProvider");
        if (hasResolve && result["resolveProvider"] is null)
        {
            result["resolveProvider"] = resolveDefault;
        }
        return result;
    }

    private static JsonObject SemanticTokens(JsonNode? value, JsonObject? legend)
    {
        var result = new JsonObject
        {
            ["full"] = new JsonObject { ["delta"] = true },
            ["range"] = true,
            ["legend"] = legend?.DeepClone() ?? new JsonObject
            {
                ["tokenTypes"] = LspJson.StringArray(
                    "namespace", "type", "class", "enum", "interface", "struct",
                    "typeParameter", "parameter", "variable", "property", "enumMember",
                    "event", "function", "method", "macro", "keyword", "modifier",
                    "comment", "string", "number", "regexp", "operator", "decorator"),
                ["tokenModifiers"] = LspJson.StringArray(
                    "declaration", "definition", "readonly", "static", "deprecated",
                    "abstract", "async", "modification", "documentation",
                    "defaultLibrary")
            }
        };

        if (value is JsonObject options)
        {
            foreach ((string key, JsonNode? item) in options)
            {
                result[key] = item?.DeepClone();
            }
            if (options["legend"] is JsonObject customLegend)
            {
                result["legend"] = customLegend.DeepClone();
            }
        }

        return result;
    }
}
