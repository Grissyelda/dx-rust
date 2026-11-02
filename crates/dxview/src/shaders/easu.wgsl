// Edge-adaptive spatial upsampling: a WGSL port of the EASU pass of AMD FidelityFX Super Resolution 1
// (ffx_fsr1.h, Copyright (c) 2021 Advanced Micro Devices, Inc., MIT licence - see THIRD_PARTY.md). Altered: plain
// textureLoad taps instead of gathers, and an invertible tone curve around it (FSR expects a perceptual,
// low-dynamic-range picture; ours is linear HDR).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var src: texture_2d<f32>;

fn ld(p: vec2<i32>) -> vec3<f32> {
    let d = vec2<i32>(textureDimensions(src)) - vec2<i32>(1);
    let c = max(textureLoad(src, clamp(p, vec2<i32>(0), d), 0).rgb, vec3<f32>(0.0));
    // Reinhard by the brightest channel (keeps hue, into 0..1), then roughly perceptual.
    let t = c / (1.0 + max(c.r, max(c.g, c.b)));
    return sqrt(t);
}

fn luma(c: vec3<f32>) -> f32 {
    return c.b * 0.5 + c.r * 0.5 + c.g;
}

// Direction and edge length from one bilinear quad's cross of luma (FsrEasuSetF).
fn set_f(w: f32, la: f32, lb: f32, lc: f32, ld_: f32, le: f32) -> vec3<f32> {
    let dc = ld_ - lc;
    let cb = lc - lb;
    var len_x = max(abs(dc), abs(cb));
    len_x = 1.0 / max(len_x, 1e-5);
    let dir_x = ld_ - lb;
    len_x = clamp(abs(dir_x) * len_x, 0.0, 1.0);
    len_x = len_x * len_x;
    let ec = le - lc;
    let ca = lc - la;
    var len_y = max(abs(ec), abs(ca));
    len_y = 1.0 / max(len_y, 1e-5);
    let dir_y = le - la;
    len_y = clamp(abs(dir_y) * len_y, 0.0, 1.0);
    len_y = len_y * len_y;
    return vec3<f32>(dir_x * w, dir_y * w, (len_x + len_y) * w);
}

struct Acc {
    c: vec3<f32>,
    w: f32,
}

// One tap of the stretched, rotated lanczos-like kernel (FsrEasuTapF).
fn tap(acc: ptr<function, Acc>, off: vec2<f32>, dir: vec2<f32>, len2: vec2<f32>, lob: f32, clp: f32, c: vec3<f32>) {
    var v = vec2<f32>(off.x * dir.x + off.y * dir.y, off.x * (-dir.y) + off.y * dir.x);
    v = v * len2;
    var d2 = v.x * v.x + v.y * v.y;
    d2 = min(d2, clp);
    var wb = 2.0 / 5.0 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb = wb * wb;
    wa = wa * wa;
    wb = 25.0 / 16.0 * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    (*acc).c += c * w;
    (*acc).w += w;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let isz = vec2<f32>(textureDimensions(src));
    var pp = in.uv * isz - vec2<f32>(0.5);
    let fp = floor(pp);
    pp = pp - fp;
    let o = vec2<i32>(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let b = ld(o + vec2<i32>(0, -1));
    let c = ld(o + vec2<i32>(1, -1));
    let e = ld(o + vec2<i32>(-1, 0));
    let f = ld(o + vec2<i32>(0, 0));
    let g = ld(o + vec2<i32>(1, 0));
    let h = ld(o + vec2<i32>(2, 0));
    let i = ld(o + vec2<i32>(-1, 1));
    let j = ld(o + vec2<i32>(0, 1));
    let k = ld(o + vec2<i32>(1, 1));
    let l = ld(o + vec2<i32>(2, 1));
    let n = ld(o + vec2<i32>(0, 2));
    let oo = ld(o + vec2<i32>(1, 2));
    let bl = luma(b); let cl = luma(c); let el = luma(e); let fl = luma(f); let gl = luma(g); let hl = luma(h);
    let il = luma(i); let jl = luma(j); let kl = luma(k); let ll = luma(l); let nl = luma(n); let ol = luma(oo);

    var acc3 = set_f((1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    acc3 += set_f(pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    acc3 += set_f((1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    acc3 += set_f(pp.x * pp.y, gl, jl, kl, ll, ol);
    var dir = acc3.xy;
    var len = acc3.z;

    let dir2 = dir * dir;
    var dir_r = dir2.x + dir2.y;
    let zro = dir_r < (1.0 / 32768.0);
    dir_r = inverseSqrt(max(dir_r, 1e-12));
    if zro {
        dir_r = 1.0;
        dir.x = 1.0;
    }
    dir = dir * dir_r;
    len = len * 0.5;
    len = len * len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) / max(abs(dir.x), abs(dir.y));
    let len2 = vec2<f32>(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = 1.0 / lob;

    var acc: Acc;
    acc.c = vec3<f32>(0.0);
    acc.w = 0.0;
    tap(&acc, vec2<f32>(0.0, -1.0) - pp, dir, len2, lob, clp, b);
    tap(&acc, vec2<f32>(1.0, -1.0) - pp, dir, len2, lob, clp, c);
    tap(&acc, vec2<f32>(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
    tap(&acc, vec2<f32>(0.0, 1.0) - pp, dir, len2, lob, clp, j);
    tap(&acc, vec2<f32>(0.0, 0.0) - pp, dir, len2, lob, clp, f);
    tap(&acc, vec2<f32>(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
    tap(&acc, vec2<f32>(1.0, 1.0) - pp, dir, len2, lob, clp, k);
    tap(&acc, vec2<f32>(2.0, 1.0) - pp, dir, len2, lob, clp, l);
    tap(&acc, vec2<f32>(2.0, 0.0) - pp, dir, len2, lob, clp, h);
    tap(&acc, vec2<f32>(1.0, 0.0) - pp, dir, len2, lob, clp, g);
    tap(&acc, vec2<f32>(1.0, 2.0) - pp, dir, len2, lob, clp, oo);
    tap(&acc, vec2<f32>(0.0, 2.0) - pp, dir, len2, lob, clp, n);

    // Deringing: within the four nearest texels.
    let mn = min(min(f, g), min(j, k));
    let mx = max(max(f, g), max(j, k));
    var t = clamp(acc.c / max(acc.w, 1e-5), mn, mx);
    t = t * t;
    let col = t / max(1.0 - max(t.r, max(t.g, t.b)), 1e-4);
    return vec4<f32>(col, 1.0);
}
