// Vertex-lit mesh actors drawn by gpuanim.rs (pawns, animated decorations): StandardMaterial's unlit fragment with
// real-time shadows on top (shadowlight.rs). Their light is in the vertex colour (dx-level's light_part: the summed
// Unreal terms x LIGHT_GAIN, clamped, to linear); where a shadow light's map says the pixel is blocked, that light's
// own term comes off it - its share of the vertex light, never quite to black.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    forward_io::{VertexOutput, FragmentOutput},
}
#import dxview::shadow_spots as spot_shadows

const GAIN: f32 = 4.0;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
#ifdef VERTEX_COLORS
    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let count = spot_shadows::gather(in.world_position, n, in.position, false);
    if (count > 0u) {
        let lit = max(in.color.rgb, vec3<f32>(1e-4));
        let g = pow(lit, vec3<f32>(1.0 / 2.2));
        var removed = vec3<f32>(0.0);
        var total = vec3<f32>(0.0);
        for (var i = 0u; i < count; i = i + 1u) {
            let sp = spot_shadows::spots[i];
            let to = sp.pos - in.world_position.xyz;
            let d = length(to);
            let own = sp.colour * (1.0 - d / sp.range) * max(dot(n, to / d), 0.0) * GAIN;
            total += own;
            removed += own * (1.0 - sp.shadow);
        }
        let share = min(vec3<f32>(1.0), g / max(total, vec3<f32>(1e-4)));
        let left = max(g - removed * share * 0.85, g * 0.15);
        out.color = vec4<f32>(out.color.rgb * pow(left / g, vec3<f32>(2.2)), out.color.a);
    }
#endif
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
