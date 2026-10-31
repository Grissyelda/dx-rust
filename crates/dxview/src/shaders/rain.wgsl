// Rain (weather.rs): each quad's corners are built here from two data texels - head (xyz, brightness) and tail
// (xyz, half width), Bevy world metres - as a streak facing the camera. POSITION carries (quad, corner).

#import bevy_pbr::{
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var data: texture_2d<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) g: f32,
};

fn texel(i: u32) -> vec4<f32> {
    let w = textureDimensions(data).x;
    return textureLoad(data, vec2<i32>(i32(i % w), i32(i / w)), 0);
}

@vertex
fn vertex(v: Vertex) -> Out {
    var o: Out;
    let q = u32(v.position.x + 0.5);
    let corner = u32(v.position.y + 0.5);
    let a = texel(q * 2u);
    let b = texel(q * 2u + 1u);
    o.uv = v.uv;
    o.g = a.w;
    let head = a.xyz;
    let tail = b.xyz;
    let along = head - tail;
    if (a.w <= 0.0 || dot(along, along) < 1e-12) {
        o.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return o;
    }
    let dir = normalize(along);
    let to_eye = normalize(view.world_position - head);
    var side = cross(dir, to_eye);
    if (dot(side, side) < 1e-10) {
        side = vec3<f32>(1.0, 0.0, 0.0);
    }
    side = normalize(side) * b.w;
    var p = head - side;
    if (corner == 1u) {
        p = head + side;
    } else if (corner == 2u) {
        p = tail + side;
    } else if (corner == 3u) {
        p = tail - side;
    }
    o.clip = position_world_to_clip(p);
    return o;
}

@fragment
fn fragment(i: Out) -> @location(0) vec4<f32> {
    let c = textureSample(tex, smp, i.uv);
    return vec4<f32>(c.rgb * i.g, c.a);
}
