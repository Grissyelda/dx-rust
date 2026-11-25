// Skeletal characters (skeletal.rs): lit per pixel by up to sixteen Unreal lights (the rest as an even fill), the way dx-level's light_part
// lights a mesh's vertices: colour x (1 - d / r) x max(N.L, 0) (|N.L| on two-sided sections), summed, x LIGHT_GAIN
// x ScaleGlow, clamped and taken to linear (Unreal adds up light in gamma).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}
#import dxview::shadow_spots as spot_shadows
#import bevy_pbr::mesh_view_bindings::{globals, view}

struct ActorLights {
    pos: array<vec4<f32>, 16>,
    col: array<vec4<f32>, 16>,
    misc: vec4<f32>,
    wet: vec4<f32>,
    amb: vec4<f32>,
    wpos: array<vec4<f32>, 12>,
    winfo: array<vec4<f32>, 12>,
    wmisc: vec4<f32>,
}

// ---- wounds (wounds.rs): drawn on the skin where they are in the body's rest space, so they stay on it in any pose

fn w_hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn w_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(w_hash(i), w_hash(i + vec3(1.0, 0.0, 0.0)), u.x), mix(w_hash(i + vec3(0.0, 1.0, 0.0)), w_hash(i + vec3(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(w_hash(i + vec3(0.0, 0.0, 1.0)), w_hash(i + vec3(1.0, 0.0, 1.0)), u.x), mix(w_hash(i + vec3(0.0, 1.0, 1.0)), w_hash(i + vec3(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

struct WoundLook {
    col: vec3<f32>,
    a: f32,
    // How wet the blood here is (a glossy highlight), and light of its own (embers of a burn).
    wet: f32,
    glow: vec3<f32>,
}

fn blood_col(age: f32, thick: f32) -> vec3<f32> {
    // Fresh blood darkens as it dries; thick blood is darker.
    let dry = smoothstep(15.0, 120.0, age);
    let fresh = mix(vec3<f32>(0.42, 0.015, 0.02), vec3<f32>(0.2, 0.005, 0.008), dry);
    return mix(fresh, fresh * 0.35, thick);
}

fn wounds_at(rest: vec3<f32>, now: f32) -> WoundLook {
    var o: WoundLook;
    o.col = vec3<f32>(0.0);
    o.a = 0.0;
    o.wet = 0.0;
    o.glow = vec3<f32>(0.0);
    let n = u32(lights.wmisc.x);
    let dead = lights.wmisc.y;
    for (var i = 0u; i < 12u; i = i + 1u) {
        if (i >= n) {
            break;
        }
        let w = lights.wpos[i];
        let info = lights.winfo[i];
        var age = now - info.x;
        if (age < 0.0) {
            age = age + 3600.0;
        }
        let kind = u32(info.y + 0.5);
        let r = w.w;
        let seed = info.w;
        let rel = rest - w.xyz;
        let dist = length(rel);
        // Too far for any part of it (the runs reach down, the stain spreads).
        if (dist > r * 5.0 + info.z + 0.02) {
            continue;
        }
        let nz = w_noise(rest * 140.0 + vec3<f32>(seed * 17.0));
        let nz2 = w_noise(rest * 420.0 + vec3<f32>(seed * 3.1));
        let d = dist / r * (0.8 + 0.4 * nz);
        // The stain soaking out round it: faster on the dead (nothing to stop it), slower on the living.
        let soak_t = select(30.0, 9.0, dead > 0.0 && now > dead);
        let soak = 1.0 - exp(-age / soak_t);
        var col = vec3<f32>(0.0);
        var a = 0.0;
        var wet = 0.0;
        if (kind == 2u) {
            // Burn: charred skin, blistered red round it, embers at the edge for the first moments.
            let char_a = 1.0 - smoothstep(0.8, 1.15, d);
            let blister = (1.0 - smoothstep(1.1, 1.9, d)) * 0.55;
            col = mix(vec3<f32>(0.32, 0.07, 0.04), vec3<f32>(0.025, 0.018, 0.012) * (0.6 + 0.8 * nz2), char_a);
            a = max(char_a, blister);
            let ember = (1.0 - smoothstep(0.0, 0.15, abs(d - 0.85))) * max(0.0, 1.0 - age / 2.5) * nz2;
            o.glow = o.glow + vec3<f32>(1.0, 0.35, 0.06) * ember * 2.0;
        } else if (kind == 5u) {
            // A bruise (a blow): dark, a little purple, soft-edged.
            a = (1.0 - smoothstep(0.3, 1.0, dist / r)) * 0.55 * min(1.0, age / 1.5);
            col = vec3<f32>(0.24, 0.08, 0.12);
        } else {
            // An opening: dark inside, torn red flesh round it, a stain soaking out.
            let hole = 1.0 - smoothstep(0.28, 0.48, d);
            let flesh = 1.0 - smoothstep(0.8, 1.05, d);
            // The stain soaks out: further round an exit wound, least round a stump (the cut is already big).
            var halo_r = 1.4 + soak * select(1.5, 2.2, kind == 1u);
            if (kind == 4u) {
                halo_r = 1.25 + soak * 0.6;
            }
            let halo = (1.0 - smoothstep(0.45, 1.0, d / halo_r)) * (0.38 + 0.25 * nz);
            col = blood_col(age, 0.4 + 0.3 * nz2);
            a = halo;
            wet = halo * 0.6;
            let torn = vec3<f32>(0.5, 0.07, 0.06) * (0.55 + 0.6 * nz2);
            col = mix(col, torn, flesh);
            a = max(a, flesh);
            col = mix(col, vec3<f32>(0.035, 0.0, 0.0), hole);
            wet = max(wet, flesh);
            if (kind == 4u) {
                // A severed limb's stump: bone in the middle of it.
                let bone = 1.0 - smoothstep(0.12, 0.2, d);
                col = mix(col, vec3<f32>(0.8, 0.72, 0.6) * (0.75 + 0.3 * nz2), bone);
            }
            // Blood runs down from it (rest space: +y is up): 1-3 rivulets lengthening for a few seconds, a bead at
            // each tip.
            let below = -rel.y;
            if (below > 0.0 && info.z > 0.0) {
                let runs = 1u + u32(fract(seed * 7.13) * 2.99);
                for (var j = 0u; j < 3u; j = j + 1u) {
                    if (j >= runs) {
                        break;
                    }
                    let sj = fract(seed * 13.7 + f32(j) * 0.618);
                    let len = info.z * (0.45 + 0.55 * sj) * (1.0 - exp(-age / (2.0 + 3.0 * sj)));
                    if (below > len + r) {
                        continue;
                    }
                    let off = vec2<f32>(sj - 0.5, fract(sj * 5.3) - 0.5) * r * 1.4;
                    let wig = vec2<f32>(sin(below * 60.0 + seed * 9.0 + f32(j)), cos(below * 47.0 + seed * 5.0)) * r * 0.18;
                    let lat = length(rel.xz - off - wig);
                    let k = clamp(below / max(len, 1e-4), 0.0, 1.0);
                    var width = r * (0.34 - 0.16 * k);
                    // The bead at the tip.
                    width = width + r * 0.22 * (1.0 - smoothstep(0.0, r * 0.6, abs(below - len)));
                    let ra = (1.0 - smoothstep(width * 0.55, width, lat)) * (1.0 - smoothstep(len, len + r * 0.3, below));
                    if (ra > 0.0) {
                        col = mix(col, blood_col(age, 0.25), ra * (1.0 - a * 0.5));
                        a = max(a, ra * 0.95);
                        wet = max(wet, ra);
                    }
                }
            }
        }
        if (a > 0.0) {
            o.col = select(mix(o.col, col, a), col, o.a <= 0.0);
            o.a = max(o.a, a);
            o.wet = max(o.wet, wet * (1.0 - smoothstep(25.0, 150.0, age)));
        }
    }
    return o;
}

#ifdef BINDLESS
// Bindless (skeletal.rs): the material's slot picks its lights out of a shared array, so every body's sections are
// drawn in a few GPU multi-draw calls.
#import bevy_pbr::mesh_bindings::mesh
struct ActorLitIndices {
    lights: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<storage> actor_indices: array<ActorLitIndices>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var<storage> actor_lights: array<ActorLights>;
var<private> lights: ActorLights;
fn lights_fetch(instance_index: u32) {
    let slot = mesh[instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    lights = actor_lights[actor_indices[slot].lights];
}
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> lights: ActorLights;
fn lights_fetch(instance_index: u32) {}
#endif

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    lights_fetch(in.instance_index);
    // The first-person body: nothing above the cut (head, collar, shoulders round the eye).
    if (lights.wet.w > 0.5 && in.world_position.y > lights.wet.z) {
        discard;
    }
    // Blinking (skeletal.rs lid_uvs): the lids' picture slides to the crease's as they close.
    var vin = in;
#ifdef VERTEX_COLORS
    // The vertex colour is the rest position (wounds), not a tint.
    let rest = in.color.xyz;
    vin.color = vec4<f32>(1.0);
#endif
#ifdef VERTEX_UVS_B
    if (lights.wet.x > 0.0) {
        vin.uv = mix(in.uv, in.uv_b, lights.wet.x);
    }
#endif
    var pbr_input = pbr_input_from_standard_material(vin, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var wl: WoundLook;
    wl.col = vec3<f32>(0.0);
    wl.a = 0.0;
    wl.wet = 0.0;
    wl.glow = vec3<f32>(0.0);
#ifdef VERTEX_COLORS
    if (lights.wmisc.x > 0.5 && lights.misc.z < 0.5) {
        wl = wounds_at(rest, globals.time);
        pbr_input.material.base_color = vec4<f32>(mix(pbr_input.material.base_color.rgb, wl.col, wl.a), pbr_input.material.base_color.a);
    }
#endif
    var out: FragmentOutput;
    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let p = in.world_position.xyz;
    var c = vec3<f32>(0.0);
    var spec = vec3<f32>(0.0);
    let to_eye = normalize(view.world_position.xyz - p);
    let count = u32(lights.misc.x);
    // Real-time shadows (shadowlight.rs): a light whose shadow map says this pixel is blocked (another actor, or
    // the body's own arm) gives 85% less of its light here. Compiled in only where shadow lights can exist
    // (ACTOR_SPOT_SHADOWS, shadowlight.rs `ActorShadowKey`).
#ifdef ACTOR_SPOT_SHADOWS
    let nspots = spot_shadows::gather(in.world_position, n, in.position, false);
#else
    let nspots = 0u;
#endif
    for (var i = 0u; i < 16u; i = i + 1u) {
        if (i >= count) {
            break;
        }
        let to = lights.pos[i].xyz - p;
        let d = length(to);
        let r = lights.pos[i].w;
        if (d >= r) {
            continue;
        }
        var ndl = dot(n, to / max(d, 1e-4));
        if (lights.misc.w > 0.5) {
            ndl = abs(ndl);
        }
        var shade = 1.0;
        for (var k = 0u; k < nspots; k = k + 1u) {
            if (distance(spot_shadows::spots[k].pos, lights.pos[i].xyz) < 0.05) {
                shade = mix(1.0, spot_shadows::spots[k].shadow, 0.85);
            }
        }
        // Unreal's mesh falloff (dx_level::lighting::mesh_falloff) x MESH_GLOW 1.5.
        let v = max(d / r, 1e-3);
        let fall = min((1.0 + 2.0 * v * v * v - 3.0 * v * v) / v, 1.0);
        c = c + lights.col[i].rgb * fall * 1.5 * max(ndl, 0.0) * shade;
        if (wl.wet > 0.0) {
            // Wet blood shines.
            let h = normalize(to / max(d, 1e-4) + to_eye);
            spec = spec + lights.col[i].rgb * fall * pow(max(dot(n, h), 0.0), 70.0) * shade;
        }
    }
    c = c + lights.amb.rgb;
    var lin = pow(clamp(c * lights.misc.y, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    if (lights.misc.z > 0.5) {
        lin = vec3<f32>(1.0);
    }
    var shine = vec3<f32>(0.0);
    if (wl.wet > 0.0) {
        shine = pow(clamp(spec * lights.misc.y * 0.8, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2)) * wl.wet;
    }
    out.color = vec4<f32>(pbr_input.material.base_color.rgb * lin + shine + wl.glow, pbr_input.material.base_color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
