//! Hexal Text Editor
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

mod core;
mod languages;
mod lsp;
mod platform;
mod scripting;
mod ui;
mod util;

#[global_allocator]
static GLOBAL: platform::alloc::LargeCacheAllocator = platform::alloc::LargeCacheAllocator::new();

use core::document::Document;
use platform::terminal::Terminal;
use scripting::lua::LuaRuntime;
use ui::editor::Editor;

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
  Ctrl+C           Toggle comments on the selection or current line
  :rc              Remove comments from the active selection
  :fmt / :fmta     Format the buffer/selection or every file with an extension
  :lang <id>       Override the language for the current buffer
  :cfg             Open (or create) ~/.hexal/.hexcfg
  :rcfg            Reload configuration, grammars, themes and language servers
  :lua <file>      Load a Lua plugin from a file
  :plugins         List registered Lua commands
  Space            Open the SPACE menu (bottom-right)
  Space f          File picker
  Space /          Search across files
  Space g          Go to line
  Space K / A / S  Language-server hover / code actions / symbols
  Space d / D / Z  Document / workspace diagnostics / line diagnostic
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
