-- Eclipse JDTLS language server.
--
-- Debugging requires the java-debug bundle. Point at
-- com.microsoft.java.debug.plugin.jar in ~/.hexal/plugin-config/java.json:
--
--   { "javaDebugJar": "/path/to/com.microsoft.java.debug.plugin.jar" }
local servers = {
    { name = "jdtls", command = "jdtls", args = {} },
    { name = "java-language-server", command = "java-language-server", args = {} },
}

local java_debug_jar = hexal.config.get("javaDebugJar", nil)
if type(java_debug_jar) ~= "string" or java_debug_jar == "" then
    java_debug_jar = nil
end

local function start_for_buffer(event)
    if event.language ~= "java" then
        return
    end

    for _, server in ipairs(servers) do
        local options = {
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "java",
        }
        if java_debug_jar then
            options.initialization_options = {
                bundles = { java_debug_jar },
            }
        end
        if hexal.lsp.start(options) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
