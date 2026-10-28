//! Gutter markers for line-level Git changes.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::ui::theme::Style;
use crate::util::git::{self, GitDiffLineKind};

const DIFF_MARKER: char = '▎';

pub(super) struct DiffLineState {
    path: String,
    lines: BTreeMap<usize, GitDiffLineKind>,
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
    pub(super) fn refresh_git_diff(&mut self) {
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
        editor
            .git_diff
            .lines
            .insert(2, GitDiffLineKind::Modified);
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
        editor
            .git_diff
            .lines
            .insert(1, GitDiffLineKind::Modified);
        let digits = crate::util::digit_count(editor.doc.line_count());
        assert_eq!(editor.gutter_width(), digits + 2);
        editor.render_line_number(1, 1);
        assert_eq!(
            editor.screen.at(editor.editor_left(), 1).ch,
            '▎'
        );

        editor.sidebar.show_modified = false;
        assert_eq!(editor.gutter_width(), digits + 1);
    }
}
