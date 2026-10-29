-- Generated Hexal language-server association.
-- Starts Liger, the default Crystal language server, when available.
local servers = {
    { name = "liger", command = "liger", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "crystal" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "crystal",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
