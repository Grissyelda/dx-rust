"""Pack a baked body glb (bake_humans.py: <DXMesh>.glb, the Machine God team's rebuilt body on their rig) into the
small binary the game embeds. Usage: pack_body.py GM_Trench.glb GM_Trench.dxbody

Format (little endian): b"DXBD", u32 version 1,
  u16 joints; per joint: u8 name length + name, inverse bind matrix (16 f32, column major)
  u8 morphs; per morph: u8 name length + name
  u8 sections; per section: u8 name length + material name ("NNN_STYLE_..."), u32 verts, positions (3 f32), normals
    (3 f32), uvs (2 f32), joints (4 u8), weights (4 u16 unorm), u32 indices, indices (u16),
    then per morph: u8 flag (1 = data follows), position deltas (3 f32 per vertex)
Positions are glTF (Y up, metres as baked), relative to the rig node, like the joints' bind poses."""
import json, struct, sys
import numpy as np

src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20 + jl])
binb = b[20 + jl + 8:]
acc, bvs, nodes = j['accessors'], j['bufferViews'], j['nodes']
NC = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}
DT = {5126: '<f4', 5125: '<u4', 5123: '<u2', 5121: 'u1', 5122: '<i2', 5120: 'i1'}

def read(ai):
    a = acc[ai]
    n = NC[a['type']]
    if 'bufferView' not in a:
        return np.zeros((a['count'], n), dtype=DT[a['componentType']])
    v = bvs[a['bufferView']]
    off = v.get('byteOffset', 0) + a.get('byteOffset', 0)
    dt = np.dtype(DT[a['componentType']])
    stride = v.get('byteStride', 0)
    if stride and stride != dt.itemsize * n:
        raw = np.frombuffer(binb, dtype='u1', count=stride * a['count'], offset=off).reshape(a['count'], stride)
        arr = raw[:, :dt.itemsize * n].copy().view(dt).reshape(a['count'], n)
    else:
        arr = np.frombuffer(binb, dtype=dt, count=a['count'] * n, offset=off).reshape(a['count'], n)
    if a.get('normalized') and dt.kind == 'u':
        arr = arr.astype(np.float32) / np.iinfo(dt).max
    return arr

def s8(t):
    e = t.encode(); return struct.pack('<B', len(e)) + e

mesh_node = next(n for n in nodes if 'mesh' in n and 'skin' in n)
skin = j['skins'][mesh_node['skin']]
mesh = j['meshes'][mesh_node['mesh']]
ibm = read(skin['inverseBindMatrices'])
out = bytearray(b'DXBD') + struct.pack('<I', 1)
out += struct.pack('<H', len(skin['joints']))
for k, ji in enumerate(skin['joints']):
    out += s8(nodes[ji]['name']) + ibm[k].astype('<f4').tobytes()
morphs = mesh.get('extras', {}).get('targetNames', [])
out += struct.pack('<B', len(morphs))
for m in morphs:
    out += s8(m)
prims = mesh['primitives']
out += struct.pack('<B', len(prims))
for p in prims:
    at = p['attributes']
    name = j['materials'][p['material']]['name'] if 'material' in p else 'none'
    pos = read(at['POSITION']).astype('<f4'); nrm = read(at['NORMAL']).astype('<f4')
    uv = read(at['TEXCOORD_0']).astype('<f4')
    jn = read(at['JOINTS_0']).astype('u1'); wt = read(at['WEIGHTS_0']).astype(np.float32)
    wt = wt / np.maximum(wt.sum(axis=1, keepdims=True), 1e-6)
    wq = np.round(wt * 65535).astype('<u2')
    idx = read(p['indices'])[:, 0].astype('<u2')
    out += s8(name) + struct.pack('<I', len(pos)) + pos.tobytes() + nrm.tobytes() + uv.tobytes() + jn.tobytes() + wq.tobytes()
    out += struct.pack('<I', len(idx)) + idx.tobytes()
    tg = p.get('targets', [])
    for k in range(len(morphs)):
        d = read(tg[k]['POSITION']).astype('<f4') if k < len(tg) and 'POSITION' in tg[k] else None
        if d is None or not np.any(np.abs(d) > 1e-6):
            out += b'\0'
        else:
            out += b'\1' + d.tobytes()
open(dst, 'wb').write(out)
print(f'{src}: {len(skin["joints"])} joints, {len(morphs)} morphs, {len(prims)} sections, '
      f'{sum(read(p["attributes"]["POSITION"]).shape[0] for p in prims)} verts -> {len(out)/1e3:.0f} KB')
