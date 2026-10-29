//! Hexal Text Editor
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

mod core;
mod dap;
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
use std::io;
use std::path::{Path, PathBuf};
use ui::editor::Editor;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT_HASH: &str = env!("HEXAL_COMMIT_HASH");

const HELP_TEXT: &str = concat!(
    "Hexal v",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("HEXAL_COMMIT_HASH"),
    ")\n",
    "https://hexal.pages.dev\n",
    "(C) Navid Momtahen - 2026\n",
    "\n",
    "Usage: \n",
    "  hexal [options] file\n",
    "  hexal [options] directory\n",
    "  hexal install-plugin <name>\n",
    "\n",
    "Options:\n",
    "  -h, --help       Print this help and exit\n",
    "  -v, --version    Print the version and exit\n",
    "  --               Treat remaining arguments as file names\n",
);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--lua-bench") {
        if let Err(error) = run_lua_bench(&args[2..]) {
            eprintln!("hexal: lua bench: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args
        .get(1)
        .is_some_and(|argument| argument.eq_ignore_ascii_case("install-plugin"))
    {
        std::process::exit(run_install_plugin(&args[1..]));
    }

    let mut file: Option<String> = None;
    let mut directory: Option<String> = None;
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
            println!("hexal {VERSION} ({COMMIT_HASH})");
            return;
        }
        if !options_ended && arg.len() > 1 && arg.starts_with('-') {
            eprintln!("hexal: unknown option '{arg}'\n\n{HELP_TEXT}");
            std::process::exit(2);
        }
        if file.is_some() || directory.is_some() {
            eprintln!("hexal: this prototype opens one file or one directory at a time");
            std::process::exit(2);
        }
        match std::fs::metadata(arg) {
            Ok(metadata) if metadata.is_dir() => {
                if let Err(error) = std::fs::read_dir(arg) {
                    eprintln!("hexal: cannot open directory '{arg}': {error}");
                    std::process::exit(4);
                }
                directory = Some(absolute_path(arg));
            }
            Ok(_) => file = Some(arg.clone()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let parent = Path::new(arg)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                if parent.is_dir() {
                    // Opening a nonexistent name creates a new buffer, as long
                    // as its parent directory is valid.
                    file = Some(arg.clone());
                } else {
                    eprintln!("hexal: no such file or directory: '{arg}'");
                    std::process::exit(4);
                }
            }
            Err(error) => {
                eprintln!("hexal: cannot access '{arg}': {error}");
                std::process::exit(4);
            }
        }
    }

    let mut terminal = match Terminal::init() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("hexal: cannot initialize terminal: {error}");
            std::process::exit(1);
        }
    };

    std::thread::spawn(platform::desktop::ensure_launcher);

    let document = match file.as_deref() {
        Some(path) => match Document::open(path) {
            Ok(document) => document,
            Err(error) => {
                drop(terminal);
                eprintln!("hexal: cannot open buffer '{path}': {error}");
                std::process::exit(4);
            }
        },
        None => Document::new(),
    };

    let open_file_picker = directory.is_some();
    let workspace_root = directory.as_deref().map(PathBuf::from);
    let mut editor = Editor::with_options(
        document,
        terminal.width,
        terminal.height,
        workspace_root,
        open_file_picker,
    );
    let result = editor.run(&mut terminal);
    drop(terminal);
    if let Err(error) = result {
        eprintln!("hexal: editor error: {error}");
        std::process::exit(1);
    }
}

fn absolute_path(path: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| PathBuf::from(path))
            .to_string_lossy()
            .into_owned()
    }
}

fn run_install_plugin(arguments: &[String]) -> i32 {
    if arguments.len() != 2 {
        eprintln!("usage: hexal install-plugin <name>");
        return 2;
    }

    let (config, config_error) = core::config::Config::load_default();
    if !config_error.is_empty() {
        eprintln!("hexal: config: {config_error}");
    }

    let repository = if config.central_repository_url.trim().is_empty() {
        core::config::DEFAULT_CENTRAL_REPOSITORY_URL
    } else {
        config.central_repository_url.trim()
    };
    let directory = util::plugin_install::plugin_directory();
    match util::plugin_install::install(&arguments[1], repository, &directory) {
        Ok(path) => {
            println!("Installed plugin '{}' to {}", arguments[1], path.display());
            0
        }
        Err(error) => {
            eprintln!("hexal: install-plugin: {error}");
            1
        }
    }
}

fn run_lua_bench(args: &[String]) -> Result<(), String> {
    if args.len() < 3 || args.len() > 4 {
        return Err("usage: hexal --lua-bench <plugin.lua> <command> <iterations> [file]".into());
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

    let _ = &cursor;
    let host = std::rc::Rc::new(std::cell::RefCell::new(scripting::lua::HostState::default()));
    let start = std::time::Instant::now();
    let mut runtime = LuaRuntime::new("lua-bench", ".", host)
        .map_err(|error| format!("cannot start Lua: {error}"))?;
    runtime
        .load_file(plugin)
        .map_err(|error| format!("cannot load '{plugin}': {error}"))?;
    let init_ns = start.elapsed().as_nanos();

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
