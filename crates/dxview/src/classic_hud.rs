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
    std::env::var("DXVIEW_GAMMA").ok().and_then(|v| v.parse().ok()).unwrap_or_else(crate::settings::gamma)
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
    const ALL: [Win; 15] = [
        Win::Hit,
        Win::Compass,
        Win::Ammo,
        Win::Belt,
        Win::Log,
        Win::Damage,
        Win::Received,
        Win::Active,
        Win::InfoLink,
        Win::ConFirst,
        Win::ConUpper,
        Win::ConLower,
        Win::Bark,
        Win::ConCursor,
        Win::Start,
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
            | Win::Start => (0, 0),
            Win::ConCursor => (64, 64),
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
        self.tex(assets, &format!("{UI}{name}"))
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
        self.defaults
            .entry(class.to_string())
            .or_insert_with(|| assets.classes.defaults(class))
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
struct HeldView {
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
struct Snapshot {
    parts: [i32; 6],
    energy: f32,
    breath: Option<f32>,
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
    mut swim_clock: Local<f32>,
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
    let to_logical = |v: i32| v as f32 * s as f32 / window.scale_factor();

    if hud.true_north.is_none() {
        let n = level
            .0
            .actors
            .iter()
            .find(|a| a.class == "DeusExLevelInfo")
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
    let p = &ph.parts;
    let snap = Snapshot {
        parts: [P_HEAD, P_TORSO, P_ARM_L, P_ARM_R, P_LEG_L, P_LEG_R]
            .map(|i| p[i].max(0.0).round() as i32),
        energy: ph.energy,
        breath: ph.breath,
        swim_time: *swim_clock,
        yaw,
        dead: ph.dead,
        held: weapons.held.as_ref().map(|h| HeldView {
            class: h.stats.class.clone(),
            ammo_class: h.stats.ammo_class.clone(),
            reload_count: h.stats.reload_count,
            loaded: *clips.0.get(&h.stats.class).unwrap_or(&0) as i32,
            reloading: h.state == WState::Reloading,
            can_track: h.stats.can_track,
            item: h.stats.item || h.stats.class == "POVCorpse",
        }),
        pending: weapons.pending,
        loaded_ammo: weapons.loaded_ammo.clone(),
        lock_text: lock.0 .0.clone(),
        lock_color: {
            let c = lock.1 .0.to_srgba();
            [
                (c.red * 255.0) as u8,
                (c.green * 255.0) as u8,
                (c.blue * 255.0) as u8,
            ]
        },
        // bHUDShowAllAugs: every installed aug that can be toggled, lit while on.
        augs: {
            let g = crate::abilities::lock();
            let mut v: Vec<(i32, String, bool)> = g
                .as_ref()
                .map(|a| {
                    a.augs
                        .list
                        .iter()
                        .filter(|x| x.has && !x.def.always_active)
                        .map(|x| (x.hotkey, x.def.small_icon.clone(), x.active))
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by_key(|x| x.0);
            v
        },
        items: charged
            .0
            .iter()
            .map(|c| (c.class.clone(), (c.charge / c.max.max(1.0)).clamp(0.0, 1.0)))
            .collect(),
    };

    // Player messages (ClientMessage) go to the message log; the death prompt and keypad entry line stay where
    // they are. A message is new when its text changes or its timer is set again.
    if msg.text != hud.seen_msg.0 || msg.time_left > hud.seen_msg.1 + 0.01 {
        if !msg.text.is_empty() && msg.time_left > 1.0 && msg.time_left <= 100.0 {
            let t = msg.text.clone();
            hud.add_log(&t);
            msg.time_left = 0.0;
        }
    }
    hud.seen_msg = (msg.text.clone(), msg.time_left);
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
    let layout = |w: Win| -> (i32, i32, i32, i32, bool) {
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
                let (aw, ah) = active_size(&snap, sh - Win::Belt.size().1);
                (
                    sw - aw,
                    0,
                    aw.max(1),
                    ah.max(1),
                    (!snap.augs.is_empty() || !snap.items.is_empty()) && !snap.dead && !menu,
                )
            }
        }
    };

    let tiny = hud.font(assets, "FontTiny");
    for w in Win::ALL {
        let (x, y, ww, wh, visible) = layout(w);
        // Enhancement: HUD customisation (each part hidden, sized and faded on its own).
        let (show, k, alpha) = hud_part(w);
        let visible = visible && show;
        if !visible {
            // Hidden windows keep their last image: no canvas, no upload (a hidden ConLower alone was a
            // screen-wide f32 canvas allocated, cleared and converted every frame).
            for (hw, _, mut vis, _) in &mut nodes {
                if hw.0 == w && *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
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
            Win::ConFirst => "hud::ConFirst",
            Win::ConUpper => "hud::ConUpper",
            Win::ConLower => "hud::ConLower",
            Win::Bark => "hud::Bark",
            Win::ConCursor => "hud::ConCursor",
            Win::Start => "hud::Start",
        });
        // Windows that only change with what they show are redrawn only when that changes.
        let key: Option<u64> = match w {
            Win::Belt => {
                let mut ammo: Vec<_> = snap.loaded_ammo.iter().collect();
                ammo.sort();
                Some(hkey(&(
                    format!("{:?}", inv.0.belt),
                    inv.0
                        .belt
                        .iter()
                        .flatten()
                        .map(|&i| format!("{:?}", inv.0.items.get(i)))
                        .collect::<Vec<_>>(),
                    snap.held
                        .as_ref()
                        .map(|h| (h.class.clone(), h.ammo_class.clone(), h.loaded)),
                    snap.pending,
                    format!("{ammo:?}{:?}", inv.0.ammo),
                    (ww, wh, s),
                )))
            }
            Win::ConCursor => Some(hkey(&(ww, wh, s))),
            // The hit display only changes with its colours and bars.
            Win::Hit => {
                let cols: [Rgb; 6] = std::array::from_fn(|i| hud.parts[i].color(dt, snap.parts[i]));
                hud.hit_cols = cols;
                let blink = snap.breath.is_some_and(|b| b < 0.1) && snap.swim_time % 0.5 > 0.25;
                Some(hkey(&(
                    cols,
                    (snap.energy.clamp(0.0, 100.0) * 0.55) as i32,
                    snap.breath.map(|b| (b.clamp(0.0, 1.0) * 55.0) as i32),
                    blink,
                    (ww, wh, s),
                )))
            }
            // The compass only changes when the tick strip moves a pixel.
            Win::Compass => Some(hkey(&(compass_positions(snap.yaw, hud.true_north.unwrap_or(0)), (ww, wh, s)))),
            Win::Active => Some(hkey(&(
                format!("{:?}", snap.augs),
                snap.items
                    .iter()
                    .map(|(c, f)| (c.clone(), (f * 1000.0) as i32))
                    .collect::<Vec<_>>(),
                (ww, wh, s),
            ))),
            _ => None,
        };
        // A settings change (theme, translucency, borders) redraws everything.
        let key = key.map(|k| k ^ crate::settings::generation().wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let unchanged = key.is_some() && hud.keys.get(&w) == key.as_ref() && hud.last.contains_key(&w);
        if !unchanged {
        if let Some(k) = key {
            hud.keys.insert(w, k);
        } else {
            hud.keys.remove(&w);
        }
        let mut canvas = Canvas::new(ww, wh, s);
        {
            match w {
                Win::Hit => draw_hit(hud, assets, &mut canvas, tiny.as_deref(), &snap, dt),
                Win::Compass => draw_compass(hud, assets, &mut canvas, &snap),
                Win::Ammo => draw_ammo(hud, assets, &mut canvas, tiny.as_deref(), &snap, &inv),
                Win::Belt => draw_belt(hud, assets, &mut canvas, tiny.as_deref(), &snap, &inv),
                Win::Log => draw_log(hud, assets, &mut canvas, log_font.as_deref(), &log_lines),
                Win::Damage => draw_damage(hud, assets, &mut canvas),
                Win::Received => {
                    if let (Some(h), Some(sm)) = (&rec_head, &rec_small) {
                        draw_received(hud, assets, &mut canvas, h, sm);
                    }
                }
                Win::Active => draw_active(
                    hud,
                    assets,
                    &mut canvas,
                    tiny.as_deref(),
                    &snap,
                    sh - Win::Belt.size().1,
                ),
                Win::InfoLink => {
                    if let Some(l) = &con.link {
                        crate::conui::draw_infolink(hud, assets, &mut canvas, l);
                    }
                }
                Win::ConFirst => {
                    if let Some(p) = &con.play {
                        crate::conui::draw_first(hud, assets, &mut canvas, p);
                    }
                }
                Win::ConUpper => crate::conui::draw_upper(hud, assets, &mut canvas),
                Win::Bark => crate::conui::draw_barks(hud, assets, &mut canvas, &con.barks),
                Win::ConCursor => crate::inventory_screen::cursor_with_shadow(hud, assets, &mut canvas),
                Win::Start => {
                    if let Some(f) = &start_font {
                        for (i, l) in start_lines.iter().enumerate() {
                            let y = i as i32 * f.height;
                            canvas.text(f, 2, y + 2, ww, f.height, HAlign::Left, VAlign::Top, [0, 0, 0], l);
                            canvas.text(f, 0, y, ww, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], l);
                        }
                    }
                }
                Win::ConLower => {
                    crate::conui::LOWER_Y.store(y, std::sync::atomic::Ordering::Relaxed);
                    if let Some(p) = &con.play {
                        crate::conui::draw_lower(hud, assets, &mut canvas, p, sw);
                    } else {
                        // CinematicWindow: the letterbox alone.
                        canvas.fill(0, 0, ww, wh, crate::uicanvas::Style::Normal, [0, 0, 0]);
                    }
                }
            }
        }
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
            for (hw, _, _, mat) in &nodes {
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
        for (hw, mut node, mut vis, mat) in &mut nodes {
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

/// (shown, size, opacity) of a HUD window: settings HUD_<Part>_Hide / _Size / _Fade.
fn hud_part(w: Win) -> (bool, f32, f32) {
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
        if let Some(f) = hud.font(assets, "TechSmall") {
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
    if let Some(b) = breath_pct {
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
            if crate::settings::get("AmmoTotal") > 0.5 { "TOTAL" } else { "CLIPS" },
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
    let full = crate::settings::get("AmmoMaxCyan") > 0.5 && has_ammo_type && amount >= max_ammo;
    let col = if full {
        [0, 255, 255]
    } else if amount < low_mark {
        COL_AMMO_LOW
    } else {
        COL_AMMO
    };
    let (in_clip, clips_real) = ammo_counts(reload, has_ammo_type, amount, loaded);
    let clips_left = if crate::settings::get("AmmoTotal") > 0.5 && has_ammo_type {
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
                None,
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
            breath: Some(0.6),
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
