//! Ex command-line completion for command names, files and languages.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

const BUILTIN_COMMANDS: &[&str] = &[
    "w",
    "write",
    "wq",
    "q",
    "quit",
    "e",
    "edit",
    "path",
    "help",
    "theme",
    "themes",
    "cfg",
    "rcfg",
    "a",
    "about",
    "cw",
    "rc",
    "lua",
    "plugins",
    "lang",
    "fmt",
    "fmta",
    "debug-lsp",
    "dl",
    "gi",
    "gdl",
    "smc",
    "sidebar",
    "adi",
];

impl Editor {
    pub(super) fn try_open_command_completion(&mut self) -> bool {
        let line = self.command_line.clone();
        let cursor = self.command_cursor.min(line.len());
        let bytes = line.as_bytes();

        let mut first_start = 0usize;
        while first_start < bytes.len() && (bytes[first_start] as char).is_ascii_whitespace() {
            first_start += 1;
        }
        let mut first_end = first_start;
        while first_end < bytes.len() && !(bytes[first_end] as char).is_ascii_whitespace() {
            first_end += 1;
        }
        let command = line[first_start..first_end].to_lowercase();
        let file_command = matches!(
            command.as_str(),
            "e" | "edit" | "w" | "write" | "debug" | "dbg"
        );
        let at_or_inside_command = cursor <= first_end;
        let has_arguments = !line[first_end..].trim().is_empty();

        if at_or_inside_command && (!file_command || cursor < first_end || has_arguments) {
            let filter_end = cursor.clamp(first_start, first_end);
            let filter = line[first_start..filter_end].to_string();
            let items = self.filter_command_names(&filter);
            self.open_command_completion(
                CommandCompletionKind::Commands,
                items,
                line[..first_start].to_string(),
                line[first_end..].to_string(),
                filter,
            );
            return true;
        }

        if file_command {
            let mut argument_start = first_end;
            while argument_start < bytes.len()
                && (bytes[argument_start] as char).is_ascii_whitespace()
            {
                argument_start += 1;
            }
            let cursor = cursor.max(argument_start);
            let mut quote = '\0';
            let mut token_start = argument_start;
            let mut token_end = cursor;
            if argument_start < bytes.len()
                && (bytes[argument_start] == b'"' || bytes[argument_start] == b'\'')
            {
                quote = bytes[argument_start] as char;
                token_start = argument_start + 1;
                token_end = line[token_start..]
                    .find(quote)
                    .map(|index| token_start + index)
                    .unwrap_or(cursor.max(token_start));
            }
            token_start = token_start.min(line.len());
            token_end = token_end.clamp(token_start, line.len());
            let filter = line[token_start..token_end].to_string();
            let items = self.filter_file_completion_items(&filter);
            let mut prefix = line[..token_start].to_string();
            if token_start == first_end && first_end == line.len() {
                prefix.push(' ');
            }
            let mut suffix = line[token_end..].to_string();
            if quote != '\0' && !suffix.starts_with(quote) {
                suffix.insert(0, quote);
            }
            self.open_command_completion(
                CommandCompletionKind::Files,
                items,
                prefix,
                suffix,
                filter,
            );
            return true;
        }

        if command == "lang" {
            let mut argument_start = first_end;
            while argument_start < bytes.len()
                && (bytes[argument_start] as char).is_ascii_whitespace()
            {
                argument_start += 1;
            }
            let cursor = cursor.max(argument_start);
            let token_start = argument_start.min(line.len());
            let token_end = cursor.clamp(token_start, line.len());
            let filter = line[token_start..token_end].to_string();
            let mut names: Vec<String> = self
                .languages
                .languages()
                .iter()
                .map(|language| language.id.clone())
                .collect();
            names.sort_by_key(|name| name.to_lowercase());
            let items = Self::filter_candidates(names, &filter, self.search_match_case);
            self.open_command_completion(
                CommandCompletionKind::Commands,
                items,
                line[..token_start].to_string(),
                line[token_end..].to_string(),
                filter,
            );
            return true;
        }

        self.set_message("No completions available.");
        false
    }

    pub(super) fn refresh_command_completion(&mut self) {
        if self.command_completion.visible {
            let _ = self.try_open_command_completion();
        }
    }

    fn open_command_completion(
        &mut self,
        kind: CommandCompletionKind,
        items: Vec<String>,
        prefix: String,
        suffix: String,
        filter: String,
    ) {
        self.command_completion.visible = true;
        self.command_completion.kind = kind;
        self.command_completion.items = items;
        self.command_completion.selected = 0;
        self.command_completion.top = 0;
        self.command_completion.prefix = prefix;
        self.command_completion.suffix = suffix;
        self.command_completion.filter = filter;
    }

    pub(super) fn move_command_completion(&mut self, delta: isize, wrap: bool) {
        let count = self.command_completion.items.len();
        if count == 0 {
            return;
        }
        let mut index = self.command_completion.selected as isize + delta;
        if wrap {
            index = index.rem_euclid(count as isize);
        } else {
            index = index.clamp(0, count as isize - 1);
        }
        self.command_completion.selected = index as usize;
        let rows = self.command_completion.rows();
        if self.command_completion.selected < self.command_completion.top {
            self.command_completion.top = self.command_completion.selected;
        } else if self.command_completion.selected >= self.command_completion.top + rows {
            self.command_completion.top = self.command_completion.selected + 1 - rows;
        }
        let max_top = count.saturating_sub(rows);
        self.command_completion.top = self.command_completion.top.min(max_top);
    }

    pub(super) fn accept_command_completion(&mut self) -> bool {
        if !self.command_completion.visible || self.command_completion.items.is_empty() {
            return false;
        }
        let Some(selected) = self
            .command_completion
            .items
            .get(self.command_completion.selected)
            .cloned()
        else {
            return false;
        };
        let kind = self.command_completion.kind;
        let mut inserted = selected;
        if kind == CommandCompletionKind::Files && inserted.chars().any(char::is_whitespace) {
            inserted = format!("\"{}\"", inserted.replace('"', "\\\""));
        }
        let line = format!(
            "{}{}{}",
            self.command_completion.prefix, inserted, self.command_completion.suffix
        );
        let cursor = self.command_completion.prefix.len() + inserted.len();
        self.command_completion.close();
        self.command_line = line;
        self.command_cursor = cursor.min(self.command_line.len());
        if kind == CommandCompletionKind::Files {
            self.submit_command();
        }
        true
    }

    fn filter_command_names(&self, filter: &str) -> Vec<String> {
        let mut names: Vec<String> = BUILTIN_COMMANDS
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        names.extend(self.plugin_command_names());
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| seen.insert(name.to_lowercase()));
        Self::filter_candidates(names, filter, self.search_match_case)
    }

    fn filter_file_completion_items(&mut self, filter: &str) -> Vec<String> {
        let files = self
            .command_files
            .get_or_insert_with(|| files::scan(20_000));
        Self::filter_candidates(files.clone(), filter, self.search_match_case)
    }

    fn filter_candidates(candidates: Vec<String>, filter: &str, match_case: bool) -> Vec<String> {
        let mut matches: Vec<(usize, String)> = candidates
            .into_iter()
            .filter_map(|candidate| {
                fuzzy_score(&candidate, filter, match_case).map(|score| (score, candidate))
            })
            .collect();
        matches.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.to_lowercase().cmp(&right.1.to_lowercase()))
        });
        matches
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect()
    }
}

fn fuzzy_score(text: &str, query: &str, match_case: bool) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let text_chars: Vec<char> = if match_case {
        text.chars().collect()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    };
    let query_chars: Vec<char> = if match_case {
        query.chars().collect()
    } else {
        query.chars().flat_map(char::to_lowercase).collect()
    };
    if query_chars.is_empty() {
        return Some(0);
    }
    let mut text_index = 0usize;
    let mut last_match: isize = -2;
    let mut gap_score = 0usize;
    for wanted in query_chars {
        let mut found = false;
        while text_index < text_chars.len() {
            if text_chars[text_index] == wanted {
                gap_score += (text_index as isize - last_match - 1).max(0) as usize;
                last_match = text_index as isize;
                text_index += 1;
                found = true;
                break;
            }
            text_index += 1;
        }
        if !found {
            return None;
        }
    }
    Some(10_000 + gap_score + text_chars.len().min(1_000))
}

pub(super) fn render_command_completion(editor: &mut Editor) {
    if !editor.command_completion.visible {
        return;
    }
    let rows = editor.command_completion.rows();
    let mut width = 28usize;
    for item in editor
        .command_completion
        .items
        .iter()
        .skip(editor.command_completion.top)
        .take(rows)
    {
        width = width.max(util::text_width(item) + 4);
    }
    width = width.min(editor.cols.saturating_sub(4).max(12));
    let height = rows + 2;
    let bottom = editor.status_row();
    if bottom < height + 1 || width < 12 {
        return;
    }
    let y = bottom.saturating_sub(height);
    let x = 1usize.min(editor.cols.saturating_sub(width + 1));
    let title = match editor.command_completion.kind {
        CommandCompletionKind::Files => format!(
            " Files {}/{} ",
            if editor.command_completion.items.is_empty() {
                0
            } else {
                editor.command_completion.selected + 1
            },
            editor.command_completion.items.len()
        ),
        CommandCompletionKind::Commands => format!(
            " Commands {}/{} ",
            if editor.command_completion.items.is_empty() {
                0
            } else {
                editor.command_completion.selected + 1
            },
            editor.command_completion.items.len()
        ),
    };
    editor.draw_box(x, y, width, height, &title);
    if editor.command_completion.items.is_empty() {
        let message = if editor.command_completion.filter.is_empty() {
            "No completions available"
        } else {
            "No matches"
        };
        editor.screen.put_str(
            (x + 2) as i64,
            (y + 1) as i64,
            message,
            editor.theme.muted_style(),
            width.saturating_sub(4),
        );
        return;
    }
    for row in 0..rows {
        let index = editor.command_completion.top + row;
        if index >= editor.command_completion.items.len() {
            break;
        }
        let style = if index == editor.command_completion.selected {
            editor.theme.selected_item_style()
        } else {
            editor.theme.text()
        };
        editor.screen.fill(
            (x + 1) as i64,
            (y + 1 + row) as i64,
            width - 2,
            1,
            ' ',
            style,
        );
        editor.screen.put_str(
            (x + 2) as i64,
            (y + 1 + row) as i64,
            &editor.command_completion.items[index],
            style,
            width.saturating_sub(4),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.command_files = Some(vec![
            "README.md".to_string(),
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
        ]);
        editor
    }

    #[test]
    fn file_completion_filters_and_accepts() {
        let mut editor = editor();
        editor.command_line = "e".into();
        editor.command_cursor = 1;
        assert!(editor.try_open_command_completion());
        assert!(editor.command_completion.visible);
        assert_eq!(editor.command_completion.items.len(), 3);

        editor.command_line = "e ma".into();
        editor.command_cursor = 4;
        editor.refresh_command_completion();
        assert_eq!(
            editor.command_completion.items.first().map(String::as_str),
            Some("src/main.rs")
        );
        assert!(editor.accept_command_completion());
        assert_eq!(
            editor
                .doc
                .path
                .as_deref()
                .filter(|path| path.ends_with("src/main.rs"))
                .is_some(),
            true
        );
    }

    #[test]
    fn commands_are_case_insensitive() {
        let mut editor = editor();
        editor.command_line = "E src/main.rs".into();
        editor.command_cursor = editor.command_line.len();
        editor.submit_command();
        assert!(
            editor
                .doc
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with("src/main.rs"))
        );
    }

    #[test]
    fn adi_detects_spaces() {
        let mut editor = editor();
        editor.doc = Document::from_bytes(None, b"    one\n    two\n".to_vec());
        editor.command_line = "adi".into();
        editor.command_cursor = 3;
        editor.submit_command();
        assert_eq!(editor.doc.indent_width(), 4);
        assert!(!editor.doc.use_tabs());
        assert!(editor.message.contains("Detected spaces"));
    }

    #[test]
    fn backspace_in_normal_mode_does_nothing() {
        let mut editor = editor();
        editor.doc = Document::from_bytes(None, b"abc".to_vec());
        editor.cursor = 1;
        let event = Event {
            key: Key::Backspace,
            ch: '\0',
            ctrl: false,
            alt: false,
            shift: false,
        };
        editor.handle_normal(event);
        assert_eq!(editor.doc.all_text(), b"abc");
        assert_eq!(editor.cursor, 1);
    }

    #[test]
    fn picker_shows_underlying_mode() {
        let mut editor = editor();
        editor.picker.kind = PickerKind::Files;
        assert_eq!(editor.mode_label(), "NORMAL");
        editor.mode = Mode::Insert;
        assert_eq!(editor.mode_label(), "INSERT");
    }
}
