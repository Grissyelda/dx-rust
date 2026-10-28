//! Per-class overrides of ScriptedPawn behaviour from the pawn subclasses' scripts.
//!
//! * ShieldDamage / ModifyDamage / IgnoreDamageType: MJ12Commando ignores tear and poison gas, takes half from fire
//!   and poison; GuntherHermann and WaltonSimons take nothing from fire, stun and knockout and a tenth from gases,
//!   radiation, shocks and poison; AnnaNavarre takes nothing from stun, knockout and poison; RiotCops shrug off tear
//!   gas and halon (GotoDisabledState: no RubbingEyes).
//! * SpawnCarcass -> Explode: MIB, WIB, AnnaNavarre and GuntherHermann blow up when killed, unless stunned.
//! (Robot skin colours and female cries are applied to the props at load: dx_level::actors::class_begin_play.)

use crate::classtree::is_a;

/// Damage multiplier a class applies before HandleDamage (0 = ignored).
pub fn damage_scale(class: &str, damage_type: &str) -> f32 {
    if is_a(class, "MJ12Commando") {
        return match damage_type {
            "TearGas" | "PoisonGas" => 0.0,
            "Burned" | "Flamed" | "Poison" | "PoisonEffect" => 0.5,
            _ => 1.0,
        };
    }
    if is_a(class, "GuntherHermann") || is_a(class, "WaltonSimons") {
        return match damage_type {
            "Flamed" | "Burned" | "Stunned" | "KnockedOut" => 0.0,
            "TearGas" | "PoisonGas" | "HalonGas" | "Radiation" | "Shocked" | "Poison" | "PoisonEffect" => 0.1,
            _ => 1.0,
        };
    }
    if is_a(class, "AnnaNavarre") {
        return match damage_type {
            "Stunned" | "KnockedOut" | "Poison" | "PoisonEffect" => 0.0,
            _ => 1.0,
        };
    }
    1.0
}

/// RiotCop.GotoDisabledState: tear gas and halon don't make it rub its eyes.
pub fn ignores_gas_reaction(class: &str) -> bool {
    is_a(class, "RiotCop") || is_a(class, "MJ12Commando")
}

/// SpawnCarcass explodes instead of leaving a body (when not stunned).
pub fn explodes_on_death(class: &str) -> bool {
    ["MIB", "WIB", "AnnaNavarre", "GuntherHermann"].iter().any(|c| is_a(class, c))
}

/// MJ12Commando.PlayShoot / TweenToShoot: the wrist-gun pose (Shoot2), never the reload or idle anims.
pub fn shoot_anim(class: &str) -> Option<&'static str> {
    is_a(class, "MJ12Commando").then_some("Shoot2")
}

