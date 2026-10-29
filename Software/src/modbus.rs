//! Modbus RTU master for the SAKO SKI-70 VFD, over an auto-direction
//! TTL-RS485 module (breadboard build — the module keys its driver off the
//! TX line itself, so there's no DE/RE pin to drive). Register map and
//! timing per CLAUDE.md's "VFD Modbus" section — empirically discovered,
//! not in the manual.

use embassy_time::{Duration, Timer, with_timeout};
use esp_hal::Async;
use esp_hal::uart::Uart;

use crate::config;

#[derive(Debug, Clone, Copy)]
pub enum ModbusError {
    /// No response within the timeout window.
    Timeout,
    /// UART read/write failed.
    Io,
    /// Response too short to contain a valid frame.
    ShortFrame,
    /// CRC16 mismatch.
    BadCrc,
    /// Response didn't echo our slave address / function code, or the
    /// VFD returned an exception response (function code with high bit set).
    UnexpectedResponse,
}

const MAX_FRAME_LEN: usize = 5 + 2 * config::MODBUS_MAX_REGS_PER_READ as usize;

pub struct SkiVfd<'d> {
    uart: Uart<'d, Async>,
    byte_time: Duration,
}

impl<'d> SkiVfd<'d> {
    pub fn new(uart: Uart<'d, Async>) -> Self {
        // One byte-time (11 bits: start + 8 data + stop) — see `transact`.
        let byte_us = 11 * 1_000_000 / config::MODBUS_BAUD_RATE as u64;
        Self {
            uart,
            byte_time: Duration::from_micros(byte_us),
        }
    }

    pub async fn set_run(&mut self, run: bool) -> Result<(), ModbusError> {
        let value = if run { 1 } else { 5 };
        self.write_single_register(config::MODBUS_CONTROL_WORD_REG, value)
            .await
    }

    pub async fn set_frequency_hz(&mut self, hz: f32) -> Result<(), ModbusError> {
        let value = (hz * 200.0) as u16;
        self.write_single_register(config::MODBUS_FREQ_SETPOINT_REG, value)
            .await
    }

    /// Returns (running_freq_hz, output_current_a, bus_voltage_v).
    pub async fn read_monitor(&mut self) -> Result<(f32, f32, f32), ModbusError> {
        let mut regs = [0u16; 3];
        self.read_holding_registers(config::MODBUS_MONITOR_BASE_REG, &mut regs)
            .await?;
        Ok((
            regs[0] as f32 / 100.0,
            regs[1] as f32 / 100.0,
            regs[2] as f32 / 10.0,
        ))
    }

    /// Returns (min_hz, max_hz), read at startup per CLAUDE.md.
    pub async fn read_freq_limits(&mut self) -> Result<(f32, f32), ModbusError> {
        // 61453..=61455 spans 3 registers (min, unused, max) — one read.
        let mut regs = [0u16; 3];
        self.read_holding_registers(config::MODBUS_FREQ_MIN_REG, &mut regs)
            .await?;
        Ok((regs[0] as f32 / 200.0, regs[2] as f32 / 200.0))
    }

    async fn write_single_register(&mut self, reg: u16, value: u16) -> Result<(), ModbusError> {
        let mut frame = [0u8; 8];
        frame[0] = config::MODBUS_SLAVE_ADDR;
        frame[1] = 0x06;
        frame[2..4].copy_from_slice(&reg.to_be_bytes());
        frame[4..6].copy_from_slice(&value.to_be_bytes());
        let crc = crc16(&frame[..6]);
        frame[6..8].copy_from_slice(&crc.to_le_bytes());

        let mut response = [0u8; 8];
        self.transact(&frame, &mut response).await?;

        if response[..6] != frame[..6] {
            return Err(ModbusError::UnexpectedResponse);
        }
        Ok(())
    }

    async fn read_holding_registers(
        &mut self,
        start_reg: u16,
        out: &mut [u16],
    ) -> Result<(), ModbusError> {
        assert!(out.len() <= config::MODBUS_MAX_REGS_PER_READ as usize);

        let count = out.len() as u16;
        let mut frame = [0u8; 8];
        frame[0] = config::MODBUS_SLAVE_ADDR;
        frame[1] = 0x03;
        frame[2..4].copy_from_slice(&start_reg.to_be_bytes());
        frame[4..6].copy_from_slice(&count.to_be_bytes());
        let crc = crc16(&frame[..6]);
        frame[6..8].copy_from_slice(&crc.to_le_bytes());

        let expected_len = 5 + 2 * out.len();
        let mut response = [0u8; MAX_FRAME_LEN];
        self.transact(&frame, &mut response[..expected_len]).await?;

        if response[0] != config::MODBUS_SLAVE_ADDR
            || response[1] != 0x03
            || response[2] as usize != 2 * out.len()
        {
            return Err(ModbusError::UnexpectedResponse);
        }
        for (i, reg) in out.iter_mut().enumerate() {
            let hi = response[3 + 2 * i];
            let lo = response[4 + 2 * i];
            *reg = u16::from_be_bytes([hi, lo]);
        }
        Ok(())
    }

    /// Write `request` (already CRC'd), then read exactly `response.len()`
    /// bytes into it, verifying the trailing CRC16.
    async fn transact(&mut self, request: &[u8], response: &mut [u8]) -> Result<(), ModbusError> {
        self.discard_rx();
        embedded_io_async::Write::write_all(&mut self.uart, request)
            .await
            .map_err(|_| ModbusError::Io)?;
        embedded_io_async::Write::flush(&mut self.uart)
            .await
            .map_err(|_| ModbusError::Io)?;
        // Auto-direction modules can hand our own transmission (or a glitch
        // byte from the driver switching off) back on RO. The VFD waits at
        // least 3.5 byte-times before answering, so after one byte-time
        // anything in the FIFO is ours, not the reply.
        Timer::after(self.byte_time).await;
        self.discard_rx();

        with_timeout(
            Duration::from_millis(200),
            read_exact(&mut self.uart, response),
        )
        .await
        .map_err(|_| ModbusError::Timeout)??;

        if response.len() < 2 {
            return Err(ModbusError::ShortFrame);
        }
        let (data, crc_bytes) = response.split_at(response.len() - 2);
        let expected_crc = u16::from_le_bytes([crc_bytes[0], crc_bytes[1]]);
        if crc16(data) != expected_crc {
            return Err(ModbusError::BadCrc);
        }
        Ok(())
    }
}

impl SkiVfd<'_> {
    /// Empties the RX FIFO without waiting. Bounded: `read_buffered`
    /// reports (and clears) a latched framing/overflow error instead of
    /// reading, so a few passes are needed at most.
    fn discard_rx(&mut self) {
        let mut junk = [0u8; 32];
        for _ in 0..8 {
            match self.uart.read_buffered(&mut junk) {
                Ok(0) => return,
                Ok(_) | Err(_) => {}
            }
        }
    }
}

async fn read_exact(uart: &mut Uart<'_, Async>, buf: &mut [u8]) -> Result<(), ModbusError> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = embedded_io_async::Read::read(uart, &mut buf[filled..])
            .await
            .map_err(|_| ModbusError::Io)?;
        if n == 0 {
            return Err(ModbusError::Io);
        }
        filled += n;
    }
    Ok(())
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}
