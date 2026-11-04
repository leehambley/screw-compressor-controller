#![no_std]
#![no_main]

extern crate alloc;

mod board;
mod config;
mod modbus;
mod outputs;
mod pid;
mod sensors;
mod state_machine;
mod storage;
mod ui;
mod valve;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Instant, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use modbus::SkiVfd;
use static_cell::StaticCell;
use state_machine::{Controller, FaultReason, State};
use storage::CounterStore;
use ui::{UiEvent, UiState};
use valve::PwmValve;

use board::ads1115::Ads1115;
use board::display::{self, Display};
use board::gt911::Gt911;
use board::pca9685::Pca9685;
use board::st7701::St7701;
use board::tca9554::Tca9554;

esp_bootloader_esp_idf::esp_app_desc!();

/// Piggyback ADC channels (see Piggyback/README.md).
mod adc {
    // U1 at 0x48
    pub const PRESSURE_1: u8 = 0;
    pub const PRESSURE_2: u8 = 1;
    pub const SUPPLY_HALF: u8 = 2; // 3V3_A / 2
    pub const SAFE_HALF: u8 = 3; // SAFE / 2
    // U2 at 0x49
    pub const NTC_ROTOR: u8 = 0;
    pub const NTC_ROTOR_2: u8 = 1;
    pub const NTC_MOTOR: u8 = 2;
    pub const TEMP_OK_HALF: u8 = 3; // TEMP_OK / 2
    /// Conversions averaged per reading.
    pub const SAMPLES: usize = 2;
}

/// One round of piggyback readings.
struct Readings {
    pressure_v: Option<f32>,
    rotor_ratio: Option<f32>,
    rotor2_ratio: Option<f32>,
    motor_ratio: Option<f32>,
    interlock_ok: Option<bool>,
    hw_temp_ok: Option<bool>,
}

async fn read_piggyback(pressure_adc: &mut Ads1115, ntc_adc: &mut Ads1115) -> Readings {
    let n = adc::SAMPLES;
    let pressure_v = pressure_adc.read_mean_v(adc::PRESSURE_1, n).await.ok();
    let supply = pressure_adc.read_mean_v(adc::SUPPLY_HALF, n).await.ok().map(|v| v * 2.0);
    let safe = pressure_adc.read_mean_v(adc::SAFE_HALF, 1).await.ok().map(|v| v * 2.0);
    let ratio = |v: Option<f32>| match (v, supply) {
        (Some(v), Some(s)) if s > 2.5 => Some(v / s),
        _ => None,
    };
    let rotor_ratio = ratio(ntc_adc.read_mean_v(adc::NTC_ROTOR, n).await.ok());
    let rotor2_ratio = ratio(ntc_adc.read_mean_v(adc::NTC_ROTOR_2, n).await.ok());
    let motor_ratio = ratio(ntc_adc.read_mean_v(adc::NTC_MOTOR, n).await.ok());
    let temp_ok = ntc_adc.read_mean_v(adc::TEMP_OK_HALF, 1).await.ok().map(|v| v * 2.0);
    // Logic levels from 3V3_D seen through a 10k/10k divider: high is ~3.3V.
    let high = |v: Option<f32>| v.map(|v| v > 1.65);
    Readings {
        pressure_v,
        rotor_ratio,
        rotor2_ratio,
        motor_ratio,
        interlock_ok: high(safe),
        hw_temp_ok: high(temp_ok),
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
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);


    // PSRAM: the first 768 KB is the framebuffer, the rest is heap (Slint).
    // Internal RAM gets a small heap of its own for anything that needs it.
    esp_alloc::heap_allocator!(size: 64 * 1024);
    let (psram, psram_len) = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);
    assert!(psram_len > display::FB_BYTES, "no PSRAM?");
    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            psram.add(display::FB_BYTES),
            psram_len - display::FB_BYTES,
            esp_alloc::MemoryCapability::External.into(),
        ));
    }

    // One I2C bus for everything: touch, expander and RTC on the Waveshare,
    // the ADCs and PWM chip on the piggyback (through H4). 100 kHz because
    // the piggyback hangs off a harness.
    let i2c = I2c::new(peripherals.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(peripherals.GPIO15)
        .with_scl(peripherals.GPIO7)
        .into_async();
    static I2C_BUS: StaticCell<board::Bus> = StaticCell::new();
    let bus: &'static board::Bus = I2C_BUS.init(embassy_sync::mutex::Mutex::new(i2c));
    let dev = || I2cDevice::new(bus);

    // Panel and touch reset through the TCA9554. GT911 picks address 0x5D
    // when its INT (GPIO16) is low as reset is released.
    let mut exio = Tca9554::new(dev(), board::EXIO_INPUTS).await.ok();
    if exio.is_none() {
        log::error!("tca9554 not found: no panel reset or backlight");
    }
    if let Some(x) = exio.as_mut() {
        x.set(board::EXIO_TP_RST | board::EXIO_LCD_RST | board::EXIO_BL_EN, false).await;
    }
    let touch_int = Output::new(peripherals.GPIO16, Level::Low, OutputConfig::default());
    Timer::after_millis(10).await;
    if let Some(x) = exio.as_mut() {
        x.set(board::EXIO_TP_RST | board::EXIO_LCD_RST, true).await;
    }
    Timer::after_millis(120).await;
    drop(touch_int); // back to an input

    St7701 {
        cs: Output::new(peripherals.GPIO42, Level::High, OutputConfig::default()),
        sck: Output::new(peripherals.GPIO2, Level::High, OutputConfig::default()),
        sda: Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default()),
    }
    .init()
    .await;

    let display = unsafe {
        Display::start(
            peripherals.LCD_CAM,
            peripherals.DMA_CH0,
            display::Pins {
                pclk: peripherals.GPIO41,
                de: peripherals.GPIO40,
                hsync: peripherals.GPIO38,
                vsync: peripherals.GPIO39,
                b: (peripherals.GPIO5, peripherals.GPIO45, peripherals.GPIO48, peripherals.GPIO47, peripherals.GPIO21),
                g: (
                    peripherals.GPIO14,
                    peripherals.GPIO13,
                    peripherals.GPIO12,
                    peripherals.GPIO11,
                    peripherals.GPIO10,
                    peripherals.GPIO9,
                ),
                r: (peripherals.GPIO46, peripherals.GPIO3, peripherals.GPIO8, peripherals.GPIO18, peripherals.GPIO17),
            },
            psram,
        )
    };
    if let Some(x) = exio.as_mut() {
        x.set(board::EXIO_BL_EN, true).await; // backlight on
    }
    spawner.spawn(ui::task(display, Gt911::new(dev()))).unwrap();

    // Piggyback. A missing piggyback is logged and shows on the status bar
    // (interlock unreadable); the controller won't run without pressure.
    let mut pressure_adc = Ads1115::new(dev(), board::ADDR_ADS_PRESSURE);
    let mut ntc_adc = Ads1115::new(dev(), board::ADDR_ADS_NTC);
    let pca = match Pca9685::new(dev(), board::ADDR_PCA9685, config::VALVE_PWM_HZ).await {
        Ok(p) => Some(p),
        Err(()) => {
            log::error!("pca9685 not found: valves can't be driven, watchdog not armed");
            None
        }
    };
    spawner.spawn(outputs::task(pca)).unwrap();
    let solenoid_unload = PwmValve::new(outputs::CH_START_ASSIST, "start-assist");
    let solenoid_drain = PwmValve::new(outputs::CH_DRAIN, "drain");

    // Modbus RTU to the SKI-70 through the onboard MAX13487, which switches
    // direction by itself; A/B leave on H4 and the piggyback passes them to a
    // terminal. UART0's pins, so the log is on USB-Serial-JTAG.
    let modbus_uart = Uart::new(
        peripherals.UART0,
        UartConfig::default().with_baudrate(config::MODBUS_BAUD_RATE),
    )
    .unwrap()
    .with_tx(peripherals.GPIO43)
    .with_rx(peripherals.GPIO44)
    .into_async();

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
    let mut interlock_ok: Option<bool> = None;
    let mut hw_temp_ok: Option<bool> = None;

    // One task, two wake-ups: the control tick every ADC_SAMPLE_MS, and HMI
    // input as soon as it arrives (so +/- and the hold buttons respond
    // immediately rather than on the next tick). Either way the display
    // gets a fresh UiState afterwards.
    loop {
        match select(ticker.next(), ui::EVENTS.receive()).await {
            Either::First(()) => {
                let r = read_piggyback(&mut pressure_adc, &mut ntc_adc).await;
                if let Some(v) = r.pressure_v {
                    log::info!("adc: pressure {v:.3}V {:.2}mA", sensors::loop_current_ma(v));
                }
                let temp = |ratio: Option<f32>| {
                    ratio.filter(|&q| !sensors::ntc_sensor_fault(q)).map(sensors::ntc_temp_c)
                };
                pressure_bar = r
                    .pressure_v
                    .filter(|&v| !sensors::pressure_sensor_fault(v))
                    .map(sensors::pressure_bar);
                temp_c = temp(r.rotor_ratio);
                log::debug!(
                    "temps: rotor {temp_c:?}, rotor2 {:?}, motor {:?}",
                    temp(r.rotor2_ratio),
                    temp(r.motor_ratio)
                );
                interlock_ok = r.interlock_ok;
                hw_temp_ok = r.hw_temp_ok;

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
        outputs::control_alive();
        let faulted = controller.fault.is_some();
        outputs::set_duty(outputs::CH_LED_OK, if faulted { 0 } else { 10 });
        outputs::set_duty(outputs::CH_LED_FAULT, if faulted { 50 } else { 0 });
        ui::STATE.signal(UiState {
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
            interlock_ok,
            hw_temp_ok,
        });
    }
}
