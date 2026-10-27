//! Modal editor state, key handling, Ex commands, pickers and rendering.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::io;
use std::path::{Path, PathBuf};

use crate::diff;
use crate::document::Document;
use crate::files;
use crate::format;
use crate::git;
use crate::help::{HelpLineKind, HelpState, HELP_LINES};
use crate::lua::LuaRuntime;
use crate::renderer::Renderer;
use crate::screen::Screen;
use crate::sidebar::{self, Action as SidebarAction, Sidebar};
use crate::syntax;
use crate::syntax::Language;
use crate::terminal::{Event, Key, Terminal};
use crate::theme::{SyntaxKind, Theme, ThemeRegistry};
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
    SidebarDelete,
    SidebarDiff,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    None,
    Files,
    Search,
    Themes,
    Folders,
}

struct PickerItem {
    display: String,
    file_index: usize,
    line: usize,
    column: usize,
    theme_id: Option<String>,
    target: Option<PathBuf>,
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

#[derive(Clone)]
enum SidebarPromptKind {
    NewFolder { parent: PathBuf },
    NewFile { parent: PathBuf },
    Rename { old: PathBuf, is_directory: bool },
    Copy { source: PathBuf, is_directory: bool },
}

struct SidebarPrompt {
    label: String,
    text: String,
    cursor: usize,
    kind: SidebarPromptKind,
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
        key: 't',
        label: "Select theme",
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
    line_buffer: Vec<u8>,
    themes: ThemeRegistry,
    theme_before_picker: Option<String>,
    help: HelpState,
    force_full_redraw: bool,
    sidebar: Sidebar,
    sidebar_prompt: Option<SidebarPrompt>,
    pending_sidebar_deletes: Vec<(PathBuf, bool)>,
    pending_sidebar_diff: Option<(String, String)>,
    sidebar_transfer: Option<(Vec<(PathBuf, bool)>, bool)>,
    sidebar_folders: Vec<PathBuf>,
}

impl Editor {
    pub fn new(doc: Document, cols: usize, rows: usize) -> Editor {
        let message = if doc.path.is_some() {
            format!("Opened {}", doc.display_name())
        } else {
            String::new()
        };
        let themes = ThemeRegistry::load_default();
        let theme = themes.active().clone();
        let mut editor = Editor {
            doc,
            screen: Screen::new(),
            renderer: Renderer::new(),
            theme,
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
            line_buffer: Vec::new(),
            themes,
            theme_before_picker: None,
            help: HelpState::new(),
            force_full_redraw: true,
            sidebar: Sidebar::new(),
            sidebar_prompt: None,
            pending_sidebar_deletes: Vec::new(),
            pending_sidebar_diff: None,
            sidebar_transfer: None,
            sidebar_folders: Vec::new(),
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
        if self.help.visible {
            if self.help.handle(event, self.cols, self.rows, 3) {
                self.force_full_redraw = true;
            }
            return;
        }
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
        if self.sidebar_prompt.is_some() {
            self.handle_sidebar_prompt(event);
            return;
        }
        if self.sidebar.visible && self.sidebar.focused {
            self.handle_sidebar(event);
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
            't' | 'T' => self.open_picker(PickerKind::Themes),
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
            "rcfg" => {
                self.reload_themes();
            }
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
            "fmt" => self.format_current_document(false),
            "gi" => {
                self.sidebar.show_git_ignored = !self.sidebar.show_git_ignored;
                self.sidebar.refresh(None);
                self.force_full_redraw = true;
                self.set_message(if self.sidebar.show_git_ignored {
                    "Showing git-ignored files"
                } else {
                    "Hiding git-ignored files"
                });
            }
            "gdl" => {
                self.sidebar.show_modified = !self.sidebar.show_modified;
                self.sidebar.refresh(None);
                self.force_full_redraw = true;
                self.set_message(if self.sidebar.show_modified {
                    "Showing Git markers"
                } else {
                    "Hiding Git markers"
                });
            }
            "sidebar" => self.toggle_sidebar(),
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
        if kind == PickerKind::Themes {
            let active = self.theme.id.clone();
            self.themes = ThemeRegistry::load_all();
            self.themes.set_active(&active);
            self.theme_before_picker = Some(active);
        } else {
            self.theme_before_picker = None;
        }
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

    fn cancel_picker(&mut self) {
        if self.picker.kind == PickerKind::Folders {
            self.sidebar_transfer = None;
            self.set_message("Cancelled.");
        }
        if self.picker.kind == PickerKind::Themes {
            if let Some(previous) = self.theme_before_picker.take() {
                if let Some(theme) = self.themes.find(&previous).cloned() {
                    self.theme = theme;
                    self.themes.set_active(&previous);
                }
            }
        }
        self.close_picker();
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
                        theme_id: None,
                        target: None,
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
                            theme_id: None,
                            target: None,
                        });
                    }
                }
            }
            PickerKind::Folders => {
                let labels = sidebar::folder_labels(&self.sidebar_folders, &self.sidebar.root);
                for (folder, label) in self.sidebar_folders.iter().zip(labels) {
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(
                            label.as_bytes(),
                            self.picker.query.as_bytes(),
                        )
                        .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: label,
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: None,
                        target: Some(folder.clone()),
                    });
                }
            }
            PickerKind::Themes => {
                for theme in self.themes.themes() {
                    let haystack = format!(
                        "{} {} {} {}",
                        theme.name, theme.description, theme.author, theme.license
                    );
                    if !self.picker.query.is_empty()
                        && util::index_of_ignore_case(
                            haystack.as_bytes(),
                            self.picker.query.as_bytes(),
                        )
                        .is_none()
                    {
                        continue;
                    }
                    self.picker.total += 1;
                    self.picker.items.push(PickerItem {
                        display: theme.name.clone(),
                        file_index: 0,
                        line: 0,
                        column: 0,
                        theme_id: Some(theme.id.clone()),
                        target: None,
                    });
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
            self.cancel_picker();
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
                self.preview_theme_selection();
            }
            Key::Down => {
                if self.picker.selected + 1 < self.picker.items.len() {
                    self.picker.selected += 1;
                }
                self.preview_theme_selection();
            }
            Key::PageUp => {
                let rows = self.picker_list_rows();
                self.picker.selected = self.picker.selected.saturating_sub(rows);
                self.preview_theme_selection();
            }
            Key::PageDown => {
                let rows = self.picker_list_rows();
                if !self.picker.items.is_empty() {
                    self.picker.selected =
                        (self.picker.selected + rows).min(self.picker.items.len() - 1);
                }
                self.preview_theme_selection();
            }
            Key::Home => {
                self.picker.selected = 0;
                self.preview_theme_selection();
            }
            Key::End => {
                self.picker.selected = self.picker.items.len().saturating_sub(1);
                self.preview_theme_selection();
            }
            Key::Enter => self.open_picker_selection(),
            _ => {}
        }
    }

    fn open_picker_selection(&mut self) {
        if self.picker.items.is_empty() || self.picker.selected >= self.picker.items.len() {
            return;
        }
        if self.picker.kind == PickerKind::Folders {
            let target = self.picker.items[self.picker.selected].target.clone();
            self.close_picker();
            if let Some(target) = target {
                self.complete_transfer(target);
            }
            return;
        }
        if self.picker.kind == PickerKind::Themes {
            let item = &self.picker.items[self.picker.selected];
            if let Some(id) = item.theme_id.clone() {
                self.apply_theme(&id, true);
            }
            self.theme_before_picker = None;
            self.close_picker();
            return;
        }
        let item = &self.picker.items[self.picker.selected];
        let path = self.picker.files[item.file_index].clone();
        let line = item.line;
        let column = item.column;
        self.close_picker();
        self.request_open(&path, line, column, false);
    }

    fn preview_theme_selection(&mut self) {
        if self.picker.kind != PickerKind::Themes {
            return;
        }
        let Some(item) = self.picker.items.get(self.picker.selected) else {
            return;
        };
        let Some(id) = item.theme_id.clone() else {
            return;
        };
        if let Some(theme) = self.themes.find(&id).cloned() {
            self.theme = theme;
        }
    }

    fn apply_theme(&mut self, id: &str, persist: bool) {
        let Some(theme) = self.themes.find(id).cloned() else {
            self.set_error(&format!("unknown theme '{id}'"));
            return;
        };
        let name = theme.name.clone();
        self.theme = theme;
        self.themes.set_active(id);
        if persist {
            if let Err(error) = crate::theme::save_active_theme(id) {
                self.set_error(&format!("theme: {error}"));
                return;
            }
        }
        self.force_full_redraw = true;
        self.set_message(&format!("Theme: {name}"));
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

        let gutter = self.editor_left() + self.gutter_width();
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
        let gutter = self.editor_left() + self.gutter_width();
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
        self.render_sidebar();
        self.render_status_line();
        self.render_bottom_line();
        self.render_space_menu();
        self.render_sidebar_context_menu();
        self.render_picker();
        self.render_find();
        self.render_help();

        let force = self.force_full_redraw;
        self.force_full_redraw = false;
        let output = self.renderer.render(&self.screen, force);
        terminal.write(output)
    }

    fn render_line_number(&mut self, line: usize, row: usize) {
        let digits = util::digit_count(self.doc.line_count());
        let number = line + 1;
        let x = self.editor_left() + digits - util::digit_count(number);
        let style = if line == self.cursor_line() {
            self.theme.current_line_number_style()
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
        self.doc.copy_line(line, &mut self.line_buffer);
        let text = std::mem::take(&mut self.line_buffer);
        let line_override = if let Some(language) = language {
            if language.id == "diff" {
                match text.first() {
                    Some(b'+') => Some(self.theme.diff_added_style()),
                    Some(b'-') => Some(self.theme.diff_deleted_style()),
                    Some(b'@') => Some(self.theme.accent_style()),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };
        let tokens = if line_override.is_some() {
            Vec::new()
        } else {
            syntax::tokenize_line(&text, language)
        };
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
            let style = line_override.unwrap_or_else(|| self.theme.syntax_style(kind));

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
        self.line_buffer = text;
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

        if let Some(prompt) = &self.sidebar_prompt {
            let text = format!("{}{}", prompt.label, prompt.text);
            self.screen
                .put_str(1, row as i64, &text, self.theme.accent_style(), width);
            let column = util::text_width(&prompt.label)
                + util::visual_column(prompt.text.as_bytes(), prompt.cursor, 1);
            let x = 1 + column;
            if x < self.cols {
                let mut style = self.screen.at(x, row).style;
                style.reverse = true;
                self.screen.apply_style(x, row, style);
            }
            return;
        }

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
                Pending::SidebarDelete => {
                    if self.pending_sidebar_deletes.len() == 1 {
                        "Delete the selected item? (y/N)"
                    } else {
                        "Delete the selected items? (y/N)"
                    }
                }
                Pending::SidebarDiff => "Unsaved changes. Discard and show diff? (y/N)",
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
            PickerKind::Themes => "Select theme",
            PickerKind::Folders => {
                if self
                    .sidebar_transfer
                    .as_ref()
                    .map(|(_, moving)| *moving)
                    .unwrap_or(false)
                {
                    "Move to folder"
                } else {
                    "Copy to folder"
                }
            }
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

    fn render_help(&mut self) {
        if !self.help.visible {
            return;
        }
        self.screen.clear(self.theme.base());
        let (width, height, x, y, rows) = self.help.layout(self.cols, self.rows);
        self.draw_box(x, y, width, height, " Help ");
        self.help.top = self.help.top.min(self.help.max_top(self.cols, self.rows));

        let section_style = self.theme.accent_style();
        let mut key_style = self.theme.syntax_style(SyntaxKind::Default);
        key_style.fg = self.theme.accent;
        let text_style = self.theme.text();
        let muted_style = self.theme.muted_style();
        let mut key_width = (width / 3).clamp(8, 30);
        key_width = key_width.min(width.saturating_sub(8).max(4));

        for row in 0..rows {
            let index = self.help.top + row;
            if index >= HELP_LINES.len() {
                break;
            }
            let line = HELP_LINES[index];
            let screen_row = y + 1 + row;
            match line.kind {
                HelpLineKind::Section => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        section_style,
                        width.saturating_sub(4),
                    );
                }
                HelpLineKind::Entry => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        key_style,
                        key_width,
                    );
                    self.screen.put_str(
                        (x + 2 + key_width) as i64,
                        screen_row as i64,
                        line.right,
                        text_style,
                        width.saturating_sub(key_width + 4),
                    );
                }
                HelpLineKind::Text => {
                    self.screen.put_str(
                        (x + 2) as i64,
                        screen_row as i64,
                        line.left,
                        muted_style,
                        width.saturating_sub(4),
                    );
                }
                HelpLineKind::Blank => {}
            }
        }

        let max_top = self.help.max_top(self.cols, self.rows);
        if max_top > 0 && height > 2 {
            let track_height = height.saturating_sub(2).max(1);
            let thumb_height =
                ((track_height as f64 * rows as f64) / HELP_LINES.len() as f64).round() as usize;
            let thumb_height = thumb_height.max(1);
            let thumb_top = (track_height - thumb_height) * self.help.top / max_top;
            for row in 0..track_height {
                let thumb = row >= thumb_top && row < thumb_top + thumb_height;
                self.screen.put(
                    (x + width - 2) as i64,
                    (y + 1 + row) as i64,
                    if thumb { '█' } else { '│' },
                    if thumb { key_style } else { muted_style },
                    1,
                );
            }
        }

        let hint_row = y + height;
        if hint_row < self.rows {
            let hint = "Esc / q close    Up/Down scroll    PgUp/PgDn page    Home/End";
            let width = util::text_width(hint);
            let col = self.cols.saturating_sub(width) / 2;
            self.screen.put_str(
                col as i64,
                hint_row as i64,
                hint,
                muted_style,
                self.cols.saturating_sub(col),
            );
        }
    }

    fn reload_themes(&mut self) {
        let active = self.theme.id.clone();
        self.themes = ThemeRegistry::load_all();
        if !self.themes.set_active(&active) {
            if let Some(first) = self.themes.themes().first().cloned() {
                self.theme = first;
            }
        } else if let Some(theme) = self.themes.find(&active) {
            self.theme = theme.clone();
        }
        self.force_full_redraw = true;
        if let Some(error) = self.themes.errors().first() {
            self.set_error(&format!("theme: {error}"));
        } else {
            self.set_message(&format!("Reloaded {} theme(s)", self.themes.themes().len()));
        }
    }

    // ---------------------------------------------------------------------
    // Sidebar
    // ---------------------------------------------------------------------

    fn editor_left(&self) -> usize {
        self.sidebar.width(self.cols)
    }

    fn toggle_sidebar(&mut self) {
        let path = self.doc.path.clone().map(PathBuf::from);
        self.sidebar.toggle(path.as_deref());
        self.force_full_redraw = true;
        if self.sidebar.visible {
            self.set_message("Sidebar focused (Ctrl+B to close)");
        } else {
            self.clear_message();
        }
    }

    fn handle_sidebar(&mut self, event: Event) {
        if self.sidebar.menu_visible {
            self.handle_sidebar_menu(event);
            return;
        }
        if event.key == Key::Char && event.ctrl {
            match event.ch {
                'b' => {
                    self.sidebar.visible = false;
                    self.sidebar.focused = false;
                    self.force_full_redraw = true;
                }
                _ => {}
            }
            return;
        }
        if event.key == Key::Left && event.ctrl {
            self.sidebar.resize(self.cols, -2);
            self.force_full_redraw = true;
            self.set_message(&format!("Sidebar width: {}", self.sidebar.width(self.cols)));
            return;
        }
        if event.key == Key::Right && event.ctrl {
            self.sidebar.resize(self.cols, 2);
            self.force_full_redraw = true;
            self.set_message(&format!("Sidebar width: {}", self.sidebar.width(self.cols)));
            return;
        }

        let rows = self.text_rows();
        match event.key {
            Key::Escape => {
                if self.sidebar.selection_count() > 1 {
                    self.sidebar.anchor = self.sidebar.selected;
                } else {
                    self.sidebar.focused = false;
                    self.clear_message();
                }
            }
            Key::Tab => self.sidebar.move_selection(1, true, false, rows),
            Key::BackTab => self.sidebar.move_selection(-1, true, false, rows),
            Key::Up => self.sidebar.move_selection(-1, false, event.shift, rows),
            Key::Down => self.sidebar.move_selection(1, false, event.shift, rows),
            Key::PageUp => self
                .sidebar
                .move_selection(-(rows as isize), false, event.shift, rows),
            Key::PageDown => self
                .sidebar
                .move_selection(rows as isize, false, event.shift, rows),
            Key::Home => self.sidebar.set_selection(0, event.shift),
            Key::End => {
                let last = self.sidebar.nodes.len().saturating_sub(1);
                self.sidebar.set_selection(last, event.shift);
            }
            Key::Left => {
                self.sidebar.collapse_or_select_parent();
            }
            Key::Right => {
                self.sidebar.expand_or_select_child();
            }
            Key::Enter => self.activate_sidebar_node(),
            Key::Char if !event.ctrl && !event.alt => match event.ch {
                'c' | 'C' => self.open_sidebar_context_menu(),
                ' ' => {
                    self.sidebar.focused = false;
                    self.open_space_menu();
                }
                _ => {}
            },
            _ => {}
        }
        self.sidebar.update_top(rows);
        self.force_full_redraw = true;
    }

    fn activate_sidebar_node(&mut self) {
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        if node.is_directory {
            self.sidebar.toggle_directory();
            return;
        }
        self.sidebar.menu_visible = false;
        self.request_open(&node.path.to_string_lossy(), 0, 0, false);
    }

    fn open_sidebar_context_menu(&mut self) {
        if self.sidebar.selected_node().is_none() {
            return;
        }
        self.sidebar.menu_visible = true;
        self.sidebar.menu_selected = 0;
        self.sidebar.focused = true;
        self.clear_message();
    }

    fn handle_sidebar_menu(&mut self, event: Event) {
        let items = self.sidebar.menu_items();
        match event.key {
            Key::Escape => {
                self.sidebar.menu_visible = false;
                return;
            }
            Key::Tab => {
                self.sidebar.menu_selected = (self.sidebar.menu_selected + 1) % items.len();
                return;
            }
            Key::BackTab => {
                self.sidebar.menu_selected =
                    (self.sidebar.menu_selected + items.len() - 1) % items.len();
                return;
            }
            Key::Up => {
                self.sidebar.menu_selected = self.sidebar.menu_selected.saturating_sub(1);
                return;
            }
            Key::Down => {
                self.sidebar.menu_selected =
                    (self.sidebar.menu_selected + 1).min(items.len().saturating_sub(1));
                return;
            }
            Key::Enter => {
                self.activate_sidebar_menu_item();
                return;
            }
            _ => {}
        }
        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }
        match event.ch {
            'c' | 'C' => self.sidebar.menu_visible = false,
            ' ' => {
                self.sidebar.menu_visible = false;
                self.sidebar.focused = false;
                self.open_space_menu();
            }
            character => {
                if let Some(index) = self.sidebar.menu_hotkey(character) {
                    self.sidebar.menu_selected = index;
                    self.activate_sidebar_menu_item();
                }
            }
        }
    }

    fn activate_sidebar_menu_item(&mut self) {
        let items = self.sidebar.menu_items();
        if self.sidebar.menu_selected >= items.len() {
            self.sidebar.menu_visible = false;
            return;
        }
        let action = items[self.sidebar.menu_selected].action;
        self.sidebar.menu_visible = false;
        self.sidebar.focused = true;
        let nodes = self
            .sidebar
            .effective_selection()
            .into_iter()
            .map(|node| (node.path.clone(), node.is_directory))
            .collect::<Vec<_>>();
        self.run_sidebar_action(action, nodes);
    }

    fn run_sidebar_action(&mut self, action: SidebarAction, nodes: Vec<(PathBuf, bool)>) {
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        match action {
            SidebarAction::Open => {
                if node.is_directory {
                    self.sidebar.toggle_directory();
                } else {
                    self.sidebar.focused = true;
                    self.request_open(&node.path.to_string_lossy(), 0, 0, false);
                }
            }
            SidebarAction::NewFolder => {
                if nodes.len() == 1 {
                    let parent = self.sidebar_parent_for_new(&nodes[0]);
                    self.start_sidebar_prompt(
                        "New folder name: ",
                        "",
                        SidebarPromptKind::NewFolder { parent },
                    );
                }
            }
            SidebarAction::NewFile => {
                if nodes.len() == 1 {
                    let parent = self.sidebar_parent_for_new(&nodes[0]);
                    self.start_sidebar_prompt(
                        "New file name: ",
                        "",
                        SidebarPromptKind::NewFile { parent },
                    );
                }
            }
            SidebarAction::Rename => {
                if nodes.len() == 1 {
                    let (path, is_directory) = nodes[0].clone();
                    let name = git::file_name(&path);
                    self.start_sidebar_prompt(
                        &format!("Rename {name}: "),
                        &name,
                        SidebarPromptKind::Rename {
                            old: path,
                            is_directory,
                        },
                    );
                }
            }
            SidebarAction::Duplicate => self.duplicate_sidebar_nodes(&nodes),
            SidebarAction::Format => self.format_sidebar_nodes(&nodes),
            SidebarAction::Diff => self.begin_sidebar_diff(&nodes),
            SidebarAction::Copy => {
                if nodes.len() == 1 {
                    let (source, is_directory) = nodes[0].clone();
                    let initial = git::unique_copy_path(&source, is_directory);
                    self.start_sidebar_prompt(
                        "Copy to: ",
                        &initial.to_string_lossy(),
                        SidebarPromptKind::Copy {
                            source,
                            is_directory,
                        },
                    );
                } else {
                    self.open_folder_picker(nodes, false);
                }
            }
            SidebarAction::Move => self.open_folder_picker(nodes, true),
            SidebarAction::Delete => self.request_delete_sidebar_nodes(nodes),
        }
    }

    fn sidebar_parent_for_new(&self, node: &(PathBuf, bool)) -> PathBuf {
        if node.1 {
            node.0.clone()
        } else {
            node.0
                .parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| self.sidebar.root.clone())
        }
    }

    fn start_sidebar_prompt(&mut self, label: &str, initial: &str, kind: SidebarPromptKind) {
        self.sidebar.focused = self.sidebar.visible;
        self.sidebar_prompt = Some(SidebarPrompt {
            label: label.to_string(),
            text: initial.to_string(),
            cursor: initial.len(),
            kind,
        });
        self.clear_message();
    }

    fn handle_sidebar_prompt(&mut self, event: Event) {
        let Some(prompt) = self.sidebar_prompt.as_mut() else {
            return;
        };
        match event.key {
            Key::Escape => {
                let kind = prompt.kind.clone();
                self.sidebar_prompt = None;
                self.sidebar.focused = self.sidebar.visible;
                self.cancel_sidebar_prompt(kind);
            }
            Key::Enter => {
                let kind = prompt.kind.clone();
                let text = prompt.text.trim().to_string();
                self.sidebar_prompt = None;
                self.sidebar.focused = self.sidebar.visible;
                self.complete_sidebar_prompt(kind, &text);
            }
            Key::Backspace => {
                if prompt.cursor > 0 {
                    let previous = util::previous_boundary(prompt.text.as_bytes(), prompt.cursor);
                    prompt.text.replace_range(previous..prompt.cursor, "");
                    prompt.cursor = previous;
                }
            }
            Key::Delete => {
                if prompt.cursor < prompt.text.len() {
                    let next = util::next_boundary(prompt.text.as_bytes(), prompt.cursor);
                    prompt.text.replace_range(prompt.cursor..next, "");
                }
            }
            Key::Left => {
                prompt.cursor = util::previous_boundary(prompt.text.as_bytes(), prompt.cursor);
            }
            Key::Right => {
                prompt.cursor = util::next_boundary(prompt.text.as_bytes(), prompt.cursor);
            }
            Key::Home => prompt.cursor = 0,
            Key::End => prompt.cursor = prompt.text.len(),
            Key::Char if !event.ctrl && !event.alt && event.ch != '\0' => {
                prompt.text.insert(prompt.cursor, event.ch);
                prompt.cursor += event.ch.len_utf8();
            }
            _ => {}
        }
    }

    fn cancel_sidebar_prompt(&mut self, kind: SidebarPromptKind) {
        match kind {
            SidebarPromptKind::NewFolder { .. } | SidebarPromptKind::NewFile { .. } => {
                self.sidebar.refresh(None);
                self.set_message("Cancelled.");
            }
            SidebarPromptKind::Rename { old, .. } => {
                self.sidebar.refresh(Some(&old));
                self.set_message("Rename cancelled.");
            }
            SidebarPromptKind::Copy { source, .. } => {
                self.sidebar.refresh(Some(&source));
                self.set_message("Copy cancelled.");
            }
        }
    }

    fn complete_sidebar_prompt(&mut self, kind: SidebarPromptKind, text: &str) {
        match kind {
            SidebarPromptKind::NewFolder { parent } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid folder name.");
                    self.sidebar.refresh(None);
                    return;
                }
                let target = parent.join(text);
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(None);
                    return;
                }
                match std::fs::create_dir(&target) {
                    Ok(()) => {
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Created folder {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Create folder failed: {error}"));
                        self.sidebar.refresh(None);
                    }
                }
            }
            SidebarPromptKind::NewFile { parent } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid file name.");
                    self.sidebar.refresh(None);
                    return;
                }
                let target = parent.join(text);
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(None);
                    return;
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                {
                    Ok(_) => {
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Created file {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Create file failed: {error}"));
                        self.sidebar.refresh(None);
                    }
                }
            }
            SidebarPromptKind::Rename { old, is_directory } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid file or folder name.");
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                let parent = old
                    .parent()
                    .map(|parent| parent.to_path_buf())
                    .unwrap_or_else(|| self.sidebar.root.clone());
                let target = parent.join(text);
                if target == old {
                    self.set_message("Name unchanged.");
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                let result = if is_directory {
                    std::fs::rename(&old, &target)
                } else {
                    std::fs::rename(&old, &target)
                };
                match result {
                    Ok(()) => {
                        if is_directory {
                            self.sidebar.update_expanded_after_rename(&old, &target);
                        }
                        self.rename_open_document(&old, &target, is_directory);
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Renamed to {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Rename failed: {error}"));
                        self.sidebar.refresh(Some(&old));
                    }
                }
            }
            SidebarPromptKind::Copy {
                source,
                is_directory,
            } => {
                if text.is_empty() {
                    self.sidebar.refresh(Some(&source));
                    self.set_message("Copy cancelled.");
                    return;
                }
                let mut destination = PathBuf::from(text);
                if !destination.is_absolute() {
                    destination = self.sidebar.root.join(destination);
                }
                destination = git::absolute(&destination);
                if destination.is_dir() {
                    destination = destination.join(git::file_name(&source));
                }
                match git::copy_path(&source, &destination, is_directory) {
                    Ok(()) => {
                        self.sidebar.expand_to(Some(&destination));
                        self.sidebar.refresh(Some(&destination));
                        self.set_message(&format!("Copied to {}", git::file_name(&destination)));
                    }
                    Err(error) => {
                        self.set_error(&error);
                        self.sidebar.refresh(Some(&source));
                    }
                }
            }
        }
    }

    fn rename_open_document(&mut self, old: &Path, new: &Path, is_directory: bool) {
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        let document_path = git::absolute(Path::new(&path));
        let old = git::absolute(old);
        let new = git::absolute(new);
        let updated = if document_path == old {
            Some(new.clone())
        } else if is_directory && git::is_inside(&old, &document_path) {
            document_path
                .strip_prefix(&old)
                .ok()
                .map(|relative| new.join(relative))
        } else {
            None
        };
        if let Some(updated) = updated {
            self.doc.path = Some(updated.to_string_lossy().into_owned());
        }
    }

    fn duplicate_sidebar_nodes(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.is_empty() {
            return;
        }
        let mut duplicated = 0usize;
        let mut first_error = None;
        let mut last_target = None;
        for (path, is_directory) in nodes {
            let target = git::unique_copy_path(path, *is_directory);
            match git::copy_path(path, &target, *is_directory) {
                Ok(()) => {
                    duplicated += 1;
                    last_target = Some(target);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(target) = last_target.as_ref() {
            self.sidebar.expand_to(Some(target));
            self.sidebar.refresh(Some(target));
        } else {
            self.sidebar.refresh(None);
        }
        if let Some(error) = first_error {
            self.set_error(&if duplicated == 0 {
                error
            } else {
                format!("Duplicated {duplicated} of {} items; {error}", nodes.len())
            });
        } else {
            let target_name = last_target
                .as_deref()
                .map(git::file_name)
                .unwrap_or_default();
            self.set_message(&if duplicated == 1 {
                format!("Duplicated as {target_name}")
            } else {
                format!("Duplicated {duplicated} items")
            });
        }
    }

    fn format_sidebar_nodes(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.is_empty() {
            return;
        }
        if nodes.len() == 1 {
            if let Some(path) = &self.doc.path {
                if git::same_path(Path::new(path), &nodes[0].0) && !nodes[0].1 {
                    self.format_current_document(true);
                    return;
                }
            }
        }

        let mut files = Vec::new();
        for (path, is_directory) in nodes {
            if *is_directory {
                collect_files(path, self.sidebar.show_git_ignored, &mut files);
            } else if path.is_file() {
                files.push(path.clone());
            }
        }
        files.sort();
        files.dedup();

        let mut formatted = 0usize;
        let mut changed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut first_error = None;
        let open_path = self
            .doc
            .path
            .as_ref()
            .map(|path| git::absolute(Path::new(path)));
        for file in files {
            if let Some(open) = &open_path {
                if git::same_path(open, &file) {
                    skipped += 1;
                    continue;
                }
            }
            let language = syntax::from_path(&file.to_string_lossy());
            let Some(language) = language else {
                skipped += 1;
                continue;
            };
            let Some(formatter) = format::for_language(language.id) else {
                skipped += 1;
                continue;
            };
            let input = match std::fs::read(&file) {
                Ok(input) => input,
                Err(error) => {
                    failed += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {error}", git::file_name(&file)));
                    }
                    continue;
                }
            };
            match format::run(&formatter, &input, Some(&file)) {
                Ok(output) => {
                    formatted += 1;
                    let output = if git::has_utf8_bom(&input) && !git::has_utf8_bom(&output) {
                        let mut with_bom = Vec::with_capacity(output.len() + 3);
                        with_bom.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                        with_bom.extend_from_slice(&output);
                        with_bom
                    } else {
                        output
                    };
                    if output != input {
                        if let Err(error) = git::atomic_write(&file, &output) {
                            failed += 1;
                            if first_error.is_none() {
                                first_error = Some(format!("{}: {error}", git::file_name(&file)));
                            }
                            continue;
                        }
                        changed += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {error}", git::file_name(&file)));
                    }
                }
            }
        }

        self.sidebar.refresh(None);
        if let Some(error) = first_error {
            self.set_error(&format!(
                "Formatted {formatted} file(s), {failed} failed: {error}"
            ));
        } else if formatted == 0 {
            self.set_error("No formatter configured for the selected files.");
        } else {
            let mut summary = format!("Formatted {formatted} file(s)");
            if changed > 0 {
                summary.push_str(&format!(", {changed} changed"));
            }
            if skipped > 0 {
                summary.push_str(&format!(", {skipped} skipped"));
            }
            self.set_message(&summary);
        }
    }

    fn format_current_document(&mut self, save_after: bool) {
        let language = self.active_language();
        let Some(language) = language else {
            self.set_error("No language detected for the current buffer.");
            return;
        };
        let Some(formatter) = format::for_language(language.id) else {
            self.set_error("No formatter configured for this language.");
            return;
        };
        let input = self.doc.all_text();
        let path = self.doc.path.clone().map(PathBuf::from);
        match format::run(&formatter, &input, path.as_deref()) {
            Ok(output) => {
                if output != input {
                    self.doc.replace_all_owned(output);
                    self.doc.break_undo_group();
                }
                if save_after && path.is_some() {
                    self.save_document(None);
                }
                self.set_message("Formatted buffer.");
            }
            Err(error) => self.set_error(&error),
        }
    }

    fn begin_sidebar_diff(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.len() != 2 || nodes[0].1 || nodes[1].1 {
            self.set_error("Diff requires exactly two files.");
            return;
        }
        let first = std::fs::read(&nodes[0].0);
        let second = std::fs::read(&nodes[1].0);
        let (Ok(first), Ok(second)) = (first, second) else {
            self.set_error("Cannot read file for diff.");
            return;
        };
        let old_label = git::display_relative(&self.sidebar.root, &nodes[0].0);
        let new_label = git::display_relative(&self.sidebar.root, &nodes[1].0);
        let old_text = String::from_utf8_lossy(git::trim_utf8_bom(&first)).into_owned();
        let new_text = String::from_utf8_lossy(git::trim_utf8_bom(&second)).into_owned();
        let diff = diff::create(&old_label, &old_text, &new_label, &new_text);
        let summary = format!("{old_label} vs {new_label}");
        if self.doc.is_dirty() {
            self.pending_sidebar_diff = Some((diff, summary));
            self.pending = Pending::SidebarDiff;
            self.clear_message();
            return;
        }
        self.show_sidebar_diff(&diff, &summary);
    }

    fn show_sidebar_diff(&mut self, diff: &str, summary: &str) {
        self.sidebar.menu_visible = false;
        self.sidebar.focused = false;
        self.doc = Document::from_bytes(None, diff.as_bytes().to_vec());
        self.language_override = Some("diff".into());
        self.mode = Mode::Normal;
        self.cursor = 0;
        self.anchor = 0;
        self.top_line = 0;
        self.left_column = 0;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.find = FindState::new();
        self.doc.break_undo_group();
        self.snap_cursor();
        self.set_message(&format!("Diff: {summary}"));
    }

    fn request_delete_sidebar_nodes(&mut self, nodes: Vec<(PathBuf, bool)>) {
        if nodes.is_empty() {
            return;
        }
        if let Some(path) = &self.doc.path {
            let document = git::absolute(Path::new(path));
            for (selected, is_directory) in &nodes {
                let selected = git::absolute(selected);
                if document == selected || (*is_directory && git::is_inside(&selected, &document)) {
                    self.set_error("Cannot delete the open file or its parent folder.");
                    return;
                }
            }
        }
        self.pending_sidebar_deletes = nodes;
        self.pending = Pending::SidebarDelete;
        self.clear_message();
    }

    fn delete_sidebar_paths(&mut self) {
        let items = std::mem::take(&mut self.pending_sidebar_deletes);
        let mut deleted = 0usize;
        let mut first_error = None;
        for (path, is_directory) in items {
            let result = if is_directory {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    deleted += 1;
                    if is_directory {
                        self.sidebar.remove_expanded_under(&path);
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(format!(
                            "Delete failed for '{}': {error}",
                            git::file_name(&path)
                        ));
                    }
                }
            }
        }
        self.sidebar.refresh(None);
        if let Some(error) = first_error {
            self.set_error(&error);
        } else {
            self.set_message(&format!("Deleted {deleted} item(s)"));
        }
    }

    fn open_folder_picker(&mut self, nodes: Vec<(PathBuf, bool)>, move_items: bool) {
        let references = nodes
            .iter()
            .map(|(path, is_directory)| (path.clone(), *is_directory))
            .collect::<Vec<_>>();
        let sidebar_nodes = references
            .iter()
            .filter_map(|(path, _)| self.sidebar.nodes.iter().find(|node| &node.path == path))
            .collect::<Vec<_>>();
        self.sidebar_folders = sidebar::folder_choices(
            &sidebar_nodes,
            self.sidebar.show_git_ignored,
            &self.sidebar.root,
        );
        self.sidebar_transfer = Some((references, move_items));
        self.picker.kind = PickerKind::Folders;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();
        self.rebuild_picker();
        self.clear_message();
        self.set_message(if move_items {
            "Choose a destination folder to move to"
        } else {
            "Choose a destination folder to copy to"
        });
    }

    fn complete_transfer(&mut self, destination: PathBuf) {
        let Some((sources, move_items)) = self.sidebar_transfer.take() else {
            return;
        };
        let mut targets = std::collections::HashSet::new();
        for (source, _) in &sources {
            let target = destination.join(git::file_name(source));
            if !targets.insert(target) {
                self.set_error("Multiple items would collide in that folder.");
                self.sidebar.refresh(None);
                return;
            }
        }
        let mut done = 0usize;
        let mut first_error = None;
        let mut last_target = None;
        for (source, is_directory) in sources {
            let target = destination.join(git::file_name(&source));
            let result = if move_items {
                if is_directory && (destination == source || git::is_inside(&source, &destination))
                {
                    Err("Cannot move a folder inside itself.".to_string())
                } else if target.exists() {
                    Err(format!(
                        "'{}' already exists in that folder.",
                        git::file_name(&target)
                    ))
                } else {
                    std::fs::rename(&source, &target)
                        .map_err(|error| format!("Move failed: {error}"))
                }
            } else {
                git::copy_path(&source, &target, is_directory)
            };
            match result {
                Ok(()) => {
                    if move_items {
                        if is_directory {
                            self.sidebar.update_expanded_after_rename(&source, &target);
                        }
                        self.rename_open_document(&source, &target, is_directory);
                    }
                    done += 1;
                    last_target = Some(target);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(target) = last_target {
            self.sidebar.expand_to(Some(&target));
            self.sidebar.refresh(Some(&target));
        } else {
            self.sidebar.refresh(None);
        }
        if let Some(error) = first_error {
            self.set_error(&if done == 0 {
                error
            } else {
                format!("Completed {done} item(s); {error}")
            });
        } else {
            self.set_message(&format!(
                "{} {done} item(s) to {}",
                if move_items { "Moved" } else { "Copied" },
                destination.display()
            ));
        }
    }

    fn render_sidebar(&mut self) {
        if !self.sidebar.visible {
            return;
        }
        let width = self.sidebar.width(self.cols);
        let rows = self.text_rows();
        if width == 0 || rows == 0 {
            return;
        }
        self.screen.fill(0, 0, width, rows, ' ', self.theme.base());
        let border_style = if self.sidebar.focused {
            self.theme.accent_style()
        } else {
            self.theme.border_style()
        };
        for row in 0..rows {
            self.screen
                .put((width - 1) as i64, row as i64, '│', border_style, 1);
        }
        if self.sidebar.nodes.is_empty() {
            self.screen.put_str(
                1,
                0,
                "No files",
                self.theme.muted_style(),
                width.saturating_sub(2),
            );
            return;
        }

        self.sidebar.update_top(rows);
        let list_rows = rows;
        let selected_style = self.theme.selected_item_style();
        let folder_style = self.theme.syntax_style(SyntaxKind::Default);
        let mut folder_style = folder_style;
        folder_style.fg = self.theme.accent;
        let file_style = self.theme.text();
        let marker_style = self.theme.git_modified_style();

        for row in 0..list_rows {
            let index = self.sidebar.top + row;
            let Some(node) = self.sidebar.nodes.get(index) else {
                break;
            };
            let selected =
                index >= self.sidebar.selection_start() && index <= self.sidebar.selection_end();
            let style = if selected {
                selected_style
            } else if node.is_directory {
                folder_style
            } else {
                file_style
            };
            if selected {
                self.screen.fill(0, row as i64, width - 1, 1, ' ', style);
            }
            let modified = self.sidebar.is_git_modified(node);
            let untracked = self.sidebar.is_git_untracked(node);
            let marker_count = modified as usize + untracked as usize;
            let indent = 1 + node.depth * 2;
            let marker = if node.is_directory {
                if node.expanded {
                    "▾"
                } else {
                    "▸"
                }
            } else {
                " "
            };
            self.screen.put_str(
                indent as i64,
                row as i64,
                marker,
                style,
                width.saturating_sub(indent + 1),
            );
            let name_x = indent + 2;
            if name_x < width - 1 {
                let mut name_width = width - name_x - 1;
                if marker_count > 0 {
                    name_width = name_width.saturating_sub(marker_count + 1);
                }
                self.screen
                    .put_str(name_x as i64, row as i64, &node.name, style, name_width);
            }
            if marker_count > 0 {
                let mut marker_x = width - 2 - marker_count;
                let mut marker_style = marker_style;
                if selected {
                    marker_style.bg = self.theme.selection_bg;
                }
                if modified && marker_x < width {
                    self.screen
                        .put(marker_x as i64, row as i64, 'M', marker_style, 1);
                    marker_x += 1;
                }
                if untracked && marker_x < width {
                    self.screen
                        .put(marker_x as i64, row as i64, 'N', marker_style, 1);
                }
            }
        }
    }

    fn render_sidebar_context_menu(&mut self) {
        if !self.sidebar.menu_visible {
            return;
        }
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        let items = self.sidebar.menu_items();
        let mut width = 4usize;
        for item in items {
            width = width.max(item.label.len() + 7);
        }
        let rows = self.text_rows();
        let height = items.len() + 2;
        if rows < 3 || self.cols < 10 {
            return;
        }
        width = width.min(self.cols.saturating_sub(2));
        let anchor_row = 1 + self.sidebar.selected.saturating_sub(self.sidebar.top);
        let y = anchor_row.min(rows.saturating_sub(height));
        let title = if self.sidebar.selection_count() > 1 {
            format!("{} selected", self.sidebar.selection_count())
        } else if node.is_directory {
            "Folder".into()
        } else {
            "File".into()
        };
        self.draw_box(1, y, width, height, &title);
        let selected_style = self.theme.selected_item_style();
        let item_style = self.theme.text();
        for (index, item) in items.iter().enumerate() {
            let row = y + 1 + index;
            let style = if index == self.sidebar.menu_selected {
                selected_style
            } else {
                item_style
            };
            self.screen
                .fill(2, row as i64, width.saturating_sub(2), 1, ' ', style);
            self.screen
                .put_str(3, row as i64, item.label, style, width.saturating_sub(7));
            self.screen.put_str(
                (width.saturating_sub(2)) as i64,
                row as i64,
                &item.hotkey.to_string(),
                style,
                2,
            );
        }
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
            return "MODAL";
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
            return self.theme.alert_style();
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

fn collect_files(path: &Path, show_git_ignored: bool, files: &mut Vec<PathBuf>) {
    if files.len() >= 20_000 {
        return;
    }
    let ignore = if show_git_ignored {
        None
    } else {
        Some(git::GitIgnore::new(&self::git::project_root()))
    };
    collect_files_inner(path, ignore.as_ref(), files, 0);
}

fn collect_files_inner(
    path: &Path,
    ignore: Option<&git::GitIgnore>,
    files: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 24 || files.len() >= 20_000 {
        return;
    }
    let ignore = ignore.map(|rules| rules.with_directory(path));
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let child = entry.path();
        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if git::is_built_in_ignored_directory(&name) {
                continue;
            }
            if let Some(rules) = ignore.as_ref() {
                if rules.is_ignored(&child, true) {
                    continue;
                }
            }
            collect_files_inner(&child, ignore.as_ref(), files, depth + 1);
        } else if file_type.is_file() {
            if let Some(rules) = ignore.as_ref() {
                if rules.is_ignored(&child, false) {
                    continue;
                }
            }
            files.push(child);
            if files.len() >= 20_000 {
                return;
            }
        }
    }
}
