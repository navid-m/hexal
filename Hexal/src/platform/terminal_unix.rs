//! POSIX terminal handling: termios, poll-based input and an ANSI decoder.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use std::io;

pub struct Terminal {
    input_fd: libc::c_int,
    output_fd: libc::c_int,
    owns_input_fd: bool,
    owns_output_fd: bool,
    nonblocking_input: bool,
    original: libc::termios,
    raw: bool,
    pending: Option<u8>,
    pub width: usize,
    pub height: usize,
}

impl Terminal {
    pub fn init() -> io::Result<Terminal> {
        // Use stdin/stdout when they are both terminals, but fall back to the
        // controlling terminal when a launcher such as Git redirects either
        // stream. Reading/writing fd 0/1 in that case fails with ENODEV.
        let stdin_is_tty = unsafe { libc::isatty(0) } == 1;
        let stdout_is_tty = unsafe { libc::isatty(1) } == 1;
        let (mut input_fd, mut output_fd) = (0, 1);
        let (mut owns_input_fd, mut owns_output_fd) = (false, false);
        let mut nonblocking_input = false;
        if !stdin_is_tty || !stdout_is_tty {
            // Use separate descriptors: input must be nonblocking because
            // macOS cannot poll/select /dev/tty, while output stays blocking
            // so large frame writes cannot fail with EAGAIN.
            let tty_in = unsafe {
                libc::open(
                    b"/dev/tty\0".as_ptr() as *const libc::c_char,
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            let tty_out = unsafe {
                libc::open(
                    b"/dev/tty\0".as_ptr() as *const libc::c_char,
                    libc::O_WRONLY | libc::O_CLOEXEC,
                )
            };
            if tty_in >= 0 && tty_out >= 0 {
                input_fd = tty_in;
                output_fd = tty_out;
                owns_input_fd = true;
                owns_output_fd = true;
                nonblocking_input = true;
            } else {
                if tty_in >= 0 {
                    unsafe {
                        libc::close(tty_in);
                    }
                }
                if tty_out >= 0 {
                    unsafe {
                        libc::close(tty_out);
                    }
                }
            }
        }

        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(input_fd, &mut original) } != 0 {
            let error = io::Error::last_os_error();
            if owns_input_fd {
                unsafe {
                    libc::close(input_fd);
                }
            }
            if owns_output_fd {
                unsafe {
                    libc::close(output_fd);
                }
            }
            return Err(error);
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
        if unsafe { libc::tcsetattr(input_fd, libc::TCSAFLUSH, &raw) } != 0 {
            let error = io::Error::last_os_error();
            if owns_input_fd {
                unsafe {
                    libc::close(input_fd);
                }
            }
            if owns_output_fd {
                unsafe {
                    libc::close(output_fd);
                }
            }
            return Err(error);
        }

        let mut term = Terminal {
            input_fd,
            output_fd,
            owns_input_fd,
            owns_output_fd,
            nonblocking_input,
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
                    self.output_fd,
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
        let result = unsafe { libc::ioctl(self.input_fd, libc::TIOCGWINSZ, &mut ws) };
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

        if self.nonblocking_input {
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(timeout_ms.max(0) as u64);
            loop {
                let mut byte = 0u8;
                let count = unsafe {
                    libc::read(
                        self.input_fd,
                        &mut byte as *mut u8 as *mut libc::c_void,
                        1,
                    )
                };
                if count > 0 {
                    return Ok(Some(byte));
                }
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "terminal closed",
                    ));
                }
                let error = io::Error::last_os_error();
                let raw = error.raw_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.kind() != io::ErrorKind::WouldBlock
                    && raw != Some(libc::EAGAIN)
                    && raw != Some(libc::EWOULDBLOCK)
                {
                    return Err(error);
                }
                if std::time::Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        let mut fd = libc::pollfd {
            fd: self.input_fd,
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
        let count = unsafe { libc::read(self.input_fd, &mut byte as *mut u8 as *mut libc::c_void, 1) };
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
                libc::tcsetattr(self.input_fd, libc::TCSAFLUSH, &self.original);
            }
            self.raw = false;
        }
        if self.owns_input_fd {
            unsafe {
                libc::close(self.input_fd);
            }
            self.owns_input_fd = false;
        }
        if self.owns_output_fd {
            unsafe {
                libc::close(self.output_fd);
            }
            self.owns_output_fd = false;
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
