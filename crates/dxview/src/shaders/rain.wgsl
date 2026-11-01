// Rain (weather.rs), moved on the GPU. Each quad is one drop or splashlet written once by the CPU into three data
// texels: start position (Bevy metres) + start time, velocity (m/s) + end time (drops) or life (splashlets), and
// length / brightness / column top (drops) or the surface's plane (splashlets). The shader works out where it is
// now on Bevy's clock (globals.time) and builds the camera-facing streak. POSITION carries (quad, corner).

#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var data: texture_2d<f32>;
// x: 0 drops / 1 splashlets, y z: lean (player velocity x 0.2, Bevy x / z), w: gravity (m/s^2).
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> params: vec4<f32>;

// Unreal units in a Bevy metre (dx_level::METERS_PER_UNIT = 1 / 52.5).
const UU: f32 = 52.5;

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

fn hidden() -> Out {
    var o: Out;
    o.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    o.uv = vec2<f32>(0.0);
    o.g = 0.0;
    return o;
}

@vertex
fn vertex(v: Vertex) -> Out {
    let q = u32(v.position.x + 0.5);
    let corner = u32(v.position.y + 0.5);
    let a = texel(q * 3u);
    let b = texel(q * 3u + 1u);
    let c = texel(q * 3u + 2u);
    let now = globals.time;
    let t = now - a.w;
    var head: vec3<f32>;
    var tail: vec3<f32>;
    var g: f32;
    var half: f32;
    if (params.x < 0.5) {
        // A drop: straight down its line until it ends (lands, falls into water, or its life runs out).
        if (t < 0.0 || now >= b.w || c.y <= 0.0) {
            return hidden();
        }
        head = a.xyz + b.xyz * t;
        let dir = normalize(b.xyz - vec3<f32>(params.y, 0.0, params.z));
        // Far streaks get wider and dimmer by the same amount (the same light on screen) instead of thinning to
        // flickering sub-pixel lines.
        let spread = max(length(view.world_position - head) * UU / 400.0, 1.0);
        tail = head - dir * c.x;
        // Fading in under the column's top (over 700 units) hides where the rain begins.
        let under = clamp((c.z - head.y) * UU / 700.0, 0.0, 1.0);
        g = c.y * 0.55 / spread * under * under;
        half = 0.35 * spread / UU;
    } else {
        // A splashlet: thrown out of the impact and falling back; gone when back down to the surface.
        if (t < 0.0 || t >= b.w || b.w <= 0.0) {
            return hidden();
        }
        let gv = vec3<f32>(0.0, -params.w, 0.0);
        head = a.xyz + b.xyz * t + 0.5 * gv * t * t;
        let vel = b.xyz + gv * t;
        let n = c.xyz;
        let above = (dot(head, n) - c.w) * UU;
        if (above <= 0.4) {
            return hidden();
        }
        let speed = max(length(vel) * UU, 1.0);
        let dir = vel * UU / speed;
        var len = clamp(speed * 0.025, 1.0, 4.0);
        // The tail never reaches under the surface it was thrown off.
        let fall = -dot(dir, n);
        if (fall > 0.0) {
            len = min(len, max(above - 0.3, 0.0) / fall);
        }
        tail = head - dir * (len / UU);
        g = clamp(1.0 - t / b.w, 0.0, 1.0) * 0.5;
        half = 0.35 / UU;
    }
    let along = head - tail;
    if (g <= 0.0 || dot(along, along) < 1e-12) {
        return hidden();
    }
    let dir = normalize(along);
    let to_eye = normalize(view.world_position - head);
    var side = cross(dir, to_eye);
    if (dot(side, side) < 1e-10) {
        side = vec3<f32>(1.0, 0.0, 0.0);
    }
    side = normalize(side) * half;
    var p = head - side;
    if (corner == 1u) {
        p = head + side;
    } else if (corner == 2u) {
        p = tail + side;
    } else if (corner == 3u) {
        p = tail - side;
    }
    var o: Out;
    o.uv = v.uv;
    o.g = g;
    o.clip = position_world_to_clip(p);
    return o;
}

@fragment
fn fragment(i: Out) -> @location(0) vec4<f32> {
    let c = textureSample(tex, smp, i.uv);
    return vec4<f32>(c.rgb * i.g, c.a);
}
