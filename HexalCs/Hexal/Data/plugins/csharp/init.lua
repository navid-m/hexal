local started = false

local function ensure_server(event)
    if started or event.language ~= "csharp" then
        return
    end

    started = true

    -- Prefer OmniSharp when it is installed. hexal.lsp.start returns false
    -- without starting anything when the executable is not in PATH.
    if hexal.lsp.start({
        name = "omnisharp",
        command = "omnisharp",
        args = { "--languageserver" },
        language_id = "csharp",
    }) then
        return
    end

    -- Fall back to the built-in Lua C# language server.
    local server = require("cs_server")
    server.create()
end

hexal.events.on("buffer_opened", ensure_server)
