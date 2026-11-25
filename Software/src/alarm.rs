//! The beeper: errors only, never on touch or at start-up (docs/alerts.md).
//! `main` publishes the level each control tick; a touch anywhere acknowledges.
//! This task owns the CH32 expander after start-up, so nothing else writes its
//! output register (which also holds SYS_EN, the power latch).

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Ticker, Duration};

use crate::alarm_pattern::{Alarm, Level};
use crate::board::{self, ch32_exio::Ch32Exio};
use crate::config;
use crate::state_machine::FaultReason;
use crate::ui::UiState;

pub static LEVEL: Signal<CriticalSectionRawMutex, Level> = Signal::new();
pub static TOUCHED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// The alarm level for what the screen shows. Warnings are the amber
/// status-bar messages; their 10 s debounce is in `Alarm`.
pub fn level(s: &UiState, uptime_ms: u64) -> Level {
    if let Some(fault) = s.fault {
        return match fault {
            FaultReason::OverPressure | FaultReason::RotorOverTemp | FaultReason::MotorOverTemp => Level::Trip,
            FaultReason::PressureSensorFault
            | FaultReason::TempSensorFault
            | FaultReason::PressureBuildTimeout
            | FaultReason::StartRateLimitExceeded
            | FaultReason::ModbusCommsLost => Level::Fault,
        };
    }
    if s.hw_temp_ok == Some(false) || (s.interlock_ok == Some(false) && uptime_ms >= config::ALARM_BOOT_GRACE_MS) {
        return Level::Trip;
    }
    if s.interlock_ok.is_none() || s.pressure_bar.is_none() || s.temp_c.is_none() || !s.vfd_comms_ok {
        return Level::Warning;
    }
    Level::None
}

#[embassy_executor::task]
pub async fn task(mut exio: Ch32Exio) {
    let mut alarm = Alarm::new();
    let mut raw = Level::None;
    let mut on = false;
    let mut ticker = Ticker::every(Duration::from_millis(20));
    loop {
        ticker.next().await;
        if let Some(l) = LEVEL.try_take() {
            raw = l;
        }
        let touched = TOUCHED.try_take().is_some();
        let (was, was_acked) = (alarm.level(), alarm.acked());
        let want = alarm.update(raw, Instant::now().as_millis(), touched);
        if alarm.level() != was {
            log::warn!("alarm: {:?}", alarm.level());
        } else if alarm.acked() && !was_acked {
            log::info!("alarm: acknowledged");
        }
        if want != on {
            exio.set(board::EXIO_BUZZER, want).await;
            on = want;
        }
    }
}
