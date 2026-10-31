// Skeletal characters (skeletal.rs): lit per pixel by up to eight Unreal lights, the way dx-level's light_part
// lights a mesh's vertices: colour x (1 - d / r) x max(N.L, 0) (|N.L| on two-sided sections), summed, x LIGHT_GAIN
// x ScaleGlow, clamped and taken to linear (Unreal adds up light in gamma).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
}

struct ActorLights {
    pos: array<vec4<f32>, 8>,
    col: array<vec4<f32>, 8>,
    misc: vec4<f32>,
    wet: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> lights: ActorLights;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let p = in.world_position.xyz;
    var c = vec3<f32>(0.0);
    var sheen = vec3<f32>(0.0);
    let wet = lights.wet.x;
    let v = normalize(view.world_position - p);
    let count = u32(lights.misc.x);
    for (var i = 0u; i < 8u; i = i + 1u) {
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
        c = c + lights.col[i].rgb * (1.0 - d / r) * max(ndl, 0.0);
        if (wet > 0.0) {
            // Wet: a tight highlight from each light.
            let h = normalize(to / max(d, 1e-4) + v);
            sheen = sheen + lights.col[i].rgb * (1.0 - d / r) * pow(max(dot(n, h), 0.0), 40.0);
        }
    }
    var lin = pow(clamp(c * lights.misc.y, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    if (lights.misc.z > 0.5) {
        lin = vec3<f32>(1.0);
    }
    // Wet cloth and skin go darker, with the sheen on top.
    let base = pbr_input.material.base_color.rgb * mix(1.0, 0.62, wet);
    let shine = pow(clamp(sheen * lights.misc.y * 0.5, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2)) * wet;
    out.color = vec4<f32>(base * lin + shine, pbr_input.material.base_color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
