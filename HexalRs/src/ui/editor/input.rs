//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn handle_normal(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            self.pending_g = false;
            match event.ch {
                'r' => self.redo(),
                'c' => self.toggle_comment(),
                's' => {
                    self.save_document(None);
                }
                'q' => self.request_quit(false),
                'b' => self.toggle_sidebar(),
                _ => {}
            }
            return;
        }
        if event.key == Key::Char && event.alt {
            self.pending_g = false;
            return;
        }

        if self.pending_g {
            self.pending_g = false;
            if event.key == Key::Char {
                match event.ch {
                    'g' | 'G' => self.jump_to_document_start(),
                    'e' | 'E' => self.jump_to_document_end(),
                    'p' | 'P' => self.move_to_previous_location(),
                    'n' | 'N' => self.move_to_next_location(),
                    'd' | 'D' => self.go_to_definition(),
                    _ => {}
                }
            }
            return;
        }

        match event.key {
            Key::Left => {
                self.move_left();
                return;
            }
            Key::Right => {
                self.move_right();
                return;
            }
            Key::Up => {
                self.move_vertical(-1);
                return;
            }
            Key::Down => {
                self.move_vertical(1);
                return;
            }
            Key::PageUp => {
                self.move_page(-1);
                return;
            }
            Key::PageDown => {
                self.move_page(1);
                return;
            }
            Key::Home => {
                if event.ctrl {
                    self.jump_to_document_start();
                } else {
                    self.move_to_line_start();
                }
                return;
            }
            Key::End => {
                if event.ctrl {
                    self.jump_to_document_end();
                } else {
                    self.move_to_line_end();
                }
                return;
            }
            Key::Escape => {
                self.doc.break_undo_group();
                self.pending_g = false;
                self.clear_message();
                return;
            }
            Key::Delete => {
                self.delete_at_cursor();
                return;
            }
            _ => {}
        }

        if event.key != Key::Char {
            return;
        }

        match event.ch {
            ' ' => self.open_space_menu(),
            ':' => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_cursor = 0;
                self.command_completion.close();
                self.doc.break_undo_group();
                self.clear_message();
            }
            '/' => self.open_find(),
            'i' | 'I' => {
                if event.ch == 'I' {
                    self.move_to_line_start();
                }
                self.enter_insert();
            }
            'a' => {
                let end = self.line_end_of_cursor();
                if self.cursor < end {
                    self.cursor = self.next_scalar(self.cursor);
                }
                self.enter_insert();
            }
            'A' => {
                self.cursor = self.line_end_of_cursor();
                self.preferred_column = None;
                self.enter_insert();
            }
            'o' => self.insert_newline_below(),
            'O' => self.insert_newline_above(),
            'h' => self.move_left(),
            'l' => self.move_right(),
            'j' => self.move_vertical(1),
            'k' => self.move_vertical(-1),
            'w' | 'W' => self.move_word_forward(),
            'b' | 'B' | 'q' | 'Q' => self.move_word_backward(),
            'd' | 'D' => self.delete_at_cursor(),
            'u' => self.undo(),
            'U' => self.redo(),
            'g' | 'G' => self.pending_g = true,
            'v' | 'V' => self.start_character_selection(),
            'x' | 'X' => self.start_line_selection(),
            '%' => self.select_all(),
            '<' => self.indent_selection(true),
            '>' => self.indent_selection(false),
            'y' | 'Y' => self.yank_selection(),
            'p' => self.paste_clipboard(false),
            'P' => self.paste_clipboard(true),
            'C' => self.set_message("Multi-cursor is not implemented in this prototype."),
            ',' => self.set_message("Only the primary cursor is active."),
            _ => {}
        }
    }

    pub(super) fn handle_insert(&mut self, event: Event) {
        if self.completion.visible && self.handle_completion_input(&event) {
            return;
        }
        if event.key == Key::Escape {
            self.mode = Mode::Normal;
            self.pending_g = false;
            self.doc.break_undo_group();
            return;
        }

        if event.key == Key::Char && event.ctrl {
            match event.ch {
                'r' => self.redo(),
                's' => {
                    self.save_document(None);
                }
                ' ' => self.request_lsp_completion(),
                _ => {}
            }
            return;
        }
        if event.key == Key::Char && event.alt {
            return;
        }

        match event.key {
            Key::Left => self.move_left(),
            Key::Right => self.move_right(),
            Key::Up => self.move_vertical(-1),
            Key::Down => self.move_vertical(1),
            Key::PageUp => self.move_page(-1),
            Key::PageDown => self.move_page(1),
            Key::Home => {
                if event.ctrl {
                    self.jump_to_document_start();
                } else {
                    self.move_to_line_start();
                }
            }
            Key::End => {
                if event.ctrl {
                    self.jump_to_document_end();
                } else {
                    self.move_to_line_end();
                }
            }
            Key::Enter => self.insert_newline(),
            Key::Backspace => self.backspace(),
            Key::Delete => self.delete_forward(),
            Key::Tab => self.insert_indent(),
            Key::Char => self.insert_character(event.ch),
            _ => {}
        }
    }

    pub(super) fn handle_select(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            match event.ch {
                'r' => {
                    self.cancel_selection();
                    self.redo();
                }
                'c' => self.toggle_comment(),
                's' => {
                    self.save_document(None);
                }
                _ => {}
            }
            return;
        }
        if event.key == Key::Char && event.alt {
            return;
        }

        if self.pending_g {
            self.pending_g = false;
            if event.key == Key::Char {
                match event.ch {
                    'g' | 'G' => self.jump_to_document_start(),
                    'e' | 'E' => self.jump_to_document_end(),
                    'p' | 'P' => self.move_to_previous_location(),
                    _ => {}
                }
            }
            return;
        }

        match event.key {
            Key::Left => {
                self.move_left();
                return;
            }
            Key::Right => {
                self.move_right();
                return;
            }
            Key::Up => {
                self.move_vertical(-1);
                return;
            }
            Key::Down => {
                self.move_vertical(1);
                return;
            }
            Key::PageUp => {
                self.move_page(-1);
                return;
            }
            Key::PageDown => {
                self.move_page(1);
                return;
            }
            Key::Home => {
                if event.ctrl {
                    self.jump_to_document_start();
                } else {
                    self.move_to_line_start();
                }
                return;
            }
            Key::End => {
                if event.ctrl {
                    self.jump_to_document_end();
                } else {
                    self.move_to_line_end();
                }
                return;
            }
            Key::Escape => {
                self.cancel_selection();
                return;
            }
            Key::Delete => {
                self.delete_selection();
                return;
            }
            _ => {}
        }

        if event.key != Key::Char {
            return;
        }

        match event.ch {
            ' ' => self.open_space_menu(),
            ':' => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_cursor = 0;
                self.command_completion.close();
                self.doc.break_undo_group();
                self.clear_message();
            }
            '%' => self.select_all(),
            'x' | 'X' => self.extend_line_selection(),
            'd' | 'D' => self.delete_selection(),
            '<' => self.indent_selection(true),
            '>' => self.indent_selection(false),
            'h' => self.move_left(),
            'l' => self.move_right(),
            'j' => self.move_vertical(1),
            'k' => self.move_vertical(-1),
            'w' | 'W' => self.move_word_forward(),
            'b' | 'B' | 'q' | 'Q' => self.move_word_backward(),
            'g' | 'G' => self.pending_g = true,
            'i' | 'I' => self.enter_insert_from_selection(),
            'y' | 'Y' => self.yank_selection(),
            'p' | 'P' => self.paste_clipboard(event.ch == 'P'),
            '.' => self.set_message("Multi-cursor find is not implemented in this prototype."),
            _ => {}
        }
    }

    // ---------------------------------------------------------------------
    // SPACE menu
    // ---------------------------------------------------------------------

    pub(super) fn open_space_menu(&mut self) {
        self.space_menu_open = true;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn handle_space_menu(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.space_menu_open = false;
            return;
        }
        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }

        self.space_menu_open = false;
        match event.ch {
            '/' => self.open_picker(PickerKind::Search),
            'f' | 'F' => self.open_picker(PickerKind::Files),
            'g' | 'G' => self.open_goto_prompt(),
            'y' | 'Y' => self.yank_selection(),
            'x' | 'X' => self.cut_selection(),
            'p' => self.paste_clipboard(false),
            'P' => self.paste_clipboard(true),
            'K' | 'k' => self.show_hover(),
            'A' | 'a' => self.show_code_actions(),
            'S' | 's' => self.show_document_symbols(),
            'D' => self.show_problems(true),
            'd' => self.show_problems(false),
            'z' | 'Z' => self.show_line_diagnostic(),
            'w' | 'W' => {
                self.save_document(None);
            }
            '?' => self.set_message(HELP_TEXT),
            'q' | 'Q' => self.request_quit(false),
            _ => {}
        }
    }

    // ---------------------------------------------------------------------
    // Command line
    // ---------------------------------------------------------------------

    pub(super) fn handle_command_input(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            return;
        }
        match event.key {
            Key::Escape => {
                if self.command_completion.visible {
                    self.command_completion.close();
                    return;
                }
                self.mode = Mode::Normal;
                self.command_line.clear();
                self.command_cursor = 0;
            }
            Key::Enter => {
                if self.command_completion.visible && self.accept_command_completion() {
                    return;
                }
                self.submit_command();
            }
            Key::Tab => {
                if self.command_completion.visible {
                    self.move_command_completion(1, true);
                } else {
                    self.try_open_command_completion();
                }
            }
            Key::BackTab => {
                if self.command_completion.visible {
                    self.move_command_completion(-1, true);
                } else {
                    self.try_open_command_completion();
                }
            }
            Key::Backspace => {
                self.command_completion.close();
                if self.command_cursor > 0 {
                    let previous =
                        util::previous_boundary(self.command_line.as_bytes(), self.command_cursor);
                    self.command_line
                        .replace_range(previous..self.command_cursor, "");
                    self.command_cursor = previous;
                }
            }
            Key::Delete => {
                self.command_completion.close();
                if self.command_cursor < self.command_line.len() {
                    let next =
                        util::next_boundary(self.command_line.as_bytes(), self.command_cursor);
                    self.command_line
                        .replace_range(self.command_cursor..next, "");
                }
            }
            Key::Left => {
                self.command_completion.close();
                self.command_cursor =
                    util::previous_boundary(self.command_line.as_bytes(), self.command_cursor);
            }
            Key::Right => {
                self.command_completion.close();
                self.command_cursor =
                    util::next_boundary(self.command_line.as_bytes(), self.command_cursor);
            }
            Key::Home => {
                self.command_completion.close();
                self.command_cursor = 0;
            }
            Key::End => {
                self.command_completion.close();
                self.command_cursor = self.command_line.len();
            }
            Key::Up => {
                if self.command_completion.visible {
                    self.move_command_completion(-1, false);
                }
            }
            Key::Down => {
                if self.command_completion.visible {
                    self.move_command_completion(1, false);
                }
            }
            Key::Char => {
                if event.alt {
                    return;
                }
                if event.ch == '\0' {
                    return;
                }
                self.command_line.insert(self.command_cursor, event.ch);
                self.command_cursor += event.ch.len_utf8();
                self.refresh_command_completion();
            }
            _ => {}
        }
    }

    pub(super) fn submit_command(&mut self) {
        self.command_completion.close();
        let line = self.command_line.trim().to_string();
        self.mode = Mode::Normal;
        self.command_line.clear();
        self.command_cursor = 0;
        if line.is_empty() {
            return;
        }

        let (mut name, rest) = match line.split_once(|c: char| c == ' ' || c == '\t') {
            Some((name, rest)) => (name.to_string(), rest.trim()),
            None => (line.clone(), ""),
        };
        let mut force = false;
        if name.ends_with('!') {
            force = true;
            name.pop();
        }
        let args = strip_quotes(rest);
        let command_name = name.to_lowercase();
        self.doc.break_undo_group();

        match command_name.as_str() {
            "w" | "write" => {
                let path = if args.is_empty() {
                    None
                } else {
                    Some(args.as_str())
                };
                self.save_document(path);
            }
            "wq" => {
                let path = if args.is_empty() {
                    None
                } else {
                    Some(args.as_str())
                };
                if self.save_document(path) {
                    self.running = false;
                }
            }
            "q" | "quit" => self.request_quit(force),
            "e" | "edit" => {
                if args.is_empty() {
                    self.set_error("usage: :e <filename>");
                } else {
                    self.request_open(&args, 0, 0, force);
                }
            }
            "path" => match &self.doc.path {
                Some(path) => {
                    let absolute = std::fs::canonicalize(path)
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| path.clone());
                    self.set_message(&absolute);
                }
                None => self.set_message("[No Name]"),
            },
            "help" => {
                self.help.open();
                self.force_full_redraw = true;
                self.clear_message();
            }
            "theme" | "themes" => {
                if args.is_empty() {
                    self.open_picker(PickerKind::Themes);
                } else {
                    self.apply_theme(&args, true);
                }
            }
            "cfg" => {
                if args.is_empty() {
                    self.open_config_file();
                } else {
                    self.set_error("usage: :cfg");
                }
            }
            "rcfg" => {
                if args.is_empty() {
                    self.reload_configuration();
                } else {
                    self.set_error("usage: :rcfg");
                }
            }
            "a" | "about" => {
                if args.is_empty() {
                    self.open_about();
                } else {
                    self.set_error("usage: :about");
                }
            }
            "cw" => self.trim_trailing_whitespace(),
            "rc" => {
                if args.is_empty() {
                    self.remove_comments_from_selection();
                } else {
                    self.set_error("usage: :rc");
                }
            }
            "lua" => {
                if args.is_empty() {
                    self.set_error("usage: :lua <plugin.lua>");
                } else {
                    self.load_lua_plugin(&args);
                }
            }
            "plugins" => self.list_lua_commands(),
            "lang" => {
                if args.is_empty() {
                    self.set_error("usage: :lang <language>");
                } else if let Some(language) = self.languages.find(&args) {
                    let name = language.name.clone();
                    self.language_override = Some(args.clone());
                    self.refresh_language();
                    self.lsp.stop();
                    self.lsp_fallback_blocked = false;
                    self.restart_lsp_for_buffer();
                    self.set_message(&format!("Language: {name}"));
                } else {
                    self.set_error(&format!("unknown language: {args}"));
                }
            }
            "fmt" => self.format_current_document(false),
            "fmta" => self.format_workspace_extension(&args),
            "debug-lsp" | "dl" => self.open_lsp_debug(),
            "gi" => self.toggle_git_ignored(),
            "gdl" => self.toggle_diff_lines(),
            "smc" => {
                if args.is_empty() {
                    self.toggle_search_match_case();
                } else {
                    self.set_error("usage: :smc");
                }
            }
            "sidebar" => self.toggle_sidebar(),
            "adi" => {
                if args.is_empty() {
                    self.auto_detect_indentation(true);
                } else {
                    self.set_error("usage: :adi");
                }
            }
            _ => {
                if self.try_lua_command(&name) {
                    return;
                }
                self.set_error(&format!("unknown command: {name}"));
            }
        }
    }

    pub(super) fn request_quit(&mut self, force: bool) {
        if force || !self.doc.is_dirty() {
            self.running = false;
        } else if !self.config.confirm_quit_without_saving {
            self.set_error("Unsaved changes; use :q! to quit without saving.");
        } else {
            self.pending = Pending::Quit;
        }
    }

    // ---------------------------------------------------------------------
    // Confirmations and prompts
    // ---------------------------------------------------------------------

    pub(super) fn handle_confirm(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.cancel_pending();
            self.set_message("Cancelled.");
            return;
        }
        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }
        if event.ch == 'y' || event.ch == 'Y' {
            let action = self.pending;
            self.pending = Pending::None;
            match action {
                Pending::Quit => self.running = false,
                Pending::Open => {
                    if let Some(path) = self.pending_path.take() {
                        let line = self.pending_line;
                        let column = self.pending_column;
                        self.load_file(&path, line, column);
                    }
                }
                Pending::SidebarDelete => self.delete_sidebar_paths(),
                Pending::SidebarDiff => {
                    if let Some((diff, summary)) = self.pending_sidebar_diff.take() {
                        self.show_sidebar_diff(&diff, &summary);
                    }
                }
                Pending::None => {}
            }
        } else if event.ch == 'n' || event.ch == 'N' {
            self.cancel_pending();
            self.set_message("Cancelled.");
        }
    }

    pub(super) fn cancel_pending(&mut self) {
        self.pending = Pending::None;
        self.pending_path = None;
    }

    pub(super) fn open_goto_prompt(&mut self) {
        self.goto_prompt = Some(String::new());
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn handle_goto_prompt(&mut self, event: Event) {
        match event.key {
            Key::Escape => {
                self.goto_prompt = None;
                self.set_message("Cancelled.");
            }
            Key::Backspace => {
                if let Some(input) = self.goto_prompt.as_mut() {
                    input.pop();
                }
            }
            Key::Enter => {
                let input = self.goto_prompt.take().unwrap_or_default();
                let Ok(number) = input.parse::<usize>() else {
                    self.set_error("go to line: enter a line number");
                    return;
                };
                if number == 0 {
                    self.set_error("go to line: line numbers start at 1");
                    return;
                }
                self.record_jump();
                let line = (number - 1).min(self.doc.line_count() - 1);
                self.cursor = self.doc.line_start(line);
                self.anchor = self.cursor;
                self.preferred_column = None;
                self.mode = Mode::Normal;
                self.top_line = line.saturating_sub(self.text_rows() / 2);
                self.snap_cursor();
                self.set_message(&format!("Line {line}"));
            }
            Key::Char => {
                if event.ctrl || event.alt {
                    return;
                }
                if event.ch.is_ascii_digit() {
                    if let Some(input) = self.goto_prompt.as_mut() {
                        input.push(event.ch);
                    }
                }
            }
            _ => {}
        }
    }
}

fn strip_quotes(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
}
