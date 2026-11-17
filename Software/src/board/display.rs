//! 480x480 RGB panel (ST7701S, set up over SPI by `st7701`) on the S3's
//! LCD_CAM in DPI (RGB) mode, scanning out a single RGB565 framebuffer in
//! PSRAM forever.
//!
//! The DMA walks a circular descriptor chain over the framebuffer and the LCD
//! has `next_frame_en` set, so after `start` the panel refreshes with no CPU
//! involvement. Drawing goes through the data cache, so call `flush_rows`
//! after each render to write the cache back to PSRAM, where the DMA reads it.
//!
//! Pins and porches are Waveshare's BSP v3.0.0 (h: sync 10, back 10, front 20;
//! v: sync 10, back 10, front 10; syncs idle high). Verified on the Rev 4.0
//! board 2026-11-17 with the PSRAM at 80 MHz (see `main.rs`). The pixel clock is
//! limited by PSRAM bandwidth: 8 and 12 MHz are stable, 14 and 16 MHz make the
//! DMA fall behind and the picture scroll left (and slowly up). Check by
//! sampling the DMA ring position at each vsync: it must stay constant.

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
// Waveshare's BSP (ST7701_480_480_PANEL_60HZ_RGB_TIMING): 16 MHz, h 10/10/20, v 10/10/10.
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

/// Even with the ESP-IDF start order (`restart_in_idf_order`) the picture comes
/// out shifted right by a fixed amount, the left strip showing the end of the
/// previous row: the frame starts that many pixels before the ring start. Cancel
/// it by entering the ring that many pixels into its first chunk (once; after
/// that every lap is exactly one frame).
///
/// Measured on the Rev 4.0 board 2026-11-17: 128 px, within ~10 px by eye (a
/// +80 lead-in doubled the shift, 80 skipped left ~40 px too little). 128 px is
/// 256 bytes, four 64-byte bursts, so most likely the DMA/LCD FIFO depth rather
/// than a coincidence. Keep it a multiple of 32 px so the entry address stays
/// 64-byte aligned. More skip moves the content left.
const START_SKIP_PX: usize = 128;
const START_SKIP_BYTES: usize = START_SKIP_PX * 2;
const _: () = assert!(START_SKIP_BYTES < CHUNK);
const MAX_RING: usize = DESCRIPTORS + 1;

/// Descriptors must live in internal RAM.
static mut DESCRIPTOR_RING: [DmaDescriptor; MAX_RING] = [DmaDescriptor::EMPTY; MAX_RING];

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
        // ring[0..DESCRIPTORS] loop over the frame; ring[DESCRIPTORS] is the
        // entry point (rest of chunk 0), run once, then on to chunk 1.
        for i in 0..DESCRIPTORS {
            let d = &mut ring[i];
            d.buffer = unsafe { self.fb.add(i * CHUNK) };
            d.set_size(CHUNK);
            d.set_length(CHUNK);
            d.set_owner(Owner::Dma);
            d.set_suc_eof(i == DESCRIPTORS - 1);
            d.next = unsafe { base.add((i + 1) % DESCRIPTORS) };
        }
        let start = if START_SKIP_BYTES > 0 {
            let d = &mut ring[DESCRIPTORS];
            d.buffer = unsafe { self.fb.add(START_SKIP_BYTES) };
            d.set_size(CHUNK - START_SKIP_BYTES);
            d.set_length(CHUNK - START_SKIP_BYTES);
            d.set_owner(Owner::Dma);
            d.set_suc_eof(false);
            d.next = unsafe { base.add(1) };
            unsafe { base.add(DESCRIPTORS) }
        } else {
            base
        };
        Preparation {
            start,
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

/// esp-hal's `Dpi::send` starts the DMA and only then resets the LCD FIFO, which
/// throws away whatever the DMA already prefetched, so the picture ends up
/// shifted by an arbitrary amount. ESP-IDF's `lcd_rgb_panel_start_transmission`
/// (what Waveshare's demos run) does it in the other order: reset DMA and LCD
/// FIFOs, start the DMA at the ring head, give it ~1 us to fill the FIFO, then
/// start the LCD. Redo the start that way once `send` has configured it all.
unsafe fn restart_in_idf_order() {
    let lcd = esp_hal::peripherals::LCD_CAM::regs();
    let ch = esp_hal::peripherals::DMA::regs().ch(0);
    let ring = addr_of_mut!(DESCRIPTOR_RING) as *mut DmaDescriptor;
    let head = (if START_SKIP_BYTES > 0 { unsafe { ring.add(DESCRIPTORS) } } else { ring }) as u32;

    lcd.lcd_user().modify(|_, w| w.lcd_start().clear_bit());
    ch.out_link().modify(|_, w| w.outlink_stop().set_bit());
    ch.out_conf0().modify(|_, w| w.out_rst().set_bit());
    ch.out_conf0().modify(|_, w| w.out_rst().clear_bit());
    lcd.lcd_user().modify(|_, w| w.lcd_reset().set_bit());
    lcd.lcd_misc().modify(|_, w| w.lcd_afifo_reset().set_bit());

    ch.out_link().modify(|_, w| unsafe { w.outlink_addr().bits(head & 0xf_ffff) });
    ch.out_link().modify(|_, w| w.outlink_stop().clear_bit().outlink_start().set_bit());
    esp_hal::delay::Delay::new().delay_micros(2);
    lcd.lcd_user().modify(|_, w| w.lcd_update().set_bit().lcd_start().set_bit());
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
        unsafe { restart_in_idf_order() };
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
