local function trim(value)
    return (value:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function split_lines(text)
    local lines = {}
    if text == nil or text == "" then
        return lines
    end

    for line in (text .. "\n"):gmatch("(.-)\n") do
        if line:sub(-1) == "\r" then
            line = line:sub(1, -2)
        end
        lines[#lines + 1] = line
    end

    if #lines > 0 and lines[#lines] == "" then
        lines[#lines] = nil
    end
    return lines
end

local function split_buffer(text)
    local lines = {}
    local position = 1
    while true do
        local newline = text:find("\n", position, true)
        if not newline then
            break
        end

        local line = text:sub(position, newline - 1)
        if line:sub(-1) == "\r" then
            line = line:sub(1, -2)
        end
        lines[#lines + 1] = line
        position = newline + 1
    end
    if position <= #text then
        lines[#lines + 1] = text:sub(position)
    end
    return lines, text:sub(-1) == "\n"
end

local function detect_eol(text)
    if text:find("\r\n", 1, true) then
        return "\r\n"
    end
    return "\n"
end

local function run_git(args, cwd)
    local result = hexal.exec.run("git", args, {
        cwd = cwd,
        timeout = 30000,
    })
    if result.ok then
        return result.stdout or ""
    end

    local message = trim(result.error or "")
    if message == "" then
        message = trim(result.stderr or "")
    end
    if message == "" then
        message = "git command failed"
    end
    return nil, message
end

local function current_path()
    local root = hexal.editor.get_workspace_root()
    local path = hexal.editor.get_current_buffer()
    if path == nil or path == "" or path == "[No Name]" then
        return nil
    end

    if path:sub(1, 7) == "file://" then
        path = path:sub(8)
    end
    if path:sub(1, 1) ~= "/" and not path:match("^%a:[/\\]") then
        if root and root ~= "" then
            path = hexal.fs.join(root, path)
        end
    end
    return hexal.fs.realpath(path)
end

local function repository_info(file)
    local directory = hexal.fs.dirname(file)
    local top, error_message = run_git({ "rev-parse", "--show-toplevel" }, directory)
    if not top then
        return nil, nil, error_message
    end

    top = trim(top)
    local normalized_file = file:gsub("\\", "/")
    local normalized_top = top:gsub("\\", "/")
    if normalized_top:sub(-1) ~= "/" then
        normalized_top = normalized_top .. "/"
    end
    if normalized_file:sub(1, #normalized_top) ~= normalized_top then
        return nil, nil, "the current file is outside the git repository"
    end

    local relative = normalized_file:sub(#normalized_top + 1)
    if relative == "" then
        return nil, nil, "the current buffer is not a file inside the repository"
    end
    return top, relative
end

local function style_for_line(raw)
    if raw:match("^diff %-%-git") or raw:match("^index ") or
       raw:match("^%-%-%- ") or raw:match("^%+%+%+ ") or raw:match("^@@") or
       raw:match("^new file mode") or raw:match("^deleted file mode") or
       raw:match("^old mode") or raw:match("^new mode") or
       raw:match("^similarity index") or raw:match("^rename ") or raw:match("^copy ") then
        return "accent"
    elseif raw:sub(1, 1) == "+" then
        return "diff_added"
    elseif raw:sub(1, 1) == "-" then
        return "diff_deleted"
    elseif raw:sub(1, 1) == "\\" then
        return "muted"
    end
    return "text"
end

local function make_lines(raw_lines)
    local lines = {}
    for index = 1, #raw_lines do
        local raw = raw_lines[index]
        lines[#lines + 1] = {
            segments = { { text = raw, style = style_for_line(raw) } },
            selectable = false,
        }
    end
    return lines
end

local function parse_hunk_header(raw)
    local old_start, old_count, new_start, new_count =
        raw:match("^@@ %-(%d+),?(%d*) %+(%d+),?(%d*) @@")
    if not old_start then
        return nil
    end

    return {
        old_start = tonumber(old_start),
        old_count = old_count == "" and 1 or tonumber(old_count),
        new_start = tonumber(new_start),
        new_count = new_count == "" and 1 or tonumber(new_count),
        header = raw,
        lines = { raw },
        old_lines = {},
        new_lines = {},
    }
end

local function parse_diff(text)
    local header = {}
    local hunks = {}
    local current = nil

    for _, raw in ipairs(split_lines(text)) do
        if raw:sub(1, 2) == "@@" then
            local hunk = parse_hunk_header(raw)
            if hunk then
                current = hunk
                hunks[#hunks + 1] = hunk
            end
        elseif current then
            current.lines[#current.lines + 1] = raw
            local prefix = raw:sub(1, 1)
            if prefix == "-" then
                current.old_lines[#current.old_lines + 1] = raw:sub(2)
            elseif prefix == "+" then
                current.new_lines[#current.new_lines + 1] = raw:sub(2)
            elseif prefix == " " then
                local line = raw:sub(2)
                current.old_lines[#current.old_lines + 1] = line
                current.new_lines[#current.new_lines + 1] = line
            end
        else
            header[#header + 1] = raw
        end
    end

    return header, hunks
end

local function selection_range(selection)
    local first = selection.start_line
    local last = selection.end_line
    -- A characterwise selection that ends at column zero does not actually
    -- cover that final line.
    if last > first and selection.end_column == 0 and not selection.linewise then
        last = last - 1
    end
    return first, last
end

local function hunk_intersects(hunk, first, last)
    if hunk.new_count > 0 then
        local hunk_first = hunk.new_start - 1
        local hunk_last = hunk_first + hunk.new_count - 1
        return hunk_last >= first and hunk_first <= last
    end

    -- Pure deletions have no lines in the new file. Treat the position
    -- between lines as part of an adjacent selected line.
    local insertion = hunk.new_start
    return insertion >= first and insertion <= last + 1
end

local function hunk_position(hunk)
    if hunk.new_count > 0 then
        return hunk.new_start
    end
    return hunk.new_start + 1
end

-- Applies zero-context hunks to a line array. Working on whole lines keeps
-- CRLF handling and missing final newlines predictable.
local function apply_hunk(lines, hunk)
    local position = hunk_position(hunk)
    position = math.max(1, math.min(position, #lines + 1))

    local remove = math.min(hunk.new_count, #lines - position + 1)
    for _ = 1, math.max(0, remove) do
        table.remove(lines, position)
    end
    for offset, line in ipairs(hunk.old_lines) do
        table.insert(lines, position + offset - 1, line)
    end
end

local function revert_selection(hunks, text, eol)
    local lines, trailing_newline = split_buffer(text)
    local ordered = {}
    for index, hunk in ipairs(hunks) do
        ordered[index] = hunk
    end
    table.sort(ordered, function(left, right)
        return hunk_position(left) > hunk_position(right)
    end)

    for _, hunk in ipairs(ordered) do
        apply_hunk(lines, hunk)
    end

    local updated = table.concat(lines, eol)
    if trailing_newline and #lines > 0 then
        updated = updated .. eol
    end
    return updated
end

local function open_git_diff()
    local file = current_path()
    if not file then
        hexal.ui.warn("gitdiff: the current buffer is not a saved file")
        return
    end

    local repository, relative, error_message = repository_info(file)
    if not repository then
        hexal.ui.warn("gitdiff: " .. (error_message or "not inside a git repository"))
        return
    end

    local current_text = hexal.editor.get_text()
    local selection = hexal.editor.get_selection()
    local arguments = {
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-renames",
    }
    if selection then
        arguments[#arguments + 1] = "--unified=0"
    end
    arguments[#arguments + 1] = "--"
    arguments[#arguments + 1] = relative

    local output, diff_error = run_git(arguments, repository)
    if not output then
        hexal.ui.warn("gitdiff: " .. (diff_error or "git diff failed"))
        return
    end
    if trim(output) == "" then
        hexal.ui.message("No unstaged changes for " .. hexal.fs.basename(file))
        return
    end

    local header, hunks = parse_diff(output)
    local raw_lines = nil
    local revert_hunks = nil
    local title = " Git diff · " .. hexal.fs.basename(file) .. " "

    if selection then
        local first, last = selection_range(selection)
        revert_hunks = {}
        raw_lines = {}
        for _, line in ipairs(header) do
            raw_lines[#raw_lines + 1] = line
        end
        for _, hunk in ipairs(hunks) do
            if hunk_intersects(hunk, first, last) then
                revert_hunks[#revert_hunks + 1] = hunk
                for _, line in ipairs(hunk.lines) do
                    raw_lines[#raw_lines + 1] = line
                end
            end
        end

        if #revert_hunks == 0 then
            hexal.ui.message("No changes on the selected lines.")
            return
        end
        title = string.format(" Git diff · %s · lines %d-%d ",
            hexal.fs.basename(file), first + 1, last + 1)
    else
        raw_lines = split_lines(output)
    end

    local eol = detect_eol(current_text)
    local panel

    local function revert()
        local cursor = hexal.editor.get_cursor()
        local text = hexal.editor.get_text()

        if selection then
            if text ~= current_text then
                panel.close()
                hexal.ui.warn("gitdiff: the buffer changed while the diff was open")
                return
            end

            local updated = revert_selection(revert_hunks, text, eol)
            hexal.editor.replace_range(0, #text, updated)
            hexal.editor.set_cursor(cursor.line, cursor.column)
            panel.close()
            hexal.ui.message("Reverted selected changes · press U to redo")
            return
        end

        local index_text, index_error = run_git({ "show", ":" .. relative }, repository)
        if not index_text then
            hexal.ui.warn("gitdiff: cannot read the index version: " ..
                (index_error or "git show failed"))
            return
        end

        hexal.editor.replace_range(0, #text, index_text)
        hexal.editor.set_cursor(cursor.line, cursor.column)
        panel.close()
        hexal.ui.message("Reverted changes · press U to redo")
    end

    panel = hexal.ui.panel({
        title = title,
        footer = "j/k scroll · u revert · q close",
        selectable = false,
        selected = 1,
        lines = make_lines(raw_lines),
    }, function(key)
        if key == "u" or key == "U" then
            revert()
            return true
        end
        if key == "q" or key == "Q" then
            panel.close()
            return true
        end
        return false
    end)
end

hexal.commands.register("gd", open_git_diff,
    "Show the git diff for the current file or selection and revert changes")
