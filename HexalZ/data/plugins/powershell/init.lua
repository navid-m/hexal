-- Generated Hexal language-server association.
-- Starts the first server found in PATH for this language.
--
-- PowerShell Editor Services is not a standalone executable. It is launched
-- through PowerShell, so it needs the path of the extracted PSES bundle.
-- Configure it in ~/.hexal/plugin-config/powershell.json:
--
--   { "bundlePath": "/path/to/PowerShellEditorServices" }
local servers = {
    { name = "powershell-language-server", command = "powershell-language-server", args = {} },
}

local bundlePath = hexal.config.get("bundlePath", nil)
if type(bundlePath) == "string" and bundlePath ~= "" then
    local script = bundlePath .. "/PowerShellEditorServices/Start-EditorServices.ps1"
    table.insert(servers, 1, {
        name = "powershell-es",
        command = "pwsh",
        args = {
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "& '" .. script .. "' -BundledModulesPath '" .. bundlePath .. "' -Stdio -LogLevel Error",
        },
    })
end

local function start_for_buffer(event)
    if event.language ~= "powershell" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "powershell",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
