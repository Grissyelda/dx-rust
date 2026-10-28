//! Augmentation canisters (AugmentationCannister): each holds a choice of two augmentations (AddAugs[0..1]) that a
//! medical bot installs one of (HUDMedBotAddAugsScreen). They don't stack (bCanHaveMultipleCopies False): every
//! canister is its own inventory item in its own slot, and carries its two augmentations in `Item::data`.

use crate::pickups::PlayerInventory;

pub const CLASS: &str = "AugmentationCannister";

/// The canister pairs placed in the original's maps (every AugmentationCannister's AddAugs across the game).
pub const VARIANTS: [[&str; 2]; 9] = [
    ["AugAqualung", "AugEnviro"],
    ["AugCloak", "AugRadarTrans"],
    ["AugDefense", "AugDrone"],
    ["AugEMP", "AugBallistic"],
    ["AugHealing", "AugShield"],
    ["AugMuscle", "AugCombat"],
    ["AugPower", "AugHeartLung"],
    ["AugSpeed", "AugStealth"],
    ["AugTarget", "AugVision"],
];

/// The canisters carried, in inventory order: what each holds.
pub fn list(inv: &PlayerInventory) -> Vec<[String; 2]> {
    inv.0
        .items
        .iter()
        .filter(|i| i.def.class == CLASS)
        .map(|i| {
            let g = |k: usize| i.data.get(k).cloned().unwrap_or_default();
            [g(0), g(1)]
        })
        .collect()
}

/// The AddAugs of a carried canister by its item key (to drop it with its contents).
pub fn contents(inv: &PlayerInventory, key: &str) -> Option<[String; 2]> {
    let it = inv.0.items.iter().find(|i| i.key() == key && i.def.class == CLASS)?;
    let g = |k: usize| it.data.get(k).cloned().unwrap_or_default();
    Some([g(0), g(1)])
}

/// Pick a canister up into its own slot. Err with the "no room" message if it doesn't fit.
pub fn add(inv: &mut PlayerInventory, def: &imsim_core::inventory::ItemDef, augs: [String; 2]) -> Result<String, String> {
    match inv.0.add_new_at(def, 1) {
        Some(idx) => {
            inv.0.items[idx].data = augs.to_vec();
            Ok(format!("{} {} {}", def.pickup_message, def.article, def.name))
        }
        None => Err(imsim_core::inventory::no_room(def)),
    }
}

/// The `idx`-th canister (in list order) used up by the medical bot (DeleteInventory).
pub fn remove(inv: &mut PlayerInventory, idx: usize) {
    let at = inv
        .0
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.def.class == CLASS)
        .nth(idx)
        .map(|(k, _)| k);
    if let Some(k) = at {
        inv.0.remove(k);
    }
}
