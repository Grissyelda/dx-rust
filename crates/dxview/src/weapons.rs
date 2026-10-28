//! Held weapons: first-person view model (drawn by its own camera so it never clips into walls), the
//! select/idle/shoot/reload/holster cycle, and instant-hit firing with Deus Ex's spread and damage.

use crate::actors::{Blockers, World};
use crate::movers::{HudMessage, Movers};
use crate::pickups::PlayerInventory;
use crate::{FlyCam, LevelRes, Player, SkyCam};
use bevy::{
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, Exposure},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::{CursorGrabMode, CursorOptions},
};
use dx_level::actors::{anim_mesh, light_part, load_mesh, mesh_parts, AnimMesh, MeshPart};
use dx_level::{bevy_to_ue, SurfaceClass, METERS_PER_UNIT};
use glam::DVec3;
use imsim_core::inventory::ItemKind;
use imsim_core::movement::CollisionWorld;
use std::collections::HashMap;
use std::sync::Arc;
use unreal_pkg::classes::ClassDb;
use unreal_pkg::mesh::LodMesh;
use unreal_pkg::props::{find, Property, Value};

/// Layer the view model and its camera use.
pub const VIEW_LAYER: usize = 2;
/// Deus Ex's default field of view (horizontal degrees).
pub const DEFAULT_FOV: f32 = 75.0;

#[derive(Component)]
pub struct ViewModelCam;
#[derive(Component)]
pub struct ViewModelRoot;

#[derive(Debug, Clone)]
pub struct WeaponStats {
    /// Weapon ShakeTime, ShakeMag, ShakeVert (the kick on firing).
    pub shake: [f32; 3],
    pub class: String,
    pub name: String,
    pub hit_damage: f32,
    pub shot_time: f32,
    pub reload_count: u32,
    pub reload_time: f32,
    pub base_accuracy: f32,
    pub max_range: f32,
    pub hand_to_hand: bool,
    pub ammo_class: Option<String>,
    pub view_offset: [f32; 3],
    pub view_scale: f32,
    pub view_mesh: String,
    /// Mesh drawn in a pawn's hand, and its scale.
    pub third_mesh: Option<String>,
    pub third_scale: f32,
    /// Weapons of Mass >= 30 are held in two hands (ScriptedPawn.HasTwoHandedWeapon).
    pub mass: f32,
    /// Player skill value for this weapon's GoverningSkill (negative is better), refreshed every frame.
    pub skill: f32,
    /// GoverningSkill class (SkillWeaponPistol...).
    pub governing_skill: Option<String>,
    /// How loud firing is to AIs (DeusExWeapon.NoiseLevel; GetAIVolume radius = 800 x this).
    pub noise_level: f32,
    pub silenced: bool,
    pub has_scope: bool,
    /// ScopeFOV: the field of view through the scope.
    pub scope_fov: f32,
    /// Instant-hit weapons with a StunDuration do 'Stunned' damage (WeaponDamageType).
    pub stun: bool,
    /// bInstantHit; otherwise the weapon fires `projectile` (by ammo: AmmoNames[i] fires ProjectileNames[i]).
    pub instant: bool,
    pub projectile: Option<String>,
    pub ammo_names: Vec<String>,
    pub projectile_names: Vec<String>,
    /// bHasMuzzleFlash: mesh texture slot 2 (normally BlackMaskTex) shows FlatFXTex34/37 for 0.1 s per shot.
    pub muzzle_flash: bool,
    /// bAutomatic: keeps firing while the trigger is held.
    pub automatic: bool,
    /// bPenetrating: draws blood from pawns it hits.
    pub penetrating: bool,
    /// Not a weapon: a pickup held in hand (medkit, food, tools, binoculars).
    pub item: bool,
    /// AreaOfEffect: 1 = AOE_Cone (shotguns fire 5 slugs; cone projectile weapons fire 3), 2 = sphere.
    pub area_of_effect: u8,
    /// AccurateRange and the AI's preferred range limits (GetWeaponBestRange).
    pub accurate_range: f32,
    pub ai_min_range: f32,
    pub ai_max_range: f32,
    /// bFallbackWeapon: only used by pawns when nothing else will do (knives, crowbars).
    pub fallback: bool,
    /// recoilStrength: view kick while the Shoot animation plays.
    pub recoil: f32,
    /// Which weapon mods fit (bCanHaveModBaseAccuracy, ReloadCount, AccurateRange, ReloadTime, RecoilStrength,
    /// bCanHaveScope, Silencer, Laser).
    pub can_mod: [bool; 8],
    pub has_laser: bool,
    /// bCanTrack (GEP gun): hold the crosshair on a pawn for LockTime to lock on.
    pub can_track: bool,
    pub lock_time: f32,
    /// AIFireDelay: once a pawn fires this weapon it drops out of SwitchToBestWeapon (fallback level 0) until the
    /// pawn's FireTimer runs out (WeaponKarkianBump: 8 s), so the Karkian rams once and then bites.
    pub ai_fire_delay: f32,
    pub snd: crate::sound::WeaponSounds,
}

/// Mods fitted to one of the player's weapons (DeusExWeapon.Mod* travel vars), applied whenever it's drawn.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModState {
    pub accuracy: f32,
    pub clip: f32,
    pub range: f32,
    pub reload: f32,
    pub recoil: f32,
    pub scope: bool,
    pub silencer: bool,
    pub laser: bool,
}

/// WeaponMod*.ApplyMod on a weapon's default stats.
pub fn apply_mods(w: &mut WeaponStats, m: &ModState) {
    let base_acc = w.base_accuracy;
    if m.accuracy > 0.0 {
        w.base_accuracy = if base_acc == 0.0 {
            -m.accuracy
        } else {
            base_acc - base_acc * m.accuracy
        };
    }
    if m.clip > 0.0 {
        let steps = (m.clip / 0.1).round() as u32;
        let diff = ((w.reload_count as f32 * 0.1) as u32).max(1);
        w.reload_count += diff * steps;
    }
    w.accurate_range += w.accurate_range * m.range;
    w.reload_time = (w.reload_time + w.reload_time * m.reload).max(0.0);
    w.recoil = (w.recoil + w.recoil * m.recoil).max(0.0);
    w.has_scope |= m.scope;
    w.silenced |= m.silencer;
    w.has_laser |= m.laser;
}

/// WeaponMod.CanUpgradeWeapon + ApplyMod bookkeeping. Returns false if this mod doesn't fit or is maxed out (0.5).
pub fn fit_mod(mod_class: &str, w: &WeaponStats, m: &mut ModState) -> bool {
    let c = &w.can_mod;
    let step = |v: &mut f32, ok: bool, d: f32| {
        if ok && (v.abs() + 1e-4) < 0.5 {
            *v += d;
            true
        } else {
            false
        }
    };
    match mod_class {
        "WeaponModAccuracy" => step(&mut m.accuracy, c[0], 0.1),
        "WeaponModClip" => step(&mut m.clip, c[1], 0.1),
        "WeaponModRange" => step(&mut m.range, c[2], 0.1),
        "WeaponModReload" => step(&mut m.reload, c[3], -0.1),
        "WeaponModRecoil" => step(&mut m.recoil, c[4], -0.1),
        "WeaponModScope" if c[5] && !w.has_scope && !m.scope => {
            m.scope = true;
            true
        }
        "WeaponModSilencer" if c[6] && !w.silenced && !m.silencer => {
            m.silencer = true;
            true
        }
        "WeaponModLaser" if c[7] && !w.has_laser && !m.laser => {
            m.laser = true;
            true
        }
        _ => false,
    }
}

impl WeaponStats {
    /// ProcessTraceHit's damage multiplier for the player: the Combat Strength aug's value for melee weapons (1
    /// otherwise) minus twice GetWeaponSkill.
    pub fn damage_mult(&self) -> f32 {
        let combat = if self.hand_to_hand {
            crate::abilities::aug("AugCombat").unwrap_or(1.0)
        } else {
            1.0
        };
        combat - 2.0 * self.skill
    }

    /// GetShotTime for the player: Combat Strength makes melee swings faster.
    pub fn player_shot_time(&self) -> f32 {
        match (self.hand_to_hand, crate::abilities::aug("AugCombat")) {
            (true, Some(v)) if v > 0.0 => self.shot_time / v,
            _ => self.shot_time,
        }
    }
}

impl WeaponStats {
    /// WeaponDamageType for instant-hit weapons: 'Stunned' with a StunDuration (prod), 'Sabot' with sabot shells,
    /// 'KnockedOut' for the baton, else 'Shot'.
    pub fn damage_type(&self, ammo: Option<&str>) -> &'static str {
        if self.class == "WeaponBaton" {
            "KnockedOut"
        } else if self.stun {
            "Stunned"
        } else if ammo == Some("AmmoSabot") {
            "Sabot"
        } else {
            "Shot"
        }
    }

    /// TraceFire: instant-hit cone weapons fire 5 slugs; ProjectileFire: cone weapons launch 3.
    pub fn shots_per_fire(&self) -> usize {
        match (self.area_of_effect, self.instant) {
            (1, true) => 5,
            (1, false) => 3,
            _ => 1,
        }
    }
}

impl WeaponStats {
    /// The projectile class this weapon fires with the given ammo loaded.
    pub fn projectile_for(&self, ammo: Option<&str>) -> Option<String> {
        if self.instant {
            return None;
        }
        if let Some(a) = ammo {
            if let Some(i) = self
                .ammo_names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(a))
            {
                if let Some(p) = self.projectile_names.get(i) {
                    return Some(p.clone());
                }
            }
        }
        self.projectile.clone()
    }
}

fn short_class(p: &str) -> String {
    p.rsplit('.').next().unwrap_or(p).to_string()
}

fn path_array(p: &[Property], n: &str) -> Vec<String> {
    let mut v: Vec<(u32, String)> = p
        .iter()
        .filter(|x| x.name == n)
        .filter_map(|x| match &x.value {
            Value::Path(s) => Some((x.index, short_class(s))),
            _ => None,
        })
        .collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

fn fp(p: &[Property], n: &str, d: f32) -> f32 {
    match find(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}

pub fn weapon_stats(class: &str, p: &[Property]) -> Option<WeaponStats> {
    let view_mesh = match find(p, "PlayerViewMesh") {
        Some(Value::Path(s)) => s.clone(),
        _ => return None,
    };
    let ammo = match find(p, "AmmoName") {
        Some(Value::Path(s)) if !s.ends_with("AmmoNone") => {
            Some(s.rsplit('.').next().unwrap_or("").to_string())
        }
        _ => None,
    };
    let view_offset = match find(p, "PlayerViewOffset") {
        Some(Value::Vector(v)) => *v,
        _ => [20.0, -10.0, -14.0],
    };
    Some(WeaponStats {
        shake: [fp(p, "ShakeTime", 0.1), fp(p, "ShakeMag", 300.0), fp(p, "ShakeVert", 5.0)],
        class: class.to_string(),
        name: match find(p, "ItemName") {
            Some(Value::Str(s)) => s.clone(),
            _ => class.to_string(),
        },
        hit_damage: fp(p, "HitDamage", 10.0),
        shot_time: fp(p, "ShotTime", 0.5),
        reload_count: fp(p, "ReloadCount", 0.0) as u32,
        reload_time: fp(p, "ReloadTime", 2.0),
        base_accuracy: fp(p, "BaseAccuracy", 0.5),
        max_range: fp(p, "maxRange", 4800.0),
        hand_to_hand: matches!(find(p, "bHandToHand"), Some(Value::Bool(true))),
        ammo_class: ammo,
        view_offset,
        view_scale: fp(p, "PlayerViewScale", 1.0),
        view_mesh,
        third_mesh: match find(p, "ThirdPersonMesh") {
            Some(Value::Path(s)) => Some(s.clone()),
            _ => None,
        },
        third_scale: fp(p, "ThirdPersonScale", 1.0),
        mass: fp(p, "Mass", 10.0),
        skill: 0.0,
        governing_skill: match find(p, "GoverningSkill") {
            Some(Value::Path(s)) => Some(short_class(s)),
            _ => None,
        },
        noise_level: fp(p, "NoiseLevel", 1.0),
        silenced: matches!(find(p, "bHasSilencer"), Some(Value::Bool(true))),
        has_scope: matches!(find(p, "bHasScope"), Some(Value::Bool(true))),
        scope_fov: fp(p, "ScopeFOV", 10.0),
        stun: matches!(find(p, "bInstantHit"), Some(Value::Bool(true)))
            && fp(p, "StunDuration", 0.0) > 0.0,
        instant: matches!(find(p, "bInstantHit"), Some(Value::Bool(true))),
        projectile: match find(p, "ProjectileClass") {
            Some(Value::Path(s)) => Some(short_class(s)),
            _ => None,
        },
        muzzle_flash: matches!(find(p, "bHasMuzzleFlash"), Some(Value::Bool(true))),
        automatic: matches!(find(p, "bAutomatic"), Some(Value::Bool(true))),
        penetrating: matches!(find(p, "bPenetrating"), Some(Value::Bool(true))),
        item: (!class.starts_with("Weapon") || class.starts_with("WeaponMod"))
            && class != "POVCorpse",
        ammo_names: path_array(p, "AmmoNames"),
        projectile_names: path_array(p, "ProjectileNames"),
        area_of_effect: match find(p, "AreaOfEffect") {
            Some(Value::Byte(b)) => *b,
            _ => 0,
        },
        accurate_range: fp(p, "AccurateRange", fp(p, "maxRange", 4800.0) * 0.5),
        ai_min_range: fp(p, "AIMinRange", 0.0),
        ai_max_range: fp(p, "AIMaxRange", 0.0),
        fallback: matches!(find(p, "bFallbackWeapon"), Some(Value::Bool(true))),
        recoil: fp(p, "recoilStrength", 0.0),
        can_mod: [
            "bCanHaveModBaseAccuracy",
            "bCanHaveModReloadCount",
            "bCanHaveModAccurateRange",
            "bCanHaveModReloadTime",
            "bCanHaveModRecoilStrength",
            "bCanHaveScope",
            "bCanHaveSilencer",
            "bCanHaveLaser",
        ]
        .map(|n| matches!(find(p, n), Some(Value::Bool(true)))),
        has_laser: matches!(find(p, "bHasLaser"), Some(Value::Bool(true))),
        can_track: matches!(find(p, "bCanTrack"), Some(Value::Bool(true))),
        lock_time: fp(p, "LockTime", 2.0),
        ai_fire_delay: fp(p, "AIFireDelay", 0.0),
        snd: crate::sound::WeaponSounds::from_props(p),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WState {
    Selecting,
    Idle,
    Firing,
    Reloading,
    Lowering,
}

/// A simple keyframe player over an `AnimMesh` sequence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub start: usize,
    pub frames: usize,
    pub rate: f32,
    pub looping: bool,
    pub t: f32,
}

impl Clip {
    pub fn named(mesh: &AnimMesh, name: &str, looping: bool) -> Option<Clip> {
        let s = mesh
            .seqs
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))?;
        Some(Clip {
            start: s.start_frame.max(0) as usize,
            frames: s.num_frames.max(1) as usize,
            rate: s.rate,
            looping,
            t: 0.0,
        })
    }
    pub fn frame(&self) -> f32 {
        let f = self.t * self.rate;
        if self.looping {
            f
        } else {
            f.min(self.frames as f32 - 1.0)
        }
    }
    pub fn done(&self) -> bool {
        !self.looping && self.t * self.rate >= self.frames as f32 - 1.0
    }
}

pub struct Held {
    pub stats: WeaponStats,
    pub mesh: Arc<AnimMesh>,
    pub parts: Vec<MeshPart>,
    pub handles: Vec<Handle<Mesh>>,
    pub state: WState,
    pub clip: Clip,
    pub timer: f32,
    pub relight: f32,
    /// SkilledTool UseIt/StopIt: a lockpick or multitool is being worked (UseBegin, UseLoop, UseEnd).
    pub using: bool,
    /// Laser sight on (ToggleLaser, ']'): shots go dead on.
    pub lasing: bool,
    /// A grenade thrown or placed partway through the Attack/Place animation: (class, where, direction, wall normal).
    pub pending_throw: Option<(String, DVec3, DVec3, Option<DVec3>)>,
    /// That was the last one: the weapon goes once the animation ends.
    pub last_throw: bool,
    /// Lock-on (bCanTrack): mode (0 none, 1 invalid, 2 range, 3 acquire, 4 locked), timer, locked target, range.
    pub lock_mode: u8,
    pub lock_timer: f32,
    pub lock_target: Option<usize>,
    pub target_range: f64,
    /// SoundTimer for the lock-on beeps.
    pub beep_timer: f32,
    /// The sawed-off's Shoot animation notify (SawedOffCockSound at 40%) has fired for this shot.
    pub racked: bool,
    /// FlameThrowerOn.flameShotCount: the fire sound and firing anim come every seventh shot of a held burst.
    pub flame_shots: u32,
    /// A melee swing whose hit hasn't landed yet: the Attack animations call HandToHandAttack halfway through.
    pub melee_pending: bool,
    /// Reload state script position: 0 ReloadBegin, 1 looping Reload, 2 ReloadEnd.
    pub phase: u8,
    pub flash: Option<Entity>,
}

/// Rounds loaded in each weapon class (kept when switching weapons).
#[derive(Resource, Default)]
pub struct Clips(pub HashMap<String, u32>);

#[derive(Resource, Default)]
pub struct Weapons {
    pub held: Option<Held>,
    /// Belt slot to switch to once the current weapon is lowered.
    pub pending: Option<usize>,
    /// A body in our hands (POVCorpse): the dead pawn's entity, hidden while carried.
    pub corpse: Option<Entity>,
    /// A body just picked up, to be raised as the POVCorpse view model.
    pub want_corpse: Option<Entity>,
    /// Mods fitted, by weapon class.
    pub mods: HashMap<String, ModState>,
    /// The last real weapon drawn (unmodded stats): where a weapon mod used from the belt goes.
    pub last_weapon: Option<WeaponStats>,
    /// Ammo type loaded in each weapon class (CycleAmmo), when it isn't the default.
    pub loaded_ammo: HashMap<String, String>,
    /// A change of loaded ammo made while the weapon was borrowed, recorded next frame.
    pub ammo_note: Option<(String, String)>,
    /// FireExtinguisher: seconds of halon spray left (a ProjectileGenerator for 3 s), and its emit accumulator.
    pub spray: f32,
    pub spray_acc: f32,
    /// PutInHand from the inventory screen: the item (by class) to raise once the hands are free.
    pub pending_class: Option<String>,
    /// Inventory screen Use: activate this pickup without raising it.
    pub use_request: Option<String>,
    /// Enhancement "Use Consumables From The Floor": a consumable frobbed with no room for it is used on the spot.
    pub floor_use: Option<String>,
}

/// A carried body thrown or dropped: (pawn entity, where, velocity).
#[derive(Resource, Default)]
pub struct CorpseDrops(pub Vec<(Entity, DVec3, DVec3)>);

/// Damage taken by pawns (by blocker index).
#[derive(Resource, Default)]
pub struct PawnHealth(pub HashMap<usize, f32>);

/// Who last hurt each pawn (blocker index -> attacker blocker index, or None for the player).
#[derive(Resource, Default)]
pub struct LastAttacker(pub HashMap<usize, Option<usize>>);

#[derive(Component)]
pub struct AmmoText;

/// Input injected by automated tests (select a belt slot, pull the trigger).
#[derive(Resource, Default)]
pub struct ScriptedInput {
    pub select: Option<usize>,
    pub fire: bool,
}

/// Lazily opened game data for things created after load (weapon view meshes, icons).
#[derive(Resource)]
pub struct RuntimeAssets {
    pub classes: ClassDb,
    pub lib: dx_level::TexLib,
    pub images: HashMap<String, Option<Handle<Image>>>,
    pub materials: HashMap<(Option<String>, SurfaceClass), Handle<StandardMaterial>>,
    /// Animated procedural textures and the images they draw into.
    pub fire: Vec<(dx_level::firetex::FireTex, Handle<Image>)>,
}

pub fn spawn_view_camera(commands: &mut Commands) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 3,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 50f32.to_radians(),
            near: 0.01,
            far: 20.0,
            ..default()
        }),
        Tonemapping::None,
        Exposure { ev100: -0.263 },
        RenderLayers::layer(VIEW_LAYER),
        Transform::default(),
        // The view model is vertex-lit: a single light cluster.
        bevy::light::cluster::ClusterConfig::Single,
        ViewModelCam,
        // The last 3D camera to draw: brightness (gamma) applies to the whole picture here.
        crate::dynlight::Brightness {
            gamma: crate::dynlight::default_gamma(),
            ..default()
        },
    ));
    commands.spawn((Transform::default(), Visibility::default(), ViewModelRoot));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.9, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(14.0),
            bottom: Val::Px(96.0),
            ..default()
        },
        // The original ammo display (classic_hud.rs) shows this now.
        Visibility::Hidden,
        AmmoText,
    ));
    // HUDAmmoDisplay's targeting line for weapons that can lock on.
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgb(0.0, 1.0, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(14.0),
            bottom: Val::Px(118.0),
            ..default()
        },
        // Read by the ammo display's targeting line (classic_hud.rs).
        Visibility::Hidden,
        LockText,
    ));
}

#[derive(Component)]
pub struct LockText;

/// A zoomed view (binoculars: 20 degrees), or None.
#[derive(Resource, Default)]
pub struct Zoom(pub Option<f32>);

/// Keep the horizontal field of view at Deus Ex's 75 degrees whatever the window shape (or the zoom).
pub fn fov_system(
    windows: Query<&Window>,
    zoom: Res<Zoom>,
    ph: Res<crate::ai::PlayerHealth>,
    mut n: Local<u32>,
    mut cams: Query<(&mut Projection, Has<ViewModelCam>), (With<Camera3d>, Without<crate::computers::FeedCam>)>,
) {
    let _prof = crate::prof::scope("weapons::fov_system");
    let Some(w) = windows.iter().next() else {
        return;
    };
    let aspect = (w.width() / w.height().max(1.0)).max(0.1);
    let world_fov = crate::cinematic::fov()
        .unwrap_or_else(|| crate::drug::drug_fov(&ph, zoom.0.unwrap_or(DEFAULT_FOV), &mut n));
    for (mut p, is_vm) in &mut cams {
        let fov = if is_vm { DEFAULT_FOV } else { world_fov };
        let vfov = 2.0 * ((fov.to_radians() / 2.0).tan() / aspect).atan();
        if let Projection::Perspective(pp) = &mut *p {
            if (pp.fov - vfov).abs() > 1e-4 {
                pp.fov = vfov;
            }
        }
    }
}

/// Unreal view space (X forward, Y right, Z up) to the view camera's space (Bevy: -Z forward, +X right, +Y up).
fn view_to_cam(v: [f32; 3]) -> [f32; 3] {
    [
        v[1] * METERS_PER_UNIT,
        v[2] * METERS_PER_UNIT,
        -v[0] * METERS_PER_UNIT,
    ]
}

pub fn image_for(
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    path: &str,
) -> Option<Handle<Image>> {
    if let Some(h) = assets.images.get(path) {
        return h.clone();
    }
    // Procedural fire/electricity: an image the fire system redraws every frame.
    if let Some(mut f) = assets.lib.load_fire(path) {
        for _ in 0..40 {
            f.step(); // warm up so it doesn't start black
        }
        let mut data = Vec::new();
        f.rgba(&mut data);
        let mut img = Image::new(
            Extent3d {
                width: f.width as u32,
                height: f.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        let h = images.add(img);
        assets.fire.push((f, h.clone()));
        assets.images.insert(path.to_string(), Some(h.clone()));
        return Some(h);
    }
    let h = assets
        .lib
        .load_image(path)
        .or_else(|| assets.lib.load_wet(path))
        .map(|t| {
            let mut data = Vec::new();
            for m in &t.mips {
                data.extend_from_slice(m);
            }
            let mut img = Image::new_uninit(
                Extent3d {
                    width: t.width,
                    height: t.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            img.texture_descriptor.mip_level_count = t.mips.len() as u32;
            img.data = Some(data);
            img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                ..default()
            });
            images.add(img)
        });
    if let Some(h) = &h {
        crate::anim_tex::register(&mut assets.lib, path, h, false, false);
    }
    assets.images.insert(path.to_string(), h.clone());
    h
}

/// `image_for` for STY_Modulated drawing: Unreal modulates as dst x src x 2 (mid-grey leaves the scene as it
/// is), Bevy's AlphaMode::Multiply is dst x src, so the colours are doubled (in linear light) up front.
pub fn modulated_image_for(
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    path: &str,
) -> Option<Handle<Image>> {
    let key = format!("{path}#mod2x");
    if let Some(h) = assets.images.get(&key) {
        return h.clone();
    }
    let lut: Vec<u8> = (0..256)
        .map(|v| {
            // UE1 modulation is dst x 2 x src in gamma space: mid-grey (128) leaves the floor unchanged. Doubling the
            // stored (sRGB) value gives exactly that once the GPU multiplies in linear space ((2s)^2.2 x dst).
            let s = v as f32 / 255.0;
            let o = (s * 2.0).min(1.0);
            (o * 255.0 + 0.5) as u8
        })
        .collect();
    let h = assets.lib.load_image(path).map(|t| {
        let mut data = Vec::new();
        for m in &t.mips {
            data.extend_from_slice(m);
        }
        for px in data.chunks_mut(4) {
            for c in px.iter_mut().take(3) {
                *c = lut[*c as usize];
            }
        }
        let mut img = Image::new_uninit(
            Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.texture_descriptor.mip_level_count = t.mips.len() as u32;
        img.data = Some(data);
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        images.add(img)
    });
    assets.images.insert(key, h.clone());
    h
}

/// `image_for`, with palette index 0 forced transparent.
pub fn masked_image_for(
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    path: &str,
) -> Option<Handle<Image>> {
    let key = format!("{path}#masked");
    if let Some(h) = assets.images.get(&key) {
        return h.clone();
    }
    let h = assets.lib.load_image_masked(path).map(|t| {
        let mut data = Vec::new();
        for m in &t.mips {
            data.extend_from_slice(m);
        }
        let mut img = Image::new_uninit(
            Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.texture_descriptor.mip_level_count = t.mips.len() as u32;
        img.data = Some(data);
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        images.add(img)
    });
    assets.images.insert(key, h.clone());
    h
}

/// Build a weapon mesh's parts (frame 0) with materials, as children of `root`. `map` turns an Unreal-space
/// vertex into the root's local space; `layer` puts the parts on a render layer.
#[allow(clippy::too_many_arguments)]
pub fn spawn_mesh_parts(
    mesh_path: &str,
    scale: f32,
    defaults: &[Property],
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
    root: Entity,
    map: &dyn Fn([f32; 3]) -> [f32; 3],
    layer: Option<RenderLayers>,
) -> Option<(LodMesh, Vec<MeshPart>, Vec<Handle<Mesh>>)> {
    let (m, pkg) = load_mesh(&assets.classes, mesh_path)?;
    // Resolve textures to paths first, then to images.
    let mut tex_paths: Vec<String> = Vec::new();
    // The first-person weapon: Lay D Denton's own hands (travel::view_tex).
    let view = layer.as_ref().is_some_and(|l| *l == RenderLayers::layer(VIEW_LAYER));
    let parts = {
        let (lib, classes) = (&mut assets.lib, &assets.classes);
        let mut resolve = |tp: &str| -> Option<(usize, bool)> {
            let swapped = if view { crate::travel::view_tex(tp) } else { None };
            let tp = swapped.as_deref().unwrap_or(tp);
            let i = match tex_paths.iter().position(|x| x == tp) {
                Some(i) => i,
                None => {
                    tex_paths.push(tp.to_string());
                    tex_paths.len() - 1
                }
            };
            // A texture flagged bMasked draws masked on any section (the first-person combat knife's hilt).
            let masked = lib.load(tp).map(|(t, _)| t.masked).unwrap_or(false);
            Some((i, masked))
        };
        mesh_parts(
            &m,
            &pkg,
            classes,
            defaults,
            0,
            false,
            scale,
            0,
            &mut resolve,
        )
    };
    let mut handles = Vec::new();
    for part in &parts {
        let tex_path = part.texture.map(|i| tex_paths[i].clone());
        // PF_Masked mesh sections: palette index 0 is see-through even if the texture itself isn't flagged masked
        // (otherwise it shows as the palette's purple, e.g. the third-person combat knife).
        let img = if part.class == SurfaceClass::Masked {
            tex_path
                .as_deref()
                .and_then(|p| masked_image_for(assets, images, p))
        } else {
            tex_path
                .as_deref()
                .and_then(|p| image_for(assets, images, p))
        };
        let key = (tex_path.clone(), part.class);
        let mat = assets
            .materials
            .entry(key)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color_texture: img,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    alpha_mode: match part.class {
                        SurfaceClass::Masked => AlphaMode::Mask(0.5),
                        SurfaceClass::Translucent => AlphaMode::Add,
                        SurfaceClass::Modulated => AlphaMode::Multiply,
                        _ => AlphaMode::Opaque,
                    },
                    base_color: if part.class == SurfaceClass::Modulated {
                        Color::LinearRgba(LinearRgba::rgb(4.59, 4.59, 4.59))
                    } else {
                        Color::WHITE
                    },
                    ..default()
                })
            })
            .clone();
        let pos: Vec<[f32; 3]> = part.positions.iter().map(|p| map(*p)).collect();
        let nrm: Vec<[f32; 3]> = part
            .normals
            .iter()
            .map(|n| {
                let o = Vec3::from(map(*n)) - Vec3::from(map([0.0; 3]));
                o.normalize_or_zero().to_array()
            })
            .collect();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs.clone());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![[0.5f32, 0.5, 0.5, 1.0]; part.positions.len()],
        );
        mesh.insert_indices(Indices::U32(part.indices.clone()));
        let h = meshes.add(mesh);
        let mut child = commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(mat)));
        if let Some(l) = &layer {
            child.insert(l.clone());
        }
        if part.env {
            child.insert(crate::actors::EnvMapped);
        }
        let child = child.id();
        commands.entity(root).add_child(child);
        handles.push(h);
    }
    Some((m, parts, handles))
}

#[allow(clippy::too_many_arguments)]
fn build_held(
    stats: WeaponStats,
    defaults: &[Property],
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
    root: Entity,
) -> Option<Held> {
    let (m, parts, handles) = spawn_mesh_parts(
        &stats.view_mesh.clone(),
        stats.view_scale,
        defaults,
        assets,
        images,
        meshes,
        materials,
        commands,
        root,
        &view_to_cam,
        Some(RenderLayers::layer(VIEW_LAYER)),
    )?;
    let am = Arc::new(anim_mesh(&m));
    let flash = if stats.muzzle_flash {
        add_muzzle_flash(
            commands,
            assets,
            images,
            materials,
            root,
            &parts,
            &handles,
            Some(RenderLayers::layer(VIEW_LAYER)),
        )
    } else {
        None
    };
    let clip = Clip::named(&am, "Select", false).unwrap_or(Clip {
        start: 0,
        frames: 1,
        rate: 1.0,
        looping: false,
        t: 0.0,
    });
    // Start in the first frame of Select (lowered, out of view) so the mesh's rest pose never flashes up for a frame.
    for (pi, hm) in handles.iter().enumerate() {
        if let Some(mut mm) = meshes.get_mut(hm) {
            let pos: Vec<[f32; 3]> = parts[pi]
                .vert_ids
                .iter()
                .map(|&v| {
                    let p = am.sample(clip.start, clip.frames, 0.0, v as usize);
                    view_to_cam([
                        p[0] * stats.view_scale,
                        p[1] * stats.view_scale,
                        p[2] * stats.view_scale,
                    ])
                })
                .collect();
            mm.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        }
    }
    // DeusExWeapon BringUp: Owner.PlaySound(SelectSound, SLOT_None,,, 1024).
    if let Some(snd) = &stats.snd.select {
        crate::sound::play_player(snd, stats.snd.volume, 1024.0);
    }
    Some(Held {
        stats,
        mesh: am,
        parts,
        handles,
        state: WState::Selecting,
        clip,
        timer: 0.0,
        relight: 0.0,
        phase: 0,
        flash,
        using: false,
        lasing: false,
        pending_throw: None,
        last_throw: false,
        lock_mode: 0,
        lock_timer: 0.0,
        lock_target: None,
        target_range: 0.0,
        beep_timer: 0.0,
        racked: true,
        melee_pending: false,
        flame_shots: 0,
    })
}

/// Number keys pick a belt slot; weapons are raised, the previous one lowered first.
#[allow(clippy::too_many_arguments)]
pub fn select_system(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    inv: Res<PlayerInventory>,
    mut weapons: ResMut<Weapons>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    root: Single<Entity, With<ViewModelRoot>>,
    children: Query<&Children>,
    mut clips: ResMut<Clips>,
    mut scripted: ResMut<ScriptedInput>,
    (carrying, hacking, wheel): (
        Res<crate::carry::Carrying>,
        Res<crate::devices::Hacking>,
        Res<bevy::input::mouse::AccumulatedMouseScroll>,
    ),
) {
    let _prof = crate::prof::scope("weapons::select_system");
    if hacking.keypad.is_some() {
        return; // digits go to the keypad
    }
    const KEYS: [(KeyCode, usize); 10] = [
        (KeyCode::Digit1, 1),
        (KeyCode::Digit2, 2),
        (KeyCode::Digit3, 3),
        (KeyCode::Digit4, 4),
        (KeyCode::Digit5, 5),
        (KeyCode::Digit6, 6),
        (KeyCode::Digit7, 7),
        (KeyCode::Digit8, 8),
        (KeyCode::Digit9, 9),
        (KeyCode::Digit0, 0),
    ];
    let injected = scripted.select.take();
    if carrying.held.is_some() {
        return; // hands are full
    }
    // Picking up a body puts the POVCorpse "item" in hand; nothing else can be selected while carrying it.
    if let Some(body) = weapons.want_corpse.take() {
        if let Ok(ch) = children.get(*root) {
            for c in ch.iter() {
                commands.entity(c).despawn();
            }
        }
        weapons.held = None;
        weapons.pending = None;
        let d = assets.classes.defaults("POVCorpse");
        if let Some(stats) = weapon_stats("POVCorpse", &d) {
            let mut a = assets;
            if let Some(h) = build_held(
                stats,
                &d,
                &mut a,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut commands,
                *root,
            ) {
                weapons.held = Some(h);
                weapons.corpse = Some(body);
            }
        }
        return;
    }
    if weapons.corpse.is_some() {
        return;
    }
    // Weapons and the usable pickups that have a first-person view (medkit, food, lockpick, binoculars...).
    let mut usable = |slot: usize| -> bool {
        // Slot 0 holds the NanoKey ring when nothing else was put there.
        if slot == 0 && inv.0.belt[0].is_none() {
            return true;
        }
        inv.0.belt[slot]
            .and_then(|i| inv.0.items.get(i))
            .map(|it| {
                it.def.kind == ItemKind::Weapon
                    || (matches!(it.def.kind, ItemKind::Pickup { .. })
                        && find(&assets.classes.defaults(&it.def.class), "PlayerViewMesh").is_some())
            })
            .unwrap_or(false)
    };
    // NextBeltItem / PrevBeltItem (MouseWheelDown / MouseWheelUp): the next usable belt slot after the one in
    // hand (or coming up), wrapping round.
    let mut injected = injected;
    if injected.is_none()
        && wheel.delta.y != 0.0
        && !crate::inventory_screen::screen_open()
        && !crate::consys::third_person()
    {
        let cur = weapons
            .pending
            .or_else(|| {
                let class = weapons.held.as_ref().map(|h| h.stats.class.clone())?;
                (0..10).find(|&s| {
                    inv.0.belt[s]
                        .and_then(|i| inv.0.items.get(i))
                        .is_some_and(|it| it.def.class == class)
                })
            })
            .unwrap_or(0);
        let step = if wheel.delta.y < 0.0 { 1 } else { 9 };
        // Empty-handed, the start slot counts too (only the key ring in slot 0 on an empty belt: it was never
        // reached). With something in hand it doesn't (that would put it away).
        let empty_handed = weapons.held.is_none() && weapons.pending.is_none();
        let mut s = cur;
        for _ in 0..10 {
            s = (s + step) % 10;
            if s == cur && !empty_handed {
                break;
            }
            if usable(s) {
                injected = Some(s);
                break;
            }
            if s == cur {
                break;
            }
        }
    }
    for (k, slot) in KEYS {
        if keys.just_pressed(k) || injected == Some(slot) {
            let is_weapon = usable(slot);
            if !is_weapon {
                continue;
            }
            let w = &mut *weapons;
            match w.held.as_mut() {
                Some(h) if h.state != WState::Lowering => {
                    let same = inv.0.belt[slot]
                        .and_then(|i| inv.0.items.get(i))
                        .map(|it| it.def.class == h.stats.class)
                        .unwrap_or(slot == 0 && h.stats.class == "NanoKeyRing");
                    if same {
                        // Pressing the same slot again puts the weapon away.
                        w.pending = None;
                    } else {
                        w.pending = Some(slot);
                    }
                    h.state = WState::Lowering;
                    h.clip = Clip::named(&h.mesh, "Down", false).unwrap_or(h.clip.clone());
                    h.clip.t = 0.0;
                }
                Some(_) => w.pending = Some(slot),
                None => w.pending = Some(slot),
            }
        }
    }
    // Once put away, the old view model goes (even when nothing new is coming up).
    if weapons.held.is_none() && weapons.pending.is_none() && weapons.pending_class.is_none() {
        if let Ok(ch) = children.get(*root) {
            for c in ch.iter() {
                commands.entity(c).despawn();
            }
        }
    }
    // Raise the pending weapon once nothing is held.
    if weapons.held.is_none() {
        let want: Option<String> = match weapons.pending.take() {
            Some(0) if inv.0.belt[0].is_none() => Some("NanoKeyRing".to_string()),
            Some(slot) => inv.0.belt[slot]
                .and_then(|i| inv.0.items.get(i))
                .map(|i| i.def.class.clone()),
            None => weapons
                .pending_class
                .take()
                .and_then(|c| inv.0.items.iter().find(|i| i.def.class == c))
                .map(|i| i.def.class.clone()),
        };
        if let Some(class) = want {
            let d = assets.classes.defaults(&class);
            if let Some(mut stats) = weapon_stats(&class, &d) {
                if !stats.item && stats.class != "POVCorpse" {
                    weapons.last_weapon = Some(stats.clone());
                }
                if let Some(m) = weapons.mods.get(&class) {
                    apply_mods(&mut stats, m);
                }
                if let Some(a) = weapons.loaded_ammo.get(&class) {
                    stats.ammo_class = Some(a.clone());
                }
                if let Ok(ch) = children.get(*root) {
                    for c in ch.iter() {
                        commands.entity(c).despawn();
                    }
                }
                let mut a = assets;
                if let Some(h) = build_held(
                    stats,
                    &d,
                    &mut a,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                    &mut commands,
                    *root,
                ) {
                    clips.0.entry(class).or_insert(h.stats.reload_count);
                    weapons.held = Some(h);
                }
            }
        }
    }
}

/// Distance-scaled spread like DeusExWeapon.TraceFire: the end point is jittered by accuracy * 1000 units.
pub fn spread_dir(
    forward: DVec3,
    right: DVec3,
    up: DVec3,
    accuracy: f64,
    range: f64,
    seed: &mut u64,
) -> DVec3 {
    let mut rnd = || {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64) / (1u64 << 31) as f64
    };
    let end = right * (accuracy * (rnd() - 0.5) * 1000.0)
        + up * (accuracy * (rnd() - 0.5) * 1000.0)
        + forward * range.max(1024.0);
    end.normalize()
}

/// Fire, reload and animate the held weapon; apply hits to pawns.
#[allow(clippy::too_many_arguments)]
pub fn weapon_system(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    cam: Single<&Transform, (With<FlyCam>, Without<SkyCam>, Without<ViewModelRoot>)>,
    mut root: Single<&mut Transform, (With<ViewModelRoot>, Without<FlyCam>)>,
    (mut weapons, mut inv, mut clips): (ResMut<Weapons>, ResMut<PlayerInventory>, ResMut<Clips>),
    (player, level, blockers, mv): (Res<Player>, Res<LevelRes>, Res<Blockers>, Res<Movers>),
    mut meshes: ResMut<Assets<Mesh>>,
    (mut hud, mut ammo_text, mut seed): (
        ResMut<HudMessage>,
        Single<&mut Text, (With<AmmoText>, Without<LockText>)>,
        Local<u64>,
    ),
    (mut lock_text, pawn_health): (
        Single<(&mut Text, &mut TextColor), (With<LockText>, Without<AmmoText>)>,
        Res<PawnHealth>,
    ),
    (mut scripted, mut deco_hits, mut pawn_hits, aim): (
        ResMut<ScriptedInput>,
        ResMut<crate::destruct::DecoHits>,
        ResMut<crate::damage::PawnHits>,
        Res<crate::hud::Aim>,
    ),
    (mut noises, mut launches, mut flashes, mut shells, mut blood, mut corpse_drops): (
        ResMut<crate::stealth::Noises>,
        ResMut<crate::projectiles::Launches>,
        Query<&mut MuzzleFlashPart>,
        ResMut<crate::destruct::Shells>,
        ResMut<crate::effects::BloodFx>,
        ResMut<CorpseDrops>,
    ),
    (mut ph, mut zoom, mut grenades, picking, hacking): (
        ResMut<crate::ai::PlayerHealth>,
        ResMut<Zoom>,
        ResMut<crate::grenades::GrenadeThrows>,
        Res<crate::movers::Picking>,
        Res<crate::devices::Hacking>,
    ),
    (mut charged, mut rt_assets, mut flares): (
        ResMut<crate::charged::ActiveCharged>,
        ResMut<RuntimeAssets>,
        ResMut<crate::ai::FlareThrows>,
    ),
) {
    let _prof = crate::prof::scope("weapons::weapon_system");
    let dt = time.delta_secs();
    // What pawns see the player doing (reactions.rs): a weapon out (BringUp's WeaponDrawn), a tool at work on a lock
    // or a device (MegaFutz).
    {
        let drawn = weapons.held.as_ref().is_some_and(|h| {
            h.stats.class.starts_with("Weapon") && !matches!(h.state, WState::Lowering)
        });
        crate::reactions::set_weapon_drawn(drawn && !ph.dead);
        let at = player.body.location;
        crate::reactions::set_hacking((picking.0.is_some() || hacking.job.is_some()).then_some(at));
    }
    let injected_fire = std::mem::take(&mut scripted.fire);
    let mut pending_ammo: Option<(String, String)> = None;
    // Fire extinguisher spray: halon clouds out along the view at 300 units/s, living 1.5 s, ~9 a second.
    if weapons.spray > 0.0 {
        weapons.spray -= dt;
        weapons.spray_acc += dt * 9.0;
        let eye = bevy_to_ue(cam.translation.to_array());
        let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
        let f = cam.forward().as_vec3();
        let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
        while weapons.spray_acc >= 1.0 {
            weapons.spray_acc -= 1.0;
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let dir = crate::projectiles::projectile_dir(fw, 0.15, &mut *seed);
            launches.0.push(crate::projectiles::Launch {
                class: "HalonGas".into(),
                pos: eye + fw * player.params.radius,
                dir,
                from: None,
                mult: 1.0,
                life: Some(1.5),
                scale: None,
                still: false,
                track: None,
                track_at: None,
                speed: None,
                ownerless: false,
            });
        }
    }
    if let Some((c, a)) = std::mem::take(&mut weapons.ammo_note) {
        weapons.loaded_ammo.insert(c, a);
    }
    // Ours: a consumable used straight from the floor (no room in the inventory): used without taking one.
    if let Some(class) = weapons.floor_use.take() {
        let r = activate_pickup(&class, &mut ph, &cam, &player, &mut launches, &mut flares, &mut zoom, &mut charged, &mut rt_assets, &mut weapons);
        if !r.msg.is_empty() {
            hud.text = r.msg;
            hud.time_left = 2.0;
        }
    }
    // Use from the inventory screen (UseSelectedItem: inv.Activate()).
    if let Some(class) = weapons.use_request.take() {
        let r = activate_pickup(
            &class,
            &mut ph,
            &cam,
            &player,
            &mut launches,
            &mut flares,
            &mut zoom,
            &mut charged,
            &mut rt_assets,
            &mut weapons,
        );
        if r.spray {
            weapons.spray = 3.0;
            weapons.spray_acc = 1.0;
        }
        if !r.msg.is_empty() {
            hud.text = r.msg;
            hud.time_left = 2.0;
        }
        if r.consumed
            && inv.0.use_one(&class) == 0
            && weapons
                .held
                .as_ref()
                .is_some_and(|h| h.stats.class == class)
        {
            weapons.held = None;
            zoom.0 = None;
        }
    }
    let Some(h) = weapons.held.as_mut() else {
        ammo_text.0.clear();
        lock_text.0 .0.clear();
        return;
    };
    // GetWeaponSkill: the governing skill's value plus the Targeting aug's.
    h.stats.skill = h
        .stats
        .governing_skill
        .as_deref()
        .map_or(0.0, crate::abilities::skill)
        + crate::abilities::aug("AugTarget").unwrap_or(0.0);
    // View model placement: Weapon.SetHand scales PlayerViewOffset by 100, then Inventory.CalcDrawOffset uses
    // 0.9 / FOV of it, plus the walk bob damped by BobDamping (0.96). The camera itself already carries the full
    // walk bob, and the view model is drawn relative to the camera, so only the difference shows: the weapon lags
    // the head bob by 4% sideways and ~2% vertically.
    let k = 0.9 / DEFAULT_FOV * 100.0;
    let bob = player.view.walk_bob;
    let r = cam.right().as_vec3();
    let damping = 0.96f32;
    let side = (bob.x * r.x as f64 + bob.y * r.z as f64) as f32 * (damping - 1.0);
    let up = (0.45 + 0.55 * damping - 1.0) * bob.z as f32;
    // Right-handed (Handedness -1): Weapon.SetHand mirrors a weapon's Y offset to the right side of the view.
    // Pickups (tools, medkits, bodies) have no SetHand, so their offset is used as it is (+Y is already right).
    let ysign = if h.stats.item || h.stats.class == "POVCorpse" {
        1.0
    } else {
        -1.0
    };
    let off = [
        h.stats.view_offset[0] * k,
        ysign * h.stats.view_offset[1] * k + side,
        h.stats.view_offset[2] * k + up,
    ];
    root.translation = Vec3::from(view_to_cam(off));

    h.clip.t += dt;
    h.timer -= dt;
    // DeusExWeapon.Tick lock-on (bCanTrack): AcquireTarget traces 10000 units along the view for the first
    // decoration or pawn. A pawn within MaxRange held for LockTime (2 s untrained) locks; a different pawn breaks
    // the lock; anything else is INVALID. The timer runs back down while there's no good target.
    if h.stats.can_track {
        let eye = bevy_to_ue(cam.translation.to_array());
        let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
        let f = cam.forward().as_vec3();
        let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
        let world = World {
            level: &level.0,
            blockers: &blockers.list,
            ignore: None,
            pushable: None,
            movers: Some(&mv.rt),
        };
        let hit = world.trace(eye, eye + fw * 10000.0, DVec3::ZERO);
        let target = hit.actor;
        let reloading = h.state == WState::Reloading;
        match target.filter(|_| !reloading) {
            None => {
                h.lock_mode = 0;
                h.lock_target = None;
                h.lock_timer -= dt;
            }
            Some(b) => {
                h.target_range = blockers.list[b].center.distance(player.body.location);
                let a = &level.0.actors[blockers.list[b].actor];
                let alive_pawn = find(&a.props, "GroundSpeed").is_some()
                    && pawn_health.0.get(&b).map_or(true, |&hp| hp > 0.0);
                if h.lock_mode == 4
                    && alive_pawn
                    && h.lock_target.is_some()
                    && h.lock_target != Some(b)
                {
                    h.lock_mode = 0;
                    h.lock_target = None;
                } else if !alive_pawn {
                    h.lock_mode = 1;
                    h.lock_timer -= dt;
                } else if h.target_range > h.stats.max_range as f64 {
                    h.lock_mode = 2;
                    h.lock_timer -= dt;
                } else {
                    let lock_time = (h.stats.lock_time + 3.0 * h.stats.skill).max(0.0);
                    h.lock_timer += dt;
                    if h.lock_timer >= lock_time {
                        h.lock_mode = 4;
                        h.lock_target = Some(b);
                    } else {
                        h.lock_mode = 3;
                    }
                }
            }
        }
        h.lock_timer = h.lock_timer.max(0.0);
        let lock_time = (h.stats.lock_time + 3.0 * h.stats.skill).max(0.0);
        // Acquiring beeps faster as the lock nears (beepspeed = clamp(time left / LockTime, 0.2, 1)); one tone on lock.
        h.beep_timer += dt;
        if h.lock_mode == 3 {
            let beep = ((lock_time - h.lock_timer) / h.stats.lock_time.max(0.01)).clamp(0.2, 1.0);
            if h.beep_timer > beep {
                h.beep_timer = 0.0;
                if let Some(s) = &h.stats.snd.tracking {
                    crate::sound::play_player(s, 1.0, 0.0);
                }
            }
        }
        // While locked, PlayLockSound every 0.1 s: a held tone.
        if h.lock_mode == 4 {
            if let Some(s) = &h.stats.snd.locked {
                crate::sound::hold(crate::sound::key_of("gep-lock"), s, None, 1.0, 0.0, 1.0);
            }
        }
        let ft = (h.target_range / 16.0) as i32;
        let (msg, col) = match h.lock_mode {
            1 => ("INVALID".to_string(), Color::srgb(0.0, 1.0, 0.0)),
            2 => (format!("RANGE {ft} FT"), Color::srgb(0.0, 1.0, 0.0)),
            3 => (
                format!("ACQUIRE {:.2} SEC", (lock_time - h.lock_timer).max(0.0)),
                Color::srgb(1.0, 1.0, 0.0),
            ),
            4 => (format!("LOCKED {ft} FT"), Color::srgb(1.0, 0.0, 0.0)),
            _ => ("NONE".to_string(), Color::srgb(0.0, 1.0, 0.0)),
        };
        let (ref mut t, ref mut c) = *lock_text;
        if t.0 != msg {
            t.0 = msg;
        }
        c.0 = col;
    } else if !lock_text.0 .0.is_empty() {
        lock_text.0 .0.clear();
    }
    // SkilledTool: UseBegin then a looping UseLoop while picking or hacking; UseEnd when it stops.
    if matches!(h.stats.class.as_str(), "Lockpick" | "Multitool") && h.state == WState::Idle {
        let active = picking.0.is_some() || hacking.job.is_some();
        if active && !h.using {
            h.using = true;
            if let Some(c) = Clip::named(&h.mesh, "UseBegin", false) {
                h.clip = c;
            }
        } else if active && h.clip.done() && !h.clip.looping {
            if let Some(c) = Clip::named(&h.mesh, "UseLoop", true) {
                h.clip = c;
            }
        } else if !active && h.using {
            h.using = false;
            if let Some(c) = Clip::named(&h.mesh, "UseEnd", false) {
                h.clip = c;
            }
        }
    }
    if h.using {
        if let Some(snd) = &h.stats.snd.use_loop {
            crate::sound::hold(crate::sound::key_of("tool-use"), snd, None, 0.7, 825.0, 1.0);
        }
    }
    // Shotgun mesh notify: Shoot at 0.40 calls SawedOffCockSound (the pump rack, if there's ammo left).
    if h.state == WState::Firing
        && !h.racked
        && h.clip.t * h.clip.rate >= h.clip.frames as f32 * 0.4
    {
        h.racked = true;
        let left = *clips.0.get(&h.stats.class).unwrap_or(&0);
        if h.stats.class == "WeaponSawedOffShotgun" && left > 0 {
            if let Some(s) = &h.stats.snd.select {
                crate::sound::play_player(s, 1.0, 1024.0);
            }
        }
    }
    // Hand-to-hand weapons: the mesh notify HandToHandAttack (Attack/Attack2/Attack3 at 0.5) does TraceFire.
    if h.state == WState::Firing
        && h.melee_pending
        && h.clip.t * h.clip.rate >= h.clip.frames as f32 * 0.5
    {
        h.melee_pending = false;
        let eye = bevy_to_ue(cam.translation.to_array());
        let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
        let f = cam.forward().as_vec3();
        let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
        let range = h.stats.max_range as f64;
        let world = World {
            level: &level.0,
            blockers: &blockers.list,
            ignore: None,
            pushable: None,
            movers: Some(&mv.rt),
        };
        let hit = world.trace(eye, eye + fw * range, DVec3::splat(0.5));
        let bird =
            crate::critters::trace_birds(eye, eye + fw * range).filter(|b| b.1 < hit.fraction);
        if let Some((b, _)) = bird {
            crate::critters::hit_bird(b, h.stats.hit_damage * h.stats.damage_mult());
            if let Some(s) = &h.stats.snd.misc[0] {
                crate::sound::play_player(s, 1.0, 1024.0);
            }
        } else if hit.fraction < 1.0 {
            let at = eye + fw * range * hit.fraction;
            if hit.actor.is_none() {
                crate::movers::hit_line(
                    eye,
                    at,
                    h.stats.hit_damage * h.stats.damage_mult(),
                    h.stats.damage_type(h.stats.ammo_class.as_deref()),
                );
            }
            let props = hit
                .actor
                .map(|b| &level.0.actors[blockers.list[b].actor].props);
            let pawn = props.map_or(false, |p| find(p, "GroundSpeed").is_some());
            // SpawnEffectSounds: flesh (Misc1), a decoration (Misc3), anything else hard (Misc2).
            let k = if pawn {
                0
            } else if props.is_some() {
                2
            } else {
                1
            };
            // Hard world surfaces textured from the Glass group: BulletProofHit.
            let glass = props.is_none()
                && level.0.wall_group(at.to_array(), hit.normal.to_array()).is_some_and(|g| g.eq_ignore_ascii_case("Glass"));
            if glass {
                crate::sound::play_player("DeusExSounds.Generic.BulletProofHit", 1.0, 1024.0);
            } else if let Some(s) = &h.stats.snd.misc[k] {
                crate::sound::play_player(s, 1.0, 1024.0);
            }
            if let Some(b) = hit.actor {
                let dt_name = h.stats.damage_type(h.stats.ammo_class.as_deref());
                if !pawn {
                    if props.is_some_and(|p| matches!(find(p, "bOwned"), Some(Value::Bool(true)))) {
                        crate::reactions::futz(blockers.list[b].center);
                    }
                    deco_hits.0.push(crate::destruct::DecoHit {
                        blocker: b,
                        damage: h.stats.hit_damage * h.stats.damage_mult(),
                        damage_type: dt_name,
                    });
                } else {
                    if h.stats.penetrating
                        && props.map_or(true, |p| find(p, "EMPHitPoints").is_none())
                    {
                        blood.0.push((at - fw, fw));
                    }
                    pawn_hits.0.push(crate::damage::PawnHit {
                        blocker: b,
                        damage: h.stats.hit_damage * h.stats.damage_mult(),
                        damage_type: dt_name,
                        hit_loc: at,
                        attacker: None,
                        attacker_pos: player.body.location,
                    });
                }
            }
        }
    }
    // The grenade leaves the hand halfway through the throw (or place) animation.
    if h.state == WState::Firing && h.clip.t * h.clip.rate >= h.clip.frames as f32 * 0.5 {
        if let Some(t) = h.pending_throw.take() {
            grenades.0.push((t.0, t.1, t.2, None, t.3));
        }
    }
    let loaded = *clips.0.get(&h.stats.class).unwrap_or(&0);
    let pool = h.stats.ammo_class.as_ref().map(|a| {
        inv.0
            .ammo
            .iter()
            .find(|x| &x.0 == a)
            .map(|x| x.1)
            .unwrap_or(0)
    });
    match h.state {
        WState::Selecting if h.clip.done() => {
            h.state = WState::Idle;
            h.clip = Clip::named(&h.mesh, "Idle1", false).unwrap_or(h.clip.clone());
            h.timer = 3.0;
        }
        // NormalFire: Sleep(ShotTime), then FinishAnim() unless automatic.
        WState::Firing if h.timer <= 0.0 && (h.stats.automatic || h.clip.done()) => {
            if h.last_throw {
                // Hand-to-hand with ReloadCount and no ammo left: the weapon is destroyed.
                let class = h.stats.class.clone();
                inv.0.use_one(&class);
                weapons.held = None;
                return;
            }
            let thrown = h.stats.hand_to_hand && h.stats.projectile.is_some();
            if thrown && h.pending_throw.is_none() {
                // The next grenade comes up into the hand.
                h.state = WState::Selecting;
                h.clip = Clip::named(&h.mesh, "Select", false).unwrap_or(h.clip.clone());
            } else {
                h.state = WState::Idle;
                h.clip = Clip::named(&h.mesh, "Idle1", false).unwrap_or(h.clip.clone());
                h.timer = 3.0;
            }
        }
        // DeusExWeapon state Reload: PlayAnim ReloadBegin, LoopAnim Reload for the reload time, PlayAnim ReloadEnd,
        // then the clip is full.
        WState::Reloading if h.phase == 0 && h.clip.done() => {
            h.phase = 1;
            h.timer = h.stats.reload_time + h.stats.skill * h.stats.reload_time;
            h.clip = Clip::named(&h.mesh, "Reload", true).unwrap_or(h.clip.clone());
        }
        WState::Reloading if h.phase == 1 && h.timer <= 0.0 => {
            h.phase = 2;
            if let Some(s) = &h.stats.snd.reload_end {
                crate::sound::play_player(s, 1.0, 1024.0);
            }
            match Clip::named(&h.mesh, "ReloadEnd", false) {
                Some(c) => h.clip = c,
                None => h.phase = 3,
            }
        }
        WState::Reloading if (h.phase == 2 && h.clip.done()) || h.phase == 3 => {
            if let Some(a) = h.stats.ammo_class.clone() {
                if let Some(entry) = inv.0.ammo.iter_mut().find(|x| x.0 == a) {
                    let need = h.stats.reload_count.saturating_sub(loaded);
                    let take = need.min(entry.1);
                    entry.1 -= take;
                    clips.0.insert(h.stats.class.clone(), loaded + take);
                }
            }
            h.state = WState::Idle;
            h.clip = Clip::named(&h.mesh, "Idle1", false).unwrap_or(h.clip.clone());
            h.timer = 3.0;
        }
        WState::Lowering if h.clip.done() => {
            weapons.held = None;
            return;
        }
        // State Idle: every 3 s there's a 30% chance of one of the fidget animations (PlayIdleAnim).
        WState::Idle if h.timer <= 0.0 && !h.using => {
            h.timer = 3.0;
            if *seed == 0 {
                *seed = 0x9E37_79B9_7F4A_7C15;
            }
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let rnd = ((*seed >> 33) as f32) / (1u64 << 31) as f32;
            let name = if rnd < 0.1 {
                Some("Idle1")
            } else if rnd < 0.2 {
                Some("Idle2")
            } else if rnd < 0.3 {
                Some("Idle3")
            } else {
                None
            };
            if let Some(c) = name.and_then(|n| Clip::named(&h.mesh, n, false)) {
                h.clip = c;
            }
        }
        _ => {}
    }
    // Carrying a body: either button throws it (DropItem): from 0.8 x radius ahead at eye height. The original
    // halves view x 300 x AugMuscle + 220 up + 40 x VRand for a corpse, which barely leaves the hands; ours throws
    // it at the full speed, like a crate (the user's call), and it slides on landing (actors::deco_physics).
    if let Some(body) = weapons.corpse {
        let clicked = cursor.grab_mode == CursorGrabMode::Locked
            && (mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right));
        if clicked {
            let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64).normalize();
            let fw = conv(cam.forward().as_vec3());
            if *seed == 0 {
                *seed = 0x9E37_79B9_7F4A_7C15;
            }
            let mut rnd = || {
                *seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((*seed >> 33) as f64) / (1u64 << 31) as f64 * 2.0 - 1.0
            };
            let jitter = DVec3::new(rnd(), rnd(), rnd()).normalize_or_zero() * 40.0;
            let vel = fw * 300.0 * crate::abilities::aug_level("AugMuscle").map_or(1.0, |l| 1.25 + 0.25 * l as f64) + DVec3::new(0.0, 0.0, 220.0) + jitter;
            let flat = DVec3::new(fw.x, fw.y, 0.0).normalize_or_zero();
            let pos = player.body.location
                + flat * (0.8 * player.params.radius)
                + DVec3::new(0.0, 0.0, player.eye_height);
            corpse_drops.0.push((body, pos, vel));
            weapons.corpse = None;
            weapons.held = None;
            return;
        }
    }
    let Some(h) = weapons.held.as_mut() else {
        return;
    };
    let locked = cursor.grab_mode == CursorGrabMode::Locked || injected_fire;
    // Pickups in hand: left click uses them (DeusExPickup Activated states).
    if h.stats.item {
        if h.stats.class != "Binoculars" {
            zoom.0 = None;
        }
        root.scale = if zoom.0.is_some() {
            Vec3::ZERO
        } else {
            Vec3::ONE
        };
        if locked && mouse.just_pressed(MouseButton::Left) && h.state == WState::Idle {
            let class = h.stats.class.clone();
            let r = activate_pickup(
                &class,
                &mut ph,
                &cam,
                &player,
                &mut launches,
                &mut flares,
                &mut zoom,
                &mut charged,
                &mut rt_assets,
                &mut weapons,
            );
            let (consumed, msg, weapons_spray) = (r.consumed, r.msg, r.spray);
            if weapons_spray {
                weapons.spray = 3.0;
                weapons.spray_acc = 1.0;
            }
            if !msg.is_empty() {
                hud.text = msg;
                hud.time_left = 2.0;
            }
            if consumed {
                println!("USE {class}");
                if inv.0.use_one(&class) == 0 {
                    weapons.held = None;
                    zoom.0 = None;
                    root.scale = Vec3::ONE;
                    return;
                }
            }
        }
    } else if h.stats.has_scope {
        // ScopeToggle ('[' in the default keys): only from Idle; reloading or switching takes the scope off.
        if (keys.just_pressed(KeyCode::BracketLeft) || keys.just_pressed(KeyCode::KeyZ))
            && h.state == WState::Idle
        {
            zoom.0 = if zoom.0.is_some() {
                None
            } else {
                Some(h.stats.scope_fov)
            };
        }
        if !matches!(h.state, WState::Idle | WState::Firing) {
            zoom.0 = None;
        }
        root.scale = if zoom.0.is_some() {
            Vec3::ZERO
        } else {
            Vec3::ONE
        };
    } else {
        zoom.0 = None;
        root.scale = Vec3::ONE;
    }
    let Some(h) = weapons.held.as_mut() else {
        return;
    };
    let pulled = weapons_can_fire(&h.stats)
        && (mouse.just_pressed(MouseButton::Left)
            || injected_fire
            || (h.stats.automatic && mouse.pressed(MouseButton::Left)));
    if !pulled {
        h.flame_shots = 0;
    }

    // ToggleLaser (']'): only with a laser mod fitted.
    if keys.just_pressed(KeyCode::BracketRight) && h.stats.has_laser {
        h.lasing = !h.lasing;
        hud.text = if h.lasing {
            "Laser sight on".into()
        } else {
            "Laser sight off".into()
        };
        hud.time_left = 1.5;
    }
    // CycleAmmo (' in the default keys, also T): load the next ammo type this weapon takes that we have. The
    // rounds left in the clip go back to their box, then the weapon reloads with the new type.
    if (keys.just_pressed(KeyCode::Quote) || keys.just_pressed(KeyCode::KeyT))
        && matches!(h.state, WState::Idle)
        && h.stats.ammo_names.len() > 1
    {
        let cur = h.stats.ammo_class.clone().unwrap_or_default();
        let names = h.stats.ammo_names.clone();
        let i = names.iter().position(|n| *n == cur).unwrap_or(0);
        let next = (1..names.len())
            .map(|k| names[(i + k) % names.len()].clone())
            .find(|n| inv.0.ammo.iter().any(|x| &x.0 == n && x.1 > 0));
        match next {
            Some(n) => {
                if loaded > 0 {
                    match inv.0.ammo.iter_mut().find(|x| x.0 == cur) {
                        Some(e) => e.1 += loaded,
                        None => inv.0.ammo.push((cur.clone(), loaded)),
                    }
                }
                clips.0.insert(h.stats.class.clone(), 0);
                h.stats.ammo_class = Some(n.clone());
                weapons_loaded_ammo_set(&mut pending_ammo, &h.stats.class, &n);
                let pretty = rt_assets.classes.defaults(&n);
                let pretty = match find(&pretty, "ItemName") {
                    Some(Value::Str(s)) => s.clone(),
                    _ => n.clone(),
                };
                hud.text = format!("{} now has {} loaded", h.stats.name, pretty);
                hud.time_left = 2.0;
                // Reload with it straight away.
                h.state = WState::Reloading;
                if let Some(s) = &h.stats.snd.reload_begin {
                    crate::sound::play_player(s, 1.0, 1024.0);
                }
                h.timer = h.stats.reload_time + h.stats.skill * h.stats.reload_time;
                match Clip::named(&h.mesh, "ReloadBegin", false) {
                    Some(c) => {
                        h.clip = c;
                        h.phase = 0;
                    }
                    None => {
                        h.clip = Clip::named(&h.mesh, "Reload", true).unwrap_or(h.clip.clone());
                        h.phase = 1;
                    }
                }
            }
            None => {
                hud.text = "No other ammo for this weapon".into();
                hud.time_left = 1.5;
            }
        }
    }
    // Reload on R, or automatically when the clip is empty and ammo remains.
    let can_reload =
        h.stats.reload_count > 0 && loaded < h.stats.reload_count && pool.unwrap_or(0) > 0;
    if h.state == WState::Idle
        && can_reload
        && (keys.just_pressed(KeyCode::KeyR) || (loaded == 0 && locked && pulled && crate::settings::auto_reload()))
    {
        h.state = WState::Reloading;
        if let Some(s) = &h.stats.snd.reload_begin {
            crate::sound::play_player(s, 1.0, 1024.0);
        }
        h.timer = h.stats.reload_time + h.stats.skill * h.stats.reload_time;
        match Clip::named(&h.mesh, "ReloadBegin", false) {
            Some(c) => {
                h.clip = c;
                h.phase = 0;
            }
            None => {
                h.clip = Clip::named(&h.mesh, "Reload", true).unwrap_or(h.clip.clone());
                h.phase = 1;
            }
        }
    } else if h.state == WState::Idle && locked && pulled {
        let thrown = h.stats.hand_to_hand && h.stats.reload_count > 0;
        let has_round = if thrown {
            pool.unwrap_or(1) > 0
        } else {
            h.stats.hand_to_hand || h.stats.reload_count == 0 || loaded > 0
        };
        if !has_round {
            // Dry fire click.
            if !h.stats.hand_to_hand {
                if let Some(s) = &h.stats.snd.misc[0] {
                    crate::sound::play_player(s, h.stats.snd.volume * 2.0, 1024.0);
                }
            }
            hud.text = "Out of ammo".into();
            hud.time_left = 2.0;
        } else {
            if !h.stats.hand_to_hand && h.stats.reload_count > 0 {
                clips.0.insert(h.stats.class.clone(), loaded - 1);
            }
            h.state = WState::Firing;
            h.timer = h.stats.player_shot_time();
            h.racked = false;
            // PlayFiringSound: a silencer swaps in StealthPistolFire.
            let fire = if h.stats.silenced && !h.stats.hand_to_hand {
                Some("DeusExSounds.Weapons.StealthPistolFire".to_string())
            } else {
                h.stats.snd.fire.clone()
            };
            // The flamethrower (FlameThrowerOn) roars once per seven shots instead of every 0.1 s.
            let flame = h.stats.class == "WeaponFlamethrower";
            let sound_now = !flame || h.flame_shots == 0;
            if flame {
                h.flame_shots = if h.flame_shots == 0 {
                    6
                } else {
                    h.flame_shots - 1
                };
            }
            if let (Some(s), true) = (fire, sound_now) {
                crate::sound::play_player(&s, h.stats.snd.volume, 2048.0);
            }
            if let Some(f) = h.flash.and_then(|e| flashes.get_mut(e).ok()) {
                f.into_inner().left = 0.1;
            }
            // MuzzleFlashLight: Owner.Location + view x CollisionRadius x 2, 0.1 s.
            if h.stats.muzzle_flash {
                let f = cam.forward().as_vec3();
                let fwd = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize_or_zero();
                crate::dynlight::flash(
                    player.body.location + fwd * player.params.radius * 2.0,
                    crate::dynlight::MUZZLE_FLASH,
                    0.1,
                );
            }
            if !h.stats.hand_to_hand {
                if let Some(class) = crate::destruct::shell_for(
                    h.stats.ammo_class.as_deref(),
                    h.stats.silenced || h.stats.noise_level < 0.1,
                ) {
                    let f = cam.forward().as_vec3();
                    let fwd = DVec3::new(f.x as f64, f.z as f64, f.y as f64);
                    let (pos, vel) = crate::destruct::shell_launch(
                        player.body.location,
                        player.params.radius,
                        player.params.height,
                        fwd,
                        &mut seed,
                    );
                    shells.0.push((class, pos, vel));
                }
            }
            // GetAIVolume: unsilenced guns are heard (WeaponFire + LoudNoise) out to 800 x NoiseLevel.
            if let Some(n) = crate::stealth::weapon_noise(
                &h.stats,
                player.body.location,
                crate::ai::Target::Player,
            ) {
                // AISendEvent WeaponFire (needs the listener to see the shooter) and LoudNoise (heard anywhere in range).
                let mut loud = n.clone();
                loud.kind = crate::stealth::NoiseKind::Loud;
                noises.0.push(n);
                noises.0.push(loud);
            }
            // PlaySelectiveFiring: Shoot, or one of Attack/Attack2/Attack3 at random for hand-to-hand.
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let r3 = ((*seed >> 33) % 3) as usize;
            let attack = ["Attack", "Attack2", "Attack3"][r3];
            let first = if h.stats.hand_to_hand {
                attack
            } else {
                "Shoot"
            };
            h.clip = Clip::named(&h.mesh, first, false)
                .or_else(|| Clip::named(&h.mesh, "Shoot", false))
                .or_else(|| Clip::named(&h.mesh, "Attack", false))
                .unwrap_or(h.clip.clone());

            // Instant-hit trace from the eye.
            let eye = bevy_to_ue(cam.translation.to_array());
            let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
            let conv = |v: Vec3| DVec3::new(v.x as f64, v.z as f64, v.y as f64).normalize();
            let (fw, rt, up) = (
                conv(cam.forward().as_vec3()),
                conv(cam.right().as_vec3()),
                conv(cam.up().as_vec3()),
            );
            if *seed == 0 {
                *seed = 0x9E37_79B9_7F4A_7C15;
            }
            let acc = aim.accuracy as f64;
            // DeusExWeapon.Fire: shake us based on accuracy.
            if !h.stats.hand_to_hand {
                let [t, m, v] = h.stats.shake;
                crate::shake::shake_view(t, aim.accuracy * m + m, aim.accuracy * v);
                let heavy = matches!(h.stats.class.as_str(), "WeaponGEPGun" | "WeaponLAW" | "WeaponSawedOffShotgun" | "WeaponAssaultShotgun");
                let s = if heavy { 1.0 } else { (h.stats.hit_damage / 30.0).clamp(0.25, 0.9) };
                crate::dualsense::event(crate::dualsense::Haptic::Shot(s));
            }
            let range = h.stats.max_range as f64;
            // Ammo for hand-to-hand weapons with a ReloadCount (grenades, throwing knives) is used per throw
            // (Fire: AmmoType.UseAmmo(1)); out of it, the weapon goes once the animation ends. Single-use launchers
            // (ReloadCount 0: LAW, PS20) are destroyed after one shot.
            if h.stats.hand_to_hand && h.stats.reload_count > 0 {
                if let Some(a) = h.stats.ammo_class.clone() {
                    if let Some(e) = inv.0.ammo.iter_mut().find(|x| x.0 == a) {
                        e.1 = e.1.saturating_sub(1);
                        h.last_throw = e.1 == 0;
                    }
                }
            } else if h.stats.reload_count == 0 && !h.stats.hand_to_hand && !h.stats.item {
                h.last_throw = true;
            }
            let thrown = matches!(
                h.stats.class.as_str(),
                "WeaponLAM" | "WeaponGasGrenade" | "WeaponEMPGrenade" | "WeaponNanoVirusGrenade"
            );
            let grenade = h.stats.projectile.clone().filter(|_| thrown);
            if let Some(class) = grenade {
                // Thrown weapons (LAM, gas, EMP): placed on a wall within a foot (NearWallCheck), else thrown.
                // NearWallCheck's Trace has bTraceActors off: only the level and movers count, so a crate, a pawn
                // or an invisible blocker in front of the wall doesn't turn the placement into a throw.
                let world = World {
                    level: &level.0,
                    blockers: &[],
                    ignore: None,
                    pushable: None,
                    movers: Some(&mv.rt),
                };
                let wh = world.trace(eye, eye + fw * 32.0, DVec3::ZERO);
                if wh.fraction < 1.0 && wh.actor.is_none() {
                    let at = eye + fw * 32.0 * wh.fraction + wh.normal * 2.0;
                    h.pending_throw = Some((class, at, fw, Some(wh.normal)));
                    if let Some(c) = Clip::named(&h.mesh, "Place", false) {
                        h.clip = c;
                    }
                } else {
                    h.pending_throw = Some((class, eye + fw * 16.0, fw, None));
                }
            } else if let Some(class) = h.stats.projectile_for(h.stats.ammo_class.as_deref()) {
                // Projectile weapons (mini-crossbow): launch from just in front of the eyes.
                for _ in 0..h.stats.shots_per_fire() {
                    let dir = crate::projectiles::projectile_dir(fw, acc, &mut seed);
                    // Locked on: the rocket gets the target and tracks it.
                    let track = if h.stats.can_track && h.lock_mode == 4 {
                        h.lock_target
                    } else {
                        None
                    };
                    launches.0.push(crate::projectiles::Launch {
                        class: class.clone(),
                        pos: eye + fw * 12.0,
                        dir,
                        from: None,
                        mult: h.stats.damage_mult(),
                        life: None,
                        scale: None,
                        still: false,
                        track,
                        track_at: None,
                        speed: None,
                        ownerless: false,
                    });
                }
            } else if h.stats.hand_to_hand {
                // The swing (FireSound) starts now; the blow lands at the HandToHandAttack notify.
                h.melee_pending = true;
            } else {
                let mut ricocheted = false;
                for _ in 0..h.stats.shots_per_fire() {
                    let dir = if h.stats.hand_to_hand {
                        fw
                    } else {
                        spread_dir(fw, rt, up, acc, range, &mut seed)
                    };
                    let end = eye + dir * range;
                    let world = World {
                        level: &level.0,
                        blockers: &blockers.list,
                        ignore: None,
                        pushable: None,
                        movers: Some(&mv.rt),
                    };
                    let hit = world.trace(eye, end, DVec3::splat(0.5));
                    if let Some((bird, f)) = crate::critters::trace_birds(eye, end) {
                        if f < hit.fraction {
                            crate::critters::hit_bird(
                                bird,
                                h.stats.hit_damage * h.stats.damage_mult(),
                            );
                            continue;
                        }
                    }
                    if hit.fraction < 1.0 {
                        let at = eye + dir * range * hit.fraction;
                        let props = hit
                            .actor
                            .map(|b| &level.0.actors[blockers.list[b].actor].props);
                        let pawn = props.map_or(false, |p| find(p, "GroundSpeed").is_some());
                        let robot = props.map_or(false, |p| find(p, "EMPHitPoints").is_some());
                        if h.stats.hand_to_hand {
                            // SpawnEffectSounds: flesh (Misc1), a decoration (Misc3), anything else hard (Misc2).
                            let k = if pawn {
                                0
                            } else if props.is_some() {
                                2
                            } else {
                                1
                            };
                            let glass = props.is_none()
                                && level.0.wall_group(at.to_array(), hit.normal.to_array()).is_some_and(|g| g.eq_ignore_ascii_case("Glass"));
                            if glass {
                                crate::sound::play_player("DeusExSounds.Generic.BulletProofHit", 1.0, 1024.0);
                            } else if let Some(s) = &h.stats.snd.misc[k] {
                                crate::sound::play_player(s, 1.0, 1024.0);
                            }
                        } else if !ricocheted
                            && h.stats.penetrating
                            && (!pawn || robot)
                            && matches!(
                                h.stats.damage_type(h.stats.ammo_class.as_deref()),
                                "Shot" | "Sabot"
                            )
                        {
                            // One ricochet per trigger pull: five overlapping ones from a shotgun blast sound wrong.
                            ricocheted = true;
                            // TraceHitSpawner.PlayHitSound: a ricochet, or ArmorRicochet off robots and tough decorations.
                            let armored = robot
                                || props.map_or(false, |p| {
                                    crate::sound::fprop(p, "minDamageThreshold", 0.0) > 10.0
                                });
                            let snd = if armored {
                                "DeusExSounds.Generic.ArmorRicochet".to_string()
                            } else {
                                format!(
                                    "DeusExSounds.Generic.Ricochet{}",
                                    1 + (crate::sound::frand() * 4.0) as u32 % 4
                                )
                            };
                            crate::sound::play(
                                &snd,
                                at,
                                1.0,
                                1024.0,
                                1.1 - 0.2 * crate::sound::frand(),
                            );
                        }
                    }
                    if hit.fraction < 1.0 && hit.actor.is_none() {
                        // ProcessTraceHit on the level or a mover: breakable doors take HitDamage x the skill multiplier.
                        crate::movers::hit_line(
                            eye,
                            eye + dir * range * hit.fraction,
                            h.stats.hit_damage * h.stats.damage_mult(),
                            h.stats.damage_type(h.stats.ammo_class.as_deref()),
                        );
                    }
                    if let Some(b) = hit.actor {
                        let ai = blockers.list[b].actor;
                        let a = &level.0.actors[ai];
                        let is_pawn = find(&a.props, "GroundSpeed").is_some();
                        // ProcessTraceHit: shooting something bOwned is a Futz.
                        if matches!(find(&a.props, "bOwned"), Some(Value::Bool(true))) {
                            crate::reactions::futz(blockers.list[b].center);
                        }
                        if !is_pawn {
                            deco_hits.0.push(crate::destruct::DecoHit {
                                blocker: b,
                                damage: h.stats.hit_damage * h.stats.damage_mult(),
                                damage_type: h.stats.damage_type(h.stats.ammo_class.as_deref()),
                            });
                        }
                        if is_pawn
                            && h.stats.penetrating
                            && find(&a.props, "EMPHitPoints").is_none()
                        {
                            blood
                                .0
                                .push((eye + dir * range * hit.fraction - dir * 1.0, dir));
                        }
                        if is_pawn {
                            // ProcessTraceHit: damage scaled by skill (1 - 2 * skill value).
                            let dmg = h.stats.hit_damage * h.stats.damage_mult();
                            pawn_hits.0.push(crate::damage::PawnHit {
                                blocker: b,
                                damage: dmg,
                                damage_type: h.stats.damage_type(h.stats.ammo_class.as_deref()),
                                hit_loc: eye + dir * range * hit.fraction,
                                attacker: None,
                                attacker_pos: player.body.location,
                            });
                        }
                    }
                }
            }
        }
    }

    // Animate the view model.
    let f = h.clip.frame();
    for (pi, hm) in h.handles.iter().enumerate() {
        let part = &h.parts[pi];
        if let Some(mut m) = meshes.get_mut(hm) {
            let pos: Vec<[f32; 3]> = part
                .vert_ids
                .iter()
                .map(|&v| {
                    let p = h.mesh.sample(
                        h.clip.start,
                        h.clip.frames,
                        if h.clip.looping {
                            f
                        } else {
                            f.min(h.clip.frames as f32 - 1.001)
                        },
                        v as usize,
                    );
                    view_to_cam([
                        p[0] * h.stats.view_scale,
                        p[1] * h.stats.view_scale,
                        p[2] * h.stats.view_scale,
                    ])
                })
                .collect();
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        }
    }
    // Light the weapon from the static lights around the player a few times a second (every frame while a
    // dynamic light such as a muzzle flash or the flashlight is on).
    h.relight -= dt;
    let dyn_on = !crate::dynlight::current().is_empty();
    if h.relight <= 0.0 || dyn_on {
        h.relight = 0.2;
        let eye = bevy_to_ue(cam.translation.to_array());
        let all = crate::dynlight::lights(&level.0.lights);
        for (pi, hm) in h.handles.iter().enumerate() {
            if let Some(mut m) = meshes.get_mut(hm) {
                let c = light_part(&h.parts[pi], eye, [0, 0, 0], 1.0, &all);
                // Never fully black: Deus Ex lights the weapon with a minimum glow.
                let c: Vec<[f32; 4]> = c
                    .into_iter()
                    .map(|x| [x[0].max(0.06), x[1].max(0.06), x[2].max(0.06), 1.0])
                    .collect();
                m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
            }
        }
    }

    let clip_now = *clips.0.get(&h.stats.class).unwrap_or(&0);
    ammo_text.0 = match (h.stats.hand_to_hand || h.stats.reload_count == 0, pool) {
        (true, _) => h.stats.name.to_uppercase(),
        (false, Some(p)) => format!("{}  {} / {}", h.stats.name.to_uppercase(), clip_now, p),
        (false, None) => format!("{}  {}", h.stats.name.to_uppercase(), clip_now),
    };
    if let Some(p) = pending_ammo {
        weapons.ammo_note = Some(p);
    }
    let _ = &mut commands;
}

fn weapons_loaded_ammo_set(slot: &mut Option<(String, String)>, class: &str, ammo: &str) {
    *slot = Some((class.to_string(), ammo.to_string()));
}

/// Developer aid: `DXVIEW_FIRETEST="ue_x;ue_y;ue_z;ue_yaw_deg;pitch_deg;shots;out.png"` stands the player there, raises
/// the belt-slot-1 weapon and fires the given number of shots, then saves a screenshot and exits.
#[derive(Resource)]
pub struct FireTest {
    pub pos: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub shots: u32,
    pub out: String,
    pub t: f32,
    pub fired: u32,
    pub next: f32,
    pub done_frames: u32,
    /// Belt slot to raise (default 1, the pistol).
    pub slot: usize,
    pub aim_shot: bool,
    /// Keep the view on this placed actor (by name), e.g. a patrolling trooper.
    pub track: Option<String>,
    /// After the tracked pawn goes down, look at the body and search it.
    pub search: bool,
    pub searched_at: Option<f32>,
}

pub fn fire_test_cfg() -> Option<FireTest> {
    let v = std::env::var("DXVIEW_FIRETEST").ok()?;
    let p: Vec<&str> = v.split(';').collect();
    let f = |i: usize| p.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    Some(FireTest {
        pos: DVec3::new(f(0), f(1), f(2)),
        yaw: f(3) as f32,
        pitch: f(4) as f32,
        shots: f(5) as u32,
        out: p.get(6).unwrap_or(&"firetest.png").to_string(),
        t: 0.0,
        fired: 0,
        next: std::env::var("DXVIEW_FIRETEST_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1.2),
        done_frames: 0,
        slot: p.get(7).and_then(|s| s.parse().ok()).unwrap_or(1),
        aim_shot: false,
        track: p.get(8).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        search: p.get(9).map(|s| *s == "search").unwrap_or(false),
        searched_at: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn fire_test(
    mut commands: Commands,
    time: Res<Time>,
    ft: Option<ResMut<FireTest>>,
    mut player: ResMut<Player>,
    mut cam: Single<&mut FlyCam, Without<SkyCam>>,
    mut scripted: ResMut<ScriptedInput>,
    weapons: Res<Weapons>,
    level: Res<LevelRes>,
    blockers: Res<Blockers>,
    mut exit: MessageWriter<AppExit>,
    mut frob: ResMut<crate::pickups::FrobRequest>,
    bodies: Query<&crate::ai::Npc>,
) {
    let Some(mut ft) = ft else { return };
    let dt = time.delta_secs().min(0.05);
    if ft.t == 0.0 {
        player.noclip = false;
        let params = player.params.clone();
        player.body = imsim_core::movement::Character::new(ft.pos);
        player.body.settle(
            &World {
                level: &level.0,
                blockers: &blockers.list,
                ignore: None,
                pushable: None,
                movers: None,
            },
            &params,
        );
    }
    cam.yaw = (-ft.yaw - 90.0).to_radians();
    cam.pitch = ft.pitch.to_radians();
    if let Some(name) = &ft.track {
        if ft.t < 0.05 {
            println!(
                "FIRETEST tracking {name}: found={}",
                blockers
                    .list
                    .iter()
                    .any(|c| level.0.actors[c.actor].name.eq_ignore_ascii_case(name))
            );
        }
        // A downed pawn: look at the body on the floor and search it once.
        let down = bodies.iter().find(|n| {
            level.0.actors[n.actor].name.eq_ignore_ascii_case(name)
                && n.state == crate::ai::NpcState::Dead
        });
        if let (true, Some(n)) = (ft.search, down) {
            let c = blockers.list[n.blocker].center - DVec3::new(0.0, 0.0, n.params.height - 10.0);
            let eye = player.body.location + DVec3::new(0.0, 0.0, player.params.height * 0.8);
            let to = c - eye;
            cam.yaw = (-(to.y.atan2(to.x).to_degrees() as f32) - 90.0).to_radians();
            cam.pitch = to.z.atan2(DVec3::new(to.x, to.y, 0.0).length()) as f32;
            if ft.searched_at.is_none() {
                ft.searched_at = Some(ft.t);
            } else if ft
                .searched_at
                .map(|t| ft.t - t > 1.5 && ft.t - t < 1.6)
                .unwrap_or(false)
            {
                frob.0 = true;
            }
        } else if let Some(c) = blockers
            .list
            .iter()
            .find(|c| c.radius > 0.0 && level.0.actors[c.actor].name.eq_ignore_ascii_case(name))
        {
            let eye = player.body.location + DVec3::new(0.0, 0.0, player.params.height * 0.8);
            let to = c.center + DVec3::new(0.0, 0.0, c.height * 0.3) - eye;
            let yaw = to.y.atan2(to.x).to_degrees() as f32;
            let pitch =
                to.z.atan2(DVec3::new(to.x, to.y, 0.0).length())
                    .to_degrees() as f32;
            cam.yaw = (-yaw - 90.0).to_radians();
            cam.pitch = pitch.to_radians();
        }
    }
    ft.t += dt;
    if ft.t > 0.3 && weapons.held.is_none() && ft.fired == 0 {
        scripted.select = Some(ft.slot);
    }
    // A screenshot just before the first shot shows the crosshair on the target.
    if !ft.aim_shot
        && ft.t >= ft.next - 0.05
        && weapons
            .held
            .as_ref()
            .map(|h| h.state == WState::Idle)
            .unwrap_or(false)
    {
        ft.aim_shot = true;
        let aim_out = ft.out.replace(".png", "_aim.png");
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(aim_out));
    }
    if ft.fired < ft.shots && ft.t >= ft.next {
        if weapons
            .held
            .as_ref()
            .map(|h| h.state == WState::Idle)
            .unwrap_or(false)
        {
            scripted.fire = true;
            ft.fired += 1;
            println!("FIRETEST shot {}", ft.fired);
        }
        ft.next = ft.t + 0.3;
    }
    let search_pending =
        ft.search && ft.searched_at.map(|t| ft.t - t < 3.0).unwrap_or(true) && ft.t < ft.next + 6.0;
    if ft.fired >= ft.shots && ft.t >= ft.next + 1.5 && !search_pending {
        ft.done_frames += 1;
        if ft.done_frames == 1 {
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(ft.out.clone()));
        }
        if ft.done_frames == 30 {
            exit.write(AppExit::Success);
        }
    }
}

/// Advance procedural textures at a steady rate and upload the new frames.
pub fn fire_texture_system(
    time: Res<Time>,
    mut acc: Local<f32>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut buf: Local<Vec<u8>>,
) {
    let _prof = crate::prof::scope("weapons::fire_texture_system");
    const RATE: f32 = 1.0 / 40.0;
    *acc += time.delta_secs().min(0.25);
    if *acc < RATE {
        return;
    }
    let steps = ((*acc / RATE) as u32).min(4);
    *acc -= steps as f32 * RATE;
    for (f, h) in assets.fire.iter_mut() {
        for _ in 0..steps {
            f.step();
        }
        if let Some(mut img) = images.get_mut(h) {
            f.rgba(&mut buf);
            img.data = Some(buf.clone());
        }
    }
}

/// Marks the extra copy of a weapon's flash slot drawn with the muzzle-flash texture.
#[derive(Component)]
pub struct MuzzleFlashPart {
    /// Seconds left to show.
    pub left: f32,
    pub mats: [Handle<StandardMaterial>; 2],
}

/// DeusExWeapon.SwapMuzzleFlashTexture: weapons with a muzzle flash have a translucent, unlit slot (BlackMaskTex,
/// so invisible) that shows FlatFXTex34 or FlatFXTex37 for a tenth of a second per shot. We draw a second copy
/// of that part with the flash texture and show it when firing. Returns the flash entity.
#[allow(clippy::too_many_arguments)]
pub fn add_muzzle_flash(
    commands: &mut Commands,
    assets: &mut RuntimeAssets,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    parts: &[MeshPart],
    handles: &[Handle<Mesh>],
    layer: Option<RenderLayers>,
) -> Option<Entity> {
    let i = parts
        .iter()
        .position(|p| p.class == SurfaceClass::Translucent && p.unlit)?;
    let mut mats = Vec::new();
    for t in [
        "DeusExItems.Skins.FlatFXTex34",
        "DeusExItems.Skins.FlatFXTex37",
    ] {
        let img = image_for(assets, images, t);
        mats.push(materials.add(StandardMaterial {
            base_color_texture: img,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Add,
            ..default()
        }));
    }
    let mut e = commands.spawn((
        Mesh3d(handles[i].clone()),
        MeshMaterial3d(mats[0].clone()),
        Visibility::Hidden,
        MuzzleFlashPart {
            left: 0.0,
            mats: [mats[0].clone(), mats[1].clone()],
        },
    ));
    if let Some(l) = layer {
        e.insert(l);
    }
    let id = e.id();
    commands.entity(root).add_child(id);
    Some(id)
}

/// Show/hide muzzle flashes and pick the texture each time one starts.
pub fn muzzle_flash_system(
    time: Res<Time>,
    mut q: Query<(
        &mut MuzzleFlashPart,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    mut n: Local<u32>,
) {
    let _prof = crate::prof::scope("weapons::muzzle_flash_system");
    let dt = time.delta_secs();
    for (mut f, mut vis, mut mat) in &mut q {
        if f.left > 0.0 {
            if *vis == Visibility::Hidden {
                *n = n.wrapping_mul(1103515245).wrapping_add(12345);
                mat.0 = f.mats[((*n >> 16) & 1) as usize].clone();
            }
            *vis = Visibility::Inherited;
            f.left -= dt;
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

/// Pickups in hand (binoculars, medkits, tools) never fire; they have their own use on the button.
fn weapons_can_fire(w: &WeaponStats) -> bool {
    w.class != "POVCorpse" && !w.item
}

/// MedKit.healAmount (30) - kept simple: the medkit is the only pickup with a class-specific amount.

#[cfg(test)]
mod mod_tests {
    use super::*;

    fn pistol() -> WeaponStats {
        let p = vec![
            Property {
                name: "PlayerViewMesh".into(),
                index: 0,
                value: Value::Path("X.Y".into()),
            },
            Property {
                name: "ReloadCount".into(),
                index: 0,
                value: Value::Int(6),
            },
            Property {
                name: "BaseAccuracy".into(),
                index: 0,
                value: Value::Float(0.7),
            },
            Property {
                name: "bCanHaveModBaseAccuracy".into(),
                index: 0,
                value: Value::Bool(true),
            },
            Property {
                name: "bCanHaveModReloadCount".into(),
                index: 0,
                value: Value::Bool(true),
            },
        ];
        weapon_stats("WeaponPistol", &p).unwrap()
    }

    #[test]
    fn accuracy_mod_stacks_to_half() {
        let w = pistol();
        let mut m = ModState::default();
        for _ in 0..5 {
            assert!(fit_mod("WeaponModAccuracy", &w, &mut m));
        }
        // HasMaxAccuracyMod at 0.5.
        assert!(!fit_mod("WeaponModAccuracy", &w, &mut m));
        assert!(!fit_mod("WeaponModRange", &w, &mut m));
        let mut x = w.clone();
        apply_mods(&mut x, &m);
        assert!((x.base_accuracy - 0.35).abs() < 1e-4);
    }

    #[test]
    fn clip_mod_adds_at_least_one() {
        let w = pistol();
        let mut m = ModState::default();
        assert!(fit_mod("WeaponModClip", &w, &mut m));
        let mut x = w.clone();
        apply_mods(&mut x, &m);
        assert_eq!(x.reload_count, 7);
    }
}

/// What using a pickup did (DeusExPickup Activated states): whether a copy was used up, the message to show,
/// and whether the fire extinguisher spray starts.
pub struct UseResult {
    pub consumed: bool,
    pub msg: String,
    pub spray: bool,
}

/// Activate a pickup of `class`, held in the hand (left click) or from the inventory screen's Use button.
#[allow(clippy::too_many_arguments)]
pub fn activate_pickup(
    class: &str,
    ph: &mut crate::ai::PlayerHealth,
    cam: &Transform,
    player: &Player,
    launches: &mut crate::projectiles::Launches,
    flares: &mut crate::ai::FlareThrows,
    zoom: &mut Zoom,
    charged: &mut crate::charged::ActiveCharged,
    rt_assets: &mut RuntimeAssets,
    weapons: &mut Weapons,
) -> UseResult {
    let class = class.to_string();
    let d: Option<f32> = if class == "MedKit" { Some(30.0) } else { None };
    let heal = |ph: &mut crate::ai::PlayerHealth, n: f32| {
        ph.heal(n);
    };
    let mut consumed = true;
    let mut msg = String::new();
    let mut weapons_spray = false;
    match class.as_str() {
        "MedKit" => {
            // HealPlayer(healAmount, bUseMedicineSkill): x the Medicine skill's value.
            let n = (d.unwrap_or(30.0) * crate::abilities::skill("SkillMedicine")).floor();
            heal(ph, n);
            // HealPlayer with the Medicine skill: MedicalHiss.
            crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
            msg = format!("Healed {} points", n as i32);
        }
        "Sodacan" => {
            heal(ph, 2.0);
            crate::sound::play_player(&crate::travel::voice("DeusExSounds.Player.MaleBurp"), 1.0, 0.0);
        }
        "Candybar" => heal(ph, 2.0),
        "SoyFood" => heal(ph, 5.0),
        "Liquor40oz" => {
            heal(ph, 2.0);
            ph.drug_timer += 10.0;
        }
        "WineBottle" => {
            heal(ph, 2.0);
            ph.drug_timer += 5.0;
        }
        "Cigarettes" => {
            // 5 PoisonGas to JC, and a puff of smoke in front of his face.
            ph.damage(5.0, "PoisonGas", DVec3::ZERO);
            crate::sound::play_owned(
                crate::sound::PLAYER,
                crate::sound::slot::PAIN,
                &crate::travel::voice("DeusExSounds.Player.MaleCough"),
                None,
                1.0,
                0.0,
                1.0,
            );
            let eye = bevy_to_ue(cam.translation.to_array());
            let eye = DVec3::new(eye[0] as f64, eye[1] as f64, eye[2] as f64);
            let f = cam.forward().as_vec3();
            let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
            launches.0.push(crate::projectiles::Launch {
                class: "SmokeTrail".into(),
                pos: eye + fw * (2.0 * player.params.radius),
                dir: DVec3::Z,
                from: None,
                mult: 0.0,
                life: Some(1.0),
                scale: None,
                still: true,
                track: None,
                track_at: None,
                speed: None,
                ownerless: false,
            });
        }
        "LiquorBottle" => {
            heal(ph, 2.0);
            ph.drug_timer += 5.0;
        }
        "VialCrack" => {
            // Zyme: a minute of the drug effect and 10 health off.
            ph.drug_timer += 60.0;
            ph.heal(-10.0);
        }
        "VialAmbrosia" => msg = "Strange...nothing happens...".into(),
        "BioelectricCell" => {
            ph.energy = (ph.energy + 25.0).min(100.0);
            crate::sound::play_player("DeusExSounds.Generic.BioElectricHiss", 1.0, 256.0);
            msg = "Recharged 25 points".into();
        }
        "FireExtinguisher" => {
            ph.on_fire = None;
            weapons_spray = true;
        }
        "Flare" => {
            // LightFlare: thrown from eye height at view x 500 + 220 up; burns for 30 s.
            let f = cam.forward().as_vec3();
            let fw = DVec3::new(f.x as f64, f.z as f64, f.y as f64).normalize();
            let at = player.body.location
                + DVec3::new(fw.x, fw.y, 0.0) * (0.8 * player.params.radius)
                + DVec3::new(0.0, 0.0, player.eye_height);
            flares
                .0
                .push((at, fw * 500.0 + DVec3::new(0.0, 0.0, 220.0)));
        }
        "NanoKeyRing" => {
            // Used by frobbing a door with it in hand.
            consumed = false;
        }
        "AugmentationCannister" | "AugmentationUpgradeCannister" => {
            consumed = false;
            msg = "Augmentations arrive with the augmentation screen".into();
        }
        "Binoculars" => {
            zoom.0 = if zoom.0.is_some() { None } else { Some(20.0) };
            consumed = false;
        }
        c if crate::charged::is_charged(c) => {
            // ChargedPickup Activated: bOneUseOnly, so it leaves the hand and runs until used up.
            let d = rt_assets.classes.defaults(&class);
            if charged.activate(&class, &d) {
                let name = match unreal_pkg::props::find(&d, "ItemName") {
                    Some(Value::Str(n)) => n.clone(),
                    _ => class.clone(),
                };
                msg = format!("{name} activated");
            } else {
                consumed = false;
                msg = "That's already in use".into();
            }
        }
        m if m.starts_with("WeaponMod") => {
            // Stand-in for dragging the mod onto a weapon in the inventory screen: it goes on the last
            // weapon you had out.
            match weapons.last_weapon.clone() {
                Some(w) => {
                    let st = weapons.mods.entry(w.class.clone()).or_default();
                    if fit_mod(m, &w, st) {
                        msg = format!("{} upgraded", w.name);
                    } else {
                        consumed = false;
                        msg = format!("This can't be fitted to the {}", w.name);
                    }
                }
                None => {
                    consumed = false;
                    msg = "Draw a weapon first, then use the mod".into();
                }
            }
        }
        _ => consumed = false,
    }
    UseResult {
        consumed,
        msg,
        spray: weapons_spray,
    }
}
