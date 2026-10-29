-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "llvm-ir-lsp", command = "llvm-ir-lsp", args = {} },
    { name = "llvm-language-server", command = "llvm-language-server", args = {} },
}

local function start_for_buffer(event)
    if event.language ~= "llvm" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "llvm",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
