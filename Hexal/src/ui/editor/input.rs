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
                    'g' | 'G' => {
                        self.for_each_selection(|editor| editor.jump_to_document_start(), false)
                    }
                    'e' | 'E' => {
                        self.for_each_selection(|editor| editor.jump_to_document_end(), false)
                    }
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
                self.for_each_selection(|editor| editor.move_left(), false);
                return;
            }
            Key::Right => {
                self.for_each_selection(|editor| editor.move_right(), false);
                return;
            }
            Key::Up => {
                self.for_each_selection(|editor| editor.move_vertical(-1), false);
                return;
            }
            Key::Down => {
                self.for_each_selection(|editor| editor.move_vertical(1), false);
                return;
            }
            Key::PageUp => {
                self.for_each_selection(|editor| editor.move_page(-1), false);
                return;
            }
            Key::PageDown => {
                self.for_each_selection(|editor| editor.move_page(1), false);
                return;
            }
            Key::Home => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_start(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_start(), false);
                }
                return;
            }
            Key::End => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_end(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_end(), false);
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
                self.for_each_selection(|editor| editor.delete_at_cursor(), true);
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
                self.command_selection = false;
                self.command_line.clear();
                self.command_cursor = 0;
                self.command_completion.close();
                self.doc.break_undo_group();
                self.clear_message();
            }
            '/' => self.open_find(false, false, false),
            '\\' => self.open_find(true, true, false),
            '\'' => self.open_find(false, true, false),
            '.' => self.open_find(true, false, false),
            'i' | 'I' => {
                if event.ch == 'I' {
                    self.for_each_selection(|editor| editor.move_to_line_start(), false);
                }
                self.enter_insert();
            }
            'a' => {
                self.for_each_selection(
                    |editor| {
                        let end = editor.line_end_of_cursor();
                        if editor.cursor < end {
                            let line = editor.cursor_line();
                            let start = editor.doc.line_start(line);
                            let text = editor.doc.line_text(line);
                            let local = (editor.cursor - start).min(text.len());
                            editor.cursor = start + util::next_boundary(&text, local);
                        }
                        editor.enter_insert();
                    },
                    false,
                );
            }
            'A' => {
                self.for_each_selection(
                    |editor| {
                        editor.cursor = editor.line_end_of_cursor();
                        editor.preferred_column = None;
                        editor.enter_insert();
                    },
                    false,
                );
            }
            'o' => self.for_each_selection(|editor| editor.insert_newline_below(), true),
            'O' => self.for_each_selection(|editor| editor.insert_newline_above(), true),
            'h' => self.for_each_selection(|editor| editor.move_left(), false),
            'l' => self.for_each_selection(|editor| editor.move_right(), false),
            'j' => self.for_each_selection(|editor| editor.move_vertical(1), false),
            'k' => self.for_each_selection(|editor| editor.move_vertical(-1), false),
            'w' | 'W' => self.for_each_selection(|editor| editor.move_word_forward(), false),
            'b' | 'B' | 'q' | 'Q' => {
                self.for_each_selection(|editor| editor.move_word_backward(), false)
            }
            'd' | 'D' => self.for_each_selection(|editor| editor.delete_at_cursor(), true),
            'u' => self.undo(),
            'U' => self.redo(),
            'g' | 'G' => self.pending_g = true,
            'v' | 'V' => {
                self.for_each_selection(|editor| editor.start_character_selection(), false)
            }
            'x' | 'X' => self.for_each_selection(|editor| editor.start_line_selection(), false),
            '%' => self.select_all(),
            '<' => self.indent_selection(true),
            '>' => self.indent_selection(false),
            'y' | 'Y' => self.yank_selection(),
            'p' => self.paste_clipboard(false),
            'P' => self.paste_clipboard(true),
            'C' => self.add_cursor_below(),
            ',' => self.keep_primary_selection(),
            'n' | 'N' if self.workspace_search.is_some() => self.workspace_search_step(true),
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
            Key::Left => self.for_each_selection(|editor| editor.move_left(), false),
            Key::Right => self.for_each_selection(|editor| editor.move_right(), false),
            Key::Up => self.for_each_selection(|editor| editor.move_vertical(-1), false),
            Key::Down => self.for_each_selection(|editor| editor.move_vertical(1), false),
            Key::PageUp => self.for_each_selection(|editor| editor.move_page(-1), false),
            Key::PageDown => self.for_each_selection(|editor| editor.move_page(1), false),
            Key::Home => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_start(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_start(), false);
                }
            }
            Key::End => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_end(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_end(), false);
                }
            }
            Key::Enter => self.for_each_selection(|editor| editor.insert_newline(), true),
            Key::Backspace => self.for_each_selection(|editor| editor.backspace(), true),
            Key::Delete => self.for_each_selection(|editor| editor.delete_forward(), true),
            Key::Tab => self.for_each_selection(|editor| editor.insert_indent(), true),
            Key::Char => {
                let character = event.ch;
                self.for_each_selection(|editor| editor.insert_character(character), true);
            }
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
                    'g' | 'G' => {
                        self.for_each_selection(|editor| editor.jump_to_document_start(), false)
                    }
                    'e' | 'E' => {
                        self.for_each_selection(|editor| editor.jump_to_document_end(), false)
                    }
                    'p' | 'P' => self.move_to_previous_location(),
                    'n' | 'N' => self.move_to_next_location(),
                    _ => {}
                }
            }
            return;
        }

        match event.key {
            Key::Left => {
                self.for_each_selection(|editor| editor.move_left(), false);
                return;
            }
            Key::Right => {
                self.for_each_selection(|editor| editor.move_right(), false);
                return;
            }
            Key::Up => {
                self.for_each_selection(|editor| editor.move_vertical(-1), false);
                return;
            }
            Key::Down => {
                self.for_each_selection(|editor| editor.move_vertical(1), false);
                return;
            }
            Key::PageUp => {
                self.for_each_selection(|editor| editor.move_page(-1), false);
                return;
            }
            Key::PageDown => {
                self.for_each_selection(|editor| editor.move_page(1), false);
                return;
            }
            Key::Home => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_start(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_start(), false);
                }
                return;
            }
            Key::End => {
                if event.ctrl {
                    self.for_each_selection(|editor| editor.jump_to_document_end(), false);
                } else {
                    self.for_each_selection(|editor| editor.move_to_line_end(), false);
                }
                return;
            }
            Key::Escape => {
                self.cancel_selection();
                return;
            }
            Key::Delete => {
                self.for_each_selection(|editor| editor.delete_selection(), true);
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
                self.command_selection = true;
                self.command_line.clear();
                self.command_cursor = 0;
                self.command_completion.close();
                self.doc.break_undo_group();
                self.clear_message();
            }
            '%' => self.select_all(),
            '.' => self.open_find(true, false, false),
            '\\' => self.open_find(true, true, false),
            'x' | 'X' => self.for_each_selection(|editor| editor.extend_line_selection(), false),
            'd' | 'D' => self.for_each_selection(|editor| editor.delete_selection(), true),
            '<' => self.indent_selection(true),
            '>' => self.indent_selection(false),
            'h' => self.for_each_selection(|editor| editor.move_left(), false),
            'l' => self.for_each_selection(|editor| editor.move_right(), false),
            'j' => self.for_each_selection(|editor| editor.move_vertical(1), false),
            'k' => self.for_each_selection(|editor| editor.move_vertical(-1), false),
            'w' | 'W' => self.for_each_selection(|editor| editor.move_word_forward(), false),
            'b' | 'B' | 'q' | 'Q' => {
                self.for_each_selection(|editor| editor.move_word_backward(), false)
            }
            'g' | 'G' => self.pending_g = true,
            'i' | 'I' => {
                self.for_each_selection(|editor| editor.enter_insert_from_selection(), true)
            }
            'y' | 'Y' => self.yank_selection(),
            'p' | 'P' => self.paste_clipboard(event.ch == 'P'),
            'C' => self.add_cursor_below(),
            ',' => self.keep_primary_selection(),
            'u' => {
                self.cancel_selection();
                self.undo();
            }
            'U' => {
                self.cancel_selection();
                self.redo();
            }
            'n' | 'N' if self.workspace_search.is_some() => self.workspace_search_step(true),
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
            '.' => self.open_cross_find(false),
            '\\' => self.open_cross_find(true),
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
            'b' | 'B' => self.toggle_breakpoint(),
            'w' | 'W' => {
                self.save_document(None);
            }
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
                self.mode = if self.command_selection {
                    Mode::Select
                } else {
                    Mode::Normal
                };
                self.command_selection = false;
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
        let restore_selection = self.command_selection;
        self.command_selection = false;
        self.mode = if restore_selection {
            Mode::Select
        } else {
            Mode::Normal
        };
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
                if args.is_empty() {
                    self.save_document(None);
                } else {
                    self.save_document(Some(&util::expand_home(&args)));
                }
            }
            "wq" => {
                let saved = if args.is_empty() {
                    self.save_document(None)
                } else {
                    self.save_document(Some(&util::expand_home(&args)))
                };
                if saved {
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
            "rla" => {
                if args.is_empty() {
                    self.reload_all(force);
                } else {
                    self.set_error("usage: :rla[!]");
                }
            }
            "open-log" | "ol" => {
                if args.is_empty() {
                    self.open_log_file();
                } else {
                    self.set_error(&format!("usage: :{command_name}"));
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
                    self.load_lua_plugin(&util::expand_home(&args));
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
                    self.restart_lsp_for_buffer();
                    self.fire_plugin_buffer_event("buffer_opened");
                    self.set_message(&format!("Language: {name}"));
                } else {
                    self.set_error(&format!("unknown language: {args}"));
                }
            }
            "fmt" => self.format_current_document(false),
            "fmta" => self.format_workspace_extension(&args),
            "debug-lsp" | "dl" => self.open_lsp_debug(),
            "debug" | "dbg" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) => self.start_debug_session(&arguments),
                Err(error) => self.set_error(&error),
            },
            "dbg-break" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.toggle_breakpoint(),
                Ok(_) => self.set_error("usage: :dbg-break"),
                Err(error) => self.set_error(&error),
            },
            "dbg-cont" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.continue_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-cont"),
                Err(error) => self.set_error(&error),
            },
            "dbg-next" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.next_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-next"),
                Err(error) => self.set_error(&error),
            },
            "dbg-step" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.step_in_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-step"),
                Err(error) => self.set_error(&error),
            },
            "dbg-out" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.step_out_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-out"),
                Err(error) => self.set_error(&error),
            },
            "dbg-pause" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.pause_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-pause"),
                Err(error) => self.set_error(&error),
            },
            "dbg-stop" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.stop_debug_session(),
                Ok(_) => self.set_error("usage: :dbg-stop"),
                Err(error) => self.set_error(&error),
            },
            "dbg-sv" | "dbg-vars" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.toggle_debug_variables(),
                Ok(_) => self.set_error(&format!("usage: :{command_name}")),
                Err(error) => self.set_error(&error),
            },
            "dbg-watch" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => {
                    self.set_error("usage: :dbg-watch <expression>");
                }
                Ok(arguments) => self.add_debug_watch(&arguments.join(" ")),
                Err(error) => self.set_error(&error),
            },
            "dbg-unwatch" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => {
                    self.set_error("usage: :dbg-unwatch <expression>");
                }
                Ok(arguments) => self.remove_debug_watch(&arguments.join(" ")),
                Err(error) => self.set_error(&error),
            },
            "dbg-clear-watches" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.clear_debug_watches(),
                Ok(_) => self.set_error("usage: :dbg-clear-watches"),
                Err(error) => self.set_error(&error),
            },
            "debug-dap" | "dd" => match Self::parse_debug_command_arguments(rest) {
                Ok(arguments) if arguments.is_empty() => self.open_dap_debug(),
                Ok(_) => self.set_error(&format!("usage: :{command_name}")),
                Err(error) => self.set_error(&error),
            },
            "gi" => self.toggle_git_ignored(),
            "gdl" => self.toggle_diff_lines(),
            "gj" => {
                if args.is_empty() {
                    self.jump_to_diff_line(true);
                } else {
                    self.set_error("usage: :gj");
                }
            }
            "gjp" => {
                if args.is_empty() {
                    self.jump_to_diff_line(false);
                } else {
                    self.set_error("usage: :gjp");
                }
            }
            "ww" => {
                if args.is_empty() {
                    self.toggle_word_wrap();
                } else {
                    self.set_error("usage: :ww");
                }
            }
            "smc" => {
                if args.is_empty() {
                    self.toggle_search_match_case();
                } else {
                    self.set_error("usage: :smc");
                }
            }
            "sidebar" => self.toggle_sidebar(),
            "vs" => {
                if args.is_empty() {
                    self.split_pane(PaneSplitOrientation::Vertical);
                } else {
                    self.set_error("usage: :vs");
                }
            }
            "hs" => {
                if args.is_empty() {
                    self.split_pane(PaneSplitOrientation::Horizontal);
                } else {
                    self.set_error("usage: :hs");
                }
            }
            "cb" => {
                if args.is_empty() {
                    self.close_current_buffer();
                } else {
                    self.set_error("usage: :cb");
                }
            }
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
        if force || !self.any_pane_document_dirty() {
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
                        let record_jump = self.pending_open_records_jump;
                        self.load_file_with_jump(&path, line, column, record_jump);
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
        self.pending_open_records_jump = true;
    }

    pub(super) fn open_goto_prompt(&mut self) {
        self.goto_prompt = Some(String::new());
        self.goto_cursor = 0;
        self.doc.break_undo_group();
        self.clear_message();
    }

    pub(super) fn handle_goto_prompt(&mut self, event: Event) {
        match event.key {
            Key::Escape => {
                self.goto_prompt = None;
                self.goto_cursor = 0;
                self.set_message("Cancelled.");
            }
            Key::Backspace => {
                if let Some(input) = self.goto_prompt.as_mut()
                    && self.goto_cursor > 0
                {
                    let previous = util::previous_boundary(input.as_bytes(), self.goto_cursor);
                    input.replace_range(previous..self.goto_cursor, "");
                    self.goto_cursor = previous;
                }
            }
            Key::Delete => {
                if let Some(input) = self.goto_prompt.as_mut()
                    && self.goto_cursor < input.len()
                {
                    let next = util::next_boundary(input.as_bytes(), self.goto_cursor);
                    input.replace_range(self.goto_cursor..next, "");
                }
            }
            Key::Left => {
                if let Some(input) = self.goto_prompt.as_ref() {
                    self.goto_cursor = util::previous_boundary(input.as_bytes(), self.goto_cursor);
                }
            }
            Key::Right => {
                if let Some(input) = self.goto_prompt.as_ref() {
                    self.goto_cursor = util::next_boundary(input.as_bytes(), self.goto_cursor);
                }
            }
            Key::Home => self.goto_cursor = 0,
            Key::End => {
                self.goto_cursor = self.goto_prompt.as_ref().map_or(0, String::len);
            }
            Key::Enter => {
                let input = self.goto_prompt.take().unwrap_or_default();
                self.goto_cursor = 0;
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
                self.top_line = line.saturating_sub(self.active_pane_rows() / 2);
                self.snap_cursor();
                self.set_message(&format!("Line {}", line + 1));
            }
            Key::Char => {
                if event.ctrl || event.alt {
                    return;
                }
                if event.ch.is_ascii_digit()
                    && let Some(input) = self.goto_prompt.as_mut()
                {
                    let cursor = self.goto_cursor.min(input.len());
                    input.insert(cursor, event.ch);
                    self.goto_cursor = cursor + event.ch.len_utf8();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let mut editor = Editor::new(Document::new(), 80, 24);
        let directory =
            std::env::temp_dir().join(format!("hexal-command-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        editor.config.source_path = directory.join(".hexcfg");
        editor
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
    fn goto_prompt_supports_cursor_movement_and_editing() {
        let mut editor = editor();
        editor.open_goto_prompt();
        for ch in "12".chars() {
            editor.handle_goto_prompt(character(ch));
        }
        assert_eq!(editor.goto_prompt.as_deref(), Some("12"));
        assert_eq!(editor.goto_cursor, 2);

        editor.handle_goto_prompt(key(Key::Left));
        editor.handle_goto_prompt(character('3'));
        assert_eq!(editor.goto_prompt.as_deref(), Some("132"));
        assert_eq!(editor.goto_cursor, 2);

        editor.handle_goto_prompt(key(Key::Home));
        editor.handle_goto_prompt(key(Key::Delete));
        assert_eq!(editor.goto_prompt.as_deref(), Some("32"));
        assert_eq!(editor.goto_cursor, 0);

        editor.handle_goto_prompt(key(Key::End));
        editor.handle_goto_prompt(key(Key::Backspace));
        assert_eq!(editor.goto_prompt.as_deref(), Some("3"));
        assert_eq!(editor.goto_cursor, 1);
    }

    #[test]
    fn search_keys_pick_the_expected_find_modes() {
        let mut single_regex = editor();
        single_regex.handle_normal(character('\''));
        assert!(single_regex.find.active);
        assert!(single_regex.find.regex);
        assert!(!single_regex.find.multi_cursor);

        let mut multi_regex = editor();
        multi_regex.handle_normal(character('\\'));
        assert!(multi_regex.find.active);
        assert!(multi_regex.find.regex);
        assert!(multi_regex.find.multi_cursor);

        let mut multi_literal = editor();
        multi_literal.handle_normal(character('.'));
        assert!(multi_literal.find.active);
        assert!(!multi_literal.find.regex);
        assert!(multi_literal.find.multi_cursor);
    }

    #[test]
    fn write_and_quit_resolves_paths_against_the_opened_folder() {
        let directory =
            std::env::temp_dir().join(format!("hexal-write-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let name = format!("created-{}.txt", std::process::id());

        let mut editor =
            Editor::with_options(Document::new(), 80, 24, Some(directory.clone()), false);
        editor.doc.insert(0, b"hello\n");
        editor.command_line = format!("wq {name}");
        editor.submit_command();

        assert!(!editor.running);
        assert_eq!(
            std::fs::read(directory.join(&name)).unwrap(),
            b"hello\n".to_vec()
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn word_wrap_command_toggles_the_setting() {
        let mut editor = editor();
        let before = editor.word_wrap;
        editor.command_line = "ww".into();
        editor.submit_command();
        assert_ne!(editor.word_wrap, before);
        assert!(!editor.message.is_empty());
    }

    #[test]
    fn git_diff_and_log_commands_are_recognized() {
        let mut editor = editor();
        for command in ["gj", "gjp"] {
            editor.command_line = command.into();
            editor.submit_command();
            assert!(
                !editor.message.contains("unknown command"),
                "{command}: {}",
                editor.message
            );
        }
        editor.command_line = "open-log --bad".into();
        editor.submit_command();
        assert!(editor.message.contains("usage: :open-log"));
    }
}
