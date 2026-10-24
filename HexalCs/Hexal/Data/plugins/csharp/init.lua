local active_buffer = nil
local builtin_server = nil

local function ensure_server(event)
    if event.language ~= "csharp" then
        return
    end

    -- Ask to attach each newly active buffer. The host reuses a compatible
    -- initialized server and switches its active document; it only starts a
    -- new process when the server or workspace changes. Repeated events for
    -- the same buffer are ignored while its server is still running.
    local buffer = event.path or event.id or ""
    if buffer == active_buffer and event.lsp_running ~= false then
        return
    end
    active_buffer = buffer

    if hexal.lsp.start({
        name = "OmniSharp",
        command = "OmniSharp",
        args = { "--languageserver" },
        language_id = "csharp",
    }) then
        return
    end

    if builtin_server == nil then
        builtin_server = require("cs_server").create()
    elseif builtin_server then
        builtin_server:start()
    end
end

hexal.events.on("buffer_opened", ensure_server)
