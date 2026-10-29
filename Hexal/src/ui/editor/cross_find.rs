//! Find and replace across workspace files.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

const MAX_CROSS_FIND_MATCHES: usize = 50_000;
const CROSS_FIND_DIALOG_WIDTH: usize = 80;
const CROSS_FIND_DIALOG_HEIGHT: usize = 7;
const CROSS_FIND_MAX_FILES: usize = 20_000;

impl Editor {
    pub(super) fn open_cross_find(&mut self, regex: bool) {
        self.close_cross_find();
        self.cross_find.active = true;
        self.cross_find.regex = regex;
        self.cross_find.field = 0;
        self.cross_find.pattern.clear();
        self.cross_find.pattern.push('*');
        self.cross_find.pattern_cursor = 1;
        self.cross_find.query.clear();
        self.cross_find.query_cursor = 0;
        self.cross_find.replacement.clear();
        self.cross_find.replacement_cursor = 0;
        self.cross_find.pattern_error.clear();
        self.cross_find.query_error.clear();
        self.cross_find.matched_files = 0;
        self.pending_g = false;
        self.doc.break_undo_group();
        self.refresh_cross_find_file_count();
        self.clear_message();
    }

    pub(super) fn close_cross_find(&mut self) {
        self.cross_find.active = false;
        self.cross_find.regex = false;
        self.cross_find.field = 0;
        self.cross_find.pattern.clear();
        self.cross_find.pattern_cursor = 0;
        self.cross_find.query.clear();
        self.cross_find.query_cursor = 0;
        self.cross_find.replacement.clear();
        self.cross_find.replacement_cursor = 0;
        self.cross_find.matched_files = 0;
        self.cross_find.pattern_error.clear();
        self.cross_find.query_error.clear();
        self.doc.break_undo_group();
    }

    fn cross_find_buffer(&self) -> &String {
        match self.cross_find.field {
            0 => &self.cross_find.pattern,
            1 => &self.cross_find.query,
            _ => &self.cross_find.replacement,
        }
    }

    fn cross_find_buffer_mut(&mut self) -> &mut String {
        match self.cross_find.field {
            0 => &mut self.cross_find.pattern,
            1 => &mut self.cross_find.query,
            _ => &mut self.cross_find.replacement,
        }
    }

    fn cross_find_cursor(&self) -> usize {
        match self.cross_find.field {
            0 => self.cross_find.pattern_cursor,
            1 => self.cross_find.query_cursor,
            _ => self.cross_find.replacement_cursor,
        }
    }

    fn set_cross_find_cursor(&mut self, value: usize) {
        match self.cross_find.field {
            0 => self.cross_find.pattern_cursor = value,
            1 => self.cross_find.query_cursor = value,
            _ => self.cross_find.replacement_cursor = value,
        }
    }

    pub(super) fn handle_cross_find(&mut self, event: Event) {
        if event.key == Key::Escape {
            self.close_cross_find();
            return;
        }

        match event.key {
            Key::Tab => {
                self.cross_find.field = (self.cross_find.field + 1) % 3;
                return;
            }
            Key::BackTab => {
                self.cross_find.field = (self.cross_find.field + 2) % 3;
                return;
            }
            Key::Up => {
                self.cross_find.field = (self.cross_find.field + 2) % 3;
                return;
            }
            Key::Down => {
                self.cross_find.field = (self.cross_find.field + 1) % 3;
                return;
            }
            Key::Enter => {
                if self.cross_find.field < 2 {
                    self.cross_find.field += 1;
                } else {
                    self.apply_cross_find();
                }
                return;
            }
            _ => {}
        }

        match event.key {
            Key::Backspace => {
                let cursor = self.cross_find_cursor();
                if cursor == 0 {
                    return;
                }
                let previous = util::previous_boundary(self.cross_find_buffer().as_bytes(), cursor);
                self.cross_find_buffer_mut()
                    .replace_range(previous..cursor, "");
                self.set_cross_find_cursor(previous);
                self.refresh_cross_find_validation();
            }
            Key::Delete => {
                let cursor = self.cross_find_cursor();
                if cursor >= self.cross_find_buffer().len() {
                    return;
                }
                let next = util::next_boundary(self.cross_find_buffer().as_bytes(), cursor);
                self.cross_find_buffer_mut().replace_range(cursor..next, "");
                self.refresh_cross_find_validation();
            }
            Key::Left => {
                let cursor = self.cross_find_cursor();
                let previous = util::previous_boundary(self.cross_find_buffer().as_bytes(), cursor);
                self.set_cross_find_cursor(previous);
            }
            Key::Right => {
                let cursor = self.cross_find_cursor();
                let next = util::next_boundary(self.cross_find_buffer().as_bytes(), cursor);
                self.set_cross_find_cursor(next);
            }
            Key::Home => self.set_cross_find_cursor(0),
            Key::End => {
                let length = self.cross_find_buffer().len();
                self.set_cross_find_cursor(length);
            }
            Key::Char => {
                if event.ctrl {
                    if matches!(event.ch, 'a' | 'A') {
                        self.apply_cross_find();
                    }
                    return;
                }
                if event.alt || event.ch < '\u{20}' || event.ch == '\u{7f}' {
                    return;
                }
                let cursor = self.cross_find_cursor();
                self.cross_find_buffer_mut().insert(cursor, event.ch);
                self.set_cross_find_cursor(cursor + event.ch.len_utf8());
                self.refresh_cross_find_validation();
            }
            _ => {}
        }
    }

    fn refresh_cross_find_validation(&mut self) {
        self.cross_find.query_error.clear();
        if self.cross_find.regex && !self.cross_find.query.is_empty() {
            let query = self.cross_find.query.clone();
            if let Err(error) = self.create_cross_find_regex(&query) {
                self.cross_find.query_error = error.to_string();
            }
        }
        if self.cross_find.field == 0 {
            self.refresh_cross_find_file_count();
        }
    }

    fn create_cross_find_regex(&self, pattern: &str) -> Result<regex::Regex, regex::Error> {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(!self.search_match_case)
            .size_limit(2 * 1024 * 1024)
            .build()
    }

    fn build_cross_find_glob(pattern: &str) -> Result<(regex::Regex, bool), String> {
        let mut text = pattern.trim();
        if text.is_empty() {
            text = "*";
        }
        let normalized = text.replace('\\', "/");
        let mut text = normalized.as_str();
        while let Some(stripped) = text.strip_prefix("./") {
            text = stripped;
        }
        let basename_only = !text.contains('/');

        let characters: Vec<char> = text.chars().collect();
        let mut expression = String::with_capacity(characters.len() * 2 + 4);
        expression.push('^');
        let mut index = 0usize;
        while index < characters.len() {
            match characters[index] {
                '*' => {
                    if index + 1 < characters.len() && characters[index + 1] == '*' {
                        index += 1;
                    }
                    expression.push_str(".*");
                }
                '?' => expression.push('.'),
                character => expression.push_str(&regex::escape(&character.to_string())),
            }
            index += 1;
        }
        expression.push('$');

        let mut builder = regex::RegexBuilder::new(&expression);
        if cfg!(windows) || cfg!(target_os = "macos") {
            builder.case_insensitive(true);
        }
        let glob = builder.build().map_err(|error| error.to_string())?;
        Ok((glob, basename_only))
    }

    fn cross_find_completion_files(&mut self, root: &Path, show_ignored: bool) -> Vec<String> {
        let valid = self.cross_find.cached_files.is_some()
            && self.cross_find.cached_root == root
            && self.cross_find.cached_show_ignored == show_ignored;
        if !valid {
            self.cross_find.cached_files = Some(files::scan_root_with_ignored(
                root,
                CROSS_FIND_MAX_FILES,
                show_ignored,
            ));
            self.cross_find.cached_root = root.to_path_buf();
            self.cross_find.cached_show_ignored = show_ignored;
        }
        self.cross_find.cached_files.take().unwrap_or_default()
    }

    fn cross_find_files(&mut self, pattern: &str) -> Result<Vec<String>, String> {
        let (glob, basename_only) = Self::build_cross_find_glob(pattern)?;
        let root = git::absolute(&self.sidebar.root);
        let show_ignored = self.sidebar.show_git_ignored;
        let cached = self.cross_find_completion_files(&root, show_ignored);
        let mut matches = Vec::new();
        for relative in &cached {
            let normalized = relative.replace('\\', "/");
            let candidate = if basename_only {
                normalized.rsplit('/').next().unwrap_or(&normalized)
            } else {
                normalized.as_str()
            };
            if glob.is_match(candidate) {
                matches.push(root.join(relative).to_string_lossy().into_owned());
            }
        }
        self.cross_find.cached_files = Some(cached);
        Ok(matches)
    }

    fn refresh_cross_find_file_count(&mut self) {
        self.cross_find.matched_files = 0;
        self.cross_find.pattern_error.clear();
        let pattern = self.cross_find.pattern.clone();
        match self.cross_find_files(&pattern) {
            Ok(files) => self.cross_find.matched_files = files.len(),
            Err(error) => self.cross_find.pattern_error = format!("pattern: {error}"),
        }
    }

    fn cross_find_error(&self) -> &str {
        if !self.cross_find.pattern_error.is_empty() {
            &self.cross_find.pattern_error
        } else {
            &self.cross_find.query_error
        }
    }

    pub(super) fn apply_cross_find(&mut self) {
        let pattern = self.cross_find.pattern.clone();
        let query = self.cross_find.query.clone();
        let replacement = self.cross_find.replacement.clone();

        if query.is_empty() {
            self.cross_find.query_error = "find text is required".into();
            return;
        }

        let regex = if self.cross_find.regex {
            match self.create_cross_find_regex(&query) {
                Ok(regex) => Some(regex),
                Err(error) => {
                    self.cross_find.query_error = format!("invalid replacement: {error}");
                    return;
                }
            }
        } else {
            None
        };

        let files = match self.cross_find_files(&pattern) {
            Ok(files) => files,
            Err(error) => {
                self.cross_find.pattern_error = format!("pattern: {error}");
                return;
            }
        };
        if files.is_empty() {
            self.cross_find.pattern_error = "no files match the pattern".into();
            return;
        }

        let active_id = self.doc.id();
        let mut open_documents: HashMap<PathBuf, u64> = HashMap::new();
        for pane in &self.panes {
            let id = pane.doc_id;
            let path = if id == active_id {
                self.doc.path.clone()
            } else {
                self.documents
                    .get(&id)
                    .and_then(|document| document.path.clone())
            };
            if let Some(path) = path {
                open_documents.insert(git::absolute(Path::new(&path)), id);
            }
        }

        let mut replaced = 0usize;
        let mut changed_files = 0usize;
        let mut failed = 0usize;
        let mut skipped_open = 0usize;
        let mut first_error = String::new();
        let mut active_changed = false;

        for file in files {
            let full_path = git::absolute(Path::new(&file));

            if let Some(&document_id) = open_documents.get(&full_path) {
                if document_id != active_id {
                    skipped_open += 1;
                    continue;
                }

                let matches = Self::scan_cross_find_matches(
                    &self.doc,
                    &query,
                    regex.as_ref(),
                    &replacement,
                    self.search_match_case,
                );
                if matches.is_empty() {
                    continue;
                }
                Self::apply_cross_find_matches(&mut self.doc, &matches);
                replaced += matches.len();
                changed_files += 1;
                active_changed = true;
                continue;
            }

            let path = full_path.to_string_lossy().into_owned();
            let mut document = match Document::open(&path) {
                Ok(document) => document,
                Err(error) => {
                    failed += 1;
                    if first_error.is_empty() {
                        first_error = format!("{}: {error}", git::file_name(&full_path));
                    }
                    continue;
                }
            };

            let matches = Self::scan_cross_find_matches(
                &document,
                &query,
                regex.as_ref(),
                &replacement,
                self.search_match_case,
            );
            if matches.is_empty() {
                continue;
            }

            Self::apply_cross_find_matches(&mut document, &matches);
            replaced += matches.len();
            changed_files += 1;

            if let Err(error) = document.save(None, self.config.fsync_on_save) {
                failed += 1;
                if first_error.is_empty() {
                    first_error = format!("{}: {error}", git::file_name(&full_path));
                }
            }
        }

        self.close_cross_find();

        if active_changed {
            self.cursor = self.cursor.min(self.doc.size());
            self.anchor = self.cursor;
            self.selection_linewise = false;
            self.extra_selections.clear();
            self.preferred_column = None;
            self.snap_cursor();
            self.refresh_git_diff();
            self.ensure_cursor_visible();
        }

        if failed > 0 {
            let detail = if first_error.is_empty() {
                String::new()
            } else {
                format!(": {first_error}")
            };
            self.set_error(&format!(
                "Replaced {replaced} match{} in {changed_files} file{}, {failed} failed{detail}",
                if replaced == 1 { "" } else { "es" },
                if changed_files == 1 { "" } else { "s" },
            ));
            return;
        }

        let mut message = format!(
            "Replaced {replaced} match{} in {changed_files} file{}",
            if replaced == 1 { "" } else { "es" },
            if changed_files == 1 { "" } else { "s" },
        );
        if skipped_open > 0 {
            message.push_str(&format!(
                ", {skipped_open} open buffer{} skipped",
                if skipped_open == 1 { "" } else { "s" }
            ));
        }
        self.set_message(&message);
    }

    fn scan_cross_find_matches(
        document: &Document,
        query: &str,
        regex: Option<&regex::Regex>,
        replacement_pattern: &str,
        match_case: bool,
    ) -> Vec<FindMatch> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }

        for line in 0..document.line_count() {
            let line_text = document.line_text(line);
            let line_start = document.line_start(line);

            if let Some(regex) = regex {
                let Ok(text) = std::str::from_utf8(&line_text) else {
                    continue;
                };
                for captures in regex.captures_iter(text) {
                    let Some(match_) = captures.get(0) else {
                        continue;
                    };
                    let start = line_start + match_.start();
                    let end = line_start + match_.end();
                    let mut replacement = String::new();
                    captures.expand(replacement_pattern, &mut replacement);
                    matches.push(FindMatch {
                        start,
                        end,
                        line,
                        replacement,
                    });
                    if matches.len() >= MAX_CROSS_FIND_MATCHES {
                        return matches;
                    }
                }
            } else {
                if line_text.is_empty() {
                    continue;
                }
                let needle = query.as_bytes();
                let mut search_start = 0usize;
                while search_start < line_text.len() {
                    let Some(relative) =
                        util::index_of(&line_text[search_start..], needle, match_case)
                    else {
                        break;
                    };
                    let index = search_start + relative;
                    let match_length = needle.len().min(line_text.len() - index);
                    matches.push(FindMatch {
                        start: line_start + index,
                        end: line_start + index + match_length,
                        line,
                        replacement: replacement_pattern.to_string(),
                    });
                    if matches.len() >= MAX_CROSS_FIND_MATCHES {
                        return matches;
                    }
                    search_start = index + match_length.max(1);
                }
            }
        }

        matches
    }

    fn apply_cross_find_matches(document: &mut Document, matches: &[FindMatch]) {
        document.begin_composite_edit();
        for match_ in matches.iter().rev() {
            document.replace(
                match_.start,
                match_.end - match_.start,
                match_.replacement.as_bytes(),
            );
        }
        document.end_composite_edit();
        document.break_undo_group();
    }

    pub(super) fn render_cross_find(&mut self) {
        if !self.cross_find.active {
            return;
        }

        let available = self.status_row();
        let width = self.cols.saturating_sub(2).min(CROSS_FIND_DIALOG_WIDTH);
        let height = CROSS_FIND_DIALOG_HEIGHT;
        if available < height + 1 || width < 24 {
            return;
        }

        let x = (self.cols - width) / 2;
        let y = available.saturating_sub(height);
        let title = if self.cross_find.regex {
            "Regex replace across files"
        } else {
            "Replace across files"
        };
        self.draw_box(x, y, width, height, title);

        let label_width = 9usize;
        let field_x = x + 2 + label_width;
        let field_width = width.saturating_sub(label_width + 4);
        let accent_style = self.theme.accent_style();
        let muted_style = self.theme.muted_style();
        let text_style = self.theme.text();

        let fields = [
            (
                "Files",
                self.cross_find.pattern.clone(),
                self.cross_find.pattern_cursor,
            ),
            (
                "Find",
                self.cross_find.query.clone(),
                self.cross_find.query_cursor,
            ),
            (
                "Replace",
                self.cross_find.replacement.clone(),
                self.cross_find.replacement_cursor,
            ),
        ];

        for (index, (label, text, cursor)) in fields.iter().enumerate() {
            let focused = index == self.cross_find.field;
            let label_style = if focused { accent_style } else { muted_style };
            self.screen.put_str(
                (x + 2) as i64,
                (y + 1 + index) as i64,
                label,
                label_style,
                label_width,
            );
            self.screen.put_str(
                field_x as i64,
                (y + 1 + index) as i64,
                text,
                text_style,
                field_width,
            );
            if focused {
                self.render_find_cursor(field_x, y + 1 + index, text, *cursor, field_width);
            }
        }

        let error = self.cross_find_error().to_string();
        let mut status_style = accent_style;
        let status = if !error.is_empty() {
            status_style = self.theme.error_style();
            error
        } else {
            let file_word = if self.cross_find.matched_files == 1 {
                "file"
            } else {
                "files"
            };
            let matches_word = if self.cross_find.matched_files == 1 {
                "matches"
            } else {
                "match"
            };
            if self.cross_find.query.is_empty() {
                format!(
                    "{} {file_word} {matches_word} the pattern  ·  type to find",
                    self.cross_find.matched_files
                )
            } else {
                format!(
                    "{} {file_word} {matches_word} the pattern  ·  Enter to replace all",
                    self.cross_find.matched_files
                )
            }
        };
        self.screen.put_str(
            (x + 2) as i64,
            (y + 4) as i64,
            &status,
            status_style,
            width.saturating_sub(4),
        );
        self.screen.put_str(
            (x + 2) as i64,
            (y + 5) as i64,
            "Tab/Up/Down: switch  ·  Enter: next/apply  ·  Ctrl+A: all  ·  Esc: cancel",
            muted_style,
            width.saturating_sub(4),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hexal-cross-find-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn editor_at(root: &Path, path: &str) -> Editor {
        let document = Document::open(&root.join(path).to_string_lossy()).unwrap();
        let mut editor = Editor::new(document, 100, 32);
        editor.sidebar.set_root(root);
        editor
    }

    #[test]
    fn literal_replace_rewrites_every_matching_file() {
        let root = temp_root("literal");
        fs::write(root.join("one.txt"), b"foo one\nfoo two\n").unwrap();
        fs::write(root.join("two.txt"), b"keep foo\n").unwrap();
        fs::write(root.join("skip.md"), b"foo markdown\n").unwrap();

        let mut editor = editor_at(&root, "one.txt");
        editor.open_cross_find(false);
        editor.cross_find.pattern = "*.txt".into();
        editor.cross_find.query = "foo".into();
        editor.cross_find.replacement = "bar".into();
        editor.refresh_cross_find_file_count();
        assert_eq!(editor.cross_find.matched_files, 2);
        assert_eq!(editor.cross_find.pattern_error, "");
        editor.apply_cross_find();

        // The active buffer is updated in memory but not written; other files
        // are saved immediately, matching the C# editor.
        assert_eq!(
            fs::read(root.join("one.txt")).unwrap(),
            b"foo one\nfoo two\n"
        );
        assert_eq!(fs::read(root.join("two.txt")).unwrap(), b"keep bar\n");
        assert_eq!(fs::read(root.join("skip.md")).unwrap(), b"foo markdown\n");
        assert_eq!(editor.doc.all_text(), b"bar one\nbar two\n");
        assert!(editor.message.starts_with("Replaced 3 matches in 2 files"));
        assert!(!editor.cross_find.active);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn regex_replace_expands_capture_groups_across_files() {
        let root = temp_root("regex");
        fs::write(root.join("one.txt"), b"let one = 1;\n").unwrap();
        fs::write(root.join("two.txt"), b"let two = 2;\n").unwrap();

        let mut editor = editor_at(&root, "one.txt");
        editor.open_cross_find(true);
        editor.cross_find.pattern = "*.txt".into();
        editor.cross_find.query = r"let (\w+) = (\d+);".into();
        editor.cross_find.replacement = "$1=$2".into();
        editor.refresh_cross_find_file_count();
        editor.apply_cross_find();

        assert_eq!(fs::read(root.join("one.txt")).unwrap(), b"let one = 1;\n");
        assert_eq!(fs::read(root.join("two.txt")).unwrap(), b"two=2\n");
        assert_eq!(editor.doc.all_text(), b"one=1\n");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn glob_without_a_slash_matches_basenames() {
        let root = temp_root("glob");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/deep.rs"), b"todo\n").unwrap();

        let mut editor = editor_at(&root, "nested/deep.rs");
        editor.open_cross_find(false);
        editor.cross_find.pattern = "*.rs".into();
        editor.refresh_cross_find_file_count();
        assert_eq!(editor.cross_find.matched_files, 1);

        editor.cross_find.pattern = "nested/*.rs".into();
        editor.refresh_cross_find_file_count();
        assert_eq!(editor.cross_find.matched_files, 1);

        editor.cross_find.pattern = "*.toml".into();
        editor.refresh_cross_find_file_count();
        assert_eq!(editor.cross_find.matched_files, 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn open_buffers_in_other_panes_are_skipped() {
        let root = temp_root("open");
        fs::write(root.join("one.txt"), b"foo\n").unwrap();
        fs::write(root.join("two.txt"), b"foo\n").unwrap();

        let mut editor = editor_at(&root, "one.txt");
        let other = Document::open(&root.join("two.txt").to_string_lossy()).unwrap();
        editor.panes[0].doc_id = editor.doc.id();
        editor.panes.push(PaneState {
            id: 2,
            doc_id: other.id(),
            language_override: None,
            mode: Mode::Normal,
            cursor: 0,
            anchor: 0,
            selection_linewise: false,
            extra_selections: Vec::new(),
            preferred_column: None,
            top_line: 0,
            left_column: 0,
            pending_g: false,
            mouse_dragging: false,
            mouse_drag_anchor: 0,
            find: FindState::new(),
            jumps: Vec::new(),
            jump_index: -1,
            highlighter: Highlighter::default(),
            git_diff: diff_lines::DiffLineState::new(),
        });
        editor.documents.insert(other.id(), other);

        editor.open_cross_find(false);
        editor.cross_find.pattern = "*.txt".into();
        editor.cross_find.query = "foo".into();
        editor.cross_find.replacement = "bar".into();
        editor.refresh_cross_find_file_count();
        editor.apply_cross_find();

        assert_eq!(editor.doc.all_text(), b"bar\n");
        assert_eq!(fs::read(root.join("one.txt")).unwrap(), b"foo\n");
        assert_eq!(fs::read(root.join("two.txt")).unwrap(), b"foo\n");
        assert!(editor.message.contains("1 open buffer skipped"));

        let _ = fs::remove_dir_all(&root);
    }
}
