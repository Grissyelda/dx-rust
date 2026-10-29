//! Player settings (the original's DeusEx.ini / User.ini values the menus edit), kept in settings.txt beside
//! the exe as `Key=Value` lines. Keys are the MenuChoice class names without the prefix (ObjectNames,
//! MusicVolume...) plus our own (HUDScale). Values are the choice's value as the menu stores it (enum index or
//! slider value). Game systems read them through the typed helpers below.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

static VALUES: RwLock<Option<BTreeMap<String, f32>>> = RwLock::new(None);
/// Bumped on every change so systems can re-apply cheaply.
static GENERATION: AtomicU64 = AtomicU64::new(1);

/// (key, default) for every setting; defaults are the original's (MenuChoice_* defaultValue, player defaults).
pub const DEFAULTS: &[(&str, f32)] = &[
    // Game options (MenuScreenOptions). Enabled/Disabled enums: 0 = Enabled.
    ("ObjectNames", 0.0),
    ("WeaponAutoReload", 0.0),
    ("GoreLevel", 1.0),
    ("Subtitles", 1.0),
    ("Crosshairs", 0.0),
    ("HUDAugDisplay", 1.0),
    ("UIBackground", 0.0),
    ("HeadBob", 0.0),
    ("LogTimeoutValue", 3.0),
    // Display.
    ("TextureColorBits", 1.0),
    ("WorldTextureDetail", 2.0),
    ("ObjectTextureDetail", 1.0),
    ("DetailTextures", 1.0),
    ("Decals", 1.0),
    ("Resolution", -1.0),
    ("FullScreen", 0.0),
    // Ours: 0 full screen, 1 windowed, 2 borderless.
    ("DisplayMode", 1.0),
    // Ours: vertical sync (1 on, 0 off).
    ("VSync", 1.0),
    ("AdjustBrightness", 0.5),
    // HUD size (ours): 0 small, 1 medium (the original's rule), 2 large.
    ("HUDScale", 1.0),
    // Sound.
    ("MusicVolume", 153.0),
    ("SoundVolume", 204.0),
    ("SpeechVolume", 255.0),
    ("EffectsChannels", 16.0),
    ("SampleRate", 2.0),
    ("SoundQuality", 1.0),
    ("ReverseStereo", 0.0),
    ("SurroundSound", 1.0),
    ("Use3DHardware", 1.0),
    // Controls.
    ("AlwaysRun", 0.0),
    ("ToggleCrouch", 1.0),
    ("InvertMouse", 1.0),
    ("MouseSensitivity", 3.0),
    // Colors.
    ("MenuTranslucency", 1.0),
    ("MenuColor", 10.0),
    ("HUDBordersVisible", 1.0),
    ("HUDBorderTranslucency", 1.0),
    ("HUDTranslucency", 1.0),
    ("HUDColor", 8.0),
    // Ours: the HUD / inventory screens take the menu theme's colours (1) until a HUD theme is picked (0).
    ("HUDFollowsMenu", 1.0),
    // DeusExPlayer.bAskedToTrain.
    ("AskedToTrain", 0.0),
    // DeusExPlayer.bConfirmSaveDeletes (0 = on).
    ("ConfirmSaveDeletes", 0.0),
    // Ours (Enhancements): developer cheats and readouts, 1 = enabled.
    ("DebugMode", 0.0),
    // DeusExPlayer.bQuotesEnabled (globalconfig): the credits' QUOTES easter egg, then the quotes after the credits.
    ("QuotesEnabled", 0.0),
    // Ours (Enhancements, gamepad): glyph set 0 auto / 1 Xbox / 2 PlayStation; stick deadzone and right-stick look
    // speed (steps 0..4); look invert; aim slowdown on enemies; D-pad aug hotkey pages instead of belt stepping.
    ("PadGlyphs", 0.0),
    ("PadDeadzone", 2.0),
    ("PadLookSensitivity", 2.0),
    ("PadInvertLook", 0.0),
    ("PadAimSlowdown", 0.0),
    ("PadAugPages", 0.0),
    // Controller prompts: the hint bar on free-cursor screens (on), the Use glyph beside frob labels (off).
    ("PadHints", 1.0),
    ("PadFrobPrompt", 0.0),
    // Enhancement: Joe Wintergreen-style lip sync (smoother mouth tweens, visible blinks). On by default.
    ("SmoothLipSync", 1.0),
    // Small v2 enhancements (all off unless noted).
    ("NPCFiniteAmmo", 0.0),
    ("FloorConsumables", 0.0),
    ("LadderGrab", 0.0),
    ("NotesAutofill", 0.0),
    ("DialogueHotkeys", 0.0),
    ("InventorySort", 0.0),
    ("PersistentDebris", 0.0),
    ("DiscordPresence", 1.0),
    // v2 medium enhancements (all off unless noted).
    ("DTSEnergy", 0.0),
    ("BodyAlarms", 0.0),
    ("ThrownNoise", 0.0),
    ("SurfaceFootsteps", 0.0),
    ("WeaponInertia", 0.0),
    ("FlashlightCone", 0.0),
    ("BulletPenetration", 0.0),
    ("SmoothLean", 0.0),
    ("MeleeArc", 0.0),
    // GMDX bugfixes: the ones that change nothing but the bug are on, the ones that change the balance off.
    ("FixSkillAwardOnce", 1.0),
    ("FixTurretAnimals", 1.0),
    ("FixBlastDoors", 0.0),
    ("FixGibLoot", 0.0),
    ("FixCarryJump", 0.0),
    // GMDX quality of life (all off).
    ("ToolCounts", 0.0),
    ("ItemCounts", 0.0),
    ("DuplicateKeys", 0.0),
    ("SearchedText", 0.0),
    ("RestrictedCombatMusic", 0.0),
    ("NoStrobeLights", 0.0),
    ("AmmoTotal", 0.0),
    ("AmmoMaxCyan", 0.0),
    ("ShowMaxStacks", 0.0),
    ("PickupSize", 0.0),
    ("StreamlinedRepairBot", 0.0),
    ("SmartBinoculars", 0.0),
    ("FasterInfolinks", 0.0),
    ("DropStacks", 0.0),
    // Mods (used only while the mod is on in the launcher): New Vision textures, HDTP item / decoration models,
    // HDTP characters. Taken up when the next map loads.
    ("HDTextures", 1.0),
    ("HDModels", 1.0),
    ("HDCharacters", 1.0),
    // Augmentique: NPC outfits (iEquipNPCs: 0 none, 1 all but unique characters, 2 everyone); every outfit open
    // from the start even when GMDX Augmented's maps can be searched for them.
    ("AugNPCs", 1.0),
    ("AugAllOutfits", 0.0),
    // Lay D Denton: Human.bGaveNewGameTips (the checkbox tip is shown once).
    ("LDDPGaveNewGameTips", 0.0),
    // DualSense (ROADMAP Phase 0.3): light bar 0 off / 1 health; player LEDs 0 energy meter / 1 player one / 2 off;
    // adaptive triggers 0 off / 1 light / 2 full; haptics 0 classic rumble / 1 HD (USB); rumble strength 0 off ..
    // 3 strong (every pad); infolinks through the pad speaker (USB) and its volume 0..2.
    ("PadLightbar", 1.0),
    ("PadPlayerLeds", 0.0),
    ("PadTriggers", 2.0),
    ("PadHaptics", 1.0),
    // Footsteps through the haptics (on).
    ("PadHapticSteps", 1.0),
    ("PadRumble", 2.0),
    ("PadSpeaker", 0.0),
    ("PadSpeakerVolume", 1.0),
];

fn path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("settings.txt")))
        .unwrap_or_else(|| "settings.txt".into())
}

fn ensure() {
    if VALUES.read().map(|v| v.is_some()).unwrap_or(false) {
        return;
    }
    let mut m: BTreeMap<String, f32> = DEFAULTS.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    if let Ok(t) = std::fs::read_to_string(path()) {
        for line in t.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if let Ok(v) = v.trim().parse::<f32>() {
                    m.insert(k.trim().to_string(), v);
                }
            }
        }
    }
    if let Ok(mut w) = VALUES.write() {
        *w = Some(m);
    }
}

pub fn get(key: &str) -> f32 {
    ensure();
    VALUES
        .read()
        .ok()
        .and_then(|v| v.as_ref().and_then(|m| m.get(key).copied()))
        .or_else(|| DEFAULTS.iter().find(|d| d.0 == key).map(|d| d.1))
        .unwrap_or(0.0)
}

pub fn default_of(key: &str) -> f32 {
    DEFAULTS.iter().find(|d| d.0 == key).map(|d| d.1).unwrap_or(0.0)
}

pub fn set(key: &str, v: f32) {
    ensure();
    if let Ok(mut w) = VALUES.write() {
        if let Some(m) = w.as_mut() {
            if m.get(key) != Some(&v) {
                m.insert(key.to_string(), v);
                GENERATION.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Write settings.txt (SaveConfig).
pub fn save() {
    ensure();
    let Ok(r) = VALUES.read() else { return };
    let Some(m) = r.as_ref() else { return };
    let mut out = String::from("# Deus Ex rewrite settings (written by the Settings menus)\n");
    for (k, v) in m {
        out.push_str(&format!("{k}={v}\n"));
    }
    let _ = std::fs::write(path(), out);
}

// ---------------------------------------------------------------- typed views

fn enabled(key: &str) -> bool {
    get(key) < 0.5
}
fn on(key: &str) -> bool {
    get(key) >= 0.5
}

pub fn object_names() -> bool {
    enabled("ObjectNames")
}
pub fn auto_reload() -> bool {
    enabled("WeaponAutoReload")
}
pub fn low_gore() -> bool {
    get("GoreLevel") < 0.5
}
pub fn subtitles() -> bool {
    on("Subtitles")
}
pub fn crosshairs() -> bool {
    get("Crosshairs") < 0.5
}
pub fn head_bob() -> bool {
    enabled("HeadBob")
}
pub fn log_timeout() -> f32 {
    get("LogTimeoutValue")
}
pub fn always_run() -> bool {
    enabled("AlwaysRun")
}
pub fn toggle_crouch() -> bool {
    enabled("ToggleCrouch")
}
pub fn invert_mouse() -> bool {
    enabled("InvertMouse")
}
/// PlayerPawn.MouseSensitivity (default 3).
pub fn mouse_sensitivity() -> f32 {
    get("MouseSensitivity")
}
pub fn menu_translucent() -> bool {
    on("MenuTranslucency")
}
pub fn hud_borders() -> bool {
    on("HUDBordersVisible")
}
pub fn hud_border_translucent() -> bool {
    on("HUDBorderTranslucency")
}
pub fn hud_translucent() -> bool {
    on("HUDTranslucency")
}
/// UIBackground: 0 render the game (masked), 1 snapshot, 2 black.
pub fn ui_background() -> u8 {
    get("UIBackground") as u8
}
/// Volumes 0..1.
pub fn music_volume() -> f32 {
    get("MusicVolume") / 255.0
}
pub fn sound_volume() -> f32 {
    get("SoundVolume") / 255.0
}
pub fn speech_volume() -> f32 {
    get("SpeechVolume") / 255.0
}
pub fn reverse_stereo() -> bool {
    on("ReverseStereo")
}
/// Renderer gamma for the Brightness slider (0..1, default 0.5 = 1.15).
pub fn gamma() -> f32 {
    0.8 + 0.7 * get("AdjustBrightness")
}
/// Debug Mode (Enhancements): the spawn menu, debug unlock, god mode and the corner readout.
pub fn debug_mode() -> bool {
    on("DebugMode")
}
/// HUD size step: -1 small, 0 medium, +1 large.
pub fn hud_scale_step() -> i32 {
    get("HUDScale").round() as i32 - 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn defaults_match_the_original_menus() {
        assert_eq!(super::default_of("MusicVolume"), 153.0);
        assert_eq!(super::default_of("MouseSensitivity"), 3.0);
        assert_eq!(super::default_of("LogTimeoutValue"), 3.0);
    }
}
