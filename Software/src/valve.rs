//! Solenoid valve output with PWM hold.
//!
//! A solenoid needs full current to pull its plunger in, but far less to
//! hold it once the magnetic gap has closed. On energise the gate is driven
//! at 100% for `VALVE_PULL_IN_MS`, then PWM'd at `VALVE_HOLD_DUTY_PCT`. At
//! 50% the coil dissipates roughly a quarter of its full power (P ∝ I²),
//! which matters here: both valves are normally-open and held shut for
//! hours at a time (start-assist while compressing, drain always), and the
//! 24V DC coils are ~40Ω / ~14W.
//!
//! During every PWM off-time the coil current freewheels through the
//! flyback diode (SS34 Schottky on the piggyback). The PWM comes from the
//! piggyback's PCA9685 at `VALVE_PWM_HZ`, through its safety AND gates, so a
//! missed watchdog heartbeat or a hardware over-temperature trip drops both
//! valves whatever this module asks for.

use embassy_time::Instant;

use crate::config;
use crate::outputs;

pub struct PwmValve {
    /// PCA9685 channel on the piggyback; `outputs::task` writes the duty.
    channel: u8,
    name: &'static str,
    energised_since: Option<Instant>,
    holding: bool,
}

impl PwmValve {
    pub fn new(channel: u8, name: &'static str) -> Self {
        Self {
            channel,
            name,
            energised_since: None,
            holding: false,
        }
    }

    /// Drive the coil on or off. Call repeatedly (every control-loop pass):
    /// the step from pull-in to hold happens on the first call after
    /// `VALVE_PULL_IN_MS`, so pull-in actually lasts up to one loop period
    /// longer — harmless, it only means a little extra heat per switch-on.
    pub fn set(&mut self, energise: bool, now: Instant) {
        match (energise, self.energised_since) {
            (true, None) => {
                self.duty(100);
                self.energised_since = Some(now);
                self.holding = false;
            }
            (true, Some(since))
                if !self.holding
                    && now.duration_since(since).as_millis() >= config::VALVE_PULL_IN_MS =>
            {
                self.duty(config::VALVE_HOLD_DUTY_PCT);
                self.holding = true;
            }
            (false, Some(_)) => {
                self.duty(0);
                self.energised_since = None;
            }
            _ => {}
        }
    }

    /// While `quiet`, a holding valve's MOSFET is switched fully off, so
    /// ADC sampling sees neither PWM edges nor coil current in the shared
    /// breadboard ground: the coil's current freewheels round its flyback
    /// diode instead, decaying over its L/R time — far longer than the few
    /// ms of a sample burst, and the plunger takes longer still to drop
    /// out. No effect on a valve that is off or pulling in.
    pub fn quiet(&self, quiet: bool) {
        if self.holding && self.energised_since.is_some() && config::VALVE_HOLD_DUTY_PCT < 100 {
            self.duty(if quiet { 0 } else { config::VALVE_HOLD_DUTY_PCT });
        }
    }

    fn duty(&self, pct: u8) {
        log::debug!("valve {}: {pct}%", self.name);
        outputs::set_duty(self.channel, pct);
    }
}
