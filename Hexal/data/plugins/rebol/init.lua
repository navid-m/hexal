-- Recoil language server for Rebol.
-- recoil-lsp.r3 is a Rebol script. Set its full path in
-- ~/.hexal/plugin-config/rebol.json when it is not in the working directory:
--
--   { "lspScript": "/path/to/recoil-lsp.r3" }
local servers = {
    { name = "recoil-lsp", command = "recoil-lsp.r3", args = {} },
    { name = "recoil-lsp", command = "r3", args = { "recoil-lsp.r3" } },
}

local script_path = hexal.config.get("lspScript", nil)
if type(script_path) == "string" and script_path ~= "" then
    table.insert(servers, 1, {
        name = "recoil-lsp",
        command = "r3",
        args = { script_path },
    })
end

local function start_for_buffer(event)
    if event.language ~= "rebol" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "rebol",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
