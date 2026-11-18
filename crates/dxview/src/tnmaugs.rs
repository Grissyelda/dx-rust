//! The Nameless Mod's two augmentations of its own, written for DX RUST from how they play in the player's copy of
//! the mod (their numbers and messages are read from its packages at run time):
//!
//! - **AugBot**: switching it on takes over the nearest robot the mod marks dominatable, if it is within the
//!   aug's reach (its LevelValues) and not disabled. The robot turns to the player's side for good and stops acting
//!   for itself; while the aug runs, the movement keys drive it along the view (a hovering robot also up and down)
//!   and it faces where the player looks, the player stands frozen with his hands empty. Checked every second: no
//!   robot, or a robot knocked out, switches the aug off. Switched off, the robot stays where it is, on the player's
//!   side, doing nothing.
//! - **AugAimbot**: every LevelValues seconds (1 / 0.8 / 0.5 / 0.2) it looks for a hostile pawn in sight - first
//!   within the weapon's accurate range, then its maximum range, then anywhere - skipping the dying, the invincible,
//!   knocked-out robots and pawns the weapon can't touch any more (fire on the burning, gas on the eye-rubbing, a
//!   stun on the stunned). With one in range it turns the view onto it (leading a projectile weapon, at the head when
//!   that's in sight, else the body), crouches for small or crouching targets and fires; an empty clip is reloaded,
//!   a dry weapon switches ammo or, from level 4, weapon (a GEP gun or EMP / nanovirus against robots, then one of
//!   the same skill in range, then anything in range, anything at all); with nothing usable the aug goes off. From
//!   level 4 it also reloads after eight seconds without a target and uses a medkit (at 60 health, torso or head 40)
//!   or a bioelectric cell (at 10 energy). The HUD names the target with its range (and "out of range", "out of
//!   ammo", "behind") and draws the dotted line to it.

use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

use crate::actors::{Blockers, Deco};
use crate::ai::{Npc, NpcState};

#[derive(Default)]
struct Bot {
    /// The robot taken over (blocker) and whether it hovers (bIsHoverBot).
    blocker: Option<usize>,
    hover: bool,
    /// The movement the keys ask for this frame (unit vector, world).
    input: DVec3,
    /// The player's view yaw (Unreal radians): the robot turns to it.
    look_yaw: f64,
    was_on: bool,
    timer: f32,
}

#[derive(Default)]
struct Aim {
    was_on: bool,
    wait: f32,
    last_seen: Option<f32>,
    reloaded: bool,
    duck: bool,
    /// The target for the HUD (tnmaugmentationdisplaywindow.aimbotTarget): (its blocker, UnfamiliarName, beyond the
    /// weapon's MaxRange, the weapon's ammo all gone).
    view: Option<(usize, String, bool, bool)>,
    clock: f32,
}

static BOT: Mutex<Option<Bot>> = Mutex::new(None);
static AIM: Mutex<Option<Aim>> = Mutex::new(None);
static RELOAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CYCLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn bot<R>(f: impl FnOnce(&mut Bot) -> R) -> R {
    let mut g = BOT.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Bot::default))
}
fn aim<R>(f: impl FnOnce(&mut Aim) -> R) -> R {
    let mut g = AIM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Aim::default))
}

/// The player is driving a robot (the keys go to it; he stands still).
pub fn dominating() -> bool {
    bot(|b| b.blocker.is_some())
}

/// The keys' movement for the robot this frame.
pub fn bot_input(v: DVec3) {
    bot(|b| b.input = v);
}

/// The robot being driven gets this wish (the ai's move step for that pawn).
pub fn bot_drive(blocker: usize) -> Option<(DVec3, f64)> {
    bot(|b| (b.blocker == Some(blocker)).then_some((b.input, b.look_yaw)))
}

/// Whether the robot being driven hovers (its keys move it up and down too).
pub fn bot_hovers() -> bool {
    bot(|b| b.hover)
}

/// The aimbot wants the player crouched (a small or crouching target).
pub fn aimbot_duck() -> bool {
    aim(|a| a.duck)
}

/// The aimbot asks for a reload / an ammo change this frame.
pub fn take_reload() -> bool {
    RELOAD.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn take_cycle() -> bool {
    CYCLE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The aimbot's target for the HUD.
pub fn aimbot_view() -> Option<(usize, String, bool, bool)> {
    aim(|a| a.view.clone())
}

pub fn reset_level() {
    *BOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *AIM.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Damage types a robot shrugs off (Robot.IgnoreDamageType) and those nobody is worth shooting with (the gases).
fn robot_ignores(dt: &str) -> bool {
    matches!(dt, "TearGas" | "HalonGas" | "PoisonGas" | "Poison" | "PoisonEffect" | "KnockedOut" | "Radiation")
}

/// Whether a weapon of this damage type can still hurt this pawn (FilterDamageType / ModifyDamage / ShieldDamage,
/// as far as they are known here).
fn hurts(dt: &str, class: &str, robot: bool) -> bool {
    if matches!(dt, "TearGas" | "HalonGas") {
        return false;
    }
    if robot {
        return !robot_ignores(dt) || matches!(dt, "EMP" | "NanoVirus");
    }
    let gas_proof = ["Karkian", "Greasel", "Gray"].iter().any(|c| crate::classtree::is_a(class, c));
    !(gas_proof && matches!(dt, "PoisonGas" | "TearGas"))
}

/// The pawn is in a state this damage type already put it in (no point hitting it again with it).
fn already(dt: &str, n: &Npc) -> bool {
    match dt {
        "Flamed" => n.on_fire.is_some(),
        "TearGas" => matches!(n.state, NpcState::RubbingEyes(_)),
        "Stunned" => matches!(n.state, NpcState::Stunned(_)),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tnm_aug_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    blockers: Res<Blockers>,
    player: Res<crate::Player>,
    mut cams: Query<&mut crate::FlyCam, Without<crate::SkyCam>>,
    mut weapons: ResMut<crate::weapons::Weapons>,
    mut scripted: ResMut<crate::weapons::ScriptedInput>,
    inv: Res<crate::pickups::PlayerInventory>,
    clips: Res<crate::weapons::Clips>,
    ph: Res<crate::ai::PlayerHealth>,
    mut hud: ResMut<crate::movers::HudMessage>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut npcs: Query<(&mut Npc, &Deco)>,
) {
    // The reload / ammo requests last one frame (the weapon system runs after this one).
    RELOAD.store(false, std::sync::atomic::Ordering::Relaxed);
    CYCLE.store(false, std::sync::atomic::Ordering::Relaxed);
    if !crate::tnmmode::active() {
        return;
    }
    let lvl = &level.0;
    let dt = time.delta_secs();
    let me = player.body.location;
    let eye = me + DVec3::new(0.0, 0.0, player.eye_height);

    // ---------------------------------------------------------------------------------------- AugBot
    let bot_on = crate::abilities::aug_active("AugBot");
    let (was_on, have) = bot(|b| (b.was_on, b.blocker));
    if bot_on && !was_on {
        // FindNearestBot: the nearest dominatable robot that isn't disabled, if within the aug's reach.
        let reach = crate::abilities::aug("AugBot").unwrap_or(0.0) as f64;
        let mut best: Option<(f64, usize, bool)> = None;
        for (n, _) in &npcs {
            let a = &lvl.actors[n.actor];
            if !crate::classtree::is_a(&a.class, "TNMRobot") || n.emp_hp <= 0.0 || n.state == NpcState::Dead {
                continue;
            }
            if !matches!(find(&a.props, "dominatable"), Some(Value::Bool(true))) {
                continue;
            }
            let Some(c) = blockers.list.get(n.blocker) else { continue };
            let d = c.center.distance(me);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, n.blocker, matches!(find(&a.props, "bishoverbot"), Some(Value::Bool(true)))));
            }
        }
        let pick = best.filter(|b| b.0 < reach);
        if let Some((_, b, _)) = pick {
            for (mut n, _) in &mut npcs {
                if n.blocker == b {
                    // becomedominated: alliance 'player', state dominated (ignores everything).
                    n.dominated = true;
                    n.alliance = "Player".into();
                    n.alliances.set_own("Player");
                    n.target = None;
                    n.state = NpcState::Idle;
                }
            }
            println!("TNM AugBot: took over a robot");
        }
        bot(|s| {
            s.blocker = pick.map(|p| p.1);
            s.hover = pick.is_some_and(|p| p.2);
            s.timer = 0.0;
        });
    }
    if bot_on {
        let mut off = false;
        bot(|s| {
            s.timer += dt;
            if s.timer >= 1.0 {
                s.timer = 0.0;
                off = match s.blocker {
                    None => true,
                    Some(b) => npcs.iter().find(|(n, _)| n.blocker == b).is_none_or(|(n, _)| n.emp_hp <= 0.0 || n.state == NpcState::Dead),
                };
            }
        });
        if off {
            crate::abilities::deactivate_aug("AugBot");
        }
        if have.is_some() {
            crate::weapons::put_away(&mut weapons);
            if let Some(c) = cams.iter().next() {
                let y = -(c.yaw as f64) - std::f64::consts::FRAC_PI_2;
                bot(|s| s.look_yaw = y);
            }
        }
    } else if was_on {
        bot(|s| {
            s.blocker = None;
            s.input = DVec3::ZERO;
        });
    }
    bot(|s| s.was_on = bot_on);

    // ---------------------------------------------------------------------------------------- AugAimbot
    let aim_on = crate::abilities::aug_active("AugAimbot");
    if !aim_on {
        aim(|a| {
            if a.was_on {
                *a = Aim::default();
            }
        });
        return;
    }
    let level_n = crate::abilities::aug_level("AugAimbot").unwrap_or(0);
    let interval = crate::abilities::aug("AugAimbot").unwrap_or(1.0).max(0.05);
    let due = aim(|a| {
        a.was_on = true;
        a.clock += dt;
        a.wait -= dt;
        if a.wait <= 0.0 {
            a.wait = interval;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }
    let held = weapons.held.as_ref().map(|h| h.stats.clone());
    let ammo_of = |class: Option<&String>| class.map_or(0, |a| inv.0.ammo.iter().find(|x| &x.0 == a).map_or(0, |x| x.1));
    let dtype = held.as_ref().map_or("Shot", |w| w.damage_type(weapons.loaded_ammo.get(&w.class).map(|s| s.as_str())));
    // SearchForTarget.
    let visible = |p: DVec3| lvl.collision.trace(eye, p, DVec3::ZERO).fraction >= 1.0;
    struct T {
        blocker: usize,
        pos: DVec3,
        height: f64,
        radius: f64,
        vel: DVec3,
        name: String,
        class: String,
        robot: bool,
    }
    let mut cands: Vec<(f64, T, bool)> = Vec::new();
    for (n, d) in &npcs {
        if n.state == NpcState::Dead || n.hp_body.invincible || n.dominated || n.net.is_some() || !n.hates("Player") {
            continue;
        }
        if n.hp_body.robot && n.emp_hp <= 0.0 {
            continue;
        }
        let Some(c) = blockers.list.get(n.blocker) else { continue };
        if c.radius <= 0.0 || !visible(c.center) {
            continue;
        }
        let a = &lvl.actors[n.actor];
        let name = match find(&a.props, "UnfamiliarName") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            _ => a.class.clone(),
        };
        cands.push((
            c.center.distance(me),
            T { blocker: n.blocker, pos: c.center, height: c.height, radius: c.radius, vel: d.body.velocity, name, class: a.class.clone(), robot: n.hp_body.robot },
            already(dtype, n),
        ));
    }
    let pick = held.as_ref().and_then(|w| {
        cands.iter().position(|(d, _, done)| *d <= w.accurate_range as f64 && !done).or_else(|| cands.iter().position(|(d, _, done)| *d <= w.max_range as f64 && !done))
    });
    let pick = pick.or_else(|| (!cands.is_empty()).then_some(0));
    let Some(k) = pick else {
        // No target: from level 4, reload after eight seconds without one.
        if level_n >= 3 {
            if let Some(w) = &held {
                let loaded = clips.0.get(&w.class).copied().unwrap_or(w.reload_count);
                let pool = ammo_of(w.ammo_class.as_ref());
                let reload = aim(|a| {
                    let due = !a.reloaded && a.last_seen.is_some_and(|t| a.clock - t > 8.0);
                    if due && pool > 0 && loaded < w.reload_count && w.reload_count.saturating_sub(loaded) < pool {
                        a.reloaded = true;
                        true
                    } else {
                        false
                    }
                });
                if reload {
                    RELOAD.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            use_consumables(&ph, &inv, &mut weapons, &mut hud, &mut assets);
        }
        aim(|a| {
            a.view = None;
            a.duck = false;
        });
        return;
    };
    let t = cands.swap_remove(k).1;
    let dist = t.pos.distance(me);
    if let Some(w) = held.clone() {
        aim(|a| {
            a.last_seen = Some(a.clock);
            a.reloaded = false;
        });
        let in_range = dist - t.radius / 2.0 + 5.0 <= w.max_range as f64;
        if in_range {
            // AdjustAim: lead a projectile, aim at the head when it can be seen, else the body.
            let mut spot = t.pos;
            if !w.instant {
                let speed = w.projectile.as_deref().map_or(1000.0, |p| match find(&assets.classes.defaults(p), "speed") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 1000.0,
                });
                spot += t.vel * (dist / speed.max(1.0));
            }
            let head = spot + DVec3::new(0.0, 0.0, t.height * 0.85);
            let aim_at = if visible(head) { head } else { spot };
            let d = aim_at - eye;
            let yaw = d.y.atan2(d.x);
            let pitch = d.z.atan2((d.x * d.x + d.y * d.y).sqrt());
            for mut c in &mut cams {
                c.yaw = (-yaw - std::f64::consts::FRAC_PI_2) as f32;
                c.pitch = (pitch as f32).clamp(-1.55, 1.55);
            }
            if t.height <= 38.0 {
                aim(|a| a.duck = true);
            }
            let loaded = clips.0.get(&w.class).copied().unwrap_or(w.reload_count);
            let pool = ammo_of(w.ammo_class.as_ref());
            let has_round = w.reload_count == 0 || loaded > 0 || w.hand_to_hand;
            if has_round {
                let usable = if t.robot && matches!(dtype, "EMP" | "NanoVirus") { true } else { hurts(dtype, &t.class, t.robot) };
                if usable {
                    let in_water = crate::actors::water_at(lvl, eye).is_some();
                    let airless = crate::zones::no_oxygen(lvl, eye);
                    // EnviroEffective (DeusExWeapon's enum order: All, Air, Water, Vacuum, AirWater, AirVacuum,
                    // WaterVacuum); the aug treats AirVacuum like Air.
                    let env = match find(&assets.classes.defaults(&w.class), "EnviroEffective") {
                        Some(Value::Byte(b)) => *b,
                        _ => 0,
                    };
                    let ok_here = match env {
                        1 | 5 => !in_water && !airless,
                        3 => !in_water,
                        _ => true,
                    };
                    if ok_here && weapons.pending_class.is_none() && weapons.pending.is_none() {
                        scripted.fire = true;
                    }
                } else {
                    find_new_weapon(level_n, t.robot, &t.class, t.radius, dist, &inv, &mut weapons, &mut assets, true);
                }
            } else if pool > 0 {
                RELOAD.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                let other = w.ammo_names.iter().any(|a| Some(a) != w.ammo_class.as_ref() && ammo_of(Some(a)) > 0);
                if other {
                    CYCLE.store(true, std::sync::atomic::Ordering::Relaxed);
                } else {
                    find_new_weapon(level_n, t.robot, &t.class, t.radius, dist, &inv, &mut weapons, &mut assets, true);
                }
            }
        }
        let loaded = clips.0.get(&w.class).copied().unwrap_or(0);
        let empty = w.ammo_class.is_some() && ammo_of(w.ammo_class.as_ref()) == 0 && loaded == 0;
        aim(|a| a.view = Some((t.blocker, t.name.clone(), dist > w.max_range as f64, empty)));
    } else {
        aim(|a| a.view = Some((t.blocker, t.name.clone(), false, false)));
    }
    if level_n >= 3 {
        use_consumables(&ph, &inv, &mut weapons, &mut hud, &mut assets);
    }
}

/// CheckHealth / CheckEnergy: a medkit at 60 health (or torso / head at 40), a bioelectric cell at 10 energy, with
/// the "N left" line.
fn use_consumables(
    ph: &crate::ai::PlayerHealth,
    inv: &crate::pickups::PlayerInventory,
    weapons: &mut crate::weapons::Weapons,
    hud: &mut crate::movers::HudMessage,
    assets: &mut crate::weapons::RuntimeAssets,
) {
    let health = ph.hp;
    let low = health <= 60.0 || ph.parts[crate::ai::P_TORSO] <= 40.0 || ph.parts[crate::ai::P_HEAD] <= 40.0;
    for (class, want) in [("MedKit", low), ("BioelectricCell", ph.energy <= 10.0)] {
        if !want || weapons.use_request.is_some() {
            continue;
        }
        let Some(it) = inv.0.items.iter().find(|i| i.def.class.eq_ignore_ascii_case(class)) else { continue };
        let left = match find(&assets.classes.defaults("AugAimbot"), "CountRemainingString") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        hud.text = format!("{} {} {}", it.def.name, left, it.count.max(1) - 1);
        hud.time_left = 3.0;
        weapons.use_request = Some(it.def.class.clone());
    }
}

/// FindNewWeapon (level 4 and up): against a robot a GEP gun or an EMP / nanovirus weapon; then a weapon of the
/// same skill within range; anything within range; the same skill anywhere; anything (one-use weapons last). None:
/// the aug goes off.
#[allow(clippy::too_many_arguments)]
fn find_new_weapon(
    level_n: usize,
    robot: bool,
    class: &str,
    radius: f64,
    dist: f64,
    inv: &crate::pickups::PlayerInventory,
    weapons: &mut crate::weapons::Weapons,
    assets: &mut crate::weapons::RuntimeAssets,
    _skip_one_use: bool,
) {
    if level_n < 3 {
        return;
    }
    let held = weapons.held.as_ref().map(|h| h.stats.clone());
    let mut list: Vec<crate::weapons::WeaponStats> = Vec::new();
    for it in &inv.0.items {
        if it.def.kind != imsim_core::inventory::ItemKind::Weapon {
            continue;
        }
        let d = assets.classes.defaults(&it.def.class);
        if let Some(w) = crate::weapons::weapon_stats(&it.def.class, &d) {
            list.push(w);
        }
    }
    let pool = |w: &crate::weapons::WeaponStats| w.ammo_class.as_ref().map_or(0, |a| inv.0.ammo.iter().find(|x| &x.0 == a).map_or(0, |x| x.1));
    let acceptable = |w: &crate::weapons::WeaponStats, skip_one_use: bool| {
        if ["WeaponTripBomb", "WeaponMine", "WeaponBlackBox"].iter().any(|c| crate::classtree::is_a(&w.class, c)) {
            return false;
        }
        if !(pool(w) > 0 || w.reload_count == 0) {
            return false;
        }
        if skip_one_use && !w.hand_to_hand && w.reload_count == 0 {
            return false;
        }
        let dt = w.damage_type(None);
        if robot {
            matches!(dt, "EMP" | "NanoVirus") || hurts(dt, class, true)
        } else {
            hurts(dt, class, false)
        }
    };
    let in_range = |w: &crate::weapons::WeaponStats| dist <= w.max_range as f64 + radius;
    let same_skill = |w: &crate::weapons::WeaponStats| held.as_ref().is_some_and(|h| h.governing_skill == w.governing_skill);
    let passes: [&dyn Fn(&crate::weapons::WeaponStats) -> bool; 6] = [
        &|w| {
            held.is_some()
                && robot
                && (["WeaponGEPGun", "WeaponTNMGEPGun"].iter().any(|c| crate::classtree::is_a(&w.class, c)) || matches!(w.damage_type(None), "EMP" | "NanoVirus"))
                && acceptable(w, true)
        },
        &|w| held.is_some() && same_skill(w) && in_range(w) && acceptable(w, true),
        &|w| in_range(w) && acceptable(w, true),
        &|w| held.is_some() && same_skill(w) && acceptable(w, true),
        &|w| acceptable(w, true),
        &|w| acceptable(w, false),
    ];
    for pass in passes {
        if let Some(w) = list.iter().find(|w| pass(w)) {
            if held.as_ref().is_some_and(|h| h.class == w.class) {
                return;
            }
            weapons.pending_class = Some(w.class.clone());
            return;
        }
    }
    crate::abilities::deactivate_aug("AugAimbot");
}

