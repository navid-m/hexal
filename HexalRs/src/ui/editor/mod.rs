//! Modal editor state, key handling, Ex commands, pickers and rendering.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

mod about;
mod command_completion;
mod comments;
mod debug;
mod diff_lines;
mod editing;
mod find;
mod input;
mod lsp_ui;
mod navigation;
mod palette;
mod panes;
mod picker;
mod plugin_manager;
mod plugin_panels;
mod plugins;
mod reload_all;
mod render;
mod sidebar;
mod status;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

use crate::core::config::Config;
use crate::core::diff;
use crate::core::document::Document;
use crate::core::format;
use crate::languages::{FormatterConfig, Highlighter, Language, Registry, Token};
use crate::lsp::{self, Manager as LspManager};
use crate::platform::terminal::{Event, Key, MouseButton, Terminal};
use crate::scripting::plugin::PluginManager;
use crate::ui::help::{HelpLineKind, HelpState, HELP_LINES};
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

/// A line/visual-column cursor position used to keep the cursor stable while
/// a formatter rewrites the buffer. Byte offsets are not stable across
/// formatting because indentation and line breaks can move.
#[derive(Clone, Copy)]
struct CursorSnapshot {
    cursor_line: usize,
    cursor_column: usize,
    anchor_line: usize,
    anchor_column: usize,
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
    DiffWith,
}

#[derive(Clone)]
enum PaletteAction {
    OpenFiles,
    SearchFiles,
    OpenConfig,
    GotoLine,
    OpenThemes,
    ToggleSidebar,
    ToggleGitIgnored,
    ToggleDiffLines,
    ToggleSearchMatchCase,
    SplitPaneVertical,
    SplitPaneHorizontal,
    ClosePane,
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
    LspDebug,
    DebugVariables,
    ManagePlugins,
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
    files: Arc<Vec<String>>,
    files_loaded: bool,
    items: Vec<PickerItem>,
    selected: usize,
    top: usize,
    total: usize,
    status: Option<String>,
}

impl Picker {
    fn new() -> Picker {
        Picker {
            kind: PickerKind::None,
            query: String::new(),
            files: Arc::new(Vec::new()),
            files_loaded: false,
            items: Vec::new(),
            selected: 0,
            top: 0,
            total: 0,
            status: None,
        }
    }
}

struct PickerSearchResult {
    generation: u64,
    hits: Vec<files::SearchHit>,
}

struct PickerSearchTask {
    receiver: mpsc::Receiver<PickerSearchResult>,
    cancel: Arc<AtomicBool>,
}

struct PickerScanResult {
    generation: u64,
    files: Vec<String>,
}

struct PickerScanTask {
    receiver: mpsc::Receiver<PickerScanResult>,
    cancel: Arc<AtomicBool>,
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

#[derive(Clone)]
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

type PaneId = u64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaneSplitOrientation {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy)]
enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone)]
enum PaneNode {
    Leaf(PaneId),
    Split {
        orientation: PaneSplitOrientation,
        ratio: f64,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

/// Everything that belongs to one split view rather than to the editor as a
/// whole. Documents themselves live either in `Editor::doc` (the active one)
/// or in `Editor::documents`, keyed by document id, so panes that show the
/// same buffer share a single document object.
#[derive(Clone)]
struct PaneState {
    id: PaneId,
    doc_id: u64,
    language_override: Option<String>,
    mode: Mode,
    cursor: usize,
    anchor: usize,
    selection_linewise: bool,
    preferred_column: Option<usize>,
    top_line: usize,
    left_column: usize,
    pending_g: bool,
    mouse_dragging: bool,
    mouse_drag_anchor: usize,
    find: FindState,
    jumps: Vec<usize>,
    jump_index: isize,
    highlighter: Highlighter,
    git_diff: diff_lines::DiffLineState,
}

#[derive(Clone, Copy)]
struct PaneView {
    pane: PaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy)]
struct PaneDivider {
    x: usize,
    y: usize,
    length: usize,
    vertical: bool,
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
        label: "LSP code actions",
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommandCompletionKind {
    Files,
    Commands,
}

struct CommandCompletion {
    visible: bool,
    kind: CommandCompletionKind,
    items: Vec<String>,
    selected: usize,
    top: usize,
    prefix: String,
    suffix: String,
    filter: String,
}

impl CommandCompletion {
    fn new() -> CommandCompletion {
        CommandCompletion {
            visible: false,
            kind: CommandCompletionKind::Commands,
            items: Vec::new(),
            selected: 0,
            top: 0,
            prefix: String::new(),
            suffix: String::new(),
            filter: String::new(),
        }
    }

    fn close(&mut self) {
        self.visible = false;
        self.items.clear();
        self.selected = 0;
        self.top = 0;
        self.prefix.clear();
        self.suffix.clear();
        self.filter.clear();
    }

    fn rows(&self) -> usize {
        self.items.len().clamp(1, 10)
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

struct SidebarFormatProgress {
    total: AtomicUsize,
    completed: AtomicUsize,
}

impl SidebarFormatProgress {
    fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    fn completed(&self) -> usize {
        self.completed.load(Ordering::Relaxed)
    }
}

struct SidebarFormatResult {
    formatted: usize,
    changed: usize,
    failed: usize,
    skipped: usize,
    skipped_open: usize,
    first_error: Option<String>,
}

struct SidebarFormatOperation {
    receiver: mpsc::Receiver<SidebarFormatResult>,
    progress: Arc<SidebarFormatProgress>,
    last_total: usize,
    last_completed: usize,
}

struct DapDebugState {
    visible: bool,
    top: usize,
    status: String,
}

impl DapDebugState {
    fn new() -> DapDebugState {
        DapDebugState {
            visible: false,
            top: 0,
            status: String::new(),
        }
    }
}

pub struct Editor {
    doc: Document,
    config: Config,
    screen: Screen,
    pane_screen: Screen,
    renderer: Renderer,
    theme: Theme,

    mode: Mode,
    running: bool,
    open_file_picker_on_start: bool,
    cols: usize,
    rows: usize,

    cursor: usize,
    anchor: usize,
    selection_linewise: bool,
    preferred_column: Option<usize>,
    top_line: usize,
    left_column: usize,

    panes: Vec<PaneState>,
    pane_root: PaneNode,
    active_pane: PaneId,
    next_pane_id: PaneId,
    documents: HashMap<u64, Document>,
    editor_left_override: Option<usize>,
    pending_window: bool,

    message: String,
    message_is_error: bool,
    command_line: String,
    command_cursor: usize,
    command_completion: CommandCompletion,
    command_files: Option<Vec<String>>,
    command_files_show_ignored: bool,
    clipboard: String,
    pending_osc: Option<Vec<u8>>,

    pending_g: bool,
    mouse_dragging: bool,
    mouse_drag_anchor: usize,
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
    picker_generation: u64,
    picker_scan: Option<PickerScanTask>,
    picker_search: Option<PickerSearchTask>,
    diff_with_source: Option<PathBuf>,
    palette_commands: Vec<PaletteCommand>,
    find: FindState,
    search_match_case: bool,
    plugins: Option<PluginManager>,
    plugin_status: Vec<(String, String)>,
    line_buffer: Vec<u8>,
    themes: ThemeRegistry,
    theme_before_picker: Option<String>,
    help: HelpState,
    about: about::AboutState,
    force_full_redraw: bool,
    sidebar: Sidebar,
    sidebar_prompt: Option<SidebarPrompt>,
    pending_sidebar_deletes: Vec<(PathBuf, bool)>,
    pending_sidebar_diff: Option<(String, String)>,
    sidebar_transfer: Option<(Vec<(PathBuf, bool)>, bool)>,
    sidebar_folders: Vec<PathBuf>,
    git_diff: diff_lines::DiffLineState,

    lsp: LspManager,
    lsp_dirty: bool,
    lsp_synced_revision: u64,
    lsp_dirty_since: std::time::Instant,
    lsp_fallback_blocked: bool,
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

    dap: crate::dap::Manager,
    breakpoints: BTreeMap<String, BTreeSet<usize>>,
    dap_status: String,
    dap_stopped_path: String,
    dap_stopped_line: i64,
    dap_stopped: bool,
    debug_path_source: String,
    debug_path: String,

    debug_variables_visible: bool,
    debug_variables_top: usize,
    debug_variable_lines: Vec<(String, crate::ui::theme::Style)>,
    debug_scopes: Vec<crate::dap::DapScope>,
    debug_scoped_variables: Vec<crate::dap::DapScopeVariables>,
    debug_watches: Vec<String>,
    debug_watch_values: HashMap<String, crate::dap::DapWatchValue>,

    dap_debug: DapDebugState,
    dap_debug_lines: Vec<String>,
    sidebar_format: Option<SidebarFormatOperation>,
    plugin_manager: plugin_manager::PluginManagerState,
    plugin_panel: Option<plugin_panels::PluginPanelState>,
    plugin_panel_dismissed: i64,
}

impl Editor {
    #[allow(dead_code)]
    pub fn new(doc: Document, cols: usize, rows: usize) -> Editor {
        Editor::with_options(doc, cols, rows, None, false)
    }

    pub fn with_options(
        doc: Document,
        cols: usize,
        rows: usize,
        workspace_root: Option<PathBuf>,
        open_file_picker: bool,
    ) -> Editor {
        let message = if doc.path.is_some() {
            format!("Opened {}", doc.display_name())
        } else {
            String::new()
        };
        let themes = ThemeRegistry::load_default();
        let theme = themes.active().clone();
        let languages = Registry::load();
        let (config, config_error) = Config::load_default();
        let mut editor = Editor {
            doc,
            config,
            screen: Screen::new(),
            pane_screen: Screen::new(),
            renderer: Renderer::new(),
            theme,
            mode: Mode::Normal,
            running: true,
            open_file_picker_on_start: open_file_picker,
            cols,
            rows,
            cursor: 0,
            anchor: 0,
            selection_linewise: false,
            preferred_column: None,
            top_line: 0,
            left_column: 0,
            panes: Vec::new(),
            pane_root: PaneNode::Leaf(1),
            active_pane: 1,
            next_pane_id: 2,
            documents: HashMap::new(),
            editor_left_override: None,
            pending_window: false,
            message,
            message_is_error: false,
            command_line: String::new(),
            command_cursor: 0,
            command_completion: CommandCompletion::new(),
            command_files: None,
            command_files_show_ignored: false,
            clipboard: String::new(),
            pending_osc: None,
            pending_g: false,
            mouse_dragging: false,
            mouse_drag_anchor: 0,
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
            picker_generation: 0,
            picker_scan: None,
            picker_search: None,
            diff_with_source: None,
            palette_commands: Vec::new(),
            find: FindState::new(),
            search_match_case: false,
            plugins: None,
            plugin_status: Vec::new(),
            line_buffer: Vec::new(),
            themes,
            theme_before_picker: None,
            help: HelpState::new(),
            about: about::AboutState::new(),
            force_full_redraw: true,
            sidebar: Sidebar::new(),
            sidebar_prompt: None,
            pending_sidebar_deletes: Vec::new(),
            pending_sidebar_diff: None,
            sidebar_transfer: None,
            sidebar_folders: Vec::new(),
            git_diff: diff_lines::DiffLineState::new(),
            lsp: LspManager::new(),
            lsp_dirty: false,
            lsp_synced_revision: u64::MAX,
            lsp_dirty_since: std::time::Instant::now(),
            lsp_fallback_blocked: false,
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
            dap: crate::dap::Manager::new(),
            breakpoints: BTreeMap::new(),
            dap_status: String::new(),
            dap_stopped_path: String::new(),
            dap_stopped_line: -1,
            dap_stopped: false,
            debug_path_source: String::new(),
            debug_path: String::new(),
            debug_variables_visible: false,
            debug_variables_top: 0,
            debug_variable_lines: Vec::new(),
            debug_scopes: Vec::new(),
            debug_scoped_variables: Vec::new(),
            debug_watches: Vec::new(),
            debug_watch_values: HashMap::new(),
            dap_debug: DapDebugState::new(),
            dap_debug_lines: Vec::new(),
            sidebar_format: None,
            plugin_manager: plugin_manager::PluginManagerState::new(),
            plugin_panel: None,
            plugin_panel_dismissed: -1,
        };
        editor.last_revision = editor.doc.revision();
        editor.sidebar.show_modified = editor.config.show_diff_lines;
        editor.sidebar.show_git_ignored = editor.config.show_git_ignored;
        editor.search_match_case = editor.config.search_match_case_enabled;
        if let Some(root) = workspace_root {
            editor.sidebar.set_root(&root);
        }
        editor.initialize_panes();
        editor.refresh_language();
        editor.refresh_git_diff();
        editor.load_plugins();
        if !config_error.is_empty() {
            editor.set_error(&format!("config: {config_error}"));
        }
        editor
    }

    pub fn run(&mut self, terminal: &mut Terminal) -> io::Result<()> {
        let mut last_width = terminal.width;
        let mut last_height = terminal.height;

        if self.open_file_picker_on_start {
            self.open_file_picker_on_start = false;
            self.open_picker(PickerKind::Files);
        }

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

            match terminal.read_event(
                if self
                    .plugins
                    .as_ref()
                    .map(|plugins| plugins.has_due_deferred_tasks())
                    .unwrap_or(false)
                {
                    1
                } else {
                    30
                },
            ) {
                Ok(Some(event)) => {
                    let cursor_before = self.cursor;
                    self.handle_event(event);
                    if self.doc.revision() != self.last_revision {
                        self.last_revision = self.doc.revision();
                        self.lsp_dirty = self.lsp.is_running();
                        self.lsp_dirty_since = std::time::Instant::now();
                        if self
                            .plugins
                            .as_ref()
                            .map(|plugins| plugins.has_subscribers("buffer_changed"))
                            .unwrap_or(false)
                        {
                            self.fire_plugin_buffer_event("buffer_changed");
                        }
                    }
                    if self.cursor != cursor_before
                        && self
                            .plugins
                            .as_ref()
                            .map(|plugins| plugins.has_subscribers("cursor_moved"))
                            .unwrap_or(false)
                    {
                        let line = self.doc.line_of_offset(self.cursor);
                        let start = self.doc.line_start(line);
                        let column = String::from_utf8_lossy(
                            &self.doc.range(start, self.cursor.saturating_sub(start)),
                        )
                        .chars()
                        .count();
                        let mut data = crate::util::json::Value::Object(Vec::new());
                        data.set("line", crate::util::json::Value::Number(line as f64));
                        data.set("column", crate::util::json::Value::Number(column as f64));
                        self.fire_plugin_event("cursor_moved", data);
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
            self.poll_plugin_tasks();
        }
        self.fire_plugin_event(
            "editor_shutdown",
            crate::util::json::Value::Object(Vec::new()),
        );
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        if self.plugin_manager.visible {
            self.handle_plugin_manager_input(event);
            return;
        }
        if self.plugin_panel.is_some() {
            self.handle_plugin_panel_input(event);
            return;
        }
        if self.lsp_debug.visible {
            self.handle_lsp_debug_input(event);
            return;
        }
        if self.dap_debug.visible {
            self.handle_dap_debug_input(event);
            return;
        }
        if self.hover.visible {
            self.handle_hover_input(event);
            return;
        }
        if self.about.visible {
            self.handle_about_input(event);
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
        if self.pending_window {
            self.pending_window = false;
            self.handle_pane_command(event);
            return;
        }
        if matches!(
            event.key,
            Key::MouseDown | Key::MouseUp | Key::MouseMove | Key::ScrollUp | Key::ScrollDown
        ) {
            if matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Select) {
                self.handle_editor_mouse(event);
            }
            return;
        }
        if event.key == Key::Char
            && event.ctrl
            && matches!(event.ch, 'e' | 'E')
            && matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Select)
        {
            self.toggle_breakpoint();
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
        if event.key == Key::Char
            && event.ctrl
            && matches!(event.ch, 'w' | 'W')
            && matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Select)
        {
            self.pending_window = true;
            self.set_message("Pane: arrows/hjkl focus, +/- resize, q close, Esc cancel");
            return;
        }
        if matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Select) {
            let mode = match self.mode {
                Mode::Insert => "insert",
                Mode::Select => "select",
                _ => "normal",
            };
            if let Some(handled) = self.try_plugin_key(&event, mode) {
                if handled {
                    return;
                }
            }
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
