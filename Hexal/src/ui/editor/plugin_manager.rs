//! Plugin manager modal, mirroring the C# `EditorPlugins` manager UI.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::scripting::plugin::PluginInfo;
use crate::ui::theme::Style;

pub(super) struct PluginManagerState {
    pub visible: bool,
    selected: usize,
    top: usize,
    filter: String,
    filter_cursor: usize,
    filtering: bool,
    items: Vec<PluginInfo>,
}

impl PluginManagerState {
    pub(super) fn new() -> PluginManagerState {
        PluginManagerState {
            visible: false,
            selected: 0,
            top: 0,
            filter: String::new(),
            filter_cursor: 0,
            filtering: false,
            items: Vec::new(),
        }
    }
}

impl Editor {
    pub(super) fn open_plugin_manager(&mut self) {
        self.clear_message();
        self.plugin_manager.filter.clear();
        self.plugin_manager.filter_cursor = 0;
        self.plugin_manager.filtering = false;
        self.refresh_plugin_manager_items(false);
        self.plugin_manager.visible = true;
        self.force_full_redraw = true;
    }

    fn close_plugin_manager(&mut self) {
        self.plugin_manager.visible = false;
        self.plugin_manager.top = 0;
        self.plugin_manager.filter.clear();
        self.plugin_manager.filter_cursor = 0;
        self.plugin_manager.filtering = false;
        self.force_full_redraw = true;
    }

    fn plugin_manager_matches(&self, plugin: &PluginInfo) -> bool {
        let filter = self.plugin_manager.filter.to_ascii_lowercase();
        if filter.is_empty() {
            return true;
        }
        plugin.name.to_ascii_lowercase().contains(&filter)
            || plugin.id.to_ascii_lowercase().contains(&filter)
            || plugin.description.to_ascii_lowercase().contains(&filter)
            || plugin.author.to_ascii_lowercase().contains(&filter)
    }

    fn refresh_plugin_manager_items(&mut self, keep_selection: bool) {
        let selected_id = if keep_selection {
            self.plugin_manager
                .items
                .get(self.plugin_manager.selected)
                .map(|plugin| plugin.id.clone())
        } else {
            None
        };

        let mut items: Vec<PluginInfo> = self
            .plugins
            .as_ref()
            .map(|manager| manager.plugins())
            .unwrap_or_default();
        items.retain(|plugin| self.plugin_manager_matches(plugin));
        self.plugin_manager.items = items;
        self.plugin_manager.selected = 0;
        if let Some(id) = selected_id {
            if let Some(index) = self
                .plugin_manager
                .items
                .iter()
                .position(|plugin| plugin.id == id)
            {
                self.plugin_manager.selected = index;
            }
        }
        let last = self.plugin_manager.items.len().saturating_sub(1);
        self.plugin_manager.selected = self.plugin_manager.selected.min(last);
        let rows = self.plugin_manager_list_rows();
        self.plugin_manager.top = self
            .plugin_manager
            .top
            .min(self.plugin_manager.items.len().saturating_sub(rows));
    }

    fn plugin_manager_height(&self) -> usize {
        (self.rows.saturating_sub(2)).max(5).min(24)
    }

    fn plugin_manager_width(&self) -> usize {
        self.cols.saturating_sub(4).clamp(20, 100)
    }

    fn plugin_manager_list_rows(&self) -> usize {
        self.plugin_manager_height().saturating_sub(3).max(1)
    }

    fn plugin_manager_selected_item(&self) -> Option<PluginInfo> {
        self.plugin_manager
            .items
            .get(self.plugin_manager.selected)
            .cloned()
    }

    fn move_plugin_manager_selection(&mut self, delta: isize) {
        let count = self.plugin_manager.items.len();
        if count == 0 {
            return;
        }
        let next = (self.plugin_manager.selected as isize + delta)
            .clamp(0, count.saturating_sub(1) as isize);
        self.plugin_manager.selected = next as usize;
        self.ensure_plugin_manager_selection_visible();
    }

    fn ensure_plugin_manager_selection_visible(&mut self) {
        let rows = self.plugin_manager_list_rows();
        if self.plugin_manager.selected < self.plugin_manager.top {
            self.plugin_manager.top = self.plugin_manager.selected;
        } else if self.plugin_manager.selected >= self.plugin_manager.top + rows {
            self.plugin_manager.top = self.plugin_manager.selected + 1 - rows;
        }
        self.plugin_manager.top = self
            .plugin_manager
            .top
            .min(self.plugin_manager.items.len().saturating_sub(rows));
    }

    pub(super) fn handle_plugin_manager_input(&mut self, event: Event) {
        if self.plugin_manager.filtering {
            self.handle_plugin_manager_filter_input(event);
            return;
        }

        match event.key {
            Key::Escape => {
                if !self.plugin_manager.filter.is_empty() {
                    self.plugin_manager.filter.clear();
                    self.plugin_manager.filter_cursor = 0;
                    self.refresh_plugin_manager_items(false);
                    self.force_full_redraw = true;
                    return;
                }
                self.close_plugin_manager();
                return;
            }
            Key::Up | Key::ScrollUp => {
                self.move_plugin_manager_selection(-1);
                return;
            }
            Key::Down | Key::ScrollDown => {
                self.move_plugin_manager_selection(1);
                return;
            }
            Key::PageUp => {
                let rows = self.plugin_manager_list_rows() as isize;
                self.move_plugin_manager_selection(-rows);
                return;
            }
            Key::PageDown => {
                let rows = self.plugin_manager_list_rows() as isize;
                self.move_plugin_manager_selection(rows);
                return;
            }
            Key::Home => {
                self.plugin_manager.selected = 0;
                self.ensure_plugin_manager_selection_visible();
                return;
            }
            Key::End => {
                self.plugin_manager.selected = self.plugin_manager.items.len().saturating_sub(1);
                self.ensure_plugin_manager_selection_visible();
                return;
            }
            Key::Enter => {
                self.toggle_selected_plugin();
                return;
            }
            _ => {}
        }

        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }
        match event.ch {
            '/' => {
                self.plugin_manager.filtering = true;
                self.plugin_manager.filter_cursor = self.plugin_manager.filter.len();
                self.force_full_redraw = true;
            }
            'q' | 'Q' => self.close_plugin_manager(),
            'k' | 'K' => self.move_plugin_manager_selection(-1),
            'j' | 'J' => self.move_plugin_manager_selection(1),
            ' ' | 'e' | 'E' | 't' | 'T' => self.toggle_selected_plugin(),
            's' | 'S' => self.open_selected_plugin_source(),
            'r' => self.reload_selected_plugin(),
            'R' => self.reload_all_plugins(),
            _ => {}
        }
    }

    fn handle_plugin_manager_filter_input(&mut self, event: Event) {
        match event.key {
            Key::Escape => {
                self.plugin_manager.filter.clear();
                self.plugin_manager.filter_cursor = 0;
                self.plugin_manager.filtering = false;
                self.refresh_plugin_manager_items(false);
                self.force_full_redraw = true;
                return;
            }
            Key::Enter => {
                self.plugin_manager.filtering = false;
                self.force_full_redraw = true;
                return;
            }
            Key::Backspace => {
                if self.plugin_manager.filter_cursor > 0 {
                    let previous = util::previous_boundary(
                        self.plugin_manager.filter.as_bytes(),
                        self.plugin_manager.filter_cursor,
                    );
                    self.plugin_manager
                        .filter
                        .replace_range(previous..self.plugin_manager.filter_cursor, "");
                    self.plugin_manager.filter_cursor = previous;
                    self.refresh_plugin_manager_items(true);
                    self.force_full_redraw = true;
                }
                return;
            }
            Key::Delete => {
                if self.plugin_manager.filter_cursor < self.plugin_manager.filter.len() {
                    let next = util::next_boundary(
                        self.plugin_manager.filter.as_bytes(),
                        self.plugin_manager.filter_cursor,
                    );
                    self.plugin_manager
                        .filter
                        .replace_range(self.plugin_manager.filter_cursor..next, "");
                    self.refresh_plugin_manager_items(true);
                    self.force_full_redraw = true;
                }
                return;
            }
            Key::Left => {
                self.plugin_manager.filter_cursor = util::previous_boundary(
                    self.plugin_manager.filter.as_bytes(),
                    self.plugin_manager.filter_cursor,
                );
                return;
            }
            Key::Right => {
                self.plugin_manager.filter_cursor = util::next_boundary(
                    self.plugin_manager.filter.as_bytes(),
                    self.plugin_manager.filter_cursor,
                );
                return;
            }
            Key::Up | Key::ScrollUp => {
                self.move_plugin_manager_selection(-1);
                return;
            }
            Key::Down | Key::ScrollDown => {
                self.move_plugin_manager_selection(1);
                return;
            }
            Key::PageUp => {
                let rows = self.plugin_manager_list_rows() as isize;
                self.move_plugin_manager_selection(-rows);
                return;
            }
            Key::PageDown => {
                let rows = self.plugin_manager_list_rows() as isize;
                self.move_plugin_manager_selection(rows);
                return;
            }
            Key::Home => {
                if event.ctrl {
                    self.plugin_manager.selected = 0;
                    self.ensure_plugin_manager_selection_visible();
                } else {
                    self.plugin_manager.filter_cursor = 0;
                }
                return;
            }
            Key::End => {
                if event.ctrl {
                    self.plugin_manager.selected =
                        self.plugin_manager.items.len().saturating_sub(1);
                    self.ensure_plugin_manager_selection_visible();
                } else {
                    self.plugin_manager.filter_cursor = self.plugin_manager.filter.len();
                }
                return;
            }
            _ => {}
        }
        if event.key != Key::Char || event.ctrl || event.alt || event.ch == '\0' {
            return;
        }
        let cursor = self
            .plugin_manager
            .filter_cursor
            .min(self.plugin_manager.filter.len());
        self.plugin_manager.filter.insert(cursor, event.ch);
        self.plugin_manager.filter_cursor = cursor + event.ch.len_utf8();
        self.refresh_plugin_manager_items(true);
        self.force_full_redraw = true;
    }

    fn toggle_selected_plugin(&mut self) {
        let Some(plugin) = self.plugin_manager_selected_item() else {
            return;
        };
        let enable = !plugin.enabled;
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        let result = manager.set_enabled(&plugin.id, enable, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.apply_plugin_effects();
        if let Err(error) = result {
            crate::util::log::write(&format!("plugin '{}': {error}", plugin.id));
            self.set_error(&format!("plugin '{}': {error}", plugin.id));
        } else {
            if enable {
                self.fire_plugin_started_and_opened();
            }
            self.set_message(&format!(
                "Plugin {} {}.",
                plugin.id,
                if enable { "enabled" } else { "disabled" }
            ));
        }
        self.refresh_plugin_manager_items(true);
        self.force_full_redraw = true;
    }

    fn open_selected_plugin_source(&mut self) {
        let Some(plugin) = self.plugin_manager_selected_item() else {
            return;
        };
        let path = if !plugin.entry_path.is_empty() {
            plugin.entry_path
        } else if !plugin.directory.is_empty() {
            Path::new(&plugin.directory)
                .join("init.lua")
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };
        if path.is_empty() {
            self.set_error(&format!(
                "plugin directory is unavailable for {}",
                plugin.id
            ));
            return;
        }
        self.close_plugin_manager();
        self.request_open(&path, 0, 0, false);
    }

    fn reload_selected_plugin(&mut self) {
        let Some(plugin) = self.plugin_manager_selected_item() else {
            return;
        };
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        let result = manager.reload_plugin(&plugin.id, &mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.apply_plugin_effects();
        if let Err(error) = result {
            crate::util::log::write(&format!("plugin '{}': {error}", plugin.id));
            self.set_error(&format!("plugin '{}': {error}", plugin.id));
        } else {
            self.fire_plugin_started_and_opened();
            self.set_message(&format!("Reloaded plugin {}.", plugin.id));
        }
        self.refresh_plugin_manager_items(true);
        self.force_full_redraw = true;
    }

    fn reload_all_plugins(&mut self) {
        let Some(mut manager) = self.plugins.take() else {
            return;
        };
        self.sync_plugin_context(&mut manager);
        let result = manager.reload_all(&mut self.doc, &mut self.cursor);
        self.plugins = Some(manager);
        self.apply_plugin_effects();
        self.fire_plugin_started_and_opened();
        match result {
            Ok(()) => self.set_message("Reloaded all plugins."),
            Err(error) => {
                crate::util::log::write(&format!("plugins: {error}"));
                self.set_error(&format!("plugins: {error}"));
            }
        }
        self.refresh_plugin_manager_items(true);
        self.force_full_redraw = true;
    }

    fn fire_plugin_started_and_opened(&mut self) {
        let mut data = crate::util::json::Value::Object(Vec::new());
        data.set("api_version", crate::util::json::Value::Number(1.0));
        self.fire_plugin_event("editor_started", data);
        self.fire_plugin_buffer_event("buffer_opened");
    }

    pub(super) fn render_plugin_manager(&mut self) {
        if !self.plugin_manager.visible {
            return;
        }
        let width = self.plugin_manager_width();
        let height = self.plugin_manager_height();
        if width < 24 || height < 5 || width > self.cols || height > self.rows {
            return;
        }
        let x = self.cols.saturating_sub(width) / 2;
        let y = self.rows.saturating_sub(height) / 2;
        let title = if !self.plugin_manager.filter.is_empty() || self.plugin_manager.filtering {
            if self.plugin_manager.filtering {
                let cursor = self
                    .plugin_manager
                    .filter_cursor
                    .min(self.plugin_manager.filter.len());
                let (before, after) = self.plugin_manager.filter.split_at(cursor);
                format!(" Plugins · /{before}▏{after} ")
            } else {
                format!(" Plugins · /{} ", self.plugin_manager.filter)
            }
        } else {
            " Plugins ".to_string()
        };
        self.draw_box(x, y, width, height, &title);

        let content_rows = height.saturating_sub(3).max(1);
        let list_width = (width / 3).clamp(14, 34);
        let detail_x = x + 2 + list_width;
        let detail_width = width.saturating_sub(list_width + 4);
        let border_style = self.theme.border_style();
        for row in 0..content_rows {
            self.screen.put(
                (x + 1 + list_width) as i64,
                (y + 1 + row) as i64,
                '│',
                border_style,
                1,
            );
        }

        if self.plugin_manager.items.is_empty() {
            let message = if self.plugin_manager.filter.is_empty() {
                "No plugins discovered."
            } else {
                "No matching plugins."
            };
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1) as i64,
                message,
                self.theme.muted_style(),
                detail_width,
            );
        } else {
            let last = self.plugin_manager.items.len().saturating_sub(1);
            self.plugin_manager.selected = self.plugin_manager.selected.min(last);
            let max_top = self.plugin_manager.items.len().saturating_sub(content_rows);
            self.plugin_manager.top = self.plugin_manager.top.min(max_top);

            for row in 0..content_rows {
                let index = self.plugin_manager.top + row;
                let Some(plugin) = self.plugin_manager.items.get(index) else {
                    break;
                };
                let label = format!(
                    "{} [{}]",
                    plugin.name,
                    if plugin.enabled { "on" } else { "off" }
                );
                let style = if index == self.plugin_manager.selected {
                    self.theme.selected_item_style()
                } else if plugin.enabled {
                    self.theme.text()
                } else {
                    self.theme.muted_style()
                };
                self.screen.put_str(
                    (x + 2) as i64,
                    (y + 1 + row) as i64,
                    &label,
                    style,
                    list_width.saturating_sub(1),
                );
            }

            if let Some(plugin) = self.plugin_manager_selected_item() {
                let lines = plugin_detail_lines(self, &plugin, detail_width);
                let bottom = y + height - 2;
                for (row, (text, style)) in lines.into_iter().enumerate() {
                    let screen_row = y + 1 + row;
                    if screen_row >= bottom {
                        break;
                    }
                    self.screen.put_str(
                        detail_x as i64,
                        screen_row as i64,
                        &text,
                        style,
                        detail_width,
                    );
                }
            }
        }

        let footer = if self.plugin_manager.filtering {
            "type to filter · ↑↓ move · Enter apply · Esc clear"
        } else {
            "j/k move · Enter toggle · s source · r reload · R all · / search · Esc close"
        };
        self.screen.put_str(
            (x + 2) as i64,
            (y + height - 2) as i64,
            footer,
            self.theme.muted_style(),
            width.saturating_sub(4),
        );
    }
}

fn plugin_detail_lines(editor: &Editor, plugin: &PluginInfo, width: usize) -> Vec<(String, Style)> {
    let text = editor.theme.text();
    let accent = editor.theme.accent_style();
    let muted = editor.theme.muted_style();
    let error = editor.theme.error_style();
    let mut lines = Vec::new();
    lines.push((format!("Name: {}", plugin.name), accent));
    lines.push((format!("Id: {}", plugin.id), text));
    if !plugin.version.is_empty() {
        lines.push((format!("Version: {}", plugin.version), text));
    }
    let status = if plugin.deferred {
        "Status: enabled (loads when its language is opened)"
    } else if plugin.enabled {
        "Status: enabled"
    } else {
        "Status: disabled"
    };
    lines.push((
        status.to_string(),
        if plugin.enabled { text } else { muted },
    ));
    if !plugin.author.is_empty() {
        lines.push((format!("Author: {}", plugin.author), text));
    }
    if !plugin.license.is_empty() {
        lines.push((format!("License: {}", plugin.license), text));
    }
    if !plugin.description.is_empty() {
        push_wrapped(
            &mut lines,
            &format!("Description: {}", plugin.description),
            width,
            text,
        );
    }
    let commands = if plugin.commands.is_empty() {
        "none".to_string()
    } else {
        plugin.commands.join(", ")
    };
    push_wrapped(&mut lines, &format!("Commands: {commands}"), width, text);
    if !plugin.error.is_empty() {
        push_wrapped(
            &mut lines,
            &format!("Error: {}", plugin.error),
            width,
            error,
        );
    }
    if !plugin.entry_path.is_empty() {
        push_wrapped(
            &mut lines,
            &format!("Entry: {}", plugin.entry_path),
            width,
            muted,
        );
    }
    if !plugin.manifest_path.is_empty() {
        push_wrapped(
            &mut lines,
            &format!("Manifest: {}", plugin.manifest_path),
            width,
            muted,
        );
    }
    lines
}

fn push_wrapped(lines: &mut Vec<(String, Style)>, text: &str, width: usize, style: Style) {
    for line in super::command_completion::wrap_words(text, width.max(1)) {
        lines.push((line, style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;

    fn plugin() -> PluginInfo {
        PluginInfo {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1.0".into(),
            description: "A demo plugin.".into(),
            author: "Hexal".into(),
            license: "MIT".into(),
            directory: "/plugins/demo".into(),
            entry_path: "/plugins/demo/init.lua".into(),
            manifest_path: "/plugins/demo/plugin.toml".into(),
            enabled: true,
            running: true,
            deferred: false,
            error: String::new(),
            commands: vec!["demo".into(), "gl".into()],
        }
    }

    #[test]
    fn renders_plugin_list_and_details() {
        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.screen.resize(100, 30);
        editor.plugin_manager.visible = true;
        editor.plugin_manager.items = vec![plugin()];
        editor.plugin_manager.selected = 0;

        editor.render_plugin_manager();
        let screen: String = (0..30)
            .map(|row| {
                (0..100)
                    .map(|column| editor.screen.at(column, row).ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Plugins"), "{screen}");
        assert!(screen.contains("Demo [on]"), "{screen}");
        assert!(screen.contains("Description: A demo plugin."), "{screen}");
        assert!(screen.contains("Commands: demo, gl"), "{screen}");
    }

    fn character(ch: char) -> Event {
        Event {
            key: Key::Char,
            ch,
            ctrl: false,
            alt: false,
            shift: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button: MouseButton::None,
        }
    }

    fn key(key: Key) -> Event {
        Event {
            key,
            ch: '\0',
            ctrl: false,
            alt: false,
            shift: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button: MouseButton::None,
        }
    }

    #[test]
    fn plugin_manager_filter_supports_cursor_movement_and_editing() {
        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.plugin_manager.visible = true;
        editor.plugin_manager.filtering = true;

        for ch in "ac".chars() {
            editor.handle_plugin_manager_filter_input(character(ch));
        }
        assert_eq!(editor.plugin_manager.filter, "ac");

        editor.handle_plugin_manager_filter_input(key(Key::Left));
        editor.handle_plugin_manager_filter_input(character('b'));
        assert_eq!(editor.plugin_manager.filter, "abc");
        assert_eq!(editor.plugin_manager.filter_cursor, 2);

        editor.handle_plugin_manager_filter_input(key(Key::Home));
        editor.handle_plugin_manager_filter_input(key(Key::Delete));
        assert_eq!(editor.plugin_manager.filter, "bc");
        assert_eq!(editor.plugin_manager.filter_cursor, 0);

        editor.handle_plugin_manager_filter_input(key(Key::End));
        editor.handle_plugin_manager_filter_input(key(Key::Backspace));
        assert_eq!(editor.plugin_manager.filter, "b");
        assert_eq!(editor.plugin_manager.filter_cursor, 1);
    }
}
