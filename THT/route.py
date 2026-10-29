"""Autoroute THT.kicad_pcb with Freerouting, then pour ground on both layers.

Run after gen_board.py, with KiCad's bundled Python. GND is left out of the
autorouter: every pad is through-hole, so the two ground pours reach all of
them, and a near-solid ground plane is the point of the exercise (the
breadboard's analog readings were wrecked by a shared ground path).
"""

import os
import re
import subprocess
import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
PCB = os.path.join(HERE, "THT.kicad_pcb")
DSN = os.path.join(HERE, "build", "THT.dsn")
SES = os.path.join(HERE, "build", "THT.ses")
FREEROUTING = os.path.expanduser(
    "~/Documents/KiCad/10.0/3rdparty/plugins/app_freerouting_kicad-plugin/jar/freerouting-2.3.0.jar"
)

JAVA = "/opt/homebrew/opt/openjdk@25/bin/java"  # Freerouting 2.3 needs Java 25
POWER = ["+24V", "VIN_RAW", "VIN_F", "SW", "+5V", "5V_DK", "DRAIN1", "DRAIN2"]
# um (the DSN's resolution unit)
SIGNAL_WIDTH, SIGNAL_CLEAR = 400, 250
POWER_WIDTH, POWER_CLEAR = 1000, 300
VIA = "Via[0-1]_1000:500_um"


def patch_dsn(text):
    text = text.replace("Via[0-1]_600:300_um", VIA)
    text = re.sub(r"\(circle (F|B)\.Cu 600\)", r"(circle \1.Cu 1000)", text)
    text = text.replace("(width 200)\n      (clearance 200)",
                        f"(width {SIGNAL_WIDTH})\n      (clearance {SIGNAL_CLEAR})", 1)
    # Drop GND from the network so the router leaves it to the pours.
    text = re.sub(r"\n\s*\(net GND\n\s*\(pins[^)]*\)\n\s*\)", "", text)
    m = re.search(r"\(class kicad_default (.*?)\n\s*\(circuit", text, re.S)
    nets = [n for n in m.group(1).split() if n != "GND"]
    signal = [n for n in nets if n not in POWER]
    power = [n for n in nets if n in POWER]
    classes = (
        f"(class signal {' '.join(signal)}\n"
        f"      (circuit (use_via \"{VIA}\"))\n"
        f"      (rule (width {SIGNAL_WIDTH}) (clearance {SIGNAL_CLEAR}))\n    )\n"
        f"    (class power {' '.join(power)}\n"
        f"      (circuit (use_via \"{VIA}\"))\n"
        f"      (rule (width {POWER_WIDTH}) (clearance {POWER_CLEAR}))\n    )"
    )
    return re.sub(r"\(class kicad_default .*?\n    \)", classes, text, count=1, flags=re.S)


def add_gnd_zone(board, layer, gnd):
    z = pcbnew.ZONE(board)
    z.SetLayer(layer)
    z.SetNet(gnd)
    z.SetLocalClearance(pcbnew.FromMM(0.4))
    z.SetMinThickness(pcbnew.FromMM(0.3))
    z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
    z.SetThermalReliefGap(pcbnew.FromMM(0.5))
    z.SetThermalReliefSpokeWidth(pcbnew.FromMM(0.8))
    z.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
    o = z.Outline()
    o.NewOutline()
    bb = board.GetBoardEdgesBoundingBox()
    for x, y in ((bb.GetLeft(), bb.GetTop()), (bb.GetRight(), bb.GetTop()),
                 (bb.GetRight(), bb.GetBottom()), (bb.GetLeft(), bb.GetBottom())):
        o.Append(pcbnew.VECTOR2I(x, y))
    board.Add(z)


def main():
    os.makedirs(os.path.dirname(DSN), exist_ok=True)
    board = pcbnew.LoadBoard(PCB)
    for t in list(board.GetTracks()):
        board.Remove(t)
    for z in list(board.Zones()):
        if not z.GetIsRuleArea():
            board.Remove(z)
    assert pcbnew.ExportSpecctraDSN(board, DSN)
    with open(DSN) as f:
        text = patch_dsn(f.read())
    with open(DSN, "w") as f:
        f.write(text)

    subprocess.run([JAVA, "-jar", FREEROUTING, "-de", DSN, "-do", SES, "-mp", "100",
                    "--gui.enabled=false"], check=True)

    assert pcbnew.ImportSpecctraSES(board, SES)
    # Power vias were imported at the DSN size; give them a matching drill.
    for t in board.GetTracks():
        if isinstance(t, pcbnew.PCB_VIA):
            t.SetWidth(pcbnew.FromMM(1.0))
            t.SetDrill(pcbnew.FromMM(0.5))

    gnd = board.FindNet("GND")
    add_gnd_zone(board, pcbnew.F_Cu, gnd)
    add_gnd_zone(board, pcbnew.B_Cu, gnd)
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    pcbnew.SaveBoard(PCB, board)
    print("routed and poured:", PCB)


main()
