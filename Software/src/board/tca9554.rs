//! TCA9554 8-bit I/O expander: panel and touch resets, backlight, buzzer.

use embedded_hal_async::i2c::I2c;

use super::Dev;

const REG_OUTPUT: u8 = 0x01;
const REG_CONFIG: u8 = 0x03; // 1 = input

pub struct Tca9554 {
    dev: Dev,
    addr: u8,
    out: u8,
}

impl Tca9554 {
    /// Finds the expander in 0x20-0x27 (the board's strap resistors are
    /// partly NC), drives every output high and makes `inputs` inputs.
    pub async fn new(mut dev: Dev, inputs: u8) -> Result<Self, ()> {
        let mut found = None;
        for addr in 0x20..=0x27 {
            if dev.write(addr, &[REG_OUTPUT, 0xFF & !super::EXIO_BUZZER]).await.is_ok() {
                found = Some(addr);
                break;
            }
        }
        let addr = found.ok_or(())?;
        let mut me = Self { dev, addr, out: 0xFF & !super::EXIO_BUZZER };
        me.dev.write(addr, &[REG_CONFIG, inputs]).await.map_err(|_| ())?;
        log::info!("tca9554 at {addr:#04x}");
        Ok(me)
    }

    pub async fn set(&mut self, mask: u8, high: bool) {
        self.out = if high { self.out | mask } else { self.out & !mask };
        if self.dev.write(self.addr, &[REG_OUTPUT, self.out]).await.is_err() {
            log::warn!("tca9554: write failed");
        }
    }
}
