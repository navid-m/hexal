//! Split panes: layout, focus movement, resizing and rendering.
//!
//! The tree and behaviour mirror the C# editor's `EditorPanes.cs`. The active
//! pane's view state lives in the `Editor` fields; inactive panes keep their
//! state in `Editor::panes`. Documents are moved between `Editor::doc` and
//! `Editor::documents` by id, so multiple panes showing the same buffer see the
//! same document object and edits made in one pane are visible in the others.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

const MIN_PANE_SPLIT_WIDTH: usize = 12;
const MIN_PANE_SPLIT_HEIGHT: usize = 3;
const PANE_RESIZE_STEP: f64 = 0.05;

impl Editor {
    pub(super) fn initialize_panes(&mut self) {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes = vec![PaneState {
            id,
            doc_id: self.doc.id(),
            language_override: self.language_override.clone(),
            mode: Mode::Normal,
            cursor: 0,
            anchor: 0,
            selection_linewise: false,
            extra_selections: Vec::new(),
            preferred_column: None,
            top_line: 0,
            left_column: 0,
            pending_g: false,
            mouse_dragging: false,
            mouse_drag_anchor: 0,
            find: FindState::new(),
            jumps: Vec::new(),
            jump_index: -1,
            highlighter: Highlighter::default(),
            git_diff: diff_lines::DiffLineState::new(),
        }];
        self.pane_root = PaneNode::Leaf(id);
        self.active_pane = id;
        self.capture_pane_state(id);
    }

    fn pane_index(&self, id: PaneId) -> Option<usize> {
        self.panes.iter().position(|pane| pane.id == id)
    }

    fn pane_by_id(&self, id: PaneId) -> Option<&PaneState> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    fn first_leaf(node: &PaneNode) -> Option<PaneId> {
        match node {
            PaneNode::Leaf(id) => Some(*id),
            PaneNode::Split { first, .. } => Self::first_leaf(first),
        }
    }

    fn collect_panes(node: &PaneNode, panes: &mut Vec<PaneId>) {
        match node {
            PaneNode::Leaf(id) => panes.push(*id),
            PaneNode::Split { first, second, .. } => {
                Self::collect_panes(first, panes);
                Self::collect_panes(second, panes);
            }
        }
    }

    fn all_pane_ids(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        Self::collect_panes(&self.pane_root, &mut panes);
        panes
    }

    pub(super) fn pane_count(&self) -> usize {
        self.all_pane_ids().len()
    }

    pub(super) fn capture_pane_state(&mut self, id: PaneId) {
        let Some(index) = self.pane_index(id) else {
            return;
        };
        let doc_id = self.doc.id();
        let language_override = self.language_override.clone();
        let mode = self.mode;
        let cursor = self.cursor;
        let anchor = self.anchor;
        let selection_linewise = self.selection_linewise;
        let extra_selections = std::mem::take(&mut self.extra_selections);
        let preferred_column = self.preferred_column;
        let top_line = self.top_line;
        let left_column = self.left_column;
        let pending_g = self.pending_g;
        let mouse_dragging = self.mouse_dragging;
        let mouse_drag_anchor = self.mouse_drag_anchor;
        let find = std::mem::replace(&mut self.find, FindState::new());
        let jumps = std::mem::take(&mut self.jumps);
        let jump_index = self.jump_index;
        let highlighter = std::mem::take(&mut self.highlighter);
        let git_diff = std::mem::replace(&mut self.git_diff, diff_lines::DiffLineState::new());

        let pane = &mut self.panes[index];
        pane.doc_id = doc_id;
        pane.language_override = language_override;
        pane.mode = mode;
        pane.cursor = cursor;
        pane.anchor = anchor;
        pane.selection_linewise = selection_linewise;
        pane.extra_selections = extra_selections;
        pane.preferred_column = preferred_column;
        pane.top_line = top_line;
        pane.left_column = left_column;
        pane.pending_g = pending_g;
        pane.mouse_dragging = mouse_dragging;
        pane.mouse_drag_anchor = mouse_drag_anchor;
        pane.find = find;
        pane.jumps = jumps;
        pane.jump_index = jump_index;
        pane.highlighter = highlighter;
        pane.git_diff = git_diff;
    }

    /// Move a document into the active slot, keeping the previous one in the
    /// document store. Callers prune unreferenced documents afterwards.
    fn activate_document(&mut self, id: u64) {
        if id == self.doc.id() {
            return;
        }
        let Some(next) = self.documents.remove(&id) else {
            return;
        };
        let previous_id = self.doc.id();
        let previous = std::mem::replace(&mut self.doc, next);
        self.documents.insert(previous_id, previous);
    }

    pub(super) fn load_pane_state(&mut self, id: PaneId) {
        let Some(index) = self.pane_index(id) else {
            return;
        };
        let doc_id = self.panes[index].doc_id;
        self.activate_document(doc_id);
        let pane = &mut self.panes[index];
        self.language_override = pane.language_override.take();
        self.mode = pane.mode;
        self.cursor = pane.cursor;
        self.anchor = pane.anchor;
        self.selection_linewise = pane.selection_linewise;
        self.extra_selections = std::mem::take(&mut pane.extra_selections);
        self.preferred_column = pane.preferred_column;
        self.top_line = pane.top_line;
        self.left_column = pane.left_column;
        self.pending_g = pane.pending_g;
        self.mouse_dragging = pane.mouse_dragging;
        self.mouse_drag_anchor = pane.mouse_drag_anchor;
        self.find = std::mem::replace(&mut pane.find, FindState::new());
        self.jumps = std::mem::take(&mut pane.jumps);
        self.jump_index = pane.jump_index;
        self.highlighter = std::mem::take(&mut pane.highlighter);
        self.git_diff = std::mem::replace(&mut pane.git_diff, diff_lines::DiffLineState::new());
        self.last_revision = self.doc.revision();
    }

    /// Store a newly opened document as the active one. The old document is
    /// retained only when another pane still displays it.
    pub(super) fn assign_active_document(&mut self, document: Document) {
        let previous_id = self.doc.id();
        let previous = std::mem::replace(&mut self.doc, document);
        self.documents.insert(previous_id, previous);
        self.prune_documents();
    }

    pub(super) fn prune_documents(&mut self) {
        let active_id = self.doc.id();
        let active_pane = self.active_pane;
        let referenced: std::collections::HashSet<u64> = self
            .panes
            .iter()
            .filter(|pane| pane.id != active_pane)
            .map(|pane| pane.doc_id)
            .collect();
        self.documents
            .retain(|id, _| *id != active_id && referenced.contains(id));
    }

    pub(super) fn active_pane_rows(&self) -> usize {
        self.active_pane_bounds().height
    }

    pub(super) fn active_pane_bounds(&self) -> PaneView {
        let views = self.pane_views();
        views
            .iter()
            .find(|view| view.pane == self.active_pane)
            .copied()
            .unwrap_or_else(|| {
                let x = self.editor_left();
                PaneView {
                    pane: self.active_pane,
                    x,
                    y: 0,
                    width: self.cols.saturating_sub(x).max(1),
                    height: self.text_rows(),
                }
            })
    }

    pub(super) fn pane_views(&self) -> Vec<PaneView> {
        let mut views = Vec::new();
        let mut dividers = Vec::new();
        let x = self.editor_left();
        let width = self.cols.saturating_sub(x);
        let height = self.text_rows();
        if width == 0 || height == 0 {
            return views;
        }
        Self::layout_pane_node(
            &self.pane_root,
            self.active_pane,
            x,
            0,
            width,
            height,
            &mut views,
            &mut dividers,
        );
        views
    }

    pub(super) fn pane_dividers(&self) -> Vec<PaneDivider> {
        let mut views = Vec::new();
        let mut dividers = Vec::new();
        let x = self.editor_left();
        let width = self.cols.saturating_sub(x);
        let height = self.text_rows();
        if width == 0 || height == 0 {
            return dividers;
        }
        Self::layout_pane_node(
            &self.pane_root,
            self.active_pane,
            x,
            0,
            width,
            height,
            &mut views,
            &mut dividers,
        );
        dividers
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_pane_node(
        node: &PaneNode,
        active: PaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        views: &mut Vec<PaneView>,
        dividers: &mut Vec<PaneDivider>,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let PaneNode::Split {
            orientation,
            ratio,
            first,
            second,
        } = node
        else {
            if let PaneNode::Leaf(pane) = node {
                views.push(PaneView {
                    pane: *pane,
                    x,
                    y,
                    width,
                    height,
                });
            }
            return;
        };

        let vertical = *orientation == PaneSplitOrientation::Vertical;
        let span = if vertical { width } else { height };
        if span < 3 {
            let visible = if Self::pane_node_contains(first, active) {
                first
            } else {
                second
            };
            Self::layout_pane_node(visible, active, x, y, width, height, views, dividers);
            return;
        }

        let available = span - 1;
        let first_size = ((available as f64) * ratio).round_ties_even() as usize;
        let first_size = first_size.clamp(1, available.saturating_sub(1));
        let second_size = available - first_size;
        if vertical {
            Self::layout_pane_node(first, active, x, y, first_size, height, views, dividers);
            Self::layout_pane_node(
                second,
                active,
                x + first_size + 1,
                y,
                second_size,
                height,
                views,
                dividers,
            );
            dividers.push(PaneDivider {
                x: x + first_size,
                y,
                length: height,
                vertical: true,
            });
        } else {
            Self::layout_pane_node(first, active, x, y, width, first_size, views, dividers);
            Self::layout_pane_node(
                second,
                active,
                x,
                y + first_size + 1,
                width,
                second_size,
                views,
                dividers,
            );
            dividers.push(PaneDivider {
                x,
                y: y + first_size,
                length: width,
                vertical: false,
            });
        }
    }

    fn pane_node_contains(node: &PaneNode, target: PaneId) -> bool {
        match node {
            PaneNode::Leaf(id) => *id == target,
            PaneNode::Split { first, second, .. } => {
                Self::pane_node_contains(first, target) || Self::pane_node_contains(second, target)
            }
        }
    }

    fn replace_leaf(node: &mut PaneNode, target: PaneId, replacement: PaneNode) -> bool {
        match node {
            PaneNode::Leaf(id) if *id == target => {
                *node = replacement;
                true
            }
            PaneNode::Split { first, second, .. } => {
                Self::replace_leaf(first, target, replacement.clone())
                    || Self::replace_leaf(second, target, replacement)
            }
            _ => false,
        }
    }

    fn remove_leaf(node: &mut PaneNode, target: PaneId) -> bool {
        let PaneNode::Split { first, second, .. } = node else {
            return false;
        };
        if matches!(**first, PaneNode::Leaf(id) if id == target) {
            *node = *std::mem::replace(second, Box::new(PaneNode::Leaf(0)));
            return true;
        }
        if matches!(**second, PaneNode::Leaf(id) if id == target) {
            *node = *std::mem::replace(first, Box::new(PaneNode::Leaf(0)));
            return true;
        }
        Self::remove_leaf(first, target) || Self::remove_leaf(second, target)
    }

    pub(super) fn split_pane(&mut self, orientation: PaneSplitOrientation) {
        let bounds = self.active_pane_bounds();
        if orientation == PaneSplitOrientation::Vertical
            && bounds.width < MIN_PANE_SPLIT_WIDTH * 2 + 1
        {
            self.set_error("Pane is too narrow to split vertically.");
            return;
        }
        if orientation == PaneSplitOrientation::Horizontal
            && bounds.height < MIN_PANE_SPLIT_HEIGHT * 2 + 1
        {
            self.set_error("Pane is too short to split horizontally.");
            return;
        }
        let Some(index) = self.pane_index(self.active_pane) else {
            self.set_error("Cannot split the active pane.");
            return;
        };

        self.capture_pane_state(self.active_pane);
        let mut created = self.panes[index].clone();
        created.id = self.next_pane_id;
        self.next_pane_id += 1;
        created.mode = Mode::Normal;
        created.pending_g = false;
        created.mouse_dragging = false;
        created.mouse_drag_anchor = 0;
        created.find.active = false;
        created.find.editing = false;
        created.highlighter = Highlighter::default();
        let created_id = created.id;
        self.panes.push(created);

        let old_id = self.active_pane;
        let replacement = PaneNode::Split {
            orientation,
            ratio: 0.5,
            first: Box::new(PaneNode::Leaf(old_id)),
            second: Box::new(PaneNode::Leaf(created_id)),
        };
        if !Self::replace_leaf(&mut self.pane_root, old_id, replacement) {
            self.panes.retain(|pane| pane.id != created_id);
            self.set_error("Cannot split the active pane.");
            return;
        }

        self.pending_window = false;
        self.active_pane = created_id;
        self.load_pane_state(created_id);
        self.hover.close();
        self.snap_cursor();
        self.ensure_cursor_visible();
        self.force_full_redraw = true;
        self.set_message(match orientation {
            PaneSplitOrientation::Vertical => "Split pane vertically.",
            PaneSplitOrientation::Horizontal => "Split pane horizontally.",
        });
    }

    pub(super) fn close_current_buffer(&mut self) {
        if self.panes.len() <= 1 {
            self.set_message("Only one buffer is open; use :q or :q! to quit Hexal.");
            return;
        }
        self.close_active_pane();
    }

    fn close_active_pane(&mut self) {
        if self.panes.len() <= 1 {
            self.set_error("Cannot close the last pane.");
            return;
        }
        let active = self.active_pane;
        let active_doc_id = self.doc.id();
        let displayed_elsewhere = self
            .panes
            .iter()
            .any(|pane| pane.id != active && pane.doc_id == active_doc_id);
        if self.doc.is_dirty() && !displayed_elsewhere {
            self.set_error("Save the buffer before closing its last pane.");
            return;
        }
        let Some(index) = self.pane_index(active) else {
            self.set_error("Cannot close the last pane.");
            return;
        };
        let same_document = self
            .all_pane_ids()
            .into_iter()
            .filter(|id| *id != active)
            .find(|id| {
                self.pane_by_id(*id)
                    .is_some_and(|pane| pane.doc_id == active_doc_id)
            });

        if !Self::remove_leaf(&mut self.pane_root, active) {
            self.set_error("Cannot close the last pane.");
            return;
        }
        self.panes.remove(index);
        let target = same_document
            .or_else(|| Self::first_leaf(&self.pane_root))
            .unwrap_or(active);
        let previous_doc_id = self.doc.id();
        self.load_pane_state(target);
        self.active_pane = target;
        self.pending_window = false;
        self.hover.close();
        if previous_doc_id != self.doc.id() {
            self.restart_lsp_for_buffer();
            self.fire_plugin_buffer_event("buffer_opened");
        }
        self.prune_documents();
        self.snap_cursor();
        self.ensure_cursor_visible();
        self.force_full_redraw = true;
        self.set_message("Closed pane.");
    }

    pub(super) fn switch_to_pane(&mut self, id: PaneId) {
        if id == self.active_pane || self.pane_index(id).is_none() {
            return;
        }
        let previous_doc_id = self.doc.id();
        self.capture_pane_state(self.active_pane);
        self.load_pane_state(id);
        self.active_pane = id;
        self.pending_window = false;
        self.hover.close();
        if previous_doc_id != self.doc.id() {
            self.restart_lsp_for_buffer();
            self.fire_plugin_buffer_event("buffer_opened");
        }
        self.prune_documents();
        self.snap_cursor();
        self.ensure_cursor_visible();
        self.force_full_redraw = true;
    }

    pub(super) fn focus_pane_direction(&mut self, direction: PaneDirection) {
        let views = self.pane_views();
        let Some(current) = views.iter().find(|view| view.pane == self.active_pane) else {
            return;
        };
        let current = *current;
        let mut choice = current;
        let mut best_overlap = 0usize;
        let mut best_distance = usize::MAX;

        for candidate in &views {
            if candidate.pane == self.active_pane {
                continue;
            }
            let overlap = Self::pane_overlap(current, *candidate, direction);
            if overlap == 0 {
                continue;
            }
            let Some(distance) = Self::pane_distance(current, *candidate, direction) else {
                continue;
            };
            if overlap > best_overlap || (overlap == best_overlap && distance < best_distance) {
                choice = *candidate;
                best_overlap = overlap;
                best_distance = distance;
            }
        }

        if choice.pane != self.active_pane {
            self.switch_to_pane(choice.pane);
            self.clear_message();
        }
    }

    fn pane_overlap(current: PaneView, candidate: PaneView, direction: PaneDirection) -> usize {
        match direction {
            PaneDirection::Left | PaneDirection::Right => (current.y + current.height)
                .min(candidate.y + candidate.height)
                .saturating_sub(current.y.max(candidate.y)),
            PaneDirection::Up | PaneDirection::Down => (current.x + current.width)
                .min(candidate.x + candidate.width)
                .saturating_sub(current.x.max(candidate.x)),
        }
    }

    fn pane_distance(
        current: PaneView,
        candidate: PaneView,
        direction: PaneDirection,
    ) -> Option<usize> {
        match direction {
            PaneDirection::Left if candidate.x + candidate.width <= current.x => {
                Some(current.x - (candidate.x + candidate.width))
            }
            PaneDirection::Right if candidate.x >= current.x + current.width => {
                Some(candidate.x - (current.x + current.width))
            }
            PaneDirection::Up if candidate.y + candidate.height <= current.y => {
                Some(current.y - (candidate.y + candidate.height))
            }
            PaneDirection::Down if candidate.y >= current.y + current.height => {
                Some(candidate.y - (current.y + current.height))
            }
            _ => None,
        }
    }

    fn adjust_pane_ratio(node: &mut PaneNode, target: PaneId, grow: bool) -> bool {
        let PaneNode::Split {
            ratio,
            first,
            second,
            ..
        } = node
        else {
            return false;
        };
        let first_target = matches!(**first, PaneNode::Leaf(id) if id == target);
        let second_target = matches!(**second, PaneNode::Leaf(id) if id == target);
        if !first_target && !second_target {
            return Self::adjust_pane_ratio(first, target, grow)
                || Self::adjust_pane_ratio(second, target, grow);
        }
        let mut delta = if first_target {
            PANE_RESIZE_STEP
        } else {
            -PANE_RESIZE_STEP
        };
        if !grow {
            delta = -delta;
        }
        *ratio = (*ratio + delta).clamp(0.1, 0.9);
        true
    }

    fn resize_active_pane(&mut self, grow: bool) {
        if !Self::adjust_pane_ratio(&mut self.pane_root, self.active_pane, grow) {
            self.set_message("Cannot resize the last pane.");
            return;
        }
        self.force_full_redraw = true;
        let ratio = self
            .pane_parent_ratio(self.active_pane)
            .map(|ratio| format!("{:.0}%", ratio * 100.0))
            .unwrap_or_else(|| "50%".into());
        self.set_message(&format!("Pane ratio {ratio}."));
    }

    fn pane_parent_ratio(&self, target: PaneId) -> Option<f64> {
        fn find(node: &PaneNode, target: PaneId) -> Option<f64> {
            match node {
                PaneNode::Leaf(_) => None,
                PaneNode::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    if matches!(**first, PaneNode::Leaf(id) if id == target)
                        || matches!(**second, PaneNode::Leaf(id) if id == target)
                    {
                        return Some(*ratio);
                    }
                    find(first, target).or_else(|| find(second, target))
                }
            }
        }
        find(&self.pane_root, target)
    }

    pub(super) fn handle_pane_command(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.clear_message();
            return;
        }

        let direction = match event.key {
            Key::Left => PaneDirection::Left,
            Key::Right => PaneDirection::Right,
            Key::Up => PaneDirection::Up,
            Key::Down => PaneDirection::Down,
            Key::Char if !event.ctrl && !event.alt => match event.ch {
                'h' | 'H' => PaneDirection::Left,
                'l' | 'L' => PaneDirection::Right,
                'k' | 'K' => PaneDirection::Up,
                'j' | 'J' => PaneDirection::Down,
                '+' | '=' => {
                    self.resize_active_pane(true);
                    return;
                }
                '-' | '_' => {
                    self.resize_active_pane(false);
                    return;
                }
                'q' | 'Q' | 'x' | 'X' => {
                    self.close_active_pane();
                    return;
                }
                _ => {
                    self.clear_message();
                    return;
                }
            },
            _ => return,
        };

        self.focus_pane_direction(direction);
    }

    pub(super) fn active_pane_ordinal(&self) -> usize {
        self.pane_views()
            .iter()
            .position(|view| view.pane == self.active_pane)
            .map(|index| index + 1)
            .unwrap_or(1)
    }

    pub(super) fn try_pane_at(&self, x: usize, y: usize) -> Option<PaneId> {
        self.pane_views()
            .into_iter()
            .find(|view| {
                x >= view.x && x < view.x + view.width && y >= view.y && y < view.y + view.height
            })
            .map(|view| view.pane)
    }

    pub(super) fn any_pane_document_dirty(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        if self.doc.is_dirty() {
            return true;
        }
        seen.insert(self.doc.id());
        for pane in &self.panes {
            if pane.id == self.active_pane || !seen.insert(pane.doc_id) {
                continue;
            }
            if self
                .documents
                .get(&pane.doc_id)
                .is_some_and(Document::is_dirty)
            {
                return true;
            }
        }
        false
    }

    fn render_pane_content(&mut self, view: PaneView, focused: bool) {
        if view.width == 0 || view.height == 0 {
            return;
        }
        let visible_height = if focused {
            self.pane_rows_above_find(&view).max(1)
        } else {
            view.height
        };

        self.pane_screen.resize(view.width, view.height);
        self.pane_screen.clear(self.theme.base());
        std::mem::swap(&mut self.screen, &mut self.pane_screen);

        let previous_cols = self.cols;
        let previous_rows = self.rows;
        let previous_left = self.editor_left_override;
        self.cols = view.width;
        self.rows = view.height + 2;
        self.editor_left_override = Some(0);

        let max_top = if self.word_wrap {
            self.doc.line_count().saturating_sub(1)
        } else {
            self.doc.line_count().saturating_sub(visible_height)
        };
        if self.top_line > max_top {
            self.top_line = max_top;
        }
        let gutter = self.editor_left() + self.gutter_width();
        let language = self.active_language();
        if self.word_wrap {
            let text_width = view.width.saturating_sub(gutter).max(1);
            let wrap_rows = self.build_wrap_rows(self.top_line, view.height, text_width);
            for (row, wrapped) in wrap_rows.iter().enumerate() {
                if wrapped.first_segment {
                    self.render_wrapped_line_number(wrapped.line, row, gutter);
                }
            }
            self.render_wrapped_text_lines(self.top_line, view.height, gutter, language.as_deref());
            self.render_wrapped_selection(self.top_line, view.height, gutter);
            self.render_wrapped_find_matches(self.top_line, view.height, gutter);
            if focused {
                self.render_wrapped_cursor(self.top_line, view.height, gutter);
            }
        } else {
            for row in 0..view.height {
                let line = self.top_line + row;
                if line >= self.doc.line_count() {
                    break;
                }
                self.render_line_number(line, row);
                self.render_line_text(line, row, gutter, language.as_deref());
            }
            self.render_selection(self.top_line, view.height, gutter);
            self.render_find_matches(self.top_line, view.height, gutter);
            if focused {
                self.render_cursor(self.top_line, view.height, gutter);
            }
        }

        self.cols = previous_cols;
        self.rows = previous_rows;
        self.editor_left_override = previous_left;
        std::mem::swap(&mut self.screen, &mut self.pane_screen);
        self.screen.blit(&self.pane_screen, view.x, view.y);
    }

    pub(super) fn render_panes(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let active = self.active_pane;
        self.capture_pane_state(active);
        let views = self.pane_views();
        for view in &views {
            self.load_pane_state(view.pane);
            self.render_pane_content(*view, view.pane == active);
            self.capture_pane_state(view.pane);
        }
        self.load_pane_state(active);
        self.render_pane_dividers();
    }

    fn render_pane_dividers(&mut self) {
        if self.pane_count() <= 1 {
            return;
        }
        let style = self.theme.border_style();
        for divider in self.pane_dividers() {
            if divider.vertical {
                for row in 0..divider.length {
                    self.screen
                        .put(divider.x as i64, (divider.y + row) as i64, '│', style, 1);
                }
            } else {
                for column in 0..divider.length {
                    self.screen
                        .put((divider.x + column) as i64, divider.y as i64, '─', style, 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(
            Document::from_bytes(None, b"one\ntwo\nthree\n".to_vec()),
            100,
            30,
        )
    }

    #[test]
    fn vertical_split_creates_a_focused_pane_on_the_same_document() {
        let mut editor = editor();
        let first = editor.active_pane;
        let doc_id = editor.doc.id();

        editor.split_pane(PaneSplitOrientation::Vertical);

        assert_eq!(editor.pane_count(), 2);
        assert_ne!(editor.active_pane, first);
        assert_eq!(editor.pane_by_id(first).unwrap().doc_id, doc_id);
        assert_eq!(
            editor.pane_by_id(editor.active_pane).unwrap().doc_id,
            doc_id
        );
        assert_eq!(editor.pane_views().len(), 2);
        assert!(editor.pane_views()[1].x > editor.pane_views()[0].x);
    }

    #[test]
    fn panes_showing_the_same_document_share_edits() {
        let mut editor = editor();
        let first = editor.active_pane;
        editor.split_pane(PaneSplitOrientation::Vertical);
        let second = editor.active_pane;

        editor.doc.insert(0, b"shared ");
        editor.switch_to_pane(first);
        assert!(editor.doc.all_text().starts_with(b"shared "));
        editor.switch_to_pane(second);
        assert!(editor.doc.all_text().starts_with(b"shared "));
    }

    #[test]
    fn horizontal_split_is_refused_when_the_pane_is_too_short() {
        let mut editor = Editor::new(Document::from_bytes(None, b"one\ntwo\n".to_vec()), 100, 6);
        let before = editor.active_pane;
        editor.split_pane(PaneSplitOrientation::Horizontal);
        assert_eq!(editor.pane_count(), 1);
        assert_eq!(editor.active_pane, before);
        assert!(editor.message.contains("too short"), "{}", editor.message);
    }

    #[test]
    fn vertical_split_is_refused_when_the_pane_is_too_narrow() {
        let mut editor = Editor::new(Document::from_bytes(None, b"one\ntwo\n".to_vec()), 24, 30);
        let before = editor.active_pane;
        editor.split_pane(PaneSplitOrientation::Vertical);
        assert_eq!(editor.pane_count(), 1);
        assert_eq!(editor.active_pane, before);
        assert!(editor.message.contains("too narrow"), "{}", editor.message);
    }

    #[test]
    fn quit_checks_dirty_documents_in_every_pane() {
        let mut editor = editor();
        editor.doc.insert(0, b"dirty ");
        editor.split_pane(PaneSplitOrientation::Vertical);
        editor.assign_active_document(Document::from_bytes(None, b"clean\n".to_vec()));
        assert!(!editor.doc.is_dirty());
        assert!(editor.any_pane_document_dirty());

        editor.request_quit(false);
        assert!(matches!(editor.pending, Pending::Quit));
    }

    #[test]
    fn pane_resize_clamps_the_ratio() {
        let mut editor = editor();
        editor.split_pane(PaneSplitOrientation::Vertical);
        let active = editor.active_pane;
        for _ in 0..30 {
            editor.resize_active_pane(true);
        }
        let ratio = editor.pane_parent_ratio(active).unwrap();
        assert!((ratio - 0.1).abs() < f64::EPSILON);
        for _ in 0..30 {
            editor.resize_active_pane(false);
        }
        let ratio = editor.pane_parent_ratio(active).unwrap();
        assert!((ratio - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn focus_direction_follows_the_pane_layout() {
        let mut editor = editor();
        editor.split_pane(PaneSplitOrientation::Vertical);
        let right = editor.active_pane;
        editor.split_pane(PaneSplitOrientation::Horizontal);
        let bottom_right = editor.active_pane;

        editor.focus_pane_direction(PaneDirection::Up);
        assert_ne!(editor.active_pane, bottom_right);
        assert_eq!(editor.active_pane, right);

        editor.focus_pane_direction(PaneDirection::Left);
        assert_ne!(editor.active_pane, right);
        assert_eq!(editor.pane_views()[0].pane, editor.active_pane);
    }

    #[test]
    fn closing_a_dirty_document_in_its_last_pane_is_refused() {
        let mut editor = editor();
        editor.doc.insert(0, b"dirty ");
        let dirty_doc = editor.doc.id();
        let first = editor.active_pane;
        editor.split_pane(PaneSplitOrientation::Vertical);
        editor.assign_active_document(Document::from_bytes(None, b"other\n".to_vec()));

        editor.switch_to_pane(first);
        assert_eq!(editor.doc.id(), dirty_doc);
        editor.close_current_buffer();

        assert_eq!(editor.pane_count(), 2);
        assert!(
            editor
                .message
                .contains("Save the buffer before closing its last pane"),
            "{}",
            editor.message
        );
    }

    #[test]
    fn a_closed_pane_document_is_dropped_when_nothing_else_displays_it() {
        let mut editor = editor();
        editor.split_pane(PaneSplitOrientation::Vertical);
        editor.assign_active_document(Document::from_bytes(None, b"other\n".to_vec()));
        assert_eq!(editor.documents.len(), 1);

        editor.close_current_buffer();
        assert_eq!(editor.pane_count(), 1);
        assert!(editor.documents.is_empty());
    }

    #[test]
    fn panes_render_with_a_divider_between_them() {
        let mut editor = editor();
        editor.screen.resize(editor.cols, editor.rows);
        editor.split_pane(PaneSplitOrientation::Vertical);
        editor.render_panes();
        let divider = editor.pane_dividers()[0];
        assert!(divider.vertical);
        assert_eq!(editor.screen.at(divider.x, 0).ch, '│');
    }
}
