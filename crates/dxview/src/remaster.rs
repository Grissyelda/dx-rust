//! Classic / Modern (ours, after Halo 2: Anniversary's switch): one control for every enhancement this rewrite adds.
//!
//! Settings > Enhancements has a row at the top that flips them all, and in play Tab (rebindable: "Classic /
//! Modern") does the same with a cinematic wipe: the frame on screen is frozen, everything switches underneath it,
//! and a glowing edge sweeps across the screen revealing the other look.
//!
//! Going Classic remembers each enhancement's value ("Modern.<key>"), so going back to Modern restores the player's
//! own mix rather than switching on everything; with nothing remembered yet, Modern is the full set (`PRESET`).
//! The visual mods switch too (HDTP, New Vision, Helios, Visible Attachments - not Lay D Denton, GMDX's fixes or
//! Augmentique's outfits): they're chosen when a map loads, so in play a change of them reloads the map in place
//! (a save and a load) under the frozen frame. Fixes, controller and DualSense options, the HUD layout,
//! accessibility (strobe lights) and Discord are left alone: they aren't a look.

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{AsBindGroup, BlendState, Extent3d, RenderPipelineDescriptor, TextureDimension, TextureFormat},
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    shader::ShaderRef,
    ui_render::ui_material::{UiMaterial, UiMaterialKey},
};
use std::sync::Mutex;

/// Every enhancement the switch covers, with its Modern value (Classic is 0 for all: the original game).
pub const PRESET: &[(&str, f32)] = &[
    // Visuals.
    ("AmbientOcclusion", 1.0),
    ("AtmosphericFog", 1.0),
    ("Bloom", 1.0),
    ("Clouds", 1.0),
    ("EnhancedFX", 1.0),
    ("SkeletalCharacters", 1.0),
    ("Ragdolls", 1.0),
    ("FluidBlood", 1.0),
    ("AugGlows", 1.0),
    ("WetBodies", 1.0),
    ("CoatCloth", 1.0),
    ("DiegeticHealing", 1.0),
    ("FirstPersonBody", 1.0),
    ("HurtPoses", 1.0),
    ("BodyWounds", 1.0),
    ("FlashlightCone", 1.0),
    ("FogVolumes", 1.0),
    ("LightHalos", 1.0),
    ("LightShafts", 1.0),
    ("ModernCoronas", 1.0),
    ("ModernVision", 1.0),
    ("ModernWater", 1.0),
    ("PersistentDebris", 1.0),
    ("RealtimeShadows", 2.0),
    ("Skyline", 1.0),
    ("SmoothLipSync", 1.0),
    ("SoftShadows", 1.0),
    ("Weather", 2.0),
    ("WeaponInertia", 1.0),
    // Sound.
    ("HQAudio", 1.0),
    ("RoomReverb", 1.0),
    ("SoundOcclusion", 1.0),
    ("Soundscape", 1.0),
    ("RestrictedCombatMusic", 1.0),
    // Gameplay and AI.
    ("BodyAlarms", 1.0),
    ("SmarterAI", 1.0),
    ("DiegeticScreens", 1.0),
    ("AlertIndicators", 1.0),
    ("BulletPenetration", 1.0),
    ("CarryCover", 1.0),
    ("DTSEnergy", 1.0),
    ("Elements", 1.0),
    ("FloorConsumables", 1.0),
    ("LadderGrab", 1.0),
    ("Mantling", 1.0),
    ("MeleeArc", 1.0),
    ("ModernAim", 1.0),
    ("NPCFiniteAmmo", 1.0),
    ("NotesAutofill", 1.0),
    ("SmartBinoculars", 1.0),
    ("SmoothLean", 1.0),
    ("SurfaceFootsteps", 1.0),
    ("ThirdPerson", 1.0),
    ("ThrownNoise", 1.0),
    // Interface.
    ("AmmoMaxCyan", 1.0),
    ("AmmoTotal", 1.0),
    ("DialogueHotkeys", 1.0),
    ("DropStacks", 1.0),
    ("DuplicateKeys", 1.0),
    ("FasterInfolinks", 1.0),
    ("InventorySort", 1.0),
    ("ItemCounts", 1.0),
    ("PickupSize", 1.0),
    ("SearchedText", 1.0),
    ("ShowMaxStacks", 1.0),
    ("StreamlinedRepairBot", 1.0),
    ("ToolCounts", 1.0),
    // Visual mods (only with the mod installed; applied at map load).
    ("HDTextures", 1.0),
    ("HDModels", 1.0),
    ("HDCharacters", 1.0),
    ("DdsCharacters", 1.0),
    ("DdsItems", 1.0),
    ("DdsWorld", 0.0),
    ("VisAttach", 1.0),
];

/// The visual mods' settings and the mod each needs.
const MOD_KEYS: &[(&str, &str)] = &[
    ("HDTextures", "newvision"),
    ("HDModels", "hdtp"),
    ("HDCharacters", "hdtp"),
    ("DdsCharacters", "helios"),
    ("DdsItems", "helios"),
    ("DdsWorld", "helios"),
    ("VisAttach", "visibleattachments"),
];

/// Keys that only exist with a mod installed (left alone without it).
fn applies(key: &str) -> bool {
    if let Some((_, id)) = MOD_KEYS.iter().find(|m| m.0 == key) {
        return unreal_pkg::mods::active(id).is_some();
    }
    key != "Weather" || crate::weather::available()
}

/// Modern right now. Before the switch was ever used, the look of the settings decides: Modern when most of the
/// enhancements are on.
pub fn is_modern() -> bool {
    if crate::settings::has("ModernMode") {
        return crate::settings::get("ModernMode") > 0.5;
    }
    let keys: Vec<&str> = PRESET.iter().map(|p| p.0).filter(|k| applies(k)).collect();
    let on = keys.iter().filter(|k| crate::settings::get(k) > 0.0).count();
    on * 2 > keys.len()
}

/// The values a switch gives: (key, value) for every covered setting, and the memory to keep ("Modern.<key>",
/// value) - written when going Classic. `current` reads a setting as it stands (the menu passes its unsaved rows).
pub fn switch_values(to_modern: bool, current: impl Fn(&str) -> f32) -> (Vec<(String, f32)>, Vec<(String, f32)>) {
    let mut vals = Vec::new();
    let mut memory = Vec::new();
    let remembered = crate::settings::get("Modern.Saved") > 0.5;
    // Going Classic with nothing on (already classic by hand): keep the remembered mix rather than remember "all
    // off" as the player's Modern.
    let all_off = PRESET.iter().filter(|p| applies(p.0)).all(|p| current(p.0) <= 0.0);
    for &(k, modern) in PRESET.iter().filter(|p| applies(p.0)) {
        if to_modern {
            let mk = format!("Modern.{k}");
            let v = if remembered && crate::settings::has(&mk) { crate::settings::get(&mk) } else { modern };
            vals.push((k.to_string(), v));
        } else {
            if !all_off {
                memory.push((format!("Modern.{k}"), current(k)));
            }
            vals.push((k.to_string(), 0.0));
        }
    }
    if !to_modern && !all_off {
        memory.push(("Modern.Saved".to_string(), 1.0));
    }
    (vals, memory)
}

/// Switch now (the in-game key): settings set and saved. True when a visual mod changed (the map must reload
/// for it to show).
pub fn switch(to_modern: bool) -> bool {
    let (vals, memory) = switch_values(to_modern, crate::settings::get);
    let mods_changed = vals.iter().any(|(k, v)| MOD_KEYS.iter().any(|m| m.0 == k) && crate::settings::get(k) != *v);
    for (k, v) in memory.iter().chain(vals.iter()) {
        crate::settings::set(k, *v);
    }
    crate::settings::set("ModernMode", if to_modern { 1.0 } else { 0.0 });
    crate::settings::save();
    mods_changed
}

/// The save the in-place reload goes through.
fn swap_path() -> std::path::PathBuf {
    crate::save::dir().join("ClassicModernSwap.ron")
}

// ------------------------------------------------------------------------------------------------ the wipe

const WIPE_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6f0d1c55-2b8e-4a7d-9e43-0c1b5a9e7d21");

/// The frozen frame, wiped away by a glowing edge.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct WipeMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub tex: Handle<Image>,
    /// x = progress 0..1, y = seconds, z = aspect (w / h), w = 1 going Modern / 0 going Classic.
    #[uniform(2)]
    pub p: Vec4,
}

impl UiMaterial for WipeMaterial {
    fn fragment_shader() -> ShaderRef {
        WIPE_SHADER.into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        if let Some(t) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)).and_then(|t| t.as_mut()) {
            t.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        }
    }
}

pub fn setup_wipe_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(&WIPE_SHADER, Shader::from_wgsl(include_str!("shaders/wipe.wgsl"), "dxview/wipe.wgsl"));
}

#[derive(Component)]
pub struct WipeNode;

/// The in-place reload behind a visual-mod change.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Swap {
    #[default]
    None,
    /// The save was asked for at this many seconds in.
    Saving(f32),
    /// The map is reloading; settled for this long since it finished.
    Loading(f32),
}

/// A 1-pixel node drawn with the wipe's material all the time (fully wiped: nothing shows), so its pipeline is
/// compiled before the first switch - else the overlay missed its first frames and the switch showed through.
#[derive(Component)]
pub struct WipeWarm;

#[derive(Default)]
pub enum Phase {
    #[default]
    Idle,
    /// The screenshot is on its way (to Modern?).
    Capturing(bool, f32),
    /// The frozen frame covers the screen; t seconds in, `frames` drawn, switched yet, the wipe started at `start`
    /// (once nothing is compiling), and the in-place reload for the visual mods (`Swap`).
    Wiping { t: f32, to_modern: bool, start: Option<f32>, calm: u32, frames: u32, switched: bool, swap: Swap },
}

/// The captured frame, handed over by the screenshot observer.
static CAPTURED: Mutex<Option<Image>> = Mutex::new(None);

/// The wipe: its material, shader, the system, and a render-world count of the pipelines still compiling.
pub struct RemasterPlugin;

impl Plugin for RemasterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::ui_render::UiMaterialPlugin::<WipeMaterial>::default())
            .add_systems(Startup, setup_wipe_shader)
            .add_systems(Update, wipe_system);
        use bevy::render::{Render, RenderApp, RenderSystems};
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.add_systems(Render, count_waiting.in_set(RenderSystems::Cleanup));
        }
    }
}

/// Pipelines the renderer is still compiling (a switch can need new ones for every material: a depth / normal
/// prepass, MSAA off, new materials). The frozen frame stays up until they're ready, so nothing pops in.
static WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn count_waiting(cache: Res<bevy::render::render_resource::PipelineCache>) {
    WAITING.store(cache.waiting_pipelines().count(), std::sync::atomic::Ordering::Relaxed);
}

/// The most the frozen frame is held for pipelines before the wipe goes anyway.
const MAX_HOLD: f32 = 8.0;

/// Hold the frozen frame this long first (the switch rebuilds things - the skyline, clouds, shaders - and any
/// hitch hides under it), then sweep it away over WIPE.
const HOLD: f32 = 0.12;
const WIPE: f32 = 1.1;

#[allow(clippy::too_many_arguments)]
pub fn wipe_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    game_time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut phase: Local<Phase>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<WipeMaterial>>,
    nodes: Query<(Entity, &MaterialNode<WipeMaterial>), With<WipeNode>>,
    warm: Query<(), With<WipeWarm>>,
    ph: Option<Res<crate::ai::PlayerHealth>>,
    (mut assets, mut layers, windows, mut words, mut font, mut vis): (
        Option<ResMut<crate::weapons::RuntimeAssets>>,
        ResMut<Assets<crate::classic_hud::HudLayer>>,
        Query<&Window>,
        Local<Option<Entity>>,
        Local<Option<Option<std::sync::Arc<crate::uicanvas::UiFont>>>>,
        Query<&mut Visibility>,
    ),
) {
    let dt = time.delta_secs().min(0.1);
    // The blue LOADING over the frozen frame while the switch hitches (compiling, reloading), gone just before
    // the wipe starts.
    let hitching = matches!(&*phase, Phase::Wiping { start: None, switched: true, .. });
    if hitching {
        if words.is_none() {
            if let (Some(a), Some(win)) = (assets.as_mut(), windows.iter().next()) {
                *words = crate::loader::spawn_loading_words(&mut commands, a, &mut images, &mut layers, win, &mut *font, 5001);
                if let Some(e) = *words {
                    commands.entity(e).insert(crate::loader::Persist);
                }
            }
        } else if let (Some(e), Phase::Wiping { t, .. }) = (*words, &*phase) {
            // A slow blink (on 0.7 s, off 0.25 s) while it waits.
            if let Ok(mut v) = vis.get_mut(e) {
                *v = if (*t % 0.95) < 0.7 { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
    } else if let Some(e) = words.take() {
        commands.entity(e).despawn();
    }
    if warm.is_empty() {
        let img = images.add(Image::new(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![0, 0, 0, 0],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ));
        let mat = mats.add(WipeMaterial { tex: img, p: Vec4::new(1.0, 0.0, 1.0, 0.0) });
        commands.spawn((
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(1.0), height: Val::Px(1.0), ..default() },
            MaterialNode(mat),
            WipeWarm,
        ));
    }
    match &mut *phase {
        Phase::Idle => {
            // DXVIEW_REMASTER_AT=secs: (test shots) switch once at that time.
            // DXVIEW_REMASTER_AT="secs,secs,...": (tests) switch at those game times.
            let auto = std::env::var("DXVIEW_REMASTER_AT").ok().is_some_and(|v| {
                static DONE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let times: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                let i = DONE.load(std::sync::atomic::Ordering::Relaxed);
                let due = times.get(i).is_some_and(|&at| game_time.elapsed_secs() >= at);
                if due {
                    DONE.store(i + 1, std::sync::atomic::Ordering::Relaxed);
                }
                due
            });
            let free = !crate::loader::busy()
                && !crate::menus::open()
                && !crate::inventory_screen::screen_open()
                && !crate::computers::open()
                && !crate::credits::open();
            if auto && std::env::var_os("DXVIEW_REMASTER_NOWIPE").is_some() {
                eprintln!("REMASTER switch (no wipe)");
                let _ = switch(!is_modern());
                return;
            }
            if (keys.just_pressed(KeyCode::Tab) && free) || auto {
                eprintln!("REMASTER capture");
                if let Ok(mut c) = CAPTURED.lock() {
                    *c = None;
                }
                commands.spawn(Screenshot::primary_window()).observe(|cap: On<ScreenshotCaptured>| {
                    if let Ok(mut c) = CAPTURED.lock() {
                        *c = Some(cap.image.clone());
                    }
                });
                *phase = Phase::Capturing(!is_modern(), 0.0);
            }
        }
        Phase::Capturing(to_modern, waited) => {
            let to_modern = *to_modern;
            *waited += dt;
            let img = CAPTURED.lock().ok().and_then(|mut c| c.take());
            let Some(img) = img else {
                // No frame came back: switch without the wipe.
                if *waited > 1.0 {
                    let _ = switch(to_modern);
                    *phase = Phase::Idle;
                }
                return;
            };
            // The swapchain's format (BGRA, maybe HDR alpha): to plain sRGB RGBA, opaque.
            let Ok(dynimg) = img.try_into_dynamic() else {
                let _ = switch(to_modern);
                *phase = Phase::Idle;
                return;
            };
            let rgba = dynimg.to_rgba8();
            let (w, h) = rgba.dimensions();
            let mut px = rgba.into_raw();
            for a in px.iter_mut().skip(3).step_by(4) {
                *a = 255;
            }
            let frozen = images.add(Image::new(
                Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                TextureDimension::D2,
                px,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            ));
            let mat = mats.add(WipeMaterial { tex: frozen, p: Vec4::new(0.0, 0.0, w as f32 / h.max(1) as f32, to_modern as u8 as f32) });
            eprintln!("REMASTER frame {w}x{h}, wiping");
            commands.spawn((
                Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                MaterialNode(mat),
                GlobalZIndex(5000),
                WipeNode,
                crate::loader::Persist,
            ));
            crate::sound::play_player(
                if to_modern { "DeusExSounds.Augmentation.CloakUp" } else { "DeusExSounds.Augmentation.CloakDown" },
                1.0,
                0.0,
            );
            *phase = Phase::Wiping { t: 0.0, to_modern, start: None, calm: 0, frames: 0, switched: false, swap: Swap::None };
        }
        Phase::Wiping { t, to_modern, start, calm, frames, switched, swap } => {
            *t += dt;
            *frames += 1;
            // The switch waits for the frozen frame to be on screen (its texture uploaded, drawn a frame).
            if !*switched {
                if *frames >= 3 {
                    *switched = true;
                    *t = 0.0;
                    let dead = ph.as_ref().is_some_and(|p| p.dead);
                    if switch(*to_modern) && crate::save::can_save(dead) && !crate::loader::busy() {
                        // The visual mods take effect at map load: save and load in place, under the frozen frame.
                        let _ = std::fs::remove_file(swap_path());
                        crate::save::request(swap_path(), "Classic / Modern");
                        *swap = Swap::Saving(*t);
                        eprintln!("REMASTER reloading the map for the visual mods");
                    }
                }
                return;
            }
            match *swap {
                Swap::None => {}
                Swap::Saving(t0) => {
                    if swap_path().exists() {
                        crate::save::load(std::path::Path::new(""), &swap_path());
                        *swap = Swap::Loading(0.0);
                    } else if *t - t0 > 3.0 {
                        // Couldn't save here: the mods show from the next map.
                        *swap = Swap::None;
                    }
                    return;
                }
                Swap::Loading(settled) => {
                    if crate::loader::busy() {
                        *swap = Swap::Loading(0.0);
                    } else if settled < 0.2 {
                        *swap = Swap::Loading(settled + dt);
                    } else {
                        let _ = std::fs::remove_file(swap_path());
                        *swap = Swap::None;
                        *calm = 0;
                        *t = 0.0;
                    }
                    return;
                }
            }
            if start.is_none() {
                // Wait for the switch's pipelines (a few quiet frames in a row), at least HOLD, at most MAX_HOLD.
                *calm = if WAITING.load(std::sync::atomic::Ordering::Relaxed) == 0 { *calm + 1 } else { 0 };
                if (*t >= HOLD && *calm >= 2) || *t >= MAX_HOLD {
                    *start = Some(*t);
                }
            }
            let k = start.map_or(0.0, |s0| ((*t - s0) / WIPE).clamp(0.0, 1.0));
            // Ease in and out.
            let prog = k * k * (3.0 - 2.0 * k);
            for (_, m) in &nodes {
                if let Some(mut m) = mats.get_mut(&m.0) {
                    m.p.x = prog;
                    m.p.y = *t;
                    m.p.w = *to_modern as u8 as f32;
                }
            }
            if k >= 1.0 {
                eprintln!("REMASTER wipe done");
                for (e, m) in &nodes {
                    if let Some(mm) = mats.get(&m.0) {
                        let tex = mm.tex.clone();
                        images.remove(&tex);
                    }
                    mats.remove(&m.0);
                    commands.entity(e).despawn();
                }
                *phase = Phase::Idle;
            }
        }
    }
}
