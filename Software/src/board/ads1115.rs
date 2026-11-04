//! ADS1115 16-bit ADC, single-shot, single-ended, ±4.096 V range.
//!
//! The piggyback runs it from 3V3_A, so inputs never exceed ~3.3 V even
//! though the range reaches 4.096 V; 1 LSB = 125 µV.

use embassy_time::Timer;
use embedded_hal_async::i2c::I2c;

use super::Dev;

const REG_CONVERSION: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;

const OS_START: u16 = 1 << 15;
const PGA_4V096: u16 = 0b001 << 9;
const MODE_SINGLE: u16 = 1 << 8;
const DR_128SPS: u16 = 0b100 << 5;
const COMP_DISABLE: u16 = 0b11;
const LSB_V: f32 = 4.096 / 32768.0;

pub struct Ads1115 {
    dev: Dev,
    addr: u8,
}

impl Ads1115 {
    pub fn new(dev: Dev, addr: u8) -> Self {
        Self { dev, addr }
    }

    /// One conversion of AINn against GND, in volts. About 9 ms.
    pub async fn read_v(&mut self, channel: u8) -> Result<f32, ()> {
        let mux = (0b100 | (channel as u16 & 0b11)) << 12;
        let config = OS_START | mux | PGA_4V096 | MODE_SINGLE | DR_128SPS | COMP_DISABLE;
        let [hi, lo] = config.to_be_bytes();
        self.dev.write(self.addr, &[REG_CONFIG, hi, lo]).await.map_err(|_| ())?;
        Timer::after_millis(9).await; // 1/128 s plus margin
        let mut raw = [0u8; 2];
        self.dev
            .write_read(self.addr, &[REG_CONVERSION], &mut raw)
            .await
            .map_err(|_| ())?;
        Ok(i16::from_be_bytes(raw) as f32 * LSB_V)
    }

    /// Mean of `n` conversions, discarding a failed read rather than the batch.
    pub async fn read_mean_v(&mut self, channel: u8, n: usize) -> Result<f32, ()> {
        let (mut sum, mut got) = (0.0, 0);
        for _ in 0..n {
            if let Ok(v) = self.read_v(channel).await {
                sum += v;
                got += 1;
            }
        }
        if got == 0 { Err(()) } else { Ok(sum / got as f32) }
    }
}
