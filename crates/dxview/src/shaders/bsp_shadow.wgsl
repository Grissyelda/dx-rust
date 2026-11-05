// The lit BSP with real-time shadows over its lightmap (shadowlight.rs). Otherwise Bevy's StandardMaterial
// fragment shader. Each shadow-mapped point light in the fragment's cluster is one of shadowlight's lights: its
// colour x intensity carries the Unreal light's colour (x ENCODE), its range the Unreal radius. Where its shadow
// map says the texel is blocked, the light's own term of the bake comes off the lightmap - in gamma space, as
// dx-level's bake summed it: colour x (1 - d / r) x |N.L| x LIGHT_GAIN.
//
// BSP_SPOT_SHADOWS and BSP_RELIGHT (shadowlight.rs, BspShadowKey) compile the spot-shadow walk and remastered
// lighting in only for materials that use them: the branches that guard them are uniform per material, so a material
// without them never ran them, but the shader carried their code. BSP_CLASSIC_LIGHTS (Lean Rendering) lights the
// surface with classic_lighting below instead of Bevy's apply_pbr_lighting. BSP_DETAIL samples the detail texture only
// where the surface has one and Detail Textures is on, the only places it was ever used. BSP_NO_DYNLIGHTS leaves the
// Classic lights' walk out where no dynamic light can reach (the sky room's faces), and BSP_SPOT_LIGHTS puts its spot
// cone in only with Flashlight Cone on.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
}
#import dxview::shadow_spots as spot_shadows

struct BspShadowParams {
    p: vec4<f32>,
    q: vec4<f32>,
    r: vec4<f32>,
    s: vec4<f32>,
    u: vec4<f32>,
}

#ifdef BINDLESS
// Bindless (shadowlight.rs): the material's slot picks its params and textures out of shared arrays. `bsp` and the
// texture indices are set once at the top of the fragment shader.
#import bevy_render::bindless::{bindless_textures_2d, bindless_samplers_filtering}
#import bevy_pbr::mesh_bindings::mesh

struct BspShadowIndices {
    params: u32,        // 100
    mask: u32,          // 101
    mask_sampler: u32,  // 102
    lights: u32,        // 103
    detail: u32,        // 104
    detail_sampler: u32, // 105
}
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<storage> bsp_indices: array<BspShadowIndices>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var<storage> bsp_params: array<BspShadowParams>;
var<private> bsp: BspShadowParams;
var<private> bsp_ix: BspShadowIndices;

fn bsp_load(instance_index: u32) {
    let slot = mesh[instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    bsp_ix = bsp_indices[slot];
    bsp = bsp_params[bsp_ix.params];
}
fn mask_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(bindless_textures_2d[bsp_ix.mask], bindless_samplers_filtering[bsp_ix.mask_sampler], uv, 0.0);
}
fn lights_load(c: vec2<i32>) -> vec4<f32> {
    return textureLoad(bindless_textures_2d[bsp_ix.lights], c, 0);
}
fn detail_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(bindless_textures_2d[bsp_ix.detail], bindless_samplers_filtering[bsp_ix.detail_sampler], uv);
}
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> bsp: BspShadowParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var relight_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var relight_mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var relight_lights: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var detail_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var detail_tex_sampler: sampler;

fn bsp_load(instance_index: u32) {}
fn mask_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(relight_mask, relight_mask_sampler, uv, 0.0);
}
fn lights_load(c: vec2<i32>) -> vec4<f32> {
    return textureLoad(relight_lights, c, 0);
}
fn detail_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(detail_tex, detail_tex_sampler, uv);
}
#endif

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
    let vis = mask_sample(uv_b);
    let gain = bsp.p.z;
    for (var c: u32 = 0u; c < 3u; c = c + 1u) {
        let idf = ids[c];
        if (idf < 0.75) {
            continue;
        }
        let id = i32(round(idf) - 1.0);
        let lp = lights_load(vec2<i32>(id * 2, 0));
        let lc = lights_load(vec2<i32>(id * 2 + 1, 0)).rgb;
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
        // (The peak is capped: a mirror-smooth surface (glass, r 0.08) peaked at ~800 and blew out under bloom.)
        spec = spec + lc * reach * mapped * min(ggx(ndh, bsp.r.z), 12.0) * f * gain * 0.25;
    }
    s = max(s, vec3<f32>(0.0)) * bsp.s.x + vec3<f32>(bsp.s.y);
    var out: Relit;
    out.light = from_srgb(s);
    out.spec = min(spec, vec3<f32>(1.2));
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

#ifdef BSP_CLASSIC_LIGHTS
// Lean Rendering's Classic lights (shadowlight.rs, classic_lights): the lit surface as Unreal 1 drew it, in place of
// Bevy's apply_pbr_lighting, whose loop over the dynamic lights held this shader at 96 registers. The lightmap carries
// the level's own lights; each dynamic light (dynlight.rs, marked by its Unreal brightness, negated, in the depth bias)
// is added to it as dx-level's bake added the static ones: colour x brightness x (1 - d / r) x N.L x LIGHT_GAIN, in the
// bake's gamma space, diffuse only. Unmarked lights (shadowlight's, Light Shafts') skip a lightmapped surface's diffuse
// in Bevy's lighting too, so they add nothing here. Directional lights (lightning) add their diffuse in Bevy's units.
// Where no light reaches, this is apply_pbr_lighting's lightmapped surface: exposure x lightmap x diffuse + emissive.
// The walk over the cluster's lights is compiled in only where a dynamic light can reach (not the sky room's faces:
// BspShadowKey), and its spot cone only with Flashlight Cone on (BSP_SPOT_LIGHTS); the surface's colour is worked out
// after it, so the walk holds fewer values live.
fn classic_lighting(in: PbrInput) -> vec4<f32> {
    let base = in.material.base_color;
#ifndef BSP_NO_DYNLIGHTS
    let view_z = dot(vec4<f32>(
        view_bindings::view.view_from_world[0].z,
        view_bindings::view.view_from_world[1].z,
        view_bindings::view.view_from_world[2].z,
        view_bindings::view.view_from_world[3].z
    ), in.world_position);
    let cluster_index = clustering::view_fragment_cluster_index(in.frag_coord.xy, view_z, in.is_orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
#endif
    var added = vec3<f32>(0.0);
#ifndef BSP_NO_DYNLIGHTS
    for (var i: u32 = ranges.first_point_light_index_offset;
            i < ranges.first_reflection_probe_index_offset;
            i = i + 1u) {
        let light = &view_bindings::clustered_lights.data[clustering::get_clusterable_object_id(i)];
        let brightness = -(*light).shadow_depth_bias;
        if (brightness <= 0.0 ||
            ((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE_BIT) == 0u) {
            continue;
        }
        let to = (*light).position_radius.xyz - in.world_position.xyz;
        let d = length(to);
        let r = (*light).range;
        if (d >= r || d < 1e-4) {
            continue;
        }
        let l = to / d;
        var reach = (1.0 - d / r) * max(dot(in.N, l), 0.0);
#ifdef BSP_SPOT_LIGHTS
        if (((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_BIT) != 0u) {
            // Bevy's cone (spot_light in pbr_lighting.wgsl): the flashlight's, with Flashlight Cone on.
            var spot_dir = vec3<f32>((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
            spot_dir.y = sqrt(max(0.0, 1.0 - spot_dir.x * spot_dir.x - spot_dir.z * spot_dir.z));
            if (((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u) {
                spot_dir.y = -spot_dir.y;
            }
            let cone = saturate(dot(-l, spot_dir) * (*light).light_custom_data.z + (*light).light_custom_data.w);
            reach = reach * cone * cone;
        }
#endif
        // The colour: Bevy's colour x intensity scaled to its brightest channel (hsv_white's 1), back in gamma space.
        let rgb = (*light).color_inverse_square_range.rgb;
        added += to_srgb(rgb / max(max(rgb.r, rgb.g), max(rgb.b, 1e-8))) * (brightness * reach);
    }
#endif // BSP_NO_DYNLIGHTS
    var lm = in.lightmap_light;
    if (max(added.r, max(added.g, added.b)) > 0.0) {
        lm = from_srgb(to_srgb(lm) + added * bsp.p.z);
    }
    let diffuse_color = base.rgb * (1.0 - in.material.metallic) * (1.0 - in.material.specular_transmission) *
        (1.0 - in.material.diffuse_transmission);
    var direct = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let dl = &view_bindings::lights.directional_lights[i];
        if (((*dl).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE_BIT) != 0u) {
            direct += diffuse_color * (*dl).color.rgb * (max(dot(in.N, (*dl).direction_to_light), 0.0) / 3.14159265);
        }
    }
    let emissive_light = in.material.emissive.rgb * base.a *
        mix(1.0, view_bindings::view.exposure, in.material.emissive.a);
    return vec4<f32>(view_bindings::view.exposure * (direct + lm * diffuse_color) + emissive_light, base.a);
}
#endif // BSP_CLASSIC_LIGHTS

// The surface's own share of one shadow light at this texel, as the bake had it: when the light is one of the
// surface's three strongest (remastered lighting's ids, in the vertex colour) the bake's per-texel reach of it -
// 0 where the map's own geometry already shadowed it, so a guard's shadow never darkens a wall twice.
fn baked_reach(lp: vec3<f32>, ids: vec4<f32>, uv_b: vec2<f32>) -> f32 {
    var vis = vec4<f32>(-1.0);
    for (var c: u32 = 0u; c < 3u; c = c + 1u) {
        let idf = ids[c];
        if (idf < 0.75) {
            continue;
        }
        let id = i32(round(idf) - 1.0);
        let tp = lights_load(vec2<i32>(id * 2, 0));
        if (distance(tp.xyz, lp) < 0.05) {
            if (vis.x < -0.5) {
                vis = mask_sample(uv_b);
            }
            return vis[c];
        }
    }
    return -1.0;
}

fn shadowed_lightmap(lm: vec3<f32>, world_position: vec4<f32>, n: vec3<f32>, frag_coord: vec4<f32>, ortho: bool, ids: vec4<f32>, uv_b: vec2<f32>, has_ids: bool) -> vec3<f32> {
    let count = spot_shadows::gather(world_position, n, frag_coord, ortho);
    let s = to_srgb(lm);
    var removed = vec3<f32>(0.0);
    var total = vec3<f32>(0.0);
    var dbg_own = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let sp = spot_shadows::spots[i];
        let to = sp.pos - world_position.xyz;
        let d = length(to);
        var own = sp.colour * (1.0 - d / sp.range) * abs(dot(to / d, n)) * bsp.p.z;
        // Only take away light the bake actually had: its reach of this light here, when it knows it...
        var gate = -1.0;
        if (has_ids) {
            gate = baked_reach(sp.pos, ids, uv_b);
        }
        if (gate >= 0.0) {
            own = own * gate;
        } else if (max(s.r, max(s.g, s.b)) < 0.6 * max(own.r, max(own.g, own.b)) && bsp.p.w < 1.5) {
            // ...else a texel the lightmap shows dark for this light (a wall between them) has nothing to lose.
            continue;
        }
        total += own;
        removed += own * (1.0 - sp.shadow);
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
    if (max(removed.r, max(removed.g, removed.b)) <= 0.0) {
        return lm;
    }
    // Each light's share of what the texel really holds: where the lights' terms add up to more than the lightmap
    // (lights the bake clipped, overlapping pools), they're scaled down to it, so a shadow is never darker than
    // the light it blocks.
    let share = min(vec3<f32>(1.0), s / max(total, vec3<f32>(1e-4)));
    removed = removed * share;
    // Never quite black: what's left of the texel stands in for light bounced about.
    let left = max(s - removed * bsp.p.x, s * (1.0 - bsp.p.x));
    return from_srgb(left);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    bsp_load(in.instance_index);
    // Panning textures (PF_AutoUPan / VPan) on the GPU's clock.
    var vin = in;
#ifdef VERTEX_UVS_A
    vin.uv = in.uv + fract(bsp.u.xy * view_bindings::globals.time);
#endif
    var pbr_input = pbr_input_from_standard_material(vin, is_front);
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
#ifdef VERTEX_UVS_A
#ifdef BSP_DETAIL
    // Detail textures (the original renderers' DetailTexture pass): within 380 units the surface is modulated by
    // its detail picture (grey = no change, at half contrast), fading out by 760 units. q.y = the detail's UV scale;
    // compiled in only where it's above 0 with Detail Textures on (q.z), BspShadowKey.
    let detail = detail_sample(vin.uv * bsp.q.y);
    let units = length(view_bindings::view.world_position.xyz - pbr_input.world_position.xyz) * 52.5;
    let a = clamp(2.0 - units / 380.0, 0.0, 1.0);
    let d = (detail.rgb - 0.5) * 0.5 + 1.0;
    pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * mix(vec3<f32>(1.0), d, a), pbr_input.material.base_color.a);
#endif
#endif
#ifdef BSP_RELIGHT
    var spec = vec3<f32>(0.0);
    var unlit_glow = 0.0;
#endif
#ifdef LIGHTMAP
#ifdef BSP_SPOT_SHADOWS
    if (bsp.p.x > 0.0) {
        var ids = vec4<f32>(0.0);
        var has_ids = false;
#ifdef VERTEX_COLORS
        ids = in.color;
        has_ids = true;
#endif
        pbr_input.lightmap_light = shadowed_lightmap(
            pbr_input.lightmap_light,
            pbr_input.world_position,
            pbr_input.world_normal,
            pbr_input.frag_coord,
            pbr_input.is_orthographic,
            ids,
            in.uv_b,
            has_ids,
        );
    }
#endif // BSP_SPOT_SHADOWS
#endif
#ifdef LIGHTMAP
#ifdef VERTEX_COLORS
#ifdef BSP_RELIGHT
    if (bsp.r.x > 0.5) {
        // Surfaces drawn full bright (PF_Unlit: lamps, screens, signs) have a white lightmap: they glow instead.
        // (Only faces the bake gave no lights: a face lit brightly enough to saturate its lightmap - Gunther's cell
        // window under the cell's lamps - isn't a lamp, and glowing it blew out to a white sheet under bloom.)
        if (min(pbr_input.lightmap_light.r, min(pbr_input.lightmap_light.g, pbr_input.lightmap_light.b)) > 0.98 && in.color.x < 0.75) {
            unlit_glow = bsp.s.w;
        } else {
            let v = normalize(view_bindings::view.world_position.xyz - pbr_input.world_position.xyz);
            let r = relight(pbr_input.lightmap_light, pbr_input.world_position.xyz, pbr_input.world_normal, mapped_n, in.color, in.uv_b, v);
            pbr_input.lightmap_light = r.light;
            spec = r.spec * bsp.r.y;
        }
    }
#endif // BSP_RELIGHT
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
#ifdef BSP_CLASSIC_LIGHTS
        out.color = classic_lighting(pbr_input);
#else
        out.color = apply_pbr_lighting(pbr_input);
#endif
    } else {
        out.color = pbr_input.material.base_color;
    }
    // Highlights (metal tints them by its colour) and glow (bloom picks up what goes past white). Without remastered
    // lighting there's no highlight and no unlit glow, only the named glowing textures' (s.z, 0 or more).
    let base = pbr_input.material.base_color.rgb;
#ifdef BSP_RELIGHT
    let tint = mix(vec3<f32>(1.0), base * 2.0, bsp.r.w);
    out.color = vec4<f32>(out.color.rgb + spec * tint, out.color.a);
    let lum = dot(base, vec3<f32>(0.299, 0.587, 0.114));
    let glow = max(bsp.s.z, unlit_glow * smoothstep(0.35, 0.85, lum));
#else
    let glow = bsp.s.z;
#endif
    out.color = vec4<f32>(out.color.rgb + base * glow, out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
