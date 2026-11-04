//! The Rev 4.0 board's I/O expander: a CH32V003 running Waveshare's firmware
//! at 0x24. It drives the panel and touch resets, the system power latch and
//! the buzzer, and it dims the backlight by PWM.
//!
//! Registers and bits are from Waveshare's V4 demo (`io_extension`).

use embedded_hal_async::i2c::I2c;

use super::Dev;

const ADDR: u8 = 0x24;
const REG_DIRECTION: u8 = 0x02; // 1 = output
const REG_OUTPUT: u8 = 0x03;
const REG_PWM: u8 = 0x05; // backlight, 0-255

pub struct Ch32Exio {
    dev: Dev,
    out: u8,
}

impl Ch32Exio {
    /// Makes every pin but `inputs` an output and drives `initial` on them.
    /// (Waveshare's demo makes all eight outputs; TP_INT and RTC_INT are
    /// outputs of the GT911 and RTC, so we leave those as inputs.)
    pub async fn new(mut dev: Dev, inputs: u8, initial: u8) -> Result<Self, ()> {
        dev.write(ADDR, &[REG_DIRECTION, !inputs]).await.map_err(|_| ())?;
        dev.write(ADDR, &[REG_OUTPUT, initial]).await.map_err(|_| ())?;
        log::info!("ch32 expander at {ADDR:#04x}");
        Ok(Self { dev, out: initial })
    }

    pub async fn set(&mut self, mask: u8, high: bool) {
        self.out = if high { self.out | mask } else { self.out & !mask };
        if self.dev.write(ADDR, &[REG_OUTPUT, self.out]).await.is_err() {
            log::warn!("ch32 expander: write failed");
        }
    }

    /// Backlight brightness, 0-100 %.
    pub async fn backlight_pct(&mut self, pct: u8) {
        let duty = (pct.min(100) as u16 * 255 / 100) as u8;
        if self.dev.write(ADDR, &[REG_PWM, duty]).await.is_err() {
            log::warn!("ch32 expander: backlight write failed");
        }
    }
}
