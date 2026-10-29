//! Optional startup timings shown by the command palette.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Entry {
    pub category: String,
    pub name: String,
    pub milliseconds: f64,
    pub detail: String,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

pub fn set_enabled(enabled: bool) {
    let was_enabled = ENABLED.swap(enabled, Ordering::Relaxed);
    if enabled && !was_enabled {
        if let Ok(mut entries) = ENTRIES.lock() {
            entries.clear();
        }
    } else if !enabled {
        if let Ok(mut entries) = ENTRIES.lock() {
            entries.clear();
        }
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn add(category: &str, name: &str, milliseconds: f64, detail: &str) {
    if !enabled() {
        return;
    }
    let Ok(mut entries) = ENTRIES.lock() else {
        return;
    };
    entries.push(Entry {
        category: category.to_string(),
        name: name.to_string(),
        milliseconds: milliseconds.max(0.0),
        detail: detail.to_string(),
    });
}

pub fn add_since(category: &str, name: &str, started: Option<Instant>, detail: &str) {
    let Some(started) = started else {
        return;
    };
    add(
        category,
        name,
        started.elapsed().as_secs_f64() * 1000.0,
        detail,
    );
}

pub fn snapshot() -> Vec<Entry> {
    let Ok(entries) = ENTRIES.lock() else {
        return Vec::new();
    };
    entries.clone()
}

pub fn sorted() -> Vec<Entry> {
    let mut entries = snapshot();
    entries.sort_by(|left, right| right.milliseconds.total_cmp(&left.milliseconds));
    entries
}
