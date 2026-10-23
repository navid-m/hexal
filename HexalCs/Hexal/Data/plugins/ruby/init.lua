-- Starts the first Ruby language server found in PATH.
local servers = {
    { name = "ruby-lsp", command = "ruby-lsp", args = {} },
    { name = "solargraph", command = "solargraph", args = { "stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "ruby" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "ruby",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
