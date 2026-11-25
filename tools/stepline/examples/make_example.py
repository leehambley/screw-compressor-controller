# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["cadquery"]
# ///
"""Generate the example assemblies (original geometry): bracket.step, flange.step, gear.step.

    uv run --python 3.12 tools/stepline/examples/make_example.py
"""
import math
from pathlib import Path

import cadquery as cq

here = Path(__file__).parent


def save(parts, name):
    asm = cq.Assembly()
    for i, p in enumerate(parts):
        asm.add(p, name=f"part{i}")
    path = here / f"{name}.step"
    asm.export(str(path))
    print(path)


def bracket():
    """Tapered arm on a forked base, with a bolt and washer per foot."""
    b = (
        cq.Workplane("XZ")
        .polyline([(-30, 0), (30, 0), (30, 22), (16, 30), (22, 110),
                   (6, 140), (0, 140), (-14, 90), (-26, 40), (-30, 22)])
        .close()
        .extrude(14, both=True)
    )
    b = b.cut(cq.Workplane("XZ").center(0, 8).rect(26, 36).extrude(20, both=True))
    b = b.cut(cq.Workplane("YZ").workplane(offset=-40).center(0, 10).circle(4).extrude(80))
    for z in (50, 70, 90, 110):
        x = 14 - (z - 50) * 0.2
        b = b.cut(cq.Workplane("XZ").workplane(offset=-20).center(x, z).circle(1.6).extrude(40))
    parts = [b]
    for side in (1, -1):
        bolt = cq.Workplane("YZ").circle(6).extrude(4).union(cq.Workplane("YZ").circle(3.5).extrude(-26))
        if side < 0:
            bolt = bolt.rotate((0, 0, 0), (0, 0, 1), 180)
        parts.append(bolt.translate((side * 62, 0, 10)))
        parts.append(cq.Workplane("YZ").circle(9).circle(4).extrude(2).translate((side * 50, 0, 10)))
    return parts


def flange():
    """Hubbed flange, gasket and six cap screws, stacked along Z."""
    body = cq.Workplane("XY").circle(40).extrude(8).faces(">Z").workplane().circle(18).extrude(14)
    body = body.cut(cq.Workplane("XY").circle(10).extrude(40))
    bolt_pts = [(30 * math.cos(a), 30 * math.sin(a)) for a in (i * math.pi / 3 for i in range(6))]
    body = body.cut(cq.Workplane("XY").pushPoints(bolt_pts).circle(2.7).extrude(40))
    gasket = cq.Workplane("XY").circle(40).circle(10).extrude(1.5).translate((0, 0, -1.5))
    gasket = gasket.cut(cq.Workplane("XY").pushPoints(bolt_pts).circle(2.7).extrude(-5))
    parts = [body, gasket]
    for x, y in bolt_pts:
        screw = cq.Workplane("XY").circle(4.5).extrude(4).union(cq.Workplane("XY").circle(2.5).extrude(-14))
        parts.append(screw.translate((x, y, 8 + 4)))  # head resting 4 mm above the flange face
    return parts


def gear():
    """Spur gear on a keyed shaft."""
    teeth, r_root, r_tip = 18, 28.0, 33.0
    pts = []
    for i in range(teeth):
        a = 2 * math.pi * i / teeth
        w = math.pi / teeth
        for da, r in ((-0.50 * w, r_root), (-0.28 * w, r_tip), (0.28 * w, r_tip), (0.50 * w, r_root)):
            pts.append((r * math.cos(a + da), r * math.sin(a + da)))
    g = cq.Workplane("XY").polyline(pts).close().extrude(10)
    g = g.faces(">Z").workplane().circle(12).extrude(6)
    g = g.cut(cq.Workplane("XY").circle(6).extrude(30))
    g = g.cut(cq.Workplane("XY").center(6.2, 0).rect(3, 3).extrude(30))
    shaft = cq.Workplane("XY").circle(6).extrude(46).translate((0, 0, -14))
    key = cq.Workplane("XY").center(6, 0).rect(3, 3).extrude(14).translate((0, 0, 2))
    return [g, shaft, key]


save(bracket(), "bracket")
save(flange(), "flange")
save(gear(), "gear")
