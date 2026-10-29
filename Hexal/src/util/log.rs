//! Append-only diagnostics log in `~/.hexal/hexal.log`.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::core::config::hexal_home_directory;
use crate::util::time;

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn path() -> PathBuf {
    hexal_home_directory().join("hexal.log")
}

pub fn write(message: &str) {
    let path = path();
    let Some(directory) = path.parent() else {
        return;
    };
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        let _ = fs::write(&path, []);
    }
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(file, "{} {message}", timestamp());
}

fn timestamp() -> String {
    time::format_utc(time::now_utc())
}
