local util = require("cs_util")
local parser = require("cs_parser")
local builtins = require("cs_builtins")

local A = {}

local models = {}
local workspace_models = {}
local workspace_indexed = false
local workspace_index_time = 0

local symbol_kinds = {
    namespace = 3,
    class = 5,
    record = 5,
    struct = 23,
    interface = 11,
    enum = 10,
    delegate = 12,
    method = 6,
    constructor = 9,
    destructor = 6,
    property = 7,
    indexer = 7,
    field = 8,
    event = 24,
    enumMember = 22,
    parameter = 13,
    variable = 13,
    using = 3,
}

local completion_kinds = {
    namespace = 9,
    class = 7,
    record = 7,
    struct = 22,
    interface = 8,
    enum = 13,
    delegate = 7,
    method = 2,
    constructor = 4,
    destructor = 2,
    property = 10,
    indexer = 10,
    field = 5,
    event = 23,
    enumMember = 20,
    parameter = 6,
    variable = 6,
    using = 9,
}

local type_kinds = {
    class = true,
    record = true,
    struct = true,
    interface = true,
    enum = true,
    delegate = true,
}

local member_kinds = {
    method = true,
    constructor = true,
    destructor = true,
    property = true,
    indexer = true,
    field = true,
    event = true,
}

local control_keywords = {
    ["if"] = true, ["for"] = true, ["foreach"] = true, ["while"] = true,
    ["switch"] = true, ["return"] = true, ["throw"] = true, ["new"] = true,
    ["await"] = true, ["using"] = true, ["lock"] = true, ["do"] = true,
    ["else"] = true, ["try"] = true, ["catch"] = true, ["finally"] = true,
    ["yield"] = true, ["case"] = true, ["default"] = true, ["goto"] = true,
    ["break"] = true, ["continue"] = true, ["checked"] = true,
    ["unchecked"] = true, ["fixed"] = true, ["stackalloc"] = true,
}

A.symbol_kinds = symbol_kinds
A.completion_kinds = completion_kinds
A.type_kinds = type_kinds
A.member_kinds = member_kinds

local function is_name_token(token)
    if not token then
        return false
    end
    return token.type == "identifier" or
        (token.type == "keyword" and token.value ~= "this" and
            token.value ~= "base" and token.value ~= "var" and
            token.value ~= "dynamic" and token.value ~= "value")
end

local function token_value(tokens, index)
    local token = tokens[index]
    return token and token.value or nil
end

local function add_symbol(model, symbol)
    model.symbols[#model.symbols + 1] = symbol
    local list = model.by_name[symbol.name]
    if not list then
        list = {}
        model.by_name[symbol.name] = list
    end
    list[#list + 1] = symbol
    if symbol.nameStart then
        model.declarations[symbol.nameStart] = symbol
    end
    return symbol
end

local function namespace_join(container, name)
    if not container or container == "" then
        return name or ""
    end
    if not name or name == "" then
        return container
    end
    return container .. "." .. name
end

local function collect_docs(model)
    local comments = model.comments
    local function at_comment(index, node_start, node_end)
        local collected = {}
        local cursor = node_start
        local current = index
        while current >= 1 do
            local comment = comments[current]
            if comment["end"] <= cursor then
                local gap = model.text:sub(comment["end"], cursor - 1)
                if gap:match("^%s*$") then
                    if comment.doc then
                        table.insert(collected, 1, comment.value)
                        cursor = comment.start
                        current = current - 1
                    else
                        break
                    end
                else
                    break
                end
            else
                current = current - 1
            end
        end
        if #collected == 0 then
            return ""
        end
        local lines = {}
        for _, value in ipairs(collected) do
            if value:sub(1, 3) == "/**" then
                local body = value:gsub("^/%*%*?", ""):gsub("%*/$", "")
                for line in body:gmatch("[^\n]*") do
                    line = line:gsub("^%s*%*%s?", "")
                    lines[#lines + 1] = line
                end
            else
                lines[#lines + 1] = value:gsub("^%s*///%s?", "")
            end
        end
        local text = table.concat(lines, "\n")
        local summary = text:match("<summary>(.-)</summary>")
        local remarks = text:match("<remarks>(.-)</remarks>")
        local returns = text:match("<returns>(.-)</returns>")
        local parts = {}
        local function clean(value)
            value = value:gsub("<[^>]+>", "")
            value = value:gsub("&lt;", "<"):gsub("&gt;", ">")
                :gsub("&amp;", "&"):gsub("&quot;", '"')
            return util.trim(value:gsub("%s+", " "))
        end
        if summary then
            parts[#parts + 1] = clean(summary)
        else
            local plain = text:gsub("<[^>]+>", "")
            plain = clean(plain)
            if plain ~= "" then
                parts[#parts + 1] = plain
            end
        end
        if returns then
            local value = clean(returns)
            if value ~= "" then
                parts[#parts + 1] = "Returns: " .. value
            end
        end
        if remarks then
            local value = clean(remarks)
            if value ~= "" then
                parts[#parts + 1] = "Remarks: " .. value
            end
        end
        return table.concat(parts, "\n\n")
    end

    table.sort(model.declaration_nodes, function(left, right)
        return left.start < right.start
    end)
    local comment_index = #comments
    for _, node in ipairs(model.declaration_nodes) do
        if comment_index > #comments then
            comment_index = #comments
        end
        while comment_index >= 1 and comments[comment_index].start >= node.start do
            comment_index = comment_index - 1
        end
        if comment_index >= 1 then
            node.documentation = at_comment(comment_index, node.start, node["end"])
        end
    end
end

local function describe_type_node(node)
    local values = {}
    for _, modifier in ipairs(node.modifiers or {}) do
        values[#values + 1] = modifier
    end
    values[#values + 1] = node.kind
    values[#values + 1] = node.name
    if node.typeParameters and #node.typeParameters > 0 then
        values[#values + 1] = "<" .. table.concat(node.typeParameters, ", ") .. ">"
    end
    if node.baseTypes and #node.baseTypes > 0 then
        local base_values = {}
        for _, base in ipairs(node.baseTypes) do
            base_values[#base_values + 1] = base.name
        end
        values[#values + 1] = ": " .. table.concat(base_values, ", ")
    end
    return table.concat(values, " ")
end

local function describe_member_node(node)
    local values = {}
    for _, modifier in ipairs(node.modifiers or {}) do
        values[#values + 1] = modifier
    end
    if node.kind == "constructor" then
        local parameters = {}
        for _, parameter in ipairs(node.params or {}) do
            parameters[#parameters + 1] = parameter.type .. " " .. parameter.name
        end
        values[#values + 1] = node.name .. "(" .. table.concat(parameters, ", ") .. ")"
        return table.concat(values, " ")
    end
    if node.kind == "destructor" then
        values[#values + 1] = "~" .. node.name .. "()"
        return table.concat(values, " ")
    end
    if node.kind == "field" or node.kind == "event" then
        values[#values + 1] = node.type
        values[#values + 1] = node.name
        return table.concat(values, " ")
    end
    if node.kind == "property" or node.kind == "indexer" or
        node.kind == "enumMember" then
        if node.type ~= "" and node.type ~= "enum" then
            values[#values + 1] = node.type
        end
        values[#values + 1] = node.name
        return table.concat(values, " ")
    end
    if node.kind == "method" then
        if node.type and node.type ~= "" then
            values[#values + 1] = node.type
        end
        local parameters = {}
        for _, parameter in ipairs(node.params or {}) do
            local text = parameter.type
            if text ~= "" then
                text = text .. " "
            end
            text = text .. parameter.name
            if parameter.default then
                text = text .. " = ..."
            end
            parameters[#parameters + 1] = text
        end
        values[#values + 1] = node.name .. "(" .. table.concat(parameters, ", ") .. ")"
        return table.concat(values, " ")
    end
    return table.concat(values, " ")
end

local function make_symbol(model, node, container, kind)
    local symbol = {
        name = node.name,
        kind = kind,
        container = container or "",
        fullName = namespace_join(container, node.name),
        start = node.start,
        ["end"] = node["end"],
        nameStart = node.nameStart,
        nameEnd = node.nameEnd,
        modifiers = node.modifiers or {},
        node = node,
        uri = model.uri,
        documentation = "",
        type = node.type or "",
        params = node.params,
            baseTypes = node.baseTypes,
    }
    symbol.range = model.source:range(symbol.start, symbol["end"])
    symbol.selectionRange = model.source:range(symbol.nameStart, symbol.nameEnd)
    symbol.symbolKind = symbol_kinds[kind] or 13
    symbol.completionKind = completion_kinds[kind] or 6
    node.symbol = symbol
    add_symbol(model, symbol)
    model.declaration_nodes[#model.declaration_nodes + 1] = node
    return symbol
end

local function collect_node(model, node, container, namespace)
    if not node then
        return
    end
    model.node_list[#model.node_list + 1] = node
    if node.kind == "compilationUnit" then
        for _, child in ipairs(node.children or {}) do
            collect_node(model, child, container, namespace)
        end
    elseif node.kind == "namespace" then
        local full = namespace_join(namespace, node.name)
        local symbol = make_symbol(model, node, namespace or "", "namespace")
        symbol.fullName = full
        model.namespaces[#model.namespaces + 1] = symbol
        for _, child in ipairs(node.children or {}) do
            collect_node(model, child, full, full)
        end
    elseif node.kind == "using" then
        model.using_nodes[#model.using_nodes + 1] = node
        model.usings[#model.usings + 1] = {
            name = node.name,
            alias = node.alias,
            static = node.isStatic,
            global = node.isGlobal,
            start = node.start,
            ["end"] = node["end"],
            node = node,
        }
        if node.alias then
            local symbol = {
                name = node.alias,
                kind = "namespace",
                container = "",
                fullName = node.alias,
                start = node.start,
                ["end"] = node["end"],
                nameStart = node.targetStart,
                nameEnd = node.targetEnd,
                modifiers = {},
                node = node,
                uri = model.uri,
                isUsingAlias = true,
                target = node.name,
                documentation = "Alias for " .. node.name .. ".",
            }
            symbol.range = model.source:range(symbol.start, symbol["end"])
            symbol.selectionRange = model.source:range(symbol.nameStart,
                symbol.nameEnd)
            symbol.symbolKind = 3
            symbol.completionKind = 9
            add_symbol(model, symbol)
            model.using_aliases[symbol.name] = symbol
        end
    elseif type_kinds[node.kind] then
        local symbol = make_symbol(model, node, container or namespace or "",
            node.kind)
        model.types[#model.types + 1] = symbol
        local existing = model.types_by_name[symbol.name]
        if not existing then
            existing = {}
            model.types_by_name[symbol.name] = existing
        end
        existing[#existing + 1] = symbol
        model.types_by_full[symbol.fullName] = symbol
        model.type_nodes[node] = symbol
        if node.kind == "enum" then
            for _, child in ipairs(node.children or {}) do
                local enum_symbol = make_symbol(model, child, symbol.fullName,
                    "enumMember")
                enum_symbol.type = symbol.name
                enum_symbol.container = symbol.fullName
                enum_symbol.fullName = namespace_join(symbol.fullName, child.name)
                child.symbol = enum_symbol
            end
        else
            for _, child in ipairs(node.children or {}) do
                collect_node(model, child, symbol.fullName, namespace)
            end
        end
    elseif member_kinds[node.kind] then
        local symbol = make_symbol(model, node, container or "", node.kind)
        if node.kind == "constructor" then
            symbol.name = node.name
        end
        if node.kind == "property" or node.kind == "indexer" or
            node.kind == "field" or node.kind == "event" or
            node.kind == "method" then
            symbol.type = node.type or ""
        end
        model.members[#model.members + 1] = symbol
        model.member_nodes[node] = symbol
        if node.kind == "property" or node.kind == "indexer" or
            node.kind == "event" then
            for _, accessor in ipairs(node.accessors or {}) do
                if accessor.bodyStart then
                    node.hasBodies = true
                    local accessor_node = {
                        kind = "accessor",
                        name = accessor.name,
                        start = accessor.start,
                        ["end"] = accessor["end"],
                        bodyStart = accessor.bodyStart,
                        bodyEnd = accessor.bodyEnd,
                        params = {},
                    }
                    model.node_list[#model.node_list + 1] = accessor_node
                end
            end
        end
    end
end

local function count_parameters(node)
    return #(node.params or {})
end

local function build_local_symbol(model, node, name_token, type_name, scope_node,
    is_parameter)
    if not name_token then
        return nil
    end
    local name = name_token.value:gsub("^@", "")
    local symbol = {
        name = name,
        kind = is_parameter and "parameter" or "variable",
        container = node.fullName or node.container or "",
        fullName = name,
        start = name_token.start,
        ["end"] = scope_node and scope_node["end"] or name_token["end"],
        nameStart = name_token.start,
        nameEnd = name_token["end"],
        modifiers = {},
        type = type_name or "",
        isLocal = true,
        isParameter = is_parameter,
        scopeNode = scope_node or node,
        hasInitializer = false,
        node = node,
        uri = model.uri,
        documentation = "",
    }
    symbol.range = model.source:range(symbol.start, symbol["end"])
    symbol.selectionRange = model.source:range(symbol.start, symbol["end"])
    symbol.symbolKind = 13
    symbol.completionKind = is_parameter and 6 or 6
    model.locals[#model.locals + 1] = symbol
    local list = model.locals_by_name[name]
    if not list then
        list = {}
        model.locals_by_name[name] = list
    end
    list[#list + 1] = symbol
    model.declarations[name_token.start] = symbol
    return symbol
end

local function parse_type_span(tokens, index, limit)
    local cursor = index
    local value = token_value(tokens, cursor)
    while value == "ref" or value == "readonly" or value == "scoped" or
        value == "const" do
        cursor = cursor + 1
        value = token_value(tokens, cursor)
    end
    if not value then
        return nil
    end
    if value == "(" then
        local depth = 0
        while cursor <= limit do
            local current = token_value(tokens, cursor)
            if current == "(" then
                depth = depth + 1
            elseif current == ")" then
                depth = depth - 1
                if depth == 0 then
                    cursor = cursor + 1
                    break
                end
            end
            cursor = cursor + 1
        end
    else
        local token = tokens[cursor]
        if not token then
            return nil
        end
        local is_type = token.type == "identifier" or
            parser.builtin_types[value] == true or value == "var" or
            value == "dynamic"
        if not is_type then
            return nil
        end
        cursor = cursor + 1
        while cursor <= limit do
            local current = token_value(tokens, cursor)
            if current == "." or current == "::" then
                cursor = cursor + 1
                if not tokens[cursor] then
                    return nil
                end
                cursor = cursor + 1
            elseif current == "<" then
                local depth = 1
                cursor = cursor + 1
                while cursor <= limit and depth > 0 do
                    local inner = token_value(tokens, cursor)
                    if inner == "<" then
                        depth = depth + 1
                    elseif inner == ">" then
                        depth = depth - 1
                    elseif inner == ">>" then
                        depth = depth - 2
                    end
                    cursor = cursor + 1
                end
            elseif current == "?" then
                cursor = cursor + 1
            elseif current == "[" then
                local depth = 1
                cursor = cursor + 1
                while cursor <= limit and depth > 0 do
                    local inner = token_value(tokens, cursor)
                    if inner == "[" then
                        depth = depth + 1
                    elseif inner == "]" then
                        depth = depth - 1
                    end
                    cursor = cursor + 1
                end
            elseif current == "*" then
                cursor = cursor + 1
            else
                break
            end
        end
    end
    return cursor
end

local function skip_expression(tokens, index, limit, stops)
    local cursor = index
    local depth = 0
    local angle = 0
    while cursor <= limit do
        local value = token_value(tokens, cursor)
        if depth == 0 and angle == 0 and stops[value] then
            return cursor
        end
        if value == "(" or value == "[" or value == "{" then
            depth = depth + 1
        elseif value == ")" or value == "]" or value == "}" then
            depth = depth - 1
        elseif value == "<" then
            angle = angle + 1
        elseif value == ">" then
            angle = math.max(0, angle - 1)
        elseif value == ">>" then
            angle = math.max(0, angle - 2)
        end
        cursor = cursor + 1
    end
    return cursor
end

local function collect_locals(model)
    local tokens = model.tokens
    local limit = #tokens
    local function add_parameter(model_node, symbol_node, parameter)
        local fake = {
            value = parameter.name,
            start = parameter.nameStart,
            ["end"] = parameter.nameEnd,
        }
        local local_symbol = build_local_symbol(model, model_node or symbol_node,
            fake, parameter.type, symbol_node, true)
        if local_symbol then
            local_symbol.modifiers = parameter.modifiers or {}
            local_symbol.parameterIndex = parameter
        end
    end
    for _, node in ipairs(model.node_list) do
        if node.params then
            for _, parameter in ipairs(node.params) do
                add_parameter(node, node, parameter)
            end
        end
    end

    local function token_index_for(offset)
        local low = 1
        local high = limit
        while low <= high do
            local middle = math.floor((low + high) / 2)
            local token = tokens[middle]
            if token.start < offset then
                low = middle + 1
            elseif token.start > offset then
                high = middle - 1
            else
                return middle
            end
        end
        return math.max(1, math.min(low, limit))
    end

    for _, node in ipairs(model.node_list) do
        if node.bodyStart and node.bodyEnd and
            (member_kinds[node.kind] or node.kind == "accessor") then
            local start_index = token_index_for(node.bodyStart)
            local end_index = token_index_for(node.bodyEnd)
            local index = start_index
            while index <= end_index do
                local token = tokens[index]
                local value = token and token.value
                local previous = tokens[index - 1] and tokens[index - 1].value

                if value == "foreach" then
                    local cursor = index + 1
                    if token_value(tokens, cursor) == "(" then
                        cursor = cursor + 1
                        local type_end = parse_type_span(tokens, cursor,
                            end_index)
                        if type_end and tokens[type_end] then
                            local name_token = tokens[type_end]
                            local type_values = {}
                            for position = cursor, type_end - 1 do
                                type_values[#type_values + 1] =
                                    tokens[position].value
                            end
                            if token_value(tokens, type_end + 1) == "in" then
                                local local_symbol = build_local_symbol(model,
                                    node, name_token,
                                    parser.render_values(type_values),
                                    node, false)
                                if local_symbol then
                                    local_symbol.hasInitializer = true
                                    local collection_token = tokens[type_end + 2]
                                    if collection_token and
                                        collection_token.type == "identifier" then
                                        local identifier = {
                                            value = collection_token.value:gsub(
                                                "^@", ""),
                                            start = collection_token.start,
                                            ["end"] = collection_token["end"],
                                            index = type_end + 2,
                                        }
                                        local collection = A.resolve_identifier(
                                            model, identifier,
                                            collection_token.start)
                                        local collection_type = collection and
                                            A.type_of_symbol(model, collection)
                                        if collection_type then
                                            local element = collection_type:match(
                                                "[%w_]+<(.*)>") or
                                                collection_type:match("(.-)%[%]")
                                            local_symbol.inferredType = element
                                        end
                                    end
                                end
                            end
                        end
                    end
                elseif value == "catch" then
                    local cursor = index + 1
                    if token_value(tokens, cursor) == "(" then
                        local depth = 1
                        cursor = cursor + 1
                        local last_name = nil
                        local type_values = {}
                        while cursor <= end_index and depth > 0 do
                            local current = tokens[cursor]
                            local current_value = current.value
                            if current_value == "(" then
                                depth = depth + 1
                            elseif current_value == ")" then
                                depth = depth - 1
                                if depth == 0 then
                                    break
                                end
                            elseif is_name_token(current) then
                                last_name = current
                            elseif last_name == nil then
                                type_values[#type_values + 1] = current_value
                            end
                            cursor = cursor + 1
                        end
                        if last_name then
                            local type_name = parser.render_values(type_values)
                            if type_name == "" then
                                type_name = "Exception"
                            end
                            build_local_symbol(model, node, last_name, type_name,
                                node, false)
                        end
                    end
                elseif value == "out" then
                    local cursor = index + 1
                    local type_end = parse_type_span(tokens, cursor, end_index)
                    if type_end and tokens[type_end] then
                        local type_values = {}
                        for position = cursor, type_end - 1 do
                            type_values[#type_values + 1] =
                                tokens[position].value
                        end
                        local name_token = tokens[type_end]
                        if is_name_token(name_token) then
                            local local_symbol = build_local_symbol(model, node,
                                name_token, parser.render_values(type_values),
                                node, false)
                            if local_symbol then
                                local_symbol.hasInitializer = true
                            end
                        end
                    end
                else
                    local boundary = previous == nil or previous == "{" or
                        previous == "}" or previous == ";" or previous == ":"
                    local maybe_declaration = tokens[index] and
                        (is_name_token(tokens[index]) or
                            parser.builtin_types[value] == true or
                            value == "var" or value == "dynamic" or
                            value == "const" or value == "ref" or
                            value == "readonly" or value == "scoped")
                    if boundary and not control_keywords[value] and
                        maybe_declaration then
                        local type_end = parse_type_span(tokens, index,
                            end_index)
                        if type_end and tokens[type_end] then
                            local type_values = {}
                            for position = index, type_end - 1 do
                                type_values[#type_values + 1] =
                                    tokens[position].value
                            end
                            local type_name = parser.render_values(type_values)
                            local name_token = tokens[type_end]
                            if is_name_token(name_token) and
                                name_token.value ~= "this" then
                                local after = token_value(tokens, type_end + 1)
                                if after == "=" or after == ";" or after == "," or
                                    after == "in" or after == ")" then
                                    local cursor = type_end
                                    local declared_any = false
                                    while cursor <= end_index do
                                        local current_name = tokens[cursor]
                                        if not is_name_token(current_name) then
                                            break
                                        end
                                        local local_symbol = build_local_symbol(
                                            model, node, current_name, type_name,
                                            node, false)
                                        if not local_symbol then
                                            break
                                        end
                                        declared_any = true
                                        cursor = cursor + 1
                                        if token_value(tokens, cursor) == "=" then
                                            local_symbol.hasInitializer = true
                                            cursor = skip_expression(tokens,
                                                cursor + 1, end_index, {
                                                    [","] = true,
                                                    [";"] = true,
                                                    ["in"] = true,
                                                    [")"] = true,
                                                })
                                        end
                                        if token_value(tokens, cursor) == "," then
                                            cursor = cursor + 1
                                        else
                                            break
                                        end
                                    end
                                    if declared_any then
                                        if token_value(tokens, cursor) == ";" then
                                            index = cursor
                                        else
                                            index = math.max(index, cursor - 1)
                                        end
                                    end
                                end
                            end
                        end
                    end
                end
                index = index + 1
            end
        end
    end
end

local function collect_identifiers(model)
    local tokens = model.tokens
    for index, token in ipairs(tokens) do
        if token.type == "identifier" then
            local value = token.value:gsub("^@", "")
            local entry = {
                value = value,
                token = token,
                start = token.start,
                ["end"] = token["end"],
                index = index,
                prev = tokens[index - 1],
                next = tokens[index + 1],
                prevValue = tokens[index - 1] and tokens[index - 1].value,
                nextValue = tokens[index + 1] and tokens[index + 1].value,
                declaration = model.declarations[token.start],
            }
            model.identifiers[#model.identifiers + 1] = entry
            model.identifiers_by_start[token.start] = entry
        end
    end
end

local function add_diagnostic(model, diagnostic)
    model.diagnostics[#model.diagnostics + 1] = diagnostic
end

local function validate(model)
    for _, error in ipairs(model.errors or {}) do
        add_diagnostic(model, {
            start = error.start,
            ["end"] = error["end"],
            message = error.message,
            code = error.code,
            severity = error.severity or 1,
            data = error.data,
        })
    end

    local type_seen = {}
    local member_seen = {}
    for _, symbol in ipairs(model.symbols) do
        if type_kinds[symbol.kind] then
            local key = symbol.container .. "|" .. symbol.name
            if type_seen[key] then
                add_diagnostic(model, {
                    start = symbol.nameStart,
                    ["end"] = symbol.nameEnd,
                    message = "The namespace already contains a definition for '" ..
                        symbol.name .. "'.",
                    code = "CS0101",
                    severity = 2,
                })
            else
                type_seen[key] = true
            end
        elseif member_kinds[symbol.kind] and symbol.kind ~= "constructor" then
            local key = symbol.container .. "|" .. symbol.name .. "|" ..
                symbol.kind
            if member_seen[key] then
                add_diagnostic(model, {
                    start = symbol.nameStart,
                    ["end"] = symbol.nameEnd,
                    message = "The type already contains a definition for '" ..
                        symbol.name .. "'.",
                    code = "CS0102",
                    severity = 2,
                })
            else
                member_seen[key] = true
            end
        end
    end

    for _, local_symbol in ipairs(model.locals) do
        if not local_symbol.isParameter and local_symbol.type == "var" and
            not local_symbol.hasInitializer then
            add_diagnostic(model, {
                start = local_symbol.nameStart,
                ["end"] = local_symbol.nameEnd,
                message = "Implicitly-typed variables must be initialized.",
                code = "CS0818",
                severity = 1,
                data = {
                    fix = "initializeVar",
                    name = local_symbol.name,
                },
            })
        end
    end

    for _, node in ipairs(model.node_list) do
        if node.kind == "method" and node.bodyStart and node.bodyEnd then
            local has_async = false
            for _, modifier in ipairs(node.modifiers or {}) do
                if modifier == "async" then
                    has_async = true
                    break
                end
            end
            if has_async then
                local has_await = false
                for _, token in ipairs(model.tokens) do
                    if token.start >= node.bodyStart and token["end"] <=
                        node.bodyEnd and token.value == "await" then
                        has_await = true
                        break
                    end
                end
                if not has_await then
                    local modifier_start = nil
                    local modifier_end = nil
                    for _, token in ipairs(model.tokens) do
                        if token.start >= node.start and token["end"] <=
                            node.bodyStart and token.value == "async" then
                            modifier_start = token.start
                            modifier_end = token["end"]
                            break
                        end
                    end
                    add_diagnostic(model, {
                        start = modifier_start or node.start,
                        ["end"] = modifier_end or node.nameStart,
                        message = "This async method lacks 'await' operators and will run synchronously.",
                        code = "CS1998",
                        severity = 2,
                        data = {
                            fix = "removeAsync",
                        },
                    })
                end
            end
        end
    end

    local seen_using = {}
    for _, using in ipairs(model.usings) do
        local key = (using.alias and (using.alias .. "=") or "") ..
            (using.static and "static:" or "") .. using.name
        if seen_using[key] then
            add_diagnostic(model, {
                start = using.start,
                ["end"] = using["end"],
                message = "The using directive appeared previously in this namespace.",
                code = "CS0105",
                severity = 2,
                tags = { 1 },
                data = {
                    fix = "removeUsing",
                    start = using.start,
                    finish = using["end"],
                },
            })
        else
            seen_using[key] = true
        end
    end

    local region_depth = 0
    for _, directive in ipairs(model.directives or {}) do
        if directive.directive == "region" then
            region_depth = region_depth + 1
        elseif directive.directive == "endregion" then
            region_depth = region_depth - 1
            if region_depth < 0 then
                add_diagnostic(model, {
                    start = directive.start,
                    ["end"] = directive["end"],
                    message = "Unexpected #endregion directive.",
                    code = "CS1028",
                    severity = 2,
                })
                region_depth = 0
            end
        end
    end
    if region_depth > 0 then
        add_diagnostic(model, {
            start = 1,
            ["end"] = 1,
            message = "Expected #endregion directive.",
            code = "CS1038",
            severity = 2,
        })
    end
end

local function build_model(uri, text, version)
    local parsed = parser.parse(text or "", uri or "")
    local model = {
        uri = uri or "",
        text = text or "",
        version = version or 0,
        source = parsed.source,
        unit = parsed.unit,
        tokens = parsed.tokens,
        token_index_by_start = {},
        all_tokens = parsed.all_tokens,
        comments = parsed.comments,
        directives = parsed.directives,
        errors = parsed.errors,
        symbols = {},
        members = {},
        types = {},
        namespaces = {},
        locals = {},
        locals_by_name = {},
        identifiers = {},
        identifiers_by_start = {},
        by_name = {},
        types_by_name = {},
        types_by_full = {},
        using_aliases = {},
        usings = {},
        using_nodes = {},
        declarations = {},
        declaration_nodes = {},
        node_list = {},
        type_nodes = {},
        member_nodes = {},
        diagnostics = {},
    }
    for index, token in ipairs(model.tokens) do
        model.token_index_by_start[token.start] = index
    end
    collect_node(model, parsed.unit, "", "")
    collect_docs(model)
    collect_locals(model)
    collect_identifiers(model)
    validate(model)
    return model
end

local function open_document(uri)
    local documents = hexal and hexal.lsp and hexal.lsp.documents
    if not documents then
        return nil
    end
    local ok, document = pcall(documents.get, uri)
    if ok then
        return document
    end
    return nil
end

local function read_uri(uri)
    local document = open_document(uri)
    if document then
        return document.text, document.version
    end
    local ok, path = pcall(hexal.lsp.uri.to_path, uri)
    if not ok or not path or path == "" then
        return nil, 0
    end
    local read_ok, text = pcall(hexal.fs.read_file, path)
    if not read_ok or type(text) ~= "string" then
        return nil, 0
    end
    local stat_ok, stat = pcall(hexal.fs.stat, path)
    local version = 0
    if stat_ok and type(stat) == "table" and stat.mtime then
        version = stat.mtime
    end
    return text, version
end

function A.get_model(uri, text, version)
    uri = uri or ""
    if text == nil then
        local loaded, loaded_version = read_uri(uri)
        if loaded ~= nil then
            text, version = loaded, loaded_version
        elseif models[uri] then
            return models[uri]
        end
    end
    if type(text) ~= "string" then
        return nil
    end
    local cached = models[uri]
    if cached and cached.text == text then
        cached.version = version or cached.version
        return cached
    end
    local model = build_model(uri, text, version)
    models[uri] = model
    return model
end

function A.invalidate(uri)
    if uri then
        models[uri] = nil
        workspace_models[uri] = nil
        workspace_indexed = false
    else
        models = {}
        workspace_models = {}
        workspace_indexed = false
    end
end

function A.invalidate_workspace()
    workspace_models = {}
    workspace_indexed = false
    workspace_index_time = 0
end

local function should_skip_path(path)
    local normalized = path:gsub("\\", "/")
    return normalized:find("/obj/", 1, true) ~= nil or
        normalized:find("/bin/", 1, true) ~= nil or
        normalized:find("/.git/", 1, true) ~= nil or
        normalized:find("/node_modules/", 1, true) ~= nil or
        normalized:find("/.vs/", 1, true) ~= nil or
        normalized:find("/packages/", 1, true) ~= nil
end

function A.index_workspace(force)
    if workspace_indexed and not force then
        return workspace_models
    end
    if force then
        workspace_models = {}
    end

    local ok, paths = pcall(hexal.fs.glob, "**/*.cs", { max_results = 20000 })
    if ok and type(paths) == "table" then
        for _, path in ipairs(paths) do
            if not should_skip_path(path) then
                local uri = hexal.lsp.uri.from_path(path)
                local text, version = read_uri(uri)
                if text and not workspace_models[uri] then
                    workspace_models[uri] = build_model(uri, text, version)
                end
            end
        end
    end

    local documents_ok, documents = pcall(hexal.lsp.documents.open_documents)
    if documents_ok then
        for document in documents do
            local uri = document.uri
            if uri and uri:match("%.cs$") then
                local text, version = read_uri(uri)
                if text then
                    workspace_models[uri] = build_model(uri, text, version)
                end
            end
        end
    end

    workspace_indexed = true
    workspace_index_time = os.time()
    return workspace_models
end

function A.update_workspace(uri)
    if not uri or not workspace_indexed then
        return
    end
    if uri:match("%.cs$") then
        local text, version = read_uri(uri)
        if text then
            workspace_models[uri] = build_model(uri, text, version)
        else
            workspace_models[uri] = nil
        end
    end
end

function A.all_models()
    local result = {}
    local seen = {}
    for uri, model in pairs(workspace_models) do
        result[#result + 1] = model
        seen[uri] = true
    end
    for uri, model in pairs(models) do
        if not seen[uri] then
            result[#result + 1] = model
        end
    end
    return result
end

function A.workspace_indexed()
    return workspace_indexed
end

function A.workspace_age()
    return os.time() - workspace_index_time
end

function A.token_at(model, offset)
    if not model then
        return nil
    end
    offset = math.max(1, offset or 1)
    local tokens = model.tokens
    local low = 1
    local high = #tokens
    local candidate = nil
    while low <= high do
        local middle = math.floor((low + high) / 2)
        local token = tokens[middle]
        if token.start <= offset then
            candidate = token
            low = middle + 1
        else
            high = middle - 1
        end
    end
    if candidate then
        if offset <= candidate["end"] then
            return candidate
        end
        if candidate.type == "eof" then
            return nil
        end
    end
    return candidate
end

function A.identifier_at(model, offset)
    if not model then
        return nil
    end
    offset = math.max(1, offset or 1)
    local identifiers = model.identifiers
    local low = 1
    local high = #identifiers
    local candidate = nil
    while low <= high do
        local middle = math.floor((low + high) / 2)
        local entry = identifiers[middle]
        if entry.start <= offset then
            candidate = entry
            low = middle + 1
        else
            high = middle - 1
        end
    end
    if candidate and offset <= candidate["end"] then
        return candidate
    end
    return nil
end

function A.node_at(model, offset, predicate)
    local best = nil
    for _, node in ipairs(model.node_list) do
        if node.start <= offset and offset <= node["end"] then
            if not predicate or predicate(node) then
                if not best or (node["end"] - node.start) <
                    (best["end"] - best.start) then
                    best = node
                end
            end
        end
    end
    return best
end

function A.enclosing_member(model, offset)
    return A.node_at(model, offset, function(node)
        return member_kinds[node.kind] or type_kinds[node.kind] or
            node.kind == "namespace"
    end)
end

function A.enclosing_type(model, offset)
    return A.node_at(model, offset, function(node)
        return type_kinds[node.kind]
    end)
end

function A.enclosing_namespace(model, offset)
    return A.node_at(model, offset, function(node)
        return node.kind == "namespace"
    end)
end

local function locals_for(model, name, offset)
    local result = {}
    for _, local_symbol in ipairs(model.locals_by_name[name] or {}) do
        local scope = local_symbol.scopeNode
        if scope and scope.start <= offset and offset <= scope["end"] then
            result[#result + 1] = local_symbol
        end
    end
    table.sort(result, function(left, right)
        return left.start > right.start
    end)
    return result
end

local function member_by_name(model, type_name, name)
    local type_symbol = nil
    for _, symbol in ipairs(model.types_by_name[type_name] or {}) do
        type_symbol = symbol
        break
    end
    if not type_symbol then
        for _, other in ipairs(A.all_models()) do
            for _, symbol in ipairs(other.types_by_name[type_name] or {}) do
                type_symbol = symbol
                break
            end
            if type_symbol then
                break
            end
        end
    end
    if type_symbol then
        for _, child in ipairs(type_symbol.node.children or {}) do
            if child.name == name and (member_kinds[child.kind] or
                child.kind == "enumMember") then
                return child.symbol
            end
        end
    end
    return nil
end

local function builtin_member(type_name, name)
    local candidates = builtins.members(type_name)
    for _, item in ipairs(candidates) do
        if item.name == name then
            return item
        end
    end
    local simple = type_name and type_name:match("^([%w_]+)")
    if simple and simple ~= type_name then
        for _, item in ipairs(builtins.members(simple)) do
            if item.name == name then
                return item
            end
        end
    end
    return nil
end

function A.members_of_type(model, type_name)
    local result = {}
    local seen = {}
    local visited = {}

    local function add_member(item)
        if not item or not item.name then
            return
        end
        local key = item.name .. "|" .. tostring(item.kind)
        if seen[key] then
            return
        end
        seen[key] = true
        result[#result + 1] = item
    end

    local function add_symbol_members(symbol, depth)
        if not symbol or depth > 8 or visited[symbol] then
            return
        end
        visited[symbol] = true
        for _, child in ipairs(symbol.node.children or {}) do
            if type_kinds[child.kind] then
                local nested = child.symbol
                if nested then
                    add_member({
                        name = child.name,
                        kind = child.kind,
                        type = child.name,
                        signature = describe_type_node(child),
                        documentation = nested.documentation or "",
                        symbol = nested,
                        modifiers = child.modifiers or {},
                    })
                end
            elseif member_kinds[child.kind] or child.kind == "enumMember" then
                local member_symbol = child.symbol
                if member_symbol then
                    add_member({
                        name = child.name,
                        kind = child.kind,
                        type = child.type or "",
                        signature = describe_member_node(child),
                        documentation = member_symbol.documentation or "",
                        static = (function()
                            for _, modifier in ipairs(child.modifiers or {}) do
                                if modifier == "static" then
                                    return true
                                end
                            end
                            return false
                        end)(),
                        symbol = member_symbol,
                        modifiers = child.modifiers or {},
                        params = child.params,
                    })
                end
            end
        end
        for _, base in ipairs(symbol.node.baseTypes or {}) do
            local base_symbol = A.find_type(model, base.name)
            if base_symbol then
                add_symbol_members(base_symbol, depth + 1)
            else
                local builtin = builtins.type(base.name)
                if builtin then
                    for _, item in ipairs(builtin.members or {}) do
                        add_member(item)
                    end
                end
            end
        end
    end

    local simple = type_name and type_name:gsub("<.*$", ""):match(
        "([%w_]+)") or type_name
    local symbol = A.find_type(model, simple)
    if symbol then
        add_symbol_members(symbol, 0)
    else
        local builtin = builtins.type(simple or type_name)
        if builtin then
            for _, item in ipairs(builtin.members or {}) do
                add_member(item)
            end
            if builtin.kind == "enum" then
                for _, item in ipairs(builtins.enum_members) do
                    add_member(item)
                end
            end
        end
    end

    if type_name and (type_name:match("%[%]$") or type_name:match("%[%]")) then
        for _, item in ipairs(builtins.members("Array")) do
            add_member(item)
        end
        add_member({
            name = "Length",
            kind = "property",
            type = "int",
            signature = "Length",
            documentation = "Gets the number of elements in the array.",
            readonly = true,
        })
    end

    for _, item in ipairs(builtins.members("object")) do
        add_member(item)
    end

    if model then
        local has_linq = false
        for _, using in ipairs(model.usings) do
            if using.name == "System.Linq" then
                has_linq = true
                break
            end
        end
        if has_linq then
            for _, item in ipairs(builtins.enumerable_members) do
                add_member(item)
            end
        end
    end

    table.sort(result, function(left, right)
        local left_static = left.static and 0 or 1
        local right_static = right.static and 0 or 1
        if left_static ~= right_static then
            return left_static < right_static
        end
        return left.name < right.name
    end)
    return result
end

function A.find_type(model, name)
    if not name or name == "" then
        return nil
    end
    local simple = name:gsub("%?", ""):gsub("%[%]", ""):gsub("<.*$", "")
        :gsub("^global::", "")
    simple = simple:match("([%w_]+)$") or simple
    if model then
        for _, symbol in ipairs(model.types_by_name[simple] or {}) do
            return symbol
        end
        for _, symbol in ipairs(model.types) do
            if symbol.fullName == name then
                return symbol
            end
        end
    end
    for _, other in ipairs(A.all_models()) do
        for _, symbol in ipairs(other.types_by_name[simple] or {}) do
            return symbol
        end
    end
    return nil
end

function A.find_symbols(model, name)
    local result = {}
    local seen = {}
    local function add(symbol)
        if symbol and not seen[symbol] then
            seen[symbol] = true
            result[#result + 1] = symbol
        end
    end
    for _, symbol in ipairs(model.by_name[name] or {}) do
        add(symbol)
    end
    for _, other in ipairs(A.all_models()) do
        if other ~= model then
            for _, symbol in ipairs(other.by_name[name] or {}) do
                add(symbol)
            end
        end
    end
    return result
end

local function type_from_call(model, token_index)
    local tokens = model.tokens
    local open_index = nil
    local depth = 0
    for cursor = token_index, 1, -1 do
        local value = tokens[cursor] and tokens[cursor].value
        if value == ")" then
            depth = depth + 1
        elseif value == "(" then
            depth = depth - 1
            if depth == 0 then
                open_index = cursor
                break
            end
        end
    end
    if not open_index then
        return nil
    end
    local name_token = tokens[open_index - 1]
    if not name_token then
        return nil
    end
    if name_token.value == ">" or name_token.value == ">>" then
        local cursor = open_index - 1
        local angle = 0
        while cursor >= 1 do
            local value = tokens[cursor].value
            if value == ">" then
                angle = angle + 1
            elseif value == ">>" then
                angle = angle + 2
            elseif value == "<" then
                angle = angle - 1
                if angle == 0 then
                    cursor = cursor - 1
                    break
                end
            end
            cursor = cursor - 1
        end
        name_token = tokens[cursor]
        if not name_token then
            return nil
        end
    end
    local identifier = {
        value = name_token.value:gsub("^@", ""),
        start = name_token.start,
        ["end"] = name_token["end"],
        index = open_index - 1,
    }
    local previous = tokens[open_index - 2]
    if previous and previous.value == "new" then
        return identifier.value
    end
    local symbol = A.resolve_identifier(model, identifier, name_token.start)
    if symbol then
        if symbol.kind == "constructor" then
            return symbol.container
        end
        if symbol.type and symbol.type ~= "" then
            return symbol.type
        end
        if symbol.kind == "class" or symbol.kind == "struct" or
            symbol.kind == "interface" or symbol.kind == "enum" or
            symbol.kind == "record" then
            return symbol.name
        end
    end
    local builtin = builtins.type(identifier.value)
    if builtin then
        if previous and previous.value == "new" then
            return builtin.name
        end
        return builtin.name
    end
    return nil
end

function A.type_before(model, token_index)
    local tokens = model.tokens
    local previous = tokens[token_index - 1]
    if not previous then
        return nil
    end
    local value = previous.value
    if value == "this" then
        local node = A.enclosing_type(model, previous.start)
        return node and node.name or nil
    elseif value == "base" then
        local node = A.enclosing_type(model, previous.start)
        if node and node.baseTypes and node.baseTypes[1] then
            return node.baseTypes[1].name
        end
        return nil
    elseif previous.type == "string" then
        return "string"
    elseif previous.type == "char" then
        return "char"
    elseif previous.type == "number" then
        if value:find("[fF]") then
            return "float"
        elseif value:find("[mM]") then
            return "decimal"
        elseif value:find("[dD]") then
            return "double"
        elseif value:find("[lL]") then
            return "long"
        elseif value:find("[uU]") then
            return "uint"
        elseif value:find("%.") or value:find("[eE]") then
            return "double"
        end
        return "int"
    elseif value == "true" or value == "false" then
        return "bool"
    elseif value == ")" or value == "]" then
        if value == ")" then
            return type_from_call(model, token_index - 1)
        end
        local depth = 0
        local open_index = nil
        for cursor = token_index - 1, 1, -1 do
            local current = tokens[cursor] and tokens[cursor].value
            if current == "]" then
                depth = depth + 1
            elseif current == "[" then
                depth = depth - 1
                if depth == 0 then
                    open_index = cursor
                    break
                end
            end
        end
        if open_index then
            local before = tokens[open_index - 1]
            if before then
                local identifier = {
                    value = before.value:gsub("^@", ""),
                    start = before.start,
                    ["end"] = before["end"],
                    index = open_index - 1,
                }
                local symbol = A.resolve_identifier(model, identifier,
                    before.start)
                if symbol and symbol.type then
                    local element = symbol.type:match("^(.-)%[%]")
                    if element then
                        return element
                    end
                end
            end
        end
    elseif value == ">" or value == ">>" then
        local cursor = token_index - 1
        local angle = 0
        while cursor >= 1 do
            local current = tokens[cursor].value
            if current == ">" then
                angle = angle + 1
            elseif current == ">>" then
                angle = angle + 2
            elseif current == "<" then
                angle = angle - 1
                if angle == 0 then
                    cursor = cursor - 1
                    break
                end
            end
            cursor = cursor - 1
        end
        local name_token = tokens[cursor]
        if name_token then
            local identifier = {
                value = name_token.value:gsub("^@", ""),
                start = name_token.start,
                ["end"] = name_token["end"],
                index = cursor,
            }
            local symbol = A.resolve_identifier(model, identifier,
                name_token.start)
            if symbol then
                return symbol.kind == "class" and symbol.name or symbol.type
            end
            local builtin = builtins.type(identifier.value)
            if builtin then
                return builtin.name
            end
        end
    elseif previous.type == "identifier" then
        local identifier = {
            value = value:gsub("^@", ""),
            start = previous.start,
            ["end"] = previous["end"],
            index = token_index - 1,
        }
        if token_index - 2 >= 1 and
            (tokens[token_index - 2].value == "." or
                tokens[token_index - 2].value == "?.") then
            local owner_type = A.type_before(model, token_index - 2)
            local member = A.find_member(model, owner_type, identifier.value)
            if member then
                return member.type
            end
        end
        local symbol = A.resolve_identifier(model, identifier, previous.start)
        if symbol then
            if type_kinds[symbol.kind] then
                return symbol.name
            end
            local symbol_type = A.type_of_symbol(model, symbol)
            if symbol_type and symbol_type ~= "" then
                return symbol_type
            end
            if symbol.type and symbol.type ~= "" then
                return symbol.type
            end
        end
        local builtin = builtins.type(identifier.value)
        if builtin then
            return builtin.name
        end
    end
    return nil
end

function A.find_member(model, type_name, name)
    if not type_name then
        return nil
    end
    local symbol = member_by_name(model, type_name:gsub("<.*$", ""), name)
    if symbol then
        return {
            name = symbol.name,
            kind = symbol.kind,
            type = symbol.type,
            symbol = symbol,
            documentation = symbol.documentation,
        }
    end
    local item = builtin_member(type_name, name)
    if item then
        return item
    end
    local members = A.members_of_type(model, type_name)
    for _, member in ipairs(members) do
        if member.name == name then
            return member
        end
    end
    return nil
end

function A.resolve_identifier(model, identifier, offset)
    if not identifier or not model then
        return nil
    end
    local token = identifier.token or identifier
    local start = token.start or identifier.start
    local declaration = model.declarations[start]
    if declaration then
        return declaration
    end
    local name = identifier.value
    local previous = identifier.prev or (identifier.index and
        model.tokens[identifier.index - 1])
    local previous_value = previous and previous.value
    if previous_value == "." or previous_value == "?." then
        local owner_type = A.type_before(model, identifier.index - 1)
        local member = A.find_member(model, owner_type, name)
        if member then
            return member.symbol or member
        end
    end
    local locals = locals_for(model, name, start)
    if #locals > 0 then
        return locals[1]
    end
    local enclosing = A.enclosing_type(model, start)
    if enclosing then
        local member = member_by_name(model, enclosing.name, name)
        if member then
            return member
        end
        for _, base in ipairs(enclosing.baseTypes or {}) do
            local member_from_base = member_by_name(model, base.name, name)
            if member_from_base then
                return member_from_base
            end
        end
    end
    local symbols = A.find_symbols(model, name)
    if #symbols > 0 then
        local best = symbols[1]
        for _, symbol in ipairs(symbols) do
            if symbol.uri == model.uri then
                best = symbol
                break
            end
        end
        return best
    end
    if model.using_aliases[name] then
        local alias = model.using_aliases[name]
        local target = A.find_type(model, alias.target)
        if target then
            return target
        end
        return alias
    end
    for _, using in ipairs(model.usings) do
        if using.alias == name then
            local target = A.find_type(model, using.name)
            if target then
                return target
            end
        end
    end
    local builtin = builtins.type(name)
    if builtin then
        return builtin
    end
    for _, namespace in ipairs(builtins.namespaces) do
        if namespace == name or namespace:match("%." .. name .. "$") then
            return {
                name = name,
                kind = "namespace",
                symbolKind = 3,
                completionKind = 9,
                documentation = "Namespace " .. namespace .. ".",
            }
        end
    end
    return nil
end

function A.resolve_at(model, offset)
    local identifier = A.identifier_at(model, offset)
    if not identifier then
        return nil, nil
    end
    return identifier, A.resolve_identifier(model, identifier, offset)
end

function A.type_of_local(model, local_symbol)
    if local_symbol.inferredType then
        return local_symbol.inferredType
    end
    local type_name = local_symbol.type
    if type_name ~= "var" then
        local_symbol.inferredType = type_name
        return type_name
    end
    local tokens = model.tokens
    local index = nil
    for cursor, token in ipairs(tokens) do
        if token.start == local_symbol.nameStart then
            index = cursor
            break
        end
    end
    if not index then
        return nil
    end
    while tokens[index] and tokens[index].value ~= "=" do
        index = index + 1
    end
    if not tokens[index] then
        return nil
    end
    index = index + 1
    local token = tokens[index]
    if not token then
        return nil
    end
    local inferred = nil
    if token.value == "new" then
        local next_token = tokens[index + 1]
        if next_token then
            if next_token.value == "[" then
                inferred = "[]"
            else
                inferred = next_token.value:gsub("^@", "")
            end
        end
    elseif token.type == "string" then
        inferred = "string"
    elseif token.type == "char" then
        inferred = "char"
    elseif token.type == "number" then
        local value = token.value
        if value:find("[fF]") then
            inferred = "float"
        elseif value:find("[mM]") then
            inferred = "decimal"
        elseif value:find("[dD]") then
            inferred = "double"
        elseif value:find("[lL]") then
            inferred = "long"
        elseif value:find("%.") or value:find("[eE]") then
            inferred = "double"
        else
            inferred = "int"
        end
    elseif token.value == "true" or token.value == "false" then
        inferred = "bool"
    elseif token.value == "await" then
        local inner = A.type_before(model, index + 2)
        if inner then
            local match = inner:match("Task<(.*)>")
            inferred = match or inner
        end
    elseif token.value == "default" or token.value == "typeof" then
        if tokens[index + 1] and tokens[index + 1].value == "(" then
            local inner = tokens[index + 2]
            if token.value == "typeof" then
                inferred = "Type"
            elseif inner then
                inferred = inner.value
            end
        end
    elseif token.value == "(" then
        local inner = tokens[index + 1]
        if inner then
            inferred = inner.value
        end
    else
        local identifier = {
            value = token.value:gsub("^@", ""),
            start = token.start,
            ["end"] = token["end"],
            index = index,
        }
        local symbol = A.resolve_identifier(model, identifier, token.start)
        if symbol then
            if type_kinds[symbol.kind] then
                local next_token = tokens[index + 1]
                if next_token and next_token.value == "(" then
                    inferred = symbol.name
                else
                    inferred = symbol.name
                end
            elseif symbol.type and symbol.type ~= "" then
                inferred = symbol.type
            end
        elseif token.value == "new" then
            inferred = "object"
        end
    end
    local_symbol.inferredType = inferred
    return inferred
end

function A.type_of_symbol(model, symbol)
    if not symbol then
        return nil
    end
    if symbol.isLocal then
        return A.type_of_local(model, symbol)
    end
    if type_kinds[symbol.kind] then
        return symbol.name
    end
    if symbol.type and symbol.type ~= "" then
        return symbol.type
    end
    return nil
end

function A.document_symbols(model)
    local function convert(node)
        local symbol = node.symbol
        if not symbol then
            return nil
        end
        local children = {}
        if node.children then
            for _, child in ipairs(node.children) do
                if type_kinds[child.kind] or member_kinds[child.kind] or
                    child.kind == "enumMember" then
                    local converted = convert(child)
                    if converted then
                        children[#children + 1] = converted
                    end
                end
            end
        end
        return {
            name = node.name,
            detail = node.kind,
            kind = symbol.symbolKind,
            range = symbol.range,
            selectionRange = symbol.selectionRange,
            children = children,
        }
    end

    local result = {}
    for _, child in ipairs(model.unit.children or {}) do
        if child.kind == "namespace" then
            local symbol = child.symbol
            local children = {}
            for _, nested in ipairs(child.children or {}) do
                if type_kinds[nested.kind] then
                    local converted = convert(nested)
                    if converted then
                        children[#children + 1] = converted
                    end
                end
            end
            if symbol then
                result[#result + 1] = {
                    name = child.name,
                    detail = "namespace",
                    kind = 3,
                    range = symbol.range,
                    selectionRange = symbol.selectionRange,
                    children = children,
                }
            end
        elseif type_kinds[child.kind] then
            local converted = convert(child)
            if converted then
                result[#result + 1] = converted
            end
        end
    end
    return result
end

function A.workspace_symbols(query, limit)
    A.index_workspace(false)
    query = (query or ""):lower()
    limit = limit or 256
    local result = {}
    local seen = {}
    for _, model in ipairs(A.all_models()) do
        for _, symbol in ipairs(model.symbols) do
            if (type_kinds[symbol.kind] or member_kinds[symbol.kind]) and
                (query == "" or symbol.name:lower():find(query, 1, true)) then
                local key = symbol.uri .. "|" .. symbol.fullName .. "|" ..
                    symbol.start
                if not seen[key] then
                    seen[key] = true
                    result[#result + 1] = symbol
                    if #result >= limit then
                        return result
                    end
                end
            end
        end
    end
    table.sort(result, function(left, right)
        if left.name:lower() == query and right.name:lower() ~= query then
            return true
        end
        if right.name:lower() == query and left.name:lower() ~= query then
            return false
        end
        return left.name < right.name
    end)
    return result
end

function A.references(model, symbol, include_declaration)
    if not symbol then
        return {}
    end
    local name = symbol.name
    local result = {}
    local seen = {}
    local function add(uri, start_offset, end_offset)
        local key = uri .. "|" .. start_offset
        if seen[key] then
            return
        end
        seen[key] = true
        result[#result + 1] = {
            uri = uri,
            start = start_offset,
            ["end"] = end_offset,
        }
    end
    if include_declaration and symbol.uri and symbol.nameStart then
        add(symbol.uri, symbol.nameStart, symbol.nameEnd)
    end
    for _, other in ipairs(A.all_models()) do
        for _, entry in ipairs(other.identifiers) do
            if entry.value == name then
                local add_this = true
                if entry.declaration == symbol then
                    add_this = include_declaration
                end
                if add_this then
                    add(other.uri, entry.start, entry["end"])
                end
            end
        end
    end
    return result
end

function A.lsp_diagnostic(model, diagnostic)
    return {
        range = model.source:range(diagnostic.start, diagnostic["end"]),
        severity = diagnostic.severity or 1,
        code = diagnostic.code,
        source = "csharp",
        message = diagnostic.message,
        tags = diagnostic.tags,
        data = diagnostic.data,
        relatedInformation = diagnostic.relatedInformation,
    }
end

function A.render_signature(symbol)
    if not symbol then
        return ""
    end
    if symbol.node then
        if type_kinds[symbol.kind] then
            return describe_type_node(symbol.node)
        end
        return describe_member_node(symbol.node)
    end
    if symbol.signature then
        return symbol.signature
    end
    return symbol.name or ""
end

function A.describe_member(member)
    if not member then
        return ""
    end
    if member.signature then
        return member.signature
    end
    if member.symbol and member.symbol.node then
        return describe_member_node(member.symbol.node)
    end
    return member.name or ""
end

function A.locals_visible_at(model, offset)
    local result = {}
    for _, local_symbol in ipairs(model.locals) do
        local scope = local_symbol.scopeNode
        if local_symbol.start <= offset and (not scope or offset <= scope["end"]) then
            result[#result + 1] = local_symbol
        end
    end
    return result
end

function A.current_type(model, offset)
    local node = A.enclosing_type(model, offset)
    if node then
        return node.symbol
    end
    return nil
end

function A.current_namespace(model, offset)
    local node = A.enclosing_namespace(model, offset)
    return node and node.symbol or nil
end

function A.nodes_containing(model, offset)
    local result = {}
    for _, node in ipairs(model.node_list) do
        if node.start <= offset and offset <= node["end"] then
            result[#result + 1] = node
        end
    end
    table.sort(result, function(left, right)
        return (left["end"] - left.start) < (right["end"] - right.start)
    end)
    return result
end

function A.documents_from_symbols(symbols, default_uri)
    local result = {}
    for _, symbol in ipairs(symbols or {}) do
        if symbol.uri and symbol.selectionRange then
            result[#result + 1] = {
                uri = symbol.uri,
                range = symbol.selectionRange,
            }
        end
    end
    if #result == 0 and default_uri then
        return nil
    end
    return result
end

function A.imported_namespaces(model)
    local result = {}
    for _, using in ipairs(model.usings) do
        if not using.alias and not using.static then
            result[#result + 1] = using.name
        end
    end
    return result
end

function A.using_node_at(model, offset)
    return A.node_at(model, offset, function(node)
        return node.kind == "using"
    end)
end

function A.document_text(model)
    return model.text
end

function A.describe_type(type_name)
    local builtin = builtins.type(type_name)
    if builtin then
        return builtin
    end
    return nil
end

return A
