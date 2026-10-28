-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "sqls", command = "sqls", args = {} },
    { name = "sql-language-server", command = "sql-language-server", args = { "up", "--method", "stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "sql" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "sql",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
