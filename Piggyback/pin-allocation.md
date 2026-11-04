# Waveshare ESP32-S3-Touch-LCD-4 (Rev 4.0): pins and what's left for us

Sources, all fetched 2026-11-04:
- `ESP32-S3-Touch-LCD-4_V4.0` schematic (GPIO table, RS485 block, H4)
- the product page dimension drawing (marked Rev4.0)
- Waveshare's V4 demo (`waveshareteam/ESP32-S3-Touch-LCD-4`): expander registers and panel timings

Boards sold now are Rev 4.0. V3.0 boards (TCA9554 expander, GT911 INT on GPIO16, UART pins the other way round) are **not** supported by this firmware.

## Every S3 GPIO is spoken for

| GPIO | Board use | Usable by us? |
|---|---|---|
| 3, 5, 8–14, 17, 18, 21, 38–41, 45–48 | RGB panel bus (R1-5 = 46,3,8,18,17; G0-5 = 14,13,12,11,10,9; B1-5 = 5,45,48,47,21; HSYNC 38, VSYNC 39, DE 40, PCLK 41) | no |
| 1 / 2 / 42 | ST7701S 3-wire SPI: SDA / SCL / CS (also SD card MOSI/SCK on 1/2) | no |
| 4 | SD card MISO | no |
| 7 / 15 | **I2C SCL / SDA**: touch, CH32 expander, RTC, IMU | **yes, shared bus** |
| 16 | SYS_OUT (power-key / latch sense) | no: firmware leaves it alone |
| 0 / 6 | CAN RX / TX. IO0 is also the BOOT strap | no |
| 19 / 20 | native USB (USB-Serial-JTAG: flashing + log) | no |
| 35–37 | octal PSRAM | no |
| 44 / 43 | UART0 **TX → SP3485 DI / RX ← SP3485 RO** (DE driven by an RC timer on TX) | **yes, RS485 on H4** |

The schematic's GPIO table labels GPIO43 "RS485_TX / TX0". That is the net name: the net goes to the transceiver's RO pin. So the S3 receives on 43, as Waveshare's demo does.

### CH32V003 expander at 0x24 (Waveshare firmware)

Registers:
- 0x02 direction (1 = output)
- 0x03 output
- 0x04 input
- 0x05 backlight PWM (0–255)
- 0x06 battery ADC
- 0x07 RTC_INT

| Bit | EXIO | Use | Firmware |
|---|---|---|---|
| 1 | EXIO1 | TP_RST | output |
| 2 | EXIO2 | TP_INT | input (GT911 drives it) |
| 3 | EXIO3 | LCD_RST | output |
| 4 | EXIO4 | SD_CS | output, high |
| 5 | EXIO5 | SYS_EN (power latch) | output, **always high** |
| 6 | EXIO6 | BEE_EN (buzzer) | output, low |
| 7 | EXIO7 | RTC_INT | input |

TP_INT isn't driven during reset, so the firmware probes the GT911 at 0x5D and then 0x14.

## H4: 10-pin 3.5 mm pluggable screw terminal (the piggyback's only connection)

| Pin | Silk | Net |
|---|---|---|
| 1 | VIN | 7–36 V in |
| 2 | GND | |
| 3 | A | RS485 A |
| 4 | B | RS485 B |
| 5 / 6 | L / H | CANL / CANH (unused) |
| 7 | VCC | I2C_VCC (3V3 or 5V by jumper H3; **set 3V3**) |
| 8 | GND | |
| 9 / 10 | SDA / SCL | D_SDA / D_SCL (GPIO15 / GPIO7) |

- SW1 on the board switches the 120 Ω terminations: one position for RS485 (A–B), one for CAN.
  The Waveshare sits at one end of the Modbus bus, so turn the RS485 termination on. Terminate the SKI-70 end as well.
- The bus is biased by 10k pull-up and pull-down on the board.

## I2C addresses

| Device | Board | Address |
|---|---|---|
| CH32V003 expander | Waveshare | 0x24 |
| PCF85063 RTC | Waveshare | 0x51 |
| GT911 touch | Waveshare | 0x5D or 0x14 |
| QMI8658 IMU | Waveshare | 0x6A / 0x6B |
| ADS1115 (U1) | piggyback | 0x48 |
| ADS1115 (U2) | piggyback | 0x49 |
| PCA9685 | piggyback | 0x40 (all-call 0x70) |

No clashes.

## Mechanical (from the Rev 4.0 dimension drawing)

- PCB: 74.50 × 76.50 mm.
- Cover glass: 84.2 × 84.2 mm. Active area: 71.86 × 70.18 mm.
- 4 mounting holes on a 66.50 × 68.50 mm rectangle.
- H4 sits on the back edge.

## Consequence

| Function | Path |
|---|---|
| 2× 4–20 mA + 3× NTC | 2× ADS1115 (0x48, 0x49) |
| Valve PWM hold (start-assist, drain) | PCA9685 (0x40) LED0/LED1 → safety AND gates → 2× N-MOSFET |
| Watchdog heartbeat | PCA9685 LED4, toggled by firmware only while the control loop runs |
| Interlock status | SAFE and TEMP_OK read back on ADS1115 AIN3s |
| Modbus RTU to the VFD | UART0 → onboard SP3485 → H4 3/4 → piggyback J11 (not isolated) |
| Logging / flashing | USB-C (USB-Serial-JTAG) |
| Power | piggyback 24 V → H4 VIN; piggyback 3V3 from H4 VCC |
