//! 480x480 RGB panel (ST7701S, set up over SPI by `st7701`) on the S3's
//! LCD_CAM in DPI (RGB) mode, scanning out an RGB565 framebuffer in PSRAM.
//!
//! The LCD's FIFO holds only a few bytes, so reading the framebuffer straight
//! from PSRAM with DMA means any bus stall (a cache write-back after a render,
//! say) starves the panel and shifts the picture. So, like ESP-IDF's "bounce
//! buffer" mode (what Waveshare's demos use), the DMA reads two small buffers in
//! internal RAM, each `BB_LINES` lines tall, and an interrupt at the end of each
//! one refills it from the framebuffer. The CPU reads the framebuffer through
//! the data cache, so renders need no cache write-back and a stall of up to
//! about one buffer time (`BB_LINES` lines, ~0.7 ms) is harmless.
//!
//! A vsync interrupt checks that the refill sequence is still in step with the
//! frame and restarts it if not (a missed refill glitches one frame, no more).
//!
//! Pins and porches are Waveshare's BSP v3.0.0 (h: sync 10, back 10, front 20;
//! v: sync 10, back 10, front 10; syncs idle high). Verified on the Rev 4.0
//! board 2026-11-17 with the PSRAM at 80 MHz (see `main.rs`).

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use esp_hal::dma::{BurstConfig, DmaDescriptor, DmaTxBuffer, Owner, Preparation, TransferDirection};
use esp_hal::gpio::Level;
use esp_hal::lcd_cam::LcdCam;
use esp_hal::lcd_cam::lcd::dpi::{Config, Dpi, DpiTransfer, Format, FrameTiming};
use esp_hal::lcd_cam::lcd::{ClockMode, Phase, Polarity};
use esp_hal::peripherals::{
    DMA_CH0, GPIO10, GPIO11, GPIO12, GPIO13, GPIO14, GPIO17, GPIO18, GPIO21, GPIO3, GPIO38,
    GPIO39, GPIO40, GPIO41, GPIO45, GPIO46, GPIO47, GPIO48, GPIO5, GPIO8, GPIO9, LCD_CAM,
};
use esp_hal::interrupt::Priority;
use esp_hal::peripherals::Interrupt;
use esp_hal::time::Rate;
use esp_hal::Blocking;
use esp_hal::handler;

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

/// Bounce buffers: two, `BB_LINES` lines each, in internal RAM. A descriptor holds
/// at most 4095 bytes, so each buffer is `DESC_PER_BUF` descriptors of 4 lines.
const BB_LINES: usize = 16;
const BB_BYTES: usize = BB_LINES * WIDTH * 2;
const BB_WORDS: usize = BB_BYTES / 4;
const DESC_BYTES: usize = 4 * WIDTH * 2;
const DESC_PER_BUF: usize = BB_BYTES / DESC_BYTES;
/// Chunks per frame. Must be even so buffer 0 always holds the even chunks.
const CHUNKS: usize = HEIGHT / BB_LINES;
const _: () = assert!(HEIGHT % BB_LINES == 0 && CHUNKS % 2 == 0);
const _: () = assert!(BB_BYTES % DESC_BYTES == 0 && DESC_BYTES <= 4095);

#[repr(align(64))]
struct Bounce([[u8; BB_BYTES]; 2]);

static mut BOUNCE: Bounce = Bounce([[0; BB_BYTES]; 2]);
/// Descriptors must live in internal RAM too. A0..A3, B0..B3, looping.
static mut RING: [DmaDescriptor; 2 * DESC_PER_BUF] = [DmaDescriptor::EMPTY; 2 * DESC_PER_BUF];

/// Framebuffer address (for the interrupt handlers).
static FB: AtomicUsize = AtomicUsize::new(0);
/// Which bounce buffer finishes next, and which chunk of the frame goes into it.
static NEXT_BUF: AtomicUsize = AtomicUsize::new(0);
static NEXT_FILL: AtomicUsize = AtomicUsize::new(2);

/// Copy chunk `chunk` of the framebuffer into bounce buffer `buf`. Runs in the
/// interrupt handlers, so: inlined (no calls into flash) and a plain volatile
/// loop (the compiler must not turn it into a `memcpy` call).
#[inline(always)]
fn fill(buf: usize, chunk: usize) {
    let src = unsafe { (FB.load(Relaxed) as *const u32).add(chunk * BB_WORDS) };
    let dst = unsafe { (addr_of_mut!(BOUNCE) as *mut u8).add(buf * BB_BYTES) } as *mut u32;
    let mut i = 0;
    while i < BB_WORDS {
        unsafe { write_volatile(dst.add(i), read_volatile(src.add(i))) };
        i += 1;
    }
}

/// Start state: buffers hold chunks 0 and 1, the next to finish is buffer 0 and
/// it gets chunk 2.
#[inline(always)]
fn reset_fill_state() {
    fill(0, 0);
    fill(1, 1);
    NEXT_BUF.store(0, Relaxed);
    NEXT_FILL.store(2, Relaxed);
}

/// The bounce ring as a looping DMA source.
struct FrameRing;

unsafe impl DmaTxBuffer for FrameRing {
    type View = FrameRing;
    type Final = FrameRing;

    fn prepare(&mut self) -> Preparation {
        let ring = unsafe { &mut *addr_of_mut!(RING) };
        let base = ring.as_mut_ptr();
        let bounce = addr_of_mut!(BOUNCE) as *mut u8;
        for (i, d) in ring.iter_mut().enumerate() {
            d.buffer = unsafe { bounce.add(i * DESC_BYTES) };
            d.set_size(DESC_BYTES);
            d.set_length(DESC_BYTES);
            d.set_owner(Owner::Dma);
            // EOF at the end of each bounce buffer: that's the refill interrupt.
            d.set_suc_eof(i % DESC_PER_BUF == DESC_PER_BUF - 1);
            d.next = unsafe { base.add((i + 1) % (2 * DESC_PER_BUF)) };
        }
        Preparation {
            start: base,
            direction: TransferDirection::Out,
            accesses_psram: false,
            burst_transfer: BurstConfig::DEFAULT,
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
/// FIFOs, start the DMA at the ring head, give it time to fill the FIFO, then
/// start the LCD. Redo the start that way once `send` has configured it all.
unsafe fn restart_in_idf_order() {
    let lcd = esp_hal::peripherals::LCD_CAM::regs();
    let ch = esp_hal::peripherals::DMA::regs().ch(0);
    let head = addr_of_mut!(RING) as u32;

    lcd.lcd_user().modify(|_, w| w.lcd_start().clear_bit());
    ch.out_link().modify(|_, w| w.outlink_stop().set_bit());
    ch.out_conf0().modify(|_, w| w.out_rst().set_bit());
    ch.out_conf0().modify(|_, w| w.out_rst().clear_bit());
    lcd.lcd_user().modify(|_, w| w.lcd_reset().set_bit());
    lcd.lcd_misc().modify(|_, w| w.lcd_afifo_reset().set_bit());

    reset_fill_state();
    ch.out_int().clr().write(|w| w.out_eof().clear_bit_by_one());
    ch.out_int().ena().modify(|_, w| w.out_eof().set_bit());

    ch.out_link().modify(|_, w| unsafe { w.outlink_addr().bits(head & 0xf_ffff) });
    ch.out_link().modify(|_, w| w.outlink_stop().clear_bit().outlink_start().set_bit());
    // IDF waits 1 us, but from cold memory the first burst can take longer, and
    // an LCD that starts before data is in the FIFO clocks out junk pixels first
    // (a different picture offset each boot). The FIFO just fills and stalls, so
    // waiting is free.
    esp_hal::delay::Delay::new().delay_micros(2000);
    lcd.lcd_user().modify(|_, w| w.lcd_update().set_bit().lcd_start().set_bit());
}

/// A bounce buffer has been fully read by the DMA: refill it with the chunk two
/// ahead of the one that just finished (the other buffer is being read now).
#[handler(priority = Priority::Priority3)]
fn dma_eof() {
    let ch = esp_hal::peripherals::DMA::regs().ch(0);
    if ch.out_int().st().read().out_eof().bit_is_set() {
        ch.out_int().clr().write(|w| w.out_eof().clear_bit_by_one());
        let buf = NEXT_BUF.load(Relaxed);
        let chunk = NEXT_FILL.load(Relaxed);
        fill(buf, chunk);
        NEXT_BUF.store(buf ^ 1, Relaxed);
        NEXT_FILL.store(if chunk + 1 == CHUNKS { 0 } else { chunk + 1 }, Relaxed);
    }
}

/// End of the vsync pulse, ~0.4 ms before the first active line. A whole number
/// of chunks was sent since the last one, so the refill state must be back at its
/// start; if not (a refill was missed), restart the stream from the top.
#[handler(priority = Priority::Priority3)]
fn lcd_vsync() {
    let lcd = esp_hal::peripherals::LCD_CAM::regs();
    if lcd.lc_dma_int_st().read().lcd_vsync_int_st().bit_is_set() {
        lcd.lc_dma_int_clr().write(|w| w.lcd_vsync_int_clr().set_bit());
        if NEXT_BUF.load(Relaxed) != 0 || NEXT_FILL.load(Relaxed) != 2 {
            let ch = esp_hal::peripherals::DMA::regs().ch(0);
            let head = addr_of_mut!(RING) as u32;
            ch.out_link().modify(|_, w| w.outlink_stop().set_bit());
            ch.out_conf0().modify(|_, w| w.out_rst().set_bit());
            ch.out_conf0().modify(|_, w| w.out_rst().clear_bit());
            lcd.lcd_misc().modify(|_, w| w.lcd_afifo_reset().set_bit());
            ch.out_int().clr().write(|w| w.out_eof().clear_bit_by_one());
            reset_fill_state();
            ch.out_link().modify(|_, w| unsafe { w.outlink_addr().bits(head & 0xf_ffff) });
            ch.out_link().modify(|_, w| w.outlink_stop().clear_bit().outlink_start().set_bit());
        }
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
        FB.store(fb as usize, Relaxed);

        let mut lcd_cam = LcdCam::new(lcd_cam);
        lcd_cam.set_interrupt_handler(lcd_vsync);
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

        let transfer = match dpi.send(true, FrameRing) {
            Ok(t) => t,
            Err((e, _, _)) => panic!("DPI start: {e:?}"),
        };
        unsafe {
            esp_hal::interrupt::bind_interrupt(Interrupt::DMA_OUT_CH0, dma_eof.handler());
        }
        esp_hal::interrupt::enable(Interrupt::DMA_OUT_CH0, dma_eof.priority()).unwrap();
        unsafe { restart_in_idf_order() };
        esp_hal::peripherals::LCD_CAM::regs()
            .lc_dma_int_clr()
            .write(|w| w.lcd_vsync_int_clr().set_bit());
        esp_hal::peripherals::LCD_CAM::regs()
            .lc_dma_int_ena()
            .modify(|_, w| w.lcd_vsync_int_ena().set_bit());
        Self {
            fb: fb.cast(),
            _transfer: transfer,
        }
    }

    pub fn pixels(&mut self) -> &mut [u16] {
        unsafe { core::slice::from_raw_parts_mut(self.fb, WIDTH * HEIGHT) }
    }

    /// Nothing to do: the refill interrupt reads the framebuffer through the same
    /// data cache the renderer writes through. Kept so callers don't change.
    pub fn flush_rows(&self, _y0: usize, _y1: usize) {}
}
