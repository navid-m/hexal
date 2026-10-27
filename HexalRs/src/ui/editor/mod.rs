//! Modal editor state, key handling, Ex commands, pickers and rendering.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

mod comments;
mod editing;
mod find;
mod input;
mod lsp_ui;
mod navigation;
mod palette;
mod picker;
mod plugins;
mod render;
mod sidebar;
mod status;

use std::io;
use std::path::{Path, PathBuf};

use crate::core::diff;
use crate::core::document::Document;
use crate::core::format;
use crate::languages::{FormatterConfig, Highlighter, Language, Registry, Token};
use crate::lsp::{self, Manager as LspManager};
use crate::platform::terminal::{Event, Key, Terminal};
use crate::scripting::lua::LuaRuntime;
use crate::ui::help::{HELP_LINES, HelpLineKind, HelpState};
use crate::ui::renderer::Renderer;
use crate::ui::screen::Screen;
use crate::ui::sidebar::{self as ui_sidebar, Action as SidebarAction, Sidebar};
use crate::ui::theme::{SyntaxKind, Theme, ThemeRegistry};
use crate::util;
use crate::util::files;
use crate::util::git;

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
    Palette,
    CodeActions,
    Symbols,
    Problems,
}

#[derive(Clone)]
enum PaletteAction {
    OpenFiles,
    SearchFiles,
    GotoLine,
    OpenThemes,
    ToggleSidebar,
    ToggleGitIgnored,
    ToggleDiffLines,
    ToggleSearchMatchCase,
    FormatDocument,
    Hover,
    GoToDefinition,
    CodeActions,
    DocumentSymbols,
    DocumentDiagnostics,
    WorkspaceDiagnostics,
    CleanWhitespace,
    Save,
    SaveAndQuit,
    ReloadThemes,
    ShowHelp,
    About,
    ShowPath,
    Quit,
    Plugin(String),
}

#[derive(Clone)]
struct PaletteCommand {
    title: String,
    description: String,
    action: PaletteAction,
}

struct PickerItem {
    display: String,
    file_index: usize,
    line: usize,
    column: usize,
    theme_id: Option<String>,
    target: Option<PathBuf>,
    palette_index: Option<usize>,
    action_index: Option<usize>,
    symbol_index: Option<usize>,
    problem: Option<(String, usize, usize)>,
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
        key: 'K',
        label: "LSP hover",
    },
    SpaceItem {
        key: 'A',
        label: "Code actions",
    },
    SpaceItem {
        key: 'S',
        label: "Document symbols",
    },
    SpaceItem {
        key: 'd',
        label: "Document diagnostics",
    },
    SpaceItem {
        key: 'D',
        label: "Workspace diagnostics",
    },
    SpaceItem {
        key: 'Z',
        label: "Diagnostic on this line",
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

const HELP_TEXT: &str = "i/a/o insert  v select  x line select  d delete  u/U undo/redo  gg/ge/gp  :w :wq :q :e  / find  :lua <file>  Ctrl+P palette  Space menu";

struct CompletionState {
    visible: bool,
    pending: bool,
    request_id: i64,
    all_items: Vec<lsp::CompletionItem>,
    items: Vec<lsp::CompletionItem>,
    filter: String,
    selected: usize,
    top: usize,
}

impl CompletionState {
    fn new() -> CompletionState {
        CompletionState {
            visible: false,
            pending: false,
            request_id: 0,
            all_items: Vec::new(),
            items: Vec::new(),
            filter: String::new(),
            selected: 0,
            top: 0,
        }
    }

    fn close(&mut self) {
        self.visible = false;
        self.pending = false;
        self.request_id = 0;
        self.all_items.clear();
        self.items.clear();
        self.filter.clear();
        self.selected = 0;
        self.top = 0;
    }
}

#[derive(Clone)]
struct HoverSegment {
    text: String,
    kind: SyntaxKind,
}

#[derive(Clone)]
struct HoverLine {
    segments: Vec<HoverSegment>,
}

struct HoverState {
    visible: bool,
    title: String,
    lines: Vec<HoverLine>,
    top: usize,
}

impl HoverState {
    fn new() -> HoverState {
        HoverState {
            visible: false,
            title: "Hover".into(),
            lines: Vec::new(),
            top: 0,
        }
    }

    fn close(&mut self) {
        self.visible = false;
        self.lines.clear();
        self.top = 0;
    }
}

struct LspDebugState {
    visible: bool,
    top: usize,
    status: String,
}

impl LspDebugState {
    fn new() -> LspDebugState {
        LspDebugState {
            visible: false,
            top: 0,
            status: String::new(),
        }
    }
}

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
    languages: Registry,
    highlighter: Highlighter,
    line_tokens: Vec<Token>,
    jumps: Vec<usize>,
    jump_index: isize,

    picker: Picker,
    palette_commands: Vec<PaletteCommand>,
    find: FindState,
    search_match_case: bool,
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

    lsp: LspManager,
    lsp_dirty: bool,
    lsp_synced_revision: u64,
    lsp_dirty_since: std::time::Instant,
    lsp_fallback_blocked: bool,
    lsp_start_pending: bool,
    startup_work_pending: bool,
    last_revision: u64,
    hover_request_pending: bool,
    definition_request_pending: bool,
    code_action_request_pending: bool,
    pending_code_action_resolve: bool,
    symbol_request_pending: bool,
    code_actions: Vec<lsp::CodeAction>,
    document_symbols: Vec<lsp::DocumentSymbol>,
    unnecessary_diagnostics: std::collections::HashMap<usize, Vec<(i64, i64)>>,
    completion: CompletionState,
    hover: HoverState,
    lsp_debug: LspDebugState,
    lsp_debug_lines: Vec<String>,
    problems_workspace: bool,
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
        let languages = Registry::load();
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
            languages,
            highlighter: Highlighter::default(),
            line_tokens: Vec::new(),
            jumps: Vec::new(),
            jump_index: -1,
            picker: Picker::new(),
            palette_commands: Vec::new(),
            find: FindState::new(),
            search_match_case: false,
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
            lsp: LspManager::new(),
            lsp_dirty: false,
            lsp_synced_revision: u64::MAX,
            lsp_dirty_since: std::time::Instant::now(),
            lsp_fallback_blocked: false,
            lsp_start_pending: true,
            startup_work_pending: true,
            last_revision: 0,
            hover_request_pending: false,
            definition_request_pending: false,
            code_action_request_pending: false,
            pending_code_action_resolve: false,
            symbol_request_pending: false,
            code_actions: Vec::new(),
            document_symbols: Vec::new(),
            unnecessary_diagnostics: std::collections::HashMap::new(),
            completion: CompletionState::new(),
            hover: HoverState::new(),
            lsp_debug: LspDebugState::new(),
            lsp_debug_lines: Vec::new(),
            problems_workspace: false,
        };
        editor.last_revision = editor.doc.revision();
        editor.refresh_language();
        editor.load_plugins();
        editor
    }

    pub fn run(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        let mut last_width = terminal.width;
        let mut last_height = terminal.height;

        // Paint before polling input. A slow event wait must never delay the
        // first frame, which is also what the startup benchmark measures.
        self.render(terminal)?;
        self.poll_background();

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
                    if self.doc.revision() != self.last_revision {
                        self.last_revision = self.doc.revision();
                        self.lsp_dirty = self.lsp.is_running();
                        self.lsp_dirty_since = std::time::Instant::now();
                    }
                    if let Some(sequence) = self.pending_osc.take() {
                        let _ = terminal.write(&sequence);
                    }
                }
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error),
            }

            self.poll_background();
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
        if self.lsp_debug.visible {
            self.handle_lsp_debug_input(event);
            return;
        }
        if self.hover.visible {
            self.handle_hover_input(event);
            return;
        }
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
        if event.key == Key::Char
            && event.ctrl
            && matches!(event.ch, 'p' | 'P')
            && matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Select)
        {
            self.open_palette();
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
}
