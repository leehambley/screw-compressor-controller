//! 480x480 RGB panel (ST7701S, set up over SPI by `st7701`) on the S3's
//! LCD_CAM in DPI (RGB) mode, scanning out a single RGB565 framebuffer in
//! PSRAM forever.
//!
//! The DMA walks a circular descriptor chain over the framebuffer and the LCD
//! has `next_frame_en` set, so after `start` the panel refreshes with no CPU
//! involvement. Drawing goes through the data cache, so call `flush_rows`
//! after each render to write the cache back to PSRAM, where the DMA reads it.
//!
//! Timings and pins are ESPHome's tested `WAVESHARE-4-480X480` model: 12 MHz,
//! clock not inverted, porches 20/10/10 (h) and 10/10/10 (v). UNVERIFIED with
//! this driver (board not in hand, 2026-11-04). If the picture jitters or
//! drifts, the PSRAM can't keep up: drop PCLK_MHZ first.

use core::ptr::addr_of_mut;

use esp_hal::dma::{
    BurstConfig, DmaDescriptor, DmaTxBuffer, ExternalBurstConfig, Owner, Preparation,
    TransferDirection,
};
use esp_hal::gpio::Level;
use esp_hal::lcd_cam::LcdCam;
use esp_hal::lcd_cam::lcd::dpi::{Config, Dpi, DpiTransfer, Format, FrameTiming};
use esp_hal::lcd_cam::lcd::{ClockMode, Phase, Polarity};
use esp_hal::peripherals::{
    DMA_CH0, GPIO10, GPIO11, GPIO12, GPIO13, GPIO14, GPIO17, GPIO18, GPIO21, GPIO3, GPIO38,
    GPIO39, GPIO40, GPIO41, GPIO45, GPIO46, GPIO47, GPIO48, GPIO5, GPIO8, GPIO9, LCD_CAM,
};
use esp_hal::time::Rate;
use esp_hal::Blocking;

pub const WIDTH: usize = 480;
pub const HEIGHT: usize = 480;
pub const FB_BYTES: usize = WIDTH * HEIGHT * 2;

const PCLK_MHZ: u32 = 12;
const H_SYNC: usize = 10;
const H_BACK: usize = 10;
const H_FRONT: usize = 20;
const V_SYNC: usize = 10;
const V_BACK: usize = 10;
const V_FRONT: usize = 10;

/// Bytes per DMA descriptor: under the 4095 limit, a multiple of the 64-byte
/// PSRAM burst, and an exact divisor of the frame (460800 / 3840 = 120).
const CHUNK: usize = 3840;
const DESCRIPTORS: usize = FB_BYTES / CHUNK;
const _: () = assert!(FB_BYTES % CHUNK == 0 && CHUNK % 64 == 0);

/// Descriptors must live in internal RAM.
static mut DESCRIPTOR_RING: [DmaDescriptor; DESCRIPTORS] = [DmaDescriptor::EMPTY; DESCRIPTORS];

/// The framebuffer as a looping DMA source.
struct FrameRing {
    fb: *mut u8,
}

unsafe impl DmaTxBuffer for FrameRing {
    type View = FrameRing;
    type Final = FrameRing;

    fn prepare(&mut self) -> Preparation {
        let ring = unsafe { &mut *addr_of_mut!(DESCRIPTOR_RING) };
        let base = ring.as_mut_ptr();
        for (i, d) in ring.iter_mut().enumerate() {
            d.buffer = unsafe { self.fb.add(i * CHUNK) };
            d.set_size(CHUNK);
            d.set_length(CHUNK);
            d.set_owner(Owner::Dma);
            d.set_suc_eof(i == DESCRIPTORS - 1);
            d.next = unsafe { base.add((i + 1) % DESCRIPTORS) };
        }
        Preparation {
            start: base,
            direction: TransferDirection::Out,
            accesses_psram: true,
            burst_transfer: BurstConfig {
                external_memory: ExternalBurstConfig::Size64,
                ..BurstConfig::DEFAULT
            },
            // The DMA never hands descriptors back (owner bit stays Dma), so
            // it must not stop on them the second time round.
            check_owner: Some(false),
            auto_write_back: false,
        }
    }

    fn into_view(self) -> Self::View {
        self
    }

    fn from_view(view: Self::View) -> Self::Final {
        view
    }
}

pub struct Pins {
    pub pclk: GPIO41<'static>,
    pub de: GPIO40<'static>,
    pub hsync: GPIO38<'static>,
    pub vsync: GPIO39<'static>,
    // Panel B1..B5, G0..G5, R1..R5 as wired by Waveshare.
    pub b: (GPIO5<'static>, GPIO45<'static>, GPIO48<'static>, GPIO47<'static>, GPIO21<'static>),
    pub g: (
        GPIO14<'static>,
        GPIO13<'static>,
        GPIO12<'static>,
        GPIO11<'static>,
        GPIO10<'static>,
        GPIO9<'static>,
    ),
    pub r: (GPIO46<'static>, GPIO3<'static>, GPIO8<'static>, GPIO18<'static>, GPIO17<'static>),
}

pub struct Display {
    fb: *mut u16,
    _transfer: DpiTransfer<'static, FrameRing, Blocking>,
}

impl Display {
    /// `fb` must point at `FB_BYTES` of PSRAM, 64-byte aligned, that nothing
    /// else uses. Starts the panel scanning out immediately.
    pub unsafe fn start(lcd_cam: LCD_CAM<'static>, dma: DMA_CH0<'static>, pins: Pins, fb: *mut u8) -> Self {
        unsafe { core::ptr::write_bytes(fb, 0, FB_BYTES) };
        flush_range(fb, FB_BYTES);

        let lcd_cam = LcdCam::new(lcd_cam);
        let config = Config::default()
            .with_frequency(Rate::from_mhz(PCLK_MHZ))
            // Data changes on the falling edge, panel samples on the rising one.
            .with_clock_mode(ClockMode {
                polarity: Polarity::IdleLow,
                phase: Phase::ShiftHigh, // ESPHome pclk_inverted = false
            })
            .with_format(Format {
                enable_2byte_mode: true,
                ..Default::default()
            })
            .with_timing(FrameTiming {
                horizontal_active_width: WIDTH,
                horizontal_total_width: WIDTH + H_SYNC + H_BACK + H_FRONT,
                horizontal_blank_front_porch: H_SYNC + H_BACK,
                vertical_active_height: HEIGHT,
                vertical_total_height: HEIGHT + V_SYNC + V_BACK + V_FRONT,
                vertical_blank_front_porch: V_SYNC + V_BACK,
                hsync_width: H_SYNC,
                vsync_width: V_SYNC,
                hsync_position: 0,
            })
            .with_vsync_idle_level(Level::High)
            .with_hsync_idle_level(Level::High)
            .with_de_idle_level(Level::Low)
            .with_disable_black_region(false);

        let p = pins;
        let dpi = Dpi::new(lcd_cam.lcd, dma, config)
            .expect("DPI config")
            .with_pclk(p.pclk)
            .with_de(p.de)
            .with_hsync(p.hsync)
            .with_vsync(p.vsync)
            .with_data0(p.b.0)
            .with_data1(p.b.1)
            .with_data2(p.b.2)
            .with_data3(p.b.3)
            .with_data4(p.b.4)
            .with_data5(p.g.0)
            .with_data6(p.g.1)
            .with_data7(p.g.2)
            .with_data8(p.g.3)
            .with_data9(p.g.4)
            .with_data10(p.g.5)
            .with_data11(p.r.0)
            .with_data12(p.r.1)
            .with_data13(p.r.2)
            .with_data14(p.r.3)
            .with_data15(p.r.4);

        let transfer = match dpi.send(true, FrameRing { fb }) {
            Ok(t) => t,
            Err((e, _, _)) => panic!("DPI start: {e:?}"),
        };
        Self {
            fb: fb.cast(),
            _transfer: transfer,
        }
    }

    pub fn pixels(&mut self) -> &mut [u16] {
        unsafe { core::slice::from_raw_parts_mut(self.fb, WIDTH * HEIGHT) }
    }

    /// Write rows `y0..y1` back from the cache so the DMA sees them.
    pub fn flush_rows(&self, y0: usize, y1: usize) {
        let start = unsafe { self.fb.add(y0 * WIDTH) }.cast::<u8>();
        flush_range(start, (y1.min(HEIGHT) - y0) * WIDTH * 2);
    }
}

fn flush_range(ptr: *mut u8, len: usize) {
    unsafe extern "C" {
        fn rom_Cache_WriteBack_Addr(addr: u32, size: u32);
        fn Cache_Suspend_DCache_Autoload() -> u32;
        fn Cache_Resume_DCache_Autoload(value: u32);
    }
    unsafe {
        let autoload = Cache_Suspend_DCache_Autoload();
        rom_Cache_WriteBack_Addr(ptr as u32, len as u32);
        Cache_Resume_DCache_Autoload(autoload);
    }
}
