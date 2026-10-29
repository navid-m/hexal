//! Windows Console terminal handling.
//!
//! Uses `ReadConsoleInputW` for input (including mouse events) and virtual
//! terminal sequences for output, matching the C# `WindowsTerminal`.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;
use std::ffi::c_void;
use std::io;

type Handle = *mut c_void;

const STD_INPUT_HANDLE: i32 = -10;
const STD_OUTPUT_HANDLE: i32 = -11;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;

const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
const ENABLE_LINE_INPUT: u32 = 0x0002;
const ENABLE_ECHO_INPUT: u32 = 0x0004;
const ENABLE_WINDOW_INPUT: u32 = 0x0008;
const ENABLE_MOUSE_INPUT: u32 = 0x0010;
const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;

const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x0008;

const LEFT_ALT_PRESSED: u32 = 0x0001;
const LEFT_CTRL_PRESSED: u32 = 0x0002;
const RIGHT_ALT_PRESSED: u32 = 0x0004;
const RIGHT_CTRL_PRESSED: u32 = 0x0008;
const SHIFT_PRESSED: u32 = 0x0010;

const KEY_EVENT: u16 = 0x0001;
const MOUSE_EVENT: u16 = 0x0002;
const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;
const FOCUS_EVENT: u16 = 0x0010;

const MOUSE_MOVED: u32 = 0x0001;
const MOUSE_WHEELED: u32 = 0x0004;
const MOUSE_HWHEELED: u32 = 0x0008;
const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;

const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x0000_0102;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmallRect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ConsoleScreenBufferInfo {
    size: Coord,
    cursor_position: Coord,
    attributes: u16,
    window: SmallRect,
    maximum_window_size: Coord,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyEventRecord {
    key_down: i32,
    repeat_count: u16,
    virtual_key_code: u16,
    virtual_scan_code: u16,
    unicode_char: u16,
    control_key_state: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MouseEventRecord {
    mouse_position: Coord,
    button_state: u32,
    control_key_state: u32,
    event_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WindowBufferSizeRecord {
    size: Coord,
}

#[repr(C)]
#[derive(Clone, Copy)]
union EventUnion {
    key: KeyEventRecord,
    mouse: MouseEventRecord,
    window: WindowBufferSizeRecord,
    focus: i32,
}

impl Default for EventUnion {
    fn default() -> EventUnion {
        EventUnion {
            key: KeyEventRecord::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputRecord {
    event_type: u16,
    _padding: u16,
    event: EventUnion,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(std_handle: i32) -> Handle;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetConsoleScreenBufferInfo(
        console_handle: Handle,
        info: *mut ConsoleScreenBufferInfo,
    ) -> i32;
    fn GetConsoleMode(console_handle: Handle, mode: *mut u32) -> i32;
    fn SetConsoleMode(console_handle: Handle, mode: u32) -> i32;
    fn ReadConsoleInputW(
        console_handle: Handle,
        records: *mut InputRecord,
        length: u32,
        read: *mut u32,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetConsoleCP() -> u32;
    fn GetConsoleOutputCP() -> u32;
    fn SetConsoleCP(code_page: u32) -> i32;
    fn SetConsoleOutputCP(code_page: u32) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const c_void,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

pub struct Terminal {
    input: Handle,
    output: Handle,
    owns_input: bool,
    owns_output: bool,
    original_input_mode: u32,
    original_output_mode: u32,
    original_input_code_page: u32,
    original_output_code_page: u32,
    raw: bool,
    pending_high_surrogate: u16,
    stashed_key: Option<KeyEventRecord>,
    repeat_event: Option<Event>,
    repeat_remaining: u16,
    mouse_left_down: bool,
    pub width: usize,
    pub height: usize,
}

impl Terminal {
    pub fn init() -> io::Result<Terminal> {
        let mut input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        let mut owns_input = false;
        let mut owns_output = false;
        let mut mode = 0u32;

        if invalid_handle(input) || unsafe { GetConsoleMode(input, &mut mode) } == 0 {
            input = create_console_file("CONIN$");
            if invalid_handle(input) {
                return Err(io::Error::last_os_error());
            }
            owns_input = true;
        }
        if invalid_handle(output) || unsafe { GetConsoleMode(output, &mut mode) } == 0 {
            output = create_console_file("CONOUT$");
            if invalid_handle(output) {
                if owns_input {
                    unsafe {
                        CloseHandle(input);
                    }
                }
                return Err(io::Error::last_os_error());
            }
            owns_output = true;
        }

        let mut original_input_mode = 0u32;
        let mut original_output_mode = 0u32;
        if unsafe { GetConsoleMode(input, &mut original_input_mode) } == 0
            || unsafe { GetConsoleMode(output, &mut original_output_mode) } == 0
        {
            close_owned(input, output, owns_input, owns_output);
            return Err(io::Error::last_os_error());
        }

        let input_mode = (original_input_mode
            & !(ENABLE_ECHO_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_QUICK_EDIT_MODE))
            | ENABLE_WINDOW_INPUT
            | ENABLE_EXTENDED_FLAGS
            | ENABLE_MOUSE_INPUT;
        if unsafe { SetConsoleMode(input, input_mode) } == 0 {
            close_owned(input, output, owns_input, owns_output);
            return Err(io::Error::last_os_error());
        }

        let output_mode = original_output_mode
            | ENABLE_PROCESSED_OUTPUT
            | ENABLE_VIRTUAL_TERMINAL_PROCESSING
            | DISABLE_NEWLINE_AUTO_RETURN;
        if unsafe { SetConsoleMode(output, output_mode) } == 0 {
            unsafe {
                SetConsoleMode(input, original_input_mode);
            }
            close_owned(input, output, owns_input, owns_output);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Windows 10 or newer with virtual terminal support is required",
            ));
        }

        let original_input_code_page = unsafe { GetConsoleCP() };
        let original_output_code_page = unsafe { GetConsoleOutputCP() };
        unsafe {
            SetConsoleCP(65001);
            SetConsoleOutputCP(65001);
        }

        let mut terminal = Terminal {
            input,
            output,
            owns_input,
            owns_output,
            original_input_mode,
            original_output_mode,
            original_input_code_page,
            original_output_code_page,
            raw: true,
            pending_high_surrogate: 0,
            stashed_key: None,
            repeat_event: None,
            repeat_remaining: 0,
            mouse_left_down: false,
            width: 80,
            height: 24,
        };
        terminal.update_size();
        terminal.write(b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H")?;
        Ok(terminal)
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut offset = 0usize;
        while offset < bytes.len() {
            let chunk = (bytes.len() - offset).min(u32::MAX as usize) as u32;
            let mut written = 0u32;
            let result = unsafe {
                WriteFile(
                    self.output,
                    bytes[offset..].as_ptr() as *const c_void,
                    chunk,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if result == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                break;
            }
            offset += written as usize;
        }
        Ok(())
    }

    pub fn update_size(&mut self) {
        let mut info = ConsoleScreenBufferInfo::default();
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } != 0 {
            let columns = info.window.right - info.window.left + 1;
            let rows = info.window.bottom - info.window.top + 1;
            if columns > 0 && rows > 0 {
                self.width = columns as usize;
                self.height = rows as usize;
                return;
            }
        }
        self.width = 80;
        self.height = 24;
    }

    pub fn read_event(&mut self, timeout_ms: i32) -> io::Result<Option<Event>> {
        loop {
            if self.repeat_remaining > 0 {
                self.repeat_remaining -= 1;
                if let Some(event) = self.repeat_event {
                    return Ok(Some(event));
                }
            }

            let key = if let Some(key) = self.stashed_key.take() {
                key
            } else {
                match self.read_key_event(timeout_ms)? {
                    Some(ReadRecord::Key(key)) => key,
                    Some(ReadRecord::Event(event)) => return Ok(Some(event)),
                    None => return Ok(None),
                }
            };

            let character = key.unicode_char;

            if self.pending_high_surrogate != 0 {
                let high = self.pending_high_surrogate;
                self.pending_high_surrogate = 0;
                if is_low_surrogate(character) {
                    let code =
                        0x10000 + (((high as u32 - 0xD800) << 10) | (character as u32 - 0xDC00));
                    let mut event = Event::plain(Key::Char);
                    event.ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                    apply_modifiers(key.control_key_state, &mut event);
                    self.store_repeat(key.repeat_count, event);
                    return Ok(Some(event));
                }
                self.stashed_key = Some(key);
                let mut event = Event::plain(Key::Char);
                event.ch = '\u{FFFD}';
                return Ok(Some(event));
            }

            if is_high_surrogate(character) {
                self.pending_high_surrogate = character;
                continue;
            }

            let mut event = decode_key(key);
            if is_low_surrogate(character) {
                event.key = Key::Char;
                event.ch = '\u{FFFD}';
            }
            self.store_repeat(key.repeat_count, event);
            return Ok(Some(event));
        }
    }

    fn read_key_event(&mut self, timeout_ms: i32) -> io::Result<Option<ReadRecord>> {
        loop {
            if timeout_ms >= 0 {
                match unsafe { WaitForSingleObject(self.input, timeout_ms as u32) } {
                    WAIT_TIMEOUT => return Ok(None),
                    WAIT_FAILED => return Err(io::Error::last_os_error()),
                    WAIT_OBJECT_0 => {}
                    _ => {}
                }
            }

            let mut record = InputRecord::default();
            let mut read = 0u32;
            if unsafe { ReadConsoleInputW(self.input, &mut record, 1, &mut read) } == 0 || read == 0
            {
                return Err(io::Error::last_os_error());
            }

            match record.event_type {
                WINDOW_BUFFER_SIZE_EVENT => {
                    self.update_size();
                    continue;
                }
                FOCUS_EVENT => continue,
                MOUSE_EVENT => {
                    let mouse = unsafe { record.event.mouse };
                    if let Some(event) = self.decode_mouse(mouse) {
                        return Ok(Some(ReadRecord::Event(event)));
                    }
                    continue;
                }
                KEY_EVENT => {
                    let key = unsafe { record.event.key };
                    if key.key_down == 0 {
                        continue;
                    }
                    return Ok(Some(ReadRecord::Key(key)));
                }
                _ => continue,
            }
        }
    }

    fn decode_mouse(&mut self, mouse: MouseEventRecord) -> Option<Event> {
        if mouse.event_flags & MOUSE_HWHEELED != 0 {
            return None;
        }

        let mut event = Event::plain(Key::Unknown);
        event.mouse_x = mouse.mouse_position.x.max(0) as usize;
        event.mouse_y = mouse.mouse_position.y.max(0) as usize;
        apply_modifiers(mouse.control_key_state, &mut event);

        if mouse.event_flags & MOUSE_WHEELED != 0 {
            let delta = ((mouse.button_state >> 16) & 0xFFFF) as u16 as i16;
            if delta == 0 {
                return None;
            }
            event.key = if delta > 0 {
                Key::ScrollUp
            } else {
                Key::ScrollDown
            };
            return Some(event);
        }

        let left_down = mouse.button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0;
        if mouse.event_flags & MOUSE_MOVED != 0 {
            if self.mouse_left_down && left_down {
                event.key = Key::MouseMove;
                event.mouse_button = MouseButton::Left;
                return Some(event);
            }
            return None;
        }

        if left_down && !self.mouse_left_down {
            self.mouse_left_down = true;
            event.key = Key::MouseDown;
            event.mouse_button = MouseButton::Left;
            return Some(event);
        }
        if !left_down && self.mouse_left_down {
            self.mouse_left_down = false;
            event.key = Key::MouseUp;
            event.mouse_button = MouseButton::Left;
            return Some(event);
        }
        None
    }

    fn store_repeat(&mut self, repeat_count: u16, event: Event) {
        if repeat_count <= 1 {
            return;
        }
        self.repeat_event = Some(event);
        self.repeat_remaining = repeat_count - 1;
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.raw {
            let _ = self.write(b"\x1b[0m\x1b[?25h\x1b[?1049l");
            unsafe {
                SetConsoleMode(self.input, self.original_input_mode);
                SetConsoleMode(self.output, self.original_output_mode);
                SetConsoleCP(self.original_input_code_page);
                SetConsoleOutputCP(self.original_output_code_page);
            }
            close_owned(self.input, self.output, self.owns_input, self.owns_output);
            self.raw = false;
        }
    }
}

enum ReadRecord {
    Key(KeyEventRecord),
    Event(Event),
}

fn create_console_file(name: &str) -> Handle {
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    }
}

fn invalid_handle(handle: Handle) -> bool {
    handle.is_null() || handle as isize == -1
}

fn close_owned(input: Handle, output: Handle, owns_input: bool, owns_output: bool) {
    unsafe {
        if owns_input && !invalid_handle(input) {
            CloseHandle(input);
        }
        if owns_output && !invalid_handle(output) {
            CloseHandle(output);
        }
    }
}

fn is_high_surrogate(value: u16) -> bool {
    (0xD800..=0xDBFF).contains(&value)
}

fn is_low_surrogate(value: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&value)
}

fn decode_key(key: KeyEventRecord) -> Event {
    let mut result = Event::plain(Key::Unknown);
    apply_modifiers(key.control_key_state, &mut result);

    let mut ctrl = result.ctrl;
    let mut alt = result.alt;
    let character = key.unicode_char;

    match key.virtual_key_code {
        VK_RETURN => {
            result.key = Key::Enter;
            return result;
        }
        VK_ESCAPE => {
            result.key = Key::Escape;
            return result;
        }
        VK_BACK => {
            result.key = Key::Backspace;
            return result;
        }
        VK_TAB => {
            result.key = if result.shift { Key::BackTab } else { Key::Tab };
            return result;
        }
        VK_DELETE => {
            result.key = Key::Delete;
            return result;
        }
        VK_LEFT => {
            result.key = Key::Left;
            return result;
        }
        VK_RIGHT => {
            result.key = Key::Right;
            return result;
        }
        VK_UP => {
            result.key = Key::Up;
            return result;
        }
        VK_DOWN => {
            result.key = Key::Down;
            return result;
        }
        VK_HOME => {
            result.key = Key::Home;
            return result;
        }
        VK_END => {
            result.key = Key::End;
            return result;
        }
        VK_PRIOR => {
            result.key = Key::PageUp;
            return result;
        }
        VK_NEXT => {
            result.key = Key::PageDown;
            return result;
        }
        VK_INSERT => {
            result.key = Key::Insert;
            return result;
        }
        _ => {}
    }

    if ctrl && character == 0x1B {
        result.key = Key::Escape;
        return result;
    }
    if character == '\r' as u16 || character == '\n' as u16 {
        result.key = Key::Enter;
        return result;
    }
    if character == '\t' as u16 {
        result.key = Key::Tab;
        return result;
    }
    if character == 0x08 || character == 0x7F {
        result.key = Key::Backspace;
        return result;
    }
    if ctrl && alt && character >= 0x20 {
        ctrl = false;
        alt = false;
    }
    if ctrl && !alt && (0x01..=0x1A).contains(&character) {
        result.key = Key::Char;
        result.ch = (b'a' + (character as u8 - 1)) as char;
        result.ctrl = true;
        result.alt = false;
        return result;
    }
    if character == 0 {
        if ctrl && !alt && (b'A' as u16..=b'Z' as u16).contains(&key.virtual_key_code) {
            result.key = Key::Char;
            result.ch = (b'a' + (key.virtual_key_code as u8 - b'A')) as char;
            result.ctrl = true;
            result.alt = false;
            return result;
        }
        result.key = Key::Unknown;
        return result;
    }
    if character < 0x20 {
        result.key = Key::Unknown;
        return result;
    }

    result.key = Key::Char;
    result.ch = char::from_u32(character as u32).unwrap_or('\u{FFFD}');
    result.ctrl = ctrl;
    result.alt = alt;
    result.shift = false;
    result
}

fn apply_modifiers(state: u32, event: &mut Event) {
    event.ctrl = state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
    event.alt = state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0;
    event.shift = state & SHIFT_PRESSED != 0;
}
