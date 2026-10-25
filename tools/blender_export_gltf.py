# SPDX-License-Identifier: MPL-2.0
"""Convert a Blender scene to binary glTF (.glb) for CrySirror (ADR 0007).

Runs inside Blender, which is the only reliable reader of .blend files:

    Blender --background scene.blend --python tools/blender_export_gltf.py -- --output out.glb
    Blender --background --factory-startup --python tools/blender_export_gltf.py -- \
        --build-fixture --output examples/engine/assets/lantern.glb

--build-fixture replaces the scene with a small procedurally generated model
(original CrySirror content, safe to distribute) so the pipeline is
reproducible without any user .blend file.
"""
import argparse
import sys

import bpy


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--output", required=True)
    p.add_argument("--build-fixture", action="store_true")
    return p.parse_args(argv)


def material(name, rgba):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = rgba
    return mat


def build_fixture():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    stone = material("stone", (0.05, 0.05, 0.06, 1.0))
    paper = material("paper", (0.9, 0.55, 0.12, 1.0))
    roof = material("roof", (0.6, 0.03, 0.02, 1.0))

    bpy.ops.mesh.primitive_cylinder_add(vertices=12, radius=0.6, depth=0.3, location=(0, 0, 0.15))
    base = bpy.context.active_object
    base.name = "base"
    base.data.materials.append(stone)

    bpy.ops.mesh.primitive_cube_add(size=0.8, location=(0, 0, 0.7))
    body = bpy.context.active_object
    body.name = "body"
    body.data.materials.append(paper)
    body.rotation_euler = (0, 0, 0.785398)

    bpy.ops.mesh.primitive_cone_add(vertices=4, radius1=0.75, depth=0.5, location=(0, 0, 1.35))
    top = bpy.context.active_object
    top.name = "roof"
    top.data.materials.append(roof)

    # Hierarchy base -> body -> roof, keeping world placement.
    for child, parent in ((body, base), (top, body)):
        child.parent = parent
        child.matrix_parent_inverse = parent.matrix_world.inverted()


def main():
    a = args()
    if a.build_fixture:
        build_fixture()
    bpy.ops.export_scene.gltf(
        filepath=a.output,
        export_format="GLB",
        export_apply=True,
        export_yup=True,
    )
    print(f"crysirror: exported {a.output}")


main()
