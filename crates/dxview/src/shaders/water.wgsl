// Water surfaces (see water.rs): the WetTexture's source picture displaced by a travelling swell plus rings
// from splashes, shaded with the perturbed normal (a sky glint, more opaque at grazing angles).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals, view_transmission_texture, view_transmission_sampler},
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
    grid: vec4<f32>,
    mask: vec4<f32>,
    heights: array<vec4<f32>, 4>,
    ssr: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: WaterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var reflection_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var reflection_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var mask_tex: texture_2d<u32>;

// The water mask (water::build_mask) at a Bevy (x, z): bit 7 = water, bits 0-4 = its height's index.
fn mask_at(p: vec2<f32>) -> u32 {
    let q = floor((p - water.mask.xy) / max(water.mask.z, 1e-4));
    let dim = vec2<f32>(textureDimensions(mask_tex));
    if (q.x < 0.0 || q.y < 0.0 || q.x >= dim.x || q.y >= dim.y) {
        return 0u;
    }
    return textureLoad(mask_tex, vec2<i32>(q), 0).r;
}

// The wave grid's centre for this view: the eye, snapped to the grid's step (water::grid_mesh).
fn grid_centre() -> vec2<f32> {
    return floor(view.world_position.xz / water.grid.z) * water.grid.z;
}

// A water face or the open sea gives way to the wave grid inside its square, where the mask has water.
fn under_grid(p: vec2<f32>) -> bool {
    if (water.grid.x < 0.5 || water.grid.x > 1.5 || water.m.x < 0.5) {
        return false;
    }
    let d = abs(p - grid_centre());
    if (max(d.x, d.y) >= water.grid.y) {
        return false;
    }
    return (mask_at(p) & 128u) != 0u;
}

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
    g += storm_wave(p, t).yz / (1.0 + pow(fp * 4.0 / 31.0, 2.0));
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
    let t = globals.time;
    // The wave grid where there's water; the flat faces everywhere else.
    if (water.grid.x > 1.5) {
        if ((mask_at(in.world_position.xz) & 128u) == 0u) {
            discard;
        }
    } else if (under_grid(in.world_position.xz)) {
        discard;
    }
    if (water.m.x > 0.5) {
        return modern_water(in, is_front, t);
    }
    // Classic: the original's water - its WetTexture (animated on the CPU like the engine did: watertex.rs) drawn
    // PF_Translucent, added onto what's below, as dark as its lit picture. No swell, rings or glints of ours.
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    let to_eye = view.world_position.xyz - in.world_position.xyz;
    var out: FragmentOutput;
    if (water.far.z > 0.5) {
        out.color = vec4<f32>(color.rgb * water.far.z + vec3<f32>(0.3, 0.0, 0.0), 1.0);
        return out;
    }
    // Keep the alpha main_pass_post_lighting_processing left: Bevy draws AlphaMode::Add with the premultiplied blend
    // and an alpha of 0 is what makes it add (an alpha of 1 covered what's under the water: issue #13).
    let lit = color.rgb * water.t.z;
    out.color = vec4<f32>(lit, color.a);
    return out;
}

#ifdef DEPTH_PREPASS
// Where a world point falls on screen: (pixel x, y, NDC depth, inside the view?).
fn ssr_project(q: vec3<f32>) -> vec4<f32> {
    let c = view.clip_from_world * vec4<f32>(q, 1.0);
    if (c.w <= 1e-4) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let inside = select(0.0, 1.0, all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)));
    return vec4<f32>(view.viewport.xy + uv * view.viewport.zw, ndc.z, inside);
}

// How far a screen pixel is from the screen's edge, as a fade (reflections leaving the picture fade out instead of
// stopping at a line).
fn ssr_edge(px: vec2<f32>) -> f32 {
    let uv = (px - view.viewport.xy) / view.viewport.zw;
    let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    return smoothstep(0.0, 0.07, e);
}

fn ssr_color(px: vec2<f32>) -> vec3<f32> {
    let uv = (px - view.viewport.xy) / view.viewport.zw;
    let c = textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv, 0.0).rgb;
    // HDR lamps and coronas picked up by a stray ray came out as white sparks (with bloom on top): keep the
    // reflection in the range of what a lit wall gives.
    let l = max(max(c.r, c.g), c.b);
    return c * min(1.0, 1.5 / max(l, 1e-4));
}

fn ssr_clamp_px(px: vec2<f32>) -> vec2<f32> {
    return clamp(px, view.viewport.xy + vec2<f32>(0.5), view.viewport.xy + view.viewport.zw - vec2<f32>(0.5));
}

// The reflected ray from `p0` along `r`, stepping out with growing strides through the scene's depth; on the first
// step that passes behind a surface (by less than the surface could be thick) the step is refined and the colour
// there taken. Rays that pass everything take what lies far off in their direction (the sky, the skyline) when it
// is on screen. rgb = colour, a = how much to trust it.
fn ssr_trace(p0: vec3<f32>, r: vec3<f32>, dist: f32, steps: i32, bend_px: vec2<f32>) -> vec4<f32> {
    var s = 0.25 + dist * 0.008;
    var prev = 0.0;
    for (var i = 0; i < steps; i += 1) {
        let q = ssr_project(p0 + r * s);
        if (q.w < 0.5) {
            break;
        }
        let scene = prepass_utils::prepass_depth(vec4<f32>(q.xy, 0.0, 0.0), 0u);
        if (scene > 0.0) {
            let ray_z = depth_ndc_to_view_z(q.z);
            let scene_z = depth_ndc_to_view_z(scene);
            if (ray_z < scene_z) {
                let thick = 0.8 + s * 0.06;
                if (scene_z - ray_z > thick) {
                    // Passed behind something thick (the ray went under a wall it can't have hit): nothing to show.
                    break;
                }
                // Refine between the last step in front and this one.
                var a = prev;
                var b = s;
                for (var k = 0; k < 5; k += 1) {
                    let m = 0.5 * (a + b);
                    let qm = ssr_project(p0 + r * m);
                    let sm = prepass_utils::prepass_depth(vec4<f32>(qm.xy, 0.0, 0.0), 0u);
                    if (sm > 0.0 && depth_ndc_to_view_z(qm.z) < depth_ndc_to_view_z(sm)) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                let qh = ssr_project(p0 + r * b);
                // Rays coming back toward the eye see the backs of things the picture doesn't have.
                let facing = smoothstep(-0.2, 0.15, dot(r, -normalize(view.world_position.xyz - p0)));
                // Far along the ray the hit is less sure (coarse steps): fade it.
                let sure = 1.0 - smoothstep(60.0, 160.0, b);
                let hp = ssr_clamp_px(qh.xy + bend_px);
                return vec4<f32>(ssr_color(hp), ssr_edge(qh.xy) * facing * sure);
            }
        }
        prev = s;
        s *= 1.32;
    }
    // Past everything: what's far off that way, if the picture has it and it is far (sky, skyline, distant towers).
    let far = ssr_project(p0 + r * 20000.0);
    if (far.w > 0.5) {
        let scene = prepass_utils::prepass_depth(vec4<f32>(far.xy, 0.0, 0.0), 0u);
        let far_enough = scene <= 0.0 || -depth_ndc_to_view_z(scene) > 150.0;
        if (far_enough) {
            return vec4<f32>(ssr_color(ssr_clamp_px(far.xy + bend_px)), ssr_edge(far.xy));
        }
    }
    return vec4<f32>(0.0);
}
#endif

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
    // The swell's slopes at the height the vertices move by (far.w metres per unit, more in a storm, when the wind
    // also roughens the short waves).
    let storm = water.m.z;
    var g = swell_grad_lod(p, t, fp) * max(water.far.w, 0.16) * 1.12 * calm
        + (detail_lod(p, t, fp) * (1.0 + storm * 1.6) + rain_rings(p, t, water.m.z)) * fine;
    // Far out (the statue's top, the open bay) the waves above all average out and the sea went a flat grey sheet:
    // there a drifting ripple pattern whose size grows with the distance (about the same size on screen, like the
    // original's animated water texture) keeps the surface moving and breaks up the reflection.
    let far_k = smoothstep(45.0, 160.0, d_eye);
    if (far_k > 0.0) {
        let sc = 0.22 / (1.0 + d_eye / 55.0);
        let q = p * sc + vec2<f32>(t * 0.035, -t * 0.026);
        let q2 = p * sc * 2.3 - vec2<f32>(t * 0.05, t * 0.031);
        let e = 0.35;
        let gx = (vnoise(q + vec2<f32>(e, 0.0)) - vnoise(q - vec2<f32>(e, 0.0))) + 0.5 * (vnoise(q2 + vec2<f32>(e, 0.0)) - vnoise(q2 - vec2<f32>(e, 0.0)));
        let gy = (vnoise(q + vec2<f32>(0.0, e)) - vnoise(q - vec2<f32>(0.0, e))) + 0.5 * (vnoise(q2 + vec2<f32>(0.0, e)) - vnoise(q2 - vec2<f32>(0.0, e)));
        g += vec2<f32>(gx, gy) * (0.22 + 0.25 * storm) * far_k;
    }
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let up = select(-1.0, 1.0, pbr_input.world_normal.y >= 0.0);
    let n = normalize(vec3<f32>(-g.x, up, -g.y));
    pbr_input.N = n;
    // The light reaching the surface, without the texture's picture.
    pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    // Low roughness: the dynamic lights (lamps, muzzle flashes, the flashlight) leave sharp glints.
    pbr_input.material.perceptual_roughness = 0.06;
    // Not the lightmap: each water surface has its own coarse lightmap block, so in the body colour their edges showed
    // as blue-grey angular patches and seams across the harbour. The lamps still show in the reflection and as
    // glints of the dynamic lights.
    pbr_input.lightmap_light = vec3<f32>(0.0);
    // (The material carries a little specular transmission only to be drawn in the transmissive pass, where the
    // scene behind is on hand for the screen-space reflection: none of it is to be lit as transmission.)
    pbr_input.material.specular_transmission = 0.0;
    var light = apply_pbr_lighting(pbr_input).rgb;
    let to_eye = view.world_position.xyz - in.world_position.xyz;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);
    let ndv_n = clamp(abs(dot(n, v)), 0.0, 1.0);
    // Fresnel mostly from the flat surface: taken from the wave normal it swung the reflection (and the alpha) up and
    // down with every crossing wave at shallow angles - a dark plaid sliding over the water. The waves still show
    // in the light, the glints and the bent reflection.
    let ndv_flat = clamp(abs(v.y), 0.0, 1.0);
    let ndv = mix(ndv_flat, ndv_n, 0.2 * calm * fine + 0.05 + 0.25 * far_k);
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
        uv += vec2<f32>(-g.x, g.y) * water.m.w * (1.0 / (1.0 + dist * 0.08) + 0.04 * far_k) * (0.4 + 0.6 * ndv_flat);
        uv = clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999));
        refl = textureSampleLevel(reflection_texture, reflection_sampler, uv, 0.0).rgb * 0.6;
    }
#ifdef DEPTH_PREPASS
    // Screen-space: the reflected ray marched through the depth of the scene already drawn (Bevy's transmission
    // copy of it) - no second view of the world for the CPU to build.
    if (water.ssr.x > 0.5 && up > 0.0) {
        // The ray leaves the flat surface (so neighbouring pixels find the same thing from frame to frame: rays bent
        // by every wave hit something different each frame - a grey shimmer of stray samples); the waves bend where
        // the picture is read instead, like the planar reflection's.
        let r = reflect(-v, vec3<f32>(0.0, 1.0, 0.0));
        let bend_px = vec2<f32>(-g.x, g.y) * view.viewport.w * water.m.w * (1.0 / (1.0 + dist * 0.08) + 0.04 * far_k) * (0.4 + 0.6 * ndv_flat);
        let hit = ssr_trace(in.world_position.xyz + vec3<f32>(0.0, 0.02, 0.0), r, dist, i32(water.ssr.y), bend_px);
        refl = mix(refl, hit.rgb * 0.6, hit.a);
    }
#endif
    // The water body: its own deep colour, lit a little by the light falling into it (lamp pools glow blue-green).
    let body = water.tint.rgb * 0.55 + light * vec3<f32>(0.010, 0.022, 0.027);
    // Physically 2% straight down; a little more than Schlick at middle angles reads better.
    let k_refl = clamp(fres + 0.12 * pow(1.0 - ndv, 2.0), 0.0, 1.0);
    var col = mix(body, refl, k_refl);
    // Indoor water (no wave grid over it: water::build_mask leaves out water without sky above): no night sky to
    // mirror and no light but the lightmap's, so its body is the original's lit picture, under the reflection and
    // the glints.
#ifdef VERTEX_UVS_A
    if (water.grid.x < 0.5 || (water.grid.x < 1.5 && (mask_at(p) & 128u) == 0u)) {
        var ci = pbr_input_from_standard_material(in, is_front);
        ci.material.specular_transmission = 0.0;
        let c_in = apply_pbr_lighting(ci).rgb * water.t.z;
        col = c_in + refl * k_refl * 0.5;
    }
#endif
    // Far out the modern surface has nothing left to show (its waves average out to a flat sheet): it hands over
    // to the original's animated water picture, lit by its lightmap, under the same reflection - so the far bay
    // still moves and there's no line where the near water's detail ends.
    // (Only the level's water sheets and the open sea carry the picture's UVs; the wave grid round the eye, out to
    // 150 m, has none.)
#ifdef VERTEX_UVS_A
    let far_tex = smoothstep(110.0, 220.0, d_eye) * 0.75;
    if (far_tex > 0.0) {
        // Lit exactly as the Classic water is (texture, lightmap and lamps; the classic path below).
        // Its picture spread much larger than up close (1/12 the UV rate): at this range the original's tile is
        // a few pixels and filters down to one flat grey; enlarged, its animated ripples still show and move.
        var in_far = in;
        in_far.uv = in.uv * 0.083;
        var cp = pbr_input_from_standard_material(in_far, is_front);
        cp.material.specular_transmission = 0.0;
        let c_lit = apply_pbr_lighting(cp).rgb * water.t.z;
        col = mix(col, c_lit + refl * k_refl * 0.6, far_tex);
    }
#endif
    // Foam on the crests: where the swell peaks, broken up by drifting noise, thinning out with distance.
    let crest = f.x / 1.99;
    // (the crest is the whole field; fine)
    let n1 = vnoise(p * 1.7 + vec2<f32>(t * 0.21, t * 0.13));
    let n2 = vnoise(p * 5.3 - vec2<f32>(t * 0.37, t * 0.29));
    let streaks = n1 * 0.65 + n2 * 0.35;
    // Whitecaps: a storm breaks far more of the crests.
    let cap_lo = 0.5 - water.m.z * 0.3;
    var foam = smoothstep(cap_lo, cap_lo + 0.4, crest + (streaks - 0.5) * 0.55) * smoothstep(0.4 - water.m.z * 0.25, 0.8, streaks);
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
#ifdef DEPTH_PREPASS
    if (water.ssr.x > 0.5) {
        // Drawn opaque over a copy of the scene: blend onto it here, the scene under the water bent a little by
        // the waves (not where that would pull in something standing in front of the water).
        var px = in.position.xy + vec2<f32>(g.x, -g.y) * view.viewport.w * 0.012 / (1.0 + dist * 0.12);
        let sd = prepass_utils::prepass_depth(vec4<f32>(px, 0.0, 0.0), 0u);
        if (sd > in.position.z) {
            px = in.position.xy;
        }
        px = clamp(px, view.viewport.xy + vec2<f32>(0.5), view.viewport.xy + view.viewport.zw - vec2<f32>(0.5));
        out.color = vec4<f32>(mix(ssr_color(px), col, alpha), 1.0);
    }
#endif
    // DXVIEW_WATER_DEBUG=2: the reflection alone (red where this plane has none); 3: the depth through the water.
    if (water.far.z > 1.5) {
        out.color = select(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(refl * 3.0, 1.0), water.m.y > 0.5 || water.ssr.x > 0.5);
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
    return wave(p, t, 0.90, 17.0, 2.1, 0.85).x + storm_wave(p, t).x;
}

// A storm's long swell (water::STORM_WAVE; water.m.z = how hard it rains, 0..1): rolls in with the weather.
fn storm_wave(p: vec2<f32>, t: f32) -> vec3<f32> {
    return wave(p, t, 0.55, 31.0, 3.4, 1.1) * water.m.z;
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
    if (water.grid.x > 1.5) {
        // The wave grid: round this view's eye, at the water's height the mask gives, with the swell - fading to
        // nothing at the square's edge, where the flat faces take over.
        let c = grid_centre();
        wp = vec4<f32>(vertex.position.x + c.x, 0.0, vertex.position.z + c.y, 1.0);
        let k = mask_at(wp.xz) & 31u;
        wp.y = water.heights[k / 4u][k % 4u];
        let edge = max(abs(vertex.position.x), abs(vertex.position.z)) / water.grid.y;
        // And with distance from the eye: out where a 1.5 m grid step is a good part of a wave, the crests stood up
        // from the sea as rows of pale triangles against the flat water beyond (the shading still carries the waves
        // there: swell_grad_lod).
        let de = length(view.world_position.xz - wp.xz);
        let st = water.m.z;
        let near = 1.0 - smoothstep(35.0 + 25.0 * st, 75.0 + 35.0 * st, de);
        let fade = (1.0 - smoothstep(0.6, 0.95, edge)) * near;
        wp.y += swell_h(wp.xz, globals.time) * water.far.w * fade;
        out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    }
    // The flat sheets and the open sea stay flat: their triangles are tens to hundreds of metres across, so moving
    // their corners with the swell (the old storm rule ran it out to 900 m) tilted whole faces - facets and lines
    // across the far water. The wave grid carries the swell near the eye; out here the shading does.
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
