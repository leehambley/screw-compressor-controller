//! ST7701S register names.
//!
//! The ST7701S datasheet is under NDA, so the names and the meaning of each
//! field come from what is public:
//! - the Linux kernel driver `drivers/gpu/drm/panel/panel-sitronix-st7701.c`
//!   (names, bit fields and the voltage formulas used in `panel.rs`);
//! - Waveshare's board package for this panel (the values, `bsp/esp32_s3_touch_lcd_4`
//!   in waveshareteam/Waveshare-ESP32-components);
//! - the MIPI DCS standard for the "command 1" set.
//!
//! The chip has a standard command set ("command 1") and two banks of
//! manufacturer registers ("command 2", banks 0 and 1) that share the same
//! command codes. `CND2BKXSEL` switches between them; see `bank_select`.

// ---- Command 1: MIPI DCS standard -----------------------------------------

/// Leave sleep mode. The panel needs 120 ms before the next command.
pub const SLPOUT: u8 = 0x11;
/// Display inversion on. This IPS panel needs it for correct colours.
pub const INVON: u8 = 0x21;
/// Display on.
pub const DISPON: u8 = 0x29;
/// Memory access control: scan direction and RGB/BGR order. 0 = normal, RGB.
pub const MADCTL: u8 = 0x36;
/// Interface pixel format. Bits 6:4 = RGB interface, bits 2:0 = MCU interface;
/// 5 = 16 bit/pixel, 6 = 18 bit/pixel, 7 = 24 bit/pixel.
pub const COLMOD: u8 = 0x3A;

// ---- Register bank select -------------------------------------------------

/// Select command 1 or one of the command 2 banks. Takes five parameters,
/// the last of which is `CMD1` or `cmd2(bank)`.
pub const CND2BKXSEL: u8 = 0xFF;
/// Fixed first four parameters of `CND2BKXSEL`.
pub const CND2BKXSEL_KEY: [u8; 4] = [0x77, 0x01, 0x00, 0x00];
/// Fifth parameter: back to the standard command set.
pub const CMD1: u8 = 0x00;
/// Fifth parameter: command 2, bank `n` (0 or 1 here).
pub const fn cmd2(bank: u8) -> u8 {
    0x10 | bank
}

// ---- Command 2, bank 0: display geometry and gamma -------------------------

/// Positive voltage gamma, 16 parameters (see `panel::PV_GAMMA`).
pub const BK0_PVGAMCTRL: u8 = 0xB0;
/// Negative voltage gamma, 16 parameters (see `panel::NV_GAMMA`).
pub const BK0_NVGAMCTRL: u8 = 0xB1;
/// Display line setting: number of lines, in multiples of 8.
pub const BK0_LNESET: u8 = 0xC0;
/// Porch control: vertical back porch, vertical front porch.
pub const BK0_PORCTRL: u8 = 0xC1;
/// Inversion selection and frame rate: `NLINV` and `RTNI` (pixel clocks per line).
pub const BK0_INVSEL: u8 = 0xC2;
/// Not named in the sources above. Waveshare sends 0x08.
pub const BK0_CD: u8 = 0xCD;

// ---- Command 2, bank 1: panel voltages and timing ---------------------------

/// Vop (source driver amplitude): 3.5375 V + value * 12.5 mV.
pub const BK1_VRHS: u8 = 0xB0;
/// VCOM amplitude: 0.1 V + value * 12.5 mV.
pub const BK1_VCOM: u8 = 0xB1;
/// VGH (gate on voltage): 11.5 V + value[3:0] * 0.5 V.
pub const BK1_VGHSS: u8 = 0xB2;
/// Test command. Bit 7 is set.
pub const BK1_TESTCMD: u8 = 0xB3;
/// VGL (gate off voltage), non-linear: see `panel::vgls`.
pub const BK1_VGLS: u8 = 0xB5;
/// Power control 1: operational amplifier bias currents.
pub const BK1_PWCTLR1: u8 = 0xB7;
/// Power control 2: AVDD in bits 5:4, AVCL in bits 1:0.
pub const BK1_PWCTLR2: u8 = 0xB8;
/// Source pre-drive timing 1 (T2D).
pub const BK1_SPD1: u8 = 0xC1;
/// Source EQ2 setting (T3D).
pub const BK1_SPD2: u8 = 0xC2;

/// Gate driver ("GIP") timing, bank 1, E0 to ED. Fully vendor specific: the
/// Linux driver says "ST7701_SPEC_V1.2 is unable to provide enough information"
/// and copies it from the panel maker's code, as we do. Copied, not understood.
pub const BK1_GIP_E0: u8 = 0xE0;
pub const BK1_GIP_E1: u8 = 0xE1;
pub const BK1_GIP_E2: u8 = 0xE2;
pub const BK1_GIP_E3: u8 = 0xE3;
pub const BK1_GIP_E4: u8 = 0xE4;
pub const BK1_GIP_E5: u8 = 0xE5;
pub const BK1_GIP_E6: u8 = 0xE6;
pub const BK1_GIP_E7: u8 = 0xE7;
pub const BK1_GIP_E8: u8 = 0xE8;
pub const BK1_GIP_EB: u8 = 0xEB;
pub const BK1_GIP_EC: u8 = 0xEC;
pub const BK1_GIP_ED: u8 = 0xED;
