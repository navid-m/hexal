local analysis = require("cs_analysis")
local features = require("cs_features")
local builtins = require("cs_builtins")

local M = {}

local token_types = {
    "namespace", "type", "class", "enum", "interface", "struct",
    "typeParameter", "parameter", "variable", "property", "enumMember",
    "event", "function", "method", "macro", "keyword", "modifier", "comment",
    "string", "number", "regexp", "operator", "decorator",
}

local token_modifiers = {
    "declaration", "definition", "readonly", "static", "deprecated",
    "abstract", "async", "modification", "documentation", "defaultLibrary",
}

local function publish_diagnostics(document)
    if not document or not document.uri then
        return
    end
    local model = analysis.get_model(document.uri, document.text,
        document.version)
    if not model then
        return
    end
    local items = {}
    for _, diagnostic in ipairs(model.diagnostics) do
        items[#items + 1] = analysis.lsp_diagnostic(model, diagnostic)
    end
    local diagnostics = hexal.lsp.diagnostics
    diagnostics.set(document.uri, "csharp", items, {
        version = document.version,
    })
    analysis.update_workspace(document.uri)
end

local function install_handlers(server)
    server:register("textDocument/hover", features.hover)
    server:register("textDocument/completion", features.completion)
    server:register("completionItem/resolve", features.completion_resolve)
    server:register("textDocument/signatureHelp", features.signature_help)
    server:register("textDocument/definition", features.definition)
    server:register("textDocument/declaration", features.declaration)
    server:register("textDocument/typeDefinition", features.type_definition)
    server:register("textDocument/implementation", features.implementation)
    server:register("textDocument/references", features.references)
    server:register("textDocument/documentSymbol", features.document_symbols)
    server:register("workspace/symbol", features.workspace_symbols)
    server:register("textDocument/prepareRename", features.prepare_rename)
    server:register("textDocument/rename", features.rename)
    server:register("textDocument/codeAction", features.code_actions)
    server:register("codeAction/resolve", features.code_action_resolve)
    server:register("textDocument/formatting", features.formatting)
    server:register("textDocument/rangeFormatting", features.range_formatting)
    server:register("textDocument/onTypeFormatting", features.on_type_formatting)
    server:register("textDocument/semanticTokens/full",
        features.semantic_tokens)
    server:register("textDocument/semanticTokens/full/delta",
        features.semantic_tokens)
    server:register("textDocument/semanticTokens/range",
        features.semantic_tokens)
    server:register("textDocument/inlayHint", features.inlay_hints)
    server:register("inlayHint/resolve", features.inlay_hint_resolve)
    server:register("textDocument/foldingRange", features.folding_ranges)
    server:register("textDocument/codeLens", features.code_lens)
    server:register("codeLens/resolve", features.code_lens_resolve)
    server:register("textDocument/documentLink", features.document_links)
    server:register("documentLink/resolve", features.document_link_resolve)
    server:register("textDocument/selectionRange", features.selection_range)
    server:register("textDocument/documentHighlight",
        features.document_highlight)
    server:register("textDocument/linkedEditingRange",
        features.linked_editing)
    server:register("textDocument/prepareCallHierarchy",
        features.prepare_call_hierarchy)
    server:register("callHierarchy/incomingCalls", features.incoming_calls)
    server:register("callHierarchy/outgoingCalls", features.outgoing_calls)
    server:register("textDocument/prepareTypeHierarchy",
        features.prepare_type_hierarchy)
    server:register("typeHierarchy/supertypes", features.supertypes)
    server:register("typeHierarchy/subtypes", features.subtypes)
end

local function install_events(server)
    server:on("initialize", function(params, ctx)
        return {
            capabilities = server:advertised_capabilities(),
            serverInfo = {
                name = "hexal-csharp",
                version = "1.0.0",
            },
        }
    end)

    server:on("initialized", function(params, ctx)
        local documents = hexal.lsp.documents
        local ok, iterator = pcall(documents.open_documents)
        if ok then
            for document in iterator do
                publish_diagnostics(document)
            end
        end
        if hexal.async and hexal.async.spawn then
            hexal.async.spawn(function()
                analysis.index_workspace(true)
            end)
        end
    end)

    server:on("document_opened", function(document, ctx)
        publish_diagnostics(document)
    end)

    server:on("document_changed", function(document, ctx)
        publish_diagnostics(document)
    end)

    server:on("document_saved", function(document, ctx)
        publish_diagnostics(document)
    end)

    server:on("document_closed", function(document, ctx)
        analysis.invalidate(document.uri)
    end)

    server:on("workspace_changed", function(event, ctx)
        analysis.invalidate_workspace()
    end)

    server:on("configuration_changed", function(settings, ctx)
        if hexal.async and hexal.async.spawn then
            hexal.async.spawn(function()
                analysis.index_workspace(true)
            end)
        end
    end)

    server:on("shutdown", function(params, ctx)
        hexal.lsp.diagnostics.clear_all()
    end)
end

function M.create()
    local server = hexal.lsp.create_server({
        name = "hexal-csharp",
        version = "1.0.0",
        filetypes = { "csharp" },
        extensions = { ".cs" },
        root_markers = {
            ".git", ".sln", ".slnx", ".csproj", ".fsproj", ".vbproj",
            "global.json", "Directory.Build.props", "Directory.Packages.props",
        },
        encoding = "utf-16",
        settings = {
            csharp = {
                inlayHints = true,
            },
        },
        permissions = {
            read = true,
            write = true,
            workspace_only = true,
        },
    })
    if not server then
        return nil
    end

    server:capabilities({
        text_document_sync = {
            open_close = true,
            change = "incremental",
            save = {
                include_text = true,
            },
        },
        hover = true,
        completion = {
            triggerCharacters = { ".", "<", ":", "\"", "/", " " },
            resolveProvider = true,
            completionItem = {
                snippetSupport = true,
                documentationFormat = { "markdown", "plaintext" },
            },
        },
        definition = true,
        declaration = true,
        type_definition = true,
        implementation = true,
        references = true,
        document_symbols = true,
        workspace_symbols = true,
        rename = {
            prepareProvider = true,
        },
        code_actions = {
            codeActionKinds = {
                "quickfix", "source.organizeImports", "source.fixAll",
            },
            resolveProvider = true,
        },
        formatting = true,
        range_formatting = true,
        on_type_formatting = {
            firstTriggerCharacter = "}",
            moreTriggerCharacter = { ";", "\n" },
        },
        signature_help = {
            triggerCharacters = { "(", "," },
            retriggerCharacters = { "," },
        },
        semantic_tokens = {
            legend = {
                tokenTypes = token_types,
                tokenModifiers = token_modifiers,
            },
            full = { delta = true },
            range = true,
        },
        inlay_hints = {
            resolveProvider = false,
        },
        folding_ranges = true,
        code_lens = {
            resolveProvider = true,
        },
        document_links = {
            resolveProvider = true,
        },
        selection_ranges = true,
        linked_editing = true,
        call_hierarchy = true,
        type_hierarchy = true,
        documentHighlightProvider = true,
    })

    install_handlers(server)
    install_events(server)
    server:start()
    return server
end

return M
