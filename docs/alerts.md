# Audible alerts: what the beeper is for and what the beeps mean

**Status: a plan. Nothing in the firmware drives the buzzer yet.**

The beeper is for errors only. **Touching the screen never beeps.**

## The hardware

The Waveshare ESP32-S3-Touch-LCD-4 (Rev 4.0) has an onboard buzzer. Checked
against Waveshare's schematic (`ESP32-S3-Touch-LCD-4 V4.0.pdf`, page 1) and
their expander documentation:

- Buzzer **B1** sits between 3V3 and an NPN transistor (**Q1**, SS8050).
  Q1's base is driven by the net **BEE_EN** through 1 kΩ, with 4.7 kΩ to GND.
- BEE_EN is **EXIO6 of the CH32V003 expander** (I²C address 0x24). Writing bit 6 of
  the output register switches the buzzer: 1 = on. In the firmware this is
  `board::EXIO_BUZZER`. [Piggyback/pin-allocation.md](../Piggyback/pin-allocation.md)
  lists it with the other expander pins.

What that means for us:

- **It is on/off only.** There is no pitch and no volume control. All
  information has to be carried by timing: how many beeps, how long, how often.
- **The tone is whatever the buzzer makes.** Waveshare's own demo just switches
  it on for 80 ms, which fits a self-oscillating ("active") buzzer. The schematic
  doesn't say which type it is; check by ear (see [Open questions](#open-questions)).
- **Timing goes over I²C**, a write of about 0.3 ms at 100 kHz, shared with the
  touch controller and the piggyback. Pulses of 100 ms and longer are fine.
- **It is small and the compressor is loud.** Expect it to be heard at the
  machine, not across the room. Test in place before relying on it.
- **Only the firmware can sound it.** If the firmware hangs, the piggyback's
  hardware watchdog still stops the motor, but nothing beeps. See
  [Limits](#limits).

## Principles

1. **Errors only.** No touch feedback, no start-up chirp, no "stopped" or
   "started" tones. A beep always means someone should look at the screen.
2. **The beep gets attention; the screen says why.** There are three patterns,
   not one per fault. The status bar already names the cause.
3. **Quiet by default.** An alarm sounds when it appears, repeats at a calm rate,
   and stops when acknowledged.
4. **Acknowledge by touching the screen anywhere.** That silences the repeats
   (and, like any touch, makes no sound). It does not clear a fault: the
   RESET button does that. A different or worse alarm sounds again.
5. **Highest severity wins.** If several are active at once, only the most
   severe one's pattern sounds.

## Patterns

Three patterns, easy to tell apart by rhythm. On and off times are in milliseconds.

| Level | Meaning | Pattern | Repeats |
|---|---|---|---|
| **TRIP** | The machine was stopped for safety | 4 fast beeps: `100 on, 100 off` × 4 | every 5 s for 2 min, then every 30 s |
| **FAULT** | Stopped or can't run, needs the operator | 2 long beeps: `400 on, 200 off` × 2 | every 30 s for 10 min, then every 5 min |
| **WARNING** | Still running or idle, but something is degraded | 1 short beep: `200 on` | once, again after 10 min if still present |

Repeats continue until acknowledged (touch) or the cause goes away.
When the cause goes away the sound stops at once. Nothing beeps for "cleared".

## What triggers what

Taken from the firmware's own list: `FaultReason` in `src/state_machine.rs` and
the warnings in `status_line` in `src/ui.rs`.

### TRIP: stopped for safety

| Cause | Source |
|---|---|
| Over pressure | `FaultReason::OverPressure` |
| Oil (rotor) over temperature | `FaultReason::RotorOverTemp` |
| Motor over temperature | `FaultReason::MotorOverTemp` |
| Hardware over-temperature trip | piggyback comparator (`hw_temp_ok` low) |
| Safety interlock open | piggyback SAFE low (watchdog or temperature cutout) |

The interlock is expected to be open for a few seconds after power-up, before the
firmware's heartbeat starts, so it is **ignored for the first 10 s after boot**.

### FAULT: stopped, needs a reset

| Cause | Source |
|---|---|
| Pressure sensor fault | `FaultReason::PressureSensorFault` |
| Temperature sensor fault | `FaultReason::TempSensorFault` |
| No pressure build | `FaultReason::PressureBuildTimeout` |
| Too many starts | `FaultReason::StartRateLimitExceeded` |
| VFD comms lost | `FaultReason::ModbusCommsLost` |

### WARNING: degraded, not stopped

These are the amber status-bar messages while no fault is latched:
"Piggyback I/O not responding", "Pressure sensor fault", "Temp sensor fault",
"VFD not responding". To avoid beeping at blips, a warning must be present for
**10 s** before it sounds.

### Never beeps

Touches and button presses, START and STOP, setpoint changes, the normal
transitions (standby, running, stopping, low-demand idle stop), the drain and
unload buttons, and a fault being cleared.

## How it would be built

This is the plan; none of it exists yet.

- **`src/alarm.rs`**, new, with three parts:
  - `level(&UiState) -> Level`: a pure function, `None / Warning / Fault / Trip`,
    from the same `UiState` the control loop already publishes to the UI. Match on
    every `FaultReason` with no `_` arm, so a new fault can't be added without
    choosing its level.
  - The patterns as `const` tables of (on, off) times, and a small scheduler that
    turns (level, time since it started, acknowledged) into "buzzer on or off".
    Both are plain Rust with no hardware, so they can be tested on a computer
    (same approach as `Software/tools/st7701_golden.rs`).
  - An Embassy task that waits on the level, runs the scheduler and switches the
    buzzer.
- **Acknowledge** is one more signal, raised from the touch handler in `ui.rs`
  when a finger goes down. The touch itself stays silent.
- **One owner for the expander's output register.** `Ch32Exio` keeps a copy of
  the register that also holds the panel and touch resets and **SYS_EN, the board's
  power latch, which must stay high**. Two writers with separate copies would
  overwrite each other. Put the single `Ch32Exio` behind a mutex that `main` (resets,
  backlight) and the alarm task share, rather than creating a second one.
- **Constants** (pattern timings, repeat intervals, the 10 s boot grace and the
  10 s warning debounce) go in `src/config.rs` with the other tunables.
- **The buzzer is off at boot.** `Ch32Exio::new` already writes the output
  register with the buzzer bit clear, so a restart silences it.

## Limits

- **A hung firmware beeps nothing.** The piggyback's watchdog still drops SAFE and
  stops the motor and valves, and the screen freezes, but only the firmware can
  drive the buzzer. If audible warning of a hang matters, the piggyback could drive
  its own buzzer from the watchdog (WD_OK low). That is a board change, not firmware.
- **A very quiet buzzer in a loud room.** If it can't be heard at the machine, the
  fix is a louder device (a panel beeper or beacon on a spare output), not longer
  beeps.
- **No pitch**, so no rising or falling tones. Severity is carried by rhythm only.

## Open questions

- **Loudness: answered.** A temporary 3-beep test at boot (100 ms, 300 ms, 1 s,
  on 2026-11-17) was loud enough to be unwelcome indoors in a quiet house, so the
  buzzer is audible. Still to learn: whether it is audible next to the running
  compressor, and the tone quality (steady or buzzy).
- Are the repeat intervals right? The values above are a first guess: frequent
  enough not to be missed for a stopped compressor, rare enough not to be nagging.
- Is "touch anywhere to acknowledge" what you want, or only the RESET button?
- Should a reboot that wasn't a power-up (a crash or watchdog reset) chirp
  once? It is useful to know the board restarted on its own, but it is not an
  error the operator can act on, so it is left out for now.
