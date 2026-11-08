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

// The reflection, with the empty parts filled from their neighbours. The reflection cameras clear to transparent
// black, so a texel nothing was drawn into (past the clip plane: under the floor, behind the foot of a wall) has
// alpha 0. The reflection is drawn at two thirds or a third of the window's size, so along every floor / wall
// corner the smoothly sampled image mixed some of that nothing - the black, or the outside of the level - into the
// seam (#45). Where the sample isn't solid, the solid texels around it (one and two texels out) stand in for it,
// weighted by how solid they are.
fn reflection_at(uv: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(mirror_texture, mirror_sampler, uv, 0.0);
    if (c.a >= 0.999) {
        return c.rgb;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(mirror_texture));
    var sum = c.rgb * c.a;
    var w = c.a;
    for (var r = 1.0; r <= 2.0; r += 1.0) {
        for (var i = 0; i < 8; i++) {
            let a = f32(i) * 0.785398;
            let s = textureSampleLevel(mirror_texture, mirror_sampler, uv + vec2<f32>(cos(a), sin(a)) * texel * r, 0.0);
            sum += s.rgb * s.a;
            w += s.a;
        }
        if (w > 0.5) {
            break;
        }
    }
    if (w <= 0.001) {
        return vec3<f32>(0.0);
    }
    return sum / w;
}

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
        let r = reflection_at(uv);
        out.color = vec4<f32>(r + surface.rgb * mirror_params.y, 1.0);
    } else {
        out.color = vec4<f32>(surface.rgb, 1.0);
    }
    return out;
}
