//! Init sequence for the Waveshare ESP32-S3-Touch-LCD-4 (Rev 4.0) panel:
//! 480x480 ST7701S, RGB interface, 18 bit/pixel.
//!
//! The values are Waveshare's (their `esp32_s3_touch_lcd_4.c` init table).
//! Where a register is understood it is written here in physical units (volts,
//! microseconds, lines) through the `const fn`s below, whose formulas come from
//! the Linux driver `panel-sitronix-st7701.c`; where it is not (gamma curves,
//! gate driver timing) the raw bytes are kept, grouped and labelled. A host
//! test, run when this module was written, checked that the bytes sent are
//! identical to the flat table this replaced.

use super::regs::*;

/// One command, its parameters, and a settling delay afterwards.
pub struct Step {
    pub cmd: u8,
    pub params: &'static [u8],
    pub delay_ms: u32,
}

const fn step(cmd: u8, params: &'static [u8]) -> Step {
    Step { cmd, params, delay_ms: 0 }
}

const fn step_then_wait(cmd: u8, params: &'static [u8], delay_ms: u32) -> Step {
    Step { cmd, params, delay_ms }
}

/// The five parameters of `CND2BKXSEL` that switch register banks.
const BANK_CMD1: &[u8] = &[CND2BKXSEL_KEY[0], CND2BKXSEL_KEY[1], CND2BKXSEL_KEY[2], CND2BKXSEL_KEY[3], CMD1];
const BANK_0: &[u8] = &[CND2BKXSEL_KEY[0], CND2BKXSEL_KEY[1], CND2BKXSEL_KEY[2], CND2BKXSEL_KEY[3], cmd2(0)];
const BANK_1: &[u8] = &[CND2BKXSEL_KEY[0], CND2BKXSEL_KEY[1], CND2BKXSEL_KEY[2], CND2BKXSEL_KEY[3], cmd2(1)];

// ---- Physical units to register values (formulas from the Linux driver) -----

/// Number of lines the panel drives, in multiples of 8: `(value + 1) * 8`.
const fn lines_by_8(lines: u16) -> u8 {
    (lines / 8 - 1) as u8
}

/// Vop = 3.5375 V + value * 12.5 mV.
const fn vop_uv(uv: u32) -> u8 {
    ((uv - 3_537_500 + 6_250) / 12_500) as u8
}

/// VCOM = 0.1 V + value * 12.5 mV.
const fn vcom_uv(uv: u32) -> u8 {
    ((uv - 100_000 + 6_250) / 12_500) as u8
}

/// VGH = 11.5 V + value * 0.5 V (4 bit field).
const fn vgh_mv(mv: u32) -> u8 {
    ((mv - 11_500 + 250) / 500) as u8
}

/// VGL is not linear: the 16 allowed values, in mV, indexed by register value.
const VGL_MV: [i32; 16] = [
    -7060, -7470, -7910, -8140, -8650, -8920, -9210, -9510, -9830, -10170, -10530, -10910, -11310,
    -11730, -12200, -12690,
];

const fn vgl_mv(mv: i32) -> u8 {
    let mut i = 0;
    while i < 16 {
        if VGL_MV[i] == mv {
            return i as u8;
        }
        i += 1;
    }
    panic!("not a valid VGL voltage")
}

/// AVDD = 6.2 V + value * 0.2 V (2 bit field).
const fn avdd_mv(mv: i32) -> u8 {
    ((mv - 6_200 + 100) / 200) as u8
}

/// AVCL = -4.4 V - value * 0.2 V (2 bit field).
const fn avcl_mv(mv: i32) -> u8 {
    ((-4_400 - mv + 100) / 200) as u8
}

/// T2D = 0.2 us * value (4 bit field).
const fn t2d_ns(ns: u32) -> u8 {
    ((ns + 100) / 200) as u8
}

/// T3D = 4 us + 0.8 us * value (4 bit field).
const fn t3d_ns(ns: u32) -> u8 {
    ((ns - 4_000 + 400) / 800) as u8
}

/// Pixel clocks per line = 512 + value * 16.
const fn clocks_per_line(clocks: u32) -> u8 {
    ((clocks - 512) / 16) as u8
}

// ---- Vendor data kept as raw bytes -------------------------------------------

/// Gamma curves, one byte per control point. Order, as in the Linux driver:
/// VC0, VC4, VC8, VC16, VC24, VC52, VC80, VC108, VC147, VC175, VC203, VC231,
/// VC239, VC247, VC251, VC255 (the number is the grey level the point is for).
/// The top two bits of the 0xAA / 0xA9 entries are the AJ adjustment field.
const PV_GAMMA: [u8; 16] =
    [0x00, 0x11, 0x18, 0x0E, 0x11, 0x06, 0x07, 0x08, 0x07, 0x22, 0x04, 0x12, 0x0F, 0xAA, 0x31, 0x18];
const NV_GAMMA: [u8; 16] =
    [0x00, 0x11, 0x19, 0x0E, 0x12, 0x07, 0x08, 0x08, 0x08, 0x22, 0x04, 0x11, 0x11, 0xA9, 0x32, 0x18];

// ---- The sequence ------------------------------------------------------------

/// Everything to send after the panel's hardware reset, in order.
pub const WAVESHARE_4_480X480: &[Step] = &[
    step_then_wait(SLPOUT, &[], 120),
    // Bank 0: geometry and gamma.
    step(CND2BKXSEL, BANK_0),
    // 480 lines, no half-step adjustment.
    step(BK0_LNESET, &[lines_by_8(480), 0x00]),
    // Vertical back porch 13, front porch 2 (lines).
    step(BK0_PORCTRL, &[13, 2]),
    // Inversion selection: byte 1 is 0x21 (NLINV = 1; the Linux driver also
    // sets bits 5:4, Waveshare does not). Byte 2: 512 + 8 * 16 = 640 clocks/line.
    step(BK0_INVSEL, &[0x21, clocks_per_line(640)]),
    step(BK0_CD, &[0x08]),
    step(BK0_PVGAMCTRL, &PV_GAMMA),
    step(BK0_NVGAMCTRL, &NV_GAMMA),
    // Bank 1: voltages and timing.
    step(CND2BKXSEL, BANK_1),
    step(BK1_VRHS, &[vop_uv(4_737_500)]),
    step(BK1_VCOM, &[vcom_uv(700_000)]),
    // VGH 15.0 V. Bit 7 is also set by Waveshare; not in the sources we have.
    step(BK1_VGHSS, &[0x80 | vgh_mv(15_000)]),
    step(BK1_TESTCMD, &[0x80]),
    // Bit 6 is always set; VGL -10.17 V.
    step(BK1_VGLS, &[0x40 | vgl_mv(-10_170)]),
    // Op-amp bias: gamma (bits 7:6) = 2, input (3:2) = 1, output (1:0) = 1.
    step(BK1_PWCTLR1, &[(2 << 6) | (1 << 2) | 1]),
    // AVDD 6.6 V, AVCL -4.6 V.
    step(BK1_PWCTLR2, &[(avdd_mv(6_600) << 4) | avcl_mv(-4_600)]),
    // Bits 6:4 always set; T2D 1.6 us.
    step(BK1_SPD1, &[0x70 | t2d_ns(1_600)]),
    // Bits 6:4 always set; T3D 10.4 us. Settle 20 ms after this one.
    step_then_wait(BK1_SPD2, &[0x70 | t3d_ns(10_400)], 20),
    // Gate driver timing: vendor data, see regs.rs.
    step(BK1_GIP_E0, &[0x00, 0x1B, 0x02]),
    step(BK1_GIP_E1, &[0x08, 0xA0, 0x00, 0x00, 0x07, 0xA0, 0x00, 0x00, 0x00, 0x44, 0x44]),
    step(BK1_GIP_E2, &[0x11, 0x11, 0x44, 0x44, 0xED, 0xA0, 0x00, 0x00, 0xEC, 0xA0, 0x00, 0x00]),
    step(BK1_GIP_E3, &[0x00, 0x00, 0x11, 0x11]),
    step(BK1_GIP_E4, &[0x44, 0x44]),
    step(BK1_GIP_E5, &[0x0A, 0xE9, 0xD8, 0xA0, 0x0C, 0xEB, 0xD8, 0xA0, 0x0E, 0xED, 0xD8, 0xA0, 0x10, 0xEF, 0xD8, 0xA0]),
    step(BK1_GIP_E6, &[0x00, 0x00, 0x11, 0x11]),
    step(BK1_GIP_E7, &[0x44, 0x44]),
    step(BK1_GIP_E8, &[0x09, 0xE8, 0xD8, 0xA0, 0x0B, 0xEA, 0xD8, 0xA0, 0x0D, 0xEC, 0xD8, 0xA0, 0x0F, 0xEE, 0xD8, 0xA0]),
    step(BK1_GIP_EB, &[0x02, 0x00, 0xE4, 0xE4, 0x88, 0x00, 0x40]),
    step(BK1_GIP_EC, &[0x3C, 0x00]),
    step(BK1_GIP_ED, &[0xAB, 0x89, 0x76, 0x54, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x20, 0x45, 0x67, 0x98, 0xBA]),
    // Back to the standard command set.
    step(CND2BKXSEL, BANK_CMD1),
    step(MADCTL, &[0x00]),
    // RGB interface and MCU interface both 18 bit/pixel. The board wires only
    // 16 data lines (R0 and B0 are tied low): the panel is fed RGB565.
    step(COLMOD, &[0x66]),
    step_then_wait(INVON, &[], 120),
    step(DISPON, &[]),
];
