//! Piggyback outputs: valve PWM duties and the watchdog heartbeat, all on
//! the piggyback's PCA9685 (the heartbeat is channel 4, wired to the
//! watchdog's trigger input).
//!
//! The control loop stays synchronous: `PwmValve` just stores the duty it
//! wants here, and this task pushes changes over I2C. The heartbeat toggles
//! only while the control loop keeps calling `control_alive`, so the
//! piggyback's watchdog (74LVC1G123, ~1 s) catches a hung control loop, not
//! just a hung executor. When it times out, the piggyback's AND gates drop
//! both valves and open the VFD enable contact, whatever we write.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use embassy_time::{Duration, Instant, Ticker};

use crate::board::pca9685::Pca9685;

/// PCA9685 channels.
pub const CH_START_ASSIST: u8 = 0;
pub const CH_DRAIN: u8 = 1;
pub const CH_LED_OK: u8 = 2;
pub const CH_LED_FAULT: u8 = 3;
const CH_HEARTBEAT: u8 = 4;
const CHANNELS: usize = 4;

const UNSET: u8 = 0xFF;
static DUTY: [AtomicU8; CHANNELS] = [const { AtomicU8::new(0) }; CHANNELS];
/// Low 32 bits of the uptime in ms (no 64-bit atomics on Xtensa).
static LAST_CONTROL_MS: AtomicU32 = AtomicU32::new(0);

/// Heartbeat half-period: toggles at 5 Hz while healthy.
const TOGGLE_MS: u64 = 100;
/// Longest gap between control-loop passes before the heartbeat stops.
/// The loop runs every ADC_SAMPLE_MS (500 ms), and a Modbus timeout can add
/// a few hundred ms more.
const CONTROL_STALE_MS: u32 = 1_500;

pub fn set_duty(channel: u8, pct: u8) {
    if let Some(d) = DUTY.get(channel as usize) {
        d.store(pct.min(100), Ordering::Relaxed);
    }
}

/// Called once per control-loop pass.
pub fn control_alive() {
    LAST_CONTROL_MS.store(now_ms(), Ordering::Relaxed);
}

#[embassy_executor::task]
pub async fn task(mut pca: Option<Pca9685>) {
    let mut written = [UNSET; CHANNELS];
    let mut beat = false;
    let mut ticker = Ticker::every(Duration::from_millis(TOGGLE_MS));
    loop {
        ticker.next().await;

        if let Some(p) = pca.as_mut() {
            for (ch, want) in DUTY.iter().enumerate() {
                let want = want.load(Ordering::Relaxed);
                if written[ch] != want {
                    match p.set_duty_pct(ch as u8, want).await {
                        Ok(()) => written[ch] = want,
                        Err(()) => {
                            log::warn!("pca9685: write ch{ch} failed");
                            written[ch] = UNSET; // retry next tick
                        }
                    }
                }
            }
        }

        let since = now_ms().wrapping_sub(LAST_CONTROL_MS.load(Ordering::Relaxed));
        if since < CONTROL_STALE_MS
            && let Some(p) = pca.as_mut()
        {
            beat = !beat;
            if p.set_duty_pct(CH_HEARTBEAT, if beat { 100 } else { 0 }).await.is_err() {
                log::warn!("pca9685: heartbeat write failed");
            }
        }
    }
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}
