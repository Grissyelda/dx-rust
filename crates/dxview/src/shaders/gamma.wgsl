// Display brightness, the way Deus Ex's renderer brightness slider works: a gamma curve on the final image
// (lifts the darks a lot, the brights a little). The screen texture holds linear values.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
struct Settings {
    gamma: f32,
    fog_r: f32,
    fog_g: f32,
    fog_b: f32,
    nv: f32,
    nv_t: f32,
    nv_ir: f32,
    nv_box: f32,
}
@group(0) @binding(2) var<uniform> settings: Settings;

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Modern Vision Augmentation: an image intensifier. The light is amplified many times (the darks come up, anything
// lit blows out), bright lights flare into the tube around them, the picture is green phosphor with grain that
// thickens in the dark, a soft round edge; warm bodies (drawn white-hot by vision.rs) glow.
fn night_vision(uv: vec2<f32>, c: vec3<f32>) -> vec3<f32> {
    // In display terms (what the eye sees), amplified about four times.
    let gain = 4.0 * settings.nv;
    let l = pow(max(lum(c), 0.0), 1.0 / 2.2);
    // Flare: the brightest of a ring of taps around, spilled outward.
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let px = 1.0 / dims.y;
    var flare = 0.0;
    for (var i = 0; i < 12; i++) {
        let a = f32(i) / 12.0 * 6.2831853;
        for (var r = 1; r < 4; r++) {
            let o = vec2<f32>(cos(a), sin(a)) * px * f32(r * r) * 9.0 * vec2<f32>(dims.y / dims.x, 1.0);
            let s = pow(max(lum(textureSampleLevel(screen_texture, texture_sampler, uv + o, 0.0).rgb), 0.0), 1.0 / 2.2) * gain;
            flare += max(s - 1.2, 0.0) / f32(r * r);
        }
    }
    flare = flare / 12.0 * 0.8;
    var v = l * gain;
    // The tube's response: linear in the dark, rolling off into white.
    v = 1.0 - exp(-v * 1.3);
    v += flare;
    // Grain: stronger where it's dark (the tube is working hard), and a faint line structure.
    let n = hash(uv * dims + vec2<f32>(settings.nv_t * 61.0, settings.nv_t * 37.0)) - 0.5;
    v += n * mix(0.22, 0.06, clamp(v, 0.0, 1.0));
    v *= 0.96 + 0.04 * sin(uv.y * dims.y * 1.5);
    // Round tube edge.
    let d = length((uv - 0.5) * vec2<f32>(dims.x / dims.y, 1.0));
    v *= 1.0 - smoothstep(0.55, 0.92, d);
    v = max(v, 0.0);
    // Green phosphor, white where it saturates.
    let g = vec3<f32>(0.18, 1.0, 0.25) * v + vec3<f32>(0.75, 0.45, 0.65) * max(v - 0.85, 0.0) * 2.0;
    // Back to linear for the rest of the pass.
    return pow(clamp(g, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c = textureSample(screen_texture, texture_sampler, in.uv);
    if (settings.nv > 0.0) {
        let inside = settings.nv_box < 0.5 || all(abs(in.uv - 0.5) < vec2<f32>(0.25));
        if (inside) {
            c = vec4<f32>(night_vision(in.uv, c.rgb), c.a);
        }
    }
    let display = pow(max(c.rgb, vec3(0.0)), vec3(1.0 / 2.2));
    let lifted = pow(display, vec3(1.0 / max(settings.gamma, 0.1)));
    // Unreal adds the view fog (FlashFog / zone ViewFog) to the finished picture.
    let fogged = min(lifted + vec3(settings.fog_r, settings.fog_g, settings.fog_b), vec3(1.0));
    return vec4(pow(fogged, vec3(2.2)), c.a);
}
