-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "nixd", command = "nixd", args = {} },
    { name = "nil", command = "nil", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "nix" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "nix",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
