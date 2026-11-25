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
    ("PosableBodies", 1.0),
    ("SmoothAnimation", 1.0),
    ("FluidBlood", 1.0),
    ("RemasteredLighting", 1.0),
    ("EmissiveSurfaces", 1.0),
    ("BounceLight", 1.0),
    ("Tonemapping", 1.0),
    ("Reflections", 1.0),
    ("EyeAdaptation", 1.0),
    ("AugGlows", 1.0),
    ("DiegeticHealing", 1.0),
    ("FirstPersonBody", 1.0),
    ("HurtPoses", 1.0),
    ("BodyWounds", 1.0),
    ("Dismemberment", 1.0),
    ("FlashlightCone", 1.0),
    ("MaterialSheen", 1.0),
    ("ModernGlass", 1.0),
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
    ("RestrictedCombatMusic", 0.0),
    // Gameplay and AI.
    ("BodyAlarms", 1.0),
    ("SmarterAI", 1.0),
    ("DiegeticScreens", 1.0),
    ("AlertIndicators", 1.0),
    ("HudFade", 1.0),
    ("ConversationLight", 1.0),
    ("Grass", 2.0),
    ("Trees", 1.0),
    ("BulletPenetration", 1.0),
    ("CarryCover", 1.0),
    ("DTSEnergy", 1.0),
    ("Elements", 1.0),
    ("FloorConsumables", 1.0),
    ("LadderGrab", 1.0),
    ("Mantling", 1.0),
    ("Sprint", 1.0),
    ("MeleeArc", 1.0),
    ("MeleeBash", 1.0),
    ("ModernAim", 1.0),
    ("DiegeticScope", 1.0),
    ("NotesAutofill", 1.0),
    ("SmartBinoculars", 1.0),
    ("SmoothLean", 1.0),
    ("SurfaceFootsteps", 1.0),
    ("ThirdPerson", 1.0),
    ("ThirdPersonCover", 1.0),
    ("ThrownNoise", 1.0),
    ("StrongBodyThrow", 1.0),
    // Interface.
    ("AmmoMaxCyan", 1.0),
    ("AmmoTotal", 1.0),
    ("DialogueHotkeys", 1.0),
    ("DropStacks", 1.0),
    ("DuplicateKeys", 1.0),
    ("FasterInfolinks", 1.0),
    ("InventorySort", 1.0),
    ("InventoryRotate", 1.0),
    ("ItemCounts", 1.0),
    ("PickupSize", 1.0),
    ("SearchedText", 1.0),
    ("ShowMaxStacks", 1.0),
    ("StreamlinedRepairBot", 1.0),
    ("ToolCounts", 1.0),
    // Visual mods (only with the mod installed; applied at map load).
    ("HDTextures", 1.0),
    ("HDModels", 1.0),
    ("HDCharacters", 0.0),
    ("DdsCharacters", 1.0),
    ("DdsItems", 1.0),
    ("DdsWorld", 0.0),
    ("VisAttach", 1.0),
    // The optional bug fixes (the bigger gameplay changes stay as the player set them).
    ("OptionalFixes", 1.0),
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

/// A setting only the Modern look or a mod has (left out of Android's Enhancements: Android is Classic only).
pub fn modern_only(key: &str) -> bool {
    PRESET.iter().any(|p| p.0 == key) || MOD_KEYS.iter().any(|m| m.0 == key) || key.starts_with("Modern")
}

/// Android: every Modern feature off for this run (Classic only), whatever the settings file says.
pub fn force_classic() {
    for (k, _) in PRESET {
        crate::settings::set(k, 0.0);
    }
    crate::settings::set("ModernMode", 0.0);
}

/// Modern right now. Before the switch was ever used, the look of the settings decides: Modern when most of the
/// enhancements are on.
pub fn is_modern() -> bool {
    if crate::settings::has("ModernMode") {
        return setting!("ModernMode") > 0.5;
    }
    let keys: Vec<&str> = PRESET.iter().map(|p| p.0).filter(|k| applies(k)).collect();
    let on = keys.iter().filter(|k| crate::settings::get(k) > 0.0).count();
    on * 2 > keys.len()
}

/// Playing Modern with a settings file from before a feature joined the preset: the new feature is on (in memory
/// until a menu saves), as it would have been had the switch been made after it existed. Only keys the file doesn't
/// mention at all - a feature the player turned off stays off.
pub fn fill_new_modern_keys() {
    // Coat Cloth left the Modern mix (2026-11-06: the coats looked better swaying as one piece): switched off once
    // for players who had it from the preset; a player who turns it back on keeps it.
    if !crate::settings::has("CoatClothRetired") {
        crate::settings::set("CoatClothRetired", 1.0);
        crate::settings::set("CoatCloth", 0.0);
        if crate::settings::has("Modern.CoatCloth") {
            crate::settings::set("Modern.CoatCloth", 0.0);
        }
        crate::settings::save();
    }
    // NPC Ammo Runs Out is the original's rule, so it is on in Classic too (2026-11-10: guards never ran dry or
    // went to their knives): it left the Classic / Modern switch and is switched on once for everyone.
    if !crate::settings::has("NPCFiniteAmmoDefault") {
        crate::settings::set("NPCFiniteAmmoDefault", 1.0);
        crate::settings::set("NPCFiniteAmmo", 1.0);
        crate::settings::save();
    }
    // Lean Updates (work that changes nothing on screen) and Static Batching are on by default since 2026-11-22;
    // switched on once for settings files written before (they carry LeanUpdates=0); a player can turn them off.
    if !crate::settings::has("PerfDefaults1") {
        crate::settings::set("PerfDefaults1", 1.0);
        crate::settings::set("LeanUpdates", 1.0);
        crate::settings::set("StaticBatching", 1.0);
        crate::settings::save();
    }
    if !crate::settings::has("ModernMode") || !is_modern() {
        return;
    }
    for (k, v) in PRESET {
        if !crate::settings::has(k) && applies(k) {
            crate::settings::set(k, *v);
        }
    }
}

/// The values a switch gives: (key, value) for every covered setting, and the memory to keep ("Modern.<key>",
/// value) - written when going Classic. `current` reads a setting as it stands (the menu passes its unsaved rows).
pub fn switch_values(to_modern: bool, current: impl Fn(&str) -> f32) -> (Vec<(String, f32)>, Vec<(String, f32)>) {
    let mut vals = Vec::new();
    let mut memory = Vec::new();
    let remembered = setting!("Modern.Saved") > 0.5;
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
    // Skeletal characters switch HDTP's people off (hdtp::characters_on), so with HDTP that is a reload too.
    let mods_changed = vals.iter().any(|(k, v)| {
        (MOD_KEYS.iter().any(|m| m.0 == k)
            || crate::relight::RELOAD_KEYS.contains(&k.as_str())
            || (k == "SkeletalCharacters" && unreal_pkg::mods::active("hdtp").is_some()))
            && crate::settings::get(k) != *v
    });
    for (k, v) in memory.iter().chain(vals.iter()) {
        crate::settings::set(k, *v);
    }
    crate::settings::set("ModernMode", if to_modern { 1.0 } else { 0.0 });
    crate::settings::save();
    mods_changed
}

/// The save the in-place reload goes through.
/// 0 idle, 1 asked, 2 saving, 3 loading.
static RELOAD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Rebuild the map being played in place (save, then load that save), without the wipe: multiplayer uses it when a
/// room's look turns mods or the visual mod settings on or off while a map is up. Nothing happens on the logo map
/// or where the game can't save (the next map load picks the change up there).
pub fn request_reload() {
    if !crate::menus::intro_map() {
        let _ = RELOAD.compare_exchange(0, 1, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
    }
}

fn reload_system(time: Res<Time<Real>>, mut since: Local<f32>) {
    use std::sync::atomic::Ordering::Relaxed;
    match RELOAD.load(Relaxed) {
        1 => {
            if crate::loader::busy() {
                return;
            }
            if !crate::save::can_save(false) {
                RELOAD.store(0, Relaxed);
                return;
            }
            let _ = std::fs::remove_file(swap_path());
            crate::save::request(swap_path(), "Multiplayer look");
            *since = 0.0;
            RELOAD.store(2, Relaxed);
        }
        2 => {
            *since += time.delta_secs();
            if swap_path().exists() {
                crate::save::load(std::path::Path::new(""), &swap_path());
                RELOAD.store(3, Relaxed);
            } else if *since > 3.0 {
                RELOAD.store(0, Relaxed);
            }
        }
        3 => {
            if !crate::loader::busy() {
                let _ = std::fs::remove_file(swap_path());
                RELOAD.store(0, Relaxed);
            }
        }
        _ => {}
    }
}

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
    Wiping { t: f32, to_modern: bool, start: Option<f32>, calm: u32, frames: u32, switched: bool, swap: Swap, since: f32 },
}

/// True while the frozen frame still hides the world (the switch, the reload behind it, the warm-up). Systems that
/// ease things in over time (shadow lights, halos, fog) jump to where they're going instead, so nothing fades or
/// pops in once the wipe shows the new look.
static COVERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn covered() -> bool {
    COVERED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Systems still building something the new look shows (the grass field, a view model) call this each frame
/// they aren't ready; the wipe waits for a few frames without it.
static NOT_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn not_ready() {
    if covered() {
        NOT_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The captured frame, handed over by the screenshot observer.
static CAPTURED: Mutex<Option<Image>> = Mutex::new(None);

/// The wipe: its material, shader, the system, and a render-world count of the pipelines still compiling.
pub struct RemasterPlugin;

impl Plugin for RemasterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::ui_render::UiMaterialPlugin::<WipeMaterial>::default())
            .add_systems(Startup, setup_wipe_shader)
            .add_systems(Update, (wipe_system, reload_system));
        use bevy::render::{Render, RenderApp, RenderSystems};
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.add_systems(Render, count_waiting.in_set(RenderSystems::Cleanup));
        }
    }
}

/// Pipelines the renderer is still compiling (a switch can need new ones for every material: a depth / normal
/// prepass, MSAA off, new materials). The frozen frame stays up until they're ready, so nothing pops in.
static WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Render pipelines still compiling (last render frame).
pub fn pipelines_waiting() -> usize {
    WAITING.load(std::sync::atomic::Ordering::Relaxed)
}

/// Pipelines that have sat in the queue past STUCK_SECS without starting to compile: their shader never loaded
/// (Bevy keeps such a pipeline queued for ever). They will never finish, so they don't hold the frozen frame or the
/// loading screen - on the PC a few of these kept the Classic -> Modern switch waiting for its caps (6 s + 8 s).
static STUCK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Every pipeline the cache has made (grows with each new material / view variant).
static TOTAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Labels of the stuck pipelines (for the profiler / the REMASTER log).
static STUCK_LABELS: Mutex<String> = Mutex::new(String::new());
const STUCK_SECS: f32 = 2.5;

/// (pipelines made, still compiling, stuck in the queue) - for the profiler.
pub fn pipeline_stats() -> (usize, usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (TOTAL.load(Relaxed), WAITING.load(Relaxed), STUCK.load(Relaxed))
}

/// The labels of the pipelines stuck in the queue.
pub fn stuck_labels() -> String {
    STUCK_LABELS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn count_waiting(
    cache: Res<bevy::render::render_resource::PipelineCache>,
    mut seen: Local<bevy::platform::collections::HashMap<usize, std::time::Instant>>,
    mut last_label: Local<Option<std::time::Instant>>,
) {
    use bevy::render::render_resource::{CachedPipelineState, PipelineDescriptor};
    use std::sync::atomic::Ordering::Relaxed;
    let now = std::time::Instant::now();
    let waiting: Vec<usize> = cache.waiting_pipelines().collect();
    seen.retain(|id, _| waiting.contains(id));
    let all: Vec<_> = cache.pipelines().collect();
    let (mut live, mut stuck) = (0, Vec::new());
    for id in waiting {
        let t0 = *seen.entry(id).or_insert(now);
        // A pipeline compiling (Creating) is making progress however long it takes; one still Queued after
        // STUCK_SECS is waiting for a shader that isn't coming.
        let queued = all.get(id).is_none_or(|p| matches!(p.state, CachedPipelineState::Queued));
        if queued && now.duration_since(t0).as_secs_f32() > STUCK_SECS {
            stuck.push(id);
        } else {
            live += 1;
        }
    }
    WAITING.store(live, Relaxed);
    STUCK.store(stuck.len(), Relaxed);
    TOTAL.store(all.len(), Relaxed);
    if !stuck.is_empty() && last_label.is_none_or(|t| now.duration_since(t).as_secs_f32() > 1.0) {
        *last_label = Some(now);
        let mut s = String::new();
        for id in stuck.iter().take(8) {
            let name = match all.get(*id).map(|p| &p.descriptor) {
                Some(PipelineDescriptor::RenderPipelineDescriptor(d)) => {
                    format!("{} [{:?}]", d.label.as_deref().unwrap_or("?"), d.vertex.shader.path())
                }
                Some(PipelineDescriptor::ComputePipelineDescriptor(d)) => format!("{} (compute)", d.label.as_deref().unwrap_or("?")),
                None => "?".into(),
            };
            if !s.is_empty() {
                s.push_str("; ");
            }
            s.push_str(&name);
        }
        if let Ok(mut l) = STUCK_LABELS.lock() {
            *l = s;
        }
    }
}

/// The most the frozen frame is held for pipelines / builders after the switch (or the reload) before the wipe goes
/// anyway.
const MAX_HOLD: f32 = 4.0;

/// Hold the frozen frame this long first (the switch rebuilds things - the skyline, clouds, shaders - and any
/// hitch hides under it), then sweep it away over WIPE.
const HOLD: f32 = 0.12;
/// After an in-place reload: this much game time runs under the frozen frame once everything is compiled, so the
/// new map's first frames (shadow lights picked, halos traced, people posed, fog filled) are behind it.
const WARM_RELOAD: f32 = 0.6;
/// Frames in a row with nothing compiling and nobody building before the wipe goes.
const CALM_FRAMES: u32 = 4;
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
                *words = crate::loader::spawn_loading_words(&mut commands, a, &mut images, &mut layers, win, &mut font, 5001);
                if let Some(e) = *words {
                    commands.entity(e).insert(crate::loader::Persist);
                }
            }
        } else if let (Some(e), Phase::Wiping { t, .. }) = (*words, &*phase) {
            // A slow blink (on 0.7 s, off 0.25 s) while it waits.
            if let Ok(mut v) = vis.get_mut(e) {
                v.set_if_neq(if (*t % 0.95) < 0.7 { Visibility::Inherited } else { Visibility::Hidden });
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
            let auto = env_val!("DXVIEW_REMASTER_AT").map(str::to_owned).is_some_and(|v| {
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
            if auto && env_on!("DXVIEW_REMASTER_NOWIPE") {
                eprintln!("REMASTER switch (no wipe)");
                let _ = switch(!is_modern());
                return;
            }
            // Multiplayer: the room sets the look for everyone, and Tab is the scoreboard there (scoreboard.rs).
            let free = free && !crate::netplay::look_locked();
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
            *phase = Phase::Wiping { t: 0.0, to_modern, start: None, calm: 0, frames: 0, switched: false, swap: Swap::None, since: 0.0 };
        }
        Phase::Wiping { t, to_modern, start, calm, frames, switched, swap, since } => {
            *t += dt;
            *since += time.delta_secs();
            *frames += 1;
            COVERED.store(start.is_none(), std::sync::atomic::Ordering::Relaxed);
            // The switch waits for the frozen frame to be on screen (its texture uploaded, drawn a frame).
            if !*switched {
                if *frames >= 3 {
                    *switched = true;
                    *t = 0.0;
                    let dead = ph.as_ref().is_some_and(|p| p.dead);
                    let reload = switch(*to_modern);
                    eprintln!("REMASTER {:.2} s: settings switched to {}", *since, if *to_modern { "Modern" } else { "Classic" });
                    if reload && crate::save::can_save(dead) && !crate::loader::busy() {
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
                Swap::Loading(_) => {
                    // The loader's own settle (time paused until the new map's pipelines are built) is the wait;
                    // then the warm-up below runs with time going.
                    if crate::loader::busy() || crate::loader::settling() || crate::save::restoring() {
                        *swap = Swap::Loading(0.0);
                    } else {
                        eprintln!("REMASTER {:.2} s: map reloaded and settled", *since);
                        let _ = std::fs::remove_file(swap_path());
                        *swap = Swap::None;
                        *calm = 0;
                        *t = -(WARM_RELOAD - HOLD);
                    }
                    return;
                }
            }
            if start.is_none() {
                // Wait for the switch's pipelines and builders (a few quiet frames in a row), at least HOLD (plus
                // the warm-up after a reload), at most MAX_HOLD.
                let busy = WAITING.load(std::sync::atomic::Ordering::Relaxed) > 0 || NOT_READY.swap(false, std::sync::atomic::Ordering::Relaxed);
                *calm = if busy { 0 } else { *calm + 1 };
                if (*t >= HOLD && *calm >= CALM_FRAMES) || *t >= MAX_HOLD {
                    eprintln!(
                        "REMASTER {:.2} s: wipe starts ({}; {} pipelines made, {} stuck)",
                        *since,
                        if *t >= MAX_HOLD { "gave up waiting" } else { "all ready" },
                        TOTAL.load(std::sync::atomic::Ordering::Relaxed),
                        STUCK.load(std::sync::atomic::Ordering::Relaxed)
                    );
                    if STUCK.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                        eprintln!("REMASTER stuck pipelines: {}", stuck_labels());
                    }
                    *start = Some(*t);
                    COVERED.store(false, std::sync::atomic::Ordering::Relaxed);
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
                eprintln!("REMASTER {:.2} s: wipe done", *since);
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
