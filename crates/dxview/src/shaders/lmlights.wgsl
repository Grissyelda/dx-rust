// Lean Rendering's lightmap lights (lmlights.rs): each dynamic light added into the level's lightmap where it reaches,
// as dx-level's bake added the static ones and as Unreal 1 lit its lightmaps, so the level's shader needs no walk over
// the lights in each pixel's cluster. One instance a surface block (its 1-texel border included); the block's texels
// get the baked texel plus its lights, worked out as bsp_shadow.wgsl's classic_lighting did per pixel:
// from_srgb(to_srgb(baked) + sum of colour x brightness x LIGHT_GAIN x (1 - d / r) x N.L [x cone^2]). An instance
// without lights copies the baked texels (a block a light has left, or the whole atlas when a map loads).
// Unreal units throughout.

struct Light {
    // xyz where it is, w how far it reaches.
    pos_radius: vec4<f32>,
    // rgb: colour x brightness x LIGHT_GAIN, in the bake's gamma space; w: 1 for a spot light.
    color_spot: vec4<f32>,
    // A spot light's direction, and its cone's scale and offset (Bevy's spot_light).
    dir_scale: vec4<f32>,
    offset: vec4<f32>,
};

@group(0) @binding(0) var baked: texture_2d<f32>;
@group(0) @binding(1) var<uniform> lights: array<Light, 32>;

struct Block {
    @builtin(vertex_index) vi: u32,
    // x, y, width, height in texels, border included.
    @location(0) rect: vec4<u32>,
    // The surface's texels across and down, the lights reaching it (bits into `lights`), flags (1: two-sided).
    @location(1) info: vec4<u32>,
    @location(2) origin: vec4<f32>,
    @location(3) du: vec4<f32>,
    @location(4) dv: vec4<f32>,
    @location(5) normal: vec4<f32>,
};

struct ToFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<u32>,
    @location(1) @interpolate(flat) info: vec4<u32>,
    @location(2) @interpolate(flat) origin: vec3<f32>,
    @location(3) @interpolate(flat) du: vec3<f32>,
    @location(4) @interpolate(flat) dv: vec3<f32>,
    @location(5) @interpolate(flat) normal: vec3<f32>,
};

@vertex
fn vertex(in: Block) -> ToFragment {
    var corner = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let size = vec2<f32>(textureDimensions(baked));
    let px = vec2<f32>(in.rect.xy) + corner[in.vi] * vec2<f32>(in.rect.zw);
    var out: ToFragment;
    out.position = vec4<f32>(px.x / size.x * 2.0 - 1.0, 1.0 - px.y / size.y * 2.0, 0.0, 1.0);
    out.rect = in.rect;
    out.info = in.info;
    out.origin = in.origin.xyz;
    out.du = in.du.xyz;
    out.dv = in.dv.xyz;
    out.normal = in.normal.xyz;
    return out;
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: ToFragment) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(in.position.xy));
    let base = textureLoad(baked, px, 0).rgb;
    var mask = in.info.z;
    if (mask == 0u) {
        return vec4<f32>(base, 1.0);
    }
    // The border texels light as the edge texels they repeat.
    let t = clamp(vec2<f32>(px) - vec2<f32>(in.rect.xy) - vec2<f32>(1.0), vec2<f32>(0.0), vec2<f32>(in.info.xy) - vec2<f32>(1.0));
    let p = in.origin + in.du * t.x + in.dv * t.y;
    let two_sided = (in.info.w & 1u) != 0u;
    var added = vec3<f32>(0.0);
    loop {
        if (mask == 0u) {
            break;
        }
        let light = lights[firstTrailingBit(mask)];
        mask = mask & (mask - 1u);
        let to = light.pos_radius.xyz - p;
        let d = length(to);
        let r = light.pos_radius.w;
        if (d >= r || d < 1e-4) {
            continue;
        }
        let ndl = dot(in.normal, to / d);
        var reach = (1.0 - d / r) * select(max(ndl, 0.0), abs(ndl), two_sided);
        if (light.color_spot.w > 0.5) {
            let cone = saturate(dot(-to / d, light.dir_scale.xyz) * light.dir_scale.w + light.offset.x);
            reach = reach * cone * cone;
        }
        added += light.color_spot.rgb * reach;
    }
    return vec4<f32>(from_srgb(to_srgb(base) + added), 1.0);
}
