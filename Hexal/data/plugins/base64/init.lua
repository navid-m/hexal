-- Base64 and Base32 encoding/decoding for text.
--
-- With a selection, the selected text is replaced in place. Without one, the
-- current line is transformed. Decoding ignores whitespace and accepts
-- URL-safe Base64 input.

local editor = hexal.editor

local BASE64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"

local BASE32_VALUES = {}
for index = 1, #BASE32_ALPHABET do
    BASE32_VALUES[BASE32_ALPHABET:sub(index, index)] = index - 1
end

local BASE64_VALUES = {}
for index = 1, #BASE64_ALPHABET do
    BASE64_VALUES[BASE64_ALPHABET:sub(index, index)] = index - 1
end

local function scalar_length(text)
    if type(utf8) == "table" and type(utf8.len) == "function" then
        local ok, length = pcall(utf8.len, text)
        if ok and length ~= nil then
            return length
        end
    end
    return #text
end

local function is_text(text)
    if type(utf8) ~= "table" or type(utf8.len) ~= "function" then
        return true
    end
    local ok, length = pcall(utf8.len, text)
    return ok and length ~= nil
end

local function encode_base64(text)
    local output = {}
    local buffer, bits = 0, 0

    for index = 1, #text do
        buffer = buffer * 256 + text:byte(index)
        bits = bits + 8
        while bits >= 6 do
            bits = bits - 6
            local value = math.floor(buffer / 2 ^ bits) % 64
            output[#output + 1] = BASE64_ALPHABET:sub(value + 1, value + 1)
        end
        buffer = buffer % 2 ^ bits
    end

    if bits > 0 then
        local value = math.floor(buffer * 2 ^ (6 - bits)) % 64
        output[#output + 1] = BASE64_ALPHABET:sub(value + 1, value + 1)
    end
    while #output % 4 ~= 0 do
        output[#output + 1] = "="
    end
    return table.concat(output)
end

local function decode_base64(text)
    local cleaned = text:gsub("%s+", ""):gsub("-", "+"):gsub("_", "/")
    local body = cleaned:gsub("=+$", "")
    if body:find("[^A-Za-z0-9+/]") then
        return nil, "invalid Base64 character"
    end
    local remainder = #body % 4
    if remainder == 1 then
        return nil, "invalid Base64 length"
    end

    local output = {}
    local buffer, bits = 0, 0
    for index = 1, #body do
        local value = BASE64_VALUES[body:sub(index, index)]
        buffer = buffer * 64 + value
        bits = bits + 6
        while bits >= 8 do
            bits = bits - 8
            output[#output + 1] = string.char(math.floor(buffer / 2 ^ bits) % 256)
        end
        buffer = buffer % 2 ^ bits
    end

    if buffer ~= 0 then
        return nil, "invalid Base64 padding"
    end
    local decoded = table.concat(output)
    if not is_text(decoded) then
        return nil, "decoded data is not valid UTF-8 text"
    end
    return decoded
end

local function encode_base32(text)
    local output = {}
    local buffer, bits = 0, 0

    for index = 1, #text do
        buffer = buffer * 256 + text:byte(index)
        bits = bits + 8
        while bits >= 5 do
            bits = bits - 5
            local value = math.floor(buffer / 2 ^ bits) % 32
            output[#output + 1] = BASE32_ALPHABET:sub(value + 1, value + 1)
        end
        buffer = buffer % 2 ^ bits
    end

    if bits > 0 then
        local value = math.floor(buffer * 2 ^ (5 - bits)) % 32
        output[#output + 1] = BASE32_ALPHABET:sub(value + 1, value + 1)
    end
    while #output % 8 ~= 0 do
        output[#output + 1] = "="
    end
    return table.concat(output)
end

local function decode_base32(text)
    local body = text:gsub("%s+", ""):gsub("=+$", "")
    local remainder = #body % 8
    if remainder == 1 or remainder == 3 or remainder == 6 then
        return nil, "invalid Base32 length"
    end

    local output = {}
    local buffer, bits = 0, 0
    for index = 1, #body do
        local character = body:sub(index, index):upper()
        local value = BASE32_VALUES[character]
        if value == nil then
            return nil, "invalid Base32 character"
        end
        buffer = buffer * 32 + value
        bits = bits + 5
        while bits >= 8 do
            bits = bits - 8
            output[#output + 1] = string.char(math.floor(buffer / 2 ^ bits) % 256)
        end
        buffer = buffer % 2 ^ bits
    end

    if buffer ~= 0 then
        return nil, "invalid Base32 padding"
    end
    local decoded = table.concat(output)
    if not is_text(decoded) then
        return nil, "decoded data is not valid UTF-8 text"
    end
    return decoded
end

local function transform(run, label)
    local selection = editor.get_selection()
    local start_position, end_position, text, target

    if selection ~= nil then
        start_position = { line = selection.start_line, column = selection.start_column }
        end_position = { line = selection.end_line, column = selection.end_column }
        text = selection.text
        target = "selection"
    else
        local cursor = editor.get_cursor()
        if cursor == nil then
            hexal.ui.warn("base64: no cursor position")
            return
        end
        local line = editor.get_line(cursor.line)
        if line == nil then
            hexal.ui.warn("base64: current line is not available")
            return
        end
        start_position = { line = cursor.line, column = 0 }
        end_position = { line = cursor.line, column = scalar_length(line) }
        text = line
        target = string.format("line %d", cursor.line + 1)
    end

    if text == "" then
        hexal.ui.warn("base64: nothing to transform")
        return
    end

    local ok, output, error_message = pcall(run, text)
    if not ok then
        hexal.ui.warn("base64: " .. tostring(output))
        return
    end
    if output == nil then
        hexal.ui.warn("base64: " .. (error_message or "invalid input"))
        return
    end
    if not editor.replace_range(start_position, end_position, output) then
        hexal.ui.warn("base64: cannot replace " .. target)
        return
    end

    hexal.ui.message(string.format("base64: %s %s", label, target))
end

local function command(run, label)
    return function()
        transform(run, label)
    end
end

hexal.commands.register(
    "b64e",
    command(encode_base64, "encoded"),
    "Base64-encode the selection or current line"
)
hexal.commands.register(
    "b64d",
    command(decode_base64, "decoded"),
    "Base64-decode the selection or current line"
)
hexal.commands.register(
    "b32e",
    command(encode_base32, "encoded"),
    "Base32-encode the selection or current line"
)
hexal.commands.register(
    "b32d",
    command(decode_base32, "decoded"),
    "Base32-decode the selection or current line"
)
