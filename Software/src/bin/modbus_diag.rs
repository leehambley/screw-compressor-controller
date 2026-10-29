//! Diagnostic-only: continuously blasts a Modbus RTU "read holding
//! register" request over the MAX3485 line (GPIO10 DE/RE, GPIO19 DI/TX,
//! GPIO18 RO/RX — corrected 2026-09-09, see main.rs) — near-continuous bus
//! chatter rather than a once-a-second blip, so it's easy to find on a
//! scope even with imperfect trigger settings. Logs whether a response
//! came back (throttled to ~1/sec), but works either way — this confirms
//! the ESP32 side is actually driving the line, not that a VFD is
//! listening yet.
//!
//! Not part of the compressor firmware — flash with
//! `cargo run --bin modbus-diag`, then reflash the real firmware
//! (`cargo run --bin compressor-controller`) when done.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer, with_timeout};
use esp_backtrace as _;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};

esp_bootloader_esp_idf::esp_app_desc!();

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

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let mut de_re = Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default());
    let mut uart = Uart::new(peripherals.UART1, UartConfig::default().with_baudrate(9600))
        .unwrap()
        .with_tx(peripherals.GPIO18)
        .with_rx(peripherals.GPIO19)
        .into_async();

    log::info!(
        "modbus-diag: booting, sending one frame/sec on GPIO18(TX)/GPIO19(RX), DE/RE on GPIO10"
    );

    // Read holding register 61453 (0xF00D — the VFD's min-frequency
    // register, P0-12), 1 register, slave address 1. Same request the
    // real firmware sends at startup, so a real VFD should actually
    // answer this one if it's connected and configured.
    let mut frame = [0x01u8, 0x03, 0xF0, 0x0D, 0x00, 0x01, 0x00, 0x00];
    let crc = crc16(&frame[..6]);
    frame[6..8].copy_from_slice(&crc.to_le_bytes());

    let mut n: u32 = 0;
    loop {
        n += 1;
        let verbose = n % 50 == 0; // log ~once/sec instead of ~20x/sec
        if verbose {
            log::info!("modbus-diag: -> frame #{n}: {:02X?}", frame);
        }

        de_re.set_high();
        embedded_io_async::Write::write_all(&mut uart, &frame)
            .await
            .unwrap();
        embedded_io_async::Write::flush(&mut uart).await.unwrap();
        Timer::after(Duration::from_millis(2)).await; // let the last byte clear the line
        de_re.set_low();

        let mut resp = [0u8; 16];
        match with_timeout(
            Duration::from_millis(20),
            embedded_io_async::Read::read(&mut uart, &mut resp),
        )
        .await
        {
            Ok(Ok(len)) if verbose => {
                log::info!("modbus-diag: <- {len} bytes: {:02X?}", &resp[..len])
            }
            Err(_) if verbose => {
                log::info!("modbus-diag: no response (timeout) — fine if nothing's listening yet")
            }
            _ => {}
        }

        Timer::after(Duration::from_millis(20)).await;
    }
}
