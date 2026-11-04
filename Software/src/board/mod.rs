//! Waveshare ESP32-S3-Touch-LCD-4 (V3.0) board support, and the I2C chips on
//! the piggyback I/O board. Pin facts: `Piggyback/pin-allocation.md`.
//!
//! Every S3 GPIO is taken by the board, so everything we add sits on the one
//! I2C bus (GPIO15 SDA / GPIO7 SCL), which reaches the piggyback through the
//! H4 header. Modbus uses the onboard MAX13487 on UART0 (GPIO43 TX / GPIO44
//! RX, auto direction); the log goes over USB-Serial-JTAG instead.

pub mod ads1115;
pub mod display;
pub mod gt911;
pub mod pca9685;
pub mod st7701;
pub mod tca9554;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use esp_hal::Async;
use esp_hal::i2c::master::I2c;

pub type Bus = Mutex<CriticalSectionRawMutex, I2c<'static, Async>>;
pub type Dev = I2cDevice<'static, CriticalSectionRawMutex, I2c<'static, Async>>;

// 7-bit addresses on the shared bus. Board: TCA9554 0x20-0x27 (found by
// probing), PCF85063 RTC 0x51, GT911 0x5D, QMI8658 IMU 0x6A/0x6B (not fitted).
pub const ADDR_GT911: u8 = 0x5D; // INT held low during reset selects 0x5D
pub const ADDR_ADS_PRESSURE: u8 = 0x48; // piggyback U1 (ADDR = GND)
pub const ADDR_ADS_NTC: u8 = 0x49; // piggyback U2 (ADDR = VDD)
pub const ADDR_PCA9685: u8 = 0x40; // piggyback U6

// TCA9554 port bits (schematic EXIO1..EXIO8 = P0..P7).
pub const EXIO_TP_RST: u8 = 1 << 0;
pub const EXIO_BL_EN: u8 = 1 << 1;
pub const EXIO_LCD_RST: u8 = 1 << 2;
pub const EXIO_SD_CS: u8 = 1 << 3;
pub const EXIO_BUZZER: u8 = 1 << 5;
/// RTC_INT and IRQ are inputs.
pub const EXIO_INPUTS: u8 = (1 << 6) | (1 << 7);
