//! The Nameless Mod's pawn damage rules (TNMPawn / TNMRobot TakeDamage, TakeDamageBase, ModifyDamage and
//! HandleDamage where they differ from ScriptedPawn / Robot). Only the rules live here: every number that is a
//! property of a class is read from the player's own TNM packages.
// uc: TNMPawn.TakeDamage, TNMPawn.TakeDamageBase, TNMPawn.ModifyDamage, TNMPawn.HandleDamage, TNMRobot.TakeDamageBase, TNMRobot.IgnoreDamageType

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
        "EMP" if t.firewall && !robot => h.damage_type = "Shot",
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
    for c in ["tnmAugBallistic", "tnmAugEMP", "tnmAugShield", "tnmAugEnviro", "tnmnpcAugCombat"] {
        if let Some(Value::Float(v)) = find(&classes.defaults(c), "Value") {
            out.push((c.to_ascii_lowercase(), *v));
        }
    }
    if let Ok(mut t) = AUG_VALUES.lock() {
        *t = out;
    }
}
