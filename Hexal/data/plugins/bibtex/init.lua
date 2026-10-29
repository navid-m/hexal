-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "texlab", command = "texlab", args = {} },
    { name = "bibtex-language-server", command = "bibtex-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "bibtex" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "bibtex",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
