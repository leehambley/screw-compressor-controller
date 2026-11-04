# THT controller board (rev A)

A 100 × 100 mm, 2-layer through-hole version of the breadboard controller,
for hand soldering. There is no schematic: [gen_board.py](gen_board.py) declares
every part, position and net, and is the source of truth.

## Ordering (JLCPCB)

Upload `production/THT-gerbers-JLCPCB.zip`. Defaults are fine: 2 layers,
1.6 mm, HASL, 1 oz copper. Parts list: `production/BOM.csv`.

## Rebuilding

```sh
PY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
$PY gen_board.py   # parts + nets, unrouted
$PY route.py       # Freerouting (needs Homebrew openjdk@25) + GND pours
```

Then re-export:

```sh
K=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli
rm -rf production/gerbers && mkdir -p production/gerbers
$K pcb drc -o build/drc.txt THT.kicad_pcb
$K pcb export gerbers --layers F.Cu,B.Cu,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts --no-protel-ext -o production/gerbers/ THT.kicad_pcb
$K pcb export drill --format excellon --excellon-separate-th -o production/gerbers/ THT.kicad_pcb
(cd production/gerbers && rm -f ../THT-gerbers-JLCPCB.zip && zip -q ../THT-gerbers-JLCPCB.zip *)
```

## Circuit

| Block | Parts | Notes |
|---|---|---|
| 24 V in | J1, F1 (2 A slow 5×20), D1 1N5822, C1, C2 | D1 = reverse-polarity protection |
| 5 V | U2 LM2596T-5.0, L1 33 µH, D2 1N5822, C3, C4 | feeds Nextion directly, DevKit 5V pin via D3 1N5819 (no back-feed into USB) |
| 3V3 | DevKit's own regulator | MAX3485, NTC pull-ups, ADC clamp |
| Valve 1 (start-assist, GPIO20) | Q1 IRLZ34N, R10 100R gate, R12 100k pull-down, D4 1N5819, LED2 | |
| Valve 2 (drain, GPIO21) | Q2, R11, R13, D5, LED3 | |
| Pressure 4–20 mA (GPIO0) | J4, R20 100R shunt, R21 1k + C20 100nF, D20 BAT85 clamp | J4: top = +24 V loop supply, bottom = signal |
| NTC1 rotor (GPIO1), NTC2 motor (GPIO2), NTC3 rotor 2 (GPIO3) | 10k pull-up, 1k + 100nF each | probes are non-polar |
| RS485 (GPIO18 DI, GPIO19 RO, GPIO10 DE/RE) | U3 MAX3485 (DIP-8, socketed), R40 120R via JP1, R42 10k RO pull-up | J8: A, B, GND |
| Nextion (GPIO22 RX, GPIO23 TX) | J9 JST XH 4-pin (B4B-XH-A): 5V, TX, RX, GND | same connector and pin order as the display end |

Bottom layer is an almost unbroken ground plane, top is ground-poured too.
Valve current returns along the right-hand side to J1; the analog front
end is on the far left, away from it. That is the lesson from the
breadboard, whose readings were wrecked by a shared, poorly soldered ground.

Flyback diodes are Schottky (1N5819), so the valve PWM hold can go well
above 1 kHz (quiet) without the 1N400x reverse-recovery heating.

## Check before ordering parts

- F1 clips: footprint is Littelfuse 100-series clips, 20.5 mm pitch. Buy
  matching clips or swap the footprint for your holder.
- L1: 12 mm radial, 5 mm lead pitch, ≥ 1.5 A.
- U2: footprint is for the staggered-lead TO-220-5 (LM2596T-5.0 /
  "flow LB03"); straight-lead parts can be bent to fit.
- DevKit goes in 2× 1x16 female headers; its USB ports overhang the bottom
  edge so it can still be flashed in place.

## Firmware follow-ups

- NTC2 (GPIO2) and NTC3 (GPIO3) are new; the firmware only reads GPIO1.
