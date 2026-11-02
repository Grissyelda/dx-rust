// shadowlight.rs's shadow lights as seen from one fragment: the spot lights marked by their depth bias (SHADOW_MARK)
// in the fragment's cluster, inside their cone and range, with the Unreal light's colour decoded and the shadow
// map's verdict (1 lit, 0 blocked). Shared by the BSP (bsp_shadow.wgsl) and the people (actor_lit.wgsl,
// anim_frag.wgsl), which each take their own light away where it's blocked.
#define_import_path dxview::shadow_spots

#import bevy_pbr::{
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
    shadows,
}

struct Spot {
    pos: vec3<f32>,
    range: f32,
    colour: vec3<f32>,
    shadow: f32,
}

const DECODE: f32 = 10000.0;

var<private> spots: array<Spot, 4>;

fn gather(world_position: vec4<f32>, n: vec3<f32>, frag_coord: vec4<f32>, ortho: bool) -> u32 {
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), world_position);
    let cluster_index = clustering::view_fragment_cluster_index(frag_coord.xy, view_z, ortho);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    var count = 0u;
    for (var i: u32 = ranges.first_spot_light_index_offset; i < ranges.first_reflection_probe_index_offset; i = i + 1u) {
        if (count >= 4u) {
            break;
        }
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
        // Only inside the spot's cone: outside it (or behind the light) the shadow map has nothing for this
        // fragment and the lookup reads another texel's depth.
        var spot_dir = vec3<f32>((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
        spot_dir.y = sqrt(max(0.0, 1.0 - spot_dir.x * spot_dir.x - spot_dir.z * spot_dir.z));
        if (((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u) {
            spot_dir.y = -spot_dir.y;
        }
        let c = dot(-to / d, spot_dir);
        let t = (*light).spot_light_tan_angle * 0.95;
        if (c <= 0.05 || c * c * (1.0 + t * t) < 1.0) {
            continue;
        }
        var s: Spot;
        s.pos = (*light).position_radius.xyz;
        s.range = r;
        s.colour = (*light).color_inverse_square_range.rgb * DECODE;
        s.shadow = shadows::fetch_spot_shadow(light_id, world_position, n, (*light).shadow_map_near_z, frag_coord.xy);
        // Soft at the cone's rim (the spot narrows and widens as it follows its casters).
        let rim = clamp((c * c * (1.0 + t * t) - 1.0) * 40.0, 0.0, 1.0);
        s.shadow = mix(1.0, s.shadow, rim);
        spots[count] = s;
        count = count + 1u;
    }
    return count;
}
