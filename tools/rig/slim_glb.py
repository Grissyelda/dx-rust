"""Slim a baked animation glb: drop the channels (per bone and path) that hold the bone's rest value in every clip
(scale everywhere, translation on most bones), and merge the clips' time inputs. Usage: slim_glb.py in.glb out.glb"""
import json, struct, sys
import numpy as np

src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
jl = struct.unpack('<I', b[12:16])[0]
j = json.loads(b[20:20 + jl])
bin_off = 20 + jl + 8
binb = b[bin_off:]
acc, bvs = j['accessors'], j['bufferViews']
NC = {'SCALAR': 1, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16, 'VEC2': 2}

def read(ai):
    a = acc[ai]; v = bvs[a['bufferView']]
    n = a['count'] * NC[a['type']]
    off = v.get('byteOffset', 0) + a.get('byteOffset', 0)
    assert a['componentType'] == 5126
    return np.frombuffer(binb, dtype='<f4', count=n, offset=off).reshape(a['count'], NC[a['type']])

nodes = j['nodes']
def rest(ni, path):
    n = nodes[ni]
    return np.array(n.get({'translation': 'translation', 'rotation': 'rotation', 'scale': 'scale'}[path],
                    {'translation': [0, 0, 0], 'rotation': [0, 0, 0, 1], 'scale': [1, 1, 1]}[path]), dtype=np.float32)

# Which (node, path) ever leave their rest value?
moving = set()
for a in j['animations']:
    for ch in a['channels']:
        t = ch['target']; s = a['samplers'][ch['sampler']]
        out = read(s['output'])
        r = rest(t['node'], t['path'])
        if t['path'] == 'rotation':
            d = np.minimum(np.abs(out - r).max(axis=1), np.abs(out + r).max(axis=1)).max()
        else:
            d = np.abs(out - r).max()
        if d > 1e-4:
            moving.add((t['node'], t['path']))

# Rebuild: new binary with only the kept channels.
new_bin = bytearray(); new_acc = []; new_bv = []
def add(arr, typ, minmax=False):
    arr = np.ascontiguousarray(arr, dtype='<f4')
    while len(new_bin) % 4: new_bin.append(0)
    off = len(new_bin); new_bin.extend(arr.tobytes())
    new_bv.append({'buffer': 0, 'byteOffset': off, 'byteLength': arr.nbytes})
    a = {'bufferView': len(new_bv) - 1, 'componentType': 5126, 'count': arr.shape[0], 'type': typ}
    if minmax:
        a['min'] = [float(x) for x in arr.min(axis=0)]; a['max'] = [float(x) for x in arr.max(axis=0)]
    new_acc.append(a)
    return len(new_acc) - 1

kept = 0; dropped = 0
for a in j['animations']:
    chans, samps = [], []
    time_cache = {}
    for ch in a['channels']:
        t = ch['target']; s = a['samplers'][ch['sampler']]
        if (t['node'], t['path']) not in moving:
            dropped += 1; continue
        inp = read(s['input'])
        key = inp.tobytes()
        if key not in time_cache:
            time_cache[key] = add(inp, 'SCALAR', True)
        out = read(s['output'])
        oi = add(out, 'VEC4' if t['path'] == 'rotation' else 'VEC3')
        samps.append({'input': time_cache[key], 'output': oi, 'interpolation': s.get('interpolation', 'LINEAR')})
        chans.append({'sampler': len(samps) - 1, 'target': t})
        kept += 1
    a['channels'] = chans; a['samplers'] = samps

# Keep non-animation accessors (skin, mesh) too: copy them over.
remap = {}
for i, a in enumerate(acc):
    pass
users = set()
for m in j.get('meshes', []):
    for p in m['primitives']:
        for v in p['attributes'].values(): users.add(v)
        if 'indices' in p: users.add(p['indices'])
        for tg in p.get('targets', []):
            for v in tg.values(): users.add(v)
for sk in j.get('skins', []):
    if 'inverseBindMatrices' in sk: users.add(sk['inverseBindMatrices'])
for ai in sorted(users):
    a = acc[ai]
    if 'bufferView' not in a:  # zero-filled accessor: no data to copy
        new_acc.append(dict(a)); remap[ai] = len(new_acc) - 1; continue
    v = bvs[a['bufferView']]
    raw = binb[v.get('byteOffset', 0):v.get('byteOffset', 0) + v['byteLength']]
    while len(new_bin) % 4: new_bin.append(0)
    off = len(new_bin); new_bin.extend(raw)
    nv = {k: v[k] for k in v if k not in ('byteOffset', 'buffer')}; nv['buffer'] = 0; nv['byteOffset'] = off
    new_bv.append(nv)
    na = dict(a); na['bufferView'] = len(new_bv) - 1
    new_acc.append(na); remap[ai] = len(new_acc) - 1
for m in j.get('meshes', []):
    for p in m['primitives']:
        p['attributes'] = {k: remap[v] for k, v in p['attributes'].items()}
        if 'indices' in p: p['indices'] = remap[p['indices']]
        if 'targets' in p: p['targets'] = [{k: remap[v] for k, v in tg.items()} for tg in p['targets']]
for sk in j.get('skins', []):
    if 'inverseBindMatrices' in sk: sk['inverseBindMatrices'] = remap[sk['inverseBindMatrices']]
j['accessors'] = new_acc; j['bufferViews'] = new_bv
while len(new_bin) % 4: new_bin.append(0)
j['buffers'] = [{'byteLength': len(new_bin)}]
js = json.dumps(j, separators=(',', ':')).encode()
while len(js) % 4: js += b' '
out = struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(new_bin))
out += struct.pack('<II', len(js), 0x4E4F534A) + js + struct.pack('<II', len(new_bin), 0x004E4942) + bytes(new_bin)
open(dst, 'wb').write(out)
print(f'{src}: kept {kept} channels, dropped {dropped}; moving (node, path) pairs {len(moving)}; {len(b)/1e6:.1f} MB -> {len(out)/1e6:.1f} MB')
