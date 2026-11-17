//! ST7701S panel controller setup over its 3-wire, 9-bit SPI (bit-banged:
//! it only runs once at boot).
//!
//! - `regs`: register names, with where each comes from.
//! - `panel`: the init sequence for our panel, in physical units where the
//!   register is understood.
//! - this file: the bit-banged link that sends it.
//!
//! The panel itself is then fed pixels over the RGB bus (see `display`).

mod panel;
mod regs;

use embassy_time::{Delay, Timer};
use embedded_hal::delay::DelayNs;
use esp_hal::gpio::Output;

use panel::Step;

pub struct St7701 {
    pub cs: Output<'static>,
    pub sck: Output<'static>,
    pub sda: Output<'static>,
}

impl St7701 {
    /// Call after the panel's hardware reset has been released.
    pub async fn init(&mut self) {
        self.cs.set_high();
        self.sck.set_high(); // mode 3: idle high
        for step in panel::WAVESHARE_4_480X480 {
            self.write(step);
            if step.delay_ms > 0 {
                Timer::after_millis(step.delay_ms as u64).await;
            }
        }
    }

    /// One command with its parameters, in a single chip-select frame. Every
    /// byte is preceded by a ninth bit: 0 = command, 1 = parameter.
    fn write(&mut self, step: &Step) {
        self.cs.set_low();
        self.byte(false, step.cmd);
        for &p in step.params {
            self.byte(true, p);
        }
        self.cs.set_high();
        Delay.delay_us(2);
    }

    fn byte(&mut self, is_param: bool, byte: u8) {
        self.bit(is_param);
        for b in (0..8).rev() {
            self.bit(byte & (1 << b) != 0);
        }
    }

    /// Data changes while SCK is low, panel samples on the rising edge.
    fn bit(&mut self, high: bool) {
        self.sck.set_low();
        if high { self.sda.set_high() } else { self.sda.set_low() }
        Delay.delay_ns(500); // 2 MHz max
        self.sck.set_high();
        Delay.delay_ns(500);
    }
}
