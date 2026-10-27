//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

impl Editor {
    // ---------------------------------------------------------------------
    // Small helpers
    // ---------------------------------------------------------------------

    pub(super) fn active_language(&self) -> Option<&'static Language> {
        if let Some(id) = &self.language_override {
            if let Some(language) = syntax::from_id(id) {
                return Some(language);
            }
        }
        self.doc.path.as_deref().and_then(syntax::from_path)
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
