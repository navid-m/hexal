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

local function open_picker()
    local lines = {}
    for index, color in ipairs(PALETTE) do
        lines[index] = {
            segments = {
                { text = "██ ", style = color.hex },
                { text = color.hex, style = "accent" },
                { text = "  " .. color.name, style = "text" },
            },
            selectable = true,
        }
    end

    local panel
    panel = hexal.ui.panel({
        title = " Pick Color ",
        footer = "j/k move · Enter insert · q close",
        selectable = true,
        selected = 1,
        lines = lines,
    }, function(key, selected)
        if key == "enter" then
            local color = PALETTE[selected]
            if color then
                panel.close()
                hexal.editor.insert_text(color.hex)
                hexal.ui.message("Inserted " .. color.hex)
            end
            return true
        elseif key == "q" or key == "Q" then
            panel.close()
            return true
        end
        return false
    end)
end

hexal.commands.register("pc", open_picker,
    "Pick a color from a palette and insert its hex code")
