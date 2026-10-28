//! ParticleGenerator (and WaterDrips): every checkTime, a `frequency` chance of numPerSpawn particles ejected along
//! the generator's rotation at ejectSpeed (bRandomEject adds 0.2 x ejectSpeed in a random direction). Particles
//! live particleLifeSpan seconds, rise at riseRate (or fall under zone gravity with bGravity), grow (bScale) and
//! fade (bFade); at most 64 per generator (ParticleIterator.Particles[64]). bTriggered generators start off unless
//! bInitiallyOn, and Trigger toggles / UnTrigger stops them. Away from the player and out of view they freeze
//! (bFrozen). Faucets (Faucet.PostBeginPlay) each own one: small water drops just past the spout, running while
//! the tap is open.

use crate::weapons::{image_for, RuntimeAssets};
use crate::LevelRes;
use bevy::prelude::*;
use dx_level::actors::rotator_axes;
use dx_level::ue_to_bevy;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

static INBOX: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

pub fn event(name: &str, on: bool) {
    if let Ok(mut q) = INBOX.lock() {
        q.push((name.to_string(), on));
    }
}

struct Particle {
    pos: DVec3,
    vel: DVec3,
    life: f32,
    entity: Entity,
}

struct Gen {
    tag: String,
    pos: DVec3,
    dir: DVec3,
    frequency: f32,
    rise: f64,
    eject: f64,
    tex: String,
    life: f32,
    draw_scale: f32,
    scale: bool,
    fade: bool,
    random: bool,
    check: f32,
    gravity: bool,
    num: i32,
    spewing: bool,
    /// Owned by a faucet (blocker): spews while it's open.
    faucet: Option<usize>,
    /// SetBase(decoration): rides that blocker (offset from its centre); gone with it (DelayedDestroy).
    follow: Option<(usize, DVec3)>,
    time: f32,
    parts: Vec<Particle>,
    size: f32,
    /// Texture height / width.
    aspect: f32,
    mat: Option<Handle<StandardMaterial>>,
    /// Actor.LifeSpan: the generator stops after this long and goes once its particles have died.
    lifespan: Option<f32>,
    /// Sound played for each burst (ParticleGenerator.spawnSound).
    spawn_sound: Option<String>,
}

#[derive(Resource, Default)]
pub struct Particles {
    gens: Vec<Gen>,
    built: bool,
    quad: Option<Handle<Mesh>>,
    seed: u64,
}

#[derive(Component)]
pub struct ParticleSprite;

fn f(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        _ => d,
    }
}
fn b(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn rot(p: &[Property]) -> [i32; 3] {
    match find(p, "Rotation") {
        Some(Value::Rotator(r)) => *r,
        _ => [0; 3],
    }
}
fn forward(r: [i32; 3]) -> DVec3 {
    let ax = rotator_axes(r);
    DVec3::new(ax[0][0] as f64, ax[0][1] as f64, ax[0][2] as f64)
}

fn build(level: &dx_level::Level, blockers: &crate::actors::Blockers) -> Vec<Gen> {
    let mut out = Vec::new();
    for a in &level.actors {
        if !matches!(a.class.as_str(), "ParticleGenerator" | "WaterDrips") {
            continue;
        }
        let p = &a.props;
        let l = a.location();
        let triggered = b(p, "bTriggered", false);
        out.push(Gen {
            tag: crate::devices::name_prop(p, "Tag"),
            pos: DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
            dir: forward(rot(p)),
            frequency: f(p, "Frequency", 1.0),
            rise: f(p, "RiseRate", 10.0) as f64,
            eject: f(p, "ejectSpeed", 10.0) as f64,
            tex: crate::sound::prop(p, "particleTexture").unwrap_or_else(|| "Effects.Smoke.SmokePuff1".into()),
            life: f(p, "particleLifeSpan", 4.0),
            draw_scale: f(p, "particleDrawScale", 0.1),
            scale: b(p, "bScale", true),
            fade: b(p, "bFade", true),
            random: b(p, "bRandomEject", false),
            check: f(p, "checkTime", 0.1),
            gravity: b(p, "bGravity", false),
            num: f(p, "numPerSpawn", 1.0) as i32,
            spewing: !triggered || b(p, "bInitiallyOn", true),
            faucet: None,
            follow: None,
            time: 0.0,
            parts: Vec::new(),
            size: 0.0,
            aspect: 1.0,
            mat: None,
            lifespan: None,
            spawn_sound: None,
        });
    }
    // Faucet.PostBeginPlay: a generator CollisionRadius x 0.9 in front of the tap, pitched down 12288.
    for (bi, c) in blockers.list.iter().enumerate() {
        let a = &level.actors[c.actor];
        if a.class != "Faucet" {
            continue;
        }
        let p = &a.props;
        let r = rot(p);
        let l = a.location();
        let pos = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            + forward(r) * (f(p, "CollisionRadius", 8.0) as f64 * 0.9);
        out.push(Gen {
            tag: String::new(),
            pos,
            dir: forward([r[0] - 12288, r[1], r[2]]),
            frequency: 1.0,
            rise: 10.0,
            eject: 50.0,
            tex: "Effects.Generated.WtrDrpSmall".into(),
            life: 0.5,
            draw_scale: 0.05,
            scale: false,
            fade: true,
            random: true,
            check: 0.05,
            gravity: true,
            num: 5,
            spewing: false,
            faucet: Some(bi),
            follow: None,
            time: 0.0,
            parts: Vec::new(),
            size: 0.0,
            aspect: 1.0,
            mat: None,
            lifespan: None,
            spawn_sound: None,
        });
    }
    // HKIncenseBurner.PostBeginPlay: a smoke generator at 0.6 x CollisionHeight, straight up.
    for (bi, c) in blockers.list.iter().enumerate() {
        let a = &level.actors[c.actor];
        if a.class != "HKIncenseBurner" {
            continue;
        }
        let off = DVec3::new(0.0, 0.0, f(&a.props, "CollisionHeight", 8.0) as f64 * 0.6);
        out.push(spec_gen(GenSpec {
            pos: c.center + off,
            dir: DVec3::Z,
            tex: "Effects.Smoke.SmokePuff1".into(),
            draw_scale: 0.2,
            check: 0.25,
            frequency: 0.9,
            rise: 0.0,
            eject: 10.0,
            life: 4.0,
            random: true,
            follow: Some((bi, off)),
            gravity: false,
            lifespan: None,
            spawn_sound: None,
        }));
    }
    out
}

/// A generator made at run time (Barrel1's smoke leak, HKIncenseBurner).
pub struct GenSpec {
    pub pos: DVec3,
    pub dir: DVec3,
    pub tex: String,
    pub draw_scale: f32,
    pub check: f32,
    pub frequency: f32,
    pub rise: f64,
    pub eject: f64,
    pub life: f32,
    pub random: bool,
    pub follow: Option<(usize, DVec3)>,
    pub gravity: bool,
    pub lifespan: Option<f32>,
    pub spawn_sound: Option<String>,
}

fn spec_gen(g: GenSpec) -> Gen {
    Gen {
        tag: String::new(),
        pos: g.pos,
        dir: g.dir,
        frequency: g.frequency,
        rise: g.rise,
        eject: g.eject,
        tex: g.tex,
        life: g.life,
        draw_scale: g.draw_scale,
        scale: true,
        fade: true,
        random: g.random,
        check: g.check,
        gravity: g.gravity,
        num: 1,
        spewing: true,
        faucet: None,
        follow: g.follow,
        time: 0.0,
        parts: Vec::new(),
        size: 0.0,
        aspect: 1.0,
        mat: None,
        lifespan: g.lifespan,
        spawn_sound: g.spawn_sound,
    }
}

static ADD: Mutex<Vec<GenSpec>> = Mutex::new(Vec::new());

pub fn add_generator(g: GenSpec) {
    if let Ok(mut q) = ADD.lock() {
        q.push(g);
    }
}

fn tex_size(assets: &mut RuntimeAssets, tex: &str) -> (f32, f32) {
    if let Some(fx) = assets.lib.load_fire(tex) {
        return (fx.width as f32, fx.height as f32);
    }
    assets.lib.load(tex).map(|(t, _)| (t.width() as f32, t.height() as f32)).unwrap_or((32.0, 32.0))
}

#[allow(clippy::too_many_arguments)]
pub fn particle_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    fixtures: Res<crate::fixtures::Fixtures>,
    mut st: ResMut<Particles>,
    cam: Single<&Transform, (With<crate::FlyCam>, Without<crate::SkyCam>, Without<ParticleSprite>)>,
    mut sprites: Query<(&mut Transform, &mut Visibility), With<ParticleSprite>>,
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    let _prof = crate::prof::scope("particles::particle_system");
    let st = &mut *st;
    if !st.built {
        st.gens = build(&level.0, &blockers);
        st.built = true;
        st.seed = 0x9A27_1C1E;
    }
    for g in ADD.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        st.gens.push(spec_gen(g));
    }
    if st.gens.is_empty() {
        return;
    }
    let quad = st.quad.get_or_insert_with(|| meshes.add(Rectangle::new(1.0, 1.0))).clone();
    let dt = time.delta_secs().min(0.1);
    let inbox: Vec<(String, bool)> = INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let eye = dx_level::bevy_to_ue(cam.translation.to_array());
    let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
    let look = cam.forward();
    let mut seed = st.seed;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 11) as f64 / (1u64 << 53) as f64, seed)
    };
    for g in st.gens.iter_mut() {
        for (n, on) in &inbox {
            if !g.tag.is_empty() && n.eq_ignore_ascii_case(&g.tag) {
                // Trigger toggles; UnTrigger stops.
                g.spewing = if *on { !g.spewing } else { false };
            }
        }
        if let Some(fb) = g.faucet {
            g.spewing = fixtures.is_on(fb);
        }
        if let Some(t) = g.lifespan.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                g.spewing = false;
            }
        }
        if let Some((b, off)) = g.follow {
            match blockers.list.get(b).filter(|c| c.radius > 0.0) {
                Some(c) => g.pos = c.center + off,
                None => g.spewing = false,
            }
        }
        // bFrozen: near the player (320) or seen lately; here, near or in front of the view and within range.
        let to = g.pos - eye;
        let d = to.length();
        let in_front = {
            let lb = Vec3::from(ue_to_bevy(to.as_vec3().to_array()));
            lb.dot(*look) > -0.2 * lb.length()
        };
        let frozen = !(d < 320.0 || (d < 4000.0 && in_front));
        if frozen {
            for p in &g.parts {
                if let Ok((_, mut v)) = sprites.get_mut(p.entity) {
                    *v = Visibility::Hidden;
                }
            }
            continue;
        }
        if g.mat.is_none() {
            let img = image_for(&mut assets, &mut images, &g.tex);
            let (w, h) = tex_size(&mut assets, &g.tex);
            g.size = w;
            g.aspect = h / w.max(1.0);
            g.mat = Some(materials.add(StandardMaterial {
                base_color_texture: img,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Add,
                ..default()
            }));
        }
        // Spew.
        if g.spewing {
            g.time += dt;
            if g.time > g.check {
                g.time = 0.0;
                if (rnd().0 as f32) <= g.frequency {
                    if let Some(snd) = &g.spawn_sound {
                        crate::sound::play(snd, g.pos, 1.0, 1024.0, 1.0);
                    }
                    for _ in 0..g.num.max(0) {
                        if g.parts.len() >= 64 {
                            break;
                        }
                        let mut vel = g.dir * g.eject;
                        if g.random {
                            let v = loop {
                                let v = DVec3::new(rnd().0 * 2.0 - 1.0, rnd().0 * 2.0 - 1.0, rnd().0 * 2.0 - 1.0);
                                if v.length_squared() <= 1.0 && v.length_squared() > 1e-6 {
                                    break v.normalize();
                                }
                            };
                            vel += v * 0.2 * g.eject;
                        }
                        let entity = commands
                            .spawn((
                                Mesh3d(quad.clone()),
                                MeshMaterial3d(g.mat.clone().unwrap()),
                                Transform::from_translation(Vec3::from(ue_to_bevy(g.pos.as_vec3().to_array())))
                                    .with_scale(Vec3::ZERO),
                                Visibility::Inherited,
                                ParticleSprite,
                                bevy::light::NotShadowCaster,
                            ))
                            .id();
                        g.parts.push(Particle { pos: g.pos, vel, life: g.life, entity });
                    }
                }
            }
        }
        if std::env::var("DXVIEW_PARTLOG").is_ok() && g.spewing {
            println!("PART {} at {:.0},{:.0},{:.0} n={} tex={} size={}", if g.faucet.is_some() { "faucet" } else { &g.tag }, g.pos.x, g.pos.y, g.pos.z, g.parts.len(), g.tex, g.size);
        }
        // UpdateParticles.
        let (life0, size, ds, aspect) = (g.life.max(0.001), g.size, g.draw_scale, g.aspect);
        let (scale, fade, gravity, rise) = (g.scale, g.fade, g.gravity, g.rise);
        g.parts.retain_mut(|p| {
            p.life -= dt;
            if p.life <= 0.0 {
                commands.entity(p.entity).despawn();
                return false;
            }
            if gravity {
                p.vel.z += -950.0 * dt as f64;
            }
            p.pos += (p.vel + DVec3::new(0.0, 0.0, if gravity { 0.0 } else { rise })) * dt as f64;
            let k = 1.0 - p.life / life0;
            let grow = if scale { 1.0 + 2.0 * k } else { 1.0 };
            let glow = if fade { 1.0 - k } else { 1.0 };
            if let Ok((mut tf, mut v)) = sprites.get_mut(p.entity) {
                tf.translation = Vec3::from(ue_to_bevy(p.pos.as_vec3().to_array()));
                tf.rotation = cam.rotation;
                // Fading is drawn as shrinking brightness; additive blending makes a smaller sprite read dimmer.
                // The software renderer never draws a sprite under a pixel or so: tiny water drops stay visible.
                let w = (size * ds * grow * glow.max(0.15).sqrt()).max(p.pos.distance(eye) as f32 * 0.0025);
                tf.scale = Vec3::new(w, w * aspect, w) * dx_level::METERS_PER_UNIT;
                *v = Visibility::Inherited;
            }
            true
        });
    }
    st.seed = rnd().1;
    // LifeSpan ran out and the last particle has died: the generator goes.
    st.gens.retain(|g| !(g.lifespan.is_some_and(|t| t <= 0.0) && g.parts.is_empty()));
}

/// A new map: drop generators queued for the old one.
pub fn reset_level() {
    if let Ok(mut q) = ADD.lock() {
        q.clear();
    }
    if let Ok(mut q) = INBOX.lock() {
        q.clear();
    }
}
