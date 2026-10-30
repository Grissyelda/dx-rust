// Water surfaces (see water.rs): the WetTexture's source picture displaced by a travelling swell plus rings
// from splashes, shaded with the perturbed normal (a sky glint, more opaque at grazing angles).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    mesh_functions,
    view_transformations::{position_world_to_clip, depth_ndc_to_view_z},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

struct WaterParams {
    t: vec4<f32>,
    far: vec4<f32>,
    rings: array<vec4<f32>, 8>,
    m: vec4<f32>,
    tint: vec4<f32>,
    moon: vec4<f32>,
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
    f += wave(p, t, 0.90, 17.0, 2.1, 0.85);
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

// The fine waves faded out by pixel footprint (fp, metres) like the swell below.
fn detail_lod(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    g += wave(p, t, 0.6, 0.42, 0.55, 0.020).yz / (1.0 + pow(fp * 4.0 / 0.42, 2.0));
    g += wave(p, t, 2.2, 0.31, 0.47, 0.016).yz / (1.0 + pow(fp * 4.0 / 0.31, 2.0));
    g += wave(p, t, 3.7, 0.23, 0.40, 0.012).yz / (1.0 + pow(fp * 4.0 / 0.23, 2.0));
    g += wave(p, t, 5.1, 0.17, 0.34, 0.009).yz / (1.0 + pow(fp * 4.0 / 0.17, 2.0));
    g += wave(p, t, 1.3, 0.11, 0.28, 0.006).yz / (1.0 + pow(fp * 4.0 / 0.11, 2.0));
    g += wave(p, t, 4.4, 0.075, 0.22, 0.004).yz / (1.0 + pow(fp * 4.0 / 0.075, 2.0));
    return g * 0.3;
}

// The swell's gradient with each wave faded out once a pixel spans a good part of its length (no grain far off).
// fp = the width of the water a pixel covers, metres.
fn swell_grad_lod(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    g += wave(p, t, 0.35, 6.0, 1.1, 0.50).yz / (1.0 + pow(fp * 4.0 / 6.0, 2.0));
    g += wave(p, t, 0.90, 17.0, 2.1, 0.85).yz / (1.0 + pow(fp * 4.0 / 17.0, 2.0));
    g += wave(p, t, 1.95, 3.3, 0.85, 0.30).yz / (1.0 + pow(fp * 4.0 / 3.3, 2.0));
    g += wave(p, t, 4.10, 1.8, 0.65, 0.18).yz / (1.0 + pow(fp * 4.0 / 1.8, 2.0));
    g += wave(p, t, 2.75, 0.95, 0.45, 0.10).yz / (1.0 + pow(fp * 4.0 / 0.95, 2.0));
    g += wave(p, t, 5.30, 0.55, 0.38, 0.06).yz / (1.0 + pow(fp * 4.0 / 0.55, 2.0));
    return g;
}

// Smooth value noise (foam breakup).
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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
    // A ring grows to 0.385 m (+ its 0.05 m ripple); cells are bigger than that so the 3x3 lookup never clips one
    // into a square.
    let cell = 0.5;
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

// Modern Water: a shader-pack surface. Clear water whose colour comes from how much of it you look through (the
// scene's depth behind it): shallow water shows what's under it, deep water turns a dark blue-green; the planar
// reflection by Fresnel, bent by the waves; sharp glints of the moon and the lamps. The WetTexture's picture isn't
// drawn (its dirty streaks tiled into a dark plaid); only the light falling on the water (lightmap, lamps) colours
// the water body.
fn modern_water(in: VertexOutput, is_front: bool, t: f32) -> FragmentOutput {
    let p = in.world_position.xz;
    let f = field(p, t);
    // The fine waves and rain rings fade with distance (a pixel there covers many of them: no sparkle).
    let d_eye = length(view.world_position.xyz - in.world_position.xyz);
    let fine = 1.0 / (1.0 + d_eye * d_eye * 0.004);
    // A pixel's footprint on the water (grows at grazing angles too).
    let to_eye0 = normalize(view.world_position.xyz - in.world_position.xyz);
    let fp = d_eye * 0.0015 / max(abs(to_eye0.y), 0.08);
    // Far off, the swell's slopes only alias into dark bands (a "plaid" towards the horizon): the far water
    // calms to a smooth mirror of the sky.
    let calm = mix(0.12, 1.0, 1.0 / (1.0 + (d_eye / 40.0) * (d_eye / 40.0)));
    let g = swell_grad_lod(p, t, fp) * 0.18 * calm + (detail_lod(p, t, fp) + rain_rings(p, t, water.m.z)) * fine;
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let up = select(-1.0, 1.0, pbr_input.world_normal.y >= 0.0);
    let n = normalize(vec3<f32>(-g.x, up, -g.y));
    pbr_input.N = n;
    // The light reaching the surface, without the texture's picture.
    pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    // Low roughness: the dynamic lights (lamps, muzzle flashes, the flashlight) leave sharp glints.
    pbr_input.material.perceptual_roughness = 0.06;
    var light = apply_pbr_lighting(pbr_input).rgb;
    let to_eye = view.world_position.xyz - in.world_position.xyz;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);
    let ndv_n = clamp(abs(dot(n, v)), 0.0, 1.0);
    // Fresnel mostly from the flat surface: taken from the wave normal it swung the reflection (and the alpha) up and
    // down with every crossing wave at shallow angles - a dark plaid sliding over the water. The waves still show
    // in the light, the glints and the bent reflection.
    let ndv_flat = clamp(abs(v.y), 0.0, 1.0);
    let ndv = mix(ndv_flat, ndv_n, 0.2 * calm * fine + 0.05);
    let fres = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
    // How much water the eye looks through to the scene behind (metres along the view).
    var depth = 1000.0;
#ifdef DEPTH_PREPASS
    let scene = prepass_utils::prepass_depth(in.position, 0u);
    if (scene > 0.0) {
        let dz = depth_ndc_to_view_z(in.position.z) - depth_ndc_to_view_z(scene);
        depth = max(dz, 0.0) / max(ndv, 0.15);
    }
#endif
    // The reflection: this plane's mirror image when it has a slot, else a dim night sky.
    var refl = vec3<f32>(0.004, 0.006, 0.011) + vec3<f32>(0.012, 0.016, 0.024) * pow(1.0 - ndv, 2.0);
    if (water.m.y > 0.5 && up > 0.0) {
        var uv = (in.position.xy - view.viewport.xy) / view.viewport.zw;
        uv.x = 1.0 - uv.x;
        // Waves bend the reflection, less far away (where a pixel spans more water).
        uv += vec2<f32>(-g.x, g.y) * water.m.w / (1.0 + dist * 0.08) * (0.4 + 0.6 * ndv_flat);
        uv = clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999));
        refl = textureSampleLevel(reflection_texture, reflection_sampler, uv, 0.0).rgb * 0.5;
    }
    // The water body: its own deep colour, lit a little by the light falling into it (lamp pools glow blue-green).
    let body = water.tint.rgb * 0.55 + light * vec3<f32>(0.010, 0.022, 0.027);
    // Physically 2% straight down; a little more than Schlick at middle angles reads better.
    let k_refl = clamp(fres + 0.12 * pow(1.0 - ndv, 2.0), 0.0, 1.0);
    var col = mix(body, refl, k_refl);
    // Foam on the crests: where the swell peaks, broken up by drifting noise, thinning out with distance.
    let crest = f.x / 1.99;
    // (the crest is the whole field; fine)
    let n1 = vnoise(p * 1.7 + vec2<f32>(t * 0.21, t * 0.13));
    let n2 = vnoise(p * 5.3 - vec2<f32>(t * 0.37, t * 0.29));
    let streaks = n1 * 0.65 + n2 * 0.35;
    var foam = smoothstep(0.5, 0.9, crest + (streaks - 0.5) * 0.55) * smoothstep(0.4, 0.8, streaks);
    foam *= 1.0 / (1.0 + d_eye * d_eye * 0.0006);
    let foam_col = max(light * 0.12, vec3<f32>(0.03, 0.035, 0.04)) + vec3<f32>(0.015);
    col = mix(col, foam_col, clamp(foam, 0.0, 1.0) * 0.4);
    // The moon on the water: a tight sparkle on the facets that catch it, over a soft path towards it.
    let l = normalize(water.moon.xyz);
    let hv = normalize(l + v);
    let nh = max(dot(n, hv), 0.0);
    let moon_col = vec3<f32>(0.75, 0.82, 1.0);
    col += moon_col * (pow(nh, 900.0) * 0.6 + pow(nh, 90.0) * 0.02) * water.moon.w * (0.3 + 0.7 * fine);
    // See-through by how much water there is: clear in the shallows, its own colour once deep (and always by
    // the reflection at grazing angles).
    let absorb = 1.0 - exp(-depth * 0.55);
    let alpha = clamp(max(absorb, k_refl + foam * 0.4), 0.0, 1.0);
    var out: FragmentOutput;
    out.color = vec4<f32>(col, alpha);
    // DXVIEW_WATER_DEBUG=2: the reflection alone (red where this plane has none); 3: the depth through the water.
    if (water.far.z > 1.5) {
        out.color = select(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(refl * 3.0, 1.0), water.m.y > 0.5);
    }
    if (water.far.z > 2.5) {
        out.color = vec4<f32>(vec3<f32>(1.0 - exp(-depth * 0.2)), 1.0);
    }
    return out;
}

// The swell alone (height in its own units): what the surface rises and falls by (x far.w metres) and what floating
// things ride (water::bob). Keep in step with water::WAVES.
// Only the long swell (>= water::DISPLACE_MIN_LEN) moves the vertices: the mesh is 2 m, and the 6 m and shorter
// waves came out as facets.
fn swell_h(p: vec2<f32>, t: f32) -> f32 {
    return wave(p, t, 0.90, 17.0, 2.1, 0.85).x;
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
};

// Modern Water: the surface itself moves with the swell (far.w metres per unit of swell; 0 = flat as the original).
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let ii = vertex.instance_index;
    let world_from_local = mesh_functions::get_world_from_local(ii);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, ii);
    if (water.far.w > 0.0 && abs(out.world_normal.y) > 0.7) {
        // Full near the eye, flat far away (where a vertex is many waves apart and the far sea meets the map's).
        let de = length(view.world_position.xz - wp.xz);
        let k = 1.0 - smoothstep(90.0, 220.0, de);
        wp.y += swell_h(wp.xz, water.t.x) * water.far.w * k;
    }
#endif
    out.world_position = wp;
    out.position = position_world_to_clip(wp.xyz);
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = ii;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(ii, world_from_local[3]);
#endif
    return out;
}
