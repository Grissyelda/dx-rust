"""Pack the baked rig animations (anims_<male|female>.glb from bake_humans.py) into a small binary the game embeds.
Channels that never leave the bone's rest value are dropped; rotations are stored as 4 x i16 (snorm).
Usage: pack_anims.py anims_male.glb anims_male.dxanim

Format (little endian): b"DXAN", u32 version 1,
  u16 nodes; per node: u16 parent (0xFFFF = root), u8 name length + name, rest T (3 f32), R (4 f32 xyzw), S (3 f32)
  u16 clips; per clip: u8 name length + name, f32 duration, u8 time sets (u16 n, n f32 each),
    u16 channels; per channel: u16 node, u8 path (0 T, 1 R, 2 S), u8 time set, values (T/S: n x 3 f32, R: n x 4 i16)
Node 0 order is glTF's; the root is the armature object (MaleRig / FemaleRig), named like the body glbs' rig node."""
import json, struct, sys
import numpy as np

src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20 + jl])
binb = b[20 + jl + 8:]
acc, bvs, nodes = j['accessors'], j['bufferViews'], j['nodes']
NC = {'SCALAR': 1, 'VEC3': 3, 'VEC4': 4}

def read(ai):
    a = acc[ai]; v = bvs[a['bufferView']]
    off = v.get('byteOffset', 0) + a.get('byteOffset', 0)
    return np.frombuffer(binb, dtype='<f4', count=a['count'] * NC[a['type']], offset=off).reshape(a['count'], NC[a['type']])

# Keep the armature's node tree only (no meshes).
root = None
for i, n in enumerate(nodes):
    if 'children' in n and 'mesh' not in n and any('DEF' in nodes[c].get('name', '') for c in n['children']):
        root = i
parent = {}
for i, n in enumerate(nodes):
    for c in n.get('children', []):
        parent[c] = i
order = []
def walk(i):
    order.append(i)
    for c in nodes[i].get('children', []):
        if 'mesh' not in nodes[c]:
            walk(c)
walk(root)
index = {n: k for k, n in enumerate(order)}

REST = {'translation': [0, 0, 0], 'rotation': [0, 0, 0, 1], 'scale': [1, 1, 1]}
def rest(ni, path):
    return np.array(nodes[ni].get(path, REST[path]), dtype=np.float32)

out = bytearray(b'DXAN') + struct.pack('<I', 1)
def s8(t):
    e = t.encode(); return struct.pack('<B', len(e)) + e
out += struct.pack('<H', len(order))
for ni in order:
    p = parent.get(ni)
    out += struct.pack('<H', index[p] if p in index else 0xFFFF) + s8(nodes[ni].get('name', f'node{ni}'))
    out += struct.pack('<3f', *rest(ni, 'translation')) + struct.pack('<4f', *rest(ni, 'rotation')) + struct.pack('<3f', *rest(ni, 'scale'))

PATH = {'translation': 0, 'rotation': 1, 'scale': 2}
anims = j['animations']
out += struct.pack('<H', len(anims))
kept = dropped = 0
for a in anims:
    times = []; chans = []
    for ch in a['channels']:
        t = ch['target']
        if t['node'] not in index:
            continue
        s = a['samplers'][ch['sampler']]
        val = read(s['output']); r = rest(t['node'], t['path'])
        if t['path'] == 'rotation':
            d = np.minimum(np.abs(val - r).max(axis=1), np.abs(val + r).max(axis=1)).max()
        else:
            d = np.abs(val - r).max()
        if d <= 1e-4:
            dropped += 1; continue
        tm = read(s['input'])[:, 0]
        key = tm.tobytes()
        ks = [k for k, x in enumerate(times) if x.tobytes() == key]
        if ks:
            ts = ks[0]
        else:
            times.append(tm); ts = len(times) - 1
        chans.append((index[t['node']], PATH[t['path']], ts, val))
        kept += 1
    dur = max((float(t[-1]) for t in times), default=0.0)
    out += s8(a['name']) + struct.pack('<f', dur) + struct.pack('<B', len(times))
    for t in times:
        out += struct.pack('<H', len(t)) + t.astype('<f4').tobytes()
    out += struct.pack('<H', len(chans))
    for node, path, ts, val in chans:
        out += struct.pack('<HBB', node, path, ts)
        if path == 1:
            q = val / np.linalg.norm(val, axis=1, keepdims=True)
            out += np.round(np.clip(q, -1, 1) * 32767).astype('<i2').tobytes()
        else:
            out += val.astype('<f4').tobytes()
open(dst, 'wb').write(out)
print(f'{src}: {len(order)} nodes, {len(anims)} clips, {kept} channels kept, {dropped} dropped -> {len(out)/1e6:.2f} MB')
