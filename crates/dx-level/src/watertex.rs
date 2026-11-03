//! Unreal's water procedural textures (Engine WaterTexture: WetTexture, WaveTexture), which the original engine
//! animates at run time (the package holds no moving picture). The engine's own code isn't available, so this is
//! a reconstruction from what the packages store and how the effect looks:
//!
//! * A height field at half the texture's size (the drops' X / Y are in those cells) runs the classic two-buffer
//!   wave equation: new = (the four neighbours) / 2 - old, a little damped, wrapping at the edges so it tiles.
//! * The texture's Drops (up to 256 {Type, Depth, X, Y, ByteA..D}) disturb it every step, each kind as its name
//!   says: fixed spots, spots oscillating in phase, whirling points, lines, oscillating lines, rain, leaky taps.
//! * WetTexture: its SourceTexture's picture pushed about by the water's slope (WaveAmp pixels per unit).
//! * WaveTexture: the slope lit by a light from BumpMapAngle (BumpMapLight strength, a Phong highlight of
//!   PhongSize within PhongRange) looked up in its palette.
//!
//! Used through firetex::FireTex (the procedural texture the viewer steps at 40 Hz while it's on screen).

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Drop {
    kind: u8,
    depth: u8,
    x: u8,
    y: u8,
    a: u8,
    b: u8,
    c: u8,
    d: u8,
}

#[derive(Debug, Clone)]
pub enum Look {
    /// WetTexture: the source picture (RGBA, the texture's size), displaced.
    Wet { source: Vec<[u8; 4]> },
    /// WaveTexture: the slope shaded through the palette.
    Wave { palette: [[u8; 4]; 256], light: f32, angle: f32, phong_range: f32, phong_size: f32 },
}

#[derive(Debug, Clone)]
pub struct WaterSim {
    pub width: usize,
    pub height: usize,
    /// Half-resolution height field (current, previous).
    gw: usize,
    gh: usize,
    cur: Vec<f32>,
    old: Vec<f32>,
    drops: Vec<Drop>,
    wave_amp: f32,
    look: Look,
    frame: u32,
    seed: u32,
}

impl WaterSim {
    /// `drops`: the texture's ADrop structs (8 bytes each); `wave_amp`: WaveAmp.
    pub fn new(width: usize, height: usize, drops: &[[u8; 8]], wave_amp: u8, look: Look) -> WaterSim {
        let (gw, gh) = ((width / 2).max(2), (height / 2).max(2));
        let mut s = WaterSim {
            width: width.max(1),
            height: height.max(1),
            gw,
            gh,
            cur: vec![0.0; gw * gh],
            old: vec![0.0; gw * gh],
            drops: drops
                .iter()
                .map(|d| Drop { kind: d[0], depth: d[1], x: d[2], y: d[3], a: d[4], b: d[5], c: d[6], d: d[7] })
                .collect(),
            wave_amp: wave_amp as f32,
            look,
            frame: 0,
            seed: 0x2545_f491,
        };
        // Settle into motion so it never starts flat.
        for _ in 0..60 {
            s.step();
        }
        s
    }

    fn rnd(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    fn frand(&mut self) -> f32 {
        (self.rnd() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A drop's push: the cell to `v`, its neighbours half way there (a single cell only makes the field ring at
    /// its finest wavelength - a checkerboard).
    fn set(&mut self, x: i32, y: i32, v: f32) {
        let (w, h) = (self.gw as i32, self.gh as i32);
        for (dx, dy, k) in [(0, 0, 1.0f32), (1, 0, 0.5), (-1, 0, 0.5), (0, 1, 0.5), (0, -1, 0.5)] {
            let i = ((x + dx).rem_euclid(w) + (y + dy).rem_euclid(h) * w) as usize;
            self.cur[i] += (v - self.cur[i]) * k;
        }
    }

    #[allow(dead_code)]
    fn scale(&mut self, x: i32, y: i32, k: f32) {
        let (w, h) = (self.gw as i32, self.gh as i32);
        let i = (x.rem_euclid(w) + y.rem_euclid(h) * w) as usize;
        self.cur[i] *= k;
    }

    /// Advance one step: the drops, then the waves.
    pub fn step(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let f = self.frame;
        let drops = std::mem::take(&mut self.drops);
        let mut out = drops.clone();
        for (dr, o) in drops.iter().zip(out.iter_mut()) {
            let (x, y) = (dr.x as i32, dr.y as i32);
            let amp = (dr.depth as f32 - 128.0) / 128.0;
            let phase = |p: u8| (p as f32 / 256.0 * std::f32::consts::TAU).sin();
            match dr.kind {
                // FixedDepth: the spot held at its depth.
                0 => self.set(x, y, amp),
                // PhaseSpot: the spot oscillates; Depth is its phase, ByteD the speed.
                1 => {
                    o.depth = dr.depth.wrapping_add(dr.d.max(1));
                    self.set(x, y, phase(o.depth));
                }
                // ShallowSpot / HalfAmpl: PhaseSpots at a lower swing (Depth = phase, ByteD = speed). The Island's
                // harbour water (Effects.water.drtywater_a) is nothing but ShallowSpots, so they must drive the
                // ripples: read as damping, the water stood still.
                2 | 3 => {
                    o.depth = dr.depth.wrapping_add(dr.d.max(1));
                    self.set(x, y, phase(o.depth) * if dr.kind == 2 { 0.6 } else { 0.5 });
                }
                // RandomMover: a point that wanders, pushing the water.
                4 => {
                    let r = self.rnd();
                    o.x = (dr.x as i32 + (r & 3) as i32 - 1).rem_euclid(self.gw as i32) as u8;
                    o.y = (dr.y as i32 + ((r >> 2) & 3) as i32 - 1).rem_euclid(self.gh as i32) as u8;
                    self.set(o.x as i32, o.y as i32, amp.abs().max(0.5));
                }
                // FixedRandomSpot: a random depth at the spot.
                5 => {
                    let v = self.frand() * 2.0 - 1.0;
                    self.set(x, y, v);
                }
                // WhirlyThing / BigWhirly: a point running round a circle (ByteA radius, ByteD speed).
                6 | 7 => {
                    let r = (dr.a as f32).max(1.0) * if dr.kind == 7 { 2.0 } else { 0.5 };
                    o.c = dr.c.wrapping_add(dr.d.max(1));
                    let ang = o.c as f32 / 256.0 * std::f32::consts::TAU;
                    let (px, py) = (x as f32 + ang.cos() * r, y as f32 + ang.sin() * r);
                    self.set(px as i32, py as i32, 1.0);
                }
                // Lines (horizontal, vertical, two diagonals) of ByteA length held at the depth, and the same
                // oscillating (Depth = phase, ByteD = speed).
                8..=15 => {
                    let v = if dr.kind >= 12 {
                        o.depth = dr.depth.wrapping_add(dr.d.max(1));
                        phase(o.depth)
                    } else {
                        amp
                    };
                    let (dx, dy) = match (dr.kind - 8) % 4 {
                        0 => (1, 0),
                        1 => (0, 1),
                        2 => (1, 1),
                        _ => (1, -1),
                    };
                    let n = (dr.a as i32).max(1);
                    for k in 0..n {
                        self.set(x + dx * k, y + dy * k, v);
                    }
                }
                // RainDrops: now and then a drop lands near the spot (ByteD: how often).
                16 => {
                    if self.frand() < (dr.d as f32 / 64.0).clamp(0.01, 1.0) {
                        let ox = (self.rnd() % 5) as i32 - 2;
                        let oy = (self.rnd() % 5) as i32 - 2;
                        self.set(x + ox, y + oy, -amp.abs().max(0.6));
                    }
                }
                // AreaClamp: a calm area of ByteA radius.
                17 => {
                    let r = (dr.a as i32).max(1);
                    for yy in -r..=r {
                        for xx in -r..=r {
                            if xx * xx + yy * yy <= r * r {
                                self.set(x + xx, y + yy, 0.0);
                            }
                        }
                    }
                }
                // LeakyTap: a drop every so often (ByteD the period); DrippyTap: the same, irregular.
                18 | 19 => {
                    let period = (dr.d as u32 + 8).max(8);
                    let due = if dr.kind == 18 {
                        (f + (dr.x as u32 * 7 + dr.y as u32 * 13)).is_multiple_of(period)
                    } else {
                        self.frand() < 1.0 / period as f32
                    };
                    if due {
                        self.set(x, y, -amp.abs().max(0.8));
                    }
                }
                _ => {}
            }
        }
        self.drops = out;
        // The wave equation (wrapping, so the picture tiles).
        let (w, h) = (self.gw, self.gh);
        for yy in 0..h {
            let up = ((yy + h - 1) % h) * w;
            let dn = ((yy + 1) % h) * w;
            let row = yy * w;
            for xx in 0..w {
                let l = row + (xx + w - 1) % w;
                let r = row + (xx + 1) % w;
                let i = row + xx;
                // Eight neighbours (sides and corners weighted 2:1, summing to 2): the four-neighbour form lets the
                // finest wavelength through as a checkerboard.
                let (lu, ru) = (up + (xx + w - 1) % w, up + (xx + 1) % w);
                let (ld, rd) = (dn + (xx + w - 1) % w, dn + (xx + 1) % w);
                let sides = self.cur[l] + self.cur[r] + self.cur[up + xx] + self.cur[dn + xx];
                let corners = self.cur[lu] + self.cur[ru] + self.cur[ld] + self.cur[rd];
                let v = (sides * 2.0 + corners) / 6.0 - self.old[i];
                self.old[i] = v * (1.0 - 1.0 / 40.0);
            }
        }
        std::mem::swap(&mut self.cur, &mut self.old);
    }

    /// The height field at a texture pixel (bilinear over the half-resolution cells, wrapping).
    fn height_at(&self, px: f32, py: f32) -> f32 {
        let (w, h) = (self.gw as i32, self.gh as i32);
        let (gx, gy) = (px * 0.5, py * 0.5);
        let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        let at = |x: i32, y: i32| self.cur[(x.rem_euclid(w) + y.rem_euclid(h) * w) as usize];
        let a = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
        let b = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// The current picture as RGBA8.
    pub fn rgba(&self, out: &mut Vec<u8>) {
        let (w, h) = (self.width, self.height);
        out.resize(w * h * 4, 0);
        match &self.look {
            Look::Wet { source } => {
                // WaveAmp pixels of displacement per unit of slope.
                // (A quarter of this - /24 - moved the picture by under a pixel: the harbour looked frozen.)
                let k = self.wave_amp / 6.0;
                for y in 0..h {
                    for x in 0..w {
                        let (fx, fy) = (x as f32, y as f32);
                        let gx = self.height_at(fx + 1.0, fy) - self.height_at(fx - 1.0, fy);
                        let gy = self.height_at(fx, fy + 1.0) - self.height_at(fx, fy - 1.0);
                        let sx = (x as i32 + (gx * k).round() as i32).rem_euclid(w as i32) as usize;
                        let sy = (y as i32 + (gy * k).round() as i32).rem_euclid(h as i32) as usize;
                        let c = source.get(sy * w + sx).copied().unwrap_or([0, 0, 0, 255]);
                        out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                    }
                }
            }
            Look::Wave { palette, light, angle, phong_range, phong_size } => {
                let (lx, ly) = (angle.cos(), angle.sin());
                for y in 0..h {
                    for x in 0..w {
                        let (fx, fy) = (x as f32, y as f32);
                        let gx = self.height_at(fx + 1.0, fy) - self.height_at(fx - 1.0, fy);
                        let gy = self.height_at(fx, fy + 1.0) - self.height_at(fx, fy - 1.0);
                        let slope = (gx * lx + gy * ly) * self.wave_amp / 64.0;
                        let mut v = 128.0 + slope * *light;
                        // The Phong highlight where the surface turns to the light.
                        if slope > 0.0 && slope * 64.0 < *phong_range {
                            v += *phong_size * (1.0 - slope * 64.0 / phong_range.max(1.0));
                        }
                        let c = palette[v.clamp(0.0, 255.0) as usize];
                        out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                    }
                }
            }
        }
    }
}
