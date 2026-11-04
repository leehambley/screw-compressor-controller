//! Top-level compressor state machine.
//!
//! Idle -> Unloading -> Running -> UnloadIdle -> Stopping -> Fault
//!
//! No load/unload air valve exists in this design (see CLAUDE.md) —
//! capacity control is 100% VFD speed via `pid::Pid`. "Unloading" and
//! "UnloadIdle" are carried-over names from the old valve-based design;
//! see CLAUDE.md's "Capacity control strategy" for what they actually mean
//! here (open-loop fill ramp, and idling at minimum frequency).

use embassy_time::{Duration, Instant};

use crate::config;
use crate::modbus::SkiVfd;
use crate::pid::Pid;
use crate::storage::{CounterStore, Counters};
use crate::valve::PwmValve;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Unloading,
    Running,
    UnloadIdle,
    Stopping,
    Fault,
}

impl State {
    /// For telemetry (Nextion `statusTxt`, logging) — same text `Debug`
    /// would give, spelled out so call sites don't need a format buffer.
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Idle => "Idle",
            State::Unloading => "Unloading",
            State::Running => "Running",
            State::UnloadIdle => "UnloadIdle",
            State::Stopping => "Stopping",
            State::Fault => "Fault",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultReason {
    OverPressure,
    RotorOverTemp,
    MotorOverTemp,
    PressureSensorFault,
    TempSensorFault,
    PressureBuildTimeout,
    StartRateLimitExceeded,
    ModbusCommsLost,
}

impl FaultReason {
    /// Operator-facing text for the HMI status line.
    pub fn as_str(&self) -> &'static str {
        match self {
            FaultReason::OverPressure => "Over pressure",
            FaultReason::RotorOverTemp => "Oil over temp",
            FaultReason::MotorOverTemp => "Motor over temp",
            FaultReason::PressureSensorFault => "Pressure sensor",
            FaultReason::TempSensorFault => "Temp sensor",
            FaultReason::PressureBuildTimeout => "No pressure build",
            FaultReason::StartRateLimitExceeded => "Too many starts",
            FaultReason::ModbusCommsLost => "VFD comms lost",
        }
    }
}

/// Whether the speed loop's last output sat on one of its frequency
/// limits — surfaced on the HMI (tMotorNote) so the operator can see the
/// PID has run out of authority in that direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedClamp {
    None,
    Min,
    Max,
}

/// Rolling `STARTS_WINDOW_MS` start-rate limiter. Fixed-capacity ring
/// buffer sized exactly to `MAX_STARTS_PER_HOUR`: the slot about to be
/// overwritten is always the Nth-most-recent start, so checking it alone
/// tells us whether we've had `MAX_STARTS_PER_HOUR` starts within the
/// window.
struct StartLimiter {
    history: [Option<Instant>; config::MAX_STARTS_PER_HOUR as usize],
    next: usize,
}

impl StartLimiter {
    fn new() -> Self {
        Self {
            history: [None; config::MAX_STARTS_PER_HOUR as usize],
            next: 0,
        }
    }

    fn can_start(&self, now: Instant) -> bool {
        match self.history[self.next] {
            None => true,
            Some(t) => now.duration_since(t).as_millis() >= config::STARTS_WINDOW_MS,
        }
    }

    fn record_start(&mut self, now: Instant) {
        self.history[self.next] = Some(now);
        self.next = (self.next + 1) % self.history.len();
    }

    fn clear(&mut self) {
        self.history = [None; config::MAX_STARTS_PER_HOUR as usize];
        self.next = 0;
    }
}

pub struct Controller<'d> {
    pub state: State,
    pub fault: Option<FaultReason>,
    enabled: bool,
    vfd: SkiVfd<'d>,
    vfd_min_hz: f32,
    vfd_max_hz: f32,
    /// Whether `vfd_min_hz`/`vfd_max_hz` came from the VFD itself, or are
    /// the config fallback because it didn't answer at boot (re-read on the
    /// first successful `poll_vfd`).
    limits_from_vfd: bool,
    /// Live running frequency / output current from the VFD's monitor
    /// registers, `None` while it isn't answering.
    motor_hz: Option<f32>,
    motor_amps: Option<f32>,
    speed_clamp: SpeedClamp,
    /// Operator setpoint (bar), persisted. See config.rs SETPOINT_*.
    setpoint_bar: f32,
    setpoint_changed_at: Option<Instant>,
    /// Setpoint from the most recent `set_frequency_hz` call, successful or
    /// not — telemetry only, doesn't affect control (see `pid` for that).
    setpoint_hz: f32,
    /// Whether the most recent Modbus call to the VFD succeeded —
    /// telemetry only. A transient failure doesn't fault the controller by
    /// itself; see `tick_running`/`tick_idle` for what actually does.
    vfd_comms_ok: bool,
    pid: Pid,
    starts: StartLimiter,
    store: CounterStore,
    counters: Counters,
    state_entered_at: Instant,
    run_started_at: Option<Instant>,
    last_persist_at: Instant,
    /// Latest rotor/oil temperature reading, stored so `clear_fault` can
    /// gate re-arming a `RotorOverTemp` fault on it. `None` when the probe
    /// isn't connected (it isn't yet — see CLAUDE.md "Known firmware gap").
    temp_c: Option<f32>,
    /// Start-assist / blow-down-to-intake valve (GPIO20), normally-open.
    /// Closed (energised) at rest and while compressing, to hold air; open
    /// only for the start ramp, stopping and Fault — see config.rs.
    /// Driven only through `apply_valves`: the state machine decides
    /// `assist_closed`, and a held HMI bUnload overrides it open.
    unload_valve: PwmValve,
    assist_closed: bool,
    unload_hold_until: Option<Instant>,
    /// Waste condensate drain (GPIO21). Manual (hold bPurge) only, not part
    /// of automatic capacity control.
    drain_valve: PwmValve,
    drain_until: Option<Instant>,
    /// Consecutive ticks over the trip threshold — see FAULT_CONFIRM_TICKS.
    over_temp_ticks: u8,
    over_pressure_ticks: u8,
}

/// Counts consecutive `true` ticks in `count`, resetting on `false`; true
/// once a condition has held for `FAULT_CONFIRM_TICKS` in a row.
pub fn confirmed(count: &mut u8, condition: bool) -> bool {
    *count = if condition { count.saturating_add(1) } else { 0 };
    *count >= config::FAULT_CONFIRM_TICKS
}

const PERSIST_INTERVAL_MS: u64 = 300_000; // 5 min — see CLAUDE.md Persistence

impl<'d> Controller<'d> {
    /// Reads the VFD's live frequency limits (P0-12/P0-14) and clamps them
    /// against our own `VFD_FREQ_MIN_HZ`/`MAX_HZ` — the tighter of the two
    /// on each side — and loads persisted counters. Falls back to our own
    /// config bounds if the VFD doesn't respond (comms not up yet).
    pub async fn new(
        mut vfd: SkiVfd<'d>,
        mut store: CounterStore,
        unload_valve: PwmValve,
        drain_valve: PwmValve,
    ) -> Self {
        let now = Instant::now();

        let (vfd_min_hz, vfd_max_hz, vfd_comms_ok) = match vfd.read_freq_limits().await {
            Ok((min, max)) => (
                min.max(config::VFD_FREQ_MIN_HZ),
                max.min(config::VFD_FREQ_MAX_HZ),
                true,
            ),
            Err(_) => (config::VFD_FREQ_MIN_HZ, config::VFD_FREQ_MAX_HZ, false),
        };

        let counters = store.load();
        // NaN or out-of-range (corrupt-but-CRC-valid is unlikely, a changed
        // SETPOINT_MAX_BAR less so) falls back to the default.
        let setpoint_bar = if (config::SETPOINT_MIN_BAR..=config::SETPOINT_MAX_BAR)
            .contains(&counters.setpoint_bar)
        {
            counters.setpoint_bar
        } else {
            config::P_MODULATE_START_BAR
        };

        Self {
            state: State::Idle,
            fault: None,
            enabled: false,
            vfd,
            vfd_min_hz,
            vfd_max_hz,
            limits_from_vfd: vfd_comms_ok,
            motor_hz: None,
            motor_amps: None,
            speed_clamp: SpeedClamp::None,
            setpoint_bar,
            setpoint_changed_at: None,
            setpoint_hz: vfd_min_hz,
            vfd_comms_ok,
            pid: Pid::new(
                config::PID_KP,
                config::PID_KI,
                config::PID_KD,
                setpoint_bar,
                vfd_min_hz,
                vfd_max_hz,
            ),
            starts: StartLimiter::new(),
            store,
            counters,
            state_entered_at: now,
            run_started_at: None,
            last_persist_at: now,
            temp_c: None,
            unload_valve,
            assist_closed: true, // boots into Idle: hold air
            unload_hold_until: None,
            drain_valve,
            drain_until: None,
            over_temp_ticks: 0,
            over_pressure_ticks: 0,
        }
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Live VFD frequency bounds (already clamped against our own config
    /// bounds — see `new()`), for telemetry.
    pub fn vfd_min_hz(&self) -> f32 {
        self.vfd_min_hz
    }

    pub fn vfd_max_hz(&self) -> f32 {
        self.vfd_max_hz
    }

    /// Setpoint from the most recent `set_frequency_hz` call, for
    /// telemetry — see the field doc comment above.
    pub fn setpoint_hz(&self) -> f32 {
        self.setpoint_hz
    }

    /// Whether the most recent Modbus call to the VFD succeeded, for
    /// telemetry — see the field doc comment above.
    pub fn vfd_comms_ok(&self) -> bool {
        self.vfd_comms_ok
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn setpoint_bar(&self) -> f32 {
        self.setpoint_bar
    }

    /// Pressure at or below which an enabled, idle compressor starts.
    pub fn load_threshold_bar(&self) -> f32 {
        self.setpoint_bar - config::LOAD_HYSTERESIS_BAR
    }

    /// Pressure above which a minimum-speed compressor hands off to
    /// UnloadIdle.
    pub fn unload_threshold_bar(&self) -> f32 {
        (self.setpoint_bar + config::UNLOAD_MARGIN_BAR).min(config::P_UNLOAD_BAR)
    }

    pub fn motor_hz(&self) -> Option<f32> {
        self.motor_hz
    }

    pub fn motor_amps(&self) -> Option<f32> {
        self.motor_amps
    }

    pub fn speed_clamp(&self) -> SpeedClamp {
        self.speed_clamp
    }

    pub fn unload_held(&self) -> bool {
        self.unload_hold_until.is_some()
    }

    pub fn drain_open(&self) -> bool {
        self.drain_until.is_some()
    }

    /// Whether the start-assist valve is currently open (de-energized),
    /// for whatever reason — state machine or held bUnload.
    pub fn assist_open(&self) -> bool {
        !self.assist_closed || self.unload_held()
    }

    /// Seconds until UnloadIdle gives up and stops the motor, while in it.
    pub fn idle_stop_in_s(&self) -> Option<u32> {
        if self.state != State::UnloadIdle {
            return None;
        }
        let elapsed = Instant::now().duration_since(self.state_entered_at).as_millis();
        Some((config::IDLE_TIMEOUT_MS.saturating_sub(elapsed) / 1000) as u32)
    }

    /// HMI bRunState: one button, meaning depends on where we are — in
    /// Fault it acknowledges (`clear_fault`, which may refuse, see there);
    /// otherwise it toggles the run request (stop is graceful, via
    /// Stopping).
    pub fn toggle_run(&mut self) {
        if self.state == State::Fault {
            self.clear_fault();
        } else {
            self.enabled = !self.enabled;
        }
    }

    /// HMI bSetPointMinus/Plus. Snapped to the step grid and clamped to
    /// SETPOINT_MIN/MAX_BAR; takes effect on the PID immediately, written
    /// to flash by `service` once the operator stops pressing.
    pub fn adjust_setpoint(&mut self, delta_bar: f32) {
        let step = config::SETPOINT_STEP_BAR;
        let target = libm::roundf((self.setpoint_bar + delta_bar) / step) * step;
        let target = target.clamp(config::SETPOINT_MIN_BAR, config::SETPOINT_MAX_BAR);
        if target != self.setpoint_bar {
            self.setpoint_bar = target;
            self.pid.set_setpoint(target);
            self.setpoint_changed_at = Some(Instant::now());
        }
    }

    /// HMI bUnload press/release: forces the start-assist valve open while
    /// held (manual unload), up to UNLOAD_MAX_HOLD_MS.
    pub fn set_unload_hold(&mut self, held: bool) {
        self.unload_hold_until =
            held.then(|| Instant::now() + Duration::from_millis(config::UNLOAD_MAX_HOLD_MS));
        self.apply_valves();
    }

    /// HMI bPurge press/release: condensate drain open while held, up to
    /// DRAIN_MAX_HOLD_MS. Independent of compressor state.
    pub fn set_drain_hold(&mut self, held: bool) {
        self.drain_until =
            held.then(|| Instant::now() + Duration::from_millis(config::DRAIN_MAX_HOLD_MS));
        self.apply_valves();
    }

    /// Housekeeping that must run every loop pass whether or not `tick`
    /// does (it's skipped on a sensor fault): expires held-button
    /// overrides, drives the valve outputs, and persists a changed
    /// setpoint once it has settled.
    pub fn service(&mut self, now: Instant) {
        if self.unload_hold_until.is_some_and(|until| now >= until) {
            log::warn!("bUnload held past UNLOAD_MAX_HOLD_MS (release lost?), releasing");
            self.unload_hold_until = None;
        }
        if self.drain_until.is_some_and(|until| now >= until) {
            log::warn!("bPurge held past DRAIN_MAX_HOLD_MS (release lost?), closing drain");
            self.drain_until = None;
        }
        self.apply_valves();

        if let Some(changed) = self.setpoint_changed_at
            && now.duration_since(changed).as_millis() >= config::SETPOINT_PERSIST_DELAY_MS
        {
            self.setpoint_changed_at = None;
            self.counters.setpoint_bar = self.setpoint_bar;
            self.persist(now);
        }
    }

    fn apply_valves(&mut self) {
        let now = Instant::now();
        let open = self.assist_open();
        self.unload_valve
            .set(open == config::UNLOAD_VALVE_ENERGISE_TO_OPEN, now);
        let open = self.drain_open();
        self.drain_valve
            .set(open == config::DRAIN_VALVE_ENERGISE_TO_OPEN, now);
    }

    /// Stop/resume PWM on held valves around ADC sampling (see
    /// `PwmValve::quiet`).
    pub fn valves_quiet(&self, quiet: bool) {
        self.unload_valve.quiet(quiet);
        self.drain_valve.quiet(quiet);
    }

    /// Reads the VFD's monitor registers (running frequency, current) for
    /// the HMI, and doubles as the comms-health probe while idle. If the
    /// frequency limits couldn't be read at boot, retries them here.
    /// Doesn't fault on failure by itself — `tick_*` does that wherever a
    /// failed write actually matters.
    pub async fn poll_vfd(&mut self) {
        match self.vfd.read_monitor().await {
            Ok((hz, amps, _bus_v)) => {
                self.motor_hz = Some(hz);
                self.motor_amps = Some(amps);
                self.vfd_comms_ok = true;
            }
            Err(e) => {
                // Only a start or a running motor faults on this (see
                // `tick_*`); idle it's a status warning, logged once.
                if self.vfd_comms_ok {
                    log::warn!("vfd: monitor read failed: {e:?}");
                }
                self.motor_hz = None;
                self.motor_amps = None;
                self.vfd_comms_ok = false;
                return;
            }
        }
        if !self.limits_from_vfd
            && let Ok((min, max)) = self.vfd.read_freq_limits().await
        {
            self.vfd_min_hz = min.max(config::VFD_FREQ_MIN_HZ);
            self.vfd_max_hz = max.min(config::VFD_FREQ_MAX_HZ);
            self.pid.set_output_limits(self.vfd_min_hz, self.vfd_max_hz);
            self.limits_from_vfd = true;
            log::info!("vfd: limits {}..{} Hz", self.vfd_min_hz, self.vfd_max_hz);
        }
    }

    /// HMI hook: clear a fault. For `StartRateLimitExceeded` this also
    /// resets the start-history ring buffer — otherwise the very next
    /// start attempt would immediately re-trip the same limit.
    ///
    /// For `RotorOverTemp` this is a dismissible-but-gated cooldown, not
    /// an immediate clear: refuses (silently — the fault just stays put)
    /// until the probe reads back down to `T_ROTOR_SAFE_RESET_C`, well
    /// below the `T_ROTOR_MAX_C` trip point, so the machine can't be
    /// re-armed right at the boundary only to re-trip moments later. If
    /// the probe reading is unavailable (`None`) when this is called,
    /// that also counts as "not cooled" — fail safe.
    pub fn clear_fault(&mut self) {
        if self.fault == Some(FaultReason::RotorOverTemp) {
            let cooled = self
                .temp_c
                .is_some_and(|t| t <= config::T_ROTOR_SAFE_RESET_C);
            if !cooled {
                return;
            }
        }
        if self.fault == Some(FaultReason::StartRateLimitExceeded) {
            self.starts.clear();
        }
        self.fault = None;
        self.state = State::Idle;
        self.state_entered_at = Instant::now();
    }

    /// Report a fault detected outside the normal pressure-driven
    /// transitions (e.g. a sensor fault noticed by the caller before
    /// converting a raw reading to bar/°C).
    pub async fn report_fault(&mut self, reason: FaultReason) {
        self.enter_fault(reason, Instant::now()).await;
    }

    /// Advance the state machine by one control cycle. `pressure_bar` is
    /// the primary pressure sensor reading (already validated — sensor
    /// faults should go through `report_fault` instead). `temp_c` is the
    /// rotor/oil probe reading — `None` while it's unconnected (not
    /// physically installed yet, see CLAUDE.md), in which case the
    /// over-temp hard cutoff below simply can't trip; it does not itself
    /// raise `TempSensorFault` (that's for a probe that's wired but
    /// reading implausibly, via `report_fault`, same as
    /// `PressureSensorFault`). `dt_s` the time since the previous call in
    /// seconds (for the PID's integral/derivative).
    pub async fn tick(&mut self, pressure_bar: f32, temp_c: Option<f32>, dt_s: f32) {
        let now = Instant::now();
        self.temp_c = temp_c;

        // Hard thermal cutoff — checked ahead of the pressure fault below
        // since overheating is the more time-critical of the two (thermal
        // mass means it won't spike as fast as pressure can, but there's
        // no safe "modulate around it" response the way pressure has).
        let over_temp = temp_c.is_some_and(|t| t >= config::T_ROTOR_MAX_C);
        if confirmed(&mut self.over_temp_ticks, over_temp) && self.state != State::Fault {
            self.enter_fault(FaultReason::RotorOverTemp, now).await;
            return;
        }

        let over_pressure = pressure_bar >= config::P_MAX_FAULT_BAR;
        if confirmed(&mut self.over_pressure_ticks, over_pressure) && self.state != State::Fault {
            self.enter_fault(FaultReason::OverPressure, now).await;
            return;
        }

        match self.state {
            State::Idle => self.tick_idle(pressure_bar, now).await,
            State::Unloading => self.tick_unloading(pressure_bar, now).await,
            State::Running => self.tick_running(pressure_bar, dt_s, now).await,
            State::UnloadIdle => self.tick_unload_idle(pressure_bar, now).await,
            State::Stopping => self.tick_stopping(now).await,
            State::Fault => {} // stays until clear_fault() from the HMI
        }
        if self.state != State::Running {
            self.speed_clamp = SpeedClamp::None;
        }
        self.apply_valves();

        if matches!(self.state, State::Running | State::UnloadIdle)
            && now.duration_since(self.last_persist_at).as_millis() >= PERSIST_INTERVAL_MS
        {
            self.persist(now);
        }
    }

    async fn tick_idle(&mut self, pressure_bar: f32, now: Instant) {
        // At rest the valve holds air; only a start opens it.
        self.assist_closed = true;
        if !self.enabled || pressure_bar > self.load_threshold_bar() {
            return;
        }
        if !self.starts.can_start(now) {
            self.enter_fault(FaultReason::StartRateLimitExceeded, now)
                .await;
            return;
        }

        self.starts.record_start(now);
        self.counters.total_starts += 1;
        self.persist(now);
        self.run_started_at = Some(now);
        self.pid.reset();

        // Start-assist: open the blow-down-to-intake valve before spinning
        // the motor up, so it starts against ~no compression load. Closed
        // again in tick_unloading once the VFD's accel ramp finishes.
        // (Idle had it closed; this is the one point a start opens it.)
        self.assist_closed = false;
        self.apply_valves();

        self.vfd_comms_ok = self.vfd.set_run(true).await.is_ok();
        if !self.vfd_comms_ok {
            self.enter_fault(FaultReason::ModbusCommsLost, now).await;
            return;
        }
        // Open-loop fill at max speed — see tick_unloading.
        self.vfd_comms_ok = self.vfd.set_frequency_hz(self.vfd_max_hz).await.is_ok();
        self.setpoint_hz = self.vfd_max_hz;
        self.enter_state(State::Unloading, now);
    }

    async fn tick_unloading(&mut self, pressure_bar: f32, now: Instant) {
        if !self.enabled {
            self.enter_state(State::Stopping, now);
            return;
        }

        // Motor should be up to speed by now — close the start-assist valve
        // so real compression (and pressure build) can begin.
        if now.duration_since(self.state_entered_at).as_millis() >= config::VFD_ACCEL_MS {
            self.assist_closed = true;
        }

        if pressure_bar >= config::P_MIN_BUILD_BAR {
            self.pid.reset();
            self.enter_state(State::Running, now);
            return;
        }
        if now.duration_since(self.state_entered_at).as_millis() >= config::P_BUILD_TIMEOUT_MS {
            self.enter_fault(FaultReason::PressureBuildTimeout, now)
                .await;
        }
    }

    async fn tick_running(&mut self, pressure_bar: f32, dt_s: f32, now: Instant) {
        if !self.enabled {
            self.enter_state(State::Stopping, now);
            return;
        }

        let hz = self.pid.update(pressure_bar, dt_s);
        self.speed_clamp = if hz <= self.vfd_min_hz {
            SpeedClamp::Min
        } else if hz >= self.vfd_max_hz {
            SpeedClamp::Max
        } else {
            SpeedClamp::None
        };
        self.vfd_comms_ok = self.vfd.set_frequency_hz(hz).await.is_ok();
        self.setpoint_hz = hz;
        if !self.vfd_comms_ok {
            self.enter_fault(FaultReason::ModbusCommsLost, now).await;
            return;
        }

        // PID saturated at minimum speed and pressure still climbing past
        // the unload threshold: demand is too low to shed further by
        // slowing down. Hand off to UnloadIdle rather than overshoot.
        if pressure_bar >= self.unload_threshold_bar() && hz <= self.vfd_min_hz {
            self.enter_state(State::UnloadIdle, now);
        }
    }

    async fn tick_unload_idle(&mut self, pressure_bar: f32, now: Instant) {
        if !self.enabled {
            self.enter_state(State::Stopping, now);
            return;
        }
        self.vfd_comms_ok = self.vfd.set_frequency_hz(self.vfd_min_hz).await.is_ok();
        self.setpoint_hz = self.vfd_min_hz;

        if pressure_bar < self.unload_threshold_bar() {
            // Demand resumed — pressure is dropping again.
            self.pid.reset();
            self.enter_state(State::Running, now);
            return;
        }
        if now.duration_since(self.state_entered_at).as_millis() >= config::IDLE_TIMEOUT_MS {
            self.enter_state(State::Stopping, now);
        }
    }

    async fn tick_stopping(&mut self, now: Instant) {
        // Open the start-assist valve to bleed discharge pressure back to
        // intake as the motor decelerates; Idle closes it again once the
        // motor has stopped.
        self.assist_closed = false;
        let _ = self.vfd.set_run(false).await;
        if now.duration_since(self.state_entered_at).as_millis()
            < config::VFD_DECEL_MS + config::UNLOAD_STOP_MS
        {
            return;
        }
        if let Some(started) = self.run_started_at.take() {
            self.counters.total_runtime_s += now.duration_since(started).as_secs();
        }
        self.persist(now);
        self.enter_state(State::Idle, now);
    }

    fn enter_state(&mut self, state: State, now: Instant) {
        self.state = state;
        self.state_entered_at = now;
    }

    async fn enter_fault(&mut self, reason: FaultReason, now: Instant) {
        // A fault drops the run request: RESET returns to a stopped Idle and
        // the operator presses START again. Otherwise the request outlives
        // the fault and Idle retries the start (and re-trips) straight after
        // RESET.
        self.enabled = false;
        self.assist_closed = false; // fail safe: open / de-energized
        self.apply_valves();
        let _ = self.vfd.set_run(false).await;
        if let Some(started) = self.run_started_at.take() {
            self.counters.total_runtime_s += now.duration_since(started).as_secs();
            self.persist(now);
        }
        self.fault = Some(reason);
        self.enter_state(State::Fault, now);
    }

    fn persist(&mut self, now: Instant) {
        self.store.save(self.counters);
        self.last_persist_at = now;
    }
}
