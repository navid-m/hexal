//! Command palette.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn open_palette(&mut self) {
        self.rebuild_palette_commands();
        self.open_picker(PickerKind::Palette);
    }

    fn rebuild_palette_commands(&mut self) {
        self.palette_commands.clear();

        self.add_palette_command(
            "Open file picker",
            "Search the workspace and open a file.",
            PaletteAction::OpenFiles,
        );
        self.add_palette_command(
            "Search across files",
            "Search file contents in the workspace.",
            PaletteAction::SearchFiles,
        );
        self.add_palette_command(
            "Open active .hexcfg",
            "Open the active Hexal configuration, creating it with the default schema when missing.",
            PaletteAction::OpenConfig,
        );
        self.add_palette_command(
            "Go to line",
            "Jump to a line number in the current buffer.",
            PaletteAction::GotoLine,
        );
        self.add_palette_command(
            "Open theme selector",
            "Search and preview installed themes.",
            PaletteAction::OpenThemes,
        );
        self.add_palette_command(
            "Toggle sidebar",
            "Show or hide the file sidebar.",
            PaletteAction::ToggleSidebar,
        );
        self.add_palette_command(
            "Toggle git-ignored files",
            "Show or hide files matched by .gitignore in the sidebar and pickers.",
            PaletteAction::ToggleGitIgnored,
        );
        self.add_palette_command(
            "Toggle git diff lines",
            "Show or hide added, deleted and modified line markers in the gutter.",
            PaletteAction::ToggleDiffLines,
        );
        self.add_palette_command(
            "Toggle search match case",
            "Make all searches case-sensitive or case-insensitive.",
            PaletteAction::ToggleSearchMatchCase,
        );
        self.add_palette_command(
            "Format document",
            "Format the current buffer with its language formatter.",
            PaletteAction::FormatDocument,
        );
        self.add_palette_command(
            "LSP: Show hover",
            "Show language-server hover information at the cursor.",
            PaletteAction::Hover,
        );
        self.add_palette_command(
            "LSP: Go to definition",
            "Jump to the definition of the symbol under the cursor.",
            PaletteAction::GoToDefinition,
        );
        self.add_palette_command(
            "LSP: Code actions",
            "List code actions at the cursor.",
            PaletteAction::CodeActions,
        );
        self.add_palette_command(
            "LSP: Document symbols",
            "Pick a symbol from the current document.",
            PaletteAction::DocumentSymbols,
        );
        self.add_palette_command(
            "LSP: Document diagnostics",
            "Show diagnostics for the current document.",
            PaletteAction::DocumentDiagnostics,
        );
        self.add_palette_command(
            "LSP: Workspace diagnostics",
            "Show diagnostics for the whole workspace.",
            PaletteAction::WorkspaceDiagnostics,
        );
        self.add_palette_command(
            "Clean trailing whitespace",
            "Remove trailing spaces and tabs from every line.",
            PaletteAction::CleanWhitespace,
        );
        self.add_palette_command(
            "Save file",
            "Write the current buffer to disk.",
            PaletteAction::Save,
        );
        self.add_palette_command(
            "Save and quit",
            "Write the current buffer, then quit.",
            PaletteAction::SaveAndQuit,
        );
        self.add_palette_command(
            "Reload themes",
            "Reload installed themes from disk.",
            PaletteAction::ReloadThemes,
        );
        self.add_palette_command(
            "Show key help",
            "Show a short reminder of the available keys.",
            PaletteAction::ShowHelp,
        );
        self.add_palette_command(
            "About Hexal",
            "Show the Hexal title, author and ASCII branding.",
            PaletteAction::About,
        );
        self.add_palette_command(
            "Show current path",
            "Show the current buffer's absolute path.",
            PaletteAction::ShowPath,
        );
        self.add_palette_command(
            "Quit",
            "Quit Hexal; unsaved changes are confirmed first.",
            PaletteAction::Quit,
        );

        let plugin_commands = self
            .lua
            .as_ref()
            .map(|runtime| runtime.command_names())
            .unwrap_or_default();
        for name in plugin_commands {
            self.palette_commands.push(PaletteCommand {
                title: format!("Plugin: {name}"),
                description: format!("Run the plugin command :{name}"),
                action: PaletteAction::Plugin(name),
            });
        }
    }

    fn add_palette_command(&mut self, title: &str, description: &str, action: PaletteAction) {
        self.palette_commands.push(PaletteCommand {
            title: title.to_string(),
            description: description.to_string(),
            action,
        });
    }

    pub(super) fn run_palette_command(&mut self, index: usize) {
        let Some(action) = self
            .palette_commands
            .get(index)
            .map(|command| command.action.clone())
        else {
            return;
        };

        match action {
            PaletteAction::OpenFiles => self.open_picker(PickerKind::Files),
            PaletteAction::SearchFiles => self.open_picker(PickerKind::Search),
            PaletteAction::OpenConfig => self.open_config_file(),
            PaletteAction::GotoLine => self.open_goto_prompt(),
            PaletteAction::OpenThemes => self.open_picker(PickerKind::Themes),
            PaletteAction::ToggleSidebar => self.toggle_sidebar(),
            PaletteAction::ToggleGitIgnored => self.toggle_git_ignored(),
            PaletteAction::ToggleDiffLines => self.toggle_diff_lines(),
            PaletteAction::ToggleSearchMatchCase => self.toggle_search_match_case(),
            PaletteAction::FormatDocument => self.format_current_document(false),
            PaletteAction::Hover => self.show_hover(),
            PaletteAction::GoToDefinition => self.go_to_definition(),
            PaletteAction::CodeActions => self.show_code_actions(),
            PaletteAction::DocumentSymbols => self.show_document_symbols(),
            PaletteAction::DocumentDiagnostics => self.show_problems(false),
            PaletteAction::WorkspaceDiagnostics => self.show_problems(true),
            PaletteAction::CleanWhitespace => self.trim_trailing_whitespace(),
            PaletteAction::Save => {
                self.save_document(None);
            }
            PaletteAction::SaveAndQuit => {
                if self.save_document(None) {
                    self.running = false;
                }
            }
            PaletteAction::ReloadThemes => self.reload_themes(),
            PaletteAction::ShowHelp => {
                self.help.open();
                self.force_full_redraw = true;
                self.clear_message();
            }
            PaletteAction::About => self.open_about(),
            PaletteAction::ShowPath => self.show_current_path(),
            PaletteAction::Quit => self.request_quit(false),
            PaletteAction::Plugin(name) => {
                if !self.try_lua_command(&name) {
                    self.set_error(&format!("plugin command ':{name}' is not available"));
                }
            }
        }
    }

    pub(super) fn toggle_git_ignored(&mut self) {
        self.sidebar.show_git_ignored = !self.sidebar.show_git_ignored;
        self.sidebar.refresh(None);
        self.force_full_redraw = true;
        self.persist_display_settings();
        self.set_message(if self.sidebar.show_git_ignored {
            "Showing git-ignored files"
        } else {
            "Hiding git-ignored files"
        });
    }

    pub(super) fn toggle_diff_lines(&mut self) {
        self.sidebar.show_modified = !self.sidebar.show_modified;
        self.sidebar.refresh(None);
        self.force_full_redraw = true;
        self.persist_display_settings();
        self.set_message(if self.sidebar.show_modified {
            "Showing Git markers"
        } else {
            "Hiding Git markers"
        });
    }

    pub(super) fn reload_configuration(&mut self) {
        let (config, config_error) = Config::load_default();
        self.config = config;
        self.sidebar.show_modified = self.config.show_diff_lines;
        self.sidebar.show_git_ignored = self.config.show_git_ignored;
        self.search_match_case = self.config.search_match_case_enabled;
        self.languages = Registry::load();
        self.refresh_language();
        self.reload_themes();
        self.lsp.stop();
        self.lsp_fallback_blocked = false;
        self.restart_lsp_for_buffer();
        self.sidebar.refresh(None);
        self.force_full_redraw = true;
        if !config_error.is_empty() {
            self.set_error(&format!("config: {config_error}"));
        } else if let Some(error) = self.languages.errors().first() {
            self.set_error(&format!("grammar: {error}"));
        } else {
            self.set_message("Reloaded configuration, grammars, themes and language servers.");
        }
    }

    pub(super) fn open_config_file(&mut self) {
        match Config::ensure_default_file() {
            Ok(path) => {
                self.request_open(&path.to_string_lossy(), 0, 0, false);
            }
            Err(error) => self.set_error(&format!("config: {error}")),
        }
    }

    pub(super) fn persist_display_settings(&mut self) {
        let path = self.config.source_path.clone();
        if let Err(error) = Config::update_display_settings(
            &path,
            self.sidebar.show_modified,
            self.sidebar.show_git_ignored,
            self.search_match_case,
        ) {
            self.set_error(&format!("config: {error}"));
        }
    }

    fn show_current_path(&mut self) {
        match &self.doc.path {
            Some(path) => {
                let absolute = std::fs::canonicalize(path)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.clone());
                self.set_message(&absolute);
            }
            None => self.set_message("[No Name]"),
        }
    }
}
