//! The Nameless Mod's Legendary mode (TNMWeapon.RollForLegendary / ApplyAttribute and the attribute checks in its
//! fire and hit code, TNMPawn.HandleDamage, Trestkon.ProcessMove / Landed). With Legendary on in the custom
//! difficulty, every weapon rolls for attributes the first time it is met: while FRand() <= legendaryChance (halved
//! after each success) it gains one of its class's possibleAttributes. An attribute is a bit in a mask (the enum's
//! order, as Attributes1 / Attributes2 hold them). Names, chances and lists are read from the player's own TNM.u.
// uc: TNMWeapon.RollForLegendary, TNMWeapon.ApplyAttribute, TNMWeapon.HasAttribute, TNMWeapon.ProcessTraceHit,
// uc: TNMWeapon.WeaponDamageType, TNMPawn.HandleDamage

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

use crate::weapons::WeaponStats;

/// EAttribute, in the enum's order (Att_None = 0).
#[allow(dead_code)]
pub mod att {
    pub const LUBRICATED: u8 = 1;
    pub const RUTHLESS: u8 = 2;
    pub const DIRECTED: u8 = 3;
    pub const ZOOMED: u8 = 4;
    pub const SILENCED: u8 = 5;
    pub const PUNCTURING: u8 = 6;
    pub const ARMOR_PIERCING: u8 = 7;
    pub const PAINFUL: u8 = 8;
    pub const BLINDING: u8 = 9;
    pub const PARALYZING: u8 = 10;
    pub const PHREAKING: u8 = 11;
    pub const DESPERATE: u8 = 12;
    pub const JUNKIE: u8 = 13;
    pub const ASSASSIN: u8 = 14;
    pub const VALIANT: u8 = 15;
    pub const QUAKING: u8 = 16;
    pub const VENOMOUS: u8 = 17;
    pub const TECHNOPHOBIC: u8 = 18;
    pub const FRYING: u8 = 19;
    pub const LAMAR: u8 = 20;
    pub const OON: u8 = 21;
    pub const NARCISSIAN: u8 = 22;
    pub const LUMINOUS: u8 = 23;
    pub const PENETRATIVE: u8 = 24;
    pub const RAPID: u8 = 25;
    pub const EMBIGGENING: u8 = 26;
    pub const EXPLOSIVE: u8 = 27;
    pub const DOUBLE_BARRELED: u8 = 28;
    pub const WILD: u8 = 29;
    pub const ENERGIZED: u8 = 30;
    pub const MESSY: u8 = 31;
    pub const LUCKY: u8 = 32;
    pub const RESOURCEFUL: u8 = 33;
    pub const LEECHING: u8 = 34;
    pub const PRECISE: u8 = 35;
    pub const ENLARGED: u8 = 36;
    pub const AUGMENTED: u8 = 37;
    pub const LOW_TECH: u8 = 38;
    pub const DISMANTLING: u8 = 39;
    pub const FOCUSED: u8 = 40;
    pub const INTENSE: u8 = 41;
    pub const LONG: u8 = 42;
    pub const FLEETING: u8 = 43;
    pub const SWIFT: u8 = 44;
    pub const INCENDIARY: u8 = 45;
    pub const DOCILE: u8 = 46;
    pub const PINPOINT: u8 = 47;
    pub const GASEOUS: u8 = 48;
    pub const IRRITATING: u8 = 49;
    pub const PANICKED: u8 = 50;
    pub const SHRUNKEN: u8 = 51;
    pub const LIGHTENED: u8 = 52;
    pub const CRAZED: u8 = 53;
    pub const ENVIRONMENTAL: u8 = 54;
    pub const HUNTER: u8 = 55;
    pub const IMMOLATIVE: u8 = 56;
    pub const TRACKING: u8 = 57;
    pub const FAITHFUL: u8 = 58;
}

/// TNMWeapon.GetAttributeColor: the colour a weapon's belt label takes, by the group its attributes fall in (the
/// first group holding one of them wins). None for a weapon without attributes.
// uc: TNMWeapon.GetAttributeColor
pub fn attribute_color(mask: u64) -> Option<[u8; 3]> {
    use att::*;
    if mask == 0 {
        return None;
    }
    let any = |l: &[u8]| l.iter().any(|a| has(mask, *a));
    Some(if any(&[
        DESPERATE, VALIANT, OON, RUTHLESS, VENOMOUS, DOUBLE_BARRELED, LAMAR, JUNKIE, EXPLOSIVE, HUNTER, LOW_TECH,
        AUGMENTED, TECHNOPHOBIC, NARCISSIAN, LUCKY, PINPOINT, ARMOR_PIERCING, ASSASSIN, EMBIGGENING, FOCUSED, FAITHFUL,
    ]) {
        [170, 0, 0]
    } else if any(&[FLEETING, LUMINOUS]) {
        [56, 243, 171]
    } else if any(&[
        CRAZED, ENVIRONMENTAL, LEECHING, MESSY, GASEOUS, IRRITATING, FRYING, QUAKING, TRACKING, PARALYZING, INTENSE,
        SWIFT, RESOURCEFUL, PUNCTURING, PHREAKING, PENETRATIVE, PANICKED, PAINFUL, IMMOLATIVE, INCENDIARY, BLINDING,
        DISMANTLING, ENERGIZED,
    ]) {
        [112, 176, 74]
    } else if any(&[DIRECTED, LONG, DOCILE, LIGHTENED, ZOOMED, WILD, LUBRICATED, SHRUNKEN, SILENCED, RAPID, PRECISE, ENLARGED]) {
        [207, 106, 50]
    } else {
        [134, 80, 172]
    })
}

pub fn has(mask: u64, a: u8) -> bool {
    mask & (1u64 << a) != 0
}

/// The player's weapons' attributes by class (lower case); a class met without a roll isn't in here yet.
static OWNED: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
/// The weapon in the player's hands: its attributes, for the rules that read "the instigator's weapon".
static HELD: AtomicU64 = AtomicU64::new(0);
static SEED: AtomicU64 = AtomicU64::new(0x4C45_4745_4E44_0001);

pub fn on() -> bool {
    crate::tnmmode::active() && crate::tnmmode::difficulty().is_some_and(|d| d.legendary)
}

fn frand() -> f32 {
    let mut s = SEED.load(Ordering::Relaxed);
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    SEED.store(s, Ordering::Relaxed);
    (s >> 40) as f32 / (1u64 << 24) as f32
}

/// RollForLegendary on a weapon of this class (its defaults `d`) on a map of this mission: the attribute mask.
pub fn roll(d: &[Property], mission: i32) -> u64 {
    let mut chance = match find(d, "legendaryChance") {
        Some(Value::Float(v)) => *v,
        _ => 0.0,
    };
    let mut list = [0u8; 59];
    for p in d.iter().filter(|p| p.name.eq_ignore_ascii_case("possibleAttributes")) {
        if let (Some(s), Value::Byte(b)) = (list.get_mut(p.index as usize), &p.value) {
            *s = *b;
        }
    }
    let n = list.iter().position(|a| *a == 0).unwrap_or(list.len());
    let mut mask = 0u64;
    if n == 0 {
        return 0;
    }
    let pick = || list[((frand() * n as f32) as usize).min(n - 1)];
    let cult = |a: u8| matches!(a, att::NARCISSIAN | att::LAMAR | att::OON);
    let mut guard = 0;
    while frand() <= chance && guard < 16 {
        guard += 1;
        chance /= 2.0;
        let mut a = pick();
        // Past the second day no weapon is made against the cults.
        let mut i = 0;
        while mission > 22 && cult(a) && i < 1000 {
            a = pick();
            i += 1;
        }
        let mut i = 0;
        while has(mask, a) && i < 100 {
            a = pick();
            i += 1;
        }
        if a != 0 && a < 62 {
            mask |= 1u64 << a;
        }
    }
    mask
}

/// Legendary weapons pawns carry, by (owner class, weapon class): what the player finds on the body.
static PAWN_WEAPONS: Mutex<Vec<(String, String, u64)>> = Mutex::new(Vec::new());

pub fn note_pawn_weapon(owner: &str, weapon: &str, mask: u64) {
    if let Ok(mut q) = PAWN_WEAPONS.lock() {
        q.retain(|x| !(x.0.eq_ignore_ascii_case(owner) && x.1.eq_ignore_ascii_case(weapon)));
        q.push((owner.to_string(), weapon.to_string(), mask));
    }
}

pub fn pawn_weapon(owner: &str, weapon: &str) -> Option<u64> {
    PAWN_WEAPONS.lock().ok().and_then(|q| q.iter().find(|x| x.0.eq_ignore_ascii_case(owner) && x.1.eq_ignore_ascii_case(weapon)).map(|x| x.2))
}

/// The mission of the map in play (set by trestkon's frame; the cult attributes stop after day two).
static MISSION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(20);

pub fn mission() -> i32 {
    MISSION.load(Ordering::Relaxed)
}

pub fn set_mission(m: i32) {
    MISSION.store(m, Ordering::Relaxed);
}

/// The player's attributes for a weapon class (0 = none or not rolled).
pub fn mask_of(class: &str) -> u64 {
    OWNED.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&class.to_ascii_lowercase()).copied())).unwrap_or(0)
}

/// Rolls made before the weapon is picked up (dxegInterface.SummonActor: legendaryChance 1.5), by class.
static PREROLLED: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

/// Roll this class now with another legendaryChance; the next copy the player takes carries it.
pub fn preroll(classes: &mut unreal_pkg::classes::ClassDb, class: &str, mission: i32, chance: f32) {
    if !on() {
        return;
    }
    let mut d = classes.defaults(class).to_vec();
    d.retain(|p| !p.name.eq_ignore_ascii_case("legendaryChance"));
    d.push(Property { name: "legendaryChance".into(), index: 0, value: Value::Float(chance) });
    let mask = roll(&d, mission);
    if let Ok(mut q) = PREROLLED.lock() {
        q.retain(|x| !x.0.eq_ignore_ascii_case(class));
        q.push((class.to_string(), mask));
    }
    // The copy that comes out replaces what this class rolled before.
    if let Ok(mut g) = OWNED.lock() {
        if let Some(m) = g.as_mut() {
            m.remove(&class.to_ascii_lowercase());
        }
    }
}

/// The player has this weapon class now: its roll, once. `from` = the attributes the copy came with (a pawn's
/// weapon taken from its body); a rolled weapon keeps what it has.
pub fn acquire(classes: &mut unreal_pkg::classes::ClassDb, class: &str, mission: i32, from: Option<u64>) {
    if !on() || !crate::classtree::is_a(class, "TNMWeapon") {
        return;
    }
    let key = class.to_ascii_lowercase();
    let pre = PREROLLED.lock().ok().and_then(|mut q| q.iter().position(|x| x.0.eq_ignore_ascii_case(class)).map(|i| q.remove(i).1));
    let from = match pre {
        Some(m) if from.is_none_or(|f| f == 0) => Some(m),
        _ => from,
    };
    let Ok(mut g) = OWNED.lock() else { return };
    let m = g.get_or_insert_with(HashMap::new);
    if m.get(&key).is_some_and(|v| *v != 0) {
        return;
    }
    if m.contains_key(&key) && from.is_none_or(|f| f == 0) {
        return;
    }
    let mask = match from {
        Some(f) if f != 0 => f,
        _ => roll(&classes.defaults(class), mission),
    };
    m.insert(key, mask);
    if mask != 0 {
        println!("LEGENDARY {class}: {}", names(classes, mask).join(", "));
    }
}

pub fn names(classes: &mut unreal_pkg::classes::ClassDb, mask: u64) -> Vec<String> {
    let d = classes.defaults("TNMWeapon");
    (1..59u8)
        .filter(|a| has(mask, *a))
        .map(|a| {
            d.iter()
                .find(|p| p.name.eq_ignore_ascii_case("AttributeName") && p.index == a as u32)
                .and_then(|p| match &p.value {
                    Value::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| format!("#{a}"))
        })
        .collect()
}

/// The attributes' lines (UpdateInfo; Faithful's carries the weapon's level).
pub fn descriptions(classes: &mut unreal_pkg::classes::ClassDb, mask: u64, class: &str) -> Vec<String> {
    let d = classes.defaults("TNMWeapon");
    let level = faith_level(class).to_string();
    (1..59u8)
        .filter(|a| has(mask, *a))
        .filter_map(|a| {
            d.iter().find(|p| p.name.eq_ignore_ascii_case("AttributeDesc") && p.index == a as u32).and_then(|p| match &p.value {
                Value::Str(s) if !s.is_empty() => Some(if a == att::FAITHFUL { crate::tnmpc::fmtn(s, &[&level]) } else { s.clone() }),
                _ => None,
            })
        })
        .collect()
}

/// itemName: strTitlePrefix[count] @ the attribute names @ the class's ItemName.
pub fn item_name(classes: &mut unreal_pkg::classes::ClassDb, mask: u64, base: &str) -> String {
    if mask == 0 {
        return base.to_string();
    }
    let n = names(classes, mask);
    let d = classes.defaults("TNMWeapon");
    let prefix = |k: usize| {
        d.iter().find(|p| p.name.eq_ignore_ascii_case("strTitlePrefix") && p.index == k as u32).and_then(|p| match &p.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        })
    };
    let title = prefix(n.len().min(6)).or_else(|| prefix(0)).unwrap_or_default();
    // The newest attribute's name goes in front.
    let mut names = n;
    names.reverse();
    format!("{title} {} {base}", names.join(" ")).trim().to_string()
}

/// ApplyAttribute: what the attributes do to the weapon's own numbers. `combat` = the fight is on (the combat music
/// for the player, Attacking for a pawn): Panicked weapons fire faster then.
pub fn apply_stats(w: &mut WeaponStats, mask: u64, combat: bool) {
    w.legendary = mask;
    if mask == 0 {
        return;
    }
    if has(mask, att::TRACKING) {
        w.can_track = true;
        w.lock_time = 0.5;
    }
    if has(mask, att::LIGHTENED) {
        w.mass *= 0.5;
    }
    w.has_laser |= has(mask, att::DIRECTED);
    w.has_scope |= has(mask, att::ZOOMED);
    w.silenced |= has(mask, att::SILENCED);
    if has(mask, att::ENVIRONMENTAL) {
        w.enviro = 0;
    }
    w.automatic |= has(mask, att::WILD);
    if has(mask, att::RAPID) || (has(mask, att::PANICKED) && combat) {
        w.shot_time *= 0.55;
    }
    if has(mask, att::LONG) {
        w.accurate_range *= 1.5;
        w.max_range *= 1.5;
    }
    if has(mask, att::DOCILE) {
        w.recoil = 0.0;
    }
    if has(mask, att::LUBRICATED) {
        // Five reload mods' worth, as far as the mods already fitted leave room (to half).
        w.reload_time = (w.reload_time * 0.5).max(0.0);
    }
    if has(mask, att::PRECISE) {
        w.base_accuracy = if w.base_accuracy == 0.0 { -0.5 } else { w.base_accuracy * 0.5 };
    }
    if has(mask, att::ENLARGED) && w.reload_count > 0 {
        let diff = ((w.reload_count as f32 * 0.1) as u32).max(1);
        w.reload_count = (w.reload_count + diff * 5).min(255);
    }
}

static FRAME: AtomicU64 = AtomicU64::new(0);
static HELD_AT: AtomicU64 = AtomicU64::new(0);

/// The weapon in the player's hands this frame.
pub fn set_held(mask: u64) {
    HELD.store(mask, Ordering::Relaxed);
    HELD_AT.store(FRAME.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// The attributes of the weapon in the player's hands (0 with empty hands).
pub fn held() -> u64 {
    if FRAME.load(Ordering::Relaxed).saturating_sub(HELD_AT.load(Ordering::Relaxed)) <= 1 {
        HELD.load(Ordering::Relaxed)
    } else {
        0
    }
}

/// WeaponDamageType: armour-piercing rounds are 'AutoShot', puncturing ones 'Sabot'.
pub fn damage_type(mask: u64, dt: &'static str) -> &'static str {
    if dt == "Shot" && has(mask, att::ARMOR_PIERCING) {
        "AutoShot"
    } else if dt == "Shot" && has(mask, att::PUNCTURING) {
        "Sabot"
    } else {
        dt
    }
}

/// The shot's own multiplier: Lucky (one in ten is a critical hit, x3, with its sound) and Desperate (default health
/// over health, 1 to 5). `health` = the owner's (health, default health).
pub fn fire_mult(mask: u64, health: (f32, f32)) -> f32 {
    if mask == 0 {
        return 1.0;
    }
    let mut m = 1.0;
    if has(mask, att::LUCKY) && frand() <= 0.1 {
        m *= 3.0;
        crate::sound::play_player("TNM.CriticalHit", 1.0, 1024.0);
    }
    if has(mask, att::DESPERATE) && health.0 > 0.0 {
        m *= ((health.1 / health.0) as i32).clamp(1, 5) as f32;
    }
    m
}

/// Resourceful: one shot in ten costs no ammunition.
pub fn free_shot(mask: u64) -> bool {
    has(mask, att::RESOURCEFUL) && frand() <= 0.1
}

/// What the shooter is to the hit rules.
#[derive(Clone, Copy, Default)]
pub struct Shooter {
    /// The player fired (the attributes that ask about DeusExPlayer).
    pub player: bool,
    pub drugged: bool,
    pub augs_active: u32,
}

/// The player as a shooter this frame: (state, health, default health, crouched).
static PLAYER: Mutex<(Shooter, f32, f32, bool)> = Mutex::new((Shooter { player: true, drugged: false, augs_active: 0 }, 100.0, 100.0, false));

pub fn set_player(drugged: bool, augs_active: u32, health: f32, default_health: f32, crouched: bool) {
    if let Ok(mut g) = PLAYER.lock() {
        *g = (Shooter { player: true, drugged, augs_active }, health, default_health, crouched);
    }
}

pub fn player() -> Shooter {
    PLAYER.lock().map(|g| g.0).unwrap_or_default()
}

pub fn player_health() -> (f32, f32) {
    PLAYER.lock().map(|g| (g.1, g.2)).unwrap_or((100.0, 100.0))
}

pub fn player_crouched() -> bool {
    PLAYER.lock().map(|g| g.3).unwrap_or(false)
}

/// What a shot hit, as ProcessTraceHit's attribute rules ask.
pub struct Target<'a> {
    pub class: &'a str,
    pub robot: bool,
    pub augmented: bool,
    /// Health left before this hit (None = not known).
    pub health: Option<f32>,
}

/// More damage against the weapon's sworn enemies (x2), or per running augmentation.
pub fn target_mult(mask: u64, t: &Target, s: Shooter) -> f32 {
    if mask == 0 {
        return 1.0;
    }
    let is = |c: &str| crate::classtree::is_a(t.class, c);
    if (has(mask, att::LAMAR) || has(mask, att::NARCISSIAN)) && (is("GenericGoat") || is("Athena") || is("DarkTemplar")) {
        2.0
    } else if (has(mask, att::OON) || has(mask, att::NARCISSIAN)) && (is("GenericLlama") || is("Beefman") || is("Abomnation") || is("SD")) {
        2.0
    } else if has(mask, att::HUNTER) && (is("TNMGreasel") || is("RedGreasel") || is("TNMKarkian") || is("TNMGray")) {
        2.0
    } else if has(mask, att::TECHNOPHOBIC) && t.robot {
        2.0
    } else if has(mask, att::LOW_TECH) && t.augmented {
        2.0
    } else if has(mask, att::JUNKIE) && s.player && s.drugged {
        2.0
    } else if has(mask, att::AUGMENTED) && s.player {
        1.0 + s.augs_active as f32 * 0.2
    } else {
        1.0
    }
}

/// What the player gets back from a kill.
#[derive(Default)]
pub struct Reward {
    pub heal: f32,
    pub charge: f32,
}

static REWARDS: Mutex<Vec<Reward>> = Mutex::new(Vec::new());
static BLASTS: Mutex<Vec<(bevy::math::DVec3, f32, f64)>> = Mutex::new(Vec::new());

/// The second hit some attributes land on a pawn (first match, as the original's chain): (damage, type). `dmg` =
/// HitDamage x mult, `raw` = HitDamage. Leeching / Energized pay out when the hit kills.
pub fn extra_hit(mask: u64, t: &Target, s: Shooter, dmg: f32, raw: f32) -> Option<(f32, &'static str)> {
    if mask == 0 {
        return None;
    }
    let kills = t.health.is_some_and(|h| h - dmg <= 0.0);
    if has(mask, att::CRAZED) && t.robot {
        Some((dmg * 10.0, "NanoVirus"))
    } else if has(mask, att::FRYING) && t.robot {
        Some((dmg, "EMP"))
    } else if has(mask, att::BLINDING) {
        Some((dmg, "TearGas"))
    } else if has(mask, att::PARALYZING) {
        Some((dmg, "Stunned"))
    } else if has(mask, att::EMBIGGENING) {
        Some((dmg, "Fat"))
    } else if has(mask, att::VENOMOUS) {
        Some((raw, "Poison"))
    } else if has(mask, att::LEECHING) && s.player && kills {
        if let Ok(mut q) = REWARDS.lock() {
            q.push(Reward { heal: raw, charge: 0.0 });
        }
        None
    } else if has(mask, att::ENERGIZED) && s.player && kills {
        if let Ok(mut q) = REWARDS.lock() {
            q.push(Reward { heal: 0.0, charge: 5.0 });
        }
        None
    } else if has(mask, att::IMMOLATIVE) {
        Some((1.0, "Flamed"))
    } else {
        None
    }
}

/// Pinpoint: one point of 'headstun' before the hit itself.
pub fn pinpoint(mask: u64) -> bool {
    has(mask, att::PINPOINT)
}

/// Explosive rounds: a blast where the slug lands (damage x5, radius min(dmg, 25 x mult) x 1.75).
pub fn blast(mask: u64, at: bevy::math::DVec3, dmg: f32, mult: f32) {
    // Incendiary first (TNMWeapon.ProcessTraceHit's chain): a fire left burning where the shot lands.
    if has(mask, att::INCENDIARY) {
        ground_fire(at);
        return;
    }
    if has(mask, att::EXPLOSIVE) {
        let r = (dmg * 1.75).min(25.0 * mult * 1.75) as f64;
        if let Ok(mut q) = BLASTS.lock() {
            q.push((at, dmg * 5.0, r));
        }
    }
}

/// Clouds to leave behind: (class, where, DrawScale, LifeSpan). projectiles::launch_system makes them.
static CLOUDS: Mutex<Vec<(&'static str, bevy::math::DVec3, f32, f32)>> = Mutex::new(Vec::new());

pub fn take_clouds() -> Vec<(&'static str, bevy::math::DVec3, f32, f32)> {
    CLOUDS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Incendiary: an incinerator (a burning cloud) at the hit, DrawScale 2-2.5, 30-40 s, floating.
// uc: TNMWeapon.ProcessTraceHit, TNMProjectile.SpawnEffects, incinerator.DoZoneCheck
pub fn ground_fire(at: bevy::math::DVec3) {
    if let Ok(mut q) = CLOUDS.lock() {
        q.push(("incinerator", at, frand() * 0.5 + 2.0, frand() * 10.0 + 30.0));
    }
}

/// An exploding projectile's Irritating (tear gas) or Gaseous (poison gas) clouds: blastRadius / 36 tries over the
/// blast, nine in ten made, scattered over a square of the radius and 32 up.
// uc: TNMProjectile.Explode, TNMProjectile.SpawnTearGas, TNMProjectile.SpawnPoisonGas
pub fn blast_gas(mask: u64, at: bevy::math::DVec3, radius: f64) {
    let class = if has(mask, att::IRRITATING) {
        "TearGas"
    } else if has(mask, att::GASEOUS) {
        "PoisonGas"
    } else {
        return;
    };
    let n = (radius / 36.0) as usize;
    let Ok(mut q) = CLOUDS.lock() else { return };
    for _ in 0..n {
        if frand() < 0.9 {
            let p = at + bevy::math::DVec3::new(frand() as f64 * radius - radius * 0.5, frand() as f64 * radius - radius * 0.5, 32.0);
            q.push((class, p, frand() * 0.5 + 2.0, frand() * 10.0 + 30.0));
        }
    }
}

/// Messy: a pawn this weapon kills is left at Health -200 (gibbed). Pawns hit lately by a messy weapon.
static MESSY: Mutex<Vec<(usize, std::time::Instant)>> = Mutex::new(Vec::new());
static MESSY_AREAS: Mutex<Vec<(bevy::math::DVec3, f64, std::time::Instant)>> = Mutex::new(Vec::new());

pub fn messy_hit(mask: u64, blocker: usize) {
    if has(mask, att::MESSY) {
        if let Ok(mut q) = MESSY.lock() {
            q.retain(|x| x.1.elapsed().as_secs_f32() < 1.0 && x.0 != blocker);
            q.push((blocker, std::time::Instant::now()));
        }
    }
}
pub fn messy_blast(mask: u64, at: bevy::math::DVec3, radius: f64) {
    if has(mask, att::MESSY) {
        if let Ok(mut q) = MESSY_AREAS.lock() {
            q.retain(|x| x.2.elapsed().as_secs_f32() < 1.0);
            q.push((at, radius, std::time::Instant::now()));
        }
    }
}
/// The pawn dying now was hit by a messy weapon (or stood in its blast) a moment ago: it's gibbed.
pub fn messy_kill(blocker: usize, center: bevy::math::DVec3) -> bool {
    let hit = MESSY.lock().map(|q| q.iter().any(|x| x.0 == blocker && x.1.elapsed().as_secs_f32() < 0.5)).unwrap_or(false);
    hit || MESSY_AREAS.lock().map(|q| q.iter().any(|x| x.2.elapsed().as_secs_f32() < 0.5 && (x.0 - center).length() <= x.1 + 40.0)).unwrap_or(false)
}

/// Faithful: kills with the weapon, and its level (every tenth kill up to fifty; at five it hits like a cone).
static FAITH: Mutex<Option<HashMap<String, (u32, u32)>>> = Mutex::new(None);

/// The player killed a pawn with this weapon class (TNMWeapon.ProcessTraceHit's numKills++).
// uc: TNMWeapon.ProcessTraceHit, TNMWeapon.UpdateInfo
pub fn note_kill(class: &str) {
    let mask = mask_of(class);
    let Ok(mut g) = FAITH.lock() else { return };
    let m = g.get_or_insert_with(HashMap::new);
    let e = m.entry(class.to_ascii_lowercase()).or_insert((0, 0));
    e.0 += 1;
    if has(mask, att::FAITHFUL) && e.0 <= 50 && e.0 % 10 == 0 {
        e.1 += 1;
        crate::classic_hud::client_message("Weapon level increased!");
        crate::sound::play_player("TNM.Weapons.FirestaffReload", 1.0, 0.0);
    }
}

static HELD_CLASS: Mutex<String> = Mutex::new(String::new());

/// The player's weapon in hand (the one credited with a kill).
pub fn set_held_class(class: &str) {
    if let Ok(mut g) = HELD_CLASS.lock() {
        if *g != class {
            *g = class.to_string();
        }
    }
}
pub fn held_class() -> String {
    HELD_CLASS.lock().map(|g| g.clone()).unwrap_or_default()
}

/// katanaLevel of the weapon class.
pub fn faith_level(class: &str) -> u32 {
    FAITH.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&class.to_ascii_lowercase()).map(|e| e.1))).unwrap_or(0)
}

/// Shrunken: one column narrower in the inventory (and the small icon when it's one wide).
// uc: TNMWeapon.ApplyAttribute
pub fn shrink(class: &str, size: &mut (u32, u32), large_icon: &mut Option<String>, large_w: &mut u32, icon: Option<&str>, default_w: u32) {
    if !on() || !has(mask_of(class), att::SHRUNKEN) {
        return;
    }
    if size.0 > 1 {
        size.0 = default_w.max(2) - 1;
    }
    if let (1, Some(icon)) = (size.0, icon) {
        *large_icon = Some(icon.to_string());
        let c = crate::tnmweapon::canon(class);
        *large_w = match c {
            "WeaponBlowPipe" => 52,
            _ => 42,
        };
    }
}

/// Dismantling: five times the damage to walls and doors.
pub fn wall_mult(mask: u64) -> f32 {
    if has(mask, att::DISMANTLING) {
        5.0
    } else {
        1.0
    }
}

/// TNMPawn.HandleDamage: the instigator's weapon doubles the hit from behind (Assassin's), from the front (Valiant),
/// below the head (Ruthless), or while its player crouches (Focused). `off` = the hit in the pawn's own axes.
pub fn handle_mult(mask: u64, off_x: f32, below_head: bool, player_crouched: bool) -> f32 {
    if mask == 0 {
        1.0
    } else if off_x < 0.0 && has(mask, att::ASSASSIN) {
        2.0
    } else if off_x > 0.0 && has(mask, att::VALIANT) {
        2.0
    } else if below_head && has(mask, att::RUTHLESS) {
        2.0
    } else if player_crouched && has(mask, att::FOCUSED) {
        2.0
    } else {
        1.0
    }
}

/// A legendary pawn (its weapon rolled attributes) takes less: damage / ((ln(count) + 1.5) x 1.1).
pub fn pawn_resist(count: u32) -> f32 {
    if count == 0 {
        1.0
    } else {
        1.0 / (((count as f32).ln() + 1.5) * 1.1)
    }
}

/// Payouts and blasts queued by this frame's hits.
pub fn legendary_system(
    mut ph: bevy::prelude::ResMut<crate::ai::PlayerHealth>,
    mut booms: bevy::prelude::ResMut<crate::explosion::Explosions>,
    player: bevy::prelude::Res<crate::Player>,
    level: bevy::prelude::Res<crate::LevelRes>,
    flags: bevy::prelude::Res<crate::ai::GameFlags>,
) {
    FRAME.fetch_add(1, Ordering::Relaxed);
    // What the mod's screens ask about the map and the story so far (tnmweb's page loader).
    let location = match level.0.level_info().and_then(|a| find(&a.props, "MissionLocation")) {
        Some(Value::Str(s)) => s.as_str(),
        _ => "",
    };
    crate::tnmmode::set_level_place(level.0.mission_number(), location);
    if bevy::prelude::DetectChanges::is_changed(&flags) {
        crate::tnmmode::set_true_flags(flags.0.iter().filter(|(_, v, _)| matches!(v, imsim_core::flags::FlagValue::Bool(true))).map(|(k, _, _)| k.to_ascii_lowercase()).collect());
    }
    if !on() {
        return;
    }
    set_mission(level.0.mission_number());
    set_player(ph.drug_timer > 0.0, crate::abilities::augs_active_count(), ph.hp, 100.0, player.crouched || player.force_duck);
    for r in REWARDS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        if r.heal > 0.0 && !ph.dead {
            ph.heal(r.heal);
        }
        if r.charge > 0.0 {
            ph.energy = (ph.energy + r.charge).min(100.0);
        }
    }
    for (pos, damage, radius) in BLASTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        booms.0.push(crate::explosion::Boom { pos, damage, radius, damage_type: "Exploded", from: None, rings: false, fireball: false, fx: 0, world: false });
    }
}

pub fn save() -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = OWNED.lock().ok().and_then(|g| g.as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())).unwrap_or_default();
    v.sort();
    v
}

pub fn restore(v: &[(String, u64)]) {
    if let Ok(mut g) = OWNED.lock() {
        *g = Some(v.iter().cloned().collect());
    }
}

/// Faithful kills and levels by weapon class (saved with Trestkon).
pub fn save_faith() -> Vec<(String, u32, u32)> {
    let mut v: Vec<(String, u32, u32)> = FAITH.lock().ok().and_then(|g| g.as_ref().map(|m| m.iter().map(|(k, e)| (k.clone(), e.0, e.1)).collect())).unwrap_or_default();
    v.sort();
    v
}

pub fn restore_faith(v: &[(String, u32, u32)]) {
    if let Ok(mut g) = FAITH.lock() {
        *g = Some(v.iter().map(|(k, a, b)| (k.clone(), (*a, *b))).collect());
    }
}

pub fn new_game() {
    if let Ok(mut g) = OWNED.lock() {
        *g = None;
    }
    if let Ok(mut g) = FAITH.lock() {
        *g = None;
    }
    HELD.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_respects_the_list() {
        let mk = |name: &str, index: u32, value: Value| Property { name: name.into(), index, value };
        let d = vec![
            mk("legendaryChance", 0, Value::Float(1.0)),
            mk("possibleAttributes", 0, Value::Byte(att::RAPID)),
            mk("possibleAttributes", 1, Value::Byte(att::LONG)),
        ];
        for _ in 0..50 {
            let m = roll(&d, 20);
            assert!(m != 0);
            assert_eq!(m & !((1 << att::RAPID) | (1 << att::LONG)), 0);
        }
        assert_eq!(roll(&[mk("legendaryChance", 0, Value::Float(1.0))], 20), 0);
    }

    #[test]
    fn handle_rules() {
        assert_eq!(handle_mult(1 << att::ASSASSIN, -1.0, true, false), 2.0);
        assert_eq!(handle_mult(1 << att::ASSASSIN, 1.0, false, false), 1.0);
        assert_eq!(handle_mult(1 << att::VALIANT, 1.0, false, false), 2.0);
        assert_eq!(damage_type(1 << att::PUNCTURING, "Shot"), "Sabot");
        assert_eq!(damage_type(1 << att::PUNCTURING, "Stunned"), "Stunned");
        assert!((pawn_resist(1) - 1.0 / 1.65).abs() < 1e-5);
    }
}
