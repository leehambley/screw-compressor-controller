# Waveshare ESP32-S3-Touch-LCD-4 (V3.0): pins and what's left for us

Read from the drawn pages of Waveshare's `ESP32-S3-Touch-LCD-4_V3.0.pdf`
(the GPIO table, module U9, header H4), fetched 2026-11-04. Panel pins and
timings are cross-checked against ESPHome's tested `WAVESHARE-4-480X480`
mipi_rgb model. (The 5" board this file first described is now only an
alternate.)

## Every S3 GPIO is spoken for

| GPIO | Board use | Usable by us? |
|---|---|---|
| 3, 5, 8–14, 17, 18, 21, 38–41, 45–48 | RGB panel bus (R1-5 = 46,3,8,18,17; G0-5 = 14,13,12,11,10,9; B1-5 = 5,45,48,47,21; HSYNC 38, VSYNC 39, DE 40, PCLK 41) | no |
| 1 / 2 / 42 | ST7701S 3-wire SPI: SDA / SCL / CS (also SD card MOSI/SCK on 1/2) | no |
| 4 | SD card MISO | no |
| 7 / 15 | **I2C SCL / SDA**: touch, TCA9554, RTC, IMU footprint | **yes, shared bus** |
| 16 | GT911 touch INT | no |
| 0 / 6 | CAN RX / TX (TJA1051). IO0 is also the BOOT strap | no (only by removing the TJA1051) |
| 19 / 20 | native USB (USB-Serial-JTAG: flashing + log) | no |
| 35–37 | octal PSRAM | no |
| 43 / 44 | UART0 TX / RX → MAX13487 (auto direction) | **yes, RS485 on H4** |

TCA9554 expander: EXIO1 = TP_RST, EXIO2 = BL_EN, EXIO3 = LCD_RST,
EXIO4 = SDCS, EXIO5 free, EXIO6 = buzzer, EXIO7 = RTC_INT, EXIO8 = IRQ.

## H4, 10-pin expansion header (the piggyback's only connection)

| Pin | Net |
|---|---|
| 1 | VIN (5–36 V, SMAJ40CA TVS) |
| 2 | GND |
| 3 | RS485 A ("TX+") |
| 4 | RS485 B ("TX−") |
| 5 / 6 | CANL / CANH (unused) |
| 7 | I2C_VCC (3V3 or 5V by jumper H3; default 3V3) |
| 8 | GND |
| 9 / 10 | D_SDA / D_SCL (GPIO15 / GPIO7 through the NDC7002N level shifter) |

The supplement's claim that IO15, IO16, TX0 and RX0 are on H4 is wrong for V3.0.
None of them is. **H4 pitch is not on the schematic: confirm it on the board
before ordering J2.**

I2C addresses taken: TCA9554 0x20–0x27 (firmware probes for it), PCF85063
0x51, GT911 0x5D, QMI8658 0x6A/0x6B (marked NC). Piggyback: ADS1115
0x48/0x49 and PCA9685 0x40 (all-call 0x70). No clashes.

## Consequence

| Function | Path |
|---|---|
| 2× 4–20 mA + 3× NTC | 2× ADS1115 (0x48, 0x49) |
| Valve PWM hold (start-assist, drain) | PCA9685 (0x40) LED0/LED1 → safety AND gates → 2× N-MOSFET |
| Watchdog heartbeat | PCA9685 LED4, toggled by firmware only while the control loop runs |
| Interlock status | SAFE and TEMP_OK read back on ADS1115 AIN3s |
| Modbus RTU to the VFD | UART0 → onboard MAX13487 → H4 3/4 → piggyback J11 (not isolated) |
| Logging / flashing | USB-C (USB-Serial-JTAG) |
| Power | piggyback 24 V → H4 VIN; piggyback 3V3 from H4 I2C_VCC |
