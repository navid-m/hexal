-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "csharp-ls", command = "csharp-ls", args = {} },
    { name = "roslyn-language-server", command = "roslyn-language-server", args = { "--stdio" } },
    { name = "omnisharp", command = "omnisharp", args = { "--languageserver" } },
}

local function start_for_buffer(event)
    if event.language ~= "csharp" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "csharp",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
