//! Double buffered cell grid.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use crate::ui::theme::Style;
use crate::util;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    pub wide_continuation: bool,
}

impl Cell {
    fn blank(style: Style) -> Cell {
        Cell {
            ch: ' ',
            style,
            wide_continuation: false,
        }
    }
}

pub struct Screen {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
}

impl Screen {
    pub fn new() -> Screen {
        Screen {
            width: 0,
            height: 0,
            cells: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.cells = vec![Cell::blank(Style::default()); width * height];
    }

    pub fn clear(&mut self, style: Style) {
        self.cells.fill(Cell::blank(style));
    }

    pub fn put(&mut self, x: i64, y: i64, ch: char, style: Style, width: usize) {
        if width == 0 || x < 0 || y < 0 {
            return;
        }
        let ux = x as usize;
        let uy = y as usize;
        if ux >= self.width || uy >= self.height {
            return;
        }
        if width > 1 && ux + width > self.width {
            return;
        }
        let index = uy * self.width + ux;

        let ch = if ch.is_control() { ' ' } else { ch };

        if ux > 0 && self.cells[index].wide_continuation {
            self.cells[index - 1] = Cell::blank(self.cells[index - 1].style);
        }
        if ux + 1 < self.width && self.cells[index + 1].wide_continuation {
            self.cells[index + 1] = Cell::blank(self.cells[index + 1].style);
        }

        self.cells[index] = Cell {
            ch,
            style,
            wide_continuation: false,
        };
        for offset in 1..width {
            self.cells[index + offset] = Cell {
                ch: ' ',
                style,
                wide_continuation: true,
            };
        }
    }

    pub fn put_str(&mut self, x: i64, y: i64, text: &str, style: Style, max_width: usize) {
        let mut column = 0usize;
        for cp in text.chars() {
            let width = util::display_width(cp);
            if width == 0 {
                continue;
            }
            if column >= max_width
                || x + column as i64 >= self.width as i64
                || column + width > max_width
                || x + (column + width) as i64 > self.width as i64
            {
                break;
            }
            self.put(x + column as i64, y, cp, style, width);
            column += width;
        }
    }

    pub fn fill(&mut self, x: i64, y: i64, width: usize, height: usize, ch: char, style: Style) {
        for row in 0..height {
            for column in 0..width {
                self.put(x + column as i64, y + row as i64, ch, style, 1);
            }
        }
    }

    pub fn apply_style(&mut self, x: usize, y: usize, style: Style) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y * self.width + x;
        self.cells[index].style = style;
        if self.cells[index].wide_continuation && x > 0 {
            self.cells[index - 1].style = style;
        } else if x + 1 < self.width && self.cells[index + 1].wide_continuation {
            self.cells[index + 1].style = style;
        }
    }

    pub fn at(&self, x: usize, y: usize) -> &Cell {
        &self.cells[y * self.width + x]
    }

    /// Copy one screen into another at the given top-left position.
    pub fn blit(&mut self, source: &Screen, x: usize, y: usize) {
        for source_y in 0..source.height {
            let target_y = y + source_y;
            if target_y >= self.height {
                break;
            }
            for source_x in 0..source.width {
                let target_x = x + source_x;
                if target_x >= self.width {
                    break;
                }
                self.cells[target_y * self.width + target_x] =
                    source.cells[source_y * source.width + source_x];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_never_stores_terminal_control_characters() {
        let mut screen = Screen::new();
        screen.resize(10, 1);
        screen.put_str(0, 0, "a\nb\tc\x1b[31m", Style::default(), 10);

        let text: String = screen.cells.iter().map(|cell| cell.ch).collect();
        assert_eq!(text, "a b c [31m");
        assert!(!text.chars().any(char::is_control));
    }
}
