-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "ada_language_server", command = "ada_language_server", args = {} },
    { name = "als", command = "als", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "ada" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "ada",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
