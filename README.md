## Hexal

A general purpose modal text editor with powerful Lua scripting capabilities builtin. Documentation is a WIP.

Links of note:

- [Documentation](https://hexal.pages.dev)
- [Community Plugins](https://github.com/navid-m/hexal-plugins)

Fundamentals of the editor, along with useful shortcuts can be learned by typing the Ex command `:help` into the editor whilst in NORMAL mode.

---

The primary design goal of the editor is performance on large files + searching through large sets of large files.

Currently, Hexal is fitted with some less-than-stock features OOTB:

- Lua scripting (via `hexal` namespace) for plugins.
- Regex find/replace across files.
- Git difflines in gutters.
- File explorer sidebar, with bulk formatting capabilities (run rustfmt on an entire directory and its subdirectories, for example).
- Full LSP integration and support for 114 languages.
- Debugger integration through [DAP](https://microsoft.github.io/debug-adapter-protocol).

---

&copy; Navid Momtahen - 2026

Licensed under GPL-3.0-only.
