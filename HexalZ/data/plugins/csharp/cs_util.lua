local M = {}

local utf8 = utf8

local function byte_units(byte)
    if byte < 0x80 then
        return 1, 1
    elseif byte < 0xE0 then
        return 1, 2
    elseif byte < 0xF0 then
        return 1, 3
    end
    return 2, 4
end

function M.source(text)
    text = text or ""
    local line_starts = { 1 }
    local index = 1
    local length = #text
    while true do
        local newline = text:find("\n", index, true)
        if not newline then
            break
        end
        line_starts[#line_starts + 1] = newline + 1
        index = newline + 1
        if index > length then
            break
        end
    end

    local source = {
        text = text,
        size = length,
        line_starts = line_starts,
        line_count = #line_starts,
    }

    function source:line_of(offset)
        offset = math.max(1, math.min(offset or 1, self.size + 1))
        local low = 1
        local high = #self.line_starts
        while low < high do
            local middle = math.floor((low + high + 1) / 2)
            if self.line_starts[middle] <= offset then
                low = middle
            else
                high = middle - 1
            end
        end
        return low - 1
    end

    function source:line_start(line)
        line = math.max(0, math.min(line or 0, #self.line_starts - 1))
        return self.line_starts[line + 1]
    end

    function source:line_end(line)
        local start = self:line_start(line)
        if line + 1 < #self.line_starts then
            local next_start = self.line_starts[line + 2]
            local ending = next_start - 1
            if ending > start and self.text:byte(ending - 1) == 13 then
                ending = ending - 1
            end
            return ending
        end
        return self.size + 1
    end

    function source:line_text(line)
        local start = self:line_start(line)
        local ending = self:line_end(line)
        if ending <= start then
            return ""
        end
        return self.text:sub(start, ending - 1)
    end

    function source:position(offset)
        offset = math.max(1, math.min(offset or 1, self.size + 1))
        local line = self:line_of(offset)
        local start = self:line_start(line)
        local units = 0
        local index = start
        while index < offset do
            local byte = self.text:byte(index) or 0
            local added, width = byte_units(byte)
            units = units + added
            index = index + width
        end
        return { line = line, character = units }
    end

    function source:offset(line, character)
        line = math.max(0, math.min(line or 0, #self.line_starts - 1))
        character = math.max(0, character or 0)
        local start = self:line_start(line)
        local ending = self:line_end(line)
        local units = 0
        local index = start
        while index < ending and units < character do
            local byte = self.text:byte(index) or 0
            local added, width = byte_units(byte)
            if units + added > character then
                break
            end
            units = units + added
            index = index + width
        end
        return math.max(start, math.min(index, ending))
    end

    function source:offset_at(position)
        return self:offset(position and position.line or 0,
            position and position.character or 0)
    end

    function source:range(start_offset, end_offset)
        return {
            start = self:position(start_offset),
            ["end"] = self:position(end_offset),
        }
    end

    function source:position_range(start_position, end_position)
        return {
            start = { line = start_position.line, character = start_position.character },
            ["end"] = { line = end_position.line, character = end_position.character },
        }
    end

    function source:range_text(start_offset, end_offset)
        return self.text:sub(start_offset, math.max(start_offset, end_offset - 1))
    end

    function source:length(start_offset, end_offset)
        local first = self:position(start_offset)
        local last = self:position(end_offset)
        if first.line == last.line then
            return math.max(0, last.character - first.character)
        end
        local total = 0
        for line = first.line, last.line do
            local start_units = line == first.line and first.character or 0
            local end_units = line == last.line and last.character
                or #(self:line_text(line))
            total = total + math.max(0, end_units - start_units)
        end
        return total
    end

    function source:segments(start_offset, end_offset)
        local segments = {}
        local first = self:position(start_offset)
        local last = self:position(end_offset)
        if first.line == last.line then
            segments[#segments + 1] = {
                line = first.line,
                character = first.character,
                length = math.max(0, last.character - first.character),
                start = start_offset,
                finish = end_offset,
            }
            return segments
        end

        for line = first.line, last.line do
            local line_start = self:line_start(line)
            local line_end = self:line_end(line)
            local segment_start = line == first.line and start_offset or line_start
            local segment_end = line == last.line and end_offset or line_end
            local start_position = self:position(segment_start)
            local end_position = self:position(segment_end)
            if end_position.character > start_position.character then
                segments[#segments + 1] = {
                    line = line,
                    character = start_position.character,
                    length = end_position.character - start_position.character,
                    start = segment_start,
                    finish = segment_end,
                }
            end
        end
        return segments
    end

    return source
end

function M.position(line, character)
    return { line = line or 0, character = character or 0 }
end

function M.range(start_position, end_position)
    return {
        start = start_position,
        ["end"] = end_position,
    }
end

function M.compare_positions(left, right)
    if left.line ~= right.line then
        return left.line < right.line and -1 or 1
    end
    if left.character ~= right.character then
        return left.character < right.character and -1 or 1
    end
    return 0
end

function M.compare_ranges(left, right)
    local comparison = M.compare_positions(left.start, right.start)
    if comparison ~= 0 then
        return comparison
    end
    return M.compare_positions(left["end"], right["end"])
end

function M.range_contains(range, position)
    return M.compare_positions(range.start, position) <= 0 and
        M.compare_positions(position, range["end"]) <= 0
end

function M.ranges_intersect(left, right)
    return M.compare_positions(left["end"], right.start) >= 0 and
        M.compare_positions(right["end"], left.start) >= 0
end

function M.range_contains_range(outer, inner)
    return M.compare_positions(outer.start, inner.start) <= 0 and
        M.compare_positions(inner["end"], outer["end"]) <= 0
end

function M.strip_at_identifier(value)
    if value and value:sub(1, 1) == "@" then
        return value:sub(2)
    end
    return value
end

function M.is_identifier(value)
    return type(value) == "string" and
        value:match("^@?[%a_][%w_]*$") ~= nil
end

function M.token_text(source, token)
    if not token then
        return ""
    end
    return source.text:sub(token.start, token["end"] - 1)
end

function M.trim(value)
    return (value:gsub("^%s+", ""):gsub("%s+$", ""))
end

function M.escape_markdown(value)
    return (value:gsub("([\\`%*_{}%[%]%(%)#%+%-%.!])", "\\%1"))
end

function M.is_keyword(value, keywords)
    return value ~= nil and keywords[value] == true
end

function M.clone_list(list)
    local result = {}
    for index, value in ipairs(list or {}) do
        result[index] = value
    end
    return result
end

function M.sorted_keys(table_value)
    local keys = {}
    for key in pairs(table_value or {}) do
        keys[#keys + 1] = key
    end
    table.sort(keys)
    return keys
end

function M.split(value, separator)
    local result = {}
    local pattern = "([^" .. separator .. "]+)"
    for item in value:gmatch(pattern) do
        result[#result + 1] = item
    end
    return result
end

function M.line_indent(text, line)
    local source = M.source(text)
    local line_text = source:line_text(line)
    local indent = line_text:match("^(%s*)") or ""
    return indent
end

function M.first_difference(left, right)
    local limit = math.min(#left, #right)
    local index = 1
    while index <= limit and left:byte(index) == right:byte(index) do
        index = index + 1
    end
    return index
end

return M
