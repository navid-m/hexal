# Hexal (C#)

A C#/.NET 9 port of the C++ Hexal MVP. The project directory is `HexalCs`
because it sits alongside the original C++ tree, but the project and root
namespace are `Hexal` (`HexalCs/Hexal`).

The implementation is a cross-platform, modal, terminal text editor built on
the .NET 9 runtime with **no third-party packages** and no UI toolkit. It uses
the same architecture and observable behaviour as the C++ MVP:

- Chunked piece table with an array-backed implicit treap line index
- Viewport-only rendering with a front/back cell-grid diff renderer
- Raw ANSI terminal output, POSIX `termios` input and Windows Console input
- UTF-8, BOM preservation, LF/CRLF preservation
- NORMAL and INSERT modes, undo/redo, Ex commands and unsaved-change prompts
- **Background saves from an immutable piece snapshot** so large files can be
  saved without freezing input or scrolling

## Why this stack

The hot path avoids allocations and managed indirection:

- `PieceTable` stores treap nodes in one reusable `Node[]` with a free list,
  not one object per piece.
- The original file and inserted text are split into stable chunks. A save
  snapshot captures chunk references and piece descriptors; the worker streams
  those chunks while the UI keeps editing.
- Rendering writes UTF-8 ANSI into a reusable `ByteBuffer`, compares two
  `Cell[]` grids and emits only changed runs.
- Visible lines are copied into a reusable line buffer, not one `string` per
  line per frame.
- P/Invoke uses `LibraryImport` with runtime marshalling disabled, and the
  project is marked AOT/trim compatible.
- `PublishAot` produces a native executable for a C++-like startup and memory
  profile.

## Requirements

- .NET 9 SDK
- Linux/macOS: an ANSI UTF-8 terminal
- Windows 10/11 with virtual terminal processing (Windows Terminal is ideal)

## Build and run

```sh
cd HexalCs
dotnet build Hexal.sln -c Release
./Hexal/bin/Release/net9.0/hexal path/to/file.txt
```

On Windows:

```bat
dotnet build Hexal.sln -c Release
Hexal\bin\Release\net9.0\hexal.exe path\to\file.txt
```

A NativeAOT build:

```sh
dotnet publish Hexal/Hexal.csproj -c Release -r linux-x64 -p:PublishAot=true
./Hexal/bin/Release/net9.0/linux-x64/publish/hexal file.txt
```

(Use the matching runtime identifier on macOS and Windows.)

## Test

The test project is a dependency-free console harness, matching the C++ MVP's
CTest approach:

```sh
cd HexalCs
dotnet run --project Hexal.Tests -c Release
```

It covers:

- piece-table insertion, deletion and randomized differential editing
- line indexing on 100,000-line buffers and chunk-spanning inserts
- Unicode decoding, boundaries, display widths and tabs
- Ex command tokenization, quoted filenames and Windows paths
- LF/CRLF, BOM and final-line handling
- undo/redo grouping, dirty-state tracking and redo-branch invalidation
- atomic save failure preservation
- save-snapshot isolation while the live document keeps changing
- revision-based dirty tracking for edits made during a background save
- project file enumeration, ignored directories and content search offsets

## Usage

```text
hexal [options] [file]
```

| Option | Meaning |
| --- | --- |
| `-h`, `--help` | Print help and exit |
| `-v`, `--version` | Print version and exit |
| `--` | Treat remaining arguments as file names |

With no file, Hexal starts with an unnamed empty buffer. Relative file names in
`:e` and `:w` resolve against the current working directory.

Exit codes: `0` success, `1` terminal/fatal error, `2` invalid command line,
`4` file error at startup.

### Keys

| Key | Action |
| --- | --- |
| `i`, `I` | Enter INSERT mode |
| `a` / `A` | Append after cursor / at end of line, then INSERT |
| `o` / `O` | Open a line below / above, then INSERT |
| `Esc` | Return to NORMAL mode or cancel a prompt |
| `h` `j` `k` `l`, arrows | Move |
| `Home` / `End` | Start / end of line |
| `Ctrl+Home` / `Ctrl+End` | Start / end of document |
| `PageUp` / `PageDown` | Move by one screen |
| `gg` / `ge` / `gp` | First line / last line / previous location |
| `x` | Delete the character under the cursor |
| `u` / `Ctrl+R` | Undo / redo |
| `:` | Open the Ex command line |
| `Space` | Open the space menu (bottom-right) |
| `Space` `/` | Search across project files |
| `Space` `f` | Open the file picker |
| `Tab` | Insert a tab in INSERT mode |

### Space menu

Pressing `Space` in NORMAL mode opens a small menu in the bottom-right corner,
modelled on Helix's which-key menu. Press `Esc` to dismiss it, `/` to search
across files, or `f` to open the file picker. Both pickers filter as you type;
use Up/Down (or PageUp/PageDown) to select and Enter to open the result. File
pickers start from the current working directory and skip common build and VCS
directories. Search results open the file at the matching line.

### Commands

| Command | Action |
| --- | --- |
| `:w` | Save the current buffer in the background |
| `:w <file>` | Save as `<file>` and update the buffer name |
| `:wq` | Save, then quit once the saved snapshot has no newer edits |
| `:q` | Quit; asks for confirmation if dirty |
| `:q!` | Quit without saving |
| `:e <file>` | Open a file |
| `:e! <file>` | Open a file, discarding unsaved changes |
| `:help` | Show a short key reminder |

`:w`/`:wq` return immediately. The status bar shows `[saving]` while the worker
writes; scrolling and editing remain available. If an edit lands after the
snapshot, the document stays dirty, and `:wq` saves the newer state before
quitting. `:q`/`:q!` wait for an in-flight save rather than terminating the
process mid-write.

## Data safety

- Saves write a sibling temporary file, flush it, copy the original POSIX
  permission bits, then rename over the target. The original is never truncated
  in place.
- A failed write or rename removes the temporary file and leaves the original
  untouched.
- A UTF-8 BOM is preserved; invalid UTF-8 is preserved on disk and displayed as
  U+FFFD.
- LF/CRLF is detected from the first newline and used for newly inserted line
  endings; mixed files are preserved byte-for-byte.
- Background saves never mark newer edits as persisted.

## Architecture

```text
HexalCs/Hexal/
  Program.cs                 command-line parsing and startup
  Core/
    PieceTable.cs            chunked piece table + array treap + save snapshots
    Document.cs              file metadata, undo/redo, dirty revisions, saving
    ExCommandParser.cs       quoted Ex command tokenizer
  Platform/
    Terminal.cs              terminal interface and factory
    PosixTerminal.cs         termios, poll, SIGWINCH, ANSI input decoder
    WindowsTerminal.cs       Console API, ReadConsoleInputW, VT output
  Ui/
    Screen.cs                cell grid and wide-character cells
    Renderer.cs              ANSI diff renderer
    Editor.cs                modes, key handling, commands, async save state
  Util/
    ByteBuffer.cs            reusable growable byte buffer
    UnicodeUtil.cs           UTF-8, boundaries, display widths
    FileSystemUtil.cs        binary read and atomic replacement
    FileSearch.cs            project file enumeration and text search
```

`Core` has no terminal or UI dependency and is what the test project exercises.
The platform layer is the only place with OS-specific code.

### Buffer strategy

The buffer is a piece table backed by an implicit treap. The original file is
never mutated; edits append to stable chunks. Node arrays are augmented with
subtree byte and newline counts, giving expected `O(log p)` insertion,
deletion, offset-to-line and line-to-offset queries. Pieces are split at
roughly 8 KiB newline boundaries to bound newline scans.

### Coordinate model

1. **Byte offsets** into UTF-8 text identify storage positions.
2. **Unicode scalar boundaries** govern cursor movement and deletion.
3. **Display columns** are terminal cells: tabs expand to multiples of four,
   East Asian wide scalars occupy two cells, combining marks are zero width,
   and control characters render as U+FFFD.

The cursor is a byte offset; display conversion happens only at the UI edge.

### Undo / redo

Adjacent typing and deletion are grouped. Cursor movement, mode changes, save,
undo and redo close the current group. Groups store only inserted/removed text,
never document snapshots. History is bounded by 4096 groups or 64 MiB. The
modified flag follows the saved undo pose and a revision counter so a
background save cannot mark later edits clean.

### Rendering

Every frame composes visible text, line numbers, status and command lines into
a cell grid. The renderer diffs against the previous grid and emits ANSI cursor
addressing plus SGR sequences only for changed runs. The physical cursor is
hidden; the block cursor is a reversed cell. Only viewport lines are extracted.

## Differences from the C++ MVP

The feature set and command behaviour match, including asynchronous saves with
a `[saving]` indicator and revision-aware dirty tracking. The C# port
additionally uses chunked storage for both original and inserted text (avoiding
a single large managed array) and supports NativeAOT publishing.

File **loading** remains synchronous in both implementations; only saving is
backgrounded.
