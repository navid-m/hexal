local function text_width(text)
    if type(utf8) == "table" and type(utf8.len) == "function" then
        local ok, length = pcall(utf8.len, text)
        if ok and length then
            return length
        end
    end
    return #text
end

local function split_path(path)
    local parts = {}
    for part in path:gmatch("[^/\\]+") do
        if part ~= "." and part ~= ".." then
            parts[#parts + 1] = part
        end
    end
    return parts
end

local function node_before(left, right)
    if left.is_dir ~= right.is_dir then
        return left.is_dir
    end

    local left_name = left.name:lower()
    local right_name = right.name:lower()
    if left_name == right_name then
        return left.name < right.name
    end
    return left_name < right_name
end

local function build_rows(root_node)
    local rows = {
        {
            indent = "",
            label = "◆ " .. root_node.name .. "/",
            style = "accent",
            root = true,
        },
    }

    local function walk(node, prefix)
        table.sort(node.children, node_before)
        for index, child in ipairs(node.children) do
            local last = index == #node.children
            local branch = last and "└── " or "├── "
            rows[#rows + 1] = {
                indent = prefix .. branch,
                label = child.name .. (child.is_dir and "/" or ""),
                style = child.is_dir and "accent" or "default",
                root = false,
            }

            if child.is_dir then
                walk(child, prefix .. (last and "    " or "│   "))
            end
        end
    end

    walk(root_node, "")
    return rows
end

local function open_project_tree()
    local root = hexal.editor.get_workspace_root()
    if not root or root == "" then
        hexal.ui.warn("projtree: no workspace root is available")
        return
    end

    local root_name = hexal.fs.basename(root)
    if root_name == "" then
        root_name = root
    end

    local files = hexal.fs.workspace_files({ max_results = 50000 })
    local root_node = {
        name = root_name,
        children = {},
        child_map = {},
        is_dir = true,
    }

    for _, path in ipairs(files) do
        local parts = split_path(path)
        local node = root_node
        for index, part in ipairs(parts) do
            local child = node.child_map[part]
            if not child then
                child = {
                    name = part,
                    children = {},
                    child_map = {},
                    is_dir = index < #parts,
                }
                node.child_map[part] = child
                node.children[#node.children + 1] = child
            elseif index < #parts then
                child.is_dir = true
            end
            node = child
        end
    end

    local rows = build_rows(root_node)
    local maximum_width = 0
    for _, row in ipairs(rows) do
        local width = text_width(row.indent) + text_width(row.label)
        if width > maximum_width then
            maximum_width = width
        end
    end

    local size = hexal.ui.terminal_size()
    local columns = size and size.columns or 80
    local panel_width = math.min(math.max(20, columns - 2), 160)
    local content_width = math.max(20, panel_width - 4)
    local left_pad = math.max(0, math.floor((content_width - maximum_width) / 2))
    local root_width = text_width(rows[1].label)
    local root_extra = math.max(0, math.floor((maximum_width - root_width) / 2))

    local lines = {}
    for _, row in ipairs(rows) do
        local pad = left_pad + (row.root and root_extra or 0)
        local segments = {}
        if pad > 0 then
            segments[#segments + 1] = { text = string.rep(" ", pad), style = "default" }
        end
        if row.indent ~= "" then
            segments[#segments + 1] = { text = row.indent, style = "muted" }
        end
        segments[#segments + 1] = { text = row.label, style = row.style }
        lines[#lines + 1] = { segments = segments, selectable = false }
    end

    local file_word = #files == 1 and "file" or "files"
    hexal.ui.panel({
        title = " Project Tree · " .. root_name .. " ",
        footer = string.format(
            "%d %s · non-ignored · ↑↓/j/k scroll · g/G top/bottom · q close",
            #files,
            file_word
        ),
        selectable = false,
        selected = 1,
        lines = lines,
    }, nil)
end

hexal.commands.register("pt", open_project_tree, "Show a Unicode tree of workspace files")
