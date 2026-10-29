-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "intelephense", command = "intelephense", args = { "--stdio" } },
    { name = "phpactor", command = "phpactor", args = { "language-server" } },
    { name = "php-language-server", command = "php-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "php" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "php",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
