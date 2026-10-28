local lexer = require("cs_lexer")
local util = require("cs_util")

local M = {}

local control_words = {
    ["if"] = true, ["for"] = true, ["foreach"] = true, ["while"] = true,
    ["switch"] = true, ["catch"] = true, ["using"] = true, ["lock"] = true,
    ["fixed"] = true, ["typeof"] = true, ["sizeof"] = true,
    ["default"] = true, ["checked"] = true, ["unchecked"] = true,
}

local function indentation(options, level)
    local tab_size = options and options.tabSize or 4
    local insert_spaces = options == nil or options.insertSpaces ~= false
    if insert_spaces then
        return string.rep(" ", math.max(0, level) * tab_size)
    end
    return string.rep("\t", math.max(0, level))
end

local function token_spacing(previous, current)
    if not previous then
        return ""
    end
    local previous_value = previous.value
    local value = current.value
    if value == "," or value == ";" or value == ")" or value == "]" or
        value == "}" or value == "." or value == "::" then
        return ""
    end
    if previous_value == "(" or previous_value == "[" or previous_value == "{" or
        previous_value == "." or previous_value == "::" then
        return ""
    end
    if value == "(" then
        if previous.type == "keyword" and control_words[previous_value] then
            return " "
        end
        return ""
    end
    if value == "[" then
        if previous.type == "identifier" or previous.type == "keyword" or
            previous.type == "string" then
            return ""
        end
        return " "
    end
    if value == "<" or value == ">" then
        return ""
    end
    if previous_value == "<" or previous_value == ">" then
        return ""
    end
    if value == "?" then
        return ""
    end
    if previous_value == "?" then
        return " "
    end
    if value == ":" then
        return ""
    end
    if previous_value == ":" then
        return " "
    end
    if value == "=>" or previous_value == "=>" then
        return " "
    end
    if value == "=" or value == "==" or value == "!=" or value == "<=" or
        value == ">=" or value == "+=" or value == "-=" or value == "*=" or
        value == "/=" or value == "%=" or value == "&&" or value == "||" or
        value == "??" then
        return " "
    end
    if previous_value == "=" or previous_value == "==" or previous_value == "!=" or
        previous_value == "<=" or previous_value == ">=" or
        previous_value == "+=" or previous_value == "-=" or
        previous_value == "*=" or previous_value == "/=" or
        previous_value == "%=" or previous_value == "&&" or
        previous_value == "||" or previous_value == "??" then
        return " "
    end
    if value == "}" then
        return " "
    end
    if previous_value == "," or previous_value == ";" then
        return " "
    end
    return " "
end

local function format_code_line(line, options)
    if line:match("^%s*$") then
        return ""
    end
    local parsed = lexer.lex(line)
    local tokens = {}
    for _, token in ipairs(parsed.all_tokens) do
        if token.type ~= "eof" then
            tokens[#tokens + 1] = token
        end
    end
    if #tokens == 0 then
        return ""
    end
    if tokens[1].type == "preprocessor" then
        return util.trim(line)
    end
    local result = {}
    local previous = nil
    for _, token in ipairs(tokens) do
        if previous then
            local text = line:sub(previous["end"], token.start - 1)
            local whitespace = text:match("^%s*$") ~= nil
            if whitespace then
                result[#result + 1] = token_spacing(previous, token)
            else
                result[#result + 1] = text
            end
        end
        result[#result + 1] = line:sub(token.start, token["end"] - 1)
        previous = token
    end
    return util.trim(table.concat(result))
end

local function protected_lines(model)
    local protected = {}
    for _, token in ipairs(model.all_tokens or {}) do
        if token.type == "string" or (token.type == "comment" and
            token.comment_kind == "block") then
            local first_line = model.source:line_of(token.start)
            local last_line = model.source:line_of(math.max(token.start,
                token["end"] - 1))
            if last_line > first_line then
                for line = first_line, last_line do
                    protected[line] = true
                end
            end
        end
    end
    return protected
end

local function brace_counts(model)
    local counts = {}
    for line = 0, model.source.line_count - 1 do
        counts[line] = { opens = 0, closes = 0, leading = 0, seen = false }
    end
    for _, token in ipairs(model.all_tokens) do
        local line = model.source:line_of(token.start)
        local entry = counts[line]
        if entry then
            if token.type == "preprocessor" then
                entry.seen = true
            elseif token.value == "{" then
                entry.opens = entry.opens + 1
                entry.seen = true
            elseif token.value == "}" then
                entry.closes = entry.closes + 1
                if not entry.seen then
                    entry.leading = entry.leading + 1
                end
                entry.seen = true
            elseif token.type ~= "eof" then
                entry.seen = true
            end
        end
    end
    return counts
end

local function compute_indents(model)
    local protected = protected_lines(model)
    local counts = brace_counts(model)
    local lines = {}
    local depth = 0
    for line = 0, model.source.line_count - 1 do
        local entry = counts[line] or { opens = 0, closes = 0, leading = 0 }
        local level = math.max(0, depth - entry.leading)
        lines[line] = {
            level = level,
            protected = protected[line] == true,
            opens = entry.opens,
            closes = entry.closes,
        }
        depth = math.max(0, depth + entry.opens - entry.closes)
    end
    return lines, protected
end

local function build_edits(model, first_line, last_line, options)
    local indents = compute_indents(model)
    local edits = {}
    for line = first_line, last_line do
        if line >= 0 and line < model.source.line_count and
            not indents[line].protected then
            local original = model.source:line_text(line)
            local trimmed = original:gsub("%s+$", "")
            local content = format_code_line(trimmed, options)
            local formatted
            if content == "" then
                formatted = ""
            else
                formatted = indentation(options, indents[line].level) .. content
            end
            if formatted ~= original then
                local start_offset = model.source:line_start(line)
                local end_offset = model.source:line_end(line)
                edits[#edits + 1] = {
                    range = model.source:range(start_offset, end_offset),
                    newText = formatted,
                }
            end
        end
    end
    return edits
end

function M.format(model, options)
    if not model then
        return {}
    end
    return build_edits(model, 0, model.source.line_count - 1, options or {})
end

function M.range(model, range, options)
    if not model then
        return {}
    end
    local first_line = math.max(0, range.start.line)
    local last_line = math.max(0, range["end"].line)
    return build_edits(model, first_line, last_line, options or {})
end

function M.on_type(model, position, character, options)
    if character ~= "}" then
        return nil
    end
    local offsets = compute_indents(model)
    local line = position.line
    if line < 0 or line >= model.source.line_count then
        return nil
    end
    local original = model.source:line_text(line)
    local content = format_code_line(original:gsub("%s+$", ""), options or {})
    local formatted = indentation(options or {}, offsets[line] and
        offsets[line].level or 0) .. content
    if formatted == original then
        return nil
    end
    return {
        {
            range = model.source:range(model.source:line_start(line),
                model.source:line_end(line)),
            newText = formatted,
        },
    }
end

return M
