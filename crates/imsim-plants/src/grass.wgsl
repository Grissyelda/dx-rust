// Grass blades grown on the GPU (imsim-plants grass.rs). The mesh is only blade slots: POSITION = (slot, corner, 0),
// seven corners a blade (left / right at three steps up it, then the tip). A slot's tile comes from the packed data
// texture (three texels a tile: corner x z + height + mask / kind; slope x z + light-map uv; light-map gradients);
// the blade's place in the tile, height, turn and lean come from a hash of the tile's cell in the world and the
// slot's number in it, so a blade stands in the same spot every frame. Blades thin with distance (all within
// params.eye.w, then falling with the square of the distance, none at the far edge), sway in travelling wind gusts,
// lean away from what stands on them and lie flat under it. Lit by the ground's light map at the root, darker toward
// the root where the blades around shut out the light.
//
// Bindings (material group): 0 data (RGBA32F, unfiltered), 1 / 2 the light map + sampler, 3 params, 4 pushes.

#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
}
#ifdef DISTANCE_FOG
#import bevy_pbr::{mesh_view_bindings as view_bindings, pbr_functions}
#endif

struct Params {
    // x tiles packed, y slots per tile, z wind time, w level
    a: vec4<f32>,
    // wind direction x z, strength, gusts
    wind: vec4<f32>,
    // eye xyz, full-density reach (m)
    eye: vec4<f32>,
    // light gain, pushes, height scale, far edge (m)
    b: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var data: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lm_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lm_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> params: Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<uniform> pushes: array<vec4<f32>, 16>;

const TILE: f32 = 0.5;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) lm_uv: vec2<f32>,
    @location(1) col: vec3<f32>,
    @location(2) world: vec3<f32>,
};

fn texel(i: u32) -> vec4<f32> {
    let w = textureDimensions(data).x;
    return textureLoad(data, vec2<i32>(i32(i % w), i32(i / w)), 0);
}

fn hash(x0: u32) -> u32 {
    var x = x0;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn rnd(s: ptr<function, u32>) -> f32 {
    *s = hash(*s);
    return f32(*s & 0xffffffu) / 16777216.0;
}

fn hidden() -> Out {
    var o: Out;
    o.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    o.lm_uv = vec2<f32>(0.0);
    o.col = vec3<f32>(0.0);
    o.world = vec3<f32>(0.0);
    return o;
}

// Travelling gusts along the wind: a long wave and a shorter cross-wise one, squared so the gusts come as pulses.
fn wind_at(p: vec2<f32>, t: f32) -> vec2<f32> {
    let d = params.wind.xy;
    let along = dot(p, d);
    let across = -p.x * d.y + p.y * d.x;
    let p1 = (along - 3.0 * t) / 9.0;
    let p2 = (along * 0.61 - 2.5 * t) / 3.3 + across / 8.0;
    var g = 0.5 + 0.5 * sin(6.2831853 * p1) * (0.6 + 0.4 * sin(6.2831853 * (p2 + 0.27)));
    g = g * g;
    return d * (params.wind.z + params.wind.w * g);
}

fn kind_colour(kind: u32) -> vec3<f32> {
    switch kind {
        case 1u: { return vec3<f32>(0.40, 0.50, 0.20); }
        case 2u: { return vec3<f32>(0.19, 0.28, 0.11); }
        case 3u: { return vec3<f32>(0.34, 0.40, 0.19); }
        default: { return vec3<f32>(0.30, 0.42, 0.16); }
    }
}

@vertex
fn vertex(v: Vertex) -> Out {
    let slot = u32(v.position.x + 0.5);
    let corner = u32(v.position.y + 0.5);
    let per = max(u32(params.a.y), 1u);
    let tile = slot / per;
    let k = slot % per;
    if (tile >= u32(params.a.x)) {
        return hidden();
    }
    let t0 = texel(tile * 3u);
    let t1 = texel(tile * 3u + 1u);
    let t2 = texel(tile * 3u + 2u);
    let packed = u32(t0.w);
    let mask = packed & 0xffffu;
    let kind = packed >> 16u;
    let cx = i32(round(t0.x / TILE));
    let cz = i32(round(t0.y / TILE));
    let cell = hash(u32(cx) * 73856093u ^ u32(cz) * 19349663u ^ 0x5bd1e995u);
    // The far layer (params.a.z = where it fades in, m) draws a few wide clumps per tile from seeds of its own.
    let far_mode = params.a.z > 0.0;
    var s = hash(cell ^ ((k + 1u + select(0u, 977u, far_mode)) * 0x9E3779B9u));
    let f = vec2<f32>(rnd(&s), rnd(&s));
    let sub = vec2<u32>(min(u32(f.x * 4.0), 3u), min(u32(f.y * 4.0), 3u));
    if (((mask >> (sub.y * 4u + sub.x)) & 1u) == 0u) {
        return hidden();
    }
    let off = f * TILE;
    let root = vec3<f32>(t0.x + off.x, t0.z + t1.x * off.x + t1.y * off.y, t0.y + off.y);
    // Thinning: every slot near, then falling with the distance squared, none at the far edge; a blade just kept
    // is small and grows as the eye comes nearer, so none pops up at full height.
    // The view's own eye (not a uniform: the material then never changes while the player only moves).
    let d = distance(root, view.world_position);
    let near = params.eye.w;
    let far = params.b.w;
    var keep = 1.0;
    if (far_mode) {
        // Comes in as the near blades thin out (their own fade ends at their far edge), gone at its own edge.
        keep = smoothstep(params.a.z, params.a.z + 7.0, d) * clamp((far - d) / (far * 0.3), 0.0, 1.0);
    } else {
        if (d > near) {
            let r = near / d;
            keep = r * r;
        }
        keep = keep * clamp((far - d) / (far * 0.25), 0.0, 1.0);
    }
    let kept = keep - (f32(k) + 0.5) / f32(per);
    if (kept <= 0.0) {
        return hidden();
    }
    let grow = clamp(kept * 8.0, 0.0, 1.0);
    var h = mix(0.12, 0.34, rnd(&s)) * params.b.z * grow;
    if (kind == 3u) {
        h = h * 1.3;
    }
    let turn = rnd(&s) * 6.2831853;
    let face = vec2<f32>(cos(turn), sin(turn));
    let along = vec2<f32>(-face.y, face.x);
    var tip = along * ((0.1 + 0.3 * rnd(&s)) * h);
    let pace = 4.0 + 3.0 * rnd(&s);
    let phase = 6.2831853 * rnd(&s);
    let t = globals.time;
    tip = tip + wind_at(root.xz, t) * ((1.0 + 0.12 * sin(t * pace + phase)) * h);
    // What stands on it: blades lean away, and lie flat underneath.
    var flat = 0.0;
    let n = u32(params.b.y);
    for (var i = 0u; i < 16u; i = i + 1u) {
        if (i >= n) {
            break;
        }
        let q = pushes[i];
        let dy = root.y - q.y;
        if (dy < -0.5 || dy > 0.6) {
            continue;
        }
        let dd = root.xz - q.xz;
        let l = length(dd);
        let r = q.w + 0.12;
        if (l >= r) {
            continue;
        }
        let ff = 1.0 - l / r;
        var away = along;
        if (l > 0.001) {
            away = dd / l;
        }
        tip = tip + away * (ff * h * 1.5);
        flat = max(flat, ff);
    }
    let lie = h * (1.0 - 0.7 * flat);
    // The corner: up the blade, bent toward its tip, narrowing to the point.
    let up3 = min(corner / 2u, 3u);
    var steps = array<f32, 4>(0.0, 0.4, 0.75, 1.0);
    let tt = steps[up3];
    var side = 0.0;
    if (up3 < 3u) {
        side = select(0.5, -0.5, (corner & 1u) == 0u);
    }
    let wide = select(0.036, 0.1, far_mode) * (0.8 + 0.4 * rnd(&s)) * pow(clamp(1.0 - tt, 0.0, 1.0), 0.7);
    let bend = dot(tip, tip) / max(lie * lie, 1e-4);
    let up = lie * tt * sqrt(clamp(1.0 - 0.5 * bend * tt * tt, 0.0, 1.0));
    let p = root + vec3<f32>(face.x * side * wide + tip.x * tt * tt, up, face.y * side * wide + tip.y * tt * tt);

    var o: Out;
    o.clip = position_world_to_clip(p);
    o.world = p;
    o.lm_uv = t1.zw + t2.xy * (off.x - TILE * 0.5) + t2.zw * (off.y - TILE * 0.5);
    // Colour: the kind's green, by clump fresher, darker or yellowed from the winter, each blade a little lighter
    // or darker, lighter to the tip; the shade along it where its neighbours shut out the light from above.
    var cs = cell ^ 0x2545F491u;
    let pick = rnd(&cs);
    var c = kind_colour(kind);
    if (pick < 0.12) {
        c = mix(c, vec3<f32>(0.52, 0.47, 0.22), 0.55);
    } else if (pick < 0.32) {
        c = c * 0.75;
    } else if (pick < 0.55) {
        c = c * vec3<f32>(1.04, 1.1, 0.95);
    }
    let bright = 0.88 + 0.24 * rnd(&s);
    c = mix(c, c * 1.25 + vec3<f32>(0.02, 0.025, 0.0), tt);
    let shade = mix(0.42, 1.0, tt);
    // To linear (the colours above are as seen).
    o.col = pow(c * bright, vec3<f32>(2.2)) * shade;
    return o;
}

@fragment
fn fragment(i: Out) -> @location(0) vec4<f32> {
    let light = textureSample(lm_tex, lm_smp, i.lm_uv).rgb * params.b.x;
    var c = vec4<f32>(i.col * light, 1.0);
#ifdef DISTANCE_FOG
    c = pbr_functions::apply_fog(view_bindings::fog, c, i.world, view.world_position, i.clip.xy);
#endif
    return c;
}
