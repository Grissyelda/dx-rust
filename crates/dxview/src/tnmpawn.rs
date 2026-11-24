//! The Nameless Mod's pawn damage rules (TNMPawn / TNMRobot TakeDamage, TakeDamageBase, ModifyDamage and
//! HandleDamage where they differ from ScriptedPawn / Robot). Only the rules live here: every number that is a
//! property of a class is read from the player's own TNM packages.
// uc: TNMPawn.TakeDamage, TNMPawn.TakeDamageBase, TNMPawn.ModifyDamage, TNMPawn.HandleDamage, TNMRobot.TakeDamageBase, TNMRobot.IgnoreDamageType

use bevy::prelude::{Query, Res, Time, Visibility};
use crate::damage::PawnHit;
use unreal_pkg::props::{find, Property, Value};

/// What a TNM pawn carries beyond a Deus Ex one.
#[derive(Debug, Clone, Default)]
pub struct TnmBody {
    /// bHelmeted: blows to the head knock out at a quarter; a rice bag bounces off.
    pub helmeted: bool,
    /// bFirewall: a program - EMP hurts it.
    pub firewall: bool,
    /// bImmuneToGib: the Gibinator does nothing special.
    pub immune_gib: bool,
    /// Fatness (the mesh's Fatness byte): the fat rifle pumps it up; past 255 the pawn bursts.
    pub fat: f32,
    pub fat_default: f32,
    /// temporalness: the temporal weapon's build-up; at twice the health the pawn is gone.
    pub temporal: f32,
    /// Game time of the last change of `fat` / `temporal` (both wear off).
    pub stamp: f64,
    /// bAugmented pawns: the augmentation classes they carry (Augs[]) and their bioenergy (EMP drains it; with
    /// none left the augmentations are off).
    pub augs: Vec<String>,
    pub energy: f32,
    /// AugHealthLimit: at or under this health the health-activated augmentations come on.
    pub aug_health_limit: f32,
    /// Which of `augs` are running (same order), and whether the manager has made them yet.
    pub aug_on: Vec<bool>,
    pub aug_made: bool,
    /// The one-second clock of the healing / health-cloak loops, and whether we hid the pawn (cloaked).
    pub aug_clock: f32,
    pub cloaked: bool,
    pub was_attacking: bool,
    pub was_burning: bool,
    /// Legendary mode: how many attributes this pawn's weapons rolled (TNMPawn.LegendaryCount).
    pub legendary_count: u32,
    /// Nightmare mode: seconds until this body gets up again as a new pawn (RespawnInfo).
    pub respawn: Option<f32>,
}

fn flag(p: &[Property], n: &str) -> bool {
    matches!(find(p, n), Some(Value::Bool(true)))
}

impl TnmBody {
    pub fn new(p: &[Property]) -> TnmBody {
        let fat = match find(p, "Fatness") {
            Some(Value::Byte(b)) => *b as f32,
            Some(Value::Int(i)) => *i as f32,
            _ => 128.0,
        };
        TnmBody {
            helmeted: flag(p, "bHelmeted"),
            firewall: flag(p, "bFirewall"),
            immune_gib: flag(p, "bImmuneToGib"),
            fat,
            fat_default: fat,
            temporal: 0.0,
            stamp: 0.0,
            augs: if flag(p, "bAugmented") {
                p.iter()
                    .filter(|x| x.name.eq_ignore_ascii_case("Augs"))
                    .filter_map(|x| match &x.value {
                        Value::Path(c) => Some(c.rsplit('.').next().unwrap_or(c).to_ascii_lowercase()),
                        _ => None,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            energy: match find(p, "Energy") {
                Some(Value::Float(v)) => *v,
                Some(Value::Int(v)) => *v as f32,
                _ => 0.0,
            },
            aug_health_limit: match find(p, "AugHealthLimit") {
                Some(Value::Float(v)) => *v,
                Some(Value::Int(v)) => *v as f32,
                _ => 0.0,
            },
            aug_on: Vec::new(),
            aug_made: false,
            aug_clock: 0.0,
            cloaked: false,
            was_attacking: false,
            was_burning: false,
            respawn: None,
            legendary_count: 0,
        }
    }

    /// tnmpawnaugmanager.getcombatvalue: the combat augmentation's Value while it runs (hand-to-hand speed).
    // uc: tnmpawnaugmanager.getcombatvalue
    pub fn combat_value(&self) -> f32 {
        match self.augs.iter().position(|a| a == "tnmnpcaugcombat") {
            Some(i) if self.aug_on.get(i).copied().unwrap_or(false) => aug_value("tnmnpcaugcombat").unwrap_or(1.0),
            _ => 1.0,
        }
    }

    fn running(&self, aug: &str) -> bool {
        self.augs.iter().zip(&self.aug_on).any(|(a, on)| *on && a == aug)
    }

    /// tnmAugSpeed running: five times the ground speed.
    pub fn speed_mult(&self) -> f64 {
        if self.running("tnmaugspeed") {
            5.0
        } else {
            1.0
        }
    }

    /// TNMPawn.augdamage: what an augmented pawn's active shield leaves of a hit of this type.
    // uc: TNMPawn.augdamage, TNMPawn.empdamage
    fn aug_scale(&self, dt: &str) -> f32 {
        if self.augs.is_empty() || self.energy <= 0.0 {
            return 1.0;
        }
        let aug = match dt {
            "Shot" | "AutoShot" | "Sabot" => "tnmaugballistic",
            "EMP" => "tnmaugemp",
            "Burned" | "Flamed" | "Exploded" | "Shocked" | "Fat" => "tnmaugshield",
            "TearGas" | "PoisonGas" | "Radiation" | "HalonGas" | "PoisonEffect" | "Poison" => "tnmaugenviro",
            _ => return 1.0,
        };
        if !self.augs.iter().any(|a| a == aug) {
            return 1.0;
        }
        AUG_VALUES.lock().ok().and_then(|v| v.iter().find(|x| x.0 == aug).map(|x| x.1)).unwrap_or(1.0)
    }

    /// Fatness goes down one a second (TNMPawn.Tick), temporalness one per quarter second (TemporalInfo / the
    /// StunnedTemporal state).
    fn decay(&mut self, now: f64) {
        let dt = (now - self.stamp).max(0.0) as f32;
        self.stamp = now;
        if self.fat > self.fat_default {
            self.fat = (self.fat - dt).max(self.fat_default);
        }
        self.temporal = (self.temporal - dt * 4.0).max(0.0);
    }
}

/// What the pre-pass decided.
pub enum Pre {
    /// Go on with the (possibly rewritten) hit.
    Go,
    /// Nothing more to do with this hit.
    Skip,
}

/// How long a blow of this (original) type stuns, when the mod names its own time.
pub fn stun_secs(original: &str) -> Option<f32> {
    match original {
        "StunnedTaser" => Some(5.0),
        "FistStunned" => Some(4.0),
        "Temporal" => Some(2.0),
        _ => None,
    }
}

/// TNMPawn.TakeDamage's own damage types, turned into ones the rest of the game knows. `robot`: a TNMRobot.
/// Returns Skip when the hit is used up, and sets `banned` when the pawn should fade out the mod's way.
pub fn pre_hit(h: &mut PawnHit, t: &mut TnmBody, robot: bool, health: f32, now: f64, banned: &mut bool) -> Pre {
    t.decay(now);
    // Augmented pawns: the shield for this kind of damage, then EMP comes off the bioenergy.
    if !t.augs.is_empty() {
        let k = t.aug_scale(h.damage_type);
        if k != 1.0 && h.damage > 0.0 {
            h.damage = (h.damage * k).floor();
            if h.damage < 1.0 && k > 0.0 {
                h.damage = 1.0;
            }
        }
        if h.damage_type == "EMP" {
            t.energy = (t.energy - h.damage).max(0.0);
            return Pre::Skip;
        }
    }
    match h.damage_type {
        "Uberexploded" => h.damage_type = "Exploded",
        "StunnedTaser" => h.damage_type = "Stunned",
        "Fat" => {
            if robot {
                return Pre::Skip;
            }
            // TotalFat = Fatness + (Damage + ModifyDamage(Damage, 'Shot')) / 2.
            let total = t.fat + h.damage;
            if total > 255.0 {
                h.damage = 10_000.0;
                h.damage_type = "Exploded";
            } else {
                t.fat = total;
                h.damage = 1.0;
                h.damage_type = "Shot";
            }
        }
        "gibbed" => {
            if t.immune_gib {
                h.damage_type = "Shot";
            } else {
                h.damage = 10_000.0;
                h.damage_type = "Exploded";
            }
        }
        "nogib" => {
            h.damage = 100_000.0;
            h.damage_type = "Shot";
        }
        "Temporal" => {
            if h.damage > 0.0 {
                t.temporal += h.damage;
            }
            if t.temporal >= health.max(1.0) * 2.0 {
                *banned = true;
                h.damage = 100_000.0;
                h.damage_type = if robot { "Exploded" } else { "Shot" };
            } else if robot {
                return Pre::Skip;
            } else {
                // StunnedTemporal: held still while the charge lasts, no damage.
                h.damage = 0.0;
                h.damage_type = "Stunned";
            }
        }
        "EMP" if (t.firewall || ALL_FIREWALL.load(std::sync::atomic::Ordering::Relaxed)) && !robot => h.damage_type = "Shot",
        "headstun" => {}
        _ => {}
    }
    Pre::Go
}

/// How much the pawn has faded toward being gone (0 = solid, 1 = gone): StunnedTemporal's ScaleGlow.
pub fn temporal_fade(t: &TnmBody, health: f32) -> f32 {
    (t.temporal / (health.max(1.0) * 2.0)).clamp(0.0, 1.0)
}

/// TNMPawn.ModifyDamage for a person: the multiplier on the damage.
pub fn modify(dt: &str, stunned: bool, behind: bool, dist: f64) -> f32 {
    let special = matches!(
        dt,
        "Exploded" | "PoisonGas" | "HalonGas" | "Radiation" | "Poison" | "fell" | "Stomped" | "PoisonEffect" | "Fat"
    );
    if !special {
        if dist < 96.0 && (stunned || behind) {
            if dt == "KnockedOut" { 7.0 * 1.4 } else { 7.0 }
        } else if stunned {
            2.0
        } else {
            1.0
        }
    } else if behind && dist < 64.0 {
        10.0
    } else {
        1.0
    }
}

/// TNMPawn.HandleDamage: the multiplier for a blow to the head. None = the blow knocks the pawn out outright
/// (a rice bag on a bare head).
pub fn head_mult(dt: &str, helmeted: bool) -> Option<f32> {
    match dt {
        "headstun" if helmeted => Some(0.0),
        "headstun" => None,
        "KnockedOut" if helmeted => Some(3.0 * 0.25),
        "Stunned" | "KnockedOut" | "Stomped" => Some(3.0),
        _ => Some(8.0),
    }
}

/// The Deus Ex creature a pawn class behaves as: The Nameless Mod's own copies (TNMKarkian, TNMGray, TNMGreasel,
/// TNMRat and their children; its aliens filter damage as the Gray does) answer to the original's name, so the
/// creature rules written for Deus Ex apply. Anything else is itself.
// uc: TNMKarkian.FilterDamageType, TNMGray.FilterDamageType, TNMGreasel.FilterDamageType, Alien.FilterDamageType, TNMGray.Tick
pub fn kind(class: &str) -> &str {
    if !crate::tnmmode::active() {
        return class;
    }
    let is = |c: &str| crate::classtree::is_a(class, c);
    if is("TNMKarkian") {
        "Karkian"
    } else if is("TNMGray") {
        "Gray"
    } else if is("GenericAlien") {
        "Alien"
    } else if is("TNMGreasel") || is("RedGreasel") || is("InvaderGreasel") {
        "Greasel"
    } else if is("TNMRat") || is("NBDRat") {
        "Rat"
    } else {
        class
    }
}

/// tnmconplay.g_FlagSet: what a flag reads as in The Nameless Mod's conversations (and its news). A name
/// "Killed_<map>_and_<map>..._<percent>" is true while the player has taken down less than that share of the
/// people those maps started with (or none at all); "Killed_M23_<percent>" names the three ABI maps; two flags ask
/// whose body the player carries. Anything else is the flag itself.
// uc: tnmconplay.g_FlagSet, tnmconplay.IsList, tnmconplay.GetMap
pub fn flag_set(flags: &imsim_core::flags::FlagBase, name: &str) -> bool {
    use imsim_core::flags::FlagValue;
    if !crate::tnmmode::active() {
        return flags.get_bool(name);
    }
    let strip = |s: &str, pre: &str| -> Option<String> {
        (s.len() >= pre.len() && s[..pre.len()].eq_ignore_ascii_case(pre)).then(|| s[pre.len()..].to_string())
    };
    let list = if let Some(r) = strip(name, "Killed_M23_") {
        format!("23_ABIExterior_and_23_ABIInterior_and_23_ABIRuins_{r}")
    } else if let Some(r) = strip(name, "Killed_") {
        r
    } else if name.eq_ignore_ascii_case("HAS_LlamaHostageCorpse") {
        return crate::carry::carried_bind().eq_ignore_ascii_case("LlamaHostage");
    } else if name.eq_ignore_ascii_case("HAS_MasterKaleCorpse") {
        return crate::carry::carried_bind().eq_ignore_ascii_case("Master_Kale");
    } else {
        return flags.get_bool(name);
    };
    let int = |n: &str| match flags.get(n) {
        Some(FlagValue::Int(v)) => *v,
        _ => 0,
    };
    let mut maps: Vec<&str> = list.split("_and_").collect();
    let last = maps.pop().unwrap_or("");
    let (last_map, pct) = last.rsplit_once('_').unwrap_or((last, "0"));
    let pct: i32 = pct.parse().unwrap_or(0);
    maps.push(last_map);
    let (mut killed, mut base) = (0, 0);
    for m in maps {
        let start = int(&format!("{m}_StartCount"));
        if start > 0 {
            killed += start - int(&format!("{m}_Count"));
            base += start;
        }
    }
    base == 0 || 100 * killed / base < pct || killed == 0
}

/// Each pawn augmentation class's Value (lower-case class name), read once from the mod's packages.
static AUG_VALUES: std::sync::Mutex<Vec<(String, f32)>> = std::sync::Mutex::new(Vec::new());

/// Read the pawn augmentations' values (called once the class database is up).
pub fn load(classes: &mut unreal_pkg::classes::ClassDb) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut out = Vec::new();
    let mut kinds = Vec::new();
    for c in [
        "tnmAugBallistic", "tnmAugEMP", "tnmAugShield", "tnmAugEnviro", "tnmnpcAugCombat", "tnmAugHealing", "tnmAugSpeed",
        "tnmAugCloak", "tnmAugCloakhealth", "tnmAugCloakcombat", "tnmAugStealth", "tnmAugRadarTrans", "tnmAugTarget",
        "tnmAugVision", "tnmAugDefense", "tnmAugDrone", "tnmAugHeartLung", "tnmAugLight",
    ] {
        let d = classes.defaults(c);
        if let Some(Value::Float(v)) = find(&d, "Value") {
            out.push((c.to_ascii_lowercase(), *v));
        }
        kinds.push((c.to_ascii_lowercase(), flag(&d, "bhealthactivated"), flag(&d, "bcombatactivated")));
    }
    if let Ok(mut t) = AUG_KINDS.lock() {
        *t = kinds;
    }
    if let Ok(mut t) = AUG_VALUES.lock() {
        *t = out;
    }
}

/// Each pawn augmentation class: (name, bhealthactivated, bcombatactivated).
static AUG_KINDS: std::sync::Mutex<Vec<(String, bool, bool)>> = std::sync::Mutex::new(Vec::new());

fn aug_value(aug: &str) -> Option<f32> {
    AUG_VALUES.lock().ok().and_then(|v| v.iter().find(|x| x.0 == aug).map(|x| x.1))
}

fn aug_kind(aug: &str) -> (bool, bool) {
    AUG_KINDS.lock().ok().and_then(|v| v.iter().find(|x| x.0 == aug).map(|x| (x.1, x.2))).unwrap_or((false, false))
}

/// The augmentations of augmented pawns (tnmpawnaugmanager and its tnmAugmentations): plain ones run from the
/// start, health ones come on at AugHealthLimit (the healing one mends every part by its Value a second until the
/// pawn is over the limit again; the health cloak drops then too), combat ones when the pawn catches fire - the
/// only place the mod switches them on. Everything goes off when the pawn stops attacking, runs out of bioenergy
/// or dies. A cloaked pawn is all but invisible (the original draws it at a twentieth of its glow): hidden here.
// uc: tnmpawnaugmanager.CreateAugmentations, tnmpawnaugmanager.ActivateAugs, tnmpawnaugmanager.DeactivateAll,
// uc: tnmAugCloak.Active, tnmAugCloakhealth.Active, tnmAugCloakcombat.Active, tnmAugHealing.Active, tnmAugSpeed.Active
pub fn aug_system(time: Res<Time>, mut pawns: Query<(&mut crate::ai::Npc, &mut Visibility, &crate::actors::Deco)>) {
    let dt = time.delta_secs().min(0.1);
    for (mut npc, mut vis, deco) in &mut pawns {
        if npc.hp_body.tnm.as_ref().is_none_or(|t| t.augs.is_empty()) {
            continue;
        }
        let npc = &mut *npc;
        let dead = matches!(npc.state, crate::ai::NpcState::Dead);
        let attacking = matches!(npc.state, crate::ai::NpcState::Attacking);
        let burning = npc.on_fire.is_some();
        let health = npc.hp_body.health;
        let Some(t) = npc.hp_body.tnm.as_mut() else { continue };
        let was_cloaked = ["tnmaugcloak", "tnmaugcloakhealth", "tnmaugcloakcombat"].iter().any(|a| t.running(a));
        if !t.aug_made {
            t.aug_made = true;
            t.aug_on = t.augs.iter().map(|a| aug_kind(a) == (false, false) && t.energy > 0.0).collect();
        }
        let activate = |t: &mut TnmBody, health_kind: bool, combat_kind: bool| {
            // ActivateAugs: with no bioenergy everything goes off instead.
            if t.energy <= 0.0 {
                t.aug_on.iter_mut().for_each(|o| *o = false);
                return;
            }
            for i in 0..t.augs.len() {
                let (h, c) = aug_kind(&t.augs[i]);
                if (!h && !c) || (health_kind && h) || (combat_kind && c) {
                    t.aug_on[i] = true;
                }
            }
        };
        let mut heal = 0.0f32;
        if dead {
            t.aug_on.iter_mut().for_each(|o| *o = false);
        } else {
            if t.was_attacking && !attacking {
                t.aug_on.iter_mut().for_each(|o| *o = false);
            }
            if burning && !t.was_burning {
                activate(t, false, true);
            }
            if health <= t.aug_health_limit && health > 0.0 && !t.running("tnmaughealing") && !t.running("tnmaugcloakhealth") {
                activate(t, true, false);
            }
            t.aug_clock += dt;
            if t.aug_clock >= 1.0 {
                t.aug_clock -= 1.0;
                for i in 0..t.augs.len() {
                    if !t.aug_on[i] {
                        continue;
                    }
                    match t.augs[i].as_str() {
                        "tnmaughealing" => {
                            if health < t.aug_health_limit {
                                heal = aug_value("tnmaughealing").unwrap_or(0.0);
                            } else {
                                t.aug_on[i] = false;
                            }
                        }
                        "tnmaugcloakhealth" if health >= t.aug_health_limit => t.aug_on[i] = false,
                        _ => {}
                    }
                }
            }
        }
        t.was_attacking = attacking;
        t.was_burning = burning;
        let cloaked = ["tnmaugcloak", "tnmaugcloakhealth", "tnmaugcloakcombat"].iter().any(|a| t.running(a));
        if cloaked != was_cloaked || cloaked != t.cloaked {
            let at = deco.body.location;
            if cloaked && !t.cloaked {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                    t.cloaked = true;
                    crate::sound::play("DeusExSounds.Augmentation.CloakUp", at, 0.85, 768.0, 1.0);
                }
            } else if !cloaked && t.cloaked {
                t.cloaked = false;
                *vis = Visibility::Inherited;
                crate::sound::play("DeusExSounds.Augmentation.CloakDown", at, 0.85, 768.0, 1.0);
            }
        }
        if heal > 0.0 {
            // HealPawn: each part in turn takes the points, what spills over goes on to the next.
            let b = &mut npc.hp_body;
            if b.animal {
                b.health = (b.health + heal).min(b.max_health);
            } else {
                let mut amt = heal;
                for i in 0..6 {
                    let cap = b.defaults[i];
                    let v = b.parts[i] + amt;
                    amt = (v - cap).max(0.0);
                    b.parts[i] = v.min(cap);
                }
                b.total();
            }
        }
    }
}

/// Nightmare mode (TNMPawn.SpawnCarcass + RespawnInfo): a pawn that leaves a body - dead or knocked out - comes
/// back 25 seconds later (RespawnInfo's LifeSpan of 30 running under 5) as a new pawn of its class with the same
/// orders, alliances and inventory, in a flash, and the body goes. Not SD, Athena, Treskton or Kevo; not when the
/// body was blown apart, and not while it is being carried (the carcass is gone from the world then).
// uc: TNMPawn.SpawnCarcass, RespawnInfo.Tick, RespawnInfo.Respawn
pub fn respawn_system(
    mut commands: bevy::prelude::Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut blockers: bevy::prelude::ResMut<crate::actors::Blockers>,
    mut pawns: Query<(bevy::prelude::Entity, &mut crate::ai::Npc, &crate::actors::Deco)>,
    mut assets: bevy::prelude::ResMut<crate::weapons::RuntimeAssets>,
) {
    if !crate::tnmmode::difficulty().is_some_and(|d| d.nightmare) {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let lvl = &level.0;
    let life = match find(&assets.classes.defaults("RespawnInfo"), "LifeSpan") {
        Some(Value::Float(v)) => (*v - 5.0).max(1.0),
        _ => 25.0,
    };
    for (e, mut npc, deco) in &mut pawns {
        if !npc.nightmare || !matches!(npc.state, crate::ai::NpcState::Dead) {
            continue;
        }
        let carried = npc.carried;
        let actor = npc.actor;
        let blocker = npc.blocker;
        let Some(t) = npc.hp_body.tnm.as_mut() else { continue };
        let Some(a) = lvl.actors.get(actor) else { continue };
        match t.respawn {
            None => {
                if ["SD", "Athena", "Treskton", "Kevo"].iter().any(|c| crate::classtree::is_a(&a.class, c)) {
                    t.respawn = Some(f32::INFINITY);
                } else {
                    t.respawn = Some(life);
                }
            }
            Some(left) if left.is_finite() => {
                if carried {
                    // The carcass left the world: its RespawnInfo has nothing to bring back.
                    t.respawn = Some(f32::INFINITY);
                    continue;
                }
                let left = left - dt;
                t.respawn = Some(left);
                if left > 0.0 {
                    continue;
                }
                t.respawn = Some(f32::INFINITY);
                let at = deco.body.location;
                let h = match find(&a.props, "CollisionHeight") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 40.0,
                };
                let props: Vec<Property> = a
                    .props
                    .iter()
                    .filter(|p| !matches!(p.name.as_str(), "Location" | "Rotation" | "Physics" | "Region" | "Level" | "Base" | "OldLocation"))
                    .cloned()
                    .collect();
                let yaw = match find(&a.props, "Rotation") {
                    Some(Value::Rotator(r)) => r[1],
                    _ => 0,
                };
                crate::spawn::request(crate::spawn::SpawnReq {
                    class: a.class.clone(),
                    pos: at + bevy::math::DVec3::new(0.0, 0.0, (h - deco.body.height).max(0.0)),
                    yaw,
                    transient: false,
                    props,
                    generator: None,
                    net_player: None,
                });
                crate::missions::fx(crate::missions::Fx::SpawnFlash(at));
                if let Some(c) = blockers.list.get_mut(blocker) {
                    c.radius = 0.0;
                    c.height = 0.0;
                }
                commands.entity(e).despawn();
                println!("NIGHTMARE {} is back", a.name);
            }
            _ => {}
        }
    }
}

/// The TrestWoke cheat: every TNMPawn and TNMRobot of the map turned firewall (bFirewall = true).
// uc: Trestkon.TrestWoke
static ALL_FIREWALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_all_firewall() {
    ALL_FIREWALL.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A new map's pawns are their own again.
pub fn reset_cheats() {
    ALL_FIREWALL.store(false, std::sync::atomic::Ordering::Relaxed);
}
