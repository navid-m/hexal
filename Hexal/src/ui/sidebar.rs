//! File explorer tree, selection and context-menu model.
//!
//! The C# editor scans the tree on a background task. The Rust editor is
//! single-threaded and this scan is deliberately synchronous: it keeps the
//! state model simple and is fast enough for normal workspaces.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::util::git;

const MIN_WIDTH: usize = 20;
const MAX_WIDTH: usize = 80;
const MIN_EDITOR_WIDTH: usize = 16;
const MAX_DEPTH: usize = 24;
const MAX_NODES: usize = 20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    NewFolder,
    NewFile,
    Rename,
    Duplicate,
    Format,
    Diff,
    DiffWith,
    Copy,
    Move,
    Delete,
}

#[derive(Clone, Copy)]
pub struct MenuItem {
    pub label: &'static str,
    pub hotkey: char,
    pub action: Action,
}

pub const FILE_MENU: &[MenuItem] = &[
    MenuItem {
        label: "Open",
        hotkey: 'O',
        action: Action::Open,
    },
    MenuItem {
        label: "New Folder",
        hotkey: 'X',
        action: Action::NewFolder,
    },
    MenuItem {
        label: "New File",
        hotkey: 'E',
        action: Action::NewFile,
    },
    MenuItem {
        label: "Rename",
        hotkey: 'R',
        action: Action::Rename,
    },
    MenuItem {
        label: "Duplicate",
        hotkey: 'D',
        action: Action::Duplicate,
    },
    MenuItem {
        label: "Format",
        hotkey: 'F',
        action: Action::Format,
    },
    MenuItem {
        label: "Diff with...",
        hotkey: 'C',
        action: Action::DiffWith,
    },
    MenuItem {
        label: "Copy to...",
        hotkey: 'T',
        action: Action::Copy,
    },
    MenuItem {
        label: "Move to...",
        hotkey: 'M',
        action: Action::Move,
    },
    MenuItem {
        label: "Delete",
        hotkey: 'd',
        action: Action::Delete,
    },
];

pub const FOLDER_MENU: &[MenuItem] = &[
    MenuItem {
        label: "Expand/Collapse",
        hotkey: 'O',
        action: Action::Open,
    },
    MenuItem {
        label: "New Folder",
        hotkey: 'X',
        action: Action::NewFolder,
    },
    MenuItem {
        label: "New File",
        hotkey: 'E',
        action: Action::NewFile,
    },
    MenuItem {
        label: "Rename",
        hotkey: 'R',
        action: Action::Rename,
    },
    MenuItem {
        label: "Duplicate",
        hotkey: 'D',
        action: Action::Duplicate,
    },
    MenuItem {
        label: "Format",
        hotkey: 'F',
        action: Action::Format,
    },
    MenuItem {
        label: "Copy to...",
        hotkey: 'T',
        action: Action::Copy,
    },
    MenuItem {
        label: "Move to...",
        hotkey: 'M',
        action: Action::Move,
    },
    MenuItem {
        label: "Delete",
        hotkey: 'd',
        action: Action::Delete,
    },
];

pub const MULTI_MENU: &[MenuItem] = &[
    MenuItem {
        label: "Duplicate",
        hotkey: 'D',
        action: Action::Duplicate,
    },
    MenuItem {
        label: "Format",
        hotkey: 'F',
        action: Action::Format,
    },
    MenuItem {
        label: "Copy to...",
        hotkey: 'T',
        action: Action::Copy,
    },
    MenuItem {
        label: "Move to...",
        hotkey: 'M',
        action: Action::Move,
    },
    MenuItem {
        label: "Delete",
        hotkey: 'd',
        action: Action::Delete,
    },
];

pub const TWO_FILE_MENU: &[MenuItem] = &[
    MenuItem {
        label: "Duplicate",
        hotkey: 'D',
        action: Action::Duplicate,
    },
    MenuItem {
        label: "Format",
        hotkey: 'F',
        action: Action::Format,
    },
    MenuItem {
        label: "Diff",
        hotkey: 'I',
        action: Action::Diff,
    },
    MenuItem {
        label: "Copy to...",
        hotkey: 'T',
        action: Action::Copy,
    },
    MenuItem {
        label: "Move to...",
        hotkey: 'M',
        action: Action::Move,
    },
    MenuItem {
        label: "Delete",
        hotkey: 'd',
        action: Action::Delete,
    },
];

#[derive(Clone)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_directory: bool,
    pub expanded: bool,
}

struct GitStatusTask {
    receiver: mpsc::Receiver<git::GitStatus>,
    generation: u64,
}

pub struct Sidebar {
    pub visible: bool,
    pub focused: bool,
    pub menu_visible: bool,
    pub menu_selected: usize,
    pub selected: usize,
    pub anchor: usize,
    pub top: usize,
    pub show_git_ignored: bool,
    pub show_modified: bool,
    pub nodes: Vec<Node>,
    pub expanded: HashSet<PathBuf>,
    pub root: PathBuf,
    pub width_override: Option<usize>,
    pub git_modified_files: HashSet<PathBuf>,
    pub git_modified_directories: HashSet<PathBuf>,
    pub git_untracked_files: HashSet<PathBuf>,
    pub git_untracked_directories: HashSet<PathBuf>,
    git_status_task: Option<GitStatusTask>,
    git_status_generation: u64,
    git_status_stale: bool,
}

impl Sidebar {
    pub fn new() -> Sidebar {
        Sidebar {
            visible: false,
            focused: false,
            menu_visible: false,
            menu_selected: 0,
            selected: 0,
            anchor: 0,
            top: 0,
            show_git_ignored: false,
            show_modified: true,
            nodes: Vec::new(),
            expanded: HashSet::new(),
            root: git::project_root(),
            width_override: None,
            git_modified_files: HashSet::new(),
            git_modified_directories: HashSet::new(),
            git_untracked_files: HashSet::new(),
            git_untracked_directories: HashSet::new(),
            git_status_task: None,
            git_status_generation: 0,
            git_status_stale: false,
        }
    }

    pub fn width(&self, columns: usize) -> usize {
        if !self.visible || columns < 36 {
            return 0;
        }
        let maximum = MIN_WIDTH.max(MAX_WIDTH.min(columns.saturating_sub(MIN_EDITOR_WIDTH)));
        let width = self
            .width_override
            .unwrap_or_else(|| (columns / 3).clamp(MIN_WIDTH, 40));
        width.clamp(MIN_WIDTH, maximum)
    }

    pub fn resize(&mut self, columns: usize, delta: isize) {
        let maximum = MIN_WIDTH.max(MAX_WIDTH.min(columns.saturating_sub(MIN_EDITOR_WIDTH)));
        let target = (self.width(columns) as isize + delta)
            .clamp(MIN_WIDTH as isize, maximum as isize) as usize;
        self.width_override = Some(target);
    }

    pub fn toggle(&mut self, select: Option<&Path>) {
        if !self.visible {
            self.visible = true;
            self.focused = true;
            self.menu_visible = false;
            self.expand_to(select);
            self.refresh(select);
        } else if !self.focused {
            self.focused = true;
            self.menu_visible = false;
            self.expand_to(select);
            self.refresh(select);
        } else {
            self.visible = false;
            self.focused = false;
            self.menu_visible = false;
        }
    }

    pub fn set_root(&mut self, root: &Path) {
        let root = git::absolute(root);
        if root == self.root {
            return;
        }
        self.root = root;
        self.expanded.clear();
        self.nodes.clear();
        self.selected = 0;
        self.anchor = 0;
        self.top = 0;
        self.git_status_generation = self.git_status_generation.wrapping_add(1);
        self.git_status_task = None;
        self.git_status_stale = false;
        self.clear_git_status();
    }

    pub fn refresh(&mut self, select_path: Option<&Path>) {
        let previous = select_path
            .map(|path| path.to_path_buf())
            .or_else(|| self.selected_node().map(|node| node.path.clone()));
        self.nodes.clear();
        let ignore = if self.show_git_ignored {
            None
        } else {
            Some(git::GitIgnore::new(&self.root))
        };
        self.scan_directory(&self.root.clone(), 0, ignore.as_ref());
        if let Some(previous) = previous {
            self.select_path(&previous);
        }
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        self.anchor = self.anchor.min(self.nodes.len().saturating_sub(1));
        self.ensure_visible();
        self.refresh_git_status();
    }

    /// Refresh only the Git marker sets. Formatting changes file contents but
    /// not the tree, so this avoids rescanning the whole workspace when a bulk
    /// format operation completes.
    ///
    /// Git status can take seconds on large or network-backed Windows
    /// checkouts, so it is always computed on a worker thread and applied by
    /// [`Sidebar::poll_git_status`].
    pub fn refresh_git_status(&mut self) {
        self.git_status_generation = self.git_status_generation.wrapping_add(1);
        if !self.show_modified {
            self.git_status_stale = false;
            self.clear_git_status();
            return;
        }
        if self.git_status_task.is_some() {
            self.git_status_stale = true;
            return;
        }
        self.start_git_status();
    }

    fn start_git_status(&mut self) {
        let root = self.root.clone();
        let generation = self.git_status_generation;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let status = git::load_git_status(&root);
            let _ = sender.send(status);
        });
        self.git_status_task = Some(GitStatusTask {
            receiver,
            generation,
        });
        self.git_status_stale = false;
    }

    pub fn poll_git_status(&mut self) {
        let Some((generation, received)) = self
            .git_status_task
            .as_ref()
            .map(|task| (task.generation, task.receiver.try_recv()))
        else {
            return;
        };
        match received {
            Ok(status) => {
                self.git_status_task = None;
                if generation == self.git_status_generation {
                    if self.show_modified {
                        self.git_modified_files = status.modified_files;
                        self.git_modified_directories = status.modified_directories;
                        self.git_untracked_files = status.untracked_files;
                        self.git_untracked_directories = status.untracked_directories;
                    } else {
                        self.clear_git_status();
                    }
                    self.git_status_stale = false;
                    return;
                }
                if self.git_status_stale {
                    self.git_status_stale = false;
                    self.refresh_git_status();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.git_status_task = None;
                if self.git_status_stale {
                    self.git_status_stale = false;
                    self.refresh_git_status();
                }
            }
        }
    }

    fn clear_git_status(&mut self) {
        self.git_modified_files.clear();
        self.git_modified_directories.clear();
        self.git_untracked_files.clear();
        self.git_untracked_directories.clear();
    }

    fn scan_directory(&mut self, directory: &Path, depth: usize, ignore: Option<&git::GitIgnore>) {
        if depth > MAX_DEPTH || self.nodes.len() >= MAX_NODES {
            return;
        }
        let ignore = ignore.map(|rules| rules.with_directory(directory));
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut directories = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if file_type.is_symlink() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.is_empty() || git::is_built_in_ignored_directory(&name) {
                    continue;
                }
                if let Some(rules) = ignore.as_ref() {
                    if rules.is_ignored(&path, true) {
                        continue;
                    }
                }
                directories.push((name, path));
            } else if file_type.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.is_empty() {
                    continue;
                }
                if let Some(rules) = ignore.as_ref() {
                    if rules.is_ignored(&path, false) {
                        continue;
                    }
                }
                files.push((name, path));
            }
        }
        directories.sort_by(|left, right| left.0.cmp(&right.0));
        files.sort_by(|left, right| left.0.cmp(&right.0));

        for (name, path) in directories {
            let expanded = self.expanded.contains(&path);
            self.nodes.push(Node {
                path: path.clone(),
                name,
                depth,
                is_directory: true,
                expanded,
            });
            if expanded {
                self.scan_directory(&path, depth + 1, ignore.as_ref());
            }
        }
        for (name, path) in files {
            self.nodes.push(Node {
                path,
                name,
                depth,
                is_directory: false,
                expanded: false,
            });
        }
    }

    pub fn expand_to(&mut self, path: Option<&Path>) {
        let Some(path) = path else {
            return;
        };
        let path = git::absolute(path);
        let mut directory = if path.is_dir() {
            Some(path.clone())
        } else {
            path.parent().map(|parent| parent.to_path_buf())
        };
        while let Some(current) = directory {
            if current == self.root || !git::is_inside(&self.root, &current) {
                break;
            }
            self.expanded.insert(current.clone());
            directory = current.parent().map(|parent| parent.to_path_buf());
        }
    }

    pub fn select_path(&mut self, path: &Path) {
        let path = git::absolute(path);
        if let Some(index) = self.nodes.iter().position(|node| node.path == path) {
            self.selected = index;
            self.anchor = index;
            self.ensure_visible();
        }
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.nodes.get(self.selected)
    }

    pub fn selection_start(&self) -> usize {
        self.selected.min(self.anchor)
    }

    pub fn selection_end(&self) -> usize {
        self.selected.max(self.anchor)
    }

    pub fn selection_count(&self) -> usize {
        if self.nodes.is_empty() {
            0
        } else {
            self.selected.abs_diff(self.anchor) + 1
        }
    }

    pub fn selected_nodes(&self) -> Vec<&Node> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let start = self.selection_start().min(self.nodes.len() - 1);
        let end = self.selection_end().min(self.nodes.len() - 1);
        self.nodes[start..=end].iter().collect()
    }

    /// Remove children that are already covered by a selected parent folder.
    pub fn effective_selection(&self) -> Vec<&Node> {
        let selected = self.selected_nodes();
        if selected.len() < 2 {
            return selected;
        }
        let directories: Vec<&Path> = selected
            .iter()
            .filter(|node| node.is_directory)
            .map(|node| node.path.as_path())
            .collect();
        if directories.is_empty() {
            return selected;
        }
        selected
            .into_iter()
            .filter(|node| {
                !directories.iter().any(|directory| {
                    node.path != *directory && git::is_inside(directory, &node.path)
                })
            })
            .collect()
    }

    pub fn menu_items(&self) -> &'static [MenuItem] {
        if self.selection_count() > 1 {
            if self.selection_count() == 2 {
                let selected = self.selected_nodes();
                if selected.len() == 2 && !selected[0].is_directory && !selected[1].is_directory {
                    return TWO_FILE_MENU;
                }
            }
            return MULTI_MENU;
        }
        match self.selected_node() {
            Some(node) if node.is_directory => FOLDER_MENU,
            _ => FILE_MENU,
        }
    }

    pub fn menu_hotkey(&self, character: char) -> Option<usize> {
        let items = self.menu_items();
        if let Some(index) = items.iter().position(|item| item.hotkey == character) {
            return Some(index);
        }
        let upper = character.to_ascii_uppercase();
        items.iter().position(|item| item.hotkey == upper)
    }

    pub fn move_selection(&mut self, delta: isize, wrap: bool, extend: bool, rows: usize) {
        if self.nodes.is_empty() {
            return;
        }
        let count = self.nodes.len() as isize;
        let mut index = self.selected as isize + delta;
        if wrap {
            index = index.rem_euclid(count);
        } else {
            index = index.clamp(0, count - 1);
        }
        self.set_selection(index as usize, extend);
        self.ensure_visible_rows(rows);
    }

    pub fn set_selection(&mut self, index: usize, extend: bool) {
        if self.nodes.is_empty() {
            self.selected = 0;
            self.anchor = 0;
            return;
        }
        self.selected = index.min(self.nodes.len() - 1);
        if !extend {
            self.anchor = self.selected;
        }
        self.ensure_visible();
    }

    pub fn ensure_visible(&mut self) {
        self.ensure_visible_rows(self.top + 1);
    }

    pub fn ensure_visible_rows(&mut self, _rows: usize) {
        // The renderer calls `update_top` with the real row count; keep the
        // selection inside a conservative window here.
        if self.selected < self.top {
            self.top = self.selected;
        }
        if self.top > self.selected {
            self.top = self.selected;
        }
    }

    pub fn update_top(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + rows {
            self.top = self.selected + 1 - rows;
        }
        self.top = self.top.min(self.nodes.len().saturating_sub(rows));
    }

    pub fn collapse_or_select_parent(&mut self) -> bool {
        let Some(node) = self.selected_node() else {
            return false;
        };
        let node = node.clone();
        if node.is_directory && node.expanded {
            self.expanded.remove(&node.path);
            self.refresh(Some(&node.path));
            return true;
        }
        if let Some(parent) = node.path.parent() {
            if parent != self.root && git::is_inside(&self.root, parent) {
                self.select_path(parent);
            }
        }
        false
    }

    pub fn expand_or_select_child(&mut self) -> bool {
        let Some(node) = self.selected_node() else {
            return false;
        };
        let node = node.clone();
        if !node.is_directory {
            return false;
        }
        if !node.expanded {
            self.expanded.insert(node.path.clone());
            self.refresh(Some(&node.path));
            return true;
        }
        if let Some(next) = self.nodes.get(self.selected + 1) {
            if next.depth > node.depth {
                self.set_selection(self.selected + 1, false);
            }
        }
        false
    }

    pub fn toggle_directory(&mut self) -> bool {
        let Some(node) = self.selected_node() else {
            return false;
        };
        let node = node.clone();
        if !node.is_directory {
            return false;
        }
        if !self.expanded.remove(&node.path) {
            self.expanded.insert(node.path.clone());
        }
        self.refresh(Some(&node.path));
        true
    }

    pub fn remove_expanded_under(&mut self, path: &Path) {
        self.expanded
            .retain(|expanded| expanded != path && !git::is_inside(path, expanded));
    }

    pub fn update_expanded_after_rename(&mut self, old: &Path, new: &Path) {
        let mut updated = HashSet::new();
        for expanded in &self.expanded {
            if expanded == old {
                updated.insert(new.to_path_buf());
            } else if git::is_inside(old, expanded) {
                if let Ok(relative) = expanded.strip_prefix(old) {
                    updated.insert(new.join(relative));
                }
            } else {
                updated.insert(expanded.clone());
            }
        }
        self.expanded = updated;
    }

    pub fn is_git_modified(&self, node: &Node) -> bool {
        if node.is_directory {
            self.git_modified_directories.contains(&node.path)
        } else {
            self.git_modified_files.contains(&node.path)
        }
    }

    pub fn is_git_untracked(&self, node: &Node) -> bool {
        if node.is_directory {
            self.git_untracked_directories.contains(&node.path)
        } else {
            self.git_untracked_files.contains(&node.path)
        }
    }
}

pub fn folder_choices(nodes: &[&Node], show_git_ignored: bool, root: &Path) -> Vec<PathBuf> {
    let mut result = vec![root.to_path_buf()];
    let ignore = if show_git_ignored {
        None
    } else {
        Some(git::GitIgnore::new(root))
    };
    add_folders(root, nodes, &mut result, ignore.as_ref());
    result
}

fn add_folders(
    directory: &Path,
    sources: &[&Node],
    output: &mut Vec<PathBuf>,
    ignore: Option<&git::GitIgnore>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let ignore = ignore.map(|rules| rules.with_directory(directory));
    let mut directories: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            file_type.is_dir().then_some(entry.path())
        })
        .collect();
    directories.sort();
    for child in directories {
        let name = git::file_name(&child);
        if name.is_empty() {
            continue;
        }
        if let Some(rules) = ignore.as_ref() {
            if git::is_built_in_ignored_directory(&name) || rules.is_ignored(&child, true) {
                continue;
            }
        }
        if sources.iter().any(|node| {
            node.is_directory && (child == node.path || git::is_inside(&node.path, &child))
        }) {
            continue;
        }
        output.push(child.clone());
        add_folders(&child, sources, output, ignore.as_ref());
    }
}

pub fn folder_labels(folders: &[PathBuf], root: &Path) -> Vec<String> {
    folders
        .iter()
        .map(|folder| {
            let relative = git::display_relative(root, folder);
            if relative == "./" {
                "./ (project root)".into()
            } else {
                format!("{relative}/")
            }
        })
        .collect()
}
