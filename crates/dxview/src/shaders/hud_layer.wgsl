// A classic HUD window: premultiplied pixels drawn on the CPU (uicanvas.rs), blended premultiplied so the
// original's translucent (additive) parts add to the view and the masked parts cover it. The Brightness setting's
// gamma curve is applied here as on the 3D view (gamma.wgsl), so menus and the HUD brighten with the world.
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var hud_texture: texture_2d<f32>;
@group(1) @binding(1) var hud_sampler: sampler;
@group(1) @binding(2) var<uniform> gamma: vec4<f32>;

fn lift(c: vec3<f32>) -> vec3<f32> {
    let display = pow(max(c, vec3(0.0)), vec3(1.0 / 2.2));
    let lifted = pow(display, vec3(1.0 / max(gamma.x, 0.1)));
    return pow(lifted, vec3(2.2));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(hud_texture, hud_sampler, in.uv);
    if (c.a > 0.001) {
        // Premultiplied: lift the colour, not the coverage.
        return vec4(lift(min(c.rgb / c.a, vec3(1.0))) * c.a, c.a);
    }
    // Additive (translucent) texels.
    return vec4(lift(c.rgb), c.a);
}
