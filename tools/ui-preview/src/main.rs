//! Usage: cargo run --release -- <output dir>
//! Writes one 480x480 PNG per scene.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Platform, WindowAdapter};
use slint::Color;

slint::include_modules!();

// Same colours as Software/src/ui.rs.
const WHITE: u32 = 0xFFFFFF;
const BLACK: u32 = 0x000000;
const GREEN: u32 = 0x00FC00;
const GREEN_DARK: u32 = 0x007C00;
const RED: u32 = 0xF80000;
const AMBER: u32 = 0xF8A400;
const BLUE: u32 = 0x2868F8;
const GREY: u32 = 0x808080;
const GREY_DARK: u32 = 0x404040;

struct Scene {
    file: &'static str,
    pressure: &'static str,
    pressure_fg: u32,
    setpoint: &'static str,
    rpm: &'static str,
    kw: &'static str,
    motor_fg: u32,
    note: &'static str,
    note_fg: u32,
    status: &'static str,
    status_bg: u32,
    status_fg: u32,
    run_label: &'static str,
    run_bg: u32,
    run_fg: u32,
}

const SCENES: &[Scene] = &[
    Scene {
        file: "ui-running.png",
        pressure: "7.9", pressure_fg: GREEN, setpoint: "8.0",
        rpm: "2870", kw: "6.4", motor_fg: WHITE,
        note: "", note_fg: WHITE,
        status: "Running | 74C", status_bg: GREEN_DARK, status_fg: WHITE,
        run_label: "STOP", run_bg: RED, run_fg: WHITE,
    },
    Scene {
        file: "ui-standby.png",
        pressure: "8.3", pressure_fg: AMBER, setpoint: "8.0",
        rpm: "--", kw: "--", motor_fg: GREY,
        note: "At min speed, stop in 3:42", note_fg: AMBER,
        status: "Standby - restart below 6.0 bar | 61C", status_bg: BLUE, status_fg: WHITE,
        run_label: "STOP", run_bg: RED, run_fg: WHITE,
    },
    Scene {
        file: "ui-fault.png",
        pressure: "--", pressure_fg: RED, setpoint: "8.0",
        rpm: "--", kw: "--", motor_fg: GREY,
        note: "No reply from VFD", note_fg: RED,
        status: "FAULT: Pressure sensor fault - press RESET", status_bg: RED, status_fg: WHITE,
        run_label: "RESET", run_bg: AMBER, run_fg: BLACK,
    },
];

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

struct P(Rc<MinimalSoftwareWindow>);
impl Platform for P {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: ui-preview <output dir>");
    std::fs::create_dir_all(&out).unwrap();

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(480, 480));
    slint::platform::set_platform(Box::new(P(window.clone()))).unwrap();
    let ui = MainWindow::new().unwrap();
    ui.show().unwrap();

    for s in SCENES {
        ui.set_pressure(s.pressure.into());
        ui.set_pressure_fg(rgb(s.pressure_fg));
        ui.set_setpoint(s.setpoint.into());
        ui.set_rpm(s.rpm.into());
        ui.set_kw(s.kw.into());
        ui.set_motor_fg(rgb(s.motor_fg));
        ui.set_note(s.note.into());
        ui.set_note_fg(rgb(s.note_fg));
        ui.set_status(s.status.into());
        ui.set_status_bg(rgb(s.status_bg));
        ui.set_status_fg(rgb(s.status_fg));
        ui.set_run_label(s.run_label.into());
        ui.set_run_bg(rgb(s.run_bg));
        ui.set_run_fg(rgb(s.run_fg));

        slint::platform::update_timers_and_animations();
        // RGB565, like the board's framebuffer, so colour depth matches.
        let mut buf = vec![Rgb565Pixel(0); 480 * 480];
        window.draw_if_needed(|r| {
            r.render(&mut buf, 480);
        });
        let mut rgb8 = Vec::with_capacity(480 * 480 * 3);
        for p in &buf {
            let (r, g, b) = ((p.0 >> 11) & 31, (p.0 >> 5) & 63, p.0 & 31);
            rgb8.push(((r << 3) | (r >> 2)) as u8);
            rgb8.push(((g << 2) | (g >> 4)) as u8);
            rgb8.push(((b << 3) | (b >> 2)) as u8);
        }
        let path = format!("{out}/{}", s.file);
        let file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let mut enc = png::Encoder::new(file, 480, 480);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb8).unwrap();
        println!("wrote {path}");
    }
}
