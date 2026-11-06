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
//!
//! Combat movement, grenades and alarms (ROADMAP 38, GMDX again):
//! - Firing positions (ScriptedPawn.ComputeBestFiringPosition, which the classic setting doesn't run yet): a pawn in
//!   range keeps re-picking where to stand - holding its weapon's range, side-stepping out of the enemy's aim
//!   (bAvoidAim / AvoidAccuracy, CloseCombatMult within 512), sprinting sideways (bSprint / SprintRate), backing
//!   off from someone with a melee weapon - and strafes there facing the enemy and firing (ShouldStrafe: GMDX's
//!   HumanMilitary / HumanThug only strafe to spots that can see the enemy, 90% of the time with a shot).
//! - Smart strafing (bSmartStrafe / smartStrafeRate): chasing an enemy just lost round a corner, a military pawn
//!   often (0.45, thugs and commandos 0.35, x0.75 below Hard) runs facing where it was, gun up, within 1536.
//! - Defensive style (bDefensiveStyle): 40% of military pawns and thugs (not MIBs, commandos, Anna, Walton) with a
//!   long-range gun hold their ground for the first part of a close fight instead of rushing you.
//! - Grenades (bGrenadier: NSF, UNATCO, MJ12 troops, soldiers, riot cops, HK military): on a shot or a strafe a
//!   2% chance (3.5% with shotguns / flamethrower / crossbow) to check for a throw - you seen this fight, 640-1792
//!   away, not much above or below, not on a ladder, a trace that ends within 192 of you, no friend of theirs within
//!   224 of you, floor around you - then an Attack-swing throw: gas from NSF / UNATCO / riot cops / HK, a mix of gas,
//!   EMP, scramble and LAMs from MJ12 and soldiers, blast radius 352 (512 on Realistic).
//! - Alarms (AlarmUnit / HandleAlarm): heard out to 24 x 129 = 3096 (was 1625), pawns look 3200 for a box to push
//!   (was 2400); military pawns, thugs and robots answer any alarm (not only one set off by their enemy), get a
//!   one-time edge (reflexes, sight, aim), and search where the alarm said the intruder was (40% a post-combat
//!   search there).
//! - Ours: a pawn starting a reload close to its enemy ducks round the nearest corner to do it.

use crate::ai::Npc;
use crate::stealth::{Noise, NoiseKind};
use glam::DVec3;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

pub fn on() -> bool {
    setting!("SmarterAI") > 0.5
}

/// Alert indicators over guards' heads (setting AlertIndicators).
pub fn indicators() -> bool {
    setting!("AlertIndicators") > 0.5
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
    /// Ours: seconds the enemy has been out of sight (a glimpse lost for a moment doesn't restart the react time).
    pub unseen: f32,
    /// Ours: the state last frame (to call out on starting an attack).
    pub was_attacking: bool,
    /// HumanMilitary / HumanThug (class ancestry looked up once).
    military: Option<bool>,
    /// The pawn's own combat properties (read when it is made).
    pub props: CombatProps,
    /// Class rules looked up once (smartStrafeRate, bGrenadier, bDefensiveStyle).
    class_done: bool,
    pub strafe_rate: f32,
    pub grenadier: bool,
    pub defensive: bool,
    /// A move picked by `best_firing_position` (or a dash for cover) under way.
    pub repos: Option<Repos>,
    /// Seconds to the next PickDestination while standing in range.
    pub pick_t: f32,
    /// bSmartStrafe for the current chase, and whether the pawn was chasing last frame.
    pub smart_strafe: bool,
    pub chasing: bool,
    /// EnemyTimer: seconds since this fight began.
    pub engage_t: f32,
    /// CombatSeeTimer: seconds the enemy has been in sight this time.
    pub seen_t: f32,
    /// A grenade check is due (after a shot or the start of a strafe).
    pub nade_roll: bool,
    /// A shot since the last picked move (each move ends at the Fire label, which shoots before the next pick).
    pub shot_since_move: bool,
    pub nade: Option<NadeThrow>,
    /// bAlarmStatIncrease: had the one-time edge from an alarm.
    pub alarm_boost: bool,
    /// Reloading last frame (to catch the start of a reload), and holding in cover until it's done.
    pub was_reloading: bool,
    pub in_cover: bool,
}

impl SmartState {
    pub fn new(p: &[Property]) -> SmartState {
        SmartState { props: CombatProps::from_props(p), ..Default::default() }
    }
}

/// The vanilla combat properties ComputeBestFiringPosition reads.
#[derive(Debug, Clone, Default)]
pub struct CombatProps {
    pub avoid_aim: bool,
    pub avoid_accuracy: f32,
    pub close_combat_mult: f32,
    pub sprint: bool,
    pub sprint_rate: f32,
    pub min_range: f64,
    pub max_range: f64,
    pub has_cloak: bool,
}

impl CombatProps {
    pub fn from_props(p: &[Property]) -> CombatProps {
        let f = |n: &str, d: f32| match find(p, n) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f32,
            _ => d,
        };
        let b = |n: &str| matches!(find(p, n), Some(Value::Bool(true)));
        CombatProps {
            avoid_aim: b("bAvoidAim"),
            avoid_accuracy: f("AvoidAccuracy", 0.4),
            close_combat_mult: f("CloseCombatMult", 0.0),
            sprint: b("bSprint"),
            sprint_rate: f("SprintRate", 0.0),
            min_range: f("MinRange", 0.0) as f64,
            max_range: f("MaxRange", 4000.0) as f64,
            has_cloak: b("bHasCloak"),
        }
    }
}

/// A combat move under way: where to, for how long so far, and whether the pawn faces (and fires at) the enemy on
/// the way (StrafeFacing) or the way it goes (MoveTo).
#[derive(Debug, Clone, Copy)]
pub struct Repos {
    pub dest: DVec3,
    pub t: f32,
    pub max_t: f32,
    pub strafe: bool,
    /// Ours: running for cover to reload.
    pub cover: bool,
}

/// A grenade throw under way (GMDX's nadeThrow sequence): turning to `aim`, then the Attack swing; the grenade
/// leaves the hand 0.3 s into it.
#[derive(Debug, Clone)]
pub struct NadeThrow {
    pub class: &'static str,
    /// Chance of the wide (inaccurate) throw.
    pub wide: f32,
    pub aim: DVec3,
    pub t: f32,
    pub swing: Option<f32>,
    pub thrown: bool,
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
    if !npc.smart.class_done {
        npc.smart.class_done = true;
        class_rules(npc, class);
    }
    if on() {
        let mil = *npc.smart.military.get_or_insert_with(|| military(class));
        if mil {
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
        if npc.smart.alarm_boost {
            // HandleAlarm's bAlarmStatIncrease: quicker to react and sharper-eyed for the rest of the level.
            if surprise > 0.5 {
                surprise -= 0.1;
            }
            if vis > 0.002 {
                vis -= 0.0005;
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

/// GMDX's per-class combat defaults, looked up once: smartStrafeRate (HumanMilitary 0.45, HumanThug and MJ12Commando
/// 0.35; DifficultyMod x0.75 below Hard), bGrenadier, bDefensiveStyle (PostBeginPlay: FRand() < 0.4, not MIBs,
/// commandos, Anna or Walton) and HumanMilitary.PostBeginPlay's bAvoidAim for strafers that don't defend home.
fn class_rules(npc: &mut Npc, class: &str) {
    let isa = |b: &str| crate::classtree::is_a(class, b) || class.eq_ignore_ascii_case(b);
    let mil = isa("HumanMilitary");
    let thug = isa("HumanThug");
    let mut rate = if isa("MJ12Commando") || (thug && !mil) {
        0.35
    } else if mil {
        0.45
    } else {
        0.0
    };
    if crate::travel::combat_difficulty() <= 2.0 {
        rate *= 0.75;
    }
    npc.smart.strafe_rate = rate;
    npc.smart.grenadier = ["Terrorist", "UNATCOTroop", "MJ12Troop", "Soldier", "RiotCop", "HKMilitary"]
        .iter()
        .any(|c| isa(c));
    let no_defence = ["MIB", "MJ12Commando", "AnnaNavarre", "WaltonSimons"].iter().any(|c| isa(c));
    npc.smart.defensive = (mil || thug) && !no_defence && crate::sound::frand() < 0.4;
    if mil && npc.can_strafe && !npc.defend_home {
        npc.smart.props.avoid_aim = true;
    }
}

/// HandleAlarm (GMDX): military pawns, thugs and robots answer an alarm whoever set it off.
pub fn answers_any_alarm(class: &str) -> bool {
    on() && (military(class) || robot(class))
}

/// HandleAlarm's bAlarmStatIncrease (HumanMilitary / HumanThug, once): SurprisePeriod -0.1 and VisibilityThreshold
/// -0.0005 (applied in `tune`), BaseAccuracy -0.01, and a 1 in 10 chance to walk 15% faster.
pub fn alarm_boost(npc: &mut Npc, class: &str) {
    if npc.smart.alarm_boost || !military(class) {
        return;
    }
    npc.smart.alarm_boost = true;
    if npc.base_accuracy > 0.01 {
        npc.base_accuracy -= 0.01;
    }
    if crate::sound::frand() < 0.1 {
        npc.walk_frac *= 1.15;
        npc.walk_anim_mult *= 1.15;
    }
}

/// What ComputeBestFiringPosition needs to know about the fight.
pub struct FirePosIn {
    pub me: DVec3,
    pub radius: f64,
    pub ground_speed: f64,
    pub enemy: DVec3,
    pub enemy_radius: f64,
    /// The enemy's facing (radians).
    pub enemy_yaw: f64,
    pub enemy_is_player: bool,
    /// The enemy holds a weapon (Enemy.Weapon != None) and its ranges (GetPawnWeaponRanges: min, max).
    pub enemy_armed: bool,
    pub enemy_min: f64,
    pub enemy_max: f64,
    /// Our weapon's minimum and most accurate range.
    pub self_min: f64,
    pub self_max: f64,
    pub can_strafe: bool,
    pub reloading: bool,
}

pub enum FirePos {
    /// DEST_SameLocation: stay and shoot.
    Same,
    /// DEST_NewLocation.
    New(DVec3),
    /// DEST_Failure: nowhere better (PickDestination then paths toward the enemy, or stays).
    Fail,
}

fn yaw_dir(yaw: f64) -> DVec3 {
    DVec3::new(yaw.cos(), yaw.sin(), 0.0)
}

/// ScriptedPawn.ComputeBestFiringPosition (GMDX's: CloseCombatMult applies within 512, not 180). `try_loc(pos, min)`
/// is TryLocation (AIDirectionReachable that way, bDefendHome), returning where the pawn can actually get to;
/// `random_dest(yaw, max)` is the AIPickRandomDestination fallback (biased toward `yaw`).
pub fn best_firing_position(
    p: &CombatProps,
    i: &FirePosIn,
    rand: &mut dyn FnMut() -> f32,
    try_loc: &mut dyn FnMut(DVec3, f64) -> Option<DVec3>,
    random_dest: &mut dyn FnMut(f64, f64) -> Option<DVec3>,
) -> FirePos {
    use std::f64::consts::PI;
    let extra = i.enemy_radius * 0.5;
    let fudge = 100.0;
    let min_area = 35.0;
    let mut self_min = i.self_min;
    let mut self_max = i.self_max.min(1200.0);
    let enemy_min = i.enemy_min;
    let enemy_max = i.enemy_max.min(1200.0);
    // Hack, to prevent non-strafing NPCs from trying to back up.
    if !i.can_strafe {
        self_min = 0.0;
    }
    let min_dist = i.enemy_radius + i.radius - (extra + 1.0);
    self_min = self_min.max(min_dist).max(p.min_range);
    self_max = self_max.min(p.max_range);
    let to_me = i.me - i.enemy;
    let dist = to_me.length();
    let mut inner = [self_min, self_max];
    let mut outer = [self_min, self_max];
    if self_max > enemy_min {
        inner[1] = enemy_min;
    }
    if self_min < enemy_max && i.can_strafe {
        outer[0] = enemy_max;
    }
    if outer[1] - outer[0] < min_area {
        outer = [0.0, 0.0];
    }
    if inner[1] - inner[0] < min_area {
        inner = outer;
        outer = [0.0, 0.0];
    }
    // If the enemy can reach us through our entire weapon range, just use the range.
    if inner[0] >= inner[1] && outer[0] >= outer[1] {
        inner = [self_min, self_max];
    }
    for r in inner.iter_mut().chain(outer.iter_mut()) {
        *r += extra;
    }
    let inner_ok = inner[0] < inner[1];
    let outer_ok = outer[0] < outer[1];
    if !inner_ok {
        return FirePos::Fail;
    }
    let rel_yaw = to_me.y.atan2(to_me.x);
    // The angle between where the enemy faces and where we stand from it.
    let mut h = rel_yaw - i.enemy_yaw;
    h = h.sin().atan2(h.cos());
    let mut away = dist;
    let mut across = 0.0f64;
    let max_dist = i.ground_speed * 0.6;
    let margin = ((inner[1] - inner[0]) * 0.5).min(fudge);
    if away < inner[0] {
        away = inner[0] + margin;
    } else if away > inner[1] {
        away = inner[1] - margin;
    }
    if outer_ok {
        let margin = ((outer[1] - outer[0]) * 0.5).min(fudge);
        if away > outer[1] {
            away = outer[1] - margin;
        }
    }
    away = away.clamp(dist - max_dist, dist + max_dist);
    // Used to determine whether NPCs should sprint / avoid aim.
    let mut move_mult = 1.0f32;
    if dist <= 512.0 && i.enemy_is_player && i.enemy_armed && enemy_max < 180.0 {
        move_mult = p.close_combat_mult;
    }
    if p.avoid_aim && rand() <= p.avoid_accuracy * move_mult && away < enemy_max + max_dist + 50.0 && away < 800.0 && i.enemy_armed {
        // Inside the enemy's aim (within two of our radii either side of where it faces): step across it.
        let angle = if dist > 0.0 { (i.radius * 2.0 / dist).atan() } else { PI * 0.5 };
        if h.abs() <= angle {
            let pad = 128.0 / 32768.0 * PI;
            across = if h < 0.0 { (-angle - h) - pad } else { (angle - h) + pad };
            if rand() < 0.05 {
                across = -across;
            }
        }
    }
    let reload_mult = if i.reloading && i.enemy_is_player { 0.5 } else { 1.0 };
    let mut sprint = None;
    if p.sprint && i.can_strafe && rand() <= p.sprint_rate * 0.5 * move_mult * reload_mult && (outer_ok || inner[1] > 100.0) {
        // Sprint on long-range weapons only: off to one side of the enemy.
        let to_enemy = (-to_me).y.atan2(-to_me.x);
        let side = if rand() < 0.5 { PI * 0.5 } else { -PI * 0.5 };
        let yaw = to_enemy + side + (rand() as f64 - 0.5) * PI * 0.5;
        sprint = Some(yaw_dir(yaw) * i.ground_speed * (rand() as f64 + 0.5));
    }
    if across == 0.0 && away == dist && sprint.is_none() {
        return FirePos::Same;
    }
    let min = i.radius + 16.0;
    let mut dest = None;
    if rand() >= 1.0 / 40.0 {
        if across != 0.0 && away != dist {
            dest = try_loc(i.enemy + yaw_dir(rel_yaw + across) * away, min);
        }
        if dest.is_none() && away != dist {
            dest = try_loc(i.enemy + yaw_dir(rel_yaw) * away, min);
        }
        if dest.is_none() && across != 0.0 {
            dest = try_loc(i.enemy + yaw_dir(rel_yaw + across) * dist, min);
        }
        if dest.is_none() {
            if let Some(v) = sprint {
                dest = try_loc(i.me + v, min);
            }
        }
    }
    if dest.is_none() {
        if move_mult >= 0.5 || rand() <= move_mult {
            dest = random_dest(rel_yaw + PI, max_dist);
        } else {
            return FirePos::Same;
        }
    }
    match dest {
        Some(d) => FirePos::New(d),
        None => FirePos::Fail,
    }
}

/// ShouldStrafe (GMDX): HumanMilitary / HumanThug only strafe to a spot that can see the enemy when it was seen
/// again within the last second, strafe 90% of the time they have a shot, and otherwise strafe within 1536 of
/// it only on a smart strafe; everyone else strafes when it has a shot (AICanShoot).
pub fn strafe_ok(npc: &Npc, class: &str, dest_sees_enemy: bool, can_shoot: bool, dist: f64) -> bool {
    if !military(class) {
        return can_shoot;
    }
    if npc.smart.seen_t > 0.0 && npc.smart.seen_t < 1.0 && can_shoot && !dest_sees_enemy {
        return false;
    }
    if can_shoot {
        crate::sound::frand() < 0.9
    } else {
        dist < 1536.0 && npc.smart.smart_strafe
    }
}

/// RunToRange's bSmartStrafe roll, on starting a chase: bCanStrafe, FRand() < smartStrafeRate, and the enemy seen
/// within EnemyTimeout - 1.5 s.
pub fn roll_smart_strafe(npc: &mut Npc, since_seen: f32) {
    npc.smart.smart_strafe =
        on() && npc.can_strafe && crate::sound::frand() < npc.smart.strafe_rate && since_seen < npc.enemy_timeout - 1.5;
}

/// GMDX's grenade kinds by class (nadeThrow): gas from NSF / UNATCO / riot cops / HK military (thrown wider 1 in
/// 10); MJ12 troops and soldiers 30% gas, then 30% of the rest EMP, 30% of what's left scramble, else a LAM.
/// Returns (class, chance of the wide throw).
pub fn grenade_kind(class: &str) -> (&'static str, f32) {
    let isa = |b: &str| crate::classtree::is_a(class, b) || class.eq_ignore_ascii_case(b);
    if isa("MJ12Troop") || isa("Soldier") {
        let r = crate::sound::frand;
        let c = if r() < 0.3 {
            "GasGrenade"
        } else if r() < 0.3 {
            "EMPGrenade"
        } else if r() < 0.3 {
            "NanoVirusGrenade"
        } else {
            "LAM"
        };
        (c, 0.0)
    } else {
        ("GasGrenade", 0.1)
    }
}

/// nadeMult (Attacking.BeginState): +0.015 with a flamethrower, crossbow or shotgun in hand, +0.01 with a cloak.
pub fn nade_chance(npc: &Npc) -> f32 {
    let mut m = 0.0;
    if npc.weapon.as_ref().is_some_and(|w| {
        matches!(w.class.as_str(), "WeaponFlamethrower" | "WeaponMiniCrossbow" | "WeaponSawedOffShotgun" | "WeaponAssaultShotgun")
    }) {
        m = 0.015;
    }
    if npc.smart.props.has_cloak {
        m += 0.01;
    }
    0.02 + m
}

/// nadeThrow's velocity: by range 1000 / 1150 / 1300 forward with 300 / 350 / 420 up, a random 60 (140 for a wide
/// throw), less up the higher the thrower stands (1.1 x the height difference), x0.6 from more than 160 above, and
/// the lift x0.6 under a low ceiling.
pub fn nade_velocity(yaw: f64, dist: f64, dz_above: f64, wide: bool, low_ceiling: bool) -> DVec3 {
    let (speed, up) = if dist < 900.0 {
        (1000.0, 300.0)
    } else if dist < 1300.0 {
        (1150.0, 350.0)
    } else {
        (1300.0, 420.0)
    };
    let spread = if wide { 140.0 } else { 60.0 };
    let vrand = loop {
        let r = || crate::sound::frand() as f64 * 2.0 - 1.0;
        let v = DVec3::new(r(), r(), r());
        if v.length_squared() <= 1.0 && v.length_squared() > 1e-6 {
            break v.normalize();
        }
    };
    let mut v = yaw_dir(yaw) * speed + vrand * spread;
    v.z += up;
    v.z -= dz_above * 1.1;
    if dz_above > 160.0 {
        v *= 0.6;
    }
    if low_ceiling {
        v.z *= 0.6;
    }
    v
}

/// The grenade's blast radius: 352, 512 on Realistic (CombatDifficulty > 2).
pub fn nade_blast() -> f64 {
    if crate::travel::combat_difficulty() > 2.0 {
        512.0
    } else {
        352.0
    }
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
        let moving = (speed / 320.0).clamp(0.0, 1.0);
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
    // EnemyTimer and CombatSeeTimer.
    npc.smart.engage_t += dt;
    if seen {
        npc.smart.seen_t += dt;
    } else if npc.smart.unseen >= 0.5 {
        npc.smart.seen_t = 0.0;
    }
    if !on() {
        npc.smart.sfire = 0.0;
        return true;
    }
    let weapon = npc.weapon.clone();
    let (wclass, skill, crossbow) = weapon
        .as_ref()
        .map(|w| (w.class.as_str(), w.governing_skill.as_deref().unwrap_or_default(), w.class == "WeaponMiniCrossbow"))
        .unwrap_or_default();
    // Sight of the enemy flickers (a doorway, cover, the 0.1 s sight checks): only losing it for half a second
    // counts as losing it. Restarting the react time on every flicker left a pawn staring without ever firing.
    const LOST: f32 = 0.5;
    if seen {
        if target_is_player {
            npc.smart.spotted = true;
        }
        npc.smart.sfire = 0.0;
        npc.smart.unseen = 0.0;
        npc.smart.react_left -= dt;
    } else {
        let was = npc.smart.unseen;
        npc.smart.unseen += dt;
        if npc.smart.was_seen && was < LOST && npc.smart.unseen >= LOST {
            npc.smart.react_left = fire_react_time(class, wclass, skill);
            let human = !robot(class) && !crate::classtree::is_a(class, "Animal");
            let mil_bot = crate::classtree::is_a(class, "MilitaryBot");
            if npc.smart.spotted && !crossbow && (human || mil_bot) && crate::sound::frand() < 0.33 {
                npc.smart.sfire = 0.4 + crate::sound::frand() * 4.0;
            }
        }
        npc.smart.sfire = (npc.smart.sfire - dt).max(0.0);
    }
    if seen || npc.smart.unseen >= LOST {
        npc.smart.was_seen = seen;
    }
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
    npc.smart.engage_t = 0.0;
    npc.smart.seen_t = 0.0;
    npc.smart.repos = None;
    npc.smart.smart_strafe = false;
    npc.smart.nade = None;
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

/// A noise made outside the systems that hold the Noises resource (heard by the NPCs next frame).
pub fn push_noise(n: Noise) {
    if let Ok(mut q) = NOISES.lock() {
        q.push(n);
    }
}

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
    on().then_some(Noise {
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
    use glam::DVec3;

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

    fn input(me: DVec3, enemy_max: f64) -> super::FirePosIn {
        super::FirePosIn {
            me,
            radius: 20.0,
            ground_speed: 200.0,
            enemy: DVec3::ZERO,
            enemy_radius: 20.0,
            enemy_yaw: 0.0,
            enemy_is_player: true,
            enemy_armed: true,
            enemy_min: 0.0,
            enemy_max,
            self_min: 0.0,
            self_max: 1000.0,
            can_strafe: true,
            reloading: false,
        }
    }

    #[test]
    fn steps_out_of_the_enemys_aim() {
        // Standing right where an armed enemy faces: step across his aim, keeping the range.
        let p = super::CombatProps { avoid_aim: true, avoid_accuracy: 1.0, close_combat_mult: 0.3, max_range: 1000.0, ..Default::default() };
        let mut rand = || 0.5f32;
        let mut try_loc = |pos: DVec3, _min: f64| Some(pos);
        let mut random_dest = |_y: f64, _m: f64| None;
        match super::best_firing_position(&p, &input(DVec3::new(400.0, 0.0, 0.0), 1200.0), &mut rand, &mut try_loc, &mut random_dest) {
            super::FirePos::New(d) => {
                assert!((d.length() - 400.0).abs() < 1.0, "{d:?}");
                assert!(d.y > 10.0, "{d:?}");
            }
            _ => panic!("expected a new spot"),
        }
    }

    #[test]
    fn backs_off_from_a_knife() {
        // The enemy holds a melee weapon: back out beyond its reach (at most 0.6 s of running per move).
        let p = super::CombatProps { max_range: 1000.0, ..Default::default() };
        let mut rand = || 0.5f32;
        let mut try_loc = |pos: DVec3, _min: f64| Some(pos);
        let mut random_dest = |_y: f64, _m: f64| None;
        match super::best_firing_position(&p, &input(DVec3::new(60.0, 0.0, 0.0), 96.0), &mut rand, &mut try_loc, &mut random_dest) {
            super::FirePos::New(d) => assert!((d.length() - 180.0).abs() < 1.0, "{d:?}"),
            _ => panic!("expected a new spot"),
        }
        // Well inside our range against a gun and out of his aim: stay and shoot.
        let mut rand = || 0.9f32;
        let mut input = input(DVec3::new(0.0, 400.0, 0.0), 1200.0);
        input.enemy_yaw = 0.0;
        assert!(matches!(
            super::best_firing_position(&p, &input, &mut rand, &mut try_loc, &mut random_dest),
            super::FirePos::Same
        ));
    }
}
