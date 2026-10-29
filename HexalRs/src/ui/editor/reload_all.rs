//! Reload every file-backed buffer from disk (`:rla` / `:rla!`).
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
struct ReloadPosition {
    cursor_line: usize,
    cursor_byte: usize,
    anchor_line: usize,
    anchor_byte: usize,
    top_line: usize,
    left_column: usize,
}

impl Editor {
    pub(super) fn reload_all(&mut self, force: bool) {
        self.capture_pane_state(self.active_pane);

        let active_document_id = self.doc.id();
        let mut positions: HashMap<PaneId, ReloadPosition> = HashMap::new();
        for pane in &self.panes {
            let Some(document) = self.document_for_id(pane.doc_id) else {
                continue;
            };
            positions.insert(
                pane.id,
                ReloadPosition {
                    cursor_line: line_for_offset(document, pane.cursor),
                    cursor_byte: byte_column_for_offset(document, pane.cursor),
                    anchor_line: line_for_offset(document, pane.anchor),
                    anchor_byte: byte_column_for_offset(document, pane.anchor),
                    top_line: pane.top_line,
                    left_column: pane.left_column,
                },
            );
        }

        let mut seen: HashSet<u64> = HashSet::new();
        let mut replacements: HashMap<u64, Document> = HashMap::new();
        let mut reloaded = 0usize;
        let mut kept_dirty = 0usize;
        let mut first_error = String::new();

        for pane in &self.panes {
            let document_id = pane.doc_id;
            if !seen.insert(document_id) {
                continue;
            }
            let Some(document) = self.document_for_id(document_id) else {
                continue;
            };
            let Some(path) = document.path.clone() else {
                continue;
            };

            if !force && document.is_dirty() {
                kept_dirty += 1;
                continue;
            }

            if !Path::new(&path).exists() {
                if first_error.is_empty() {
                    first_error = format!("'{path}' no longer exists");
                }
                continue;
            }

            match Document::open(&path) {
                Ok(mut loaded) => {
                    copy_indentation_settings(document, &mut loaded);
                    replacements.insert(document_id, loaded);
                    reloaded += 1;
                }
                Err(error) => {
                    if first_error.is_empty() {
                        first_error = format!("cannot open '{path}': {error}");
                    }
                }
            }
        }

        for pane in &mut self.panes {
            let Some(loaded) = replacements.get(&pane.doc_id) else {
                continue;
            };
            let new_document_id = loaded.id();
            pane.doc_id = new_document_id;
            pane.language_override = None;
            pane.mode = Mode::Normal;
            pane.pending_g = false;
            pane.find.active = false;
            pane.find.editing = false;
            pane.find.matches.clear();
            pane.find.index = None;
            pane.preferred_column = None;
            pane.highlighter = Highlighter::default();
            pane.git_diff = diff_lines::DiffLineState::new();

            if let Some(position) = positions.get(&pane.id).copied() {
                pane.cursor =
                    offset_for_line_byte(loaded, position.cursor_line, position.cursor_byte);
                pane.anchor =
                    offset_for_line_byte(loaded, position.anchor_line, position.anchor_byte);
                pane.top_line = position.top_line.min(loaded.line_count().saturating_sub(1));
                pane.left_column = position.left_column;
            }
        }

        self.documents.remove(&active_document_id);
        for (old_document_id, loaded) in replacements {
            if old_document_id == active_document_id {
                self.doc = loaded;
                continue;
            }
            self.documents.remove(&old_document_id);
            self.documents.insert(loaded.id(), loaded);
        }

        self.load_pane_state(self.active_pane);

        self.refresh_all_pane_diff_lines();
        self.restart_lsp_for_buffer();
        self.plugin_status.retain(|(name, _)| name != "lsp");
        self.fire_plugin_buffer_event("buffer_opened");
        self.cancel_picker_work();
        self.command_files = None;
        self.command_files_show_ignored = false;
        self.picker.files_loaded = false;
        self.picker.files = Arc::new(Vec::new());

        if self.sidebar.visible {
            let selected = self.sidebar.selected_node().map(|node| node.path.clone());
            self.sidebar.refresh(selected.as_deref());
        }

        self.renderer.reset();
        self.screen = Screen::new();
        self.force_full_redraw = true;
        let reset = b"\x1b[0m\x1b[r\x1b[2J\x1b[H";
        match &mut self.pending_osc {
            Some(sequence) => sequence.extend_from_slice(reset),
            None => self.pending_osc = Some(reset.to_vec()),
        }
        self.snap_cursor();
        self.ensure_cursor_visible();

        if !first_error.is_empty() {
            self.set_error(&format!("Reloaded {reloaded} file(s); {first_error}."));
        } else if reloaded == 0 && kept_dirty > 0 {
            self.set_message(&format!(
                "Kept {kept_dirty} modified file(s); nothing reloaded."
            ));
        } else if kept_dirty > 0 {
            self.set_message(&format!(
                "Reloaded {reloaded} file(s); kept {kept_dirty} modified."
            ));
        } else if reloaded == 0 {
            self.set_message("No file-backed buffers to reload.");
        } else {
            self.set_message(&format!("Reloaded {reloaded} file(s)."));
        }
    }

    fn document_for_id(&self, id: u64) -> Option<&Document> {
        if id == self.doc.id() {
            Some(&self.doc)
        } else {
            self.documents.get(&id)
        }
    }
}

fn line_for_offset(document: &Document, offset: usize) -> usize {
    document.line_of_offset(offset.min(document.size()))
}

fn byte_column_for_offset(document: &Document, offset: usize) -> usize {
    let offset = offset.min(document.size());
    let line = document.line_of_offset(offset);
    let start = document.line_start(line);
    offset.saturating_sub(start)
}

fn offset_for_line_byte(document: &Document, line: usize, byte_column: usize) -> usize {
    let line = line.min(document.line_count().saturating_sub(1));
    let start = document.line_start(line);
    let end = document.line_end(line);
    start.saturating_add(byte_column).clamp(start, end)
}

fn copy_indentation_settings(source: &Document, target: &mut Document) {
    target.set_indentation(source.use_tabs(), source.indent_width(), source.tab_width());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(1);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> TempDir {
            let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("hexal-reload-all-{}-{unique}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }

        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn editor_with(path: &Path) -> Editor {
        let document = Document::open(path.to_string_lossy().as_ref()).unwrap();
        Editor::new(document, 100, 30)
    }

    #[test]
    fn reloads_clean_file_and_preserves_cursor() {
        let temp = TempDir::new();
        let original: String = (0..100).map(|line| format!("line {line}\n")).collect();
        let path = temp.file("a.txt", &original);
        let mut editor = editor_with(&path);
        let cursor_line = 42;
        editor.cursor = editor.doc.line_start(cursor_line) + 2;
        editor.anchor = editor.cursor;
        editor.top_line = 40;

        let replacement: String = (0..100).map(|line| format!("LINE {line}\n")).collect();
        fs::write(&path, replacement).unwrap();
        editor.reload_all(false);

        let expected: String = (0..100).map(|line| format!("LINE {line}\n")).collect();
        assert_eq!(editor.doc.all_text(), expected.as_bytes());
        assert_eq!(editor.cursor, editor.doc.line_start(cursor_line) + 2);
        assert_eq!(editor.top_line, 40);
        assert_eq!(editor.message, "Reloaded 1 file(s).");
        assert!(!editor.doc.is_dirty());
    }

    #[test]
    fn dirty_buffer_is_kept_without_force() {
        let temp = TempDir::new();
        let path = temp.file("a.txt", "old\n");
        let mut editor = editor_with(&path);
        editor.doc.insert(0, b"unsaved ");

        fs::write(&path, "disk\n").unwrap();
        editor.reload_all(false);

        assert_eq!(editor.doc.all_text(), b"unsaved old\n");
        assert!(editor.doc.is_dirty());
        assert_eq!(editor.message, "Kept 1 modified file(s); nothing reloaded.");
    }

    #[test]
    fn force_discards_dirty_buffer() {
        let temp = TempDir::new();
        let path = temp.file("a.txt", "old\n");
        let mut editor = editor_with(&path);
        editor.doc.insert(0, b"unsaved ");

        fs::write(&path, "disk\n").unwrap();
        editor.reload_all(true);

        assert_eq!(editor.doc.all_text(), b"disk\n");
        assert_eq!(editor.message, "Reloaded 1 file(s).");
    }

    #[test]
    fn shared_document_is_reloaded_once_and_stays_shared() {
        let temp = TempDir::new();
        let path = temp.file("a.txt", "one\ntwo\n");
        let mut editor = editor_with(&path);
        editor.cursor = editor.doc.line_start(1);
        editor.split_pane(PaneSplitOrientation::Vertical);
        let second = editor.active_pane;

        fs::write(&path, "ONE\nTWO\n").unwrap();
        editor.reload_all(false);

        assert_eq!(editor.message, "Reloaded 1 file(s).");
        assert_eq!(editor.pane_count(), 2);
        assert_eq!(editor.doc.all_text(), b"ONE\nTWO\n");
        assert!(
            editor
                .panes
                .iter()
                .all(|pane| pane.doc_id == editor.doc.id())
        );
        assert_eq!(
            editor
                .panes
                .iter()
                .find(|pane| pane.id == second)
                .unwrap()
                .cursor,
            editor.doc.line_start(1)
        );
    }

    #[test]
    fn reloads_distinct_documents_across_panes() {
        let temp = TempDir::new();
        let first_path = temp.file("a.txt", "a old\n");
        let second_path = temp.file("b.txt", "b old\n");
        let mut editor = editor_with(&first_path);
        let first_pane = editor.active_pane;

        editor.split_pane(PaneSplitOrientation::Vertical);
        let second_pane = editor.active_pane;
        let second_document = Document::open(second_path.to_string_lossy().as_ref()).unwrap();
        editor.assign_active_document(second_document);
        editor.capture_pane_state(second_pane);

        fs::write(&first_path, "a new\n").unwrap();
        fs::write(&second_path, "b new\n").unwrap();
        editor.reload_all(true);

        assert_eq!(editor.message, "Reloaded 2 file(s).");
        assert_eq!(editor.doc.all_text(), b"b new\n");
        let first_document_id = editor
            .panes
            .iter()
            .find(|pane| pane.id == first_pane)
            .unwrap()
            .doc_id;
        assert_eq!(editor.documents[&first_document_id].all_text(), b"a new\n");
    }

    #[test]
    fn missing_file_reports_an_error_and_leaves_the_buffer_alone() {
        let temp = TempDir::new();
        let path = temp.file("gone.txt", "still here\n");
        let mut editor = editor_with(&path);
        fs::remove_file(&path).unwrap();

        editor.reload_all(true);

        assert_eq!(editor.doc.all_text(), b"still here\n");
        assert!(editor.message_is_error);
        assert!(
            editor.message.contains("no longer exists"),
            "{}",
            editor.message
        );
    }

    #[test]
    fn command_dispatch_rejects_arguments_and_honours_force() {
        let temp = TempDir::new();
        let path = temp.file("a.txt", "old\n");
        let mut editor = editor_with(&path);
        editor.doc.insert(0, b"dirty ");

        editor.command_line = "rla extra".into();
        editor.submit_command();
        assert_eq!(editor.message, "E: usage: :rla[!]");

        editor.command_line = "rla!".into();
        editor.submit_command();
        assert_eq!(editor.doc.all_text(), b"old\n");
        assert_eq!(editor.message, "Reloaded 1 file(s).");
    }
}
