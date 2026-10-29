//! Opt-in verbose startup tracing (`-vv` / `--verbose`).
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::util::log;

const PREFIX: &str = "[startup] ";

static ENABLED: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

pub fn begin(arguments: &[String]) {
    if !arguments
        .iter()
        .any(|argument| argument == "-vv" || argument == "--verbose")
    {
        return;
    }
    ENABLED.store(true, Ordering::Relaxed);
    let _ = START.set(Instant::now());
    write_header(arguments);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn timestamp() -> Option<Instant> {
    if enabled() {
        Some(Instant::now())
    } else {
        None
    }
}

pub fn mark(name: &str, started: Option<Instant>, detail: &str) {
    if !enabled() {
        return;
    }
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let suffix = if detail.is_empty() {
        String::new()
    } else {
        format!(" ({detail})")
    };
    log::write(&format!("{PREFIX}{name}: {elapsed:.1} ms{suffix}"));
}

pub fn note(message: &str) {
    if enabled() {
        log::write(&format!("{PREFIX}{message}"));
    }
}

fn write_header(arguments: &[String]) {
    let elapsed = START
        .get()
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    log::write(&format!("{PREFIX}=== verbose startup trace ==="));
    log::write(&format!(
        "{PREFIX}first log write: {elapsed:.1} ms to {}",
        log::path().display()
    ));
    log::write(&format!(
        "{PREFIX}version: {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("HEXAL_COMMIT_HASH")
    ));
    log::write(&format!(
        "{PREFIX}arguments: {:?}",
        arguments.get(1..).unwrap_or(&[])
    ));
    log::write(&format!(
        "{PREFIX}process path: {}",
        std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "<unknown>".into())
    ));
    log::write(&format!(
        "{PREFIX}working directory: {}",
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "<unknown>".into())
    ));
    log::write(&format!(
        "{PREFIX}home directory: {}",
        crate::core::config::hexal_home_directory().display()
    ));
    log::write(&format!(
        "{PREFIX}os: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    log::write(&format!(
        "{PREFIX}processor count: {}",
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(0)
    ));
    log::write(&format!(
        "{PREFIX}PATH: {}",
        std::env::var("PATH").unwrap_or_else(|_| "<unset>".into())
    ));
}
