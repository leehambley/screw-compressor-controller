#![no_std]
#![no_main]

mod config;
mod modbus;
mod nextion;
mod pid;
mod sensors;
mod state_machine;
mod storage;
mod valve;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Instant, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::analog::adc::{Adc, AdcCalCurve, AdcConfig, Attenuation};
use esp_hal::gpio::DriveMode;
use esp_hal::ledc::channel::{self as ledc_channel, ChannelIFace};
use esp_hal::ledc::timer::{self as ledc_timer, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{PulseCode, Rmt};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use modbus::SkiVfd;
use nextion::{UiEvent, UiState};
use core::sync::atomic::{AtomicBool, Ordering};
use smart_leds::hsv::{Hsv, hsv2rgb};
use smart_leds::{RGB8, SmartLedsWriteAsync};
use static_cell::StaticCell;
use state_machine::{Controller, FaultReason, State};
use storage::CounterStore;
use valve::PwmValve;

esp_bootloader_esp_idf::esp_app_desc!();

/// Trimmed mean of 32 back-to-back calibrated readings (mV): sorts, drops
/// the 8 lowest and 8 highest, averages the middle 16. The trim rejects
/// single-shot glitches the way the old median-of-5 did; averaging what's
/// left also beats down the sample-to-sample noise, which on the
/// breadboard was ±25mV (±0.25 bar) and enough to trip the 4mA-floor
/// sensor-fault check. A macro because spelling out `AdcPin`'s generic
/// parameters for a helper fn costs more than it saves.
macro_rules! trimmed_mean_mv {
    ($adc:expr, $pin:expr) => {{
        let mut samples = [0u16; 32];
        for sample in &mut samples {
            *sample = nb::block!($adc.read_oneshot(&mut $pin)).unwrap();
        }
        samples.sort_unstable();
        let kept = &samples[8..24];
        kept.iter().map(|&mv| mv as u32).sum::<u32>() as f32 / kept.len() as f32
    }};
}

/// Placeholder periodic task — proves the executor is actually scheduling
/// more than one task concurrently with `main`. Replace with the real
/// sensor-poll task (ADC_SAMPLE_MS cadence) once ADC init lands.
#[embassy_executor::task]
async fn heartbeat() {
    loop {
        log::info!("heartbeat");
        Timer::after(Duration::from_secs(5)).await;
    }
}

const LED_BUFFER_LEN: usize = buffer_size_async(1);

/// Mirrors `controller.fault.is_some()` for `led_task`.
static FAULTED: AtomicBool = AtomicBool::new(false);

/// Onboard WS2812 proof of life: a slow, dim rainbow while all is well,
/// a red blink while faulted. Its own task so the animation stays smooth
/// whatever the control loop is waiting on (Modbus timeouts and so on).
/// The rainbow keeps cycling as long as the executor is alive, so a
/// frozen colour means a hung firmware.
#[embassy_executor::task]
async fn led_task(mut led: SmartLedsAdapterAsync<'static, LED_BUFFER_LEN>) {
    let mut hue: u8 = 0;
    let mut ticks: u32 = 0;
    loop {
        let colour = if FAULTED.load(Ordering::Relaxed) {
            if (ticks / 25) % 2 == 0 {
                RGB8 { r: 40, g: 0, b: 0 }
            } else {
                RGB8::default()
            }
        } else {
            hsv2rgb(Hsv {
                hue,
                sat: 255,
                val: 30,
            })
        };
        let _ = led.write([colour].into_iter()).await;
        hue = hue.wrapping_add(1);
        ticks = ticks.wrapping_add(1);
        Timer::after_millis(20).await; // ~5s per rainbow cycle
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    log::info!("compressor-controller booting");

    // esp-rtos owns one hardware timer + one software interrupt to drive
    // the Embassy executor and its timer queue — this is the "RTOS" layer
    // everything else (sensor poll, Modbus comms, state machine tick,
    // button debounce) gets spawned as tasks on top of.
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    spawner.spawn(heartbeat()).unwrap();

    // Pressure sensor (4-20mA loop, 100Ω shunt) — see sensors::pressure_bar.
    // NTC rotor/oil probe (10k pull-up divider) — see sensors::ntc_temp_c.
    //
    // Corrected 2026-09-10, two separate bugs:
    // 1. `enable_pin` (no calibration scheme) leaves the ADC's hardware
    //    init-code calibration register at an uncalibrated default — found
    //    while chasing a ~40x offset between a multimeter reading directly
    //    on R2 (tens of mV) and what read_oneshot reported (over 1V) for
    //    the same node at the same moment. `AdcCalCurve` applies esp-idf's
    //    factory-derived per-attenuation error-correction polynomial (real
    //    hardware calibration, not a software fudge factor) and — this
    //    changes what read_oneshot returns — makes it return
    //    already-calibrated **millivolts** directly, not a raw ADC count,
    //    so the raw-to-volts conversion below changed to match.
    // 2. 11dB attenuation gives a ~2.5-3.9V full-scale range, wasting most
    //    of the ADC's 12-bit resolution on a signal that's nominally
    //    0.4-2.0V (4-20mA * 100Ω) at most. 6dB's ~1.75-2.2V full scale is a
    //    much closer match. The NTC divider spans most of 0-3.3V (room
    //    temperature is ~3.0V with the 10k pull-up), so it needs 11dB.
    let mut adc1_config = AdcConfig::new();
    let mut pressure_pin =
        adc1_config.enable_pin_with_cal::<_, AdcCalCurve<_>>(peripherals.GPIO0, Attenuation::_6dB);
    let mut temp_pin =
        adc1_config.enable_pin_with_cal::<_, AdcCalCurve<_>>(peripherals.GPIO1, Attenuation::_11dB);
    let mut adc1 = Adc::new(peripherals.ADC1, adc1_config);
    // The first conversions after init are garbage (seen on the breadboard:
    // 484mV pressure / 100mV NTC = 250°C on the very first tick) — throw a
    // round away before anything acts on a reading.
    let _ = trimmed_mean_mv!(adc1, pressure_pin);
    let _ = trimmed_mean_mv!(adc1, temp_pin);

    // Modbus RTU to the SKI-70 over an auto-direction TTL-RS485 module
    // (breadboard build) — no DE/RE pin, the module switches on TX activity.
    // GPIO18 -> module RXD (our TX), GPIO19 <- module TXD (our RX).
    let modbus_uart = Uart::new(
        peripherals.UART1,
        UartConfig::default().with_baudrate(config::MODBUS_BAUD_RATE),
    )
    .unwrap()
    .with_tx(peripherals.GPIO18)
    .with_rx(peripherals.GPIO19)
    .into_async();

    // Nextion HMI, plain UART (3.3V logic levels both ways). Started at the
    // fast baud: the link task's first
    // probe finds out whether the display is already there and switches it
    // over if not — see nextion.rs.
    let nextion_uart = Uart::new(
        peripherals.UART0,
        UartConfig::default().with_baudrate(config::NEXTION_BAUD_RATE),
    )
    .unwrap()
    .with_tx(peripherals.GPIO23)
    .with_rx(peripherals.GPIO22)
    .into_async();
    spawner.spawn(nextion::link_task(nextion_uart)).unwrap();

    // Solenoid MOSFET gates (IRLZ34N, low side), PWM'd by LEDC for pull-in
    // then hold — see valve.rs. Both start at 0% (off, so both valves open:
    // both are normally-open); the controller drives them to their idle
    // state on its first `service`, within one loop pass of boot.
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    static VALVE_TIMER: StaticCell<ledc_timer::Timer<'static, LowSpeed>> = StaticCell::new();
    let valve_timer = VALVE_TIMER.init(ledc.timer::<LowSpeed>(ledc_timer::Number::Timer0));
    valve_timer
        .configure(ledc_timer::config::Config {
            duty: ledc_timer::config::Duty::Duty10Bit,
            clock_source: ledc_timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(config::VALVE_PWM_HZ),
        })
        .expect("valve PWM timer");
    let valve_timer: &'static ledc_timer::Timer<'static, LowSpeed> = valve_timer;
    let valve_channel = |number, pin: esp_hal::gpio::AnyPin<'static>| {
        let mut channel = ledc.channel::<LowSpeed>(number, pin);
        channel
            .configure(ledc_channel::config::Config {
                timer: valve_timer,
                duty_pct: 0,
                drive_mode: DriveMode::PushPull,
            })
            .expect("valve PWM channel");
        channel
    };
    let solenoid_unload = PwmValve::new(
        valve_channel(ledc_channel::Number::Channel0, peripherals.GPIO20.into()),
        "start-assist",
    );
    let solenoid_drain = PwmValve::new(
        valve_channel(ledc_channel::Number::Channel1, peripherals.GPIO21.into()),
        "drain",
    );

    // Onboard WS2812 on GPIO8 (ESP32-C6-DevKitC-1), driven over RMT — see
    // led_task. GPIO8 is a strapping pin but nothing else uses it on this
    // build; the LED's own pull doesn't upset the boot strap.
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .expect("failed to init RMT")
        .into_async();
    static RMT_BUFFER: StaticCell<[PulseCode; LED_BUFFER_LEN]> = StaticCell::new();
    let rmt_buffer = RMT_BUFFER.init([PulseCode::default(); LED_BUFFER_LEN]);
    let led = SmartLedsAdapterAsync::new(rmt.channel0, peripherals.GPIO8, rmt_buffer);
    spawner.spawn(led_task(led)).unwrap();

    let vfd = SkiVfd::new(modbus_uart);
    let store = CounterStore::new(peripherals.FLASH);
    let mut controller = Controller::new(vfd, store, solenoid_unload, solenoid_drain).await;

    log::info!(
        "counters at boot: {} starts, {}s runtime, setpoint {:.1} bar",
        controller.counters().total_starts,
        controller.counters().total_runtime_s,
        controller.setpoint_bar()
    );

    let mut ticker = Ticker::every(Duration::from_millis(config::ADC_SAMPLE_MS));
    let mut last_tick = Instant::now();
    let mut pressure_bar: Option<f32> = None;
    let mut temp_c: Option<f32> = None;
    let mut sensor_fault_ticks: u8 = 0;

    // One task, two wake-ups: the control tick every ADC_SAMPLE_MS, and HMI
    // input as soon as it arrives (so +/- and the hold buttons respond
    // immediately rather than on the next tick). Either way the display
    // gets a fresh UiState afterwards.
    loop {
        match select(ticker.next(), nextion::EVENTS.receive()).await {
            Either::First(()) => {
                // Median-of-5: occasional single-shot glitches (2026-09-10
                // bench test — a steady ~4.2mA reading spiking to a spurious
                // ~9mA or ~38mA for exactly one sample, never corroborated by
                // an in-series ammeter) are what Espressif's ADC docs warn
                // about. The breadboard build adds the RC filter they
                // recommend (4.7k/100nF pressure, 1k/100nF NTC); the median
                // stays as a second line of defence.
                let quiet_from = Instant::now();
                controller.valves_quiet(true);
                Timer::after_millis(config::VALVE_QUIET_SETTLE_MS).await;
                let pressure_mv = trimmed_mean_mv!(adc1, pressure_pin);
                let temp_mv = trimmed_mean_mv!(adc1, temp_pin);
                controller.valves_quiet(false);
                log::debug!("adc: valves quiet for {}us", quiet_from.elapsed().as_micros());
                // With AdcCalCurve, read_oneshot returns calibrated
                // millivolts, so this is a unit conversion, not a scale.
                let pressure_v =
                    pressure_mv / 1000.0 * config::PRESSURE_ADC_GAIN_CORRECTION;
                let temp_v = temp_mv / 1000.0;
                log::info!(
                    "adc: pressure raw_mv={pressure_mv:.1} -> {pressure_v:.3}V {:.2}mA, ntc raw_mv={temp_mv:.1} (compare against a meter on GPIO0/GPIO1)",
                    sensors::loop_current_ma(pressure_v)
                );

                pressure_bar =
                    (!sensors::pressure_sensor_fault(pressure_v)).then(|| sensors::pressure_bar(pressure_v));
                temp_c = (!sensors::ntc_sensor_fault(temp_v)).then(|| sensors::ntc_temp_c(temp_v));

                let now = Instant::now();
                let dt_s = now.duration_since(last_tick).as_millis() as f32 / 1000.0;
                last_tick = now;

                controller.poll_vfd().await;

                // A bad sensor only faults the machine once it's been asked
                // to run (or is running). Stopped and idle, it's shown as a
                // warning on tStatus instead of demanding a RESET.
                let wants_to_run = controller.enabled() || controller.state != State::Idle;
                let sensor_fault = match (pressure_bar, temp_c) {
                    (None, _) => Some(FaultReason::PressureSensorFault),
                    (Some(_), None) if config::TEMP_SENSOR_REQUIRED => {
                        Some(FaultReason::TempSensorFault)
                    }
                    _ => None,
                };
                match (sensor_fault, pressure_bar) {
                    (Some(reason), _) => {
                        log::warn!("sensor fault: {reason:?}");
                        // Skips this tick (outputs hold) until confirmed —
                        // see FAULT_CONFIRM_TICKS.
                        if state_machine::confirmed(&mut sensor_fault_ticks, true)
                            && wants_to_run
                            && controller.state != State::Fault
                        {
                            controller.report_fault(reason).await;
                        }
                    }
                    (None, Some(bar)) => {
                        state_machine::confirmed(&mut sensor_fault_ticks, false);
                        log::info!("pressure: {bar:.2} bar, temp: {temp_c:?}, coil {}, {:?}, motor {:?}Hz {:?}A", if controller.assist_open() { "off" } else { "on" }, controller.state, controller.motor_hz(), controller.motor_amps());
                        controller.tick(bar, temp_c, dt_s).await;
                    }
                    (None, None) => unreachable!("no pressure is always a sensor fault"),
                }
            }
            Either::Second(event) => match event {
                UiEvent::RunState => controller.toggle_run(),
                UiEvent::SetPointUp => controller.adjust_setpoint(config::SETPOINT_STEP_BAR),
                UiEvent::SetPointDown => controller.adjust_setpoint(-config::SETPOINT_STEP_BAR),
                UiEvent::Unload(held) => controller.set_unload_hold(held),
                UiEvent::Purge(held) => controller.set_drain_hold(held),
            },
        }

        controller.service(Instant::now());
        FAULTED.store(controller.fault.is_some(), Ordering::Relaxed);
        nextion::STATE.signal(UiState {
            pressure_bar,
            temp_c,
            setpoint_bar: controller.setpoint_bar(),
            restart_below_bar: controller.load_threshold_bar(),
            unload_above_bar: controller.unload_threshold_bar(),
            motor_hz: controller.motor_hz(),
            motor_amps: controller.motor_amps(),
            vfd_comms_ok: controller.vfd_comms_ok(),
            state: controller.state,
            fault: controller.fault,
            enabled: controller.enabled(),
            clamp: controller.speed_clamp(),
            assist_open: controller.assist_open(),
            unload_held: controller.unload_held(),
            drain_open: controller.drain_open(),
            idle_stop_in_s: controller.idle_stop_in_s(),
        });
    }
}
