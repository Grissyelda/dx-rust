//! Mod support: Visible Attachments (the player's own VisibleAttachments.u, enabled on the launcher's Mods page).
//! Its AllWeapons_Mod meshes are the weapons with extra sections for each weapon mod - silencer, laser sight (and
//! the laser's mount), the laser's beam stub, scope (and a scope mount drawn with another texture while a scope is
//! on). Every weapon class uses them for its first-person, third-person and pickup meshes; a section shows while the
//! weapon has that mod (bHasSilencer / bHasLaser / bHasScope, the beam while the laser is on) and is masked out
//! (PinkMaskTex) otherwise. The section numbers come from the package's own MESHMAP SETTEXTURE lines.

use unreal_pkg::props::Value;

const HIDE: &str = "DeusExItems.Skins.PinkMaskTex";

/// Which mesh sections (texture slots) each part is.
#[derive(Clone, Copy, Default)]
pub struct Roles {
    pub silencer: Option<u32>,
    pub laser: &'static [u32],
    pub dot: Option<u32>,
    pub scope: Option<u32>,
    /// A section drawn with another texture while a scope is on: (slot, texture).
    pub scope_mount: Option<(u32, &'static str)>,
    /// Hidden while a silencer is on (the sniper rifle's muzzle brake).
    pub brake: Option<u32>,
}

const fn r(silencer: Option<u32>, laser: &'static [u32], dot: u32, scope: Option<u32>) -> Roles {
    Roles { silencer, laser, dot: Some(dot), scope, scope_mount: None, brake: None }
}

/// (mesh name in VisibleAttachments, roles).
fn table(name: &str) -> Option<Roles> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "assaultshotgun_mod" | "assaultgun_mod" | "glock_mod" => r(Some(4), &[5], 6, Some(7)),
        "assaultshotgun3rd_mod" | "assaultshotgunpickup_mod" | "assaultgun3rd_mod" | "assaultgunpickup_mod" | "glock3rd_mod"
        | "glockpickup_mod" => r(Some(3), &[4], 5, Some(6)),
        "minicrossbow_mod" => r(None, &[4], 5, Some(6)),
        "minicrossbow3rd_mod" | "minicrossbowpickup_mod" => r(None, &[2, 3], 4, Some(5)),
        "gepgun_mod" => Roles { scope_mount: Some((1, "VisibleAttachments.Skins.HeavyWeaponModsPOVTex1")), ..r(None, &[2], 3, Some(4)) },
        "gepgun3rd_mod" | "gepgunpickup_mod" => {
            Roles { scope_mount: Some((2, "VisibleAttachments.Skins.GEPGun3rdTex2")), ..r(None, &[3], 4, Some(5)) }
        }
        "plasmarifle_mod" => r(None, &[3], 4, Some(5)),
        "plasmarifle3rd_mod" | "plasmariflepickup_mod" => r(None, &[2], 3, Some(4)),
        "shotgun_mod" => r(Some(4), &[5], 6, Some(7)),
        "shotgun3rd_mod" | "shotgunpickup_mod" => {
            Roles { scope_mount: Some((3, "VisibleAttachments.Skins.Shotgun3rdTex2")), ..r(Some(4), &[5], 6, Some(7)) }
        }
        // The rifle's scope section is part of the gun (its own texture): always shown.
        "sniperrifle_mod" | "sniperrifle3rd_mod" | "sniperriflepickup_mod" => Roles { brake: Some(3), ..r(Some(4), &[5], 6, None) },
        "stealthpistol_mod" => r(None, &[4], 5, Some(6)),
        "stealthpistol3rd_mod" | "stealthpistolpickup_mod" => r(None, &[2], 3, Some(4)),
        _ => return None,
    })
}

pub fn active() -> bool {
    unreal_pkg::mods::active("visibleattachments").is_some()
}

/// The Visible Attachments version of one of the game's weapon meshes ("DeusExItems.AssaultShotgun3rd" ->
/// "VisibleAttachments.AssaultShotgun3rd_Mod"), when the mod is on and has it.
pub fn mod_mesh(path: &str) -> Option<String> {
    if !active() {
        return None;
    }
    let name = path.rsplit('.').next()?;
    let m = format!("{name}_Mod");
    table(&m)?;
    Some(format!("VisibleAttachments.{m}"))
}

/// What's fitted: silencer, laser, laser on, scope.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Fitted {
    pub silencer: bool,
    pub laser: bool,
    pub lasing: bool,
    pub scope: bool,
}

/// MultiSkins for a Visible Attachments mesh: hidden sections masked out, the scope mount's scope texture.
pub fn skins(mesh_path: &str, f: Fitted) -> Vec<(u32, String)> {
    let Some(roles) = mesh_path.rsplit('.').next().and_then(table) else { return Vec::new() };
    let mut out = Vec::new();
    let mut hide = |slot: Option<u32>, show: bool| {
        if let Some(s) = slot {
            if !show {
                out.push((s, HIDE.to_string()));
            }
        }
    };
    hide(roles.silencer, f.silencer);
    for &l in roles.laser {
        hide(Some(l), f.laser);
    }
    hide(roles.dot, f.laser && f.lasing);
    hide(roles.scope, f.scope);
    hide(roles.brake, !f.silencer);
    if let (Some((slot, tex)), true) = (roles.scope_mount, f.scope) {
        out.push((slot, tex.to_string()));
    }
    out
}

/// `skins`, or None when the mesh isn't one of the mod's.
pub fn skins_for(mesh_path: &str, f: Fitted) -> Option<Vec<(u32, String)>> {
    mesh_path.rsplit('.').next().and_then(table)?;
    Some(skins(mesh_path, f))
}

/// Apply `skins` to a property list (MultiSkins[slot]).
pub fn apply_skins(props: &mut Vec<unreal_pkg::props::Property>, skins: &[(u32, String)]) {
    for (slot, tex) in skins {
        props.retain(|p| !(p.name.eq_ignore_ascii_case("MultiSkins") && p.index == *slot));
        props.push(unreal_pkg::props::Property { name: "MultiSkins".into(), index: *slot, value: Value::Path(tex.clone()) });
    }
}

/// The class override layer ("visibleattachments"): every weapon class whose meshes the mod has uses its meshes;
/// the pickup / third-person mesh shows what the class has fitted by default (the rifle's scope, the stealth
/// pistol's built-in silencer stays the gun's own).
pub fn build_layer(classes: &mut unreal_pkg::classes::ClassDb) {
    let mut list = Vec::new();
    if active() {
        let weapons = [
            "WeaponAssaultShotgun",
            "WeaponAssaultGun",
            "WeaponMiniCrossbow",
            "WeaponGEPGun",
            "WeaponPlasmaRifle",
            "WeaponSawedOffShotgun",
            "WeaponRifle",
            "WeaponStealthPistol",
            "WeaponPistol",
        ];
        for class in weapons {
            let d = classes.base_defaults(class);
            let path = |n: &str| match unreal_pkg::props::find(&d, n) {
                Some(Value::Path(p)) => Some(p.clone()),
                _ => None,
            };
            let flag = |n: &str| matches!(unreal_pkg::props::find(&d, n), Some(Value::Bool(true)));
            let fitted = Fitted { silencer: false, laser: false, lasing: false, scope: flag("bHasScope") && class != "WeaponRifle" };
            for prop in ["PlayerViewMesh", "PickupViewMesh", "ThirdPersonMesh", "Mesh"] {
                if let Some(m) = path(prop).and_then(|p| mod_mesh(&p)) {
                    list.push((class.to_string(), prop.to_string(), 0, Value::Path(m)));
                }
            }
            // MultiSkins for the pickup / third-person layout (they share one).
            if let Some(m) = path("PickupViewMesh").and_then(|p| mod_mesh(&p)) {
                for (slot, tex) in skins(&m, fitted) {
                    list.push((class.to_string(), "MultiSkins".to_string(), slot, Value::Path(tex)));
                }
            }
        }
    }
    if let Ok(mut l) = LAYER.write() {
        *l = list;
    }
    apply();
}

static LAYER: std::sync::RwLock<Vec<(String, String, u32, Value)>> = std::sync::RwLock::new(Vec::new());

/// Set the layer for this map: HDTP's own weapon models (while its HD items are on) carry their mod sections, so
/// those classes keep them.
pub fn apply() {
    let hd = crate::hdtp::weapon_classes();
    let list: Vec<_> = LAYER
        .read()
        .map(|l| l.iter().filter(|(c, ..)| !hd.iter().any(|h| h.eq_ignore_ascii_case(c))).cloned().collect())
        .unwrap_or_default();
    unreal_pkg::classes::set_override_layer("visibleattachments", list);
}
