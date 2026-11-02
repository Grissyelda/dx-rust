// Generated trees and bushes (trees.rs) swaying in the wind: UV_1 carries how far out along the plant a vertex is
// (0 at the foot, 1 at the outermost twigs) and a phase. The trunk barely moves, the crown leans with the gusts,
// the leaves flutter a little more on top. Otherwise Bevy's mesh vertex shader.
#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::VertexOutput,
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
}

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

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let ii = vertex.instance_index;
    let world_from_local = mesh_functions::get_world_from_local(ii);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
#ifdef VERTEX_UVS_B
    let flex = vertex.uv_b.x;
    let phase = vertex.uv_b.y * 6.2831853;
    let t = globals.time;
    let root = world_from_local[3].xyz;
    // A gust passing over (the same breeze as the grass: along +x +z), the crown's slow lean and the leaves' flutter.
    let d = normalize(vec2<f32>(0.8, 0.6));
    let along = dot(root.xz, d);
    let gust = 0.5 + 0.5 * sin(6.2831853 * (along - 3.0 * t) / 9.0);
    let lean = (0.35 + 0.65 * gust * gust) * (0.6 + 0.4 * sin(t * 0.9 + along));
    let height = max(wp.y - root.y, 0.0);
    let k = flex * flex * min(height, 8.0) * 0.022;
    var off = vec3<f32>(d.x, 0.0, d.y) * lean * k;
    off = off + vec3<f32>(sin(t * 4.3 + phase), 0.4 * sin(t * 5.1 + phase * 1.7), cos(t * 3.7 + phase)) * 0.012 * flex * flex;
    wp = vec4<f32>(wp.xyz + off, wp.w);
#endif
    out.world_position = wp;
    out.position = position_world_to_clip(wp.xyz);
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, ii);
#endif
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
