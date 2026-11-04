//! GT911 capacitive touch controller: first touch point only.

use embedded_hal_async::i2c::I2c;

use super::Dev;

const REG_STATUS: u16 = 0x814E;
const REG_POINT1: u16 = 0x8150; // x lo, x hi, y lo, y hi

pub struct Gt911 {
    dev: Dev,
}

impl Gt911 {
    pub fn new(dev: Dev) -> Self {
        Self { dev }
    }

    /// `Ok(None)`: no new frame since the last read. `Ok(Some(Some((x, y))))`:
    /// touched there. `Ok(Some(None))`: not touched.
    pub async fn read(&mut self) -> Result<Option<Option<(u16, u16)>>, ()> {
        let mut status = [0u8];
        self.read_reg(REG_STATUS, &mut status).await?;
        if status[0] & 0x80 == 0 {
            return Ok(None); // no fresh frame yet
        }
        let touches = status[0] & 0x0F;
        let point = if touches > 0 {
            let mut p = [0u8; 4];
            self.read_reg(REG_POINT1, &mut p).await?;
            Some((u16::from_le_bytes([p[0], p[1]]), u16::from_le_bytes([p[2], p[3]])))
        } else {
            None
        };
        // Hand the buffer back to the controller.
        let [hi, lo] = REG_STATUS.to_be_bytes();
        self.dev.write(super::ADDR_GT911, &[hi, lo, 0]).await.map_err(|_| ())?;
        Ok(Some(point))
    }

    async fn read_reg(&mut self, reg: u16, buf: &mut [u8]) -> Result<(), ()> {
        self.dev
            .write_read(super::ADDR_GT911, &reg.to_be_bytes(), buf)
            .await
            .map_err(|_| ())
    }
}
