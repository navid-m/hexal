-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "protols", command = "protols", args = {} },
    { name = "bufls", command = "bufls", args = {} },
    { name = "protobuf-language-server", command = "protobuf-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "protobuf" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "protobuf",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
