# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["cadquery"]
# ///
"""STEP -> parts-catalog line-art PNG.

Tessellates every solid in a STEP file (cadquery/OCC), then hands the parts to
Blender, which renders them with Freestyle outlines and flat two-tone shading
(see blender_style.py).

    uv run tools/stepline/stepline.py part.step -o part.png
    uv run tools/stepline/stepline.py asm.step -o asm.png --explode 1.2 --size 2400

Needs `blender` on PATH (or --blender /path/to/blender). Meshes (.stl .obj .glb
.gltf .ply) are accepted too and skip the STEP step.
"""
import argparse
import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MESH_EXT = {".stl", ".obj", ".glb", ".gltf", ".ply"}


def tessellate_step(step: Path, outdir: Path, tol: float, ang_tol: float) -> int:
    import cadquery as cq

    shape = cq.importers.importStep(str(step))
    solids = shape.solids().vals()
    if not solids:  # sheet/surface-only STEP: fall back to the whole compound
        solids = [shape.val()]
    for i, s in enumerate(solids):
        cq.exporters.export(
            cq.Workplane(obj=s),
            str(outdir / f"part_{i:03d}.stl"),
            tolerance=tol,
            angularTolerance=ang_tol,
        )
    return len(solids)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("input", type=Path, help=".step/.stp or a mesh file")
    ap.add_argument("-o", "--out", type=Path, help="output PNG (default: <input>.png)")
    ap.add_argument("--size", type=int, default=1600, help="longest image side in px (default 1600)")
    ap.add_argument("--azimuth", type=float, default=45, help="camera azimuth in degrees (default 45)")
    ap.add_argument("--elevation", type=float, default=math.degrees(math.atan(1 / math.sqrt(2))),
                    help="camera elevation in degrees (default 35.264 = true isometric)")
    ap.add_argument("--explode", type=float, default=0.0,
                    help="push parts apart from the assembly centre by this factor (0 = off, try 0.6-1.5)")
    ap.add_argument("--outline", type=float, default=5.0, help="silhouette line weight (default 5.0)")
    ap.add_argument("--detail", type=float, default=1.3, help="interior line weight (default 1.3)")
    ap.add_argument("--shade", type=float, default=0.78, help="sRGB grey of the shadow side, 0-1 (default 0.78)")
    ap.add_argument("--crease", type=float, default=134,
                    help="faces meeting at less than this angle get an edge line; 180 = flat (default 134)")
    ap.add_argument("--tol", type=float, default=0.05, help="STEP tessellation tolerance in model units")
    ap.add_argument("--blender", default=shutil.which("blender") or "blender")
    ap.add_argument("--keep", action="store_true", help="keep intermediate meshes (printed path)")
    a = ap.parse_args()

    src = a.input.resolve()
    if not src.exists():
        sys.exit(f"no such file: {src}")
    out = (a.out or src.with_suffix(".png")).resolve()

    tmp = Path(tempfile.mkdtemp(prefix="stepline_"))
    try:
        if src.suffix.lower() in MESH_EXT:
            mesh_arg = src
        elif src.suffix.lower() in {".step", ".stp"}:
            n = tessellate_step(src, tmp, a.tol, 0.2)
            print(f"tessellated {n} solid(s)", file=sys.stderr)
            mesh_arg = tmp
        else:
            sys.exit(f"unsupported input type: {src.suffix}")

        cmd = [
            a.blender, "-b", "--factory-startup", "-P", str(HERE / "blender_style.py"), "--",
            "--input", str(mesh_arg), "--out", str(out), "--size", str(a.size),
            "--azimuth", str(a.azimuth), "--elevation", str(a.elevation),
            "--explode", str(a.explode), "--outline", str(a.outline), "--detail", str(a.detail),
            "--shade", str(a.shade), "--crease", str(a.crease),
        ]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0 or not out.exists():
            sys.stderr.write(r.stdout[-3000:] + r.stderr[-3000:])
            return r.returncode or 1
        print(out)
        return 0
    finally:
        if a.keep:
            print(f"kept {tmp}", file=sys.stderr)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
