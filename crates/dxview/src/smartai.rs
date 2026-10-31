//! Enhancement #38, Smarter AI (setting SmarterAI, off by default).
//!
//! Two halves. The first is GMDX's AI work (Sarge, CyberP and RSD's changes to ScriptedPawn, HumanMilitary,
//! HumanThug, DeusExPlayer and SecurityCamera in GMDX), ported here by hand as numbers and rules:
//! - Perception by difficulty (DifficultyMod for HumanMilitary / HumanThug): VisibilityThreshold, HearingThreshold,
//!   SurprisePeriod and EnemyTimeout tightened per combat difficulty (Easy 0.010 / 0.150 / 2 s / 8 s, Medium 0.007 /
//!   0.145 / 1.5 s / 9 s, Hard and Realistic 0.006 / 0.145 / 1.0-1.4 s / 10 s), and the player 1.3x as visible
//!   (CalculatePlayerVisibility's 1.3 base).
//! - Quick reflexes (bQuickReflexes): a pawn that sights you, or hears you within 500 units, may skip searching and
//!   open fire at once (fHighAlertChance: 0.2 most, 0.4 MJ12 troops, 0.6 commandos, 0 NSF terrorists; never robots).
//! - Fire react time (fireReactTime / GetFireReactTime): after it loses sight of you a pawn has to see you again for
//!   a moment before it can shoot (0.1 commandos .. 0.6 terrorists, +0.2 on Easy, pistols quicker, rifles and heavy
//!   weapons slower, at most 0.75 s) - no more instant headshots the frame you peek out.
//! - Suppressive fire (sFire): a pawn that loses sight of you has a 1 in 3 chance to keep shooting where it last
//!   saw you for 0.4-4.4 s (humans and military bots, not with the mini-crossbow).
//! - Moving targets are harder to hit (PawnAccuracyModifier): +0.075 inaccuracy against a target faster than 100,
//!   +0.15 over 400, +0.4 while it falls; pistols and the mini-crossbow get 0.2 back.
//! - Cameras (SecurityCamera, bHardcoreAI1 / bCameraSensors): they see twice as far (2048), raise the alarm sooner
//!   (2.75 s), and while one beeps at you every guard within 1024 of you hears where you are.
//!
//! The second half is ours, because GMDX makes guards sharper rather than making darkness matter more:
//! - Proper darkness: in deep shadow a guard can only make you out up close (closer still if you keep still or
//!   crouch); a light on you - your flashlight included - gives you away from as far as they can see.
//! - Alert levels: a guard that searched and found nothing stays on edge for a minute (it spots and hears you
//!   faster, keeps its gun out); one that sees you calls out to the others nearby.
//! - Disturbances: guards that see a light go out, or a door opened by someone, go and look.

use crate::ai::Npc;
use crate::stealth::{Noise, NoiseKind};
use glam::DVec3;
use std::sync::Mutex;

pub fn on() -> bool {
    crate::settings::get("SmarterAI") > 0.5
}

/// Alert indicators over guards' heads (setting AlertIndicators).
pub fn indicators() -> bool {
    crate::settings::get("AlertIndicators") > 0.5
}

/// Per-pawn state for the enhancement.
#[derive(Debug, Clone, Default)]
pub struct SmartState {
    /// The pawn's own VisibilityThreshold, HearingThreshold, SurprisePeriod and EnemyTimeout.
    base: Option<[f32; 4]>,
    /// Suppressive fire: seconds left shooting at where the enemy was last seen.
    pub sfire: f32,
    /// bSpottedPlayer: saw the player during this fight.
    pub spotted: bool,
    /// Fire react time still to wait while the enemy is in sight.
    pub react_left: f32,
    /// fHighAlertState: seconds of high alert (fires without the react time).
    pub high_alert: f32,
    /// Ours: seconds left on edge after a search came up empty.
    pub alert: f32,
    /// Whether the enemy was in sight last frame (to catch losing sight).
    pub was_seen: bool,
    /// Ours: the state last frame (to call out on starting an attack).
    pub was_attacking: bool,
}

struct ClassTune {
    high_alert: f32,
    fire_react: f32,
}

/// GMDX's class defaults (fHighAlertChance, fireReactTime).
fn class_tune(class: &str) -> ClassTune {
    let isa = |b: &str| crate::classtree::is_a(class, b) || class.eq_ignore_ascii_case(b);
    if isa("MJ12Commando") {
        ClassTune { high_alert: 0.6, fire_react: 0.1 }
    } else if isa("MJ12Troop") {
        ClassTune { high_alert: 0.4, fire_react: 0.35 }
    } else if isa("Terrorist") {
        ClassTune { high_alert: 0.0, fire_react: 0.6 }
    } else {
        ClassTune { high_alert: 0.2, fire_react: 0.4 }
    }
}

fn military(class: &str) -> bool {
    ["HumanMilitary", "HumanThug"].iter().any(|b| crate::classtree::is_a(class, b) || class.eq_ignore_ascii_case(b))
}

fn robot(class: &str) -> bool {
    crate::classtree::is_a(class, "Robot")
}

/// DifficultyMod (HumanMilitary / HumanThug) while the enhancement is on, the pawn's own values when off; and our
/// heightened alert on top. Called every frame (cheap).
pub fn tune(npc: &mut Npc, class: &str, dt: f32) {
    let base = *npc.smart.base.get_or_insert([npc.vis_threshold, npc.hear_threshold, npc.surprise, npc.enemy_timeout]);
    let [mut vis, mut hear, mut surprise, mut timeout] = base;
    npc.smart.high_alert = (npc.smart.high_alert - dt).max(0.0);
    npc.smart.alert = (npc.smart.alert - dt).max(0.0);
    if on() {
        if military(class) {
            let d = crate::travel::combat_difficulty();
            if d <= 1.0 {
                vis = vis.max(0.010);
                hear = hear.max(0.150);
                timeout = 8.0;
                surprise = surprise.max(2.0);
            } else if d <= 2.0 {
                hear = hear.max(0.145);
                surprise = surprise.max(1.5);
                timeout = 9.0;
                vis = 0.007;
            } else {
                hear = hear.max(0.145);
                if surprise < 1.0 {
                    surprise = 1.2;
                }
                timeout = 10.0;
                vis = 0.006;
            }
        }
        if npc.smart.alert > 0.0 {
            // On edge: hears a quarter quieter sounds, gets over the surprise quicker.
            hear *= 0.75;
            surprise *= 0.6;
            npc.weapon_drawn = npc.weapon_drawn || npc.weapon.is_some();
        }
    }
    npc.vis_threshold = vis;
    npc.hear_threshold = hear;
    npc.surprise = surprise;
    npc.enemy_timeout = timeout;
}

/// How fast readiness to attack builds (x1.5 on edge).
pub fn rise_mult(npc: &Npc) -> f32 {
    if on() && npc.smart.alert > 0.0 {
        1.5
    } else {
        1.0
    }
}

/// The player's visibility to a pawn `dist` away: GMDX's 1.3 base, then our darkness rule - in deep shadow a pawn
/// only makes you out within a short range (shorter still crouched or keeping still).
pub fn player_vis(vis: f32, light: f32, dist: f64, crouched: bool, speed: f64) -> f32 {
    if !on() {
        return vis;
    }
    let mut v = (vis * 1.3).min(1.0);
    // Light on you below about 0.12 is shadow; the deeper the shadow, the closer they have to be.
    let shadow = (1.0 - light / 0.12).clamp(0.0, 1.0);
    if shadow > 0.0 {
        let moving = (speed / 320.0).clamp(0.0, 1.0) as f64;
        let mut reach = 160.0 + 440.0 * (1.0 - shadow as f64) + 200.0 * moving;
        if crouched {
            reach *= 0.6;
        }
        if dist > reach {
            // Beyond it they see a shape at most: fading out over a further half of the reach.
            let k = (1.0 - (dist - reach) / (reach * 0.5)).clamp(0.0, 1.0) as f32;
            v *= k;
        }
    }
    v
}

/// Quick reflexes on sighting or hearing an enemy player: true = skip the search and attack now (high alert).
pub fn quick_reflex(npc: &mut Npc, class: &str, dist: f64, heard: bool) -> bool {
    if !on() || robot(class) {
        return false;
    }
    let chance = class_tune(class).high_alert;
    let r = crate::sound::frand();
    if (heard && dist < 500.0) || r < chance {
        npc.smart.high_alert = 4.0;
        npc.reaction = 1.0;
        true
    } else {
        false
    }
}

/// GetFireReactTime: how long the enemy has to be in sight before this pawn fires again.
pub fn fire_react_time(class: &str, weapon_class: &str, weapon_skill: &str) -> f32 {
    let mut add = 0.0;
    if crate::travel::combat_difficulty() < 2.0 {
        add += 0.2;
    }
    if weapon_skill.contains("Pistol") {
        add -= 0.1;
    } else if weapon_skill.contains("Rifle") {
        add += 0.1;
    } else if weapon_skill.contains("Heavy") {
        add += 0.3;
    }
    if weapon_class.eq_ignore_ascii_case("WeaponRifle") {
        add += 0.2;
    }
    add += crate::sound::frand() * 0.1;
    (class_tune(class).fire_react + add).min(0.75)
}

/// Per-frame combat bookkeeping (UpdateActorVisibility's sFire / CombatSeeTimer part). `seen` = the enemy is in
/// sight now. Returns whether the pawn may fire this frame as far as the react time goes.
pub fn combat_tick(npc: &mut Npc, class: &str, seen: bool, target_is_player: bool, dt: f32) -> bool {
    if !on() {
        npc.smart.sfire = 0.0;
        return true;
    }
    let (wclass, skill, crossbow) = npc
        .weapon
        .as_ref()
        .map(|w| (w.class.clone(), w.governing_skill.clone().unwrap_or_default(), w.class == "WeaponMiniCrossbow"))
        .unwrap_or_default();
    if seen {
        if target_is_player {
            npc.smart.spotted = true;
        }
        npc.smart.sfire = 0.0;
        npc.smart.react_left -= dt;
    } else {
        if npc.smart.was_seen {
            npc.smart.react_left = fire_react_time(class, &wclass, &skill);
            let human = !robot(class) && !crate::classtree::is_a(class, "Animal");
            let mil_bot = crate::classtree::is_a(class, "MilitaryBot");
            if npc.smart.spotted && !crossbow && (human || mil_bot) && crate::sound::frand() < 0.33 {
                npc.smart.sfire = 0.4 + crate::sound::frand() * 4.0;
            }
        }
        npc.smart.sfire = (npc.smart.sfire - dt).max(0.0);
    }
    npc.smart.was_seen = seen;
    npc.smart.react_left <= 0.0 || npc.smart.high_alert > 0.0
}

/// A fresh attack: the react time starts (unless on high alert).
pub fn attack_started(npc: &mut Npc, class: &str) {
    if !on() {
        return;
    }
    let (wclass, skill) = npc.weapon.as_ref().map(|w| (w.class.clone(), w.governing_skill.clone().unwrap_or_default())).unwrap_or_default();
    npc.smart.react_left = fire_react_time(class, &wclass, &skill);
    npc.smart.spotted = false;
    npc.smart.sfire = 0.0;
}

/// PawnAccuracyModifier: extra inaccuracy against a moving or falling target.
pub fn accuracy_mod(target_vel: DVec3, falling: bool, weapon_class: &str) -> f64 {
    if !on() {
        return 0.0;
    }
    let v = target_vel.length();
    let mut m = if v > 400.0 {
        if falling {
            0.4
        } else {
            0.15
        }
    } else if v > 100.0 {
        0.075
    } else {
        0.0
    };
    if weapon_class == "WeaponMiniCrossbow" || weapon_class == "WeaponPistol" {
        m -= 0.2;
    }
    m
}

/// A search ended without finding anyone: on edge for a minute.
pub fn search_failed(npc: &mut Npc) {
    npc.smart.alert = 60.0;
}

/// Cameras: range and trigger delay.
pub fn camera_range(range: f64) -> f64 {
    if on() {
        range.max(2048.0)
    } else {
        range
    }
}

pub fn camera_delay(delay: f32) -> f32 {
    if on() {
        delay.min(2.75)
    } else {
        delay
    }
}

static NOISES: Mutex<Vec<Noise>> = Mutex::new(Vec::new());

/// A camera beeping at the player: guards within 1024 of the player hear where they are (bHardcoreAI1).
pub fn camera_spotted(player_pos: DVec3) {
    if !on() {
        return;
    }
    if let Ok(mut q) = NOISES.lock() {
        q.push(Noise {
            pos: player_pos,
            volume: 1.0,
            radius: 1024.0,
            maker: None,
            instigator: Some(crate::ai::Target::Player),
            kind: NoiseKind::Loud,
        });
    }
}

/// Ours: a guard that sees the player calls out (an Alert others nearby come to look at).
pub fn call_out(at: DVec3, who: usize) -> Option<Noise> {
    on().then(|| Noise {
        pos: at,
        volume: 1.0,
        radius: 1200.0,
        maker: Some(crate::ai::Target::Pawn(who)),
        instigator: Some(crate::ai::Target::Player),
        kind: NoiseKind::Alert,
    })
}

pub fn take_noises() -> Vec<Noise> {
    NOISES.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// What a guard can notice happening in the world (ours).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Disturb {
    /// A light went out.
    LightOut,
    /// A door opened (or closed) by the player.
    Door,
}

static DISTURB: Mutex<Vec<(DVec3, Disturb)>> = Mutex::new(Vec::new());

pub fn disturb(at: DVec3, kind: Disturb) {
    if !on() {
        return;
    }
    if let Ok(mut q) = DISTURB.lock() {
        if q.len() < 32 {
            q.push((at, kind));
        }
    }
}

pub fn take_disturbances() -> Vec<(DVec3, Disturb)> {
    DISTURB.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// How far a guard notices a disturbance it can see.
pub fn disturb_range(kind: Disturb) -> f64 {
    match kind {
        Disturb::LightOut => 1800.0,
        Disturb::Door => 1200.0,
    }
}

pub fn reset_level() {
    if let Ok(mut q) = NOISES.lock() {
        q.clear();
    }
    if let Ok(mut q) = DISTURB.lock() {
        q.clear();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn darkness_limits_sight_range() {
        crate::settings::set("SmarterAI", 1.0);
        // Pitch dark, standing still: seen up close, not from across the room.
        assert!(super::player_vis(0.5, 0.0, 120.0, false, 0.0) > 0.0);
        assert_eq!(super::player_vis(0.5, 0.0, 800.0, false, 0.0), 0.0);
        // Lit: the whole sight radius.
        assert!(super::player_vis(0.5, 0.5, 2000.0, false, 0.0) > 0.0);
        crate::settings::set("SmarterAI", 0.0);
        assert_eq!(super::player_vis(0.5, 0.0, 800.0, false, 0.0), 0.5);
    }
}
