//! Modal editor state, key handling, Ex commands, pickers and rendering.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::io;
use std::path::Path;

use crate::document::Document;
use crate::files;
use crate::lua::LuaRuntime;
use crate::renderer::Renderer;
use crate::screen::Screen;
use crate::syntax::{self, Language};
use crate::terminal::{Event, Key, Terminal};
use crate::theme::{SyntaxKind, Theme};
use crate::util;

const TAB_WIDTH: usize = 4;
const INDENT_WIDTH: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
    Select,
    Command,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Select => "SELECT",
            Mode::Command => "COMMAND",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    Quit,
    Open,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    None,
    Files,
    Search,
}

struct PickerItem {
    display: String,
    file_index: usize,
    line: usize,
    column: usize,
}

struct Picker {
    kind: PickerKind,
    query: String,
    files: Vec<String>,
    files_loaded: bool,
    items: Vec<PickerItem>,
    selected: usize,
    top: usize,
    total: usize,
}

impl Picker {
    fn new() -> Picker {
        Picker {
            kind: PickerKind::None,
            query: String::new(),
            files: Vec::new(),
            files_loaded: false,
            items: Vec::new(),
            selected: 0,
            top: 0,
            total: 0,
        }
    }
}

struct FindState {
    active: bool,
    editing: bool,
    query: String,
    matches: Vec<(usize, usize)>,
    index: Option<usize>,
    origin: usize,
}

impl FindState {
    fn new() -> FindState {
        FindState {
            active: false,
            editing: false,
            query: String::new(),
            matches: Vec::new(),
            index: None,
            origin: 0,
        }
    }
}

struct SpaceItem {
    key: char,
    label: &'static str,
}

const SPACE_ITEMS: &[SpaceItem] = &[
    SpaceItem {
        key: '/',
        label: "Search across files",
    },
    SpaceItem {
        key: 'f',
        label: "Open file picker",
    },
    SpaceItem {
        key: 'g',
        label: "Go to line",
    },
    SpaceItem {
        key: 'y',
        label: "Yank selection",
    },
    SpaceItem {
        key: 'X',
        label: "Cut selection",
    },
    SpaceItem {
        key: 'p',
        label: "Paste on current line",
    },
    SpaceItem {
        key: 'P',
        label: "Paste on new line",
    },
    SpaceItem {
        key: 'w',
        label: "Save buffer",
    },
    SpaceItem {
        key: '?',
        label: "Key reference",
    },
    SpaceItem {
        key: 'q',
        label: "Quit",
    },
];

const HELP_TEXT: &str = "i/a/o insert  v select  x line select  d delete  u/U undo/redo  gg/ge/gp  :w :wq :q :e  / find  :lua <file>  Space menu";

pub struct Editor {
    doc: Document,
    screen: Screen,
    renderer: Renderer,
    theme: Theme,

    mode: Mode,
    running: bool,
    cols: usize,
    rows: usize,

    cursor: usize,
    anchor: usize,
    selection_linewise: bool,
    preferred_column: Option<usize>,
    top_line: usize,
    left_column: usize,

    message: String,
    message_is_error: bool,
    command_line: String,
    command_cursor: usize,
    clipboard: String,
    pending_osc: Option<Vec<u8>>,

    pending_g: bool,
    space_menu_open: bool,
    pending: Pending,
    pending_path: Option<String>,
    pending_line: usize,
    pending_column: usize,
    goto_prompt: Option<String>,

    language_override: Option<String>,
    jumps: Vec<usize>,
    jump_index: isize,

    picker: Picker,
    find: FindState,
    lua: Option<LuaRuntime>,
}

impl Editor {
    pub fn new(doc: Document, cols: usize, rows: usize) -> Editor {
        let message = if doc.path.is_some() {
            format!("Opened {}", doc.display_name())
        } else {
            String::new()
        };
        let mut editor = Editor {
            doc,
            screen: Screen::new(),
            renderer: Renderer::new(),
            theme: Theme::default(),
            mode: Mode::Normal,
            running: true,
            cols,
            rows,
            cursor: 0,
            anchor: 0,
            selection_linewise: false,
            preferred_column: None,
            top_line: 0,
            left_column: 0,
            message,
            message_is_error: false,
            command_line: String::new(),
            command_cursor: 0,
            clipboard: String::new(),
            pending_osc: None,
            pending_g: false,
            space_menu_open: false,
            pending: Pending::None,
            pending_path: None,
            pending_line: 0,
            pending_column: 0,
            goto_prompt: None,
            language_override: None,
            jumps: Vec::new(),
            jump_index: -1,
            picker: Picker::new(),
            find: FindState::new(),
            lua: None,
        };
        editor.load_plugins();
        editor
    }

    pub fn run(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        let mut last_width = terminal.width;
        let mut last_height = terminal.height;

        // Paint before polling input. A slow event wait must never delay the
        // first frame, which is also what the startup benchmark measures.
        self.render(terminal)?;

        while self.running {
            terminal.update_size();
            if terminal.width != last_width || terminal.height != last_height {
                last_width = terminal.width;
                last_height = terminal.height;
                self.cols = last_width;
                self.rows = last_height;
                self.screen.resize(self.cols, self.rows);
                self.clamp_viewport();
            }

            match terminal.read_event(30) {
                Ok(Some(event)) => {
                    self.handle_event(event);
                    if let Some(sequence) = self.pending_osc.take() {
                        let _ = terminal.write(&sequence);
                    }
                }
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error),
            }

            terminal.update_size();
            if terminal.width != self.cols || terminal.height != self.rows {
                self.cols = terminal.width;
                self.rows = terminal.height;
                last_width = self.cols;
                last_height = self.rows;
                self.screen.resize(self.cols, self.rows);
                self.clamp_viewport();
            }

            self.render(terminal)?;
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        if self.picker.kind != PickerKind::None {
            self.handle_picker(event);
            return;
        }
        if self.find.active {
            self.handle_find(event);
            return;
        }
        if self.space_menu_open {
            self.handle_space_menu(event);
            return;
        }
        if self.goto_prompt.is_some() {
            self.handle_goto_prompt(event);
            return;
        }
        if self.pending != Pending::None {
            self.handle_confirm(event);
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(event),
            Mode::Insert => self.handle_insert(event),
            Mode::Select => self.handle_select(event),
            Mode::Command => self.handle_command_input(event),
        }
    }

    // ---------------------------------------------------------------------
    // Modes
    // ---------------------------------------------------------------------

    fn handle_normal(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            self.pending_g = false;
            match event.ch {
                'r' => self.redo(),
                's' => {
                    self.save_document(None);
                }
                'q' => self.request_quit(false),
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
            Key::Backspace => {
                self.backspace();
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

    fn handle_insert(&mut self, event: Event) {
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
            Key::Tab => self.insert_text("    "),
            Key::Char => {
                let mut buffer = [0u8; 4];
                self.insert_text(event.ch.encode_utf8(&mut buffer));
            }
            _ => {}
        }
    }

    fn handle_select(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            match event.ch {
                'r' => {
                    self.cancel_selection();
                    self.redo();
                }
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

    fn open_space_menu(&mut self) {
        self.space_menu_open = true;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn handle_space_menu(&mut self, event: Event) {
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

    fn handle_command_input(&mut self, event: Event) {
        if event.key == Key::Char && event.ctrl {
            return;
        }
        match event.key {
            Key::Escape => {
                self.mode = Mode::Normal;
                self.command_line.clear();
                self.command_cursor = 0;
            }
            Key::Enter => self.submit_command(),
            Key::Backspace => {
                if self.command_cursor > 0 {
                    let previous =
                        util::previous_boundary(self.command_line.as_bytes(), self.command_cursor);
                    self.command_line
                        .replace_range(previous..self.command_cursor, "");
                    self.command_cursor = previous;
                }
            }
            Key::Delete => {
                if self.command_cursor < self.command_line.len() {
                    let next =
                        util::next_boundary(self.command_line.as_bytes(), self.command_cursor);
                    self.command_line
                        .replace_range(self.command_cursor..next, "");
                }
            }
            Key::Left => {
                self.command_cursor =
                    util::previous_boundary(self.command_line.as_bytes(), self.command_cursor);
            }
            Key::Right => {
                self.command_cursor =
                    util::next_boundary(self.command_line.as_bytes(), self.command_cursor);
            }
            Key::Home => self.command_cursor = 0,
            Key::End => self.command_cursor = self.command_line.len(),
            Key::Char => {
                if event.alt {
                    return;
                }
                if event.ch == '\0' {
                    return;
                }
                self.command_line.insert(self.command_cursor, event.ch);
                self.command_cursor += event.ch.len_utf8();
            }
            _ => {}
        }
    }

    fn submit_command(&mut self) {
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
        self.doc.break_undo_group();

        match name.as_str() {
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
            "help" => self.set_message(HELP_TEXT),
            "about" => self.set_message("HexalRs 0.1.0 - Rust 2024 prototype of the Hexal editor."),
            "cw" => self.trim_trailing_whitespace(),
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
                } else if let Some(language) = syntax::from_id(&args) {
                    self.language_override = Some(args.clone());
                    self.set_message(&format!("Language: {}", language.name));
                } else {
                    self.set_error(&format!("unknown language: {args}"));
                }
            }
            "fmt" => self.set_error("no formatter is configured in this prototype"),
            _ => {
                if self.try_lua_command(&name) {
                    return;
                }
                self.set_error(&format!("unknown command: {name}"));
            }
        }
    }

    fn request_quit(&mut self, force: bool) {
        if force || !self.doc.is_dirty() {
            self.running = false;
        } else {
            self.pending = Pending::Quit;
        }
    }

    // ---------------------------------------------------------------------
    // Confirmations and prompts
    // ---------------------------------------------------------------------

    fn handle_confirm(&mut self, event: Event) {
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
                Pending::None => {}
            }
        } else if event.ch == 'n' || event.ch == 'N' {
            self.cancel_pending();
            self.set_message("Cancelled.");
        }
    }

    fn cancel_pending(&mut self) {
        self.pending = Pending::None;
        self.pending_path = None;
    }

    fn open_goto_prompt(&mut self) {
        self.goto_prompt = Some(String::new());
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn handle_goto_prompt(&mut self, event: Event) {
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

    // ---------------------------------------------------------------------
    // Find in file
    // ---------------------------------------------------------------------

    fn open_find(&mut self) {
        self.find.active = true;
        self.find.editing = true;
        self.find.query.clear();
        self.find.matches.clear();
        self.find.index = None;
        self.find.origin = self.cursor;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn close_find(&mut self) {
        self.find.active = false;
        self.find.editing = false;
        self.find.query.clear();
        self.find.matches.clear();
        self.find.index = None;
        self.doc.break_undo_group();
    }

    fn rebuild_find_matches(&mut self) {
        self.find.matches.clear();
        if self.find.query.is_empty() {
            self.find.index = None;
            return;
        }
        let data = self.doc.all_text();
        let needle = self.find.query.as_bytes();
        let mut search_from = 0usize;
        while search_from < data.len() && self.find.matches.len() < 20_000 {
            let Some(relative) = util::index_of_ignore_case(&data[search_from..], needle) else {
                break;
            };
            let start = search_from + relative;
            let end = start + needle.len();
            self.find.matches.push((start, end));
            search_from = if end > start { end } else { start + 1 };
        }

        if self.find.matches.is_empty() {
            self.find.index = None;
            return;
        }
        let index = self
            .find
            .matches
            .iter()
            .position(|&(start, _)| start >= self.find.origin)
            .unwrap_or(0);
        self.find.index = Some(index);
    }

    fn handle_find(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.close_find();
            return;
        }

        if !self.find.editing {
            match event.key {
                Key::Enter => self.find.editing = true,
                Key::Up => self.find_step(-1),
                Key::Down => self.find_step(1),
                Key::Char => match event.ch {
                    'n' | 'N' => self.find_step(1),
                    'p' | 'P' => self.find_step(-1),
                    '/' => self.find.editing = true,
                    _ => {}
                },
                _ => {}
            }
            return;
        }

        match event.key {
            Key::Enter => {
                self.find.editing = false;
                if self.find.index.is_some() {
                    self.jump_to_find_match();
                }
            }
            Key::Up => self.find_step(-1),
            Key::Down => self.find_step(1),
            Key::Backspace => {
                self.find.query.pop();
                self.rebuild_find_matches();
            }
            Key::Char => {
                if event.ctrl || event.alt || event.ch == '\0' {
                    return;
                }
                self.find.query.push(event.ch);
                self.rebuild_find_matches();
            }
            _ => {}
        }
    }

    fn find_step(&mut self, delta: isize) {
        if self.find.matches.is_empty() {
            return;
        }
        let length = self.find.matches.len() as isize;
        let current = self.find.index.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(length) as usize;
        self.find.index = Some(next);
        self.jump_to_find_match();
    }

    fn jump_to_find_match(&mut self) {
        let Some(index) = self.find.index else {
            return;
        };
        let Some(&(start, _)) = self.find.matches.get(index) else {
            return;
        };
        self.cursor = start;
        self.anchor = start;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.doc.break_undo_group();
    }

    // ---------------------------------------------------------------------
    // File opening and pickers
    // ---------------------------------------------------------------------

    fn request_open(&mut self, raw_path: &str, line: usize, column: usize, force: bool) {
        let expanded = util::expand_home(raw_path);
        if self.doc.is_dirty() && !force {
            self.pending_path = Some(expanded);
            self.pending_line = line;
            self.pending_column = column;
            self.pending = Pending::Open;
            return;
        }
        self.load_file(&expanded, line, column);
    }

    fn load_file(&mut self, path: &str, line: usize, column: usize) {
        let new_doc = match Document::open(path) {
            Ok(doc) => doc,
            Err(error) => {
                self.set_error(&format!("cannot open '{path}': {error}"));
                return;
            }
        };
        self.doc = new_doc;
        self.mode = Mode::Normal;
        self.anchor = 0;
        self.cursor = 0;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.top_line = 0;
        self.left_column = 0;
        self.pending_g = false;
        self.find = FindState::new();
        self.language_override = None;
        self.jumps.clear();
        self.jump_index = -1;

        let target_line = line.min(self.doc.line_count() - 1);
        let start = self.doc.line_start(target_line);
        let text = self.doc.line_text(target_line);
        let local = util::byte_offset_for_visual_column(&text, column, TAB_WIDTH);
        self.cursor = (start + local).min(self.doc.size());
        self.snap_cursor();
        self.anchor = self.cursor;
        self.top_line = target_line.saturating_sub(self.text_rows() / 2);
        self.doc.break_undo_group();
        self.set_message(&format!("Opened {}", self.doc.display_name()));
    }

    fn open_picker(&mut self, kind: PickerKind) {
        self.picker.kind = kind;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();

        if !self.picker.files_loaded {
            self.picker.files = files::scan(20_000);
            self.picker.files_loaded = true;
        }
        self.rebuild_picker();
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn close_picker(&mut self) {
        self.picker.kind = PickerKind::None;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();
    }

    fn rebuild_picker(&mut self) {
        self.picker.items.clear();
        self.picker.total = 0;

        match self.picker.kind {
            PickerKind::Files => {
                for (index, path) in self.picker.files.iter().enumerate() {
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(path.as_bytes(), self.picker.query.as_bytes())
                            .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    if self.picker.items.len() >= 1000 {
                        continue;
                    }
                    self.picker.items.push(PickerItem {
                        display: path.clone(),
                        file_index: index,
                        line: 0,
                        column: 0,
                    });
                }
            }
            PickerKind::Search => {
                if !self.picker.query.is_empty() {
                    let hits = files::search(&self.picker.files, &self.picker.query, 500);
                    self.picker.total = hits.len();
                    for hit in hits {
                        let path = &self.picker.files[hit.file_index];
                        self.picker.items.push(PickerItem {
                            display: format!("{}:{}: {}", path, hit.line + 1, hit.preview),
                            file_index: hit.file_index,
                            line: hit.line,
                            column: hit.column,
                        });
                    }
                }
            }
            PickerKind::None => {}
        }

        if self.picker.selected >= self.picker.items.len() {
            self.picker.selected = self.picker.items.len().saturating_sub(1);
        }
        if self.picker.top > self.picker.selected {
            self.picker.top = self.picker.selected;
        }
    }

    fn handle_picker(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.close_picker();
            return;
        }

        if event.key == Key::Char && !event.ctrl && !event.alt && event.ch != '\0' {
            self.picker.query.push(event.ch);
            self.picker.selected = 0;
            self.picker.top = 0;
            self.rebuild_picker();
            return;
        }

        match event.key {
            Key::Backspace => {
                if self.picker.query.pop().is_some() {
                    self.picker.selected = 0;
                    self.picker.top = 0;
                    self.rebuild_picker();
                }
            }
            Key::Up => {
                if self.picker.selected > 0 {
                    self.picker.selected -= 1;
                }
            }
            Key::Down => {
                if self.picker.selected + 1 < self.picker.items.len() {
                    self.picker.selected += 1;
                }
            }
            Key::PageUp => {
                let rows = self.picker_list_rows();
                self.picker.selected = self.picker.selected.saturating_sub(rows);
            }
            Key::PageDown => {
                let rows = self.picker_list_rows();
                if !self.picker.items.is_empty() {
                    self.picker.selected =
                        (self.picker.selected + rows).min(self.picker.items.len() - 1);
                }
            }
            Key::Home => self.picker.selected = 0,
            Key::End => {
                self.picker.selected = self.picker.items.len().saturating_sub(1);
            }
            Key::Enter => self.open_picker_selection(),
            _ => {}
        }
    }

    fn open_picker_selection(&mut self) {
        if self.picker.items.is_empty() || self.picker.selected >= self.picker.items.len() {
            return;
        }
        let item = &self.picker.items[self.picker.selected];
        let path = self.picker.files[item.file_index].clone();
        let line = item.line;
        let column = item.column;
        self.close_picker();
        self.request_open(&path, line, column, false);
    }

    fn picker_height(&self) -> usize {
        let available = self.status_row();
        if available == 0 {
            return 0;
        }
        available.saturating_sub(1).min(18)
    }

    fn picker_list_rows(&self) -> usize {
        let height = self.picker_height();
        if height >= 3 {
            height - 3
        } else {
            1
        }
    }

    // ---------------------------------------------------------------------
    // Editing
    // ---------------------------------------------------------------------

    fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn enter_insert_from_selection(&mut self) {
        self.delete_selection();
        self.enter_insert();
    }

    fn insert_text(&mut self, text: &str) {
        if self.mode == Mode::Select {
            self.delete_selection();
        }
        self.mode = Mode::Insert;
        self.doc.insert(self.cursor, text.as_bytes());
        self.cursor += text.len();
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    fn insert_newline(&mut self) {
        let line = self.cursor_line();
        let line_start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - line_start).min(text.len());

        let mut indent_end = 0usize;
        while indent_end < local && (text[indent_end] == b' ' || text[indent_end] == b'\t') {
            indent_end += 1;
        }
        let indent = &text[..indent_end];
        let previous = if local > 0 { text[local - 1] } else { 0 };
        let next = if local < text.len() { text[local] } else { 0 };
        let opens = matches!(previous, b'{' | b'(' | b'[' | b':');

        let eol = self.doc.eol.as_bytes();
        let mut builder = Vec::new();
        builder.extend_from_slice(eol);
        builder.extend_from_slice(indent);
        if opens {
            builder.extend_from_slice(b"    ");
        }

        if matches!(next, b'}' | b')' | b']') {
            builder.extend_from_slice(eol);
            builder.extend_from_slice(indent);
            self.doc.insert(self.cursor, &builder);
            let extra = if opens { INDENT_WIDTH } else { 0 };
            self.cursor += eol.len() + indent.len() + extra;
            self.anchor = self.cursor;
            self.preferred_column = None;
            self.mode = Mode::Insert;
            self.snap_cursor();
        } else {
            let text = String::from_utf8_lossy(&builder).into_owned();
            self.insert_text(&text);
        }
    }

    fn insert_newline_below(&mut self) {
        let line = self.cursor_line();
        let end = self.doc.line_end(line);
        let text = self.doc.line_text(line);
        let mut indent = 0usize;
        while indent < text.len() && (text[indent] == b' ' || text[indent] == b'\t') {
            indent += 1;
        }
        let eol = self.doc.eol.as_bytes();
        let mut builder = Vec::new();
        builder.extend_from_slice(eol);
        builder.extend_from_slice(&text[..indent]);
        self.doc.insert(end, &builder);
        self.cursor = end + eol.len() + indent;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    fn insert_newline_above(&mut self) {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let mut indent = 0usize;
        while indent < text.len() && (text[indent] == b' ' || text[indent] == b'\t') {
            indent += 1;
        }
        let mut builder = Vec::new();
        builder.extend_from_slice(&text[..indent]);
        builder.extend_from_slice(self.doc.eol.as_bytes());
        self.doc.insert(start, &builder);
        self.cursor = start + indent;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut previous = self.previous_scalar(self.cursor);
        if previous > 0
            && self.doc.byte_at(previous) == Some(b'\n')
            && self.doc.byte_at(previous - 1) == Some(b'\r')
        {
            previous -= 1;
        }
        self.doc.erase(previous, self.cursor - previous);
        self.cursor = previous;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.snap_cursor();
    }

    fn delete_forward(&mut self) {
        if self.cursor >= self.doc.size() {
            return;
        }
        let line_end = self.line_end_of_cursor();
        let mut end = self.next_scalar(self.cursor);
        if self.cursor == line_end
            && self.doc.byte_at(self.cursor) == Some(b'\r')
            && self.doc.byte_at(self.cursor + 1) == Some(b'\n')
        {
            end = self.cursor + 2;
        }
        self.doc.erase(self.cursor, end - self.cursor);
        self.snap_cursor();
    }

    fn delete_at_cursor(&mut self) {
        if self.mode == Mode::Select {
            self.delete_selection();
        } else {
            self.delete_forward();
        }
    }

    fn delete_selection(&mut self) {
        if self.mode != Mode::Select {
            return;
        }
        let (start, end) = self.selection_range();
        if end > start {
            self.doc.replace(start, end - start, b"");
        }
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.preferred_column = None;
        self.snap_cursor();
    }

    fn indent_selection(&mut self, unindent: bool) {
        if self.mode != Mode::Select {
            self.set_message("Select lines before indenting.");
            return;
        }
        let (range_start, range_end) = self.selection_range();
        let first_line = self.doc.line_of_offset(range_start);
        let mut last_line = self.doc.line_of_offset(range_end);
        if last_line > first_line && range_end == self.doc.line_start(last_line) {
            last_line -= 1;
        }
        for line in first_line..=last_line {
            let start = self.doc.line_start(line);
            if unindent {
                let text = self.doc.line_text(line);
                let mut count = 0usize;
                while count < text.len() && count < INDENT_WIDTH && text[count] == b' ' {
                    count += 1;
                }
                if count == 0 && text.first() == Some(&b'\t') {
                    count = 1;
                }
                if count > 0 {
                    self.doc.erase(start, count);
                }
            } else {
                self.doc.insert(start, b"    ");
            }
        }
        self.doc.break_undo_group();
    }

    // ---------------------------------------------------------------------
    // Selection and clipboard
    // ---------------------------------------------------------------------

    fn start_character_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn start_line_selection(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = true;
        self.anchor = self.doc.line_start(self.cursor_line());
        self.cursor = self.anchor;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn extend_line_selection(&mut self) {
        if !self.selection_linewise {
            self.selection_linewise = true;
            self.anchor = self.doc.line_start(self.cursor_line());
            self.cursor = self.anchor;
        } else {
            let line = self.cursor_line();
            if line + 1 < self.doc.line_count() {
                self.cursor = self.doc.line_start(line + 1);
            } else {
                self.set_message("Already at the last line.");
            }
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    fn select_all(&mut self) {
        self.mode = Mode::Select;
        self.selection_linewise = false;
        self.anchor = 0;
        self.cursor = self.doc.size();
        self.preferred_column = None;
        self.doc.break_undo_group();
    }

    fn cancel_selection(&mut self) {
        self.mode = Mode::Normal;
        self.selection_linewise = false;
        self.anchor = self.cursor;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.clear_message();
    }

    fn selection_range(&self) -> (usize, usize) {
        if self.selection_linewise {
            let anchor_line = self.doc.line_of_offset(self.anchor);
            let cursor_line = self.doc.line_of_offset(self.cursor);
            let first = anchor_line.min(cursor_line);
            let last = anchor_line.max(cursor_line);
            let start = self.doc.line_start(first);
            let end = if last + 1 < self.doc.line_count() {
                self.doc.line_start(last + 1)
            } else {
                self.doc.size()
            };
            return (start, end);
        }

        let mut range_start = self.anchor.min(self.cursor);
        let mut range_end = self.anchor.max(self.cursor);
        if range_end < self.doc.size() {
            let line = self.doc.line_of_offset(range_end);
            let line_end = self.doc.line_end(line);
            if range_end < line_end {
                range_end = self.next_scalar(range_end);
            } else if range_end == line_end && line + 1 < self.doc.line_count() {
                range_end = self.doc.line_start(line + 1);
            }
        }
        if range_start > range_end {
            range_start = range_end;
        }
        (range_start, range_end)
    }

    fn yank_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to yank.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        if self.mode == Mode::Select {
            self.cursor = start;
            self.cancel_selection();
        }
        self.set_message(&format!("Yanked {} byte(s).", text.len()));
    }

    fn cut_selection(&mut self) {
        let (start, end) = if self.mode == Mode::Select {
            self.selection_range()
        } else {
            let line = self.cursor_line();
            let start = self.doc.line_start(line);
            let end = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };
            (start, end)
        };
        if end <= start {
            self.set_message("Nothing to cut.");
            return;
        }
        let text = self.doc.range(start, end - start);
        self.store_clipboard(&text);
        self.doc.replace(start, end - start, b"");
        self.cursor = start.min(self.doc.size());
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.set_message(&format!("Cut {} byte(s).", text.len()));
    }

    fn store_clipboard(&mut self, text: &[u8]) {
        self.clipboard = String::from_utf8_lossy(text).into_owned();
        let encoded = util::base64_encode(text);
        self.pending_osc = Some(format!("\x1b]52;c;{encoded}\x07").into_bytes());
    }

    fn paste_clipboard(&mut self, new_line: bool) {
        if self.clipboard.is_empty() {
            self.set_error("clipboard is empty");
            return;
        }
        let text = self.clipboard.clone();
        if new_line {
            let line = self.cursor_line();
            let end = self.doc.line_end(line);
            let mut builder = Vec::new();
            builder.extend_from_slice(self.doc.eol.as_bytes());
            builder.extend_from_slice(text.as_bytes());
            self.doc.insert(end, &builder);
            self.cursor = (end + builder.len()).min(self.doc.size());
        } else if self.mode == Mode::Select {
            let (start, end) = self.selection_range();
            self.doc.replace(start, end - start, text.as_bytes());
            self.cursor = (start + text.len()).min(self.doc.size());
        } else {
            self.doc.insert(self.cursor, text.as_bytes());
            self.cursor += text.len();
        }
        self.mode = Mode::Normal;
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.snap_cursor();
        self.set_message(&format!("Pasted {} byte(s).", text.len()));
    }

    // ---------------------------------------------------------------------
    // Movement
    // ---------------------------------------------------------------------

    fn cursor_line(&self) -> usize {
        self.doc
            .line_of_offset(self.cursor)
            .min(self.doc.line_count() - 1)
    }

    fn line_end_of_cursor(&self) -> usize {
        self.doc.line_end(self.cursor_line())
    }

    fn cursor_visual_column(&self) -> usize {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let local = (self.cursor - start).min(text.len());
        util::visual_column(&text, local, TAB_WIDTH)
    }

    fn next_scalar(&self, offset: usize) -> usize {
        if offset >= self.doc.size() {
            return self.doc.size();
        }
        let first = self.doc.byte_at(offset).unwrap_or(0);
        let length = match first {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 1,
        };
        (offset + length).min(self.doc.size())
    }

    fn previous_scalar(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let mut position = offset - 1;
        while position > 0
            && self
                .doc
                .byte_at(position)
                .map(|byte| byte & 0xC0 == 0x80)
                .unwrap_or(false)
        {
            position -= 1;
        }
        position
    }

    fn snap_cursor(&mut self) {
        if self.cursor > self.doc.size() {
            self.cursor = self.doc.size();
        }
        while self.cursor > 0
            && self.cursor < self.doc.size()
            && self
                .doc
                .byte_at(self.cursor)
                .map(|byte| byte & 0xC0 == 0x80)
                .unwrap_or(false)
        {
            self.cursor -= 1;
        }
        if self.anchor > self.doc.size() {
            self.anchor = self.doc.size();
        }
        self.clamp_viewport();
    }

    fn move_left(&mut self) {
        let line = self.cursor_line();
        let start = self.doc.line_start(line);
        if self.cursor > start {
            self.cursor = self.previous_scalar(self.cursor);
        } else if line > 0 {
            self.cursor = self.doc.line_end(line - 1);
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_right(&mut self) {
        let line = self.cursor_line();
        let end = self.doc.line_end(line);
        if self.cursor < end {
            self.cursor = self.next_scalar(self.cursor);
        } else if line + 1 < self.doc.line_count() {
            self.cursor = self.doc.line_start(line + 1);
        }
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_vertical(&mut self, delta: isize) {
        let line = self.cursor_line();
        let target = line as isize + delta;
        if target < 0 || target as usize >= self.doc.line_count() {
            return;
        }
        if self.preferred_column.is_none() {
            self.preferred_column = Some(self.cursor_visual_column());
        }
        let target = target as usize;
        let target_start = self.doc.line_start(target);
        let text = self.doc.line_text(target);
        let local = util::byte_offset_for_visual_column(
            &text,
            self.preferred_column.unwrap_or(0),
            TAB_WIDTH,
        );
        self.cursor = (target_start + local).min(self.doc.size());
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_page(&mut self, direction: isize) {
        let visible = self.text_rows().max(1);
        self.move_vertical(direction * visible as isize);
    }

    fn move_to_line_start(&mut self) {
        self.cursor = self.doc.line_start(self.cursor_line());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_to_line_end(&mut self) {
        self.cursor = self.line_end_of_cursor();
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn jump_to_document_start(&mut self) {
        self.record_jump();
        self.cursor = 0;
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn jump_to_document_end(&mut self) {
        self.record_jump();
        self.cursor = self.doc.size();
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_word_forward(&mut self) {
        let mut line = self.cursor_line();
        let mut start = self.doc.line_start(line);
        let mut text = self.doc.line_text(line);
        let mut local = (self.cursor - start).min(text.len());

        while local < text.len() && util::is_word_byte(text[local]) {
            local += 1;
        }
        while local < text.len() && !util::is_word_byte(text[local]) {
            local += 1;
        }

        if local >= text.len() {
            line += 1;
            while line < self.doc.line_count() {
                start = self.doc.line_start(line);
                text = self.doc.line_text(line);
                local = 0;
                while local < text.len() && !util::is_word_byte(text[local]) {
                    local += 1;
                }
                if local < text.len() {
                    break;
                }
                line += 1;
            }
            if line >= self.doc.line_count() {
                self.cursor = self.doc.size();
                self.snap_cursor();
                return;
            }
        }

        self.cursor = (start + local).min(self.doc.size());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut line = self.cursor_line();
        let mut start = self.doc.line_start(line);
        let mut text = self.doc.line_text(line);
        let mut local = (self.cursor - start).min(text.len());

        while local > 0 {
            let previous = util::previous_boundary(&text, local);
            if util::is_word_byte(text[previous]) {
                break;
            }
            local = previous;
        }
        while local > 0 {
            let previous = util::previous_boundary(&text, local);
            if !util::is_word_byte(text[previous]) {
                break;
            }
            local = previous;
        }

        if local == 0 && self.cursor == start && line > 0 {
            line -= 1;
            start = self.doc.line_start(line);
            text = self.doc.line_text(line);
            local = text.len();
            while local > 0 {
                let previous = util::previous_boundary(&text, local);
                if !util::is_word_byte(text[previous]) {
                    break;
                }
                local = previous;
            }
        }

        self.cursor = (start + local).min(self.doc.size());
        self.preferred_column = None;
        self.doc.break_undo_group();
        self.snap_cursor();
    }

    fn record_jump(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.jumps.last() == Some(&self.cursor) {
            return;
        }
        self.jumps.push(self.cursor);
        if self.jumps.len() > 200 {
            self.jumps.remove(0);
        }
        self.jump_index = self.jumps.len() as isize - 1;
    }

    fn move_to_previous_location(&mut self) {
        if self.jump_index > 0 {
            self.jump_index -= 1;
        }
        if self.jump_index >= 0 {
            if let Some(&offset) = self.jumps.get(self.jump_index as usize) {
                self.cursor = offset;
                self.preferred_column = None;
                self.snap_cursor();
            }
        }
    }

    fn undo(&mut self) {
        if self.doc.undo() {
            self.after_undo_redo();
        } else {
            self.set_message("Already at oldest change.");
        }
    }

    fn redo(&mut self) {
        if self.doc.redo() {
            self.after_undo_redo();
        } else {
            self.set_message("Already at newest change.");
        }
    }

    fn after_undo_redo(&mut self) {
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.snap_cursor();
        self.doc.break_undo_group();
    }

    // ---------------------------------------------------------------------
    // Commands and file operations
    // ---------------------------------------------------------------------

    fn load_plugins(&mut self) {
        let Ok(entries) = std::fs::read_dir(Path::new("plugins")) else {
            return;
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("lua"))
            .collect();
        files.sort();
        if files.is_empty() {
            return;
        }

        let mut runtime = match LuaRuntime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                self.set_error(&format!("lua init failed: {error}"));
                return;
            }
        };
        let mut loaded = 0usize;
        let mut first_error: Option<String> = None;
        for file in &files {
            let path = file.to_string_lossy().into_owned();
            match runtime.load_file(&path) {
                Ok(()) => loaded += 1,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {error}", file.display()));
                    }
                }
            }
        }
        if loaded > 0 {
            self.lua = Some(runtime);
            self.set_message(&format!("Loaded {loaded} Lua plugin(s)"));
        } else if let Some(error) = first_error {
            self.set_error(&format!("lua: {error}"));
        }
    }

    fn load_lua_plugin(&mut self, path: &str) {
        if self.lua.is_none() {
            match LuaRuntime::new() {
                Ok(runtime) => self.lua = Some(runtime),
                Err(error) => {
                    self.set_error(&format!("lua init failed: {error}"));
                    return;
                }
            }
        }
        let result = self.lua.as_mut().unwrap().load_file(path);
        match result {
            Ok(()) => self.set_message(&format!("Loaded Lua plugin {path}")),
            Err(error) => self.set_error(&format!("lua: {error}")),
        }
    }

    fn list_lua_commands(&mut self) {
        let Some(runtime) = self.lua.as_ref() else {
            self.set_message("No Lua plugins loaded.");
            return;
        };
        let names = runtime.command_names();
        if names.is_empty() {
            self.set_message("No Lua commands registered.");
        } else {
            self.set_message(&format!("Lua commands: {}", names.join(", ")));
        }
    }

    fn try_lua_command(&mut self, name: &str) -> bool {
        let (result, message) = {
            let Some(runtime) = self.lua.as_mut() else {
                return false;
            };
            if !runtime.has_command(name) {
                return false;
            }
            let result = runtime.invoke(name, &mut self.doc, &mut self.cursor);
            let message = runtime.take_message();
            (result, message)
        };

        match result {
            Ok(_) => {
                self.anchor = self.cursor;
                self.selection_linewise = false;
                self.preferred_column = None;
                self.mode = Mode::Normal;
                self.snap_cursor();
                self.doc.break_undo_group();
                if let Some(message) = message {
                    self.set_message(&message);
                }
            }
            Err(error) => self.set_error(&format!("lua: {error}")),
        }
        true
    }

    fn save_document(&mut self, path: Option<&str>) -> bool {
        match self.doc.save(path) {
            Ok(()) => {
                self.set_message(&format!("Saved {}", self.doc.display_name()));
                true
            }
            Err(error) => {
                self.set_error(&format!("save failed: {error}"));
                false
            }
        }
    }

    fn trim_trailing_whitespace(&mut self) {
        let line_count = self.doc.line_count();
        let mut changed = 0usize;
        for line in (0..line_count).rev() {
            let text = self.doc.line_text(line);
            let trimmed = util::trim_trailing_whitespace(&text);
            if trimmed.len() < text.len() {
                let start = self.doc.line_start(line);
                self.doc
                    .erase(start + trimmed.len(), text.len() - trimmed.len());
                changed += 1;
            }
        }
        self.doc.break_undo_group();
        self.set_message(&format!(
            "Removed trailing whitespace from {changed} line(s)."
        ));
    }

    // ---------------------------------------------------------------------
    // Viewport and rendering
    // ---------------------------------------------------------------------

    fn status_row(&self) -> usize {
        if self.rows >= 2 {
            self.rows - 2
        } else {
            0
        }
    }

    fn command_row(&self) -> usize {
        self.rows.saturating_sub(1)
    }

    fn text_rows(&self) -> usize {
        self.status_row()
    }

    fn gutter_width(&self) -> usize {
        util::digit_count(self.doc.line_count()) + 1
    }

    fn clamp_viewport(&mut self) {
        let last_line = self.doc.line_count().saturating_sub(1);
        if self.top_line > last_line {
            self.top_line = last_line;
        }
        let max_left = self.cols.saturating_sub(1);
        if self.left_column > max_left {
            self.left_column = max_left;
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let rows = self.text_rows();
        if rows == 0 {
            self.top_line = self.cursor_line();
            return;
        }
        let line = self.cursor_line();
        if line < self.top_line {
            self.top_line = line;
        }
        if line >= self.top_line + rows {
            self.top_line = line + 1 - rows;
        }

        let gutter = self.gutter_width();
        if self.cols <= gutter {
            return;
        }
        let width = self.cols - gutter;
        let column = self.cursor_visual_column();
        if column < self.left_column {
            self.left_column = column;
        }
        if column >= self.left_column + width {
            self.left_column = column + 1 - width;
        }
        self.clamp_viewport();
    }

    fn render(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        self.screen.resize(self.cols, self.rows);
        self.screen.clear(self.theme.base());
        if self.cols == 0 || self.rows == 0 {
            return Ok(());
        }

        self.ensure_cursor_visible();
        let rows = self.text_rows();
        let gutter = self.gutter_width();
        let first_line = self.top_line;
        let language = self.active_language();

        for row in 0..rows {
            let line = first_line + row;
            if line >= self.doc.line_count() {
                break;
            }
            self.render_line_number(line, row);
            self.render_line_text(line, row, gutter, language);
        }

        self.render_selection(first_line, rows, gutter);
        self.render_find_matches(first_line, rows, gutter);
        self.render_cursor(first_line, rows, gutter);
        self.render_status_line();
        self.render_bottom_line();
        self.render_space_menu();
        self.render_picker();
        self.render_find();

        let output = self.renderer.render(&self.screen, false);
        terminal.write(output)
    }

    fn render_line_number(&mut self, line: usize, row: usize) {
        let digits = util::digit_count(self.doc.line_count());
        let number = line + 1;
        let x = digits - util::digit_count(number);
        let style = if line == self.cursor_line() {
            self.theme.accent_style()
        } else {
            self.theme.muted_style()
        };
        self.screen
            .put_str(x as i64, row as i64, &number.to_string(), style, digits);
    }

    fn render_line_text(
        &mut self,
        line: usize,
        row: usize,
        gutter: usize,
        language: Option<&'static Language>,
    ) {
        if self.cols <= gutter {
            return;
        }
        let text = self.doc.line_text(line);
        let tokens = syntax::tokenize_line(&text, language);
        let text_width = self.cols - gutter;

        let mut offset = 0usize;
        let mut visual = 0usize;
        let mut token_index = 0usize;

        while offset < text.len() {
            let rune_start = offset;
            while token_index < tokens.len() && tokens[token_index].end <= rune_start {
                token_index += 1;
            }
            let kind = if token_index < tokens.len()
                && tokens[token_index].start <= rune_start
                && rune_start < tokens[token_index].end
            {
                tokens[token_index].kind
            } else {
                SyntaxKind::Default
            };
            let style = self.theme.syntax_style(kind);

            let (mut cp, length) = util::decode_at(&text, offset);
            if length == 0 {
                break;
            }
            offset += length;

            let mut width = if cp == '\t' {
                TAB_WIDTH - (visual % TAB_WIDTH)
            } else {
                util::display_width(cp)
            };

            if cp == '\t' {
                // Rendered as spaces below.
            } else if width == 0 {
                if cp < ' ' || cp == '\u{7F}' {
                    cp = util::REPLACEMENT;
                    width = 1;
                } else {
                    continue;
                }
            } else if cp < ' ' || cp == '\u{7F}' {
                cp = util::REPLACEMENT;
                width = 1;
            }

            let next_visual = visual + width;
            if next_visual <= self.left_column {
                visual = next_visual;
                continue;
            }

            if visual < self.left_column {
                let visible = next_visual - self.left_column;
                for i in 0..visible.min(text_width) {
                    self.screen
                        .put((gutter + i) as i64, row as i64, ' ', style, 1);
                }
                visual = next_visual;
                continue;
            }

            let screen_x = gutter + (visual - self.left_column);
            if screen_x >= self.cols {
                break;
            }
            let available = self.cols - screen_x;
            if width <= available {
                if cp == '\t' {
                    for i in 0..width {
                        self.screen
                            .put((screen_x + i) as i64, row as i64, ' ', style, 1);
                    }
                } else {
                    self.screen
                        .put(screen_x as i64, row as i64, cp, style, width);
                }
            } else {
                for i in 0..available {
                    self.screen
                        .put((screen_x + i) as i64, row as i64, ' ', style, 1);
                }
            }

            visual = next_visual;
            if visual - self.left_column >= text_width {
                break;
            }
        }
    }

    fn render_selection(&mut self, first_line: usize, rows: usize, gutter: usize) {
        if self.mode != Mode::Select || rows == 0 {
            return;
        }
        let (range_start, range_end) = self.selection_range();
        if range_end <= range_start {
            return;
        }
        let style = self.theme.selection_style();

        for row in 0..rows {
            let line = first_line + row;
            if line >= self.doc.line_count() {
                break;
            }
            let line_start = self.doc.line_start(line);
            let line_end = self.doc.line_end(line);
            let next_line_start = if line + 1 < self.doc.line_count() {
                self.doc.line_start(line + 1)
            } else {
                self.doc.size()
            };

            if self.selection_linewise {
                if range_start < next_line_start && range_end > line_start {
                    for x in gutter..self.cols {
                        self.screen.apply_style(x, row, style);
                    }
                }
                continue;
            }

            let from = range_start.max(line_start);
            let to = range_end.min(line_end);
            if to <= from {
                if range_start == line_start
                    && range_end > line_start
                    && line_end == line_start
                    && gutter < self.cols
                {
                    self.screen.apply_style(gutter, row, style);
                }
                continue;
            }

            let text = self.doc.line_text(line);
            let local_from = (from - line_start).min(text.len());
            let local_to = (to - line_start).min(text.len());
            let visual_from = util::visual_column(&text, local_from, TAB_WIDTH);
            let visual_to = util::visual_column(&text, local_to, TAB_WIDTH);
            let first_x = gutter.max(gutter + visual_from.saturating_sub(self.left_column));
            let last_x = self
                .cols
                .min(gutter + visual_to.saturating_sub(self.left_column));
            for x in first_x..last_x {
                self.screen.apply_style(x, row, style);
            }
        }
    }

    fn render_find_matches(&mut self, first_line: usize, rows: usize, gutter: usize) {
        if !self.find.active || self.find.matches.is_empty() || rows == 0 {
            return;
        }
        let match_style = self.theme.match_style();
        let current_style = self.theme.current_match_style();

        for (index, &(start, end)) in self.find.matches.iter().enumerate() {
            let line = self.doc.line_of_offset(start);
            if line < first_line || line >= first_line + rows {
                continue;
            }
            let line_start = self.doc.line_start(line);
            let line_end = self.doc.line_end(line);
            if start > line_end {
                continue;
            }
            let text = self.doc.line_text(line);
            let local_from = (start - line_start).min(text.len());
            let local_to = (end - line_start).min(text.len());
            if local_to <= local_from {
                continue;
            }
            let visual_from = util::visual_column(&text, local_from, TAB_WIDTH);
            let visual_to = util::visual_column(&text, local_to, TAB_WIDTH);
            if visual_to <= self.left_column {
                continue;
            }
            let first_x = gutter.max(gutter + visual_from.saturating_sub(self.left_column));
            let last_x = self
                .cols
                .min(gutter + visual_to.saturating_sub(self.left_column));
            let style = if self.find.index == Some(index) {
                current_style
            } else {
                match_style
            };
            for x in first_x..last_x {
                self.screen.apply_style(x, line - first_line, style);
            }
        }
    }

    fn render_cursor(&mut self, first_line: usize, rows: usize, gutter: usize) {
        let line = self.cursor_line();
        if line < first_line || line >= first_line + rows {
            return;
        }
        let column = self.cursor_visual_column();
        if column < self.left_column {
            return;
        }
        let x = gutter + (column - self.left_column);
        if x >= self.cols {
            return;
        }
        let row = line - first_line;
        let mut style = self.screen.at(x, row).style;
        style.reverse = true;
        self.screen.apply_style(x, row, style);
    }

    fn render_status_line(&mut self) {
        if self.rows == 0 {
            return;
        }
        let row = self.status_row();
        let style = self.status_style();
        self.screen.fill(0, row as i64, self.cols, 1, ' ', style);

        let line = self.cursor_line();
        let column = self.cursor_visual_column();
        let language = self.active_language();
        let mut left = format!("{}    {}", self.mode_label(), self.doc.display_name());
        if self.doc.is_dirty() {
            left.push_str(" [+]");
        }
        if let Some(language) = language {
            left.push_str(&format!("  [{}]", language.name));
        }
        let mut right = format!(
            "{}:{}  {}/{}  {}",
            line + 1,
            column + 1,
            line + 1,
            self.doc.line_count(),
            self.doc.eol.label()
        );
        if self.doc.bom {
            right.push_str(" BOM");
        }

        let right_width = util::text_width(&right);
        let right_x = if self.cols > right_width + 1 {
            self.cols - right_width - 1
        } else {
            1
        };
        let left_max = right_x.saturating_sub(1);
        if left_max > 0 {
            self.screen.put_str(1, row as i64, &left, style, left_max);
        }
        if right_x < self.cols {
            self.screen.put_str(
                right_x as i64,
                row as i64,
                &right,
                style,
                self.cols - right_x,
            );
        }
    }

    fn render_bottom_line(&mut self) {
        if self.rows == 0 {
            return;
        }
        let row = self.command_row();
        self.screen
            .fill(0, row as i64, self.cols, 1, ' ', self.theme.base());
        let width = self.cols.saturating_sub(1);

        if self.mode == Mode::Command {
            self.screen
                .put(1, row as i64, ':', self.theme.accent_style(), 1);
            self.screen.put_str(
                2,
                row as i64,
                &self.command_line,
                self.theme.text(),
                self.cols.saturating_sub(2),
            );
            let cursor_column =
                util::visual_column(self.command_line.as_bytes(), self.command_cursor, 1);
            let x = 2 + cursor_column;
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

        if let Some(input) = &self.goto_prompt {
            let text = format!("Go to line: {input}");
            self.screen
                .put_str(1, row as i64, &text, self.theme.accent_style(), width);
            let x = 1 + util::text_width(&text);
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

        if self.pending != Pending::None {
            let text = match self.pending {
                Pending::Quit => "Unsaved changes. Quit without saving? (y/N)",
                Pending::Open => "Unsaved changes. Discard and open file? (y/N)",
                Pending::None => "",
            };
            self.screen
                .put_str(1, row as i64, text, self.theme.warning_style(), width);
            return;
        }

        if !self.message.is_empty() {
            let style = if self.message_is_error {
                self.theme.error_style()
            } else {
                self.theme.text()
            };
            self.screen
                .put_str(1, row as i64, &self.message, style, width);
        }
    }

    fn render_space_menu(&mut self) {
        if !self.space_menu_open {
            return;
        }

        let mut content_width = 0usize;
        for item in SPACE_ITEMS {
            content_width = content_width.max(item.label.len() + 4);
        }
        let mut width = content_width.max(14) + 2;
        let height = SPACE_ITEMS.len() + 2;
        let status_row = self.status_row();
        if self.cols < 10 || status_row < height + 1 {
            return;
        }
        if width > self.cols - 2 {
            width = self.cols - 2;
        }
        let x = if self.cols >= width + 1 {
            self.cols - width - 1
        } else {
            0
        };
        let y = if status_row >= height {
            status_row - height
        } else {
            0
        };
        self.draw_box(x, y, width, height, "SPACE");

        for (index, item) in SPACE_ITEMS.iter().enumerate() {
            let row = y + 1 + index;
            let key = item.key.to_string();
            self.screen.put_str(
                (x + 2) as i64,
                row as i64,
                &key,
                self.theme.accent_style(),
                width.saturating_sub(3),
            );
            self.screen.put_str(
                (x + 4) as i64,
                row as i64,
                item.label,
                self.theme.text(),
                width.saturating_sub(5),
            );
        }
    }

    fn draw_box(&mut self, x: usize, y: usize, width: usize, height: usize, title: &str) {
        if width < 2 || height < 2 {
            return;
        }
        self.screen
            .fill(x as i64, y as i64, width, height, ' ', self.theme.base());
        let border = self.theme.border_style();

        for column in 1..width.saturating_sub(1) {
            self.screen
                .put((x + column) as i64, y as i64, '─', border, 1);
            self.screen
                .put((x + column) as i64, (y + height - 1) as i64, '─', border, 1);
        }
        for row in 1..height.saturating_sub(1) {
            self.screen.put(x as i64, (y + row) as i64, '│', border, 1);
            self.screen
                .put((x + width - 1) as i64, (y + row) as i64, '│', border, 1);
        }
        self.screen.put(x as i64, y as i64, '┌', border, 1);
        self.screen
            .put((x + width - 1) as i64, y as i64, '┐', border, 1);
        self.screen
            .put(x as i64, (y + height - 1) as i64, '└', border, 1);
        self.screen.put(
            (x + width - 1) as i64,
            (y + height - 1) as i64,
            '┘',
            border,
            1,
        );

        if !title.is_empty() && width > 6 {
            let text = format!(" {title} ");
            self.screen.put_str(
                (x + 2) as i64,
                y as i64,
                &text,
                self.theme.accent_style(),
                width.saturating_sub(4),
            );
        }
    }

    fn render_picker(&mut self) {
        if self.picker.kind == PickerKind::None {
            return;
        }
        let available = self.status_row();
        let height = self.picker_height();
        if height < 4 || self.cols < 20 {
            return;
        }
        let width = (self.cols - 2).min(88);
        if width < 20 {
            return;
        }
        let x = (self.cols - width) / 2;
        let y = if available >= height {
            (available - height) / 2
        } else {
            0
        };
        let title = match self.picker.kind {
            PickerKind::Files => "Open file",
            PickerKind::Search => "Search across files",
            PickerKind::None => "",
        };
        self.draw_box(x, y, width, height, title);

        self.screen.put(
            (x + 2) as i64,
            (y + 1) as i64,
            '>',
            self.theme.accent_style(),
            1,
        );
        let query_width = width.saturating_sub(5);
        self.screen.put_str(
            (x + 4) as i64,
            (y + 1) as i64,
            &self.picker.query,
            self.theme.text(),
            query_width,
        );
        let query_column =
            util::visual_column(self.picker.query.as_bytes(), self.picker.query.len(), 1);
        if query_column < query_width {
            let cursor_x = x + 4 + query_column;
            if cursor_x < x + width - 1 {
                let mut style = self.screen.at(cursor_x, y + 1).style;
                style.reverse = true;
                self.screen.apply_style(cursor_x, y + 1, style);
            }
        }

        let list_rows = if height >= 3 { height - 3 } else { 1 };
        if self.picker.selected < self.picker.top {
            self.picker.top = self.picker.selected;
        }
        if self.picker.selected >= self.picker.top + list_rows {
            self.picker.top = self.picker.selected + 1 - list_rows;
        }
        let max_top = self.picker.items.len().saturating_sub(list_rows);
        if self.picker.top > max_top {
            self.picker.top = max_top;
        }

        for row in 0..list_rows {
            let index = self.picker.top + row;
            if index >= self.picker.items.len() {
                break;
            }
            let screen_row = y + 2 + row;
            let style = if index == self.picker.selected {
                self.theme.selected_item_style()
            } else {
                self.theme.text()
            };
            self.screen
                .fill((x + 1) as i64, screen_row as i64, width - 2, 1, ' ', style);
            let display = &self.picker.items[index].display;
            self.screen.put_str(
                (x + 2) as i64,
                screen_row as i64,
                display,
                style,
                width.saturating_sub(4),
            );
        }

        let status = if self.picker.kind == PickerKind::Search && self.picker.query.is_empty() {
            "Type to search".to_string()
        } else if self.picker.total == 0 {
            "No items".to_string()
        } else {
            format!("{} item(s)", self.picker.total)
        };
        self.screen.put_str(
            (x + 2) as i64,
            (y + height - 1) as i64,
            &status,
            self.theme.accent_style(),
            width.saturating_sub(4),
        );
    }

    fn render_find(&mut self) {
        if !self.find.active {
            return;
        }
        let available = self.status_row();
        let height = 4usize;
        let width = self.cols.saturating_sub(2).min(72);
        if available < height + 1 || self.cols < 20 || width < 20 {
            return;
        }
        let x = (self.cols - width) / 2;
        let y = available - height;
        self.draw_box(x, y, width, height, "Find");

        self.screen.put(
            (x + 2) as i64,
            (y + 1) as i64,
            '>',
            self.theme.accent_style(),
            1,
        );
        let query_width = width.saturating_sub(6);
        self.screen.put_str(
            (x + 4) as i64,
            (y + 1) as i64,
            &self.find.query,
            self.theme.text(),
            query_width,
        );
        if self.find.editing {
            let column = util::visual_column(self.find.query.as_bytes(), self.find.query.len(), 1);
            if column < query_width {
                let cursor_x = x + 4 + column;
                if cursor_x < x + width - 1 {
                    let mut style = self.screen.at(cursor_x, y + 1).style;
                    style.reverse = true;
                    self.screen.apply_style(cursor_x, y + 1, style);
                }
            }
        }

        let status = if self.find.query.is_empty() {
            "Type to search".to_string()
        } else if self.find.matches.is_empty() {
            "No matches".to_string()
        } else if let Some(index) = self.find.index {
            format!(
                "{}/{} matches  Enter: navigate  n/p: next/previous  Esc: cancel",
                index + 1,
                self.find.matches.len()
            )
        } else {
            "matches".to_string()
        };
        self.screen.put_str(
            (x + 2) as i64,
            (y + 2) as i64,
            &status,
            self.theme.accent_style(),
            width.saturating_sub(4),
        );
    }

    // ---------------------------------------------------------------------
    // Small helpers
    // ---------------------------------------------------------------------

    fn active_language(&self) -> Option<&'static Language> {
        if let Some(id) = &self.language_override {
            if let Some(language) = syntax::from_id(id) {
                return Some(language);
            }
        }
        self.doc.path.as_deref().and_then(syntax::from_path)
    }

    fn mode_label(&self) -> &'static str {
        if self.pending != Pending::None {
            return "CONFIRM";
        }
        if self.find.active {
            return "FIND";
        }
        if self.picker.kind != PickerKind::None {
            return "PICKER";
        }
        if self.goto_prompt.is_some() {
            return "GOTO";
        }
        self.mode.label()
    }

    fn status_style(&self) -> crate::theme::Style {
        if self.pending != Pending::None
            || self.find.active
            || self.picker.kind != PickerKind::None
            || self.goto_prompt.is_some()
        {
            return self.theme.prompt_style();
        }
        match self.mode {
            Mode::Insert => self.theme.insert_style(),
            Mode::Select => self.theme.select_style(),
            Mode::Command => self.theme.command_style(),
            Mode::Normal => self.theme.status_style(),
        }
    }

    fn set_message(&mut self, text: &str) {
        self.message.clear();
        self.message.push_str(text);
        self.message_is_error = false;
    }

    fn set_error(&mut self, text: &str) {
        self.message.clear();
        if !text.starts_with("E: ") {
            self.message.push_str("E: ");
        }
        self.message.push_str(text);
        self.message_is_error = true;
    }

    fn clear_message(&mut self) {
        self.message.clear();
        self.message_is_error = false;
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
