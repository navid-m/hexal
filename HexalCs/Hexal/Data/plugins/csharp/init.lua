local active_buffer = nil
local builtin_server = nil

local function ensure_server(event)
    if event.language ~= "csharp" then
        return
    end

    -- The editor stops the old server when it opens another buffer, so restart
    -- on every buffer change. Repeated events for the same buffer are ignored
    -- to avoid unnecessary PATH scans and server restarts.
    local buffer = event.path or event.id or ""
    if buffer == active_buffer then
        return
    end
    active_buffer = buffer

    -- Prefer OmniSharp when it is installed. hexal.lsp.start returns false
    -- without starting anything when the executable is not in PATH.
    if hexal.lsp.start({
        name = "OmniSharp",
        command = "OmniSharp",
        args = { "--languageserver" },
        language_id = "csharp",
    }) then
        return
    end

    -- Fall back to the built-in Lua C# language server. The server is created
    -- once and reattached to each new buffer without rebuilding its analysis.
    if builtin_server == nil then
        builtin_server = require("cs_server").create()
    elseif builtin_server then
        builtin_server:start()
    end
end

hexal.events.on("buffer_opened", ensure_server)
