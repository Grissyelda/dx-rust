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
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: WaterParams;

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

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let t = water.t.x;
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
