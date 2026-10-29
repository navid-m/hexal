-- Starts the first Markdown language server found in PATH.
local servers = {
    { name = "marksman", command = "marksman", args = {} },
    {
        name = "markdown-language-server",
        command = "markdown-language-server",
        args = { "--stdio" },
    },
}

local function start_for_buffer(event)
    if event.language ~= "markdown" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "markdown",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
