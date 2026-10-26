# SPDX-License-Identifier: MPL-2.0
# Generates crates/crysirror-asset/tests/data/bar.fbx (self-authored fixture):
#   Blender -b --factory-startup --python tools/blender/make_fbx_fixture.py -- <out.fbx>
# Recorded output came from Blender 5.2.2 LTS.
import bpy, sys, math
out = sys.argv[sys.argv.index("--")+1]
bpy.ops.wm.read_factory_settings(use_empty=True)
# Self-authored: a 2-segment bar skinned to 2 bones, bending over 1 s, with a 4x4 texture.
bpy.ops.mesh.primitive_cube_add(size=1)
bar = bpy.context.active_object; bar.name = "Bar"
bar.scale = (0.2, 0.2, 1.0); bpy.ops.object.transform_apply(scale=True)
bar.location = (0, 0, 1.0); bpy.ops.object.transform_apply(location=True)
bpy.ops.object.mode_set(mode='EDIT'); bpy.ops.mesh.subdivide(number_cuts=3); bpy.ops.object.mode_set(mode='OBJECT')
img = bpy.data.images.new("checker", 4, 4, alpha=False)
img.pixels = [c for i in range(16) for c in ((1,0.2,0.2,1) if (i%4+i//4)%2 else (0.2,0.2,1,1))]
img.filepath_raw = out.replace(".fbx", "_checker.png"); img.file_format = 'PNG'; img.save()
mat = bpy.data.materials.new("BarMat"); mat.use_nodes = True
bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
tex = mat.node_tree.nodes.new("ShaderNodeTexImage"); tex.image = img
mat.node_tree.links.new(tex.outputs[0], bsdf.inputs[0])
bar.data.materials.append(mat)
bpy.ops.object.armature_add(location=(0,0,0))
arm = bpy.context.active_object; arm.name = "Rig"
bpy.ops.object.mode_set(mode='EDIT')
b0 = arm.data.edit_bones[0]; b0.name = "Lower"; b0.head = (0,0,0); b0.tail = (0,0,1)
b1 = arm.data.edit_bones.new("Upper"); b1.head = (0,0,1); b1.tail = (0,0,2); b1.parent = b0
bpy.ops.object.mode_set(mode='OBJECT')
bar.select_set(True); arm.select_set(True); bpy.context.view_layer.objects.active = arm
bpy.ops.object.parent_set(type='ARMATURE_AUTO')
bpy.ops.object.mode_set(mode='POSE')
pb = arm.pose.bones["Upper"]; pb.rotation_mode = 'XYZ'
scene = bpy.context.scene; scene.frame_start = 1; scene.frame_end = 25; scene.render.fps = 24
pb.rotation_euler = (0,0,0); pb.keyframe_insert("rotation_euler", frame=1)
pb.rotation_euler = (math.radians(90),0,0); pb.keyframe_insert("rotation_euler", frame=25)
bpy.ops.object.mode_set(mode='OBJECT')
bpy.ops.export_scene.fbx(filepath=out, path_mode='COPY', embed_textures=True, bake_anim=True, add_leaf_bones=False)
print("WROTE", out)
