-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "purescript-language-server", command = "purescript-language-server", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "purescript" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "purescript",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
