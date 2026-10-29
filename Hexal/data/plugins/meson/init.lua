-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "mesonlsp", command = "mesonlsp", args = { "--lsp" } },
}

local function start_for_buffer(event)
    if event.language ~= "meson" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "meson",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
