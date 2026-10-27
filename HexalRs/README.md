# HexalRs

Prototype of Hexal in Rust (2024).

## Features

- NORMAL / INSERT / SELECT / COMMAND modes
- Movement: arrows, `h j k l`, `w`/`b`, Home/End, Ctrl+Home/End, PageUp/PageDown,
  `gg`, `ge`, `gp`
- Editing: `i`, `a`, `I`, `A`, `o`, `O`, delete, line/character selection,
  indent/unindent, auto-indent on Enter, grouped undo/redo
- In-file literal find (`/`, `n`, `p`) with live match highlighting
- Ex commands: `:w`, `:w <file>`, `:wq`, `:q`, `:q!`, `:e`, `:e!`, `:path`,
  `:cw`, `:lang`, `:lua`, `:plugins`, `:help`, `:about`
- SPACE menu: cross-file search, file picker, go-to-line, yank/cut/paste,
  save, key reference, quit
- File picker and cross-file search with filtering
- Internal clipboard plus best-effort OSC 52 system clipboard
- Syntax highlighting for Rust, Zig, C/C++, C#, Python, JS/TS, Go, Lua, Shell,
  JSON, TOML, YAML, Markdown and config files
- UTF-8 aware cursor movement, tabs and wide-character display
- LF/CRLF and BOM preservation, atomic saves, dirty confirmation
- Embedded Lua 5.4.9 plugins loaded from `plugins/*.lua`, with Ex command
  registration and a small buffer API

Not implemented in this prototype: LSP, DAP, themes beyond the built-in palette,
panes, regex search, multi-cursor, formatters.

## Build

The release build statically links glibc and the vendored Lua 5.4.9 runtime. The
target and link flags live in `.cargo/config.toml`. On Linux:

```sh
cd HexalRs
cargo build --release
./target/x86_64-unknown-linux-gnu/release/hexalrs path/to/file
```

The binary is non-PIE, statically linked and stripped:

```text
$ file target/x86_64-unknown-linux-gnu/release/hexalrs
ELF 64-bit LSB executable, x86-64, statically linked, stripped
$ stat -c %s target/x86_64-unknown-linux-gnu/release/hexalrs
2153920
```

The size includes the editor, glibc and Lua 5.4.9.

## Lua plugins

Drop `.lua` files into `plugins/` next to the working directory. They are loaded
at startup, sorted by filename. A plugin can also be loaded manually with
`:lua path/to/plugin.lua`; `:plugins` lists the registered commands.

```lua
hexal.register_command("wordcount", function()
    local words = 0
    for _ in hexal.get_text():gmatch("%S+") do
        words = words + 1
    end
    hexal.message(words .. " word(s)")
end)
```

Available API:

| Function | Purpose |
| --- | --- |
| `hexal.register_command(name, fn)` | Register an Ex command; `hexal.command` is an alias |
| `hexal.message(text)` | Set the status-bar message |
| `hexal.line_count()` | Number of lines (includes a final empty line after a trailing newline) |
| `hexal.get_line(index)` | 0-based line text, without the line ending |
| `hexal.set_line(index, text)` | Replace one line |
| `hexal.get_text()` / `hexal.set_text(text)` | Read/replace the whole buffer |
| `hexal.cursor()` | Current `line, column` (0-based, byte column) |
| `hexal.set_cursor(line, column)` | Move the cursor |
| `hexal.insert(text)` | Insert at the cursor |

Plugin command edits are applied as a single document replacement, so one
command is one undo step. See `plugins/wordcount.lua` for working examples
(`:wordcount`, `:upper`, `:sortlines`).

## Test

```sh
cargo test --release
```

The tests cover grouped undo/redo, line indexing and randomized piece-table
edits against a `Vec<u8>` reference.

## Benchmarks

### Startup

`benchmarks/run_benchmarks.sh` measures raw startup with `hyperfine -N`, tmux
time-to-first-frame, RSS and shipped size. Full results are in
`benchmarks/results/summary.md`. Current machine:

| Metric | HexalRs (Rust static) | HexalZ (Zig static) | HexalCs (NativeAOT) |
| --- | ---: | ---: | ---: |
| Raw startup, mean | 0.533 ms | **0.375 ms** | 4.085 ms |
| First frame, empty, median | 1.085 ms | **0.944 ms** | 5.155 ms |
| First frame, 100 lines, median | 1.160 ms | **1.038 ms** | 5.416 ms |
| RSS at first frame, empty | 1.8 MB | **1.5 MB** | 16.2 MB |
| Shipped size | 2.05 MiB | **1.85 MiB** | 8.23 MiB |

Both native binaries embed Lua 5.4.9 and remain statically linked.

### Lua plugins

`benchmarks/lua/run_lua_bench.sh` runs the same plugin file in both editors and
writes `SUMMARY.md` at the repository root. It measures plugin load time and
command throughput for:

- empty command dispatch (`noop`)
- many small cursor API calls (`cursor_walk`)
- read-heavy word counting over 5k and 50k lines
- bulk whole-buffer read/write (`bulk_upper_50k`)

Current results:

| Case | HexalRs (Rust) | HexalZ (Zig) | Rust / Zig |
| --- | ---: | ---: | ---: |
| noop | **0.05 µs** | 0.05 µs | 0.92× |
| cursor_walk | **257.67 µs** | 259.25 µs | 0.99× |
| wordcount_5k | **4672.56 µs** | 4686.63 µs | 1.00× |
| wordcount_50k | **47478.33 µs** | 50298.56 µs | 0.94× |
| bulk_upper_50k | **6298.52 µs** | 8319.64 µs | 0.76× |

HexalRs now wins four of the five cases outright and ties the fifth. The
largest win is the bulk whole-buffer path. Key Rust-side changes:

- Hot `hexal.*` callbacks use the Lua C API directly instead of mlua's
  high-level callback layer; mlua still owns state creation, loading and errors.
- The single-threaded host uses `UnsafeCell` instead of per-call `RefCell`
  checks, caches the last command reference, and skips redundant cursor syncs.
- `PieceTable::replace` rebuilds the line index once per whole-buffer edit.
- A hybrid allocator sends large buffers and Lua strings (≥1 MiB) to mimalloc
  while keeping small startup allocations on the system allocator, so
  plugin-free startup and RSS stay close to the earlier build.

Plugin load remains essentially tied at roughly 125 µs.
