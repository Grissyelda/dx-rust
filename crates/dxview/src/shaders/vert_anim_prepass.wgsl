// The prepass / shadow-map version of vert_anim.wgsl: the same GPU vertex animation, so animated meshes cast
// shadows in their current pose (shadowlight.rs, the flashlight cone). Otherwise Bevy's prepass vertex shader for an
// unskinned mesh.

#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    prepass_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

#ifdef BINDLESS
#import bevy_render::bindless::bindless_textures_2d

struct VertAnimIndices {
    hi: u32,
    lo: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage> anim_indices: array<VertAnimIndices>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var anim_hi: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var anim_lo: texture_2d<f32>;
#endif

fn key(texel: u32, ii: u32) -> vec3<f32> {
    let c = vec2<i32>(i32(texel % 4096u), i32(texel / 4096u));
#ifdef BINDLESS
    let slot = mesh[ii].material_and_lightmap_bind_group_slot & 0xffffu;
    let ix = anim_indices[slot];
    return textureLoad(bindless_textures_2d[ix.hi], c, 0).xyz + textureLoad(bindless_textures_2d[ix.lo], c, 0).xyz;
#else
    return textureLoad(anim_hi, c, 0).xyz + textureLoad(anim_lo, c, 0).xyz;
#endif
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let ii = vertex.instance_index;
    let world_from_local = mesh_functions::get_world_from_local(ii);

    var local = vertex.position;
#ifdef VERTEX_UVS_B
    let tag = mesh_functions::get_tag(ii);
    if (tag != 0xffffffffu) {
        let a = tag & 0x7ffu;
        let b = (tag >> 11u) & 0x7ffu;
        let t = f32(tag >> 22u) / 1023.0;
        let first = u32(vertex.uv_b.x);
        let nv = u32(vertex.uv_b.y);
        let pa = key(first + a * nv, ii);
        let pb = key(first + b * nv, ii);
        local = pa + (pb - pa) * t;
    }
#endif

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, ii);
#endif
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.previous_world_position = out.world_position;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = ii;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(ii, world_from_local[3]);
#endif
    return out;
}
