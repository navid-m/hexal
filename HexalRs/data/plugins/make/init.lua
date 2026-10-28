-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "make-ls", command = "make-ls", args = {} },
    { name = "makefile-lsp", command = "makefile-lsp", args = {} },
    { name = "autotools-language-server", command = "autotools-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "make" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "make",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
