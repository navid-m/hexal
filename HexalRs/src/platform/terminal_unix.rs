//! POSIX terminal handling: termios, poll-based input and an ANSI decoder.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use std::io;

pub struct Terminal {
    original: libc::termios,
    raw: bool,
    pending: Option<u8>,
    pub width: usize,
    pub height: usize,
}

impl Terminal {
    pub fn init() -> io::Result<Terminal> {
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut raw = original;
        raw.c_iflag &= !(libc::IGNBRK
            | libc::BRKINT
            | libc::PARMRK
            | libc::ISTRIP
            | libc::INLCR
            | libc::IGNCR
            | libc::ICRNL
            | libc::IXON);
        raw.c_oflag &= !libc::OPOST;
        raw.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        raw.c_cflag |= libc::CS8;
        raw.c_cflag &= !libc::PARENB;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut term = Terminal {
            original,
            raw: true,
            pending: None,
            width: 80,
            height: 24,
        };
        term.update_size();
        term.write(
            b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[?1004h\x1b[?1000h\x1b[?1002h\x1b[?1006h",
        )?;
        Ok(term)
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        // Write straight to fd 1. Rust's Stdout wrapper adds a line buffer and
        // an extra copy, which is measurable on the first frame.
        let mut offset = 0usize;
        while offset < bytes.len() {
            let written = unsafe {
                libc::write(
                    1,
                    bytes[offset..].as_ptr() as *const libc::c_void,
                    bytes.len() - offset,
                )
            };
            if written < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if written == 0 {
                break;
            }
            offset += written as usize;
        }
        Ok(())
    }

    pub fn update_size(&mut self) {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let result = unsafe { libc::ioctl(0, libc::TIOCGWINSZ, &mut ws) };
        if result == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            self.width = ws.ws_col as usize;
            self.height = ws.ws_row as usize;
        } else {
            self.width = 80;
            self.height = 24;
        }
    }

    /// Returns `None` when the poll timeout expires. The timeout is used both
    /// for resize polling and to keep the loop cheap when idle.
    pub fn read_event(&mut self, timeout_ms: i32) -> io::Result<Option<Event>> {
        let first = match self.read_byte(timeout_ms)? {
            Some(byte) => byte,
            None => return Ok(None),
        };
        if first == 0x1B {
            return self.read_escape().map(Some);
        }

        let event = match first {
            b'\r' | b'\n' => Event::plain(Key::Enter),
            0x7F | 0x08 => Event::plain(Key::Backspace),
            b'\t' => Event::plain(Key::Tab),
            0x00 => Event {
                key: Key::Char,
                ch: ' ',
                ctrl: true,
                alt: false,
                shift: false,
                mouse_x: 0,
                mouse_y: 0,
                mouse_button: MouseButton::None,
            },
            0x01..=0x1A => Event {
                key: Key::Char,
                ch: (b'a' + (first - 1)) as char,
                ctrl: true,
                alt: false,
                shift: false,
                mouse_x: 0,
                mouse_y: 0,
                mouse_button: MouseButton::None,
            },
            0x1C..=0x1F => Event::plain(Key::Unknown),
            0x20..=0x7E => Event {
                key: Key::Char,
                ch: first as char,
                ctrl: false,
                alt: false,
                shift: false,
                mouse_x: 0,
                mouse_y: 0,
                mouse_button: MouseButton::None,
            },
            _ => {
                let mut bytes = [0u8; 4];
                bytes[0] = first;
                let needed = utf8_len(first).unwrap_or(1);
                let mut count = 1;
                while count < needed {
                    match self.read_byte(50)? {
                        Some(next) => {
                            bytes[count] = next;
                            count += 1;
                        }
                        None => break,
                    }
                }
                let ch = std::str::from_utf8(&bytes[..count])
                    .ok()
                    .and_then(|s| s.chars().next())
                    .unwrap_or('\u{FFFD}');
                Event {
                    key: Key::Char,
                    ch,
                    ctrl: false,
                    alt: false,
                    shift: false,
                    mouse_x: 0,
                    mouse_y: 0,
                    mouse_button: MouseButton::None,
                }
            }
        };
        Ok(Some(event))
    }

    fn read_byte(&mut self, timeout_ms: i32) -> io::Result<Option<u8>> {
        if let Some(byte) = self.pending.take() {
            return Ok(Some(byte));
        }

        let mut fd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Ok(None);
        }
        if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "terminal closed",
            ));
        }
        if fd.revents & libc::POLLIN == 0 {
            return Ok(None);
        }

        let mut byte = 0u8;
        let count = unsafe { libc::read(0, &mut byte as *mut u8 as *mut libc::c_void, 1) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "terminal closed",
            ));
        }
        Ok(Some(byte))
    }

    fn read_escape(&mut self) -> io::Result<Event> {
        let next = match self.read_byte(25)? {
            Some(byte) => byte,
            None => return Ok(Event::plain(Key::Escape)),
        };

        if next != b'[' && next != b'O' {
            // A bare ESC followed quickly by another key is far more likely to
            // be Esc-then-key than Alt+key. Re-queue the byte so the next event
            // reads it normally; this also makes scripted Esc followed by ':'
            // reliable.
            self.pending = Some(next);
            return Ok(Event::plain(Key::Escape));
        }

        let csi = next == b'[';
        let mut params = Vec::with_capacity(8);
        let mut final_byte = 0u8;
        for _ in 0..32 {
            let byte = match self.read_byte(25)? {
                Some(byte) => byte,
                None => return Ok(Event::plain(Key::Escape)),
            };
            if (0x40..=0x7E).contains(&byte) {
                final_byte = byte;
                break;
            }
            params.push(byte);
        }
        if final_byte == 0 {
            return Ok(Event::plain(Key::Unknown));
        }

        if !csi {
            let key = match final_byte {
                b'A' => Key::Up,
                b'B' => Key::Down,
                b'C' => Key::Right,
                b'D' => Key::Left,
                b'H' => Key::Home,
                b'F' => Key::End,
                _ => Key::Unknown,
            };
            return Ok(Event::plain(key));
        }

        if params.first() == Some(&b'<') && matches!(final_byte, b'M' | b'm') {
            return Ok(decode_mouse(&params[1..], final_byte));
        }

        let mut event = Event::plain(Key::Unknown);
        apply_modifiers(&params, &mut event);
        event.key = match final_byte {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            b'H' => Key::Home,
            b'F' => Key::End,
            b'Z' => Key::BackTab,
            b'~' => decode_tilde(&params),
            b'I' | b'O' => Key::Unknown,
            _ => Key::Unknown,
        };
        Ok(event)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.raw {
            let _ = self
                .write(b"\x1b[0m\x1b[?25h\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?1004l\x1b[?1049l");
            unsafe {
                libc::tcsetattr(0, libc::TCSAFLUSH, &self.original);
            }
            self.raw = false;
        }
    }
}

fn decode_mouse(parameters: &[u8], final_byte: u8) -> Event {
    let mut index = 0usize;
    let button = read_mouse_number(parameters, &mut index);
    let x = read_mouse_number(parameters, &mut index);
    let y = read_mouse_number(parameters, &mut index);

    let mut event = Event::plain(Key::Unknown);
    event.mouse_x = x.saturating_sub(1) as usize;
    event.mouse_y = y.saturating_sub(1) as usize;
    event.shift = button & 4 != 0;
    event.alt = button & 8 != 0;
    event.ctrl = button & 16 != 0;

    let button_number = button & 3;
    if button & 64 != 0 {
        event.key = match button_number {
            0 => Key::ScrollUp,
            1 => Key::ScrollDown,
            _ => Key::Unknown,
        };
        return event;
    }

    event.mouse_button = match button_number {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    };
    if final_byte != b'm' && button & 32 != 0 {
        event.key = Key::MouseMove;
        return event;
    }
    event.key = if final_byte == b'm' {
        Key::MouseUp
    } else {
        Key::MouseDown
    };
    event
}

fn read_mouse_number(parameters: &[u8], index: &mut usize) -> i64 {
    let mut value = 0i64;
    while *index < parameters.len() && parameters[*index].is_ascii_digit() {
        value = value * 10 + (parameters[*index] - b'0') as i64;
        *index += 1;
    }
    if *index < parameters.len() && parameters[*index] == b';' {
        *index += 1;
    }
    value
}

fn apply_modifiers(params: &[u8], event: &mut Event) {
    let Some(semicolon) = params.iter().rposition(|&b| b == b';') else {
        return;
    };
    let mut modifier = 0u32;
    for &byte in &params[semicolon + 1..] {
        if !byte.is_ascii_digit() {
            break;
        }
        modifier = modifier * 10 + (byte - b'0') as u32;
    }
    if modifier == 0 {
        return;
    }
    let mask = modifier - 1;
    event.shift = mask & 1 != 0;
    event.alt = mask & 2 != 0;
    event.ctrl = mask & 4 != 0;
}

fn decode_tilde(params: &[u8]) -> Key {
    let mut code = 0u32;
    for &byte in params {
        if !byte.is_ascii_digit() {
            break;
        }
        code = code * 10 + (byte - b'0') as u32;
    }
    match code {
        1 | 7 => Key::Home,
        4 | 8 => Key::End,
        2 => Key::Insert,
        3 => Key::Delete,
        5 => Key::PageUp,
        6 => Key::PageDown,
        _ => Key::Unknown,
    }
}

fn utf8_len(first: u8) -> Option<usize> {
    match first {
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, MouseButton, decode_mouse};

    #[test]
    fn decodes_sgr_mouse_events() {
        let down = decode_mouse(b"0;10;5", b'M');
        assert_eq!(down.key, Key::MouseDown);
        assert_eq!(down.mouse_button, MouseButton::Left);
        assert_eq!((down.mouse_x, down.mouse_y), (9, 4));

        let up = decode_mouse(b"0;10;5", b'm');
        assert_eq!(up.key, Key::MouseUp);
        assert_eq!(up.mouse_button, MouseButton::Left);

        let drag = decode_mouse(b"32;11;6", b'M');
        assert_eq!(drag.key, Key::MouseMove);
        assert_eq!(drag.mouse_button, MouseButton::Left);
        assert_eq!((drag.mouse_x, drag.mouse_y), (10, 5));

        let wheel = decode_mouse(b"64;1;1", b'M');
        assert_eq!(wheel.key, Key::ScrollUp);
    }
}
