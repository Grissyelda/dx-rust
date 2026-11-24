//! Sector lockdowns (MULTIPLAYER roadmap Phase 3, Modern; the room's "Lockdowns" switch).
//!
//! Each map's data names its sectors (data/deathmatch.ron `lockdowns`): the security computers that control one, its
//! doors and its hazard. From the Overwatch console (overwatch.rs) the seated player seals a sector: its doors shut
//! and lock, and its hazard runs for `secs` - gas, steam or fire beside the sealed doors (or at the data's spots), or
//! the current through the water zones tagged `zone` ("shock"). Then the sector can't be sealed again for `cooldown`.
//! The hazard hurts every player in it (each player's own game hurts him; the kill goes to whoever sealed it).
//!
//! Answers (counters.rs): a multitool forces a sealed door after `override_secs`; a hazmat suit takes the hazard's
//! damage down to `hazard_mult`.

use crate::mech::{self, MechMsg};
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

#[derive(Default, Clone)]
struct Sector {
    active: f32,
    cool: f32,
    by: u8,
}

#[derive(Default)]
struct State {
    sectors: Vec<Sector>,
    /// The map these sectors belong to (loader generation).
    gen: u64,
    /// Host: sectors to seal / open now.
    start: Vec<u8>,
    stop: Vec<u8>,
    /// Host: doors forced with a multitool.
    forced: Vec<u16>,
    /// Hazard spots per sector, worked out once per map (and the water zones of a "shock").
    spots: Vec<Vec<DVec3>>,
    zones: Vec<Vec<usize>>,
    /// The doors each sector holds shut (mover index) and whether they were locked before.
    held: Vec<Vec<(usize, bool)>>,
    send_t: f32,
    hurt_t: f32,
    /// Our multitool on a sealed door: (mover, seconds so far).
    forcing: Option<(usize, f32)>,
    /// Hazard effects spawned for these sectors already.
    fx: Vec<bool>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn enabled() -> bool {
    mech::on(|r| r.lockdowns)
}

fn specs(lvl: &dx_level::Level) -> &'static [crate::deathmatch::LockdownSpec] {
    crate::deathmatch::map_entry(&lvl.name).map_or(&[], |m| m.lockdowns.as_slice())
}

fn secs_of(s: &crate::deathmatch::LockdownSpec) -> (f32, f32) {
    let t = mech::tuning();
    (s.secs.unwrap_or(t.lockdown_secs), s.cooldown.unwrap_or(t.lockdown_cooldown))
}

/// The sectors a security computer controls: (index, name, seconds running, seconds of cooldown left).
pub fn sectors_for(lvl: &dx_level::Level, seat: usize) -> Vec<(u8, String, f32, f32)> {
    if !enabled() {
        return Vec::new();
    }
    let Some(name) = lvl.actors.get(seat).map(|a| a.name.clone()) else { return Vec::new() };
    let state = with(|s| s.sectors.clone());
    specs(lvl)
        .iter()
        .enumerate()
        .filter(|(_, sp)| sp.terminals.iter().any(|t| t.eq_ignore_ascii_case(&name)))
        .map(|(i, sp)| {
            let st = state.get(i).cloned().unwrap_or_default();
            (i as u8, sp.name.clone(), st.active, st.cool)
        })
        .collect()
}

/// The console's button.
pub fn ask(i: u8) {
    mech::to_host(MechMsg::LockdownAsk(i));
}

/// Host: `from` seals sector `i`.
pub fn host_ask(from: u8, i: u8) {
    if !enabled() || !mech::host() {
        return;
    }
    let ok = with(|s| {
        let Some(st) = s.sectors.get_mut(i as usize) else { return false };
        if st.active > 0.0 || st.cool > 0.0 {
            return false;
        }
        st.by = from;
        s.start.push(i);
        true
    });
    if !ok {
        crate::netplay::tell(from, "That sector isn't ready.");
    }
}

/// The host's sector states.
pub fn set_state(v: Vec<(f32, f32, u8)>) {
    with(|s| {
        s.sectors = v.into_iter().map(|(active, cool, by)| Sector { active, cool, by }).collect();
    });
}

/// Host: a joiner's multitool forced a sealed door.
pub fn host_override(_from: u8, mover: u16) {
    if mech::host() {
        with(|s| s.forced.push(mover));
    }
}

/// A door held shut by a running lockdown.
pub fn sealed(mover: usize) -> bool {
    with(|s| s.held.iter().enumerate().any(|(i, h)| s.sectors.get(i).is_some_and(|x| x.active > 0.0) && h.iter().any(|d| d.0 == mover)))
}

/// pickups: Use on a door. A multitool on a sealed one starts forcing it (counter "Multitool"). True when taken.
pub fn try_override(mover: usize, multitool: bool, hud: &mut crate::movers::HudMessage) -> bool {
    if !enabled() || !sealed(mover) {
        return false;
    }
    if !multitool {
        hud.text = "Sealed by a lockdown. A multitool could force it.".into();
        hud.time_left = 2.0;
        return true;
    }
    with(|s| {
        if s.forcing.is_none_or(|f| f.0 != mover) {
            s.forcing = Some((mover, 0.0));
        }
    });
    true
}

/// HUD lines: sectors sealed right now.
pub fn hud_lines() -> Vec<(String, [u8; 3])> {
    if !enabled() {
        return Vec::new();
    }
    let names: Vec<String> = crate::deathmatch::rules().map(|r| r.map).and_then(|m| crate::deathmatch::map_entry(&m)).map(|m| m.lockdowns.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
    with(|s| {
        s.sectors
            .iter()
            .enumerate()
            .filter(|(_, x)| x.active > 0.0)
            .map(|(i, x)| (format!("LOCKDOWN: {} {}s", names.get(i).cloned().unwrap_or_default().to_uppercase(), x.active.ceil() as i32), [255, 90, 60]))
            .collect()
    })
}

pub fn reset_level() {
    with(|s| *s = State::default());
}

/// Damage type of a hazard.
fn kind_of(hazard: &str) -> &'static str {
    match hazard {
        "gas" => "PoisonGas",
        "shock" => "Shocked",
        _ => "Burned",
    }
}

/// Where a sector's hazard is: its data spots, else the nav points within reach of each sealed door (both sides).
fn hazard_spots(lvl: &dx_level::Level, sp: &crate::deathmatch::LockdownSpec) -> Vec<DVec3> {
    if !sp.spots.is_empty() {
        return sp.spots.iter().map(|p| DVec3::new(p.0 as f64, p.1 as f64, p.2 as f64)).collect();
    }
    let mut out: Vec<DVec3> = Vec::new();
    for tag in &sp.doors {
        for m in lvl.movers.iter().filter(|m| m.tag.eq_ignore_ascii_case(tag)) {
            let at = DVec3::new(m.location[0] as f64, m.location[1] as f64, m.location[2] as f64);
            let mut near: Vec<(f64, DVec3)> =
                lvl.nav.points.iter().map(|p| DVec3::new(p.pos[0], p.pos[1], p.pos[2])).map(|p| (p.distance(at), p)).filter(|(d, _)| *d < 360.0).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, p) in near.into_iter().take(2) {
                if !out.iter().any(|q| q.distance(p) < 96.0) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// The water zones a "shock" electrifies: tagged `zone`, or holding one of its spots.
fn shock_zones(lvl: &dx_level::Level, sp: &crate::deathmatch::LockdownSpec) -> Vec<usize> {
    let mut v: Vec<usize> = lvl.zone_rules.iter().enumerate().filter(|(_, r)| !sp.zone.is_empty() && r.tag.eq_ignore_ascii_case(&sp.zone)).map(|(i, _)| i).collect();
    for p in &sp.spots {
        let z = lvl.collision.zone_at(DVec3::new(p.0 as f64, p.1 as f64, p.2 as f64));
        if z > 0 && !v.contains(&(z as usize)) {
            v.push(z as usize);
        }
    }
    v
}

#[allow(clippy::too_many_arguments)]
pub fn lockdown_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut movers: ResMut<crate::movers::Movers>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    player: Res<crate::Player>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut msg: ResMut<crate::movers::HudMessage>,
) {
    if !enabled() {
        return;
    }
    let lvl = &level.0;
    let dt = time.delta_secs();
    let sp = specs(lvl);
    // A new map: its sectors, their hazard spots.
    let gen = crate::loader::generation();
    if with(|s| s.gen != gen) {
        let spots: Vec<Vec<DVec3>> = sp.iter().map(|x| if x.hazard == "shock" { Vec::new() } else { hazard_spots(lvl, x) }).collect();
        let zones: Vec<Vec<usize>> = sp.iter().map(|x| if x.hazard == "shock" { shock_zones(lvl, x) } else { Vec::new() }).collect();
        with(|s| {
            *s = State { gen, sectors: vec![Sector::default(); sp.len()], spots, zones, held: vec![Vec::new(); sp.len()], fx: vec![false; sp.len()], ..Default::default() };
        });
    }
    // Our multitool on a sealed door (counter "Multitool").
    if let Some((m, t)) = with(|s| s.forcing) {
        let need = crate::counters::value("Lockdown", "Multitool", "override_secs", 5.0);
        let at = movers.rt.get(m).map(|r| r.location).unwrap_or_default();
        if !sealed(m) || at.distance(player.body.location) > 160.0 || ph.dead {
            with(|s| s.forcing = None);
        } else if t + dt >= need {
            with(|s| s.forcing = None);
            inv.0.use_one("Multitool");
            if mech::host() {
                with(|s| s.forced.push(m as u16));
            } else {
                mech::to_host(MechMsg::Override(m as u16));
            }
            msg.text = "Door forced.".into();
            msg.time_left = 2.0;
        } else {
            with(|s| s.forcing = Some((m, t + dt)));
            msg.text = format!("Forcing the sealed door... {}%", ((t + dt) / need * 100.0) as i32);
            msg.time_left = 0.3;
        }
    }
    // Host: start, stop, forced doors, the clocks, telling everyone.
    if mech::host() {
        let (start, forced) = with(|s| (std::mem::take(&mut s.start), std::mem::take(&mut s.forced)));
        for i in start {
            let Some(spec) = sp.get(i as usize) else { continue };
            let (secs, cool) = secs_of(spec);
            let mut held = Vec::new();
            for (j, info) in lvl.movers.iter().enumerate() {
                if spec.doors.iter().any(|t| t.eq_ignore_ascii_case(&info.tag)) {
                    held.push((j, movers.rt[j].locked));
                    crate::movers::seal(info, &mut movers.rt[j]);
                }
            }
            let by = with(|s| {
                let st = &mut s.sectors[i as usize];
                st.active = secs;
                st.cool = secs + cool;
                s.held[i as usize] = held;
                st.by
            });
            let who = if by == crate::deathmatch::my_id() { crate::travel::player_name() } else { crate::netplay::players().into_iter().find(|p| p.0 == by).map_or_else(String::new, |p| p.1) };
            let line = format!("LOCKDOWN: {} sealed by {who}.", spec.name);
            crate::netplay::host_news(&line);
            println!("{line}");
        }
        for m in forced {
            let m = m as usize;
            if m < movers.rt.len() {
                crate::movers::force_open(&lvl.movers[m], &mut movers.rt[m]);
                with(|s| {
                    for h in &mut s.held {
                        h.retain(|d| d.0 != m);
                    }
                });
            }
        }
        let ended: Vec<usize> = with(|s| {
            let mut ended = Vec::new();
            for (i, st) in s.sectors.iter_mut().enumerate() {
                let was = st.active > 0.0;
                st.active = (st.active - dt).max(0.0);
                st.cool = (st.cool - dt).max(0.0);
                if was && st.active <= 0.0 {
                    ended.push(i);
                }
            }
            ended
        });
        for i in ended {
            for (j, locked) in with(|s| std::mem::take(&mut s.held[i])) {
                movers.rt[j].locked = locked;
            }
            if let Some(spec) = sp.get(i) {
                crate::netplay::host_news(&format!("LOCKDOWN: {} open again.", spec.name));
            }
        }
        let send = with(|s| {
            s.send_t -= dt;
            if s.send_t <= 0.0 {
                s.send_t = 1.0;
                Some(s.sectors.iter().map(|x| (x.active, x.cool, x.by)).collect::<Vec<_>>())
            } else {
                None
            }
        });
        if let Some(v) = send {
            mech::to_joiners(MechMsg::Lockdowns(v));
        }
    } else {
        with(|s| {
            for st in &mut s.sectors {
                st.active = (st.active - dt).max(0.0);
                st.cool = (st.cool - dt).max(0.0);
            }
        });
    }
    // The hazards: their look (once per lockdown), and what they do to this player.
    let (sectors, spots, zones) = with(|s| (s.sectors.clone(), s.spots.clone(), s.zones.clone()));
    for (i, st) in sectors.iter().enumerate() {
        let Some(spec) = sp.get(i) else { continue };
        let started = with(|s| {
            let was = s.fx.get(i).copied().unwrap_or(false);
            if let Some(f) = s.fx.get_mut(i) {
                *f = st.active > 0.0;
            }
            st.active > 0.0 && !was
        });
        if started {
            crate::sound::play_player("DeusExSounds.Generic.Klaxon2", 0.4, 0.0);
            for p in spots.get(i).into_iter().flatten() {
                let (tex, scale, freq, rise) = match spec.hazard.as_str() {
                    "steam" => ("Effects.Smoke.SmokePuff1", 1.2, 0.05, 220.0),
                    "fire" => ("Effects.Fire.OneFlame_J", 0.5, 0.06, 90.0),
                    _ => ("Effects.Smoke.SmokePuff1", 2.4, 0.12, 25.0),
                };
                crate::particles::add_generator(crate::particles::GenSpec {
                    pos: *p,
                    dir: DVec3::Z,
                    tex: tex.into(),
                    draw_scale: scale,
                    check: 0.05,
                    frequency: freq,
                    rise,
                    eject: 60.0,
                    life: 3.0,
                    random: true,
                    follow: None,
                    gravity: false,
                    lifespan: Some(st.active),
                    spawn_sound: None,
                });
            }
        }
    }
    let tick = with(|s| {
        s.hurt_t -= dt;
        if s.hurt_t <= 0.0 {
            s.hurt_t = 0.5;
            true
        } else {
            false
        }
    });
    if !tick || ph.dead {
        return;
    }
    let me = player.body.location;
    let my_zone = lvl.collision.zone_at(me).max(0) as usize;
    let t = mech::tuning();
    for (i, st) in sectors.iter().enumerate().filter(|(_, st)| st.active > 0.0) {
        let Some(spec) = sp.get(i) else { continue };
        let inside = if spec.hazard == "shock" {
            zones.get(i).is_some_and(|z| z.contains(&my_zone))
        } else {
            spots.get(i).into_iter().flatten().any(|p| p.distance(me) < 170.0 && lvl.collision.trace(*p + DVec3::Z * 20.0, me, DVec3::ZERO).fraction >= 0.999)
        };
        if !inside {
            continue;
        }
        let dps = t.hazard_dps.get(&spec.hazard).copied().unwrap_or(8.0);
        let mult = if ph.hazmat { crate::counters::value("Lockdown", "HazMatSuit", "hazard_mult", 0.25) } else { 1.0 };
        if st.by != crate::deathmatch::my_id() {
            crate::netplay::note_hurt_by(st.by);
        }
        ph.damage(dps * 0.5 * mult, kind_of(&spec.hazard), DVec3::new(0.0, 0.0, 0.0));
    }
}
