-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "matlab-language-server", command = "matlab-language-server", args = { "--stdio" } },
    { name = "matlab_ls", command = "matlab_ls", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "matlab" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "matlab",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
