-- Scar currently ships syntax support without a public language server.
-- Keeping the association means an LSP can be added later without changing
-- the grammar.
local servers = {}

local function start_for_buffer(event)
    if event.language ~= "scar" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "scar",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
