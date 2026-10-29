"""Generate the through-hole controller board (THT.kicad_pcb).

This script is the board's source of truth: there is no schematic. Every
part, its position and every net is declared below; re-run it to rebuild
the unrouted board, then `route.sh` autoroutes it and pours ground.

Run with KiCad's bundled Python:
  /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 gen_board.py

Circuit summary (all THT, hand-soldered, 2-layer):
  24V in -> 5x20 fuse -> 1N5822 reverse-polarity diode -> +24V
  +24V -> LM2596T-5.0 buck -> +5V (Nextion, DevKit via 1N5819)
  DevKit 3V3 pin -> MAX3485, NTC pull-ups, ADC clamp
  2x IRLZ34N low-side valve drivers, 1N5819 Schottky flyback (fast enough
    for PWM hold well above 1 kHz), LED per valve
  4-20mA pressure: 100R shunt -> 1k/100nF -> GPIO0, BAT85 clamp to 3V3
  3x NTC: 10k pull-up, 1k/100nF -> GPIO1/2/3
  RS485: MAX3485 DIP-8, 120R termination via jumper
"""

import os
import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "THT.kicad_pcb")
KLIB = "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints/"
ESPLIB = os.path.expanduser(
    "~/Documents/KiCad/10.0/3rdparty/footprints/com_github_espressif_kicad-libraries/Espressif.pretty"
)

W, H = 100.0, 100.0
OX, OY = 100.0, 50.0  # board origin on the page

TERM2 = "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2-5.08_1x02_P5.08mm_Horizontal"
TERM3 = "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-3-5.08_1x03_P5.08mm_Horizontal"
RES = "Resistor_THT:R_Axial_DIN0207_L6.3mm_D2.5mm_P7.62mm_Horizontal"
CDISC = "Capacitor_THT:C_Disc_D5.0mm_W2.5mm_P5.00mm"
DO41 = "Diode_THT:D_DO-41_SOD81_P10.16mm_Horizontal"
DO35 = "Diode_THT:D_DO-35_SOD27_P7.62mm_Horizontal"
DO201 = "Diode_THT:D_DO-201AD_P15.24mm_Horizontal"
LED = "LED_THT:LED_D3.0mm"
TO220 = "Package_TO_SOT_THT:TO-220-3_Vertical"
MH = "MountingHole:MountingHole_3.2mm_M3_Pad_Via"

# Terminal rotations: the footprint's wire entry faces local +y.
LEFT, RIGHT, DOWN = 270, 90, 0

# (ref, footprint, value, x, y, rotation, {pad: net})
PARTS = [
    # --- DevKit (socketed on 2x 1x16 female headers), USB at the bottom edge
    ("U1", "ESP:ESP32-C6-DevKitC-1", "ESP32-C6-DevKitC-1", 37, 50, 0, {
        "1": "+3V3", "7": "ADC0", "8": "ADC1", "10": "MB_DE", "12": "ADC2",
        "13": "ADC3", "14": "5V_DK", "15": "GND", "18": "GND", "21": "GND",
        "23": "MB_DI", "24": "MB_RO", "25": "V1_IN", "26": "V2_IN",
        "27": "NEX_TX", "28": "NEX_RX", "32": "GND",
    }),

    # --- 24V input and protection
    ("J1", TERM2, "24V IN", 94, 18, RIGHT, {"1": "VIN_RAW", "2": "GND"}),
    ("F1", "Fuse:Fuseholder_Clip-5x20mm_Littelfuse_100_Inline_P20.50x4.60mm_D1.30mm_Horizontal",
     "2A slow 5x20", 60, 8, 0, {"1": "VIN_F", "2": "VIN_RAW"}),
    ("D1", DO201, "1N5822", 38, 8, 0, {"1": "+24V", "2": "VIN_F"}),
    ("C1", "Capacitor_THT:CP_Radial_D10.0mm_P5.00mm", "220uF 50V", 28, 9, 0, {"1": "+24V", "2": "GND"}),
    ("C2", CDISC, "100nF 50V", 47, 13, 0, {"1": "+24V", "2": "GND"}),

    # --- 5V buck (LM2596 datasheet fixed-5V application)
    ("U2", "Package_TO_SOT_THT:TO-220-5_P3.4x3.7mm_StaggerOdd_Lead3.8mm_Vertical", "LM2596T-5.0", 37, 24, 0,
     {"1": "+24V", "2": "SW", "3": "GND", "4": "+5V", "5": "GND"}),
    # Freewheel diode right under U2/L1 to keep the switching loop small.
    ("D2", DO201, "1N5822", 46, 31.5, 180, {"1": "SW", "2": "GND"}),
    ("L1", "Inductor_THT:L_Radial_D12.0mm_P5.00mm_Fastron_11P", "33uH 3A", 50, 22, 0, {"1": "SW", "2": "+5V"}),
    ("C3", "Capacitor_THT:CP_Radial_D8.0mm_P3.50mm", "220uF 25V low-ESR", 62.5, 22, 0, {"1": "+5V", "2": "GND"}),
    ("C4", CDISC, "100nF", 52, 32, 0, {"1": "+5V", "2": "GND"}),
    ("R1", RES, "1k", 71, 22, 0, {"1": "+5V", "2": "LED_PWR"}),
    ("LED1", LED, "green 3mm", 81.5, 22, 0, {"1": "GND", "2": "LED_PWR"}),
    # Diode keeps board 5V and USB VBUS from back-feeding each other.
    ("D3", DO41, "1N5819", 64, 88, 0, {"1": "5V_DK", "2": "+5V"}),

    # --- Valve 1: start-assist (GPIO20)
    ("J2", TERM2, "VALVE1", 94, 34, RIGHT, {"1": "+24V", "2": "DRAIN1"}),
    ("Q1", TO220, "IRLZ34N", 68, 32, 0, {"1": "G1", "2": "DRAIN1", "3": "GND"}),
    ("D4", DO41, "1N5819", 65, 37, 0, {"1": "+24V", "2": "DRAIN1"}),
    ("R10", RES, "100R", 65, 41, 0, {"1": "V1_IN", "2": "G1"}),
    ("R12", RES, "100k", 75, 41, 0, {"1": "G1", "2": "GND"}),
    ("R14", RES, "4k7", 78, 37, 0, {"1": "+24V", "2": "LED_V1"}),
    ("LED2", LED, "red 3mm", 80, 31, 0, {"1": "DRAIN1", "2": "LED_V1"}),

    # --- Valve 2: drain (GPIO21)
    ("J3", TERM2, "VALVE2", 94, 48, RIGHT, {"1": "+24V", "2": "DRAIN2"}),
    ("Q2", TO220, "IRLZ34N", 68, 47, 0, {"1": "G2", "2": "DRAIN2", "3": "GND"}),
    ("D5", DO41, "1N5819", 65, 52, 0, {"1": "+24V", "2": "DRAIN2"}),
    ("R11", RES, "100R", 65, 56, 0, {"1": "V2_IN", "2": "G2"}),
    ("R13", RES, "100k", 75, 56, 0, {"1": "G2", "2": "GND"}),
    ("R15", RES, "4k7", 78, 52, 0, {"1": "+24V", "2": "LED_V2"}),
    ("LED3", LED, "red 3mm", 80, 46, 0, {"1": "DRAIN2", "2": "LED_V2"}),

    # --- Pressure 4-20mA (GPIO0)
    ("J4", TERM2, "PRESSURE", 6, 30, LEFT, {"1": "+24V", "2": "LOOP"}),
    ("R20", RES, "100R 1%", 13, 28, 0, {"1": "LOOP", "2": "GND"}),
    ("R21", RES, "1k", 13, 32, 0, {"1": "LOOP", "2": "ADC0"}),
    ("D20", DO35, "BAT85", 13, 36, 0, {"1": "+3V3", "2": "ADC0"}),
    ("C20", CDISC, "100nF", 13, 40, 0, {"1": "ADC0", "2": "GND"}),

    # --- NTC1 rotor primary (GPIO1)
    ("J5", TERM2, "NTC1", 6, 44, LEFT, {"1": "NTC1", "2": "GND"}),
    ("R30", RES, "10k 1%", 13, 45, 0, {"1": "+3V3", "2": "NTC1"}),
    ("R33", RES, "1k", 13, 49, 0, {"1": "NTC1", "2": "ADC1"}),
    ("C30", CDISC, "100nF", 13, 53, 0, {"1": "ADC1", "2": "GND"}),

    # --- NTC2 motor winding (GPIO2)
    ("J6", TERM2, "NTC2", 6, 58, LEFT, {"1": "NTC2", "2": "GND"}),
    ("R31", RES, "10k 1%", 13, 59, 0, {"1": "+3V3", "2": "NTC2"}),
    ("R34", RES, "1k", 13, 63, 0, {"1": "NTC2", "2": "ADC2"}),
    ("C31", CDISC, "100nF", 13, 67, 0, {"1": "ADC2", "2": "GND"}),

    # --- NTC3 rotor secondary (GPIO3)
    ("J7", TERM2, "NTC3", 6, 72, LEFT, {"1": "NTC3", "2": "GND"}),
    ("R32", RES, "10k 1%", 13, 73, 0, {"1": "+3V3", "2": "NTC3"}),
    ("R35", RES, "1k", 13, 77, 0, {"1": "NTC3", "2": "ADC3"}),
    ("C32", CDISC, "100nF", 13, 81, 0, {"1": "ADC3", "2": "GND"}),

    # --- RS485 (MAX3485, DE+/RE on GPIO10)
    ("U3", "Package_DIP:DIP-8_W7.62mm_Socket", "MAX3485CPA", 25, 79, 0, {
        "1": "MB_RO", "2": "MB_DE", "3": "MB_DE", "4": "MB_DI",
        "5": "GND", "6": "RS_A", "7": "RS_B", "8": "+3V3",
    }),
    ("C40", CDISC, "100nF", 28, 92, 0, {"1": "+3V3", "2": "GND"}),
    ("R40", RES, "120R", 13, 86, 0, {"1": "RS_A", "2": "TERM"}),
    ("JP1", "Connector_PinHeader_2.54mm:PinHeader_1x02_P2.54mm_Vertical", "TERM", 28, 97, 90,
     {"1": "TERM", "2": "RS_B"}),
    ("R42", RES, "10k", 64, 66, 0, {"1": "+3V3", "2": "MB_RO"}),
    ("J8", TERM3, "RS485", 12, 95, DOWN, {"1": "RS_A", "2": "RS_B", "3": "GND"}),

    # --- Nextion (pin order matches its 4-wire cable: 5V, TX, RX, GND)
    ("J9", "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical", "NEXTION", 95, 70, 0,
     {"1": "+5V", "2": "NEX_TX", "3": "NEX_RX", "4": "GND"}),
    # --- Test points: 3V3, 5V, GND
    ("J11", "Connector_PinHeader_2.54mm:PinHeader_1x03_P2.54mm_Vertical", "TEST", 80, 80, 0,
     {"1": "+3V3", "2": "+5V", "3": "GND"}),

    ("H1", MH, "M3", 4, 4, 0, {"1": "GND"}),
    ("H2", MH, "M3", 96, 4, 0, {"1": "GND"}),
    ("H3", MH, "M3", 4, 96, 0, {"1": "GND"}),
    ("H4", MH, "M3", 96, 96, 0, {"1": "GND"}),
]

# Silkscreen labels: (text, x, y, rotation, size)
LABELS = [
    # Terminal labels sit in the gaps between terminals, where they stay
    # visible once the terminal bodies are soldered on.
    ("24V IN", 93, 8.6, 0, 1.0), ("-", 87.4, 12.92, 0, 1.4), ("+", 87.4, 18, 0, 1.4),
    ("VALVE1 ASSIST", 92, 24.4, 0, 1.0),
    ("VALVE2 DRAIN", 92, 38.4, 0, 1.0),
    ("PRESSURE +24V,SIG", 9.2, 25.5, 0, 1.0),
    ("NTC1 ROTOR", 6, 39.6, 0, 1.0),
    ("NTC2 MOTOR", 6, 53.6, 0, 1.0),
    ("NTC3 ROTOR2", 6.5, 67.6, 0, 1.0),
    ("RS485 A    B    GND", 17, 88.7, 0, 1.0),
    ("TERM", 29.3, 94.6, 0, 1.0),
    ("NEXTION", 88.6, 73.8, 90, 1.0), ("5V", 91.4, 70, 0, 1.0), ("TX", 91.4, 72.54, 0, 1.0),
    ("RX", 91.4, 75.08, 0, 1.0), ("GND", 91.0, 77.62, 0, 1.0),
    ("3V3", 83.8, 80, 0, 1.0), ("5V", 83.5, 82.54, 0, 1.0), ("GND", 83.8, 85.08, 0, 1.0),
    ("SCREW COMPRESSOR CONTROLLER", 78, 93, 0, 1.0), ("THT rev A", 78, 95.5, 0, 1.0),
]

# ESP32-C6 antenna: keep copper out from under it.
ANTENNA_KEEPOUT = (39.0, 41.5, 57.8, 48.6)


def mm(x, y):
    return pcbnew.VECTOR2I_MM(OX + x, OY + y)


def load(fp_id):
    lib, name = fp_id.split(":", 1)
    path = ESPLIB if lib == "ESP" else KLIB + lib + ".pretty"
    fp = pcbnew.FootprintLoad(path, name)
    if fp is None:
        raise SystemExit(f"footprint not found: {fp_id}")
    fp.SetFPID(pcbnew.LIB_ID("Espressif" if lib == "ESP" else lib, name))
    return fp


def rect(board, layer, x0, y0, x1, y1, width=0.1):
    for a, b in (((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))):
        s = pcbnew.PCB_SHAPE(board)
        s.SetShape(pcbnew.SHAPE_T_SEGMENT)
        s.SetStart(mm(*a))
        s.SetEnd(mm(*b))
        s.SetLayer(layer)
        s.SetWidth(pcbnew.FromMM(width))
        board.Add(s)


def main():
    board = pcbnew.BOARD()
    nets = {}

    def net(name):
        if name not in nets:
            n = pcbnew.NETINFO_ITEM(board, name)
            board.Add(n)
            nets[name] = n
        return nets[name]

    for ref, fp_id, value, x, y, rot, pads in PARTS:
        fp = load(fp_id)
        fp.SetReference(ref)
        fp.SetValue(value)
        fp.SetPosition(mm(x, y))
        fp.SetOrientationDegrees(rot)
        board.Add(fp)
        known = {p.GetNumber() for p in fp.Pads()}
        for num, n in pads.items():
            if num not in known:
                raise SystemExit(f"{ref}: no pad {num}")
        for p in fp.Pads():
            if p.GetNumber() in pads:
                p.SetNet(net(pads[p.GetNumber()]))
        # The DevKit's own pin labels make the reference/value clutter.
        if ref == "U1":
            fp.Value().SetVisible(False)
        # Mounting holes need no label; J8's sits under the RS485 label.
        elif ref.startswith("H") or ref == "J8":
            fp.Reference().SetVisible(False)
        # Small parts: reference inside the body outline, clear of neighbours.
        elif str(fp.GetFPID().GetLibItemName()).startswith(("R_Axial", "C_Disc", "D_DO", "Fuseholder", "L_Radial", "DIP")):
            r = fp.Reference()
            r.SetPosition(fp.GetBoundingBox(False).GetCenter())
            r.SetTextAngleDegrees(0)
            r.SetTextSize(pcbnew.VECTOR2I_MM(1.0, 1.0))
            r.SetTextThickness(pcbnew.FromMM(0.15))

    rect(board, pcbnew.Edge_Cuts, 0, 0, W, H, 0.15)

    for text, x, y, rot, size in LABELS:
        t = pcbnew.PCB_TEXT(board)
        t.SetText(text)
        t.SetPosition(mm(x, y))
        t.SetTextAngleDegrees(rot)
        t.SetTextSize(pcbnew.VECTOR2I_MM(size, size))
        t.SetTextThickness(pcbnew.FromMM(size * 0.15))
        t.SetLayer(pcbnew.F_SilkS)
        board.Add(t)

    x0, y0, x1, y1 = ANTENNA_KEEPOUT
    ko = pcbnew.ZONE(board)
    ko.SetIsRuleArea(True)
    ko.SetDoNotAllowTracks(True)
    ko.SetDoNotAllowVias(True)
    ko.SetDoNotAllowZoneFills(True)
    ko.SetDoNotAllowPads(False)
    ko.SetDoNotAllowFootprints(False)
    ko.SetLayerSet(pcbnew.LSET.AllCuMask())
    outline = ko.Outline()
    outline.NewOutline()
    for px, py in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
        outline.Append(mm(px, py))
    board.Add(ko)

    pcbnew.SaveBoard(OUT, board)
    print(f"wrote {OUT}: {len(PARTS)} parts, {len(nets)} nets")


main()
