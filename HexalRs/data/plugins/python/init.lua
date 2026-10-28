-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "pyright", command = "pyright-langserver", args = { "--stdio" } },
    { name = "ty", command = "ty", args = { "server" } },
    { name = "mypy", command = "mypy", args = { "--lsp" } },
    { name = "mypy-language-server", command = "mypy-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "python" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "python",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
