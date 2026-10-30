//! Gutter markers for line-level Git changes.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::ui::theme::Style;
use crate::util::git::{self, GitDiffLineKind};

const DIFF_MARKER: char = '▎';

#[derive(Clone)]
pub(super) struct DiffLineState {
    path: String,
    lines: BTreeMap<usize, GitDiffLineKind>,
}

pub(super) struct GitDiffTask {
    pub(super) receiver: mpsc::Receiver<(u64, String, BTreeMap<usize, GitDiffLineKind>)>,
}

impl DiffLineState {
    pub(super) fn new() -> DiffLineState {
        DiffLineState {
            path: String::new(),
            lines: BTreeMap::new(),
        }
    }
}

impl Editor {
    /// Recompute gutter markers for every pane, not just the active one.
    ///
    /// This path is only used when reloading all files, so it keeps the
    /// synchronous implementation: each pane's markers must be captured
    /// before moving on to the next pane.
    pub(super) fn refresh_all_pane_diff_lines(&mut self) {
        self.git_diff_generation = self.git_diff_generation.wrapping_add(1);
        self.git_diff_task = None;
        self.git_diff_pending = false;
        let active = self.active_pane;
        self.capture_pane_state(active);
        let panes: Vec<PaneId> = self.panes.iter().map(|pane| pane.id).collect();
        for pane in panes {
            self.load_pane_state(pane);
            self.refresh_git_diff_blocking();
            self.capture_pane_state(pane);
        }
        self.load_pane_state(active);
    }

    /// Queue a background recomputation of the gutter markers. `git diff` can
    /// be slow on Windows and large repositories, so opening a file must not
    /// wait for it.
    pub(super) fn refresh_git_diff(&mut self) {
        self.git_diff_generation = self.git_diff_generation.wrapping_add(1);
        if !self.sidebar.show_modified {
            self.git_diff.path.clear();
            self.git_diff.lines.clear();
            self.git_diff_pending = false;
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            self.git_diff.path.clear();
            self.git_diff.lines.clear();
            self.git_diff_pending = false;
            return;
        };
        if self.git_diff.path != path {
            self.git_diff.lines.clear();
        }
        self.git_diff.path = path;
        if self.git_diff_task.is_some() {
            self.git_diff_pending = true;
            return;
        }
        self.start_git_diff_task();
    }

    fn start_git_diff_task(&mut self) {
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        let generation = self.git_diff_generation;
        let line_count = self.doc.line_count();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let lines = git::load_diff_lines(Path::new(&path), line_count);
            let _ = sender.send((generation, path, lines));
        });
        self.git_diff_task = Some(diff_lines::GitDiffTask { receiver });
        self.git_diff_pending = false;
    }

    pub(super) fn poll_git_diff(&mut self) {
        let Some(received) = self
            .git_diff_task
            .as_ref()
            .map(|task| task.receiver.try_recv())
        else {
            return;
        };
        match received {
            Ok((generation, path, lines)) => {
                self.git_diff_task = None;
                if generation == self.git_diff_generation {
                    if self.git_diff.path == path {
                        self.git_diff.lines = lines;
                    }
                    self.git_diff_pending = false;
                } else if self.git_diff_pending {
                    self.git_diff_pending = false;
                    self.refresh_git_diff();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_diff_task = None;
                if self.git_diff_pending {
                    self.git_diff_pending = false;
                    self.refresh_git_diff();
                }
            }
        }
    }

    fn refresh_git_diff_blocking(&mut self) {
        if !self.sidebar.show_modified {
            self.git_diff.path.clear();
            self.git_diff.lines.clear();
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            self.git_diff.path.clear();
            self.git_diff.lines.clear();
            return;
        };
        self.git_diff.path = path.clone();
        self.git_diff.lines = git::load_diff_lines(Path::new(&path), self.doc.line_count());
    }

    pub(super) fn jump_to_diff_line(&mut self, forward: bool) {
        self.refresh_git_diff_blocking();
        if !self.sidebar.show_modified {
            self.set_message("Git diff lines are hidden.");
            return;
        }
        if self.git_diff.lines.is_empty() {
            self.set_message("No git diff lines.");
            return;
        }
        let current = self.cursor_line();
        let target = if forward {
            self.git_diff
                .lines
                .keys()
                .copied()
                .filter(|line| *line > current)
                .min()
        } else {
            self.git_diff
                .lines
                .keys()
                .copied()
                .filter(|line| *line < current)
                .max()
        };
        let Some(target) = target else {
            self.set_message(if forward {
                "No next git diff line."
            } else {
                "No previous git diff line."
            });
            return;
        };
        let target = target.min(self.doc.line_count().saturating_sub(1));
        self.cursor = self.doc.line_start(target);
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.extra_selections.clear();
        self.preferred_column = None;
        self.snap_cursor();
        self.ensure_cursor_visible();
        self.set_message(&format!("Git diff line {}", target + 1));
    }

    pub(super) fn git_diff_marker(&self, line: usize) -> Option<(char, Style)> {
        if !self.sidebar.show_modified {
            return None;
        }
        let kind = *self.git_diff.lines.get(&line)?;
        let style = match kind {
            GitDiffLineKind::Added => Style {
                fg: self.theme.diff_added,
                bg: self.theme.background,
                bold: true,
                ..Style::default()
            },
            GitDiffLineKind::Deleted => Style {
                fg: self.theme.diff_deleted,
                bg: self.theme.background,
                bold: true,
                ..Style::default()
            },
            GitDiffLineKind::Modified => self.theme.diff_modified_style(),
        };
        Some((DIFF_MARKER, style))
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;
    use crate::core::document::Document;
    use crate::util::git::GitDiffLineKind;

    #[test]
    fn diff_markers_follow_the_gdl_setting() {
        let mut editor = Editor::new(Document::new(), 80, 24);
        editor.sidebar.show_modified = true;
        editor.git_diff.lines.insert(2, GitDiffLineKind::Modified);
        assert!(editor.git_diff_marker(2).is_some());
        editor.sidebar.show_modified = false;
        assert!(editor.git_diff_marker(2).is_none());
    }

    #[test]
    fn gutter_reserves_and_draws_the_marker_column() {
        let mut editor = Editor::new(
            Document::from_bytes(None, b"one\ntwo\nthree\n".to_vec()),
            80,
            24,
        );
        editor.screen.resize(80, 24);
        editor.sidebar.show_modified = true;
        editor.git_diff.lines.insert(1, GitDiffLineKind::Modified);
        let digits = crate::util::digit_count(editor.doc.line_count());
        assert_eq!(editor.gutter_width(), digits + 2);
        editor.render_line_number(1, 1);
        assert_eq!(editor.screen.at(editor.editor_left(), 1).ch, '▎');

        editor.sidebar.show_modified = false;
        assert_eq!(editor.gutter_width(), digits + 1);
    }
}
