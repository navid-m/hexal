local util = require("cs_util")
local lexer = require("cs_lexer")

local P = {}

local type_keywords = {
    ["class"] = true,
    ["struct"] = true,
    ["interface"] = true,
    ["enum"] = true,
    ["record"] = true,
    ["delegate"] = true,
}

local builtin_types = {
    ["bool"] = true, ["byte"] = true, ["char"] = true, ["decimal"] = true,
    ["double"] = true, ["float"] = true, ["int"] = true, ["long"] = true,
    ["object"] = true, ["sbyte"] = true, ["short"] = true, ["string"] = true,
    ["uint"] = true, ["ulong"] = true, ["ushort"] = true, ["void"] = true,
    ["dynamic"] = true, ["nint"] = true, ["nuint"] = true,
}

local function angle_delta(value)
    local delta = 0
    if value then
        for character in value:gmatch("[<>]") do
            if character == "<" then
                delta = delta + 1
            else
                delta = delta - 1
            end
        end
    end
    return delta
end

local function sorted_name(values)
    local result = {}
    for _, value in ipairs(values) do
        if value then
            result[#result + 1] = value
        end
    end
    return table.concat(result, ".")
end

local function needs_space(previous, current)
    if previous == nil then
        return false
    end
    if current == "," or current == ";" or current == ")" or current == "]" or
        current == ">" or current == "?" or current == "." or current == "::" then
        return false
    end
    if previous == "(" or previous == "[" or previous == "<" or previous == "." or
        previous == "::" then
        return false
    end
    if current == "(" or current == "[" then
        return false
    end
    if current == "<" then
        return false
    end
    if previous == "=" or previous == ":" or previous == "|" or previous == "&" or
        previous == "*" or previous == "?" then
        return true
    end
    if current == "*" then
        return false
    end
    return true
end

local function render_values(values)
    local result = {}
    local previous = nil
    for _, value in ipairs(values) do
        if value ~= "" then
            if needs_space(previous, value) then
                result[#result + 1] = " "
            end
            result[#result + 1] = value
            previous = value
        end
    end
    return table.concat(result)
end

local function token_values(tokens, from, to)
    local values = {}
    for index = from, to do
        if tokens[index] then
            values[#values + 1] = tokens[index].value
        end
    end
    return values
end

local function create_parser(text, uri)
    local parsed = lexer.lex(text)
    local source = util.source(text)
    local parser = {
        text = text,
        uri = uri or "",
        source = source,
        tokens = parsed.tokens,
        all_tokens = parsed.all_tokens,
        comments = parsed.comments,
        directives = parsed.directives,
        errors = parsed.errors,
        index = 1,
    }

    function parser:peek(offset)
        return self.tokens[self.index + (offset or 0)]
    end

    function parser:value(offset)
        local token = self:peek(offset)
        return token and token.value or nil
    end

    function parser:type(offset)
        local token = self:peek(offset)
        return token and token.type or nil
    end

    function parser:advance()
        local token = self.tokens[self.index]
        if token then
            self.index = self.index + 1
        end
        return token
    end

    function parser:eof()
        local token = self.tokens[self.index]
        return token == nil or token.type == "eof"
    end

    function parser:last_end()
        local token = self.tokens[math.max(1, self.index - 1)]
        return token and token["end"] or 1
    end

    function parser:error(message, token, code, severity)
        token = token or self:peek()
        self.errors[#self.errors + 1] = {
            start = token and token.start or 1,
            ["end"] = token and token["end"] or 1,
            message = message,
            code = code or "CS1002",
            severity = severity or 1,
        }
    end

    function parser:is_identifier(token)
        token = token or self:peek()
        if not token then
            return false
        end
        if token.type == "identifier" then
            return true
        end
        return token.type == "keyword" and
            (lexer.is_contextual(token.value) or builtin_types[token.value] or
                token.value == "this" or token.value == "base")
    end

    function parser:skip_balanced(open_value, close_value)
        local opening = self:advance()
        local depth = 1
        local start_offset = opening and opening.start or 1
        local end_offset = opening and opening["end"] or 1
        while not self:eof() do
            local token = self:peek()
            if token.value == open_value then
                depth = depth + 1
                end_offset = token["end"]
                self:advance()
            elseif token.value == close_value then
                depth = depth - 1
                end_offset = token["end"]
                self:advance()
                if depth == 0 then
                    return start_offset, end_offset, token
                end
            else
                end_offset = token["end"]
                self:advance()
            end
        end
        self:error(string.format("Expected '%s'.", close_value), opening,
            "CS1513", 1)
        return start_offset, end_offset, nil
    end

    function parser:skip_attributes()
        local start_offset = self:peek() and self:peek().start
        while self:value() == "[" do
            self:skip_balanced("[", "]")
        end
        return start_offset
    end

    function parser:skip_until_boundary(stop_values)
        while not self:eof() do
            local value = self:value()
            if stop_values[value] then
                return
            end
            if value == "{" or value == "}" or value == ";" then
                return
            end
            if value == "(" then
                self:skip_balanced("(", ")")
            elseif value == "[" then
                self:skip_balanced("[", "]")
            else
                self:advance()
            end
        end
    end

    function parser:consume_modifiers()
        local modifiers = {}
        while lexer.is_modifier(self:value()) do
            modifiers[#modifiers + 1] = self:advance().value
        end
        return modifiers
    end

    function parser:is_type_start(offset)
        local cursor = self.index + (offset or 0)
        while self.tokens[cursor] and lexer.is_modifier(self.tokens[cursor].value) do
            cursor = cursor + 1
        end
        local token = self.tokens[cursor]
        if not token then
            return false
        end
        if token.value == "record" then
            return true
        end
        return type_keywords[token.value] == true
    end

    function parser:parse_type_parameters()
        local names = {}
        if self:value() ~= "<" then
            return names
        end
        self:advance()
        local depth = 1
        while not self:eof() and depth > 0 do
            local token = self:advance()
            if not token then
                break
            end
            if token.value == "<" then
                depth = depth + 1
            elseif token.value == ">" then
                depth = depth - 1
            elseif token.value == ">>" then
                depth = depth - 2
            elseif depth == 1 and self:is_identifier(token) then
                names[#names + 1] = token.value:gsub("^@", "")
            end
        end
        return names
    end

    function parser:parse_parameters()
        local parameters = {}
        if self:value() ~= "(" then
            return parameters
        end
        local open_token = self:advance()
        local paren_depth = 0
        local bracket_depth = 0
        local angle_depth = 0
        local segment_start = nil
        local segment_tokens = {}

        local function commit()
            if #segment_tokens == 0 then
                return
            end
            local name_index = nil
            local equals_index = nil
            local first = 1
            while first <= #segment_tokens and
                segment_tokens[first].value == "[" do
                local depth = 1
                first = first + 1
                while first <= #segment_tokens and depth > 0 do
                    if segment_tokens[first].value == "[" then
                        depth = depth + 1
                    elseif segment_tokens[first].value == "]" then
                        depth = depth - 1
                    end
                    first = first + 1
                end
            end
            local skip_modifiers = {
                ["this"] = true, ["ref"] = true, ["out"] = true, ["in"] = true,
                ["params"] = true, ["scoped"] = true, ["readonly"] = true,
            }
            while first <= #segment_tokens and
                skip_modifiers[segment_tokens[first].value] do
                first = first + 1
            end
            for index = first, #segment_tokens do
                local token = segment_tokens[index]
                if token.value == "=" and not equals_index then
                    equals_index = index
                    break
                end
                if self:is_identifier(token) and token.value ~= "this" then
                    name_index = index
                end
            end
            if name_index then
                local type_values = {}
                local type_end = (equals_index or name_index) - 1
                for index = first, type_end do
                    local value = segment_tokens[index].value
                    if index ~= name_index then
                        type_values[#type_values + 1] = value
                    end
                end
                local name_token = segment_tokens[name_index]
                local parameter = {
                    name = name_token.value:gsub("^@", ""),
                    type = render_values(type_values),
                    start = segment_tokens[1].start,
                    ["end"] = segment_tokens[#segment_tokens]["end"],
                    nameStart = name_token.start,
                    nameEnd = name_token["end"],
                    modifiers = {},
                }
                for index = 1, first - 1 do
                    parameter.modifiers[#parameter.modifiers + 1] =
                        segment_tokens[index].value
                end
                parameter.default = equals_index ~= nil
                parameters[#parameters + 1] = parameter
            end
        end

        while not self:eof() do
            local token = self:peek()
            if token.value == ")" and paren_depth == 0 and bracket_depth == 0 and
                angle_depth == 0 then
                commit()
                self:advance()
                return parameters, token
            end
            if token.value == "(" then
                paren_depth = paren_depth + 1
            elseif token.value == ")" then
                paren_depth = paren_depth - 1
            elseif token.value == "[" then
                bracket_depth = bracket_depth + 1
            elseif token.value == "]" then
                bracket_depth = bracket_depth - 1
            elseif token.value == "<" then
                angle_depth = angle_depth + 1
            elseif token.value == ">" then
                angle_depth = math.max(0, angle_depth - 1)
            elseif token.value == ">>" then
                angle_depth = math.max(0, angle_depth - 2)
            end
            if token.value == "," and paren_depth == 0 and bracket_depth == 0 and
                angle_depth == 0 then
                commit()
                segment_tokens = {}
                segment_start = nil
            else
                if not segment_start then
                    segment_start = token.start
                end
                segment_tokens[#segment_tokens + 1] = token
            end
            self:advance()
        end

        commit()
        self:error("Expected ')'.", open_token, "CS1026", 1)
        return parameters, nil
    end

    function parser:parse_base_types()
        local bases = {}
        if self:value() ~= ":" then
            return bases
        end
        self:advance()
        local segment = {}
        local angle_depth = 0
        local paren_depth = 0
        local function commit()
            if #segment == 0 then
                return
            end
            local values = {}
            for _, token in ipairs(segment) do
                values[#values + 1] = token.value
            end
            bases[#bases + 1] = {
                name = render_values(values):gsub("%s+", ""),
                start = segment[1].start,
                ["end"] = segment[#segment]["end"],
            }
            segment = {}
        end
        while not self:eof() do
            local token = self:peek()
            if token.value == "{" or token.value == ";" or
                (token.value == "where" and angle_depth == 0) then
                break
            end
            if token.value == "<" then
                angle_depth = angle_depth + 1
            elseif token.value == ">" then
                angle_depth = math.max(0, angle_depth - 1)
            elseif token.value == ">>" then
                angle_depth = math.max(0, angle_depth - 2)
            elseif token.value == "(" then
                paren_depth = paren_depth + 1
            elseif token.value == ")" then
                paren_depth = math.max(0, paren_depth - 1)
            end
            if token.value == "," and angle_depth == 0 and paren_depth == 0 then
                commit()
            else
                segment[#segment + 1] = token
            end
            self:advance()
        end
        commit()
        return bases
    end

    function parser:skip_constraints()
        while self:value() == "where" do
            while not self:eof() do
                local value = self:value()
                if value == "{" or value == ";" or value == "=>" then
                    return
                end
                if value == "(" then
                    self:skip_balanced("(", ")")
                elseif value == "[" then
                    self:skip_balanced("[", "]")
                else
                    self:advance()
                end
            end
        end
    end

    function parser:parse_accessors()
        local accessors = {}
        if self:value() ~= "{" then
            return accessors, nil
        end
        local open_token = self:advance()
        while not self:eof() and self:value() ~= "}" do
            self:skip_attributes()
            while lexer.is_modifier(self:value()) and self:value() ~= "async" do
                self:advance()
            end
            local keyword = self:peek()
            if not keyword then
                break
            end
            if keyword.value == "get" or keyword.value == "set" or
                keyword.value == "init" or keyword.value == "add" or
                keyword.value == "remove" then
                self:advance()
                local accessor = {
                    name = keyword.value,
                    start = keyword.start,
                    ["end"] = keyword["end"],
                }
                if self:value() == "{" then
                    local body_start, body_end = self:skip_balanced("{", "}")
                    accessor.bodyStart = body_start
                    accessor.bodyEnd = body_end
                    accessor["end"] = body_end
                elseif self:value() == "=>" then
                    self:advance()
                    local expression_start = self:peek() and self:peek().start
                    self:skip_until_boundary({ [";"] = true })
                    if self:value() == ";" then
                        accessor["end"] = self:advance().start
                    end
                    accessor.expressionStart = expression_start
                elseif self:value() == ";" then
                    self:advance()
                end
                accessors[#accessors + 1] = accessor
            else
                self:advance()
            end
        end
        local close_token = nil
        if self:value() == "}" then
            close_token = self:advance()
        else
            self:error("Expected '}'.", open_token, "CS1513", 1)
        end
        return accessors, close_token
    end

    function parser:find_name_before(index)
        local cursor = index - 1
        if self.tokens[cursor] and self.tokens[cursor].value == ">" then
            local depth = 0
            while cursor >= 1 do
                local value = self.tokens[cursor].value
                if value == ">" then
                    depth = depth + 1
                elseif value == ">>" then
                    depth = depth + 2
                elseif value == "<" then
                    depth = depth - 1
                    if depth == 0 then
                        cursor = cursor - 1
                        break
                    end
                end
                cursor = cursor - 1
            end
        elseif self.tokens[cursor] and self.tokens[cursor].value == ">>" then
            cursor = cursor - 1
            if self.tokens[cursor] and self.tokens[cursor].value == ">" then
                cursor = cursor - 1
            end
        end
        return self.tokens[cursor]
    end

    function parser:parse_field_declarators(type_start_index, start_offset,
        modifiers, is_event)
        local type_end_index = nil
        local name_index = self.index
        local angle_depth = 0
        local bracket_depth = 0
        local paren_depth = 0
        while self.tokens[name_index] do
            local token = self.tokens[name_index]
            local value = token.value
            if value == "<" then
                angle_depth = angle_depth + 1
            elseif value == ">" then
                angle_depth = math.max(0, angle_depth - 1)
            elseif value == ">>" then
                angle_depth = math.max(0, angle_depth - 2)
            elseif value == "[" then
                bracket_depth = bracket_depth + 1
            elseif value == "]" then
                bracket_depth = math.max(0, bracket_depth - 1)
            elseif value == "(" then
                paren_depth = paren_depth + 1
            elseif value == ")" then
                paren_depth = math.max(0, paren_depth - 1)
            elseif angle_depth == 0 and bracket_depth == 0 and paren_depth == 0 then
                if self:is_identifier(token) and token.value ~= "this" then
                    local next_value = self.tokens[name_index + 1] and
                        self.tokens[name_index + 1].value
                    if next_value == "=" or next_value == ";" or
                        next_value == "," or next_value == "[" then
                        type_end_index = name_index - 1
                        break
                    end
                end
            end
            name_index = name_index + 1
        end
        if not type_end_index then
            type_end_index = self.index - 1
        end
        local type_values = token_values(self.tokens, type_start_index,
            type_end_index)
        local type_name = render_values(type_values)
        self.index = name_index
        local fields = {}

        local function parse_declarator()
            local name_token = self:peek()
            if not name_token or not self:is_identifier(name_token) then
                return nil
            end
            self:advance()
            local node = {
                kind = is_event and "event" or "field",
                name = name_token.value:gsub("^@", ""),
                type = type_name,
                start = start_offset,
                ["end"] = name_token["end"],
                nameStart = name_token.start,
                nameEnd = name_token["end"],
                modifiers = modifiers,
                initializer = nil,
            }
            if self:value() == "[" then
                local _, end_offset = self:skip_balanced("[", "]")
                node["end"] = end_offset
            end
            if self:value() == "=" then
                self:advance()
                local initializer_start = self:peek() and self:peek().start
                local initializer_end = initializer_start
                local depth = 0
                while not self:eof() do
                    local value = self:value()
                    if value == "(" or value == "[" or value == "{" then
                        depth = depth + 1
                    elseif value == ")" or value == "]" or value == "}" then
                        depth = depth - 1
                    elseif (value == "," or value == ";") and depth == 0 then
                        break
                    end
                    local token = self:advance()
                    initializer_end = token["end"]
                end
                node.initializer = self.text:sub(initializer_start,
                    math.max(initializer_start, initializer_end - 1))
                node["end"] = initializer_end
            end
            fields[#fields + 1] = node
            return node
        end

        while not self:eof() do
            local node = parse_declarator()
            if not node then
                break
            end
            if self:value() == "," then
                self:advance()
            else
                break
            end
        end
        if self:value() == ";" then
            self:advance()
        else
            self:error("Expected ';'.", self:peek(), "CS1002", 1)
        end
        for _, node in ipairs(fields) do
            node["end"] = math.max(node["end"], self:last_end())
        end
        return fields
    end

    function parser:parse_property_like(member_start, modifiers, is_event,
        start_index, name_index)
        local name_token = self.tokens[name_index]
        if not name_token then
            return nil
        end
        local type_values = token_values(self.tokens, start_index, name_index - 1)
        local node = {
            kind = is_event and "event" or "property",
            name = name_token.value:gsub("^@", ""),
            type = render_values(type_values),
            start = member_start,
            ["end"] = name_token["end"],
            nameStart = name_token.start,
            nameEnd = name_token["end"],
            modifiers = modifiers,
            accessors = {},
        }
        self.index = name_index + 1
        if self:value() == "{" then
            local accessors, close_token = self:parse_accessors()
            node.accessors = accessors
            if close_token then
                node["end"] = close_token["end"]
            end
            if self:value() == "=" then
                self:advance()
                local initializer_start = self:peek() and self:peek().start
                self:skip_until_boundary({ [";"] = true })
                if self:value() == ";" then
                    local semicolon = self:advance()
                    node["end"] = semicolon["end"]
                end
                node.initializer = initializer_start and self.text:sub(
                    initializer_start, math.max(initializer_start,
                        node["end"] - 1)) or nil
            end
        elseif self:value() == "=>" then
            self:advance()
            self:skip_until_boundary({ [";"] = true })
            if self:value() == ";" then
                node["end"] = self:advance()["end"]
            end
        elseif self:value() == ";" then
            node["end"] = self:advance()["end"]
        else
            node["end"] = self:last_end()
        end
        return node
    end

    function parser:parse_indexer(member_start, modifiers)
        self:advance()
        local indexer = {
            kind = "indexer",
            name = "this",
            type = "",
            start = member_start,
            ["end"] = member_start,
            nameStart = member_start,
            nameEnd = member_start,
            modifiers = modifiers,
            accessors = {},
        }
        if self:value() == "[" then
            local open_token = self:peek()
            local _, end_offset = self:skip_balanced("[", "]")
            indexer.parameterStart = open_token.start + 1
            indexer.parameterEnd = end_offset - 1
        end
        if self:value() == "{" then
            local accessors, close_token = self:parse_accessors()
            indexer.accessors = accessors
            if close_token then
                indexer["end"] = close_token["end"]
            end
        elseif self:value() == "=>" then
            self:advance()
            self:skip_until_boundary({ [";"] = true })
            if self:value() == ";" then
                indexer["end"] = self:advance()["end"]
            end
        end
        return indexer
    end

    function parser:parse_members(type_node)
        local members = {}
        while not self:eof() and self:value() ~= "}" do
            local attribute_start = self:skip_attributes()
            local member_start = attribute_start or
                (self:peek() and self:peek().start) or self:last_end()

            if self:is_type_start(0) then
                local nested = self:parse_type()
                if nested then
                    nested.start = math.min(nested.start, member_start)
                    members[#members + 1] = nested
                end
            else
                local modifiers = self:consume_modifiers()
                local is_event = false
                if self:value() == "event" then
                    is_event = true
                    self:advance()
                end

                if self:value() == "~" then
                    self:advance()
                    local name_token = self:advance()
                    local parameters = {}
                    if self:value() == "(" then
                        parameters = self:parse_parameters()
                    end
                    if self:value() == "{" then
                        local body_start, body_end = self:skip_balanced("{", "}")
                        local node = {
                            kind = "destructor",
                            name = name_token and name_token.value or "~",
                            type = "",
                            start = member_start,
                            ["end"] = body_end,
                            nameStart = name_token and name_token.start or member_start,
                            nameEnd = name_token and name_token["end"] or member_start,
                            modifiers = modifiers,
                            params = parameters,
                            bodyStart = body_start,
                            bodyEnd = body_end,
                        }
                        members[#members + 1] = node
                    elseif self:value() == ";" then
                        self:advance()
                    end
                elseif self:value() == "this" and self:peek(1) and
                    self:peek(1).value == "[" then
                    local node = self:parse_indexer(member_start, modifiers)
                    if node then
                        members[#members + 1] = node
                    end
                else
                    local start_index = self.index
                    local shape, boundary_index = self:scan_member_shape()
                    if shape == "close" then
                        break
                    elseif shape == "params" then
                        local name_token = self:find_name_before(boundary_index)
                        if name_token then
                            local name_index = self.index
                            for index = self.index, boundary_index - 1 do
                                if self.tokens[index] == name_token then
                                    name_index = index
                                    break
                                end
                            end
                            local type_values = token_values(self.tokens,
                                start_index, name_index - 1)
                            local explicit_operator = nil
                            local public_name = name_token.value:gsub("^@", "")
                            local name_start = name_token.start
                            for index = start_index, name_index do
                                local value = self.tokens[index].value
                                if value == "operator" or value == "implicit" or
                                    value == "explicit" then
                                    explicit_operator = index
                                    break
                                end
                            end
                            if explicit_operator then
                                local values = token_values(self.tokens,
                                    explicit_operator, name_index)
                                public_name = render_values(values)
                                name_start = self.tokens[explicit_operator].start
                                type_values = {}
                            end
                            self.index = boundary_index
                            local parameters = self:parse_parameters()
                            self:skip_constraints()
                            local body_start = nil
                            local body_end = nil
                            local expression_body = false
                            if self:value() == "{" then
                                body_start, body_end = self:skip_balanced("{", "}")
                            elseif self:value() == "=>" then
                                expression_body = true
                                self:advance()
                                self:skip_until_boundary({ [";"] = true })
                                if self:value() == ";" then
                                    body_end = self:advance()["end"]
                                end
                            elseif self:value() == ";" then
                                body_end = self:advance()["end"]
                            end

                            local kind = "method"
                            if not explicit_operator and
                                (name_token.value == type_node.name or
                                    (#type_values == 0 and name_token.value ~=
                                        "operator")) then
                                kind = "constructor"
                            end
                            local node = {
                                kind = kind,
                                name = public_name,
                                type = render_values(type_values),
                                start = member_start,
                                ["end"] = body_end or self:last_end(),
                                nameStart = name_start,
                                nameEnd = name_token["end"],
                                modifiers = modifiers,
                                params = parameters,
                                bodyStart = body_start,
                                bodyEnd = body_end,
                                expressionBody = expression_body,
                            }
                            members[#members + 1] = node
                        else
                            self.index = boundary_index + 1
                        end
                    elseif shape == "body" or shape == "arrow" then
                        local name_token = self:find_name_before(boundary_index)
                        local name_index = nil
                        for index = self.index, boundary_index - 1 do
                            if self.tokens[index] == name_token then
                                name_index = index
                                break
                            end
                        end
                        if name_token and name_index then
                            local node = self:parse_property_like(member_start,
                                modifiers, is_event, start_index, name_index)
                            if node then
                                members[#members + 1] = node
                            end
                        else
                            self.index = boundary_index + 1
                        end
                    elseif shape == "equals" or shape == "semicolon" then
                        local fields = self:parse_field_declarators(start_index,
                            member_start, modifiers, is_event)
                        for _, field in ipairs(fields) do
                            members[#members + 1] = field
                        end
                    else
                        self:advance()
                    end
                end
            end
        end
        if self:value() == "}" then
            return members, self:advance()
        end
        return members, nil
    end

    function parser:scan_member_shape()
        local cursor = self.index
        local angle_depth = 0
        local bracket_depth = 0
        while self.tokens[cursor] do
            local token = self.tokens[cursor]
            local value = token.value
            if value == "<" then
                angle_depth = angle_depth + 1
            elseif value == ">" then
                angle_depth = math.max(0, angle_depth - 1)
            elseif value == ">>" then
                angle_depth = math.max(0, angle_depth - 2)
            elseif value == "[" then
                bracket_depth = bracket_depth + 1
            elseif value == "]" then
                bracket_depth = math.max(0, bracket_depth - 1)
            elseif angle_depth == 0 and bracket_depth == 0 then
                if value == "(" then
                    return "params", cursor
                elseif value == "{" then
                    return "body", cursor
                elseif value == "=>" then
                    return "arrow", cursor
                elseif value == "=" then
                    return "equals", cursor
                elseif value == ";" then
                    return "semicolon", cursor
                elseif value == "}" then
                    return "close", cursor
                end
            end
            cursor = cursor + 1
        end
        return "eof", cursor
    end

    function parser:parse_enum_members()
        local members = {}
        while not self:eof() and self:value() ~= "}" do
            self:skip_attributes()
            local name_token = self:peek()
            if not name_token then
                break
            end
            if not self:is_identifier(name_token) then
                self:advance()
            else
                self:advance()
                local node = {
                    kind = "enumMember",
                    name = name_token.value:gsub("^@", ""),
                    type = "enum",
                    start = name_token.start,
                    ["end"] = name_token["end"],
                    nameStart = name_token.start,
                    nameEnd = name_token["end"],
                    modifiers = {},
                }
                if self:value() == "=" then
                    self:advance()
                    while not self:eof() and self:value() ~= "," and
                        self:value() ~= "}" do
                        if self:value() == "(" or self:value() == "[" then
                            local open = self:value()
                            local close = open == "(" and ")" or "]"
                            self:skip_balanced(open, close)
                        else
                            node["end"] = self:advance()["end"]
                        end
                    end
                end
                members[#members + 1] = node
                if self:value() == "," then
                    self:advance()
                end
            end
        end
        local close_token = nil
        if self:value() == "}" then
            close_token = self:advance()
        end
        return members, close_token
    end

    function parser:parse_type()
        local start_token = self:peek()
        if not start_token then
            return nil
        end
        local start_offset = start_token.start
        local modifiers = self:consume_modifiers()
        local kind_token = self:peek()
        if not kind_token then
            return nil
        end
        local kind = kind_token.value
        local record_kind = nil
        if kind == "record" then
            self:advance()
            if self:value() == "class" or self:value() == "struct" then
                record_kind = self:advance().value
            end
            kind = "record"
        else
            self:advance()
        end

        local node = {
            kind = kind,
            name = "",
            type = "",
            start = start_offset,
            ["end"] = kind_token["end"],
            nameStart = kind_token.start,
            nameEnd = kind_token["end"],
            modifiers = modifiers,
            children = {},
            baseTypes = {},
            typeParameters = {},
            recordKind = record_kind,
        }

        if kind == "delegate" then
            local name_index = nil
            local cursor = self.index
            local angle_depth = 0
            while self.tokens[cursor] do
                local value = self.tokens[cursor].value
                if value == "<" then
                    angle_depth = angle_depth + 1
                elseif value == ">" then
                    angle_depth = math.max(0, angle_depth - 1)
                elseif value == "(" and angle_depth == 0 then
                    name_index = cursor - 1
                    break
                elseif value == ";" and angle_depth == 0 then
                    break
                end
                cursor = cursor + 1
            end
            if name_index and self.tokens[name_index] then
                local name_token = self.tokens[name_index]
                node.name = name_token.value:gsub("^@", "")
                node.nameStart = name_token.start
                node.nameEnd = name_token["end"]
                node.type = render_values(token_values(self.tokens, self.index,
                    name_index - 1))
                self.index = name_index + 1
                node.params = self:parse_parameters()
                self:skip_constraints()
                if self:value() == ";" then
                    node["end"] = self:advance()["end"]
                end
            else
                node.name = "delegate"
            end
            return node
        end

        local name_token = self:peek()
        if name_token and self:is_identifier(name_token) then
            self:advance()
            node.name = name_token.value:gsub("^@", "")
            node.nameStart = name_token.start
            node.nameEnd = name_token["end"]
        else
            self:error("Expected type name.", name_token, "CS1001", 1)
            node.name = ""
            node.nameStart = kind_token.start
            node.nameEnd = kind_token["end"]
        end

        node.typeParameters = self:parse_type_parameters()

        if self:value() == "(" then
            node.params = self:parse_parameters()
        end

        node.baseTypes = self:parse_base_types()
        self:skip_constraints()

        if self:value() == "{" then
            local open_token = self:advance()
            node.bodyStart = open_token.start
            if kind == "enum" then
                node.children, node._close = self:parse_enum_members()
            else
                node.children, node._close = self:parse_members(node)
            end
            local close_token = node._close
            node.bodyEnd = close_token and close_token["end"] or self:last_end()
            node["end"] = node.bodyEnd
            node._close = nil
        elseif self:value() == ";" then
            node["end"] = self:advance()["end"]
        else
            node["end"] = self:last_end()
        end

        return node
    end

    function parser:parse_using(seen_declaration)
        local start_offset = self:peek().start
        local node = {
            kind = "using",
            name = "",
            start = start_offset,
            ["end"] = start_offset,
            isGlobal = false,
            isStatic = false,
            alias = nil,
            targetStart = start_offset,
            targetEnd = start_offset,
        }
        if self:value() == "global" then
            node.isGlobal = true
            self:advance()
        end
        if self:value() ~= "using" then
            return nil
        end
        self:advance()
        if self:value() == "static" then
            node.isStatic = true
            self:advance()
        end
        node.targetStart = self:peek() and self:peek().start or start_offset
        local values = {}
        local angle_depth = 0
        local paren_depth = 0
        local alias_index = nil
        while not self:eof() do
            local token = self:peek()
            if token.value == ";" and angle_depth == 0 and paren_depth == 0 then
                break
            end
            if token.value == "<" then
                angle_depth = angle_depth + 1
            elseif token.value == ">" then
                angle_depth = math.max(0, angle_depth - 1)
            elseif token.value == ">>" then
                angle_depth = math.max(0, angle_depth - 2)
            elseif token.value == "(" then
                paren_depth = paren_depth + 1
            elseif token.value == ")" then
                paren_depth = math.max(0, paren_depth - 1)
            elseif token.value == "=" and angle_depth == 0 and paren_depth == 0 then
                alias_index = #values
                self:advance()
            else
                values[#values + 1] = token.value
            end
            node.targetEnd = token["end"]
            self:advance()
        end
        if self:value() == ";" then
            node["end"] = self:advance()["end"]
        else
            self:error("Expected ';'.", self:peek(), "CS1002", 1)
            node["end"] = self:last_end()
        end
        if alias_index then
            node.alias = table.concat(values, "", 1, alias_index)
            local target = {}
            for index = alias_index + 1, #values do
                target[#target + 1] = values[index]
            end
            node.name = table.concat(target, "")
        else
            node.name = table.concat(values, "")
        end
        if seen_declaration then
            self:error("Using directives must precede all other members.",
                self:peek(), "CS1529", 1)
        end
        return node
    end

    function parser:parse_namespace()
        local start_offset = self:peek().start
        self:advance()
        local parts = {}
        while not self:eof() do
            local token = self:peek()
            if token.value == "{" or token.value == ";" then
                break
            end
            if self:is_identifier(token) then
                parts[#parts + 1] = token.value:gsub("^@", "")
            end
            self:advance()
        end
        local node = {
            kind = "namespace",
            name = sorted_name(parts),
            start = start_offset,
            ["end"] = self:last_end(),
            children = {},
        }
        if self:value() == ";" then
            node["end"] = self:advance()["end"]
            node.fileScoped = true
            while not self:eof() do
                local child = self:parse_top_level(node.children)
                if child then
                    node.children[#node.children + 1] = child
                else
                    break
                end
            end
            node["end"] = self:last_end()
        elseif self:value() == "{" then
            local open_token = self:advance()
            node.bodyStart = open_token.start
            while not self:eof() and self:value() ~= "}" do
                local child = self:parse_top_level(node.children)
                if child then
                    node.children[#node.children + 1] = child
                else
                    self:advance()
                end
            end
            if self:value() == "}" then
                node["end"] = self:advance()["end"]
            else
                self:error("Expected '}'.", open_token, "CS1513", 1)
            end
            node.bodyEnd = node["end"]
        else
            self:error("Expected '{' or ';'.", self:peek(), "CS1513", 1)
        end
        return node
    end

    function parser:parse_top_level(children)
        self:skip_attributes()
        if self:eof() then
            return nil
        end
        if self:value() == "using" or
            (self:value() == "global" and self:peek(1) and
                self:peek(1).value == "using") then
            return self:parse_using(false)
        end
        if self:value() == "namespace" then
            return self:parse_namespace()
        end
        if self:is_type_start(0) then
            return self:parse_type()
        end
        return nil
    end

    function parser:parse_unit()
        local unit = {
            kind = "compilationUnit",
            name = "",
            start = 1,
            ["end"] = #self.text + 1,
            children = {},
            usings = {},
        }
        local seen_declaration = false
        while not self:eof() do
            local attribute_start = self:skip_attributes()
            if self:eof() then
                break
            end
            if self:value() == "using" or
                (self:value() == "global" and self:peek(1) and
                    self:peek(1).value == "using") then
                local using_node = self:parse_using(seen_declaration)
                if using_node then
                    if attribute_start then
                        using_node.start = math.min(using_node.start,
                            attribute_start)
                    end
                    unit.children[#unit.children + 1] = using_node
                    unit.usings[#unit.usings + 1] = using_node
                else
                    self:advance()
                end
            elseif self:value() == "namespace" then
                seen_declaration = true
                local namespace_node = self:parse_namespace()
                if namespace_node then
                    unit.children[#unit.children + 1] = namespace_node
                    for _, using_node in ipairs(namespace_node.children) do
                        if using_node.kind == "using" then
                            unit.usings[#unit.usings + 1] = using_node
                        end
                    end
                end
            elseif self:is_type_start(0) then
                seen_declaration = true
                local type_node = self:parse_type()
                if type_node then
                    if attribute_start then
                        type_node.start = math.min(type_node.start,
                            attribute_start)
                    end
                    unit.children[#unit.children + 1] = type_node
                else
                    self:advance()
                end
            else
                local token = self:peek()
                if token.value == ";" then
                    self:advance()
                else
                    if token.type == "identifier" or token.type == "keyword" then
                        seen_declaration = true
                    end
                    self:advance()
                end
            end
        end
        unit["end"] = math.max(unit["end"], self:last_end())
        return unit
    end

    local unit = parser:parse_unit()
    return {
        unit = unit,
        source = source,
        tokens = parser.tokens,
        all_tokens = parser.all_tokens,
        comments = parser.comments,
        directives = parser.directives,
        errors = parser.errors,
    }
end

function P.parse(text, uri)
    local parser = create_parser(text or "", uri or "")
    return parser
end

P.type_keywords = type_keywords
P.builtin_types = builtin_types
P.render_values = render_values
P.angle_delta = angle_delta

return P
