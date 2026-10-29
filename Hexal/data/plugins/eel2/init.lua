-- No public EEL2 language server is currently available. Keeping a plugin
-- association means an LSP can be added later without changing the grammar.
local servers = {}

local function start_for_buffer(event)
    if event.language ~= "eel2" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "eel2",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
