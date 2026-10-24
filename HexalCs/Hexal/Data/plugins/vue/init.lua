-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "vue-language-server", command = "vue-language-server", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "vue" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "vue",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
