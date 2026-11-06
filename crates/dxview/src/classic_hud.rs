//! The original HUD (DeusExHUD): body/health and bioenergy display (HUDHitDisplay), compass
//! (HUDCompassDisplay), ammo display (HUDAmmoDisplay) and the object belt (HUDObjectBelt / HUDObjectSlot),
//! drawn with the original art, fonts and default colour theme.
//!
//! Each window is ported from its DrawWindow / Tick code onto a `uicanvas::Canvas` and shown as one UI node.
//! Scaling: whole multiples of a 600-line screen (2x at 1080p), nearest-neighbour, so the art stays crisp;
//! upscaled art can replace the textures later without touching the layout.

use crate::ai::{PlayerHealth, P_ARM_L, P_ARM_R, P_HEAD, P_LEG_L, P_LEG_R, P_TORSO};
use crate::pickups::PlayerInventory;
use crate::uicanvas::{color_scaled, Canvas, HAlign, Rgb, Style, UiFont, UiTex, VAlign};
use crate::weapons::{Clips, LockText, RuntimeAssets, WState, Weapons};
use crate::LevelRes;
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendState, Extent3d, RenderPipelineDescriptor, TextureDimension,
        TextureFormat,
    },
    shader::ShaderRef,
    ui_render::ui_material::{UiMaterial, UiMaterialKey},
};
use imsim_core::inventory::ItemKind;
use std::collections::HashMap;
use std::sync::Arc;
use unreal_pkg::props::{find, Value};

// Default HUD colour theme (ColorThemeHUD_Default) and the windows' own colours.
const COL_ARMOR: Rgb = [255, 255, 255];
const COL_TICKS: Rgb = [200, 200, 200];
const COL_AMMO: Rgb = [0, 255, 0];
const COL_AMMO_LOW: Rgb = [255, 0, 0];
/// HUDObjectSlot: colObjectNum = NormalText, colSelected = Background x 0.5.
const COL_SLOT_TEXT: Rgb = [200, 200, 200];
const COL_SLOT_SELECTED: Rgb = [63, 63, 63];
const COL_SELECTION_BORDER: Rgb = [255, 255, 255];

const UI: &str = "DeusExUI.UserInterface.";

pub const HUD_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("3b7c9e21-5a4d-4f0e-8d62-91c0a7e5f3b4");

/// A HUD window's picture: premultiplied, composited over the view with premultiplied-alpha blending
/// (additive where the original drew DSTY_Translucent).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct HudLayer {
    #[texture(0)]
    #[sampler(1)]
    pub tex: Handle<Image>,
    /// x = the Brightness setting's gamma, applied to the UI too (the original's brightness lifts the whole
    /// screen, menus included); y = opacity (HUD customisation's fade, 1 = as drawn).
    #[uniform(2)]
    pub gamma: Vec4,
}

impl HudLayer {
    pub fn new(tex: Handle<Image>) -> HudLayer {
        HudLayer { tex, gamma: Vec4::new(ui_gamma(), 1.0, 0.0, 0.0) }
    }
}

/// The gamma the menus and HUD are drawn with (same as the 3D view).
pub fn ui_gamma() -> f32 {
    env_val!("DXVIEW_GAMMA").map(str::to_owned).and_then(|v| v.parse().ok()).unwrap_or_else(crate::settings::gamma)
}

/// Brightness changes reach every UI layer.
pub fn ui_gamma_system(mut layers: ResMut<Assets<HudLayer>>, mut gen: Local<u64>) {
    let g = crate::settings::generation();
    if g == *gen {
        return;
    }
    *gen = g;
    let v = ui_gamma();
    let ids: Vec<_> = layers.ids().collect();
    for id in ids {
        if let Some(mut l) = layers.get_mut(id) {
            if l.gamma.x != v {
                l.gamma.x = v;
            }
        }
    }
}

impl UiMaterial for HudLayer {
    fn fragment_shader() -> ShaderRef {
        HUD_SHADER.into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        if let Some(t) = descriptor
            .fragment
            .as_mut()
            .and_then(|f| f.targets.get_mut(0))
            .and_then(|t| t.as_mut())
        {
            t.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        }
    }
}

pub fn setup_hud_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        &HUD_SHADER,
        Shader::from_wgsl(
            include_str!("shaders/hud_layer.wgsl"),
            "dxview/hud_layer.wgsl",
        ),
    );
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Win {
    Hit,
    Compass,
    Ammo,
    Belt,
    Log,
    Damage,
    Received,
    /// HUDActiveItemsDisplay: the augmentation column and the running charged items.
    Active,
    /// HUDInfoLinkDisplay.
    InfoLink,
    /// The InfoLink's portrait with its StaticWindow interference on a window of its own (Lean Updates): the one
    /// part of the InfoLink that changes every frame.
    InfoLinkFace,
    /// HUDConWindowFirst.
    ConFirst,
    /// ConWindowActive's letterbox: upper and lower bars.
    ConUpper,
    ConLower,
    /// HUDBarkDisplay.
    Bark,
    /// The mouse cursor while a third-person conversation offers choices.
    ConCursor,
    /// HUDMissionStartTextDisplay: the map's startupMessage lines typed out mid-screen.
    Start,
    /// HUDInformationDisplay: a datacube's, book's or newspaper's text (info.rs).
    Info,
    /// A message that stays up (the death prompt), above the belt.
    Msg,
    /// The FPS counter (fps.rs), top right.
    Fps,
    /// Debug Mode's corner readout (main.rs overlay_update), top left.
    Debug,
    /// Multiplayer (ours): the news of the game (joined, left, went down, chat) at the top, and the chat line
    /// being typed under it.
    Net,
}

/// The Debug Mode readout's text (None = hidden).
static DEBUG_TEXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_debug_text(t: Option<String>) {
    if let Ok(mut g) = DEBUG_TEXT.lock() {
        if *g != t {
            *g = t;
        }
    }
}

/// The readout's lines, each long one broken at its " | " separators to fit `width` units.
fn debug_lines(f: &crate::uicanvas::UiFont, text: &str, width: i32) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if f.width(line) <= width {
            out.push(line.to_string());
            continue;
        }
        let mut cur = String::new();
        for part in line.split(" | ") {
            let next = if cur.is_empty() { part.to_string() } else { format!("{cur} | {part}") };
            if !cur.is_empty() && f.width(&next) > width {
                out.push(std::mem::take(&mut cur));
                cur = part.to_string();
            } else {
                cur = next;
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// HUDMissionStartTextDisplay state: the message, characters shown so far, seconds left once it's all out.
static START: std::sync::Mutex<Option<(String, f32, f32)>> = std::sync::Mutex::new(None);

/// MissionScript.FirstFrame: AddMessage for each startupMessage line, then StartMessage.
pub fn start_text(lines: &[String]) {
    let msg = lines.iter().filter(|l| !l.is_empty()).cloned().collect::<Vec<_>>().join("|n");
    if !msg.is_empty() {
        if let Ok(mut s) = START.lock() {
            *s = Some((msg, 0.0, 5.0));
        }
        START_CENTRED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// MissionEndgame.PrintEndgameQuote: the same window centred on the screen, shown for `secs`.
static START_CENTRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn quote_text(lines: &[String], secs: f32) {
    let msg = lines.iter().filter(|l| !l.is_empty()).cloned().collect::<Vec<_>>().join("|n");
    if let Ok(mut s) = START.lock() {
        *s = Some((msg, 0.0, secs));
    }
    START_CENTRED.store(true, std::sync::atomic::Ordering::Relaxed);
}

impl Win {
    const ALL: [Win; 21] = [
        Win::Hit,
        Win::Compass,
        Win::Ammo,
        Win::Belt,
        Win::Log,
        Win::Damage,
        Win::Received,
        Win::Active,
        Win::InfoLink,
        Win::InfoLinkFace,
        Win::ConFirst,
        Win::ConUpper,
        Win::ConLower,
        Win::Bark,
        Win::ConCursor,
        Win::Start,
        Win::Info,
        Win::Msg,
        Win::Fps,
        Win::Debug,
        Win::Net,
    ];
    /// Fixed window sizes in UI units (InitWindow SetSize / ParentRequestedPreferredSize); the log is sized
    /// by the layout.
    fn size(self) -> (i32, i32) {
        match self {
            Win::Hit => (84, 106),
            Win::Compass => (73, 40),
            Win::Ammo => (95, 77),
            Win::Belt => (541, 69),
            Win::Log
            | Win::Received
            | Win::Active
            | Win::ConFirst
            | Win::ConUpper
            | Win::ConLower
            | Win::Bark
            | Win::Start
            | Win::Info
            | Win::Msg
            | Win::Fps
            | Win::Debug
            | Win::Net => (0, 0),
            Win::ConCursor | Win::InfoLinkFace => (64, 64),
            Win::InfoLink => crate::conui::INFOLINK_SIZE,
            // DamageHUDDisplay: arrowiconWidth + iconMargin, (iconHeight + iconMargin) x 7 + arrowiconHeight.
            Win::Damage => (
                DMG_ARROW + DMG_MARGIN,
                (DMG_ICON + DMG_MARGIN) * 7 + DMG_ARROW,
            ),
        }
    }
}

const DMG_ICON: i32 = 24;
const DMG_MARGIN: i32 = 2;
const DMG_ARROW: i32 = 64;
const DMG_FADE: f32 = 2.0;

/// HUDLogDisplay (logTimeout 3 s, maxLogLines 4 from User.ini; MinLogLines 4).
/// LogTimeoutValue (Game Options, default 3 s).
fn log_timeout() -> f32 {
    crate::settings::log_timeout()
}
const LOG_LINES: usize = 4;
const LOG_MARGIN: i32 = 10;
const LOG_TOP: i32 = 13;
const LOG_MIN_HEIGHT: i32 = 40;

/// Items taken from a body (DeusExCarcass.AddReceivedItem -> HUDReceivedDisplay.AddItem), by class.
static RECEIVED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// ClientMessage: a line for the message log (several can arrive in one frame).
static MESSAGES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn client_message(msg: &str) {
    if let Ok(mut m) = MESSAGES.lock() {
        m.push(msg.to_string());
    }
}

/// Show an item in the "Received:" window.
pub fn received(class: &str) {
    if let Ok(mut r) = RECEIVED.lock() {
        r.push(class.to_string());
    }
}

/// HUDReceivedDisplay: displayLength 3 s, topMargin 5, tile margins 10, spacing 4.
const RECEIVED_TIME: f32 = 3.0;

/// DamageHUDDisplay.iconInfo: damage types, icon, colour.
const DAMAGE_ICONS: [(&[&str], Option<&str>, Rgb); 7] = [
    (&["Shocked"], Some("DamageIconElectricity"), [0, 196, 255]),
    (&["EMP"], Some("DamageIconEMP"), [0, 196, 255]),
    (
        &["Burned", "Flamed", "Exploded"],
        Some("DamageIconFire"),
        [255, 96, 0],
    ),
    (
        &["PoisonGas", "TearGas"],
        Some("DamageIconGas"),
        [0, 196, 0],
    ),
    (
        &["Drowned", "HalonGas"],
        Some("DamageIconOxygen"),
        [0, 128, 255],
    ),
    (&["Radiation"], Some("DamageIconRadiation"), [255, 255, 0]),
    (&["Shot", "Sabot"], None, [255, 0, 0]),
];

#[derive(Default, Clone, Copy)]
struct DamageIcon {
    active: bool,
    age: f32,
    /// Yaw of the hit offset, 0..65536 (0 = in front).
    dir: i32,
    center: bool,
}

#[derive(Component)]
pub struct HudWindow(Win);

/// HUDHitDisplay.BodyPart: flash white when hurt (0.4 s), count up when healed (1 s).
#[derive(Default, Clone, Copy)]
struct BodyPart {
    last: i32,
    heal_from: i32,
    shown: i32,
    damage_counter: f32,
    heal_counter: f32,
    started: bool,
}

const DAMAGE_FLASH: f32 = 0.4;
const HEAL_FLASH: f32 = 1.0;

impl BodyPart {
    /// SetHitColor.
    fn color(&mut self, dt: f32, value: i32) -> Rgb {
        if !self.started {
            // First tick: no flash for the starting health.
            self.started = true;
            self.last = value;
            self.shown = value;
        }
        self.damage_counter = (self.damage_counter - dt).max(0.0);
        self.heal_counter = (self.heal_counter - dt).max(0.0);
        if value < self.last {
            self.damage_counter = DAMAGE_FLASH;
            self.shown = value;
        } else if value > self.last {
            self.heal_counter = HEAL_FLASH;
            self.heal_from = self.shown;
        }
        self.last = value;
        self.shown = if self.heal_counter > 0.0 {
            let m = self.heal_counter / HEAL_FLASH;
            value + ((self.heal_from - value) as f32 * m) as i32
        } else {
            value
        };
        let mut c = color_scaled(self.shown as f32 / 100.0);
        if self.damage_counter > 0.0 {
            let m = self.damage_counter / DAMAGE_FLASH;
            for v in &mut c {
                *v = (*v as f32 + (255.0 - *v as f32) * m) as u8;
            }
        }
        c
    }
}

#[derive(Resource, Default)]
pub struct ClassicHud {
    textures: HashMap<String, Option<Arc<UiTex>>>,
    fonts: HashMap<String, Option<Arc<UiFont>>>,
    parts: [BodyPart; 6],
    /// This frame's part colours (BodyPart.color ticks its flash; worked out before the redraw check).
    hit_cols: [Rgb; 6],
    images: HashMap<Win, (Handle<Image>, i32)>,
    last: HashMap<Win, Vec<u8>>,
    /// What a window was last drawn from, for windows that only change with their inputs.
    keys: HashMap<Win, u64>,
    /// The InfoLink's frame (conui::draw_infolink_frame) at a size, scale and settings generation, drawn once: with
    /// its portrait on a window of its own the InfoLink is redrawn from a copy of it as its text types.
    infolink_frame: Option<(u64, Canvas)>,
    true_north: Option<i32>,
    defaults: HashMap<String, Vec<unreal_pkg::props::Property>>,
    /// Message log lines (text, age) and seconds since the last one.
    log: Vec<(String, f32)>,
    since_log: f32,
    seen_msg: (String, f32),
    damage: [DamageIcon; 7],
    absorb: f32,
    /// Received items (icon path, label) and seconds since the last one arrived.
    received: Vec<(Option<String>, String)>,
    received_t: f32,
}

impl ClassicHud {
    /// HUDLogDisplay.AddLog: a new message (wrapped later to the window width), the oldest lines drop off.
    fn add_log(&mut self, msg: &str) {
        crate::persona::log_message(msg);
        for line in msg.lines().filter(|l| !l.trim().is_empty()) {
            self.log.push((line.to_string(), 0.0));
        }
        self.since_log = 0.0;
    }

    /// DamageHUDDisplay.AddIcon.
    fn add_damage(&mut self, damage_type: &str, offset: glam::DVec3) {
        let t = if matches!(damage_type, "Poison" | "PoisonEffect") {
            "PoisonGas"
        } else {
            damage_type
        };
        for (i, (types, _, _)) in DAMAGE_ICONS.iter().enumerate() {
            if types.contains(&t) {
                let d = &mut self.damage[i];
                d.active = true;
                d.age = 0.0;
                d.center = offset == glam::DVec3::ZERO;
                d.dir = ((offset.y.atan2(offset.x) / std::f64::consts::TAU * 65536.0).round()
                    as i32)
                    .rem_euclid(65536);
            }
        }
    }
}

impl ClassicHud {
    pub(crate) fn tex(&mut self, assets: &mut RuntimeAssets, path: &str) -> Option<Arc<UiTex>> {
        // A hit (nearly always) costs no copy of the key.
        if let Some(t) = self.textures.get(path) {
            return t.clone();
        }
        self.textures
            .entry(path.to_string())
            .or_insert_with(|| {
                let fem = crate::travel::ui_tex(path);
                fem.as_deref()
                    .and_then(|f| assets.lib.load_image_masked(f))
                    .or_else(|| assets.lib.load_image_masked(path))
                    .map(|t| Arc::new(UiTex::from_rgba(t.width, t.height, &t.mips[0])))
            })
            .clone()
    }
    pub(crate) fn ui(&mut self, assets: &mut RuntimeAssets, name: &str) -> Option<Arc<UiTex>> {
        thread_local! { static BUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) }; }
        BUF.with(|b| {
            let mut b = b.borrow_mut();
            b.clear();
            b.push_str(UI);
            b.push_str(name);
            self.tex(assets, &b)
        })
    }
    pub(crate) fn font(&mut self, assets: &mut RuntimeAssets, name: &str) -> Option<Arc<UiFont>> {
        if let Some(f) = self.fonts.get(name) {
            return f.clone();
        }
        let f = load_font(assets, name).map(Arc::new);
        if f.is_none() {
            warn!("HUD font {name} not found");
        }
        self.fonts.insert(name.to_string(), f.clone());
        f
    }
    pub(crate) fn defaults(
        &mut self,
        assets: &mut RuntimeAssets,
        class: &str,
    ) -> &[unreal_pkg::props::Property] {
        if !self.defaults.contains_key(class) {
            self.defaults.insert(class.to_string(), assets.classes.defaults(class));
        }
        &self.defaults[class]
    }
    pub(crate) fn str_default(
        &mut self,
        assets: &mut RuntimeAssets,
        class: &str,
        prop: &str,
    ) -> Option<String> {
        match find(self.defaults(assets, class), prop) {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        }
    }
    pub(crate) fn path_default(
        &mut self,
        assets: &mut RuntimeAssets,
        class: &str,
        prop: &str,
    ) -> Option<String> {
        match find(self.defaults(assets, class), prop) {
            Some(Value::Path(s)) if !s.ends_with(".None") => Some(s.clone()),
            _ => None,
        }
    }
}

/// A Font from DeusExUI.u with its page textures.
pub(crate) fn load_font(assets: &mut RuntimeAssets, name: &str) -> Option<UiFont> {
    // "Package.Font" names a font in a texture package (DXFonts.MainMenuTrueType) or a code package
    // (Engine.LargeFont in System/Engine.u); a bare name is in DeusExUI.u.
    let (pkg_name, name, root) = match name.split_once('.') {
        Some((p, n)) if unreal_pkg::paths::exists(&assets.lib.system_dir().join(format!("{p}.u"))) => {
            (p.to_string(), n, assets.lib.system_dir().join(format!("{p}.u")))
        }
        Some((p, n)) => {
            let dir = assets.lib.system_dir().parent().map(|r| r.join("Textures")).unwrap_or_default();
            let file = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|f| f.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(p)))?;
            (p.to_string(), n, file)
        }
        None => ("DeusExUI".to_string(), name, assets.lib.system_dir().join("DeusExUI.u")),
    };
    let pkg = unreal_pkg::Package::open(&root).ok()?;
    let e = pkg
        .exports
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(name) && pkg.class_name(e.class_index) == "Font")?;
    let font = unreal_pkg::objects::read_font(&pkg, e).ok()?;
    let mut pages = Vec::new();
    for (tref, _) in &font.pages {
        let path = format!("{pkg_name}.{}", pkg.object_path(*tref));
        let t = assets.lib.load_image_masked(&path)?;
        pages.push(Arc::new(UiTex::from_rgba(t.width, t.height, &t.mips[0])));
    }
    Some(UiFont::new(font, pages))
}

pub fn setup_classic_hud(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<HudLayer>>,
) {
    let hud = spawn_classic_hud(&mut commands, &mut images, &mut materials);
    commands.insert_resource(hud);
}

/// Spawn the four HUD windows (their pictures are filled in by `classic_hud_system`).
pub fn spawn_classic_hud(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    materials: &mut Assets<HudLayer>,
) -> ClassicHud {
    let mut hud = ClassicHud::default();
    for w in Win::ALL {
        let img = blank_image(1, 1);
        let h = images.add(img);
        hud.images.insert(w, (h.clone(), 0));
        commands.spawn((
            MaterialNode(materials.add(HudLayer::new(h))),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            GlobalZIndex(-2),
            // The windows share a GlobalZIndex, so ZIndex decides: the InfoLink's portrait lies over the InfoLink.
            ZIndex(if matches!(w, Win::InfoLinkFace | Win::Fps | Win::Debug | Win::Net) { 1 } else { 0 }),
            HudWindow(w),
        ));
    }
    hud
}

fn hkey<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn blank_image(w: u32, h: u32) -> Image {
    let mut img = Image::new_fill(
        Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    img.sampler = ImageSampler::nearest();
    img
}

/// The weapon in hand, as the ammo display and belt see it.
#[derive(Clone, Default)]
pub struct HeldView {
    class: String,
    ammo_class: Option<String>,
    reload_count: u32,
    /// Rounds in the gun (Clips).
    loaded: i32,
    reloading: bool,
    can_track: bool,
    /// A pickup in hand (medkit, tool), not a weapon: no ammo display.
    item: bool,
}

/// What the HUD shows this frame, gathered from the game state.
#[derive(Default)]
pub struct Snapshot {
    parts: [i32; 6],
    energy: f32,
    breath: Option<f32>,
    /// Sprint stamina 0..1 (stamina.rs), when Sprint is on and not under water.
    stamina: Option<f32>,
    swim_time: f32,
    /// Unreal yaw units.
    yaw: i32,
    dead: bool,
    held: Option<HeldView>,
    pending: Option<usize>,
    loaded_ammo: HashMap<String, String>,
    lock_text: String,
    lock_color: Rgb,
    /// Installed toggleable augs by hotkey: (hotkey, small icon, active).
    augs: Vec<(i32, String, bool)>,
    /// Running charged pickups: (class, charge fraction).
    items: Vec<(String, f32)>,
}

/// This frame's inputs to the windows' layout and drawing (DeusExHUD.ConfigurationChanged): screen size, what the
/// conversation / cut-scene state hides or moves, and the text, fonts and sizes of the text windows.
struct HudFrame {
    s: i32,
    sw: i32,
    sh: i32,
    hit_h: i32,
    compass_h: i32,
    con: crate::conui::ConView,
    third: bool,
    menu: bool,
    first_w: i32,
    first_h: i32,
    up_h: i32,
    low_h: i32,
    bark_w: i32,
    bark_h: i32,
    win_pos: f32,
    rec_head: Option<Arc<UiFont>>,
    rec_small: Option<Arc<UiFont>>,
    rec_any: bool,
    rec_w: i32,
    rec_h: i32,
    start_font: Option<Arc<UiFont>>,
    start: Option<(String, f32, f32)>,
    start_lines: Vec<String>,
    start_w: i32,
    start_h: i32,
    /// The information window's text id (crate::info::shown), when one is up.
    info_id: Option<u64>,
    info_font: Option<Arc<UiFont>>,
    info_lines: Vec<(u8, [u8; 3], Vec<String>)>,
    info_w: i32,
    info_h: i32,
    stay_msg: Option<String>,
    msg_font: Option<Arc<UiFont>>,
    fps_text: Option<String>,
    fps_w: i32,
    fps_h: i32,
    debug_lines: Vec<String>,
    debug_w: i32,
    debug_h: i32,
    net_lines: Vec<(String, [u8; 3])>,
    net_w: i32,
    net_h: i32,
    msg_w: i32,
    msg_h: i32,
    ammo_visible: bool,
    log_font: Option<Arc<UiFont>>,
    log_w: i32,
    log_lines: Vec<String>,
    log_h: i32,
    log_visible: bool,
    damage_visible: bool,
    face_split: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn classic_hud_system(
    time: Res<Time>,
    window: Single<&Window>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut ph: ResMut<PlayerHealth>,
    weapons: Res<Weapons>,
    clips: Res<Clips>,
    inv: Res<PlayerInventory>,
    level: Res<LevelRes>,
    cam: Single<&Transform, With<crate::FlyCam>>,
    lock: Single<(&Text, &TextColor), With<LockText>>,
    (mut nodes, mut layers): (
        Query<(
            &HudWindow,
            &mut Node,
            &mut Visibility,
            &MaterialNode<HudLayer>,
        )>,
        ResMut<Assets<HudLayer>>,
    ),
    (mut swim_clock, mut snap_keep): (Local<f32>, Local<Snapshot>),
    (mut msg, charged): (
        ResMut<crate::movers::HudMessage>,
        Res<crate::charged::ActiveCharged>,
    ),
) {
    let _prof = crate::prof::scope("classic_hud::classic_hud_system");
    let dt = time.delta_secs();
    let hud = &mut *hud;
    let assets = &mut *assets;
    let s = crate::hud::ui_scale(&window) as i32;
    let (sw, sh) = (
        window.physical_width() as i32 / s,
        window.physical_height() as i32 / s,
    );
    let _to_logical = |v: i32| v as f32 * s as f32 / window.scale_factor();

    if hud.true_north.is_none() {
        let n = level.0.level_info()
            .and_then(|a| match find(&a.props, "TrueNorth") {
                Some(Value::Int(n)) => Some(*n),
                _ => None,
            });
        hud.true_north = Some(n.unwrap_or(0));
    }
    let fwd = cam.forward();
    let yaw = ((fwd.z.atan2(fwd.x) as f64) / std::f64::consts::TAU * 65536.0).round() as i32;
    if ph.breath.is_some() {
        *swim_clock += dt;
    }
    gather_snapshot(&mut snap_keep, &ph, &weapons, &clips, lock.0, lock.1, &charged, yaw, *swim_clock);
    let snap = &*snap_keep;
    take_messages(hud, assets, &mut msg, &mut ph, dt);
    let f = hud_frame(hud, assets, &msg, snap, s, sw, sh, dt);

    let tiny = hud.font(assets, "FontTiny");
    for w in Win::ALL {
        let (x, y, ww, wh, visible) = f.layout(w, snap, *window);
        // Enhancement: HUD customisation (each part hidden, sized and faded on its own).
        let (show, k, alpha) = hud_part(w);
        // Combat HUD (hudfade.rs): the status windows fade out of combat.
        let alpha = if matches!(w, Win::Hit | Win::Compass | Win::Ammo | Win::Belt | Win::Active) { alpha * crate::hudfade::alpha() } else { alpha };
        let visible = visible && show;
        if !visible {
            hide_window(&mut nodes, w);
            continue;
        }
        let _wprof = crate::prof::scope(match w {
            Win::Hit => "hud::Hit",
            Win::Compass => "hud::Compass",
            Win::Ammo => "hud::Ammo",
            Win::Belt => "hud::Belt",
            Win::Log => "hud::Log",
            Win::Damage => "hud::Damage",
            Win::Received => "hud::Received",
            Win::Active => "hud::Active",
            Win::InfoLink => "hud::InfoLink",
            Win::InfoLinkFace => "hud::InfoLinkFace",
            Win::ConFirst => "hud::ConFirst",
            Win::ConUpper => "hud::ConUpper",
            Win::ConLower => "hud::ConLower",
            Win::Bark => "hud::Bark",
            Win::ConCursor => "hud::ConCursor",
            Win::Start => "hud::Start",
            Win::Info => "hud::Info",
            Win::Msg => "hud::Msg",
            Win::Fps => "hud::Fps",
            Win::Debug => "hud::Debug",
            Win::Net => "hud::Net",
        });
        // Windows that only change with what they show are redrawn only when that changes.
        let key = window_key(w, &f, snap, hud, &inv, ww, wh, y, dt);
        // A settings change (theme, translucency, borders) redraws everything.
        let key = key.map(|k| k ^ crate::settings::generation().wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let unchanged = key.is_some() && hud.keys.get(&w) == key.as_ref() && hud.last.contains_key(&w);
        if !unchanged {
            if let Some(k) = key {
                hud.keys.insert(w, k);
            } else {
                hud.keys.remove(&w);
            }
            let mut canvas = window_canvas(hud, assets, w, &f, ww, wh);
            draw_window(w, hud, assets, &mut canvas, &f, snap, &inv, tiny.as_deref(), (ww, wh, y), dt);
            upload_window(hud, &mut images, &mut nodes, &mut layers, w, canvas, ww, wh, s);
        }
        place_window(&mut nodes, &mut layers, w, visible, (x, y, ww, wh), k, alpha, &f, *window);
    }
}

/// The snapshot the windows draw from: health, energy, breath, the weapon in hand, the lock line, augs and charged
/// items.
#[allow(clippy::too_many_arguments)]
fn gather_snapshot(
    snap: &mut Snapshot,
    ph: &PlayerHealth,
    weapons: &Weapons,
    clips: &Clips,
    lock_text: &Text,
    lock_color: &TextColor,
    charged: &crate::charged::ActiveCharged,
    yaw: i32,
    swim_time: f32,
) {
    let p = &ph.parts;
    // The snapshot the windows draw from: kept between frames and updated in place (strings and lists copied only
    // when they changed, so a steady HUD allocates nothing).
    snap.parts = [P_HEAD, P_TORSO, P_ARM_L, P_ARM_R, P_LEG_L, P_LEG_R].map(|i| p[i].max(0.0).round() as i32);
    snap.energy = ph.energy;
    snap.breath = ph.breath;
    snap.stamina = if ph.breath.is_some() { None } else { crate::stamina::fraction() };
    snap.swim_time = swim_time;
    snap.yaw = yaw;
    snap.dead = ph.dead;
    match weapons.held.as_ref() {
        Some(h) => {
            let v = snap.held.get_or_insert_with(HeldView::default);
            if v.class != h.stats.class {
                v.class.clone_from(&h.stats.class);
            }
            if v.ammo_class != h.stats.ammo_class {
                v.ammo_class.clone_from(&h.stats.ammo_class);
            }
            v.reload_count = h.stats.reload_count;
            v.loaded = *clips.0.get(&h.stats.class).unwrap_or(&0) as i32;
            v.reloading = h.state == WState::Reloading;
            v.can_track = h.stats.can_track;
            v.item = h.stats.item || h.stats.class == "POVCorpse";
        }
        None => snap.held = None,
    }
    snap.pending = weapons.pending;
    if snap.loaded_ammo != weapons.loaded_ammo {
        snap.loaded_ammo.clone_from(&weapons.loaded_ammo);
    }
    if snap.lock_text != lock_text.0 {
        snap.lock_text.clone_from(&lock_text.0);
    }
    snap.lock_color = {
        let c = lock_color.0.to_srgba();
        [(c.red * 255.0) as u8, (c.green * 255.0) as u8, (c.blue * 255.0) as u8]
    };
    // bHUDShowAllAugs: every installed aug that can be toggled, lit while on (by hotkey).
    {
        let g = crate::abilities::lock();
        let mut n = 0;
        if let Some(a) = g.as_ref() {
            // (Augmentation Display "Active Only": just the running ones, as AddAugmentationDisplay adds them.)
            let all = crate::settings::hud_show_all_augs();
            for x in a.augs.list.iter().filter(|x| x.has && !x.def.always_active && (all || x.active)) {
                if n == snap.augs.len() {
                    snap.augs.push((0, String::new(), false));
                }
                let e = &mut snap.augs[n];
                e.0 = x.hotkey;
                if e.1 != x.def.small_icon {
                    e.1.clone_from(&x.def.small_icon);
                }
                e.2 = x.active;
                n += 1;
            }
        }
        snap.augs.truncate(n);
        snap.augs.sort_by_key(|x| x.0);
    }
    snap.items.truncate(charged.0.len());
    for (k, c) in charged.0.iter().enumerate() {
        let f = (c.charge / c.max.max(1.0)).clamp(0.0, 1.0);
        match snap.items.get_mut(k) {
            Some(e) => {
                if e.0 != c.class {
                    e.0.clone_from(&c.class);
                }
                e.1 = f;
            }
            None => snap.items.push((c.class.clone(), f)),
        }
    }
}

/// HUDLogDisplay / DamageHUDDisplay / HUDReceivedDisplay inputs: new messages to the log, damage icons, received
/// items, and their timers.
fn take_messages(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    msg: &mut crate::movers::HudMessage,
    ph: &mut PlayerHealth,
    dt: f32,
) {
    // Player messages (ClientMessage) go to the message log; the death prompt and keypad entry line stay where
    // they are. A message is new when its text changes or its timer is set again.
    if (msg.text != hud.seen_msg.0 || msg.time_left > hud.seen_msg.1 + 0.01)
        && !msg.text.is_empty() && msg.time_left > 1.0 && msg.time_left <= 100.0 {
            let t = msg.text.clone();
            hud.add_log(&t);
            msg.time_left = 0.0;
        }
    if hud.seen_msg.0 != msg.text {
        hud.seen_msg.0.clone_from(&msg.text);
    }
    hud.seen_msg.1 = msg.time_left;
    for m in MESSAGES
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        hud.add_log(&m);
    }
    hud.since_log += dt;
    hud.log.iter_mut().for_each(|l| l.1 += dt);
    hud.log.retain(|l| l.1 <= log_timeout());
    for (t, off) in std::mem::take(&mut ph.damage_icons) {
        hud.add_damage(&t, off);
    }
    if let Some(a) = ph.absorb.take() {
        hud.absorb = a;
    }
    for d in hud.damage.iter_mut().filter(|d| d.active) {
        d.age += dt;
    }
    for class in RECEIVED
        .lock()
        .map(|mut r| std::mem::take(&mut *r))
        .unwrap_or_default()
    {
        let icon = hud.path_default(assets, &class, "Icon");
        let label = hud
            .str_default(assets, &class, "beltDescription")
            .unwrap_or_default();
        hud.received.push((icon, label));
        hud.received_t = 0.0;
    }
    hud.received_t += dt;
    if hud.received_t > RECEIVED_TIME {
        hud.received.clear();
    }
}

/// DeusExHUD.ConfigurationChanged: what hides or moves the windows this frame and the text windows' contents and
/// sizes (the mission start text types on here).
#[allow(clippy::too_many_arguments)]
fn hud_frame(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    msg: &crate::movers::HudMessage,
    snap: &Snapshot,
    s: i32,
    sw: i32,
    sh: i32,
    dt: f32,
) -> HudFrame {
    let rec_head = hud.font(assets, "FontMenuHeaders_DS");
    let rec_small = hud.font(assets, "FontMenuSmall_DS");
    let rec_any = !hud.received.is_empty();
    let (rec_w, rec_h) = match (&rec_head, &rec_small) {
        (Some(h), Some(sm)) => received_size(h, sm, &hud.received),
        _ => (0, 0),
    };

    // DeusExHUD.ConfigurationChanged.
    let hit_h = Win::Hit.size().1;
    let compass_h = Win::Compass.size().1;
    // Persona screens and third-person conversations hide the HUD.
    let con = crate::conui::view();
    // (A cut scene is the same: Paralyzed with the letterbox, HUD hidden.)
    let third = con.play.as_ref().is_some_and(|p| !p.first_person) || crate::cinematic::letterbox();
    let menu = crate::inventory_screen::screen_open() || third || crate::cinematic::transit();
    let first_w = (sw - 100).min(800);
    let first_h = match &con.play {
        Some(p) if p.first_person => crate::conui::first_height(hud, assets, p, first_w),
        _ => 0,
    };
    let cine = crate::cinematic::letterbox() || con.play.as_ref().is_some_and(|p| p.forced);
    let (up_h, low_h) = if cine {
        let b = crate::conui::cinematic_letterbox(sw, sh);
        (b, b)
    } else {
        crate::conui::letterbox(sh)
    };
    // HUDBarkDisplay: where the first-person conversation window goes, above it while that's up.
    let bark_w = (sw - 100).min(800);
    let bark_h = if con.barks.is_empty() {
        0
    } else {
        crate::conui::bark_height(hud, assets, &con.barks, bark_w)
    };
    let win_pos = if crate::cinematic::letterbox() { 1.0 } else { con.play.as_ref().map_or(0.0, |p| p.win_pos) };
    let (start_font, start, start_lines, start_w, start_h) = start_text_window(hud, assets, dt);
    let (info_id, info_font, info_lines, info_w, info_h) = info_window(hud, assets, sw);
    // A message that stays up: the death prompt.
    let stay_msg = (msg.time_left > 100.0 && !msg.text.is_empty()).then(|| msg.text.clone());
    let msg_font = hud.font(assets, "FontMenuSmall_DS");
    let fps_text = crate::fps::text().filter(|t| !t.is_empty());
    let (fps_w, fps_h) = match (&fps_text, &msg_font) {
        (Some(t), Some(f)) => (f.width(t) + 2, f.height + 2),
        _ => (1, 1),
    };
    let debug_text = DEBUG_TEXT.lock().ok().and_then(|g| g.clone());
    let debug_lines: Vec<String> = match (&debug_text, &msg_font) {
        (Some(t), Some(f)) => debug_lines(f, t, sw - 16),
        _ => Vec::new(),
    };
    let (debug_w, debug_h) = match &msg_font {
        Some(f) if !debug_lines.is_empty() => (
            debug_lines.iter().map(|l| f.width(l)).max().unwrap_or(1) + 2,
            debug_lines.len() as i32 * f.height + 2,
        ),
        _ => (1, 1),
    };
    let net_lines = crate::netplay::hud_lines();
    let (net_w, net_h) = match &msg_font {
        Some(f) if !net_lines.is_empty() => (
            net_lines.iter().map(|(l, _)| f.width(l)).max().unwrap_or(1).min(sw - 40) + 4,
            net_lines.len() as i32 * (f.height + 1) + 2,
        ),
        _ => (1, 1),
    };
    let (msg_w, msg_h) = match (&stay_msg, &msg_font) {
        (Some(t), Some(f)) => (f.width(t) + 2, f.height + 2),
        _ => (1, 1),
    };
    let ammo_visible = snap.held.as_ref().is_some_and(|h| !h.item) && !snap.dead && !menu;
    let log_font = hud.font(assets, "FontMenuSmall_DS");
    // msgLog: right of the hit display, as wide as the screen allows (no active items column yet).
    let log_w = sw - Win::Hit.size().0 - 40;
    let log_lines = log_font
        .as_ref()
        .map(|f| wrap_log(f, &hud.log, log_w - (32 + LOG_MARGIN * 3) - LOG_MARGIN))
        .unwrap_or_default();
    let line_h = log_font.as_ref().map_or(11, |f| f.height);
    let log_h =
        (LOG_TOP + line_h * log_lines.len().max(LOG_LINES) as i32 + LOG_MARGIN).max(LOG_MIN_HEIGHT);
    let log_visible = !hud.log.is_empty() && hud.since_log <= log_timeout() && !snap.dead;
    let damage_visible = hud.damage.iter().any(|d| d.active) && !snap.dead;
    // Lean Updates: the InfoLink's portrait and its interference on a window of their own (Win::InfoLinkFace), laid
    // over the InfoLink, so the rest of it is redrawn when its text changes instead of every frame. Only with a
    // portrait to draw: it's opaque, so the face window covers that spot of the InfoLink exactly (conui.rs's tests
    // check the two add up).
    let face_split = crate::perf::lean_updates()
        && con
            .link
            .as_ref()
            .is_some_and(|l| l.show_portrait && l.portrait.as_ref().is_some_and(|p| hud.tex(assets, p).is_some()));
    HudFrame {
        s,
        sw,
        sh,
        hit_h,
        compass_h,
        con,
        third,
        menu,
        first_w,
        first_h,
        up_h,
        low_h,
        bark_w,
        bark_h,
        win_pos,
        rec_head,
        rec_small,
        rec_any,
        rec_w,
        rec_h,
        start_font,
        start,
        start_lines,
        start_w,
        start_h,
        info_id,
        info_font,
        info_lines,
        info_w,
        info_h,
        stay_msg,
        msg_font,
        fps_text,
        fps_w,
        fps_h,
        debug_lines,
        debug_w,
        debug_h,
        net_lines,
        net_w,
        net_h,
        msg_w,
        msg_h,
        ammo_visible,
        log_font,
        log_w,
        log_lines,
        log_h,
        log_visible,
        damage_visible,
        face_split,
    }
}

/// The mission start text (classic_hud::start_text) typing on: its font, state, the typed lines and the window size.
#[allow(clippy::type_complexity)]
fn start_text_window(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    dt: f32,
) -> (Option<Arc<UiFont>>, Option<(String, f32, f32)>, Vec<String>, i32, i32) {
    // Mission start text: one character a frame in the original (PrintNextCharacter per Tick); here 40 a second,
    // then 5 s (displayTime) before it hides.
    let start_font = hud.font(assets, "DeusExUI.FontLocation");
    let start = {
        let mut g = START.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((msg, shown, left)) = g.as_mut() {
            let n = msg.chars().count() as f32;
            if *shown < n {
                *shown = (*shown + dt * 40.0).min(n);
            } else {
                *left -= dt;
            }
            if *left <= 0.0 {
                *g = None;
            }
        }
        g.clone()
    };
    let start_lines: Vec<String> = match (&start, &start_font) {
        (Some((msg, shown, _)), Some(f)) => {
            let typed: String = msg.chars().take(*shown as usize).collect();
            let all = crate::uicanvas::wrap(f, &msg.replace("|n", "\n"), 500);
            // Lay out the whole message (the window is sized for it), show the typed part.
            let mut left = typed.replace("|n", "").chars().count();
            all.iter()
                .map(|l| {
                    let k = l.chars().count().min(left);
                    left -= k;
                    l.chars().take(k).collect()
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let (start_w, start_h) = match (&start, &start_font) {
        (Some((msg, ..)), Some(f)) => {
            let all = crate::uicanvas::wrap(f, &msg.replace("|n", "\n"), 500);
            (all.iter().map(|l| f.width(l)).max().unwrap_or(0) + 2, all.len() as i32 * f.height + 2)
        }
        _ => (1, 1),
    };
    (start_font, start, start_lines, start_w, start_h)
}

/// HUDInformationDisplay's text: (text id, font, wrapped paragraphs, width, height).
#[allow(clippy::type_complexity)]
fn info_window(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    sw: i32,
) -> (Option<u64>, Option<Arc<UiFont>>, Vec<(u8, [u8; 3], Vec<String>)>, i32, i32) {
    // HUDInformationDisplay (DeusExHUD.ConfigurationChanged): as wide as its longest paragraph, at most
    // Min(width - 170, 800), the paragraphs word-wrapped to that; centred between the InfoLink and the belt.
    let info = crate::info::shown();
    let info_font = hud.font(assets, "FontMenuSmall_DS");
    let info_lines: Vec<(u8, [u8; 3], Vec<String>)> = match (&info, &info_font) {
        (Some((_, paras)), Some(f)) => {
            let max_w = (sw - 170).min(800);
            let pref = paras.iter().map(|p| f.width(&p.text)).max().unwrap_or(0) + INFO_SIDE * 2;
            let inner = pref.min(max_w) - INFO_SIDE * 2;
            paras
                .iter()
                .map(|p| {
                    let l = if p.text.is_empty() { vec![String::new()] } else { crate::uicanvas::wrap(f, &p.text, inner.max(1)) };
                    (p.align, p.color, l)
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let (info_w, info_h) = match (&info, &info_font) {
        (Some((_, paras)), Some(f)) => {
            let max_w = (sw - 170).min(800);
            let pref = paras.iter().map(|p| f.width(&p.text)).max().unwrap_or(0) + INFO_SIDE * 2;
            let lines: usize = info_lines.iter().map(|l| l.2.len()).sum();
            (pref.min(max_w), (lines as i32 * f.height + INFO_TOP + INFO_BOTTOM).max(INFO_MIN_H))
        }
        _ => (1, 1),
    };
    (info.as_ref().map(|i| i.0), info_font, info_lines, info_w, info_h)
}

impl HudFrame {
    /// Where a window goes this frame and whether it shows: (x, y, width, height, visible) in UI units.
    fn layout(&self, w: Win, snap: &Snapshot, window: &Window) -> (i32, i32, i32, i32, bool) {
        let HudFrame {
            s,
            sw,
            sh,
            hit_h,
            compass_h,
            ref con,
            third,
            menu,
            first_w,
            first_h,
            up_h,
            low_h,
            bark_w,
            bark_h,
            win_pos,
            rec_any,
            rec_w,
            rec_h,
            ref start,
            start_w,
            start_h,
            info_id,
            info_w,
            info_h,
            ref stay_msg,
            ref fps_text,
            fps_w,
            fps_h,
            ref debug_lines,
            debug_w,
            debug_h,
            ref net_lines,
            net_w,
            net_h,
            msg_w,
            msg_h,
            ammo_visible,
            log_w,
            log_h,
            log_visible,
            damage_visible,
            face_split,
            ..
        } = *self;
        let (ww, wh) = w.size();
        match w {
            Win::Hit => (0, 0, ww, wh, !snap.dead && !menu),
            Win::Compass => (0, hit_h + 4, ww, wh, !snap.dead && !menu),
            Win::Ammo => (0, sh - wh, ww, wh, ammo_visible),
            Win::Belt => (sw - ww, sh - wh, ww, wh, !snap.dead && !menu),
            Win::Log => (
                Win::Hit.size().0 + 20,
                10,
                log_w,
                log_h,
                log_visible && !menu,
            ),
            Win::Damage => (0, hit_h + compass_h + 4, ww, wh, damage_visible && !menu),
            Win::Received if third => (
                10,
                sh - (low_h as f32 * win_pos) as i32 - rec_h - 5,
                rec_w.max(1),
                rec_h.max(1),
                rec_any && !snap.dead,
            ),
            Win::Received => (
                sw / 2 - rec_w / 2,
                sh / 2 + 20 - first_h - bark_h,
                rec_w.max(1),
                rec_h.max(1),
                rec_any && !snap.dead && !menu,
            ),
            Win::InfoLink => (
                Win::Hit.size().0 + 20,
                if log_visible { log_h + 20 } else { 0 },
                ww,
                wh,
                con.link.is_some() && !snap.dead && !menu,
            ),
            // Over the InfoLink's portrait (draw_infolink puts it at 20, 25).
            Win::InfoLinkFace => (
                Win::Hit.size().0 + 20 + 20,
                (if log_visible { log_h + 20 } else { 0 }) + 25,
                ww,
                wh,
                face_split && !snap.dead && !menu,
            ),
            Win::ConFirst => (
                sw / 2 - first_w / 2,
                sh - Win::Belt.size().1 - first_h - 20,
                first_w.max(1),
                first_h.max(1),
                first_h > 0 && !snap.dead && !menu,
            ),
            Win::Bark => (
                sw / 2 - bark_w / 2,
                sh - Win::Belt.size().1 - bark_h - first_h - 20,
                bark_w.max(1),
                bark_h.max(1),
                bark_h > 0 && !snap.dead && !menu,
            ),
            Win::Start => (
                sw / 2 - start_w / 2,
                sh / 2 - start_h / 2 - if START_CENTRED.load(std::sync::atomic::Ordering::Relaxed) { 0 } else { 75 },
                start_w.max(1),
                start_h.max(1),
                // A centred message (the endgame quote, the intro's "Press Enter to continue") shows over a cut
                // scene too.
                start.is_some() && !snap.dead && (!menu || START_CENTRED.load(std::sync::atomic::Ordering::Relaxed)),
            ),
            Win::Info => {
                // infoTop: under the InfoLink when it's up (the log doesn't count: DeusExHUD sets logTop there);
                // infoBottom: the top of the belt.
                let link_h = if con.link.is_some() { crate::conui::INFOLINK_SIZE.1 } else { 0 };
                let link_y = if log_visible { log_h + 20 } else { 0 };
                let top = if con.link.is_some() { 10 + link_h + link_y } else { 0 };
                let bottom = sh - Win::Belt.size().1;
                (
                    sw / 2 - info_w / 2,
                    top + ((bottom - top) / 2 - info_h / 2) + 10,
                    info_w.max(1),
                    info_h.max(1),
                    info_id.is_some() && !snap.dead && !menu,
                )
            }
            Win::Fps => (sw - fps_w - 4, 2, fps_w.max(1), fps_h.max(1), fps_text.is_some()),
            Win::Debug => (4, 4, debug_w.max(1), debug_h.max(1), !debug_lines.is_empty()),
            Win::Net => (sw / 2 - net_w / 2, 8, net_w.max(1), net_h.max(1), !net_lines.is_empty()),
            Win::Msg => (
                sw / 2 - msg_w / 2,
                sh - Win::Belt.size().1 - msg_h - 20,
                msg_w.max(1),
                msg_h.max(1),
                stay_msg.is_some() && !menu,
            ),
            Win::ConCursor => {
                let pos = window.cursor_position().map(|c| {
                    let k = window.scale_factor() / s as f32;
                    ((c.x * k) as i32 - 32, (c.y * k) as i32 - 32)
                });
                let choosing = third && con.play.as_ref().is_some_and(|p| !p.choices.is_empty() && p.win_pos >= 1.0);
                let (cx, cy) = pos.unwrap_or((0, 0));
                (cx, cy, 64, 64, choosing && pos.is_some())
            }
            Win::ConUpper => (
                0,
                (-(up_h as f32) * (1.0 - win_pos)) as i32,
                sw,
                up_h.max(1),
                third,
            ),
            Win::ConLower => (
                0,
                sh - (low_h as f32 * win_pos) as i32,
                sw,
                low_h.max(1),
                third,
            ),
            // DeusExHUD: the active items display sits at the top right, above the belt.
            Win::Active => {
                let (aw, ah) = active_size(snap, sh - Win::Belt.size().1);
                (
                    sw - aw,
                    0,
                    aw.max(1),
                    ah.max(1),
                    (!snap.augs.is_empty() || !snap.items.is_empty()) && !snap.dead && !menu,
                )
            }
        }
    }
}

/// A hidden window keeps its last image: its node is hidden (and out of layout with Lean Updates).
fn hide_window(nodes: &mut Query<(&HudWindow, &mut Node, &mut Visibility, &MaterialNode<HudLayer>)>, w: Win) {
    // Hidden windows keep their last image: no canvas, no upload (a hidden ConLower alone was a
    // screen-wide f32 canvas allocated, cleared and converted every frame).
    for (hw, mut node, mut vis, _) in nodes.iter_mut() {
        if hw.0 == w {
            vis.set_if_neq(Visibility::Hidden);
            // Lean Updates: out of Bevy's layout too (Visibility keeps a node in it).
            if crate::perf::lean_updates() && node.display != Display::None {
                node.display = Display::None;
            }
        }
    }
}

/// What a window that only changes with its inputs is drawn from (None = redrawn every frame). The hit display's
/// colours tick here (BodyPart.color), so they are worked out before the redraw check.
#[allow(clippy::too_many_arguments)]
fn window_key(
    w: Win,
    f: &HudFrame,
    snap: &Snapshot,
    hud: &mut ClassicHud,
    inv: &PlayerInventory,
    ww: i32,
    wh: i32,
    y: i32,
    dt: f32,
) -> Option<u64> {
    let HudFrame { s, sw, face_split, ref con, info_id, ref stay_msg, ref fps_text, ref debug_lines, ref net_lines, ref log_lines, .. } = *f;
    match w {
        // With the portrait on a window of its own, the InfoLink changes only with its text.
        Win::InfoLink if face_split => con.link.as_ref().map(|l| {
            hkey(&(
                &l.name,
                l.name_visible,
                &l.queued,
                l.show_icon,
                &l.text,
                l.typed,
                crate::conui::infolink_cursor_shown(l),
                (ww, wh, s),
            ))
        }),
        Win::Belt => {
            // Hashed field by field (no formatted copies of the items).
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            inv.0.belt.hash(&mut h);
            for &i in inv.0.belt.iter().flatten() {
                if let Some(it) = inv.0.items.get(i) {
                    (&it.def.class, &it.def.name, &it.def.belt_name, &it.def.icon, it.count, it.uid).hash(&mut h);
                }
            }
            snap.held.as_ref().map(|v| (&v.class, &v.ammo_class, v.loaded)).hash(&mut h);
            snap.pending.hash(&mut h);
            // The loaded-ammo map in any order.
            snap.loaded_ammo.iter().fold(0u64, |a, kv| a ^ hkey(&kv)).hash(&mut h);
            inv.0.ammo.hash(&mut h);
            (ww, wh, s).hash(&mut h);
            Some(h.finish())
        }
        Win::Info => Some(hkey(&(info_id, ww, wh, s))),
        Win::Msg => Some(hkey(&(stay_msg, ww, wh, s))),
        Win::Fps => Some(hkey(&(fps_text, ww, wh, s))),
        Win::Debug => Some(hkey(&(debug_lines, ww, wh, s))),
        Win::Net => Some(hkey(&(net_lines, ww, wh, s))),
        Win::ConCursor => Some(hkey(&(ww, wh, s))),
        // The ammo display changes with the gun, its rounds, the pool and the lock line.
        Win::Ammo => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            if let Some(v) = snap.held.as_ref() {
                (&v.class, &v.ammo_class, v.reload_count, v.loaded, v.reloading, v.can_track, v.item).hash(&mut h);
            }
            inv.0.ammo.hash(&mut h);
            (&snap.lock_text, snap.lock_color).hash(&mut h);
            (ww, wh, s).hash(&mut h);
            Some(h.finish())
        }
        // The log: its lines and the age of the newest (fades by the second).
        Win::Log => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            log_lines.hash(&mut h);
            (ww, wh, s).hash(&mut h);
            Some(h.finish())
        }
        // The hit display only changes with its colours and bars.
        Win::Hit => {
            let cols: [Rgb; 6] = std::array::from_fn(|i| hud.parts[i].color(dt, snap.parts[i]));
            hud.hit_cols = cols;
            let blink = snap.breath.is_some_and(|b| b < 0.1) && snap.swim_time % 0.5 > 0.25;
            Some(hkey(&(
                cols,
                (snap.energy.clamp(0.0, 100.0) * 0.55) as i32,
                snap.breath.map(|b| (b.clamp(0.0, 1.0) * 55.0) as i32),
                snap.stamina.map(|b| (b.clamp(0.0, 1.0) * 55.0) as i32),
                blink,
                (ww, wh, s),
            )))
        }
        // The compass only changes when the tick strip moves a pixel.
        Win::Compass => Some(hkey(&(compass_positions(snap.yaw, hud.true_north.unwrap_or(0)), (ww, wh, s)))),
        Win::Active => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            snap.augs.hash(&mut h);
            for (c, f) in &snap.items {
                (c, (f * 1000.0) as i32).hash(&mut h);
            }
            (ww, wh, s).hash(&mut h);
            Some(h.finish())
        }
        // Lean Updates: the conversation windows (a cut scene is all conversation). The upper one is only its
        // background; the lower one is the speaker, the line and the choices, where the letterbox puts it (the
        // choice rows for the mouse stay as last drawn).
        Win::ConUpper if crate::perf::lean_updates() => Some(hkey(&(ww, wh, s))),
        Win::ConLower if crate::perf::lean_updates() => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (ww, wh, s, y, sw).hash(&mut h);
            con.play.as_ref().map(|p| (&p.name, &p.text, p.player_line, p.forced, &p.choices, p.focus)).hash(&mut h);
            crate::gamepad::pad_in_use().hash(&mut h);
            std::mem::discriminant(&crate::gamepad::glyph_kind()).hash(&mut h);
            Some(h.finish())
        }
        _ => None,
    }
}

/// A fresh canvas for a window (the split InfoLink starts from its cached frame).
fn window_canvas(hud: &mut ClassicHud, assets: &mut RuntimeAssets, w: Win, f: &HudFrame, ww: i32, wh: i32) -> Canvas {
    let s = f.s;
    if w == Win::InfoLink && f.face_split {
        let fk = hkey(&(ww, wh, s, crate::settings::generation()));
        if hud.infolink_frame.as_ref().map(|f| f.0) != Some(fk) {
            let mut frame = Canvas::new(ww, wh, s);
            crate::conui::draw_infolink_frame(hud, assets, &mut frame);
            hud.infolink_frame = Some((fk, frame));
        }
        hud.infolink_frame.as_ref().map_or_else(|| Canvas::new(ww, wh, s), |f| f.1.clone())
    } else {
        Canvas::new(ww, wh, s)
    }
}

/// Draw one window's picture into its canvas.
#[allow(clippy::too_many_arguments)]
fn draw_window(
    w: Win,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    canvas: &mut Canvas,
    f: &HudFrame,
    snap: &Snapshot,
    inv: &PlayerInventory,
    tiny: Option<&UiFont>,
    (ww, wh, y): (i32, i32, i32),
    dt: f32,
) {
    let HudFrame {
        sw,
        sh,
        ref con,
        face_split,
        ref rec_head,
        ref rec_small,
        ref start_font,
        ref start_lines,
        ref info_font,
        ref info_lines,
        ref stay_msg,
        ref msg_font,
        ref fps_text,
        ref debug_lines,
        ref net_lines,
        ref log_font,
        ref log_lines,
        ..
    } = *f;
    match w {
        Win::Hit => draw_hit(hud, assets, canvas, tiny, snap, dt),
        Win::Compass => draw_compass(hud, assets, canvas, snap),
        Win::Ammo => draw_ammo(hud, assets, canvas, tiny, snap, inv),
        Win::Belt => draw_belt(hud, assets, canvas, tiny, snap, inv),
        Win::Log => draw_log(hud, assets, canvas, log_font.as_deref(), log_lines),
        Win::Damage => draw_damage(hud, assets, canvas),
        Win::Received => {
            if let (Some(h), Some(sm)) = (rec_head, rec_small) {
                draw_received(hud, assets, canvas, h, sm);
            }
        }
        Win::Active => draw_active(
            hud,
            assets,
            canvas,
            tiny,
            snap,
            sh - Win::Belt.size().1,
        ),
        Win::InfoLink => {
            if let Some(l) = &con.link {
                if face_split {
                    crate::conui::draw_infolink_contents(hud, assets, canvas, l, false);
                } else {
                    crate::conui::draw_infolink(hud, assets, canvas, l, true);
                }
            }
        }
        Win::InfoLinkFace => {
            if let Some(l) = &con.link {
                crate::conui::draw_infolink_face(hud, assets, canvas, l, 0, 0);
            }
        }
        Win::ConFirst => {
            if let Some(p) = &con.play {
                crate::conui::draw_first(hud, assets, canvas, p);
            }
        }
        Win::ConUpper => crate::conui::draw_upper(hud, assets, canvas),
        Win::Bark => crate::conui::draw_barks(hud, assets, canvas, &con.barks),
        Win::ConCursor => crate::inventory_screen::cursor_with_shadow(hud, assets, canvas),
        Win::Start => {
            if let Some(f) = start_font {
                for (i, l) in start_lines.iter().enumerate() {
                    let y = i as i32 * f.height;
                    canvas.text(f, 2, y + 2, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], l);
                    canvas.text(f, 0, y, ww, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], l);
                }
            }
        }
        Win::Info => draw_info(hud, assets, canvas, info_font.as_deref(), info_lines),
        Win::Fps => {
            if let (Some(f), Some(t)) = (msg_font, fps_text) {
                canvas.text(f, 1, 1, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], t);
                canvas.text(f, 0, 0, ww, f.height, HAlign::Left, VAlign::Top, [255, 242, 140], t);
            }
        }
        Win::Debug => {
            if let Some(f) = msg_font {
                for (i, l) in debug_lines.iter().enumerate() {
                    let y = i as i32 * f.height;
                    canvas.text(f, 1, y + 1, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], l);
                    canvas.text(f, 0, y, ww, f.height, HAlign::Left, VAlign::Top, [220, 255, 220], l);
                }
            }
        }
        Win::Net => {
            if let Some(f) = msg_font {
                for (i, (l, c)) in net_lines.iter().enumerate() {
                    let y = i as i32 * (f.height + 1);
                    let x = (ww - f.width(l).min(ww)) / 2;
                    canvas.text(f, x + 1, y + 1, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], l);
                    canvas.text(f, x, y, ww, f.height, HAlign::Left, VAlign::Top, *c, l);
                }
            }
        }
        Win::Msg => {
            if let (Some(f), Some(t)) = (msg_font, stay_msg) {
                canvas.text(f, 2, 2, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], t);
                canvas.text(f, 0, 0, ww, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], t);
            }
        }
        Win::ConLower => {
            crate::conui::LOWER_Y.store(y, std::sync::atomic::Ordering::Relaxed);
            if let Some(p) = &con.play {
                crate::conui::draw_lower(hud, assets, canvas, p, sw);
            } else {
                // CinematicWindow: the letterbox alone.
                canvas.fill(0, 0, ww, wh, crate::uicanvas::Style::Normal, [0, 0, 0]);
            }
        }
    }
}

/// Upload a redrawn window: a resized image is rebuilt (and its material marked changed), changed pixels copied in.
#[allow(clippy::too_many_arguments)]
fn upload_window(
    hud: &mut ClassicHud,
    images: &mut Assets<Image>,
    nodes: &mut Query<(&HudWindow, &mut Node, &mut Visibility, &MaterialNode<HudLayer>)>,
    layers: &mut Assets<HudLayer>,
    w: Win,
    canvas: Canvas,
    ww: i32,
    wh: i32,
    s: i32,
) {
    let bytes = canvas.to_rgba8();
    let (handle, size) = hud.images.get(&w).cloned().unwrap();
    let want_size = ww * 100000 + wh * 10 + s;
    if size != want_size {
        if let Some(mut img) = images.get_mut(&handle) {
            *img = blank_image((ww * s) as u32, (wh * s) as u32);
        }
        hud.images.insert(w, (handle.clone(), want_size));
        hud.last.remove(&w);
        // A resized image is a new GPU texture; the material's bind group still points at the old one
        // until the material itself is marked changed.
        for (hw, _, _, mat) in nodes.iter() {
            if hw.0 == w {
                if let Some(m) = layers.get_mut(&mat.0) {
                    m.into_inner();
                }
            }
        }
    }
    if hud.last.get(&w) != Some(&bytes) {
        if let Some(mut img) = images.get_mut(&handle) {
            match &mut img.data {
                Some(d) if d.len() == bytes.len() => d.copy_from_slice(&bytes),
                d => *d = Some(bytes.clone()),
            }
        }
        hud.last.insert(w, bytes);
    }
}

/// Show a window's node, sized about the screen corner it sits nearest (HUD part size), at its opacity.
#[allow(clippy::too_many_arguments)]
fn place_window(
    nodes: &mut Query<(&HudWindow, &mut Node, &mut Visibility, &MaterialNode<HudLayer>)>,
    layers: &mut Assets<HudLayer>,
    w: Win,
    visible: bool,
    (x, y, ww, wh): (i32, i32, i32, i32),
    k: f32,
    alpha: f32,
    f: &HudFrame,
    window: &Window,
) {
    let (s, sw, sh) = (f.s, f.sw, f.sh);
    for (hw, mut node, mut vis, mat) in nodes.iter_mut() {
        if hw.0 != w {
            continue;
        }
        let want = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        // Back in layout when shown (a parked window is Display::None).
        if node.display != Display::Flex {
            node.display = Display::Flex;
        }
        // Sized about the screen corner it sits nearest.
        let (sx, sy) = if k == 1.0 {
            (x as f32, y as f32)
        } else {
            let fx = if x * 2 + ww < sw { x as f32 } else { (x + ww) as f32 - ww as f32 * k };
            let fy = if y * 2 + wh < sh { y as f32 } else { (y + wh) as f32 - wh as f32 * k };
            (fx, fy)
        };
        let lf = |v: f32| v * s as f32 / window.scale_factor();
        let (l, t, wd, ht) = (
            Val::Px(lf(sx)),
            Val::Px(lf(sy)),
            Val::Px(lf(ww as f32 * k)),
            Val::Px(lf(wh as f32 * k)),
        );
        if layers.get(&mat.0).is_some_and(|m| m.gamma.y != alpha) {
            if let Some(mut m) = layers.get_mut(&mat.0) {
                m.gamma.y = alpha;
            }
        }
        if node.left != l || node.top != t || node.width != wd || node.height != ht {
            node.left = l;
            node.top = t;
            node.width = wd;
            node.height = ht;
        }
    }
}

/// The HUD parts that can be customised: (window, settings key part, label).
pub const HUD_PARTS: &[(&str, &str)] = &[
    ("Hit", "Health Display"),
    ("Compass", "Compass"),
    ("Ammo", "Ammo Display"),
    ("Belt", "Object Belt"),
    ("Log", "Message Log"),
    ("Active", "Augmentations / Active Items"),
    ("Damage", "Damage Indicators"),
];

/// Size and opacity choices (index 0 = as the original).
pub const HUD_SIZES: [(&str, f32); 6] = [("100%", 1.0), ("50%", 0.5), ("75%", 0.75), ("125%", 1.25), ("150%", 1.5), ("200%", 2.0)];
pub const HUD_FADES: [(&str, f32); 4] = [("100%", 1.0), ("75%", 0.75), ("50%", 0.5), ("25%", 0.25)];

/// (shown, size, opacity) of a HUD window: settings HUD_<Part>_Hide / _Size / _Fade (the seven parts' values
/// are looked up again only when the settings change).
fn hud_part(w: Win) -> (bool, f32, f32) {
    static CACHE: std::sync::Mutex<(u64, [(bool, f32, f32); 7])> = std::sync::Mutex::new((0, [(true, 1.0, 1.0); 7]));
    let slot = match w {
        Win::Hit => 0,
        Win::Compass => 1,
        Win::Ammo => 2,
        Win::Belt => 3,
        Win::Log => 4,
        Win::Active => 5,
        Win::Damage => 6,
        _ => return (true, 1.0, 1.0),
    };
    let gen = crate::settings::generation();
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if c.0 != gen {
        c.0 = gen;
        for (i, part) in [Win::Hit, Win::Compass, Win::Ammo, Win::Belt, Win::Log, Win::Active, Win::Damage].into_iter().enumerate() {
            c.1[i] = hud_part_lookup(part);
        }
    }
    c.1[slot]
}

fn hud_part_lookup(w: Win) -> (bool, f32, f32) {
    let key = match w {
        Win::Hit => "Hit",
        Win::Compass => "Compass",
        Win::Ammo => "Ammo",
        Win::Belt => "Belt",
        Win::Log => "Log",
        Win::Active => "Active",
        Win::Damage => "Damage",
        _ => return (true, 1.0, 1.0),
    };
    let g = |n: &str| crate::settings::get(&format!("HUD_{key}_{n}")).round() as usize;
    (
        g("Hide") == 0,
        HUD_SIZES.get(g("Size")).map_or(1.0, |v| v.1),
        HUD_FADES.get(g("Fade")).map_or(1.0, |v| v.1),
    )
}

/// HUDActiveAugsBorder / HUDActiveItemsBorder metrics: border width, top/bottom margins around the icon column,
/// top/bottom texture heights and offsets, icon column position.
struct ColumnBorder {
    width: i32,
    top_margin: i32,
    bottom_margin: i32,
    top_h: i32,
    top_off: i32,
    bottom_h: i32,
    bottom_off: i32,
    tile: (i32, i32),
    tex: [&'static str; 3],
}

const AUG_COLUMN: ColumnBorder = ColumnBorder {
    width: 62,
    top_margin: 13,
    bottom_margin: 9,
    top_h: 37,
    top_off: 26,
    bottom_h: 32,
    bottom_off: 28,
    tile: (20, 13),
    tex: [
        "HUDAugmentationsBorder_Top",
        "HUDAugmentationsBorder_Center",
        "HUDAugmentationsBorder_Bottom",
    ],
};

const ITEM_COLUMN: ColumnBorder = ColumnBorder {
    width: 48,
    top_margin: 7,
    bottom_margin: 6,
    top_h: 36,
    top_off: 21,
    bottom_h: 30,
    bottom_off: 23,
    tile: (6, 7),
    tex: [
        "HUDItemsBorder_Top",
        "HUDItemsBorder_Center",
        "HUDItemsBorder_Bottom",
    ],
};

/// Icons are 34 x 34 with 2 between them (TileWindow minor spacing).
const ACTIVE_ICON: i32 = 34;

fn column_height(b: &ColumnBorder, n: usize) -> i32 {
    if n == 0 {
        return 0;
    }
    b.top_margin + n as i32 * (ACTIVE_ICON + 2) - 2 + b.bottom_margin
}

/// HUDActiveItemsDisplay.ConfigurationChanged: the aug column at x = items width; the items column below it
/// (right-aligned), or beside it at (14, 6) when both don't fit above the belt. Returns the positions and size.
fn active_layout(snap: &Snapshot, avail_h: i32) -> ((i32, i32), (i32, i32), (i32, i32)) {
    let aug_h = column_height(&AUG_COLUMN, snap.augs.len());
    let item_h = column_height(&ITEM_COLUMN, snap.items.len());
    let w = AUG_COLUMN.width + ITEM_COLUMN.width;
    let augs_at = (ITEM_COLUMN.width, 0);
    let item_x = if snap.augs.is_empty() {
        0
    } else {
        ITEM_COLUMN.width + 14
    };
    let items_at = if aug_h + item_h > avail_h {
        (14, 6)
    } else {
        (item_x, aug_h - 2)
    };
    let h = (aug_h).max(items_at.1 + item_h);
    (augs_at, items_at, (w, h))
}

fn active_size(snap: &Snapshot, avail_h: i32) -> (i32, i32) {
    active_layout(snap, avail_h).2
}

fn draw_column_border(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    b: &ColumnBorder,
    x: i32,
    y: i32,
    h: i32,
) {
    // HUDActiveItemsBorderBase.DrawBorder: top, centre pattern, bottom.
    if let Some(t) = hud.ui(assets, b.tex[0]) {
        c.texture(
            x,
            y,
            b.width,
            b.top_h,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(1),
        );
    }
    if let Some(t) = hud.ui(assets, b.tex[1]) {
        c.texture(
            x,
            y + b.top_off,
            b.width,
            h - b.top_off - b.bottom_off,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(1),
        );
    }
    if let Some(t) = hud.ui(assets, b.tex[2]) {
        c.texture(
            x,
            y + h - b.bottom_h,
            b.width,
            b.bottom_h,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(1),
        );
    }
}

/// HUDActiveAug / HUDActiveItem: the icon background, the icon (augs yellow while on, grey while off), the
/// F-key label with a black shadow, and a charge bar under charged items.
fn draw_active(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    snap: &Snapshot,
    avail_h: i32,
) {
    let (augs_at, items_at, _) = active_layout(snap, avail_h);
    let bg = hud.ui(assets, "HUDIconsBackground");
    if !snap.augs.is_empty() {
        let h = column_height(&AUG_COLUMN, snap.augs.len());
        draw_column_border(hud, assets, c, &AUG_COLUMN, augs_at.0, augs_at.1, h);
        for (k, (hotkey, icon, on)) in snap.augs.iter().enumerate() {
            let x = augs_at.0 + AUG_COLUMN.tile.0;
            let y = augs_at.1 + AUG_COLUMN.tile.1 + k as i32 * (ACTIVE_ICON + 2);
            if let Some(t) = &bg {
                c.texture(
                    x,
                    y,
                    ACTIVE_ICON,
                    ACTIVE_ICON,
                    0,
                    0,
                    t,
                    Style::Translucent,
                    crate::menuui::h_col(0),
                );
            }
            if let Some(t) = hud.tex(assets, icon) {
                c.texture(
                    x + 2,
                    y + 2,
                    32,
                    32,
                    0,
                    0,
                    &t,
                    Style::Translucent,
                    if *on { [255, 255, 0] } else { [100, 100, 100] },
                );
            }
            if let Some(f) = font {
                let label = format!("F{hotkey}");
                c.text(
                    f,
                    x + 16,
                    y + 1,
                    15,
                    8,
                    HAlign::Right,
                    VAlign::Top,
                    [0, 0, 0],
                    &label,
                );
                c.text(
                    f,
                    x + 17,
                    y,
                    15,
                    8,
                    HAlign::Right,
                    VAlign::Top,
                    crate::menuui::h_col(8),
                    &label,
                );
            }
        }
    }
    if !snap.items.is_empty() {
        let h = column_height(&ITEM_COLUMN, snap.items.len());
        draw_column_border(hud, assets, c, &ITEM_COLUMN, items_at.0, items_at.1, h);
        for (k, (class, pct)) in snap.items.iter().enumerate() {
            let x = items_at.0 + ITEM_COLUMN.tile.0;
            let y = items_at.1 + ITEM_COLUMN.tile.1 + k as i32 * (ACTIVE_ICON + 2);
            if let Some(t) = &bg {
                c.texture(
                    x,
                    y,
                    ACTIVE_ICON,
                    ACTIVE_ICON,
                    0,
                    0,
                    t,
                    Style::Translucent,
                    crate::menuui::h_col(0),
                );
            }
            if let Some(icon) = hud.path_default(assets, class, "ChargedIcon") {
                if let Some(t) = hud.tex(assets, &icon) {
                    c.texture(
                        x + 2,
                        y + 2,
                        32,
                        32,
                        0,
                        0,
                        &t,
                        Style::Translucent,
                        [255, 255, 255],
                    );
                }
            }
            // ProgressBarWindow 32 x 2 at (1, 30), scaled colour.
            let w = (32.0 * pct).round() as i32;
            c.fill(
                x + 1,
                y + 30,
                w,
                2,
                Style::Normal,
                crate::uicanvas::color_scaled(*pct),
            );
        }
    }
}

/// TextLogWindow: each message word-wrapped to the width; only the newest LOG_LINES lines are kept.
fn wrap_log(font: &UiFont, log: &[(String, f32)], width: i32) -> Vec<String> {
    let mut out = Vec::new();
    for (line, _) in log {
        let mut cur = String::new();
        for word in line.split_whitespace() {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if !cur.is_empty() && font.width(&candidate) > width {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = candidate;
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    let n = out.len();
    out.split_off(n.saturating_sub(LOG_LINES))
}

/// HUDLogDisplay: shared border window (tiled background + border), the log icon, the lines.
/// HUDInformationDisplay metrics: TopMargin 13, BottomMargin 10, MinHeight 40, the tile's side margins 20.
const INFO_TOP: i32 = 13;
const INFO_BOTTOM: i32 = 10;
const INFO_MIN_H: i32 = 40;
const INFO_SIDE: i32 = 20;

/// HUDSharedBorderWindow's background and border, then each paragraph's TextWindow (FontMenuSmall_DS, word
/// wrapped, aligned in the full width) one under the other.
fn draw_info(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    paras: &[(u8, [u8; 3], Vec<String>)],
) {
    let names = ["TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center"];
    let bg: [Option<Arc<UiTex>>; 9] = names.map(|n| hud.ui(assets, &format!("HUDWindowBackground_{n}")));
    let border: [Option<Arc<UiTex>>; 9] = names.map(|n| hud.ui(assets, &format!("HUDWindowBorder_{n}")));
    let (w, h) = (c.w, c.h);
    c.borders(0, 0, w, h, &bg, crate::menuui::bg_style(), crate::menuui::h_col(0));
    c.borders(0, 0, w, h, &border, crate::menuui::border_style(), crate::menuui::border_col());
    let Some(f) = font else { return };
    let mut y = INFO_TOP;
    for (align, col, lines) in paras {
        let ha = match align {
            1 => HAlign::Center,
            2 => HAlign::Right,
            _ => HAlign::Left,
        };
        for l in lines {
            c.text(f, INFO_SIDE, y, w - INFO_SIDE * 2, f.height, ha, VAlign::Top, *col, l);
            y += f.height;
        }
    }
}

fn draw_log(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    lines: &[String],
) {
    let names = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ];
    let bg: [Option<Arc<UiTex>>; 9] =
        names.map(|n| hud.ui(assets, &format!("HUDWindowBackground_{n}")));
    let border: [Option<Arc<UiTex>>; 9] =
        names.map(|n| hud.ui(assets, &format!("HUDWindowBorder_{n}")));
    let (w, h) = (c.w, c.h);
    c.borders(0, 0, w, h, &bg, crate::menuui::bg_style(), crate::menuui::h_col(0));
    c.borders(0, 0, w, h, &border, crate::menuui::border_style(), crate::menuui::border_col());
    if let Some(icon) = hud.ui(assets, "LogIcon") {
        c.texture(
            LOG_MARGIN * 2,
            LOG_TOP + 5,
            32,
            32,
            0,
            0,
            &icon,
            Style::Masked,
            [255, 255, 255],
        );
    }
    let Some(f) = font else { return };
    let x = 32 + LOG_MARGIN * 3;
    for (i, l) in lines.iter().enumerate() {
        c.text(
            f,
            x,
            LOG_TOP + i as i32 * f.height,
            w - x - LOG_MARGIN,
            f.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            l,
        );
    }
}

/// HUDReceivedDisplay's preferred size: "Received:" then each item (42 x 37 icon over its label), in a row.
fn received_size(head: &UiFont, small: &UiFont, items: &[(Option<String>, String)]) -> (i32, i32) {
    let mut w = 10 + head.width("Received:");
    for (_, l) in items {
        w += 4 + small.width(l).max(42);
    }
    let h = 5 + 10 + (37 + 2 + small.height).max(head.height) + 10;
    (w + 10, h.max(40))
}

fn draw_received(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    head: &UiFont,
    small: &UiFont,
) {
    let names = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ];
    let bg: [Option<Arc<UiTex>>; 9] =
        names.map(|n| hud.ui(assets, &format!("HUDWindowBackground_{n}")));
    let border: [Option<Arc<UiTex>>; 9] =
        names.map(|n| hud.ui(assets, &format!("HUDWindowBorder_{n}")));
    let (w, h) = (c.w, c.h);
    c.borders(0, 0, w, h, &bg, crate::menuui::bg_style(), crate::menuui::h_col(0));
    c.borders(0, 0, w, h, &border, crate::menuui::border_style(), crate::menuui::border_col());
    let row_h = (37 + 2 + small.height).max(head.height);
    let y0 = 5 + 10;
    c.text(
        head,
        10,
        y0,
        head.width("Received:"),
        row_h,
        HAlign::Left,
        VAlign::Center,
        crate::menuui::h_col(8),
        "Received:",
    );
    let mut x = 10 + head.width("Received:") + 4;
    for (icon, label) in hud.received.clone() {
        let iw = small.width(&label).max(42);
        if let Some(t) = icon.and_then(|p| hud.tex(assets, &p)) {
            c.texture(
                x + iw / 2 - 21,
                y0,
                42,
                37,
                0,
                0,
                &t,
                Style::Masked,
                [255, 255, 255],
            );
        }
        c.text(
            small,
            x,
            y0 + 39,
            iw,
            small.height,
            HAlign::Center,
            VAlign::Top,
            crate::menuui::h_col(8),
            &label,
        );
        x += iw + 4;
    }
}

/// DamageHUDDisplay.DrawWindow: fading damage-type icons in a column, one direction arrow under them.
fn draw_damage(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas) {
    let mut max_alpha = 0.0f32;
    let (mut front, mut rear, mut left, mut right, mut center) =
        (false, false, false, false, false);
    for i in 0..DAMAGE_ICONS.len() {
        let d = hud.damage[i];
        if !d.active {
            continue;
        }
        let alpha = 1.0 - d.age / DMG_FADE;
        max_alpha = max_alpha.max(alpha);
        if alpha <= 0.0 {
            hud.damage[i] = DamageIcon::default();
            continue;
        }
        let (_, icon, col) = DAMAGE_ICONS[i];
        let col = col.map(|v| (v as f32 * alpha) as u8);
        if let Some(t) = icon.and_then(|n| hud.tex(assets, &format!("DeusExUI.Icons.{n}"))) {
            c.texture(
                (c.w - DMG_ICON) / 2,
                i as i32 * (DMG_ICON + DMG_MARGIN),
                DMG_ICON,
                DMG_ICON,
                0,
                0,
                &t,
                Style::Translucent,
                col,
            );
        }
        if d.center {
            center = true;
        } else {
            front |= d.dir > 53248 || d.dir < 12288;
            rear |= d.dir > 20480 && d.dir < 45056;
            right |= d.dir > 4096 && d.dir < 28672;
            left |= d.dir > 36864 && d.dir < 61440;
        }
    }
    let col = DAMAGE_ICONS[6]
        .2
        .map(|v| (v as f32 * max_alpha.max(0.0)) as u8);
    let y = DAMAGE_ICONS.len() as i32 * (DMG_ICON + DMG_MARGIN);
    for (on, name) in [
        (front, "DamageIconUp"),
        (rear, "DamageIconDown"),
        (left, "DamageIconLeft"),
        (right, "DamageIconRight"),
        (center, "DamageIconCenter"),
    ] {
        if on {
            if let Some(t) = hud.tex(assets, &format!("DeusExUI.Icons.{name}")) {
                c.texture(
                    DMG_MARGIN,
                    y,
                    DMG_ARROW,
                    DMG_ARROW,
                    0,
                    0,
                    &t,
                    Style::Translucent,
                    col,
                );
            }
        }
    }
    if hud.absorb != 0.0 {
        // Font'TechSmall' lives in Extension.u (DeusEx.u imports it from there): looked up in DeusExUI.u it was
        // never found, so the percent was never drawn.
        if let Some(f) = hud.font(assets, "Extension.TechSmall") {
            let s = format!("{}% Absorb", (hud.absorb * 100.0) as i32);
            let tw = f.width(&s);
            let col = [0, (255.0 * max_alpha.max(0.0)) as u8, 0];
            c.text(
                &f,
                (c.w - tw) / 2,
                c.h - (DMG_ARROW + f.height) / 2,
                tw,
                f.height,
                HAlign::Left,
                VAlign::Top,
                col,
                &s,
            );
        }
    }
    if !hud.damage.iter().any(|d| d.active) {
        hud.absorb = 0.0;
    }
}

/// HUDHitDisplay: background, border, the six body parts coloured by health, bioenergy bar, breath bar.
fn draw_hit(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    snap: &Snapshot,
    dt: f32,
) {
    if let Some(t) = hud.ui(assets, "HUDHitDisplayBackground_1") {
        c.texture(11, 11, 60, 76, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
    }
    if let Some(t) = hud.ui(assets, "HUDHitDisplayBorder_1") {
        c.texture(0, 0, 84, 106, 0, 0, &t, crate::menuui::border_style(), crate::menuui::border_col());
    }
    // winEnergy / winBreath: vertical ProgressBarWindows 5 x 55, scaled colour.
    let energy_pct = (snap.energy / 100.0).clamp(0.0, 1.0);
    let energy_col = color_scaled(energy_pct);
    if let Some(f) = font {
        c.text(f, 13, 74, 8, 8, HAlign::Left, VAlign::Top, energy_col, "BE");
    }
    let breath_pct = snap.breath.map(|b| b.clamp(0.0, 1.0));
    if let (Some(b), Some(f)) = (breath_pct, font) {
        let mut col = color_scaled(b);
        if b * 100.0 < 10.0 {
            col[0] = if snap.swim_time % 0.5 > 0.25 { 255 } else { 0 };
        }
        c.text(f, 61, 74, 8, 8, HAlign::Left, VAlign::Top, col, "O2");
    }
    // Sprint stamina: a bar like the bioenergy one on the other side of the body (the breath bar's place, which
    // takes it back under water).
    let stamina_pct = if breath_pct.is_none() { snap.stamina.map(|b| b.clamp(0.0, 1.0)) } else { None };
    if let (Some(b), Some(f)) = (stamina_pct, font) {
        c.text(f, 61, 74, 8, 8, HAlign::Left, VAlign::Top, color_scaled(b), "ST");
    }
    // Children, bottom first: the body outline, the parts, the bars.
    if let Some(t) = hud.ui(assets, "HUDHitDisplay_Body") {
        c.texture(24, 15, 34, 68, 0, 0, &t, Style::Translucent, COL_ARMOR);
    }
    const PARTS: [(&str, i32, i32, i32, i32); 6] = [
        ("HUDHitDisplay_Head", 39, 17, 4, 7),
        ("HUDHitDisplay_Torso", 36, 25, 10, 23),
        ("HUDHitDisplay_ArmLeft", 46, 27, 10, 23),
        ("HUDHitDisplay_ArmRight", 26, 27, 10, 23),
        ("HUDHitDisplay_LegLeft", 41, 44, 8, 36),
        ("HUDHitDisplay_LegRight", 33, 44, 8, 36),
    ];
    for (i, (name, x, y, w, h)) in PARTS.iter().enumerate() {
        let col = hud.hit_cols[i];
        let _ = dt;
        if let Some(t) = hud.ui(assets, name) {
            c.texture(*x, *y, *w, *h, 0, 0, &t, Style::Translucent, col);
        }
    }
    let bar = |c: &mut Canvas, x: i32, pct: f32| {
        let size = (pct * 55.0) as i32;
        c.fill(x, 20 + 55 - size, 5, size, Style::Normal, color_scaled(pct));
    };
    bar(c, 15, energy_pct);
    if let Some(b) = breath_pct.or(stamina_pct) {
        bar(c, 61, b);
    }
}

/// HUDCompassDisplay: the tick strip scrolls with the view yaw relative to the map's TrueNorth.
fn draw_compass(hud: &mut ClassicHud, assets: &mut RuntimeAssets, c: &mut Canvas, snap: &Snapshot) {
    const CLIP_W: i32 = 55;
    const TICK_W: i32 = 240;
    if let Some(t) = hud.ui(assets, "HUDCompassBackground_1") {
        c.texture(11, 6, 60, 19, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
    }
    if let Some(t) = hud.ui(assets, "HUDCompassBorder_1") {
        c.texture(0, 0, 73, 40, 0, 0, &t, crate::menuui::border_style(), crate::menuui::border_col());
    }
    let north = hud.true_north.unwrap_or(0);
    let (draw, wrap) = compass_positions(snap.yaw, north);
    if let Some(t) = hud.ui(assets, "HUDCompassTicks") {
        c.set_clip(Some((13, 7, CLIP_W, 22)));
        c.texture(13 + draw, 7, TICK_W, 15, 0, 0, &t, Style::Masked, COL_TICKS);
        c.texture(13 + wrap, 7, TICK_W, 15, 0, 0, &t, Style::Masked, COL_TICKS);
        c.set_clip(None);
    }
    if let Some(t) = hud.ui(assets, "HUDCompassTickBox") {
        c.texture(11, 6, 60, 19, 0, 0, &t, Style::Masked, crate::menuui::h_col(0));
    }
}

/// HUDCompassDisplay.Tick: where the two tick strips sit inside the 55-unit clip window.
fn compass_positions(yaw: i32, north: i32) -> (i32, i32) {
    const CLIP_W: i32 = 55;
    const TICK_W: i32 = 240;
    let units_per_pixel = 65536 / TICK_W;
    let half = CLIP_W / 2;
    let draw = half - ((yaw - north) & 65535) / units_per_pixel;
    let wrap = if draw > 0 && draw < CLIP_W {
        draw - TICK_W
    } else if draw - TICK_W < half {
        draw + TICK_W
    } else {
        100
    };
    (draw, wrap)
}

/// HUDAmmoDisplay.
#[allow(clippy::too_many_arguments)]
fn draw_ammo(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    snap: &Snapshot,
    inv: &PlayerInventory,
) {
    let Some(h) = snap.held.as_ref() else { return };
    if let Some(t) = hud.ui(assets, "HUDAmmoDisplayBackground_1") {
        c.texture(13, 13, 80, 54, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
    }
    if let Some(f) = font {
        c.text(
            f,
            66,
            17,
            21,
            8,
            HAlign::Center,
            VAlign::Top,
            crate::menuui::h_col(8),
            "AMMO",
        );
        c.text(
            f,
            66,
            48,
            21,
            8,
            HAlign::Center,
            VAlign::Top,
            crate::menuui::h_col(8),
            // GMDX HUD: Accurate Ammo Display (setting AmmoTotal): the rounds left instead of the clips.
            if setting!("AmmoTotal") > 0.5 { "TOTAL" } else { "CLIPS" },
        );
    }
    if let Some(t) = hud.ui(assets, "HUDAmmoDisplayBorder_1") {
        c.texture(0, 0, 95, 77, 0, 0, &t, crate::menuui::border_style(), crate::menuui::border_col());
    }
    let class = h.class.clone();
    if let Some(icon) = hud
        .path_default(assets, &class, "Icon")
        .and_then(|p| hud.tex(assets, &p))
    {
        c.texture(22, 20, 40, 35, 0, 0, &icon, Style::Masked, [255, 255, 255]);
    }
    let Some(f) = font else { return };
    let low_mark = match find(hud.defaults(assets, &class), "LowAmmoWaterMark") {
        Some(Value::Int(n)) => *n,
        Some(Value::Byte(n)) => *n as i32,
        _ => 0,
    };
    let loaded = h.loaded;
    let pool = h.ammo_class.as_ref().map(|a| {
        inv.0
            .ammo
            .iter()
            .find(|x| &x.0 == a)
            .map(|x| x.1)
            .unwrap_or(0) as i32
    });
    let has_ammo_type = pool.is_some();
    // AmmoType.AmmoAmount counts the rounds in the gun too.
    let amount = if has_ammo_type {
        pool.unwrap_or(0) + if h.reload_count > 0 { loaded } else { 0 }
    } else {
        0
    };
    let reload = h.reload_count as i32;
    let reloading = h.reloading;
    // GMDX HUD: Show Max Ammo (setting AmmoMaxCyan): cyan when you carry all you can of it.
    let max_ammo = h
        .ammo_class
        .clone()
        .and_then(|a| match find(hud.defaults(assets, &a), "MaxAmmo") {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        })
        .unwrap_or(i32::MAX);
    let full = setting!("AmmoMaxCyan") > 0.5 && has_ammo_type && amount >= max_ammo;
    let col = if full {
        [0, 255, 255]
    } else if amount < low_mark {
        COL_AMMO_LOW
    } else {
        COL_AMMO
    };
    let (in_clip, clips_real) = ammo_counts(reload, has_ammo_type, amount, loaded);
    let clips_left = if setting!("AmmoTotal") > 0.5 && has_ammo_type {
        (amount - in_clip).max(0)
    } else {
        clips_real
    };
    let info_x = 66;
    if reload > 1 {
        let t = if reloading {
            "---".to_string()
        } else {
            in_clip.to_string()
        };
        c.text(
            f,
            info_x,
            26,
            20,
            9,
            HAlign::Center,
            VAlign::Center,
            col,
            &t,
        );
        let ccol = if full {
            [0, 255, 255]
        } else if clips_real == 0 || (clips_real == 1 && amount < 2 * reload) {
            COL_AMMO_LOW
        } else {
            COL_AMMO
        };
        let t = if reloading {
            "---".to_string()
        } else {
            clips_left.to_string()
        };
        c.text(
            f,
            info_x,
            38,
            20,
            9,
            HAlign::Center,
            VAlign::Center,
            ccol,
            &t,
        );
    } else {
        c.text(
            f,
            info_x,
            38,
            20,
            9,
            HAlign::Center,
            VAlign::Center,
            col,
            "N/A",
        );
        let t = if reload == 0 {
            "N/A".to_string()
        } else if reloading {
            "---".to_string()
        } else {
            amount.to_string()
        };
        c.text(
            f,
            info_x,
            26,
            20,
            9,
            HAlign::Center,
            VAlign::Center,
            col,
            &t,
        );
    }
    if h.can_track {
        c.text(
            f,
            25,
            56,
            65,
            8,
            HAlign::Center,
            VAlign::Center,
            snap.lock_color,
            &snap.lock_text,
        );
    }
}

/// DeusExWeapon.AmmoLeftInClip and NumClips.
fn ammo_counts(reload: i32, has_ammo_type: bool, amount: i32, loaded: i32) -> (i32, i32) {
    let in_clip = if reload == 0 {
        1
    } else if !has_ammo_type || amount == 0 {
        0
    } else {
        loaded.min(amount)
    };
    let clips = if reload == 0 || !has_ammo_type || amount == 0 {
        0
    } else {
        ((amount - in_clip) + (reload - 1)) / reload
    };
    (in_clip, clips)
}

/// Belt slot x positions (UI units inside the belt window): 1..9 left to right, then the key ring slot 0.
fn slot_rect(slot: usize) -> (i32, i32, i32) {
    if slot == 0 {
        (10 + 9 * 51, 6, 44)
    } else {
        (10 + (slot as i32 - 1) * 51, 6, 51)
    }
}

/// HUDObjectBelt + HUDObjectSlot.
fn draw_belt(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    c: &mut Canvas,
    font: Option<&UiFont>,
    snap: &Snapshot,
    inv: &PlayerInventory,
) {
    if let Some(t) = hud.ui(assets, "HUDObjectBeltBackground_Left") {
        c.texture(2, 6, 9, 54, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
    }
    if let Some(t) = hud.ui(assets, "HUDObjectBeltBackground_Right") {
        c.texture(514, 6, 8, 54, 0, 0, &t, crate::menuui::bg_style(), crate::menuui::h_col(0));
    }
    for (name, x, w) in [
        ("HUDObjectBeltBorder_1", 0, 256),
        ("HUDObjectBeltBorder_2", 256, 256),
        ("HUDObjectBeltBorder_3", 512, 29),
    ] {
        if let Some(t) = hud.ui(assets, name) {
            c.texture(x, 0, w, 69, 0, 0, &t, crate::menuui::border_style(), crate::menuui::border_col());
        }
    }
    let cell = hud.ui(assets, "HUDObjectBeltBackground_Cell");
    let borders: [Option<Arc<UiTex>>; 9] = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ]
    .map(|n| hud.ui(assets, &format!("PersonaItemHighlight_{n}")));
    let held_class = snap.held.as_ref().map(|h| h.class.clone());
    for slot in 0..10usize {
        let (sx, sy, sw) = slot_rect(slot);
        let sh = 54;
        // DrawHUDBackground.
        if let Some(t) = &cell {
            c.texture(sx, sy, sw, sh, 0, 0, t, crate::menuui::bg_style(), crate::menuui::h_col(0));
        }
        // What's in the slot: belt entries, or the nano key ring in slot 0.
        let item = inv.0.belt[slot].and_then(|i| inv.0.items.get(i));
        let (class, desc, icon_path, text) = match item {
            Some(it) => {
                let text = slot_text(hud, assets, it, snap, inv);
                (
                    Some(it.def.class.clone()),
                    it.def.belt_name.clone(),
                    it.def.icon.clone(),
                    text,
                )
            }
            None if slot == 0 => (
                Some("NanoKeyRing".to_string()),
                hud.str_default(assets, "NanoKeyRing", "beltDescription")
                    .unwrap_or_else(|| "KEY RING".into()),
                hud.path_default(assets, "NanoKeyRing", "Icon"),
                String::new(),
            ),
            None => (None, String::new(), None, String::new()),
        };
        let selected = class.is_some() && class == held_class;
        let pending = snap.pending == Some(slot);
        // fillMode FM_Selected: a translucent box under the icon.
        if selected {
            c.fill(
                sx + 1,
                sy + 3,
                42,
                37,
                Style::Translucent,
                COL_SLOT_SELECTED,
            );
        }
        if let Some(icon) = icon_path.and_then(|p| hud.tex(assets, &p)) {
            c.texture(
                sx + 1,
                sy + 3,
                42,
                37,
                0,
                0,
                &icon,
                Style::Masked,
                [255, 255, 255],
            );
            if let Some(f) = font {
                c.text(
                    f,
                    sx + 1,
                    sy + 42,
                    42,
                    7,
                    HAlign::Center,
                    VAlign::Center,
                    COL_SLOT_TEXT,
                    &desc,
                );
                if !text.is_empty() {
                    c.text(
                        f,
                        sx + 1,
                        sy + 32,
                        42,
                        8,
                        HAlign::Center,
                        VAlign::Center,
                        COL_SLOT_TEXT,
                        &text,
                    );
                }
            }
            if selected || pending {
                c.borders(
                    sx,
                    sy + 2,
                    44,
                    50,
                    &borders,
                    Style::Masked,
                    COL_SELECTION_BORDER,
                );
            }
        }
        if let Some(f) = font {
            c.text(
                f,
                sx + 37,
                sy + 3,
                6,
                7,
                HAlign::Right,
                VAlign::Center,
                COL_SLOT_TEXT,
                &slot.to_string(),
            );
        }
    }
}

/// HUDObjectSlot.UpdateItemText.
fn slot_text(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    it: &imsim_core::inventory::Item,
    snap: &Snapshot,
    inv: &PlayerInventory,
) -> String {
    let class = it.def.class.clone();
    match it.def.kind {
        ItemKind::Weapon => {
            let d = hud.defaults(assets, &class).to_vec();
            let ammo_name = match find(&d, "AmmoName") {
                Some(Value::Path(p)) => p.rsplit('.').next().unwrap_or("").to_string(),
                _ => "AmmoNone".into(),
            };
            let h2h = matches!(find(&d, "bHandToHand"), Some(Value::Bool(true)));
            let reload = match find(&d, "ReloadCount") {
                Some(Value::Int(n)) => *n,
                Some(Value::Byte(n)) => *n as i32,
                _ => 0,
            };
            let mut text = String::new();
            if ammo_name != "AmmoNone" && !ammo_name.is_empty() && !h2h && reload != 0 {
                let current = snap
                    .loaded_ammo
                    .get(&class)
                    .cloned()
                    .unwrap_or(ammo_name.clone());
                text = hud
                    .str_default(assets, &current, "beltDescription")
                    .unwrap_or_default();
            }
            if matches!(
                class.as_str(),
                "WeaponNanoVirusGrenade" | "WeaponGasGrenade" | "WeaponEMPGrenade" | "WeaponLAM"
            ) {
                let n = hud_ammo(inv, &ammo_name, it);
                if n > 1 {
                    text = format!("COUNT: {n}");
                }
            }
            text
        }
        ItemKind::Pickup { .. } if class != "NanoKeyRing" && it.count > 1 => {
            let label = hud
                .str_default(assets, &class, "CountLabel")
                .unwrap_or_else(|| "COUNT:".into());
            format!("{label} {}", it.count)
        }
        _ => String::new(),
    }
}

/// Grenade counts: the grenade's own ammo pool (AmmoType.AmmoAmount).
fn hud_ammo(inv: &PlayerInventory, ammo: &str, it: &imsim_core::inventory::Item) -> u32 {
    inv.0
        .ammo
        .iter()
        .find(|x| x.0 == ammo)
        .map(|x| x.1)
        .unwrap_or(it.count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_strip_wraps() {
        // Facing true north: the strip's 0 sits mid-window.
        assert_eq!(compass_positions(0, 0).0, 27);
        // A quarter turn moves it 60 units left.
        let (d, w) = compass_positions(16384, 0);
        assert_eq!(d, 27 - 60);
        assert_eq!(w, d + 240);
    }

    #[test]
    fn clips_like_the_script() {
        // Pistol: 6 per clip, 4 loaded, 10 in reserve (14 total): 4 in the clip, 2 clips.
        assert_eq!(ammo_counts(6, true, 14, 4), (4, 2));
        assert_eq!(ammo_counts(6, true, 0, 0), (0, 0));
        assert_eq!(ammo_counts(0, false, 0, 0), (1, 0));
    }

    #[test]
    fn hurt_part_flashes_then_settles() {
        let mut p = BodyPart::default();
        assert_eq!(p.color(0.0, 100), [0, 255, 0]);
        let c = p.color(0.0, 50);
        assert_eq!(c, [255, 255, 255]);
        let c = p.color(0.5, 50);
        assert_eq!(c, color_scaled(0.5));
    }

    /// HUD_PREVIEW=<out.rgba> HUD_GAME=<game root>: draw every window at 2x onto a 1920x1080 frame (raw RGBA,
    /// premultiplied composite over a dim background) to check art and layout without the game running.
    #[test]
    fn preview_frame() {
        let (Ok(out), Ok(root)) = (std::env::var("HUD_PREVIEW"), std::env::var("HUD_GAME")) else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut assets = RuntimeAssets {
            classes: unreal_pkg::classes::ClassDb::open(&root.join("System")),
            lib: dx_level::TexLib::new(&root),
            images: Default::default(),
            materials: Default::default(),
            fire: Vec::new(),
            sprites: Default::default(),
            shared_parts: Default::default(),
        };
        let mut hud = ClassicHud::default();
        hud.true_north = Some(0);
        let mut inv = PlayerInventory::default();
        let mut add =
            |assets: &mut RuntimeAssets, class: &str, kind: ItemKind, count: u32, slot: usize| {
                let d = assets.classes.defaults(class);
                let st = |n: &str| match find(&d, n) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let icon = match find(&d, "Icon") {
                    Some(Value::Path(p)) => Some(p.clone()),
                    _ => None,
                };
                let def = imsim_core::inventory::ItemDef {
                    class: class.into(),
                    name: st("ItemName"),
                    belt_name: st("beltDescription"),
                    icon,
                    kind,
                    ..Default::default()
                };
                inv.0.items.push(imsim_core::inventory::Item {
                    def,
                    count,
                    pos: None,
                    uid: 0,
                    data: Vec::new(),
                    rotated: false,
                });
                inv.0.belt[slot] = Some(inv.0.items.len() - 1);
            };
        add(&mut assets, "WeaponPistol", ItemKind::Weapon, 1, 1);
        add(&mut assets, "WeaponProd", ItemKind::Weapon, 1, 2);
        add(
            &mut assets,
            "MedKit",
            ItemKind::Pickup { max_copies: 15 },
            3,
            3,
        );
        add(&mut assets, "WeaponLAM", ItemKind::Weapon, 1, 4);
        add(
            &mut assets,
            "Lockpick",
            ItemKind::Pickup { max_copies: 20 },
            2,
            5,
        );
        inv.0.ammo.push(("Ammo10mm".into(), 10));
        inv.0.ammo.push(("AmmoLAM".into(), 3));
        let snap = Snapshot {
            parts: [100, 80, 55, 30, 5, 0],
            energy: 70.0,
            // HUD_PREVIEW_STAMINA=1: the sprint stamina bar in the breath bar's place.
            breath: if env_on!("HUD_PREVIEW_STAMINA") { None } else { Some(0.6) },
            stamina: env_on!("HUD_PREVIEW_STAMINA").then_some(0.45),
            yaw: 3000,
            held: Some(HeldView {
                class: "WeaponPistol".into(),
                ammo_class: Some("Ammo10mm".into()),
                reload_count: 6,
                loaded: 4,
                ..Default::default()
            }),
            augs: vec![
                (
                    5,
                    "DeusExUI.UserInterface.AugIconDefense_Small".into(),
                    false,
                ),
                (
                    7,
                    "DeusExUI.UserInterface.AugIconSpeedJump_Small".into(),
                    true,
                ),
                (12, "DeusExUI.UserInterface.AugIconLight_Small".into(), true),
            ],
            items: vec![("TechGoggles".into(), 0.6)],
            ..Default::default()
        };
        let tiny = hud.font(&mut assets, "FontTiny");
        assert!(tiny.is_some(), "FontTiny");
        let log_font = hud.font(&mut assets, "FontMenuSmall_DS");
        hud.add_log("You picked up the Medkit");
        hud.add_log("Healed 30 points\nThis is a much longer message that should wrap round onto a second line of the message log box at the top of the screen");
        hud.add_damage("Shot", glam::DVec3::new(-5.0, 10.0, 0.0));
        hud.add_damage("Radiation", glam::DVec3::ZERO);
        hud.damage[6].age = 0.5;
        for cl in ["WeaponCombatKnife", "Ammo10mm", "Credits"] {
            let icon = hud.path_default(&mut assets, cl, "Icon");
            let label = hud
                .str_default(&mut assets, cl, "beltDescription")
                .unwrap_or_default();
            hud.received.push((icon, label));
        }
        let (fw, fh, s) = (1920i32, 1080i32, 2i32);
        let (sw, sh) = (fw / s, fh / s);
        let mut frame = vec![[0.16f32, 0.18, 0.2]; (fw * fh) as usize];
        let log_w = sw - 84 - 40;
        let lines = wrap_log(log_font.as_ref().unwrap(), &hud.log, log_w - 72);
        let log_h =
            (LOG_TOP + 11 * lines.len().max(LOG_LINES) as i32 + LOG_MARGIN).max(LOG_MIN_HEIGHT);
        for w in Win::ALL {
            let (mut ww, mut wh) = w.size();
            let (x, y) = match w {
                Win::Hit => (0, 0),
                Win::Compass => (0, 110),
                Win::Ammo => (0, sh - wh),
                Win::Belt => (sw - ww, sh - wh),
                Win::Log => {
                    (ww, wh) = (log_w, log_h);
                    (104, 10)
                }
                Win::Damage => (0, 150),
                Win::Active => {
                    (ww, wh) = active_size(&snap, sh - 69);
                    (sw - ww, 0)
                }
                Win::Received => {
                    let (h, sm) = (
                        hud.font(&mut assets, "FontMenuHeaders_DS").unwrap(),
                        hud.font(&mut assets, "FontMenuSmall_DS").unwrap(),
                    );
                    (ww, wh) = received_size(&h, &sm, &hud.received);
                    (sw / 2 - ww / 2, sh / 2 + 20)
                }
                _ => continue,
            };
            let mut c = Canvas::new(ww, wh, s);
            match w {
                Win::Hit => draw_hit(&mut hud, &mut assets, &mut c, tiny.as_deref(), &snap, 0.0),
                Win::Compass => draw_compass(&mut hud, &mut assets, &mut c, &snap),
                Win::Ammo => draw_ammo(&mut hud, &mut assets, &mut c, tiny.as_deref(), &snap, &inv),
                Win::Belt => draw_belt(&mut hud, &mut assets, &mut c, tiny.as_deref(), &snap, &inv),
                Win::Log => draw_log(&mut hud, &mut assets, &mut c, log_font.as_deref(), &lines),
                Win::Damage => draw_damage(&mut hud, &mut assets, &mut c),
                Win::Received => {
                    let (h, sm) = (
                        hud.font(&mut assets, "FontMenuHeaders_DS").unwrap(),
                        hud.font(&mut assets, "FontMenuSmall_DS").unwrap(),
                    );
                    draw_received(&mut hud, &mut assets, &mut c, &h, &sm);
                }
                Win::Active => draw_active(
                    &mut hud,
                    &mut assets,
                    &mut c,
                    tiny.as_deref(),
                    &snap,
                    sh - 69,
                ),
                _ => {}
            }
            for py in 0..wh * s {
                for px in 0..ww * s {
                    let p = c.px[(py * ww * s + px) as usize];
                    let f = &mut frame[((y * s + py) * fw + x * s + px) as usize];
                    for k in 0..3 {
                        f[k] = (p[k] + f[k] * (1.0 - p[3])).min(1.0);
                    }
                }
            }
        }
        let bytes: Vec<u8> = frame
            .iter()
            .flat_map(|p| {
                [
                    (p[0] * 255.0) as u8,
                    (p[1] * 255.0) as u8,
                    (p[2] * 255.0) as u8,
                    255,
                ]
            })
            .collect();
        std::fs::write(out, bytes).unwrap();
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = START.lock() {
        *v = None;
    }
    if let Ok(mut v) = RECEIVED.lock() {
        v.clear();
    }
    if let Ok(mut v) = MESSAGES.lock() {
        v.clear();
    }
}
