# Hexal (C#)

A C#/.NET 9 port of the C++ Hexal MVP. The project directory is `HexalCs`
because it sits alongside the original C++ tree, but the project and root
namespace are `Hexal` (`HexalCs/Hexal`).

The implementation is a cross-platform, modal, terminal text editor built on
the .NET 9 runtime with no UI toolkit. Its single runtime dependency is NLua,
which provides the native Lua 5.4 runtime for plugins. It uses the same
architecture and observable behaviour as the C++ MVP:

- Chunked piece table with an array-backed implicit treap line index
- Viewport-only rendering with a front/back cell-grid diff renderer
- Raw ANSI terminal output, POSIX `termios` input and Windows Console input
- UTF-8, BOM preservation, LF/CRLF preservation
- NORMAL, INSERT and SELECT modes, undo/redo, Ex commands and
  unsaved-change prompts
- **Background saves from an immutable piece snapshot** so large files can be
  saved without freezing input or scrolling
- **Automatic external reload on terminal focus**: one metadata stat in the
  common case, and a full read only when the file actually changed
- **Data-driven `.hexdef` language grammars** loaded from `data/grammars/`
  beside the executable, with viewport-only incremental syntax highlighting
- **Lua plugins** through an isolated NLua state and the documented
  `hexal.*` API modules
- **Bundled grammars and LSP plugins** for Lua, Go, XML, C#, C, C++, Python,
  Cython, GLSL, Shell Script, Git Ignore, PowerShell, JSON, and Fish Shell; the
  first language server found in `PATH` is started automatically

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
- NuGet restore for NLua/KeraLua. The package carries native Lua 5.4 libraries
  for Linux (x64/arm64), macOS, and Windows (x86/x64/arm64).

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

NLua's native library is copied beside the published executable
(`liblua54.so`, `liblua54.dylib`, or `lua54.dll`). Keep it in the same directory
when installing the binary; `install.sh` does this for Linux.

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
- `.hexdef` parsing, extension detection and incremental tokenization of
  keywords, types, constants, strings, numbers and multiline comments
- theme colour parsing, named/ANSI/true-colour values and directory loading

## Usage

```text
hexal [options] [file]
```

| Option | Meaning |
| --- | --- |
| `-h`, `--help` | Print help and exit |
| `-v`, `--version` | Print version and exit |
| `--health [language]` | Print language support health and exit |
| `--` | Treat remaining arguments as file names |

With no file, Hexal starts with an unnamed empty buffer. Relative file names in
`:e` and `:w` resolve against the current working directory.

Exit codes: `0` success, `1` terminal/fatal error, `2` invalid command line,
`4` file error at startup.

`--health` prints one row per loaded grammar with the availability of its
language server, formatter and syntax highlighting. Pass a language id to show
only that row.

### Keys

| Key | Action |
| --- | --- |
| `i`, `I` | Enter INSERT mode |
| `a` / `A` | Append after cursor / at end of line, then INSERT |
| `o` / `O` | Open a line below / above, then INSERT |
| `Esc` | Return to NORMAL mode, cancel SELECT mode or cancel a prompt |
| `h` `j` `k` `l`, arrows | Move |
| `Home` / `End` | Start / end of line |
| `Ctrl+Home` / `Ctrl+End` | Start / end of document |
| `PageUp` / `PageDown` | Move by one screen |
| `gg` / `ge` / `gp` | First line / last line / previous location |
| `x` / `X` | Select the current line; repeat to extend by one line |
| `v` / `V` | Enter SELECT mode at the cursor |
| `d` / `D`, `Delete` | Delete the character/selection under the cursor |
| `u` / `Ctrl+R` | Undo / redo |
| `:` | Open the Ex command line |
| `Ctrl+P` | Open the command palette |
| `/` | Search inside the current file (case insensitive) |
| `.` | Select every match of a search term, then edit them together (NORMAL or SELECT) |
| `Space` | Open the space menu (bottom-right) |
| `Space` `/` | Search across project files |
| `Space` `f` | Open the file picker |
| `Space` `y` | Yank the selection to the system clipboard |
| `Space` `p` | Paste the system clipboard on the current line |
| `Space` `P` | Paste the system clipboard on a new line below |
| `Tab` | Insert a tab in INSERT mode |

### Space menu

Pressing `Space` in NORMAL or SELECT mode opens a small menu in the
bottom-right corner, modelled on Helix's which-key menu. Press `Esc` to
dismiss it, `/` to search across files, or `f` to open the file picker.
`y` copies the current selection to the system clipboard and returns to
NORMAL mode; `p` pastes the clipboard at the cursor (replacing the selection
when one is active) and `P` pastes it on new lines below the cursor's line.
Both pickers filter as you type; use Up/Down (or
PageUp/PageDown) to select and Enter to open the result. File pickers start
from the current working directory and skip common build and VCS directories.
Search results open the file at the matching line.

### Command palette

Press `Ctrl+P` in NORMAL, SELECT or INSERT mode to open the searchable command
palette. Type to fuzzy-filter commands and use Up/Down (or PageUp/PageDown) to
select one; `Enter` runs it and `Esc` closes the palette. Built-in entries
include opening files and project search, opening the configuration file and the
`~/.hexal` directory, toggling the sidebar, formatting, saving, diagnostics and
the theme selector. Commands registered by Lua plugins appear under `Plugin:`.

### Theme selector

The palette's **Open theme selector** command lists every theme found in
`data/themes/`. Type to filter by name or description, move with the arrow keys
to preview a theme live, press `Enter` to keep it and `Esc` to restore the
previous theme. The choice is saved in `~/.hexal/theme` and restored on the next
start. `:rcfg` also reloads themes.

### In-file search

Press `/` in NORMAL mode to open the find box. Type a query and press
`Enter` to finish editing it; all case-insensitive matches are highlighted
as you type and the first match at or after the cursor is selected. Press
`n` for the next match and `p` for the previous match (wrapping at the ends).
`Esc` closes find mode. `Up`/`Down` and `Ctrl+N`/`Ctrl+P` also move between
matches while the query is being edited.

Press `.` in NORMAL mode to open the same find box in multi-cursor mode.
Typing highlights every match as usual. Press `Enter` to close the box and turn
all matches into selections with one cursor per match. The normal selection
commands then apply to every match at once: for example, press `Delete` (or
`d`) and then `I` to type a replacement at every location. Press `,` to keep
only the primary cursor.

Pressing `.` with an active SELECT-mode selection searches only inside that
selection. Matches outside it are ignored, so a subsequent delete/replace
leaves the rest of the buffer untouched. With multiple selections, the search
is limited to the union of their selected ranges.

Clipboard access uses native helpers when available (`xclip`, `xsel`, or
`wl-copy` on Linux, `pbcopy`/`pbpaste` on macOS, and the Win32 clipboard API on
Windows). When no copy helper is present, Hexal falls back to the terminal's
OSC 52 clipboard sequence, which also works over SSH with terminals that
support it. Pasting without a clipboard helper reports an error.

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
| `:fmt` | Format the current buffer with its language formatter |
| `:cfg` | Open the active configuration file |
| `:rcfg` | Reload configuration, grammars and themes |
| `:gi` | Toggle showing git-ignored files in the sidebar and pickers |
| `:gdl` | Toggle added/deleted/modified markers in the line-number gutter |
| `:help` | Show a short key reminder |
| `:debug-lsp`, `:dl` | Inspect live language-server traffic and client diagnostics |

`:w`/`:wq` return immediately. The status bar shows `[saving]` while the worker
writes; scrolling and editing remain available. If an edit lands after the
snapshot, the document stays dirty, and `:wq` saves the newer state before
quitting. `:q`/`:q!` wait for an in-flight save rather than terminating the
process mid-write.

### External changes

The terminal's focus-reporting mode is enabled at startup. When the window
regains focus Hexal performs one `stat` of the current file and compares its
size, modification time and creation time with the values recorded at the last
open/save/reload. If nothing changed, no file data is read.

If the file did change, Hexal reloads it immediately and restores the cursor's
line and display column. This deliberately differs from Helix's explicit `:rla`
workflow: external content wins, and unsaved edits are replaced. The status bar
reports the reload and notes when unsaved edits were discarded.

Focus reporting requires terminal support (`\e[?1004h`); when unavailable,
external changes are picked up only after reopening the file.

## Data locations

Hexal prefers a portable layout beside the executable:

```text
~/Applications/Hexal/hexal
~/Applications/Hexal/data/grammars/
~/Applications/Hexal/data/plugins/
~/Applications/Hexal/data/themes/
```

If `data/grammars`, `data/plugins` or `data/themes` does not exist there, that
category falls back independently to:

```text
~/.hexal/data/grammars/
~/.hexal/data/plugins/
~/.hexal/data/themes/
```

Path resolution and load results are appended to `~/.hexal/hexal.log`. When
neither portable directory exists the log contains:

```text
Could not find grammars, plugins and themes relative to binary path, falling back to ~/.hexal directory
```

## Language grammars

Language definitions are human-readable `.hexdef` JSON files placed in
`<binary>/data/grammars/`, or `~/.hexal/data/grammars/` when the portable
directory is absent. The fallback directory is created on startup. Files are
loaded once, sorted by name, and matched to buffers by extension. Invalid
definitions are reported in the status bar and skipped without stopping the
editor.

Canonical schema:

```json
{
    "id": "cpp",
    "name": "C++",
    "extensions": [".cpp", ".cc", ".hpp", ".h"],
    "keywords": ["class", "struct", "namespace", "template"],
    "controlStatements": ["if", "else", "for", "while", "return"],
    "types": ["int", "char", "bool", "void"],
    "constants": ["true", "false", "nullptr"],
    "comments": {
        "line": "//",
        "blockStart": "/*",
        "blockEnd": "*/"
    },
    "strings": {
        "quoteCharacters": ["\"", "'"],
        "escapeCharacter": "\\"
    },
    "numbers": true,
    "formatter": {
        "command": "clang-format",
        "arguments": ["--assume-filename={file}"],
        "input": "stdin",
        "output": "stdout",
        "timeout": 15000
    },
    "formatOnSave": true
}
```

- `comments.blockStart` and `comments.blockEnd` must be supplied together.
- `lineComment` and `blockComment: { "start": ..., "end": ... }` are accepted
  as aliases when `comments` is absent.
- `numbers` defaults to `true`.
- Highlight categories are keyword, control statement, type, constant, string,
  comment, and number. Default text uses the terminal foreground.
- `:lang <id>` overrides auto-detection for the current buffer.

### Formatting

`:fmt` formats the current buffer with the language's configured formatter.
Formatter commands are data-only and are never run through a shell.

- `formatter.command` is the executable name or path.
- `formatter.arguments` is an array of arguments. `{file}` and `{path}` expand
  to the document path, `{tmp}`/`{temp}`/`{output}` expand to the temporary
  file used for file-based formatters.
- `formatter.input` is `stdin` (default) or `file`.
- `formatter.output` is `stdout` (default) or `file`.
- `formatter.timeout` is the maximum run time in milliseconds (default 15000).

On success, the formatted output replaces the buffer as a single undo step and
the cursor and selection line/column are preserved. On failure, the buffer is
left unchanged and the error is shown in the status bar.

`formatOnSave` in a language definition overrides the global setting. When it
is omitted, the global setting from `~/.hexal/.hexcfg` is used:

```json
{
    "version": 1,
    "formatOnSave": false,
    "showDiffLines": true,
    "showGitIgnored": false
}
```

`showDiffLines` controls the Git diff markers in the line-number gutter. They
are enabled by default and use green for added lines, red for deleted lines and
blue for modified lines. `showGitIgnored` controls whether files excluded by
`.gitignore` appear in the sidebar, file picker and cross-file search. Both
settings are toggled with `:gdl` and `:gi` and written back to the active
configuration file.

The active configuration is `~/.hexal/.hexcfg` on every platform; a plain
`~/.hexal/hexcfg` file is also accepted. `:cfg` opens the configuration file
and creates it when missing, while `:rcfg` reloads the configuration, grammars,
themes and language definitions. Relative formatter paths are resolved against
PATH.

Only visible lines are tokenized. Lexer states are cached per line, so block
comments remain correct while scrolling without rescanning the document on
every frame or edit. A viewport far from any cached state starts in the default
state, keeping jumps into multi-million-line files proportional to the visible
window.

## Themes

Themes are JSON files in `<binary>/data/themes/`, or
`~/.hexal/data/themes/` when the portable directory is absent. Hexal ships with
`hexal-dark`, `nord`, `gruvbox`, `solarized-light` and `mono`. To add a theme,
copy one of those files, give it a unique `id`, and restart or run `:rcfg`.
Malformed theme files are logged and skipped.

Colours can be an ANSI palette index (`0`-`255`), a name such as `"bright-cyan"`
or `"black"`, or a `"#RRGGBB"` true-colour value. Missing entries inherit the
default palette.

```json
{
    "id": "my-theme",
    "name": "My Theme",
    "description": "A short description shown in the theme selector.",
    "colors": {
        "foreground": "#d8dee9",
        "background": "#2e3440",
        "muted": "#616e88",
        "accent": "#88c0d0",
        "border": "#434c5e",
        "error": "#bf616a",
        "warning": "#ebcb8b",
        "success": "#a3be8c",
        "alert": { "fg": "#eceff4", "bg": "#bf616a" },
        "selection": { "fg": "#eceff4", "bg": "#5e81ac" },
        "status": { "fg": "#eceff4", "bg": "#4c566a" },
        "insert": { "fg": "#2e3440", "bg": "#a3be8c" },
        "select": { "fg": "#2e3440", "bg": "#ebcb8b" },
        "prompt": { "fg": "#2e3440", "bg": "#88c0d0" },
        "command": { "fg": "#2e3440", "bg": "#88c0d0" },
        "match": { "fg": "#2e3440", "bg": "#ebcb8b" },
        "currentMatch": { "fg": "#2e3440", "bg": "#a3be8c" },
        "syntax": {
            "keyword": "#b48ead",
            "control": "#81a1c1",
            "type": "#8fbcbb",
            "constant": "#d08770",
            "string": "#a3be8c",
            "comment": "#616e88",
            "number": "#d08770"
        }
    }
}
```

The active theme id is stored in `~/.hexal/theme`.

## Lua plugins

Plugins live under `<binary>/data/plugins/<plugin-id>/`, or
`~/.hexal/data/plugins/<plugin-id>/` when the portable directory is absent:

```text
<binary>/data/plugins/example/
    plugin.toml
    init.lua
```

`plugin.toml` is a small key/value manifest. `entry` defaults to `init.lua`;
`enabled = "false"` disables a plugin; `id` defaults to the directory name.

```toml
name = "Example"
version = "1.0"
entry = "init.lua"
description = "Example Hexal plugin"
```

```lua
hexal.commands.register("Hello", function()
    hexal.ui.message("Hello from Hexal")
end)

hexal.keymap.map("space h", "Hello")
```

Plugin commands are callable through the Ex prompt (`:Hello`). Keymaps accept
space-separated sequences, mode names `normal`, `insert`, or `select`, and
single-key/chord tokens such as `f`, `space`, `esc`, `ctrl+p`, or `alt+x`.

Implemented API modules:

| Module | Notes |
| --- | --- |
| `hexal.editor` | buffer text, line/cursor/selection inspection, cursor movement, insert/replace with undo integration |
| `hexal.commands` | register, unregister, execute |
| `hexal.keymap` | map, map_callback, unmap |
| `hexal.ui` | message, warn, prompt, select, set_status |
| `hexal.events` | on/off for buffer and editor lifecycle events |
| `hexal.lsp` | start/stop external servers, create Lua LSP servers, diagnostics subscriptions, custom methods and navigation |
| `hexal.config` | per-plugin JSON settings under `~/.hexal/plugin-config/<id>.json` |

Line and column values are zero-based. Columns count Unicode scalars, not bytes
or display cells. `replace_range` accepts numeric UTF-8 byte offsets or
`{ line = ..., column = ... }` tables. Edits made through the API participate
in normal undo/redo and mark the buffer dirty.

### Language servers

Bundled plugins associate each grammar with language-server candidates and
start the first executable found in `PATH` when a matching buffer is opened:

| Language | Extensions | Server candidates |
| --- | --- | --- |
| Lua | `.lua` | `lua-language-server` |
| Go | `.go` | `gopls` |
| XML | `.xml`, `.csproj`, `.rpp` | `lemminx`, `xml-language-server` |
| C# | `.cs` | `csharp-ls`, `roslyn-language-server`, `omnisharp` |
| C | `.h`, `.c` | `clangd` |
| C++ | `.h`, `.cpp`, `.hpp` | `clangd` |
| Python | `.py` | `pyright-langserver`, `ty`, `mypy`, `mypy-language-server` |
| Cython | `.pyx` | `cython-lsp`, `pyright-langserver`, `ty`, `mypy` |
| GLSL | `.gdshader`, `.shader`, `.fs`, `.vs`, `.glsl` | `glsl_analyzer`, `glsl-language-server` |
| Shell Script | `.sh`, `.bash`, `.zsh`, `.ksh`, `.dash`, `.ash`, and shell rc files | `bash-language-server` |
| Git Ignore | `.gitignore`, `.npmignore`, `.dockerignore`, and similar ignore files | none (syntax highlighting only) |
| PowerShell | `.ps1`, `.psm1`, `.psd1`, `.ps1xml`, `.pssc`, `.psrc` | `powershell-language-server`, PowerShell Editor Services |
| JSON | `.json`, `.jsonc`, `.jsonl`, `.ndjson`, `.geojson`, `.har`, `.webmanifest` | `vscode-json-languageserver` |
| Fish Shell | `.fish` | `fish-lsp` |

PowerShell Editor Services is launched through `pwsh` rather than as a
standalone executable. Set `bundlePath` to the extracted PSES directory in
`~/.hexal/plugin-config/powershell.json` to enable it.

The LSP client runs asynchronously over stdio with JSON-RPC framing. It performs
`initialize`, opens the document, sends debounced full-document changes and
save notifications, and displays diagnostic counts in the status bar.
`hexal.lsp.on_diagnostics` callbacks receive the server, URI and diagnostic
objects. Servers are stopped and restarted when the buffer or language changes.
If the executable is missing, the next candidate is attempted; absence of a
server does not affect normal editing. When configuring OmniSharp manually,
include `--languageserver` in its arguments; without that flag it writes
newline-delimited host events instead of framed LSP messages, and the client
will wait forever for an `initialize` response.

Run `:debug-lsp` (or `:dl`) to open a full-screen, live view of the language
server state and recent traffic. It shows the command and arguments, process
ID, initialization state, pending requests and the JSON-RPC messages exchanged
with the server, plus its stderr and any unframed raw stdout. Use `j`/`k`, the
arrow keys, `PageUp`/
`PageDown` and `Home`/`End` to scroll, `y` to copy the complete report to the
system clipboard, and `c` to clear the captured log. The same report notes the
path of the persistent `~/.hexal/hexal.log` file.

### Lua language servers

The same Lua plugin runtime can implement a complete LSP server. Hexal owns
JSON-RPC framing, request correlation, initialization/shutdown sequencing,
document synchronization, cancellation contexts, capability negotiation and
the editor transport. Lua owns parsing, analysis and language intelligence.

```lua
local lsp = require("hexal.lsp")
local diagnostics = require("hexal.lsp.diagnostics")

local server = lsp.create_server({
    name = "example-language",
    version = "1.0.0",
    filetypes = { "example" },
    extensions = { ".example" },
    root_markers = { ".git", "example.json" },
})

server:capabilities({
    hover = true,
    completion = true,
    definition = true,
    references = true,
    document_symbols = true,
    rename = true,
    code_actions = true,
    formatting = true,
})

server:on("document_changed", function(doc, ctx)
    diagnostics.publish(doc.uri, {
        -- Analyzer results go here.
    })
end)

server:register("textDocument/hover", function(params, ctx)
    local doc = ctx.documents:get(params.textDocument.uri)
    if doc and doc:word_at(params.position) == "greet" then
        return { contents = { kind = "markdown", value = "**greet(name)**" } }
    end
end)

server:start()
```

Calling `server:start()` registers the server with Hexal. When the active
buffer matches `filetypes` or `extensions`, the editor connects to it using the
normal LSP client path, so hover, code actions, diagnostics, saves and
incremental changes work without additional C# changes.

The runtime exposes:

| Module | Purpose |
| --- | --- |
| `hexal.lsp` | `create_server`, server lifecycle and handler registration |
| `hexal.lsp.protocol` | JSON-RPC error codes, `make_error`, error detection |
| `hexal.lsp.documents` | Documents, snapshots, virtual documents, ranges |
| `hexal.lsp.workspace` | Root folders, configuration, files, watching, indexing |
| `hexal.lsp.capabilities` | Client/server capabilities and dynamic registration |
| `hexal.lsp.diagnostics` | Namespaced diagnostics, severities and change hooks |
| `hexal.lsp.completion` | Items, lists, resolution and kind constants |
| `hexal.lsp.symbols` | Locations, document/workspace symbols and kinds |
| `hexal.lsp.edit` | Text edits, workspace edits and application |
| `hexal.lsp.types` | LSP structure constructors and validation |
| `hexal.lsp.uri` / `position` | URI and position/range utilities |
| `hexal.lsp.semantic_tokens` | Legends, encoding, caching and invalidation |
| `hexal.lsp.inlay_hint` / `signature_help` | Hint and signature structures |
| `hexal.lsp.folding_range` / `code_lens` | Folding ranges and code lenses |
| `hexal.async` | Coroutine tasks, sleep, await, debounce and scheduling |
| `hexal.json` | JSON encoding/decoding with a `json.null` sentinel |
| `hexal.fs` | Permission-aware filesystem helpers |
| `hexal.log` | Trace/debug/info/warn/error logging |
| `hexal.index` | In-memory incremental symbol/reference index |
| `hexal.testing.lsp` | In-memory LSP client for server unit tests |
| `hexal.debug` | Handler, document, capability and request introspection |

Request handlers receive `(params, ctx)`; notifications and lifecycle events
receive their event value and `ctx`. The context exposes `server`, `client`,
`documents`, `workspace`, `request_id`, `method`, `cancelled`, `progress`,
`initialization_options`, `settings` and `reject`.

Lua handlers are single-threaded and run on the editor thread. File watching,
timers and async callbacks are pumped by the editor loop; CPU-heavy work should
use the optional `hexal.process` module. Filesystem reads and writes are allowed
by default for local plugins but can be restricted per server:

```lua
permissions = {
    read = true,
    write = false,
    process = false,
    workspace_only = true,
    roots = { "/path/to/project" },
}
```

The in-process runtime targets the LSP 3.17 method set, supports UTF-8, UTF-16
and UTF-32 position encodings (UTF-16 by default), and advertises methods only
when the Lua server explicitly declares them through `server:capabilities` or
raw capability structures.

```
Plugins run in isolated in-process Lua states. NLua's CLR package is not loaded,
and only the documented `hexal.*` tables are exposed. An in-process Lua runtime
is nevertheless **not a security boundary**; do not install untrusted plugins.

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
    Clipboard.cs             native clipboard helpers and OSC 52 fallback
  Lsp/
    LspManager.cs            async JSON-RPC client, document sync, diagnostics
    LspServerConfig.cs       server config, executable lookup, diagnostic types
  Languages/
    LanguageDefinition.cs    validated .hexdef schema and token categories
    LanguageRegistry.cs      data/grammars loading and extension detection
    SyntaxHighlighter.cs     visible-line lexer with cached block states
  Scripting/
    IEditorApi.cs            narrow Lua-to-editor host contract
    LuaPluginManager.cs      isolated Lua states, modules, keymaps, events
    PluginConfigStore.cs     per-plugin JSON configuration
  Ui/
    Screen.cs                cell grid and wide-character cells
    Renderer.cs              ANSI diff renderer
    Theme.cs                 theme parsing, loading and colour application
    Editor.cs                modes, key handling, commands, async save state
    EditorPalette.cs         command palette actions and theme picker
    EditorLanguages.cs       language selection and token styles
    EditorScripting.cs       Lua host implementation, prompts and selects
  Util/
    ByteBuffer.cs            reusable growable byte buffer
    UnicodeUtil.cs           UTF-8, boundaries, display widths
    FileSystemUtil.cs        binary read and atomic replacement
    FileSearch.cs            project file enumeration and text search
    HexalPaths.cs            portable/fallback data paths and hexal.log
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

The core editing feature set and command behaviour match, including
asynchronous saves with a `[saving]` indicator and revision-aware dirty
tracking. The C# port additionally uses chunked storage for both original and
inserted text (avoiding a single large managed array), supports NativeAOT
publishing, and adds data-driven grammars and Lua plugins.

File **loading** remains synchronous in both implementations; only saving is
backgrounded.
