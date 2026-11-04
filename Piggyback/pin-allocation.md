# Waveshare ESP32-S3-Touch-LCD-5 (V1.2): pins and what's left for us

Read from Waveshare's schematic `ESP32-S3-Touch-LCD-5-Sch.pdf` (V1.2), page 1
GPIO table, module U9 and terminal P1. Fetched 2026-11-04.

## Every S3 GPIO is spoken for

| GPIO | Board use | Usable by us? |
|---|---|---|
| 0, 1, 2, 3, 5, 7, 10, 14, 17, 18, 21, 38–42, 45–48 | RGB LCD bus (0, 3, 45, 46 also strapping) | no |
| 4 | GT911 touch IRQ | no |
| 6 | **PCF85063 RTC_INT** (not free, despite the briefing) | no |
| 8 / 9 | I2C SDA / SCL (touch, CH422G, RTC) | **yes, shared bus** |
| 11 / 12 / 13 | TF card MOSI / SCK / MISO | only with the card unused and wires soldered to pads |
| 15 / 16 | CAN TX / RX → TJA1051 | only by removing the TJA1051 and tapping its pads |
| 19 / 20 | native USB (USB-Serial-JTAG: flashing + log) | no |
| 35–37 | octal PSRAM | no |
| 43 / 44 | UART0 RX / TX → SP3485 (auto direction) | **yes, RS485 on P1** |

CH422G expander (I2C): EXIO0 = DI0, EXIO1 = CTP_RST, EXIO2 = DISP (backlight),
EXIO3 = LCD_RST, EXIO4 = SDCS, EXIO5 = DI1, OD0 = DO0, OD1 = DO1.

## P1, 16-pin bottom screw terminal (what the piggyback can reach)

| Pin | Net | Notes |
|---|---|---|
| 1 | VIN | 7–36 V in (diode-ORed, D3/D17) |
| 2 | GND | |
| 3 | I2C_VCC ("VOUT" on silk) | 3V3 or 5V, picked by jumper H8 (default 3V3) |
| 4 | GND | |
| 5 | D_SDA | GPIO8 via NDC7002N level shifter, 4.7k pull-ups both sides |
| 6 | D_SCL | GPIO9, same |
| 7 / 8 | CAN_L / CAN_H | TJA1051, not isolated |
| 9 / 10 | 485_B / 485_A | SP3485 on UART0, not isolated; 120R via SW2 |
| 11 / 12 | DOUT0 / DOUT1 | opto open-drain, 5–36 V, 450 mA |
| 13 | DI_COM | |
| 14 | GND | |
| 15 / 16 | DIN0 / DIN1 | opto inputs, 5–36 V |

I2C addresses already taken: GT911 0x5D (or 0x14), CH422G 0x20–0x27 and
0x30–0x3F, PCF85063 0x51. Free and used by the piggyback: ADS1115 0x48/0x49,
PCA9685 0x40 (its all-call address 0x70 is free too).

## Consequence

The piggyback talks to the S3 over **I2C only**, plus two P1 lines:

| Function | Path |
|---|---|
| 2× 4–20 mA + 3× NTC | 2× ADS1115 (0x48, 0x49) |
| Valve PWM hold (start-assist, drain) | PCA9685 (0x40) → 2× N-MOSFET; OE gated by the watchdog |
| Watchdog heartbeat | DOUT0 toggled by firmware → piggyback supervisor; missed heartbeat drops valve drive |
| Modbus RTU to the VFD | UART0 GPIO43/44 → onboard SP3485 → P1 485_A/B (or isolated, see README) |
| Logging / flashing | USB-C (USB-Serial-JTAG), so UART0 is free for Modbus |
| Power | piggyback 24 V → P1 VIN (the board makes its own 5 V / 3V3) |
