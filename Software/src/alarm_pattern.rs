//! When the beeper is on: plain Rust with no hardware, so it can be checked on
//! a computer (`tools/alarm_check.rs`). The rules are in docs/alerts.md.

use crate::config::*;

/// Alarm severity. Ordered: the highest active one is the one that sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    None,
    Warning,
    Fault,
    Trip,
}

/// (on, off) times in ms for one burst of each level.
const TRIP: &[(u64, u64)] = &[(100, 100), (100, 100), (100, 100), (100, 100)];
const FAULT: &[(u64, u64)] = &[(400, 200), (400, 200)];
const WARNING: &[(u64, u64)] = &[(200, 0)];

/// Is the beeper on `t` ms after `level` started (and wasn't acknowledged)?
pub fn sounding(level: Level, t: u64) -> bool {
    let (burst, start) = match level {
        Level::None => return false,
        Level::Trip => (TRIP, burst_start(t, ALARM_TRIP_FAST_MS, ALARM_TRIP_FAST_FOR_MS, ALARM_TRIP_SLOW_MS)),
        Level::Fault => (FAULT, burst_start(t, ALARM_FAULT_FAST_MS, ALARM_FAULT_FAST_FOR_MS, ALARM_FAULT_SLOW_MS)),
        Level::Warning => (WARNING, t - t % ALARM_WARNING_REPEAT_MS),
    };
    let mut at = t - start;
    for &(on, off) in burst {
        if at < on {
            return true;
        }
        if at < on + off {
            return false;
        }
        at -= on + off;
    }
    false
}

/// Start of the most recent burst: every `fast` ms for the first `fast_for`
/// ms, then every `slow` ms.
fn burst_start(t: u64, fast: u64, fast_for: u64, slow: u64) -> u64 {
    if t < fast_for {
        t - t % fast
    } else {
        let s = t - fast_for;
        fast_for + s - s % slow
    }
}

/// Tracks the active level, when it started and whether it was acknowledged.
pub struct Alarm {
    level: Level,
    since_ms: u64,
    acked: bool,
    warning_since_ms: Option<u64>,
}

impl Alarm {
    pub const fn new() -> Self {
        Self { level: Level::None, since_ms: 0, acked: false, warning_since_ms: None }
    }

    /// The level that is sounding (or was, until acknowledged).
    pub fn level(&self) -> Level {
        self.level
    }

    /// Whether the active alarm has been silenced by a touch.
    pub fn acked(&self) -> bool {
        self.acked
    }

    /// Feed the raw level and any touch; returns whether the beeper should be on.
    /// A different level starts its pattern afresh and needs a new acknowledge.
    pub fn update(&mut self, raw: Level, now_ms: u64, touched: bool) -> bool {
        let level = if raw == Level::Warning {
            let since = *self.warning_since_ms.get_or_insert(now_ms);
            if now_ms - since >= ALARM_WARNING_DEBOUNCE_MS { Level::Warning } else { Level::None }
        } else {
            self.warning_since_ms = None;
            raw
        };
        if level != self.level {
            self.level = level;
            self.since_ms = now_ms;
            self.acked = false;
        }
        if touched && self.level != Level::None && !self.acked {
            self.acked = true;
        }
        !self.acked && sounding(self.level, now_ms - self.since_ms)
    }
}
