// Screen-Space Reflections (ssr.rs, setting Reflections): polished floors, tiles, metal and wet surfaces reflect
// what's on screen. Runs on the world camera after its opaque and transparent passes.
//
// Which pixels reflect, and how much, comes from the picture itself: BSP surfaces with a reflectivity write
// alpha = 1 - reflectivity / 2 (bsp_shadow.wgsl), so 0.5 .. 1. Anything else keeps 1 (or 0 where nothing drew),
// and blended things drawn over a floor push the alpha back up toward 1 (they cover it).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var normal_texture: texture_2d<f32>;
@group(0) @binding(4) var<uniform> view: View;
struct Settings {
    // x strength, y steps, z max distance (m), w debug (1 = the reflection alone)
    a: vec4<f32>,
}
@group(0) @binding(5) var<uniform> settings: Settings;

fn view_z(ndc_z: f32) -> f32 {
    // Bevy's reverse-Z infinite perspective: the view-space z (negative in front of the eye).
    return -view.clip_from_view[3][2] / max(ndc_z, 1e-7);
}

fn project(q: vec3<f32>) -> vec4<f32> {
    let c = view.clip_from_world * vec4<f32>(q, 1.0);
    if (c.w <= 1e-4) {
        return vec4<f32>(0.0);
    }
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let inside = select(0.0, 1.0, all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)));
    return vec4<f32>(uv, ndc.z, inside);
}

fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(depth_texture));
    let p = clamp(vec2<i32>(uv * size), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(depth_texture, p, 0);
}

fn edge_fade(uv: vec2<f32>) -> f32 {
    let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    return smoothstep(0.0, 0.08, e);
}

fn scene_color(uv: vec2<f32>, blur: f32) -> vec3<f32> {
    var c = textureSampleLevel(screen_texture, screen_sampler, uv, 0.0).rgb;
    if (blur > 0.0) {
        let d = blur / vec2<f32>(textureDimensions(screen_texture));
        c = c * 0.36
            + textureSampleLevel(screen_texture, screen_sampler, uv + vec2<f32>(d.x, d.y * 0.5), 0.0).rgb * 0.16
            + textureSampleLevel(screen_texture, screen_sampler, uv - vec2<f32>(d.x, d.y * 0.5), 0.0).rgb * 0.16
            + textureSampleLevel(screen_texture, screen_sampler, uv + vec2<f32>(-d.x * 0.5, d.y), 0.0).rgb * 0.16
            + textureSampleLevel(screen_texture, screen_sampler, uv + vec2<f32>(d.x * 0.5, -d.y), 0.0).rgb * 0.16;
    }
    // Lamps and coronas picked up by a ray are kept in the range of a lit wall (no sparks under bloom).
    let l = max(max(c.r, c.g), c.b);
    return c * min(1.0, 1.5 / max(l, 1e-4));
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(screen_texture, screen_sampler, in.uv, 0.0);
    // Not a reflecting surface: as it was.
    if (c.a < 0.5 || c.a > 0.995) {
        return vec4<f32>(c.rgb, 1.0);
    }
    let k = (1.0 - c.a) * 2.0;
    let d = depth_at(in.uv);
    if (d <= 0.0) {
        return vec4<f32>(c.rgb, 1.0);
    }
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let wp = view.world_from_clip * vec4<f32>(ndc, d, 1.0);
    let p0 = wp.xyz / wp.w;
    let n = normalize(textureLoad(normal_texture, vec2<i32>(in.uv * vec2<f32>(textureDimensions(normal_texture))), 0).xyz * 2.0 - 1.0);
    let to_eye = view.world_position.xyz - p0;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);
    if (dot(n, v) <= 0.02) {
        return vec4<f32>(c.rgb, 1.0);
    }
    let r = reflect(-v, n);
    // Steps grow out from the surface (jittered per pixel so the step pattern doesn't band).
    let steps = i32(settings.a.y);
    let max_d = settings.a.z;
    var s = (0.08 + dist * 0.006) * (0.75 + 0.5 * hash(in.position.xy));
    var prev = 0.0;
    var hit = vec4<f32>(0.0);
    let start = p0 + n * 0.02;
    for (var i = 0; i < steps; i += 1) {
        if (s > max_d) {
            break;
        }
        let q = project(start + r * s);
        if (q.w < 0.5) {
            break;
        }
        let scene = depth_at(q.xy);
        if (scene > 0.0) {
            let ray_z = view_z(q.z);
            let scene_z = view_z(scene);
            if (ray_z < scene_z) {
                if (scene_z - ray_z > 0.6 + s * 0.08) {
                    // Passed behind something thick: the picture doesn't hold what's there.
                    break;
                }
                var a = prev;
                var b = s;
                for (var j = 0; j < 4; j += 1) {
                    let m = 0.5 * (a + b);
                    let qm = project(start + r * m);
                    let sm = depth_at(qm.xy);
                    if (sm > 0.0 && view_z(qm.z) < view_z(sm)) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                let qh = project(start + r * b);
                // Rays heading back toward the eye would see the backs of things: fade them.
                let facing = smoothstep(-0.25, 0.1, dot(r, -v));
                let sure = 1.0 - smoothstep(max_d * 0.6, max_d, b);
                // Rougher surfaces (less reflective) and longer rays blur the reflection.
                let blur = clamp((1.0 - k) * 3.0 + b * 0.15, 0.0, 6.0);
                hit = vec4<f32>(scene_color(qh.xy, blur), edge_fade(qh.xy) * facing * sure);
                break;
            }
        }
        prev = s;
        s *= 1.3;
    }
    if (settings.a.w > 0.5) {
        return vec4<f32>(hit.rgb * hit.a, 1.0);
    }
    // Schlick's Fresnel (grazing views reflect more), scaled by how polished the surface is.
    let fres = 0.04 + 0.96 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
    let amount = clamp(k * settings.a.x * mix(0.35, 1.0, fres) * hit.a, 0.0, 0.85);
    // The reflection replaces part of the surface's own light (a mirror-like floor shows less of its own colour).
    let out = c.rgb * (1.0 - amount * 0.45) + hit.rgb * amount;
    return vec4<f32>(out, 1.0);
}
