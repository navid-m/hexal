# Hexal Design Notes (MVP)

This document records the decisions that the main specification asks the
implementation to make explicit: buffer strategy, coordinate model, undo
policy, incremental rendering, file safety and platform isolation.

## 1. Layering

```text
main
  |
  +-- Editor (ui/editor) -------------------- modes, commands, viewport
  |     |
  |     +-- Screen / Renderer (ui) ---------- cell grid + ANSI diff
  |     +-- Terminal (platform) -------------- input/output abstraction
  |
  +-- Document (core) ------------------------ file state + undo
        |
        +-- PieceTable (core) ---------------- text + line index
        +-- fsutil (util) -------------------- read / atomic write
```

`hexal_core` (Document, PieceTable, command parser, Unicode, filesystem) does
not include `<termios.h>`, `<windows.h>` or any UI header. It is linked into
both the editor and the unit tests.

The terminal interface has exactly the operations the editor needs:

- initialize / restore the terminal,
- read one decoded key event,
- write a byte string,
- query the window size.

Everything OS-specific lives in `terminal_posix.cpp` or `terminal_win32.cpp`.

## 2. Piece table

### 2.1 Data

Two byte buffers:

- `original_`: the file exactly as loaded (BOM removed, everything else
  untouched), never modified.
- `added_`: append-only storage for every insertion.

The document is an ordered sequence of pieces. A piece is
`(source, start, length)`, where `source` is either `original_` or `added_`.
A piece never stores a pointer into a string; it stores offsets, so growing
`added_` cannot invalidate the document.

### 2.2 Tree

The piece sequence is an implicit treap (a randomized balanced binary tree
keyed by position). Every node stores:

- `length` and `pieceNewlines` for its own piece,
- `bytes` = total bytes in its subtree,
- `newlines` = total `'\n'` bytes in its subtree,
- a deterministic 64-bit priority.

`split(root, byteOffset)` returns two trees whose concatenation is the original
document. `merge(left, right)` concatenates them. Insert and erase are each one
or two splits plus merges.

When a split falls in the middle of a piece, two nodes are created: the left
half inherits the original node's left subtree and priority, the right half
inherits the right subtree and the same priority. Keeping the priority
preserves the heap invariant. No text is copied.

Expected complexity with `p` pieces:

| Operation | Cost |
| --- | --- |
| insert / erase | `O(log p)` splits + merges |
| byte offset to line | `O(log p)` + scan of one piece prefix |
| line to byte offset | `O(log p)` + scan of one piece |
| extract visible line | `O(log p + line length)` |
| save | `O(n)` streaming, no second full-document string |

### 2.3 Chunking

`setOriginal` splits the file into pieces of about 8 KiB, preferring a newline
boundary. This bounds the newline scan inside a piece. Inserted text is chunked
the same way, so a large paste does not create a single huge piece. Ordinary
keystrokes create small add pieces.

### 2.4 Why not the alternatives

- **Gap buffer**: excellent for local edits but moves `O(n)` bytes on distant
  jumps and cannot cheaply retain the original file for undo.
- **Line vector**: line-number lookup is easy, but inserting a line in the
  middle shifts all following entries, and a million-line file becomes a
  million allocations.
- **Rope of strings**: similar query cost, but more memory overhead per node
  than a piece reference and more copying when splitting.
- **Vector of pieces**: simplest, but inserting/deleting pieces is `O(p)`, so
  long editing sessions degrade. The treap provides predictable logarithmic
  behavior while keeping the same piece representation.

The MVP deliberately does not implement a red-black tree or B-tree; the treap
is small, deterministic (fixed splitmix64 seed) and sufficient for the required
millions-of-lines behavior.

### 2.5 Memory behavior

- Opening a file stores exactly one copy of its bytes.
- Editing copies only inserted text (into `added_`) and the removed text
  recorded for undo.
- Saving streams pieces directly to the output file; it does not call
  `toString()`.
- `added_` is append-only for the life of a buffer. This is a known trade-off:
  a very long session with many edits can grow it beyond the live document
  size. A future compaction pass can rebuild `original_` after save or when the
  add buffer exceeds a threshold.

## 3. Coordinates

| Concept | Representation | Used by |
| --- | --- | --- |
| Byte offset | `std::size_t` into UTF-8 text | piece table, undo, file positions |
| Code point boundary | a byte offset at the start of a valid UTF-8 sequence | cursor movement, deletion |
| Display column | terminal cells, zero-based | rendering, status bar, horizontal scroll |

Details:

- `unicode::previousBoundary` / `nextBoundary` move over one code point, not one
  byte. Invalid bytes are treated as one-byte replacement characters.
- Tabs advance to the next multiple of `kTabWidth` (4).
- East Asian wide characters occupy 2 cells. Common combining marks and
  variation selectors occupy 0 cells (they are not drawn separately because a
  cell stores one code point in the MVP).
- Control characters other than tab are shown as U+FFFD.
- The cursor is stored as a byte offset and snapped to a line's text range;
  it can never sit between `\r` and `\n` of a CRLF pair.
- `Document::lineOfOffset` and `lineStart` work in bytes, so line and column
  reporting is independent of how the text is displayed.

## 4. Undo and redo

Each `Change` is `(offset, removed text, inserted text)`. A `Group` is a list
of changes and a kind (`Insert`, `Erase`, `Mixed`). Changes are applied in
reverse order for undo and forward order for redo.

Grouping rules:

- Consecutive contiguous insertions without a line break are merged. This makes
  a typed word one undo step, not one step per keystroke.
- Consecutive backspaces at decreasing offsets are merged to the left.
- Consecutive forward deletes at the same offset are merged to the right.
- A line break in inserted or removed text closes the group.
- `breakUndoGroup()` is called on cursor movement, mode changes, save, undo and
  redo.
- `replace()` always creates a single `Mixed` group. This is the primitive that
  future formatters and multi-line comments will use.

Dirty state:

- `savedPose_` is the number of applied groups at the last successful save.
- `isDirty()` is `doneGroups != savedPose_` (or an explicit "unreachable" flag).
- Undoing back to the save point marks the buffer clean; redoing forward marks
  it dirty again.
- If an edit discards a redo branch that contained the saved state, the flag is
  set to unreachable (-1).

Retention:

- At most 4096 applied/redo groups.
- At most 64 MiB of stored inserted + removed text.
- Oldest applied groups are discarded first; the redo side is trimmed only when
  memory still requires it. Discarding history adjusts the saved-pose marker,
  or invalidates it if the saved state was discarded.
- A single group is never split; a huge paste can therefore exceed the byte
  budget until it is superseded.

Cursor restoration: the Document returns a hint offset for undo (start of the
reverted insert, or end of restored removed text) and redo (end of the
reapplied insert). The Editor snaps and scrolls to it. Full per-group
cursor/selection restoration is deferred to the selection-mode phase.

## 5. Rendering

- `Screen` is a width × height vector of `Cell { char32_t, Style, wideCont }`.
- A wide character writes its code point to one cell and a continuation marker
  to the next. The renderer never writes the continuation directly; the
  terminal advances over it when the wide code point is emitted.
- Every frame starts from a cleared back buffer and composes:
  text area → line numbers → cursor → status bar → command/message line.
- `Renderer` compares against the previous front buffer and emits
  `CSI row;col H` only for changed runs. Runs split on style changes.
- Changing size clears the terminal and forces a full redraw.
- The physical cursor is hidden (`CSI ?25l`). The cursor is a reversed cell.
- Only `textRows` lines starting at `topLine_` are extracted; no traversal of
  the whole document occurs while rendering.
- Horizontal scrolling stores `leftColumn_` in display cells. Tabs and wide
  characters that straddle the left edge are padded with spaces.

Style is limited to the 16 ANSI colors plus bold/dim/reverse. This avoids
depending on true-colour support. The MVP status bar uses blue (NORMAL), green
(INSERT), cyan (COMMAND) and magenta (CONFIRM).

## 6. File handling

### Loading

- `std::ifstream` in binary mode, read in 64 KiB chunks.
- A leading UTF-8 BOM is removed from the buffer and remembered; it is written
  back on save.
- The first newline determines the preferred line ending: `\r\n` if preceded
  by `\r`, otherwise `\n`. Mixed files are preserved byte-for-byte; only newly
  inserted line endings use the detected style.
- A nonexistent path is accepted as a new file only if its parent directory
  exists and is a directory.

### Saving

1. Create a sibling temporary file with a unique name.
2. Write BOM (if any) and stream all pieces in order.
3. Flush and check the stream.
4. Copy the original file's POSIX permission bits to the temporary file.
5. Rename over the target (`std::rename` on POSIX,
   `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows).
6. On any failure, remove the temporary file; the original is untouched.

The target path is only updated after a successful `:w <file>` save.

Not implemented: fsync, backups, external-modification checks, preserving
Windows ACLs, and symlink policy beyond normal OS behavior.

## 7. Terminal layer

### POSIX

- `tcgetattr` / `tcsetattr` with a raw configuration (no echo, canonical mode,
  signals, CR/NL translation or output post-processing).
- Output uses absolute cursor addressing, so `OPOST` is disabled safely.
- `SIGWINCH` sets a `sig_atomic_t` flag and interrupts the blocking `poll`.
  The next `readEvent` returns `Key::Resize`.
- Escape sequences are parsed with short `poll` timeouts: arrows, Home/End,
  Insert/Delete, PageUp/PageDown, F1–F12, Shift+Tab and xterm modifier
  encodings are recognized. UTF-8 input is decoded with replacement on
  malformed sequences.
- Ctrl+letter is mapped to `Character` with `ctrl = true`.
- On shutdown: reset SGR, show cursor, leave the alternate screen, restore
  termios and the old `SIGWINCH` handler. Destructor repeats shutdown safely.

### Windows

- The console output mode enables `ENABLE_VIRTUAL_TERMINAL_PROCESSING` and
  disables newline auto-return. If this fails, the editor reports that
  Windows 10 or newer is required instead of writing garbled output.
- Input uses `ReadConsoleInputW`. This gives proper Unicode characters,
  surrogate pairs, arrow/home/end/page keys and window resize events without
  assembling escape sequences.
- Ctrl+letter and AltGr are handled explicitly. Input/output code pages are set
  to UTF-8 and restored on exit.
- Console modes and code pages are restored, the cursor is shown, and the
  alternate screen is left on shutdown.

## 8. Testing and benchmarks

Unit tests (`tests/test_main.cpp`) link only `hexal_core` and are run by CTest.
They include a randomized differential test of 3000 edits against
`std::string`, line-index checks on a 100,000-line buffer, and file round trips.

For a full performance report, generate representative files (100k, 1M, several
million lines) and measure with a release build:

- time to open and first render,
- cursor jump latency (`ge`, `gg`, PageUp/Down),
- single-character edit latency at the start, middle and end,
- save duration,
- peak RSS.

The current implementation's expected costs are `O(log p)` per edit/jump and
`O(visible lines + line lengths)` per frame. Remaining hot spots for future
work are per-line string extraction, newline scanning inside an unusually large
piece, and the append-only add buffer.
