using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Hexal.Util;
using NLua;
using NLua.Exceptions;

namespace Hexal.Lsp;

/// <summary>
/// Registers the documented hexal.lsp.*, hexal.async, hexal.json, hexal.fs,
/// hexal.log, hexal.index and hexal.testing modules on a plugin's Lua state.
/// </summary>
public static class LuaLspModuleBinder
{
    public static void Bind(Lua lua, LuaTable root, LuaLspCoordinator coordinator)
    {
        LuaTable lsp = GetOrCreate(coordinator, root, "lsp");
        coordinator.Modules["hexal.lsp"] = lsp;
        LuaTable protocol = AddModule(coordinator, lsp, "protocol", "hexal.lsp.protocol");
        LuaTable documentsModule = AddModule(coordinator, lsp, "documents", "hexal.lsp.documents");
        LuaTable workspaceModule = AddModule(coordinator, lsp, "workspace", "hexal.lsp.workspace");
        LuaTable capabilities = AddModule(
            coordinator,
            lsp,
            "capabilities",
            "hexal.lsp.capabilities"
        );
        LuaTable diagnostics = AddModule(coordinator, lsp, "diagnostics", "hexal.lsp.diagnostics");
        LuaTable completion = AddModule(coordinator, lsp, "completion", "hexal.lsp.completion");
        LuaTable symbols = AddModule(coordinator, lsp, "symbols", "hexal.lsp.symbols");
        LuaTable edit = AddModule(coordinator, lsp, "edit", "hexal.lsp.edit");
        LuaTable types = AddModule(coordinator, lsp, "types", "hexal.lsp.types");
        LuaTable uri = AddModule(coordinator, lsp, "uri", "hexal.lsp.uri");
        LuaTable position = AddModule(coordinator, lsp, "position", "hexal.lsp.position");
        LuaTable semanticTokens = AddModule(
            coordinator,
            lsp,
            "semantic_tokens",
            "hexal.lsp.semantic_tokens"
        );
        LuaTable inlayHint = AddModule(coordinator, lsp, "inlay_hint", "hexal.lsp.inlay_hint");
        LuaTable signatureHelp = AddModule(
            coordinator,
            lsp,
            "signature_help",
            "hexal.lsp.signature_help"
        );
        LuaTable foldingRange = AddModule(
            coordinator,
            lsp,
            "folding_range",
            "hexal.lsp.folding_range"
        );
        LuaTable codeLens = AddModule(coordinator, lsp, "code_lens", "hexal.lsp.code_lens");
        LuaTable serverModule = AddModule(coordinator, lsp, "server", "hexal.lsp.server");

        coordinator.DocumentsModule = documentsModule;
        coordinator.WorkspaceModule = workspaceModule;
        coordinator.DiagnosticsModule = diagnostics;

        LuaTable asyncModule = GetOrCreate(coordinator, root, "async");
        LuaTable jsonModule = GetOrCreate(coordinator, root, "json");
        LuaTable fsModule = GetOrCreate(coordinator, root, "fs");
        LuaTable logModule = GetOrCreate(coordinator, root, "log");
        LuaTable indexModule = GetOrCreate(coordinator, root, "index");
        LuaTable processModule = GetOrCreate(coordinator, root, "process");
        LuaTable debugModule = GetOrCreate(coordinator, root, "debug");
        LuaTable testingModule = GetOrCreate(coordinator, root, "testing");
        LuaTable testingLsp = AddModule(coordinator, testingModule, "lsp", "hexal.testing.lsp");
        coordinator.Modules["hexal.async"] = asyncModule;
        coordinator.Modules["hexal.json"] = jsonModule;
        coordinator.Modules["hexal.fs"] = fsModule;
        coordinator.Modules["hexal.log"] = logModule;
        coordinator.Modules["hexal.index"] = indexModule;
        coordinator.Modules["hexal.process"] = processModule;
        coordinator.Modules["hexal.debug"] = debugModule;
        coordinator.Modules["hexal.testing"] = testingModule;

        BindProtocol(coordinator, protocol);
        BindServerModule(coordinator, lsp, serverModule);
        BindDocuments(coordinator, documentsModule);
        BindWorkspace(coordinator, workspaceModule);
        BindCapabilities(coordinator, capabilities);
        BindDiagnostics(coordinator, diagnostics);
        BindCompletion(coordinator, completion);
        BindSymbols(coordinator, symbols);
        BindEdit(coordinator, edit);
        BindTypes(coordinator, types);
        BindUri(coordinator, uri);
        BindPosition(coordinator, position);
        BindSemanticTokens(coordinator, semanticTokens);
        BindInlayHint(coordinator, inlayHint);
        BindSignatureHelp(coordinator, signatureHelp);
        BindFoldingRange(coordinator, foldingRange);
        BindCodeLens(coordinator, codeLens);
        BindJson(coordinator, jsonModule);
        BindFs(coordinator, fsModule);
        BindLog(coordinator, logModule);
        BindIndex(coordinator, indexModule);
        BindProcess(coordinator, processModule);
        BindDebug(coordinator, debugModule);
        BindTesting(coordinator, testingLsp);

        LuaTable host = coordinator.NewTable();
        host["async_after"] = new Func<long, LuaFunction, long>(coordinator.Schedule);
        host["async_cancel"] = new Func<long, bool>(coordinator.CancelTimer);
        host["async_is_cancelled"] = new Func<bool>(() =>
            coordinator.CurrentCancellation.IsCancellationRequested
        );
        host["async_tick"] = new Func<bool>(() =>
        {
            coordinator.Tick();
            return true;
        });
        host["documents_list"] = new Func<LuaTable>(() =>
        {
            var values = new List<object?>();
            foreach (LspDocument document in coordinator.Documents.All)
            {
                values.Add(coordinator.CreateDocumentTable(document));
            }
            return coordinator.NewArray(values);
        });
        host["fs_join"] = new Func<LuaTable, string>(arguments =>
        {
            var parts = new List<string>();
            foreach (object? item in LspJson.ToList(arguments))
            {
                parts.Add(item?.ToString() ?? string.Empty);
            }
            return Path.Combine(parts.ToArray());
        });
        lua["__hexal_lsp_host"] = host;

        lua.DoString(Prelude);
        if (lua["__hexal_wrap_methods"] is LuaFunction wrap)
        {
            coordinator.WrapFunction = wrap;
        }
        if (lua["__hexal_pad"] is LuaFunction pad)
        {
            coordinator.PadFunction = pad;
        }

        foreach (LuaTable module in coordinator.Modules.Values)
        {
            coordinator.WrapTable(module);
        }
        coordinator.WrapTable(lsp);
        coordinator.WrapTable(root);

        coordinator.PadTable(lsp, "create_server", 1);
        coordinator.PadTable(protocol, "make_error", 3);
        coordinator.PadTable(serverModule, "create", 1);
        coordinator.PadTable(documentsModule, "create_virtual", 3);
        coordinator.PadTable(documentsModule, "get", 1);
        coordinator.PadTable(documentsModule, "get_open", 1);
        coordinator.PadTable(documentsModule, "snapshot", 1);
        coordinator.PadTable(workspaceModule, "configuration", 1);
        coordinator.PadTable(workspaceModule, "find_files", 2);
        coordinator.PadTable(workspaceModule, "read_file", 1);
        coordinator.PadTable(workspaceModule, "stat", 1);
        coordinator.PadTable(workspaceModule, "watch", 2);
        coordinator.PadTable(workspaceModule, "index", 1);
        coordinator.PadTable(workspaceModule, "progress", 2);
        coordinator.PadTable(capabilities, "supports_client", 1);
        coordinator.PadTable(capabilities, "register", 1);
        coordinator.PadTable(capabilities, "unregister", 1);
        coordinator.PadTable(diagnostics, "publish", 3);
        coordinator.PadTable(diagnostics, "set", 4);
        coordinator.PadTable(diagnostics, "clear", 2);
        coordinator.PadTable(diagnostics, "on_change", 1);
        coordinator.PadTable(completion, "item", 1);
        coordinator.PadTable(completion, "list", 1);
        coordinator.PadTable(completion, "resolve", 2);
        coordinator.PadTable(edit, "text_edit", 2);
        coordinator.PadTable(edit, "workspace_edit", 1);
        coordinator.PadTable(edit, "document_change", 3);
        coordinator.PadTable(edit, "apply_workspace_edit", 1);
        coordinator.PadTable(types, "position", 2);
        coordinator.PadTable(types, "range", 2);
        coordinator.PadTable(types, "location", 2);
        coordinator.PadTable(types, "text_edit", 2);
        coordinator.PadTable(types, "validate", 2);
        coordinator.PadTable(types, "validate_position", 1);
        coordinator.PadTable(types, "validate_range", 1);
        coordinator.PadTable(types, "validate_diagnostic", 1);
        coordinator.PadTable(uri, "join", 2);
        coordinator.PadTable(position, "compare", 2);
        coordinator.PadTable(position, "contains", 2);
        coordinator.PadTable(position, "intersects", 2);
        coordinator.PadTable(position, "validate", 2);
        coordinator.PadTable(position, "validate_range", 2);
        coordinator.PadTable(position, "offset_to_position", 2);
        coordinator.PadTable(position, "position_to_offset", 2);
        coordinator.PadTable(semanticTokens, "encode", 1);
        coordinator.PadTable(semanticTokens, "legend", 1);
        coordinator.PadTable(semanticTokens, "register", 1);
        coordinator.PadTable(inlayHint, "create", 1);
        coordinator.PadTable(inlayHint, "resolve", 2);
        coordinator.PadTable(signatureHelp, "create", 1);
        coordinator.PadTable(signatureHelp, "signature", 1);
        coordinator.PadTable(signatureHelp, "parameter", 1);
        coordinator.PadTable(foldingRange, "create", 1);
        coordinator.PadTable(codeLens, "create", 1);
        coordinator.PadTable(codeLens, "resolve", 2);
        coordinator.PadTable(jsonModule, "encode", 1);
        coordinator.PadTable(jsonModule, "decode", 1);
        coordinator.PadTable(jsonModule, "is_valid", 1);
        coordinator.PadTable(fsModule, "read_file", 1);
        coordinator.PadTable(fsModule, "read_file_async", 1);
        coordinator.PadTable(fsModule, "write_file", 2);
        coordinator.PadTable(fsModule, "write_file_async", 2);
        coordinator.PadTable(fsModule, "exists", 1);
        coordinator.PadTable(fsModule, "stat", 1);
        coordinator.PadTable(fsModule, "stat_async", 1);
        coordinator.PadTable(fsModule, "glob", 2);
        coordinator.PadTable(fsModule, "mkdir", 2);
        coordinator.PadTable(fsModule, "remove", 1);
        coordinator.PadTable(fsModule, "rename", 2);
        coordinator.PadTable(logModule, "trace", 2);
        coordinator.PadTable(logModule, "debug", 2);
        coordinator.PadTable(logModule, "info", 2);
        coordinator.PadTable(logModule, "warn", 2);
        coordinator.PadTable(logModule, "error", 2);
        coordinator.PadTable(logModule, "set_level", 1);
        coordinator.PadTable(logModule, "set_destination", 1);
        coordinator.PadTable(indexModule, "create", 1);
        coordinator.PadTable(processModule, "run", 3);
        coordinator.PadTable(debugModule, "registered_handlers", 1);
        coordinator.PadTable(debugModule, "set_trace", 1);
        coordinator.PadTable(testingLsp, "create_client", 1);

        var loader = new StringBuilder();
        foreach (string name in coordinator.Modules.Keys.OrderBy(name => name))
        {
            loader.AppendLine($"package.preload[{LuaString(name)}] = function() return {name} end");
        }
        loader.AppendLine("package.loaded['hexal'] = hexal");
        lua.DoString(loader.ToString());
    }

    private static string LuaString(string value)
    {
        return "'" + value.Replace("\\", "\\\\").Replace("'", "\\'") + "'";
    }

    private static LuaTable GetOrCreate(LuaLspCoordinator coordinator, LuaTable root, string name)
    {
        if (root[name] is LuaTable table)
        {
            return table;
        }
        LuaTable created = coordinator.NewTable();
        root[name] = created;
        return created;
    }

    private static LuaTable AddModule(
        LuaLspCoordinator coordinator,
        LuaTable parent,
        string shortName,
        string fullName
    )
    {
        LuaTable table = coordinator.NewTable();
        parent[shortName] = table;
        coordinator.Modules[fullName] = table;
        return table;
    }

    private static LuaTable? ToTable(LuaLspCoordinator coordinator, JsonNode? node)
    {
        return coordinator.ToLua(node) as LuaTable;
    }

    private static LuaTable? ObjectTable(LuaLspCoordinator coordinator, JsonObject obj)
    {
        return coordinator.ToLua(obj) as LuaTable;
    }

    private static JsonObject Normalize(
        JsonObject source,
        params (string From, string To)[] mappings
    )
    {
        var result = new JsonObject();
        foreach ((string key, JsonNode? value) in source)
        {
            result[key] = value?.DeepClone();
        }

        foreach ((string from, string to) in mappings)
        {
            if (string.Equals(from, to, StringComparison.Ordinal))
            {
                continue;
            }
            if (result[from] is JsonNode value)
            {
                if (result[to] is null)
                {
                    result[to] = value.DeepClone();
                }
                result.Remove(from);
            }
        }
        return result;
    }

    // -----------------------------------------------------------------------
    // protocol / server / documents / workspace / capabilities
    // -----------------------------------------------------------------------

    private static void BindProtocol(LuaLspCoordinator coordinator, LuaTable module)
    {
        LuaTable errors = coordinator.NewTable();
        errors["PARSE_ERROR"] = LspErrorCodes.ParseError;
        errors["INVALID_REQUEST"] = LspErrorCodes.InvalidRequest;
        errors["METHOD_NOT_FOUND"] = LspErrorCodes.MethodNotFound;
        errors["INVALID_PARAMS"] = LspErrorCodes.InvalidParams;
        errors["INTERNAL_ERROR"] = LspErrorCodes.InternalError;
        errors["SERVER_NOT_INITIALIZED"] = LspErrorCodes.ServerNotInitialized;
        errors["REQUEST_CANCELLED"] = LspErrorCodes.RequestCancelled;
        errors["CONTENT_MODIFIED"] = LspErrorCodes.ContentModified;
        errors["REQUEST_FAILED"] = LspErrorCodes.RequestFailed;
        errors["SERVER_CANCELLED"] = LspErrorCodes.ServerCancelled;
        module["error_codes"] = coordinator.WrapTable(errors);
        module["make_error"] = new Func<int, string, object?, LuaTable>(
            (code, message, data) =>
                coordinator.CreateErrorTable(
                    code,
                    message ?? string.Empty,
                    data is LuaTable table ? coordinator.FromLua(table) : coordinator.FromLua(data)
                )
        );
        module["is_error"] = new Func<object?, bool>(LuaLspCoordinator.IsErrorTable);
    }

    private static void BindServerModule(
        LuaLspCoordinator coordinator,
        LuaTable lsp,
        LuaTable module
    )
    {
        module["create"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    coordinator.Report("lsp.create_server expects an options table");
                    return null;
                }
                LuaLspServer? server = LuaLspServer.Create(coordinator, table);
                return server?.ServerTable;
            }
        );
        lsp["create_server"] = module["create"];
    }

    private static void BindDocuments(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["get"] = new Func<string, object?>(uri =>
        {
            LspDocument? document = coordinator.Documents.Get(uri);
            return document is null ? null : coordinator.CreateDocumentTable(document);
        });
        module["get_open"] = new Func<string, object?>(uri =>
        {
            LspDocument? document = coordinator.Documents.GetOpen(uri);
            return document is null ? null : coordinator.CreateDocumentTable(document);
        });
        module["is_open"] = new Func<string, bool>(coordinator.Documents.IsOpen);
        module["snapshot"] = new Func<string, object?>(uri =>
        {
            LspSnapshot? snapshot = coordinator.Documents.Snapshot(uri);
            return snapshot is null ? null : coordinator.CreateSnapshotTable(snapshot);
        });
        module["create_virtual"] = new Func<string, string, object?, LuaTable?>(
            (uri, text, languageId) =>
            {
                if (string.IsNullOrEmpty(uri))
                {
                    return null;
                }
                LspDocument document = coordinator.Documents.CreateVirtual(
                    uri,
                    languageId?.ToString() ?? "plaintext",
                    (coordinator.Documents.Get(uri)?.Version ?? 0) + 1,
                    text ?? string.Empty
                );
                return coordinator.CreateDocumentTable(document);
            }
        );
        module["set_virtual_text"] = new Func<string, string, bool>(
            (uri, text) =>
            {
                LspDocument? document = coordinator.Documents.Get(uri);
                if (document is null || !document.IsVirtual)
                {
                    return false;
                }
                document.SetText(document.Version + 1, text ?? string.Empty);
                return true;
            }
        );
        module["delete_virtual"] = new Func<string, bool>(uri =>
        {
            LspDocument? document = coordinator.Documents.Get(uri);
            if (document is null || !document.IsVirtual)
            {
                return false;
            }
            coordinator.Diagnostics.Clear(uri);
            return coordinator.Documents.Remove(uri);
        });
        module["close"] = new Func<string, bool>(uri =>
        {
            coordinator.Diagnostics.Clear(uri);
            return coordinator.Documents.Close(uri);
        });
    }

    private static void BindWorkspace(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["current"] = new Func<LuaTable?>(() => coordinator.ActiveServer?.WorkspaceTable);
        module["root_uri"] = new Func<string>(() =>
            coordinator.ActiveServer?.Workspace.RootUri ?? coordinator.Workspace.RootUri
        );
        module["folders"] = new Func<LuaTable>(() =>
        {
            LspWorkspace workspace = coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
            var values = new List<object?>();
            foreach ((string folderUri, string folderName) in workspace.Folders)
            {
                LuaTable folder = coordinator.NewTable();
                folder["uri"] = folderUri;
                folder["name"] = folderName;
                values.Add(folder);
            }
            return coordinator.NewArray(values);
        });
        module["configuration"] = new Func<string?, object?>(section =>
            coordinator.ToLua(
                (coordinator.ActiveServer?.Workspace ?? coordinator.Workspace).ConfigurationValue(
                    section
                )
            )
        );
        module["find_files"] = new Func<string, object?, LuaTable>(
            (glob, options) =>
            {
                LspWorkspace workspace =
                    coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
                var task = new LspTaskResult();
                try
                {
                    task.Complete(
                        coordinator.NewArray(
                            workspace
                                .FindFiles(
                                    glob,
                                    options is LuaTable table ? coordinator.AsObject(table) : null
                                )
                                .Cast<object?>()
                        )
                    );
                }
                catch (Exception ex)
                {
                    task.Fail(ex.Message);
                }
                return coordinator.CreateTask(task);
            }
        );
        module["read_file"] = new Func<string, LuaTable>(uriValue =>
        {
            LspWorkspace workspace = coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
            var task = new LspTaskResult();
            try
            {
                LspDocument? document = coordinator.Documents.Get(uriValue);
                task.Complete(
                    document is { IsOpen: true } ? document.Text : workspace.ReadText(uriValue)
                );
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return coordinator.CreateTask(task);
        });
        module["stat"] = new Func<string, LuaTable>(uriValue =>
        {
            LspWorkspace workspace = coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
            var task = new LspTaskResult();
            try
            {
                task.Complete(coordinator.ToLua(workspace.Stat(uriValue)));
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return coordinator.CreateTask(task);
        });
        module["watch"] = new Func<string, LuaFunction, object?>(
            (glob, callback) =>
            {
                if (callback is null)
                {
                    return null;
                }
                LspWorkspace workspace =
                    coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
                LspWatchRegistration watch = workspace.Watch(glob);
                return coordinator.CreateWatch(watch, callback);
            }
        );
        module["index"] = new Func<object?, LuaTable>(options =>
        {
            LspWorkspace workspace = coordinator.ActiveServer?.Workspace ?? coordinator.Workspace;
            var task = new LspTaskResult();
            task.Complete(
                coordinator.CreateIndexTable(
                    LspWorkspace.CreateIndex(
                        options is LuaTable table ? coordinator.AsObject(table) : null
                    )
                )
            );
            return coordinator.CreateTask(task);
        });
        module["progress"] = new Func<string, LuaFunction?, LuaTable>(
            (title, callback) =>
            {
                LuaLspServer? server = coordinator.ActiveServer;
                var task = new LspTaskResult();
                string token =
                    server?.NextProgressToken()
                    ?? $"hexal-progress-{coordinator.PluginId}-{Guid.NewGuid():N}";
                LuaTable progress = coordinator.CreateProgress(server, token);
                server?.BeginProgress(token, title, null);
                if (callback is not null)
                {
                    try
                    {
                        callback.Call(progress);
                    }
                    catch (Exception ex)
                        when (ex is LuaScriptException or InvalidOperationException)
                    {
                        task.Fail(ex.Message);
                        return coordinator.CreateTask(task);
                    }
                }
                task.Complete(progress);
                return coordinator.CreateTask(task);
            }
        );
    }

    private static void BindCapabilities(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["client"] = new Func<LuaTable>(() =>
            coordinator.ToLua(coordinator.ActiveServer?.ClientCapabilities ?? new JsonObject())
                as LuaTable
            ?? coordinator.NewTable()
        );
        module["server"] = new Func<LuaTable>(() =>
            coordinator.ToLua(coordinator.ActiveServer?.ServerCapabilities ?? new JsonObject())
                as LuaTable
            ?? coordinator.NewTable()
        );
        module["supports_client"] = new Func<string, bool>(path =>
            coordinator.ActiveServer?.ClientSupports(path) ?? false
        );
        module["register"] = new Func<object?, LuaTable?>(options =>
        {
            LuaLspServer? server = coordinator.ActiveServer;
            if (server is null)
            {
                return null;
            }
            LuaTable client = server.CreateClientTable();
            return client["register_capability"] is LuaFunction function
                ? function.Call(options)[0] as LuaTable
                : null;
        });
        module["unregister"] = new Func<object?, LuaTable?>(options =>
        {
            LuaLspServer? server = coordinator.ActiveServer;
            if (server is null)
            {
                return null;
            }
            LuaTable client = server.CreateClientTable();
            return client["unregister_capability"] is LuaFunction function
                ? function.Call(options)[0] as LuaTable
                : null;
        });
    }

    // -----------------------------------------------------------------------
    // diagnostics / completion / symbols / edit / types
    // -----------------------------------------------------------------------

    private static void BindDiagnostics(LuaLspCoordinator coordinator, LuaTable module)
    {
        LuaTable severity = coordinator.NewTable();
        severity["ERROR"] = 1;
        severity["WARNING"] = 2;
        severity["INFORMATION"] = 3;
        severity["HINT"] = 4;
        module["severity"] = coordinator.WrapTable(severity);

        LuaTable tags = coordinator.NewTable();
        tags["UNNECESSARY"] = 1;
        tags["DEPRECATED"] = 2;
        module["tags"] = coordinator.WrapTable(tags);

        module["publish"] = new Func<string, object?, object?, bool>(
            (uri, items, options) =>
            {
                JsonArray array = coordinator.AsArray(items);
                long? version = options is LuaTable table
                    ? LspJson.ToInt64(table["version"])
                    : null;
                coordinator.Diagnostics.Publish(uri, array, version);
                return true;
            }
        );
        module["set"] = new Func<string, string, object?, object?, bool>(
            (uri, name, items, options) =>
            {
                JsonArray array = coordinator.AsArray(items);
                long? version = options is LuaTable table
                    ? LspJson.ToInt64(table["version"])
                    : null;
                return coordinator.Diagnostics.Set(uri, name ?? "default", array, version);
            }
        );
        module["get"] = new Func<string, LuaTable>(uri =>
            ToTable(coordinator, coordinator.Diagnostics.Get(uri)) ?? coordinator.NewTable()
        );
        module["clear"] = new Func<string, object?, bool>(
            (uri, name) =>
            {
                string? namespaceName = name?.ToString();
                return string.IsNullOrEmpty(namespaceName)
                    ? coordinator.Diagnostics.Clear(uri)
                    : coordinator.Diagnostics.Clear(uri, namespaceName);
            }
        );
        module["clear_all"] = new Func<bool>(() =>
        {
            coordinator.Diagnostics.ClearAll();
            return true;
        });
        module["on_change"] = new Func<LuaFunction, long>(callback =>
            callback is null ? 0 : coordinator.AddDiagnosticsHandler(callback)
        );
        module["off_change"] = new Func<long, bool>(coordinator.RemoveDiagnosticsHandler);
    }

    private static void BindCompletion(LuaLspCoordinator coordinator, LuaTable module)
    {
        LuaTable kinds = coordinator.NewTable();
        string[] names =
        {
            "TEXT",
            "METHOD",
            "FUNCTION",
            "CONSTRUCTOR",
            "FIELD",
            "VARIABLE",
            "CLASS",
            "INTERFACE",
            "MODULE",
            "PROPERTY",
            "UNIT",
            "VALUE",
            "ENUM",
            "KEYWORD",
            "SNIPPET",
            "COLOR",
            "FILE",
            "REFERENCE",
            "FOLDER",
            "ENUM_MEMBER",
            "CONSTANT",
            "STRUCT",
            "EVENT",
            "OPERATOR",
            "TYPE_PARAMETER",
        };
        for (int index = 0; index < names.Length; ++index)
        {
            kinds[names[index]] = index + 1;
            kinds[names[index].Replace("_", string.Empty, StringComparison.Ordinal)] = index + 1;
            kinds[
                names[index].Replace("_", string.Empty, StringComparison.Ordinal).ToLowerInvariant()
            ] = index + 1;
        }
        module["kind"] = coordinator.WrapTable(kinds);

        LuaTable formats = coordinator.NewTable();
        formats["PLAIN_TEXT"] = 1;
        formats["SNIPPET"] = 2;
        module["insert_text_format"] = coordinator.WrapTable(formats);

        LuaTable triggers = coordinator.NewTable();
        triggers["INVOKED"] = 1;
        triggers["TRIGGER_CHARACTER"] = 2;
        triggers["TRIGGER_FOR_INCOMPLETE_COMPLETIONS"] = 3;
        module["trigger_kind"] = coordinator.WrapTable(triggers);

        module["item"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                JsonObject item = Normalize(
                    source,
                    ("label_details", "labelDetails"),
                    ("sort_text", "sortText"),
                    ("filter_text", "filterText"),
                    ("insert_text", "insertText"),
                    ("insert_text_format", "insertTextFormat"),
                    ("insert_text_mode", "insertTextMode"),
                    ("text_edit", "textEdit"),
                    ("text_edit_text", "textEditText"),
                    ("additional_text_edits", "additionalTextEdits"),
                    ("commit_characters", "commitCharacters")
                );
                if (
                    item["kind"] is JsonValue kindValue
                    && kindValue.TryGetValue(out string? kindName)
                )
                {
                    int? number = KindNumber(kinds, kindName);
                    if (number is not null)
                    {
                        item["kind"] = number.Value;
                    }
                }
                return ToTable(coordinator, item);
            }
        );

        module["list"] = new Func<object?, LuaTable>(items =>
        {
            LuaTable result = coordinator.NewTable();
            result["isIncomplete"] = false;
            result["items"] = items is LuaTable ? items : coordinator.NewTable();
            return coordinator.WrapTable(result);
        });

        module["resolve"] = new Func<object?, LuaFunction?, object?>(
            (item, resolver) =>
            {
                if (resolver is null)
                {
                    return item;
                }
                object[] result = resolver.Call(item);
                return result.Length > 0 ? result[0] : item;
            }
        );
    }

    private static int? KindNumber(LuaTable kinds, string name)
    {
        object? value =
            kinds[name] ?? kinds[name.ToUpperInvariant()] ?? kinds[name.ToLowerInvariant()];
        return LspJson.ToInt32(value);
    }

    private static void BindSymbols(LuaLspCoordinator coordinator, LuaTable module)
    {
        LuaTable kinds = coordinator.NewTable();
        string[] names =
        {
            "FILE",
            "MODULE",
            "NAMESPACE",
            "PACKAGE",
            "CLASS",
            "METHOD",
            "PROPERTY",
            "FIELD",
            "CONSTRUCTOR",
            "ENUM",
            "INTERFACE",
            "FUNCTION",
            "VARIABLE",
            "CONSTANT",
            "STRING",
            "NUMBER",
            "BOOLEAN",
            "ARRAY",
            "OBJECT",
            "KEY",
            "NULL",
            "ENUM_MEMBER",
            "STRUCT",
            "EVENT",
            "OPERATOR",
            "TYPE_PARAMETER",
        };
        for (int index = 0; index < names.Length; ++index)
        {
            kinds[names[index]] = index + 1;
            kinds[names[index].ToLowerInvariant()] = index + 1;
        }
        module["kind"] = coordinator.WrapTable(kinds);

        LuaTable tags = coordinator.NewTable();
        tags["DEPRECATED"] = 1;
        module["tag"] = coordinator.WrapTable(tags);

        module["create"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(coordinator, coordinator.AsObject(table) ?? new JsonObject())
                    : null
        );

        module["reference"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                return ToTable(
                    coordinator,
                    Normalize(
                        source,
                        ("target_uri", "targetUri"),
                        ("target_range", "targetRange"),
                        ("target_selection_range", "targetSelectionRange"),
                        ("origin_selection_range", "originSelectionRange"),
                        ("container_name", "containerName"),
                        ("selection_range", "selectionRange")
                    )
                );
            }
        );

        module["location"] = new Func<string, object?, LuaTable?>(
            (uriValue, range) =>
            {
                var location = new JsonObject
                {
                    ["uri"] = uriValue,
                    ["range"] = coordinator.FromLua(range),
                };
                return ToTable(coordinator, location);
            }
        );

        module["location_link"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                return ToTable(
                    coordinator,
                    Normalize(
                        source,
                        ("target_uri", "targetUri"),
                        ("target_range", "targetRange"),
                        ("target_selection_range", "targetSelectionRange"),
                        ("origin_selection_range", "originSelectionRange")
                    )
                );
            }
        );

        module["document_symbol"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                return ToTable(
                    coordinator,
                    Normalize(
                        source,
                        ("selection_range", "selectionRange"),
                        ("container_name", "containerName"),
                        ("detail", "detail"),
                        ("children", "children")
                    )
                );
            }
        );

        module["workspace_symbol"] = new Func<object?, object?>(
            (options) =>
            {
                if (options is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                return ToTable(
                    coordinator,
                    Normalize(source, ("container_name", "containerName"), ("location", "location"))
                );
            }
        );
    }

    private static void BindEdit(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["text_edit"] = new Func<object?, string, LuaTable?>(
            (range, newText) =>
            {
                var edit = new JsonObject
                {
                    ["range"] = coordinator.FromLua(range),
                    ["newText"] = newText ?? string.Empty,
                };
                return ToTable(coordinator, edit);
            }
        );

        module["workspace_edit"] = new Func<object?, LuaTable?>(
            (changes) =>
            {
                if (changes is not LuaTable table)
                {
                    return null;
                }
                JsonObject source = coordinator.AsObject(table) ?? new JsonObject();
                JsonObject edit;
                if (source["changes"] is not null || source["documentChanges"] is not null)
                {
                    edit = source;
                }
                else
                {
                    edit = new JsonObject { ["changes"] = source.DeepClone() };
                }
                return ToTable(coordinator, edit);
            }
        );

        module["document_change"] = new Func<string, object?, object?, LuaTable?>(
            (uriValue, version, edits) =>
            {
                var change = new JsonObject
                {
                    ["textDocument"] = new JsonObject
                    {
                        ["uri"] = uriValue,
                        ["version"] = version is null ? null : coordinator.FromLua(version),
                    },
                    ["edits"] = coordinator.AsArray(edits),
                };
                return ToTable(coordinator, change);
            }
        );

        module["apply_workspace_edit"] = new Func<object?, LuaTable>(edit =>
            ApplyWorkspaceEdit(coordinator, edit)
        );
        module["apply_workspace_edit_ex"] = module["apply_workspace_edit"];
    }

    private static LuaTable ApplyWorkspaceEdit(LuaLspCoordinator coordinator, object? editValue)
    {
        LuaTable result = coordinator.NewTable();
        result["ok"] = false;
        result["error"] = string.Empty;

        JsonObject? edit = coordinator.AsObject(editValue);
        if (edit is null)
        {
            result["error"] = "workspace edit must be a table";
            return coordinator.WrapTable(result);
        }

        var targets = new Dictionary<string, JsonArray>(StringComparer.Ordinal);
        if (edit["changes"] is JsonObject changes)
        {
            foreach ((string uriValue, JsonNode? value) in changes)
            {
                if (value is JsonArray array)
                {
                    targets[uriValue] = array;
                }
            }
        }
        if (edit["documentChanges"] is JsonArray documentChanges)
        {
            foreach (JsonNode? node in documentChanges)
            {
                if (node is not JsonObject change)
                {
                    continue;
                }
                if (LspJson.GetString(change["kind"]) is string kind)
                {
                    if (
                        !ApplyResourceOperation(coordinator, kind, change, out string resourceError)
                    )
                    {
                        result["error"] = resourceError;
                        return coordinator.WrapTable(result);
                    }
                    continue;
                }
                string uriValue = LspJson.GetString(change["textDocument"]?["uri"]) ?? string.Empty;
                if (change["edits"] is JsonArray edits && uriValue.Length > 0)
                {
                    if (!targets.TryGetValue(uriValue, out JsonArray? existing))
                    {
                        existing = new JsonArray();
                        targets[uriValue] = existing;
                    }
                    foreach (JsonNode? item in edits)
                    {
                        existing.Add(item?.DeepClone());
                    }
                }
            }
        }

        foreach ((string uriValue, JsonArray edits) in targets)
        {
            LspDocument? document = coordinator.Documents.Get(uriValue);
            if (document is not null)
            {
                if (!document.ApplyTextEdits(edits, null, out string editError))
                {
                    result["error"] = editError;
                    return coordinator.WrapTable(result);
                }
                continue;
            }

            string path = LspUri.ToPath(uriValue);
            try
            {
                string text = File.ReadAllText(path, Encoding.UTF8);
                var pseudo = new LspDocument(
                    uriValue,
                    string.Empty,
                    coordinator.Documents.Get(uriValue)?.Version ?? 0,
                    text,
                    coordinator.Documents.Encoding
                );
                if (!pseudo.ApplyTextEdits(edits, null, out string editError))
                {
                    result["error"] = editError;
                    return coordinator.WrapTable(result);
                }
                coordinator.Workspace.WriteText(path, pseudo.Text);
            }
            catch (Exception ex)
                when (ex
                        is IOException
                            or UnauthorizedAccessException
                            or NotSupportedException
                            or ArgumentException
                )
            {
                result["error"] = ex.Message;
                return coordinator.WrapTable(result);
            }
        }

        result["ok"] = true;
        return coordinator.WrapTable(result);
    }

    private static bool ApplyResourceOperation(
        LuaLspCoordinator coordinator,
        string kind,
        JsonObject change,
        out string error
    )
    {
        error = string.Empty;
        try
        {
            switch (kind)
            {
                case "create":
                {
                    string uriValue = LspJson.GetString(change["uri"]) ?? string.Empty;
                    string path = LspUri.ToPath(uriValue);
                    coordinator.Permissions.AuthorizeWrite(path);
                    if (LspJson.GetBool(change["options"]?["overwrite"]) == true)
                    {
                        File.WriteAllText(path, string.Empty, Encoding.UTF8);
                    }
                    else if (!File.Exists(path))
                    {
                        File.WriteAllText(path, string.Empty, Encoding.UTF8);
                    }
                    return true;
                }
                case "rename":
                {
                    string oldUri = LspJson.GetString(change["oldUri"]) ?? string.Empty;
                    string newUri = LspJson.GetString(change["newUri"]) ?? string.Empty;
                    string oldPath = LspUri.ToPath(oldUri);
                    string newPath = LspUri.ToPath(newUri);
                    coordinator.Permissions.AuthorizeWrite(oldPath);
                    coordinator.Permissions.AuthorizeWrite(newPath);
                    File.Move(
                        oldPath,
                        newPath,
                        LspJson.GetBool(change["options"]?["overwrite"]) == true
                    );
                    return true;
                }
                case "delete":
                {
                    string uriValue = LspJson.GetString(change["uri"]) ?? string.Empty;
                    string path = LspUri.ToPath(uriValue);
                    coordinator.Permissions.AuthorizeWrite(path);
                    if (Directory.Exists(path))
                    {
                        Directory.Delete(path, recursive: true);
                    }
                    else
                    {
                        File.Delete(path);
                    }
                    return true;
                }
                default:
                    error = $"unsupported resource operation '{kind}'";
                    return false;
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
            error = ex.Message;
            return false;
        }
    }

    private static void BindTypes(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["position"] = new Func<object?, object?, LuaTable?>(
            (line, character) =>
            {
                var position = new JsonObject
                {
                    ["line"] = LspJson.ToInt64(line) ?? 0,
                    ["character"] = LspJson.ToInt64(character) ?? 0,
                };
                return ToTable(coordinator, position);
            }
        );
        module["range"] = new Func<object?, object?, LuaTable?>(
            (start, end) =>
            {
                var range = new JsonObject
                {
                    ["start"] = coordinator.FromLua(start),
                    ["end"] = coordinator.FromLua(end),
                };
                return ToTable(coordinator, range);
            }
        );
        module["location"] = new Func<string, object?, LuaTable?>(
            (uriValue, range) =>
            {
                var location = new JsonObject
                {
                    ["uri"] = uriValue,
                    ["range"] = coordinator.FromLua(range),
                };
                return ToTable(coordinator, location);
            }
        );
        module["text_edit"] = new Func<object?, string, LuaTable?>(
            (range, newText) =>
            {
                var edit = new JsonObject
                {
                    ["range"] = coordinator.FromLua(range),
                    ["newText"] = newText ?? string.Empty,
                };
                return ToTable(coordinator, edit);
            }
        );
        module["diagnostic"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("code_description", "codeDescription"),
                            ("related_information", "relatedInformation")
                        )
                    )
                    : null
        );
        module["completion_item"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("label_details", "labelDetails"),
                            ("sort_text", "sortText"),
                            ("filter_text", "filterText"),
                            ("insert_text", "insertText"),
                            ("insert_text_format", "insertTextFormat"),
                            ("insert_text_mode", "insertTextMode"),
                            ("text_edit", "textEdit"),
                            ("additional_text_edits", "additionalTextEdits"),
                            ("commit_characters", "commitCharacters")
                        )
                    )
                    : null
        );
        module["workspace_edit"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("document_changes", "documentChanges")
                        )
                    )
                    : null
        );
        module["document_symbol"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("selection_range", "selectionRange"),
                            ("container_name", "containerName")
                        )
                    )
                    : null
        );
        module["workspace_symbol"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("container_name", "containerName")
                        )
                    )
                    : null
        );
        module["inlay_hint"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("padding_left", "paddingLeft"),
                            ("padding_right", "paddingRight"),
                            ("text_edits", "textEdits")
                        )
                    )
                    : null
        );
        module["signature_information"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("active_parameter", "activeParameter")
                        )
                    )
                    : null
        );

        module["validate"] = new Func<string, object?, bool>(
            (method, value) => ValidateMethod(method, coordinator.FromLua(value))
        );
        module["validate_position"] = new Func<object?, bool>(
            (value) => ValidatePosition(coordinator.FromLua(value))
        );
        module["validate_range"] = new Func<object?, bool>(
            (value) => ValidateRange(coordinator.FromLua(value))
        );
        module["validate_diagnostic"] = new Func<object?, bool>(
            (value) => ValidateDiagnostic(coordinator.FromLua(value))
        );
    }

    private static bool ValidateMethod(string method, JsonNode? value)
    {
        if (value is null)
        {
            return true;
        }
        if (
            method
            is "textDocument/hover"
                or "textDocument/definition"
                or "textDocument/declaration"
                or "textDocument/typeDefinition"
                or "textDocument/implementation"
                or "textDocument/references"
                or "textDocument/completion"
                or "textDocument/codeAction"
                or "textDocument/documentSymbol"
                or "workspace/symbol"
                or "textDocument/rename"
                or "textDocument/formatting"
                or "textDocument/rangeFormatting"
                or "textDocument/semanticTokens/full"
                or "textDocument/semanticTokens/full/delta"
                or "textDocument/semanticTokens/range"
                or "textDocument/inlayHint"
                or "textDocument/signatureHelp"
                or "textDocument/foldingRange"
                or "textDocument/codeLens"
                or "textDocument/documentLink"
                or "textDocument/selectionRange"
        )
        {
            return true;
        }
        return true;
    }

    private static bool ValidatePosition(JsonNode? value)
    {
        if (value is not JsonObject position)
        {
            return false;
        }
        return position["line"] is JsonValue line
            && line.TryGetValue(out int lineNumber)
            && lineNumber >= 0
            && position["character"] is JsonValue character
            && character.TryGetValue(out int characterNumber)
            && characterNumber >= 0;
    }

    private static bool ValidateRange(JsonNode? value)
    {
        if (value is not JsonObject range)
        {
            return false;
        }
        return ValidatePosition(range["start"]) && ValidatePosition(range["end"]);
    }

    private static bool ValidateDiagnostic(JsonNode? value)
    {
        if (value is not JsonObject diagnostic)
        {
            return false;
        }
        return ValidateRange(diagnostic["range"]) && diagnostic["message"] is JsonValue;
    }

    // -----------------------------------------------------------------------
    // uri / position / semantic tokens
    // -----------------------------------------------------------------------

    private static void BindUri(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["from_path"] = new Func<string, string>(path => LspUri.FromPath(path));
        module["to_path"] = new Func<string, string>(uriValue => LspUri.ToPath(uriValue));
        module["normalize"] = new Func<string, string>(uriValue => LspUri.Normalize(uriValue));
        module["is_file"] = new Func<string, bool>(uriValue => LspUri.IsFile(uriValue));
        module["parse"] = new Func<string, LuaTable?>(uriValue =>
            ToTable(coordinator, LspUri.Parse(uriValue))
        );
        module["join"] = new Func<string, string, string>(Path.Combine);
        module["encode"] = new Func<string, string>(LspUri.EncodePercent);
        module["decode"] = new Func<string, string>(LspUri.DecodePercent);
    }

    private static void BindPosition(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["compare"] = new Func<object?, object?, int>(
            (left, right) =>
            {
                (int leftLine, int leftCharacter) = ReadPosition(coordinator, left);
                (int rightLine, int rightCharacter) = ReadPosition(coordinator, right);
                return leftLine != rightLine
                    ? leftLine.CompareTo(rightLine)
                    : leftCharacter.CompareTo(rightCharacter);
            }
        );
        module["contains"] = new Func<object?, object?, bool>(
            (range, position) =>
            {
                (int startLine, int startCharacter, int endLine, int endCharacter) = ReadRange(
                    coordinator,
                    range
                );
                (int line, int character) = ReadPosition(coordinator, position);
                return Compare(line, character, startLine, startCharacter) >= 0
                    && Compare(line, character, endLine, endCharacter) < 0;
            }
        );
        module["intersects"] = new Func<object?, object?, bool>(
            (left, right) =>
            {
                (int aStartLine, int aStartCharacter, int aEndLine, int aEndCharacter) = ReadRange(
                    coordinator,
                    left
                );
                (int bStartLine, int bStartCharacter, int bEndLine, int bEndCharacter) = ReadRange(
                    coordinator,
                    right
                );
                return !(
                    Compare(aEndLine, aEndCharacter, bStartLine, bStartCharacter) <= 0
                    || Compare(bEndLine, bEndCharacter, aStartLine, aStartCharacter) <= 0
                );
            }
        );
        module["validate"] = new Func<object?, object?, bool>(
            (position, document) =>
            {
                (int line, int character) = ReadPosition(coordinator, position);
                if (line < 0 || character < 0)
                {
                    return false;
                }
                if (document is LuaTable table)
                {
                    string text = table["text"]?.ToString() ?? string.Empty;
                    List<int> starts = LspTextIndex.BuildLineStarts(text);
                    return line < starts.Count
                        && character <= LspTextIndex.LineText(text, starts, line).Length;
                }
                return true;
            }
        );
        module["validate_range"] = new Func<object?, object?, bool>(
            (range, document) =>
            {
                (int startLine, int startCharacter, int endLine, int endCharacter) = ReadRange(
                    coordinator,
                    range
                );
                if (
                    startLine < 0
                    || endLine < 0
                    || Compare(startLine, startCharacter, endLine, endCharacter) > 0
                )
                {
                    return false;
                }
                if (document is LuaTable table)
                {
                    string text = table["text"]?.ToString() ?? string.Empty;
                    List<int> starts = LspTextIndex.BuildLineStarts(text);
                    if (startLine >= starts.Count || endLine >= starts.Count)
                    {
                        return false;
                    }
                    return startCharacter <= LspTextIndex.LineText(text, starts, startLine).Length
                        && endCharacter <= LspTextIndex.LineText(text, starts, endLine).Length;
                }
                return true;
            }
        );
        module["offset_to_position"] = new Func<object?, object?, LuaTable?>(
            (document, offset) =>
            {
                if (document is not LuaTable table)
                {
                    return null;
                }
                string text = table["text"]?.ToString() ?? string.Empty;
                LspPositionEncoding encoding = LspEncodingUtil.Parse(table["encoding"]?.ToString());
                byte[] bytes = Encoding.UTF8.GetBytes(text);
                int byteOffset = (int)Math.Clamp(LspJson.ToInt64(offset) ?? 0, 0, bytes.Length);
                while (
                    byteOffset > 0
                    && byteOffset < bytes.Length
                    && (bytes[byteOffset] & 0xC0) == 0x80
                )
                {
                    --byteOffset;
                }
                int charOffset = Encoding.UTF8.GetCharCount(bytes, 0, byteOffset);
                List<int> starts = LspTextIndex.BuildLineStarts(text);
                (int line, int character) = LspTextIndex.CharOffsetToPosition(
                    text,
                    starts,
                    charOffset,
                    encoding
                );
                var position = new JsonObject { ["line"] = line, ["character"] = character };
                return ToTable(coordinator, position);
            }
        );
        module["position_to_offset"] = new Func<object?, object?, long>(
            (document, position) =>
            {
                if (document is not LuaTable table)
                {
                    return 0;
                }
                string text = table["text"]?.ToString() ?? string.Empty;
                LspPositionEncoding encoding = LspEncodingUtil.Parse(table["encoding"]?.ToString());
                (int line, int character) = ReadPosition(coordinator, position);
                List<int> starts = LspTextIndex.BuildLineStarts(text);
                int charOffset = LspTextIndex.PositionToCharOffset(
                    text,
                    starts,
                    line,
                    character,
                    encoding
                );
                return Encoding.UTF8.GetByteCount(text.AsSpan(0, charOffset));
            }
        );
    }

    private static object? coalesce(
        LuaLspCoordinator coordinator,
        object? range,
        int line,
        int character
    )
    {
        if (range is LuaTable)
        {
            return range;
        }
        LuaTable table = coordinator.NewTable();
        table["line"] = line;
        table["character"] = character;
        return table;
    }

    private static (int Line, int Character) ReadPosition(
        LuaLspCoordinator coordinator,
        object? value
    )
    {
        JsonNode? node = coordinator.FromLua(value);
        if (node is not JsonObject position)
        {
            return (0, 0);
        }
        return (
            LspJson.GetInt32(position["line"]) ?? 0,
            LspJson.GetInt32(position["character"]) ?? 0
        );
    }

    private static (int StartLine, int StartCharacter, int EndLine, int EndCharacter) ReadRange(
        LuaLspCoordinator coordinator,
        object? value
    )
    {
        JsonNode? node = coordinator.FromLua(value);
        if (node is not JsonObject range)
        {
            return (0, 0, 0, 0);
        }
        (int startLine, int startCharacter) = ReadPositionNode(range["start"]);
        (int endLine, int endCharacter) = ReadPositionNode(range["end"]);
        return (startLine, startCharacter, endLine, endCharacter);
    }

    private static (int Line, int Character) ReadPositionNode(JsonNode? value)
    {
        return value is JsonObject position
            ? (
                LspJson.GetInt32(position["line"]) ?? 0,
                LspJson.GetInt32(position["character"]) ?? 0
            )
            : (0, 0);
    }

    private static int Compare(int leftLine, int leftCharacter, int rightLine, int rightCharacter)
    {
        return leftLine != rightLine
            ? leftLine.CompareTo(rightLine)
            : leftCharacter.CompareTo(rightCharacter);
    }

    private static void BindSemanticTokens(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["encode"] = new Func<object?, LuaTable>(items =>
        {
            JsonArray source = coordinator.AsArray(items);
            return LspSemanticTokens.Encode(
                source,
                coordinator.SemanticLegend,
                out JsonArray data,
                out string error
            )
                ? ToTable(coordinator, data) ?? coordinator.NewTable()
                : coordinator.NewTable();
        });
        module["legend"] = new Func<object?, LuaTable>(options =>
        {
            if (options is LuaTable table)
            {
                JsonObject? legend = coordinator.AsObject(table);
                if (legend is not null)
                {
                    ApplyLegend(coordinator, legend);
                }
            }
            return ToTable(coordinator, coordinator.SemanticLegend) ?? coordinator.NewTable();
        });
        module["register"] = new Func<object?, bool>(options =>
        {
            if (options is LuaTable table)
            {
                JsonObject? legend = coordinator.AsObject(table);
                if (legend is not null)
                {
                    ApplyLegend(coordinator, legend);
                }
            }
            return true;
        });
        module["invalidate"] = new Func<string, bool>(uri =>
        {
            coordinator.ActiveServer?.InvalidateSemanticTokens(uri);
            return true;
        });
        module["clear"] = module["invalidate"];

        LuaTable types = coordinator.NewTable();
        foreach (
            string name in new[]
            {
                "namespace",
                "type",
                "class",
                "enum",
                "interface",
                "struct",
                "typeParameter",
                "parameter",
                "variable",
                "property",
                "enumMember",
                "event",
                "function",
                "method",
                "macro",
                "keyword",
                "modifier",
                "comment",
                "string",
                "number",
                "regexp",
                "operator",
                "decorator",
            }
        )
        {
            types[name] = name;
        }
        module["token_type"] = coordinator.WrapTable(types);

        LuaTable modifiers = coordinator.NewTable();
        foreach (
            string name in new[]
            {
                "declaration",
                "definition",
                "readonly",
                "static",
                "deprecated",
                "abstract",
                "async",
                "modification",
                "documentation",
                "defaultLibrary",
            }
        )
        {
            modifiers[name] = name;
        }
        module["token_modifier"] = coordinator.WrapTable(modifiers);
    }

    private static void ApplyLegend(LuaLspCoordinator coordinator, JsonObject legend)
    {
        JsonArray? types = legend["tokenTypes"] as JsonArray ?? legend["token_types"] as JsonArray;
        JsonArray? modifiers =
            legend["tokenModifiers"] as JsonArray ?? legend["token_modifiers"] as JsonArray;
        if (types is not null)
        {
            coordinator.SemanticLegend["tokenTypes"] = types.DeepClone();
        }
        if (modifiers is not null)
        {
            coordinator.SemanticLegend["tokenModifiers"] = modifiers.DeepClone();
        }
    }

    // -----------------------------------------------------------------------
    // inlay hints / signature help / folding / code lens
    // -----------------------------------------------------------------------

    private static void BindInlayHint(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("padding_left", "paddingLeft"),
                            ("padding_right", "paddingRight"),
                            ("text_edits", "textEdits")
                        )
                    )
                    : null
        );
        module["resolve"] = new Func<object?, LuaFunction?, object?>(
            (hint, resolver) =>
            {
                if (resolver is null)
                {
                    return hint;
                }
                object[] result = resolver.Call(hint);
                return result.Length > 0 ? result[0] : hint;
            }
        );
    }

    private static void BindSignatureHelp(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("active_signature", "activeSignature"),
                            ("active_parameter", "activeParameter")
                        )
                    )
                    : null
        );
        module["signature"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("active_parameter", "activeParameter")
                        )
                    )
                    : null
        );
        module["parameter"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("label", "label")
                        )
                    )
                    : null
        );
    }

    private static void BindFoldingRange(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(
                        coordinator,
                        Normalize(
                            coordinator.AsObject(table) ?? new JsonObject(),
                            ("start_line", "startLine"),
                            ("start_character", "startCharacter"),
                            ("end_line", "endLine"),
                            ("end_character", "endCharacter"),
                            ("collapsed_text", "collapsedText")
                        )
                    )
                    : null
        );
    }

    private static void BindCodeLens(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create"] = new Func<object?, object?>(
            (options) =>
                options is LuaTable table
                    ? ToTable(coordinator, coordinator.AsObject(table) ?? new JsonObject())
                    : null
        );
        module["resolve"] = new Func<object?, LuaFunction?, object?>(
            (lens, resolver) =>
            {
                if (resolver is null)
                {
                    return lens;
                }
                object[] result = resolver.Call(lens);
                return result.Length > 0 ? result[0] : lens;
            }
        );
    }

    // -----------------------------------------------------------------------
    // json / filesystem / logging / index / process / debug / testing
    // -----------------------------------------------------------------------

    private static void BindJson(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["null"] = coordinator.NullSentinel;
        module["encode"] = new Func<object?, string>(value =>
            LspJson.Encode(coordinator.FromLua(value))
        );
        module["decode"] = new Func<string, object?>(text =>
        {
            try
            {
                return coordinator.ToLua(JsonNode.Parse(text));
            }
            catch (JsonException)
            {
                return null;
            }
        });
        module["is_valid"] = new Func<string, bool>(LspJson.IsValid);
        module["is_null"] = new Func<object?, bool>(value =>
            ReferenceEquals(value, coordinator.NullSentinel)
        );
        module["array"] = new Func<LuaTable>(() => coordinator.NewTable());
        module["object"] = new Func<LuaTable>(() => coordinator.NewTable());
    }

    private static void BindFs(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["read_file"] = new Func<string, object?>(
            (pathValue) =>
            {
                try
                {
                    return coordinator.Workspace.ReadText(pathValue);
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or NotSupportedException
                                or ArgumentException
                    )
                {
                    return null;
                }
            }
        );
        module["read_file_async"] = new Func<string, LuaTable>(pathValue =>
        {
            var task = new LspTaskResult();
            try
            {
                task.Complete(coordinator.Workspace.ReadText(pathValue));
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return coordinator.CreateTask(task);
        });
        module["write_file"] = new Func<string, string, object?>(
            (pathValue, contents) =>
            {
                try
                {
                    coordinator.Workspace.WriteText(
                        pathValue,
                        contents ?? string.Empty,
                        createDirectories: true
                    );
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
                    return null;
                }
            }
        );
        module["write_file_async"] = new Func<string, string, LuaTable>(
            (pathValue, contents) =>
            {
                var task = new LspTaskResult();
                try
                {
                    coordinator.Workspace.WriteText(
                        pathValue,
                        contents ?? string.Empty,
                        createDirectories: true
                    );
                    task.Complete(true);
                }
                catch (Exception ex)
                {
                    task.Fail(ex.Message);
                }
                return coordinator.CreateTask(task);
            }
        );
        module["exists"] = new Func<string, bool>(pathValue =>
            File.Exists(coordinator.Workspace.ResolvePath(pathValue))
            || Directory.Exists(coordinator.Workspace.ResolvePath(pathValue))
        );
        module["stat"] = new Func<string, LuaTable?>(pathValue =>
            ToTable(coordinator, coordinator.Workspace.Stat(pathValue))
        );
        module["stat_async"] = new Func<string, LuaTable>(pathValue =>
        {
            var task = new LspTaskResult();
            try
            {
                task.Complete(ToTable(coordinator, coordinator.Workspace.Stat(pathValue)));
            }
            catch (Exception ex)
            {
                task.Fail(ex.Message);
            }
            return coordinator.CreateTask(task);
        });
        module["realpath"] = new Func<string, string>(pathValue =>
        {
            try
            {
                return Path.GetFullPath(coordinator.Workspace.ResolvePath(pathValue));
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                return pathValue;
            }
        });
        module["dirname"] = new Func<string, string>(pathValue =>
            Path.GetDirectoryName(pathValue) ?? string.Empty
        );
        module["basename"] = new Func<string, string>(pathValue => Path.GetFileName(pathValue));
        module["normalize"] = new Func<string, string>(pathValue =>
        {
            try
            {
                return Path.GetFullPath(coordinator.Workspace.ResolvePath(pathValue));
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                return pathValue;
            }
        });
        module["glob"] = new Func<string, object?, LuaTable>(
            (pattern, options) =>
            {
                var values = new List<object?>();
                foreach (
                    string uriValue in coordinator.Workspace.FindFiles(
                        pattern,
                        options is LuaTable table ? coordinator.AsObject(table) : null
                    )
                )
                {
                    values.Add(LspUri.ToPath(uriValue));
                }
                return coordinator.NewArray(values);
            }
        );
        module["mkdir"] = new Func<string, object?, bool>(
            (pathValue, options) =>
            {
                try
                {
                    string path = coordinator.Workspace.ResolvePath(pathValue);
                    coordinator.Permissions.AuthorizeWrite(path);
                    Directory.CreateDirectory(path);
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
                    return false;
                }
            }
        );
        module["remove"] = new Func<string, bool>(pathValue =>
        {
            try
            {
                string path = coordinator.Workspace.ResolvePath(pathValue);
                coordinator.Permissions.AuthorizeWrite(path);
                if (Directory.Exists(path))
                {
                    Directory.Delete(path, recursive: true);
                }
                else
                {
                    File.Delete(path);
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
                return false;
            }
        });
        module["rename"] = new Func<string, string, bool>(
            (oldValue, newValue) =>
            {
                try
                {
                    string oldPath = coordinator.Workspace.ResolvePath(oldValue);
                    string newPath = coordinator.Workspace.ResolvePath(newValue);
                    coordinator.Permissions.AuthorizeWrite(oldPath);
                    coordinator.Permissions.AuthorizeWrite(newPath);
                    File.Move(oldPath, newPath);
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
                    return false;
                }
            }
        );
    }

    private static void BindLog(LuaLspCoordinator coordinator, LuaTable module)
    {
        LuaTable levels = coordinator.NewTable();
        levels["TRACE"] = 0;
        levels["DEBUG"] = 1;
        levels["INFO"] = 2;
        levels["WARN"] = 3;
        levels["ERROR"] = 4;
        module["level"] = coordinator.WrapTable(levels);

        int threshold = 0;
        string destination = string.Empty;

        bool Log(int level, string prefix, string message, object? fields)
        {
            if (level < threshold)
            {
                return true;
            }

            string text = $"[{prefix}] {message}";
            if (fields is LuaTable table)
            {
                JsonNode? node = coordinator.FromLua(table);
                if (node is not null)
                {
                    text += " " + LspJson.Encode(node);
                }
            }
            if (destination.Length > 0)
            {
                try
                {
                    File.AppendAllText(destination, text + Environment.NewLine);
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or NotSupportedException
                                or ArgumentException
                    )
                {
                    HexalPaths.Log(text);
                }
            }
            else
            {
                HexalPaths.Log($"lua [{coordinator.PluginId}] {text}");
            }
            return true;
        }

        module["trace"] = new Func<string, object?, bool>(
            (message, fields) => Log(0, "trace", message, fields)
        );
        module["debug"] = new Func<string, object?, bool>(
            (message, fields) => Log(1, "debug", message, fields)
        );
        module["info"] = new Func<string, object?, bool>(
            (message, fields) => Log(2, "info", message, fields)
        );
        module["warn"] = new Func<string, object?, bool>(
            (message, fields) => Log(3, "warn", message, fields)
        );
        module["error"] = new Func<string, object?, bool>(
            (message, fields) => Log(4, "error", message, fields)
        );
        module["set_level"] = new Func<object?, bool>(level =>
        {
            threshold =
                LspJson.ToInt32(level)
                ?? (
                    level?.ToString()?.ToLowerInvariant() switch
                    {
                        "trace" => 0,
                        "debug" => 1,
                        "info" => 2,
                        "warn" or "warning" => 3,
                        "error" => 4,
                        _ => 0,
                    }
                );
            return true;
        });
        module["set_destination"] = new Func<string?, bool>(pathValue =>
        {
            destination = pathValue ?? string.Empty;
            return true;
        });
    }

    private static void BindIndex(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create"] = new Func<object?, LuaTable>(options =>
        {
            LspWorkspaceIndex index = LspWorkspace.CreateIndex(
                options is LuaTable table ? coordinator.AsObject(table) : null
            );
            return coordinator.CreateIndexTable(index);
        });
        module["new"] = module["create"];
    }

    private static void BindProcess(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["run"] = new Func<string, object?, object?, LuaTable>(
            (command, args, options) =>
            {
                var task = new LspTaskResult();
                if (!coordinator.Permissions.CreateProcesses)
                {
                    task.Fail("process permission is required");
                    return coordinator.CreateTask(task);
                }

                try
                {
                    var startInfo = new System.Diagnostics.ProcessStartInfo
                    {
                        FileName = command,
                        RedirectStandardOutput = true,
                        RedirectStandardError = true,
                        UseShellExecute = false,
                        CreateNoWindow = true,
                    };
                    if (args is LuaTable argumentTable)
                    {
                        foreach (object? item in LspJson.ToList(argumentTable))
                        {
                            startInfo.ArgumentList.Add(item?.ToString() ?? string.Empty);
                        }
                    }

                    using var process = System.Diagnostics.Process.Start(startInfo);
                    if (process is null)
                    {
                        task.Fail($"could not start '{command}'");
                        return coordinator.CreateTask(task);
                    }
                    string standardOutput = process.StandardOutput.ReadToEnd();
                    string standardError = process.StandardError.ReadToEnd();
                    process.WaitForExit();
                    var result = new JsonObject
                    {
                        ["code"] = process.ExitCode,
                        ["stdout"] = standardOutput,
                        ["stderr"] = standardError,
                    };
                    task.Complete(coordinator.ToLua(result));
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or InvalidOperationException
                                or System.ComponentModel.Win32Exception
                    )
                {
                    task.Fail(ex.Message);
                }
                return coordinator.CreateTask(task);
            }
        );
        module["is_allowed"] = new Func<bool>(() => coordinator.Permissions.CreateProcesses);
    }

    private static void BindDebug(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["registered_handlers"] = new Func<object?, LuaTable>(serverValue =>
        {
            LuaTable result = coordinator.NewTable();
            LuaLspServer? server =
                serverValue is LuaTable table
                && table["__hexal_server_name"]?.ToString() is string name
                && name.Length > 0
                    ? coordinator.FindServer(name)
                    : coordinator.ActiveServer;
            if (server is not null)
            {
                foreach (string method in server.RegisteredRequestMethods)
                {
                    result[method] = true;
                }
                foreach (string method in server.RegisteredNotificationMethods)
                {
                    result[method] = true;
                }
            }
            return coordinator.WrapTable(result);
        });
        module["capabilities"] = new Func<LuaTable?>(() =>
            coordinator.ActiveServer is { } server
                ? coordinator.ToLua(server.ServerCapabilities) as LuaTable
                : null
        );
        module["documents"] = new Func<LuaTable>(() =>
        {
            LuaTable result = coordinator.NewTable();
            foreach (LspDocument document in coordinator.Documents.All)
            {
                result[document.Uri] = document.Version;
            }
            return coordinator.WrapTable(result);
        });
        module["pending_requests"] = new Func<LuaTable>(() =>
        {
            LuaTable result = coordinator.NewTable();
            int index = 1;
            foreach (long id in coordinator.ActiveServer?.PendingRequestIds ?? Array.Empty<long>())
            {
                result[index++] = id;
            }
            return coordinator.WrapTable(result);
        });
        module["active_tasks"] = new Func<LuaTable>(() =>
            coordinator.WrapTable(coordinator.NewTable())
        );
        module["index_stats"] = new Func<LuaTable>(() =>
        {
            LuaTable result = coordinator.NewTable();
            result["documents"] = coordinator.Documents.All.Count;
            result["diagnostics"] = coordinator.Diagnostics.CountAll();
            result["servers"] = coordinator.Servers.Count;
            return coordinator.WrapTable(result);
        });
        module["set_trace"] = new Func<string?, bool>(level =>
        {
            coordinator.TraceEnabled =
                level is not null
                && !string.Equals(level, "off", StringComparison.OrdinalIgnoreCase);
            return true;
        });
        module["inspect"] = new Func<LuaTable>(() =>
        {
            LuaTable result = coordinator.NewTable();
            result["servers"] = coordinator.Servers.Count;
            result["documents"] = coordinator.Documents.All.Count;
            result["modules"] = coordinator.Modules.Count;
            return coordinator.WrapTable(result);
        });
    }

    private static void BindTesting(LuaLspCoordinator coordinator, LuaTable module)
    {
        module["create_client"] = new Func<object?, object?>(
            (serverValue) =>
            {
                LuaLspServer? server =
                    serverValue is LuaTable table
                    && table["__hexal_server_name"]?.ToString() is string name
                    && name.Length > 0
                        ? coordinator.FindServer(name)
                        : coordinator.ActiveServer;
                if (server is null)
                {
                    return null;
                }
                return new LuaLspTestClient(coordinator, server).CreateTable();
            }
        );
    }

    // -----------------------------------------------------------------------
    // Lua prelude
    // -----------------------------------------------------------------------

    private const string Prelude = """
        (function()
            local H = __hexal_lsp_host

            __hexal_wrap_methods = function(t)
                if type(t) ~= "table" or rawget(t, "__hexal_wrapped") then
                    return t
                end
                rawset(t, "__hexal_wrapped", true)
                local functions = {}
                for k, v in pairs(t) do
                    local valueType = type(v)
                    if valueType == "function" then
                        functions[k] = v
                    elseif valueType == "userdata" then
                        local mt = getmetatable(v)
                        if mt and mt.__call then
                            functions[k] = v
                        end
                    end
                end
                for k in pairs(functions) do
                    rawset(t, k, function(...)
                        local n = select("#", ...)
                        local args = table.pack(...)
                        local first = 1
                        if n > 0 and args[1] == t then
                            first = 2
                        end
                        return functions[k](table.unpack(args, first, n))
                    end)
                end
                return t
            end

            __hexal_pad = function(t, name, count)
                local original = t[name]
                if original == nil then
                    return
                end
                rawset(t, name, function(...)
                    local args = table.pack(...)
                    local first = 1
                    if args.n > 0 and args[1] == t then
                        first = 2
                    end
                    local call = {}
                    for index = 1, count do
                        call[index] = args[first + index - 1]
                    end
                    return original(table.unpack(call, 1, count))
                end)
            end

            local A = hexal.async
            local function new_task()
                local task = {
                    done = false,
                    cancelled = false,
                    value = nil,
                    callbacks = {},
                }
                function task:is_done()
                    return self.done or self.cancelled
                end
                function task:is_cancelled()
                    return self.cancelled
                end
                function task:result()
                    return self.value
                end
                function task:cancel()
                    if self.done or self.cancelled then
                        return false
                    end
                    self.cancelled = true
                    local callbacks = self.callbacks
                    self.callbacks = {}
                    for _, callback in ipairs(callbacks) do
                        callback(nil)
                    end
                    return true
                end
                function task:on_complete(callback)
                    if self:is_done() then
                        callback(self.value)
                    else
                        table.insert(self.callbacks, callback)
                    end
                end
                function task:_complete(value)
                    if self.done or self.cancelled then
                        return
                    end
                    self.done = true
                    self.value = value
                    local callbacks = self.callbacks
                    self.callbacks = {}
                    for _, callback in ipairs(callbacks) do
                        callback(value)
                    end
                end
                return task
            end

            local function step(task, co, ...)
                if task.cancelled then
                    return
                end
                local results = table.pack(coroutine.resume(co, ...))
                if not results[1] then
                    task:_complete(nil)
                    error(results[2])
                end
                if coroutine.status(co) == "dead" then
                    task:_complete(results[2])
                    return
                end
                local value = results[2]
                if type(value) == "table" then
                    if value.__sleep then
                        H.async_after(value.__sleep, function()
                            step(task, co)
                        end)
                    elseif value.__await then
                        local awaited = value.__await
                        if type(awaited) == "table" and awaited.is_done then
                            if awaited:is_done() then
                                step(task, co, awaited:result())
                            else
                                awaited:on_complete(function(result)
                                    step(task, co, result)
                                end)
                            end
                        else
                            step(task, co, awaited)
                        end
                    else
                        H.async_after(0, function() step(task, co, value) end)
                    end
                else
                    H.async_after(0, function() step(task, co, value) end)
                end
            end

            A.run = function(fn)
                local task = new_task()
                local co = coroutine.create(fn)
                step(task, co)
                return task
            end
            A.spawn = function(fn)
                local task = new_task()
                H.async_after(0, function()
                    local co = coroutine.create(fn)
                    step(task, co)
                end)
                return task
            end
            A.await = function(operation)
                if type(operation) == "table" and operation.is_done then
                    if operation:is_done() then
                        return operation:result()
                    end
                    if coroutine.isyieldable() then
                        return coroutine.yield({ __await = operation })
                    end
                    return nil
                end
                return operation
            end
            A.yield = function()
                if coroutine.isyieldable() then
                    return coroutine.yield({ __yield = true })
                end
                return nil
            end
            A.sleep = function(milliseconds)
                if coroutine.isyieldable() then
                    return coroutine.yield({ __sleep = milliseconds or 0 })
                end
                return nil
            end
            A.cancel = function(task)
                if type(task) == "number" then
                    return H.async_cancel(task)
                end
                if type(task) == "table" and task.cancel then
                    return task:cancel()
                end
                return false
            end
            A.is_cancelled = function()
                return H.async_is_cancelled()
            end
            A.schedule = function(fn)
                return A.spawn(fn)
            end
            A.tick = function()
                return H.async_tick()
            end
            A.pump = A.tick
            local debounces = {}
            A.debounce = function(key, milliseconds, fn)
                if debounces[key] then
                    H.async_cancel(debounces[key])
                end
                debounces[key] = H.async_after(milliseconds or 0, function()
                    debounces[key] = nil
                    fn()
                end)
                return debounces[key]
            end

            local D = hexal.lsp.documents
            D.open_documents = function()
                local list = H.documents_list()
                local index = 0
                return function()
                    index = index + 1
                    return list[index]
                end
            end

            local FS = hexal.fs
            FS.join = function(...)
                return H.fs_join({ ... })
            end

            local E = hexal.lsp.edit
            local apply = E.apply_workspace_edit
            E.apply_workspace_edit = function(edit)
                local result = apply(edit)
                if type(result) == "table" then
                    return result.ok, result.error
                end
                return result
            end
        end)()
        """;
}
