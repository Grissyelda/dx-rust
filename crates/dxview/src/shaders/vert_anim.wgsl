// Unreal vertex animation posed on the GPU (see gpuanim.rs). Keyframes (already in Bevy axes and scale) live
// in two Rgba16Float textures, value + remainder, 4096 texels per row; UV_1 = (this vertex's texel in frame 0,
// vertices per frame). The MeshTag packs the pose: frame a | frame b << 11 | t * 1023 << 22, or all ones for
// "use the mesh's own (CPU-posed) positions". Otherwise identical to Bevy's mesh vertex shader for an unskinned
// mesh with normals, UVs and vertex colours.

#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::VertexOutput,
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

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
};

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

#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, ii);
#endif
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = ii;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(ii, world_from_local[3]);
#endif
    return out;
}
