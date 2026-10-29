//! Configurable constants. See CLAUDE.md for rationale on each value.

// Pressure thresholds (bar). Re-tuned 2026-09-10 — P_MODULATE_START_BAR
// (the PID's actual setpoint, see state_machine::Controller::new) was
// 6.5, well below the system's documented 8 bar operating pressure (see
// CLAUDE.md "System"); P_UNLOAD_BAR/P_MAX_FAULT_BAR didn't leave headroom
// above it either. Now: setpoint 8.0, unload-idle threshold 9.0, hard
// fault 9.5 — under the compressor's 10 bar rating.
pub const P_UNLOAD_BAR: f32 = 9.0;
pub const P_MAX_FAULT_BAR: f32 = 9.5;
pub const P_MIN_BUILD_BAR: f32 = 2.0;
pub const P_MODULATE_START_BAR: f32 = 8.0;

// Operator-adjustable PID setpoint (HMI nSetPoint, bSetPointMinus/Plus).
// P_MODULATE_START_BAR is now just the first-boot default; the live value
// is persisted alongside the counters (storage.rs). The restart and
// unload-idle thresholds follow it rather than staying fixed, so a lower
// setpoint doesn't leave the compressor waiting for a 6 bar restart point
// it would never fall to:
//   restart (Idle -> start)  = setpoint - LOAD_HYSTERESIS_BAR   (8.0 -> 6.0, as before)
//   unload-idle threshold    = min(setpoint + UNLOAD_MARGIN_BAR, P_UNLOAD_BAR)
// SETPOINT_MAX_BAR stays a step below P_UNLOAD_BAR so there's always room
// between the setpoint and the unload-idle/fault thresholds above it.
pub const SETPOINT_MIN_BAR: f32 = 4.0;
pub const SETPOINT_MAX_BAR: f32 = 8.5;
pub const SETPOINT_STEP_BAR: f32 = 0.5;
pub const LOAD_HYSTERESIS_BAR: f32 = 2.0;
pub const UNLOAD_MARGIN_BAR: f32 = 1.0;
// Setpoint is written to flash this long after the last +/- press, so
// tapping through several steps costs one flash write, not one per tap.
pub const SETPOINT_PERSIST_DELAY_MS: u64 = 5_000;

// Temperature thresholds (°C). Rotor housing / oil temp probe (not yet
// physically installed — missing from the schematic entirely, see
// CLAUDE.md "Known firmware gap"). T_ROTOR_MAX_C is a hard cutoff:
// industry shutdown setpoints for oil-flooded rotary screw compressors
// typically run 100-115°C, so 105 sits comfortably mid-range.
// T_ROTOR_SAFE_RESET_C is the dismissible-cooldown re-arm point —
// state_machine::Controller::clear_fault refuses to clear a RotorOverTemp
// fault until the probe reads back down to this, not just below the trip
// point, so an operator can't immediately re-fault the machine right at
// the boundary.
pub const T_ROTOR_MAX_C: f32 = 105.0;
pub const T_ROTOR_SAFE_RESET_C: f32 = 90.0;
pub const T_MOTOR_DERATE_START_C: f32 = 100.0;
pub const T_MOTOR_DERATE_FULL_C: f32 = 120.0;
pub const T_MOTOR_FAULT_C: f32 = 130.0;

// VFD speed limits (Hz) — nominal; actual limits are re-read at startup
// from VFD registers 61453 (P0-12) / 61455 (P0-14).
pub const VFD_FREQ_MIN_HZ: f32 = 35.0;
pub const VFD_FREQ_MAX_HZ: f32 = 50.0;
pub const VFD_FREQ_IDLE_HZ: f32 = 35.0;

// Timing (ms)
pub const VFD_ACCEL_MS: u64 = 5500;
pub const UNLOAD_STOP_MS: u64 = 2000;
pub const VFD_DECEL_MS: u64 = 5500;
pub const P_BUILD_TIMEOUT_MS: u64 = 30_000;
pub const IDLE_TIMEOUT_MS: u64 = 300_000;
pub const ADC_SAMPLE_MS: u64 = 500;
pub const BUTTON_DEBOUNCE_MS: u64 = 50;

// Rolling 60-minute window, not a fixed clock-hour bucket (a clock-hour
// bucket allows a 2x burst across the boundary). Clearable via HMI.
pub const MAX_STARTS_PER_HOUR: u32 = 25;
pub const STARTS_WINDOW_MS: u64 = 3_600_000;

// Speed-loop PID (Hz output), active during State::Running, holding
// pressure at P_MODULATE_START_BAR. UNCONFIRMED — starting point only,
// needs tuning against the real plant.
pub const PID_KP: f32 = 8.0; // Hz per bar of error
pub const PID_KI: f32 = 0.5; // Hz per bar-second of accumulated error
pub const PID_KD: f32 = 0.0; // disabled — pressure ADC has no filtering yet, D would amplify noise

// NTC — BETA IS UNCONFIRMED. Verify against probe datasheet or measure
// resistance at two known temperatures before trusting shutdown thresholds.
// Breadboard build: 100k (R25) probe to GND, 10k pull-up from 3V3, 1k
// series + 100nF into GPIO1. The 10k (not 100k) pull-up puts ~1.2V on the
// pin at the 105°C trip point instead of ~0.2V, where the ADC resolves it
// far better; the price is that room temperature already sits at ~3.0V.
pub const NTC_BETA: f32 = 4000.0;
pub const NTC_REF_OHMS: f32 = 100_000.0; // probe resistance at 25°C
pub const NTC_PULLUP_OHMS: f32 = 10_000.0;
pub const NTC_SUPPLY_V: f32 = 3.282; // measured on the breadboard 3V3 rail 2026-10-29
// Refuse to run (TempSensorFault) without a plausible rotor/oil reading.
// Set false only to bench-test with no probe fitted.
pub const TEMP_SENSOR_REQUIRED: bool = true;

// Motor nameplate (WEG W22 7.5kW, 400V delta, 2-pole) — for the HMI's
// nMotorSpeed (rpm from the VFD's running frequency) and nMotorKw. The
// SKI-70's monitor block we know about (see MODBUS_MONITOR_BASE_REG) has
// frequency, current and bus voltage but no output power, so kW is
// ESTIMATED: sqrt(3) * V * I * PF, with V scaled linearly with frequency
// (V/f control). MOTOR_POWER_FACTOR is a typical loaded figure, not
// measured — expect the display to read high at light load.
pub const MOTOR_RATED_HZ: f32 = 50.0;
pub const MOTOR_RATED_RPM: f32 = 2940.0;
pub const MOTOR_RATED_V: f32 = 400.0;
pub const MOTOR_POWER_FACTOR: f32 = 0.85;

// Pressure loop shunt (R2, GPIO0 to GND) — corrected 2026-09-10 to match
// the fabricated board's actual populated value (100Ω per bom.csv);
// previously assumed 165Ω, which was never what got built.
pub const PRESSURE_SHUNT_OHMS: f32 = 100.0;

// Empirical gain correction for the pressure ADC channel, applied to
// raw_voltage in main.rs right after the raw_mv->V unit conversion, ahead
// of everything else (loop_current_ma, pressure_bar). Even with
// AdcCalCurve applied (see main.rs), this chip's ADC still reads ~3.9x
// low on this channel. Espressif's own ESP-IDF docs (adc_calibration.html)
// say per-chip ADC calibration eFuse data is guaranteed burned in at
// manufacturing, which rules out "this chip just lacks calibration data"
// as the explanation — more likely a bug in esp-hal 1.0's eFuse-decode
// path for the C6 specifically (a young release) than missing factory
// data, though the exact mechanism isn't confirmed. Derived 2026-09-10
// from ONE simultaneous comparison against an in-series ammeter (true
// 4.200mA vs 1.080mA implied by the uncorrected reading at the same
// instant) — confirmed as a pure gain error, not an offset, since a
// separate disconnect test already showed true 0V correctly reads back
// as 0V. Single-point gain estimate: reasonable given that, but re-check
// against a second ammeter reading at a different current if this ever
// seems off again.
// 2026-10-29, breadboard build: NOT needed. With the old 4.200/1.080 the
// sensor at atmospheric showed 11.0 bar on the HMI, i.e. a raw ~0.37V —
// the 4mA live zero read correctly. So the ~3.9x was a fault of the old
// PCB's pressure input, not esp-hal's calibration. Re-check with an
// in-series ammeter when calibrating PRESSURE_CAL_* below.
pub const PRESSURE_ADC_GAIN_CORRECTION: f32 = 1.0;

// Two-point linear calibration for sensors::pressure_bar: raw ADC volts
// at 0 bar gauge, and volts per additional bar. Currently just the
// nominal 4-20mA/PRESSURE_SHUNT_OHMS math for a 0-17 bar transducer
// (0.4V = 4mA * 100Ω; 1.6V span over 17 bar = (20-4)mA * 100Ω), so
// behaviour is unchanged until real numbers replace these — take a
// reading at a known bar value against the physical oil-filled gauge
// once the transducer's mounted on the real machine (two readings at
// two different pressures gives V_PER_BAR directly as the slope between
// them; one of those readings can be atmospheric = 0 bar gauge, giving
// V_AT_ZERO_BAR directly, if convenient).
//
// 2026-10-29, breadboard: zero measured — 0.378V steady (32-sample trimmed
// mean) with the sensor open to the room, vs 0.400V nominal. Treated as a
// gain error (shunt tolerance / ADC gain, ~0.945x) rather than an offset, so
// the span is scaled by the same ratio. That's an assumption until a
// second point exists: check against a known pressure (the machine's gauge)
// and replace V_PER_BAR with the measured slope.
pub const PRESSURE_CAL_V_AT_ZERO_BAR: f32 = 0.378;
pub const PRESSURE_CAL_V_PER_BAR: f32 = 1.6 / 17.0 * (0.378 / 0.400);

// GPIO pin assignments (ESP32-C6-DevKitC-1). Pure documentation — Rust can't
// generically index `peripherals.GPIOn` by a runtime/const u8, so these exist
// to keep one canonical reference next to the peripheral-construction call
// sites in main.rs. See CLAUDE.md for the full pin table and rationale.
//
// GPIO24-30 are wired internally to the module's SPI flash on every C6
// module/devkit and are NOT usable as GPIO at all — the four pins below
// marked "moved" were originally specced on that reserved range and have
// been reassigned to free pins on the same header.
pub const PRESSURE_SENSOR_GPIO: u8 = 0; // ADC1 ch0, 4-20mA loop
pub const TEMP_SENSOR_GPIO: u8 = 1; // ADC1 ch1, NTC divider (breadboard build), 11dB
// GPIO10 (old MAX3485 DE+RE) is unused on the breadboard build: the TTL-RS485
// module switches direction by itself off the TX line.
// Corrected 2026-09-09 — confirmed against the fabricated PCB's netlist
// (Main/production/netlist.ipc): MAX3485 DI/RO actually land on GPIO19/18,
// not GPIO23/22 (those instead go to the Nextion header, J3). Previously
// swapped with NEXTION_TX_GPIO/NEXTION_RX_GPIO below.
pub const MODBUS_DI_GPIO: u8 = 18; // MAX3485 DI  (MCU UART1 TX)
pub const MODBUS_RO_GPIO: u8 = 19; // MAX3485 RO  (MCU UART1 RX)
// Confirmed 2026-08-30 (had this backwards before): start-assist /
// blow-down-to-intake valve. NOT a pressure-regulation valve — recirculates
// discharge air back to the intake during motor start-up to cut starting
// torque, then closes once the motor's up to speed so real compression can
// begin. See CLAUDE.md "Capacity control strategy". Moved from GPIO26
// (SPI flash). Polarity: see UNLOAD_VALVE_ENERGISE_TO_OPEN below.
pub const SOLENOID_UNLOAD_GPIO: u8 = 20;
// true  = normally-closed valve: GPIO20 high OPENS it (unloads). Powered
//         idle, during the start ramp, stopping and in Fault; unpowered
//         while compressing. Power loss leaves it SHUT (no blow-down).
// false = normally-open valve: GPIO20 high CLOSES it. Unpowered = open =
//         unloaded, the fail-safe arrangement the old notes assumed.
// Confirmed 2026-10-29 by the operator: normally-OPEN, and it must be
// CLOSED (energised) most of the time to hold air: at rest and while
// compressing. Open only during the start ramp (first VFD_ACCEL_MS of
// Unloading, so the motor spins up unloaded), while stopping, in Fault,
// and on power loss. The coil (24V DC, ~40Ω, ~14W) is therefore on almost
// permanently — hence the PWM hold in valve.rs.
pub const UNLOAD_VALVE_ENERGISE_TO_OPEN: bool = false;
// Waste condensate drain — NOT a load/unload air valve. Manual/timed
// trigger only (see DRAIN_PULSE_MS below), not part of automatic capacity
// control. Moved from GPIO25 (SPI flash).
pub const SOLENOID_DRAIN_GPIO: u8 = 21;
// Confirmed 2026-10-29 by the operator: the drain valve is normally-OPEN,
// so GPIO21 is held HIGH (valve shut) at all times except while bPurge is
// held. Power loss or a reset therefore opens the drain and vents the tank
// through it until the firmware is back up.
pub const DRAIN_VALVE_ENERGISE_TO_OPEN: bool = false;
// Corrected 2026-09-09 — see MODBUS_DI_GPIO/MODBUS_RO_GPIO above: swapped
// with the Modbus pins after confirming the fabricated PCB's actual wiring.
pub const NEXTION_TX_GPIO: u8 = 23;
pub const NEXTION_RX_GPIO: u8 = 22;

// Modbus baud must match the SKI-70's Pd-00 (=5, 9600).
pub const MODBUS_BAUD_RATE: u32 = 9600;

// Nextion link. The display powers up at NEXTION_BOOT_BAUD (factory default,
// and what the HMI's Program.s sets); the firmware runs the link at
// NEXTION_BAUD_RATE and switches the display over with `baud=`. The link
// task re-checks every NEXTION_RESYNC_MS and repeats the switch if the
// display has dropped back (power blip, reboot) — see nextion.rs.
pub const NEXTION_BOOT_BAUD: u32 = 9600;
pub const NEXTION_BAUD_RATE: u32 = 115_200;
pub const NEXTION_RESYNC_MS: u64 = 3_000;
// Every field is re-sent this often even if unchanged, in case the display
// reloaded its page (resetting fields to their Editor defaults) without us
// noticing.
pub const NEXTION_FULL_REFRESH_MS: u64 = 10_000;
// HMI value scaling: `.val` is an integer, so fractional values are sent
// multiplied by these. Use Xfloat components with vvs1=1 for nPressure,
// nSetPoint and nMotorKw (one decimal place); nMotorSpeed is whole rpm.
pub const NEXTION_PRESSURE_SCALE: f32 = 10.0;
pub const NEXTION_SETPOINT_SCALE: f32 = 10.0;
pub const NEXTION_KW_SCALE: f32 = 10.0;

// Modbus (SKI-70, empirically discovered register map)
pub const MODBUS_SLAVE_ADDR: u8 = 1;
pub const MODBUS_CONTROL_WORD_REG: u16 = 8193; // 1 = run, 5 = stop
pub const MODBUS_FREQ_SETPOINT_REG: u16 = 4097; // Hz * 200
pub const MODBUS_MONITOR_BASE_REG: u16 = 28673; // freq/100, current/100, bus voltage/10
// NOTE: corrected 2026-08-30 — these were swapped (P0-12 is MIN, P0-14 is
// MAX, per the SKI-70 Modbus reference; the previous version of this file
// had it backwards).
pub const MODBUS_FREQ_MIN_REG: u16 = 61453; // P0-12
pub const MODBUS_FREQ_MAX_REG: u16 = 61455; // P0-14
pub const MODBUS_MAX_REGS_PER_READ: u8 = 5; // hardware limitation of the SKI-70

// VFD-side params confirmed set (2026-08-30): P0-02=2 (comms cmd source),
// P0-03=9 (comms freq source), Pd-00=5 (9600 baud), Pd-01=3 (8N1),
// Pd-02=1 (slave addr 1). Pd-params may reset on power-cycle — these are
// NOT re-written by firmware (chicken-and-egg over the same bus); if comms
// fail after a power outage, reverify these on the VFD's own keypad first.

// Hold-to-act HMI buttons (bPurge -> drain, bUnload -> start-assist valve
// forced open). Each valve follows press/release, but auto-releases after
// this long: a release event lost to a display glitch or reboot mid-press
// must not leave the drain blowing or the compressor unloaded forever.
pub const DRAIN_MAX_HOLD_MS: u64 = 30_000;
pub const UNLOAD_MAX_HOLD_MS: u64 = 30_000;

// Solenoid PWM hold (valve.rs): full drive for VALVE_PULL_IN_MS, then
// VALVE_HOLD_DUTY_PCT. 1kHz, not an inaudible 20kHz: the breadboard's
// flyback diodes are 1N400x, whose slow reverse recovery is fine at 1kHz
// but not at 20kHz (see valve.rs). The coil may hum faintly while holding.
// Tune the hold duty on the real valves: lower it until a valve drops out
// (or leaks) when held, then set it ~15 points above that. 50% is a safe
// start for most valve coils.
pub const VALVE_PWM_HZ: u32 = 1_000;
// 500, not 200: on the bench the start-assist valve sometimes hadn't fully
// seated after 200ms and then couldn't finish pulling in at hold duty.
pub const VALVE_PULL_IN_MS: u64 = 500;
pub const VALVE_HOLD_DUTY_PCT: u8 = 50;
// The coil wrecks the analog readings on the breadboard: PWM switching put
// the NTC at 100-135°C (for 25°C), and its current through the shared
// ground pulls pressure 0.3-1.7 bar low. So holding valves are switched
// off for each ADC sample burst — see PwmValve::quiet — after letting the
// sensor RC filters (≤1ms) settle for this long.
// 12, not the RC filters' ~1ms: measured on the breadboard, the pressure
// input is still settling 5ms after switch-off and flat from ~9ms. The
// valve doesn't notice: its freewheeling current barely decays in 20ms.
pub const VALVE_QUIET_SETTLE_MS: u64 = 12;

// A sensor fault, over-pressure or over-temperature must persist for this
// many consecutive control ticks (ADC_SAMPLE_MS each, so 1.5s) before it
// latches a fault — one glitched reading (a solenoid switching, a loose
// contact) mustn't stop the machine. Pressure and oil temperature can't
// physically move far in 1.5s on this machine.
pub const FAULT_CONFIRM_TICKS: u8 = 3;

// Nextion component ids are no longer configured here: the link task asks
// the display for each button's id by name (`get bUnload.id`) every time it
// (re)connects, so re-ordering components in the Editor can't break input.
