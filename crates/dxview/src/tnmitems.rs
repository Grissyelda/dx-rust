//! The Nameless Mod's pickups (TNMItems.u, TNM.u): which Deus Ex pickup each one acts as, and what its own ones do
//! when used. The classes' numbers (heal and recharge amounts, charges, messages) are read from the player's own
//! packages; only the rules of each `Activated` state are written here.
//!
//! * TNMMedKit / TNMBioelectricCell / TNMCandybar / TNMSoyFood / TNMSodacan / TNMLiquor40oz / TNMLiquorBottle /
//!   TNMWineBottle / TNMCigarettes / TNMBinoculars / TNMFlare / TNMRebreather / TNMAugmentationCannister are the
//!   Deus Ex pickups under The Nameless Mod's names (`canon`).
//! * Beans (5 health), KetchupBar (2), PlasticCoffee (2 health and 2 energy, a burp), PlasticCoffeeDecaf (2 health),
//!   RedBiocell (its rechargeAmount of energy for 24 points off the torso).
//! * Charged pickups: GasMask (needs its skill trained; gas does nothing while it's on), DuoDamage (everything the
//!   player's weapons do x2), LollerSkates (GroundSpeed x17 until the charge is gone; the fire goes out when they go
//!   on, and living through them is an achievement).
//! * Quest items (DogBlood, KarkianSnot, UraniumStick, FloppyDisk, VirusDisk, PaintingTube, NosferatusTeeth...) have
//!   an empty Activated state: using one does nothing and it isn't used up.

use std::sync::atomic::{AtomicBool, Ordering};

/// The Deus Ex pickup a Nameless Mod pickup acts as (itself when it has rules of its own, or outside the mod).
pub fn canon(class: &str) -> &str {
    if !crate::tnmmode::active() {
        return class;
    }
    crate::util::with_lower(class, |l| match l {
        "tnmmedkit" => Some("MedKit"),
        "tnmbioelectriccell" => Some("BioelectricCell"),
        "tnmcandybar" => Some("Candybar"),
        "tnmsoyfood" => Some("SoyFood"),
        "tnmsodacan" => Some("Sodacan"),
        "tnmliquor40oz" => Some("Liquor40oz"),
        "tnmliquorbottle" => Some("LiquorBottle"),
        "tnmwinebottle" => Some("WineBottle"),
        "tnmcigarettes" => Some("Cigarettes"),
        "tnmbinoculars" => Some("Binoculars"),
        "tnmflare" => Some("Flare"),
        "tnmrebreather" => Some("Rebreather"),
        "tnmaugmentationcannister" => Some("AugmentationCannister"),
        "tnmadaptivearmor" => Some("AdaptiveArmor"),
        "tnmballisticarmor" => Some("BallisticArmor"),
        "tnmhazmatsuit" => Some("HazMatSuit"),
        _ => None,
    })
    .unwrap_or(class)
}

/// A medkit under either name.
pub fn is_medkit(class: &str) -> bool {
    canon(class) == "MedKit"
}

/// The Nameless Mod's own charged pickups (besides the Deus Ex ones `canon` maps).
pub fn is_charged(class: &str) -> bool {
    crate::tnmmode::active() && (class.eq_ignore_ascii_case("GasMask") || class.eq_ignore_ascii_case("DuoDamage") || class.eq_ignore_ascii_case("LollerSkates"))
}

/// The name charged.rs keeps a Nameless Mod charged pickup under.
pub fn charged_name(class: &str) -> &'static str {
    if class.eq_ignore_ascii_case("GasMask") {
        "GasMask"
    } else if class.eq_ignore_ascii_case("DuoDamage") {
        "DuoDamage"
    } else {
        "LollerSkates"
    }
}

static DOUBLE: AtomicBool = AtomicBool::new(false);
static SKATING: AtomicBool = AtomicBool::new(false);
static MASK: AtomicBool = AtomicBool::new(false);

/// Trestkon.bDoubleDamage: a DuoDamage is running.
pub fn double_damage() -> bool {
    DOUBLE.load(Ordering::Relaxed)
}

/// Trestkon.bSkating (GetCurrentGroundSpeed: Default.GroundSpeed x 17).
pub fn skating() -> bool {
    SKATING.load(Ordering::Relaxed)
}

/// A gas mask is on (Trestkon.DXReduceDamage: TearGas, PoisonGas and HalonGas do nothing).
pub fn gas_mask() -> bool {
    MASK.load(Ordering::Relaxed)
}

/// Each frame from charged.rs: which of the mod's charged pickups run. Skates going on put a fire out; coming off
/// alive is achLollerskates.
pub fn set_running(mask: bool, double: bool, skates: bool, ph: &mut crate::ai::PlayerHealth) {
    MASK.store(mask, Ordering::Relaxed);
    DOUBLE.store(double, Ordering::Relaxed);
    let was = SKATING.swap(skates, Ordering::Relaxed);
    if skates && !was {
        ph.on_fire = None;
    }
    if was && !skates && !ph.dead {
        crate::tnmach::unlock("achLollerskates");
    }
}

pub fn reset_level() {
    // (Charged pickups travel with the player: nothing to clear per map.)
}

/// What using one of the mod's own pickups did.
pub enum Used {
    /// Not one of ours.
    No,
    /// Used up, with a message for the log ("" = none).
    Consumed(String),
    /// Nothing happens (quest items, or a mask without the skill): kept, with a message.
    Kept(String),
}

/// `Activated.BeginState` of the mod's own pickups.
pub fn activate(class: &str, d: &[unreal_pkg::props::Property], ph: &mut crate::ai::PlayerHealth) -> Used {
    use unreal_pkg::props::{find, Value};
    if !crate::tnmmode::active() {
        return Used::No;
    }
    let int = |n: &str, def: i32| match find(d, n) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        _ => def,
    };
    let text = |n: &str| match find(d, n) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    // Sprintf with one %d.
    let fmt = |s: String, n: i32| s.replacen("%d", &n.to_string(), 1);
    let burp = || crate::sound::play_player(&crate::travel::voice("DeusExSounds.Player.MaleBurp"), 1.0, 0.0);
    crate::util::with_lower(class, |l| match l {
        // HealPlayer(n, False): no Medicine skill.
        "beans" => {
            ph.heal(5.0);
            Used::Consumed(String::new())
        }
        "ketchupbar" => {
            ph.heal(2.0);
            Used::Consumed(String::new())
        }
        "plasticcoffeedecaf" => {
            ph.heal(2.0);
            burp();
            Used::Consumed(String::new())
        }
        "plasticcoffee" => {
            ph.heal(2.0);
            // ChargePlayer(2): what was really added.
            let before = ph.energy;
            ph.energy = (ph.energy + 2.0).min(100.0);
            burp();
            Used::Consumed(fmt(text("msgRecharged"), (ph.energy - before).round() as i32))
        }
        "redbiocell" => {
            // Energy += rechargeAmount (capped), and TakeDamage 'redbiocell' = 12.5 on the torso (doubled there: 24)
            // with the shocked damage icon.
            let n = int("rechargeAmount", 25);
            ph.energy = (ph.energy + n as f32).min(100.0);
            crate::sound::play_player("DeusExSounds.Generic.BioElectricHiss", 1.0, 256.0);
            ph.damage(12.0, "Shocked", glam::DVec3::ZERO);
            Used::Consumed(fmt(text("msgRecharged"), n))
        }
        // Empty Activated states: carried for somebody, never used.
        "dogblood" | "karkiansnot" | "uraniumstick" | "floppydisk" | "virusdisk" | "paintingtube" | "nosferatusteeth" | "goatdisguise" => {
            Used::Kept(String::new())
        }
        _ => Used::No,
    })
}

/// GasMask.Activate: only with its skill (skillNeeded) trained; else the class's noskillmsg.
pub fn mask_refused(class: &str, d: &[unreal_pkg::props::Property]) -> Option<String> {
    use unreal_pkg::props::{find, Value};
    if !class.eq_ignore_ascii_case("GasMask") {
        return None;
    }
    let skill = match find(d, "skillNeeded") {
        Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or(p).to_string(),
        _ => "SkillEnviro".to_string(),
    };
    if crate::abilities::skill_level(&skill) > 0 {
        return None;
    }
    Some(match find(d, "noskillmsg") {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    })
}
