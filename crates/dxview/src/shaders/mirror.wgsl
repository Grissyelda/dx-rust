// Mirrored BSP faces (see mirror.rs): the surface lit as usual, plus the reflected scene sampled at this
// fragment's screen position. Unreal draws the mirror view first and the translucent surface texture over it
// (additively), which is what out = reflection + surface does.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var mirror_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mirror_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> mirror_params: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    var surface = apply_pbr_lighting(pbr_input);
    surface = main_pass_post_lighting_processing(pbr_input, surface);
    if (mirror_params.x > 0.5) {
        var uv = (in.position.xy - view.viewport.xy) / view.viewport.zw;
        uv.x = 1.0 - uv.x; // the reflection camera's view is mirrored left to right (mirror.rs)
        let r = textureSampleLevel(mirror_texture, mirror_sampler, uv, 0.0).rgb;
        out.color = vec4<f32>(r + surface.rgb * mirror_params.y, 1.0);
    } else {
        out.color = vec4<f32>(surface.rgb, 1.0);
    }
    return out;
}
