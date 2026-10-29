//! Sensor scaling: 4-20mA pressure loops and NTC 100K temperature probes.
//!
//! NOTE: as of the last session the board only has 2 spare GPIOs bodged in
//! for temperature (not the original 3-channel design). Keep rotor primary
//! (safety-critical) and motor winding (VFD derate); rotor secondary
//! (log/display only) is dropped. This module still models 3 channels —
//! trim `TempChannel` / call sites once the bodge wiring is finalised.

use crate::config::{
    MOTOR_POWER_FACTOR, MOTOR_RATED_HZ, MOTOR_RATED_RPM, MOTOR_RATED_V, NTC_BETA, NTC_PULLUP_OHMS,
    NTC_REF_OHMS, NTC_SUPPLY_V, PRESSURE_CAL_V_AT_ZERO_BAR, PRESSURE_CAL_V_PER_BAR,
    PRESSURE_SHUNT_OHMS,
};

/// Raw ADC voltage (0.0–3.3V) across the shunt, converted to loop current
/// in mA. `V / R` alone gives amps — the `* 1000.0` is not optional
/// decoration, without it every reading downstream is off by 1000x. `pub`
/// so `main.rs` can log it directly alongside a real ammeter reading for
/// comparison — that's how the residual ~4x gain error was found
/// 2026-09-10 (ammeter: ~4.2mA; this formula from the logged raw voltage
/// at the same time: ~1.1mA). Used for `pressure_sensor_fault`'s 4mA-floor
/// check too — `pressure_bar` itself goes through the empirical
/// calibration below instead, since it's what's actually adjustable once
/// real gauge data exists.
pub fn loop_current_ma(raw_voltage: f32) -> f32 {
    raw_voltage / PRESSURE_SHUNT_OHMS * 1000.0
}

/// Convert a raw ADC voltage (0.0–3.3V) into a pressure reading in bar,
/// via a two-point linear fit (`PRESSURE_CAL_V_AT_ZERO_BAR`,
/// `PRESSURE_CAL_V_PER_BAR` in config.rs) rather than the nominal
/// 4-20mA/`PRESSURE_SHUNT_OHMS` math directly — those two config
/// constants currently just encode that same nominal math (so behaviour
/// is unchanged until they're updated), but the point is to have a
/// single place to drop in a real two-point calibration once one exists
/// (raw voltage at a known bar reading against the physical oil-filled
/// gauge, at two different pressures) without touching this function.
pub fn pressure_bar(raw_voltage: f32) -> f32 {
    (raw_voltage - PRESSURE_CAL_V_AT_ZERO_BAR) / PRESSURE_CAL_V_PER_BAR
}

/// 1 bar = 14.5038 psi, exactly (definitional, not measured — no
/// calibration concerns here unlike everything else in this module).
pub fn bar_to_psi(bar: f32) -> f32 {
    bar * 14.5038
}

/// True if the loop current is below the 4mA floor — indicates a
/// disconnected or failed sensor loop, not a real low-pressure reading.
/// Still keyed on the nominal 4-20mA/shunt math (not the `pressure_bar`
/// calibration above) since the 4mA floor is a property of the loop
/// itself, unrelated to how raw voltage maps to bar.
pub fn pressure_sensor_fault(raw_voltage: f32) -> bool {
    // 2.0, not 3.5: on the breadboard a healthy sensor at atmospheric
    // reads ~3.7mA (see PRESSURE_CAL_V_AT_ZERO_BAR), dipping to ~2.7mA in
    // glitches while the solenoid is energised. A broken loop reads ~0mA,
    // so this still catches it easily.
    loop_current_ma(raw_voltage) < 2.0
}

/// Convert the NTC divider voltage (probe to GND, `NTC_PULLUP_OHMS` from
/// 3V3 — see config.rs) into a temperature in °C. The 1k series resistor
/// into the pin carries no current worth counting, so it's ignored.
///
/// NTC_BETA is currently a placeholder (4000.0) — unconfirmed against the
/// actual probe datasheet. Do not trust shutdown thresholds derived from
/// this until it's verified.
pub fn ntc_temp_c(raw_voltage: f32) -> f32 {
    let r_ntc = NTC_PULLUP_OHMS * raw_voltage / (NTC_SUPPLY_V - raw_voltage);
    1.0 / (libm::logf(r_ntc / NTC_REF_OHMS) / NTC_BETA + 1.0 / 298.15) - 273.15
}

/// True if the divider voltage says the probe is open or shorted rather
/// than reporting a temperature. With the 10k pull-up, room temperature is
/// already ~3.0V and a 0°C probe ~3.2V, so "open" only starts above that:
/// 3.25V is roughly -5°C (not a real operating condition for this machine).
/// Below 0.05V is a probe resistance under ~150Ω, i.e. >250°C or a short.
pub fn ntc_sensor_fault(raw_voltage: f32) -> bool {
    !(0.05..=3.25).contains(&raw_voltage)
}

/// Shaft speed from the VFD's running frequency (2-pole motor, slip
/// assumed proportional to frequency — close enough for a display).
pub fn motor_rpm(hz: f32) -> f32 {
    hz / MOTOR_RATED_HZ * MOTOR_RATED_RPM
}

/// ESTIMATED electrical power in kW from running frequency and output
/// current: sqrt(3) * V * I * PF, with output voltage assumed to follow
/// V/f linearly up to `MOTOR_RATED_V`. See config.rs — the SKI-70's known
/// monitor registers have no power reading of their own.
pub fn motor_kw_estimate(hz: f32, amps: f32) -> f32 {
    let volts = MOTOR_RATED_V * (hz / MOTOR_RATED_HZ).min(1.0);
    1.732 * volts * amps * MOTOR_POWER_FACTOR / 1000.0
}

pub struct SensorData {
    pub pressure_1_bar: f32, // primary — control
    pub pressure_2_bar: f32, // secondary — log/display only
    pub temp_rotor_c: f32,   // rotor housing — compressor shutdown
    pub temp_motor_c: f32,   // motor winding — VFD derate
}

/// If the two pressure readings diverge by more than this, raise a
/// display/log-only mismatch warning (not a control fault).
pub const PRESSURE_MISMATCH_THRESHOLD_BAR: f32 = 0.5;
