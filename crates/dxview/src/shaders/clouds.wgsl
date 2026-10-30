// Volumetric night clouds (clouds.rs): the sheet is the bottom of a cloud slab; each pixel marches its view ray up
// through the slab in a wind-blown tiling 3D noise, lit by the moon (with self-shadowing) and the city below.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
}

struct CloudParams {
    slab: vec4<f32>,
    wind: vec4<f32>,
    glow: vec4<f32>,
    moon: vec4<f32>,
    fade: vec4<f32>,
    shape: vec4<f32>,
    flash: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: CloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var noise_tex: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var noise_smp: sampler;

fn height_frac(w: vec3<f32>) -> f32 {
    return clamp((w.y - p.slab.x) / p.slab.y, 0.0, 1.0);
}

fn density(w: vec3<f32>, t: f32) -> f32 {
    let h = height_frac(w);
    let wind = vec3<f32>(p.wind.x, 0.0, p.wind.z) * t;
    let q = w.xzy * vec3<f32>(p.slab.z, p.slab.z, p.slab.z * 2.5) + vec3<f32>(wind.x, wind.z, t * p.wind.y);
    let base = textureSampleLevel(noise_tex, noise_smp, q, 0.0).r;
    // Finer noise, moving a little faster, eats at the edges.
    let detail = textureSampleLevel(noise_tex, noise_smp, q * 4.3 + vec3<f32>(wind.x * 1.6, wind.z * 1.6, t * p.wind.y * 3.0), 0.0).r;
    // Flat-ish bottoms, rounded tops.
    let profile = smoothstep(0.0, 0.12, h) * (1.0 - smoothstep(mix(0.45, 0.75, p.shape.z), 1.0, h));
    // Coverage itself drifts over kilometres: banks of cloud with clear sky between.
    let big = textureSampleLevel(noise_tex, noise_smp, q * 0.23 + vec3<f32>(0.37, 0.71, t * p.wind.y * 0.2), 0.0).r;
    let cover = clamp(p.slab.w + (big - 0.5) * 0.9, 0.05, 0.95);
    var d = (base - (1.0 - cover)) / max(cover, 0.05);
    d = d * profile - (1.0 - detail) * p.shape.x;
    return clamp(d, 0.0, 1.0) * p.wind.w;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let cam = view.world_position;
    let start = in.world_position.xyz;
    let dir = normalize(start - cam);
    // Only from below (the eye is always under the slab).
    if (dir.y <= 0.02) {
        discard;
    }
    let t = globals.time;
    let len = min(p.slab.y / dir.y, p.slab.y * 7.0);
    let steps = 18;
    let dt = len / f32(steps);
    // Interleaved gradient noise jitters the start (no banding).
    let jit = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))) + fract(t * 7.123));
    var trans = 1.0;
    var col = vec3<f32>(0.0);
    let moon_col = vec3<f32>(0.62, 0.70, 0.88) * p.moon.w;
    for (var i = 0; i < steps; i = i + 1) {
        let w = start + dir * ((f32(i) + jit) * dt);
        let d = density(w, t);
        if (d > 0.002) {
            // Moon light reaching this point through the cloud above it (two taps).
            let l1 = density(w + p.moon.xyz * p.slab.y * 0.12, t);
            let l2 = density(w + p.moon.xyz * p.slab.y * 0.35, t);
            let lt = exp(-(l1 * 1.6 + l2 * 2.4));
            let h = height_frac(w);
            // City light on the bellies, fading up the cloud; a faint sky ambient everywhere.
            let below = p.glow.rgb * p.glow.w * (1.0 - h) * (1.0 - h);
            var lit = (moon_col * (0.25 + 0.75 * lt) * (0.4 + 0.6 * h) + below + vec3<f32>(0.010, 0.012, 0.018)) * p.shape.y;
            // Lightning inside the cloud: a blue-white glow round the strike, seen through the cloud.
            if (p.flash.w > 0.0) {
                let fd = length(w.xz - p.flash.xz) / p.flash.y;
                lit += vec3<f32>(0.75, 0.82, 1.0) * p.flash.w * exp(-fd * fd) * (0.4 + 0.6 * lt);
            }
            let a = 1.0 - exp(-d * dt * 0.045);
            col = col + trans * a * lit;
            trans = trans * (1.0 - a);
            if (trans < 0.02) {
                break;
            }
        }
    }
    // Fade out far away (the horizon) and at the sheet's rim.
    let r = length(start.xz - p.fade.xy);
    let edge = 1.0 - smoothstep(p.fade.z, p.fade.w, r);
    let alpha = (1.0 - trans) * edge;
    if (alpha < 0.002) {
        discard;
    }
    return vec4<f32>(col / max(1.0 - trans, 1e-3), alpha);
}
