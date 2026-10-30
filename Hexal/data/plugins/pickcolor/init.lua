local PALETTE = {
    { name = "Red", hex = "#ef4444" },
    { name = "Orange", hex = "#f97316" },
    { name = "Amber", hex = "#f59e0b" },
    { name = "Yellow", hex = "#eab308" },
    { name = "Lime", hex = "#84cc16" },
    { name = "Green", hex = "#22c55e" },
    { name = "Emerald", hex = "#10b981" },
    { name = "Teal", hex = "#14b8a6" },
    { name = "Cyan", hex = "#06b6d4" },
    { name = "Sky", hex = "#0ea5e9" },
    { name = "Blue", hex = "#3b82f6" },
    { name = "Indigo", hex = "#6366f1" },
    { name = "Violet", hex = "#8b5cf6" },
    { name = "Purple", hex = "#a855f7" },
    { name = "Fuchsia", hex = "#d946ef" },
    { name = "Pink", hex = "#ec4899" },
    { name = "Rose", hex = "#f43f5e" },
    { name = "Brown", hex = "#92400e" },
    { name = "White", hex = "#ffffff" },
    { name = "Light Gray", hex = "#d1d5db" },
    { name = "Gray", hex = "#9ca3af" },
    { name = "Dark Gray", hex = "#4b5563" },
    { name = "Charcoal", hex = "#1f2937" },
    { name = "Black", hex = "#000000" },
}

local function hex_to_rgb(hex)
    return tonumber(hex:sub(2, 3), 16) or 0,
        tonumber(hex:sub(4, 5), 16) or 0,
        tonumber(hex:sub(6, 7), 16) or 0
end

local function rgb_to_hex(red, green, blue)
    return string.format("#%02x%02x%02x", red, green, blue)
end

local function open_picker()
    local palette_lines = {}
    for index, color in ipairs(PALETTE) do
        palette_lines[index] = {
            segments = {
                { text = "██ ", style = color.hex },
                { text = color.hex, style = "accent" },
                { text = "  " .. color.name, style = "text" },
            },
            selectable = true,
        }
    end

    local state = {
        editing = false,
        field = 1,
        selected = 1,
        fresh = true,
        channels = { "255", "255", "255" },
    }
    local panel
    local refresh

    local function channel_value(index)
        return math.min(255, tonumber(state.channels[index]) or 0)
    end

    local function current_rgb()
        return channel_value(1), channel_value(2), channel_value(3)
    end

    local function selected_color(index)
        return PALETTE[index or state.selected] or PALETTE[1]
    end

    local function selected_rgb(index)
        return hex_to_rgb(selected_color(index).hex)
    end

    local function commit_channel()
        state.channels[state.field] = tostring(channel_value(state.field))
    end

    local function edit_digit(digit)
        if state.fresh then
            state.channels[state.field] = digit
            state.fresh = false
            return
        end

        local value = state.channels[state.field]
        if value == "0" or #value >= 3 then
            value = digit
        else
            value = value .. digit
        end
        if tonumber(value) > 255 then
            value = digit
        end
        state.channels[state.field] = value
    end

    local function backspace()
        local value = state.channels[state.field]
        if state.fresh then
            value = "0"
            state.fresh = false
        else
            value = value:sub(1, -2)
            if value == "" then
                value = "0"
            end
        end
        state.channels[state.field] = value
    end

    local function start_editing(index)
        local red, green, blue = selected_rgb(index)
        state.channels = { tostring(red), tostring(green), tostring(blue) }
        state.field = 1
        state.fresh = true
        state.editing = true
        state.selected = index or state.selected
    end

    local function rgb_lines()
        local red, green, blue = current_rgb()
        local hex = rgb_to_hex(red, green, blue)

        local function field(index, label)
            local value = state.channels[index]
            if value == "" then
                value = "0"
            end
            if state.field == index then
                return { text = label .. " [" .. value .. "]", style = "accent" }
            end
            return { text = label .. "  " .. value .. " ", style = "text" }
        end

        return {
            {
                segments = {
                    { text = "  RGB  ", style = "muted" },
                    field(1, "r"),
                    { text = " ", style = "text" },
                    field(2, "g"),
                    { text = " ", style = "text" },
                    field(3, "b"),
                },
                selectable = false,
            },
            {
                segments = {
                    { text = "  Preview  ", style = "muted" },
                    { text = "████████████", style = hex },
                    { text = "  " .. hex, style = "accent" },
                    { text = string.format("  rgb(%d, %d, %d)", red, green, blue), style = "text" },
                },
                selectable = false,
            },
        }
    end

    refresh = function()
        if state.editing then
            panel:set_title(" Pick Color · RGB ")
            panel:set_footer(
                "digits · Tab field · Bksp · Enter insert · r copy rgb · h copy hex · Esc back"
            )
            panel:set_selectable(false)
            panel:set_lines(rgb_lines())
            return
        end

        panel:set_title(" Pick Color ")
        panel:set_footer(
            "j/k move · Enter insert hex · i/type digits RGB · r copy rgb · h copy hex · q close"
        )
        panel:set_selectable(true)
        panel:set_lines(palette_lines)
        panel:set_selected(state.selected)
    end

    local function copy(text, label)
        if hexal.ui.copy(text) then
            hexal.ui.message("Copied " .. label .. ": " .. text)
        else
            hexal.ui.warn("pickcolor: could not copy " .. label)
        end
    end

    local function copy_rgb()
        local red, green, blue
        if state.editing then
            red, green, blue = current_rgb()
        else
            red, green, blue = selected_rgb(state.selected)
        end
        copy(string.format("rgb(%d, %d, %d)", red, green, blue), "RGB")
    end

    local function copy_hex()
        local hex
        if state.editing then
            local red, green, blue = current_rgb()
            hex = rgb_to_hex(red, green, blue)
        else
            hex = selected_color(state.selected).hex
        end
        copy(hex, "hex")
    end

    local function insert_current()
        local red, green, blue = current_rgb()
        local hex = rgb_to_hex(red, green, blue)
        panel.close()
        hexal.editor.insert_text(hex)
        hexal.ui.message("Inserted " .. hex)
    end

    local function insert_palette(index)
        local color = selected_color(index)
        panel.close()
        hexal.editor.insert_text(color.hex)
        hexal.ui.message("Inserted " .. color.hex)
    end

    panel = hexal.ui.panel({
        title = " Pick Color ",
        footer = "j/k move · Enter insert hex · i/type digits RGB · r copy rgb · h copy hex · q close",
        selectable = true,
        selected = 1,
        lines = palette_lines,
    }, function(key, selected)
        state.selected = selected or state.selected

        if state.editing then
            if key:match("^%d$") then
                edit_digit(key)
                refresh()
            elseif key == "tab" or key == "," or key == " " then
                commit_channel()
                state.field = state.field % 3 + 1
                state.fresh = true
                refresh()
            elseif key == "backspace" then
                backspace()
                refresh()
            elseif key == "enter" then
                insert_current()
            elseif key == "r" or key == "R" then
                copy_rgb()
            elseif key == "h" or key == "H" then
                copy_hex()
            elseif key == "esc" then
                state.editing = false
                refresh()
            elseif key == "q" or key == "Q" then
                panel.close()
            end
            return true
        end

        if key:match("^%d$") then
            start_editing(state.selected)
            edit_digit(key)
            refresh()
        elseif key == "i" or key == "I" then
            start_editing(state.selected)
            refresh()
        elseif key == "enter" then
            insert_palette(state.selected)
        elseif key == "r" or key == "R" then
            copy_rgb()
        elseif key == "h" or key == "H" then
            copy_hex()
        elseif key == "q" or key == "Q" then
            panel.close()
        elseif key == "backspace" then
            return true
        else
            return false
        end
        return true
    end)
end

hexal.commands.register("pc", open_picker,
    "Pick or enter a color and insert or copy its hex/RGB value")
