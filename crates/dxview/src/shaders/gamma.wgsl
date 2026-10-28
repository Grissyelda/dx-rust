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
}
@group(0) @binding(2) var<uniform> settings: Settings;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(screen_texture, texture_sampler, in.uv);
    let display = pow(max(c.rgb, vec3(0.0)), vec3(1.0 / 2.2));
    let lifted = pow(display, vec3(1.0 / max(settings.gamma, 0.1)));
    // Unreal adds the view fog (FlashFog / zone ViewFog) to the finished picture.
    let fogged = min(lifted + vec3(settings.fog_r, settings.fog_g, settings.fog_b), vec3(1.0));
    return vec4(pow(fogged, vec3(2.2)), c.a);
}
