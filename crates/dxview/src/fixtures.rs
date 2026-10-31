//! Interactive set dressing: decorations whose Frob does something small (DeusExDecoration subclasses).
//!
//! * Faucet / ShowerFaucet: toggle open (ValveOpen / ValveClose, On / Off pose); opening puts out a burning player.
//! * Valve: toggle (ValveOpen / ValveClose, Open / Close).
//! * Switch1 / Switch2 / LightSwitch: toggle (Switch4ClickOn / Off, On / Off pose). Any Event still fires.
//! * Button1: buttonSound1 on press, buttonSound2 when it pops back after buttonLitTime.
//! * Lamp1-3: switch the light (Switch4ClickOn / Off).
//! * WaterFountain / WaterCooler: heal 1 point, bubble for 2 s (Use / Bubble, WaterBubbling); 10 uses.
//! * Toilet (9 s) / Toilet2 (2 s): flush (FlushToilet / FlushUrinal, Flush).
//! * Phone: busy tone, dial tone or one of four voices. Pinball, WHPiano: a sound, 2-3 s between uses.
//! * ShipsWheel: spins about its roll axis at up to 64k units/s, slowing to a stop over 2-7 s.
//! * VendingMachine (2 credits, soda or candy by skin) / CigaretteMachine (8 credits, cigarettes): 10 uses,
//!   the product pops out of the front.
//! * Poolball: pushed away from the player at 400.

use crate::actors::{Animated, Blockers, Deco};
use crate::weapons::Clip;
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::HashMap;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

static FROBS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn is_fixture(class: &str) -> bool {
    matches!(
        class,
        "Faucet"
            | "ShowerFaucet"
            | "Valve"
            | "Switch1"
            | "Switch2"
            | "LightSwitch"
            | "Button1"
            | "Lamp1"
            | "Lamp2"
            | "Lamp3"
            | "WaterFountain"
            | "WaterCooler"
            | "Toilet"
            | "Toilet2"
            | "Phone"
            | "Pinball"
            | "WHPiano"
            | "ShipsWheel"
            | "VendingMachine"
            | "CigaretteMachine"
            | "Poolball"
    )
}

/// The player used fixture `blocker`.
pub fn frob(blocker: usize) {
    if let Ok(mut f) = FROBS.lock() {
        f.push(blocker);
    }
}

#[derive(Default)]
struct State {
    on: bool,
    /// bUsing: seconds until it can be used again.
    using: f32,
    uses: Option<i32>,
    /// ShipsWheel: (direction x speed, duration, elapsed).
    spin: Option<(f64, f32, f32)>,
    /// Button1: seconds until it pops back out.
    button: f32,
    /// Ambient bubbling while a fountain runs.
    bubbling: f32,
}

#[derive(Resource, Default)]
pub struct Fixtures {
    state: HashMap<usize, State>,
    seed: u64,
}

impl Fixtures {
    /// Is the fixture on (a faucet open, a switch on)?
    pub fn is_on(&self, blocker: usize) -> bool {
        self.state.get(&blocker).is_some_and(|s| s.on)
    }
    /// What a save keeps (save.rs): (blocker, on, uses left) of every fixture that's been used.
    pub fn save(&self) -> Vec<(usize, bool, Option<i32>)> {
        let mut v: Vec<_> = self.state.iter().map(|(b, s)| (*b, s.on, s.uses)).collect();
        v.sort_by_key(|x| x.0);
        v
    }
    pub fn restore(&mut self, v: &[(usize, bool, Option<i32>)]) {
        self.state = v.iter().map(|(b, on, uses)| (*b, State { on: *on, uses: *uses, ..Default::default() })).collect();
    }

    fn rand(&mut self) -> f64 {
        if self.seed == 0 {
            self.seed = 0xF1C5_7E5D;
        }
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn play_anim(level: &dx_level::Level, an: &mut Animated, name: &str, looping: bool) {
    let Some(anim) = level.mesh_actors[an.mesh_actor].anim.as_ref() else {
        return;
    };
    if let Some(c) = Clip::named(&anim.mesh, name, looping) {
        an.clip = Some(c);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fixture_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    player: Res<crate::Player>,
    mut st: ResMut<Fixtures>,
    mut decos: Query<(&mut Deco, Option<&mut Animated>)>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut drops: ResMut<crate::destruct::ItemThrows>,
    mut hud: ResMut<crate::movers::HudMessage>,
) {
    let _prof = crate::prof::scope("fixtures::fixture_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    let mut frobs: Vec<usize> = FROBS
        .lock()
        .map(|mut f| std::mem::take(&mut *f))
        .unwrap_or_default();
    // Developer aid: DXVIEW_DEBUG_FIXTURE="ActorName" uses that fixture once, two seconds in.
    if let Ok(name) = std::env::var("DXVIEW_DEBUG_FIXTURE") {
        static T: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
        if let Ok(mut t) = T.lock() {
            let before = *t;
            *t += dt;
            if before < 2.0 && *t >= 2.0 {
                if let Some(b) = blockers.list.iter().position(|c| lvl.actors[c.actor].name.eq_ignore_ascii_case(&name)) {
                    frobs.push(b);
                }
            }
        }
    }
    for b in frobs {
        let Some(c) = blockers.list.get(b).copied() else {
            continue;
        };
        let Some(&e) = blockers.deco.get(&b) else {
            continue;
        };
        let Ok((mut deco, mut an)) = decos.get_mut(e) else {
            continue;
        };
        let a = &lvl.actors[c.actor];
        let class = a.class.as_str();
        let pos = c.center;
        let (r, r2, r3) = (st.rand(), st.rand(), st.rand());
        let s = st.state.entry(b).or_default();
        let mut anim = |name: &str, looping: bool| {
            if let Some(an) = an.as_deref_mut() {
                play_anim(lvl, an, name, looping);
            }
        };
        let msg = |p: &[unreal_pkg::props::Property], n: &str| match find(p, n) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        match class {
            "Faucet" | "ShowerFaucet" | "Valve" => {
                s.on = !s.on;
                let valve = class == "Valve";
                crate::sound::play(
                    if s.on {
                        "MoverSFX.Other.ValveOpen"
                    } else {
                        "MoverSFX.Other.ValveClose"
                    },
                    pos,
                    1.0,
                    256.0,
                    if valve { 1.0 } else { 2.0 },
                );
                anim(
                    if valve {
                        if s.on {
                            "Open"
                        } else {
                            "Close"
                        }
                    } else if s.on {
                        "On"
                    } else {
                        "Off"
                    },
                    false,
                );
                if s.on && !valve && ph.on_fire.is_some() {
                    ph.on_fire = None;
                }
            }
            "Switch1" | "Switch2" | "LightSwitch" => {
                crate::sound::play(
                    if s.on {
                        "DeusExSounds.Special.Switch4ClickOff"
                    } else {
                        "DeusExSounds.Special.Switch4ClickOn"
                    },
                    pos,
                    1.0,
                    512.0,
                    1.0,
                );
                anim(if s.on { "Off" } else { "On" }, false);
                s.on = !s.on;
            }
            "Button1" => {
                if s.button <= 0.0 {
                    if let Some(snd) = crate::sound::prop(&a.props, "buttonSound1") {
                        crate::sound::play(&snd, pos, 1.0, 512.0, 1.0);
                    }
                    s.button = crate::sound::fprop(&a.props, "buttonLitTime", 0.5).max(0.05);
                }
            }
            "Lamp1" | "Lamp2" | "Lamp3" => {
                let initially_on =
                    !matches!(find(&a.props, "LightType"), Some(Value::Byte(0)) | None);
                s.on = !s.on;
                let lit = initially_on != s.on;
                crate::sound::play(
                    if lit {
                        "DeusExSounds.Special.Switch4ClickOn"
                    } else {
                        "DeusExSounds.Special.Switch4ClickOff"
                    },
                    pos,
                    1.0,
                    512.0,
                    1.0,
                );
                crate::dynlight::toggle_actor_light(c.actor);
                if !lit {
                    // Smarter AI (ours): guards that see a lamp go out go and look.
                    crate::smartai::disturb(pos, crate::smartai::Disturb::LightOut);
                }
                deco.scale_glow = if lit { 2.0 } else { 1.0 };
                deco.last_lit = DVec3::splat(1e9);
            }
            "WaterFountain" | "WaterCooler" => {
                let uses = s
                    .uses
                    .get_or_insert(crate::sound::fprop(&a.props, "numUses", 10.0) as i32);
                if s.using > 0.0 {
                    continue;
                }
                if *uses <= 0 {
                    hud.text = msg(&a.props, "msgEmpty");
                    hud.time_left = 2.0;
                    continue;
                }
                *uses -= 1;
                s.using = 2.0;
                s.bubbling = 2.0;
                let healed = ph.heal(1.0);
                if healed > 0.0 {
                    hud.text = "Healed 1 point".into();
                    hud.time_left = 2.0;
                }
                if class == "WaterFountain" {
                    anim("Use", true);
                } else {
                    anim("Bubble", false);
                }
            }
            "Toilet" | "Toilet2" => {
                if s.using > 0.0 {
                    continue;
                }
                s.using = if class == "Toilet" { 9.0 } else { 2.0 };
                crate::sound::play(
                    if class == "Toilet" {
                        "DeusExSounds.Special.FlushToilet"
                    } else {
                        "DeusExSounds.Special.FlushUrinal"
                    },
                    pos,
                    1.0,
                    256.0,
                    1.0,
                );
                anim("Flush", false);
            }
            "Phone" => {
                if s.using > 0.0 {
                    continue;
                }
                s.using = 3.0;
                let snd = if r < 0.1 {
                    "PhoneBusy"
                } else if r < 0.2 {
                    "PhoneDialtone"
                } else if r < 0.4 {
                    "PhoneVoice1"
                } else if r < 0.6 {
                    "PhoneVoice2"
                } else if r < 0.8 {
                    "PhoneVoice3"
                } else {
                    "PhoneVoice4"
                };
                crate::sound::play_owned(
                    crate::sound::key_of(&a.name),
                    crate::sound::slot::MISC,
                    &format!("DeusExSounds.Special.{snd}"),
                    Some(pos),
                    1.0,
                    256.0,
                    1.0,
                );
            }
            "Pinball" | "WHPiano" => {
                if s.using > 0.0 {
                    continue;
                }
                s.using = 2.0;
                let snd = if class == "Pinball" {
                    "PinballMachine"
                } else if r < 0.5 {
                    "Piano1"
                } else {
                    "Piano2"
                };
                crate::sound::play_owned(
                    crate::sound::key_of(&a.name),
                    crate::sound::slot::MISC,
                    &format!("DeusExSounds.Special.{snd}"),
                    Some(pos),
                    1.0,
                    256.0,
                    1.0,
                );
            }
            "ShipsWheel" => {
                let dir = if r < 0.5 { -1.0 } else { 1.0 };
                let speed = (r2 * 64.0).floor() * 1024.0;
                let dur = (r3 * 5.0 + 2.0) as f32;
                s.spin = Some((dir * speed, dur, 0.0));
            }
            "VendingMachine" | "CigaretteMachine" => {
                let cig = class == "CigaretteMachine";
                let price = if cig { 8 } else { 2 };
                let uses = s
                    .uses
                    .get_or_insert(crate::sound::fprop(&a.props, "numUses", 10.0) as i32);
                if *uses <= 0 {
                    hud.text = msg(&a.props, "msgEmpty");
                } else if inv.0.credits >= price {
                    crate::sound::play("DeusExSounds.Generic.VendingCoin", pos, 1.0, 512.0, 1.0);
                    // SkinColor SC_Drink (0) gives soda, SC_Snack candy.
                    let product = if cig {
                        "Cigarettes"
                    } else if matches!(find(&a.props, "SkinColor"), Some(Value::Byte(b)) if *b != 0)
                    {
                        "Candybar"
                    } else {
                        "Sodacan"
                    };
                    crate::sound::play(
                        if product == "Sodacan" {
                            "DeusExSounds.Generic.VendingCan"
                        } else {
                            "DeusExSounds.Generic.VendingSmokes"
                        },
                        pos,
                        1.0,
                        512.0,
                        1.0,
                    );
                    let yaw = deco.rotation[1] as f64 / 65536.0 * std::f64::consts::TAU;
                    let fwd = DVec3::new(yaw.cos(), yaw.sin(), 0.0);
                    // Out of the slot: Location + Vector(Rotation) * CollisionRadius * 0.8, CollisionHeight * 0.7
                    // down (CigaretteMachine 0.6), thrown out at Vector(Rotation) * 100.
                    let at = pos + fwd * (c.radius * 0.8)
                        - DVec3::new(0.0, 0.0, c.height * if cig { 0.6 } else { 0.7 });
                    drops.0.push((product.into(), at, fwd * 100.0, None, Vec::new()));
                    inv.0.credits -= price;
                    *uses -= 1;
                    hud.text = msg(&a.props, "msgDispensed");
                } else {
                    hud.text = msg(&a.props, "msgNoCredits");
                }
                hud.time_left = 2.0;
            }
            "Poolball" => {
                let mut v = deco.body.location - player.body.location;
                v.z = 0.0;
                deco.body.velocity = v.normalize_or_zero() * 400.0;
                if deco.body.physics != imsim_core::body::BodyPhysics::Falling {
                    deco.body.physics = imsim_core::body::BodyPhysics::Rolling;
                }
            }
            _ => {}
        }
    }

    // Timers: bUsing, Button1 popping back, fountains, the ship's wheel.
    let keys: Vec<usize> = st.state.keys().copied().collect();
    for b in keys {
        let Some(c) = blockers.list.get(b).copied() else {
            continue;
        };
        let a = &lvl.actors[c.actor];
        let s = st.state.get_mut(&b).unwrap();
        if s.using > 0.0 {
            s.using -= dt;
            if s.using <= 0.0 && a.class == "WaterFountain" {
                if let Some(&e) = blockers.deco.get(&b) {
                    if let Ok((_, Some(mut an))) = decos.get_mut(e) {
                        play_anim(lvl, &mut an, "Still", false);
                    }
                }
            }
        }
        if s.bubbling > 0.0 {
            s.bubbling -= dt;
            crate::sound::hold(
                0xF0_0000_0000 | b as u64,
                "Ambient.Ambient.WaterBubbling",
                Some(c.center),
                1.0,
                25.0 * 33.0,
                1.0,
            );
        }
        if s.button > 0.0 {
            s.button -= dt;
            if s.button <= 0.0 {
                if let Some(snd) = crate::sound::prop(&a.props, "buttonSound2") {
                    crate::sound::play(&snd, c.center, 1.0, 512.0, 1.0);
                }
            }
        }
        if let Some((rate, dur, t)) = s.spin.as_mut() {
            if let Some(&e) = blockers.deco.get(&b) {
                if let Ok((mut deco, _)) = decos.get_mut(e) {
                    deco.rotation[2] += (*rate * dt as f64 * ((*dur - *t) / *dur) as f64) as i32;
                    deco.spun = true;
                }
            }
            *t += dt;
            if *t >= *dur {
                s.spin = None;
            }
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = FROBS.lock() {
        v.clear();
    }
}
