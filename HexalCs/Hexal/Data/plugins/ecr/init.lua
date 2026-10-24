-- Crystal language servers do not understand ECR template markup directly,
-- so this language is syntax-only for now. Syntax highlighting, HTML tag
-- closing and indentation are provided by the bundled ECR grammar.
local servers = {}

local function start_for_buffer(event)
    if event.language ~= "ecr" then
        return
    end

    for _, server in ipairs(servers) do
        if hexal.lsp.start({
            name = server.name,
            command = server.command,
            args = server.args,
            language_id = "ecr",
        }) then
            return
        end
    end
end

hexal.events.on("buffer_opened", start_for_buffer)
