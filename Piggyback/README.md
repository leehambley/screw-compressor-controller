# Piggyback I/O board for the Waveshare ESP32-S3-Touch-LCD-4

An SMD board, assembled by JLCPCB, that carries everything the Waveshare
display board can't do itself. It connects to the board's 10-pin H4 header
only: power, I2C and the RS485 pair. Every S3 GPIO is taken (see
[pin-allocation.md](pin-allocation.md)).

**2026-11-04 evening: the display board changed from the 5" to the 4" (480x480).**
The changes that followed:
- J2 is now a 10-pin H4 harness.
- The heartbeat comes from PCA9685 LED4, not the Waveshare's DO0.
- The status lines back to the S3 (Q5/Q6) are gone. The S3 reads SAFE and TEMP_OK through the ADS1115s.
- New J11 passes Modbus A/B through to the VFD.

Lee confirmed the onboard RS485 transceiver (no isolation) and no digital inputs.

**Rev 4.0 (current production, checked 2026-11-04):**
- The transceiver is an SP3485 with RC auto-direction. It is not a MAX13487.
- UART0 is TX GPIO44 / RX GPIO43.
- The expander is a CH32V003 at 0x24.
- H4 is a **3.5 mm 10-pin pluggable screw terminal**, so J2 must mate with that.

Details are in [pin-allocation.md](pin-allocation.md).
The blocks below still describe the original 8-pin J2. The schematic is authoritative.

## Decisions (Lee, 2026-11-04)

- The S3 is the only MCU and runs all-Rust Embassy (Xtensa). The C6 and the Nextion are retired.
- Modbus to the SKI-70 goes through the Waveshare's onboard RS485 transceiver (UART0, TX 44 / RX 43, auto direction, not isolated).
  The piggyback has no RS485 parts.
- The valves are driven by MOSFETs on the piggyback, with the PWM hold kept.
- Fail-safe: a firmware watchdog **and** a hardware over-temperature cutout.
- NTC pull-up is 33k. The pressure shunt is 100 Ω, 0–17 bar, as in `Software/src/config.rs`.
- No TPS5430 and no NMA0505/ADM2483. The Waveshare runs from 24 V on its own VIN.

## Blocks

### Power
- J1: 24 V in, 2-pin pluggable terminal, 5.08 mm.
- Protection chain: polyfuse (≥ 30 V rated, ~1.5 A hold), then a series Schottky SS54 for reverse polarity, then an SMBJ30A TVS.
  The result is **24V_P**.
- 24V_P feeds the Waveshare VIN (via J2), both valves and both 4–20 mA loops.
- 3V3 comes from the Waveshare's own 3V3 rail: P1 pin 3 "I2C_VCC", with jumper H8 set to 3V3.
  The piggyback load is about 30 mA.
  - **3V3_D** = logic.
  - **3V3_A** = 3V3_D through a 600 Ω ferrite bead plus 10 µF and 100 nF. It feeds the ADS1115s, the NTC dividers and the comparator references.
- If the Waveshare 3V3 dies, all piggyback outputs drop. That is fail-safe by construction.

### J2: harness to Waveshare P1 (8-pin pluggable, 3.5 mm)

| J2 | Net | Waveshare P1 |
|---|---|---|
| 1 | 24V_P | 1 VIN |
| 2 | GND | 2 GND |
| 3 | 3V3_D | 3 I2C_VCC |
| 4 | SDA | 5 D_SDA |
| 5 | SCL | 6 D_SCL |
| 6 | HEARTBEAT | 11 DOUT0 |
| 7 | DIN_TEMP | 15 DIN0 |
| 8 | DIN_WD | 16 DIN1 |

- The Waveshare already pulls SDA and SCL up with 4.7k. The piggyback has DNP 4.7k footprints only.
- J5 is a 4-pin 2.54 mm I2C expansion header (3V3_D, GND, SDA, SCL).

### Analog inputs: 2× ADS1115 (16-bit, I2C)

U1 is at 0x48 (ADDR = GND):
- AIN0 = pressure 1, AIN1 = pressure 2.
- AIN2 = 3V3_A / 2, through a 10k/10k divider, so the NTC math can be ratiometric.
- AIN3 = SAFE / 2, through a 10k/10k divider.

U2 is at 0x49 (ADDR = VDD):
- AIN0 = NTC1 (rotor, primary), AIN1 = NTC2 (rotor, secondary), AIN2 = NTC3 (motor).
- AIN3 is spare and goes to a test point.

Both run from 3V3_A, and ALERT/RDY are not connected.

**4–20 mA (×2):**
- Terminal: 2-pin 5.08, LOOP+ and SIG.
- LOOP+ = 24V_P through a 50 mA PTC per channel.
- SIG goes to GND through a 100 Ω 0.1 % 1206 shunt.
- The shunt node goes through 1k to the ADC pin. At the pin: 100 nF to GND and a BAT54S clamp to 3V3_A / GND.
- 20 mA gives 2.0 V. Firmware sets the PGA to ±4.096 V and flags readings below 3.8 mA as open loop.

**NTC (×3), 100k B≈4000:**
- Terminal: 3-pin 5.08, NTC, AGND, SHIELD (shield goes to AGND at this end only).
- 3V3_A → 33k 0.1 % → NTCn_NODE, which goes to the terminal; the probe returns to AGND.
- NTCn_NODE → 10k → ADC pin, with 100 nF to AGND and a BAT54S clamp.
- ESD diode (PESD5V0-class) at each terminal pin.

### Hardware over-temperature cutout: LM393 dual comparator on 3V3_A

The comparators take NTC1 and NTC3 as inputs. The thresholds are ratiometric dividers from 3V3_A.

| Ch | Input | Trip | Threshold divider (IN−) | Node at trip |
|---|---|---|---|---|
| A | NTC1 rotor | ≈ 110 °C (firmware trips at 105) | 100k / 15.4k (0.1334·Vcc) | 0.44 V |
| B | NTC3 motor | ≈ 140 °C (firmware faults at 130) | 100k / 7.15k (0.0667·Vcc) | 0.22 V |

- IN+ = NTCn_NODE through 100k. Hysteresis comes from 10 MΩ output→IN+ (about 3 K). The load this puts on the NTC node is below 0.2 K.
- The open-collector outputs are wire-ANDed with a 10k pull-up to 3V3_D. That gives **TEMP_OK**: high = OK, low = too hot.
- An open probe reads cold, so firmware catches it on the ADS (voltage-window check).

### Firmware watchdog: 74LVC1G123 retriggerable monostable

- HEARTBEAT is DOUT0, the Waveshare's NPN open collector to GND. It is pulled up to 3V3_D with 10k and feeds the B input. A is tied low and CLR high.
- Rx/Cx set tw to about 1 s. Firmware toggles DOUT0 at about 5 Hz.
- Q = **WD_OK**. It is low at power-up, and goes low again if the heartbeat stops in either state.

### Safety gating: 74LVC08 quad AND (3V3_D)

- Gate A: SAFE = WD_OK · TEMP_OK
- Gate B: V1_DRV = PCA_LED0 · SAFE
- Gate C: V2_DRV = PCA_LED1 · SAFE
- Gate D: VFD_EN = SAFE · SAFE, which drives the VFD enable opto.

**Status back to the S3:**
- Two 2N7002s pull DIN0 / DIN1 low when TEMP_OK / WD_OK are high.
- The Waveshare biases its DIN inputs from its own 5 V through 3.6k plus the opto LED.

### Valve PWM: PCA9685 (0x40, A0–A5 = GND, OE = GND, EXTCLK = GND)

- LED0 = start-assist valve, LED1 = drain valve.
- LED2 and LED3 drive status LEDs. LED4–15 are not connected.
- Firmware runs it at about 1.5 kHz (maximum prescale rate) with a 50 % hold after the pull-in time.

### Valve drivers (×2)

- Vn_DRV → 100 Ω → gate of an N-MOSFET, with 100k gate to GND.
- The MOSFET is logic-level: ≥ 60 V, ≥ 2 A, R_DS(on) < 100 mΩ at 2.5–3.3 V, SOT-23 or SOT-23-3 class. It is low-side.
- Flyback: SS34 Schottky across the coil (cathode to 24V_P).
- Indicator: LED + 10k from 24V_P to the drain.
- Terminal: 4-pin 5.08 (V1+, V1−, V2+, V2−). The + pins are 24V_P.
- Loads: start-assist about 290 mA, drain about 375 mA.

### VFD enable output

- VFD_EN → 330 Ω → LED of a PC817-class optocoupler (SMD).
- The transistor goes to J9, a 2-pin 5.08 terminal (DI, COM), wired to an SKI-70 digital input.
  Configure that input as normally-closed external fault / coast stop.
- The opto conducts only while SAFE is high. So a dead piggyback, a dead S3 or an over-temperature condition stops the motor.

## Mechanical: screwed to the back of the Waveshare (Lee approved 2026-11-04)

```
 front ->  [ cover glass 84.2 x 84.2 ]
           [ Waveshare PCB 74.5 x 76.5 ]   H4 plug sticks out of its back face
                ||  4x M2.5 standoffs, 20 mm (F-F), on the 66.50 x 68.50 mm pattern
           [ piggyback 100 x 80, parts facing away from the display ]
                ||  optional standoffs / DIN carrier to the enclosure
```

- The piggyback repeats the Waveshare's hole pattern (66.50 × 68.50 mm) centred on its own outline.
  Screws go through the piggyback into the standoffs, and through the standoffs into the Waveshare.
  The display and the piggyback become one rigid module.
- Hole size: **2.7 mm (M2.5) assumed**. The Waveshare drawing gives no diameter.
  Measure the board when it arrives; the piggyback can take 3.2 mm (M3) if the Waveshare holes are M3.
- The standoffs are 20 mm. That clears the green H4 plug (wires enter it sideways, towards the Waveshare's bottom edge), the battery header and the buzzer.
- Harness: a short 10-wire loom. The Waveshare's own H4 plug sits at one end, and a 3.5 mm 10P plug (15EDGK-3.5-10P class) mates J2 at the other.
  Use 0.5 mm² for VIN/GND and a twisted pair for A/B.
- **Keep-outs on the piggyback's display-facing side**: no tall parts under the H4 plug footprint.
  That is the Waveshare's lower edge, 41 mm wide, centred 11.08 + 20.5 mm from its left edge.
  All piggyback parts go on the far side anyway.
- Field terminals sit on the two outer edges that stick out past the Waveshare:
  - Bottom edge (100 mm): J1 24 V, J9 valves, J10 VFD enable, J11 Modbus.
  - Right edge (80 mm): J3/J4 4–20 mA, J6–J8 NTC.
  - J2 sits near the Waveshare's H4, so the loom stays short.

## Layout intent

- 100 × 80 mm, 2 layers. 4× M2.5 holes on the Waveshare pattern (see Mechanical).
- Valve drivers, 24 V entry and TVS on one side; ADS, NTC and 4–20 mA front end on the other.
- Solid GND plane on the bottom. AGND and GND tie at one point near the ADS.
