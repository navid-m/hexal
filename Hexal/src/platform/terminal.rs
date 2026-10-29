//! Raw terminal handling: termios, poll-based input and a small ANSI decoder.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    ScrollUp,
    ScrollDown,
    MouseDown,
    MouseUp,
    MouseMove,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub key: Key,
    pub ch: char,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub mouse_x: usize,
    pub mouse_y: usize,
    pub mouse_button: MouseButton,
}

impl Event {
    pub(super) fn plain(key: Key) -> Event {
        Event {
            key,
            ch: '\0',
            ctrl: false,
            alt: false,
            shift: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button: MouseButton::None,
        }
    }
}

#[cfg(unix)]
#[path = "terminal_unix.rs"]
mod unix;
#[cfg(unix)]
pub use self::unix::Terminal;

#[cfg(windows)]
#[path = "terminal_windows.rs"]
mod windows;
#[cfg(windows)]
pub use self::windows::Terminal;
