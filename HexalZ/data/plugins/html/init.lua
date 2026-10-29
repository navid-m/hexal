-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
local servers = {
    { name = "vscode-html-language-server", command = "vscode-html-language-server", args = { "--stdio" } },
    { name = "superhtml", command = "superhtml", args = { "lsp" } },
    { name = "html-languageserver", command = "html-languageserver", args = { "--stdio" } },
}

local function start_for_buffer(event)
    if event.language ~= "html" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "html",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
