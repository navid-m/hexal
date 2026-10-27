//! Hexal Text Editor
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;

/// Large buffers (plugin snapshots, bulk replacements, big files) go through
/// mimalloc. Small startup allocations stay on the system allocator so a
/// plugin-free launch does not pay mimalloc's arena and RSS cost.
struct HybridAllocator;

const LARGE_ALLOCATION: usize = 1024 * 1024;
static SYSTEM_ALLOCATOR: System = System;
static MIMI_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

unsafe impl GlobalAlloc for HybridAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            if layout.size() >= LARGE_ALLOCATION {
                MIMI_ALLOCATOR.alloc(layout)
            } else {
                SYSTEM_ALLOCATOR.alloc(layout)
            }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            if layout.size() >= LARGE_ALLOCATION {
                MIMI_ALLOCATOR.dealloc(pointer, layout);
            } else {
                SYSTEM_ALLOCATOR.dealloc(pointer, layout);
            }
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let old_large = layout.size() >= LARGE_ALLOCATION;
            let new_large = new_size >= LARGE_ALLOCATION;
            match (old_large, new_large) {
                (false, false) => SYSTEM_ALLOCATOR.realloc(pointer, layout, new_size),
                (true, true) => MIMI_ALLOCATOR.realloc(pointer, layout, new_size),
                (false, true) => {
                    let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                    let new_pointer = MIMI_ALLOCATOR.alloc(new_layout);
                    if !new_pointer.is_null() {
                        ptr::copy_nonoverlapping(pointer, new_pointer, layout.size());
                        SYSTEM_ALLOCATOR.dealloc(pointer, layout);
                    }
                    new_pointer
                }
                (true, false) => {
                    let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                    let new_pointer = SYSTEM_ALLOCATOR.alloc(new_layout);
                    if !new_pointer.is_null() {
                        ptr::copy_nonoverlapping(pointer, new_pointer, new_size);
                        MIMI_ALLOCATOR.dealloc(pointer, layout);
                    }
                    new_pointer
                }
            }
        }
    }
}

#[global_allocator]
static GLOBAL: HybridAllocator = HybridAllocator;

mod document;
mod editor;
mod files;
mod lua;
mod renderer;
mod screen;
mod syntax;
mod terminal;
mod theme;
mod util;

use document::Document;
use editor::Editor;
use lua::LuaRuntime;
use terminal::Terminal;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP_TEXT: &str = "\
hexalrs [options] [file]

A small modal, terminal-based text editor written in Rust 2024.

Options:
  -h, --help       Print this help and exit
  -v, --version    Print the version and exit
  --               Treat remaining arguments as file names

Keys:
  i / I            Insert mode
  a / A            Append after cursor / at line end
  o / O            Open line below / above
  v / V            Character selection
  x / X            Line selection
  Esc              Normal mode / cancel selection
  h j k l, arrows  Move
  w / b            Next / previous word
  Home / End       Line start / end
  Ctrl+Home/End    Document start / end
  PageUp/PageDown  Move by one screen
  gg / ge / gp     First line / last line / previous location
  d / Delete       Delete character or selection
  u / U            Undo / redo
  /                Find in file (n / p to step)
  :                Ex command line (:w, :wq, :q, :e, :path, :cw)
  :lua <file>      Load a Lua plugin from a file
  :plugins         List registered Lua commands
  Space            Open the SPACE menu (bottom-right)
  Space f          File picker
  Space /          Search across files
  Space g          Go to line
  Space y / X      Yank / cut selection
  Space p / P      Paste / paste on a new line

Lua plugins:
  Plugins in ./plugins/*.lua are loaded at startup.
  Headless benchmark:
    hexalrs --lua-bench <plugin.lua> <command> <iterations> [file]
";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--lua-bench") {
        if let Err(error) = run_lua_bench(&args[2..]) {
            eprintln!("hexalrs: lua bench: {error}");
            std::process::exit(1);
        }
        return;
    }

    let mut file: Option<String> = None;
    let mut options_ended = false;

    for arg in args.iter().skip(1) {
        if !options_ended && arg == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended && (arg == "-h" || arg == "--help") {
            print!("{HELP_TEXT}");
            return;
        }
        if !options_ended && (arg == "-v" || arg == "--version") {
            println!("hexalrs {VERSION} (Rust 2024)");
            return;
        }
        if !options_ended && arg.len() > 1 && arg.starts_with('-') {
            eprintln!("hexalrs: unknown option '{arg}'\n\n{HELP_TEXT}");
            std::process::exit(2);
        }
        if file.is_some() {
            eprintln!("hexalrs: this prototype opens one file at a time");
            std::process::exit(2);
        }
        file = Some(arg.clone());
    }

    let mut terminal = match Terminal::init() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("hexalrs: cannot initialize terminal: {error}");
            std::process::exit(1);
        }
    };

    let document = match file.as_deref() {
        Some(path) => match Document::open(path) {
            Ok(document) => document,
            Err(error) => {
                drop(terminal);
                eprintln!("hexalrs: cannot open buffer: {error}");
                std::process::exit(4);
            }
        },
        None => Document::new(),
    };

    let mut editor = Editor::new(document, terminal.width, terminal.height);
    let result = editor.run(&mut terminal);
    drop(terminal);
    if let Err(error) = result {
        eprintln!("hexalrs: editor error: {error}");
        std::process::exit(1);
    }
}

fn run_lua_bench(args: &[String]) -> Result<(), String> {
    if args.len() < 3 || args.len() > 4 {
        return Err("usage: hexalrs --lua-bench <plugin.lua> <command> <iterations> [file]".into());
    }
    let plugin = &args[0];
    let command = &args[1];
    let iterations: usize = args[2]
        .parse()
        .map_err(|_| format!("invalid iteration count '{}'", args[2]))?;
    if iterations == 0 {
        return Err("iteration count must be greater than zero".into());
    }

    let mut document = match args.get(3) {
        Some(path) => {
            Document::open(path).map_err(|error| format!("cannot open '{path}': {error}"))?
        }
        None => Document::new(),
    };
    let mut cursor = 0usize;

    let start = std::time::Instant::now();
    let mut runtime = LuaRuntime::new().map_err(|error| format!("cannot start Lua: {error}"))?;
    runtime
        .load_file(plugin)
        .map_err(|error| format!("cannot load '{plugin}': {error}"))?;
    let init_ns = start.elapsed().as_nanos();

    // Warm up the document snapshot; timed invocations are steady-state.
    runtime
        .invoke(command, &mut document, &mut cursor)
        .map_err(|error| format!("warm-up failed: {error}"))?;

    let mut total_ns = 0u128;
    let mut checksum = 0i64;
    for _ in 0..iterations {
        let start = std::time::Instant::now();
        checksum += runtime
            .invoke(command, &mut document, &mut cursor)
            .map_err(|error| format!("command failed: {error}"))?;
        total_ns += start.elapsed().as_nanos();
    }

    println!(
        "lua_bench\t{}\t{}\t{}\t{}",
        init_ns, total_ns, iterations, checksum
    );
    Ok(())
}
