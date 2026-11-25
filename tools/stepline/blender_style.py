"""Blender side of stepline: flat two-tone shading + Freestyle ink lines.

Run through stepline.py, or directly:
    blender -b --factory-startup -P blender_style.py -- --input parts_dir_or_file --out out.png
"""
import argparse
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def parse():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--input", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--size", type=int, default=1600)
    p.add_argument("--azimuth", type=float, default=45)
    p.add_argument("--elevation", type=float, default=35.264)
    p.add_argument("--explode", type=float, default=0.0)
    p.add_argument("--outline", type=float, default=5.0)
    p.add_argument("--detail", type=float, default=1.3)
    p.add_argument("--shade", type=float, default=0.78)
    p.add_argument("--crease", type=float, default=134)
    return p.parse_args(argv)


def import_meshes(path: Path):
    files = sorted(path.glob("*.stl")) if path.is_dir() else [path]
    before = set(bpy.data.objects)
    for f in files:
        ext = f.suffix.lower()
        if ext == ".stl":
            bpy.ops.wm.stl_import(filepath=str(f))
        elif ext == ".obj":
            bpy.ops.wm.obj_import(filepath=str(f))
        elif ext in (".glb", ".gltf"):
            bpy.ops.import_scene.gltf(filepath=str(f))
        elif ext == ".ply":
            bpy.ops.wm.ply_import(filepath=str(f))
    objs = [o for o in bpy.data.objects if o not in before and o.type == "MESH"]
    # glTF nests meshes under empties with transforms: bake them into world space.
    for o in objs:
        mw = o.matrix_world.copy()
        o.parent = None
        o.matrix_world = mw
    for o in bpy.data.objects:
        if o not in objs and o not in before:
            bpy.data.objects.remove(o)
    return objs


def bbox_world(objs):
    pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    return lo, hi


def volume_centroid(o):
    """Volume-weighted centroid of a closed mesh, in world space."""
    me = o.data
    me.calc_loop_triangles()
    mw = o.matrix_world
    vol, acc = 0.0, Vector((0, 0, 0))
    for t in me.loop_triangles:
        p0, p1, p2 = (mw @ me.vertices[i].co for i in t.vertices)
        v = p0.dot(p1.cross(p2)) / 6.0
        vol += v
        acc += v * (p0 + p1 + p2) / 4.0
    if abs(vol) < 1e-12:
        lo, hi = bbox_world([o])
        return (lo + hi) / 2, 0.0
    return acc / vol, abs(vol)


def explode(objs, k):
    """Catalog-style explosion: each part slides along its own insertion axis.

    Axis: the long axis of a rod-like part (bolts), else the thin axis of a flat one
    (washers, gaskets). Direction: toward the heavy end of the part (a bolt's head),
    else away from the biggest part. Distance scales with k.
    """
    info = []
    for o in objs:
        lo, hi = bbox_world([o])
        cen, vol = volume_centroid(o)
        info.append((o, lo, hi, (lo + hi) / 2, cen, vol))
    ref = max(info, key=lambda t: t[5])[3]
    for o, lo, hi, bc, cen, vol in info:
        d = [hi[i] - lo[i] for i in range(3)]
        order = sorted(range(3), key=lambda i: d[i])
        if d[order[2]] >= 1.4 * d[order[1]]:
            axis = order[2]
        elif d[order[0]] <= 0.6 * d[order[1]]:
            axis = order[0]
        else:
            axis = max(range(3), key=lambda i: abs(bc[i] - ref[i]) / max(d[i], 1e-6))
        own = cen[axis] - bc[axis]
        if abs(own) > 0.08 * d[axis]:
            sign = math.copysign(1, own)
        else:
            off = bc[axis] - ref[axis]
            if abs(off) < 1e-6:
                continue
            sign = math.copysign(1, off)
        dist = k * (abs(bc[axis] - ref[axis]) + 0.25)
        move = Vector((0, 0, 0))
        move[axis] = sign * dist
        o.location += move
    bpy.context.view_layer.update()
    lo, hi = bbox_world(objs)
    centre = (lo + hi) / 2
    for o in objs:
        o.location -= centre
    bpy.context.view_layer.update()


def make_toon_material(shade: float):
    shade = shade ** 2.2
    mat = bpy.data.materials.new("toon")
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()
    diff = nt.nodes.new("ShaderNodeBsdfDiffuse")
    diff.inputs["Color"].default_value = (1, 1, 1, 1)
    s2rgb = nt.nodes.new("ShaderNodeShaderToRGB")
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.interpolation = "CONSTANT"
    ramp.color_ramp.elements[0].position = 0.0
    ramp.color_ramp.elements[0].color = (shade, shade, shade, 1)
    ramp.color_ramp.elements[1].position = 0.18
    ramp.color_ramp.elements[1].color = (1, 1, 1, 1)
    emit = nt.nodes.new("ShaderNodeEmission")
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    nt.links.new(diff.outputs["BSDF"], s2rgb.inputs["Shader"])
    nt.links.new(s2rgb.outputs["Color"], ramp.inputs["Fac"])
    nt.links.new(ramp.outputs["Color"], emit.inputs["Color"])
    nt.links.new(emit.outputs["Emission"], out.inputs["Surface"])
    return mat


def setup_world():
    """White for camera rays, black for lighting, so shading comes only from the sun."""
    w = bpy.data.worlds.new("w")
    w.use_nodes = True
    nt = w.node_tree
    nt.nodes.clear()
    lp = nt.nodes.new("ShaderNodeLightPath")
    white = nt.nodes.new("ShaderNodeBackground")
    white.inputs["Color"].default_value = (1, 1, 1, 1)
    black = nt.nodes.new("ShaderNodeBackground")
    black.inputs["Color"].default_value = (0, 0, 0, 1)
    mix = nt.nodes.new("ShaderNodeMixShader")
    out = nt.nodes.new("ShaderNodeOutputWorld")
    nt.links.new(lp.outputs["Is Camera Ray"], mix.inputs["Fac"])
    nt.links.new(black.outputs["Background"], mix.inputs[1])
    nt.links.new(white.outputs["Background"], mix.inputs[2])
    nt.links.new(mix.outputs["Shader"], out.inputs["Surface"])
    bpy.context.scene.world = w


def setup_freestyle(scene, view_layer, outline, detail, crease, width_px):
    # Line weights are authored for a ~1000 px image (longest side) and scale with the output.
    k = width_px / 1000.0
    scene.render.use_freestyle = True
    scene.render.line_thickness_mode = "ABSOLUTE"
    view_layer.use_freestyle = True
    fs = view_layer.freestyle_settings
    fs.crease_angle = math.radians(crease)
    fs.as_render_pass = False
    for ls in list(fs.linesets):
        fs.linesets.remove(ls)

    def lineset(name, thickness, **sel):
        ls = fs.linesets.new(name)
        ls.select_by_visibility = True
        ls.visibility = "VISIBLE"
        ls.select_by_edge_types = True
        for attr in ("select_silhouette", "select_border", "select_crease", "select_contour",
                     "select_external_contour", "select_material_boundary", "select_edge_mark"):
            setattr(ls, attr, bool(sel.get(attr, False)))
        ls.select_by_image_border = False
        st = bpy.data.linestyles.new(name)
        st.color = (0, 0, 0)
        st.thickness = thickness * k
        st.thickness_position = "CENTER"
        st.caps = "ROUND"
        ls.linestyle = st
        return ls

    # Thin interior detail first, heavy "grease pencil" outline on top.
    lineset("detail", detail, select_crease=True, select_contour=True, select_silhouette=True,
            select_border=True, select_material_boundary=True)
    lineset("outline", outline, select_external_contour=True)


def main():
    a = parse()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    view_layer = bpy.context.view_layer

    objs = import_meshes(Path(a.input))
    if not objs:
        sys.exit("nothing imported")

    # Normalise: centre on the origin, longest dimension = 2 units.
    lo, hi = bbox_world(objs)
    centre = (lo + hi) / 2
    size = max(hi - lo)
    s = 2.0 / size if size > 0 else 1.0
    for o in objs:
        o.location = (o.location - centre) * s
        o.scale = o.scale * s
    bpy.context.view_layer.update()

    if a.explode > 0 and len(objs) > 1:
        explode(objs, a.explode)

    mat = make_toon_material(a.shade)
    for o in objs:
        o.data.materials.clear()
        o.data.materials.append(mat)
        for p in o.data.polygons:
            p.use_smooth = False

    # Light from upper-left-front so the right-hand faces fall into shade.
    ld = bpy.data.lights.new("sun", "SUN")
    ld.use_shadow = False
    lo_ = bpy.data.objects.new("sun", ld)
    scene.collection.objects.link(lo_)
    az_r = math.radians(a.azimuth)
    right = Vector((math.cos(az_r), math.sin(az_r), 0))
    toward_cam = Vector((math.sin(az_r), -math.cos(az_r), 0))
    to_light = (-0.8 * right + 0.3 * toward_cam + Vector((0, 0, 0.75))).normalized()
    lo_.rotation_euler = (-to_light).to_track_quat("-Z", "Y").to_euler()

    setup_world()

    # Orthographic camera.
    cam_d = bpy.data.cameras.new("cam")
    cam_d.type = "ORTHO"
    cam = bpy.data.objects.new("cam", cam_d)
    scene.collection.objects.link(cam)
    scene.camera = cam
    az, el = math.radians(a.azimuth), math.radians(a.elevation)
    dist = 20.0
    cam.location = Vector((math.sin(az) * math.cos(el), -math.cos(az) * math.cos(el), math.sin(el))) * dist
    cam.rotation_euler = (math.pi / 2 - el, 0, az)

    # Fit the frame to the projected bounding box.
    bpy.context.view_layer.update()
    inv = cam.matrix_world.inverted()
    pts = [inv @ (o.matrix_world @ Vector(c)) for o in objs for c in o.bound_box]
    minx, maxx = min(p.x for p in pts), max(p.x for p in pts)
    miny, maxy = min(p.y for p in pts), max(p.y for p in pts)
    w, h = maxx - minx, maxy - miny
    pad = 1.12
    cam_d.ortho_scale = max(w, h) * pad
    cam_d.shift_x = ((minx + maxx) / 2) / cam_d.ortho_scale
    cam_d.shift_y = ((miny + maxy) / 2) / cam_d.ortho_scale
    cam_d.clip_end = 100

    aspect = w / h
    if aspect >= 1:
        rx, ry = a.size, max(1, int(a.size / aspect))
    else:
        rx, ry = max(1, int(a.size * aspect)), a.size
    scene.render.resolution_x, scene.render.resolution_y = rx, ry
    scene.render.resolution_percentage = 100

    eng = "BLENDER_EEVEE" if "BLENDER_EEVEE" in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items else "BLENDER_EEVEE_NEXT"
    scene.render.engine = eng
    scene.eevee.taa_render_samples = 32
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"

    setup_freestyle(scene, view_layer, a.outline, a.detail, a.crease, max(rx, ry))

    scene.render.filepath = a.out
    bpy.ops.render.render(write_still=True)


main()
