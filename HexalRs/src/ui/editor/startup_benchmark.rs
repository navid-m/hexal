//! Startup benchmark overlay.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use crate::util::startup_benchmark as benchmark;

impl Editor {
    pub(super) fn open_startup_benchmark(&mut self) {
        if !self.config.startup_benchmarker {
            self.set_error("The startup benchmarker is disabled in .hexcfg.");
            return;
        }
        self.benchmark_visible = true;
        self.benchmark_top = 0;
        self.force_full_redraw = true;
        self.clear_message();
    }

    pub(super) fn handle_startup_benchmark_input(&mut self, event: Event) {
        let (_, _, _, content_rows) = self.benchmark_layout();
        let entries = benchmark::sorted().len();
        let max_top = entries.saturating_sub(content_rows);
        match event.key {
            Key::Escape | Key::Enter => {
                self.benchmark_visible = false;
                self.benchmark_top = 0;
                self.force_full_redraw = true;
            }
            Key::Char if matches!(event.ch, 'q' | 'Q') && !event.ctrl && !event.alt => {
                self.benchmark_visible = false;
                self.benchmark_top = 0;
                self.force_full_redraw = true;
            }
            Key::Up => self.benchmark_top = self.benchmark_top.saturating_sub(1),
            Key::Down => self.benchmark_top = (self.benchmark_top + 1).min(max_top),
            Key::ScrollUp => self.benchmark_top = self.benchmark_top.saturating_sub(3),
            Key::ScrollDown => self.benchmark_top = (self.benchmark_top + 3).min(max_top),
            Key::PageUp => self.benchmark_top = self.benchmark_top.saturating_sub(content_rows),
            Key::PageDown => {
                self.benchmark_top = (self.benchmark_top + content_rows).min(max_top);
            }
            Key::Home => self.benchmark_top = 0,
            Key::End => self.benchmark_top = max_top,
            _ => {}
        }
    }

    fn benchmark_layout(&self) -> (usize, usize, usize, usize) {
        let available = self.status_row();
        let entries = benchmark::snapshot().len();
        let height = available.min((entries + 4).max(8));
        let width = self.cols.saturating_sub(2).min(120);
        let y = available.saturating_sub(height) / 2;
        (width, height, y, height.saturating_sub(4).max(1))
    }

    pub(super) fn render_startup_benchmark(&mut self) {
        if !self.benchmark_visible || !self.config.startup_benchmarker {
            return;
        }
        let (width, height, y, content_rows) = self.benchmark_layout();
        if width < 32 || height < 6 {
            return;
        }
        let x = self.cols.saturating_sub(width) / 2;
        self.draw_box(x, y, width, height, " Startup benchmark ");

        let entries = benchmark::sorted();
        let content_width = width.saturating_sub(4);
        let max_top = entries.len().saturating_sub(content_rows);
        self.benchmark_top = self.benchmark_top.min(max_top);

        self.screen.put_str(
            (x + 2) as i64,
            (y + 1) as i64,
            "Duration",
            self.theme.accent_style(),
            content_width,
        );
        self.screen.put_str(
            (x + 13) as i64,
            (y + 1) as i64,
            "Component",
            self.theme.accent_style(),
            content_width.saturating_sub(11),
        );

        let duration_width = 10usize;
        for row in 0..content_rows {
            let index = self.benchmark_top + row;
            let Some(entry) = entries.get(index) else {
                break;
            };
            let screen_row = y + 2 + row;
            let duration = format!(
                "{:>width$.2} ms",
                entry.milliseconds,
                width = duration_width
            );
            let duration_style = if entry.milliseconds >= 100.0 {
                self.theme.warning_style()
            } else if entry.milliseconds >= 25.0 {
                self.theme.accent_style()
            } else {
                self.theme.text()
            };
            self.screen.put_str(
                (x + 2) as i64,
                screen_row as i64,
                &duration,
                duration_style,
                content_width,
            );
            let mut column = duration_width + 1;
            let label = format!("{}: {}", entry.category, entry.name);
            self.screen.put_str(
                (x + 2 + column) as i64,
                screen_row as i64,
                &label,
                self.theme.text(),
                content_width.saturating_sub(column),
            );
            column += util::text_width(&label);
            if !entry.detail.is_empty() && column + 2 < content_width {
                self.screen.put_str(
                    (x + 2 + column) as i64,
                    screen_row as i64,
                    &format!("  {}", entry.detail),
                    self.theme.muted_style(),
                    content_width.saturating_sub(column + 2),
                );
            }
        }

        let footer = format!(
            "{} measurement{} · slowest first · Esc/q close",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        );
        self.screen.put_str(
            (x + 2) as i64,
            (y + height - 2) as i64,
            &footer,
            self.theme.muted_style(),
            width.saturating_sub(4),
        );
    }
}
