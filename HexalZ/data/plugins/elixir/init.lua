-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "elixir-ls", command = "elixir-ls", args = {} },
    { name = "language_server.sh", command = "language_server.sh", args = {} },
    { name = "elp", command = "elp", args = { "server" } },
}

local function start_for_buffer(event)
    if event.language ~= "elixir" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "elixir",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
