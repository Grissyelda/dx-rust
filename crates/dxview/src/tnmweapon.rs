//! The Nameless Mod's weapons (TNMWeapon and TNMItems.u's Weapon* classes): the rules they add to or change in
//! DeusExWeapon. Numbers come from the player's own packages (class defaults); this file holds only behaviour.
//!
//! * `canon`: WeaponTNM<X> is Deus Ex's Weapon<X> rebuilt on TNMWeapon - everything DX RUST does for a Deus Ex weapon
//!   by name (grenades, the GEP's lock, the sawed-off's rack, the flamethrower's sound...) applies to its copy.
//! * WeaponDamageType overrides (`damage_type`): the baton knocks out, the taser slug stuns ('StunnedTaser'), the
//!   fire blade burns out of water and with air, the laser weapons EMP / burn, the Gibinator gibs, the Fat Rifle
//!   fattens, the temporal weapon is 'Temporal', the mini spider bot shocks.
//! * TNMWeapon.TraceFire: a cone weapon loaded with sabot fires one slug, with no ammo class three; an unscoped
//!   scope costs no accuracy.
//! * TNMWeapon.Fire: a weapon that needs air (EnviroEffective air / water) won't fire where there is no oxygen.
//! * TNMWeapon.Tick: the standing bonus builds at 1.2 x real time up to 8 s, not while reloading, falls by
//!   0.02 x speed, and every shot takes (recoil + 2 x skill) x 2 s back off it.
//! * The DaiKatana hurts its wielder 5 a swing; the Moonlight Blade's other mode throws a bolt for a bio cell.

/// The Deus Ex weapon a Nameless Mod weapon acts as, for rules DX RUST keys on the class.
pub fn canon(class: &str) -> &str {
    if !crate::tnmmode::active() {
        return class;
    }
    crate::util::with_lower(class, |l| {
        Some(match l {
            "weapontnmpistol" | "weaponuniquezp" | "weaponuniqueao" | "weaponyaaaarrghpistol" | "weaponpolyflags" => "WeaponPistol",
            "weapontnmstealthpistol" => "WeaponStealthPistol",
            "weapontnmassaultgun" | "weaponuniquekylie" => "WeaponAssaultGun",
            "weapontnmassaultshotgun" => "WeaponAssaultShotgun",
            "weapontnmsawedoffshotgun" => "WeaponSawedOffShotgun",
            "weapontnmrifle" => "WeaponRifle",
            "weapontnmgepgun" => "WeaponGEPGun",
            "weapontnmlaw" => "WeaponLAW",
            "weapontnmflamethrower" => "WeaponFlamethrower",
            "weapontnmplasmarifle" => "WeaponPlasmaRifle",
            "weapontnmminicrossbow" => "WeaponMiniCrossbow",
            "weapontnmpeppergun" => "WeaponPepperGun",
            "weapontnmprod" => "WeaponProd",
            "weapontnmbaton" => "WeaponBaton",
            "weapontnmcombatknife" => "WeaponCombatKnife",
            "weapontnmcrowbar" => "WeaponCrowbar",
            "weapontnmsword" => "WeaponSword",
            "weapontnmnanosword" => "WeaponNanoSword",
            "weapontnmshuriken" | "weaponthrowncredits" => "WeaponShuriken",
            "weapontnmlam" => "WeaponLAM",
            "weapontnmgasgrenade" => "WeaponGasGrenade",
            "weapontnmempgrenade" => "WeaponEMPGrenade",
            "weapontnmnanovirusgrenade" => "WeaponNanoVirusGrenade",
            _ => return None,
        })
    })
    .unwrap_or(class)
}

/// The weapon's own WeaponDamageType for an instant hit, where its class overrides TNMWeapon's. `airless`: fired in
/// water or where there is no oxygen (the fire blade is only a blade there).
pub fn damage_type(class: &str, ammo: Option<&str>, airless: bool) -> Option<&'static str> {
    if !crate::tnmmode::active() {
        return None;
    }
    crate::util::with_lower(class, |l| match l {
        "weapontnmbaton" => Some("KnockedOut"),
        "weapongibinator" => Some("gibbed"),
        "weaponlaser" => Some("EMP"),
        "weapontnmlasergun" => Some("Burned"),
        "weapontemporal" => Some("Temporal"),
        "weaponspiderbotmini" => Some("Shocked"),
        "weaponfatrifle" => Some("Fat"),
        "weaponfireblade" => Some(if airless { "Shot" } else { "Flamed" }),
        // WeaponShotgun: the taser slug.
        "weaponshotgun" if ammo.is_some_and(|a| a.eq_ignore_ascii_case("AmmoSlugTaser")) => Some("StunnedTaser"),
        _ => None,
    })
}

/// TNMWeapon.TraceFire's slug count where it differs from DeusExWeapon's 5: sabot in a cone weapon is one slug, and
/// a cone weapon with no ammo class fires three.
pub fn slugs(cone: bool, instant: bool, ammo: Option<&str>) -> Option<usize> {
    if !crate::tnmmode::active() || !cone || !instant {
        return None;
    }
    match ammo {
        Some(a) if a.eq_ignore_ascii_case("AmmoSabot") => Some(1),
        None => Some(3),
        _ => None,
    }
}

/// TNMWeapon.Fire: EnviroEffective ENVEFF_Air (1), ENVEFF_Water (2) and ENVEFF_AirWater (5) need oxygen.
pub fn needs_air(enviro: u8) -> bool {
    matches!(enviro, 1 | 2 | 5)
}

/// TNMWeapon.GetAIVolume overrides: the taser slug and the rice bag are half as loud.
pub fn noise_mult(class: &str, ammo: Option<&str>) -> f32 {
    if !crate::tnmmode::active() {
        return 1.0;
    }
    let quiet = |a: &str| ammo.is_some_and(|x| x.eq_ignore_ascii_case(a));
    if (class.eq_ignore_ascii_case("WeaponShotgun") && quiet("AmmoSlugTaser")) || (class.eq_ignore_ascii_case("WeaponFatRifle") && quiet("AmmoRicebag")) {
        0.5
    } else {
        1.0
    }
}

/// CalculateAccuracy overrides for the player: the rifle is 0.5 worse unscoped and unlasered.
pub fn accuracy_add(class: &str, zoomed_or_lasing: bool) -> f32 {
    if crate::tnmmode::active() && class.eq_ignore_ascii_case("WeaponTNMRifle") && !zoomed_or_lasing {
        0.5
    } else {
        0.0
    }
}

/// TNMWeapon.Tick's standing timer: (growth per second, cap) while standing still and not reloading, and what a
/// unit of speed takes off it per second.
pub const STAND_GROW: f32 = 1.2;
pub const STAND_CAP: f32 = 8.0;
pub const STAND_MOVE: f32 = 0.02;

/// TNMWeapon.Fire: what a shot takes off the standing timer.
pub fn shot_bloom(recoil: f32, skill: f32) -> f32 {
    (recoil + skill * 2.0).max(0.0) * 2.0
}

/// WeaponDaiKatana.HandToHandAttack: 5 'DaiKatana' to its wielder on every swing.
pub fn self_harm(class: &str) -> f32 {
    if crate::tnmmode::active() && class.eq_ignore_ascii_case("WeaponDaiKatana") {
        5.0
    } else {
        0.0
    }
}

/// Weapons that are gone once fired empty (TNMWeapon NormalFire / the hide-a-gun's Reload: Destroy()).
pub fn single_use(class: &str) -> bool {
    crate::tnmmode::active() && (class.eq_ignore_ascii_case("WeaponTNMHideAGun") || class.eq_ignore_ascii_case("WeaponFireExting"))
}
