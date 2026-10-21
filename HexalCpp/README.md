# Hexal

A fast, modal, terminal-based text editor written in modern C++.

This repository contains the **Phase 1 MVP** from `SPECIFICATION.md`: a
cross-platform terminal editor with an efficient piece-table buffer, viewport
rendering, file open/save, NORMAL and INSERT modes, undo/redo, a status bar and
an Ex command line.

The design goal is to keep the editing core independent from the terminal so
that later phases (selection modes, configuration, syntax highlighting, LSP,
file-tree navigation and split windows) can be added without rewriting the
buffer.

<p align="center"><em>No external libraries. C++20 standard library only.</em></p>

---

## What the MVP implements

- Cross-platform terminal initialization and cleanup
  - Linux / macOS: `termios`, ANSI sequences, `SIGWINCH`
  - Windows: Console API + `ENABLE_VIRTUAL_TERMINAL_PROCESSING`,
    `ReadConsoleInputW` for Unicode keyboard input
- A piece-table text buffer with an augmented treap index
- Viewport-only rendering with a cell-grid diff renderer
- File opening and robust, atomic file saving
- Background saving from an immutable piece snapshot: the UI keeps accepting
  input and scrolling while a large file is written
- LF / CRLF preservation, UTF-8 (with optional BOM preservation)
- NORMAL and INSERT modes
- Cursor movement: arrows, `h/j/k/l`, Home/End, PageUp/PageDown, `gg`, `ge`,
  and `gp` (previous location)
- Undo and redo with grouped typing/deletion
- Status bar showing mode, file name, modified state, line/column, line count
  and line-ending style
- Ex commands: `:w`, `:wq`, `:q`, `:q!`, `:e`, `:e!`, `:help`
- Unsaved-change protection when quitting or switching files
- Unit tests for the buffer, line index, undo, commands, Unicode and file I/O

## Not implemented (later specification phases)

The MVP intentionally leaves these out; they are Phase 2 and later:

- SELECT MODE and LINE SELECT MODE (`V`, `X`)
- Line commenting (`Shift+C`)
- `.hexcfg` / `.hexdef` parsing, themes, `:cfg`, `:rcfg`, `:lang`
- Syntax highlighting
- LSP integration and `:fmt`
- Wildcard file search (`:sf`) and the file tree (`:g`)
- Split windows (`:sv`, `:sh`)
- Setup wizard, theme installation, configuration merging
- Multiple buffers / tabs (the editor opens one file at a time)
- Mouse support, selection, clipboard integration

The unimplemented Ex commands are recognized and produce a clear
"not implemented in this MVP" message rather than being silently ignored.

---

## Requirements

- CMake 3.20 or newer
- A C++20 compiler
  - GCC 11+, Clang 13+, or MSVC 19.30+ (Visual Studio 2022 recommended)
- An ANSI-capable terminal
  - Windows 10/11 with virtual terminal processing enabled (default in
    Windows Terminal)

There are no third-party dependencies.

---

## Build

### Linux / macOS

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
ctest --test-dir build --output-on-failure
```

Run:

```sh
./build/hexal path/to/file.txt
```

### Windows (MSVC)

From a Developer Command Prompt:

```bat
cmake -S . -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release
ctest --test-dir build -C Release --output-on-failure
```

Run:

```bat
build\Release\hexal.exe path\to\file.txt
```

MSVC builds enable `/utf-8`, `/W4` and `/permissive-`. The editor's entry point
is `wmain`, so command-line file names are converted from UTF-16 to UTF-8 and
Unicode paths work correctly.

### Install (optional)

```sh
cmake --install build --prefix /usr/local
```

---

## Usage

```text
hexal [options] [file]
```

| Option | Meaning |
| --- | --- |
| `-h`, `--help` | Print help and exit without starting the UI |
| `-v`, `--version` | Print version and exit |
| `--` | Treat remaining arguments as file names |

With no file, Hexal starts with an unnamed, empty buffer. Use `:w <file>` to
give it a name and save it. Relative file names in `:e` and `:w` are resolved
against the current working directory.

The MVP opens one file at a time. `:e <file>` switches buffers after protecting
unsaved changes.

Exit codes:

| Code | Meaning |
| --- | --- |
| `0` | Success (including `--help` and `--version`) |
| `1` | Terminal initialization or fatal runtime failure |
| `2` | Invalid command-line usage |
| `4` | File or filesystem operation failure at startup |

### Keyboard

| Key | Action |
| --- | --- |
| `i`, `I` | Enter INSERT mode at the cursor |
| `a` | Append: move right, then enter INSERT mode |
| `A` | Move to end of line, then enter INSERT mode |
| `o` / `O` | Open a new line below / above and enter INSERT mode |
| `Esc` | Return to NORMAL mode (or cancel a prompt/command) |
| `h` `j` `k` `l` | Left / down / up / right |
| Arrow keys | Move the cursor |
| `Home` / `End` | Start / end of line |
| `Ctrl+Home` / `Ctrl+End` | Start / end of document |
| `PageUp` / `PageDown` | Move by one screen |
| `gg` / `ge` | First line / last line |
| `gp` | Jump to the previous recorded location |
| `x` | Delete the character under the cursor |
| `u` | Undo |
| `Ctrl+R` | Redo |
| `:` | Open the Ex command line |
| `Enter` (INSERT) | Insert a line ending matching the file (LF or CRLF) |
| `Backspace` / `Delete` (INSERT) | Delete before / after the cursor |
| `Tab` (INSERT) | Insert a tab character |

Vertical movement keeps the preferred display column, including through tabs
and wide characters. The cursor is always scrolled back into view.

### Commands

| Command | Action |
| --- | --- |
| `:w` | Save the current buffer |
| `:w <file>` | Save as `<file>` and change the buffer's name |
| `:wq` | Save, then quit |
| `:q` | Quit; asks for confirmation if the buffer is modified |
| `:q!` | Quit without saving |
| `:e <file>` | Open a file (creates it if the parent directory exists) |
| `:e! <file>` | Open a file, discarding unsaved changes |
| `:help` | Show a short key reminder |

File names containing spaces use quotes:

```text
:e "src/my file.cpp"
:e 'another file.txt'
```

Commands and arguments are parsed without invoking a shell. Backslashes
outside quotes are preserved, so Windows paths work naturally.

When quitting or opening another file with unsaved changes, Hexal displays a
`y/N` confirmation. Press `y` to proceed or `n` / `Esc` to cancel.

---

## Data safety

- Saving captures an immutable piece snapshot on the UI thread, then streams
  that snapshot to a temporary file in the target directory on a background
  worker, flushes it, and renames it over the original. A failed write or
  rename leaves the original file untouched. The status bar shows `[saving]`
  while the worker runs, and the editor remains scrollable and editable.
- If the buffer is edited while a snapshot is being written, the document stays
  marked dirty; `:wq` saves the newer state before quitting rather than
  discarding edits.
- Existing POSIX permission bits are copied to the replacement file before the
  rename.
- LF and CRLF line endings are preserved. New lines use the file's detected
  convention. A final line without a trailing newline is preserved.
- Files are read as binary. A UTF-8 BOM is preserved on save.
- Invalid UTF-8 is preserved on disk; the UI displays U+FFFD for malformed
  bytes instead of crashing.
- The editor never silently discards modified buffers.

External file modification detection and file backups are not part of the MVP.

---

## Architecture

```text
src/
  main.cpp                command-line parsing, startup and shutdown
  core/
    piece_table.*         text storage + augmented line index
    document.*            file metadata, undo/redo, line operations, saving
    command.*             Ex command tokenizer
  platform/
    terminal.hpp          platform-independent terminal interface
    terminal_posix.cpp    termios, SIGWINCH, ANSI input decoder
    terminal_win32.cpp    Console API, ReadConsoleInputW, VT output
  ui/
    screen.*              cell grid and UTF-8/wide-character cells
    renderer.*            ANSI diff renderer
    editor.*              modes, key handling, commands, viewport, status bar
  util/
    unicode.*             UTF-8, code point boundaries, display widths
    filesystem.*          UTF-8 paths, binary read, atomic write
```

The `hexal_core` CMake target has no terminal dependency and is what the unit
tests link against. The UI depends on `Terminal`, not on `termios` or
`windows.h`, so the platform layer is the only place with OS-specific code.

### Buffer strategy

Hexal uses a **piece table** backed by an **implicit treap**. The original file
is never copied or mutated. User edits are appended to a separate add buffer;
the document is a sequence of pieces referring to ranges of either buffer.

Each treap node is augmented with:

- the number of bytes in its subtree,
- the number of newline bytes in its subtree.

This provides:

- insertion/deletion in expected `O(log p)` time (`p` = number of pieces),
- `offset -> line` and `line -> offset` in expected `O(log p)`,
- piece splits that allocate only a small node,
- no full-document copy on ordinary edits.

Initial files are split into pieces of roughly 8 KiB at newline boundaries to
bound newline scans. Inserted text is chunked the same way. See
[`docs/DESIGN.md`](docs/DESIGN.md) for details and trade-offs.

### Coordinate model

Three different values are deliberately not conflated:

1. **Byte offsets** identify positions in the UTF-8 text. They are what the
   piece table and undo records store.
2. **Code point boundaries** are used for cursor movement and deletion. The UI
   never moves the cursor into the middle of a UTF-8 sequence.
3. **Display columns** are terminal cells: tabs expand to the next multiple of
   four, East Asian wide characters occupy two cells, combining marks are
   treated as zero width, and control characters are drawn as U+FFFD.

The cursor is stored as a byte offset. Conversions to display columns happen
only at the UI boundary.

### Undo / redo

- Consecutive typed characters are merged into one undo group.
- Consecutive backspaces / forward deletes are merged into one group.
- Newline insertion or deletion ends a group.
- Cursor movement, mode changes, save, undo and redo close the current group.
- A range replacement is one group (used by future formatter/comment support).
- Each group stores only the inserted and removed text, never a document
  snapshot.
- History is bounded by 4096 groups or 64 MiB of stored edit text, whichever
  comes first. The oldest applied groups are discarded first.
- The modified flag is derived from the position of the last save in the
  history, so undoing back to the saved state marks the buffer clean again.
- Background saves also capture an edit revision. A completed save only marks
  the buffer clean when no edit happened after the snapshot was taken; an undo
  followed by a new edit that returns to the same history depth is still dirty.

### Rendering

The renderer maintains a front and back `Screen` of cells. Each frame composes
the visible text area, line numbers, status bar and command line, then emits
ANSI cursor positioning and SGR sequences only for changed runs of cells.

Only the viewport is extracted from the document. Scrolling is expressed in
lines and display columns, and the cursor is kept visible after every event.
The terminal cursor is hidden; the block cursor is drawn by reversing the cell
under the cursor, which avoids flicker.

---

## Testing

```sh
ctest --test-dir build --output-on-failure
```

The test suite covers:

- piece-table insert/erase/range operations, including randomized comparison
  against `std::string`,
- line start / line lookup on large indexes,
- LF, CRLF and final-line behavior,
- UTF-8 decoding, code point boundaries, tabs and wide characters,
- Ex command parsing with quoted/escaped file names,
- undo/redo grouping, dirty-state tracking and redo-branch invalidation,
- atomic save and failure behavior,
- background-save snapshot isolation under continued editing,
- BOM and newline preservation.

The interactive UI was additionally smoke-tested in a pseudo-terminal
(insert, navigation, save/quit, quit confirmation, `:e` with unsaved changes,
new files, Unicode input, CRLF files and terminal resizing).

---

## Known limitations and next steps

- One buffer at a time; no buffer list or split panes yet.
- No SELECT/LINE SELECT modes, so `V`, `X` and `Shift+C` show a
  "not implemented" message.
- No syntax highlighting, LSP, formatter or configuration loading yet.
- The add buffer is append-only and grows with all text inserted during a
  session. A future compaction step can fold it into a fresh original buffer.
- File loading is still synchronous in the MVP, so opening a very large file
  blocks the UI while the initial read builds the piece index. Saving is
  asynchronous and no longer blocks the UI.
- Preparing a save snapshot copies the inserted-text buffer. For ordinary
  editing sessions this is tiny; an exceptionally large paste can make the
  snapshot step briefly expensive. The original file itself is shared, never
  copied.
- `lineText()` copies one line at a time; extremely long single lines are
  handled correctly but not as efficiently as ordinary lines.
- Unicode width tables cover common combining marks and East Asian wide ranges;
  grapheme clusters and emoji ZWJ sequences are not rendered as single cells.
- No external-modification warning, file locking, backups or crash recovery.
- No clipboard integration.

The core and UI boundaries were chosen so these can be added incrementally
without changing the buffer or command architecture.
