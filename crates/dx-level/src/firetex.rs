//! Unreal's procedural "fire" textures (FireTexture): a heat field (one byte per texel) that spark emitters draw
//! into every frame, then blurred upward and cooled; the heat is shown through the texture's palette. Deus Ex uses
//! them for electricity (the riot prod, nano swords), flames and glows.
//!
//! The spark behaviors follow SurrealEngine's UFireTexture (zlib license, see THIRD_PARTY.md), rewritten here.

/// Spark kinds in the order of Unreal's ESparkType.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Burn,
    Sparkle,
    Pulse,
    Signal,
    Blaze,
    OzHasSpoken,
    Cone,
    BlazeRight,
    BlazeLeft,
    Emit,
    Wheel,
    LineLightning,
    RampLightning,
    SphereLightning,
    /// Organic / WanderOrganic / Gametes: heat blobs that wander about (the aug canister's Nano_SFX_A column).
    Wander,
    /// RandomCloud / CustomCloud / LocalCloud: heat dropped at random within ByteC of the spark (the bio cell).
    Cloud,
    Other,
}

fn kind(b: u8) -> Kind {
    match b {
        0 => Kind::Burn,
        1 => Kind::Sparkle,
        2 => Kind::Pulse,
        3 => Kind::Signal,
        4 => Kind::Blaze,
        5 => Kind::OzHasSpoken,
        6 => Kind::Cone,
        7 => Kind::BlazeRight,
        8 => Kind::BlazeLeft,
        13 => Kind::Emit,
        26 => Kind::Wheel,
        23 => Kind::LineLightning,
        24 => Kind::RampLightning,
        25 => Kind::SphereLightning,
        17 | 18 | 27 => Kind::Wander,
        19 | 20 | 21 => Kind::Cloud,
        _ => Kind::Other,
    }
}

#[derive(Debug, Clone)]
struct Spark {
    kind: Kind,
    heat: u8,
    x: u8,
    y: u8,
    a: [u8; 4],
    /// Wander: position and velocity in texels.
    pos: (f32, f32),
    vel: (f32, f32),
}

#[derive(Debug, Clone, Copy)]
enum Particle {
    Twirl { x: f32, y: f32, angle: f32, rot: f32, heat: u8, age: u8 },
    Drift { x: f32, y: f32, vx: f32, vy: f32, heat: i32, decay: i32 },
    Fall { x: f32, y: f32, vx: f32, vy: f32, heat: u8, age: u8 },
}

/// How a fire texture is drawn: the spark simulation, or a hand-made stand-in for the few textures whose spark
/// types (Jugglers, Organic...) the simulation doesn't reproduce.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Sim,
    /// A soft, flickering ball of flame (the flamethrower's flame_b projectile).
    Ball,
    /// A small licking flame standing on the bottom edge (the flamethrower's pilot light).
    Pilot,
}

#[derive(Debug, Clone)]
pub struct FireTex {
    pub mode: Mode,
    frame: u32,
    pub width: usize,
    pub height: usize,
    heat: Vec<u8>,
    work: Vec<u8>,
    sparks: Vec<Spark>,
    particles: Vec<Particle>,
    limit: usize,
    fade: [u8; 1024],
    rising: bool,
    palette: [[u8; 4]; 256],
    seed: u32,
}

impl FireTex {
    /// `sparks`: the texture's serialized sparks; `palette`: RGBA per heat value.
    pub fn new(width: usize, height: usize, sparks: &[[u8; 8]], palette: [[u8; 4]; 256], render_heat: u8, rising: bool, limit: usize) -> FireTex {
        let heat_loss = 1.0 - (255.0 - render_heat as f32) / 16.0;
        let mut fade = [0u8; 1024];
        for (i, f) in fade.iter_mut().enumerate() {
            *f = (i as f32 * 0.25 + heat_loss).clamp(0.0, 255.0) as u8;
        }
        FireTex {
            mode: Mode::Sim,
            frame: 0,
            width: width.max(1),
            height: height.max(1),
            heat: vec![0; width.max(1) * height.max(1)],
            work: vec![0; width.max(1) * height.max(1)],
            sparks: sparks.iter().map(|s| Spark { kind: kind(s[0]), heat: s[1], x: s[2], y: s[3], a: [s[4], s[5], s[6], s[7]], pos: (s[2] as f32, s[3] as f32), vel: (0.0, 0.0) }).collect(),
            particles: Vec::new(),
            limit: limit.max(1),
            fade,
            rising,
            palette,
            seed: 0x1234_5678,
        }
    }

    fn rnd(&mut self) -> u8 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as u8
    }

    fn put(&mut self, x: i32, y: i32, v: u8) {
        let (w, h) = (self.width as i32, self.height as i32);
        let x = x.rem_euclid(w);
        let y = y.rem_euclid(h);
        self.heat[(x + y * w) as usize] = v;
    }

    fn lightning(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, h0: u8, h1: u8) {
        let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt().max(1.0);
        let steps = (len * 2.0) as i32;
        let (mut jx, mut jy) = (0.0f32, 0.0f32);
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let c = h0 as f32 + (h1 as f32 - h0 as f32) * t;
            // Jagged: a random walk that is pulled back to the straight line.
            jx = (jx + (self.rnd() as f32 / 127.5 - 1.0)) * 0.9;
            jy = (jy + (self.rnd() as f32 / 127.5 - 1.0)) * 0.9;
            let x = x0 + (x1 - x0) * t + jx;
            let y = y0 + (y1 - y0) * t + jy;
            self.put(x as i32, y as i32, c as u8);
        }
    }

    /// Advance one frame.
    pub fn step(&mut self) {
        if self.mode != Mode::Sim {
            self.frame = self.frame.wrapping_add(1);
            self.draw_procedural();
            return;
        }
        let can_emit = |s: &FireTex| s.sparks.len() + s.particles.len() < s.limit;
        for i in 0..self.sparks.len() {
            let sp = self.sparks[i].clone();
            let (x, y) = (sp.x as i32, sp.y as i32);
            match sp.kind {
                Kind::Burn => {
                    let v = self.rnd();
                    self.put(x, y, v);
                }
                Kind::Wander => {
                    // Deviation (no reference for Unreal's exact rule): a random walk at up to ~1 texel a frame,
                    // drawing a 3x3 blob of the spark's heat.
                    let (mut vx, mut vy) = sp.vel;
                    vx = (vx + (self.rnd() as f32 / 127.5 - 1.0) * 0.35).clamp(-1.0, 1.0);
                    vy = (vy + (self.rnd() as f32 / 127.5 - 1.0) * 0.35).clamp(-1.0, 1.0);
                    let (w, h) = (self.width as f32, self.height as f32);
                    let px = (sp.pos.0 + vx).rem_euclid(w);
                    let py = (sp.pos.1 + vy).rem_euclid(h);
                    self.sparks[i].pos = (px, py);
                    self.sparks[i].vel = (vx, vy);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let k = if dx == 0 && dy == 0 { sp.heat } else { (sp.heat as u32 * 3 / 4) as u8 };
                            self.put(px as i32 + dx, py as i32 + dy, k);
                        }
                    }
                }
                Kind::Cloud => {
                    // Heat at a random point within ByteC of the spark, twice a frame (deviation: approximate).
                    let r = sp.a[2].max(1) as f32;
                    for _ in 0..2 {
                        let ang = self.rnd() as f32 / 256.0 * std::f32::consts::TAU;
                        let d = (self.rnd() as f32 / 255.0).sqrt() * r;
                        self.put(x + (ang.cos() * d) as i32, y + (ang.sin() * d) as i32, sp.heat);
                    }
                }
                Kind::Pulse | Kind::Signal | Kind::Sparkle | Kind::Other => {
                    let ox = (self.rnd() as i32 * sp.a[0] as i32 + 128) / 256;
                    let oy = (self.rnd() as i32 * sp.a[1] as i32 + 128) / 256;
                    match sp.kind {
                        Kind::Pulse => {
                            self.put(x + ox, y + oy, sp.heat);
                            self.sparks[i].heat = sp.heat.wrapping_add(sp.a[3]);
                        }
                        Kind::Signal => {
                            if sp.heat > sp.a[2] {
                                self.put(x + ox, y + oy, sp.heat);
                            }
                            let nh = sp.heat as u32 + sp.a[3] as u32;
                            self.sparks[i].heat = if nh < 256 { nh as u8 } else { self.rnd() };
                        }
                        _ => self.put(x + ox, y + oy, sp.heat),
                    }
                }
                Kind::Wheel => {
                    if can_emit(self) {
                        let angle = (sp.a[0] as f32 * 360.0 / 256.0).to_radians();
                        let rot = (sp.a[3] as f32 * 16.0 / 256.0 * 360.0 / 256.0).to_radians();
                        self.particles.push(Particle::Twirl { x: sp.x as f32 + 0.5, y: sp.y as f32 + 0.5, angle, rot, heat: sp.heat, age: sp.a[1] });
                    }
                    self.sparks[i].a[0] = sp.a[0].wrapping_add(sp.a[2]);
                }
                Kind::Emit => {
                    if can_emit(self) && self.rnd() < 64 {
                        self.particles.push(Particle::Drift {
                            x: sp.x as f32 + 0.5,
                            y: sp.y as f32 + 0.5,
                            vx: (sp.a[0] as i8) as f32 / 128.0,
                            vy: (sp.a[1] as i8) as f32 / 128.0,
                            heat: sp.heat as i32,
                            decay: sp.a[3] as i32,
                        });
                    }
                }
                Kind::OzHasSpoken | Kind::Blaze => {
                    if can_emit(self) && self.rnd() < 128 {
                        let vx = self.rnd() as f32 / 255.0 * 2.0 - 1.0;
                        let vy = if sp.kind == Kind::Blaze { self.rnd() as f32 / 255.0 * 2.0 - 1.0 } else { -0.5 };
                        self.particles.push(Particle::Drift { x: sp.x as f32 + 0.5, y: sp.y as f32 + 0.5, vx, vy, heat: sp.heat as i32, decay: 5 });
                    }
                }
                Kind::Cone | Kind::BlazeLeft | Kind::BlazeRight => {
                    if can_emit(self) && self.rnd() < 64 {
                        let r = self.rnd() as f32 / 255.0;
                        let (vx, age) = match sp.kind {
                            Kind::Cone => (r - 0.5, 50),
                            Kind::BlazeLeft => (-0.5 - r * 0.5, sp.a[2]),
                            _ => (0.5 + r * 0.5, sp.a[2]),
                        };
                        let vy = if sp.kind == Kind::Cone { 0.0 } else { -0.1 };
                        self.particles.push(Particle::Fall { x: sp.x as f32 + 0.5, y: sp.y as f32 + 0.5, vx, vy, heat: sp.heat, age });
                    }
                }
                Kind::SphereLightning => {
                    // Bolts from the centre in a random direction, Radius long, Frequency = chance threshold.
                    if self.rnd() >= sp.a[3] {
                        let ang = (self.rnd() as f32 * 360.0 / 256.0).to_radians();
                        let r = sp.a[2] as f32 * 0.5;
                        let (x0, y0) = (sp.x as f32 + 0.5, sp.y as f32 + 0.5);
                        self.lightning(x0, y0, x0 + ang.cos() * r, y0 + ang.sin() * r, sp.heat, sp.heat / 4);
                    }
                }
                Kind::LineLightning | Kind::RampLightning => {
                    // Bolts between (X,Y) and (A,B); C = frequency threshold.
                    if self.rnd() >= sp.a[2] {
                        let h1 = if sp.kind == Kind::RampLightning { sp.heat / 4 } else { sp.heat };
                        self.lightning(sp.x as f32, sp.y as f32, sp.a[0] as f32, sp.a[1] as f32, sp.heat, h1);
                    }
                }
            }
        }

        // Particles.
        let mut i = 0;
        while i < self.particles.len() {
            let alive = match &mut self.particles[i] {
                Particle::Twirl { x, y, angle, rot, heat, age } => {
                    if *age > 0 {
                        let (px, py, h) = (*x as i32, *y as i32, *heat);
                        *x += angle.sin() * 0.5;
                        *y += angle.cos() * 0.5;
                        *angle += *rot;
                        *age -= 1;
                        Some((px, py, h))
                    } else {
                        None
                    }
                }
                Particle::Drift { x, y, vx, vy, heat, decay } => {
                    *heat -= *decay;
                    if *heat > 0 {
                        let r = (*x as i32, *y as i32, *heat as u8);
                        *x += *vx;
                        *y += *vy;
                        Some(r)
                    } else {
                        None
                    }
                }
                Particle::Fall { x, y, vx, vy, heat, age } => {
                    *age = age.saturating_sub(1);
                    if *age > 0 {
                        let r = (*x as i32, *y as i32, *heat);
                        *x += *vx * 0.5;
                        *y += *vy * 0.5;
                        *vy = (*vy + 0.025).min(1.0);
                        Some(r)
                    } else {
                        None
                    }
                }
            };
            match alive {
                Some((px, py, h)) => {
                    self.put(px, py, h);
                    i += 1;
                }
                None => {
                    self.particles.swap_remove(i);
                }
            }
        }

        // Spread and cool: each texel becomes the faded sum of its row neighbours and the texel below.
        let (w, h) = (self.width, self.height);
        let rise = if self.rising { 1 } else { 0 };
        for y in 0..h {
            let src = ((y + rise) % h) * w;
            let next = ((y + rise + 1) % h) * w;
            for x in 0..w {
                let l = self.heat[src + (x + w - 1) % w] as usize;
                let c = self.heat[src + x] as usize;
                let r = self.heat[src + (x + 1) % w] as usize;
                let b = self.heat[next + x] as usize;
                self.work[y * w + x] = self.fade[l + c + r + b];
            }
        }
        std::mem::swap(&mut self.heat, &mut self.work);
    }

    fn draw_procedural(&mut self) {
        let (w, h) = (self.width, self.height);
        let t = self.frame as f32 / 40.0;
        // Smooth value noise.
        let hash = |x: i32, y: i32| -> f32 {
            let mut n = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
            n = (n ^ (n >> 13)).wrapping_mul(1274126177);
            (n ^ (n >> 16)) as f32 / u32::MAX as f32
        };
        let noise = |x: f32, y: f32| -> f32 {
            let (xi, yi) = (x.floor() as i32, y.floor() as i32);
            let (fx, fy) = (x - x.floor(), y - y.floor());
            let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
            let a = hash(xi, yi) + (hash(xi + 1, yi) - hash(xi, yi)) * sx;
            let b = hash(xi, yi + 1) + (hash(xi + 1, yi + 1) - hash(xi, yi + 1)) * sx;
            a + (b - a) * sy
        };
        let fbm = |x: f32, y: f32| noise(x, y) * 0.6 + noise(x * 2.1, y * 2.1) * 0.3 + noise(x * 4.3, y * 4.3) * 0.1;
        for y in 0..h {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let v = (y as f32 + 0.5) / h as f32;
                let heat = match self.mode {
                    Mode::Ball => {
                        let (dx, dy) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
                        let r = (dx * dx + dy * dy).sqrt();
                        let n = fbm(u * 5.0 + t * 1.3, v * 5.0 + t * 3.0);
                        let edge = 0.78 + 0.32 * (n - 0.5);
                        ((1.0 - r / edge).max(0.0).powf(0.8) * (0.7 + 0.3 * n)).min(1.0)
                    }
                    _ => {
                        // Base at the bottom, tip near the top; the outline wavers and licks upward.
                        let up = 1.0 - v;
                        let n = fbm(u * 4.0, up * 3.0 - t * 4.0);
                        let sway = (t * 7.0 + up * 5.0).sin() * 0.04 * up;
                        let half = 0.3 * (1.0 - up).max(0.0).powf(0.6) * (0.8 + 0.4 * n) * (up * 6.0).min(1.0).max(0.35);
                        let dx = (u - 0.5 - sway).abs();
                        let body = (1.0 - dx / half.max(1e-3)).max(0.0);
                        let fade = (1.0 - up / (0.85 + 0.15 * n)).max(0.0);
                        (body.powf(0.8) * fade.powf(0.6) * 0.8).min(1.0)
                    }
                };
                self.heat[y * w + x] = (heat * 255.0) as u8;
            }
        }
    }

    /// Current frame as RGBA8 through the palette.
    pub fn rgba(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.heat.len() * 4);
        for &v in &self.heat {
            out.extend_from_slice(&self.palette[v as usize]);
        }
    }
}
