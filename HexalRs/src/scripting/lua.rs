//! Embedded Lua 5.4 runtime and a small, buffer-oriented plugin API.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::alloc::{alloc as rust_alloc, dealloc as rust_dealloc, Layout};
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::slice;

use mlua::{ffi, Lua, StdLib};

const LUA_GRAIN: usize = 16;
const LUA_SMALL_LIMIT: usize = 1024;
const LUA_CLASSES: usize = LUA_SMALL_LIMIT / LUA_GRAIN;
const LUA_MAX_CACHED: u16 = 512;

struct FreeNode {
    next: *mut FreeNode,
}

/// Lua's allocator is extremely hot while `string.gmatch` creates short
/// strings. Keep freed small blocks in size-class lists, mirroring the Zig
/// host.
struct LuaAllocator {
    heads: [*mut FreeNode; LUA_CLASSES],
    counts: [u16; LUA_CLASSES],
}

impl LuaAllocator {
    fn new() -> Box<LuaAllocator> {
        Box::new(LuaAllocator {
            heads: [std::ptr::null_mut(); LUA_CLASSES],
            counts: [0; LUA_CLASSES],
        })
    }

    #[inline]
    fn class(size: usize) -> Option<usize> {
        if size == 0 || size > LUA_SMALL_LIMIT {
            None
        } else {
            Some((size - 1) / LUA_GRAIN)
        }
    }

    #[inline]
    unsafe fn alloc(&mut self, size: usize) -> *mut u8 {
        if let Some(class) = Self::class(size) {
            let head = self.heads[class];
            if !head.is_null() {
                self.heads[class] = unsafe { (*head).next };
                self.counts[class] -= 1;
                return head as *mut u8;
            }
            let block = (class + 1) * LUA_GRAIN;
            return unsafe {
                rust_alloc(Layout::from_size_align_unchecked(block, LUA_GRAIN))
            };
        }
        unsafe { rust_alloc(Layout::from_size_align_unchecked(size, LUA_GRAIN)) }
    }

    #[inline]
    unsafe fn dealloc(&mut self, pointer: *mut u8, size: usize) {
        if let Some(class) = Self::class(size) {
            if self.counts[class] < LUA_MAX_CACHED {
                let node = pointer as *mut FreeNode;
                unsafe { (*node).next = self.heads[class] };
                self.heads[class] = node;
                self.counts[class] += 1;
                return;
            }
            let block = (class + 1) * LUA_GRAIN;
            unsafe {
                rust_dealloc(pointer, Layout::from_size_align_unchecked(block, LUA_GRAIN))
            };
            return;
        }
        unsafe {
            rust_dealloc(pointer, Layout::from_size_align_unchecked(size, LUA_GRAIN))
        };
    }

    unsafe fn realloc(&mut self, pointer: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
        if let (Some(old_class), Some(new_class)) =
            (Self::class(old_size), Self::class(new_size))
        {
            if old_class == new_class {
                return pointer;
            }
        }
        let replacement = unsafe { self.alloc(new_size) };
        if !replacement.is_null() && !pointer.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(pointer, replacement, old_size.min(new_size));
            }
        }
        if !pointer.is_null() {
            unsafe { self.dealloc(pointer, old_size) };
        }
        replacement
    }
}

impl Drop for LuaAllocator {
    fn drop(&mut self) {
        for class in 0..LUA_CLASSES {
            let mut node = self.heads[class];
            while !node.is_null() {
                let next = unsafe { (*node).next };
                unsafe {
                    rust_dealloc(
                        node as *mut u8,
                        Layout::from_size_align_unchecked(
                            (class + 1) * LUA_GRAIN,
                            LUA_GRAIN,
                        ),
                    )
                };
                node = next;
            }
        }
    }
}

use crate::core::document::Document;

/// In-memory buffer exposed to Lua. Keeping a snapshot makes plugin callbacks
/// simple and lets commands be atomic from the editor's point of view.
struct LuaHost {
    text: Vec<u8>,
    /// Non-owning view of the document's original buffer after a write-back.
    /// Materialised into `text` before any host-side mutation.
    text_view: Option<(*const u8, usize)>,
    line_starts: Vec<usize>,
    cursor_line: usize,
    cursor_column: usize,
    dirty: bool,
    message: Option<String>,
}

impl LuaHost {
    fn new() -> LuaHost {
        LuaHost {
            text: Vec::new(),
            text_view: None,
            line_starts: vec![0],
            cursor_line: 0,
            cursor_column: 0,
            dirty: false,
            message: None,
        }
    }

    fn load(&mut self, text: Vec<u8>, cursor_line: usize, cursor_column: usize) {
        self.text = text;
        self.text_view = None;
        self.rebuild_lines();
        self.cursor_line = cursor_line.min(self.line_count().saturating_sub(1));
        self.cursor_column = cursor_column;
        self.dirty = false;
        self.message = None;
    }

    fn bytes(&self) -> &[u8] {
        match self.text_view {
            // Safety: the pointer is the document's immutable original buffer
            // and is cleared before any host-side mutation or document edit.
            Some((pointer, length)) => unsafe { slice::from_raw_parts(pointer, length) },
            None => &self.text,
        }
    }

    fn materialize(&mut self) {
        let Some((pointer, length)) = self.text_view.take() else {
            return;
        };
        // Safety: see `bytes`.
        let bytes = unsafe { slice::from_raw_parts(pointer, length) };
        self.text.clear();
        self.text.extend_from_slice(bytes);
    }

    fn rebuild_lines(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        let bytes = &self.text;
        let mut position = 0usize;
        while position < bytes.len() {
            let found = unsafe {
                libc::memchr(
                    bytes[position..].as_ptr() as *const c_void,
                    b'\n' as c_int,
                    bytes.len() - position,
                )
            };
            if found.is_null() {
                break;
            }
            let index = found as usize - bytes.as_ptr() as usize;
            self.line_starts.push(index + 1);
            position = index + 1;
        }
    }

    fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    fn line_start(&self, line: usize) -> usize {
        self.line_starts[line.min(self.line_count() - 1)]
    }

    #[inline]
    fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        if line >= self.line_starts.len() {
            return None;
        }
        let text = self.bytes();
        let start = self.line_starts[line];
        if line + 1 >= self.line_starts.len() {
            return Some((start, text.len()));
        }
        // Line starts are recorded immediately after a newline, so the byte
        // before the next start is the terminator and does not need a check.
        let mut end = self.line_starts[line + 1] - 1;
        if end > start && text[end - 1] == b'\r' {
            end -= 1;
        }
        Some((start, end))
    }

    #[inline]
    fn line_end(&self, line: usize) -> usize {
        self.line_bounds(line)
            .map(|(_, end)| end)
            .unwrap_or_else(|| self.bytes().len())
    }

    #[inline]
    fn line_bytes(&self, line: usize) -> Option<&[u8]> {
        let (start, end) = self.line_bounds(line)?;
        Some(&self.bytes()[start..end])
    }

    fn set_line(&mut self, line: usize, value: &[u8]) -> bool {
        if line >= self.line_count() {
            return false;
        }
        self.materialize();
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.text.splice(start..end, value.iter().copied());
        self.rebuild_lines();
        self.dirty = true;
        true
    }

    fn set_text(&mut self, value: &[u8]) {
        self.text_view = None;
        self.text.clear();
        self.text.extend_from_slice(value);
        self.rebuild_lines();
        self.cursor_line = self.cursor_line.min(self.line_count().saturating_sub(1));
        self.dirty = true;
    }

    fn byte_offset(&self) -> usize {
        let line = self.cursor_line.min(self.line_count() - 1);
        let (start, end) = self.line_bounds(line).unwrap_or((0, 0));
        start + self.cursor_column.min(end.saturating_sub(start))
    }

    fn set_cursor(&mut self, line: usize, column: usize) {
        self.cursor_line = line.min(self.line_count().saturating_sub(1));
        let (start, end) = self.line_bounds(self.cursor_line).unwrap_or((0, 0));
        self.cursor_column = column.min(end.saturating_sub(start));
    }

    fn insert(&mut self, value: &[u8]) {
        self.materialize();
        let offset = self.byte_offset();
        self.text.splice(offset..offset, value.iter().copied());
        self.rebuild_lines();
        let new_offset = offset + value.len();
        let line = line_of_offset_in(&self.line_starts, new_offset);
        self.cursor_line = line;
        self.cursor_column = new_offset - self.line_starts[line];
        self.dirty = true;
    }

    fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }
}

/// Pointer passed as a Lua light userdata upvalue. Boxed by LuaRuntime so its
/// address stays stable for the lifetime of the Lua state.
struct RawContext {
    // Single-threaded, synchronous callbacks: avoid RefCell checks on the hot
    // host API path. Runtime and callbacks never overlap access to the host.
    host: UnsafeCell<LuaHost>,
    commands: RefCell<HashMap<String, c_int>>,
}

pub struct LuaRuntime {
    lua: &'static Lua,
    state: *mut ffi::lua_State,
    ctx: Box<RawContext>,
    _allocator: Box<LuaAllocator>,
    loaded_revision: Option<u64>,
    synced_editor_cursor: Option<usize>,
    last_command: Option<(String, c_int)>,
    message: Option<String>,
}

impl LuaRuntime {
    pub fn new() -> Result<LuaRuntime, mlua::Error> {
        // Create the state with Rust's allocator so Lua strings and tables use
        // the same fast allocator as the rest of the editor.
        let mut allocator = LuaAllocator::new();
        let state = unsafe {
            ffi::lua_newstate(lua_alloc, (&mut *allocator) as *mut LuaAllocator as *mut c_void)
        };
        if state.is_null() {
            return Err(mlua::Error::RuntimeError("cannot create Lua state".into()));
        }
        // Safety: the state is valid for the lifetime of this runtime, and the
        // returned reference is not used after Drop closes the state.
        let lua: &'static Lua = unsafe { Lua::get_or_init_from_ptr(state) };
        if let Err(error) = lua.load_std_libs(StdLib::ALL_SAFE) {
            unsafe { ffi::lua_close(state) };
            return Err(error);
        }

        let ctx = Box::new(RawContext {
            host: UnsafeCell::new(LuaHost::new()),
            commands: RefCell::new(HashMap::new()),
        });
        // Safety: ctx is boxed, lives as long as the runtime, and is only
        // dereferenced from synchronous Lua callbacks on this thread.
        unsafe {
            register_api(state, &*ctx);
        }
        Ok(LuaRuntime {
            lua,
            state,
            ctx,
            _allocator: allocator,
            loaded_revision: None,
            synced_editor_cursor: None,
            last_command: None,
            message: None,
        })
    }

    pub fn load_file(&mut self, path: &str) -> Result<(), mlua::Error> {
        let source = fs::read_to_string(path).map_err(|error| {
            mlua::Error::RuntimeError(format!("cannot read plugin '{path}': {error}"))
        })?;
        self.last_command = None;
        self.lua.load(source).set_name(path).exec()
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.ctx.commands.borrow().contains_key(name)
    }

    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ctx.commands.borrow().keys().cloned().collect();
        names.sort();
        names
    }

    pub fn invoke(
        &mut self,
        name: &str,
        doc: &mut Document,
        cursor: &mut usize,
    ) -> Result<i64, mlua::Error> {
        let reference = match self.last_command.as_ref() {
            Some((cached_name, cached_reference)) if cached_name == name => *cached_reference,
            _ => {
                let reference = self
                    .ctx
                    .commands
                    .borrow()
                    .get(name)
                    .copied()
                    .ok_or_else(|| {
                        mlua::Error::RuntimeError(format!("unknown Lua command '{name}'"))
                    })?;
                self.last_command = Some((name.to_string(), reference));
                reference
            }
        };

        let revision = doc.revision();
        let editor_cursor = *cursor;
        if self.loaded_revision != Some(revision) {
            let line = doc.line_of_offset(editor_cursor);
            let column = editor_cursor - doc.line_start(line);
            let text = doc.all_text();
            unsafe { host_mut(&self.ctx) }.load(text, line, column);
            self.loaded_revision = Some(revision);
            self.synced_editor_cursor = Some(editor_cursor);
        } else if self.synced_editor_cursor != Some(editor_cursor) {
            let line = doc.line_of_offset(editor_cursor);
            let column = editor_cursor - doc.line_start(line);
            unsafe { host_mut(&self.ctx) }.set_cursor(line, column);
            self.synced_editor_cursor = Some(editor_cursor);
        }

        let state = self.state;
        let checksum = unsafe {
            ffi::lua_rawgeti(state, ffi::LUA_REGISTRYINDEX, reference as ffi::lua_Integer);
            let status = ffi::lua_pcallk(state, 0, 1, 0, 0, None);
            if status != ffi::LUA_OK {
                self.loaded_revision = None;
                let message = capture_error(state);
                ffi::lua_settop(state, 0);
                return Err(mlua::Error::RuntimeError(message));
            }
            let mut is_number: c_int = 0;
            let value = ffi::lua_tointegerx(state, -1, &mut is_number);
            ffi::lua_settop(state, 0);
            value
        };

        let (dirty, offset) = unsafe {
            let host = host(&self.ctx);
            (host.dirty, host.byte_offset())
        };
        if dirty {
            let host = unsafe { host_mut(&self.ctx) };
            let data = std::mem::take(&mut host.text);
            doc.replace_all_owned(data);
            // The document now owns the moved buffer. Point the host at it so
            // the next command can keep reading without another copy.
            host.text_view = doc.borrowed_text();
        }
        *cursor = offset.min(doc.size());
        self.synced_editor_cursor = Some(*cursor);
        self.message = unsafe {
            let host = host_mut(&self.ctx);
            host.dirty = false;
            host.take_message()
        };
        self.loaded_revision = Some(doc.revision());
        Ok(checksum)
    }

    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }
}

fn line_of_offset_in(line_starts: &[usize], offset: usize) -> usize {
    line_starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1)
        .min(line_starts.len().saturating_sub(1))
}

unsafe fn register_api(state: *mut ffi::lua_State, ctx: *const RawContext) {
    unsafe {
        ffi::lua_createtable(state, 0, 0);
        add_function(state, ctx, c"line_count", api_line_count);
        add_function(state, ctx, c"get_line", api_get_line);
        add_function(state, ctx, c"set_line", api_set_line);
        add_function(state, ctx, c"get_text", api_get_text);
        add_function(state, ctx, c"set_text", api_set_text);
        add_function(state, ctx, c"cursor", api_cursor);
        add_function(state, ctx, c"set_cursor", api_set_cursor);
        add_function(state, ctx, c"insert", api_insert);
        add_function(state, ctx, c"message", api_message);
        add_function(state, ctx, c"register_command", api_register_command);
        add_function(state, ctx, c"command", api_register_command);
        ffi::lua_setglobal(state, c"hexal".as_ptr());
    }
}

unsafe fn add_function(
    state: *mut ffi::lua_State,
    ctx: *const RawContext,
    name: &CStr,
    function: ffi::lua_CFunction,
) {
    unsafe {
        ffi::lua_pushlightuserdata(state, ctx as *mut c_void);
        ffi::lua_pushcclosure(state, function, 1);
        ffi::lua_setfield(state, -2, name.as_ptr());
    }
}

unsafe fn host<'a>(ctx: &'a RawContext) -> &'a LuaHost {
    unsafe { &*ctx.host.get() }
}

unsafe fn host_mut<'a>(ctx: &'a RawContext) -> &'a mut LuaHost {
    unsafe { &mut *ctx.host.get() }
}

unsafe fn context<'a>(state: *mut ffi::lua_State) -> &'a RawContext {
    unsafe {
        let pointer = ffi::lua_touserdata(state, ffi::lua_upvalueindex(1));
        &*(pointer as *const RawContext)
    }
}

unsafe fn fail(state: *mut ffi::lua_State, message: &CStr) -> ! {
    unsafe {
        ffi::lua_pushlstring(state, message.as_ptr(), message.to_bytes().len());
        ffi::lua_error(state)
    }
}

unsafe fn arg_bytes<'a>(state: *mut ffi::lua_State, index: c_int) -> Option<&'a [u8]> {
    unsafe {
        let mut length = 0usize;
        let pointer = ffi::lua_tolstring(state, index, &mut length);
        if pointer.is_null() {
            None
        } else {
            Some(slice::from_raw_parts(pointer as *const u8, length))
        }
    }
}

unsafe fn arg_integer(state: *mut ffi::lua_State, index: c_int) -> Option<ffi::lua_Integer> {
    unsafe {
        let mut is_number: c_int = 0;
        let value = ffi::lua_tointegerx(state, index, &mut is_number);
        if is_number == 0 {
            None
        } else {
            Some(value)
        }
    }
}

unsafe fn push_bytes(state: *mut ffi::lua_State, bytes: &[u8]) -> c_int {
    unsafe {
        ffi::lua_pushlstring(state, bytes.as_ptr() as *const c_char, bytes.len());
    }
    1
}

unsafe extern "C-unwind" fn api_line_count(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        ffi::lua_pushinteger(state, host(ctx).line_count() as ffi::lua_Integer);
    }
    1
}

unsafe extern "C-unwind" fn api_get_line(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(line) = arg_integer(state, 1) else {
            fail(state, c"get_line: expected a line number");
        };
        if line < 0 {
            fail(state, c"get_line: line index must be >= 0");
        }
        let host = host(ctx);
        let Some(bytes) = host.line_bytes(line as usize) else {
            fail(state, c"get_line: line out of range");
        };
        push_bytes(state, bytes)
    }
}

unsafe extern "C-unwind" fn api_set_line(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(line) = arg_integer(state, 1) else {
            fail(state, c"set_line: expected a line number");
        };
        if line < 0 {
            fail(state, c"set_line: line index must be >= 0");
        }
        let Some(value) = arg_bytes(state, 2) else {
            fail(state, c"set_line: expected a string");
        };
        let changed = { host_mut(ctx).set_line(line as usize, value) };
        if !changed {
            fail(state, c"set_line: line out of range");
        }
    }
    0
}

unsafe extern "C-unwind" fn api_get_text(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let host = host(ctx);
        push_bytes(state, host.bytes())
    }
}

unsafe extern "C-unwind" fn api_set_text(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(value) = arg_bytes(state, 1) else {
            fail(state, c"set_text: expected a string");
        };
        {
            host_mut(ctx).set_text(value);
        }
    }
    0
}

unsafe extern "C-unwind" fn api_cursor(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let host = host(ctx);
        ffi::lua_pushinteger(state, host.cursor_line as ffi::lua_Integer);
        ffi::lua_pushinteger(state, host.cursor_column as ffi::lua_Integer);
    }
    2
}

unsafe extern "C-unwind" fn api_set_cursor(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let (Some(line), Some(column)) = (arg_integer(state, 1), arg_integer(state, 2)) else {
            fail(state, c"set_cursor: expected line and column numbers");
        };
        if line < 0 || column < 0 {
            fail(state, c"set_cursor: line and column must be >= 0");
        }
        {
            host_mut(ctx).set_cursor(line as usize, column as usize);
        }
    }
    0
}

unsafe extern "C-unwind" fn api_insert(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(value) = arg_bytes(state, 1) else {
            fail(state, c"insert: expected a string");
        };
        {
            host_mut(ctx).insert(value);
        }
    }
    0
}

unsafe extern "C-unwind" fn api_message(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(value) = arg_bytes(state, 1) else {
            fail(state, c"message: expected a string");
        };
        {
            host_mut(ctx).message = Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    0
}

unsafe extern "C-unwind" fn api_register_command(state: *mut ffi::lua_State) -> c_int {
    unsafe {
        let ctx = context(state);
        let Some(name_bytes) = arg_bytes(state, 1) else {
            fail(state, c"register_command: expected a name");
        };
        if ffi::lua_type(state, 2) != ffi::LUA_TFUNCTION {
            fail(state, c"register_command: expected a function");
        }
        let Ok(name) = std::str::from_utf8(name_bytes) else {
            fail(state, c"register_command: name must be valid UTF-8");
        };

        ffi::lua_pushvalue(state, 2);
        let reference = ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
        if let Some(old) = ctx
            .commands
            .borrow_mut()
            .insert(name.to_string(), reference)
        {
            ffi::luaL_unref(state, ffi::LUA_REGISTRYINDEX, old);
        }
        ffi::lua_pushboolean(state, 1);
    }
    1
}

impl Drop for LuaRuntime {
    fn drop(&mut self) {
        unsafe {
            ffi::lua_close(self.state);
        }
    }
}

unsafe extern "C" fn lua_alloc(
    userdata: *mut c_void,
    pointer: *mut c_void,
    old_size: usize,
    new_size: usize,
) -> *mut c_void {
    unsafe {
        let allocator = &mut *(userdata as *mut LuaAllocator);
        if new_size == 0 {
            if !pointer.is_null() {
                allocator.dealloc(pointer as *mut u8, old_size);
            }
            return ptr::null_mut();
        }
        if pointer.is_null() {
            allocator.alloc(new_size) as *mut c_void
        } else {
            allocator.realloc(pointer as *mut u8, old_size, new_size) as *mut c_void
        }
    }
}

fn capture_error(state: *mut ffi::lua_State) -> String {
    unsafe {
        let mut length = 0usize;
        let pointer = ffi::lua_tolstring(state, -1, &mut length);
        if pointer.is_null() {
            return "Lua error".to_string();
        }
        let bytes = slice::from_raw_parts(pointer as *const u8, length);
        String::from_utf8_lossy(bytes).into_owned()
    }
}
