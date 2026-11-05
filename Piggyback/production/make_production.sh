#!/usr/bin/env bash
# Regenerate the JLCPCB production files from the KiCad sources.
#
#   cd Piggyback && ./production/make_production.sh
#
# Outputs, all in production/:
#   gerbers/ and Piggyback-gerbers-JLCPCB.zip  copper, mask, paste, silk, edge, drills
#   Piggyback-BOM-JLCPCB.csv                   Comment,Designator,Footprint,LCSC Part #
#   Piggyback-CPL-JLCPCB.csv                   Designator,Mid X,Mid Y,Layer,Rotation
#   Piggyback-render-{top,bottom,iso}.png
#
# Parts left out of the BOM and CPL (not JLC-assembled):
#   - anything whose Value or Assembly field contains "DNP" (R2, R3, all J connectors)
#   - anything without an LCSC number (H1-H4 mounting holes, TP1-TP9 test pads)
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=production
PCB=Piggyback.kicad_pcb
SCH=Piggyback.kicad_sch
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

rm -rf "$OUT/gerbers"
mkdir -p "$OUT/gerbers"

kicad-cli pcb export gerbers \
  --layers F.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts \
  --subtract-soldermask --no-x2 --no-protel-ext \
  -o "$OUT/gerbers/" "$PCB"
kicad-cli pcb export drill \
  --format excellon --excellon-separate-th --generate-map --map-format gerberx2 \
  --drill-origin absolute --excellon-units mm \
  -o "$OUT/gerbers/" "$PCB"
rm -f "$OUT/Piggyback-gerbers-JLCPCB.zip"
(cd "$OUT/gerbers" && zip -q -X ../Piggyback-gerbers-JLCPCB.zip ./*)

kicad-cli sch export bom \
  --fields 'Reference,Value,Footprint,LCSC,Assembly' --labels 'Reference,Value,Footprint,LCSC,Assembly' \
  --group-by '' --exclude-dnp -o "$TMP/bom_raw.csv" "$SCH"
kicad-cli pcb export pos --format csv --units mm --side front -o "$TMP/pos.csv" "$PCB"

python3 - "$TMP/bom_raw.csv" "$TMP/pos.csv" "$OUT" <<'EOF'
import csv, re, sys
from collections import OrderedDict

bom_raw, pos_csv, out = sys.argv[1:]

def natkey(ref):
    m = re.match(r"([A-Za-z]+)(\d+)", ref)
    return (m.group(1), int(m.group(2))) if m else (ref, 0)

fitted = {}
groups = OrderedDict()
for row in csv.DictReader(open(bom_raw)):
    ref, val, fp, lcsc = row["Reference"], row["Value"], row["Footprint"], row["LCSC"].strip()
    if "DNP" in val.upper() or "DNP" in row.get("Assembly", "").upper() or not lcsc:
        continue
    fitted[ref] = True
    # One BOM line per LCSC part: JLC rejects two lines matched to the same part.
    g = groups.setdefault(lcsc, {"refs": [], "vals": set(), "fps": set()})
    g["refs"].append(ref); g["vals"].add(val); g["fps"].add(fp.split(":")[-1])

bad = [(l, g) for l, g in groups.items() if len(g["vals"]) > 1 or len(g["fps"]) > 1]
for l, g in bad:
    print(f"ERROR {l}: {sorted(g['refs'], key=natkey)} disagree on value {sorted(g['vals'])} "
          f"or footprint {sorted(g['fps'])}; fix the schematic", file=sys.stderr)
if bad:
    sys.exit(1)

rows = sorted(groups.items(), key=lambda kv: natkey(sorted(kv[1]["refs"], key=natkey)[0]))
with open(f"{out}/Piggyback-BOM-JLCPCB.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #"])
    for lcsc, g in rows:
        w.writerow([g["vals"].pop(), ",".join(sorted(g["refs"], key=natkey)), g["fps"].pop(), lcsc])

with open(pos_csv) as f, open(f"{out}/Piggyback-CPL-JLCPCB.csv", "w", newline="") as g:
    w = csv.writer(g)
    w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
    for r in sorted(csv.DictReader(f), key=lambda r: natkey(r["Ref"])):
        if r["Ref"] not in fitted:
            continue
        side = "Top" if r["Side"].lower().startswith("top") else "Bottom"
        w.writerow([r["Ref"], f'{float(r["PosX"]):.6f}mm', f'{float(r["PosY"]):.6f}mm',
                    side, f'{float(r["Rot"]):.6f}'])

print(f"BOM lines: {len(rows)}  placed parts: {len(fitted)}")
EOF

kicad-cli pcb render --side top    -w 1600 -h 1300 -o "$OUT/Piggyback-render-top.png" "$PCB"
kicad-cli pcb render --side bottom -w 1600 -h 1300 -o "$OUT/Piggyback-render-bottom.png" "$PCB"
kicad-cli pcb render --side top --rotate '-45,0,45' --perspective \
  -w 1600 -h 1300 -o "$OUT/Piggyback-render-iso.png" "$PCB"

echo "done: $OUT/"
