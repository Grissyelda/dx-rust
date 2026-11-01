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
    r: vec4<f32>,
    s: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> bsp: BspShadowParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var relight_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var relight_mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var relight_lights: texture_2d<f32>;

// Remastered lighting (relight.rs): the lightmap relit per pixel by the surface's three strongest lights (their ids
// ride in the vertex colour, their reach per texel in the mask atlas, both from dx-level's bake). Each light's baked
// term - colour x (1 - d/r) x |N.L| over the flat face - is swapped for the same term over the normal-mapped
// face, so bricks and planks catch the light while every shadow of the bake stays; a highlight per light by the
// material's roughness comes on top. Returns (relit lightmap, highlight).
struct Relit {
    light: vec3<f32>,
    spec: vec3<f32>,
}

fn ggx(ndh: f32, rough: f32) -> f32 {
    let a = max(rough * rough, 0.02);
    let a2 = a * a;
    let d = ndh * ndh * (a2 - 1.0) + 1.0;
    return a2 / (3.14159 * d * d);
}

fn relight(lm: vec3<f32>, wp: vec3<f32>, flat_n: vec3<f32>, mapped_n: vec3<f32>, ids: vec4<f32>, uv_b: vec2<f32>, v: vec3<f32>) -> Relit {
    var s = to_srgb(lm);
    var spec = vec3<f32>(0.0);
    let vis = textureSampleLevel(relight_mask, relight_mask_sampler, uv_b, 0.0);
    let gain = bsp.p.z;
    for (var c: u32 = 0u; c < 3u; c = c + 1u) {
        let idf = ids[c];
        if (idf < 0.75) {
            continue;
        }
        let id = i32(round(idf) - 1.0);
        let lp = textureLoad(relight_lights, vec2<i32>(id * 2, 0), 0);
        let lc = textureLoad(relight_lights, vec2<i32>(id * 2 + 1, 0), 0).rgb;
        let to = lp.xyz - wp;
        let d = length(to);
        if (d >= lp.w || d < 1e-4) {
            continue;
        }
        let l = to / d;
        let reach = (1.0 - d / lp.w) * vis[c];
        let fd = dot(flat_n, l);
        // The bake lit both sides of a face (|N.L|): lit from behind, mirror the mapped normal through the face.
        var m = mapped_n;
        if (fd < 0.0) {
            m = m - 2.0 * dot(m, flat_n) * flat_n;
        }
        let mapped = max(dot(m, l * sign(fd + 1e-6)), 0.0);
        s = s + lc * reach * (mapped - abs(fd)) * gain;
        // Highlight: GGX with a Schlick Fresnel (metal tints it by... the caller), energy kept modest.
        let h = normalize(l * sign(fd + 1e-6) + v);
        let ndh = max(dot(m, h), 0.0);
        let f = 0.04 + 0.96 * pow(1.0 - max(dot(h, v), 0.0), 5.0);
        spec = spec + lc * reach * mapped * ggx(ndh, bsp.r.z) * f * gain * 0.25;
    }
    s = max(s, vec3<f32>(0.0)) * bsp.s.x + vec3<f32>(bsp.s.y);
    var out: Relit;
    out.light = from_srgb(s);
    out.spec = spec;
    return out;
}

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
        // Only inside the spot's cone: outside it (or behind the light) the shadow map has nothing for this
        // fragment and the lookup reads another texel's depth (black triangles all over the level).
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
#ifdef VERTEX_COLORS
    // The vertex colour carries remastered lighting's light ids, not a tint: StandardMaterial multiplied the
    // colour by it, so divide it back out (the ids are never 0).
    pbr_input.material.base_color = pbr_input.material.base_color / max(in.color, vec4<f32>(0.25));
#endif
    // The normal-mapped normal (relief maps), kept for remastered lighting before...
    let mapped_n = normalize(pbr_input.N);
    // ...the face's own normal, not the normal prepass's (Bevy loads that when MSAA is off; a texel the prepass
    // didn't cover would light the face from the wrong side).
    pbr_input.N = normalize(pbr_input.world_normal);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var spec = vec3<f32>(0.0);
    var unlit_glow = 0.0;
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
#ifdef LIGHTMAP
#ifdef VERTEX_COLORS
    if (bsp.r.x > 0.5) {
        // Surfaces drawn full bright (PF_Unlit: lamps, screens, signs) have a white lightmap: they glow instead.
        if (min(pbr_input.lightmap_light.r, min(pbr_input.lightmap_light.g, pbr_input.lightmap_light.b)) > 0.98) {
            unlit_glow = bsp.s.w;
        } else {
            let v = normalize(view_bindings::view.world_position.xyz - pbr_input.world_position.xyz);
            let r = relight(pbr_input.lightmap_light, pbr_input.world_position.xyz, pbr_input.world_normal, mapped_n, in.color, in.uv_b, v);
            pbr_input.lightmap_light = r.light;
            spec = r.spec * bsp.r.y;
        }
    }
#endif
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
    // Highlights (metal tints them by its colour) and glow (bloom picks up what goes past white).
    let base = pbr_input.material.base_color.rgb;
    let tint = mix(vec3<f32>(1.0), base * 2.0, bsp.r.w);
    out.color = vec4<f32>(out.color.rgb + spec * tint, out.color.a);
    let lum = dot(base, vec3<f32>(0.299, 0.587, 0.114));
    let glow = max(bsp.s.z, unlit_glow * smoothstep(0.35, 0.85, lum));
    out.color = vec4<f32>(out.color.rgb + base * glow, out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
