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
    tm: f32,
    exposure: f32,
    pad0: f32,
    pad1: f32,
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

// Filmic: a fitted ACES curve (Narkowicz), its input and output scaled so the original's mid-grey stays put; the
// original's white lands at 0.85 and light past it rolls off instead of clipping.
fn filmic(x: vec3<f32>) -> vec3<f32> {
    let v = x * 0.6;
    let y = (v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14);
    return clamp(y * 1.26, vec3(0.0), vec3(1.0));
}

// AgX (Troy Sobotka), the common polynomial fit of its default contrast, in and out of its own working space.
fn agx(x: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(x, vec3(1e-10));
    v = clamp(log2(v), vec3(min_ev), vec3(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    let v2 = v * v;
    let v4 = v2 * v2;
    var c = 15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v - 0.00232;
    c = outset * c;
    // The fit's output is display-encoded: back to linear for the rest of the pass.
    return pow(clamp(c, vec3(0.0), vec3(1.0)), vec3(2.2));
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
    var lin = max(c.rgb, vec3(0.0)) * select(settings.exposure, 1.0, settings.exposure <= 0.0);
    if (settings.tm > 1.5) {
        lin = agx(lin);
    } else if (settings.tm > 0.5) {
        lin = filmic(lin);
    }
    let display = pow(lin, vec3(1.0 / 2.2));
    let lifted = pow(display, vec3(1.0 / max(settings.gamma, 0.1)));
    // Unreal adds the view fog (FlashFog / zone ViewFog) to the finished picture.
    let fogged = min(lifted + vec3(settings.fog_r, settings.fog_g, settings.fog_b), vec3(1.0));
    return vec4(pow(fogged, vec3(2.2)), c.a);
}
