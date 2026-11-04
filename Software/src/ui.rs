//! Touch UI: Slint (software renderer) drawing into the PSRAM framebuffer,
//! GT911 touch fed back in as pointer events.
//!
//! The control loop publishes `UiState` on `STATE` and receives `UiEvent`s
//! on `EVENTS`; nothing in here touches the controller directly. What to
//! show (texts, colours, button label) is decided by `phase`, `motor_note`
//! and `status_line`, carried over from the Nextion UI so the button and the
//! status bar still can't disagree.

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Timer};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, RenderingRotation, RepaintBufferType, Rgb565Pixel,
};
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{Color, LogicalPosition, SharedString};

use crate::board::display::{self, Display};
use crate::board::gt911::Gt911;
use crate::config;
use crate::sensors;
use crate::state_machine::{FaultReason, SpeedClamp, State};

slint::include_modules!();

/// Operator input, display -> control loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    RunState,
    SetPointUp,
    SetPointDown,
    /// UNLOAD pressed (`true`) / released (`false`).
    Unload(bool),
    /// PURGE pressed / released.
    Purge(bool),
}

pub static EVENTS: Channel<CriticalSectionRawMutex, UiEvent, 8> = Channel::new();

/// Everything the display shows, control loop -> display. Published after
/// every control tick and every handled `UiEvent`; the UI keeps the latest.
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
    /// Piggyback hardware interlock (watchdog AND over-temperature) closed.
    /// `None` when the piggyback ADC can't be read.
    pub interlock_ok: Option<bool>,
    /// Piggyback hardware over-temperature comparator says OK.
    pub hw_temp_ok: Option<bool>,
}

pub static STATE: Signal<CriticalSectionRawMutex, UiState> = Signal::new();

const BLACK: u32 = 0x000000;
const WHITE: u32 = 0xFFFFFF;
const GREEN: u32 = 0x00FC00;
const GREEN_DARK: u32 = 0x007C00;
const RED: u32 = 0xF80000;
const AMBER: u32 = 0xF8A400;
const BLUE: u32 = 0x2868F8;
const GREY: u32 = 0x808080;
const GREY_DARK: u32 = 0x404040;

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// What the operator sees the machine doing. The run button and the status
/// bar are both derived from this one value, so they can never disagree; in
/// particular a STOP press shows as Stopping straight away, before the
/// control loop's next tick has moved `state` on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Fault,
    Stopped,
    /// Run requested, waiting for pressure to fall to the restart point.
    Standby,
    Starting,
    Running,
    LowDemand,
    Stopping,
}

fn phase(s: &UiState) -> Phase {
    match (s.state, s.enabled) {
        _ if s.fault.is_some() => Phase::Fault,
        (State::Fault, _) => Phase::Fault,
        (State::Stopping, _) => Phase::Stopping,
        (State::Unloading | State::Running | State::UnloadIdle, false) => Phase::Stopping,
        (State::Idle, false) => Phase::Stopped,
        (State::Idle, true) => Phase::Standby,
        (State::Unloading, true) => Phase::Starting,
        (State::Running, true) => Phase::Running,
        (State::UnloadIdle, true) => Phase::LowDemand,
    }
}

/// The most relevant hint about what the motor/speed loop is doing and why,
/// or empty. Returns the font colour.
fn motor_note(s: &UiState, out: &mut impl core::fmt::Write) -> u32 {
    let (text, color) = if !s.vfd_comms_ok {
        ("No reply from VFD", RED)
    } else if s.unload_held {
        ("Manual unload (held)", AMBER)
    } else if s.state == State::Unloading && s.assist_open {
        ("Starting unloaded", AMBER)
    } else if s.state == State::Unloading {
        ("Building pressure", WHITE)
    } else if let Some(secs) = s.idle_stop_in_s {
        let _ = write!(out, "At min speed, stop in {}:{:02}", secs / 60, secs % 60);
        return AMBER;
    } else if s.clamp == SpeedClamp::Min {
        ("Clamped at min speed", AMBER)
    } else if s.clamp == SpeedClamp::Max {
        ("At max speed, demand > supply", AMBER)
    } else if s.state == State::Stopping {
        ("Decelerating", WHITE)
    } else {
        ("", WHITE)
    };
    let _ = out.write_str(text);
    color
}

/// Status bar: fault > interlock > sensor warnings > state. Returns
/// (background, font).
fn status_line(s: &UiState, out: &mut impl core::fmt::Write) -> (u32, u32) {
    if let Some(fault) = s.fault {
        let _ = write!(out, "FAULT: {} - press RESET", fault.as_str());
        return (RED, WHITE);
    }

    let warning = if s.hw_temp_ok == Some(false) {
        Some("Hardware over-temperature trip")
    } else if s.interlock_ok.is_none() {
        Some("Piggyback I/O not responding")
    } else if s.interlock_ok == Some(false) {
        Some("Safety interlock open")
    } else if s.pressure_bar.is_none() {
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
        let (text, colors) = match phase(s) {
            Phase::Standby => {
                let _ = write!(out, "Standby - restart below {:.1} bar", s.restart_below_bar);
                ("", (BLUE, WHITE))
            }
            Phase::Stopped => ("Stopped", (GREY_DARK, WHITE)),
            Phase::Starting => ("Starting", (GREEN_DARK, WHITE)),
            Phase::Running => ("Running", (GREEN_DARK, WHITE)),
            Phase::LowDemand => ("Running - low demand", (GREEN_DARK, WHITE)),
            Phase::Stopping => ("Stopping", (BLUE, WHITE)),
            // Fault reason is always set alongside; belt and braces.
            Phase::Fault => ("FAULT", (RED, WHITE)),
        };
        let _ = out.write_str(text);
        colors
    };
    if let Some(t) = s.temp_c {
        let _ = write!(out, " | {t:.0}C");
    }
    if s.drain_open {
        let _ = out.write_str(" | draining");
    }
    colors
}

fn apply(ui: &MainWindow, s: &UiState) {
    let mut text = heapless_str::<16>();
    let pressure_fg = match s.pressure_bar {
        Some(bar) => {
            let _ = write!(text, "{bar:.1}");
            if bar >= config::P_MAX_FAULT_BAR {
                RED
            } else if bar >= s.unload_above_bar {
                AMBER
            } else {
                GREEN
            }
        }
        None => {
            let _ = text.write_str("--");
            RED
        }
    };
    ui.set_pressure(SharedString::from(text.as_str()));
    ui.set_pressure_fg(rgb(pressure_fg));

    let mut text = heapless_str::<16>();
    let _ = write!(text, "{:.1}", s.setpoint_bar);
    ui.set_setpoint(SharedString::from(text.as_str()));

    let (mut rpm, mut kw) = (heapless_str::<16>(), heapless_str::<16>());
    let motor_fg = match (s.motor_hz, s.motor_amps) {
        (Some(hz), Some(amps)) => {
            let _ = write!(rpm, "{:.0}", sensors::motor_rpm(hz));
            let _ = write!(kw, "{:.1}", sensors::motor_kw_estimate(hz, amps));
            WHITE
        }
        _ => {
            let _ = rpm.write_str("--");
            let _ = kw.write_str("--");
            GREY
        }
    };
    ui.set_rpm(SharedString::from(rpm.as_str()));
    ui.set_kw(SharedString::from(kw.as_str()));
    ui.set_motor_fg(rgb(motor_fg));

    let mut note = heapless_str::<48>();
    let note_fg = motor_note(s, &mut note);
    ui.set_note(SharedString::from(note.as_str()));
    ui.set_note_fg(rgb(note_fg));

    let mut status = heapless_str::<96>();
    let (bg, fg) = status_line(s, &mut status);
    ui.set_status(SharedString::from(status.as_str()));
    ui.set_status_bg(rgb(bg));
    ui.set_status_fg(rgb(fg));

    let (label, bg, fg) = match phase(s) {
        Phase::Fault => ("RESET", AMBER, BLACK),
        Phase::Stopping => ("STOPPING", GREY, WHITE),
        Phase::Stopped => ("START", GREEN_DARK, WHITE),
        Phase::Standby | Phase::Starting | Phase::Running | Phase::LowDemand => ("STOP", RED, WHITE),
    };
    ui.set_run_label(SharedString::from(label));
    ui.set_run_bg(rgb(bg));
    ui.set_run_fg(rgb(fg));
}

/// Fixed-capacity string, so formatting never allocates.
struct FixedStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

fn heapless_str<const N: usize>() -> FixedStr<N> {
    FixedStr { buf: [0; N], len: 0 }
}

impl<const N: usize> FixedStr<N> {
    fn as_str(&self) -> &str {
        // Only ever filled from &str, cut at char boundaries in write_str.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for FixedStr<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut take = s.len().min(N - self.len);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        if take < s.len() { Err(core::fmt::Error) } else { Ok(()) }
    }
}

struct Platform {
    window: Rc<MinimalSoftwareWindow>,
}

impl slint::platform::Platform for Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_micros(Instant::now().as_micros())
    }
}

const FRAME_MS: u64 = 20;

#[embassy_executor::task]
pub async fn task(mut display: Display, mut touch: Gt911) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(display::WIDTH as u32, display::HEIGHT as u32));
    slint::platform::set_platform(Box::new(Platform { window: window.clone() }))
        .expect("slint platform");

    let ui = MainWindow::new().expect("slint window");
    ui.on_run_pressed(|| send(UiEvent::RunState));
    ui.on_setpoint_up(|| send(UiEvent::SetPointUp));
    ui.on_setpoint_down(|| send(UiEvent::SetPointDown));
    ui.on_unload(|held| send(UiEvent::Unload(held)));
    ui.on_purge(|held| send(UiEvent::Purge(held)));
    ui.show().expect("slint show");

    let mut touching: Option<LogicalPosition> = None;
    let mut touch_errors: u32 = 0;
    loop {
        if let Some(s) = STATE.try_take() {
            apply(&ui, &s);
        }

        match touch.read().await {
            Ok(frame) => {
                touch_errors = 0;
                if let Some(point) = frame {
                    pointer(&window, &mut touching, point);
                }
            }
            Err(()) => {
                touch_errors += 1;
                if touch_errors == 50 {
                    log::warn!("gt911: touch reads failing");
                }
            }
        }

        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            // Rgb565Pixel is a transparent u16, same layout as the framebuffer.
            let pixels = display.pixels();
            let buf = unsafe {
                core::slice::from_raw_parts_mut(pixels.as_mut_ptr().cast::<Rgb565Pixel>(), pixels.len())
            };
            if config::DISPLAY_ROTATE_180 {
                renderer.set_rendering_rotation(RenderingRotation::Rotate180);
            }
            let dirty = renderer.render(buf, display::WIDTH);
            let bb = dirty.bounding_box_origin();
            let size = dirty.bounding_box_size();
            if size.height > 0 {
                let y0 = (bb.y.max(0) as usize).min(display::HEIGHT);
                let y1 = (y0 + size.height as usize).min(display::HEIGHT);
                display.flush_rows(y0, y1);
                if config::DISPLAY_ROTATE_180 {
                    // Not sure which frame Slint reports the region in when
                    // rotated, so write back the mirror rows as well.
                    display.flush_rows(display::HEIGHT - y1, display::HEIGHT - y0);
                }
            }
        });

        Timer::after_millis(FRAME_MS).await;
    }
}

fn pointer(window: &MinimalSoftwareWindow, touching: &mut Option<LogicalPosition>, point: Option<(u16, u16)>) {
    let button = PointerEventButton::Left;
    match (point, *touching) {
        (Some((x, y)), was) => {
            let (x, y) = if config::DISPLAY_ROTATE_180 {
                ((display::WIDTH as u16 - 1).saturating_sub(x), (display::HEIGHT as u16 - 1).saturating_sub(y))
            } else {
                (x, y)
            };
            let position = LogicalPosition::new(x as f32, y as f32);
            let event = if was.is_none() {
                WindowEvent::PointerPressed { position, button }
            } else {
                WindowEvent::PointerMoved { position }
            };
            window.dispatch_event_with_result(event).ok();
            *touching = Some(position);
        }
        (None, Some(position)) => {
            window.dispatch_event_with_result(WindowEvent::PointerReleased { position, button }).ok();
            window.dispatch_event_with_result(WindowEvent::PointerExited).ok();
            *touching = None;
        }
        (None, None) => {}
    }
}

fn send(event: UiEvent) {
    if EVENTS.try_send(event).is_err() {
        log::warn!("ui: event queue full, dropped {event:?}");
    }
}
