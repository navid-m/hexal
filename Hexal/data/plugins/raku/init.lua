-- Raku Navigator language server.
-- Tries an installed executable first. When RakuNavigator was built from
-- source, point at its server entry point in ~/.hexal/plugin-config/raku.json:
--
--   { "serverPath": "/path/to/RakuNavigator/server/out/server.js" }
local servers = {
    { name = "raku-navigator", command = "rakunavigator", args = {} },
    { name = "raku-navigator", command = "raku-navigator", args = {} },
}

local server_path = hexal.config.get("serverPath", nil)
if type(server_path) == "string" and server_path ~= "" then
    table.insert(servers, 1, {
        name = "raku-navigator",
        command = "node",
        args = { server_path, "--stdio" },
    })
end

local function start_for_buffer(event)
    if event.language ~= "raku" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "raku",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
