// Water surfaces (see water.rs): the WetTexture's source picture displaced by a travelling swell plus rings
// from splashes, shaded with the perturbed normal (a sky glint, more opaque at grazing angles).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
}

struct WaterParams {
    t: vec4<f32>,
    far: vec4<f32>,
    rings: array<vec4<f32>, 8>,
    m: vec4<f32>,
    tint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: WaterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var reflection_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var reflection_sampler: sampler;

const TAU: f32 = 6.2831853;

// Height and gradient (per meter) of the swell at p (Bevy x, z): keep in step with water::WAVES.
fn wave(p: vec2<f32>, t: f32, a: f32, len: f32, speed: f32, amp: f32) -> vec3<f32> {
    let d = vec2<f32>(cos(a), sin(a));
    let k = TAU / len;
    let ph = dot(d, p) * k - t * speed * k;
    return vec3<f32>(amp * sin(ph), amp * k * d * cos(ph));
}

fn field(p: vec2<f32>, t: f32) -> vec3<f32> {
    var f = wave(p, t, 0.35, 6.0, 1.1, 0.50);
    f += wave(p, t, 1.95, 3.3, 0.85, 0.30);
    f += wave(p, t, 4.10, 1.8, 0.65, 0.18);
    f += wave(p, t, 2.75, 0.95, 0.45, 0.10);
    f += wave(p, t, 5.30, 0.55, 0.38, 0.06);
    // Rings: a wave train spreading at 1.6 m/s from where something hit the water, fading over ~3 s.
    for (var i = 0; i < 8; i++) {
        let r = water.rings[i];
        if (r.w <= 0.0) {
            continue;
        }
        let age = t - r.z;
        if (age < 0.0 || age > 4.0) {
            continue;
        }
        let v = p - r.xy;
        let dist = length(v);
        let front = age * 1.6;
        let x = dist - front;
        let env = exp(-x * x * 6.0) * exp(-age * 0.9) * r.w * 0.9;
        let k = TAU / 0.45;
        let h = env * cos(x * k);
        let dh = -env * k * sin(x * k);
        let dir = select(vec2<f32>(0.0), v / dist, dist > 1e-3);
        f += vec3<f32>(h, dh * dir);
    }
    return f;
}

// Modern Water's fine detail: short wind waves (gradient only; the swell above moves the floating things).
fn detail(p: vec2<f32>, t: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    g += wave(p, t, 0.6, 0.42, 0.55, 0.020).yz;
    g += wave(p, t, 2.2, 0.31, 0.47, 0.016).yz;
    g += wave(p, t, 3.7, 0.23, 0.40, 0.012).yz;
    g += wave(p, t, 5.1, 0.17, 0.34, 0.009).yz;
    g += wave(p, t, 1.3, 0.11, 0.28, 0.006).yz;
    g += wave(p, t, 4.4, 0.075, 0.22, 0.004).yz;
    // Slopes of a calm harbour: the six add up to about 0.25 at most.
    return g * 0.3;
}

fn hash2(c: vec2<f32>) -> f32 {
    return fract(sin(dot(c, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Raindrop rings: in each 0.4 m cell a drop lands at a random spot and time; a ring spreads and fades in 0.6 s.
fn rain_rings(p: vec2<f32>, t: f32, amount: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    if (amount <= 0.001) {
        return g;
    }
    let cell = 0.4;
    let base = floor(p / cell);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = base + vec2<f32>(f32(i), f32(j));
            let h = hash2(c);
            if (h > amount * 1.2 + 0.05) {
                continue;
            }
            let centre = (c + vec2<f32>(hash2(c + 3.1), hash2(c + 7.7))) * cell;
            let period = 0.6 + h * 0.5;
            let age = fract(t / period + h) * period;
            let v = p - centre;
            let d = length(v);
            let x = d - age * 0.35;
            let env = exp(-x * x * 900.0) * (1.0 - age / period);
            let dh = -env * sin(x * 90.0) * 2.5;
            g += select(vec2<f32>(0.0), v / max(d, 1e-4) * dh, d > 1e-4);
        }
    }
    return g * 0.08;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let t = water.t.x;
    if (water.m.x > 0.5) {
        return modern_water(in, is_front, t);
    }
    let p = in.world_position.xz;
    let f = field(p, t);
    // The WetTexture look: the source picture pushed about by the slope of the water.
    var displaced = in;
    displaced.uv = in.uv + f.yz * water.t.y;
    var pbr_input = pbr_input_from_standard_material(displaced, is_front);
    // A normal tilted by the slope (the surfaces are flat, facing up or down).
    let up = select(-1.0, 1.0, pbr_input.world_normal.y >= 0.0);
    let n = normalize(vec3<f32>(-f.y * 0.18, up, -f.z * 0.18));
    pbr_input.N = n;
    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    let to_eye = view.world_position.xyz - in.world_position.xyz;
    let v = normalize(to_eye);
    let ndv = clamp(abs(dot(n, v)), 0.0, 1.0);
    // The original draws water PF_Translucent (added onto what is below), so it is as dark as its lit texture.
    // A faint moon glint and a little sky at grazing angles on top.
    let l = normalize(vec3<f32>(0.35, 1.0, 0.2));
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 260.0) * water.t.w;
    let fres = pow(1.0 - ndv, 4.0);
    // Far water fades out, so the edge of the map's water melts into the night instead of ending in a line.
    let fade = 1.0 - smoothstep(water.far.x, water.far.y, length(to_eye));
    var out: FragmentOutput;
    if (water.far.z > 0.5) {
        out.color = vec4<f32>(color.rgb * water.far.z + vec3<f32>(0.3, 0.0, 0.0), 1.0);
        return out;
    }
    // Everything here is lit by the surface's lightmap (shadows of the dock and the lamp pools included): an unlit
    // sky term on top made the whole sheet one even grey-blue, brightest at grazing angles, ending in a hard line
    // at the dock. Grazing angles only raise the lit texture a little.
    // A faint night-sky sheen at grazing angles keeps unlit water from reading as a hole (a quarter of what it was).
    let lit = color.rgb * water.t.z * (1.0 + fres * 0.6);
    let sheen = fres * vec3<f32>(0.005, 0.0075, 0.01);
    out.color = vec4<f32>((lit + sheen + vec3<f32>(spec) * min(1.0, dot(color.rgb, vec3<f32>(4.0)))) * fade, 1.0);
    return out;
}

// Modern Water: a shader-pack surface - the planar reflection (or the night sky) by Fresnel, distorted by the waves,
// over a dark water body you can see a little way into, with glints from the lights.
fn modern_water(in: VertexOutput, is_front: bool, t: f32) -> FragmentOutput {
    let p = in.world_position.xz;
    let f = field(p, t);
    // The fine waves and rain rings fade with distance (a pixel there covers many of them: no sparkle).
    let d_eye = length(view.world_position.xyz - in.world_position.xyz);
    let fine = 1.0 / (1.0 + d_eye * d_eye * 0.004);
    let g = vec2<f32>(f.y, f.z) * 0.18 + (detail(p, t) + rain_rings(p, t, water.m.z)) * fine;
    var displaced = in;
    displaced.uv = in.uv + g * 0.4;
    var pbr_input = pbr_input_from_standard_material(displaced, is_front);
    let up = select(-1.0, 1.0, pbr_input.world_normal.y >= 0.0);
    let n = normalize(vec3<f32>(-g.x, up, -g.y));
    pbr_input.N = n;
    // Low roughness: the dynamic lights (lamps, muzzle flashes, the flashlight) leave sharp glints.
    pbr_input.material.perceptual_roughness = 0.07;
    var lit = apply_pbr_lighting(pbr_input);
    lit = main_pass_post_lighting_processing(pbr_input, lit);
    let to_eye = view.world_position.xyz - in.world_position.xyz;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);
    let ndv = clamp(abs(dot(n, v)), 0.0, 1.0);
    let fres = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
    // The reflection: this plane's mirror image when it has a slot, else a dim night sky.
    var refl = vec3<f32>(0.006, 0.009, 0.016) + vec3<f32>(0.02, 0.025, 0.035) * pow(1.0 - ndv, 2.0);
    if (water.m.y > 0.5 && up > 0.0) {
        var uv = (in.position.xy - view.viewport.xy) / view.viewport.zw;
        uv.x = 1.0 - uv.x;
        // Waves bend the reflection, less far away (where a pixel spans more water).
        uv += vec2<f32>(-g.x, g.y) * water.m.w / (1.0 + dist * 0.05);
        uv = clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999));
        refl = textureSampleLevel(reflection_texture, reflection_sampler, uv, 0.0).rgb;
    }
    // The body: the lit WetTexture tinted dark, so lamp pools and the lightmap's shadows still show in it.
    let body = water.tint.rgb + lit.rgb * 0.35;
    // Physically water reflects 2% straight down; a little more than Schlick at middle angles reads better.
    let k_refl = clamp(fres + 0.22 * pow(1.0 - ndv, 2.0) + 0.05, 0.0, 1.0);
    var col = mix(body, refl, k_refl);
    // A faint moon glint, nearby only.
    let l = normalize(vec3<f32>(0.35, 1.0, 0.2));
    let hv = normalize(l + v);
    col += vec3<f32>(pow(max(dot(n, hv), 0.0), 400.0) * 0.06 * fine);
    // See-through looking straight down, opaque at grazing angles.
    let alpha = clamp(mix(water.tint.a, 1.0, fres * 1.5 + 0.15), 0.0, 1.0);
    let fade = 1.0 - smoothstep(water.far.x * 2.0, water.far.y * 2.0, dist);
    var out: FragmentOutput;
    out.color = vec4<f32>(col, alpha * fade);
    // DXVIEW_WATER_DEBUG=2: the reflection alone (red where this plane has none).
    if (water.far.z > 1.5) {
        out.color = select(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(refl * 3.0, 1.0), water.m.y > 0.5);
    }
    return out;
}
