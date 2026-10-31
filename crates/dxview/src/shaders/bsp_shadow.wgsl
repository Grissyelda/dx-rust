// The lit BSP with real-time shadows over its lightmap (shadowlight.rs). Otherwise Bevy's StandardMaterial
// fragment shader. Each shadow-mapped point light in the fragment's cluster is one of shadowlight's lights: its
// colour x intensity carries the Unreal light's colour (x ENCODE), its range the Unreal radius. Where its shadow
// map says the texel is blocked, the light's own term of the bake comes off the lightmap - in gamma space, as
// dx-level's bake summed it: colour x (1 - d / r) x |N.L| x LIGHT_GAIN.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
    shadows,
}

struct BspShadowParams {
    p: vec4<f32>,
    q: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> bsp: BspShadowParams;

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn shadowed_lightmap(lm: vec3<f32>, world_position: vec4<f32>, n: vec3<f32>, frag_coord: vec4<f32>, ortho: bool) -> vec3<f32> {
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), world_position);
    let cluster_index = clustering::view_fragment_cluster_index(frag_coord.xy, view_z, ortho);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    let s = to_srgb(lm);
    var removed = vec3<f32>(0.0);
    var dbg_own = vec3<f32>(0.0);
    // shadowlight.rs's lights are spot lights aimed at the things that cast (one shadow view each instead of a
    // point light's six), told apart from other spots by their depth bias (SHADOW_MARK).
    for (var i: u32 = ranges.first_spot_light_index_offset; i < ranges.first_reflection_probe_index_offset; i = i + 1u) {
        let light_id = clustering::get_clusterable_object_id(i);
        let light = &view_bindings::clustered_lights.data[light_id];
        if (((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
            continue;
        }
        if (abs((*light).shadow_depth_bias - 0.0201) > 1e-6) {
            continue;
        }
        let to = (*light).position_radius.xyz - world_position.xyz;
        let d = length(to);
        let r = (*light).range;
        if (d >= r || d < 1e-3) {
            continue;
        }
        let shadow = shadows::fetch_spot_shadow(light_id, world_position, n, (*light).shadow_map_near_z, frag_coord.xy);
        if (shadow >= 0.999 && bsp.p.w < 1.5) {
            continue;
        }
        let colour = (*light).color_inverse_square_range.rgb * bsp.p.y;
        let own = colour * (1.0 - d / r) * abs(dot(to / d, n)) * bsp.p.z;
        // A texel the lightmap shows dark for this light (a wall between them, baked) has nothing of it to lose.
        if (max(s.r, max(s.g, s.b)) < 0.6 * max(own.r, max(own.g, own.b)) && bsp.p.w < 1.5) {
            continue;
        }
        removed += own * (1.0 - shadow);
        if (bsp.p.w > 1.5) {
            dbg_own = max(dbg_own, own);
        }
    }
    // DXVIEW_SHADOW_DEBUG=1: the lightmap as the bake summed it (gamma); 2: the shadow lights' own terms in red
    // over it in green.
    if (bsp.p.w > 1.5) {
        return vec3<f32>(max(dbg_own.r, max(dbg_own.g, dbg_own.b)), max(s.r, max(s.g, s.b)), 0.0);
    }
    if (bsp.p.w > 0.5) {
        return s;
    }
    if (max(removed.r, max(removed.g, removed.b)) <= 0.0 && bsp.p.w < 0.5) {
        return lm;
    }
    // Never quite black: what's left of the texel stands in for light bounced about.
    let left = max(s - removed * bsp.p.x, s * (1.0 - bsp.p.x));
    return from_srgb(left);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // The face's own normal, not the normal prepass's (Bevy loads that when MSAA is off; a texel the prepass
    // didn't cover would light the face from the wrong side).
    pbr_input.N = normalize(pbr_input.world_normal);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef LIGHTMAP
    if (bsp.p.x > 0.0) {
        pbr_input.lightmap_light = shadowed_lightmap(
            pbr_input.lightmap_light,
            pbr_input.world_position,
            pbr_input.world_normal,
            pbr_input.frag_coord,
            pbr_input.is_orthographic,
        );
    }
#endif
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#ifdef LIGHTMAP
    // Ambient Occlusion (postfx.rs): the world's light is its lightmap, so that's what corners darken.
    let ao = textureLoad(view_bindings::screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.lightmap_light *= mix(1.0, ao, bsp.q.x);
#endif
#endif
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
