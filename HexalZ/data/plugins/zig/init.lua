-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "zls", command = "zls", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "zig" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "zig",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
