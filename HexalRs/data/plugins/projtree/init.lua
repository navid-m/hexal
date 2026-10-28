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

local EMPTY = {}

-- Box-drawing segments are constant for a given palette colour. Sharing them
-- keeps row construction allocation-free for the indentation part of a line.
local BRANCH_LAST, BRANCH_MORE = {}, {}
local CONT_LAST, CONT_MORE = {}, {}
for index, color in ipairs(PALETTE) do
    BRANCH_LAST[index] = { text = "└── ", style = color }
    BRANCH_MORE[index] = { text = "├── ", style = color }
    CONT_LAST[index] = { text = "    ", style = color }
    CONT_MORE[index] = { text = "│   ", style = color }
end

local pad_segments = {}

local function pad_segment(width)
    local segment = pad_segments[width]
    if not segment then
        segment = { text = string.rep(" ", width), style = "default" }
        pad_segments[width] = segment
    end
    return segment
end

local utf8_len = type(utf8) == "table" and utf8.len or nil

local function text_width(text)
    if utf8_len then
        local length = utf8_len(text)
        if length then
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

    local left_name = left.name_lower
    local right_name = right.name_lower
    if left_name == right_name then
        return left.name < right.name
    end
    return left_name < right_name
end

local load_generation = 0

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

    load_generation = load_generation + 1
    local generation = load_generation

    local root_node = {
        name = root_name,
        name_lower = root_name:lower(),
        name_width = text_width(root_name),
        path = "",
        depth = 0,
        children = {},
        child_map = {},
        is_dir = true,
    }

    local root_base = root:gsub("\\", "/")
    if root_base:sub(-1) ~= "/" then
        root_base = root_base .. "/"
    end

    local state = {
        generation = generation,
        query = "",
        searching = false,
        collapsed = {},
        panel = nil,
        rows = {},
        current_index = nil,
        selection_anchor = nil,
        match_count = 0,
        left_pad = 0,
        root_extra = 0,
        columns = 0,
        tree_max_width = 0,
        loading = true,
        cancelled = false,
        phase = "scan",
        files = nil,
        file_index = 1,
        nodes = { root_node },
        node_index = 1,
        sort_stack = {},
        color_stack = {},
        row_stack = nil,
        rows_total = 0,
        root_label = nil,
        root_segment = nil,
    }

    local function absolute_path(path)
        local relative = path:gsub("\\", "/")
        return root_base .. relative
    end

    local function current_columns()
        local size = hexal.ui.terminal_size()
        return size and size.columns or 80
    end

    local function layout(max_width)
        local columns = current_columns()
        local panel_width = math.min(math.max(20, columns - 2), 160)
        local content_width = math.max(20, panel_width - 4)
        local left_pad = math.max(0, math.floor((content_width - max_width) / 2))
        local root_extra = math.max(0, math.floor((max_width - root_node.label_width) / 2))
        return columns, left_pad, root_extra
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

        local matched = node.path_lower:find(query, 1, true) ~= nil
        node.matches = matched
        if matched then
            state.match_count = state.match_count + 1
        end
        return matched
    end

    local function make_child_row(node, child, continuation, continuation_text, last, searching)
        local branch = last and node.branch_last or node.branch_more
        local continuation_length = #continuation

        local indent
        if continuation_length == 0 then
            indent = { branch }
        else
            indent = {}
            for depth = 1, continuation_length do
                indent[depth] = continuation[depth]
            end
            indent[continuation_length + 1] = branch
        end

        local label, label_segment, selectable
        if searching then
            if child.is_dir then
                label = child.expand_label
                label_segment = child.expand_segment
                selectable = false
            else
                label = child.label
                label_segment = child.file_segment
                selectable = true
            end
        elseif child.is_dir then
            if state.collapsed[child.path] then
                label = child.closed_label
                label_segment = child.closed_segment
            else
                label = child.open_label
                label_segment = child.open_segment
            end
            selectable = true
        else
            label = child.label
            label_segment = child.file_segment
            selectable = true
        end

        local prefix_text = continuation_text .. branch.text
        local row = {
            node = child,
            indent = indent,
            label = label,
            label_segment = label_segment,
            style = child.style,
            selectable = selectable,
            depth = continuation_length + 1,
            width = (continuation_length + 1) * 4 + child.label_width,
            prefix_text = prefix_text,
            text = prefix_text .. label,
            root = false,
        }

        local next_cont, next_text
        if child.is_dir then
            if last then
                next_text = continuation_text .. node.cont_last.text
            else
                next_text = continuation_text .. node.cont_more.text
            end
            if continuation_length == 0 then
                next_cont = { last and node.cont_last or node.cont_more }
            else
                next_cont = {}
                for depth = 1, continuation_length do
                    next_cont[depth] = continuation[depth]
                end
                next_cont[continuation_length + 1] = last and node.cont_last or node.cont_more
            end
            child.child_cont = next_cont
            child.child_cont_text = next_text
        end

        return row, next_cont, next_text
    end

    -- Appends the visible children of `node` to `rows`. `continuation` holds
    -- the shared vertical-line segments above those children, and
    -- `continuation_text` is their plain-text form for the yank buffer.
    local function append_children(rows, node, continuation, continuation_text)
        local children = node.children
        local visible = children
        local count = #children
        local searching = state.searching
        if searching then
            visible = {}
            for index = 1, count do
                local child = children[index]
                if child.matches then
                    visible[#visible + 1] = child
                end
            end
            count = #visible
        end

        for index = 1, count do
            local child = visible[index]
            local last = index == count
            local row, next_cont, next_text = make_child_row(
                node, child, continuation, continuation_text, last, searching
            )
            rows[#rows + 1] = row

            if child.is_dir then
                local expanded = searching or not state.collapsed[child.path]
                if expanded and #child.children > 0 then
                    append_children(rows, child, next_cont, next_text)
                end
            end
        end
    end

    local function make_root_row()
        return {
            node = root_node,
            indent = EMPTY,
            label = state.root_label,
            label_segment = state.root_segment,
            style = root_node.color,
            selectable = not state.searching,
            depth = 0,
            width = root_node.label_width,
            prefix_text = "",
            text = state.root_label,
            root = true,
        }
    end

    local function build_rows()
        local rows = { make_root_row() }

        if state.searching or not state.collapsed[""] then
            append_children(rows, root_node, root_node.child_cont, root_node.child_cont_text)
        end
        return rows
    end

    local function line_for(row, in_selection)
        local segments = {}
        local pad = state.left_pad + (row.root and state.root_extra or 0)
        if pad > 0 then
            segments[1] = pad_segment(pad)
        end

        local indent = row.indent
        if in_selection then
            for index = 1, #indent do
                segments[#segments + 1] = { text = indent[index].text, style = "warning" }
            end
            segments[#segments + 1] = { text = row.label, style = "warning" }
        else
            for index = 1, #indent do
                segments[#segments + 1] = indent[index]
            end
            segments[#segments + 1] = row.label_segment
        end

        return { segments = segments, selectable = row.selectable }
    end

    local function view_title_and_footer()
        local title = " Project Tree · " .. root_name
        local footer
        local file_count = state.files and #state.files or 0
        if state.searching then
            title = title .. " · /" .. state.query
            footer = string.format(
                "%d matching %s · ↑↓ move · Shift+↑↓ select · y copy · Enter open · / search · q close",
                state.match_count,
                state.match_count == 1 and "file" or "files"
            )
        else
            footer = string.format(
                "%d %s · ↑↓ move · Shift+↑↓ select · y copy · Enter open/toggle · / search · q close",
                file_count,
                file_count == 1 and "file" or "files"
            )
        end
        return title .. " ", footer
    end

    local function full_view()
        state.match_count = 0
        state.searching = state.query ~= ""
        if state.searching then
            mark_matches(root_node, state.query)
        end

        local rows = build_rows()
        if state.searching and state.match_count == 0 then
            rows[#rows + 1] = {
                node = nil,
                indent = EMPTY,
                label = "(no matches)",
                label_segment = { text = "(no matches)", style = "muted" },
                style = "muted",
                selectable = false,
                depth = 0,
                width = text_width("(no matches)"),
                prefix_text = "",
                text = "(no matches)",
                root = false,
            }
        end

        local max_width = state.tree_max_width
        if state.searching or #rows == 1 then
            max_width = 0
            for index = 1, #rows do
                local width = rows[index].width
                if width > max_width then
                    max_width = width
                end
            end
        end

        local columns, left_pad, root_extra = layout(max_width)
        state.left_pad = left_pad
        state.root_extra = root_extra
        state.columns = columns

        local lines = {}
        for index = 1, #rows do
            lines[index] = line_for(rows[index], false)
        end

        local title, footer = view_title_and_footer()
        return {
            lines = lines,
            rows = rows,
            title = title,
            footer = footer,
        }
    end

    local function find_selected_index(rows, key)
        for index, row in ipairs(rows) do
            if row.node and row.node.path == key then
                return index
            end
        end
        return nil
    end

    local function first_selectable(rows)
        for index, row in ipairs(rows) do
            if row.selectable then
                return index
            end
        end
        return nil
    end

    local function last_selectable()
        for index = #state.rows, 1, -1 do
            if state.rows[index].selectable then
                return index
            end
        end
        return nil
    end

    local function finalize_node(node)
        node.path_lower = node.path:lower()
        local name = node.name
        local width = node.name_width
        if node.is_dir then
            local open_label = "▾ " .. name .. "/"
            local closed_label = "▸ " .. name .. "/"
            node.open_label = open_label
            node.closed_label = closed_label
            node.expand_label = open_label
            node.label_width = width + 3
            node.open_segment = { text = open_label, style = node.color }
            node.closed_segment = { text = closed_label, style = node.color }
            node.expand_segment = node.open_segment
            node.style = node.color
            node.branch_last = BRANCH_LAST[node.color_index]
            node.branch_more = BRANCH_MORE[node.color_index]
            node.cont_last = CONT_LAST[node.color_index]
            node.cont_more = CONT_MORE[node.color_index]
        else
            node.label = name
            node.expand_label = name
            node.label_width = width
            node.file_segment = { text = name, style = "default" }
            node.style = "default"
        end

        local row_width = node.depth * 4 + node.label_width
        if row_width > state.tree_max_width then
            state.tree_max_width = row_width
        end
    end

    local function insert_file(path)
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
                    name_lower = part:lower(),
                    name_width = text_width(part),
                    path = node_path,
                    depth = index,
                    children = {},
                    child_map = {},
                    is_dir = index < #parts,
                }
                node.child_map[part] = child
                node.children[#node.children + 1] = child
                state.nodes[#state.nodes + 1] = child
            elseif index < #parts then
                child.is_dir = true
            end
            node = child
        end
    end

    local load_step

    -- The initial load is cooperative: each callback stays within a small CPU
    -- budget, paints progress, then yields so input and rendering stay
    -- responsive. `hexal.ui.defer` schedules the callback after a frame.
    local function start_load_step()
        if state.cancelled or not state.loading or state.generation ~= load_generation then
            return
        end

        local deadline = os.clock() + 0.006
        while state.loading do
            local phase = state.phase
            if phase == "scan" then
                state.panel:set_footer("Scanning workspace… · q close")
                state.phase = "wait_files"
                if hexal.fs.workspace_files_async then
                    hexal.fs.workspace_files_async({ max_results = 50000 }, function(files)
                        state.files = files or {}
                        state.file_index = 1
                        state.phase = "tree"
                        if not state.cancelled and state.generation == load_generation then
                            hexal.ui.defer(load_step, 0)
                        end
                    end)
                    return
                end
                state.files = hexal.fs.workspace_files({ max_results = 50000 }) or {}
                state.file_index = 1
                state.phase = "tree"
            elseif phase == "tree" then
                local files = state.files
                local total = #files
                while state.file_index <= total do
                    local batch_end = math.min(total, state.file_index + 255)
                    for index = state.file_index, batch_end do
                        insert_file(files[index])
                    end
                    state.file_index = batch_end + 1
                    if os.clock() >= deadline then
                        break
                    end
                end
                if state.file_index > total then
                    state.panel:set_footer("Preparing project tree… · q close")
                    state.sort_stack = { root_node }
                    state.phase = "sort"
                else
                    state.panel:set_footer(string.format(
                        "Loading project tree… %d/%d files · q close",
                        state.file_index - 1,
                        total
                    ))
                    break
                end
            elseif phase == "sort" then
                local stack = state.sort_stack
                while #stack > 0 do
                    local node = stack[#stack]
                    stack[#stack] = nil
                    table.sort(node.children, node_before)
                    for _, child in ipairs(node.children) do
                        if child.is_dir then
                            stack[#stack + 1] = child
                        end
                    end
                    if os.clock() >= deadline then
                        break
                    end
                end
                if #stack == 0 then
                    state.color_stack = { { node = root_node, index = 1 } }
                    state.phase = "color"
                else
                    break
                end
            elseif phase == "color" then
                local stack = state.color_stack
                while #stack > 0 do
                    local item = stack[#stack]
                    stack[#stack] = nil
                    local node = item.node
                    node.color = PALETTE[item.index]
                    node.color_index = item.index
                    local next_index = item.index
                    for _, child in ipairs(node.children) do
                        if child.is_dir then
                            next_index = (next_index % #PALETTE) + 1
                            stack[#stack + 1] = { node = child, index = next_index }
                        end
                    end
                    if os.clock() >= deadline then
                        break
                    end
                end
                if #stack == 0 then
                    state.node_index = 1
                    state.tree_max_width = 0
                    state.phase = "finalize"
                else
                    break
                end
            elseif phase == "finalize" then
                local nodes = state.nodes
                local total = #nodes
                while state.node_index <= total do
                    local batch_end = math.min(total, state.node_index + 511)
                    for index = state.node_index, batch_end do
                        finalize_node(nodes[index])
                    end
                    state.node_index = batch_end + 1
                    if os.clock() >= deadline then
                        break
                    end
                end
                if state.node_index > total then
                    root_node.child_cont = EMPTY
                    root_node.child_cont_text = ""
                    state.root_label = "◆ " .. root_name .. "/"
                    state.root_segment = { text = state.root_label, style = root_node.color }

                    local columns, left_pad, root_extra = layout(state.tree_max_width)
                    state.columns = columns
                    state.left_pad = left_pad
                    state.root_extra = root_extra
                    state.rows = { make_root_row() }
                    state.rows_total = #state.nodes
                    state.row_stack = {
                        {
                            node = root_node,
                            continuation = root_node.child_cont,
                            text = root_node.child_cont_text,
                            index = 1,
                        },
                    }
                    state.panel:splice_lines(1, 1, { line_for(state.rows[1], false) })
                    state.phase = "rows"
                else
                    break
                end
            elseif phase == "rows" then
                local stack = state.row_stack
                local total = state.rows_total
                while #stack > 0 do
                    local produced = {}
                    for _ = 1, 512 do
                        local frame = stack[#stack]
                        local node = frame.node
                        local count = #node.children
                        if frame.index > count then
                            stack[#stack] = nil
                        else
                            local index = frame.index
                            local child = node.children[index]
                            frame.index = index + 1
                            local last = index == count
                            local row, next_cont, next_text = make_child_row(
                                node,
                                child,
                                frame.continuation,
                                frame.text,
                                last,
                                false
                            )
                            state.rows[#state.rows + 1] = row
                            produced[#produced + 1] = line_for(row, false)
                            if child.is_dir and #child.children > 0 then
                                stack[#stack + 1] = {
                                    node = child,
                                    continuation = next_cont,
                                    text = next_text,
                                    index = 1,
                                }
                            end
                        end
                        if #stack == 0 then
                            break
                        end
                    end
                    if #produced > 0 then
                        state.panel:splice_lines(#state.rows - #produced + 1, 0, produced)
                    end
                    if os.clock() >= deadline then
                        break
                    end
                end

                if #stack == 0 then
                    state.loading = false
                    state.phase = "done"
                    local title, footer = view_title_and_footer()
                    state.panel:set_title(title)
                    state.panel:set_footer(footer)
                    state.panel:set_selectable(true)
                    local selected = first_selectable(state.rows) or 1
                    state.current_index = selected
                    state.panel:set_selected(selected)
                    return
                end

                local percent = total > 0 and math.floor(#state.rows * 100 / total) or 99
                state.panel:set_footer(string.format(
                    "Loading project tree… %d%% · q close",
                    math.min(percent, 99)
                ))
                break
            else
                break
            end
        end

        if state.loading and not state.cancelled and state.generation == load_generation then
            hexal.ui.defer(load_step, 0)
        end
    end

    load_step = start_load_step

    local function refresh(select_key)
        local view = full_view()
        state.rows = view.rows
        state.panel:set_title(view.title)
        state.panel:set_footer(view.footer)
        state.panel:set_selectable(true)
        state.panel:set_lines(view.lines)

        local selected = nil
        if select_key ~= nil then
            selected = find_selected_index(view.rows, select_key)
        end
        if not selected then
            selected = first_selectable(view.rows)
        end

        state.current_index = selected or 1
        state.panel:set_selected(state.current_index)
    end

    local function selection_range()
        if state.selection_anchor ~= nil and state.current_index ~= nil then
            return math.min(state.selection_anchor, state.current_index),
                math.max(state.selection_anchor, state.current_index)
        end
        return nil, nil
    end

    -- Repaint a contiguous run of rows. `selection_first`/`selection_last`
    -- describe the new highlighted span (nil clears the highlight).
    local function paint_range(first, last, selection_first, selection_last)
        first = math.max(1, first)
        last = math.min(last, #state.rows)
        if first > last then
            return
        end

        local lines = {}
        for index = first, last do
            local in_selection = selection_first ~= nil
                and index >= selection_first
                and index <= selection_last
            lines[#lines + 1] = line_for(state.rows[index], in_selection)
        end
        if #lines > 0 then
            state.panel:splice_lines(first, #lines, lines)
        end
    end

    local function clear_selection_highlight()
        local first, last = selection_range()
        if first then
            paint_range(first, last, nil, nil)
        end
    end

    local function reset_selection_if_moved(selected)
        if state.selection_anchor ~= nil and state.current_index ~= selected then
            clear_selection_highlight()
            state.selection_anchor = nil
        end
    end

    local function is_selectable(index)
        local row = state.rows[index]
        return row ~= nil and row.selectable == true
    end

    local function move_index(selected, delta)
        local step = delta > 0 and 1 or -1
        local remaining = math.abs(delta)
        local index = selected
        while remaining > 0 do
            local candidate = index + step
            if candidate < 1 or candidate > #state.rows then
                break
            end
            index = candidate
            if is_selectable(index) then
                remaining = remaining - 1
            end
        end
        return index
    end

    local function extend_selection(selected, target)
        reset_selection_if_moved(selected)
        if not target then
            return true
        end

        local old_first, old_last = selection_range()
        if state.selection_anchor == nil then
            state.selection_anchor = selected
        end

        state.current_index = target
        local new_first = math.min(state.selection_anchor, target)
        local new_last = math.max(state.selection_anchor, target)
        local first = old_first and math.min(old_first, new_first) or new_first
        local last = old_last and math.max(old_last, new_last) or new_last

        paint_range(first, last, new_first, new_last)
        state.panel:set_selected(target)
        return true
    end

    local function copy_tree(selected)
        reset_selection_if_moved(selected)

        local first = 1
        local last = #state.rows
        if state.selection_anchor and state.current_index == selected then
            first = math.min(state.selection_anchor, selected)
            last = math.max(state.selection_anchor, selected)
        end

        local lines = {}
        for index = first, last do
            local text = state.rows[index].text
            if text and text ~= "" then
                lines[#lines + 1] = text
            end
        end

        if #lines == 0 then
            hexal.ui.warn("projtree: nothing to copy")
            return
        end

        if hexal.ui.copy(table.concat(lines, "\n")) then
            hexal.ui.message(
                string.format("Copied %d %s to clipboard", #lines, #lines == 1 and "line" or "lines")
            )
        end
    end

    local function insert_rows(start, new_rows)
        local count = #new_rows
        if count == 0 then
            return
        end
        local rows = state.rows
        local total = #rows
        if start <= total then
            table.move(rows, start, total, start + count)
        end
        for index = 1, count do
            rows[start + index - 1] = new_rows[index]
        end
    end

    local function remove_rows(start, count)
        if count <= 0 then
            return
        end
        local rows = state.rows
        local total = #rows
        local tail = start + count
        if tail <= total then
            table.move(rows, tail, total, start)
        end
        for index = total, total - count + 1, -1 do
            rows[index] = nil
        end
    end

    -- Toggle without rebuilding the rest of the tree: only the folder row and
    -- its newly inserted/removed subtree are sent to the host panel.
    local function toggle_folder(selected, node)
        local collapsed = state.collapsed[node.path] == true
        if current_columns() ~= state.columns then
            state.collapsed[node.path] = not collapsed
            refresh(node.path)
            return
        end

        local row = state.rows[selected]
        if collapsed then
            state.collapsed[node.path] = false
            row.label = node.open_label
            row.label_segment = node.open_segment
            row.text = row.prefix_text .. row.label

            local new_rows = {}
            append_children(new_rows, node, node.child_cont, node.child_cont_text)

            local new_lines = { line_for(row, false) }
            for index = 1, #new_rows do
                new_lines[#new_lines + 1] = line_for(new_rows[index], false)
            end
            state.panel:splice_lines(selected, 1, new_lines)
            state.rows[selected] = row
            insert_rows(selected + 1, new_rows)
        else
            state.collapsed[node.path] = true
            row.label = node.closed_label
            row.label_segment = node.closed_segment
            row.text = row.prefix_text .. row.label

            local depth = row.depth
            local last = selected
            local rows = state.rows
            while rows[last + 1] ~= nil and rows[last + 1].depth > depth do
                last = last + 1
            end

            local removed = last - selected
            state.panel:splice_lines(selected, 1 + removed, { line_for(row, false) })
            rows[selected] = row
            remove_rows(selected + 1, removed)
        end

        state.current_index = selected
        state.panel:set_selected(selected)
    end

    local on_key

    local function start_search(selected)
        local row = state.rows[selected]
        local previous = row and row.node and row.node.path or nil
        hexal.ui.prompt("Search project tree: ", function(value)
            if value == nil then
                return
            end

            local query = value:gsub("^%s+", ""):gsub("%s+$", ""):lower()
            state.query = query
            state.selection_anchor = nil
            state.current_index = nil
            refresh(previous)
        end)
    end

    on_key = function(key, selected)
        if state.loading then
            if key == "q" or key == "Q" or key == "esc" then
                state.cancelled = true
                return false
            end
            return true
        end

        if key == "shift+up" then
            return extend_selection(selected, move_index(selected, -1))
        elseif key == "shift+down" then
            return extend_selection(selected, move_index(selected, 1))
        elseif key == "shift+pageup" then
            return extend_selection(selected, move_index(selected, -10))
        elseif key == "shift+pagedown" then
            return extend_selection(selected, move_index(selected, 10))
        elseif key == "shift+home" then
            return extend_selection(selected, first_selectable(state.rows))
        elseif key == "shift+end" then
            return extend_selection(selected, last_selectable())
        end

        if key == "y" or key == "Y" then
            copy_tree(selected)
            return true
        end

        if key == "/" then
            if state.selection_anchor ~= nil then
                clear_selection_highlight()
            end
            state.selection_anchor = nil
            start_search(selected)
            return true
        end

        if key ~= "enter" then
            return false
        end

        if state.selection_anchor ~= nil then
            clear_selection_highlight()
            state.selection_anchor = nil
        end

        local row = state.rows[selected]
        local node = row and row.node
        if not node then
            return true
        end

        if node.is_dir then
            if state.query ~= "" then
                return true
            end

            toggle_folder(selected, node)
            return true
        end

        state.panel:close()
        hexal.commands.execute("e", absolute_path(node.path))
        return true
    end

    state.panel = hexal.ui.panel({
        title = " Project Tree · " .. root_name .. " ",
        footer = "Loading project tree… · q close",
        selectable = false,
        selected = 1,
        lines = {
            {
                segments = { { text = "  Loading project tree…", style = "muted" } },
                selectable = false,
            },
        },
    }, on_key)

    hexal.ui.defer(load_step, 0)
end

hexal.commands.register("pt", open_project_tree, "Show a Unicode tree of workspace files")
