// Skeletal characters (skeletal.rs): lit per pixel by up to sixteen Unreal lights (the rest as an even fill), the way dx-level's light_part
// lights a mesh's vertices: colour x (1 - d / r) x max(N.L, 0) (|N.L| on two-sided sections), summed, x LIGHT_GAIN
// x ScaleGlow, clamped and taken to linear (Unreal adds up light in gamma).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

struct ActorLights {
    pos: array<vec4<f32>, 16>,
    col: array<vec4<f32>, 16>,
    misc: vec4<f32>,
    wet: vec4<f32>,
    amb: vec4<f32>,
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
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let p = in.world_position.xyz;
    var c = vec3<f32>(0.0);
    let count = u32(lights.misc.x);
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
        c = c + lights.col[i].rgb * (1.0 - d / r) * max(ndl, 0.0);
    }
    c = c + lights.amb.rgb;
    var lin = pow(clamp(c * lights.misc.y, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    if (lights.misc.z > 0.5) {
        lin = vec3<f32>(1.0);
    }
    out.color = vec4<f32>(pbr_input.material.base_color.rgb * lin, pbr_input.material.base_color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
