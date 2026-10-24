local PALETTE = {
    "accent",
    "success",
    "warning",
    "keyword",
    "type",
    "constant",
    "diff_modified",
    "diff_added",
}

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

local function sort_tree(node)
    table.sort(node.children, node_before)
    for _, child in ipairs(node.children) do
        if child.is_dir then
            sort_tree(child)
        end
    end
end

local function assign_colors(node, color_index)
    node.color = PALETTE[color_index]
    local next_index = color_index
    for _, child in ipairs(node.children) do
        if child.is_dir then
            next_index = (next_index % #PALETTE) + 1
            assign_colors(child, next_index)
        end
    end
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
        path = "",
        children = {},
        child_map = {},
        is_dir = true,
    }

    for _, path in ipairs(files) do
        local parts = split_path(path)
        local node = root_node
        local node_path = ""
        for index, part in ipairs(parts) do
            if node_path == "" then
                node_path = part
            else
                node_path = node_path .. "/" .. part
            end

            local child = node.child_map[part]
            if not child then
                child = {
                    name = part,
                    path = node_path,
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

    sort_tree(root_node)
    assign_colors(root_node, 1)

    local root_base = root:gsub("\\", "/")
    if root_base:sub(-1) ~= "/" then
        root_base = root_base .. "/"
    end

    local state = {
        query = "",
        collapsed = {},
        panel = nil,
        row_nodes = {},
        match_count = 0,
    }

    local function absolute_path(path)
        local relative = path:gsub("\\", "/")
        return root_base .. relative
    end

    local function mark_matches(node, query)
        if node.is_dir then
            local any = false
            for _, child in ipairs(node.children) do
                if mark_matches(child, query) then
                    any = true
                end
            end
            node.matches = any
            return any
        end

        local matched = node.path:lower():find(query, 1, true) ~= nil
        node.matches = matched
        if matched then
            state.match_count = state.match_count + 1
        end
        return matched
    end

    local function build_view()
        local searching = state.query ~= ""
        state.match_count = 0
        if searching then
            mark_matches(root_node, state.query)
        end

        local rows = {
            {
                node = root_node,
                indent_segments = {},
                label = "◆ " .. root_name .. "/",
                style = root_node.color,
                selectable = not searching,
                root = true,
            },
        }

        local function walk(node, continuation, ancestor_colors)
            local visible = {}
            for _, child in ipairs(node.children) do
                if not searching or child.matches then
                    visible[#visible + 1] = child
                end
            end

            for index, child in ipairs(visible) do
                local last = index == #visible
                local indent_segments = {}
                for depth, has_more in ipairs(continuation) do
                    indent_segments[#indent_segments + 1] = {
                        text = has_more and "│   " or "    ",
                        style = ancestor_colors[depth],
                    }
                end
                indent_segments[#indent_segments + 1] = {
                    text = last and "└── " or "├── ",
                    style = node.color,
                }

                local collapsed = state.collapsed[child.path] == true
                local label
                if child.is_dir then
                    local marker = not searching and collapsed and "▸ " or "▾ "
                    label = marker .. child.name .. "/"
                else
                    label = child.name
                end

                rows[#rows + 1] = {
                    node = child,
                    indent_segments = indent_segments,
                    label = label,
                    style = child.is_dir and child.color or "default",
                    selectable = searching and not child.is_dir or not searching,
                    root = false,
                }

                if child.is_dir then
                    local expand = searching or not collapsed
                    if expand and #child.children > 0 then
                        local next_continuation = {}
                        local next_colors = {}
                        for depth, has_more in ipairs(continuation) do
                            next_continuation[depth] = has_more
                            next_colors[depth] = ancestor_colors[depth]
                        end
                        next_continuation[#next_continuation + 1] = not last
                        next_colors[#next_colors + 1] = node.color
                        walk(child, next_continuation, next_colors)
                    end
                end
            end
        end

        if searching or state.collapsed[""] ~= true then
            walk(root_node, {}, {})
        end

        if searching and state.match_count == 0 then
            rows[#rows + 1] = {
                node = nil,
                indent_segments = {},
                label = "(no matches)",
                style = "muted",
                selectable = false,
                root = false,
            }
        end

        local maximum_width = 0
        local root_width = text_width(rows[1].label)
        for _, row in ipairs(rows) do
            local width = text_width(row.label)
            for _, segment in ipairs(row.indent_segments) do
                width = width + text_width(segment.text)
            end
            if width > maximum_width then
                maximum_width = width
            end
        end

        local size = hexal.ui.terminal_size()
        local columns = size and size.columns or 80
        local panel_width = math.min(math.max(20, columns - 2), 160)
        local content_width = math.max(20, panel_width - 4)
        local left_pad = math.max(0, math.floor((content_width - maximum_width) / 2))
        local root_extra = math.max(0, math.floor((maximum_width - root_width) / 2))

        local lines = {}
        local row_nodes = {}
        for index, row in ipairs(rows) do
            local pad = left_pad + (row.root and root_extra or 0)
            local segments = {}
            if pad > 0 then
                segments[#segments + 1] = { text = string.rep(" ", pad), style = "default" }
            end
            for _, segment in ipairs(row.indent_segments) do
                segments[#segments + 1] = segment
            end
            segments[#segments + 1] = { text = row.label, style = row.style }
            lines[#lines + 1] = { segments = segments, selectable = row.selectable }
            row_nodes[index] = row.node
        end

        local title = " Project Tree · " .. root_name
        local footer
        if searching then
            title = title .. " · /" .. state.query
            footer = string.format(
                "%d matching %s · ↑↓/j/k select · Enter open · / search · q close",
                state.match_count,
                state.match_count == 1 and "file" or "files"
            )
        else
            footer = string.format(
                "%d %s · non-ignored · ↑↓/j/k select · Enter open/toggle · / search · q close",
                #files,
                #files == 1 and "file" or "files"
            )
        end
        title = title .. " "

        return {
            lines = lines,
            row_nodes = row_nodes,
            title = title,
            footer = footer,
        }
    end

    local function find_selected_index(row_nodes, key)
        for index, node in ipairs(row_nodes) do
            if node and node.path == key then
                return index, node
            end
        end
        return nil, nil
    end

    local function refresh(select_key)
        local view = build_view()
        state.row_nodes = view.row_nodes
        state.panel:set_title(view.title)
        state.panel:set_footer(view.footer)
        state.panel:set_selectable(true)
        state.panel:set_lines(view.lines)

        local selected = nil
        if select_key ~= nil then
            local index, node = find_selected_index(view.row_nodes, select_key)
            if index and node then
                selected = index
            end
        end

        if not selected then
            for index, node in ipairs(view.row_nodes) do
                if node and (state.query == "" or not node.is_dir) then
                    selected = index
                    break
                end
            end
        end

        state.panel:set_selected(selected or 1)
    end

    local function start_search(selected)
        local node = state.row_nodes[selected]
        local previous = node and node.path or nil
        hexal.ui.prompt("Search project tree: ", function(value)
            if value == nil then
                return
            end

            local query = value:gsub("^%s+", ""):gsub("%s+$", ""):lower()
            state.query = query
            refresh(previous)
        end)
    end

    local function on_key(key, selected)
        if key == "/" then
            start_search(selected)
            return true
        end

        if key ~= "enter" then
            return false
        end

        local node = state.row_nodes[selected]
        if not node then
            return true
        end

        if node.is_dir then
            if state.query ~= "" then
                return true
            end

            state.collapsed[node.path] = not state.collapsed[node.path]
            refresh(node.path)
            return true
        end

        state.panel:close()
        hexal.commands.execute("e", absolute_path(node.path))
        return true
    end

    local view = build_view()
    state.row_nodes = view.row_nodes
    local selected = nil
    for index, node in ipairs(view.row_nodes) do
        if node then
            selected = index
            break
        end
    end

    state.panel = hexal.ui.panel({
        title = view.title,
        footer = view.footer,
        selectable = true,
        selected = selected or 1,
        lines = view.lines,
    }, on_key)
end

hexal.commands.register("pt", open_project_tree, "Show a Unicode tree of workspace files")
