local RECORD = string.char(30)
local FIELD = string.char(31)
local GRAPH_STYLES = { "accent", "success", "warning", "diff_modified", "error" }

local function trim(value)
    return (value:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function split_lines(text)
    local lines = {}
    if text == nil then
        return lines
    end

    for line in (text .. "\n"):gmatch("(.-)\n") do
        if line:sub(-1) == "\r" then
            line = line:sub(1, -2)
        end
        lines[#lines + 1] = line
    end
    return lines
end

local function append(segments, text, style)
    if text == nil or text == "" then
        return
    end

    local last = segments[#segments]
    if last and last.style == style then
        last.text = last.text .. text
    else
        segments[#segments + 1] = { text = text, style = style }
    end
end

local function graph_segments(graph)
    local segments = {}
    for column = 1, #graph do
        local character = graph:sub(column, column)
        if character == " " then
            append(segments, character, "text")
        else
            append(segments, character,
                GRAPH_STYLES[(column - 1) % #GRAPH_STYLES + 1])
        end
    end
    return segments
end

local function run_git(args, cwd)
    local result = hexal.exec.run("git", args, {
        cwd = cwd,
        timeout = 30000,
    })
    if result.ok then
        return result.stdout
    end

    local message = result.error
    if message == nil or message == "" then
        message = result.stderr
    end
    if message == nil or message == "" then
        message = "git command failed"
    end
    return nil, trim(message)
end

local function append_decorations(segments, decorations)
    if decorations == nil or decorations == "" then
        return
    end

    for token in decorations:gmatch("[^,]+") do
        token = trim(token)
        if token ~= "" then
            local lower = token:lower()
            local style = "success"
            if lower:match("^head") then
                style = "warning"
            elseif lower:match("^tag:") then
                style = "diff_modified"
            elseif lower:match("^origin/") or
                   lower:match("^upstream/") or
                   lower:match("^remote/") then
                style = "muted"
            end

            append(segments, " ", "text")
            append(segments, "[" .. token .. "]", style)
        end
    end
end

local function parse_log(output)
    local log_lines = {}
    local commits_by_line = {}
    local commit_info = {}

    for _, raw in ipairs(split_lines(output)) do
        if raw ~= "" then
            local record_at = raw:find(RECORD, 1, true)
            local hash
            local segments

            if record_at then
                local graph = raw:sub(1, record_at - 1)
                segments = graph_segments(graph)

                local rest = raw:sub(record_at + 1)
                local fields = {}
                local start = 1
                while true do
                    local at = rest:find(FIELD, start, true)
                    if not at then
                        fields[#fields + 1] = rest:sub(start)
                        break
                    end
                    fields[#fields + 1] = rest:sub(start, at - 1)
                    start = at + 1
                end

                hash = fields[1]
                local short = fields[2] or ""
                local author = fields[3] or ""
                local date = fields[4] or ""
                local decorations = fields[5] or ""
                local subject = fields[6] or ""

                append(segments, short, "accent")
                append_decorations(segments, decorations)
                append(segments, " ", "text")
                append(segments, subject, "text")

                if author ~= "" or date ~= "" then
                    append(segments, "  ", "muted")
                    append(segments, author, "muted")
                    if date ~= "" then
                        append(segments, " · ", "muted")
                        append(segments, date, "muted")
                    end
                end

                commit_info[hash] = {
                    short = short,
                    author = author,
                    date = date,
                    subject = subject,
                }
            else
                segments = graph_segments(raw)
            end

            log_lines[#log_lines + 1] = {
                segments = segments,
                selectable = hash ~= nil,
            }
            commits_by_line[#log_lines] = hash
        end
    end

    return log_lines, commits_by_line, commit_info
end

local function build_diff_lines(hash, cwd)
    local text, error_message = run_git({
        "show",
        "--no-color",
        "--no-ext-diff",
        "--format=fuller",
        "--stat",
        "--patch",
        hash,
    }, cwd)

    if not text then
        return nil, error_message
    end

    local lines = {}
    for _, raw in ipairs(split_lines(text)) do
        local style = "text"
        if raw:match("^commit ") then
            style = "accent"
        elseif raw:match("^Author:") or raw:match("^AuthorDate:") or
               raw:match("^Commit:") or raw:match("^CommitDate:") or
               raw:match("^Merge:") then
            style = "muted"
        elseif raw:match("^diff %-%-git") or raw:match("^index ") or
               raw:match("^%-%-%- ") or raw:match("^%+%+%+ ") or
               raw:match("^@@") or raw:match("^new file mode") or
               raw:match("^deleted file mode") or
               raw:match("^similarity index") or raw:match("^rename ") then
            style = "accent"
        elseif raw:match("^%+") then
            style = "diff_added"
        elseif raw:match("^%-") then
            style = "diff_deleted"
        end

        lines[#lines + 1] = {
            segments = { { text = raw, style = style } },
            selectable = false,
        }
    end

    return lines
end

local function open_git_log()
    local cwd = hexal.editor.get_workspace_root()
    local output, error_message = run_git({
        "log",
        "--graph",
        "--date-order",
        "--decorate=short",
        "--date=relative",
        "--all",
        "--max-count=2000",
        "--no-color",
        "--pretty=format:" .. RECORD ..
            "%H" .. FIELD ..
            "%h" .. FIELD ..
            "%an" .. FIELD ..
            "%ar" .. FIELD ..
            "%D" .. FIELD ..
            "%s",
    }, cwd)

    if not output then
        hexal.ui.warn("git log: " .. (error_message or "failed"))
        return
    end

    local log_lines, commits_by_line, commit_info = parse_log(output)
    if #log_lines == 0 then
        hexal.ui.message("Git log is empty.")
        return
    end

    local first_selectable = 1
    while first_selectable <= #log_lines and
          not log_lines[first_selectable].selectable do
        first_selectable = first_selectable + 1
    end

    local state = {
        view = "log",
        log_lines = log_lines,
        commits_by_line = commits_by_line,
        commit_info = commit_info,
        log_selected = first_selectable,
        cwd = cwd,
        panel = nil,
    }

    local function show_log(selected)
        state.view = "log"
        state.panel:set_title(" Git log ")
        state.panel:set_footer(
            "j/k move · PgUp/PgDn scroll · Enter show diff · q close")
        state.panel:set_selectable(true)
        state.panel:set_lines(state.log_lines)
        state.panel:set_selected(selected or state.log_selected)
    end

    local function show_diff(hash)
        local diff_lines, diff_error = build_diff_lines(hash, state.cwd)
        if not diff_lines then
            hexal.ui.warn("git show: " .. (diff_error or "failed"))
            return
        end

        local info = state.commit_info[hash]
        local title = " Commit "
        if info and info.short and info.short ~= "" then
            title = title .. info.short .. " "
        end

        state.view = "diff"
        state.panel:set_title(title)
        state.panel:set_footer(
            "j/k scroll · PgUp/PgDn · Esc/Backspace back · q close")
        state.panel:set_selectable(false)
        state.panel:set_lines(diff_lines)
        state.panel:set_selected(1)
    end

    local function on_key(key, selected)
        if state.view == "log" then
            if key == "enter" then
                local hash = state.commits_by_line[selected]
                if hash then
                    state.log_selected = selected
                    show_diff(hash)
                    return true
                end
            end
            return false
        end

        if key == "esc" or key == "backspace" or key == "b" then
            show_log(state.log_selected)
            return true
        end

        return false
    end

    state.panel = hexal.ui.panel({
        title = " Git log ",
        footer = "j/k move · PgUp/PgDn scroll · Enter show diff · q close",
        selectable = true,
        selected = first_selectable,
        lines = log_lines,
    }, on_key)
end

hexal.commands.register("gl", function()
    open_git_log()
end, "Browse git history and inspect commit diffs")
