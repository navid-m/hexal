-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "roslyn-language-server", command = "roslyn-language-server", args = { "--stdio", "--autoLoadProjects" } },
    { name = "razor-language-server", command = "razor-language-server", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "razor" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "razor",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
