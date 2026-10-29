local util = require("cs_util")
local lexer = require("cs_lexer")
local analysis = require("cs_analysis")
local builtins = require("cs_builtins")
local formatter = require("cs_format")

local F = {}

local function document_uri(params)
    return params and params.textDocument and params.textDocument.uri
end

local function get_model(params)
    local uri = document_uri(params)
    if not uri then
        return nil
    end
    return analysis.get_model(uri)
end

local function offset_at(model, position)
    return model.source:offset(position.line, position.character)
end

local function symbol_kind(symbol)
    if symbol.symbolKind then
        return symbol.symbolKind
    end
    return analysis.symbol_kinds[symbol.kind] or 13
end

local function completion_kind(symbol)
    if symbol.completionKind then
        return symbol.completionKind
    end
    return analysis.completion_kinds[symbol.kind] or 6
end

local function location(symbol)
    if not symbol or not symbol.uri or not symbol.selectionRange then
        return nil
    end
    return { uri = symbol.uri, range = symbol.selectionRange }
end

local function locations_from_symbols(symbols)
    local result = {}
    for _, symbol in ipairs(symbols or {}) do
        local value = location(symbol)
        if value then
            result[#result + 1] = value
        end
    end
    return result
end

local function markdown(value)
    return { kind = "markdown", value = value }
end

local function code_block(value)
    return "```csharp\n" .. value .. "\n```"
end

local keyword_docs = {
    ["abstract"] = "Indicates that a type or member is abstract and must be implemented by a derived type.",
    ["async"] = "Marks a method, lambda expression, or anonymous method as asynchronous.",
    ["await"] = "Suspends evaluation of an async method until the awaited operation completes.",
    ["base"] = "Accesses members of the base class.",
    ["bool"] = "Represents a Boolean value.",
    ["break"] = "Exits the nearest enclosing loop or switch statement.",
    ["case"] = "Labels a switch section.",
    ["catch"] = "Handles an exception thrown in a try block.",
    ["class"] = "Declares a reference type.",
    ["const"] = "Declares a constant value that cannot be modified.",
    ["continue"] = "Passes control to the next iteration of the enclosing loop.",
    ["default"] = "Specifies the default switch section or produces the default value of a type.",
    ["delegate"] = "Declares a type that references a method.",
    ["do"] = "Executes a statement repeatedly until a condition becomes true.",
    ["else"] = "Executes a statement when the associated if condition is false.",
    ["enum"] = "Declares an enumeration.",
    ["event"] = "Declares an event member.",
    ["explicit"] = "Declares an explicit user-defined conversion operator.",
    ["extern"] = "Indicates that a method is implemented externally.",
    ["false"] = "The Boolean false literal.",
    ["finally"] = "Executes cleanup code after a try block completes.",
    ["fixed"] = "Pins a variable in memory for the duration of a statement.",
    ["for"] = "Executes a statement repeatedly while a condition holds.",
    ["foreach"] = "Enumerates the elements of a collection.",
    ["get"] = "Declares the accessor that reads a property value.",
    ["goto"] = "Transfers control to a labeled statement.",
    ["if"] = "Executes a statement when a condition is true.",
    ["implicit"] = "Declares an implicit user-defined conversion operator.",
    ["in"] = "Specifies that a parameter is passed by reference without modification.",
    ["init"] = "Declares an accessor that assigns a property value during object initialization.",
    ["interface"] = "Declares an interface.",
    ["internal"] = "Limits access to the current assembly.",
    ["is"] = "Tests whether an expression is compatible with a type.",
    ["lock"] = "Serializes access to a critical section.",
    ["namespace"] = "Declares a namespace.",
    ["new"] = "Creates an object or hides an inherited member.",
    ["null"] = "The null reference literal.",
    ["operator"] = "Declares a user-defined operator.",
    ["out"] = "Specifies that a parameter is passed by reference and assigned by the method.",
    ["override"] = "Provides a new implementation of an inherited virtual member.",
    ["params"] = "Specifies a parameter array.",
    ["partial"] = "Declares that a type or method may be defined in multiple parts.",
    ["private"] = "Limits access to the containing type.",
    ["protected"] = "Limits access to the containing type or derived types.",
    ["public"] = "Allows access from any code.",
    ["readonly"] = "Indicates that a field can only be assigned during initialization.",
    ["record"] = "Declares a record type with value semantics.",
    ["ref"] = "Passes a parameter by reference.",
    ["required"] = "Requires callers to initialize a member with an object initializer.",
    ["return"] = "Returns control from the enclosing method.",
    ["sealed"] = "Prevents a class from being inherited or a member from being overridden.",
    ["set"] = "Declares the accessor that writes a property value.",
    ["static"] = "Declares a member that belongs to the type rather than an instance.",
    ["string"] = "Represents a sequence of UTF-16 characters.",
    ["struct"] = "Declares a value type.",
    ["switch"] = "Selects a switch section based on a value or pattern.",
    ["this"] = "Refers to the current instance of the containing type.",
    ["throw"] = "Raises an exception.",
    ["true"] = "The Boolean true literal.",
    ["try"] = "Defines a block that is protected against exceptions.",
    ["typeof"] = "Obtains the System.Type object for a type.",
    ["unchecked"] = "Suppresses overflow checking.",
    ["unsafe"] = "Enables unsafe code in the enclosing context.",
    ["using"] = "Imports a namespace or ensures that a resource is disposed.",
    ["var"] = "Declares an implicitly typed local variable.",
    ["virtual"] = "Allows a member to be overridden in a derived class.",
    ["void"] = "Specifies that a method does not return a value.",
    ["volatile"] = "Indicates that a field may be modified by multiple threads.",
    ["when"] = "Adds a condition to a catch clause or case label.",
    ["where"] = "Specifies constraints on a generic type parameter.",
    ["while"] = "Executes a statement repeatedly while a condition holds.",
    ["yield"] = "Returns an element of an iterator.",
}

local snippets = {
    {
        label = "class",
        detail = "class declaration",
        insert = "class ${1:Name}\n{\n    $0\n}",
    },
    {
        label = "struct",
        detail = "struct declaration",
        insert = "struct ${1:Name}\n{\n    $0\n}",
    },
    {
        label = "interface",
        detail = "interface declaration",
        insert = "interface ${1:Name}\n{\n    $0\n}",
    },
    {
        label = "enum",
        detail = "enum declaration",
        insert = "enum ${1:Name}\n{\n    $0\n}",
    },
    {
        label = "record",
        detail = "record declaration",
        insert = "record ${1:Name}($0);",
    },
    {
        label = "namespace",
        detail = "namespace declaration",
        insert = "namespace ${1:Namespace};",
    },
    {
        label = "method",
        detail = "method declaration",
        insert = "public ${1:void} ${2:Name}($3)\n{\n    $0\n}",
    },
    {
        label = "property",
        detail = "auto property",
        insert = "public ${1:object} ${2:Name} { get; set; }",
    },
    {
        label = "constructor",
        detail = "constructor",
        insert = "public ${1:Type}($2)\n{\n    $0\n}",
    },
    {
        label = "if",
        detail = "if statement",
        insert = "if (${1:condition})\n{\n    $0\n}",
    },
    {
        label = "ifelse",
        detail = "if/else statement",
        insert = "if (${1:condition})\n{\n    $2\n}\nelse\n{\n    $0\n}",
    },
    {
        label = "for",
        detail = "for loop",
        insert = "for (int ${1:i} = 0; $1 < ${2:length}; $1++)\n{\n    $0\n}",
    },
    {
        label = "foreach",
        detail = "foreach loop",
        insert = "foreach (var ${1:item} in ${2:collection})\n{\n    $0\n}",
    },
    {
        label = "while",
        detail = "while loop",
        insert = "while (${1:condition})\n{\n    $0\n}",
    },
    {
        label = "switch",
        detail = "switch statement",
        insert = "switch (${1:value})\n{\n    case ${2:case}:\n        $3\n        break;\n    default:\n        $0\n        break;\n}",
    },
    {
        label = "try",
        detail = "try/catch statement",
        insert = "try\n{\n    $1\n}\ncatch (${2:Exception} ex)\n{\n    $0\n}",
    },
    {
        label = "using",
        detail = "using statement",
        insert = "using (var ${1:resource} = ${2:expression})\n{\n    $0\n}",
    },
    {
        label = "cw",
        detail = "Console.WriteLine",
        insert = "Console.WriteLine($0);",
    },
    {
        label = "prop",
        detail = "property with backing field",
        insert = "public ${1:object} ${2:Name} { get; set; } = ${0:default};",
    },
}

local function add_item(items, seen, item, group, prefix)
    local label = item.label
    if not label then
        return
    end
    if prefix and prefix ~= "" and
        label:lower():find(prefix:lower(), 1, true) ~= 1 then
        return
    end
    local key = label .. "|" .. tostring(item.kind)
    if seen[key] then
        return
    end
    seen[key] = true
    item.sortText = (group or "9") .. "|" .. label:lower()
    item.filterText = item.filterText or label
    items[#items + 1] = item
end

local function completion_item(label, kind, detail, documentation, insert,
    insert_format, prefix_range)
    local item = {
        label = label,
        kind = kind,
        detail = detail,
        documentation = documentation and markdown(documentation) or nil,
        insertTextFormat = insert_format,
        textEdit = prefix_range and {
            range = prefix_range,
            newText = insert or label,
        } or nil,
        insertText = not prefix_range and (insert or label) or nil,
    }
    return item
end

local function prefix_range_at(model, offset)
    local identifier = analysis.identifier_at(model, offset)
    if identifier then
        return model.source:range(identifier.start, identifier["end"]),
            identifier.value
    end
    return model.source:range(offset, offset), ""
end

local function add_symbol_completion(model, items, seen, symbol, group, prefix,
    prefix_range)
    local detail = symbol.type
    if analysis.type_kinds[symbol.kind] then
        detail = symbol.fullName or symbol.name
    elseif not detail or detail == "" then
        detail = symbol.container
    end
    local documentation = symbol.documentation
    if documentation == "" then
        documentation = nil
    end
    add_item(items, seen, completion_item(symbol.name, completion_kind(symbol),
        detail, documentation, nil, nil, prefix_range), group, prefix)
end

local function add_builtin_completion(items, seen, value, group, prefix,
    prefix_range)
    local kind = analysis.completion_kinds[value.kind] or 7
    add_item(items, seen, completion_item(value.name, kind, value.fullName,
        value.documentation, nil, nil, prefix_range), group, prefix)
end

local function namespace_member_completion(model, dot_index, prefix,
    prefix_range)
    local items = {}
    local seen = {}
    local previous = model.tokens[dot_index - 1]
    if not previous or previous.type ~= "identifier" then
        return nil
    end
    local identifier = {
        value = previous.value:gsub("^@", ""),
        start = previous.start,
        ["end"] = previous["end"],
        index = dot_index - 1,
    }
    local symbol = analysis.resolve_identifier(model, identifier,
        previous.start)
    local namespace = nil
    if symbol and symbol.kind == "namespace" then
        namespace = symbol.name
    elseif symbol and symbol.target then
        namespace = symbol.target
    end
    if not namespace then
        for _, candidate in ipairs(builtins.namespaces) do
            if candidate == identifier.value or
                candidate:sub(-#identifier.value - 1) == "." ..
                    identifier.value then
                namespace = candidate
                break
            end
        end
    end
    if not namespace then
        return nil
    end
    local values = {}
    local builtin_names = {}
    for name, value in pairs(builtins.types) do
        if value.namespace == namespace and not builtin_names[name] then
            builtin_names[name] = true
            values[#values + 1] = {
                name = name,
                kind = value.kind,
                fullName = value.fullName,
                documentation = value.documentation,
            }
        end
    end
    for _, other in ipairs(analysis.all_models()) do
        for _, type_symbol in ipairs(other.types) do
            if type_symbol.container == namespace then
                values[#values + 1] = {
                    name = type_symbol.name,
                    kind = type_symbol.kind,
                    fullName = type_symbol.fullName,
                    documentation = type_symbol.documentation,
                    symbol = type_symbol,
                }
            end
        end
    end
    for _, value in ipairs(values) do
        local kind = analysis.completion_kinds[value.kind] or 7
        add_item(items, seen, completion_item(value.name, kind,
            value.fullName, value.documentation, nil, nil, prefix_range), "2",
            prefix)
    end
    if #items == 0 then
        return nil
    end
    return items
end

local function member_completion(model, dot_index, prefix, prefix_range)
    local namespace_items = namespace_member_completion(model, dot_index,
        prefix, prefix_range)
    if namespace_items then
        return namespace_items
    end
    local items = {}
    local seen = {}
    local owner_type = analysis.type_before(model, dot_index)
    if not owner_type then
        return items
    end
    local members = analysis.members_of_type(model, owner_type)
    for _, member in ipairs(members) do
        local documentation = member.documentation
        if documentation == "" then
            documentation = nil
        end
        local kind = analysis.completion_kinds[member.kind] or 5
        local insert = member.name
        local insert_format = nil
        if kind == 2 or kind == 4 then
            insert = member.name .. "($0)"
            insert_format = 2
        end
        add_item(items, seen, completion_item(member.name, kind,
            member.signature or member.type, documentation, insert,
            insert_format, prefix_range), "1", prefix)
    end
    return items
end

local function type_completion(model, prefix, prefix_range)
    local items = {}
    local seen = {}
    for _, symbol in ipairs(model.types) do
        add_symbol_completion(model, items, seen, symbol, "2", prefix,
            prefix_range)
    end
    for _, other in ipairs(analysis.all_models()) do
        if other ~= model then
            for _, symbol in ipairs(other.types) do
                add_symbol_completion(model, items, seen, symbol, "2", prefix,
                    prefix_range)
            end
        end
    end
    for _, value in ipairs(builtins.all_types()) do
        if value.name and not value.alias then
            add_builtin_completion(items, seen, value, "3", prefix, prefix_range)
        end
    end
    return items
end

local function namespace_completion(prefix, prefix_range)
    local items = {}
    local seen = {}
    for _, namespace in ipairs(builtins.namespaces) do
        add_item(items, seen, completion_item(namespace, 9, "namespace", nil,
            nil, nil, prefix_range), "0", prefix)
    end
    return items
end

local function general_completion(model, offset, prefix, prefix_range)
    local items = {}
    local seen = {}

    for _, local_symbol in ipairs(analysis.locals_visible_at(model, offset)) do
        local type_name = analysis.type_of_local(model, local_symbol)
        add_item(items, seen, completion_item(local_symbol.name, 6,
            type_name or local_symbol.type, local_symbol.documentation, nil,
            nil, prefix_range), "0", prefix)
    end

    local current = analysis.current_type(model, offset)
    if current then
        local members = analysis.members_of_type(model, current.name)
        for _, member in ipairs(members) do
            if not member.static then
                local documentation = member.documentation
                if documentation == "" then
                    documentation = nil
                end
                local kind = analysis.completion_kinds[member.kind] or 5
                local insert = member.name
                local insert_format = nil
                if kind == 2 or kind == 4 then
                    insert = member.name .. "($0)"
                    insert_format = 2
                end
                add_item(items, seen, completion_item(member.name, kind,
                    member.signature or member.type, documentation, insert,
                    insert_format, prefix_range), "1", prefix)
            end
        end
    end

    for _, using in ipairs(model.usings) do
        if using.alias then
            add_item(items, seen, completion_item(using.alias, 9,
                "alias for " .. using.name, nil, nil, nil, prefix_range), "1",
                prefix)
        end
    end

    for _, symbol in ipairs(model.types) do
        add_symbol_completion(model, items, seen, symbol, "2", prefix,
            prefix_range)
    end
    for _, other in ipairs(analysis.all_models()) do
        if other ~= model then
            for _, symbol in ipairs(other.types) do
                add_symbol_completion(model, items, seen, symbol, "2", prefix,
                    prefix_range)
            end
        end
    end

    for _, value in ipairs(builtins.all_types()) do
        if value.name and not value.alias then
            add_builtin_completion(items, seen, value, "3", prefix, prefix_range)
        end
    end

    local keywords = {
        "abstract", "as", "async", "await", "base", "bool", "break", "byte",
        "case", "catch", "char", "checked", "class", "const", "continue",
        "decimal", "default", "delegate", "do", "double", "else", "enum",
        "event", "explicit", "extern", "false", "finally", "fixed", "float",
        "for", "foreach", "get", "global", "goto", "if", "implicit", "in",
        "init", "int", "interface", "internal", "is", "lock", "long",
        "namespace", "new", "null", "object", "operator", "out", "override",
        "params", "partial", "private", "protected", "public", "readonly",
        "record", "ref", "required", "return", "sbyte", "sealed", "set",
        "short", "sizeof", "stackalloc", "static", "string", "struct",
        "switch", "this", "throw", "true", "try", "typeof", "uint", "ulong",
        "unchecked", "unsafe", "ushort", "using", "var", "virtual", "void",
        "volatile", "when", "where", "while", "yield",
    }
    for _, keyword in ipairs(keywords) do
        add_item(items, seen, completion_item(keyword, 14, "keyword",
            keyword_docs[keyword], nil, nil, prefix_range), "4", prefix)
    end

    for _, snippet in ipairs(snippets) do
        add_item(items, seen, completion_item(snippet.label, 15,
            snippet.detail, nil, snippet.insert, 2, prefix_range), "5", prefix)
    end

    return items
end

function F.hover(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local token = analysis.token_at(model, offset)
    if not token then
        return nil
    end
    local range = model.source:range(token.start, token["end"])
    if token.type == "keyword" or token.type == "identifier" then
        local identifier = analysis.identifier_at(model, offset)
        if identifier then
            local symbol = analysis.resolve_identifier(model, identifier,
                offset)
            if symbol then
                local parts = {}
                if symbol.isLocal then
                    local type_name = analysis.type_of_local(model, symbol)
                    parts[#parts + 1] = code_block((type_name and
                        (type_name .. " ") or "") .. symbol.name)
                elseif analysis.type_kinds[symbol.kind] then
                    parts[#parts + 1] = code_block(analysis.render_signature(
                        symbol))
                elseif symbol.kind == "namespace" then
                    parts[#parts + 1] = code_block("namespace " .. symbol.name)
                elseif symbol.signature then
                    parts[#parts + 1] = code_block(symbol.signature)
                elseif symbol.node then
                    parts[#parts + 1] = code_block(analysis.render_signature(
                        symbol))
                else
                    parts[#parts + 1] = code_block(symbol.name)
                end
                if symbol.documentation and symbol.documentation ~= "" then
                    parts[#parts + 1] = symbol.documentation
                end
                return {
                    contents = markdown(table.concat(parts, "\n\n")),
                    range = range,
                }
            end
            local builtin = builtins.type(identifier.value)
            if builtin then
                local parts = { code_block("public " .. builtin.kind .. " " ..
                    (builtin.fullName or builtin.name)) }
                if builtin.documentation and builtin.documentation ~= "" then
                    parts[#parts + 1] = builtin.documentation
                end
                return {
                    contents = markdown(table.concat(parts, "\n\n")),
                    range = range,
                }
            end
            for _, using in ipairs(model.usings) do
                if using.alias == identifier.value then
                    return {
                        contents = markdown(code_block("using " .. using.alias ..
                            " = " .. using.name)),
                        range = range,
                    }
                end
            end
        end
        if token.type == "keyword" and keyword_docs[token.value] then
            return {
                contents = markdown(code_block(token.value) .. "\n\n" ..
                    keyword_docs[token.value]),
                range = range,
            }
        end
    elseif token.type == "string" then
        return { contents = markdown(code_block("string")), range = range }
    elseif token.type == "char" then
        return { contents = markdown(code_block("char")), range = range }
    elseif token.type == "number" then
        return { contents = markdown(code_block("number")), range = range }
    end
    return nil
end

function F.completion(params, ctx)
    local model = get_model(params)
    if not model then
        return { isIncomplete = false, items = {} }
    end
    local offset = offset_at(model, params.position)
    local range, prefix = prefix_range_at(model, offset)
    local token = analysis.token_at(model, offset)
    local index = #model.tokens + 1
    if token then
        local token_index = model.token_index_by_start[token.start]
        if token_index then
            if token.type == "identifier" and offset <= token["end"] then
                index = token_index
            elseif offset >= token["end"] then
                index = token_index + 1
            else
                index = token_index
            end
        end
    end
    local previous = model.tokens[index - 1]
    local before_previous = model.tokens[index - 2]

    if previous and (previous.value == "." or previous.value == "?.") then
        local dot_index = index - 1
        return {
            isIncomplete = false,
            items = member_completion(model, dot_index, prefix, range),
        }
    end
    if previous and previous.value == "using" then
        return {
            isIncomplete = false,
            items = namespace_completion(prefix, range),
        }
    end
    if previous and previous.value == "namespace" then
        return {
            isIncomplete = false,
            items = namespace_completion(prefix, range),
        }
    end
    if previous and (previous.value == "new" or previous.value == "typeof" or
        previous.value == "sizeof" or previous.value == "default") then
        return { isIncomplete = false, items = type_completion(model, prefix,
            range) }
    end
    if previous and previous.value == ":" and before_previous then
        local enclosing = analysis.enclosing_type(model, offset)
        if enclosing then
            return { isIncomplete = false, items = type_completion(model,
                prefix, range) }
        end
    end
    if previous and previous.value == "[" then
        return { isIncomplete = false, items = type_completion(model, prefix,
            range) }
    end
    return {
        isIncomplete = false,
        items = general_completion(model, offset, prefix, range),
    }
end

function F.completion_resolve(item, ctx)
    return item
end

local function find_call_open(model, offset)
    local tokens = model.tokens
    local token = analysis.token_at(model, offset)
    local index = token and model.token_index_by_start[token.start] or
        (#tokens + 1)
    local depth = 0
    local open_index = nil
    for cursor = index, 1, -1 do
        local value = tokens[cursor] and tokens[cursor].value
        if value == ")" then
            depth = depth + 1
        elseif value == "(" then
            if depth == 0 then
                open_index = cursor
                break
            end
            depth = depth - 1
        end
    end
    if not open_index then
        return nil
    end
    local name_index = open_index - 1
    local name_token = tokens[name_index]
    if not name_token then
        return nil
    end
    if name_token.value == ">" or name_token.value == ">>" then
        local cursor = name_index
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
        name_index = cursor
        name_token = tokens[cursor]
    end
    return open_index, name_index, name_token
end

local function signature_for_symbol(model, symbol)
    if not symbol then
        return nil
    end
    local label = nil
    local parameters = {}
    if symbol.params then
        local param_labels = {}
        for _, parameter in ipairs(symbol.params) do
            local text = parameter.name
            if parameter.type and parameter.type ~= "" then
                text = parameter.type .. " " .. text
            end
            param_labels[#param_labels + 1] = text
            parameters[#parameters + 1] = { label = text }
        end
        local prefix = ""
        if symbol.type and symbol.type ~= "" then
            prefix = symbol.type .. " "
        end
        label = prefix .. symbol.name .. "(" .. table.concat(param_labels,
            ", ") .. ")"
    elseif symbol.signature then
        label = symbol.signature
        local inner = symbol.signature:match("%((.*)%)")
        if inner then
            for part in inner:gmatch("[^,]+") do
                parameters[#parameters + 1] = { label = util.trim(part) }
            end
        end
    else
        return nil
    end
    return {
        label = label,
        documentation = symbol.documentation and
            markdown(symbol.documentation) or nil,
        parameters = parameters,
    }
end

local function resolve_call_symbol(model, open_index, name_index, name_token)
    if not name_token then
        return nil
    end
    local previous = model.tokens[name_index - 1]
    if previous and previous.value == "new" then
        local type_symbol = analysis.find_type(model,
            name_token.value:gsub("^@", ""))
        if type_symbol then
            for _, child in ipairs(type_symbol.node.children or {}) do
                if child.kind == "constructor" then
                    return child.symbol
                end
            end
            return type_symbol
        end
        local builtin = builtins.type(name_token.value)
        if builtin then
            return builtin
        end
    end
    if model.tokens[name_index - 1] and
        (model.tokens[name_index - 1].value == "." or
            model.tokens[name_index - 1].value == "?.") then
        local owner_type = analysis.type_before(model, name_index - 1)
        local member = analysis.find_member(model, owner_type,
            name_token.value:gsub("^@", ""))
        if member then
            return member.symbol or member
        end
    end
    local identifier = {
        value = name_token.value:gsub("^@", ""),
        start = name_token.start,
        ["end"] = name_token["end"],
        index = name_index,
    }
    return analysis.resolve_identifier(model, identifier, name_token.start)
end

function F.signature_help(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local open_index, name_index, name_token = find_call_open(model, offset)
    if not open_index then
        return nil
    end
    local symbol = resolve_call_symbol(model, open_index, name_index, name_token)
    if not symbol then
        return nil
    end
    local signature = signature_for_symbol(model, symbol)
    if not signature then
        return nil
    end
    local active_parameter = 0
    local depth = 0
    local token = analysis.token_at(model, offset)
    local cursor = token and model.token_index_by_start[token.start] or
        (#model.tokens + 1)
    for index = open_index + 1, cursor - 1 do
        local value = model.tokens[index] and model.tokens[index].value
        if value == "(" or value == "[" or value == "{" then
            depth = depth + 1
        elseif value == ")" or value == "]" or value == "}" then
            depth = math.max(0, depth - 1)
        elseif value == "," and depth == 0 then
            active_parameter = active_parameter + 1
        end
    end
    return {
        signatures = { signature },
        activeSignature = 0,
        activeParameter = active_parameter,
    }
end

local function symbol_definition(symbol)
    if symbol and symbol.symbol and symbol.symbol.uri then
        symbol = symbol.symbol
    end
    return location(symbol)
end

function F.definition(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local identifier, symbol = analysis.resolve_at(model, offset)
    if not symbol then
        return nil
    end
    local value = symbol_definition(symbol)
    if value then
        return value
    end
    return nil
end

function F.declaration(params, ctx)
    return F.definition(params, ctx)
end

function F.type_definition(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol then
        return nil
    end
    if analysis.type_kinds[symbol.kind] then
        return location(symbol)
    end
    local type_name = analysis.type_of_symbol(model, symbol)
    if type_name then
        local type_symbol = analysis.find_type(model, type_name)
        if type_symbol then
            return location(type_symbol)
        end
    end
    return nil
end

function F.implementation(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol then
        return nil
    end
    local result = {}
    if analysis.type_kinds[symbol.kind] then
        for _, other in ipairs(analysis.all_models()) do
            for _, candidate in ipairs(other.types) do
                for _, base in ipairs(candidate.node.baseTypes or {}) do
                    local simple = base.name:gsub("<.*$", "")
                    if simple == symbol.name then
                        local value = location(candidate)
                        if value then
                            result[#result + 1] = value
                        end
                    end
                end
            end
        end
    elseif analysis.member_kinds[symbol.kind] then
        for _, other in ipairs(analysis.all_models()) do
            for _, candidate in ipairs(other.members) do
                if candidate.name == symbol.name and candidate ~= symbol then
                    local enclosing = candidate.container
                    for _, type_symbol in ipairs(other.types) do
                        if type_symbol.fullName == enclosing then
                            for _, base in ipairs(type_symbol.node.baseTypes or {}) do
                                local simple = base.name:gsub("<.*$", "")
                                if simple == symbol.container or
                                    symbol.container:find(simple, 1, true) then
                                    local value = location(candidate)
                                    if value then
                                        result[#result + 1] = value
                                    end
                                end
                            end
                        end
                    end
                end
            end
        end
    end
    if #result == 0 then
        local value = symbol_definition(symbol)
        if value then
            result[#result + 1] = value
        end
    end
    return result
end

local function local_references(model, symbol, include_declaration)
    local result = {}
    local scope = symbol.scopeNode
    for _, entry in ipairs(model.identifiers) do
        if entry.value == symbol.name and (not scope or (scope.start <=
            entry.start and entry.start <= scope["end"])) then
            local resolved = analysis.resolve_identifier(model, entry,
                entry.start)
            if resolved == symbol or entry.declaration == symbol then
                result[#result + 1] = {
                    uri = model.uri,
                    range = model.source:range(entry.start, entry["end"]),
                }
            end
        end
    end
    if include_declaration then
        local exists = false
        for _, item in ipairs(result) do
            if item.range.start.line == symbol.selectionRange.start.line and
                item.range.start.character ==
                    symbol.selectionRange.start.character then
                exists = true
            end
        end
        if not exists then
            result[#result + 1] = { uri = symbol.uri, range =
                symbol.selectionRange }
        end
    end
    return result
end

function F.references(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol then
        return {}
    end
    local include_declaration = params.context and
        params.context.includeDeclaration ~= false
    if symbol.isLocal then
        return local_references(model, symbol, include_declaration)
    end
    local references = analysis.references(model, symbol, include_declaration)
    local result = {}
    for _, reference in ipairs(references) do
        local reference_model = nil
        for _, candidate in ipairs(analysis.all_models()) do
            if candidate.uri == reference.uri then
                reference_model = candidate
                break
            end
        end
        if reference_model then
            result[#result + 1] = {
                uri = reference.uri,
                range = reference_model.source:range(reference.start,
                    reference["end"]),
            }
        end
    end
    return result
end

function F.document_symbols(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    return analysis.document_symbols(model)
end

function F.workspace_symbols(params, ctx)
    local symbols = analysis.workspace_symbols(params.query, 512)
    local result = {}
    for _, symbol in ipairs(symbols) do
        result[#result + 1] = {
            name = symbol.name,
            kind = symbol_kind(symbol),
            location = location(symbol),
            containerName = symbol.container,
        }
    end
    return result
end

local function valid_identifier(name)
    if not name or name == "" then
        return false
    end
    if not name:match("^@?[%a_][%w_]*$") then
        return false
    end
    local bare = name:gsub("^@", "")
    return not lexer.is_reserved(bare)
end

function F.prepare_rename(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local identifier = analysis.identifier_at(model, offset)
    if not identifier then
        return nil
    end
    local token = identifier.token
    return {
        range = model.source:range(token.start, token["end"]),
        placeholder = identifier.value,
    }
end

function F.rename(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local new_name = params.newName
    if not valid_identifier(new_name) then
        return nil
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol then
        return nil
    end
    local changes = {}
    local function add(uri, start_offset, end_offset)
        if not uri then
            return
        end
        local target = changes[uri]
        if not target then
            target = {}
            changes[uri] = target
        end
        local reference_model = nil
        for _, candidate in ipairs(analysis.all_models()) do
            if candidate.uri == uri then
                reference_model = candidate
                break
            end
        end
        if reference_model then
            target[#target + 1] = {
                range = reference_model.source:range(start_offset, end_offset),
                newText = new_name,
            }
        end
    end

    if symbol.isLocal then
        local scope = symbol.scopeNode
        for _, entry in ipairs(model.identifiers) do
            if entry.value == symbol.name and (not scope or (scope.start <=
                entry.start and entry.start <= scope["end"])) then
                local resolved = analysis.resolve_identifier(model, entry,
                    entry.start)
                if resolved == symbol or entry.declaration == symbol then
                    add(model.uri, entry.start, entry["end"])
                end
            end
        end
    else
        if symbol.uri and symbol.nameStart then
            add(symbol.uri, symbol.nameStart, symbol.nameEnd)
        end
        for _, other in ipairs(analysis.all_models()) do
            for _, entry in ipairs(other.identifiers) do
                if entry.value == symbol.name then
                    local resolved = analysis.resolve_identifier(other, entry,
                        entry.start)
                    if resolved == symbol then
                        add(other.uri, entry.start, entry["end"])
                    elseif not resolved and not entry.declaration then
                        add(other.uri, entry.start, entry["end"])
                    end
                end
            end
        end
    end
    return { changes = changes }
end

local function line_range(model, start_offset, end_offset)
    local start_line = model.source:line_of(start_offset)
    local end_line = model.source:line_of(end_offset)
    return model.source:range(model.source:line_start(start_line),
        model.source:line_end(end_line))
end

local function remove_lines_edit(model, start_offset, end_offset)
    local start_line = model.source:line_of(start_offset)
    local next_line = model.source:line_of(end_offset)
    local start = model.source:line_start(start_line)
    local finish = next_line + 1 < model.source.line_count and
        model.source:line_start(next_line + 1) or model.source:line_end(next_line)
    return {
        range = model.source:range(start, finish),
        newText = "",
    }
end

local function quick_fix_actions(model, params)
    local actions = {}
    local diagnostics = params.context and params.context.diagnostics or {}
    for _, diagnostic in ipairs(diagnostics) do
        local data = diagnostic.data
        if type(data) == "table" then
            if data.fix == "removeAsync" then
                local edit_range = diagnostic.range
                actions[#actions + 1] = {
                    title = "Make method non-async",
                    kind = "quickfix",
                    diagnostics = { diagnostic },
                    edit = {
                        changes = {
                            [model.uri] = {
                                { range = edit_range, newText = "" },
                            },
                        },
                    },
                    isPreferred = true,
                }
            elseif data.fix == "removeUsing" then
                local edit = remove_lines_edit(model, data.start, data.finish)
                actions[#actions + 1] = {
                    title = "Remove duplicate using directive",
                    kind = "quickfix",
                    diagnostics = { diagnostic },
                    edit = {
                        changes = {
                            [model.uri] = { edit },
                        },
                    },
                    isPreferred = true,
                }
            elseif data.fix == "initializeVar" then
                local local_symbol = nil
                for _, candidate in ipairs(model.locals) do
                    if candidate.name == data.name and candidate.type == "var" then
                        local_symbol = candidate
                        break
                    end
                end
                if local_symbol then
                    for _, token in ipairs(model.tokens) do
                        if token.start < local_symbol.nameStart and
                            token.value == "var" then
                            actions[#actions + 1] = {
                                title = "Replace 'var' with 'object'",
                                kind = "quickfix",
                                diagnostics = { diagnostic },
                                edit = {
                                    changes = {
                                        [model.uri] = {
                                            {
                                                range = model.source:range(
                                                    token.start, token["end"]),
                                                newText = "object",
                                            },
                                        },
                                    },
                                },
                            }
                            break
                        end
                    end
                end
            end
        end
    end
    return actions
end

local function using_text(model, node)
    local text = model.source:range_text(node.start, node["end"])
    return util.trim(text)
end

local function has_comments_in_range(model, start_offset, end_offset)
    for _, comment in ipairs(model.comments) do
        if comment.start >= start_offset and comment["end"] <= end_offset then
            return true
        end
    end
    return false
end

function F.code_actions(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local actions = quick_fix_actions(model, params)
    local using_nodes = model.using_nodes
    if #using_nodes > 1 then
        local first = using_nodes[1]
        local last = using_nodes[#using_nodes]
        if not has_comments_in_range(model, first.start, last["end"]) then
            local sorted = {}
            local seen = {}
            for _, node in ipairs(using_nodes) do
                local text = using_text(model, node)
                local key = (node.alias and (node.alias .. "=") or "") ..
                    (node.name or "")
                if not seen[key] then
                    seen[key] = true
                    sorted[#sorted + 1] = { text = text, key = key }
                end
            end
            table.sort(sorted, function(left, right)
                return left.key:lower() < right.key:lower()
            end)
            local values = {}
            for _, item in ipairs(sorted) do
                values[#values + 1] = item.text
            end
            local joined = table.concat(values, "\n")
            local original = model.source:range_text(first.start, last["end"])
            if util.trim(original) ~= util.trim(joined) then
                actions[#actions + 1] = {
                    title = "Organize using directives",
                    kind = "source.organizeImports",
                    edit = {
                        changes = {
                            [model.uri] = {
                                {
                                    range = model.source:range(first.start,
                                        last["end"]),
                                    newText = joined,
                                },
                            },
                        },
                    },
                }
            end
        end
    end
    return actions
end

function F.code_action_resolve(action, ctx)
    return action
end

function F.formatting(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    return formatter.format(model, params.options)
end

function F.range_formatting(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    return formatter.range(model, params.range, params.options)
end

function F.on_type_formatting(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    return formatter.on_type(model, params.position, params.ch, params.options)
end

local semantic_types = {
    namespace = "namespace",
    class = "class",
    record = "class",
    struct = "struct",
    interface = "interface",
    enum = "enum",
    delegate = "type",
    method = "method",
    constructor = "method",
    destructor = "method",
    property = "property",
    indexer = "property",
    field = "property",
    event = "event",
    enumMember = "enumMember",
    parameter = "parameter",
    variable = "variable",
}

function F.semantic_tokens(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local result = {}
    local requested_range = params.range
    local function add_token(token, token_type, modifiers)
        local segments = model.source:segments(token.start, token["end"])
        for index, segment in ipairs(segments) do
            local position = {
                line = segment.line,
                character = segment.character,
            }
            local finish = {
                line = segment.line,
                character = segment.character + segment.length,
            }
            local include = true
            if requested_range then
                include = util.compare_positions(position,
                    requested_range["end"]) < 0 and
                    util.compare_positions(finish, requested_range.start) > 0
            end
            if include then
                local active = {}
                if index == 1 then
                    for _, modifier in ipairs(modifiers or {}) do
                        active[#active + 1] = modifier
                    end
                end
                result[#result + 1] = {
                    line = segment.line,
                    character = segment.character,
                    length = segment.length,
                    type = token_type,
                    modifiers = active,
                }
            end
        end
    end

    for _, token in ipairs(model.all_tokens) do
        if token.type == "comment" then
            local modifiers = {}
            if token.doc then
                modifiers[#modifiers + 1] = "documentation"
            end
            add_token(token, "comment", modifiers)
        elseif token.type == "preprocessor" then
            add_token(token, "macro", {})
        elseif token.type == "string" or token.type == "char" then
            add_token(token, "string", {})
        elseif token.type == "number" then
            add_token(token, "number", {})
        elseif token.type == "operator" then
            if token.value ~= "?" and token.value ~= ":" then
                add_token(token, "operator", {})
            end
        elseif token.type == "keyword" then
            if lexer.is_modifier(token.value) then
                add_token(token, "modifier", {})
            else
                add_token(token, "keyword", {})
            end
        elseif token.type == "identifier" then
            local symbol = model.declarations[token.start]
            local token_type = "variable"
            local modifiers = {}
            if symbol then
                token_type = semantic_types[symbol.kind] or "variable"
                if token.start == symbol.nameStart then
                    modifiers[#modifiers + 1] = "declaration"
                end
                for _, modifier in ipairs(symbol.modifiers or {}) do
                    if modifier == "static" or modifier == "readonly" or
                        modifier == "abstract" or modifier == "async" then
                        modifiers[#modifiers + 1] = modifier
                    end
                end
            else
                local next_token = model.tokens[
                    (model.token_index_by_start[token.start] or 0) + 1]
                local previous_token = model.tokens[
                    (model.token_index_by_start[token.start] or 0) - 1]
                if next_token and next_token.value == "(" then
                    token_type = "method"
                elseif previous_token and (previous_token.value == "." or
                    previous_token.value == "?.") then
                    token_type = "property"
                elseif token.value:match("^%u") then
                    token_type = "class"
                end
            end
            add_token(token, token_type, modifiers)
        end
    end
    return result
end

local function in_range(position, range)
    return util.compare_positions(range.start, position) <= 0 and
        util.compare_positions(position, range["end"]) <= 0
end

local function call_arguments(model, open_index)
    local tokens = model.tokens
    local arguments = {}
    local depth = 0
    local start_token = nil
    local cursor = open_index + 1
    while cursor <= #tokens do
        local token = tokens[cursor]
        local value = token.value
        if value == ")" and depth == 0 then
            if start_token then
                arguments[#arguments + 1] = start_token
            end
            return arguments, cursor
        end
        if value == "," and depth == 0 then
            if start_token then
                arguments[#arguments + 1] = start_token
            end
            start_token = nil
        else
            if value == "(" or value == "[" or value == "{" then
                depth = depth + 1
            elseif value == ")" or value == "]" or value == "}" then
                depth = math.max(0, depth - 1)
            end
            if not start_token then
                start_token = token
            end
        end
        cursor = cursor + 1
    end
    return arguments, #tokens + 1
end

function F.inlay_hints(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local settings = nil
    local workspace = hexal.lsp and hexal.lsp.workspace
    if workspace and workspace.configuration then
        local ok, configuration = pcall(workspace.configuration, "csharp")
        if ok and type(configuration) == "table" then
            settings = configuration
        end
    end
    local enabled = not settings or settings.inlayHints ~= false
    if not enabled then
        return {}
    end
    local hints = {}
    local requested = params.range

    for _, local_symbol in ipairs(model.locals) do
        if not local_symbol.isParameter and local_symbol.type == "var" then
            local inferred = analysis.type_of_local(model, local_symbol)
            if inferred and inferred ~= "" then
                local position = model.source:position(local_symbol.nameEnd)
                if in_range(position, requested) then
                    hints[#hints + 1] = {
                        position = position,
                        label = ": " .. inferred,
                        kind = 1,
                        paddingRight = true,
                        data = {
                            kind = "type",
                            name = local_symbol.name,
                        },
                    }
                end
            end
        end
    end

    local tokens = model.tokens
    for index, token in ipairs(tokens) do
        local open_index = nil
        local name_index = nil
        local name_token = nil
        if token.type == "identifier" and tokens[index + 1] and
            tokens[index + 1].value == "(" and
            not model.declarations[token.start] then
            open_index = index + 1
            name_index = index
            name_token = token
        elseif token.value == "new" and tokens[index + 1] and
            tokens[index + 1].type == "identifier" and tokens[index + 2] and
            tokens[index + 2].value == "(" then
            open_index = index + 2
            name_index = index + 1
            name_token = tokens[index + 1]
        end
        if open_index then
            local symbol = resolve_call_symbol(model, open_index, name_index,
                name_token)
            local parameters = symbol and symbol.params or nil
            if parameters and #parameters > 0 then
                local arguments = call_arguments(model, open_index)
                for arg_index, argument in ipairs(arguments) do
                    local parameter = parameters[arg_index]
                    local argument_index = model.token_index_by_start[
                        argument.start]
                    local named = argument_index and model.tokens[
                        argument_index + 1] and model.tokens[
                        argument_index + 1].value == ":"
                    if parameter and not named then
                        local position = model.source:position(argument.start)
                        if in_range(position, requested) then
                            hints[#hints + 1] = {
                                position = position,
                                label = parameter.name .. ":",
                                kind = 2,
                                paddingRight = true,
                                data = {
                                    kind = "parameter",
                                    name = parameter.name,
                                },
                            }
                        end
                    end
                end
            end
        end
    end
    return hints
end

function F.inlay_hint_resolve(hint, ctx)
    return hint
end

function F.folding_ranges(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local result = {}
    local seen = {}
    local function add(start_line, end_line, kind)
        if end_line <= start_line then
            return
        end
        local key = start_line .. "|" .. end_line .. "|" .. tostring(kind)
        if seen[key] then
            return
        end
        seen[key] = true
        result[#result + 1] = {
            startLine = start_line,
            endLine = end_line,
            kind = kind,
        }
    end
    for _, node in ipairs(model.node_list) do
        if analysis.type_kinds[node.kind] or node.kind == "namespace" or
            analysis.member_kinds[node.kind] then
            add(model.source:line_of(node.start),
                model.source:line_of(node["end"]), nil)
        end
    end
    for _, comment in ipairs(model.comments) do
        if comment.comment_kind == "block" then
            add(model.source:line_of(comment.start),
                model.source:line_of(comment["end"]), "comment")
        end
    end
    local regions = {}
    for _, directive in ipairs(model.directives) do
        if directive.directive == "region" then
            regions[#regions + 1] = directive
        elseif directive.directive == "endregion" and #regions > 0 then
            local start_directive = table.remove(regions)
            add(model.source:line_of(start_directive.start),
                model.source:line_of(directive["end"]), "region")
        end
    end
    return result
end

function F.code_lens(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local result = {}
    for _, symbol in ipairs(model.symbols) do
        if analysis.type_kinds[symbol.kind] or symbol.kind == "method" or
            symbol.kind == "property" or symbol.kind == "constructor" then
            result[#result + 1] = {
                range = symbol.selectionRange,
                data = {
                    uri = symbol.uri,
                    name = symbol.name,
                    start = symbol.nameStart,
                    kind = symbol.kind,
                },
            }
        end
    end
    return result
end

function F.code_lens_resolve(lens, ctx)
    local data = lens.data
    if not data then
        return lens
    end
    local model = analysis.get_model(data.uri)
    if not model then
        return lens
    end
    local identifier = analysis.identifier_at(model, data.start)
    local symbol = identifier and analysis.resolve_identifier(model, identifier,
        data.start) or nil
    if not symbol then
        return lens
    end
    local references = analysis.references(model, symbol, false)
    lens.command = {
        title = tostring(#references) .. " references",
        command = "editor.action.showReferences",
        arguments = {
            data.uri,
            model.source:position(data.start),
            references,
        },
    }
    return lens
end

function F.document_links(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local result = {}
    for _, comment in ipairs(model.comments) do
        local text = comment.value
        local search_start = 1
        while true do
            local first, last = text:find("https?://[^%s\"'<>]+", search_start)
            if not first then
                break
            end
            local target = text:sub(first, last):gsub("[%.,;:!%)%]]+$", "")
            local start_offset = comment.start + first - 1
            result[#result + 1] = {
                range = model.source:range(start_offset,
                    start_offset + #target),
                target = target,
                tooltip = target,
            }
            search_start = last + 1
        end
        local file_start = 1
        while true do
            local first, last, path = text:find("<include%s+file=\"([^\"]+)\"",
                file_start)
            if not first then
                break
            end
            local document_path = hexal.lsp.uri.to_path(model.uri)
            if document_path then
                local directory = hexal.fs.dirname(document_path)
                local target_path = hexal.fs.join(directory, path)
                result[#result + 1] = {
                    range = model.source:range(comment.start + first - 1,
                        comment.start + last),
                    target = hexal.lsp.uri.from_path(target_path),
                    tooltip = path,
                }
            end
            file_start = last + 1
        end
    end
    return result
end

function F.document_link_resolve(link, ctx)
    return link
end

function F.selection_range(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local positions = params.positions or {}
    local result = {}
    for _, position in ipairs(positions) do
        local offset = offset_at(model, position)
        local nodes = analysis.nodes_containing(model, offset)
        local chain = nil
        local token = analysis.token_at(model, offset)
        if token then
            chain = {
                range = model.source:range(token.start, token["end"]),
            }
        end
        for _, node in ipairs(nodes) do
            local node_range = model.source:range(node.start, node["end"])
            if chain == nil then
                chain = { range = node_range }
            elseif util.range_contains_range(node_range, chain.range) then
                chain = { range = node_range, parent = chain }
            end
        end
        if chain == nil then
            local line = model.source:line_of(offset)
            chain = {
                range = model.source:range(model.source:line_start(line),
                    model.source:line_end(line)),
            }
        end
        result[#result + 1] = chain
    end
    return result
end

function F.document_highlight(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local offset = offset_at(model, params.position)
    local identifier = analysis.identifier_at(model, offset)
    if not identifier then
        return {}
    end
    local symbol = analysis.resolve_identifier(model, identifier, offset)
    local result = {}
    local scope = symbol and symbol.scopeNode or nil
    for _, entry in ipairs(model.identifiers) do
        if entry.value == identifier.value and (not scope or
            (scope.start <= entry.start and entry.start <= scope["end"])) then
            local resolved = analysis.resolve_identifier(model, entry,
                entry.start)
            if resolved == symbol or entry == identifier or
                entry.declaration == symbol then
                local kind = 2
                if entry.declaration then
                    kind = 3
                elseif entry.nextValue == "=" or entry.nextValue == "++" or
                    entry.nextValue == "--" then
                    kind = 3
                end
                result[#result + 1] = {
                    range = model.source:range(entry.start, entry["end"]),
                    kind = kind,
                }
            end
        end
    end
    return result
end

function F.linked_editing(params, ctx)
    local model = get_model(params)
    if not model then
        return nil
    end
    local offset = offset_at(model, params.position)
    local identifier = analysis.identifier_at(model, offset)
    if not identifier then
        return nil
    end
    local symbol = analysis.resolve_identifier(model, identifier, offset)
    local ranges = {}
    local scope = symbol and symbol.scopeNode or nil
    for _, entry in ipairs(model.identifiers) do
        if entry.value == identifier.value and (not scope or
            (scope.start <= entry.start and entry.start <= scope["end"])) then
            local resolved = analysis.resolve_identifier(model, entry,
                entry.start)
            if resolved == symbol or entry == identifier then
                ranges[#ranges + 1] = model.source:range(entry.start,
                    entry["end"])
            end
        end
    end
    if #ranges < 2 then
        return nil
    end
    return {
        ranges = ranges,
        wordPattern = identifier.value,
    }
end

local function call_hierarchy_item(symbol)
    if not symbol or not symbol.uri then
        return nil
    end
    return {
        name = symbol.name,
        kind = symbol_kind(symbol),
        detail = symbol.container,
        uri = symbol.uri,
        range = symbol.range,
        selectionRange = symbol.selectionRange,
        data = {
            uri = symbol.uri,
            start = symbol.nameStart,
        },
    }
end

function F.prepare_call_hierarchy(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol or not (analysis.member_kinds[symbol.kind] or
        analysis.type_kinds[symbol.kind]) then
        return {}
    end
    local item = call_hierarchy_item(symbol)
    return item and { item } or {}
end

local function model_for_uri(uri)
    for _, model in ipairs(analysis.all_models()) do
        if model.uri == uri then
            return model
        end
    end
    return analysis.get_model(uri)
end

local function item_symbol(item)
    if not item or not item.data then
        return nil
    end
    local model = model_for_uri(item.data.uri)
    if not model then
        return nil
    end
    local identifier = analysis.identifier_at(model, item.data.start)
    if not identifier then
        return nil
    end
    return analysis.resolve_identifier(model, identifier, item.data.start),
        model
end

function F.incoming_calls(params, ctx)
    local symbol, model = item_symbol(params.item)
    if not symbol or not model then
        return {}
    end
    local grouped = {}
    local order = {}
    for _, other in ipairs(analysis.all_models()) do
        for _, entry in ipairs(other.identifiers) do
            if entry.value == symbol.name then
                local resolved = analysis.resolve_identifier(other, entry,
                    entry.start)
                if resolved == symbol then
                    local enclosing = analysis.enclosing_member(other,
                        entry.start)
                    if enclosing and enclosing.symbol and
                        (enclosing.kind == "method" or
                            enclosing.kind == "constructor") then
                        local caller = enclosing.symbol
                        local key = caller.uri .. "|" .. caller.nameStart
                        if not grouped[key] then
                            local item = call_hierarchy_item(caller)
                            if item then
                                grouped[key] = {
                                    from = item,
                                    fromRanges = {},
                                }
                                order[#order + 1] = key
                            end
                        end
                        if grouped[key] then
                            grouped[key].fromRanges[#grouped[key].fromRanges +
                                1] = other.source:range(entry.start,
                                    entry["end"])
                        end
                    end
                end
            end
        end
    end
    local result = {}
    for _, key in ipairs(order) do
        result[#result + 1] = grouped[key]
    end
    return result
end

function F.outgoing_calls(params, ctx)
    local symbol, model = item_symbol(params.item)
    if not symbol or not model then
        return {}
    end
    local node = symbol.node
    if not node or not node.bodyStart or not node.bodyEnd then
        return {}
    end
    local grouped = {}
    local order = {}
    for index, token in ipairs(model.tokens) do
        if token.start >= node.bodyStart and token["end"] <= node.bodyEnd and
            token.type == "identifier" and model.tokens[index + 1] and
            model.tokens[index + 1].value == "(" then
            local identifier = {
                value = token.value:gsub("^@", ""),
                start = token.start,
                ["end"] = token["end"],
                index = index,
            }
            local callee = analysis.resolve_identifier(model, identifier,
                token.start)
            if callee and (callee.kind == "method" or
                callee.kind == "constructor") and callee ~= symbol then
                local item = call_hierarchy_item(callee)
                if item then
                    local key = callee.uri .. "|" .. callee.nameStart
                    if not grouped[key] then
                        grouped[key] = {
                            to = item,
                            fromRanges = {},
                        }
                        order[#order + 1] = key
                    end
                    grouped[key].fromRanges[#grouped[key].fromRanges + 1] =
                        model.source:range(token.start, token["end"])
                end
            end
        end
    end
    local result = {}
    for _, key in ipairs(order) do
        result[#result + 1] = grouped[key]
    end
    return result
end

local function type_hierarchy_item(symbol)
    if not symbol or not symbol.uri then
        return nil
    end
    return {
        name = symbol.name,
        kind = symbol_kind(symbol),
        detail = symbol.container,
        uri = symbol.uri,
        range = symbol.range,
        selectionRange = symbol.selectionRange,
        data = {
            uri = symbol.uri,
            start = symbol.nameStart,
        },
    }
end

function F.prepare_type_hierarchy(params, ctx)
    local model = get_model(params)
    if not model then
        return {}
    end
    local offset = offset_at(model, params.position)
    local _, symbol = analysis.resolve_at(model, offset)
    if not symbol or not analysis.type_kinds[symbol.kind] then
        return {}
    end
    local item = type_hierarchy_item(symbol)
    return item and { item } or {}
end

function F.supertypes(params, ctx)
    local symbol, model = item_symbol(params.item)
    if not symbol or not model then
        return {}
    end
    local result = {}
    for _, base in ipairs(symbol.node.baseTypes or {}) do
        local base_symbol = analysis.find_type(model, base.name)
        if base_symbol then
            local item = type_hierarchy_item(base_symbol)
            if item then
                result[#result + 1] = item
            end
        end
    end
    return result
end

function F.subtypes(params, ctx)
    local symbol = item_symbol(params.item)
    if not symbol then
        return {}
    end
    local result = {}
    for _, model in ipairs(analysis.all_models()) do
        for _, candidate in ipairs(model.types) do
            for _, base in ipairs(candidate.node.baseTypes or {}) do
                local simple = base.name:gsub("<.*$", "")
                if simple == symbol.name then
                    local item = type_hierarchy_item(candidate)
                    if item then
                        result[#result + 1] = item
                    end
                    break
                end
            end
        end
    end
    return result
end

return F
