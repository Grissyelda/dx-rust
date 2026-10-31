//! The player's skills and augmentations (SkillManager / AugmentationManager), built from the class defaults.
//!
//! The state lives behind a global lock so any system (damage, AI, weapons, swimming...) can ask a skill or aug
//! value without new system parameters, the way the scripts ask `Player.SkillSystem` / `AugmentationSystem`.
//! `abilities_system` handles the F3..F12 hotkeys, bioelectric energy and the augs that tick (Regeneration,
//! Synthetic Heart); the rest of the effects are read where they apply.

use bevy::prelude::*;
use imsim_core::augs::{AugDef, Augs, Give, LocInfo, Toggle};
use imsim_core::skills::{SkillDef, Skills};
use std::sync::{Mutex, MutexGuard};
use unreal_pkg::props::{Property, Value};

pub struct Abilities {
    pub skills: Skills,
    pub augs: Augs,
    /// Strings from the class defaults.
    pub energy_depleted: String,
    pub no_aug_in_slot: String,
    pub location_full: String,
    pub skill_points_award: String,
}

static STATE: Mutex<Option<Abilities>> = Mutex::new(None);

/// The skills and augs (None until the first frame built them).
pub fn lock() -> MutexGuard<'static, Option<Abilities>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// SkillSystem.GetSkillLevelValue (untrained values until the system is built).
pub fn skill(class: &str) -> f32 {
    if let Some(a) = lock().as_ref() {
        return a.skills.value(class);
    }
    match class {
        "SkillLockpicking" | "SkillTech" => 0.1,
        "SkillEnviro" | "SkillMedicine" | "SkillComputer" | "SkillSwimming" => 1.0,
        "SkillWeaponPistol" => -0.1,
        _ => 0.0,
    }
}

/// SkillSystem.GetSkillLevel.
pub fn skill_level(class: &str) -> usize {
    lock().as_ref().map_or(0, |a| a.skills.level(class))
}

/// AugmentationSystem.GetAugLevelValue: None while the aug is missing or off (-1.0 in the scripts).
pub fn aug(class: &str) -> Option<f32> {
    lock().as_ref().and_then(|a| a.augs.level_value(class))
}

/// AugmentationSystem.GetClassLevel: 0..3 while active.
pub fn aug_level(class: &str) -> Option<usize> {
    lock().as_ref().and_then(|a| a.augs.class_level(class))
}

pub fn aug_active(class: &str) -> bool {
    aug(class).is_some()
}

/// Switch an aug off from game code (the spy drone after it detonates).
pub fn deactivate_aug(class: &str) {
    let mut g = lock();
    let Some(a) = g.as_mut() else { return };
    if let Some(i) = a.augs.index(class) {
        if a.augs.list[i].active {
            a.augs.deactivate(i);
            let snd = a.augs.list[i].def.deactivate_sound.clone();
            if !snd.is_empty() {
                crate::sound::play_player(&snd, 1.0, 0.0);
            }
        }
    }
}

/// Skill points awarded by a script or trigger (DeusExPlayer.SkillPointsAdd): "%d skill points awarded".
pub fn award_points(points: i32) -> Option<String> {
    let mut g = lock();
    let a = g.as_mut()?;
    a.skills.award(points);
    Some(a.skill_points_award.replace("%d", &points.to_string()))
}

fn at<'a>(p: &'a [Property], name: &str, idx: u32) -> Option<&'a Value> {
    p.iter()
        .find(|x| x.index == idx && x.name.eq_ignore_ascii_case(name))
        .map(|x| &x.value)
}
fn s(p: &[Property], name: &str) -> String {
    match at(p, name, 0) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}
fn path(p: &[Property], name: &str) -> String {
    match at(p, name, 0) {
        Some(Value::Path(v)) if !v.ends_with(".None") => v.clone(),
        _ => String::new(),
    }
}
fn f(p: &[Property], name: &str, idx: u32) -> f32 {
    match at(p, name, idx) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        _ => 0.0,
    }
}
fn i(p: &[Property], name: &str, idx: u32) -> i32 {
    match at(p, name, idx) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        _ => 0,
    }
}
fn b(p: &[Property], name: &str) -> bool {
    matches!(at(p, name, 0), Some(Value::Bool(true)))
}
fn class_name(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}

/// Build the player's skills and augs from SkillManager.skillClasses / AugmentationManager.augClasses, give the
/// default augs (Light, IFF, Infolink) and the player's starting skill points.
pub fn build(classes: &mut unreal_pkg::classes::ClassDb) -> Abilities {
    let sm = classes.defaults("SkillManager");
    let mut skill_defs = Vec::new();
    for idx in 0..15 {
        let Some(Value::Path(p)) = at(&sm, "skillClasses", idx) else {
            continue;
        };
        let c = class_name(p);
        let d = classes.defaults(&c);
        let needed = path(&d, "itemNeeded");
        skill_defs.push(SkillDef {
            name: s(&d, "SkillName"),
            description: s(&d, "Description"),
            icon: path(&d, "SkillIcon"),
            cost: [i(&d, "Cost", 0), i(&d, "Cost", 1), i(&d, "Cost", 2)],
            values: [
                f(&d, "LevelValues", 0),
                f(&d, "LevelValues", 1),
                f(&d, "LevelValues", 2),
                f(&d, "LevelValues", 3),
            ],
            item_needed: (!needed.is_empty()).then(|| class_name(&needed)),
            automatic: b(&d, "bAutomatic"),
            // SkillWeaponPistol.PreBeginPlay: CurrentLevel = 1 in single player.
            start_level: if c.eq_ignore_ascii_case("SkillWeaponPistol") {
                1
            } else {
                i(&d, "CurrentLevel", 0).max(0) as usize
            },
            class: c,
        });
    }
    let player = classes.defaults("JCDentonMale");
    let points = match at(&player, "SkillPointsAvail", 0) {
        Some(Value::Int(v)) => *v,
        _ => 5000,
    };
    // A new game spends the points in character creation; until that screen exists the default kit is kept and
    // the points stay unspent.
    let skills = Skills::new(skill_defs, points);

    let am = classes.defaults("AugmentationManager");
    let mut locs = [LocInfo::default(); 7];
    for (li, loc) in locs.iter_mut().enumerate() {
        if let Some(Value::Struct(_, raw)) = at(&am, "AugLocs", li as u32) {
            let word = |k: usize| {
                raw.get(k * 4..k * 4 + 4)
                    .map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            *loc = LocInfo {
                slots: word(0).max(0) as usize,
                count: 0,
                key_base: word(2),
            };
        }
    }
    let mut aug_defs = Vec::new();
    for idx in 0..25 {
        let Some(Value::Path(p)) = at(&am, "augClasses", idx) else {
            continue;
        };
        let c = class_name(p);
        let d = classes.defaults(&c);
        aug_defs.push(AugDef {
            name: s(&d, "AugmentationName"),
            description: s(&d, "Description"),
            icon: path(&d, "Icon"),
            small_icon: path(&d, "smallIcon"),
            energy_rate: f(&d, "EnergyRate", 0),
            max_level: i(&d, "MaxLevel", 0).max(0) as usize,
            values: [
                f(&d, "LevelValues", 0),
                f(&d, "LevelValues", 1),
                f(&d, "LevelValues", 2),
                f(&d, "LevelValues", 3),
            ],
            location: i(&d, "AugmentationLocation", 0).clamp(0, 6) as u8,
            always_active: b(&d, "bAlwaysActive"),
            rate_scales: c == "AugCloak" || c == "AugRadarTrans",
            power: c == "AugPower",
            activate_sound: path(&d, "ActivateSound"),
            deactivate_sound: path(&d, "DeactivateSound"),
            loop_sound: path(&d, "LoopSound"),
            class: c,
        });
    }
    let mut augs = Augs::new(aug_defs, locs);
    for idx in 0..3 {
        if let Some(Value::Path(p)) = at(&am, "defaultAugs", idx) {
            augs.give(&class_name(p), false);
        }
    }
    let pick = |k: &str, d: &str| {
        let v = s(&player, k);
        if v.is_empty() {
            d.to_string()
        } else {
            v
        }
    };
    let amv = |k: &str, d: &str| {
        let v = s(&am, k);
        if v.is_empty() {
            d.to_string()
        } else {
            v
        }
    };
    Abilities {
        skills,
        augs,
        energy_depleted: pick("EnergyDepleted", "Bio-electric energy reserves depleted"),
        skill_points_award: pick("SkillPointsAward", "%d skill points awarded"),
        no_aug_in_slot: amv("NoAugInSlot", "There is no augmentation in that slot"),
        location_full: amv(
            "AugLocationFull",
            "You can't add any more augmentations to that location!",
        ),
    }
}

/// Augmentation strings ("%s activated" ...), read once per class.
fn aug_string(
    classes: &mut unreal_pkg::classes::ClassDb,
    class: &str,
    key: &str,
    default: &str,
) -> String {
    let d = classes.defaults(class);
    let v = s(&d, key);
    if v.is_empty() {
        default.to_string()
    } else {
        v
    }
}

/// Give the player an augmentation (an augmentation canister installed by a medical bot, or a cheat): the
/// message GivePlayerAugmentation shows.
pub fn give_aug(
    classes: &mut unreal_pkg::classes::ClassDb,
    class: &str,
    can_upgrade: bool,
) -> Option<String> {
    let mut g = lock();
    let a = g.as_mut()?;
    let out = a.augs.give(class, can_upgrade);
    let aug = a.augs.get(class)?.clone();
    drop(g);
    Some(match out {
        Give::Added => aug_string(classes, class, "AugNowHaveAtLevel", "%s now at level %d")
            .replacen("%s", &aug.def.name, 1)
            .replacen("%d", &(aug.level + 1).to_string(), 1),
        Give::Upgraded(true) => aug_string(classes, class, "AugNowHave", "%s upgraded to level %d")
            .replacen("%s", &aug.def.name, 1)
            .replacen("%d", &(aug.level + 1).to_string(), 1),
        Give::Upgraded(false) => aug_string(
            classes,
            class,
            "AugAlreadyHave",
            "You already have the %s at the maximum level",
        )
        .replacen("%s", &aug.def.name, 1),
        Give::SlotsFull => {
            let g = lock();
            g.as_ref()
                .map_or(String::new(), |a| a.location_full.clone())
        }
        Give::Unknown => String::new(),
    })
}

/// Keys F3..F12 (ActivateAugmentation 0..9).
pub const AUG_KEYS: [KeyCode; 10] = [
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
];

#[derive(Default)]
pub struct Ticks {
    heal: f32,
    heart: f32,
}

/// Sounds and messages for augs switching on/off (Augmentation.Activate/Deactivate).
fn announce(
    classes: &mut unreal_pkg::classes::ClassDb,
    augs: &Augs,
    t: &Toggle,
    msg: &mut crate::movers::HudMessage,
) {
    let (idx, on) = match t {
        Toggle::Activated(i) => (*i, true),
        Toggle::Deactivated(i) => (*i, false),
        _ => return,
    };
    let a = &augs.list[idx];
    let snd = if on {
        &a.def.activate_sound
    } else {
        &a.def.deactivate_sound
    };
    if !snd.is_empty() {
        crate::sound::play_player(snd, 1.0, 0.0);
    }
    // AugCloak plays CloakUp / CloakDown on top.
    if a.def.class == "AugCloak" {
        crate::sound::play_player(
            if on {
                "DeusExSounds.Augmentation.CloakUp"
            } else {
                "DeusExSounds.Augmentation.CloakDown"
            },
            0.85,
            0.0,
        );
    }
    let text = if on {
        aug_string(classes, &a.def.class, "AugActivated", "%s activated")
    } else {
        aug_string(classes, &a.def.class, "AugDeactivated", "%s deactivated")
    };
    msg.text = text.replacen("%s", &a.def.name, 1);
    msg.time_left = 3.0;
}

/// Hotkeys, energy, ticking augs.
#[allow(clippy::too_many_arguments)]
pub fn abilities_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    mut msg: ResMut<crate::movers::HudMessage>,
    mut ticks: Local<Ticks>,
) {
    let _prof = crate::prof::scope("abilities::abilities_system");
    let dt = time.delta_secs();
    if lock().is_none() {
        let mut built = build(&mut assets.classes);
        // A new game: the skills and points chosen in character creation (MenuScreenNewGame.ApplySkills).
        if let Some(ng) = crate::travel::session() {
            for sk in built.skills.list.iter_mut() {
                if let Some((_, l)) = ng.skills.iter().find(|(c, _)| c.eq_ignore_ascii_case(&sk.def.class)) {
                    sk.level = (*l).min(3);
                }
            }
            built.skills.points_avail = ng.points_avail;
            built.skills.points_total = ng.points_total;
        }
        *lock() = Some(built);
        // Developer aid: DXVIEW_AUGS="AugSpeed,AugHealing" installs augs at start (no canisters on the Island).
        if let Ok(list) = std::env::var("DXVIEW_AUGS") {
            for c in list.split(',').map(str::trim).filter(|c| !c.is_empty()) {
                give_aug(&mut assets.classes, c, false);
            }
        }
    }
    let classes = &mut assets.classes;
    let mut g = lock();
    let Some(ab) = g.as_mut() else { return };
    let mut toggles = Vec::new();

    // Developer aid: DXVIEW_FX="flashlight" turns the light on at start (for screenshots).
    if std::env::var("DXVIEW_FX").is_ok_and(|v| v.contains("flashlight"))
        && time.elapsed_secs() < 0.5
    {
        if let Some(i) = ab.augs.index("AugLight") {
            toggles.push(ab.augs.activate(i, ph.energy));
        }
    }

    // DeusExPlayer.ActivateAugmentation: with no energy, the message and the first aug's deactivate sound.
    if !ph.dead && !crate::inventory_screen::screen_open() {
        for (k, key) in AUG_KEYS.iter().enumerate() {
            if !keys.just_pressed(*key) {
                continue;
            }
            if ph.energy <= 0.0 {
                msg.text = ab.energy_depleted.clone();
                msg.time_left = 3.0;
                if let Some(a) = ab.augs.list.first() {
                    crate::sound::play_player(&a.def.deactivate_sound, 1.0, 0.0);
                }
                continue;
            }
            let t = ab.augs.toggle_key(k, ph.energy);
            if t == Toggle::NoAug {
                msg.text = ab.no_aug_in_slot.clone();
                msg.time_left = 3.0;
            }
            toggles.push(t);
        }
    }

    // Energy (DeusExPlayer.Tick -> CalcEnergyUse); dying turns everything off.
    if ph.dead {
        for i in ab.augs.deactivate_all() {
            toggles.push(Toggle::Deactivated(i));
        }
    } else if ph.energy > 0.0 {
        let used = ab.augs.energy_use(dt);
        ph.energy -= used;
        // EMP drain.
        if ph.energy_drain > 0.0 {
            let d = ph.energy_drain_total * dt;
            ph.energy -= d;
            ph.energy_drain -= d;
            if ph.energy_drain <= 0.0 {
                ph.energy_drain = 0.0;
                ph.energy_drain_total = 0.0;
            }
        }
        if ph.energy <= 0.0 {
            if used > 0.0 {
                msg.text = ab.energy_depleted.clone();
                msg.time_left = 3.0;
            }
            ph.energy = 0.0;
            ph.energy_drain = 0.0;
            ph.energy_drain_total = 0.0;
            for i in ab.augs.deactivate_all() {
                toggles.push(Toggle::Deactivated(i));
            }
        }
    }

    // AugHealing: every second heal LevelValue (HealPlayer), off once at full health; a blue ClientFlash.
    if let Some(v) = ab.augs.level_value("AugHealing") {
        ticks.heal += dt;
        if ticks.heal >= 1.0 {
            ticks.heal -= 1.0;
            if ph.hp < 100.0 {
                ph.heal(v.floor());
                ph.flash = ph.flash.max(0.15);
                ph.flash_color = [0.0, 0.0, 0.5];
            } else if let Some(i) = ab.augs.index("AugHealing") {
                toggles.push(ab.augs.deactivate(i));
            }
        }
    } else {
        ticks.heal = 0.0;
    }

    // AugHeartLung: BoostAugs every second while on; off takes the boost back and switches everything off.
    let heart = ab.augs.index("AugHeartLung");
    if let Some(h) = heart {
        if ab.augs.list[h].active {
            ticks.heart -= dt;
            if ticks.heart <= 0.0 {
                ticks.heart = 1.0;
                ab.augs.boost(true, h);
            }
        }
    }
    let heart_off = toggles
        .iter()
        .any(|t| matches!(t, Toggle::Deactivated(i) if Some(*i) == heart));
    if let (true, Some(h)) = (heart_off, heart) {
        ab.augs.boost(false, h);
        for i in ab.augs.deactivate_all() {
            toggles.push(Toggle::Deactivated(i));
        }
        ticks.heart = 0.0;
    }
    // AugMuscle off: a crate too heavy to lift without it is dropped (carry.rs checks every frame).

    // AugDrone: Active builds the drone (unless the last one is still being reconstructed); off ends it.
    let drone_i = ab.augs.index("AugDrone");
    for t in toggles.iter_mut() {
        match *t {
            Toggle::Activated(i) if Some(i) == drone_i => {
                let lv = ab.augs.level_value("AugDrone").unwrap_or(10.0);
                if let Err(left) = crate::augdisplay::drone_activate(lv) {
                    ab.augs.deactivate(i);
                    msg.text = format!("Reconstruction will be complete in {left} seconds");
                    msg.time_left = 3.0;
                    *t = Toggle::NoAug;
                    continue;
                }
            }
            Toggle::Deactivated(i) if Some(i) == drone_i => crate::augdisplay::drone_deactivate(),
            _ => {}
        }
    }
    toggles.retain(|t| *t != Toggle::NoAug || false);
    for t in &toggles {
        announce(classes, &ab.augs, t, &mut msg);
    }
    // Player.AmbientSound = LoopSound while any toggled aug is on (JC's SoundVolume 64, SoundRadius 9).
    if ab.augs.num_active() > 0 {
        if let Some(a) = ab
            .augs
            .list
            .iter()
            .find(|a| a.active && !a.def.always_active)
        {
            crate::sound::hold(
                crate::sound::key_of("aug-loop"),
                &a.def.loop_sound,
                None,
                1.4 * 64.0 / 255.0,
                250.0,
                1.0,
            );
        }
    }
}

/// CalculatePlayerVisibility: the cloak hides the player from people and animals, radar transparency from robots;
/// thermoptic camo from everyone.
/// The debug menu's Invisible cheat (DeusExPlayer.Invisible: bDetectable False).
pub static CHEAT_INVISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn invisible_to(robot: bool, camo: bool) -> bool {
    // A player using a computer is bIgnore (Computers.ChangePlayerVisibility): pawns don't see him.
    camo || CHEAT_INVISIBLE.load(std::sync::atomic::Ordering::Relaxed) || crate::computers::player_ignored() || if robot {
        aug_active("AugRadarTrans")
    } else {
        aug_active("AugCloak")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_from_the_game_files() {
        let Ok(root) = std::env::var("HUD_GAME") else {
            return;
        };
        let mut db =
            unreal_pkg::classes::ClassDb::open(&std::path::Path::new(&root).join("System"));
        let a = super::build(&mut db);
        assert_eq!(a.skills.list.len(), 11);
        assert_eq!(a.skills.level("SkillWeaponPistol"), 1);
        assert_eq!(a.skills.points_avail, 5000);
        let light = a.augs.get("AugLight").unwrap();
        assert!(light.has);
        assert_eq!(light.key(), Some(9), "the flashlight is on F12");
        assert!(a.augs.get("AugIFF").unwrap().active, "always active");
        let speed = a.augs.get("AugSpeed").unwrap();
        assert_eq!(speed.def.location, imsim_core::augs::LOC_LEG);
        assert!(
            speed.def.description.contains("TECH ONE"),
            "description from DeusEx.int"
        );
        assert_eq!(
            a.skills
                .find("SkillLockpicking")
                .unwrap()
                .def
                .item_needed
                .as_deref(),
            Some("Lockpick")
        );
    }
}

/// AugDefense (Aggressive Defense System): every 0.1 s the nearest enemy projectile in flight (not gas, faster than
/// 100) is tracked (GEPGunLock beep) and blown up once it is within LevelValue units (ProdFire); the hum
/// (AugDefenseOn) repeats every 2 s.
pub fn aug_defense_system(
    time: Res<Time>,
    player: Res<crate::Player>,
    mut q: Query<&mut crate::projectiles::Projectile>,
    mut t: Local<(f32, f32)>,
) {
    let _prof = crate::prof::scope("abilities::aug_defense_system");
    let Some(range) = aug("AugDefense") else {
        *t = (0.0, 0.0);
        set_defense_target(None);
        return;
    };
    let dt = time.delta_secs();
    t.0 -= dt;
    t.1 -= dt;
    if t.0 > 0.0 {
        return;
    }
    t.0 = 0.1;
    if t.1 <= 0.0 {
        t.1 = 2.0;
        crate::sound::play_player("DeusExSounds.Augmentation.AugDefenseOn", 1.0, 0.0);
    }
    let me = player.body.location;
    let mut best: Option<(f64, Mut<crate::projectiles::Projectile>)> = None;
    for p in &mut q {
        // FindNearestProjectile: not the player's own, not gas, tracers or greasel / gray spit, faster than 100.
        if p.from.is_none()
            || p.cloud.is_some()
            || matches!(p.class.as_str(), "Tracer" | "GreaselSpit" | "GraySpit")
            || p.stuck
            || p.vel.length() <= 100.0
            || p.life <= 0.0
        {
            continue;
        }
        let d = p.pos.distance(me);
        if best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, p));
        }
    }
    if let Some((d, mut p)) = best {
        crate::sound::play_player("DeusExSounds.Weapons.GEPGunLock", 1.0, 0.0);
        let blown = d < range as f64;
        if blown {
            // bAggressiveExploded: Explode where it is.
            p.life = 0.0;
            crate::sound::play_player("DeusExSounds.Weapons.ProdFire", 1.0, 0.0);
            if let Some(b) = p.from {
                crate::achievements::ads_detonated(b);
            }
        }
        // SetDefenseAugStatus: the aug display marks it ("* ADS Tracking *" / "* ADS Detonating *").
        set_defense_target(Some((p.pos, blown)));
    } else {
        set_defense_target(None);
    }
}

/// AugmentationDisplayWindow.defenseTarget: where the projectile AugDefense tracks is, and whether it's going off.
static DEFENSE: std::sync::Mutex<Option<(bevy::math::DVec3, bool)>> = std::sync::Mutex::new(None);

fn set_defense_target(t: Option<(bevy::math::DVec3, bool)>) {
    if let Ok(mut d) = DEFENSE.lock() {
        *d = t;
    }
}

pub fn defense_target() -> Option<(bevy::math::DVec3, bool)> {
    DEFENSE.lock().ok().and_then(|d| *d)
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = STATE.lock() {
        *v = None;
    }
    set_defense_target(None);
}
