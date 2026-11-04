//! Waveshare ESP32-S3-Touch-LCD-5 (V1.2) board support, and the I2C chips on
//! the piggyback I/O board. Pin facts: `Piggyback/pin-allocation.md`.
//!
//! Every S3 GPIO is taken by the board, so everything we add sits on the one
//! I2C bus (GPIO8 SDA / GPIO9 SCL), which reaches the piggyback through the
//! P1 screw terminal. Modbus uses the onboard SP3485 on UART0 (GPIO43/44);
//! the log goes over USB-Serial-JTAG instead.

pub mod ads1115;
pub mod ch422g;
pub mod display;
pub mod gt911;
pub mod pca9685;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use esp_hal::Async;
use esp_hal::i2c::master::I2c;

pub type Bus = Mutex<CriticalSectionRawMutex, I2c<'static, Async>>;
pub type Dev = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, Async>>;

// 7-bit addresses on the shared bus.
pub const ADDR_GT911: u8 = 0x5D; // INT held low during reset selects 0x5D
pub const ADDR_ADS_PRESSURE: u8 = 0x48; // piggyback U1 (ADDR = GND)
pub const ADDR_ADS_NTC: u8 = 0x49; // piggyback U2 (ADDR = VDD)
pub const ADDR_PCA9685: u8 = 0x40; // piggyback U6

// CH422G EXIO bits (its "IO" byte).
pub const EXIO_DI0: u8 = 1 << 0;
pub const EXIO_CTP_RST: u8 = 1 << 1;
pub const EXIO_DISP: u8 = 1 << 2; // backlight enable
pub const EXIO_LCD_RST: u8 = 1 << 3;
pub const EXIO_SD_CS: u8 = 1 << 4;
pub const EXIO_DI1: u8 = 1 << 5;
// CH422G OC outputs: OC0 = DO0 = piggyback watchdog heartbeat.
pub const OC_DO0: u8 = 1 << 0;
