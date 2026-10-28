//! Editor submodule.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use super::*;

/// A `Send` snapshot of the language rules needed by a batch format worker.
/// The live registry contains `Rc<Language>` values and cannot cross threads,
/// so the worker gets an immutable, plain-data copy instead.
struct BulkFormatLanguage {
    name: String,
    formatter: Option<FormatterConfig>,
    fallback_formatter: Option<FormatterConfig>,
}

struct BulkFormatResolver {
    by_filename: HashMap<String, usize>,
    ordered_extensions: Vec<(String, usize)>,
    languages: Vec<BulkFormatLanguage>,
}

impl BulkFormatResolver {
    fn new(registry: &Registry) -> BulkFormatResolver {
        let mut by_filename = HashMap::new();
        let mut by_extension = HashMap::new();
        let mut ordered_extensions = Vec::new();
        let mut languages = Vec::new();

        for (index, language) in registry.languages().iter().enumerate() {
            for file_name in &language.file_names {
                by_filename
                    .entry(file_name.to_ascii_lowercase())
                    .or_insert(index);
            }
            for extension in &language.extensions {
                let key = extension.to_ascii_lowercase();
                if by_extension.insert(key.clone(), index).is_none() {
                    ordered_extensions.push((key, index));
                }
            }
            languages.push(BulkFormatLanguage {
                name: language.name.clone(),
                formatter: language.formatter.clone(),
                fallback_formatter: language.fallback_formatter.clone(),
            });
        }

        // Compound extensions such as `.blade.php` must win over `.php`.
        ordered_extensions.sort_by(|left, right| right.0.len().cmp(&left.0.len()));
        BulkFormatResolver {
            by_filename,
            ordered_extensions,
            languages,
        }
    }

    fn detect(&self, path: &Path) -> Option<usize> {
        let file_name = path.file_name()?.to_string_lossy();
        if file_name.is_empty() {
            return None;
        }
        let lower_name = file_name.to_ascii_lowercase();
        if let Some(index) = self.by_filename.get(&lower_name) {
            return Some(*index);
        }
        self.ordered_extensions
            .iter()
            .find(|(extension, _)| lower_name.ends_with(extension))
            .map(|(_, index)| *index)
    }
}

fn command_available(cache: &mut HashMap<String, bool>, command: &str) -> bool {
    if let Some(available) = cache.get(command) {
        return *available;
    }
    let available = crate::util::git::command_exists(command);
    cache.insert(command.to_string(), available);
    available
}

/// The reference editor formats with `Parallel.ForEach` at 2..8 workers.
fn batch_format_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(2, 8)
}

fn lock_format_result(
    result: &std::sync::Mutex<SidebarFormatResult>,
) -> std::sync::MutexGuard<'_, SidebarFormatResult> {
    result
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Editor {
    // ---------------------------------------------------------------------
    // Sidebar
    // ---------------------------------------------------------------------

    pub(super) fn editor_left(&self) -> usize {
        self.editor_left_override
            .unwrap_or_else(|| self.sidebar.width(self.cols))
    }

    pub(super) fn toggle_sidebar(&mut self) {
        let path = self.doc.path.clone().map(PathBuf::from);
        self.sidebar.toggle(path.as_deref());
        self.force_full_redraw = true;
        if self.sidebar.visible {
            self.set_message("Sidebar focused (Ctrl+B to close)");
        } else {
            self.clear_message();
        }
    }

    pub(super) fn handle_sidebar(&mut self, event: Event) {
        if self.sidebar.menu_visible {
            self.handle_sidebar_menu(event);
            return;
        }
        if event.key == Key::Char && event.ctrl {
            match event.ch {
                'b' => {
                    self.sidebar.visible = false;
                    self.sidebar.focused = false;
                    self.force_full_redraw = true;
                }
                _ => {}
            }
            return;
        }
        if event.key == Key::Left && event.ctrl {
            self.sidebar.resize(self.cols, -2);
            self.force_full_redraw = true;
            self.set_message(&format!("Sidebar width: {}", self.sidebar.width(self.cols)));
            return;
        }
        if event.key == Key::Right && event.ctrl {
            self.sidebar.resize(self.cols, 2);
            self.force_full_redraw = true;
            self.set_message(&format!("Sidebar width: {}", self.sidebar.width(self.cols)));
            return;
        }

        let rows = self.text_rows();
        match event.key {
            Key::Escape => {
                if self.sidebar.selection_count() > 1 {
                    self.sidebar.anchor = self.sidebar.selected;
                } else {
                    self.sidebar.focused = false;
                    self.clear_message();
                }
            }
            Key::Tab => self.sidebar.move_selection(1, true, false, rows),
            Key::BackTab => self.sidebar.move_selection(-1, true, false, rows),
            Key::Up => self.sidebar.move_selection(-1, false, event.shift, rows),
            Key::Down => self.sidebar.move_selection(1, false, event.shift, rows),
            Key::PageUp => self
                .sidebar
                .move_selection(-(rows as isize), false, event.shift, rows),
            Key::PageDown => self
                .sidebar
                .move_selection(rows as isize, false, event.shift, rows),
            Key::Home => self.sidebar.set_selection(0, event.shift),
            Key::End => {
                let last = self.sidebar.nodes.len().saturating_sub(1);
                self.sidebar.set_selection(last, event.shift);
            }
            Key::Left => {
                self.sidebar.collapse_or_select_parent();
            }
            Key::Right => {
                self.sidebar.expand_or_select_child();
            }
            Key::Enter => self.activate_sidebar_node(),
            Key::Char if !event.ctrl && !event.alt => match event.ch {
                'c' | 'C' => self.open_sidebar_context_menu(),
                ' ' => {
                    self.sidebar.focused = false;
                    self.open_space_menu();
                }
                _ => {}
            },
            _ => {}
        }
        self.sidebar.update_top(rows);
        self.force_full_redraw = true;
    }

    pub(super) fn activate_sidebar_node(&mut self) {
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        if node.is_directory {
            self.sidebar.toggle_directory();
            return;
        }
        self.sidebar.menu_visible = false;
        self.sidebar.focused = self.config.keep_sidebar_focus_on_open && self.sidebar.visible;
        self.request_open(&node.path.to_string_lossy(), 0, 0, false);
    }

    pub(super) fn open_sidebar_context_menu(&mut self) {
        if self.sidebar.selected_node().is_none() {
            return;
        }
        self.sidebar.menu_visible = true;
        self.sidebar.menu_selected = 0;
        self.sidebar.focused = true;
        self.clear_message();
    }

    pub(super) fn handle_sidebar_menu(&mut self, event: Event) {
        let items = self.sidebar.menu_items();
        match event.key {
            Key::Escape => {
                self.sidebar.menu_visible = false;
                return;
            }
            Key::Tab => {
                self.sidebar.menu_selected = (self.sidebar.menu_selected + 1) % items.len();
                return;
            }
            Key::BackTab => {
                self.sidebar.menu_selected =
                    (self.sidebar.menu_selected + items.len() - 1) % items.len();
                return;
            }
            Key::Up => {
                self.sidebar.menu_selected = self.sidebar.menu_selected.saturating_sub(1);
                return;
            }
            Key::Down => {
                self.sidebar.menu_selected =
                    (self.sidebar.menu_selected + 1).min(items.len().saturating_sub(1));
                return;
            }
            Key::Enter => {
                self.activate_sidebar_menu_item();
                return;
            }
            _ => {}
        }
        if event.key != Key::Char || event.ctrl || event.alt {
            return;
        }
        match event.ch {
            ' ' => {
                self.sidebar.menu_visible = false;
                self.sidebar.focused = false;
                self.open_space_menu();
            }
            character => {
                if let Some(index) = self.sidebar.menu_hotkey(character) {
                    self.sidebar.menu_selected = index;
                    self.activate_sidebar_menu_item();
                } else if matches!(character, 'c' | 'C') {
                    self.sidebar.menu_visible = false;
                }
            }
        }
    }

    pub(super) fn activate_sidebar_menu_item(&mut self) {
        let items = self.sidebar.menu_items();
        if self.sidebar.menu_selected >= items.len() {
            self.sidebar.menu_visible = false;
            return;
        }
        let action = items[self.sidebar.menu_selected].action;
        self.sidebar.menu_visible = false;
        self.sidebar.focused = true;
        let nodes = self
            .sidebar
            .effective_selection()
            .into_iter()
            .map(|node| (node.path.clone(), node.is_directory))
            .collect::<Vec<_>>();
        self.run_sidebar_action(action, nodes);
    }

    pub(super) fn run_sidebar_action(
        &mut self,
        action: SidebarAction,
        nodes: Vec<(PathBuf, bool)>,
    ) {
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        match action {
            SidebarAction::Open => {
                if node.is_directory {
                    self.sidebar.toggle_directory();
                } else {
                    self.sidebar.focused =
                        self.config.keep_sidebar_focus_on_open && self.sidebar.visible;
                    self.request_open(&node.path.to_string_lossy(), 0, 0, false);
                }
            }
            SidebarAction::NewFolder => {
                if nodes.len() == 1 {
                    let parent = self.sidebar_parent_for_new(&nodes[0]);
                    self.start_sidebar_prompt(
                        "New folder name: ",
                        "",
                        SidebarPromptKind::NewFolder { parent },
                    );
                }
            }
            SidebarAction::NewFile => {
                if nodes.len() == 1 {
                    let parent = self.sidebar_parent_for_new(&nodes[0]);
                    self.start_sidebar_prompt(
                        "New file name: ",
                        "",
                        SidebarPromptKind::NewFile { parent },
                    );
                }
            }
            SidebarAction::Rename => {
                if nodes.len() == 1 {
                    let (path, is_directory) = nodes[0].clone();
                    let name = git::file_name(&path);
                    self.start_sidebar_prompt(
                        &format!("Rename {name}: "),
                        &name,
                        SidebarPromptKind::Rename {
                            old: path,
                            is_directory,
                        },
                    );
                }
            }
            SidebarAction::Duplicate => self.duplicate_sidebar_nodes(&nodes),
            SidebarAction::Format => self.format_sidebar_nodes(&nodes),
            SidebarAction::Diff => self.begin_sidebar_diff(&nodes),
            SidebarAction::DiffWith => {
                if !node.is_directory && nodes.len() == 1 {
                    self.diff_with_source = Some(node.path.clone());
                    self.open_picker(PickerKind::DiffWith);
                }
            }
            SidebarAction::Copy => {
                if nodes.len() == 1 {
                    let (source, is_directory) = nodes[0].clone();
                    let initial = git::unique_copy_path(&source, is_directory);
                    self.start_sidebar_prompt(
                        "Copy to: ",
                        &initial.to_string_lossy(),
                        SidebarPromptKind::Copy {
                            source,
                            is_directory,
                        },
                    );
                } else {
                    self.open_folder_picker(nodes, false);
                }
            }
            SidebarAction::Move => self.open_folder_picker(nodes, true),
            SidebarAction::Delete => self.request_delete_sidebar_nodes(nodes),
        }
    }

    pub(super) fn sidebar_parent_for_new(&self, node: &(PathBuf, bool)) -> PathBuf {
        if node.1 {
            node.0.clone()
        } else {
            node.0
                .parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| self.sidebar.root.clone())
        }
    }

    pub(super) fn start_sidebar_prompt(
        &mut self,
        label: &str,
        initial: &str,
        kind: SidebarPromptKind,
    ) {
        self.sidebar.focused = self.sidebar.visible;
        self.sidebar_prompt = Some(SidebarPrompt {
            label: label.to_string(),
            text: initial.to_string(),
            cursor: initial.len(),
            kind,
        });
        self.clear_message();
    }

    pub(super) fn handle_sidebar_prompt(&mut self, event: Event) {
        let Some(prompt) = self.sidebar_prompt.as_mut() else {
            return;
        };
        match event.key {
            Key::Escape => {
                let kind = prompt.kind.clone();
                self.sidebar_prompt = None;
                self.sidebar.focused = self.sidebar.visible;
                self.cancel_sidebar_prompt(kind);
            }
            Key::Enter => {
                let kind = prompt.kind.clone();
                let text = prompt.text.trim().to_string();
                self.sidebar_prompt = None;
                self.sidebar.focused = self.sidebar.visible;
                self.complete_sidebar_prompt(kind, &text);
            }
            Key::Backspace => {
                if prompt.cursor > 0 {
                    let previous = util::previous_boundary(prompt.text.as_bytes(), prompt.cursor);
                    prompt.text.replace_range(previous..prompt.cursor, "");
                    prompt.cursor = previous;
                }
            }
            Key::Delete => {
                if prompt.cursor < prompt.text.len() {
                    let next = util::next_boundary(prompt.text.as_bytes(), prompt.cursor);
                    prompt.text.replace_range(prompt.cursor..next, "");
                }
            }
            Key::Left => {
                prompt.cursor = util::previous_boundary(prompt.text.as_bytes(), prompt.cursor);
            }
            Key::Right => {
                prompt.cursor = util::next_boundary(prompt.text.as_bytes(), prompt.cursor);
            }
            Key::Home => prompt.cursor = 0,
            Key::End => prompt.cursor = prompt.text.len(),
            Key::Char if !event.ctrl && !event.alt && event.ch != '\0' => {
                prompt.text.insert(prompt.cursor, event.ch);
                prompt.cursor += event.ch.len_utf8();
            }
            _ => {}
        }
    }

    pub(super) fn cancel_sidebar_prompt(&mut self, kind: SidebarPromptKind) {
        match kind {
            SidebarPromptKind::NewFolder { .. } | SidebarPromptKind::NewFile { .. } => {
                self.sidebar.refresh(None);
                self.set_message("Cancelled.");
            }
            SidebarPromptKind::Rename { old, .. } => {
                self.sidebar.refresh(Some(&old));
                self.set_message("Rename cancelled.");
            }
            SidebarPromptKind::Copy { source, .. } => {
                self.sidebar.refresh(Some(&source));
                self.set_message("Copy cancelled.");
            }
        }
    }

    pub(super) fn complete_sidebar_prompt(&mut self, kind: SidebarPromptKind, text: &str) {
        match kind {
            SidebarPromptKind::NewFolder { parent } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid folder name.");
                    self.sidebar.refresh(None);
                    return;
                }
                let target = parent.join(text);
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(None);
                    return;
                }
                match std::fs::create_dir(&target) {
                    Ok(()) => {
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Created folder {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Create folder failed: {error}"));
                        self.sidebar.refresh(None);
                    }
                }
            }
            SidebarPromptKind::NewFile { parent } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid file name.");
                    self.sidebar.refresh(None);
                    return;
                }
                let target = parent.join(text);
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(None);
                    return;
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                {
                    Ok(_) => {
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Created file {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Create file failed: {error}"));
                        self.sidebar.refresh(None);
                    }
                }
            }
            SidebarPromptKind::Rename { old, is_directory } => {
                if text.is_empty() || !git::valid_name(text) {
                    self.set_error("Invalid file or folder name.");
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                let parent = old
                    .parent()
                    .map(|parent| parent.to_path_buf())
                    .unwrap_or_else(|| self.sidebar.root.clone());
                let target = parent.join(text);
                if target == old {
                    self.set_message("Name unchanged.");
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                if target.exists() {
                    self.set_error(&format!("'{text}' already exists."));
                    self.sidebar.refresh(Some(&old));
                    return;
                }
                let result = if is_directory {
                    std::fs::rename(&old, &target)
                } else {
                    std::fs::rename(&old, &target)
                };
                match result {
                    Ok(()) => {
                        if is_directory {
                            self.sidebar.update_expanded_after_rename(&old, &target);
                        }
                        self.rename_open_document(&old, &target, is_directory);
                        self.sidebar.expand_to(Some(&target));
                        self.sidebar.refresh(Some(&target));
                        self.set_message(&format!("Renamed to {text}"));
                    }
                    Err(error) => {
                        self.set_error(&format!("Rename failed: {error}"));
                        self.sidebar.refresh(Some(&old));
                    }
                }
            }
            SidebarPromptKind::Copy {
                source,
                is_directory,
            } => {
                if text.is_empty() {
                    self.sidebar.refresh(Some(&source));
                    self.set_message("Copy cancelled.");
                    return;
                }
                let mut destination = PathBuf::from(text);
                if !destination.is_absolute() {
                    destination = self.sidebar.root.join(destination);
                }
                destination = git::absolute(&destination);
                if destination.is_dir() {
                    destination = destination.join(git::file_name(&source));
                }
                match git::copy_path(&source, &destination, is_directory) {
                    Ok(()) => {
                        self.sidebar.expand_to(Some(&destination));
                        self.sidebar.refresh(Some(&destination));
                        self.set_message(&format!("Copied to {}", git::file_name(&destination)));
                    }
                    Err(error) => {
                        self.set_error(&error);
                        self.sidebar.refresh(Some(&source));
                    }
                }
            }
        }
    }

    pub(super) fn rename_open_document(&mut self, old: &Path, new: &Path, is_directory: bool) {
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        let document_path = git::absolute(Path::new(&path));
        let old = git::absolute(old);
        let new = git::absolute(new);
        let updated = if document_path == old {
            Some(new.clone())
        } else if is_directory && git::is_inside(&old, &document_path) {
            document_path
                .strip_prefix(&old)
                .ok()
                .map(|relative| new.join(relative))
        } else {
            None
        };
        if let Some(updated) = updated {
            self.doc.path = Some(updated.to_string_lossy().into_owned());
        }
    }

    pub(super) fn duplicate_sidebar_nodes(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.is_empty() {
            return;
        }
        let mut duplicated = 0usize;
        let mut first_error = None;
        let mut last_target = None;
        for (path, is_directory) in nodes {
            let target = git::unique_copy_path(path, *is_directory);
            match git::copy_path(path, &target, *is_directory) {
                Ok(()) => {
                    duplicated += 1;
                    last_target = Some(target);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(target) = last_target.as_ref() {
            self.sidebar.expand_to(Some(target));
            self.sidebar.refresh(Some(target));
        } else {
            self.sidebar.refresh(None);
        }
        if let Some(error) = first_error {
            self.set_error(&if duplicated == 0 {
                error
            } else {
                format!("Duplicated {duplicated} of {} items; {error}", nodes.len())
            });
        } else {
            let target_name = last_target
                .as_deref()
                .map(git::file_name)
                .unwrap_or_default();
            self.set_message(&if duplicated == 1 {
                format!("Duplicated as {target_name}")
            } else {
                format!("Duplicated {duplicated} items")
            });
        }
    }

    pub(super) fn format_sidebar_nodes(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.is_empty() {
            return;
        }
        if self.sidebar_format.is_some() {
            self.set_message("Batch formatting is already in progress.");
            return;
        }
        if nodes.len() == 1 {
            if let Some(path) = &self.doc.path {
                if git::same_path(Path::new(path), &nodes[0].0) && !nodes[0].1 {
                    self.format_current_document(true);
                    return;
                }
            }
        }

        let resolver = BulkFormatResolver::new(&self.languages);
        let root = self.sidebar.root.clone();
        let show_git_ignored = self.sidebar.show_git_ignored;
        let targets = nodes.to_vec();
        let open_path = self
            .doc
            .path
            .as_ref()
            .map(|path| git::absolute(Path::new(path)));
        let progress = Arc::new(SidebarFormatProgress {
            total: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
        });
        let thread_progress = progress.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            // Discovery, language lookup and formatter resolution all happen
            // off the UI thread. Command availability is cached so a large
            // tree does not perform one PATH scan per file.
            let mut files = Vec::new();
            for (path, is_directory) in &targets {
                if *is_directory {
                    collect_files(path, &root, show_git_ignored, &mut files);
                } else if path.is_file() {
                    files.push(path.clone());
                }
            }
            files.sort();
            files.dedup();

            let mut command_cache = HashMap::new();
            let mut jobs = Vec::new();
            let mut skipped = 0usize;
            for file in files {
                let name = file.to_string_lossy();
                let Some(language_index) = resolver.detect(&file) else {
                    crate::util::log::write(&format!("format: no language for {name}"));
                    skipped += 1;
                    continue;
                };
                let language = &resolver.languages[language_index];
                let Some(primary) = &language.formatter else {
                    crate::util::log::write(&format!(
                        "format: no formatter configured for {name} ({})",
                        language.name
                    ));
                    skipped += 1;
                    continue;
                };
                let formatter = if command_available(&mut command_cache, &primary.command) {
                    primary.clone()
                } else if let Some(fallback) = &language.fallback_formatter {
                    if command_available(&mut command_cache, &fallback.command) {
                        fallback.clone()
                    } else {
                        crate::util::log::write(&format!(
                            "format: formatter '{}' not found for {name}",
                            primary.command
                        ));
                        skipped += 1;
                        continue;
                    }
                } else {
                    crate::util::log::write(&format!(
                        "format: formatter '{}' not found for {name}",
                        primary.command
                    ));
                    skipped += 1;
                    continue;
                };
                jobs.push((file, formatter));
            }

            let total = jobs.len();
            thread_progress.total.store(total, Ordering::Relaxed);
            let result = std::sync::Mutex::new(SidebarFormatResult {
                formatted: 0,
                changed: 0,
                failed: 0,
                skipped,
                skipped_open: 0,
                first_error: None,
            });
            let next = AtomicUsize::new(0);
            let worker_count = batch_format_parallelism().min(total);
            if worker_count > 0 {
                std::thread::scope(|scope| {
                    for _ in 0..worker_count {
                        scope.spawn(|| loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some((file, formatter)) = jobs.get(index) else {
                                break;
                            };
                            if let Some(open) = &open_path {
                                if git::same_path(open, file) {
                                    lock_format_result(&result).skipped_open += 1;
                                    thread_progress.completed.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                            }

                            let mut formatted = false;
                            let mut changed = false;
                            let mut failure = None;
                            match std::fs::read(file) {
                                Ok(input) => {
                                    match format::run(formatter, &input, Some(file.as_path())) {
                                        Ok(output) => {
                                            formatted = true;
                                            let output = if git::has_utf8_bom(&input)
                                                && !git::has_utf8_bom(&output)
                                            {
                                                let mut with_bom =
                                                    Vec::with_capacity(output.len() + 3);
                                                with_bom.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                                                with_bom.extend_from_slice(&output);
                                                with_bom
                                            } else {
                                                output
                                            };
                                            if output != input {
                                                match git::atomic_write(file, &output) {
                                                    Ok(()) => changed = true,
                                                    Err(error) => {
                                                        failure = Some(format!(
                                                            "{}: {error}",
                                                            git::file_name(file)
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            failure =
                                                Some(format!("{}: {error}", git::file_name(file)));
                                        }
                                    }
                                }
                                Err(error) => {
                                    failure = Some(format!("{}: {error}", git::file_name(file)));
                                }
                            }

                            let mut metrics = lock_format_result(&result);
                            if formatted {
                                metrics.formatted += 1;
                            }
                            if changed {
                                metrics.changed += 1;
                            }
                            if let Some(error) = failure {
                                metrics.failed += 1;
                                if metrics.first_error.is_none() {
                                    metrics.first_error = Some(error);
                                }
                            }
                            drop(metrics);
                            thread_progress.completed.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                });
            }
            let result = result
                .into_inner()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = sender.send(result);
        });

        self.sidebar_format = Some(SidebarFormatOperation {
            receiver,
            progress,
            last_total: usize::MAX,
            last_completed: usize::MAX,
        });
        let message = if nodes.len() == 1 {
            format!(
                "Formatting {}...",
                nodes[0]
                    .0
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| nodes[0].0.to_string_lossy().into_owned())
            )
        } else {
            format!("Formatting {} selected items...", nodes.len())
        };
        self.set_message(&message);
        self.force_full_redraw = true;
    }

    pub(super) fn poll_sidebar_format(&mut self) {
        let Some(mut operation) = self.sidebar_format.take() else {
            return;
        };

        match operation.receiver.try_recv() {
            Ok(result) => {
                if result.changed > 0 {
                    self.sidebar.refresh_git_status();
                }
                if result.failed > 0 {
                    let detail = result
                        .first_error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default();
                    self.set_error(&format!(
                        "Formatted {} file(s), {} failed{detail}",
                        result.formatted, result.failed
                    ));
                } else if result.formatted == 0 {
                    if result.skipped_open > 0 {
                        self.set_message("Skipped the open buffer.");
                    } else {
                        self.set_message("No files were formatted.");
                    }
                } else {
                    let mut summary = format!("Formatted {} file(s)", result.formatted);
                    if result.changed > 0 {
                        summary.push_str(&format!(", {} changed", result.changed));
                    }
                    if result.skipped > 0 {
                        summary.push_str(&format!(", {} skipped", result.skipped));
                    }
                    if result.skipped_open > 0 {
                        summary.push_str(", open buffer skipped");
                    }
                    self.set_message(&summary);
                }
                self.force_full_redraw = true;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.sidebar.refresh_git_status();
                self.set_error("Format failed: formatting task stopped unexpectedly");
                self.force_full_redraw = true;
                return;
            }
        }

        let total = operation.progress.total();
        let completed = operation.progress.completed().min(total);
        if total != operation.last_total || completed != operation.last_completed {
            operation.last_total = total;
            operation.last_completed = completed;
            let message = if total > 0 {
                format!("Formatting ({completed}/{total})...")
            } else {
                "Formatting...".to_string()
            };
            self.set_message(&message);
            self.force_full_redraw = true;
        }
        self.sidebar_format = Some(operation);
    }

    pub(super) fn format_current_document(&mut self, save_after: bool) {
        let language = self.active_language();
        let Some(language) = language else {
            self.set_error("No language detected for the current buffer.");
            return;
        };
        let Some(formatter) = language.resolve_formatter().cloned() else {
            self.set_error("No formatter configured for this language.");
            return;
        };
        let input = self.doc.all_text();
        let path = self.doc.path.clone().map(PathBuf::from);
        let selection_only = !save_after && self.mode == Mode::Select && {
            let (start, end) = self.selection_range();
            end > start
        };
        if selection_only {
            self.format_selection(&formatter, path.as_deref(), save_after);
            return;
        }
        let snapshot = self.capture_cursor_snapshot();
        match format::run(&formatter, &input, path.as_deref()) {
            Ok(output) => {
                if output != input {
                    self.doc.replace_all_owned(output);
                    self.restore_cursor_snapshot(snapshot);
                    self.doc.break_undo_group();
                    self.lsp_dirty = self.lsp.is_running();
                    self.lsp_dirty_since = std::time::Instant::now();
                }
                if save_after && path.is_some() {
                    self.save_document_skip_format(None);
                }
                self.set_message("Formatted buffer.");
            }
            Err(error) => {
                if save_after && path.is_some() {
                    self.save_document_skip_format(None);
                } else {
                    self.set_error(&error);
                }
            }
        }
    }

    pub(super) fn begin_sidebar_diff(&mut self, nodes: &[(PathBuf, bool)]) {
        if nodes.len() != 2 || nodes[0].1 || nodes[1].1 {
            self.set_error("Diff requires exactly two files.");
            return;
        }
        let first = std::fs::read(&nodes[0].0);
        let second = std::fs::read(&nodes[1].0);
        let (Ok(first), Ok(second)) = (first, second) else {
            self.set_error("Cannot read file for diff.");
            return;
        };
        let old_label = git::display_relative(&self.sidebar.root, &nodes[0].0);
        let new_label = git::display_relative(&self.sidebar.root, &nodes[1].0);
        let old_text = String::from_utf8_lossy(git::trim_utf8_bom(&first)).into_owned();
        let new_text = String::from_utf8_lossy(git::trim_utf8_bom(&second)).into_owned();
        let diff = diff::create(&old_label, &old_text, &new_label, &new_text);
        let summary = format!("{old_label} vs {new_label}");
        if self.doc.is_dirty() {
            self.pending_sidebar_diff = Some((diff, summary));
            self.pending = Pending::SidebarDiff;
            self.clear_message();
            return;
        }
        self.show_sidebar_diff(&diff, &summary);
    }

    pub(super) fn show_sidebar_diff(&mut self, diff: &str, summary: &str) {
        self.sidebar.menu_visible = false;
        self.sidebar.focused = false;
        self.assign_active_document(Document::from_bytes(None, diff.as_bytes().to_vec()));
        self.language_override = Some("diff".into());
        self.refresh_language();
        self.mode = Mode::Normal;
        self.cursor = 0;
        self.anchor = 0;
        self.top_line = 0;
        self.left_column = 0;
        self.selection_linewise = false;
        self.preferred_column = None;
        self.find = FindState::new();
        self.doc.break_undo_group();
        self.last_revision = self.doc.revision();
        self.snap_cursor();
        self.restart_lsp_for_buffer();
        self.set_message(&format!("Diff: {summary}"));
    }

    pub(super) fn request_delete_sidebar_nodes(&mut self, nodes: Vec<(PathBuf, bool)>) {
        if nodes.is_empty() {
            return;
        }
        if let Some(path) = &self.doc.path {
            let document = git::absolute(Path::new(path));
            for (selected, is_directory) in &nodes {
                let selected = git::absolute(selected);
                if document == selected || (*is_directory && git::is_inside(&selected, &document)) {
                    self.set_error("Cannot delete the open file or its parent folder.");
                    return;
                }
            }
        }
        self.pending_sidebar_deletes = nodes;
        self.pending = Pending::SidebarDelete;
        self.clear_message();
    }

    pub(super) fn delete_sidebar_paths(&mut self) {
        let items = std::mem::take(&mut self.pending_sidebar_deletes);
        let mut deleted = 0usize;
        let mut first_error = None;
        for (path, is_directory) in items {
            let result = if is_directory {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    deleted += 1;
                    if is_directory {
                        self.sidebar.remove_expanded_under(&path);
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(format!(
                            "Delete failed for '{}': {error}",
                            git::file_name(&path)
                        ));
                    }
                }
            }
        }
        self.sidebar.refresh(None);
        if let Some(error) = first_error {
            self.set_error(&error);
        } else {
            self.set_message(&format!("Deleted {deleted} item(s)"));
        }
    }

    pub(super) fn open_folder_picker(&mut self, nodes: Vec<(PathBuf, bool)>, move_items: bool) {
        let references = nodes
            .iter()
            .map(|(path, is_directory)| (path.clone(), *is_directory))
            .collect::<Vec<_>>();
        let sidebar_nodes = references
            .iter()
            .filter_map(|(path, _)| self.sidebar.nodes.iter().find(|node| &node.path == path))
            .collect::<Vec<_>>();
        self.sidebar_folders = ui_sidebar::folder_choices(
            &sidebar_nodes,
            self.sidebar.show_git_ignored,
            &self.sidebar.root,
        );
        self.sidebar_transfer = Some((references, move_items));
        self.picker.kind = PickerKind::Folders;
        self.picker.query.clear();
        self.picker.selected = 0;
        self.picker.top = 0;
        self.picker.total = 0;
        self.picker.items.clear();
        self.rebuild_picker();
        self.clear_message();
        self.set_message(if move_items {
            "Choose a destination folder to move to"
        } else {
            "Choose a destination folder to copy to"
        });
    }

    pub(super) fn complete_transfer(&mut self, destination: PathBuf) {
        let Some((sources, move_items)) = self.sidebar_transfer.take() else {
            return;
        };
        let mut targets = std::collections::HashSet::new();
        for (source, _) in &sources {
            let target = destination.join(git::file_name(source));
            if !targets.insert(target) {
                self.set_error("Multiple items would collide in that folder.");
                self.sidebar.refresh(None);
                return;
            }
        }
        let mut done = 0usize;
        let mut first_error = None;
        let mut last_target = None;
        for (source, is_directory) in sources {
            let target = destination.join(git::file_name(&source));
            let result = if move_items {
                if is_directory && (destination == source || git::is_inside(&source, &destination))
                {
                    Err("Cannot move a folder inside itself.".to_string())
                } else if target.exists() {
                    Err(format!(
                        "'{}' already exists in that folder.",
                        git::file_name(&target)
                    ))
                } else {
                    std::fs::rename(&source, &target)
                        .map_err(|error| format!("Move failed: {error}"))
                }
            } else {
                git::copy_path(&source, &target, is_directory)
            };
            match result {
                Ok(()) => {
                    if move_items {
                        if is_directory {
                            self.sidebar.update_expanded_after_rename(&source, &target);
                        }
                        self.rename_open_document(&source, &target, is_directory);
                    }
                    done += 1;
                    last_target = Some(target);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(target) = last_target {
            self.sidebar.expand_to(Some(&target));
            self.sidebar.refresh(Some(&target));
        } else {
            self.sidebar.refresh(None);
        }
        if let Some(error) = first_error {
            self.set_error(&if done == 0 {
                error
            } else {
                format!("Completed {done} item(s); {error}")
            });
        } else {
            self.set_message(&format!(
                "{} {done} item(s) to {}",
                if move_items { "Moved" } else { "Copied" },
                destination.display()
            ));
        }
    }

    pub(super) fn render_sidebar(&mut self) {
        if !self.sidebar.visible {
            return;
        }
        let width = self.sidebar.width(self.cols);
        let rows = self.text_rows();
        if width == 0 || rows == 0 {
            return;
        }
        self.screen.fill(0, 0, width, rows, ' ', self.theme.base());
        let border_style = if self.sidebar.focused {
            self.theme.accent_style()
        } else {
            self.theme.border_style()
        };
        for row in 0..rows {
            self.screen
                .put((width - 1) as i64, row as i64, '│', border_style, 1);
        }
        if self.sidebar.nodes.is_empty() {
            self.screen.put_str(
                1,
                0,
                "No files",
                self.theme.muted_style(),
                width.saturating_sub(2),
            );
            return;
        }

        self.sidebar.update_top(rows);
        let list_rows = rows;
        let selected_style = self.theme.selected_item_style();
        let folder_style = self.theme.syntax_style(SyntaxKind::Default);
        let mut folder_style = folder_style;
        folder_style.fg = self.theme.accent;
        let file_style = self.theme.text();
        let marker_style = self.theme.git_modified_style();

        for row in 0..list_rows {
            let index = self.sidebar.top + row;
            let Some(node) = self.sidebar.nodes.get(index) else {
                break;
            };
            let selected =
                index >= self.sidebar.selection_start() && index <= self.sidebar.selection_end();
            let style = if selected {
                selected_style
            } else if node.is_directory {
                folder_style
            } else {
                file_style
            };
            if selected {
                self.screen.fill(0, row as i64, width - 1, 1, ' ', style);
            }
            let modified = self.sidebar.is_git_modified(node);
            let untracked = self.sidebar.is_git_untracked(node);
            let marker_count = modified as usize + untracked as usize;
            let indent = 1 + node.depth * 2;
            let marker = if node.is_directory {
                if node.expanded {
                    "▾"
                } else {
                    "▸"
                }
            } else {
                " "
            };
            self.screen.put_str(
                indent as i64,
                row as i64,
                marker,
                style,
                width.saturating_sub(indent + 1),
            );
            let name_x = indent + 2;
            if name_x < width - 1 {
                let mut name_width = width - name_x - 1;
                if marker_count > 0 {
                    name_width = name_width.saturating_sub(marker_count + 1);
                }
                self.screen
                    .put_str(name_x as i64, row as i64, &node.name, style, name_width);
            }
            if marker_count > 0 {
                let mut marker_x = width - 2 - marker_count;
                let mut marker_style = marker_style;
                if selected {
                    marker_style.bg = self.theme.selection_bg;
                }
                if modified && marker_x < width {
                    self.screen
                        .put(marker_x as i64, row as i64, 'M', marker_style, 1);
                    marker_x += 1;
                }
                if untracked && marker_x < width {
                    self.screen
                        .put(marker_x as i64, row as i64, 'N', marker_style, 1);
                }
            }
        }

        self.render_sidebar_format_progress(width);
    }

    fn render_sidebar_format_progress(&mut self, width: usize) {
        if width <= 2 {
            return;
        }
        let Some((total, completed)) = self
            .sidebar_format
            .as_ref()
            .map(|operation| (operation.progress.total(), operation.progress.completed()))
        else {
            return;
        };
        let completed = if total > 0 { completed.min(total) } else { 0 };

        let label = "Formatting";
        let label_style = self.theme.muted_style();
        self.screen.put_str(1, 0, label, label_style, width - 2);

        let count = if total > 0 {
            format!("{completed}/{total}")
        } else {
            "…".to_string()
        };
        let count_width = count.chars().count() as isize;
        let count_x = width as isize - 1 - count_width - 1;
        if count_x > (1 + label.len() + 1) as isize {
            self.screen.put_str(
                count_x as i64,
                0,
                &count,
                label_style,
                count.chars().count(),
            );
        }

        let bar_x = 1 + label.len() + 1;
        let bar_width = count_x - bar_x as isize - 1;
        if bar_width < 4 {
            return;
        }
        let bar_width = bar_width as usize;
        let filled = if total > 0 {
            ((bar_width as f64 * (completed as f64 / total as f64)).round() as usize).min(bar_width)
        } else {
            0
        };
        let mut filled_style = self.theme.syntax_style(SyntaxKind::Default);
        filled_style.fg = self.theme.accent;
        let empty_style = self.theme.border_style();
        for index in 0..bar_width {
            self.screen.put(
                (bar_x + index) as i64,
                0,
                if index < filled { '━' } else { '─' },
                if index < filled {
                    filled_style
                } else {
                    empty_style
                },
                1,
            );
        }
    }

    pub(super) fn render_sidebar_context_menu(&mut self) {
        if !self.sidebar.menu_visible {
            return;
        }
        let Some(node) = self.sidebar.selected_node().cloned() else {
            return;
        };
        let items = self.sidebar.menu_items();
        let mut width = 4usize;
        for item in items {
            width = width.max(item.label.len() + 7);
        }
        let rows = self.text_rows();
        let height = items.len() + 2;
        if rows < 3 || self.cols < 10 {
            return;
        }
        width = width.min(self.cols.saturating_sub(2));
        let anchor_row = 1 + self.sidebar.selected.saturating_sub(self.sidebar.top);
        let y = anchor_row.min(rows.saturating_sub(height));
        let title = if self.sidebar.selection_count() > 1 {
            format!("{} selected", self.sidebar.selection_count())
        } else if node.is_directory {
            "Folder".into()
        } else {
            "File".into()
        };
        self.draw_box(1, y, width, height, &title);
        let selected_style = self.theme.selected_item_style();
        let item_style = self.theme.text();
        for (index, item) in items.iter().enumerate() {
            let row = y + 1 + index;
            let style = if index == self.sidebar.menu_selected {
                selected_style
            } else {
                item_style
            };
            self.screen
                .fill(2, row as i64, width.saturating_sub(2), 1, ' ', style);
            self.screen
                .put_str(3, row as i64, item.label, style, width.saturating_sub(7));
            self.screen.put_str(
                (width.saturating_sub(2)) as i64,
                row as i64,
                &item.hotkey.to_string(),
                style,
                2,
            );
        }
    }
}

fn collect_files(path: &Path, root: &Path, show_git_ignored: bool, files: &mut Vec<PathBuf>) {
    if files.len() >= 20_000 {
        return;
    }
    let ignore = if show_git_ignored {
        None
    } else {
        Some(git::GitIgnore::new(root))
    };
    collect_files_inner(path, ignore.as_ref(), files, 0);
}

fn collect_files_inner(
    path: &Path,
    ignore: Option<&git::GitIgnore>,
    files: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 24 || files.len() >= 20_000 {
        return;
    }
    let ignore = ignore.map(|rules| rules.with_directory(path));
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let child = entry.path();
        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if git::is_built_in_ignored_directory(&name) {
                continue;
            }
            if let Some(rules) = ignore.as_ref() {
                if rules.is_ignored(&child, true) {
                    continue;
                }
            }
            collect_files_inner(&child, ignore.as_ref(), files, depth + 1);
        } else if file_type.is_file() {
            if let Some(rules) = ignore.as_ref() {
                if rules.is_ignored(&child, false) {
                    continue;
                }
            }
            files.push(child);
            if files.len() >= 20_000 {
                return;
            }
        }
    }
}

impl Editor {
    pub(super) fn format_selection(
        &mut self,
        formatter: &FormatterConfig,
        path: Option<&Path>,
        save_after: bool,
    ) {
        let (range_start, range_end) = self.selection_range();
        if range_end <= range_start {
            self.format_current_document(save_after);
            return;
        }
        let range_linewise = self.selection_linewise;
        let selected = self.doc.range(range_start, range_end - range_start);
        let range_ends_with_newline = selected
            .last()
            .is_some_and(|byte| *byte == b'\n' || *byte == b'\r');
        let (input, indent_prefix) = remove_common_indent(&selected);

        match format::run(formatter, &input, path) {
            Ok(output) => {
                let replacement =
                    self.prepare_selection_output(&output, range_ends_with_newline, &indent_prefix);
                self.doc.break_undo_group();
                self.doc
                    .replace(range_start, range_end - range_start, &replacement);
                let mut cursor = (range_start + replacement.len()).min(self.doc.size());
                if range_linewise && cursor > range_start {
                    let end_line = self.doc.line_of_offset(cursor);
                    let end_line_start = self.doc.line_start(end_line);
                    if cursor == end_line_start && end_line > 0 {
                        cursor = self.doc.line_end(end_line - 1);
                    }
                }
                self.cursor = cursor;
                self.anchor = range_start;
                self.selection_linewise = range_linewise;
                self.mode = Mode::Select;
                self.preferred_column = None;
                self.doc.break_undo_group();
                self.lsp_dirty = self.lsp.is_running();
                self.lsp_dirty_since = std::time::Instant::now();
                if save_after {
                    self.save_document_skip_format(None);
                }
                self.set_message(&format!(
                    "Formatted selection in {}",
                    self.doc.display_name()
                ));
            }
            Err(error) => {
                if save_after {
                    self.save_document_skip_format(None);
                } else {
                    self.set_error(&error);
                }
            }
        }
    }

    fn prepare_selection_output(
        &self,
        output: &[u8],
        range_ends_with_newline: bool,
        prefix: &str,
    ) -> Vec<u8> {
        let eol = self.doc.eol.as_bytes();
        let mut normalized = normalize_eol(output, eol);
        if range_ends_with_newline {
            if normalized
                .last()
                .is_some_and(|byte| *byte != b'\n' && *byte != b'\r')
            {
                normalized.extend_from_slice(eol);
            }
        } else {
            while normalized
                .last()
                .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
            {
                normalized.pop();
            }
        }
        reindent_selection(&normalized, prefix)
    }

    pub(super) fn format_workspace_extension(&mut self, argument: &str) {
        let mut extension = argument.trim().trim_start_matches('.').to_string();
        if extension.is_empty()
            || extension
                .chars()
                .any(|character| character.is_whitespace() || character == '/' || character == '\\')
        {
            self.set_error("usage: :fmta <extension>");
            return;
        }
        extension.make_ascii_lowercase();
        let Some(language) = self.languages.detect(&format!("file.{extension}")) else {
            self.set_error(&format!(
                "No language is configured for .{extension} files."
            ));
            return;
        };
        if language.formatter.is_none() {
            self.set_error(&format!("No formatter configured for {}.", language.name));
            return;
        }

        let suffix = format!(".{extension}");
        let root = self.sidebar.root.clone();
        let open_path = self.doc.path.clone().map(PathBuf::from);
        let files = files::scan_root_with_ignored(&root, 50_000, self.sidebar.show_git_ignored);
        let mut matched = 0usize;
        let mut formatted = 0usize;
        let mut changed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut skipped_open = 0usize;
        let mut first_error: Option<String> = None;

        for relative in files {
            if !relative.to_ascii_lowercase().ends_with(&suffix) {
                continue;
            }
            matched += 1;
            let path = root.join(&relative);
            if open_path
                .as_ref()
                .is_some_and(|open| git::same_path(open, &path))
            {
                skipped_open += 1;
                continue;
            }
            let language = self.languages.detect(&relative);
            let Some(formatter) = language.and_then(|language| language.resolve_formatter()) else {
                skipped += 1;
                continue;
            };
            let formatter = formatter.clone();
            let input = match std::fs::read(&path) {
                Ok(input) => input,
                Err(error) => {
                    failed += 1;
                    first_error
                        .get_or_insert_with(|| format!("{}: {error}", git::file_name(&path)));
                    continue;
                }
            };
            match format::run(&formatter, &input, Some(&path)) {
                Ok(mut output) => {
                    formatted += 1;
                    if git::has_utf8_bom(&input) && !git::has_utf8_bom(&output) {
                        let mut with_bom = Vec::with_capacity(output.len() + 3);
                        with_bom.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                        with_bom.extend_from_slice(&output);
                        output = with_bom;
                    }
                    if output != input {
                        if let Err(error) = git::atomic_write(&path, &output) {
                            failed += 1;
                            first_error.get_or_insert_with(|| {
                                format!("{}: {error}", git::file_name(&path))
                            });
                        } else {
                            changed += 1;
                        }
                    }
                }
                Err(error) => {
                    failed += 1;
                    first_error
                        .get_or_insert_with(|| format!("{}: {error}", git::file_name(&path)));
                }
            }
        }

        if let Some(error) = first_error {
            self.set_error(&format!(
                "Formatted {formatted} .{extension} file(s), {failed} failed: {error}"
            ));
        } else if formatted == 0 {
            if matched == 0 {
                self.set_message(&format!("No .{extension} files found in the workspace."));
            } else if skipped_open > 0 && skipped_open == matched {
                self.set_message("Skipped the open buffer.");
            } else {
                self.set_message(&format!("No .{extension} files were formatted."));
            }
        } else {
            let mut summary = format!("Formatted {formatted} .{extension} file(s)");
            if changed > 0 {
                summary.push_str(&format!(", {changed} changed"));
            }
            if skipped > 0 {
                summary.push_str(&format!(", {skipped} skipped"));
            }
            if skipped_open > 0 {
                summary.push_str(", open buffer skipped");
            }
            self.set_message(&summary);
        }
    }
}

fn normalize_eol(text: &[u8], eol: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(text.len() + eol.len());
    let mut index = 0usize;
    while index < text.len() {
        match text[index] {
            b'\r' => {
                index += 1;
                if index < text.len() && text[index] == b'\n' {
                    index += 1;
                }
                output.extend_from_slice(eol);
            }
            b'\n' => {
                index += 1;
                output.extend_from_slice(eol);
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}

fn remove_common_indent(text: &[u8]) -> (Vec<u8>, String) {
    if text.is_empty() {
        return (text.to_vec(), String::new());
    }
    let mut common = usize::MAX;
    let mut first_indent_start = 0usize;
    let mut line_start = 0usize;
    loop {
        let line_end = text[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| line_start + index)
            .unwrap_or(text.len());
        let content_end = if line_end > line_start && text[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let mut indent_end = line_start;
        while indent_end < content_end && matches!(text[indent_end], b' ' | b'\t') {
            indent_end += 1;
        }
        if indent_end < content_end {
            let indent_length = indent_end - line_start;
            if common == usize::MAX {
                common = indent_length;
                first_indent_start = line_start;
            } else {
                let mut matched = 0usize;
                while matched < common.min(indent_length)
                    && text[first_indent_start + matched] == text[line_start + matched]
                {
                    matched += 1;
                }
                common = matched;
            }
            if common == 0 {
                return (text.to_vec(), String::new());
            }
        }
        if line_end >= text.len() {
            break;
        }
        line_start = line_end + 1;
    }
    if common == usize::MAX || common == 0 {
        return (text.to_vec(), String::new());
    }

    let prefix = String::from_utf8_lossy(&text[first_indent_start..first_indent_start + common])
        .into_owned();
    let mut result = Vec::with_capacity(text.len());
    line_start = 0usize;
    loop {
        let line_end = text[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| line_start + index)
            .unwrap_or(text.len());
        let content_end = if line_end > line_start && text[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let non_blank = text[line_start..content_end]
            .iter()
            .any(|byte| !matches!(byte, b' ' | b'\t'));
        let mut remove = 0usize;
        if non_blank
            && content_end - line_start >= common
            && text[line_start..line_start + common]
                == text[first_indent_start..first_indent_start + common]
        {
            remove = common;
        }
        result.extend_from_slice(&text[line_start + remove..line_end]);
        if line_end >= text.len() {
            break;
        }
        result.push(b'\n');
        line_start = line_end + 1;
    }
    (result, prefix)
}

fn reindent_selection(text: &[u8], prefix: &str) -> Vec<u8> {
    if text.is_empty() || prefix.is_empty() {
        return text.to_vec();
    }
    let indent = prefix.as_bytes();
    let mut result = Vec::with_capacity(text.len() + indent.len() * 4);
    let mut line_start = true;
    let mut index = 0usize;
    while index < text.len() {
        if line_start {
            let empty = text[index] == b'\n'
                || (text[index] == b'\r' && index + 1 < text.len() && text[index + 1] == b'\n');
            if !empty {
                result.extend_from_slice(indent);
            }
            line_start = false;
        }
        result.push(text[index]);
        if text[index] == b'\n' {
            line_start = true;
        }
        index += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{normalize_eol, reindent_selection, remove_common_indent, BulkFormatResolver};
    use crate::core::document::Document;
    use crate::languages::Registry;
    use crate::ui::editor::{Editor, PickerKind};
    use crate::ui::sidebar::{Action as SidebarAction, Node};
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    fn random_bytes(seed: &mut u64, length: usize) -> Vec<u8> {
        const ALPHABET: &[u8] = b" \t\r\nabcXYZ0123{}();:/\\";
        let mut output = Vec::with_capacity(length);
        for _ in 0..length {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            output.push(ALPHABET[(*seed as usize) % ALPHABET.len()]);
        }
        output
    }

    #[test]
    fn formatting_helpers_handle_arbitrary_bytes_without_panicking() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for length in 0..512 {
            for _ in 0..8 {
                let text = random_bytes(&mut seed, length);
                let (stripped, prefix) = remove_common_indent(&text);
                assert!(stripped.len() <= text.len());
                let _ = reindent_selection(&stripped, &prefix);
                let _ = normalize_eol(&text, b"\n");
                let _ = normalize_eol(&text, b"\r\n");
            }
        }
    }

    #[test]
    fn selection_indent_helpers_preserve_expected_text() {
        let (text, prefix) = remove_common_indent(b"    one\n    two\n");
        assert_eq!(text, b"one\ntwo\n");
        assert_eq!(prefix, "    ");
        assert_eq!(reindent_selection(&text, &prefix), b"    one\n    two\n");

        let (text, prefix) = remove_common_indent(b"\tone\n\ttwo");
        assert_eq!(text, b"one\ntwo");
        assert_eq!(prefix, "\t");

        let (text, prefix) = remove_common_indent(b"one\n  two\n");
        assert_eq!(text, b"one\n  two\n");
        assert_eq!(prefix, "");

        let (text, prefix) = remove_common_indent(b"\n\n");
        assert_eq!(text, b"\n\n");
        assert_eq!(prefix, "");

        let (text, prefix) = remove_common_indent(b"    \n    ");
        assert_eq!(text, b"    \n    ");
        assert_eq!(prefix, "");
    }

    #[test]
    fn eol_normalization_handles_all_line_endings() {
        assert_eq!(normalize_eol(b"a\r\nb\rc\nd", b"\n"), b"a\nb\nc\nd");
        assert_eq!(normalize_eol(b"a\r\nb\rc\nd", b"\r\n"), b"a\r\nb\r\nc\r\nd");
        assert_eq!(normalize_eol(b"\r", b"\n"), b"\n");
        assert_eq!(normalize_eol(b"\r\n", b"\n"), b"\n");
    }

    #[test]
    fn file_menu_offers_diff_with() {
        let item = crate::ui::sidebar::FILE_MENU
            .iter()
            .find(|item| item.action == SidebarAction::DiffWith)
            .expect("Diff with entry");
        assert_eq!(item.label, "Diff with...");
        assert_eq!(item.hotkey, 'C');
        assert!(!crate::ui::sidebar::FOLDER_MENU
            .iter()
            .any(|item| item.action == SidebarAction::DiffWith));
    }

    #[test]
    fn diff_with_picker_excludes_the_source_and_renders_the_diff() {
        let root =
            std::env::temp_dir().join(format!("hexal-diff-with-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.rs");
        let target = root.join("target.rs");
        std::fs::write(&source, b"fn main() {}\n").unwrap();
        std::fs::write(&target, b"fn main() { let x = 1; }\n").unwrap();

        let mut editor = Editor::with_options(Document::new(), 100, 30, Some(root.clone()), false);
        editor.sidebar.nodes = vec![Node {
            path: source.clone(),
            name: "source.rs".into(),
            depth: 0,
            is_directory: false,
            expanded: false,
        }];
        editor.sidebar.selected = 0;
        editor.sidebar.anchor = 0;
        editor.run_sidebar_action(SidebarAction::DiffWith, vec![(source.clone(), false)]);

        assert!(matches!(editor.picker.kind, PickerKind::DiffWith));
        assert_eq!(editor.diff_with_source.as_deref(), Some(source.as_path()));
        assert!(!editor
            .picker
            .items
            .iter()
            .any(|item| editor.picker.files[item.file_index] == "source.rs"));
        let target_index = editor
            .picker
            .items
            .iter()
            .position(|item| editor.picker.files[item.file_index] == "target.rs")
            .expect("target item");
        editor.picker.selected = target_index;
        editor.open_picker_selection();

        assert!(matches!(editor.picker.kind, PickerKind::None));
        assert_eq!(editor.language_override.as_deref(), Some("diff"));
        let diff = String::from_utf8(editor.doc.all_text()).unwrap();
        assert!(diff.contains("--- "), "{diff}");
        assert!(diff.contains("+++ "), "{diff}");
        assert!(diff.contains("@@"), "{diff}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bulk_format_resolver_matches_registry_detection() {
        let registry = Registry::load();
        let resolver = BulkFormatResolver::new(&registry);
        for path in [
            "src/main.rs",
            "Cargo.toml",
            "view.blade.php",
            "script.py",
            "file.unknown-extension",
        ] {
            let expected = registry.detect(path).map(|language| language.name.clone());
            let actual = resolver
                .detect(Path::new(path))
                .map(|index| resolver.languages[index].name.clone());
            assert_eq!(actual, expected, "{path}");
        }
    }

    #[test]
    fn bulk_format_formats_multiple_files_with_parallel_workers() {
        if !crate::util::git::command_exists("rustfmt") {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("hexal-format-parallel-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let files: Vec<_> = (0..4)
            .map(|index| root.join(format!("file{index}.rs")))
            .collect();
        for file in &files {
            std::fs::write(file, b"fn main(){let x=1;}\n").unwrap();
        }

        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.format_sidebar_nodes(&[(root.clone(), true)]);
        let deadline = Instant::now() + Duration::from_secs(15);
        while editor.sidebar_format.is_some() && Instant::now() < deadline {
            editor.poll_sidebar_format();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(editor.sidebar_format.is_none());
        assert!(!editor.message_is_error, "{}", editor.message);
        assert_eq!(editor.message, "Formatted 4 file(s), 4 changed");
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            assert!(text.contains("fn main() {"), "{text:?}");
            assert!(text.contains("let x = 1;"), "{text:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bulk_format_survives_missing_formatters_without_erroring() {
        let root = std::env::temp_dir().join(format!("hexal-format-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let python = root.join("missing_formatter.py");
        std::fs::write(&python, b"x = 1\n").unwrap();

        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.format_sidebar_nodes(&[(python.clone(), false)]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while editor.sidebar_format.is_some() && Instant::now() < deadline {
            editor.poll_sidebar_format();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(editor.sidebar_format.is_none());
        assert!(!editor.message_is_error, "{}", editor.message);
        assert_eq!(editor.message, "No files were formatted.");
        assert_eq!(std::fs::read(&python).unwrap(), b"x = 1\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sidebar_format_progress_bar_matches_the_expected_layout() {
        let mut editor = Editor::new(Document::new(), 100, 30);
        editor.screen.resize(60, 20);
        let (_sender, receiver) = mpsc::channel();
        editor.sidebar_format = Some(crate::ui::editor::SidebarFormatOperation {
            receiver,
            progress: Arc::new(crate::ui::editor::SidebarFormatProgress {
                total: AtomicUsize::new(4),
                completed: AtomicUsize::new(2),
            }),
            last_total: 0,
            last_completed: 0,
        });

        editor.render_sidebar_format_progress(40);
        let row: String = (0..40).map(|x| editor.screen.at(x, 0).ch).collect();
        assert!(row.starts_with(" Formatting "), "{row:?}");
        assert!(row.contains("2/4"), "{row:?}");
        assert!(row.contains('━'), "{row:?}");
        assert!(row.contains('─'), "{row:?}");
    }
}
