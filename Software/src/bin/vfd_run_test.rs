//! Diagnostic-only: bench/commissioning test that drives the SAKO SKI-70 VFD
//! through a real start -> hold -> stop cycle using the actual `SkiVfd`
//! driver (not main.rs's full state machine — no solenoids, no ADC, no PID).
//! Reads the VFD's own min/max frequency limits (P0-12/P0-14) at startup and
//! runs at the reported minimum for a short hold, logging monitor readback
//! (freq/current/bus voltage) throughout.
//!
//! Flash with `cargo run --bin vfd-run-test`, then reflash the real firmware
//! (`cargo run --bin compressor-controller`) when done.

#![no_std]
#![no_main]

#[path = "../config.rs"]
mod config;
#[path = "../modbus.rs"]
mod modbus;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};
use modbus::SkiVfd;

esp_bootloader_esp_idf::esp_app_desc!();

const HOLD_MS: u64 = 10_000;
const MONITOR_INTERVAL_MS: u64 = 1_000;

async fn halt_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let modbus_uart = Uart::new(
        peripherals.UART0,
        UartConfig::default().with_baudrate(config::MODBUS_BAUD_RATE),
    )
    .unwrap()
    .with_tx(peripherals.GPIO43)
    .with_rx(peripherals.GPIO44)
    .into_async();

    let mut vfd = SkiVfd::new(modbus_uart);

    log::info!("vfd-run-test: reading frequency limits (P0-12/P0-14)...");
    let (min_hz, max_hz) = match vfd.read_freq_limits().await {
        Ok(limits) => limits,
        Err(e) => {
            log::error!(
                "vfd-run-test: failed to read freq limits: {e:?} — aborting, not issuing run command"
            );
            halt_forever().await;
        }
    };
    log::info!("vfd-run-test: VFD reports min={min_hz}Hz max={max_hz}Hz");

    if !(min_hz > 0.0 && max_hz >= min_hz) {
        log::error!(
            "vfd-run-test: implausible limits (min={min_hz}Hz max={max_hz}Hz) — aborting, not issuing run command"
        );
        halt_forever().await;
    }

    let target_hz = min_hz;
    log::info!("vfd-run-test: setting frequency setpoint to {target_hz}Hz");
    if let Err(e) = vfd.set_frequency_hz(target_hz).await {
        log::error!(
            "vfd-run-test: failed to set frequency: {e:?} — aborting, not issuing run command"
        );
        halt_forever().await;
    }

    log::info!("vfd-run-test: issuing RUN command");
    if let Err(e) = vfd.set_run(true).await {
        log::error!("vfd-run-test: failed to issue run command: {e:?} — aborting");
        halt_forever().await;
    }

    let ticks = HOLD_MS / MONITOR_INTERVAL_MS;
    for tick in 0..ticks {
        Timer::after(Duration::from_millis(MONITOR_INTERVAL_MS)).await;
        let elapsed_s = (tick + 1) * MONITOR_INTERVAL_MS / 1000;
        match vfd.read_monitor().await {
            Ok((freq, current, voltage)) => {
                log::info!(
                    "vfd-run-test: t={elapsed_s}s freq={freq}Hz current={current}A bus={voltage}V"
                );
            }
            Err(e) => log::warn!("vfd-run-test: t={elapsed_s}s monitor read failed: {e:?}"),
        }
    }

    log::info!("vfd-run-test: issuing STOP command");
    if let Err(e) = vfd.set_run(false).await {
        log::error!(
            "vfd-run-test: failed to issue stop command: {e:?} — MOTOR MAY STILL BE RUNNING, cut power manually if needed"
        );
    }

    Timer::after(Duration::from_millis(config::VFD_DECEL_MS)).await;
    match vfd.read_monitor().await {
        Ok((freq, current, voltage)) => {
            log::info!("vfd-run-test: post-stop freq={freq}Hz current={current}A bus={voltage}V");
        }
        Err(e) => log::warn!("vfd-run-test: post-stop monitor read failed: {e:?}"),
    }

    log::info!("vfd-run-test: done — safe to power down or reflash compressor-controller");
    halt_forever().await;
}
