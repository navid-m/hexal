-- Starts the first AsciiDoc language server found in PATH.
local servers = {
    {
        name = "asciidoc-language-server",
        command = "asciidoc-language-server",
        args = {},
    },
    { name = "asciidoc-ls", command = "asciidoc-ls", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "asciidoc" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "asciidoc",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
