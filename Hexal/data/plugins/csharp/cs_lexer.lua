local M = {}

local reserved = {
    ["abstract"] = true, ["as"] = true, ["base"] = true, ["bool"] = true,
    ["break"] = true, ["byte"] = true, ["case"] = true, ["catch"] = true,
    ["char"] = true, ["checked"] = true, ["class"] = true, ["const"] = true,
    ["continue"] = true, ["decimal"] = true, ["default"] = true,
    ["delegate"] = true, ["do"] = true, ["double"] = true, ["else"] = true,
    ["enum"] = true, ["event"] = true, ["explicit"] = true, ["extern"] = true,
    ["false"] = true, ["finally"] = true, ["fixed"] = true, ["float"] = true,
    ["for"] = true, ["foreach"] = true, ["goto"] = true, ["if"] = true,
    ["implicit"] = true, ["in"] = true, ["int"] = true, ["interface"] = true,
    ["internal"] = true, ["is"] = true, ["lock"] = true, ["long"] = true,
    ["namespace"] = true, ["new"] = true, ["null"] = true, ["object"] = true,
    ["operator"] = true, ["out"] = true, ["override"] = true,
    ["params"] = true, ["private"] = true, ["protected"] = true,
    ["public"] = true, ["readonly"] = true, ["ref"] = true, ["return"] = true,
    ["sbyte"] = true, ["sealed"] = true, ["short"] = true, ["sizeof"] = true,
    ["stackalloc"] = true, ["static"] = true, ["string"] = true,
    ["struct"] = true, ["switch"] = true, ["this"] = true, ["throw"] = true,
    ["true"] = true, ["try"] = true, ["typeof"] = true, ["uint"] = true,
    ["ulong"] = true, ["unchecked"] = true, ["unsafe"] = true, ["ushort"] = true,
    ["using"] = true, ["virtual"] = true, ["void"] = true, ["volatile"] = true,
    ["while"] = true,
}

local contextual = {
    ["add"] = true, ["alias"] = true, ["allows"] = true, ["and"] = true,
    ["ascending"] = true, ["async"] = true, ["await"] = true,
    ["by"] = true, ["descending"] = true, ["dynamic"] = true,
    ["equals"] = true, ["file"] = true, ["from"] = true, ["get"] = true,
    ["global"] = true, ["group"] = true, ["init"] = true, ["into"] = true,
    ["join"] = true, ["let"] = true, ["managed"] = true, ["nameof"] = true,
    ["nint"] = true, ["not"] = true, ["notnull"] = true, ["nuint"] = true,
    ["on"] = true, ["or"] = true, ["orderby"] = true, ["partial"] = true,
    ["record"] = true, ["remove"] = true, ["required"] = true,
    ["scoped"] = true, ["select"] = true, ["set"] = true, ["unmanaged"] = true,
    ["value"] = true, ["var"] = true, ["when"] = true, ["where"] = true,
    ["with"] = true, ["yield"] = true,
}

local modifiers = {
    ["public"] = true, ["private"] = true, ["protected"] = true,
    ["internal"] = true, ["static"] = true, ["abstract"] = true,
    ["sealed"] = true, ["virtual"] = true, ["override"] = true,
    ["readonly"] = true, ["const"] = true, ["async"] = true,
    ["unsafe"] = true, ["extern"] = true, ["partial"] = true, ["new"] = true,
    ["volatile"] = true, ["required"] = true, ["file"] = true,
    ["fixed"] = true, ["ref"] = true, ["scoped"] = true,
    ["implicit"] = true, ["explicit"] = true,
}

local operators = {
    "<<=", ">>=", "??=", "===", "!==", "->", "=>", "==", "!=", "<=", ">=",
    "++", "--", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "<<", ">>", "??", "?.", "::", "..",
}

local single = {}
for character in ("(){}[];,.?:+-*/%&|^!~=<>"):gmatch(".") do
    single[character] = true
end

local function is_ident_start(byte)
    return byte ~= nil and ((byte >= 65 and byte <= 90) or
        (byte >= 97 and byte <= 122) or byte == 95 or byte == 64 or byte >= 128)
end

local function is_ident_continue(byte)
    return byte ~= nil and ((byte >= 65 and byte <= 90) or
        (byte >= 97 and byte <= 122) or (byte >= 48 and byte <= 57) or
        byte == 95 or byte >= 128)
end

local function is_digit(byte)
    return byte ~= nil and byte >= 48 and byte <= 57
end

local function is_hex(byte)
    return byte ~= nil and ((byte >= 48 and byte <= 57) or
        (byte >= 65 and byte <= 70) or (byte >= 97 and byte <= 102))
end

function M.keywords()
    return reserved, contextual, modifiers
end

function M.is_reserved(value)
    return reserved[value] == true
end

function M.is_modifier(value)
    return modifiers[value] == true
end

function M.is_contextual(value)
    return contextual[value] == true
end

function M.is_identifier_token(token)
    if not token then
        return false
    end
    if token.type == "identifier" then
        return true
    end
    return token.type == "keyword" and contextual[token.value] == true and
        reserved[token.value] ~= true
end

function M.lex(text)
    text = text or ""
    local length = #text
    local tokens = {}
    local comments = {}
    local directives = {}
    local errors = {}
    local index = 1

    local function push(kind, value, start_offset, end_offset, extra)
        local token = {
            type = kind,
            value = value,
            start = start_offset,
            ["end"] = end_offset,
        }
        if extra then
            for key, item in pairs(extra) do
                token[key] = item
            end
        end
        tokens[#tokens + 1] = token
        return token
    end

    local function error(message, start_offset, end_offset, code, severity)
        errors[#errors + 1] = {
            start = start_offset,
            ["end"] = end_offset,
            message = message,
            code = code or "CS1002",
            severity = severity or 1,
        }
    end

    local function line_start(offset)
        local cursor = offset
        while cursor > 1 do
            local byte = text:byte(cursor - 1)
            if byte == 10 then
                return true
            elseif byte ~= 32 and byte ~= 9 and byte ~= 13 then
                return false
            end
            cursor = cursor - 1
        end
        return true
    end

    local function scan_identifier(start_offset)
        local cursor = start_offset
        if text:byte(cursor) == 64 then
            cursor = cursor + 1
            if not is_ident_start(text:byte(cursor)) or text:byte(cursor) == 64 then
                return cursor
            end
        end
        while cursor <= length and is_ident_continue(text:byte(cursor)) do
            cursor = cursor + 1
        end
        local raw = text:sub(start_offset, cursor - 1)
        local name = raw
        if name:sub(1, 1) == "@" then
            name = name:sub(2)
        end
        local kind = "identifier"
        if raw:sub(1, 1) ~= "@" and (reserved[name] or contextual[name]) then
            kind = "keyword"
        end
        push(kind, name, start_offset, cursor, { raw = raw })
        return cursor
    end

    local function scan_number(start_offset)
        local cursor = start_offset
        local first = text:byte(cursor)
        local second = text:byte(cursor + 1)
        if first == 48 and (second == 120 or second == 88) then
            cursor = cursor + 2
            while cursor <= length do
                local byte = text:byte(cursor)
                if is_hex(byte) or byte == 95 then
                    cursor = cursor + 1
                else
                    break
                end
            end
            if text:byte(cursor) == 46 then
                cursor = cursor + 1
                while cursor <= length do
                    local byte = text:byte(cursor)
                    if is_hex(byte) or byte == 95 then
                        cursor = cursor + 1
                    else
                        break
                    end
                end
            end
            local exponent = text:byte(cursor)
            if exponent == 112 or exponent == 80 then
                cursor = cursor + 1
                local sign = text:byte(cursor)
                if sign == 43 or sign == 45 then
                    cursor = cursor + 1
                end
                while cursor <= length and (is_digit(text:byte(cursor)) or
                    text:byte(cursor) == 95) do
                    cursor = cursor + 1
                end
            end
        elseif first == 48 and (second == 98 or second == 66) then
            cursor = cursor + 2
            while cursor <= length do
                local byte = text:byte(cursor)
                if byte == 48 or byte == 49 or byte == 95 then
                    cursor = cursor + 1
                else
                    break
                end
            end
        else
            local seen_dot = false
            if first == 46 then
                seen_dot = true
                cursor = cursor + 1
            end
            while cursor <= length and (is_digit(text:byte(cursor)) or
                text:byte(cursor) == 95) do
                cursor = cursor + 1
            end
            if not seen_dot and text:byte(cursor) == 46 and
                text:byte(cursor + 1) ~= 46 then
                cursor = cursor + 1
                while cursor <= length and (is_digit(text:byte(cursor)) or
                    text:byte(cursor) == 95) do
                    cursor = cursor + 1
                end
            end
            local exponent = text:byte(cursor)
            if exponent == 101 or exponent == 69 then
                cursor = cursor + 1
                local sign = text:byte(cursor)
                if sign == 43 or sign == 45 then
                    cursor = cursor + 1
                end
                while cursor <= length and (is_digit(text:byte(cursor)) or
                    text:byte(cursor) == 95) do
                    cursor = cursor + 1
                end
            end
        end

        while cursor <= length do
            local byte = text:byte(cursor)
            if byte == 102 or byte == 70 or byte == 100 or byte == 68 or
                byte == 109 or byte == 77 or byte == 117 or byte == 85 or
                byte == 108 or byte == 76 then
                cursor = cursor + 1
            else
                break
            end
        end

        push("number", text:sub(start_offset, cursor - 1), start_offset, cursor)
        return cursor
    end

    local function scan_regular_string(start_offset, prefix_length, interpolated)
        local cursor = start_offset + prefix_length
        local closed = false
        while cursor <= length do
            local byte = text:byte(cursor)
            if byte == 92 then
                cursor = cursor + 2
            elseif byte == 34 then
                cursor = cursor + 1
                closed = true
                break
            elseif byte == 10 or byte == 13 then
                break
            else
                cursor = cursor + 1
            end
        end
        if not closed then
            error("Unterminated string literal.", start_offset,
                math.min(cursor, length + 1), "CS1039", 1)
        end
        push("string", text:sub(start_offset, cursor - 1), start_offset, cursor,
            { interpolated = interpolated })
        return cursor
    end

    local function scan_verbatim_string(start_offset, prefix_length, interpolated)
        local cursor = start_offset + prefix_length
        local closed = false
        while cursor <= length do
            local byte = text:byte(cursor)
            if byte == 34 then
                if text:byte(cursor + 1) == 34 then
                    cursor = cursor + 2
                else
                    cursor = cursor + 1
                    closed = true
                    break
                end
            else
                cursor = cursor + 1
            end
        end
        if not closed then
            error("Unterminated verbatim string literal.", start_offset,
                math.min(cursor, length + 1), "CS1039", 1)
        end
        push("string", text:sub(start_offset, cursor - 1), start_offset, cursor,
            { interpolated = interpolated, verbatim = true })
        return cursor
    end

    local function scan_raw_string(start_offset, prefix_length, quote_count,
        interpolated)
        local cursor = start_offset + prefix_length + quote_count
        local closed = false
        while cursor <= length do
            if text:byte(cursor) == 34 then
                local run = 0
                while text:byte(cursor + run) == 34 do
                    run = run + 1
                end
                if run >= quote_count then
                    cursor = cursor + run
                    closed = true
                    break
                end
                cursor = cursor + run
            else
                cursor = cursor + 1
            end
        end
        if not closed then
            error("Unterminated raw string literal.", start_offset,
                math.min(cursor, length + 1), "CS1039", 1)
        end
        push("string", text:sub(start_offset, cursor - 1), start_offset, cursor,
            { interpolated = interpolated, raw = true })
        return cursor
    end

    local function scan_char(start_offset)
        local cursor = start_offset + 1
        local closed = false
        while cursor <= length do
            local byte = text:byte(cursor)
            if byte == 92 then
                cursor = cursor + 2
            elseif byte == 39 then
                cursor = cursor + 1
                closed = true
                break
            elseif byte == 10 or byte == 13 then
                break
            else
                cursor = cursor + 1
            end
        end
        if not closed then
            error("Unterminated character literal.", start_offset,
                math.min(cursor, length + 1), "CS1010", 1)
        end
        push("char", text:sub(start_offset, cursor - 1), start_offset, cursor)
        return cursor
    end

    local function scan_string(start_offset)
        local cursor = start_offset
        local byte = text:byte(cursor)
        local next_byte = text:byte(cursor + 1)
        local interpolated = false
        local verbatim = false
        if byte == 36 then
            interpolated = true
            while text:byte(cursor) == 36 do
                cursor = cursor + 1
            end
            byte = text:byte(cursor)
            next_byte = text:byte(cursor + 1)
            if byte == 64 then
                verbatim = true
                cursor = cursor + 1
            end
        elseif byte == 64 then
            verbatim = true
            cursor = cursor + 1
            byte = text:byte(cursor)
            next_byte = text:byte(cursor + 1)
            if byte == 36 then
                interpolated = true
                cursor = cursor + 1
            end
        end

        local prefix_length = cursor - start_offset
        byte = text:byte(cursor)
        next_byte = text:byte(cursor + 1)
        if byte ~= 34 then
            return scan_regular_string(start_offset, 1, interpolated)
        end

        local quote_count = 0
        while text:byte(cursor + quote_count) == 34 do
            quote_count = quote_count + 1
        end
        if quote_count >= 3 then
            return scan_raw_string(start_offset, prefix_length, quote_count,
                interpolated)
        end
        if verbatim then
            return scan_verbatim_string(start_offset, prefix_length + 1,
                interpolated)
        end
        return scan_regular_string(start_offset, prefix_length + 1,
            interpolated)
    end

    if length >= 3 and text:byte(1) == 239 and text:byte(2) == 187 and
        text:byte(3) == 191 then
        index = 4
    end

    while index <= length do
        local byte = text:byte(index)

        if byte == 32 or byte == 9 or byte == 13 or byte == 10 or
            byte == 11 or byte == 12 then
            index = index + 1
        elseif byte == 47 and text:byte(index + 1) == 47 then
            local start_offset = index
            local cursor = index + 2
            while cursor <= length do
                local current = text:byte(cursor)
                if current == 10 or current == 13 then
                    break
                end
                cursor = cursor + 1
            end
            local value = text:sub(start_offset, cursor - 1)
            local is_doc = value:sub(1, 3) == "///" and value:sub(1, 4) ~= "////"
            local comment = push("comment", value, start_offset, cursor, {
                comment_kind = "line",
                doc = is_doc,
            })
            comments[#comments + 1] = comment
            index = cursor
        elseif byte == 47 and text:byte(index + 1) == 42 then
            local start_offset = index
            local cursor = index + 2
            local closed = false
            while cursor <= length do
                if text:byte(cursor) == 42 and text:byte(cursor + 1) == 47 then
                    cursor = cursor + 2
                    closed = true
                    break
                end
                cursor = cursor + 1
            end
            if not closed then
                error("Unterminated block comment.", start_offset,
                    math.min(cursor, length + 1), "CS1035", 1)
                cursor = math.min(cursor, length + 1)
            end
            local value = text:sub(start_offset, cursor - 1)
            local comment = push("comment", value, start_offset, cursor, {
                comment_kind = "block",
                doc = value:sub(1, 3) == "/**" and value:sub(1, 4) ~= "/***",
            })
            comments[#comments + 1] = comment
            index = cursor
        elseif byte == 35 and line_start(index) then
            local start_offset = index
            local cursor = index + 1
            while cursor <= length do
                local current = text:byte(cursor)
                if current == 10 or current == 13 then
                    break
                end
                cursor = cursor + 1
            end
            local value = text:sub(start_offset, cursor - 1)
            local directive = value:match("^#%s*([%a_]+)") or ""
            local item = push("preprocessor", value, start_offset, cursor, {
                directive = directive,
            })
            directives[#directives + 1] = item
            index = cursor
        elseif byte == 34 or byte == 39 or
            ((byte == 36 or byte == 64) and
                (text:byte(index + 1) == 34 or text:byte(index + 1) == 36 or
                    text:byte(index + 1) == 64)) then
            if byte == 39 then
                index = scan_char(index)
            else
                index = scan_string(index)
            end
        elseif is_ident_start(byte) then
            index = scan_identifier(index)
        elseif is_digit(byte) or (byte == 46 and is_digit(text:byte(index + 1))) then
            index = scan_number(index)
        else
            local matched = false
            for _, operator in ipairs(operators) do
                local operator_length = #operator
                if text:sub(index, index + operator_length - 1) == operator then
                    push("operator", operator, index, index + operator_length)
                    index = index + operator_length
                    matched = true
                    break
                end
            end
            if not matched then
                local character = string.char(byte)
                if single[character] then
                    local kind = (character == "(" or character == ")" or
                        character == "{" or character == "}" or
                        character == "[" or character == "]" or
                        character == ";" or character == "," or
                        character == "." or character == ":") and
                        "punctuation" or "operator"
                    push(kind, character, index, index + 1)
                else
                    error(string.format("Unexpected character '%s'.", character),
                        index, index + 1, "CS1056", 1)
                    push("error", character, index, index + 1)
                end
                index = index + 1
            end
        end
    end

    tokens[#tokens + 1] = {
        type = "eof",
        value = "",
        start = length + 1,
        ["end"] = length + 1,
    }

    local significant = {}
    for _, token in ipairs(tokens) do
        if token.type ~= "comment" and token.type ~= "preprocessor" then
            significant[#significant + 1] = token
        end
    end

    return {
        tokens = significant,
        all_tokens = tokens,
        comments = comments,
        directives = directives,
        errors = errors,
    }
end

return M
