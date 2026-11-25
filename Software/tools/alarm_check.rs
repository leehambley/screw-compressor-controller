// Host check for the beeper timing (src/alarm_pattern.rs). No hardware needed:
//
//     cd Software/tools && rustc --edition 2024 -o /tmp/alarm_check alarm_check.rs && /tmp/alarm_check

#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/alarm_pattern.rs"]
mod alarm_pattern;

use alarm_pattern::{Alarm, Level, sounding};

/// Beeps (rising edges) in [from, to) ms, sampled every 10 ms.
fn beeps(level: Level, from: u64, to: u64) -> usize {
    let mut n = 0;
    let mut prev = false;
    for t in (from..to).step_by(10) {
        let s = sounding(level, t);
        if s && !prev { n += 1; }
        prev = s;
    }
    n
}

fn main() {
    assert!(!sounding(Level::None, 0));
    // One burst at the start of each level.
    assert_eq!(beeps(Level::Trip, 0, 5_000), 4);
    assert_eq!(beeps(Level::Fault, 0, 30_000), 2);
    assert_eq!(beeps(Level::Warning, 0, 600_000), 1);
    // Repeat rates.
    assert_eq!(beeps(Level::Trip, 0, 120_000), 4 * 24);
    assert_eq!(beeps(Level::Trip, 120_000, 180_000), 4 * 2);
    assert_eq!(beeps(Level::Fault, 0, 600_000), 2 * 20);
    assert_eq!(beeps(Level::Fault, 600_000, 1_200_000), 2 * 2);
    assert_eq!(beeps(Level::Warning, 0, 1_800_000), 3);
    // Second trip beep starts exactly at 200 ms.
    assert!(sounding(Level::Trip, 200) && !sounding(Level::Trip, 150));

    // Nothing at boot, and a warning waits 10 s.
    let mut a = Alarm::new();
    assert!(!a.update(Level::None, 0, false));
    assert!(!a.update(Level::Warning, 1_000, false));
    assert!(!a.update(Level::Warning, 10_990, false));
    assert!(a.update(Level::Warning, 11_000, false));
    // Touch silences it; the same level stays quiet.
    assert!(!a.update(Level::Warning, 11_050, true));
    assert!(!a.update(Level::Warning, 611_000, false));
    // A worse alarm sounds again at once.
    assert!(a.update(Level::Fault, 700_000, false));
    // Cause gone: silent at once, and a touch with no alarm pre-acks nothing.
    assert!(!a.update(Level::None, 700_100, true));
    assert!(a.update(Level::Trip, 700_200, false));
    println!("alarm_check: ok");
}
