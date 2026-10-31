//! Perks from GMDX Augmented Edition (Perk / PerkSystem, ported by hand): bought with skill points on the Skills
//! screen, most tied to a skill level, a few general. Each perk's name, description, cost, level, value and icon are
//! read from the player's own GMDX DeusEx.u (class defaults: data); what each one does is ported into the system it
//! changes and asks `has()` / `value()` here. Only while the GMDX mod is switched on in the launcher.

use std::collections::BTreeSet;
use std::sync::{Mutex, RwLock};
use unreal_pkg::props::Value;

/// PerkSystem.InitializePerks: every perk, in the order the menus list them.
pub const ORDER: &[&str] = &[
    // Rifle
    "PerkSteady",
    "PerkStoppingPower",
    "PerkMarksman",
    "PerkArmorPiercing",
    // Pistol
    "PerkSidearm",
    "PerkAmbidextrous",
    "PerkHollowPoints",
    "PerkHumanCombustion",
    // Low-tech
    "PerkSharpEyed",
    "PerkPiercing",
    "PerkHemmorhage",
    "PerkInventive",
    // Heavy
    "PerkControlledBurn",
    "PerkBlastEnergy",
    "PerkHeavilyTweaked",
    "PerkHERocket",
    "PerkPlasmaBreeder",
    "PerkMobileOrdnance",
    // Demolition
    "PerkRemoteDetonation",
    "PerkSonicTransducerSensor",
    "PerkShortFuse",
    "PerkSensorBurnout",
    "PerkKnockoutGas",
    "PerkBreakdown",
    // Lockpicking
    "PerkDoorsman",
    "PerkSleightOfHand",
    "PerkLocksport",
    // Electronics
    "PerkSabotage",
    "PerkWirelessStrength",
    "PerkCracked",
    // Medicine
    "PerkToxicologist",
    "PerkBiogenic",
    "PerkCombatMedicsBag",
    // Environmental training
    "PerkHardened",
    "PerkFilterUpgrade",
    "PerkFieldRepair",
    "PerkBlastPadding",
    "PerkThermalImaging",
    "PerkChameleon",
    // Athletics
    "PerkPerserverance",
    "PerkSprinter",
    "PerkAdrenalineRush",
    "PerkEndurance",
    // Stealth
    "PerkNimble",
    "PerkSecurityLoophole",
    "PerkWetwork",
    "PerkTacticalDistraction",
    // Hacking
    "PerkDataRecovery",
    "PerkModder",
    "PerkMisfeatureExploit",
    "PerkTurretDomination",
    // General
    "PerkFirefighter",
    "PerkLawfare",
    "PerkGlutton",
    "PerkSocketJockey",
];

/// Perks left out of the menus: they need a GMDX system we don't have (NPC helmets, the secondary item slot, the
/// rocket-cam, stamina, the STOP! worm, stacking LAWs, wall sockets, biocell recharging) or would only give back
/// what the original game already does (Chameleon: camo already fools lasers; Turret Domination: every turret
/// setting already works). PerkSystem.IsVisible does the same for a few of them.
pub const HIDDEN: &[&str] = &[
    "PerkArmorPiercing",
    "PerkInventive",
    "PerkHeavilyTweaked",
    "PerkMobileOrdnance",
    "PerkFilterUpgrade",
    "PerkFieldRepair",
    "PerkChameleon",
    "PerkAdrenalineRush",
    "PerkEndurance",
    "PerkModder",
    "PerkTurretDomination",
    "PerkLawfare",
    "PerkSocketJockey",
];

/// One perk's defaults.
#[derive(Clone, Debug, Default)]
pub struct Perk {
    pub class: String,
    pub name: String,
    pub description: String,
    /// The skill class it belongs to (SkillWeaponRifle...), or None for a general perk.
    pub skill: Option<String>,
    pub cost: i32,
    pub level: usize,
    pub value: f32,
    /// EPerkValueDisplay: 0 the number, 1 x100, 2 x100 - 100.
    pub display: u8,
    /// PerkIcon (else the skill's icon).
    pub icon: Option<String>,
}

impl Perk {
    /// The description with its value put in (CreatePerkOverview's sprintf).
    pub fn text(&self) -> String {
        let v = match self.display {
            2 => format!("{}", (self.value * 100.0 - 100.0) as i32),
            1 => format!("{}", (self.value * 100.0) as i32),
            _ => {
                // TextDisplayHack(value, 3): up to three decimals, trailing zeros dropped.
                let s = format!("{:.3}", self.value);
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        };
        let mut out = String::new();
        let mut it = self.description.chars().peekable();
        while let Some(c) = it.next() {
            if c == '%' {
                match it.peek().copied() {
                    Some('%') => {
                        it.next();
                        out.push('%');
                    }
                    Some('d') | Some('s') => {
                        it.next();
                        out.push_str(&v);
                    }
                    _ => out.push('%'),
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}

/// UI strings (PersonaScreenSkills / PersonaInfoWindow defaults in GMDX's DeusEx.u).
#[derive(Clone, Debug)]
pub struct Labels {
    pub perks: String,
    pub perks_general: String,
    pub info: String,
    pub upgrade: String,
    pub purchased: String,
    pub unobtainable: String,
    pub points: String,
    pub obtained: String,
    pub general_title: String,
    pub requires: String,
}

static LIST: RwLock<Vec<Perk>> = RwLock::new(Vec::new());
static LABELS: RwLock<Option<Labels>> = RwLock::new(None);
static OWNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Read the perks from the GMDX mod's DeusEx.u (when the mod is on).
pub fn init(root: &std::path::Path) {
    let Some(code) = unreal_pkg::mods::active("gmdx").and_then(|d| unreal_pkg::mods::find_rel(&d, "System/DeusEx.u")) else {
        if let Ok(mut l) = LIST.write() {
            l.clear();
        }
        return;
    };
    let sys = root.join("System");
    let mut gm = unreal_pkg::classes::ClassDb::open_replacing(&sys, &[("DeusEx", code)]);
    let s = |p: &[unreal_pkg::props::Property], n: &str| match unreal_pkg::props::find(p, n) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    };
    let mut list = Vec::new();
    for class in ORDER {
        if HIDDEN.contains(class) {
            continue;
        }
        let d = gm.defaults(class);
        if d.is_empty() {
            continue;
        }
        let f = |n: &str| match unreal_pkg::props::find(&d, n) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f32,
            _ => 0.0,
        };
        let path = |n: &str| match unreal_pkg::props::find(&d, n) {
            Some(Value::Path(p)) => Some(p.clone()),
            _ => None,
        };
        list.push(Perk {
            class: class.to_string(),
            name: s(&d, "PerkName"),
            description: s(&d, "PerkDescription"),
            skill: path("PerkSkill").map(|p| p.rsplit('.').next().unwrap_or("").to_string()),
            cost: f("PerkCost") as i32,
            level: f("PerkLevelRequirement") as usize,
            value: f("PerkValue"),
            display: match unreal_pkg::props::find(&d, "PerkValueDisplay") {
                Some(Value::Byte(b)) => *b,
                _ => 0,
            },
            icon: path("PerkIcon"),
        });
    }
    let sk = gm.defaults("PersonaScreenSkills");
    let info = gm.defaults("PersonaInfoWindow");
    let or = |v: String, d: &str| if v.is_empty() { d.to_string() } else { v };
    let labels = Labels {
        perks: or(s(&sk, "PerksButtonLabel"), "|&Perks"),
        perks_general: or(s(&sk, "PerksButtonLabel2"), "Perks (|&G)"),
        info: or(s(&sk, "LevelsButtonLabel"), "|&Info"),
        upgrade: or(s(&info, "UpgradeButtonLabel"), "|&Upgrade"),
        purchased: or(s(&info, "PurchasedButtonLabel"), "|&Already Upgraded"),
        unobtainable: or(s(&info, "UnobtainableButtonLabel"), "|&Cannot Upgrade"),
        points: or(s(&info, "RequiredPoints"), "Points Needed: "),
        obtained: or(s(&info, "ob"), "OBTAINED PERKS"),
        general_title: or(s(&info, "GeneralPerksTitleText"), "Perks - General"),
        requires: or(s(&info, "PerkRequiredSkill"), "Requires: %s: %s"),
    };
    println!("perks: {} from GMDX", list.len());
    // Dev aid: DXVIEW_PERKS="PerkA,PerkB" owned from the start.
    if let Ok(v) = std::env::var("DXVIEW_PERKS") {
        if let Ok(mut o) = OWNED.lock() {
            o.extend(v.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()));
        }
    }
    if let Ok(mut l) = LIST.write() {
        *l = list;
    }
    if let Ok(mut l) = LABELS.write() {
        *l = Some(labels);
    }
}

/// Perks are on (the GMDX mod is).
pub fn active() -> bool {
    LIST.read().is_ok_and(|l| !l.is_empty())
}

pub fn labels() -> Option<Labels> {
    LABELS.read().ok().and_then(|l| l.clone())
}

/// Every perk, in menu order.
pub fn list() -> Vec<Perk> {
    LIST.read().map(|l| l.clone()).unwrap_or_default()
}

/// The perks for one skill (by class), or the general ones (None), in menu order.
pub fn for_skill(skill: Option<&str>) -> Vec<Perk> {
    list()
        .into_iter()
        .filter(|p| match (&p.skill, skill) {
            (None, None) => true,
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        })
        .collect()
}

/// The player owns this perk (bPerkObtained).
pub fn has(class: &str) -> bool {
    active() && OWNED.lock().is_ok_and(|o| o.contains(class))
}

/// The perk's PerkValue when owned.
pub fn value_if(class: &str) -> Option<f32> {
    if !has(class) {
        return None;
    }
    LIST.read().ok()?.iter().find(|p| p.class == class).map(|p| p.value)
}

/// Perk.IsPurchasable: not owned, the skill at the level needed, enough points.
pub fn purchasable(p: &Perk) -> bool {
    if has(&p.class) {
        return false;
    }
    let g = crate::abilities::lock();
    let Some(a) = g.as_ref() else { return false };
    let level_ok = p.skill.as_deref().map_or(true, |s| a.skills.level(s) >= p.level);
    level_ok && a.skills.points_avail >= p.cost
}

/// PerkSystem.PurchasePerk: pay the points, own it, OnPerkPurchase.
pub fn purchase(class: &str) -> bool {
    let Some(p) = list().into_iter().find(|p| p.class == class) else { return false };
    if !purchasable(&p) {
        return false;
    }
    if let Some(a) = crate::abilities::lock().as_mut() {
        a.skills.points_avail -= p.cost;
    }
    if let Ok(mut o) = OWNED.lock() {
        o.insert(p.class.clone());
    }
    // GMDXSFX.Generic.codelearned when the player has GMDX's sounds; else the skill upgrade's chime.
    let gmdx_sfx = unreal_pkg::mods::active("gmdx")
        .is_some_and(|d| unreal_pkg::mods::find_rel(&d, "System/GMDXSFX.u").or_else(|| unreal_pkg::mods::find_rel(&d, "Sounds/GMDXSFX.uax")).is_some());
    crate::sound::play_player(if gmdx_sfx { "GMDXSFX.Generic.codelearned" } else { "DeusExSounds.UserInterface.LogNoteAdded" }, 0.8, 0.0);
    on_purchase(&p.class);
    true
}

/// Perk.OnPerkPurchase / OnMapLoadAndPurchase for the perks that do something at once.
fn on_purchase(_class: &str) {}

/// The player's item in hand (class, bHandToHand), for perks that test inHand at damage time.
static IN_HAND: Mutex<Option<(String, bool)>> = Mutex::new(None);

pub fn set_in_hand(w: Option<(&str, bool)>) {
    if let Ok(mut h) = IN_HAND.lock() {
        match (h.as_ref(), w) {
            (Some((c, b)), Some((nc, nb))) if c == nc && *b == nb => {}
            _ => *h = w.map(|(c, b)| (c.to_string(), b)),
        }
    }
}

/// A melee weapon is in the player's hand (achievements: Slugger).
pub fn in_hand_melee() -> bool {
    in_hand().is_some_and(|(_, m)| m)
}

fn in_hand() -> Option<(String, bool)> {
    IN_HAND.lock().ok().and_then(|h| h.clone())
}

/// The player has this class in hand.
pub fn holding(class: &str) -> bool {
    in_hand().is_some_and(|(c, _)| c.eq_ignore_ascii_case(class))
}

/// PerkPiercing.GetPiercingPerkMult: a low-tech blade or club in hand does PerkValue x damage to bots and security
/// systems (Robot / SecurityCamera / AutoTurretGun / AlarmUnit TakeDamage, the player's damage only).
pub fn piercing_mult(target_class: &str, robot: bool) -> f32 {
    let Some(v) = value_if("PerkPiercing") else { return 1.0 };
    let low_tech = in_hand().is_some_and(|(c, _)| {
        matches!(c.as_str(), "WeaponCombatKnife" | "WeaponShuriken" | "WeaponCrowbar" | "WeaponNanoSword" | "WeaponBaton")
    });
    let security = ["SecurityCamera", "AutoTurretGun", "AlarmUnit"]
        .iter()
        .any(|b| crate::classtree::is_a(target_class, b));
    if low_tech && (robot || security) {
        v
    } else {
        1.0
    }
}

/// Piercing's flinch: with a hand-to-hand weapon in hand the pawn's pain cooldown (TakeHitTimer) is 1.3 s.
pub fn flinch_time() -> Option<f32> {
    (has("PerkPiercing") && in_hand().is_some_and(|(_, h2h)| h2h)).then_some(1.3)
}

/// Hollow Points (DeusExWeapon.ProcessTraceHit): 10mm rounds do PerkValue x damage to animals.
pub fn hollow_points_mult(ammo: Option<&str>, target_class: &str) -> f32 {
    match value_if("PerkHollowPoints") {
        Some(v) if ammo.is_some_and(|a| a.eq_ignore_ascii_case("Ammo10mm")) && crate::classtree::is_a(target_class, "Animal") => v,
        _ => 1.0,
    }
}

/// Glutton (RSDEdible.GetHealAmount): food and drink heal 1.5 x as much, rounded up.
pub fn food(amount: f32) -> f32 {
    if has("PerkGlutton") {
        (amount * 1.5 + 0.9).floor()
    } else {
        amount
    }
}

/// Hemorrhage: shuriken hits from the player waiting to start a bleed, (blocker, damage).
static BLEED: Mutex<Vec<(usize, f32)>> = Mutex::new(Vec::new());

/// A player's shuriken hit this pawn with Hemorrhage owned (DeusExProjectile: damageType 'Bleed').
pub fn mark_bleed(blocker: usize, damage: f32) {
    if let Ok(mut b) = BLEED.lock() {
        b.push((blocker, damage));
    }
}

/// The pawn's hit of `damage` was such a shuriken: StartBleed.
pub fn take_bleed(blocker: usize, damage: f32) -> bool {
    let Ok(mut b) = BLEED.lock() else { return false };
    match b.iter().position(|&(x, d)| x == blocker && (d - damage).abs() < 0.01) {
        Some(i) => {
            b.swap_remove(i);
            true
        }
        None => false,
    }
}

/// A new game: no perks.
pub fn new_game() {
    if let Ok(mut o) = OWNED.lock() {
        o.clear();
    }
}

/// The owned perks, for the save.
pub fn save() -> Vec<String> {
    OWNED.lock().map(|o| o.iter().cloned().collect()).unwrap_or_default()
}

pub fn restore(v: &[String]) {
    if let Ok(mut o) = OWNED.lock() {
        *o = v.iter().cloned().collect();
    }
}

/// Spawn menu / cheats: every perk (PerkSystem.AddAll).
pub fn add_all() {
    if let Ok(mut o) = OWNED.lock() {
        for p in list() {
            o.insert(p.class);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_values() {
        let mut p = Perk { description: "Heals %d%% faster.".into(), value: 1.25, display: 2, ..Default::default() };
        assert_eq!(p.text(), "Heals 25% faster.");
        p.display = 1;
        p.value = 0.5;
        assert_eq!(p.text(), "Heals 50% faster.");
        p.display = 0;
        p.value = 5.0;
        p.description = "+%d energy".into();
        assert_eq!(p.text(), "+5 energy");
    }
}
