//! Plugin-provided panels, mirroring the C# `EditorPluginPanels` partial.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::scripting::lua::{PanelLine, PluginPanel};
use crate::ui::theme::Style;
use std::sync::Arc;

const MOUSE_SCROLL_LINES: usize = 3;

pub(super) struct PluginPanelState {
    id: i64,
    title: String,
    footer: String,
    lines: Arc<Vec<PanelLine>>,
    selected: usize,
    top: usize,
    selectable: bool,
    host_selected: usize,
}

impl PluginPanelState {
    fn from_panel(panel: &PluginPanel) -> PluginPanelState {
        let mut state = PluginPanelState {
            id: panel.id,
            title: panel.title.clone(),
            footer: panel.footer.clone(),
            lines: panel.lines.clone(),
            selected: panel.selected,
            top: 0,
            selectable: panel.selectable,
            host_selected: panel.selected,
        };
        state.selected = state.find_selectable(state.selected);
        state
    }

    fn find_selectable(&self, preferred: usize) -> usize {
        let count = self.lines.len();
        if count == 0 {
            return 0;
        }
        let start = preferred.min(count - 1);
        if !self.selectable {
            return start;
        }
        for index in start..count {
            if self.lines[index].selectable {
                return index;
            }
        }
        for index in (0..start).rev() {
            if self.lines[index].selectable {
                return index;
            }
        }
        start
    }
}

impl Editor {
    pub(super) fn sync_plugin_panels(&mut self) {
        let panels = self
            .plugins
            .as_ref()
            .map(|manager| manager.panels())
            .unwrap_or_default();

        if let Some(state) = &mut self.plugin_panel {
            if let Some(panel) = panels.iter().find(|panel| panel.id == state.id) {
                state.title = panel.title.clone();
                state.footer = panel.footer.clone();
                state.lines = panel.lines.clone();
                state.selectable = panel.selectable;
                if panel.selected != state.host_selected {
                    state.host_selected = panel.selected;
                    state.selected = state.find_selectable(panel.selected);
                }
                state.selected = state.find_selectable(state.selected);
                return;
            }

            if let Some(panel) = panels.iter().max_by_key(|panel| panel.id) {
                if panel.id > state.id {
                    self.plugin_panel = Some(PluginPanelState::from_panel(panel));
                    return;
                }
            }
            self.plugin_panel_dismissed = state.id;
            self.plugin_panel = None;
            return;
        }

        if let Some(panel) = panels.iter().max_by_key(|panel| panel.id) {
            if panel.id > self.plugin_panel_dismissed {
                self.plugin_panel = Some(PluginPanelState::from_panel(panel));
            }
        }
    }

    fn close_plugin_panel(&mut self) {
        if let Some(panel) = self.plugin_panel.take() {
            self.plugin_panel_dismissed = panel.id;
        }
    }

    fn plugin_panel_rows(&self) -> usize {
        let available = self.status_row();
        let height = self.plugin_panel_height(available);
        height.saturating_sub(3).max(1)
    }

    fn plugin_panel_height(&self, available: usize) -> usize {
        if available < 5 {
            return available;
        }
        let lines = self
            .plugin_panel
            .as_ref()
            .map(|panel| panel.lines.len())
            .unwrap_or(0);
        available.min((lines + 3).max(5))
    }

    fn ensure_plugin_panel_selection_visible(&mut self) {
        let rows = self.plugin_panel_rows();
        let Some(state) = &mut self.plugin_panel else {
            return;
        };
        if state.selected < state.top {
            state.top = state.selected;
        } else if state.selected >= state.top + rows {
            state.top = state.selected + 1 - rows;
        }
        state.top = state.top.min(state.lines.len().saturating_sub(rows));
    }

    fn move_plugin_panel_selection(&mut self, delta: isize) {
        let rows = self.plugin_panel_rows();
        let Some(state) = &mut self.plugin_panel else {
            return;
        };
        if state.lines.is_empty() || delta == 0 {
            return;
        }
        if !state.selectable {
            let maximum = state.lines.len().saturating_sub(rows);
            let top = state.top as isize + delta;
            state.top = top.clamp(0, maximum.max(0) as isize) as usize;
            return;
        }
        let step = delta.signum();
        let mut remaining = delta.unsigned_abs();
        let mut index = state.selected;
        while remaining > 0 {
            let next = index as isize + step;
            if next < 0 || next as usize >= state.lines.len() {
                break;
            }
            index = next as usize;
            if state.lines[index].selectable {
                remaining -= 1;
            }
        }
        state.selected = index;
        self.ensure_plugin_panel_selection_visible();
    }

    fn move_plugin_panel_to_edge(&mut self, first: bool) {
        let rows = self.plugin_panel_rows();
        let Some(state) = &mut self.plugin_panel else {
            return;
        };
        if state.lines.is_empty() {
            return;
        }
        if !state.selectable {
            state.top = if first {
                0
            } else {
                state.lines.len().saturating_sub(rows)
            };
            return;
        }
        let mut index = if first { 0 } else { state.lines.len() - 1 };
        let step = if first { 1isize } else { -1 };
        while index < state.lines.len() {
            if state.lines[index].selectable {
                state.selected = index;
                self.ensure_plugin_panel_selection_visible();
                return;
            }
            let next = index as isize + step;
            if next < 0 {
                break;
            }
            index = next as usize;
        }
    }

    pub(super) fn handle_plugin_panel_input(&mut self, event: Event) {
        match event.key {
            Key::Escape => {
                if !self.plugin_panel_key("esc") {
                    self.close_plugin_panel();
                }
                self.force_full_redraw = true;
                return;
            }
            Key::Enter => {
                self.plugin_panel_key("enter");
                self.force_full_redraw = true;
                return;
            }
            Key::Backspace => {
                if !self.plugin_panel_key("backspace") {
                    self.close_plugin_panel();
                }
                self.force_full_redraw = true;
                return;
            }
            Key::Tab => {
                self.plugin_panel_key("tab");
                self.force_full_redraw = true;
                return;
            }
            Key::Up | Key::ScrollUp | Key::Down | Key::ScrollDown => {
                let direction = if matches!(event.key, Key::Up | Key::ScrollUp) {
                    -1
                } else {
                    1
                };
                let step = if matches!(event.key, Key::ScrollUp | Key::ScrollDown) {
                    MOUSE_SCROLL_LINES as isize
                } else {
                    1
                };
                let name = if direction < 0 {
                    "shift+up"
                } else {
                    "shift+down"
                };
                if event.shift {
                    if self.plugin_panel_key(name) {
                        self.force_full_redraw = true;
                        return;
                    }
                }
                self.move_plugin_panel_selection(direction * step);
                self.force_full_redraw = true;
                return;
            }
            Key::PageUp | Key::PageDown => {
                let direction = if event.key == Key::PageUp { -1 } else { 1 };
                let name = if direction < 0 {
                    "shift+pageup"
                } else {
                    "shift+pagedown"
                };
                if event.shift && self.plugin_panel_key(name) {
                    self.force_full_redraw = true;
                    return;
                }
                let rows = self.plugin_panel_rows() as isize;
                self.move_plugin_panel_selection(direction * rows);
                self.force_full_redraw = true;
                return;
            }
            Key::Home | Key::End => {
                let first = event.key == Key::Home;
                let name = if first { "shift+home" } else { "shift+end" };
                if event.shift && self.plugin_panel_key(name) {
                    self.force_full_redraw = true;
                    return;
                }
                self.move_plugin_panel_to_edge(first);
                self.force_full_redraw = true;
                return;
            }
            _ => {}
        }

        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }
        match event.ch {
            'j' | 'J' => {
                self.move_plugin_panel_selection(1);
                self.force_full_redraw = true;
            }
            'k' | 'K' => {
                self.move_plugin_panel_selection(-1);
                self.force_full_redraw = true;
            }
            'g' => {
                self.move_plugin_panel_to_edge(true);
                self.force_full_redraw = true;
            }
            'G' => {
                self.move_plugin_panel_to_edge(false);
                self.force_full_redraw = true;
            }
            'q' | 'Q' => {
                if !self.plugin_panel_key("q") {
                    self.close_plugin_panel();
                }
                self.force_full_redraw = true;
            }
            _ => {
                if !self.plugin_panel_key(&event.ch.to_string()) {
                    self.force_full_redraw = true;
                }
            }
        }
    }

    fn plugin_panel_key(&mut self, key: &str) -> bool {
        let Some(panel_id) = self.plugin_panel.as_ref().map(|panel| panel.id) else {
            return false;
        };
        let selected = self
            .plugin_panel
            .as_ref()
            .map(|panel| panel.selected)
            .unwrap_or(0);
        let Some(mut manager) = self.plugins.take() else {
            return false;
        };
        self.sync_plugin_context(&mut manager);
        let revision = self.doc.revision();
        let result =
            manager.handle_panel_key(panel_id, key, selected, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.apply_plugin_effects();
        if self.doc.revision() != revision {
            self.anchor = self.cursor;
            self.selection_linewise = false;
            self.preferred_column = None;
            self.snap_cursor();
            self.doc.break_undo_group();
        }
        match result {
            Some(Ok(handled)) => handled,
            Some(Err(error)) => {
                crate::util::log::write(&format!("plugin panel key '{key}' failed: {error}"));
                self.set_error(&format!("plugin panel: {error}"));
                false
            }
            None => false,
        }
    }

    pub(super) fn render_plugin_panel(&mut self) {
        let Some((title, footer, selected, selectable)) = self.plugin_panel.as_ref().map(|panel| {
            (
                panel.title.clone(),
                panel.footer.clone(),
                panel.selected,
                panel.selectable,
            )
        }) else {
            return;
        };
        let available = self.status_row();
        if available < 5 || self.cols < 20 {
            return;
        }
        let height = self.plugin_panel_height(available);
        let width = self.cols.saturating_sub(2).min(160);
        if width < 20 || height < 5 {
            return;
        }
        let x = (self.cols.saturating_sub(width)) / 2;
        let y = (available.saturating_sub(height)) / 2;
        self.draw_box(x, y, width, height, &title);

        let content_rows = (height - 3).max(1);
        let content_width = width.saturating_sub(4);
        let max_top = self
            .plugin_panel
            .as_ref()
            .map(|panel| panel.lines.len())
            .unwrap_or(0)
            .saturating_sub(content_rows);
        if let Some(panel) = &mut self.plugin_panel {
            panel.top = panel.top.min(max_top);
        }
        let top = self
            .plugin_panel
            .as_ref()
            .map(|panel| panel.top)
            .unwrap_or(0);

        for row in 0..content_rows {
            let index = top + row;
            let Some(line) = self
                .plugin_panel
                .as_ref()
                .and_then(|panel| panel.lines.get(index))
            else {
                break;
            };
            let line_selected = selectable && line.selectable && index == selected;
            let mut column = 0usize;
            for segment in line.segments.iter() {
                if column >= content_width {
                    break;
                }
                let mut style = self.plugin_panel_style(&segment.style);
                if line_selected && !segment.text.trim().is_empty() {
                    style.bg = self.theme.selection_bg;
                    style.bold = true;
                    style.dim = false;
                }
                self.screen.put_str(
                    (x + 2 + column) as i64,
                    (y + 1 + row) as i64,
                    &segment.text,
                    style,
                    content_width.saturating_sub(column),
                );
                column += util::text_width(&segment.text);
            }
        }

        if !footer.is_empty() {
            self.screen.put_str(
                (x + 2) as i64,
                (y + height - 2) as i64,
                &footer,
                self.theme.muted_style(),
                width.saturating_sub(4),
            );
        }
    }

    fn plugin_panel_style(&self, name: &str) -> Style {
        match name.to_ascii_lowercase().as_str() {
            "accent" => self.theme.accent_style(),
            "muted" => self.theme.muted_style(),
            "error" => self.theme.error_style(),
            "warning" => self.theme.warning_style(),
            "success" => Style {
                fg: self.theme.success,
                bg: self.theme.background,
                bold: true,
                ..Style::default()
            },
            "diff_added" => Style {
                fg: self.theme.diff_added,
                bg: self.theme.background,
                ..Style::default()
            },
            "diff_deleted" => Style {
                fg: self.theme.diff_deleted,
                bg: self.theme.background,
                ..Style::default()
            },
            "diff_modified" => Style {
                fg: self.theme.diff_modified,
                bg: self.theme.background,
                ..Style::default()
            },
            "keyword" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Keyword);
                style.bold = true;
                style
            }
            "control" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Control);
                style.bold = true;
                style
            }
            "type" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Type);
                style.bold = true;
                style
            }
            "constant" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Constant);
                style.bold = true;
                style
            }
            "string" => {
                let mut style = self.theme.syntax_style(SyntaxKind::String);
                style.bold = true;
                style
            }
            "number" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Number);
                style.bold = true;
                style
            }
            "comment" => {
                let mut style = self.theme.syntax_style(SyntaxKind::Comment);
                style.dim = true;
                style
            }
            _ => self.theme.text(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;

    #[test]
    fn renders_plugin_panel_lines_and_footer() {
        let mut editor = Editor::new(Document::new(), 80, 24);
        editor.screen.resize(80, 24);
        editor.plugin_panel = Some(PluginPanelState {
            id: 1,
            title: " Demo ".into(),
            footer: "q close".into(),
            lines: Arc::new(vec![PanelLine {
                segments: Arc::from([crate::scripting::lua::PanelSegment {
                    text: "hello panel".into(),
                    style: "accent".into(),
                }]),
                selectable: true,
            }]),
            selected: 0,
            top: 0,
            selectable: true,
            host_selected: 0,
        });

        editor.render_plugin_panel();
        let screen: String = (0..24)
            .map(|row| {
                (0..80)
                    .map(|column| editor.screen.at(column, row).ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Demo"), "{screen}");
        assert!(screen.contains("hello panel"), "{screen}");
        assert!(screen.contains("q close"), "{screen}");
    }
}
