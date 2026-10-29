//! Navigation between workspace search hits.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    /// Records the current search-picker results so that `n`/`N` can keep
    /// walking through them after the picker closes.
    pub(super) fn begin_workspace_search_session(&mut self) {
        let root = self.sidebar.root.clone();
        let files = self.picker.files.clone();
        let query = self.picker.query.clone();
        let hits: Vec<WorkspaceSearchHit> = self
            .picker
            .items
            .iter()
            .filter_map(|item| {
                let relative = files.get(item.file_index)?;
                let path = root.join(relative).to_string_lossy().into_owned();
                Some(WorkspaceSearchHit {
                    path,
                    line: item.line,
                    column: item.column,
                })
            })
            .collect();

        if hits.is_empty() {
            self.workspace_search = None;
            return;
        }

        let index = self.picker.selected.min(hits.len() - 1);
        self.workspace_search = Some(WorkspaceSearchState { query, hits, index });
    }

    pub(super) fn workspace_search_step(&mut self, forward: bool) {
        let Some(state) = self.workspace_search.as_mut() else {
            return;
        };
        let length = state.hits.len();
        if length == 0 {
            self.workspace_search = None;
            return;
        }

        state.index = if forward {
            (state.index + 1) % length
        } else {
            (state.index + length - 1) % length
        };
        let hit = state.hits[state.index].clone();
        let position = state.index + 1;
        let total = length;
        let query = state.query.clone();

        self.record_jump();
        let same = self
            .doc
            .path
            .as_deref()
            .is_some_and(|current| git::same_path(Path::new(current), Path::new(&hit.path)));
        if same {
            self.move_to_search_hit(hit.line, hit.column);
        } else {
            self.request_open(&hit.path, hit.line, hit.column, false);
        }

        let name = Path::new(&hit.path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| hit.path.clone());
        self.set_message(&format!(
            "Match {position}/{total} for '{query}'  {name}:{}",
            hit.line + 1
        ));
    }

    fn move_to_search_hit(&mut self, line: usize, column: usize) {
        let line = line.min(self.doc.line_count().saturating_sub(1));
        let start = self.doc.line_start(line);
        let text = self.doc.line_text(line);
        let mut local = column.min(text.len());
        while local < text.len() && (text[local] & 0xC0) == 0x80 {
            local += 1;
        }
        self.cursor = (start + local).min(self.doc.line_end(line));
        self.anchor = self.cursor;
        self.selection_linewise = false;
        self.mode = Mode::Normal;
        self.preferred_column = None;
        self.top_line = line.saturating_sub(self.active_pane_rows() / 2);
        self.doc.break_undo_group();
        self.snap_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::Document;

    #[test]
    fn workspace_search_wraps_between_hits() {
        let mut editor = Editor::new(
            Document::from_bytes(Some("test.rs".into()), b"one\ntwo\nthree\n".to_vec()),
            80,
            24,
        );
        editor.workspace_search = Some(WorkspaceSearchState {
            query: "t".into(),
            hits: vec![
                WorkspaceSearchHit {
                    path: "test.rs".into(),
                    line: 0,
                    column: 0,
                },
                WorkspaceSearchHit {
                    path: "test.rs".into(),
                    line: 2,
                    column: 0,
                },
            ],
            index: 0,
        });

        editor.workspace_search_step(true);
        assert_eq!(editor.workspace_search.as_ref().unwrap().index, 1);
        assert_eq!(editor.cursor_line(), 2);
        editor.workspace_search_step(true);
        assert_eq!(editor.workspace_search.as_ref().unwrap().index, 0);
        assert_eq!(editor.cursor_line(), 0);
    }
}
