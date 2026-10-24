-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "typescript-language-server", command = "typescript-language-server", args = { "--stdio" } },
    { name = "vtsls", command = "vtsls", args = { "--stdio" } },
    { name = "biome", command = "biome", args = { "lsp-proxy" } },
}

local function start_for_buffer(event)
    if event.language ~= "tsx" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "typescriptreact",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
