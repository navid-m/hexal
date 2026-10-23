using System.Text.Json.Nodes;
using NLua;

namespace Hexal.Lsp;

/// <summary>
/// An in-memory LSP client for server authors. It simulates document changes,
/// requests, notifications, cancellation and server-initiated requests without
/// starting the editor or a transport process.
/// </summary>
public sealed class LuaLspTestClient
{
    private readonly LuaLspCoordinator _coordinator;
    private readonly LuaLspServer _server;
    private readonly Queue<string> _outgoing = new();
    private readonly Dictionary<long, (bool Done, JsonNode? Result, string Error)> _pending = new();
    private readonly Dictionary<string, long> _versions = new(StringComparer.Ordinal);
    private readonly List<JsonObject> _notifications = new();
    private long _nextId = 1;
    private bool _initialized;

    public LuaLspTestClient(LuaLspCoordinator coordinator, LuaLspServer server)
    {
        _coordinator = coordinator;
        _server = server;
        _server.SetOutgoing(_outgoing.Enqueue);
    }

    public LuaTable CreateTable()
    {
        LuaTable table = _coordinator.NewTable();
        table["initialize"] = new Func<object?, LuaTable?>(options =>
        {
            var parameters = new JsonObject
            {
                ["processId"] = Environment.ProcessId,
                ["clientInfo"] = new JsonObject { ["name"] = "hexal-testing" },
                ["rootUri"] = _server.Workspace.RootUri,
                ["workspaceFolders"] = Folders(),
                ["capabilities"] = new JsonObject
                {
                    ["general"] = new JsonObject
                    {
                        ["positionEncodings"] = LspJson.StringArray("utf-16", "utf-8", "utf-32"),
                    },
                    ["workspace"] = new JsonObject
                    {
                        ["workspaceFolders"] = true,
                        ["configuration"] = true,
                    },
                    ["textDocument"] = new JsonObject
                    {
                        ["hover"] = new JsonObject
                        {
                            ["contentFormat"] = LspJson.StringArray("markdown", "plaintext"),
                        },
                        ["completion"] = new JsonObject
                        {
                            ["completionItem"] = new JsonObject
                            {
                                ["snippetSupport"] = true,
                                ["resolveSupport"] = new JsonObject
                                {
                                    ["properties"] = LspJson.StringArray("documentation", "detail"),
                                },
                            },
                        },
                        ["publishDiagnostics"] = new JsonObject
                        {
                            ["relatedInformation"] = true,
                            ["versionSupport"] = true,
                        },
                        ["synchronization"] = new JsonObject
                        {
                            ["dynamicRegistration"] = false,
                            ["didSave"] = true,
                        },
                    },
                    ["window"] = new JsonObject { ["workDoneProgress"] = true },
                },
            };

            if (options is LuaTable optionTable)
            {
                JsonObject? extra = _coordinator.AsObject(optionTable);
                if (extra is not null)
                {
                    foreach ((string key, JsonNode? value) in extra)
                    {
                        parameters[key] = value?.DeepClone();
                    }
                }
            }

            (JsonNode? result, string error) = Request("initialize", parameters);
            if (error.Length > 0)
            {
                return null;
            }
            _initialized = true;
            Notify("initialized", new JsonObject());
            return _coordinator.ToLua(result) as LuaTable;
        });

        table["initialized"] = new Func<bool>(() => _initialized);

        table["open_document"] = new Func<string, string, string, object?, bool>(
            (uriValue, languageId, text, version) =>
            {
                long nextVersion = LspJson.ToInt64(version) ?? 1;
                bool sent = Notify(
                    "textDocument/didOpen",
                    new JsonObject
                    {
                        ["textDocument"] = new JsonObject
                        {
                            ["uri"] = uriValue,
                            ["languageId"] = languageId ?? string.Empty,
                            ["version"] = nextVersion,
                            ["text"] = text ?? string.Empty,
                        },
                    }
                );
                _versions[uriValue] = nextVersion;
                return sent;
            }
        );

        table["change_document"] = new Func<string, object?, object?, bool>(
            (uriValue, changes, version) =>
            {
                long nextVersion =
                    LspJson.ToInt64(version)
                    ?? (_versions.TryGetValue(uriValue, out long current) ? current + 1 : 1);
                bool sent = Notify(
                    "textDocument/didChange",
                    new JsonObject
                    {
                        ["textDocument"] = new JsonObject
                        {
                            ["uri"] = uriValue,
                            ["version"] = nextVersion,
                        },
                        ["contentChanges"] =
                            changes is LuaTable ? _coordinator.AsArray(changes) : new JsonArray(),
                    }
                );
                _versions[uriValue] = nextVersion;
                return sent;
            }
        );

        table["save_document"] = new Func<string, object?, bool>(
            (uriValue, text) =>
            {
                var parameters = new JsonObject
                {
                    ["textDocument"] = new JsonObject { ["uri"] = uriValue },
                };
                if (text is not null)
                {
                    parameters["text"] = text.ToString();
                }
                return Notify("textDocument/didSave", parameters);
            }
        );

        table["close_document"] = new Func<string, bool>(uriValue =>
        {
            _versions.Remove(uriValue);
            return Notify(
                "textDocument/didClose",
                new JsonObject { ["textDocument"] = new JsonObject { ["uri"] = uriValue } }
            );
        });

        table["request"] = new Func<string, object?, LuaTable?>(
            (method, parameters) =>
            {
                (JsonNode? result, string error) = Request(method, parameters);
                if (error.Length > 0)
                {
                    return null;
                }
                return _coordinator.ToLua(result) as LuaTable;
            }
        );

        table["request_raw"] = new Func<string, object?, LuaTable>(
            (method, parameters) =>
            {
                (JsonNode? result, string error) = Request(method, parameters);
                LuaTable table = _coordinator.NewTable();
                table["result"] = _coordinator.ToLua(result);
                table["error"] = error;
                table["ok"] = error.Length == 0;
                return _coordinator.WrapTable(table);
            }
        );

        table["notify"] = new Func<string, object?, bool>(Notify);
        table["cancel"] = new Func<object?, bool>(idValue =>
        {
            long? id = LspJson.ToInt64(idValue);
            if (id is null)
            {
                return false;
            }
            return Notify("$/cancelRequest", new JsonObject { ["id"] = id.Value });
        });
        table["diagnostics"] = new Func<string, LuaTable>(uriValue =>
            _coordinator.ToLua(_coordinator.Diagnostics.Get(uriValue)) as LuaTable
            ?? _coordinator.NewTable()
        );
        table["notifications"] = new Func<LuaTable>(() =>
            _coordinator.NewArray(_notifications.Select(item => (object?)_coordinator.ToLua(item)))
        );
        table["messages"] = table["notifications"];
        table["server"] = new Func<LuaTable>(() => _server.ServerTable);
        table["tick"] = new Func<bool>(() =>
        {
            _coordinator.Tick();
            Drain();
            return true;
        });
        table["shutdown"] = new Func<bool>(() =>
        {
            (JsonNode? _, string error) = Request("shutdown", null);
            Notify("exit", new JsonObject { ["code"] = 0 });
            _initialized = false;
            return error.Length == 0;
        });
        table["set_outgoing"] = new Func<bool>(() =>
        {
            _server.SetOutgoing(_outgoing.Enqueue);
            return true;
        });
        _coordinator.WrapTable(table);
        _coordinator.PadTable(table, "initialize", 1);
        _coordinator.PadTable(table, "open_document", 4);
        _coordinator.PadTable(table, "change_document", 3);
        _coordinator.PadTable(table, "save_document", 2);
        _coordinator.PadTable(table, "request", 2);
        _coordinator.PadTable(table, "request_raw", 2);
        _coordinator.PadTable(table, "notify", 2);
        _coordinator.PadTable(table, "cancel", 1);
        _coordinator.PadTable(table, "diagnostics", 1);
        return table;
    }

    private JsonArray Folders()
    {
        var folders = new JsonArray();
        foreach ((string uri, string name) in _server.Workspace.Folders)
        {
            folders.Add((JsonNode)new JsonObject { ["uri"] = uri, ["name"] = name });
        }
        return folders;
    }

    private bool Notify(string method, object? parameters)
    {
        JsonNode? paramsNode = parameters is null ? null : _coordinator.FromLua(parameters);
        var message = new JsonObject { ["jsonrpc"] = "2.0", ["method"] = method };
        if (paramsNode is not null)
        {
            message["params"] = paramsNode;
        }
        _server.HandleIncoming(message.ToJsonString());
        Drain();
        return true;
    }

    private (JsonNode? Result, string Error) Request(string method, object? parameters)
    {
        long id = _nextId++;
        _pending[id] = (false, null, string.Empty);
        JsonNode? paramsNode = parameters is null ? null : _coordinator.FromLua(parameters);
        var message = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id,
            ["method"] = method,
        };
        if (paramsNode is not null)
        {
            message["params"] = paramsNode;
        }
        _server.HandleIncoming(message.ToJsonString());
        Drain();

        if (!_pending.TryGetValue(id, out (bool Done, JsonNode? Result, string Error) pending))
        {
            return (null, string.Empty);
        }
        _pending.Remove(id);
        return (pending.Result, pending.Error);
    }

    private void Drain()
    {
        while (_outgoing.Count > 0)
        {
            string message = _outgoing.Dequeue();
            JsonObject? root;
            try
            {
                root = JsonNode.Parse(message) as JsonObject;
            }
            catch (System.Text.Json.JsonException)
            {
                continue;
            }

            if (root is null)
            {
                continue;
            }

            string method = LspJson.GetString(root["method"]) ?? string.Empty;
            JsonNode? id = root["id"];
            if (method.Length > 0)
            {
                if (id is not null)
                {
                    RespondToServerRequest(root, id, method);
                }
                else
                {
                    _notifications.Add((JsonObject)root.DeepClone());
                }
                continue;
            }

            if (
                id is JsonValue value
                && value.TryGetValue(out long numeric)
                && _pending.TryGetValue(
                    numeric,
                    out (bool Done, JsonNode? Result, string Error) pending
                )
            )
            {
                string error = string.Empty;
                JsonNode? result = null;
                if (root["error"] is JsonObject errorObject)
                {
                    error = LspJson.GetString(errorObject["message"]) ?? "request failed";
                }
                else
                {
                    result = root["result"]?.DeepClone();
                }
                _pending[numeric] = (true, result, error);
            }
        }
    }

    private void RespondToServerRequest(JsonObject root, JsonNode id, string method)
    {
        JsonNode? result = null;
        if (method == "workspace/configuration")
        {
            int count = root["params"]?["items"] is JsonArray items ? items.Count : 0;
            var array = new JsonArray();
            for (int index = 0; index < count; ++index)
            {
                array.Add((JsonNode?)null);
            }
            result = array;
        }
        else if (method == "workspace/workspaceFolders")
        {
            result = Folders();
        }
        else if (
            method == "client/registerCapability"
            || method == "client/unregisterCapability"
            || method == "window/workDoneProgress/create"
        )
        {
            result = null;
        }
        else if (method == "workspace/applyEdit")
        {
            result = new JsonObject { ["applied"] = true };
        }

        var response = new JsonObject
        {
            ["jsonrpc"] = "2.0",
            ["id"] = id.DeepClone(),
            ["result"] = result,
        };
        _server.HandleIncoming(response.ToJsonString());
    }
}
