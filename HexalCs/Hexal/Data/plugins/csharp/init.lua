local created = false

local function ensure_server(event)
    if created or event.language ~= "csharp" then
        return
    end

    created = true
    local server = require("cs_server")
    server.create()
end

hexal.events.on("buffer_opened", ensure_server)
