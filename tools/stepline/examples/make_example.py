# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["cadquery"]
# ///
"""Generate examples/bracket.step: an original fork bracket with bolts and washers.

    uv run tools/stepline/examples/make_example.py
"""
from pathlib import Path

import cadquery as cq

out = Path(__file__).with_name("bracket.step")

# Tapered arm on a forked base (profile in the XZ plane, 28 mm thick in Y).
bracket = (
    cq.Workplane("XZ")
    .polyline([(-30, 0), (30, 0), (30, 22), (16, 30), (22, 110),
               (6, 140), (0, 140), (-14, 90), (-26, 40), (-30, 22)])
    .close()
    .extrude(14, both=True)
)
# U-shaped slot between the fork feet, and a cross bore through them.
bracket = bracket.cut(cq.Workplane("XZ").center(0, 8).rect(26, 36).extrude(20, both=True))
bracket = bracket.cut(cq.Workplane("YZ").workplane(offset=-40).center(0, 10).circle(4).extrude(80))
# A row of small screw holes down the face of the arm.
for z in (50, 70, 90, 110):
    x = 14 - (z - 50) * 0.2
    bracket = bracket.cut(cq.Workplane("XZ").workplane(offset=-20).center(x, z).circle(1.6).extrude(40))

parts = [bracket]
for side in (1, -1):
    bolt = (
        cq.Workplane("YZ").circle(6).extrude(4)
        .union(cq.Workplane("YZ").circle(3.5).extrude(-26))
    )
    if side < 0:
        bolt = bolt.rotate((0, 0, 0), (0, 0, 1), 180)
    parts.append(bolt.translate((side * 62, 0, 10)))
    parts.append(cq.Workplane("YZ").circle(9).circle(4).extrude(2).translate((side * 50, 0, 10)))

asm = cq.Assembly()
for i, p in enumerate(parts):
    asm.add(p, name=f"part{i}")
asm.save(str(out))
print(out)
