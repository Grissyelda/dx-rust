//! The Nameless Mod's player, Trestkon: the systems his class adds on top of DeusExPlayer.
//!
//! - **Fists and gloves** (WeaponFists): bare hands are always with him, off the belt; a left click with empty hands
//!   raises them once the hands have been empty a quarter second. Four kinds of glove are found in the world
//!   (FistPickupLethal / Shock / Stun / Ban); Change Ammo cycles through the ones he has and his Fists skill allows
//!   (lethal at Trained, EMP at Advanced, stun and ban at Master). Each changes the blow: bare = knock-out, lethal =
//!   x1.5 'Shot', EMP = 'FistEMP' (robots take 2.5x as EMP on top of a knock-out blow, devices are disrupted),
//!   stun = half damage 'FistStunned' (one point, stunned for 4 s), ban = x3 'Banned' (gone). A punch at a button or
//!   switch presses it.
//! - **Molotov cocktails** (MakeMolotov): using chemicals with an empty-able wine bottle in the inventory and at least
//!   Trained Demolition makes one - Master needs 2 chemicals, Advanced 3, Trained 4.
//! - **Credits earned** (CreditsEarned): every rise in his credits counts, for the Mission 20 subway-pass way out.
//! - **The new-game kit** (ResetPlayer): his fists, the downtown map, the key to his locker at Raving's and the
//!   note with his account PIN, all read from the mod.
//! - **Radish Adventure** (platform / RadishHUD / PickedUpRadish): see `radish.rs`.
//!
//! State is global (like `abilities`) and saved in the save's Global part.

use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

/// Glove kinds, as WeaponFists.EFistType: normal, spiked (lethal), shock (EMP), stun, ban.
pub const NORMAL: u8 = 0;

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Fists {
    /// bhasspiked / bhasshock / bhasstun / bhasban (index 1..4; 0 = bare hands, always).
    pub has: [bool; 5],
    /// fisttype.
    pub current: u8,
}

/// What travels with Trestkon (the save's Global part).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TrestkonSave {
    pub fists: Fists,
    pub credits_earned: i64,
    /// Radish Adventure: lives, hits left (radishHealth) and radishes picked up.
    pub lives: i32,
    pub radish_health: i32,
    pub radishes: i32,
    /// The difficulty chosen in the mod's menus (tnmmode::Difficulty).
    #[serde(default)]
    pub difficulty: Option<crate::tnmmode::Difficulty>,
}

struct State {
    fists: Fists,
    credits_earned: i64,
    credits_prev: Option<u32>,
    /// Seconds the hands have been empty (LastInHandTimer counted the other way).
    empty_for: f32,
}

static STATE: Mutex<State> = Mutex::new(State {
    fists: Fists { has: [true, false, false, false, false], current: 0 },
    credits_earned: 0,
    credits_prev: None,
    empty_for: 0.0,
});

/// The view model has to be rebuilt (the glove changed).
static REBUILD: AtomicBool = AtomicBool::new(false);
/// A punch landed while a button / switch was the frob target: press it (WeaponFists.ProcessTraceHit -> DoFrob).
static FIST_PRESS: AtomicBool = AtomicBool::new(false);
/// A new game's kit is still to be handed out (once the conversation system is up).
static KIT_PENDING: AtomicBool = AtomicBool::new(false);
/// Inventory requests from item use (chemicals): made by kit_system, which owns the inventory.
static MOLOTOV_REQ: AtomicBool = AtomicBool::new(false);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn active() -> bool {
    crate::tnmmode::active()
}

// ------------------------------------------------------------------------------------------------ fists

/// The glove kind worn now.
pub fn fist_type() -> u8 {
    lock().fists.current
}

/// Trestkon.CanSelectFist: bare hands always; lethal at Fists level 1, EMP at 2, stun and ban at 3.
pub fn can_select(ft: u8) -> bool {
    let lvl = crate::abilities::skill_level("SkillWeaponFists");
    match ft {
        0 => true,
        1 => lvl >= 1,
        2 => lvl >= 2,
        _ => lvl >= 3,
    }
}

/// WeaponFists.givefisttype (a glove pickup used): that kind is his for good.
pub fn give_fist(ft: u8) {
    if (1..=4).contains(&ft) {
        let mut g = lock();
        g.fists.has[ft as usize] = true;
        let all = g.fists.has[1] && g.fists.has[2] && g.fists.has[3];
        drop(g);
        println!("TNM gloves: got kind {ft}");
        if all {
            crate::tnmach::unlock("achGloves");
        }
    }
}

/// WeaponFists.CycleAmmo: the next kind he has and may use, back to bare hands after the last. Returns the kind
/// equipped (its strFistEquipped line is the caller's to show), or None when nothing else can be picked.
pub fn cycle_fists() -> Option<u8> {
    let mut g = lock();
    let start = g.fists.current;
    let mut i = start;
    loop {
        i = if i >= 4 { 0 } else { i + 1 };
        if i == start {
            return None;
        }
        if g.fists.has[i as usize] && can_select(i) {
            g.fists.current = i;
            drop(g);
            REBUILD.store(true, Ordering::Relaxed);
            return Some(i);
        }
    }
}

/// SetFistType's mesh for the glove worn (TNM.WeaponFists / ...Spike / ...Shock / ...Stun / ...Ban).
pub fn fist_mesh() -> &'static str {
    match fist_type() {
        1 => "TNM.WeaponFistsSpike",
        2 => "TNM.WeaponFistsShock",
        3 => "TNM.WeaponFistsStun",
        4 => "TNM.WeaponFistsBan",
        _ => "TNM.WeaponFists",
    }
}

/// HandToHandAttack's damage and WeaponDamageType for the glove worn: (damage multiplier, damage type).
pub fn fist_hit() -> (f32, &'static str) {
    match fist_type() {
        1 => (1.5, "Shot"),
        2 => (1.0, "FistEMP"),
        3 => (0.5, "FistStunned"),
        4 => (3.0, "Banned"),
        _ => (1.0, "KnockedOut"),
    }
}

/// Whether bPenetrating is on for the glove worn (blood on a hit): bare and lethal yes, the others no.
pub fn fist_penetrating() -> bool {
    matches!(fist_type(), 0 | 1)
}

/// The view model must be built again (the glove changed); taken once.
pub fn take_rebuild() -> bool {
    REBUILD.swap(false, Ordering::Relaxed)
}

/// Classes a punch presses (WeaponFists.ProcessTraceHit's list).
pub fn punch_presses(class: &str) -> bool {
    ["Button1", "Switch1", "Switch2", "LightSwitch", "TNMRedButton", "TNMGreyButton", "WCButton"]
        .iter()
        .any(|c| class.eq_ignore_ascii_case(c) || crate::classtree::is_a(class, c))
}

/// A punch went in: press the button / switch in front of the player, if that's what it is.
pub fn note_punch() {
    FIST_PRESS.store(true, Ordering::Relaxed);
}

pub fn take_fist_press() -> bool {
    FIST_PRESS.swap(false, Ordering::Relaxed)
}

/// Hands empty for `dt` more (or not): Trestkon.ParseLeftClick raises the fists only once LastInHandTimer ran out.
pub fn note_hands(empty: bool, dt: f32) -> bool {
    let mut g = lock();
    if empty {
        g.empty_for += dt;
    } else {
        g.empty_for = 0.0;
    }
    g.empty_for >= 0.25
}

/// The glove pickups (FistPickup*.Activated): the kind they give.
pub fn glove_pickup(class: &str) -> Option<u8> {
    if !active() {
        return None;
    }
    let is = |c: &str| class.eq_ignore_ascii_case(c) || crate::classtree::is_a(class, c);
    if is("FistPickupLethal") {
        Some(1)
    } else if is("FistPickupShock") {
        Some(2)
    } else if is("FistPickupStun") {
        Some(3)
    } else if is("FistPickupBan") {
        Some(4)
    } else {
        None
    }
}

// ------------------------------------------------------------------------------------------------ Molotovs

/// The Nameless Mod's chemicals (ChemicalBottle).
pub fn is_chemicals(class: &str) -> bool {
    active() && (class.eq_ignore_ascii_case("ChemicalBottle") || crate::classtree::is_a(class, "ChemicalBottle"))
}

/// ChemicalBottle used: make as many Molotovs as the chemicals and bottles allow... one per use, as MakeMolotov.
pub fn request_molotov() {
    MOLOTOV_REQ.store(true, Ordering::Relaxed);
}

/// Chemicals MakeMolotov needs at the player's Demolition level (None = untrained: he can't).
pub fn molotov_cost() -> Option<u32> {
    match crate::abilities::skill_level("SkillDemolition") {
        0 => None,
        1 => Some(4),
        2 => Some(3),
        _ => Some(2),
    }
}

/// The inventory screen's drag craft: dragging `a` onto `b`, one chemicals and the other a wine bottle, with a
/// Molotov makeable now. Returns the chemicals' class.
pub fn molotov_pair(inv: &imsim_core::inventory::Inventory, a: &str, b: &str) -> Option<String> {
    let bottle = |c: &str| c.eq_ignore_ascii_case("WineBottle") || crate::classtree::is_a(c, "WineBottle");
    let chem = if is_chemicals(a) && bottle(b) {
        a
    } else if is_chemicals(b) && bottle(a) {
        b
    } else {
        return None;
    };
    can_make_molotov(inv, chem).then(|| chem.to_string())
}

/// MakeMolotov from the inventory screen.
pub fn craft_molotov(inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb, chem: &str) -> String {
    make_molotov(inv, classes, chem)
}

/// Whether a Molotov can be made with these chemicals now (CanMakeMolotov), for the inventory screen.
pub fn can_make_molotov(inv: &imsim_core::inventory::Inventory, chem_class: &str) -> bool {
    let Some(need) = molotov_cost() else { return false };
    let chems = inv.items.iter().filter(|i| i.def.class == chem_class).map(|i| i.count.max(1)).sum::<u32>();
    chems >= need && wine_bottle(inv).is_some()
}

fn wine_bottle(inv: &imsim_core::inventory::Inventory) -> Option<String> {
    inv.items
        .iter()
        .find(|i| i.def.class.eq_ignore_ascii_case("WineBottle") || crate::classtree::is_a(&i.def.class, "WineBottle"))
        .map(|i| i.def.class.clone())
}

/// MakeMolotov: the message to show (or None when it worked and the pickup line says it).
fn make_molotov(
    inv: &mut imsim_core::inventory::Inventory,
    classes: &mut unreal_pkg::classes::ClassDb,
    chem_class: &str,
) -> String {
    let t = classes.defaults(crate::tnmmode::player_class());
    let s = |k: &str, d: &str| match find(&t, k) {
        Some(Value::Str(v)) => v.clone(),
        _ => d.to_string(),
    };
    let Some(need) = molotov_cost() else { return s("strNoMolotovSkill", "You need Demolition training for that.") };
    let chems: u32 = inv.items.iter().filter(|i| i.def.class == chem_class).map(|i| i.count.max(1)).sum();
    if chems < need {
        return s("strNoMolotovMaterials", "Not enough chemicals.");
    }
    let Some(bottle) = wine_bottle(inv) else { return s("strNoMolotovWine", "You need a wine bottle.") };
    let d = classes.defaults("WeaponMolBottle");
    if d.is_empty() {
        return String::new();
    }
    inv.use_one(&bottle);
    for _ in 0..need {
        inv.use_one(chem_class);
    }
    let def = crate::pickups::item_def("WeaponMolBottle", &d);
    let msg = inv.pick_up(&def, 1).unwrap_or_default();
    crate::pickups::add_weapon_ammo(inv, &d);
    println!("TNM molotov made ({need} chemicals)");
    crate::tnmach::unlock("achMolotov");
    msg
}

// ------------------------------------------------------------------------------------------------ credits

/// Trestkon.CreditsEarned: every credit that came in this game.
pub fn credits_earned() -> i64 {
    lock().credits_earned
}

// ------------------------------------------------------------------------------------------------ game / save

/// ResetPlayer for a new game: bare hands only, no credits earned, the kit to come (not on the training map).
pub fn new_game(training: bool) {
    let mut g = lock();
    g.fists = Fists { has: [true, false, false, false, false], current: NORMAL };
    g.credits_earned = 0;
    g.credits_prev = None;
    drop(g);
    crate::radish::new_game();
    crate::tnmmode::start_difficulty();
    KIT_PENDING.store(active() && !training, Ordering::Relaxed);
}

pub fn save() -> TrestkonSave {
    let g = lock();
    let (lives, radish_health, radishes) = crate::radish::save();
    TrestkonSave { fists: g.fists, credits_earned: g.credits_earned, lives, radish_health, radishes, difficulty: crate::tnmmode::difficulty() }
}

pub fn restore(s: &TrestkonSave) {
    let mut g = lock();
    g.fists = s.fists;
    g.fists.has[0] = true;
    g.credits_earned = s.credits_earned;
    g.credits_prev = None;
    drop(g);
    crate::radish::restore(s.lives, s.radish_health, s.radishes);
    crate::tnmmode::restore_difficulty(s.difficulty);
    REBUILD.store(true, Ordering::Relaxed);
}

/// Every frame in The Nameless Mod: credits earned, the new-game kit, Molotovs asked for, fists always carried.
pub fn kit_system(
    mut inv: ResMut<crate::pickups::PlayerInventory>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut hud: ResMut<crate::movers::HudMessage>,
) {
    if !active() {
        return;
    }
    crate::tnmach::load(&mut assets.classes);
    // TrestkonTick: credits_diff = Credits - CreditsPrev, counted when it rose.
    {
        let mut g = lock();
        let now = inv.0.credits;
        if let Some(prev) = g.credits_prev {
            if now > prev {
                g.credits_earned += (now - prev) as i64;
            }
        }
        g.credits_prev = Some(now);
    }
    // PostBeginPlay's failsafe: he always has his fists (off the belt).
    if !inv.0.items.iter().any(|i| i.def.class == "WeaponFists") {
        let d = assets.classes.defaults("WeaponFists");
        if !d.is_empty() {
            let def = crate::pickups::item_def("WeaponFists", &d);
            let _ = inv.0.pick_up(&def, 1);
        }
    }
    if let Some(idx) = inv.0.items.iter().position(|i| i.def.class == "WeaponFists") {
        for b in inv.0.belt.iter_mut() {
            if *b == Some(idx) {
                *b = None;
            }
        }
    }
    if MOLOTOV_REQ.swap(false, Ordering::Relaxed) {
        let chem = inv.0.items.iter().find(|i| is_chemicals(&i.def.class)).map(|i| i.def.class.clone());
        if let Some(chem) = chem {
            let msg = make_molotov(&mut inv.0, &mut assets.classes, &chem);
            if !msg.is_empty() {
                hud.text = msg;
                hud.time_left = 3.0;
            }
        }
    }
    if KIT_PENDING.load(Ordering::Relaxed) && crate::consys::ready() {
        KIT_PENDING.store(false, Ordering::Relaxed);
        let t = assets.classes.defaults(crate::tnmmode::player_class());
        let s = |k: &str| match find(&t, k) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        // The downtown map in the DataVault, the key to his locker, and the PIN note.
        crate::consys::add_image("TNMDV_DowntownMap");
        let _ = crate::keys::give("TrestkonsRavingLocker", &s("strLockerPassword"));
        let pin = s("strAccountPin");
        if !pin.is_empty() {
            crate::consys::add_vault_note(&pin);
        }
        println!("TNM new-game kit given");
    }
}
