"""Bake the Machine God team's rigged Deus Ex bodies (Humans.blend) to glTF for DX RUST's skeletal characters.

Run with Blender:  blender -b -P bake_humans.py -- <Humans.blend> <out_dir> [only: DX mesh names / anims, comma-separated]
(or with the bpy module: python bake_humans.py -- <Humans.blend> <out_dir>)
Writes, per rig (male / female):
  <out>/anims_<rig>.glb     the rig's deforming skeleton + every animation (baked from the Rigify controls),
and per body:
  <out>/<DXMesh>.glb        the rebuilt body, skinned to the same skeleton, with its face shape keys as morph targets.
Only the rebuilt "_New" bodies and the rig's own animations are exported: the reference copies of the original Deus Ex
meshes and their vertex animations that sit in the file are left out (they are the game's own data).
Rig, bodies and animations: the Machine God team (credit required).
"""
import bpy, sys, os, time

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
bpy.ops.wm.open_mainfile(filepath=argv[0])
OUT = argv[1] if len(argv) > 1 else "rig_out"
os.makedirs(OUT, exist_ok=True)
ONLY = set(argv[2].split(",")) if len(argv) > 2 else None

# Rebuilt body -> the Deus Ex mesh it stands in for.
BODIES = {
    "GM_Trench_New": "GM_Trench",
    "GM_Suit_New": "GM_Suit",
    "GM_DressShirt_New": "GM_DressShirt",
    "GM_DressShirt_B_New": "GM_DressShirt_B",
    "GM_Jumpsuit_New": "GM_Jumpsuit",
    "GMK_DressShirt_New": "GMK_DressShirt",
    "GFM_Trench_New": "GFM_Trench",
    "GFM_Dress_New": "GFM_Dress",
    "GFM_SuitSkirt_New": "GFM_SuitSkirt",
    "GFM_TShirtPants_New": "GFM_TShirtPants",
    "GFMK_Dress_New": "GFMK_Dress",
    "GFM_Jumpsuit_New": "GFM_Jumpsuit",
}
FACE_KEYS = {"Basis", "MouthA", "MouthE", "MouthF", "MouthM", "MouthO", "MouthT", "MouthU", "Blink", "BrowsUp",
             "BrowsDown", "MouthSmile", "MouthSnarl", "EyesRight", "EyesLeft", "BasisFat", "BasisSkinny", "SmirkRight"}
SKIP_ACTIONS = ("Key", "REF-", "GM_", "GFM_", "GMK_", "GFMK_", "Wall", "Still.001", "WaterHitTorsoBack.001")


def bone_action(a):
    return not a.name.startswith(SKIP_ACTIONS) and any(fc.data_path.startswith("pose.bones") for fc in a.fcurves)


def strip_keys(ob):
    k = ob.data.shape_keys
    if not k:
        return
    if k.animation_data:
        for d in list(k.animation_data.drivers):
            k.animation_data.drivers.remove(d)
        k.animation_data.action = None
    ob.active_shape_key_index = 0
    for kb in list(k.key_blocks):
        if kb.name not in FACE_KEYS:
            ob.shape_key_remove(kb)
    for kb in k.key_blocks:
        kb.value = 0.0


def select_only(objs):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.hide_set(False)
        o.hide_viewport = False
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]


def export(path, objs, anims):
    select_only(objs)
    t0 = time.time()
    bpy.ops.export_scene.gltf(
        filepath=path, use_selection=True, export_def_bones=True, export_skins=True, export_morph=True,
        export_morph_normal=False, export_morph_animation=False, export_apply=False, export_animations=anims,
        export_animation_mode="ACTIONS", export_frame_range=False, export_force_sampling=True, export_frame_step=1,
        export_optimize_animation_size=True, export_optimize_animation_keep_anim_armature=True,
        export_anim_single_armature=True, export_reset_pose_bones=True,
        export_image_format="NONE", export_texcoords=True, export_normals=True, export_yup=True,
        export_extras=False, export_cameras=False, export_lights=False)
    print("wrote", path, "in %.0f s" % (time.time() - t0), flush=True)


bpy.context.scene.render.fps = 10
for rig_name in ("MaleRig", "FemaleRig"):
    rig = bpy.data.objects[rig_name]
    all_bodies = [b for b in BODIES if bpy.data.objects.get(b) and bpy.data.objects[b].parent == rig]
    bodies = [b for b in all_bodies if ONLY is None or BODIES[b] in ONLY]
    if not all_bodies:
        continue
    for b in all_bodies:
        strip_keys(bpy.data.objects[b])
    if rig.animation_data is None:
        rig.animation_data_create()
    ad = rig.animation_data
    # Rest pose for the bodies: no action, no NLA.
    for t in list(ad.nla_tracks):
        ad.nla_tracks.remove(t)
    ad.action = None
    for pb in rig.pose.bones:
        pb.location = (0, 0, 0)
        pb.rotation_quaternion = (1, 0, 0, 0)
        pb.rotation_euler = (0, 0, 0)
        pb.scale = (1, 1, 1)
    if ONLY is None or "anims" in ONLY:
        acts = sorted(a.name for a in bpy.data.actions if bone_action(a))
        for n in acts:
            tr = ad.nla_tracks.new()
            tr.name = n
            st = tr.strips.new(n, int(bpy.data.actions[n].frame_range[0]), bpy.data.actions[n])
            tr.mute = True
        export(os.path.join(OUT, "anims_%s.glb" % ("male" if rig_name == "MaleRig" else "female")),
               [rig, bpy.data.objects[all_bodies[0]]], True)
        for t in list(ad.nla_tracks):
            ad.nla_tracks.remove(t)
    for b in bodies:
        export(os.path.join(OUT, BODIES[b] + ".glb"), [rig, bpy.data.objects[b]], False)
