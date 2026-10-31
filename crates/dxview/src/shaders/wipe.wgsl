// Classic / Modern switch (remaster.rs): the frozen frame from before the switch, swept away by a slanted glowing
// edge (after Halo 2: Anniversary). Ahead of the edge the old picture breaks up into jittering scanline rows with a
// colour split; behind it the live picture (the other look) shows through. Premultiplied output: the glow adds
// over the new picture.
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var old_tex: texture_2d<f32>;
@group(1) @binding(1) var old_smp: sampler;
// x = progress 0..1, y = seconds, z = aspect, w = 1 going Modern.
@group(1) @binding(2) var<uniform> p: vec4<f32>;

fn hash(n: f32) -> f32 {
    return fract(sin(n * 127.1) * 43758.5453);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    // A slant leaning right toward the top.
    let d = uv.x + (0.5 - uv.y) * 0.22 / max(p.z, 0.5);
    let e = mix(-0.25, 1.25, p.x);
    let ahead = d - e;
    let modern = p.w > 0.5;
    let edge_col = select(vec3<f32>(0.75, 0.85, 1.0), vec3<f32>(1.0, 0.82, 0.5), modern);
    // The edge: a hot core and a soft halo on both sides.
    let core = exp(-(ahead / 0.004) * (ahead / 0.004));
    let halo = exp(-abs(ahead) / 0.022) * 0.35;
    let glow = edge_col * (core * 2.2 + halo) * smoothstep(0.0, 0.05, p.x) * (1.0 - smoothstep(0.95, 1.0, p.x));
    if (ahead < 0.0) {
        // Behind the edge: the live picture, with the glow and a few trailing scan sparks.
        let row = floor(uv.y * 240.0);
        let spark = step(0.985, hash(row + floor(p.y * 40.0))) * exp(ahead / 0.06) * 0.6;
        return vec4<f32>(glow + edge_col * spark, 0.0);
    }
    // Ahead of the edge: the old frame, tearing up close to it.
    let band = 1.0 - smoothstep(0.0, 0.14, ahead);
    let row = floor(uv.y * 90.0);
    let jit = (hash(row + floor(p.y * 30.0) * 17.0) - 0.5) * 0.035 * band * band;
    let split = 0.006 * band;
    let suv = uv + vec2<f32>(jit, 0.0);
    let r = textureSample(old_tex, old_smp, suv + vec2<f32>(split, 0.0)).r;
    let g = textureSample(old_tex, old_smp, suv).g;
    let b = textureSample(old_tex, old_smp, suv - vec2<f32>(split, 0.0)).b;
    var col = vec3<f32>(r, g, b);
    // Scanlines and a lift near the edge.
    let scan = 0.82 + 0.18 * sin(uv.y * 900.0);
    col = mix(col, col * scan + edge_col * 0.06, band);
    // Still frame for the hold: a faint wash so it reads as stopped.
    return vec4<f32>(col + glow, 1.0);
}
