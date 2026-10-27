//! ANSI diff renderer.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use crate::ui::screen::{Cell, Screen};
use crate::ui::theme::Style;

pub struct Renderer {
    previous: Vec<Cell>,
    previous_width: usize,
    previous_height: usize,
    output: Vec<u8>,
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer {
            previous: Vec::new(),
            previous_width: 0,
            previous_height: 0,
            output: Vec::new(),
        }
    }

    pub fn render(&mut self, screen: &Screen, force_full: bool) -> &[u8] {
        self.output.clear();
        let width = screen.width;
        let height = screen.height;
        if width == 0 || height == 0 {
            self.previous.clear();
            self.previous_width = width;
            self.previous_height = height;
            return &self.output;
        }
        let expected = screen.cells.len().saturating_mul(4).min(1 << 20);
        if self.output.capacity() < expected {
            self.output.reserve(expected - self.output.capacity());
        }

        let size_changed = width != self.previous_width
            || height != self.previous_height
            || self.previous.len() != screen.cells.len();
        let full = force_full || size_changed;
        if size_changed {
            self.output.extend_from_slice(b"\x1b[2J");
        }

        let mut wrote_any = size_changed;
        for y in 0..height {
            let mut x = 0usize;
            while x < width {
                let index = y * width + x;
                let changed = full
                    || index >= self.previous.len()
                    || !cells_equal(&screen.cells[index], &self.previous[index]);
                if !changed || screen.cells[index].wide_continuation {
                    x += 1;
                    continue;
                }

                let run_start = x;
                while x < width {
                    let run_index = y * width + x;
                    let run_changed = full
                        || run_index >= self.previous.len()
                        || !cells_equal(&screen.cells[run_index], &self.previous[run_index]);
                    if !run_changed {
                        break;
                    }
                    x += 1;
                }

                wrote_any = true;
                self.output.extend_from_slice(b"\x1b[");
                append_int(&mut self.output, y + 1);
                self.output.push(b';');
                append_int(&mut self.output, run_start + 1);
                self.output.push(b'H');

                let mut current = Style::default();
                let mut current_valid = false;
                for column in run_start..x {
                    let cell = &screen.cells[y * width + column];
                    if cell.wide_continuation {
                        continue;
                    }
                    if !current_valid || cell.style != current {
                        append_style(&mut self.output, cell.style);
                        current = cell.style;
                        current_valid = true;
                    }
                    let mut buffer = [0u8; 4];
                    self.output
                        .extend_from_slice(cell.ch.encode_utf8(&mut buffer).as_bytes());
                }
            }
        }

        if wrote_any {
            self.output.extend_from_slice(b"\x1b[0m");
        }

        if self.previous.len() != screen.cells.len() {
            self.previous.resize(
                screen.cells.len(),
                Cell {
                    ch: ' ',
                    style: Style::default(),
                    wide_continuation: false,
                },
            );
        }
        self.previous.copy_from_slice(&screen.cells);
        self.previous_width = width;
        self.previous_height = height;
        &self.output
    }
}

fn cells_equal(a: &Cell, b: &Cell) -> bool {
    a.ch == b.ch && a.style == b.style && a.wide_continuation == b.wide_continuation
}

fn append_style(output: &mut Vec<u8>, style: Style) {
    output.extend_from_slice(b"\x1b[0");
    if style.bold {
        output.extend_from_slice(b";1");
    }
    if style.dim {
        output.extend_from_slice(b";2");
    }
    if style.reverse {
        output.extend_from_slice(b";7");
    }
    append_color(output, style.fg, true);
    append_color(output, style.bg, false);
    output.push(b'm');
}

fn append_color(output: &mut Vec<u8>, color: u32, foreground: bool) {
    if crate::ui::theme::is_true_color(color) {
        output.extend_from_slice(if foreground { b";38;2;" } else { b";48;2;" });
        append_int(output, ((color >> 16) & 0xFF) as usize);
        output.push(b';');
        append_int(output, ((color >> 8) & 0xFF) as usize);
        output.push(b';');
        append_int(output, (color & 0xFF) as usize);
    } else if color < 8 {
        output.extend_from_slice(if foreground { b";3" } else { b";4" });
        append_int(output, color as usize);
    } else if color < 16 {
        output.extend_from_slice(if foreground { b";9" } else { b";10" });
        append_int(output, (color - 8) as usize);
    } else {
        output.extend_from_slice(if foreground { b";38;5;" } else { b";48;5;" });
        append_int(output, (color & 0xFF) as usize);
    }
}

fn append_int(output: &mut Vec<u8>, value: usize) {
    let mut buffer = [0u8; 20];
    let mut index = buffer.len();
    let mut value = value;
    if value == 0 {
        output.push(b'0');
        return;
    }
    while value > 0 {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    output.extend_from_slice(&buffer[index..]);
}
