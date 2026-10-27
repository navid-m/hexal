//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    pub(super) fn active_language(&self) -> Option<std::rc::Rc<Language>> {
        if let Some(id) = &self.language_override {
            if let Some(language) = self.languages.find_shared(id) {
                return Some(language);
            }
        }
        self.doc
            .path
            .as_deref()
            .and_then(|path| self.languages.detect_shared(path))
    }

    pub(super) fn refresh_language(&mut self) {
        self.highlighter.clear();
        let language = if let Some(id) = &self.language_override {
            self.languages.find(id)
        } else {
            self.doc
                .path
                .as_deref()
                .and_then(|path| self.languages.detect(path))
        };
        let Some(language) = language else {
            return;
        };
        if let Some(indentation) = &language.indentation {
            self.doc.set_indentation(
                indentation.use_tabs,
                indentation.indent_width,
                indentation.tab_width,
            );
        }
    }

    /// Applies the default indentation style declared by the grammar.

    pub(super) fn auto_detect_indentation_on_open(&mut self) {
        if !self.config.auto_detect_indentation_on_startup || self.doc.size() == 0 {
            return;
        }
        let (use_tabs, width) = detect_document_indentation(&self.doc);
        if use_tabs {
            self.doc.set_indent_tabs(width);
        } else {
            self.doc.set_indent_spaces(width);
        }
    }

    pub(super) fn mode_label(&self) -> &'static str {
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
        if self.completion.visible {
            return "INSERT";
        }
        self.mode.label()
    }

    pub(super) fn status_style(&self) -> crate::ui::theme::Style {
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

    pub(super) fn set_message(&mut self, text: &str) {
        self.message.clear();
        self.message.push_str(text);
        self.message_is_error = false;
    }

    pub(super) fn set_error(&mut self, text: &str) {
        self.message.clear();
        if !text.starts_with("E: ") {
            self.message.push_str("E: ");
        }
        self.message.push_str(text);
        self.message_is_error = true;
    }

    pub(super) fn clear_message(&mut self) {
        self.message.clear();
        self.message_is_error = false;
    }
}

fn detect_document_indentation(document: &Document) -> (bool, usize) {
    const MAX_LINES: usize = 20_000;
    let line_count = document.line_count().min(MAX_LINES);
    let mut tab_lines = 0usize;
    let mut space_lines = 0usize;
    let mut space_counts: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();

    for line in 0..line_count {
        let text = document.line_text(line);
        let mut tabs = 0usize;
        let mut spaces = 0usize;
        let mut offset = 0usize;
        while offset < text.len() {
            if text[offset] == b'\t' {
                tabs += 1;
                offset += 1;
            } else if text[offset] == b' ' {
                spaces += 1;
                offset += 1;
            } else {
                break;
            }
        }
        if offset >= text.len() {
            continue;
        }
        if tabs > 0 {
            tab_lines += 1;
        } else if spaces > 0 {
            space_lines += 1;
            *space_counts.entry(spaces).or_insert(0) += 1;
        }
    }

    if tab_lines > 0 && tab_lines >= space_lines {
        return (true, document.tab_width());
    }
    if space_lines > 0 {
        let mut gcd = 0usize;
        for count in space_counts.keys() {
            gcd = greatest_common_divisor(gcd, *count);
        }
        if (2..=16).contains(&gcd) {
            return (false, gcd);
        }
        let mut best = 0usize;
        let mut best_frequency = 0usize;
        for (width, frequency) in &space_counts {
            if (1..=16).contains(width)
                && (*frequency > best_frequency || (*frequency == best_frequency && *width < best))
            {
                best = *width;
                best_frequency = *frequency;
            }
        }
        return (
            false,
            if best > 0 {
                best
            } else {
                document.indent_width()
            },
        );
    }

    (
        document.use_tabs(),
        if document.use_tabs() {
            document.tab_width()
        } else {
            document.indent_width()
        },
    )
}

fn greatest_common_divisor(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}
