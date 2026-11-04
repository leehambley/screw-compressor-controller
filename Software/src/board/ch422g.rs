//! CH422G I/O expander. It has no register pointer: each "register" is its own
//! 7-bit I2C address (0x23, 0x24, 0x38: the values Waveshare's demo code
//! passes straight to the I2C driver), and a one-byte write to it is the
//! whole command. So it can't
//! go through `I2cDevice` with a fixed address; it takes the bus itself.
//!
//! IO0-7 switch direction all together, so we keep them as outputs (the
//! board's resets and backlight live there) and don't read DI0/DI1. The
//! piggyback's status lines are read through its ADS1115 instead.

use super::Bus;

const CMD_SET: u8 = 0x24; // system parameters
const CMD_OC: u8 = 0x23; // OC0-3 open-drain outputs
const CMD_IO: u8 = 0x38; // IO0-7 outputs

const SET_IO_OE: u8 = 0x01; // IO0-7 are outputs

pub struct Ch422g {
    bus: &'static Bus,
    io: u8,
    oc: u8,
}

impl Ch422g {
    pub async fn new(bus: &'static Bus) -> Self {
        let me = Self { bus, io: 0xFF, oc: 0x0F };
        me.cmd(CMD_SET, SET_IO_OE).await;
        me.cmd(CMD_IO, me.io).await;
        me.cmd(CMD_OC, me.oc).await;
        me
    }

    pub async fn set_io(&mut self, mask: u8, high: bool) {
        self.io = if high { self.io | mask } else { self.io & !mask };
        self.cmd(CMD_IO, self.io).await;
    }

    /// `active` = the OC pin sinks current (opto LED on, DOUTn pulled low).
    pub async fn set_oc(&mut self, mask: u8, active: bool) {
        // OC bits are the pin level: 0 = sinking.
        self.oc = if active { self.oc & !mask } else { self.oc | mask };
        self.cmd(CMD_OC, self.oc).await;
    }

    async fn cmd(&self, addr: u8, value: u8) {
        if let Err(e) = self.bus.lock().await.write_async(addr, &[value]).await {
            log::warn!("ch422g: cmd {addr:#04x} failed: {e:?}");
        }
    }
}
