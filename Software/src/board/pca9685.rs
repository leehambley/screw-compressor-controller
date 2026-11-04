//! PCA9685 16-channel PWM, used for the valve gates (through the piggyback's
//! safety AND gates) and two status LEDs.

use embassy_time::Timer;
use embedded_hal_async::i2c::I2c;

use super::Dev;

const MODE1: u8 = 0x00;
const MODE2: u8 = 0x01;
const LED0_ON_L: u8 = 0x06;
const PRE_SCALE: u8 = 0xFE;

const MODE1_SLEEP: u8 = 1 << 4;
const MODE1_AI: u8 = 1 << 5;
const MODE2_OUTDRV: u8 = 1 << 2; // totem-pole outputs
const OSC_HZ: u32 = 25_000_000;

pub struct Pca9685 {
    dev: Dev,
    addr: u8,
}

impl Pca9685 {
    /// Sets the PWM frequency (the chip tops out at about 1.5 kHz) and turns
    /// every output fully off.
    pub async fn new(dev: Dev, addr: u8, pwm_hz: u32) -> Result<Self, ()> {
        let mut me = Self { dev, addr };
        let prescale = (OSC_HZ / (4096 * pwm_hz)).saturating_sub(1).clamp(3, 255) as u8;
        me.write(&[MODE1, MODE1_SLEEP | MODE1_AI]).await?;
        me.write(&[PRE_SCALE, prescale]).await?;
        me.write(&[MODE2, MODE2_OUTDRV]).await?;
        for ch in 0..16 {
            me.set_duty_pct(ch, 0).await?;
        }
        me.write(&[MODE1, MODE1_AI]).await?;
        Timer::after_micros(500).await; // oscillator start-up
        Ok(me)
    }

    pub async fn set_duty_pct(&mut self, channel: u8, pct: u8) -> Result<(), ()> {
        let reg = LED0_ON_L + 4 * channel;
        let frame = match pct {
            0 => [reg, 0, 0, 0, 0x10], // full off
            p if p >= 100 => [reg, 0, 0x10, 0, 0], // full on
            p => {
                let off = (p as u16 * 4096 / 100).to_le_bytes();
                [reg, 0, 0, off[0], off[1]]
            }
        };
        self.write(&frame).await
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        self.dev.write(self.addr, bytes).await.map_err(|_| ())
    }
}
