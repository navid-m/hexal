-- Markdown editing helpers.
--
-- Enter continues a bullet or ordered list, an empty item exits the list, and
-- thematic breaks such as "______", "****" or "- - -" are normalized to "---".
-- Key callbacks return false when the editor should handle the key normally.

local editor = hexal.editor
local keymap = hexal.keymap

local active = true

local function scalar_length(text)
    if type(utf8) == "table" and type(utf8.len) == "function" then
        local ok, length = pcall(utf8.len, text)
        if ok and length ~= nil then
            return length
        end
    end
    return #text
end

-- Returns the fence character, its length and whether the line is only a
-- closing fence. A line with an info string still opens a fence.
local function fence_marker(text)
    local indent, rest = text:match("^(%s*)(.*)$")
    if indent == nil or #indent > 3 then
        return nil
    end

    local marker = rest:match("^(`+)") or rest:match("^(~+)")
    if marker == nil or #marker < 3 then
        return nil
    end

    local after = rest:sub(#marker + 1)
    if marker:sub(1, 1) == "`" and after:find("`", 1, true) ~= nil then
        return nil
    end

    return marker:sub(1, 1), #marker, after:match("^%s*$") ~= nil
end

local function inside_fenced_code(line_number)
    local fence_character
    local fence_length = 0

    for index = 0, line_number do
        local text = editor.get_line(index)
        if text == nil then
            break
        end

        local character, length, closing = fence_marker(text)
        if character ~= nil then
            if fence_character == nil then
                fence_character = character
                fence_length = length
            elseif character == fence_character and length >= fence_length and closing then
                fence_character = nil
                fence_length = 0
            end
        end
    end

    return fence_character ~= nil
end

local function thematic_break(line)
    local indent, rest = line:match("^( *)(.*)$")
    if indent == nil or #indent > 3 then
        return nil
    end

    local compact = rest:gsub("[ \t]", "")
    if #compact < 3 then
        return nil
    end

    local character = compact:sub(1, 1)
    if character ~= "-" and character ~= "_" and character ~= "*" then
        return nil
    end

    for index = 2, #compact do
        if compact:sub(index, index) ~= character then
            return nil
        end
    end

    return indent .. "---"
end

local function list_item(line)
    local indent, marker, spacing, rest = line:match("^(%s*)([-+*])(%s+)(.*)$")
    if marker ~= nil then
        return {
            indent = indent,
            marker = marker,
            spacing = spacing,
            rest = rest,
            ordered = false,
        }
    end

    local number, delimiter
    indent, number, delimiter, spacing, rest = line:match("^(%s*)(%d+)([.)])(%s+)(.*)$")
    if number ~= nil then
        return {
            indent = indent,
            number = number,
            delimiter = delimiter,
            spacing = spacing,
            rest = rest,
            ordered = true,
        }
    end

    return nil
end

local function handle_enter()
    if not active then
        return false
    end

    local cursor = editor.get_cursor()
    if cursor == nil then
        return false
    end

    local line_number = cursor.line
    local line = editor.get_line(line_number)
    if line == nil then
        return false
    end

    local column = cursor.column
    if inside_fenced_code(line_number) then
        return false
    end

    -- Normalize a thematic break when Enter ends the line. Returning false
    -- lets the editor insert the newline with its normal behavior.
    local normalized = thematic_break(line)
    if normalized ~= nil and column >= scalar_length(line) then
        if normalized ~= line then
            editor.replace_range(
                { line = line_number, column = 0 },
                { line = line_number, column = scalar_length(line) },
                normalized
            )
        end
        return false
    end

    local item = list_item(line)
    if item == nil then
        return false
    end

    local prefix
    if item.ordered then
        prefix = item.indent .. item.number .. item.delimiter .. item.spacing
    else
        prefix = item.indent .. item.marker .. item.spacing
    end
    if column < scalar_length(prefix) then
        return false
    end

    -- An empty item ends the list instead of creating another empty bullet.
    if item.rest:match("^%s*$") then
        return editor.replace_range(
            { line = line_number, column = 0 },
            { line = line_number, column = scalar_length(line) },
            ""
        )
    end

    local continuation
    if item.ordered then
        local value = tonumber(item.number)
        if value == nil then
            return false
        end

        local next_number = tostring(value + 1)
        if #next_number < #item.number then
            next_number = string.rep("0", #item.number - #next_number) .. next_number
        end
        continuation = item.indent .. next_number .. item.delimiter .. item.spacing
    else
        continuation = prefix
    end

    editor.insert_text("\n" .. continuation)
    return true
end

local function normalize_document()
    if not active then
        hexal.ui.warn("mdtools: not a Markdown buffer")
        return
    end

    local text = editor.get_text()
    if text == nil then
        return
    end

    local original_cursor = editor.get_cursor()

    local line_count = 0
    for _ in (text .. "\n"):gmatch("(.-)\n") do
        line_count = line_count + 1
    end

    local fence_character
    local fence_length = 0
    local changed = 0

    for index = 0, line_count - 1 do
        local line = editor.get_line(index)
        if line ~= nil then
            local character, length, closing = fence_marker(line)
            local inside = fence_character ~= nil

            if character ~= nil then
                if fence_character == nil then
                    fence_character = character
                    fence_length = length
                elseif character == fence_character and length >= fence_length and closing then
                    fence_character = nil
                    fence_length = 0
                end
            end

            if not inside then
                local normalized = thematic_break(line)
                if normalized ~= nil and normalized ~= line then
                    if
                        editor.replace_range(
                            { line = index, column = 0 },
                            { line = index, column = scalar_length(line) },
                            normalized
                        )
                    then
                        changed = changed + 1
                    end
                end
            end
        end
    end

    if changed > 0 then
        hexal.ui.message(
            string.format(
                "mdtools: normalized %d thematic break%s",
                changed,
                changed == 1 and "" or "s"
            )
        )
    else
        hexal.ui.message("mdtools: no thematic breaks to normalize")
    end

    if original_cursor ~= nil then
        editor.set_cursor(original_cursor.line, original_cursor.column)
    end
end

hexal.events.on("buffer_opened", function(event)
    active = event ~= nil and event.language == "markdown"
end)

hexal.commands.register(
    "mdtools-normalize",
    normalize_document,
    "Normalize Markdown thematic breaks to ---"
)

keymap.map_callback("enter", handle_enter, "insert")
