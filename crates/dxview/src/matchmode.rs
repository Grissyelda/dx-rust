//! The match modes behind deathmatch.rs: one trait for what differs between a Classic match (the original
//! multiplayer patch, classicmp.rs) and a Modern one (ours), so the referee, the spawn code, the pickups and the
//! damage path ask the mode instead of branching. New modes (Data Heist, Uplink, Extraction...) implement it too.
//!
//! Classic follows DeusExMPGame / DeathMatchGame / TeamDMGame and DeusExPlayer's multiplayer branches:
//! * RestartPlayer: ResetPlayerToDefaults (everything taken, skills and augmentations rebuilt with the multiplayer
//!   values, energy full), GiveInitialInventory (medkit, lockpick, multitool; the pistol only with
//!   bStartWithPistol, off in the shipped game), SetupAbilities (SkillPointsAvail = SkillsAvail, InitialAugs
//!   augmentations in AugPrefs order at full level when bAugsAllowed), skills at MPSkillStartLevel, and
//!   NintendoImmunityEffect (6 s of protection).
//! * Reward: SkillsPerKill skill points and AugsPerKill augmentations.
//! * TakeDamage in a net game: three regions (GetMPHitLocation: head x2, legs, torso and arms together), the
//!   sniper rifle's mpNoScopeMult without the scope, teammates x fFriendlyFireMult, no difficulty scaling.
//! * Pickups: the map's own, every one; weapons stay where they lie (bWeaponStay), the rest come back after their
//!   RespawnTime; a weapon taken brings mpPickupAmmoCount x 3 rounds.
//! * Death: MultiplayerMessageWin ("You were killed by ...", 3 s, then Fire to continue).
//!
//! Modern keeps what deathmatch.rs always did (its damage table, balanced pickup lists, timed respawns).

use crate::deathmatch::{self, Family};
use bevy::math::DVec3;

pub trait MatchMode: Sync {
    fn family(&self) -> Family;
    /// What keeps this mode from being hosted here (a mod it needs), if anything.
    fn blocked(&self) -> Option<&'static str>;
    /// Every pickup the map places is in play (else only the map list's `keep`).
    fn keeps_all_pickups(&self) -> bool;
    /// A weapon of this class stays where it lies when taken (the taker gets a copy).
    fn weapon_stays(&self, class: &str) -> bool;
    /// Seconds before a taken pickup of this class comes back (None: it isn't a match pickup).
    fn respawn_secs(&self, class: &str) -> Option<f32>;
    /// Rounds a weapon pickup brings in this mode (None: the pickup's own).
    fn pickup_ammo(&self, class: &str) -> Option<u32>;
    /// RestartPlayer: a fresh inventory, skills and augmentations.
    fn restart_player(&self, inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb);
    /// Reward: what a kill brings the killer.
    fn reward_kill(&self);
    /// Seconds dead before getting up by itself (None: Fire, after the death screen's lockout).
    fn respawn_delay(&self) -> Option<f32>;
    /// Seconds of protection after a (re)spawn.
    fn spawn_shield(&self) -> f32;
    /// The player's three-region multiplayer body (TakeDamage's net-game branch).
    fn mp_regions(&self) -> bool;
    /// A hit on this player from another player's game: (damage, offset) for PlayerHealth::damage.
    #[allow(clippy::too_many_arguments)]
    fn player_damage(&self, weapon: &str, kind: &str, raw: f32, local: Option<DVec3>, h: f64, r: f64, scoped: bool, yaw: f64) -> (f32, DVec3);
    /// NotifyGameStatus's "N kills to go" at a fifth of the limit (the original only from 10 kills).
    fn fifth_notice(&self, frag_limit: u32) -> bool;
}

pub struct Modern;
pub struct Classic;

static MODERN: Modern = Modern;
static CLASSIC: Classic = Classic;

/// The room's mode (Modern when there is none).
pub fn current() -> &'static dyn MatchMode {
    if deathmatch::classic() {
        &CLASSIC
    } else {
        &MODERN
    }
}

/// A family's mode.
pub fn of(f: Family) -> &'static dyn MatchMode {
    match f {
        Family::Classic => &CLASSIC,
        Family::Modern => &MODERN,
    }
}

impl MatchMode for Modern {
    fn family(&self) -> Family {
        Family::Modern
    }
    fn blocked(&self) -> Option<&'static str> {
        crate::dmchar::blocked()
    }
    fn keeps_all_pickups(&self) -> bool {
        false
    }
    fn weapon_stays(&self, _class: &str) -> bool {
        false
    }
    fn respawn_secs(&self, class: &str) -> Option<f32> {
        deathmatch::respawn_secs(class)
    }
    fn pickup_ammo(&self, _class: &str) -> Option<u32> {
        None
    }
    fn restart_player(&self, inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb) {
        deathmatch::reset_player(inv, classes);
    }
    fn reward_kill(&self) {
        deathmatch::reward_kill();
    }
    fn respawn_delay(&self) -> Option<f32> {
        Some(crate::netplay::RESPAWN_SECS)
    }
    fn spawn_shield(&self) -> f32 {
        0.0
    }
    fn mp_regions(&self) -> bool {
        false
    }
    fn player_damage(&self, weapon: &str, kind: &str, raw: f32, local: Option<DVec3>, h: f64, r: f64, _scoped: bool, _yaw: f64) -> (f32, DVec3) {
        deathmatch::player_damage(weapon, kind, raw, local, h, r)
    }
    fn fifth_notice(&self, frag_limit: u32) -> bool {
        frag_limit >= 2
    }
}

/// Classic: the original's GiveInitialInventory (bStartWithPistol is off in the shipped game).
const START_KIT: [&str; 3] = ["MedKit", "Lockpick", "Multitool"];

impl MatchMode for Classic {
    fn family(&self) -> Family {
        Family::Classic
    }
    fn blocked(&self) -> Option<&'static str> {
        None
    }
    fn keeps_all_pickups(&self) -> bool {
        true
    }
    fn weapon_stays(&self, class: &str) -> bool {
        crate::classicmp::weapon_stays(class)
    }
    fn respawn_secs(&self, class: &str) -> Option<f32> {
        // Inventory.RespawnTime (0 = never).
        match crate::classicmp::class_value(class, "RespawnTime") {
            Some(unreal_pkg::props::Value::Float(t)) if t > 0.0 => Some(t),
            _ => None,
        }
    }
    fn pickup_ammo(&self, class: &str) -> Option<u32> {
        // DeusExWeapon.PostBeginPlay: PickupAmmoCount x 3 in a net game (bNeedToSetMPPickupAmmo); after a pickup
        // it is reset to Default.mpPickupAmmoCount x 3 (HandleItemPickup).
        use unreal_pkg::props::Value;
        let need = matches!(crate::classicmp::class_value(class, "bNeedToSetMPPickupAmmo"), Some(Value::Bool(true)));
        let mp = match crate::classicmp::class_value(class, "mpPickupAmmoCount") {
            Some(Value::Int(n)) if n > 0 => Some(n),
            _ => None,
        };
        let base = match crate::classicmp::class_value(class, "PickupAmmoCount") {
            Some(Value::Int(n)) => n.max(0),
            _ => return None,
        };
        let n = mp.unwrap_or(base);
        Some(if need { n * 3 } else { n } as u32)
    }
    fn restart_player(&self, inv: &mut imsim_core::inventory::Inventory, classes: &mut unreal_pkg::classes::ClassDb) {
        let r = deathmatch::rules().map(|r| r.classic).unwrap_or_default();
        *inv = imsim_core::inventory::Inventory::default();
        for class in START_KIT {
            let p = classes.defaults(class);
            if p.is_empty() {
                continue;
            }
            let def = crate::pickups::item_def(class, &p);
            let _ = inv.pick_up(&def, 1);
        }
        // ResetSkills / CreateSkills / ResetAugmentations / CreateAugmentations / AddDefaultAugmentations: built
        // again from the class defaults, which carry the multiplayer values now.
        let mut built = crate::abilities::build(classes);
        for s in built.skills.list.iter_mut() {
            s.level = (r.skill_level.clamp(0, 3)) as usize;
        }
        built.skills.points_avail = r.skills_avail.max(0);
        built.skills.points_total = r.skills_avail.max(0);
        *crate::abilities::lock() = Some(built);
        if r.augs_allowed {
            grant_augs(r.initial_augs.max(0) as u32);
        }
    }
    fn reward_kill(&self) {
        let r = deathmatch::rules().map(|r| r.classic).unwrap_or_default();
        if r.skills_per_kill > 0 {
            if let Some(msg) = crate::abilities::award_points(r.skills_per_kill) {
                crate::classic_hud::client_message(&msg);
                crate::sound::play_player("DeusExSounds.UserInterface.LogSkillPoints", 1.0, 0.0);
            }
        }
        if r.augs_allowed && r.augs_per_kill > 0 {
            grant_augs(r.augs_per_kill as u32);
        }
    }
    fn respawn_delay(&self) -> Option<f32> {
        None
    }
    fn spawn_shield(&self) -> f32 {
        // DeusExPlayer.NintendoDelay.
        6.0
    }
    fn mp_regions(&self) -> bool {
        true
    }
    fn player_damage(&self, weapon: &str, _kind: &str, raw: f32, local: Option<DVec3>, h: f64, r: f64, scoped: bool, yaw: f64) -> (f32, DVec3) {
        // The sniper rifle without its scope (mpNoScopeMult).
        let mut d = raw;
        if weapon.eq_ignore_ascii_case("WeaponRifle") && !scoped {
            if let Some(unreal_pkg::props::Value::Float(m)) = crate::classicmp::class_value("WeaponRifle", "mpNoScopeMult") {
                d *= m;
            }
        }
        // The hit in the player's own frame (X forward, Y right, Z up).
        let off = match local {
            Some(l) => {
                let (s, c) = yaw.sin_cos();
                DVec3::new(l.x * c + l.y * s, -l.x * s + l.y * c, l.z)
            }
            None => DVec3::new(0.0, 0.0, h * 0.4),
        };
        let _ = r;
        (d, off)
    }
    fn fifth_notice(&self, frag_limit: u32) -> bool {
        frag_limit >= 10
    }
}

/// DeusExPlayer.GrantAugs: through AugPrefs in order, every one not yet installed is given at its highest level and
/// counts, whether or not its slot had room (as the original's loop does).
pub fn grant_augs(n: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut left = n;
    let mut g = crate::abilities::lock();
    let Some(a) = g.as_mut() else { return out };
    for pref in crate::classicmp::aug_prefs() {
        if left == 0 {
            break;
        }
        let Some(i) = a.augs.index(&pref) else { continue };
        if a.augs.list[i].has {
            continue;
        }
        if a.augs.give(&pref, false) == imsim_core::augs::Give::Added {
            let aug = &mut a.augs.list[i];
            aug.level = aug.def.max_level;
            out.push(aug.def.name.clone());
        }
        left -= 1;
    }
    out
}
