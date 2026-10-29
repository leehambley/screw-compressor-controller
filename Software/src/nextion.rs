//! Nextion HMI link: one task owns the whole display UART (both halves —
//! the baud rate is a per-peripheral setting, so it can't be changed from a
//! split TX or RX half), and does three jobs:
//!
//! 1. **Link health / baud.** Every `NEXTION_RESYNC_MS` it sends `sendme`
//!    at `NEXTION_BAUD_RATE` and waits for the page-number reply. No reply
//!    means the display has fallen back to `NEXTION_BOOT_BAUD` (it was
//!    power-cycled or rebooted while we kept running), so it drops to the
//!    boot baud, sends `baud=<fast>`, and comes back up. When the display is
//!    already at the fast baud nothing is resent: a 9600-baud frame arrives
//!    as junk at 115200 and would corrupt whatever command came next.
//!    Whenever the link (re)connects it re-learns the button ids, forces a
//!    full redraw, and releases any held buttons.
//! 2. **Input.** Parses the display's return frames; touch events from the
//!    known buttons become `UiEvent`s on `EVENTS`.
//! 3. **Output.** Renders the latest `UiState` (from `STATE`) into
//!    `objname.attr=value` commands, sending only those whose bytes changed
//!    since last sent, plus a periodic full refresh.
//!
//! HMI requirements (page 0): number components `nPressure`, `nSetPoint`,
//! `nMotorSpeed`, `nMotorKw` (Xfloat with vvs1=1 for all but nMotorSpeed —
//! see config.rs NEXTION_*_SCALE); text `tMotorNote` (txt_maxl >= 32),
//! `tStatus` (txt_maxl >= 60); buttons `bRunState` (txt_maxl >= 8),
//! `bSetPointMinus`, `bSetPointPlus` with "Send Component ID" ticked on
//! Touch Press, and `bUnload`, `bPurge` with it ticked on both Touch Press
//! and Touch Release.

use core::fmt::Write as _;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::Async;
use esp_hal::uart::{Config as UartConfig, Uart};

use crate::config;
use crate::sensors;
use crate::state_machine::{FaultReason, SpeedClamp, State};

/// Operator input, display -> control loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    RunState,
    SetPointUp,
    SetPointDown,
    /// bUnload pressed (`true`) / released (`false`).
    Unload(bool),
    /// bPurge pressed / released.
    Purge(bool),
}

pub static EVENTS: Channel<CriticalSectionRawMutex, UiEvent, 8> = Channel::new();

/// Everything the display shows, control loop -> display. Published after
/// every control tick and every handled `UiEvent`; the link task keeps the
/// latest one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiState {
    /// `None` => sensor fault (loop current below the 4mA floor).
    pub pressure_bar: Option<f32>,
    /// `None` => probe open/shorted.
    pub temp_c: Option<f32>,
    pub setpoint_bar: f32,
    pub restart_below_bar: f32,
    pub unload_above_bar: f32,
    pub motor_hz: Option<f32>,
    pub motor_amps: Option<f32>,
    pub vfd_comms_ok: bool,
    pub state: State,
    pub fault: Option<FaultReason>,
    pub enabled: bool,
    pub clamp: SpeedClamp,
    pub assist_open: bool,
    pub unload_held: bool,
    pub drain_open: bool,
    pub idle_stop_in_s: Option<u32>,
}

pub static STATE: Signal<CriticalSectionRawMutex, UiState> = Signal::new();

/// Buttons whose ids are looked up by name on every (re)connect. Index
/// order matters to `event_for`.
const BUTTONS: [&str; 5] = [
    "bRunState",
    "bSetPointMinus",
    "bSetPointPlus",
    "bUnload",
    "bPurge",
];

fn event_for(button: usize, press: bool) -> Option<UiEvent> {
    match (button, press) {
        (0, true) => Some(UiEvent::RunState),
        (1, true) => Some(UiEvent::SetPointDown),
        (2, true) => Some(UiEvent::SetPointUp),
        (3, held) => Some(UiEvent::Unload(held)),
        (4, held) => Some(UiEvent::Purge(held)),
        _ => None,
    }
}

const POLL_MS: u64 = 5;
const REPLY_TIMEOUT: Duration = Duration::from_millis(100);

#[embassy_executor::task]
pub async fn link_task(uart: Uart<'static, Async>) {
    let mut link = Link::new(uart);
    let mut renderer = Renderer::new();
    let mut ui: Option<UiState> = None;
    let mut link_up = false;
    let mut ever_up = false;
    let mut warned_never_up = false;
    let mut dirty = false;
    let mut next_probe = Instant::now();
    let mut next_refresh = Instant::now();

    loop {
        while let Some(frame) = link.next_frame() {
            link.dispatch(frame);
        }
        if link.rebooted {
            // Display came back up at our baud (it was set persistently
            // with `bauds=`) — no probe failure will tell us, so re-init now.
            link.rebooted = false;
            link_up = false;
            next_probe = Instant::now();
        }

        let now = Instant::now();
        if now >= next_probe {
            next_probe = now + Duration::from_millis(config::NEXTION_RESYNC_MS);
            // Needing the baud switch at all means the display restarted
            // since we last spoke, losing everything we'd sent it — re-init
            // even if the previous probe had the link up.
            let mut fresh = false;
            let ok = link.probe().await || {
                fresh = true;
                link.force_baud().await;
                link.probe().await
            };
            if ok && (fresh || !link_up) {
                log::info!("nextion: link up at {} baud", config::NEXTION_BAUD_RATE);
                link.init().await;
                renderer.invalidate();
                dirty = true;
            } else if !ok && link_up {
                log::warn!("nextion: link lost, retrying every {}ms", config::NEXTION_RESYNC_MS);
            } else if !ok && !ever_up && !warned_never_up {
                // Otherwise a display that never answers is silent in the log.
                log::warn!(
                    "nextion: no reply at {} or {} baud — check display power and TX/RX wiring",
                    config::NEXTION_BAUD_RATE,
                    config::NEXTION_BOOT_BAUD
                );
                warned_never_up = true;
            }
            ever_up |= ok;
            link_up = ok;
        }

        if let Some(state) = STATE.try_take() {
            dirty |= ui != Some(state);
            ui = Some(state);
        }
        if now >= next_refresh {
            next_refresh = now + Duration::from_millis(config::NEXTION_FULL_REFRESH_MS);
            renderer.invalidate();
            dirty = true;
        }
        if link_up
            && dirty
            && let Some(state) = &ui
        {
            renderer.render(&mut link, state).await;
            dirty = false;
        }

        Timer::after_millis(POLL_MS).await;
    }
}

/// A parsed display -> MCU frame (all end in `FF FF FF`).
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// `65 page component event`, event 1 = press, 0 = release.
    Touch { component: u8, press: bool },
    /// `66 page` — reply to `sendme`.
    Page(u8),
    /// `71 b0 b1 b2 b3` — reply to `get` of a numeric attribute.
    Number(i32),
    /// `88` — display finished booting.
    Ready,
    /// `00 00 00` — display power-on.
    Startup,
    /// Single-byte return codes (bkcmd success/failure, sleep/wake, ...).
    Code(u8),
    Other,
}

/// Byte-at-a-time frame assembler. Frames with a known fixed length are
/// collected by length, since their payload can legitimately contain
/// `FF` bytes (a `get` returning -1, say); the rest end at `FF FF FF`.
struct Parser {
    buf: [u8; 16],
    len: usize,
}

impl Parser {
    fn new() -> Self {
        Self {
            buf: [0; 16],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> Option<Frame> {
        if self.len == self.buf.len() {
            self.len = 0; // overlong: junk from a baud mismatch, start over
        }
        self.buf[self.len] = byte;
        self.len += 1;

        let n = self.len;
        let done = match self.buf[0] {
            0x65 => n == 7,
            0x66 => n == 5,
            0x71 => n == 8,
            _ => n >= 4 && self.buf[n - 3..n] == [0xFF; 3],
        };
        if !done {
            return None;
        }
        self.len = 0;

        let f = &self.buf[..n];
        if f[n - 3..] != [0xFF; 3] {
            return Some(Frame::Other);
        }
        Some(match (f[0], n) {
            (0x65, 7) => Frame::Touch {
                component: f[2],
                press: f[3] == 0x01,
            },
            (0x66, 5) => Frame::Page(f[1]),
            (0x71, 8) => Frame::Number(i32::from_le_bytes([f[1], f[2], f[3], f[4]])),
            (0x88, 4) => Frame::Ready,
            (0x00, 6) if f[1] == 0 && f[2] == 0 => Frame::Startup,
            (code, 4) => Frame::Code(code),
            _ => Frame::Other,
        })
    }
}

/// What a Nextion return code means, for the log. Codes only come back for
/// failures at the `bkcmd=2` level set in `Link::init`.
fn code_meaning(code: u8) -> &'static str {
    match code {
        0x00 => "invalid instruction",
        0x01 => "ok",
        0x02 => "invalid component id/name",
        0x03 => "invalid page id",
        0x1A => "invalid variable name or attribute",
        0x1B => "invalid variable operation",
        0x1C => "assignment failed",
        0x1E => "invalid number of parameters",
        0x23 => "variable name too long",
        0x24 => "serial buffer overflow",
        0x86 => "entered sleep",
        0x87 => "woke up",
        _ => "unknown",
    }
}

struct Link {
    uart: Uart<'static, Async>,
    parser: Parser,
    rx: [u8; 64],
    rx_len: usize,
    rx_pos: usize,
    /// Component id per `BUTTONS` entry, learned in `init`.
    ids: [Option<u8>; BUTTONS.len()],
    /// Set by `dispatch` on a boot/ready frame.
    rebooted: bool,
}

impl Link {
    fn new(uart: Uart<'static, Async>) -> Self {
        Self {
            uart,
            parser: Parser::new(),
            rx: [0; 64],
            rx_len: 0,
            rx_pos: 0,
            ids: [None; BUTTONS.len()],
            rebooted: false,
        }
    }

    fn set_baud(&mut self, baud: u32) {
        if let Err(e) = self
            .uart
            .apply_config(&UartConfig::default().with_baudrate(baud))
        {
            log::error!("nextion: can't set {baud} baud: {e:?}");
        }
    }

    /// Next complete frame from what's already in the UART FIFO, without
    /// waiting.
    fn next_frame(&mut self) -> Option<Frame> {
        loop {
            if self.rx_pos == self.rx_len {
                self.rx_pos = 0;
                self.rx_len = self.uart.read_buffered(&mut self.rx).unwrap_or(0);
                if self.rx_len == 0 {
                    return None;
                }
            }
            let byte = self.rx[self.rx_pos];
            self.rx_pos += 1;
            if let Some(frame) = self.parser.push(byte) {
                return Some(frame);
            }
        }
    }

    /// Handles what every frame needs regardless of who's waiting for it
    /// (touch -> `EVENTS`, reboot detection, logging failures), and hands
    /// it back so `await_frame` can match replies.
    fn dispatch(&mut self, frame: Frame) -> Frame {
        match frame {
            Frame::Touch { component, press } => {
                let button = self.ids.iter().position(|&id| id == Some(component));
                match button.and_then(|b| event_for(b, press)) {
                    Some(event) => {
                        log::info!("nextion: {event:?}");
                        if EVENTS.try_send(event).is_err() {
                            log::warn!("nextion: event queue full, dropped {event:?}");
                        }
                    }
                    None if button.is_none() => {
                        log::warn!("nextion: touch from unknown component id {component}")
                    }
                    None => {} // e.g. release of a press-only button
                }
            }
            Frame::Ready | Frame::Startup => {
                log::info!("nextion: display (re)booted");
                self.rebooted = true;
            }
            Frame::Code(code) => {
                log::warn!("nextion: return code {code:#04x} ({})", code_meaning(code))
            }
            Frame::Page(_) | Frame::Number(_) => {}
            Frame::Other => log::debug!("nextion: unparsed frame"),
        }
        frame
    }

    async fn await_frame(&mut self, want: fn(&Frame) -> bool) -> Option<Frame> {
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            while let Some(frame) = self.next_frame() {
                let frame = self.dispatch(frame);
                if want(&frame) {
                    return Some(frame);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            Timer::after_millis(2).await;
        }
    }

    /// Sends one command plus the `FF FF FF` terminator. An empty command
    /// is just the terminator: it makes the display discard any partial
    /// junk it has buffered.
    async fn send(&mut self, cmd: &[u8]) -> bool {
        log::debug!("nextion: tx {:?}", core::str::from_utf8(cmd).unwrap_or("<bin>"));
        let ok = embedded_io_async::Write::write_all(&mut self.uart, cmd).await.is_ok()
            && embedded_io_async::Write::write_all(&mut self.uart, &[0xFF; 3])
                .await
                .is_ok()
            && embedded_io_async::Write::flush(&mut self.uart).await.is_ok();
        if !ok {
            log::warn!("nextion: uart write failed");
        }
        ok
    }

    /// Is the display answering at the fast baud?
    async fn probe(&mut self) -> bool {
        self.send(b"sendme").await;
        self.await_frame(|f| matches!(f, Frame::Page(_))).await.is_some()
    }

    /// Talks to the display at its boot baud and tells it to switch to ours.
    async fn force_baud(&mut self) {
        let mut cmd = CmdBuf::<24>::new();
        let _ = write!(cmd, "baud={}", config::NEXTION_BAUD_RATE);

        self.set_baud(config::NEXTION_BOOT_BAUD);
        self.send(b"").await;
        self.send(cmd.as_bytes()).await;
        // `flush` returns once the FIFO is empty; give the last byte time to
        // leave the shift register before the baud changes under it.
        Timer::after_millis(5).await;
        self.set_baud(config::NEXTION_BAUD_RATE);
        Timer::after_millis(50).await; // display switching over
        self.send(b"").await;
        while self.next_frame().is_some() {} // junk received while mismatched
    }

    /// Fresh-connection setup: error-only acks, button ids by name, and
    /// release of anything that might have been held when the link dropped
    /// (the release event is gone with it).
    async fn init(&mut self) {
        self.send(b"bkcmd=2").await;

        for (i, name) in BUTTONS.iter().enumerate() {
            let mut cmd = CmdBuf::<32>::new();
            let _ = write!(cmd, "get {name}.id");
            self.send(cmd.as_bytes()).await;
            self.ids[i] = match self
                .await_frame(|f| matches!(f, Frame::Number(_) | Frame::Code(_)))
                .await
            {
                Some(Frame::Number(id)) => {
                    log::info!("nextion: {name} is component {id}");
                    Some(id as u8)
                }
                other => {
                    log::error!(
                        "nextion: no id for {name} ({other:?}) — check the component name in the HMI"
                    );
                    None
                }
            };
        }

        for event in [UiEvent::Unload(false), UiEvent::Purge(false)] {
            let _ = EVENTS.try_send(event);
        }
    }
}

/// Fixed-capacity `core::fmt::Write` sink for building one outbound
/// Nextion command in a stack buffer — no allocator in this `no_std` build.
struct CmdBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> CmdBuf<N> {
    fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for CmdBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len + bytes.len();
        if end > N {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

/// RGB565. Guesses at a dark theme — adjust to taste.
const BLACK: u16 = 0;
const WHITE: u16 = 0xFFFF;
const GREEN: u16 = 0x07E0;
const GREEN_DARK: u16 = 0x03E0;
const GREEN_DARKER: u16 = 0x0200;
const RED: u16 = 0xF800;
const RED_DARK: u16 = 0xA000;
const AMBER: u16 = 0xFD20;
const AMBER_DARK: u16 = 0xC4A0;
const BLUE: u16 = 0x2B5F;
const GREY: u16 = 0x8410;
const GREY_DARK: u16 = 0x4208;

/// One entry per outbound `objname.attr`, for change detection.
#[derive(Clone, Copy)]
enum Slot {
    PressureVal,
    PressurePco,
    SetPointVal,
    SpeedVal,
    SpeedPco,
    KwVal,
    KwPco,
    NoteTxt,
    NotePco,
    StatusTxt,
    StatusBco,
    StatusPco,
    RunTxt,
    RunBco,
    RunPco,
    RunBco2,
    RunPco2,
}
const SLOTS: usize = Slot::RunPco2 as usize + 1;

struct Renderer {
    /// FNV-1a hash of the command last sent per slot; 0 = unknown, send.
    sent: [u32; SLOTS],
}

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

fn scaled(value: f32, scale: f32) -> i32 {
    libm::roundf(value * scale) as i32
}

impl Renderer {
    fn new() -> Self {
        Self { sent: [0; SLOTS] }
    }

    fn invalidate(&mut self) {
        self.sent = [0; SLOTS];
    }

    async fn emit(&mut self, link: &mut Link, slot: Slot, cmd: core::fmt::Arguments<'_>) {
        let mut buf = CmdBuf::<96>::new();
        if buf.write_fmt(cmd).is_err() {
            log::warn!("nextion: command too long, skipped");
            return;
        }
        let hash = fnv1a(buf.as_bytes());
        if self.sent[slot as usize] == hash {
            return;
        }
        self.sent[slot as usize] = if link.send(buf.as_bytes()).await { hash } else { 0 };
    }

    async fn render(&mut self, link: &mut Link, s: &UiState) {
        let (bar, pco) = match s.pressure_bar {
            Some(bar) if bar >= config::P_MAX_FAULT_BAR => (bar, RED),
            Some(bar) if bar >= s.unload_above_bar => (bar, AMBER),
            Some(bar) => (bar, GREEN),
            None => (0.0, RED),
        };
        let val = scaled(bar, config::NEXTION_PRESSURE_SCALE);
        self.emit(link, Slot::PressureVal, format_args!("nPressure.val={val}")).await;
        self.emit(link, Slot::PressurePco, format_args!("nPressure.pco={pco}")).await;

        let val = scaled(s.setpoint_bar, config::NEXTION_SETPOINT_SCALE);
        self.emit(link, Slot::SetPointVal, format_args!("nSetPoint.val={val}")).await;

        let (rpm, kw, pco) = match (s.motor_hz, s.motor_amps) {
            (Some(hz), Some(amps)) => (
                scaled(sensors::motor_rpm(hz), 1.0),
                scaled(sensors::motor_kw_estimate(hz, amps), config::NEXTION_KW_SCALE),
                BLACK,
            ),
            _ => (0, 0, GREY),
        };
        self.emit(link, Slot::SpeedVal, format_args!("nMotorSpeed.val={rpm}")).await;
        self.emit(link, Slot::SpeedPco, format_args!("nMotorSpeed.pco={pco}")).await;
        self.emit(link, Slot::KwVal, format_args!("nMotorKw.val={kw}")).await;
        self.emit(link, Slot::KwPco, format_args!("nMotorKw.pco={pco}")).await;

        let mut note = CmdBuf::<40>::new();
        let pco = motor_note(s, &mut note);
        let note = note.as_str();
        self.emit(link, Slot::NoteTxt, format_args!("tMotorNote.txt=\"{note}\"")).await;
        self.emit(link, Slot::NotePco, format_args!("tMotorNote.pco={pco}")).await;

        let mut status = CmdBuf::<64>::new();
        let (bco, pco) = status_line(s, &mut status);
        let status = status.as_str();
        self.emit(link, Slot::StatusTxt, format_args!("tStatus.txt=\"{status}\"")).await;
        self.emit(link, Slot::StatusBco, format_args!("tStatus.bco={bco}")).await;
        self.emit(link, Slot::StatusPco, format_args!("tStatus.pco={pco}")).await;

        // bco2/pco2 are the pressed-state colors.
        let (txt, bco, bco2, pco) = if s.state == State::Fault {
            ("RESET", AMBER, AMBER_DARK, BLACK)
        } else if s.enabled {
            ("STOP", RED, RED_DARK, WHITE)
        } else if s.state == State::Stopping {
            ("STOPPING", GREY, GREY_DARK, WHITE)
        } else {
            ("START", GREEN_DARK, GREEN_DARKER, WHITE)
        };
        self.emit(link, Slot::RunTxt, format_args!("bRunState.txt=\"{txt}\"")).await;
        self.emit(link, Slot::RunBco, format_args!("bRunState.bco={bco}")).await;
        self.emit(link, Slot::RunBco2, format_args!("bRunState.bco2={bco2}")).await;
        self.emit(link, Slot::RunPco, format_args!("bRunState.pco={pco}")).await;
        self.emit(link, Slot::RunPco2, format_args!("bRunState.pco2={pco}")).await;
    }
}

/// tMotorNote: the most relevant hint about what the motor/speed loop is
/// doing and why, or empty. Returns the font color.
fn motor_note(s: &UiState, out: &mut CmdBuf<40>) -> u16 {
    let (text, color) = if !s.vfd_comms_ok {
        ("No reply from VFD", RED)
    } else if s.unload_held {
        ("Manual unload (held)", AMBER)
    } else if s.state == State::Unloading && s.assist_open {
        ("Starting unloaded", AMBER)
    } else if s.state == State::Unloading {
        ("Building pressure", BLACK)
    } else if let Some(secs) = s.idle_stop_in_s {
        let _ = write!(out, "At min speed, stop in {}:{:02}", secs / 60, secs % 60);
        return AMBER;
    } else if s.clamp == SpeedClamp::Min {
        ("Clamped at min speed", AMBER)
    } else if s.clamp == SpeedClamp::Max {
        ("At max speed, demand > supply", AMBER)
    } else if s.state == State::Stopping {
        ("Decelerating", BLACK)
    } else {
        ("", BLACK)
    };
    let _ = out.write_str(text);
    color
}

/// tStatus: fault > sensor warnings > state. Returns (background, font).
fn status_line(s: &UiState, out: &mut CmdBuf<64>) -> (u16, u16) {
    if let Some(fault) = s.fault {
        let _ = write!(out, "FAULT: {} - press RESET", fault.as_str());
        return (RED, WHITE);
    }

    let warning = if s.pressure_bar.is_none() {
        Some("Pressure sensor fault")
    } else if s.temp_c.is_none() {
        Some("Temp sensor fault")
    } else if !s.vfd_comms_ok {
        Some("VFD not responding")
    } else {
        None
    };

    let colors = if let Some(warning) = warning {
        let _ = out.write_str(warning);
        (AMBER, BLACK)
    } else {
        match (s.state, s.enabled) {
            (State::Idle, true) => {
                let _ = write!(out, "Standby - restart below {:.1} bar", s.restart_below_bar);
                (BLUE, WHITE)
            }
            (State::Idle, false) => {
                let _ = out.write_str("Stopped");
                (GREY_DARK, WHITE)
            }
            (State::Unloading, _) => {
                let _ = out.write_str("Starting");
                (GREEN_DARK, WHITE)
            }
            (State::Running, _) => {
                let _ = out.write_str("Running");
                (GREEN_DARK, WHITE)
            }
            (State::UnloadIdle, _) => {
                let _ = out.write_str("Running - low demand");
                (GREEN_DARK, WHITE)
            }
            (State::Stopping, _) => {
                let _ = out.write_str("Stopping");
                (BLUE, WHITE)
            }
            (State::Fault, _) => {
                let _ = out.write_str("FAULT"); // fault reason always set alongside; belt and braces
                (RED, WHITE)
            }
        }
    };
    if let Some(t) = s.temp_c {
        let _ = write!(out, " | {t:.0}C");
    }
    if s.drain_open {
        let _ = out.write_str(" | draining");
    }
    colors
}
