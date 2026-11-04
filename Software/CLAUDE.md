# ESP32 Screw Compressor Controller — Project Context

Replacement controller for a Ceccato ES3000-based 3-phase screw compressor.
Original ES3000 board is damaged (blown triac, damaged optocouplers/traces).

## Hardware target

- Since 2026-11-04 (branch `s3-port`): **Waveshare ESP32-S3-Touch-LCD-4**
  (ESP32-S3-WROOM-1-N16R8, 480x480 ST7701S RGB panel, GT911 touch, TCA9554 expander)
  plus the SMD **piggyback** I/O board in `../Piggyback/` (2x ADS1115,
  PCA9685, watchdog + hardware over-temp interlock). Every S3 GPIO is used by
  the Waveshare board, so the piggyback hangs off its I2C bus; see
  `../Piggyback/pin-allocation.md`. The ESP32-C6 + Nextion build lives on
  `main` until the S3 board is proven.
- Framework: Embassy via `esp-rtos`, `esp-hal` 1.0, `no_std`; UI is Slint
  (software renderer) into a PSRAM framebuffer (`src/board/display.rs`).
- Toolchain: Espressif's Xtensa fork (`espup install --targets esp32s3`,
  then `. ~/export-esp.sh`). If espup fails with a missing liblzma/libiconv,
  run it with `DYLD_LIBRARY_PATH=/opt/homebrew/opt/xz/lib:/opt/homebrew/opt/libiconv/lib`.
- Flash/log over the Waveshare's USB-C (USB-Serial-JTAG): `cargo run --release`.
  UART0 (GPIO43/44) is Modbus.

## System

- VFD: SAKO SKI-70 (400V), controlled entirely over Modbus RTU — no analog
  speed signal, no digital run/stop wiring. Run/stop and frequency setpoint
  are written as Modbus registers; frequency/current/bus voltage read back
  over the same bus.
- Motor: WEG W22 Premium 7.5kW, 400V Delta, 2-pole, 50Hz, 2940 RPM.
- Compressor: rated 10 bar, operating pressure 8 bar.
- Confirmed 2026-08-30: **there is no physical load/unload (pressure
  regulation) valve in this design at all.** Capacity control is 100%
  VFD speed (PID), see "Capacity control strategy" below. There are two
  unrelated solenoids, both auxiliary to that loop:
  - **Start-assist valve** (GPIO20): blow-down-to-intake — recirculates
    discharge air back to the compressor's own intake during motor
    start-up, so the motor doesn't have to develop full compression
    torque immediately. Wired into the startup sequence, see "Capacity
    control strategy". Electrically: normally-open, 24VDC, MOSFET-driven
    (IRLML2502), de-energised = open = safe/fail state — this was
    originally documented as the "Discharge solenoid" before the
    start-assist function was identified; same physical properties,
    corrected purpose. Polarity (does energizing it close the path?)
    assumed, not yet verified on hardware.
  - **Condensate drain** (GPIO21): drains waste water from the tank.
    Manual/timed only (HMI button, `DRAIN_PULSE_MS` auto-close) — not
    part of automatic capacity control at all.
- Power: single 24VDC DIN rail PSU (Meanwell HDR-30-24) feeds everything —
  solenoid MOSFET directly, 4-20mA loop supply directly, 5V buck (TPS5430)
  for the ESP32 rail. NOTE: this section previously described an isolated
  NMA0505SC (5V→5V) supply feeding an ADM2483BRWZ's VDD2 — confirmed
  2026-08-30 that the RS485 part actually used is a MAX3485 (non-isolated,
  see below), which needs no isolated supply. Verify whether the
  NMA0505SC is populated/needed for anything else on this board, or
  whether that part of the design is now dead.
- RS485: MAX3485 (non-isolated) transceiver. Powered from the 3.3V ESP32
  rail directly — no isolation. GPIO pin wiring below is confirmed
  against this part.

### GPIO pin assignments (confirmed against ESP32-C6-DevKitC-1, 2026-08-30)

GPIO24–30 are wired internally to the module's SPI flash on every C6
module/devkit and are physically unusable as GPIO — not a footprint issue,
a chip-level reservation. Four signals originally specced in that range
were moved to free pins on the same header (J3). Source: esp-hal's
`esp-metadata/devices/esp32c6.toml` (mirrors Espressif's own pin tables)
cross-checked against the DevKitC-1 user guide's header pinout.

| Signal | GPIO | Notes |
|---|---|---|
| Pressure sensor (4-20mA loop) | GPIO0 | ADC1 ch0 |
| Temp sensor (NTC bodge) | GPIO1 | ADC1 ch1 — **not yet physically wired**, placeholder |
| Modbus DE+RE (tied) | GPIO10 | direction control, MAX3485 |
| Modbus DI (MCU TX) | GPIO18 | corrected 2026-09-09 — see note below |
| Modbus RO (MCU RX) | GPIO19 | corrected 2026-09-09 — see note below |
| Solenoid — start-assist (blow-down to intake) | GPIO20 | moved from GPIO26 (SPI flash); confirmed 2026-08-30, see above |
| Solenoid — condensate drain | GPIO21 | moved from GPIO25 (SPI flash); manual/timed only, see above |
| Nextion UART TX | GPIO23 | corrected 2026-09-09 — see note below |
| Nextion UART RX | GPIO22 | corrected 2026-09-09 — see note below |

**Corrected 2026-09-09**: Modbus and Nextion UART pins were swapped from
what's documented above until this date. `vfd-run-test` (see "Not yet
done") got a hard Modbus `Timeout` on every attempt despite correct RS485
wiring and confirmed-good VFD-side Pd-/P0- params. Cross-checking the
fabricated PCB's own netlist (`Main/production/netlist.ipc`) against the
MAX3485 (`U3`)'s pins showed RO/DI actually landed on GPIO19/GPIO18, and
the Nextion header (`J3`) landed on GPIO23/GPIO22 — the reverse of the
firmware's prior assumption (itself inherited from the original ESP32
schematic's GPIO23/22 assignment, which apparently never got carried
through correctly when GPIO24's SPI-flash conflict forced pins to move
during the C6 respin). DE/RE on GPIO10 was correct the whole time. Fixed
in `main.rs`, `config.rs`, `modbus_diag.rs`, `vfd_run_test.rs`.

Reserved, do not reassign anything onto these: GPIO4, 5, 8, 9, 15
(strapping pins — GPIO9 is the boot-mode strap, equivalent to the
original ESP32's GPIO0), GPIO12/13 (native USB D-/D+, used for
probe-rs flashing and the serial log console), GPIO24–30 (SPI flash).

Modbus and Nextion baud rates are both placeholder 9600 (UNCONFIRMED —
see `config::MODBUS_BAUD_RATE` / `config::NEXTION_BAUD_RATE`; verify
against the SKI-70's actual comms settings and the Nextion display config).

### TPS5430 cross-wiring claim — REFUTED 2026-09-11, see below

Previously documented here as an open bug (feedback divider on BOOT
instead of VSENSE, bootstrap cap on VSENSE/PH instead of BOOT/PH,
symptom: 5V rail swinging 5.1–6.6V). Checked directly against the
schematic/netlist while chasing a "noisy power rails, garbage UART
signal on a scope" report: **U4 pin 4 is VSENSE, and R13/R14/R15 all
land on that exact net (`NET-(U4-VSENSE)`); U4 pin 1 is BOOT, and C7
lands between that net and D4's cathode (`NET-(D4-K)`, the switching/PH
node)** — both correct, standard TPS5430 wiring, the opposite of what
this section used to claim. Either this was misdiagnosed originally, or
it was based on something (an earlier schematic revision? a hand
inspection that didn't match this file?) other than what's actually in
`Main.kicad_sch` now. If rail noise is real, it isn't this — look at
actual fabricated-board-vs-schematic mismatches, decoupling, ground
layout, or the 24V mains PSU's own ripple instead.

### Known firmware gap (open, unresolved as of last session)

The spun PCB omitted the temperature sensor circuit entirely. Plan is to
bodge NTC dividers onto 2 spare GPIOs rather than the original 3-channel
design (2× rotor housing + 1× motor winding NTC). With only 2 channels,
keep rotor primary (safety-critical shutdown) and motor winding (VFD
derate); drop rotor secondary (was log/display-only with a mismatch
warning, no control logic). GPIO plan needs redoing for the C6: it has a
single ADC1 with 7 channels on GPIO0–6 and no ADC2, so the "GPIO32–39,
ADC2 unusable with WiFi" constraint from the original ESP32 (Xtensa) plan
does not apply — pick 2 of GPIO0–6 for the NTC bodge instead. Use 11dB
attenuation.

## Sensors

### Pressure (4-20mA loop, 0–17 bar), applies to both pressure sensor channels

Fixed two real bugs here 2026-09-10, found while debugging why `pressureTxt`
stayed on `"NOT CONNECTED"` after the transducer (part `12147851`-type,
brown = +24V to `J2` pin 2, green = sense to `J2` pin 1 — confirmed
against the schematic, matches R2/GPIO0) was correctly wired: the shunt
(R2) is actually **100Ω per bom.csv**, not 165Ω as previously assumed
here, and the conversion was missing the volts→amps→milliamps `* 1000.0`
entirely — `current_ma` was coming out ~1000x too small, permanently
below the 4mA-floor fault threshold regardless of what was connected.

```rust
let raw_voltage: f32 = adc_read(gpio_pin); // 0.0–3.3V, 100Ω shunt (R2)
let current_ma: f32 = raw_voltage / PRESSURE_SHUNT_OHMS * 1000.0; // pressure_sensor_fault's 4mA-floor check only
let pressure_bar: f32 = (raw_voltage - PRESSURE_CAL_V_AT_ZERO_BAR) / PRESSURE_CAL_V_PER_BAR;
```

**Still showing implausible readings after both bugs above were fixed**
(2026-09-10 bench test, sensor genuinely wired correctly, confirmed
against the schematic) — logged `raw_adc`/`raw_voltage` directly
(`main.rs`'s control loop, `pressure: raw_adc=... raw_voltage=...V`):
~1.6V at atmospheric, ~1.2V with the sensor fully unplugged from `J2`
(should read close to 0V open-circuit, given R2's pulldown — reading
close to the "connected" value instead suggests a floating/noisy ADC
input rather than a clean loop-current reading), briefly ~2.1V while
blowing into the sensor by mouth (equivalent to several bar on a 0-17
bar scale, physically implausible for human breath pressure). Not
resolved — next step is an independent multimeter reading directly
across R2 (or GPIO0-to-GND) to tell whether the ESP32's own ADC reading
is trustworthy at all here, before trusting any calibration derived from
it. `pressure_bar` was refactored to go through a two-point linear
calibration (`PRESSURE_CAL_V_AT_ZERO_BAR`/`PRESSURE_CAL_V_PER_BAR` in
config.rs) instead of the nominal 4-20mA math directly, so a real
calibration (from comparing against the physical oil-filled gauge once
mounted on the machine) can just replace those two constants — current
values are just the nominal math restated, not yet real calibration
data, and don't fix the implausible-reading problem above by themselves.

**Root cause found and fixed 2026-09-10**: a multimeter directly across
R2/GPIO0, read at the same instant the serial console logged
`raw_voltage`, showed ~0.03V real vs ~1.2V logged — same node, same
moment, ~40x off, ruling out a sensor/wiring problem or a timing
mismatch between separate tests. Traced to `main.rs`'s ADC setup using
plain `enable_pin` (esp-hal's default, no calibration scheme) — per
esp-hal's own `read_oneshot` source, this leaves the ADC's actual
**hardware** calibration/init-code register at an uncalibrated default
(`ADCI::set_init_code(pin.cal_scheme.adc_cal())`, called with a no-op
scheme), not just a missing software correction — consistent with an
error this large and this consistent. Fixed by switching to
`enable_pin_with_cal::<_, AdcCalCurve<_>>` (esp-idf's factory-derived
per-attenuation error-correction polynomial; ESP32-C6 has coefficients
for all four attenuation levels including our 11dB). This also changes
what `read_oneshot` returns — already-calibrated **millivolts**, not a
raw ADC count — so the raw-to-volts conversion in `main.rs` changed from
`raw_adc/4095*3.3` to `raw_mv/1000.0` to match.

After that fix, `raw_mv` read back as exactly `0` always, zero movement
even while blowing into the sensor by mouth — a second, separate issue:
**11dB attenuation** (~2.5-3.9V full scale) wastes almost the entire
12-bit range on a signal that's nominally 0.4-2.0V (4-20mA × 100Ω) at
most, and was currently much smaller than that again (loop not
conducting normally per the multimeter readings above) — a few-mV real
signal was landing at 0-1 raw counts, pinned at the floor regardless of
input. Switched to **`Attenuation::_6dB`** (~1.75-2.2V full scale, still
has `AdcCalCurve` coefficients on the C6) as a much closer match to the
real expected range.

**Both ADC fixes confirmed real and working** (2026-09-10 bench retest):
readings now respond sensibly to physical state — connected/atmospheric
reads `~0.108V`, drops cleanly to `~0V` when the sensor is unplugged from
`J2` (matches the open-circuit-through-R2 expectation exactly), no more
noise/pinning. Still one gap open, smaller and now directionally
sane rather than wild: `0.108V` across the 100Ω shunt is `~1.08mA`, but a
healthy 4-20mA sensor at atmospheric should show the "live zero" floor of
`~4mA` (`0.4V`) — `pressure_sensor_fault` correctly flags this as a fault
(current below the 3.5mA margin) given that reading, which is arguably
correct firmware behaviour even though the root cause (why only ~1mA
instead of ~4mA) isn't resolved yet. Not yet checked: whether `J2` pin 2
(+24V, brown) is actually delivering a healthy 24V to the transducer —
if the supply itself is weak/absent, the sensor can't produce its full
rated output regardless of anything on the ESP32 side.

**Supply confirmed healthy (23.784V at J2 pin 2), sensor confirmed
healthy too** — measuring loop current directly with an ammeter in
series (breaking the loop at the green/sense wire, bypassing R2/the ADC
entirely) showed 4.220mA at atmospheric (exactly the correct "live zero"
floor) rising to 4.3mA blowing hard by mouth (~0.085 bar equivalent on a
0-17 bar scale — physically plausible for human breath). So the
transducer, wiring, and supply were all fine the whole time; the
remaining gap was purely in the ESP32's own ADC measurement chain.
Comparing the ammeter (4.200mA) against `main.rs`'s logged `implied_ma`
at the same instant (1.080mA) gave a clean **~3.889x gain error**
remaining even with `AdcCalCurve` applied. Espressif's own ESP-IDF docs
say per-chip ADC calibration eFuse data is guaranteed burned in at
manufacturing, ruling out "this chip lacks calibration data" — more
likely a bug in esp-hal 1.0's eFuse-decode path for the C6 (a young
release) than missing factory data, though unconfirmed. Fixed with an
empirical `PRESSURE_ADC_GAIN_CORRECTION` constant (config.rs, derived
from that one ammeter comparison) applied to `raw_voltage` in `main.rs`
right after the raw_mv→V conversion. Single-point gain estimate — solid
given the zero-point already tracked correctly on its own, but worth a
second ammeter check at a different current if it ever looks off again.
Confirmed working — steady-state readings now match the ammeter exactly
(`raw_mv=108` → `4.200mA`, bit-for-bit the ammeter's own reading).

**One more issue found in the same bench session**: occasional
single-sample spikes (a steady ~4.2mA reading jumping to a spurious
~9mA or ~38mA for exactly one sample, immediately back to normal next
cycle, never corroborated by the ammeter at the same moment) — exactly
what Espressif's ADC calibration docs warn about for this chip,
recommending a bypass cap on the input pad (not present on this board)
and "consider multisampling" in software. Fixed with median-of-5
sampling in `main.rs` (reads `pressure_pin` 5x back-to-back, sorts, takes
the middle value) — a median rejects a single-sample outlier outright
rather than just diluting its effect the way a plain average would. Same
approach should be applied to the temp channel once it's physically
wired, for the same reason.

Sensor 1 is primary for all control logic. Sensor 2 is secondary —
log/display only. If readings diverge by more than 0.5 bar, raise a
`SensorMismatch` warning (display/log only, not a fault).

### Temperature (NTC 100K), applies to all NTC channels

100K divider from 3.3V rail, ESP32 ADC reads the midpoint.

```rust
let raw_voltage: f32 = adc_read(gpio_pin); // 0.0–3.3V
let r_ntc: f32 = 100_000.0 * raw_voltage / (3.3 - raw_voltage);
let temp_c: f32 = 1.0 / (libm::logf(r_ntc / 100_000.0) / NTC_BETA + 1.0 / 298.15) - 273.15;
```

`NTC_BETA = 4000.0` is a placeholder — **needs confirming from the actual
probe datasheet or by measuring resistance at two known temperatures**
before this is trustworthy for the shutdown thresholds.

## Configurable constants (last agreed values)

```rust
// Pressure thresholds (bar). Re-tuned 2026-09-10 — P_MODULATE_START_BAR
// (the PID's actual setpoint) was 6.5, well below the system's own
// documented 8 bar operating pressure; P_UNLOAD_BAR/P_MAX_FAULT_BAR
// didn't leave headroom above it either.
const P_LOAD_BAR: f32           = 6.0;    // reload threshold — solenoid ON
const P_UNLOAD_BAR: f32         = 9.0;    // unload threshold — solenoid OFF
const P_MAX_FAULT_BAR: f32      = 9.5;    // over-pressure fault — under the 10 bar rating
const P_MIN_BUILD_BAR: f32      = 2.0;    // must reach within P_BUILD_TIMEOUT_MS after loading
const P_MODULATE_START_BAR: f32 = 8.0;    // PID setpoint — matches documented operating pressure

// Temperature thresholds (°C). Rotor/oil probe — not physically installed
// yet, missing from the schematic entirely (see "Known firmware gap").
// T_ROTOR_MAX_C is a hard cutoff (wired into Controller::tick as of
// 2026-09-10): industry shutdown setpoints for oil-flooded rotary screw
// compressors typically run 100-115°C. T_ROTOR_SAFE_RESET_C is the
// dismissible-cooldown re-arm point — Controller::clear_fault refuses to
// clear RotorOverTemp until the probe reads back down to this, not just
// below the trip point.
const T_ROTOR_MAX_C: f32          = 105.0; // compressor shutdown
const T_ROTOR_SAFE_RESET_C: f32   = 90.0;  // must cool to this before clear_fault re-arms
const T_MOTOR_DERATE_START_C: f32 = 100.0; // begin reducing max VFD frequency — NOT wired yet
const T_MOTOR_DERATE_FULL_C: f32  = 120.0; // clamp to VFD_FREQ_MIN_HZ — NOT wired yet
const T_MOTOR_FAULT_C: f32        = 130.0; // hard fault and stop — NOT wired yet (separate motor-winding probe, out of scope for now — only one rotor/oil probe is planned, see "Known firmware gap")

// VFD speed limits (Hz) — nominal; actual limits read dynamically at
// startup from VFD registers 61453/61455 (P0-12/P0-14)
const VFD_FREQ_MIN_HZ: f32 = 35.0; // minimum safe speed (~70%) — screw lube + motor cooling
const VFD_FREQ_MAX_HZ: f32 = 50.0; // rated speed
const VFD_FREQ_IDLE_HZ: f32 = 35.0;

// Timing (ms)
const VFD_ACCEL_MS: u64        = 5500;
const UNLOAD_STOP_MS: u64      = 2000;
const VFD_DECEL_MS: u64        = 5500;
const P_BUILD_TIMEOUT_MS: u64  = 30_000;
const IDLE_TIMEOUT_MS: u64     = 300_000; // auto-stop after 5 min in UNLOAD_IDLE
const ADC_SAMPLE_MS: u64       = 500;
const BUTTON_DEBOUNCE_MS: u64  = 50;

const MAX_STARTS_PER_HOUR: u32 = 25; // industry-typical cap for screw compressors; clearable via HMI

// Speed-loop PID (Hz output). UNCONFIRMED — starting point only, needs
// tuning against the real plant (pressure vessel volume, VFD accel rate).
const PID_KP: f32 = 8.0;  // Hz per bar of error
const PID_KI: f32 = 0.5;  // Hz per bar-second of accumulated error
const PID_KD: f32 = 0.0;  // derivative disabled initially — pressure ADC has no filtering yet, D would amplify noise

const NTC_BETA: f32       = 4000.0; // UNCONFIRMED — see note above
const NTC_REF_OHMS: f32   = 100_000.0;
```

## Capacity control strategy (confirmed 2026-08-30)

There is no load/unload (pressure-regulation) air valve. Capacity control
is 100% VFD speed, closed-loop PID. The start-assist valve (GPIO20) is a
separate concern, only active during the startup ramp — it does not
regulate running pressure.

1. **Idle**: motor stopped, VFD control word = stop. Waiting for enable
   (HMI) and pressure below `P_LOAD_BAR` (6.0), and starts-per-hour not
   exhausted.
2. **Unloading** (name predates the start-assist valve being identified —
   it's now accidentally apt again: the motor genuinely starts "unloaded"
   via that valve). On entry: start-assist valve opens, VFD run command
   issued, frequency setpoint driven open-loop (no PID yet) toward a fill
   speed. Once `VFD_ACCEL_MS` has elapsed (motor assumed up to speed),
   the start-assist valve closes and real compression begins. Must reach
   `P_MIN_BUILD_BAR` (2.0) within `P_BUILD_TIMEOUT_MS` total, else
   `FaultReason::PressureBuildTimeout`.
3. **Running**: PID takes over, holding pressure at `P_MODULATE_START_BAR`
   (6.5 bar — confirmed setpoint, matches the existing constant's own
   "start reducing VFD speed above this pressure" intent). PID output
   clamped to the *intersection* of our own `VFD_FREQ_MIN_HZ`/`MAX_HZ`
   safety bounds and whatever the VFD reports live from P0-12/P0-14 at
   startup — always the tighter of the two on each side.
4. **UnloadIdle** (name likewise carried over — no valve involved): entered
   when the PID is saturated at `VFD_FREQ_MIN_HZ` and pressure still
   climbs past `P_UNLOAD_BAR` (8.0) anyway — i.e. downstream demand is so
   low the compressor can't shed enough capacity even at minimum speed.
   VFD stays at `VFD_FREQ_MIN_HZ`. If demand doesn't resume (pressure
   doesn't drop) within `IDLE_TIMEOUT_MS` (5 min), transition to
   Stopping — cheaper than idling a 7.5kW motor for the sake of near-zero
   consumption. A future restart from Idle is subject to the
   starts-per-hour limit.
5. **Stopping**: start-assist valve re-opens (ready for next start, and
   bleeds discharge pressure back to intake during deceleration rather
   than leaving it trapped downstream), VFD decel/stop sequence
   (`VFD_DECEL_MS`, `UNLOAD_STOP_MS`).
6. **Fault**: `P_MAX_FAULT_BAR` (9.0) is a hard cutout regardless of state
   — stop immediately, re-open the start-assist valve (fail safe), don't
   wait for the PID or idle timeout to react.

Starts-per-hour: rolling 60-minute window (not a fixed clock-hour bucket —
a clock-hour bucket allows a 2x burst across the boundary). Hitting the
limit raises `FaultReason::StartRateLimitExceeded`; manually clearable
from the HMI, per the existing `FaultReason` variant. Total start count
and total runtime persist across power cycles (see Persistence below).

The condensate drain (GPIO21) is independent of all of the above — a
`Controller::trigger_drain()` HMI hook opens it for `DRAIN_PULSE_MS` then
auto-closes, regardless of compressor state.

## Persistence

Total start count and total accumulated runtime survive power-cycles,
stored in the SoC's default NVS data partition (found at boot via
`esp-bootloader-esp-idf`'s `partitions::read_partition_table`, so no
custom `partitions.csv` needed) using raw `esp-storage` reads/writes — not
ESP-IDF's actual NVS key-value format, just borrowing that partition's
reserved flash space for our own small fixed-offset record (magic +
sequence + counters + CRC32, one erase-modify-write per persist via
`embedded_storage::Storage`). Write frequency is low by design (on state
transitions and periodically during `Running`, not continuously) — well
within flash endurance over the controller's lifetime, so no ring-buffer
wear-leveling was needed.

## VFD Modbus (SKI-70, empirically discovered — not in manual)

- Serial: 9600 baud, 8N1, slave address 1
- Control word: register 8193 (1 = run, 5 = stop)
- Frequency setpoint: register 4097 (Hz × 200)
- Monitoring: registers 28673+ (running freq ÷100, current ÷100, bus voltage ÷10)
- Max 5 registers per read request (hardware limitation)
- VFD frequency limits, read at startup: min frequency register 61453
  (P0-12), max frequency register 61455 (P0-14) — corrected 2026-08-30,
  was previously swapped in `config.rs`
- VFD-side params confirmed set: P0-02=2 (comms cmd source), P0-03=9
  (comms freq source), Pd-00=5 (9600 baud), Pd-01=3 (8N1), Pd-02=1 (slave
  addr 1). These are Pd-params and **may reset on power-cycle** — reverify
  after any outage before trusting comms.

## Breadboard build + new HMI (2026-10-29) — supersedes the PCB notes below where they conflict

The failed PCB was replaced by a solder-breadboard build (same GPIOs):
auto-direction TTL-RS485 module (no DE/RE — GPIO10 unused, `SkiVfd::new`
takes just the UART), pressure via 100Ω shunt + 4.7k/100nF + 1N4148 clamp,
NTC 100k probe with a **10k** pull-up + 1k/100nF on GPIO1 at 11dB
(`NTC_PULLUP_OHMS`), IRLZ34N low-side valve drivers. `PRESSURE_ADC_GAIN_CORRECTION`
was measured on the old board — re-check it against a meter on GPIO0.

HMI components (page 0, ids learned at runtime via `get <name>.id`, so
no id constants): `nPressure`, `nSetPoint`, `nMotorKw` (Xfloat, value ×10),
`nMotorSpeed` (rpm), `tMotorNote`, `tStatus`, `bRunState` (press:
start/stop/reset), `bSetPointMinus`/`Plus` (press, ±0.5 bar, persisted),
`bUnload`/`bPurge` (press+release, hold-to-act, auto-release after 30s).
`src/nextion.rs` owns the whole UART: probes with `sendme` every 3s at
115200, and if the display doesn't answer, drops to 9600, sends
`baud=115200`, and re-inits (ids, full redraw, release held buttons).
Outbound is change-detected per attribute with a full refresh every 10s.
Restart/unload-idle thresholds now follow the setpoint (config.rs
`LOAD_HYSTERESIS_BAR`, `UNLOAD_MARGIN_BAR`). nMotorKw is an estimate
(√3·V·I·PF) — the known SKI-70 monitor registers have no power reading.

## Nextion HMI — current project: `ScrewCompressor.HMI` (2026-09-10)

Superseded the old bare-layout file below once a real HMI existed to
examine. Confirmed by extracting printable strings (`strings -n 2 -t d`)
plus empirical testing against real hardware (clicking buttons and
reading the frames back over serial) — not from any Nextion spec.

- **Component roster** (`page0`, two copies of it appear in the file —
  possibly a duplicate/backup page, not investigated further): `runBtn`,
  `stopBtn` (buttons) and `statusLbl`/`statusTxt`, `pressureLbl`/
  `pressureTxt`, `tempLbl`/`tmpTxt` (not `tempTxt`), `motorSpeedLbl`/
  `motorSpeedTxt` (not `motorSpeedStatus`) — each a label + a text field —
  plus `pressureGauge`, not yet driven by firmware.
- **Buttons already send native touch-event frames** — `0x65 <page>
  <component> <event type> 0xFF 0xFF 0xFF`, event type `0x01` = release —
  with zero Editor scripting needed. "Send Component ID" was already on.
  This makes the old marker/`printh` scheme below entirely unnecessary
  for this project; **do not paste those scripts in**, they're for the
  superseded file only. Ids confirmed empirically: page `0`, `runBtn` = 1,
  `stopBtn` = 2 (see `config::NEXTION_PAGE_MAIN`/`NEXTION_COMPONENT_*`).
  No `ClearFault`/`Drain` buttons exist in this project yet — add their
  ids the same way (click it, read the frame back over the serial console,
  see "watching logs live" note in Not-yet-done) once they do.
- **Inbound parsing**: `src/nextion.rs`'s `reader_task` collects the
  7-byte frame once it sees the leading `0x65`, verifies the trailing
  `0xFF 0xFF 0xFF`, and maps `(page, component)` release events to
  `Command::Start`/`Stop`. Every raw byte is also logged unconditionally
  (`nextion: rx 0xNN`) regardless of whether it parses, for live
  debugging over the serial console.
- **Outbound telemetry implemented**: `nextion::push_telemetry`, called
  every `ADC_SAMPLE_MS` cycle from `main.rs`. Pushes `pressureTxt`,
  `tmpTxt`, `motorSpeedTxt`, `statusTxt` as `objname.txt="..."`, plus a
  `.pco` (font color) command — red + `"NOT CONNECTED"` when that
  subsystem isn't found (pressure/temp sensor fault, VFD comms lost).
  `pressureTxt` when actually connected shows a three-tier color
  (`nextion::pressure_color`, 2026-09-10) against the *same* thresholds
  the control loop itself uses — green below `P_UNLOAD_BAR`, amber from
  there up to `P_MAX_FAULT_BAR`, red (reusing `COLOR_WARN`, same as "not
  connected" — the text differentiates) at or above it, matching the
  already-hard-faulted `OverPressure` state by that point.
  `tmpTxt`/`motorSpeedTxt` stay plain white/red binary (connected/not) —
  no threshold tiers there yet. Also toggles `runBtn`/`stopBtn` touch
  response (`tsw objname,<0|1>`) — `runBtn` disabled while in `Fault`,
  `stopBtn` disabled unless currently `Running` — and pushes a full
  background+font color pair (`bco`+`pco`), not just font color: green
  bg/white text for an enabled `runBtn`, red bg/white text for an enabled
  `stopBtn` (matches real start/stop button convention), low-contrast
  grey-on-grey for either when disabled. All color values
  (`nextion::COLOR_*`) are guesses at the project's actual theme, since we
  can't read it back — adjust in `nextion.rs` if they look wrong on the
  real display.

  Field formats (`pressureTxt`: `"{bar:.2} bar / {psi:.1} psi"`;
  `tmpTxt`: `"{:.1} C"`; `motorSpeedTxt`: `"min{}/set{}/max{}"`;
  `statusTxt`: `"{state} P{O|X} T{O|X} V{O|X}"`) sized for the **25-char
  `txt_maxl` the operator set on each field in the Nextion Editor
  2026-09-11** — plenty of room, no need to abbreviate further.

  **2026-09-10/11 "screen shows nothing at all" saga, root cause and
  false leads** — worth reading before touching this code again. Over
  about a day, the display stopped visually updating at all (not even
  button color/disable), while the log kept showing correct outbound
  commands (`nextion::send_attr` logs every literal command byte-string
  sent, `nextion: tx "..."`, left in place) and inbound touch events kept
  decoding perfectly. Wiring was confirmed end-to-end with a scope
  (signal present at the Nextion's own RX pin, not just the ESP32's TX
  pin). Several plausible-looking causes were tried and **all turned out
  to be false leads**: a per-field `txt_maxl` limit (real, but not the
  cause of total silence — fixed by widening `txt_maxl` in the Editor,
  not by shortening strings, though formats stayed reasonably terse
  anyway), a `TelemetryCache` dedup feature (added, suspected, removed —
  never actually the cause), `tsw` (suspected, temporarily disabled,
  confirmed innocent, restored). The actual root cause was that **Nextion
  was not processing any received command at all** — confirmed by
  enabling `bkcmd=3` (ACK on every instruction, `nextion::enable_acks`,
  called once at boot) and getting zero response bytes back, not even to
  the `bkcmd=3` command itself, while a scope-measured bit period (~300µs
  on the outbound line) didn't match 9600 baud either. Resolved without a
  firmware change — by the next session it was working (a display-side
  fix, e.g. baud/`txt_maxl` reset in the Editor project, or a fresh
  `.tft` upload). **If this recurs**: don't re-suspect `tsw`, the cache,
  or field length first — check with `bkcmd=3` + the existing `tx`
  logging whether Nextion acknowledges *anything*, and independently
  verify the actual baud with a scope on the display's own RX pin before
  assuming a firmware-side cause.

## Nextion HMI (reverse-engineered from the .HMI project file, 2026-08-30)

**Superseded by the section above** — kept for history/context on how
the id-recoverability problem was originally worked around, before a real
project existed to test against.

There's no FOSS/declarative tooling for Nextion's `.HMI`/`.zi` formats — both
are proprietary, undocumented binaries. Nothing here comes from a spec;
it's inferred by extracting the printable-text portions of `Screw
Compressor Controller.HMI` (`strings -n 2 -t d`) and reading the
self-describing `[field-name][value]` structure around them. Confidence
varies by claim, noted per line below.

- **Single page** (`page0`, presumably id 0 — Nextion always gives the
  first/default page id 0). No other page name string exists anywhere in
  the 10MB file. High confidence.
- **Component roster on page0**, in the order they appear in the file
  (likely == creation order, since Nextion assigns ids sequentially at
  creation time): `b0` (button) → `j0` (progress bar) → `b1` (button) →
  `b2` (button) → `b3` (button) → `t4` (text/number field — **not
  mentioned when this was described to Claude**, worth checking what it's
  for). High confidence on the roster and types (each objname sits inside
  a field-schema block — `att-42` for `b0`/`b1`/`b2`/`b3` matches
  Nextion's dual-state Button component fields exactly: `style, borderc,
  borderw, font, picc, pic2, picc2, bco2, pco2, ...`; `att-29` for `j0`
  matches Progress Bar fields: `dez, val, dis, bco, bpic, pco, ppic`;
  `att-39` for `t4` matches a Text component, notably with `key` and `pw`
  fields present — suggests it may have a popup keyboard attached, i.e.
  a numeric entry field, not just a label).
- **No event script exists on any component or page yet** — `codesdown-0`
  / `codesup-0` / `codesload-0` / etc. (touch-press, touch-release,
  page-preinit, ...) are all present as field names but every one of them
  is empty. Searched the whole file for Nextion Basic keywords (`click`,
  `.val`, `tsw`, `page N`) — the ONLY script content anywhere is the
  global power-on init (below). **This project is a bare visual layout —
  none of the buttons or the progress bar do anything yet.** High
  confidence (exhaustive keyword search, not just spot-checking).
- **Global power-on script, extracted verbatim** (this is genuine
  plaintext in the file, not inferred):
  ```
  int sys0=0,sys1=0,sys2=0
  baud=9600
  dim=100
  recmod=0
  printh 00 00 00 ff ff ff 88 ff ff ff
  page 0
  ```
  Confirms **9600 baud** for real (matches `config::NEXTION_BAUD_RATE`'s
  placeholder — no longer a guess for this side of the link). `recmod=0`
  means passive mode: the display won't proactively push anything except
  touch events — the firmware driver doesn't need to filter unsolicited
  chatter. `dim=100` is full backlight at boot.
- **Numeric page/component ids are NOT reliably recoverable from static
  analysis** — the `id` field's value is binary (not printable), and
  isolating its exact bytes without a known-good reference risks a wrong
  guess that's hard to debug later. Rather than depend on them, inbound
  events use the marker protocol below instead. Outbound commands
  (ESP32 → Nextion) never needed ids at all — Nextion parses ASCII
  commands by objname directly (e.g. `j0.val=65`).

### Wire protocol as originally planned (superseded, see section above)

The plan at the time: inbound via a `printh`-sent marker (since ids
looked unrecoverable), outbound via `objname.attr=value` (`j0`/`t4`,
never actually implemented against this file). None of this was ever
flash-tested — by the time outbound telemetry was built, a real project
(`ScrewCompressor.HMI`) existed to test against instead, which turned out
not to need the marker workaround at all. Left here only so a stray
`printh 4E 58 0N` reference elsewhere doesn't look unexplained.

## Validation rules

- Pressure: 4mA floor triggers sensor fault (below floor = disconnected/failed loop)
- NTC: voltage-window check triggers sensor fault (open or shorted probe)

## Not yet done / open items

- **Live VFD Modbus comms still not confirmed working (2026-09-09
  session)**, using the new `vfd-run-test` bin (`src/bin/vfd_run_test.rs` —
  flashes/boots standalone, reads freq limits, runs at VFD-reported min for
  10s, stops; add via `cargo run --bin vfd-run-test`). Two real bugs found
  and fixed this session, both against the fabricated PCB's own netlist
  (`Main/production/netlist.ipc`, cross-referencing MAX3485 `U3`'s pins):
  1. Modbus DI/RO were on GPIO18/19, not GPIO23/22 as originally assumed —
     those two pins actually go to the Nextion header (`J3`). Every frame
     was going out to Nextion and back, never reaching the transceiver.
  2. Having moved to the right pins, TX/RX were then assigned backwards
     (MCU TX onto RO, MCU RX onto DI) — fixed, see `MODBUS_DI_GPIO`/
     `MODBUS_RO_GPIO` in `config.rs` and the pin table above.

  After both fixes, still a hard Modbus `Timeout` (no response at all, not
  a corrupted one) with the board wired to the real VFD — tried both RS485
  A/B orientations on `J4` ("RS485 to VFD" per `bom.csv`), no change either
  way. VFD-side Pd-/P0- params were reconfirmed OK on the keypad. Leading
  hypothesis, not yet tested: missing RS485 ground reference — the MAX3485
  is non-isolated (rides the board's own 3.3V logic ground, see "System"
  above), and only A/B is currently wired to the VFD, no shared ground. A
  USB-RS485 adapter got a response over just 2 wires, but likely had an
  incidental ground path via the PC's mains earth that this DIN-rail-powered
  board doesn't have. Next session: scope the MAX3485's actual DI/RO/DE-RE
  lines directly against a transmit attempt, and compare side-by-side
  against the known-working USB adapter (same PC, same VFD) before
  concluding it's the ground reference specifically.
- Modbus RTU driver (`src/modbus.rs`), speed-loop PID (`src/pid.rs`),
  persistent starts/runtime counters (`src/storage.rs`), and the real
  state machine transition logic (`src/state_machine.rs`) are implemented
  and wired into `main.rs`'s control loop (ADC_SAMPLE_MS cadence) —
  builds clean, not yet flash-tested against a live VFD.
- **Nextion inbound + outbound both flash-tested against real hardware,
  2026-09-10** — see "Nextion HMI — current project" above for the full
  writeup. Inbound: `runBtn`/`stopBtn` clicks confirmed reaching
  `reader_task` (native touch-event frames logged and dispatched
  correctly) — but only as far as the log line; not yet confirmed that
  `Controller::enable()`/`disable()` actually flip compressor state end to
  end. `ClearFault`/`Drain` have no HMI button yet. Outbound: text fields
  confirmed appearing on the real screen (operator: "there's text on the
  screen now"). The red `"NOT CONNECTED"` state and the `runBtn`/`stopBtn`
  enable/disable-and-dim behaviour (added this same session, right after
  the objname fixes) are flashed but **not yet confirmed** on the real
  display — check next session whether the disabled-button dimming
  actually looks right, since `COLOR_DISABLED`/`OK`/`WARN` are guesses at
  the project's theme.
- Start-assist valve polarity (does energizing it close the recirculation
  path?) is an assumption, not verified on hardware — see config.rs.
- **Rotor/oil thermal cutoff wired 2026-09-10** — `Controller::tick` now
  hard-faults (`RotorOverTemp`) at `T_ROTOR_MAX_C`, and `clear_fault` gates
  re-arming on cooling back down to `T_ROTOR_SAFE_RESET_C` (dismissible
  but not immediately effective — see config.rs). Still not physically
  testable: the probe itself isn't wired to GPIO1 (`main.rs` passes
  `temp_c: None` to `tick` unconditionally), and per the original 2-channel
  bodge plan is missing from the schematic entirely. `MotorOverTemp` and
  `TempSensorFault` remain unwired — only one rotor/oil probe is in scope
  for now, not the separate motor-winding channel.
- PID gains (`PID_KP`/`KI`/`KD`) are an unconfirmed starting guess, not
  tuned against the real plant.
- `t4`'s actual purpose is unconfirmed (found during HMI reverse
  engineering, not part of the original 4-button + progress-bar spec).
- RS485 is a MAX3485 (non-isolated), confirmed 2026-08-30 — verify
  whether the NMA0505SC isolated supply (originally speced for an
  ADM2483BRWZ) is populated/needed for anything else, or dead weight.
- Modbus/Nextion baud rates are unconfirmed placeholders (9600 each).
- NTC_BETA unconfirmed.
- TPS5430 layout bug not yet reworked on hardware.
