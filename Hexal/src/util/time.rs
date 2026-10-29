//! Small cross-platform date/time helpers used by logs and the debug console.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

pub fn now_utc() -> DateTime {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    from_unix_seconds(now.as_secs() as i64, now.subsec_millis())
}

pub fn now_local() -> DateTime {
    #[cfg(unix)]
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let seconds = now.as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&seconds, &mut tm) } != std::ptr::null_mut() {
            return DateTime {
                year: tm.tm_year + 1900,
                month: (tm.tm_mon + 1) as u32,
                day: tm.tm_mday as u32,
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
                second: tm.tm_sec as u32,
                millis: now.subsec_millis(),
            };
        }
    }
    #[cfg(windows)]
    {
        if let Some(dt) = windows_local_time() {
            return dt;
        }
    }
    now_utc()
}

pub fn format_utc(dt: DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second, dt.millis
    )
}

fn from_unix_seconds(seconds: i64, millis: u32) -> DateTime {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    DateTime {
        year,
        month,
        day,
        hour: (of_day / 3_600) as u32,
        minute: ((of_day % 3_600) / 60) as u32,
        second: (of_day % 60) as u32,
        millis,
    }
}

// Howard Hinnant's days-to-civil conversion.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        (year + if month <= 2 { 1 } else { 0 }) as i32,
        month as u32,
        day as u32,
    )
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SystemTimeRecord {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLocalTime(time: *mut SystemTimeRecord);
}

#[cfg(windows)]
fn windows_local_time() -> Option<DateTime> {
    let mut value = SystemTimeRecord::default();
    unsafe { GetLocalTime(&mut value) };
    if value.year == 0 {
        return None;
    }
    Some(DateTime {
        year: value.year as i32,
        month: value.month as u32,
        day: value.day as u32,
        hour: value.hour as u32,
        minute: value.minute as u32,
        second: value.second as u32,
        millis: value.milliseconds as u32,
    })
}
