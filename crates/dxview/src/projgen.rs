//! Smaller scripted actors:
//!
//! * ProjectileGenerator (Tick): while spewing, every CheckTime a `Frequency` chance of NumPerSpawn projectiles
//!   (up to 5 spews a tick) along its Rotation (or VRand() x FRand() with bRandomEject) at EjectSpeed, living
//!   ProjectileLifeSpan; SpewTime > 0 stops it after that long, WaitTime > 0 restarts it; bTriggered ones start off
//!   unless bInitiallyOn; Trigger toggles (bOnlyOnce: never back on). The helibase's poison-gas purge, the storage
//!   room's rockets, Area 51's fire jets. Barrel1's poison leak is one too (attached to the barrel).
//! * Barrel1: radioactive barrels give 5 Radiation a second to what they can see within 128; a damaged poison or
//!   biohazard barrel leaks gas (a ProjectileGenerator of PoisonGas and a SteamVent2 hiss), a damaged flammable one
//!   smokes; destroyed poison barrels leave explosionRadius / 36 floating PoisonGas clouds.
//! * AmbientSoundTriggered: its AmbientSound only while bActive; Trigger toggles, UnTrigger stops (bTriggerOnceOnly).
//! * RandomSounds: every second a weighted pick of sounds[] - an ambient loop for a random duration (with fades and
//!   a fake doppler) or a one-shot.
//! * SequenceEvents: Events[] triggered in turn with Delays[], looping after RestartTime with bLoop, only while the
//!   player can see it with bLOSRequired.
//! * AlarmLight: Trigger turns it on (lens skin, light, spinning), UnTrigger off.

use crate::actors::{Deco, MeshActorRef};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashSet;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

static INBOX: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static BARREL_HITS: Mutex<Vec<(usize, usize, bool)>> = Mutex::new(Vec::new());

pub fn event(name: &str, on: bool) {
    if let Ok(mut q) = INBOX.lock() {
        q.push((name.to_string(), on));
    }
}

/// Barrel1.TakeDamage got past its filters: (blocker, actor, destroyed).
pub fn barrel_hit(blocker: usize, actor: usize, destroyed: bool) {
    if let Ok(mut q) = BARREL_HITS.lock() {
        q.push((blocker, actor, destroyed));
    }
}

pub fn reset_level() {
    if let Ok(mut q) = INBOX.lock() {
        q.clear();
    }
    if let Ok(mut q) = BARREL_HITS.lock() {
        q.clear();
    }
}

fn fl(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}
fn bl(p: &[Property], n: &str, d: bool) -> bool {
    match find(p, n) {
        Some(Value::Bool(v)) => *v,
        _ => d,
    }
}
fn byte_at(p: &[Property], n: &str, i: u32) -> u8 {
    p.iter()
        .find(|x| x.name.eq_ignore_ascii_case(n) && x.index == i)
        .map(|x| match x.value {
            Value::Byte(b) => b,
            Value::Int(v) => v as u8,
            _ => 0,
        })
        .unwrap_or(0)
}
fn float_at(p: &[Property], n: &str, i: u32) -> f32 {
    p.iter()
        .find(|x| x.name.eq_ignore_ascii_case(n) && x.index == i)
        .map(|x| match x.value {
            Value::Float(v) => v,
            Value::Int(v) => v as f32,
            _ => 0.0,
        })
        .unwrap_or(0.0)
}
fn path_at(p: &[Property], n: &str, i: u32) -> Option<String> {
    p.iter().find(|x| x.name.eq_ignore_ascii_case(n) && x.index == i).and_then(|x| match &x.value {
        Value::Path(s) if s != "None" => Some(s.clone()),
        _ => None,
    })
}
fn name_at(p: &[Property], n: &str, i: u32) -> String {
    p.iter()
        .find(|x| x.name.eq_ignore_ascii_case(n) && x.index == i)
        .and_then(|x| match &x.value {
            Value::Name(s) if s != "None" => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
fn loc(a: &dx_level::actors::ActorInfo) -> DVec3 {
    let l = a.location();
    DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
}
fn rot_dir(p: &[Property]) -> DVec3 {
    let r = match find(p, "Rotation") {
        Some(Value::Rotator(r)) => *r,
        _ => [0; 3],
    };
    let ax = dx_level::actors::rotator_axes(r);
    DVec3::new(ax[0][0] as f64, ax[0][1] as f64, ax[0][2] as f64)
}

struct ProjGen {
    tag: String,
    pos: DVec3,
    dir: DVec3,
    class: String,
    frequency: f32,
    eject: f64,
    life: f32,
    spew_time: f32,
    wait_time: f32,
    random: bool,
    check: f32,
    num: i32,
    only_once: bool,
    spawn_sound: Option<String>,
    sound_radius: f32,
    spewing: bool,
    period: f32,
    time: f32,
    /// Attached (SetBase) to this blocker: rides it, destroyed with it.
    follow: Option<(usize, DVec3)>,
    gone: bool,
}

struct RandSnd {
    pos: DVec3,
    sounds: Vec<(String, f32, bool, f32, f32, u8, u8, bool, bool)>,
    cur: usize,
    timer: f32,
    duration: f32,
    playing: bool,
    radius: f32,
    volume: f32,
    pitch: f32,
}

struct SeqEv {
    pos: DVec3,
    events: Vec<(String, f32)>,
    restart: f32,
    looping: bool,
    los: bool,
    /// (next event, time left before it).
    at: Option<(usize, f32)>,
    wait: f32,
}

#[derive(Default)]
pub struct State {
    built: bool,
    gens: Vec<ProjGen>,
    rand: Vec<RandSnd>,
    seqs: Vec<SeqEv>,
    /// AmbientSoundTriggered: (actor, tag, active, once, hit).
    amb: Vec<(usize, String, bool, bool, bool)>,
    /// AlarmLight: (actor, tag, on).
    alarms: Vec<(usize, String, bool)>,
    /// Radioactive barrels by blocker, with their 1 s timer.
    rad: Vec<(usize, f32)>,
    leaking: HashSet<usize>,
    /// Hissing leaks: (blocker, sound radius).
    hiss: Vec<(usize, f32)>,
    seed: u64,
}

impl State {
    fn rand(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
    fn vrand(&mut self) -> DVec3 {
        loop {
            let v = DVec3::new(self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0);
            if v.length_squared() <= 1.0 && v.length_squared() > 1e-4 {
                return v.normalize();
            }
        }
    }
}

fn class_of(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn projgen_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<crate::actors::Blockers>,
    player: Res<crate::Player>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut launches: ResMut<crate::projectiles::Launches>,
    mut pawn_hits: ResMut<crate::damage::PawnHits>,
    npcs: Query<&crate::ai::Npc>,
    mut decos: Query<(Entity, &MeshActorRef, &Deco)>,
    mut map_gen: Local<u64>,
    mut st: Local<State>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *st = State::default();
    }
    let lvl = &level.0;
    if !st.built {
        st.built = true;
        st.seed = 0x7072_6F6A_6765_6E31;
        for (i, a) in lvl.actors.iter().enumerate() {
            let p = &a.props;
            let tag = name_at(p, "Tag", 0);
            match a.class.as_str() {
                "ProjectileGenerator" => {
                    let triggered = bl(p, "bTriggered", false);
                    st.gens.push(ProjGen {
                        tag,
                        pos: loc(a),
                        dir: rot_dir(p),
                        class: path_at(p, "ProjectileClass", 0).map(|s| class_of(&s)).unwrap_or_else(|| "Fireball".into()),
                        frequency: fl(p, "Frequency", 1.0),
                        eject: fl(p, "ejectSpeed", 300.0) as f64,
                        life: fl(p, "ProjectileLifeSpan", 0.0),
                        spew_time: fl(p, "spewTime", 0.0),
                        wait_time: fl(p, "WaitTime", 0.0),
                        random: bl(p, "bRandomEject", false),
                        check: fl(p, "checkTime", 0.25),
                        num: fl(p, "numPerSpawn", 1.0) as i32,
                        only_once: bl(p, "bOnlyOnce", false),
                        spawn_sound: path_at(p, "SpawnSound", 0),
                        sound_radius: fl(p, "SpawnSoundRadius", 1024.0),
                        spewing: !(triggered && !bl(p, "bInitiallyOn", true)) && bl(p, "bSpewing", true),
                        period: 0.0,
                        time: 0.0,
                        follow: None,
                        gone: false,
                    });
                }
                "RandomSounds" => {
                    let mut sounds = Vec::new();
                    for k in 0..8u32 {
                        let Some(s) = path_at(p, "sounds", k) else { break };
                        sounds.push((
                            s,
                            float_at(p, "frequency", k),
                            byte_at(p, "bAmbient", k) > 0,
                            float_at(p, "minDuration", k),
                            float_at(p, "maxDuration", k),
                            byte_at(p, "Volume", k),
                            byte_at(p, "Pitch", k),
                            byte_at(p, "bFade", k) > 0,
                            byte_at(p, "bFakeDoppler", k) > 0,
                        ));
                    }
                    if !sounds.is_empty() {
                        st.rand.push(RandSnd {
                            pos: loc(a),
                            sounds,
                            cur: 0,
                            timer: 0.0,
                            duration: 0.0,
                            playing: false,
                            radius: 25.0 * (fl(p, "SoundRadius", 32.0) + 1.0),
                            volume: 0.0,
                            pitch: 1.0,
                        });
                    }
                }
                "SequenceEvents" => {
                    let mut events = Vec::new();
                    for k in 0..8u32 {
                        let e = name_at(p, "Events", k);
                        if e.is_empty() {
                            break;
                        }
                        events.push((e, float_at(p, "Delays", k)));
                    }
                    st.seqs.push(SeqEv {
                        pos: loc(a),
                        events,
                        restart: fl(p, "RestartTime", 0.0),
                        looping: bl(p, "bLoop", false),
                        los: bl(p, "bLOSRequired", false),
                        at: Some((0, 0.0)),
                        wait: 0.0,
                    });
                }
                "AmbientSoundTriggered" => {
                    let active = bl(p, "bActive", true);
                    if !active {
                        crate::sound::own_ambient(i);
                    }
                    st.amb.push((i, tag, active, bl(p, "bTriggerOnceOnly", false), false));
                }
                "AlarmLight" => {
                    let on = bl(p, "bIsOn", false);
                    if !on {
                        crate::dynlight::toggle_actor_light(i);
                    }
                    st.alarms.push((i, tag, on));
                }
                _ => {}
            }
        }
        for (bi, c) in blockers.list.iter().enumerate() {
            let a = &lvl.actors[c.actor];
            if a.class == "Barrel1" && byte_at(&a.props, "SkinColor", 0) == 8 {
                st.rad.push((bi, 0.0));
            }
        }
    }
    let dt = time.delta_secs().min(0.1);
    let inbox: Vec<(String, bool)> = INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    let pc = player.body.location;

    // ---- events
    for (name, on) in &inbox {
        for g in st.gens.iter_mut().filter(|g| !g.tag.is_empty() && g.tag.eq_ignore_ascii_case(name)) {
            // ProjectileGenerator.Trigger (UnTrigger does nothing).
            if !*on {
                continue;
            }
            if g.spewing {
                g.spewing = false;
            } else if !g.only_once {
                g.spewing = true;
            }
            g.period = 0.0;
            g.time = 0.0;
            println!("PROJGEN {} {}", g.tag, if g.spewing { "on" } else { "off" });
        }
        for (ai, tag, active, once, hit) in st.amb.iter_mut() {
            if tag.is_empty() || !tag.eq_ignore_ascii_case(name) || (*once && *hit) {
                continue;
            }
            *active = if *on { !*active } else { false };
            *hit = true;
            if *active {
                crate::sound::release_ambient(*ai);
            } else {
                crate::sound::own_ambient(*ai);
            }
        }
        for k in 0..st.alarms.len() {
            let (ai, tag, is_on) = st.alarms[k].clone();
            if tag.is_empty() || !tag.eq_ignore_ascii_case(name) || is_on == *on {
                continue;
            }
            st.alarms[k].2 = *on;
            crate::dynlight::toggle_actor_light(ai);
            crate::security::alarm_light(ai, *on);
            for (e, mr, d) in decos.iter_mut() {
                if lvl.mesh_actors[mr.0].actor == ai {
                    if *on {
                        if let Some(Value::Rotator(r)) = find(&lvl.actors[ai].props, "RotationRate") {
                            commands.entity(e).insert(crate::interp::Rotating {
                                rate: [r[0] as f64, r[1] as f64, r[2] as f64],
                                rot: [d.rotation[0] as f64, d.rotation[1] as f64, d.rotation[2] as f64],
                            });
                        }
                    } else {
                        commands.entity(e).remove::<crate::interp::Rotating>();
                    }
                }
            }
        }
    }

    // ---- Barrel1
    for (b, ai, destroyed) in BARREL_HITS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        let p = lvl.actors[ai].props.clone();
        let skin = byte_at(&p, "SkinColor", 0);
        let poison = skin == 0 || skin == 7;
        let flammable = matches!(skin, 4 | 5 | 6);
        let center = blockers.list.get(b).map(|c| c.center).unwrap_or_else(|| loc(&lvl.actors[ai]));
        let (radius, height) = (fl(&p, "CollisionRadius", 20.0) as f64, fl(&p, "CollisionHeight", 29.0) as f64);
        if destroyed {
            st.leaking.remove(&b);
            st.hiss.retain(|h| h.0 != b);
            for g in st.gens.iter_mut().filter(|g| g.follow.is_some_and(|f| f.0 == b)) {
                g.gone = true;
            }
            if poison {
                // Spread out a gas cloud.
                let er = fl(&p, "explosionRadius", 768.0) as f64;
                for _ in 0..(er / 36.0) as i32 {
                    let mut at = center;
                    at.x += st.rand() * er - er * 0.5;
                    at.y += st.rand() * er - er * 0.5;
                    at.z += 32.0;
                    let (scale, life) = ((st.rand() * 0.5 + 2.0) as f32, (st.rand() * 10.0 + 30.0) as f32);
                    launches.0.push(crate::projectiles::Launch {
                        class: "PoisonGas".into(),
                        pos: at,
                        dir: DVec3::X,
                        from: None,
                        mult: 1.0,
                        life: Some(life),
                        scale: Some(scale),
                        still: true,
                        track: None,
                        track_at: None,
                        speed: None,
                        ownerless: true,
                    });
                }
            }
            continue;
        }
        if st.leaking.contains(&b) {
            continue;
        }
        let size = (radius / 54.0).clamp(0.1, 4.0);
        let off = DVec3::new(0.0, 0.0, height);
        if poison {
            st.leaking.insert(b);
            st.gens.push(ProjGen {
                tag: String::new(),
                pos: center + off,
                dir: DVec3::Z,
                class: "PoisonGas".into(),
                frequency: 0.9,
                eject: 50.0,
                life: 3.0,
                spew_time: 0.0,
                wait_time: 0.0,
                random: true,
                check: 0.5,
                num: 1,
                only_once: false,
                spawn_sound: None,
                sound_radius: 1024.0,
                spewing: true,
                period: 0.0,
                time: 0.0,
                follow: Some((b, off)),
                gone: false,
            });
            if find(&p, "AmbientSound").is_none() {
                st.hiss.push((b, 25.0 * (64.0 * size as f32 + 1.0)));
            }
        } else if flammable {
            st.leaking.insert(b);
            crate::particles::add_generator(crate::particles::GenSpec {
                pos: center + off,
                dir: DVec3::Z,
                tex: "Effects.Smoke.SmokePuff1".into(),
                draw_scale: (size * 4.0) as f32,
                check: 0.1,
                frequency: 0.9,
                rise: 90.0,
                eject: 40.0,
                life: 4.0,
                random: true,
                follow: Some((b, off)),
            });
        }
    }
    for (b, r) in st.hiss.clone() {
        if let Some(c) = blockers.list.get(b).filter(|c| c.radius > 0.0) {
            crate::sound::hold(0x4849_5353_0000 | b as u64, "Ambient.Ambient.SteamVent2", Some(c.center), 192.0 / 255.0, r, 1.0);
        }
    }
    // Radioactive barrels: every second, 5 Radiation to what they can see within 128.
    for k in 0..st.rad.len() {
        let (b, t) = st.rad[k];
        let Some(c) = blockers.list.get(b).filter(|c| c.radius > 0.0) else { continue };
        let t = t + dt;
        if t <= 1.0 {
            st.rad[k].1 = t;
            continue;
        }
        st.rad[k].1 = 0.0;
        let me = c.center;
        if pc.distance(me) <= 128.0 && lvl.collision.trace(me, pc, DVec3::ZERO).fraction >= 1.0 {
            ph.damage(5.0, "Radiation", DVec3::ZERO);
        }
        for n in npcs.iter() {
            if n.state == crate::ai::NpcState::Dead {
                continue;
            }
            if let Some(nc) = blockers.list.get(n.blocker).filter(|nc| nc.radius > 0.0) {
                if nc.center.distance(me) <= 128.0 && lvl.collision.trace(me, nc.center, DVec3::ZERO).fraction >= 1.0 {
                    pawn_hits.0.push(crate::damage::PawnHit { blocker: n.blocker, damage: 5.0, damage_type: "Radiation", hit_loc: nc.center, attacker: Some(b), attacker_pos: me });
                }
            }
        }
    }

    // ---- ProjectileGenerator.Tick
    let mut spawns = Vec::new();
    let mut sounds = Vec::new();
    for gi in 0..st.gens.len() {
        if st.gens[gi].gone {
            continue;
        }
        if let Some((b, off)) = st.gens[gi].follow {
            match blockers.list.get(b).filter(|c| c.radius > 0.0) {
                Some(c) => st.gens[gi].pos = c.center + off,
                None => {
                    st.gens[gi].gone = true;
                    continue;
                }
            }
        }
        let g = &mut st.gens[gi];
        g.time += dt;
        g.period += dt;
        if g.spewing {
            if g.spew_time > 0.0 {
                if g.period >= g.spew_time {
                    g.period = 0.0;
                    g.time = 0.0;
                    g.spewing = false;
                }
            } else {
                g.period = 0.0;
            }
        } else if !g.only_once {
            if g.wait_time > 0.0 {
                if g.period >= g.wait_time {
                    g.period = 0.0;
                    g.time = 0.0;
                    g.spewing = true;
                }
            } else {
                g.period = 0.0;
            }
        }
        if !g.spewing || g.time < g.check {
            continue;
        }
        let mut count = if g.check > 0.0 {
            let c = (g.time / g.check) as i32;
            g.time -= c as f32 * g.check;
            c
        } else {
            g.time = 0.0;
            1
        };
        count = count.min(5);
        let (time_val, check) = (g.time, g.check);
        spawns.push((gi, count, time_val, check));
        if let Some(s) = &g.spawn_sound {
            sounds.push((s.clone(), g.pos, g.sound_radius));
        }
    }
    for (gi, count, time_val, check) in spawns {
        if st.rand() as f32 > st.gens[gi].frequency {
            continue;
        }
        if let Some((s, at, r)) = sounds.iter().find(|x| x.1 == st.gens[gi].pos) {
            crate::sound::play(s, *at, 1.0, *r, 1.0);
        }
        let mut tv = time_val;
        for _ in 0..count {
            for _ in 0..st.gens[gi].num.max(0) {
                let (dir, speed) = if st.gens[gi].random {
                    let d = st.vrand();
                    (d, st.gens[gi].eject * st.rand())
                } else {
                    (st.gens[gi].dir, st.gens[gi].eject)
                };
                let g = &st.gens[gi];
                launches.0.push(crate::projectiles::Launch {
                    class: g.class.clone(),
                    pos: g.pos + dir * speed * tv as f64,
                    dir,
                    from: None,
                    mult: 1.0,
                    life: (g.life > 0.0).then_some(g.life),
                    scale: None,
                    still: false,
                    track: None,
                    track_at: None,
                    speed: Some(speed),
                    ownerless: true,
                });
            }
            tv += check;
        }
    }
    st.gens.retain(|g| !g.gone);

    // ---- RandomSounds.Tick
    for k in 0..st.rand.len() {
        let r = &mut st.rand[k];
        if r.playing {
            let s = r.sounds[r.cur].clone();
            if s.2 {
                let diff = r.duration - r.timer;
                r.volume = s.5 as f32;
                if r.timer < 2.0 && s.7 {
                    r.volume = s.5 as f32 * (r.timer / 2.0);
                }
                if diff < 2.0 && s.7 {
                    r.volume = s.5 as f32 * (diff / 2.0);
                }
                r.pitch = s.6 as f32;
                if diff < 4.0 && s.8 {
                    r.pitch = s.6 as f32 * (((diff / 4.0) * 0.15) + 0.85);
                }
                if pc.distance(r.pos) <= r.radius as f64 {
                    crate::sound::hold(0x5241_4E44_0000 | k as u64, &s.0, Some(r.pos), 0.7 * 2.0 * r.volume / 255.0, r.radius, r.pitch / 64.0);
                }
            }
            if r.timer > r.duration {
                r.timer = 0.0;
                r.playing = false;
            }
        } else if r.timer >= 1.0 {
            let rnd = {
                st.seed = st.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (st.seed >> 40) as f32 / (1u64 << 24) as f32
            };
            let dur_r = {
                st.seed = st.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (st.seed >> 40) as f32 / (1u64 << 24) as f32
            };
            let r = &mut st.rand[k];
            let mut freq = 0.0;
            let mut picked = false;
            for (i, s) in r.sounds.iter().enumerate() {
                freq += s.1;
                if rnd < freq {
                    if s.2 {
                        r.duration = dur_r * (s.4 - s.3) + s.3;
                    } else {
                        crate::sound::play(&s.0, r.pos, 1.0, r.radius, 1.0);
                        r.duration = 2.0;
                    }
                    r.cur = i;
                    r.timer = 0.0;
                    r.playing = true;
                    picked = true;
                    break;
                }
            }
            if picked {
                continue;
            }
            r.timer -= 1.0;
        }
        st.rand[k].timer += dt;
    }

    // ---- SequenceEvents
    for s in st.seqs.iter_mut() {
        if s.events.is_empty() {
            continue;
        }
        if s.wait > 0.0 {
            s.wait -= dt;
            if s.wait > 0.0 {
                continue;
            }
            s.at = Some((0, 0.0));
        }
        let Some((i, left)) = s.at.as_mut() else { continue };
        if *i == 0 && *left <= 0.0 && s.los && lvl.collision.trace(pc, s.pos, DVec3::ZERO).fraction < 1.0 {
            // Not seen this pass: straight to the loop's wait.
            s.at = None;
            if s.looping {
                s.wait = s.restart.max(0.01);
            }
            continue;
        }
        *left -= dt;
        while *left <= 0.0 {
            if *i >= s.events.len() {
                s.at = None;
                if s.looping {
                    s.wait = s.restart.max(0.01);
                }
                break;
            }
            let (e, d) = s.events[*i].clone();
            crate::triggers::note_source(&e, "");
            if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
                q.push((e, true));
            }
            *i += 1;
            *left += d;
        }
    }
}
