//! CreditsWindow / QuotesWindow on CreditsScrollWindow.
//!
//! Credits: after the endgame quote (MissionEndgame.FinishCinematic -> ShowCredits(True)) the screen goes black
//! (rendering off), Credits_Music plays and DeusExText.DeusExCredits scrolls up from the bottom one UI pixel every
//! currentScrollSpeed seconds (0.04): the banner (CreditsBanner_1/2, 505 x 75), the text (<B> headers in
//! FontConversationLargeBold white, the rest FontConversationLarge (200,200,200), 550 wide, centred, empty
//! paragraphs = 15-unit blank lines), the team photo (TeamFront_1..3, 600 x 236). FadeTop / FadeBottom darken the
//! top and bottom 64 units (DSTY_Modulated). Up / Equals speed up, Down / Minus slow down, Space pauses, a click
//! ends it (FinishedScrolling), Escape closes it. When it ends the player goes back to the title map (bLoadIntro:
//! SendPlayer "dxonly"; ours is the DX.dx menu map).
//!
//! The easter egg (CreditsWindow.KeyPressed, cheats only = our Debug Mode): letters and digits typed during the
//! credits build a phrase; a letter that keeps it part of one of easterStrings clicks (EggGoodLetterSounds), one
//! that doesn't buzzes (EggBadLetterSounds, from the second letter on) and starts over; a whole phrase plays an
//! EggFoundSounds sound and for 3 s spawns up to 4 FadeTextWindows a frame with its foundStrings line (FontLocation,
//! translucent, jittering, a random hue fading out). QUOTES turns bQuotesEnabled on (saved: settings QuotesEnabled);
//! DANCEPARTY opens 99_Endgame4. With quotes on, the end of the credits plays CreditsEnd and 4 s later opens the
//! QuotesWindow: DeusExText.DeusExQuotes in green (headers 192,255,192, text 128,255,128; "Name:quote" lines as
//! AlignWindow label + wrapped text, 5 apart), TeamBack_1..3 (600 x 214), Quotes_Music, ten SpinningLogoWindows
//! bouncing under gravity 120 with a 48-frame FontSpinningDX spin and a cycling hue (Insert adds one, Delete removes
//! one, End removes all); Escape, a click or the end clears the stack (and loads the title map after the endgame).
//! THEREISNOSPOON (the Matrix view) and IAMWARREN (Warren's EMP field) act through eggs.rs; they also show
//! their phrase. Deviation: the original's colour maths writes Int(c * 256) - 1 into a byte, so a channel at 0
//! wraps to 255 (fading text and logos flash white); we clamp it to 0.

use crate::classic_hud::HudLayer;
use crate::uicanvas::{Canvas, HAlign, Style, UiFont, UiTex, VAlign};
use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static SHOW: AtomicBool = AtomicBool::new(false);
static SHOW_QUOTES: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicBool = AtomicBool::new(false);

static LOAD_INTRO: AtomicBool = AtomicBool::new(true);

/// DeusExPlayer.ShowCredits(True): after the endgame; the title map follows.
pub fn show() {
    LOAD_INTRO.store(true, Ordering::Relaxed);
    SHOW.store(true, Ordering::Relaxed);
}

/// The main menu's Credits button (bLoadIntro False): back to the menu afterwards.
pub fn show_from_menu() {
    LOAD_INTRO.store(false, Ordering::Relaxed);
    SHOW.store(true, Ordering::Relaxed);
}

/// DeusExPlayer.ShowQuotesWindow (cheats): the quotes on their own, back to the game afterwards.
pub fn show_quotes() {
    LOAD_INTRO.store(false, Ordering::Relaxed);
    SHOW_QUOTES.store(true, Ordering::Relaxed);
}

/// The credits (or quotes) have the screen and the keys.
pub fn open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

#[derive(Component)]
pub struct CreditsNode(usize);

/// The overlay (fading phrases, spinning logos) and the two fade bars.
const OVERLAY: usize = usize::MAX - 1;
const FADE_TOP: usize = usize::MAX - 2;
const FADE_BOTTOM: usize = usize::MAX - 3;
const BACKDROP: usize = usize::MAX;

const TILE_W: i32 = 600;
const TEXT_W: i32 = 550;
const BLANK: i32 = 15;
const CHUNK: i32 = 512;

const EASTER: [&str; 7] = ["QUOTES", "DANCEPARTY", "THEREISNOSPOON", "HUTHUT", "BIGHEAD", "IAMWARREN", "MOREFROGS"];
const FOUND: [&str; 7] = [
    "QUOTES ENABLED!",
    "DANCE PARTY ENABLED!",
    "MAY TRICKS MODE ENABLED!",
    "HUT HUT HUT HUT HUT!",
    "SO YOU THINK YOU'RE SMART?",
    "GENERAL PROTECTION FAULT!",
    "I HAVE TEN FINGERS AND TEN TOES!",
];
/// EggBadLetterSounds has eight slots and seven sounds: Rand(8) sometimes plays nothing.
const BAD: [Option<&str>; 8] = [
    Some("DeusExSounds.Generic.Buzz1"),
    Some("DeusExSounds.Generic.LargeExplosion1"),
    Some("DeusExSounds.Generic.GlassBreakLarge"),
    Some("DeusExSounds.Generic.SplashLarge"),
    Some("DeusExSounds.Animal.CatDie"),
    Some("DeusExSounds.NPC.ChildDeath"),
    Some("DeusExSounds.Special.FlushToilet"),
    None,
];
const GOOD: [&str; 5] = [
    "DeusExSounds.Generic.KeyboardClick1",
    "DeusExSounds.Generic.KeyboardClick2",
    "DeusExSounds.Generic.KeyboardClick1",
    "DeusExSounds.Generic.KeyboardClick2",
    "DeusExSounds.Generic.KeyboardClick3",
];
const EGG_FOUND: [&str; 3] = ["DeusExSounds.Special.Airplane2", "DeusExSounds.Generic.Beep2", "DeusExSounds.Generic.Foghorn"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Credits,
    Quotes,
}

enum Item {
    Blank,
    Text(String, bool),
    /// PrintAlignText: a label and its wrapped text beside it.
    Align(String, String),
    Picture(Vec<Arc<UiTex>>, i32, i32, i32),
}

/// FadeTextWindow.
struct Phrase {
    text: String,
    x: f32,
    y: f32,
    move_x: f32,
    move_y: f32,
    fade_speed: f32,
    fade_timer: f32,
    hue: f32,
    lum: f32,
    col: [u8; 3],
}

/// SpinningLogoWindow.
struct Logo {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    frame: u32,
    frame_timer: f32,
    color_timer: f32,
    color_value: u32,
    col: [u8; 3],
}

pub struct Scroll {
    mode: Mode,
    /// Top of the strip in UI units from the top of the screen.
    y: i32,
    height: i32,
    chunks: usize,
    timer: f32,
    speed: f32,
    paused: bool,
    scale: i32,
    /// bScrolling: false once the strip has gone off the top.
    scrolling: bool,
    /// CreditsWindow: the CreditsEnd sound playing before the quotes open.
    end_timer: Option<f32>,
    egg: String,
    egg_phrase: usize,
    egg_timer: f32,
    show_phrases: bool,
    phrases: Vec<Phrase>,
    logos: Vec<Logo>,
    /// Root size in UI units.
    root: (i32, i32),
    overlay: Option<Handle<Image>>,
    overlay_drawn: bool,
}

fn frand() -> f32 {
    crate::sound::frand()
}
fn rand(n: i32) -> i32 {
    if n <= 0 {
        0
    } else {
        ((frand() * n as f32) as i32).min(n - 1)
    }
}

/// FadeTextWindow / SpinningLogoWindow HSLToRGB.
fn hsl_to_rgb(h: f32, sl: f32, l: f32) -> [f32; 3] {
    let v = if l <= 0.5 { l * (1.0 + sl) } else { l + sl - l * sl };
    if v <= 0.0 {
        return [0.0; 3];
    }
    let m = l + l - v;
    let sv = (v - m) / v;
    let h = h * 6.0;
    let sextant = h as i32;
    let fract = h - sextant as f32;
    let vsf = v * sv * fract;
    let (mid1, mid2) = (m + vsf, v - vsf);
    match sextant {
        0 => [v, mid1, m],
        1 => [mid2, v, m],
        2 => [m, v, mid1],
        3 => [m, mid2, v],
        4 => [mid1, m, v],
        _ => [v, m, mid2],
    }
}

/// colText.r = Int(r * 256) - 1 (clamped; see the module note).
fn to_col(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| ((v * 256.0) as i32 - 1).clamp(0, 255) as u8)
}

fn items(assets: &mut crate::weapons::RuntimeAssets, mode: Mode) -> Vec<Item> {
    let mut v = Vec::new();
    let pic = |assets: &mut crate::weapons::RuntimeAssets, names: &[&str]| -> Vec<Arc<UiTex>> {
        names
            .iter()
            .filter_map(|n| assets.lib.load_image_masked(&format!("DeusExUI.UserInterface.{n}")))
            .map(|t| Arc::new(UiTex::from_rgba(t.width, t.height, &t.mips[0])))
            .collect()
    };
    if mode == Mode::Credits {
        v.push(Item::Picture(pic(assets, &["CreditsBanner_1", "CreditsBanner_2"]), 2, 505, 75));
        v.push(Item::Blank);
    }
    let name = if mode == Mode::Credits { "DeusExCredits" } else { "DeusExQuotes" };
    let text = unreal_pkg::Package::open(assets.lib.system_dir().join("DeusExText.u"))
        .ok()
        .and_then(|p| p.text_named(name))
        .unwrap_or_default();
    // DeusExTextParser: <P> starts a paragraph, <B> makes the next text a header; an empty paragraph is a blank line.
    for para in text.split("<P>").skip(1) {
        let bold = para.contains("<B>");
        let mut t = para.replace("<B>", "");
        while let (Some(a), Some(b)) = (t.find('<'), t.find('>')) {
            if b < a {
                break;
            }
            t.replace_range(a..=b, "");
        }
        // A line break straight after <P> is an empty text (a blank line) before what follows.
        let lead_break = para.trim_start_matches([' ', '\t']).starts_with(['\r', '\n']);
        let t = t.replace(['\r', '\n'], "");
        let t = t.trim();
        if t.is_empty() {
            v.push(Item::Blank);
        } else {
            if lead_break {
                v.push(Item::Blank);
            }
            // QuotesWindow.ProcessQuoteLine: "Name:quote" prints as a label beside its text.
            match (mode, bold, t.find(':')) {
                (Mode::Quotes, false, Some(c)) => v.push(Item::Align(t[..=c].to_string(), t[c + 1..].to_string())),
                _ => v.push(Item::Text(t.to_string(), bold)),
            }
        }
    }
    v.push(Item::Blank);
    match mode {
        Mode::Credits => {
            v.push(Item::Picture(pic(assets, &["TeamFront_1", "TeamFront_2", "TeamFront_3"]), 3, 600, 236));
            // This port's own thanks, after the original team.
            v.push(Item::Blank);
            v.push(Item::Text("DX RUST".into(), true));
            v.push(Item::Blank);
            v.push(Item::Text("Rebuilt human bodies, rig and animations".into(), true));
            v.push(Item::Text("The Machine God team".into(), false));
            v.push(Item::Blank);
            v.push(Item::Text("Smooth lip sync timings".into(), true));
            v.push(Item::Text("Joe Wintergreen".into(), false));
            v.push(Item::Blank);
            v.push(Item::Text("Supported mods".into(), true));
            v.push(Item::Text("The Lay D Denton Project team".into(), false));
            v.push(Item::Text("The HDTP team, the New Vision team".into(), false));
            v.push(Item::Text("Visible Attachments, Augmentique, Helios, Precipitation".into(), false));
            v.push(Item::Text("GMDX: CyberP, Sarge and the Augmented Edition team".into(), false));
            v.push(Item::Blank);
            v.push(Item::Text("Built with Bevy and Rust".into(), false));
        }
        Mode::Quotes => v.push(Item::Picture(pic(assets, &["TeamBack_1", "TeamBack_2", "TeamBack_3"]), 3, 600, 214)),
    }
    v
}

/// Lay the strip out and draw it into CHUNK-high canvases.
fn render(assets: &mut crate::weapons::RuntimeAssets, s: i32, mode: Mode) -> (i32, Vec<Canvas>) {
    let head = crate::classic_hud::load_font(assets, "FontConversationLargeBold").map(Arc::new);
    let body = crate::classic_hud::load_font(assets, "FontConversationLarge").map(Arc::new);
    let (col_head, col_text) = match mode {
        Mode::Credits => ([255, 255, 255], [200, 200, 200]),
        Mode::Quotes => ([192, 255, 192], [128, 255, 128]),
    };
    let list = items(assets, mode);
    let left = (TILE_W - TEXT_W) / 2;
    // What to draw: (x, y, width, lines, font, colour) runs of text and pictures at y.
    struct Run {
        x: i32,
        y: i32,
        w: i32,
        ha: HAlign,
        lines: Vec<String>,
        font: Arc<UiFont>,
        col: [u8; 3],
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut pics: Vec<(i32, &Item)> = Vec::new();
    let mut y = 0;
    for it in &list {
        match it {
            Item::Blank => y += BLANK,
            Item::Text(t, bold) => {
                let Some(font) = (if *bold { head.clone() } else { body.clone() }) else { continue };
                let lines = crate::uicanvas::wrap(&font, t, TEXT_W);
                let h = lines.len() as i32 * font.height;
                runs.push(Run { x: left, y, w: TEXT_W, ha: HAlign::Center, lines, font, col: if *bold { col_head } else { col_text } });
                y += h;
            }
            Item::Align(label, text) => {
                let Some(font) = body.clone() else { continue };
                // AlignWindow (child spacing 5): the label at its own width, the text wrapped in what's left.
                let lw = font.width(label);
                let tw = (TEXT_W - lw - 5).max(40);
                let lines = crate::uicanvas::wrap(&font, text, tw);
                let h = lines.len().max(1) as i32 * font.height;
                runs.push(Run { x: left, y, w: lw, ha: HAlign::Left, lines: vec![label.clone()], font: font.clone(), col: col_text });
                runs.push(Run { x: left + lw + 5, y, w: tw, ha: HAlign::Left, lines, font, col: col_text });
                y += h;
            }
            Item::Picture(_, _, _, h) => {
                pics.push((y, it));
                y += h;
            }
        }
    }
    let height = y;
    let n = ((height + CHUNK - 1) / CHUNK).max(1);
    let mut out = Vec::new();
    for c in 0..n {
        let top = c * CHUNK;
        let mut cv = Canvas::new(TILE_W, CHUNK, s);
        for r in &runs {
            for (i, l) in r.lines.iter().enumerate() {
                let ly = r.y + i as i32 * r.font.height - top;
                if ly + r.font.height < 0 || ly > CHUNK {
                    continue;
                }
                cv.text(&r.font, r.x, ly, r.w, r.font.height, r.ha, VAlign::Top, r.col, l);
            }
        }
        for (py, it) in &pics {
            let Item::Picture(texs, cols, w, h) = it else { continue };
            let py = py - top;
            if py + h < 0 || py > CHUNK {
                continue;
            }
            // PictureWindow: the textures side by side, the whole picture centred in the tile.
            let mut x = (TILE_W - w) / 2;
            for t in texs.iter().take(*cols as usize) {
                let tw = (t.w as i32).min(w - (x - (TILE_W - w) / 2));
                cv.texture(x, py, tw, (*h).min(t.h as i32), 0, 0, t, Style::Normal, [255, 255, 255]);
                x += t.w as i32;
            }
        }
        out.push(cv);
    }
    (height, out)
}

fn image(w: u32, h: u32, px: Vec<u8>, nearest: bool) -> Image {
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        px,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    if nearest {
        img.sampler = bevy::image::ImageSampler::nearest();
    }
    img
}

/// FadeTop / FadeBottom drawn DSTY_Modulated over black-backed text: dst x 2t = premultiplied black at 1 - 2t.
fn fade_image(assets: &mut crate::weapons::RuntimeAssets, name: &str) -> Option<(Image, u32)> {
    let t = assets.lib.load_image_masked(&format!("DeusExUI.UserInterface.{name}"))?;
    let mut px = Vec::with_capacity((t.width * t.height * 4) as usize);
    for c in t.mips[0].chunks_exact(4) {
        let a = (1.0 - (c[0] as f32 / 255.0 * 2.0)).clamp(0.0, 1.0);
        px.extend_from_slice(&[0, 0, 0, (a * 255.0 + 0.5) as u8]);
    }
    Some((image(t.width, t.height, px, false), t.height))
}

fn new_logo(root: (i32, i32)) -> Logo {
    Logo {
        x: rand(root.0) as f32,
        y: rand(root.1) as f32,
        dx: (rand(320) - 160) as f32,
        dy: (rand(320) - 160) as f32,
        frame: 0,
        frame_timer: 0.1,
        color_timer: 0.1,
        color_value: rand(256) as u32,
        col: [255, 255, 255],
    }
}

/// SpinningLogoWindow.Tick in a parent `root` big (32 x 32 windows).
fn tick_logo(l: &mut Logo, dt: f32, root: (i32, i32)) {
    l.frame_timer -= dt;
    if l.frame_timer < 0.0 {
        l.frame_timer = 0.1;
        l.frame = (l.frame + 1) % 48;
    }
    l.color_timer -= dt;
    if l.color_timer < 0.0 {
        l.color_timer = 0.1;
        l.color_value = (l.color_value + 1) % 256;
        l.col = to_col(hsl_to_rgb(l.color_value as f32 / 256.0, 0.7, 0.7));
    }
    let (pw, ph) = ((root.0 - 32) as f32, (root.1 - 32) as f32);
    let mut nx = l.x + l.dx * dt;
    if nx > pw {
        nx = 2.0 * pw - nx;
        l.dx = -l.dx;
    } else if nx < 0.0 {
        nx = -nx;
        l.dx = -l.dx;
    }
    let mut ny = l.y + l.dy * dt;
    if ny > ph {
        ny = 2.0 * ph - ny;
        l.dy = -l.dy;
    } else if ny < 0.0 {
        ny = -ny;
        l.dy = -l.dy;
    }
    l.dy += 120.0 * dt;
    l.x = nx;
    l.y = ny;
}

/// FadeTextWindow.Tick: false once it has faded out.
fn tick_phrase(p: &mut Phrase, dt: f32) -> bool {
    p.move_x -= dt;
    if p.move_x < 0.0 {
        p.x += (rand(4) - 2) as f32;
        p.move_x = frand();
    }
    p.move_y -= dt;
    if p.move_y < 0.0 {
        p.y += (rand(4) - 2) as f32;
        p.move_y = frand();
    }
    p.fade_timer -= dt;
    if p.fade_timer < 0.0 {
        p.fade_timer = p.fade_speed;
        p.lum -= 0.01;
        if p.lum <= 0.0 {
            return false;
        }
        p.col = to_col(hsl_to_rgb(p.hue, frand(), p.lum));
    }
    true
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn credits_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    windows: Query<&Window>,
    mut assets: Option<ResMut<crate::weapons::RuntimeAssets>>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    mut binds: ResMut<crate::keybinds::Bindings>,
    mut nodes: Query<(Entity, &CreditsNode, &mut Node, &mut Visibility)>,
    mut world_cams: Query<&mut Camera, Or<(With<crate::FlyCam>, With<crate::SkyCam>)>>,
    mut st: Local<Option<Scroll>>,
    mut fonts: Local<Option<(Option<Arc<UiFont>>, Option<Arc<UiFont>>)>>,
) {
    let Some(win) = windows.iter().next() else { return };
    // Developer aids: DXVIEW_CREDITS=1 rolls the credits at start, DXVIEW_QUOTES=1 the quotes (to look at them).
    static DEV: std::sync::Once = std::sync::Once::new();
    if std::env::var("DXVIEW_CREDITS").is_ok() {
        DEV.call_once(show_from_menu);
    } else if std::env::var("DXVIEW_QUOTES").is_ok() {
        DEV.call_once(show_quotes);
    }
    let want = if SHOW_QUOTES.swap(false, Ordering::Relaxed) {
        Some(Mode::Quotes)
    } else if SHOW.swap(false, Ordering::Relaxed) {
        Some(Mode::Credits)
    } else {
        None
    };
    if let Some(mode) = want {
        if st.as_ref().is_some_and(|s| s.mode == mode) {
            // Already showing.
        } else if let Some(assets) = assets.as_mut() {
            // A new scroll replaces the one showing (the credits hand over to the quotes).
            for (e, ..) in &nodes {
                commands.entity(e).despawn();
            }
            let s = crate::hud::ui_scale(win).max(1.0) as i32;
            let (height, chunks) = render(assets, s, mode);
            let k = s as f32 / win.scale_factor();
            let full = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            };
            // The black screen (EnableRendering(False) + a Solid black background).
            commands.spawn((CreditsNode(BACKDROP), full.clone(), BackgroundColor(Color::BLACK), GlobalZIndex(900)));
            let n = chunks.len();
            for (i, c) in chunks.into_iter().enumerate() {
                let img = images.add(image((TILE_W * s) as u32, (CHUNK * s) as u32, c.to_rgba8(), false));
                commands.spawn((
                    CreditsNode(i),
                    MaterialNode(layers.add(HudLayer::new(img))),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(((win.width() - TILE_W as f32 * k) / 2.0).round()),
                        top: Val::Px(win.height()),
                        width: Val::Px(TILE_W as f32 * k),
                        height: Val::Px(CHUNK as f32 * k),
                        ..default()
                    },
                    GlobalZIndex(901),
                ));
            }
            for (name, id) in [("FadeTop", FADE_TOP), ("FadeBottom", FADE_BOTTOM)] {
                if let Some((img, h)) = fade_image(assets, name) {
                    let hpx = (h as f32 * k).round();
                    let img = images.add(img);
                    commands.spawn((
                        CreditsNode(id),
                        MaterialNode(layers.add(HudLayer::new(img))),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: if id == FADE_TOP { Val::Px(0.0) } else { Val::Px(win.height() - hpx) },
                            width: Val::Percent(100.0),
                            height: Val::Px(hpx),
                            ..default()
                        },
                        GlobalZIndex(902),
                    ));
                }
            }
            // Phrases and logos: one canvas at 1x the root, drawn nearest-neighbour at the UI scale.
            let root = ((win.physical_width() as i32) / s, (win.physical_height() as i32) / s);
            let ov = images.add(image(root.0.max(1) as u32, root.1.max(1) as u32, vec![0; (root.0.max(1) * root.1.max(1) * 4) as usize], true));
            commands.spawn((CreditsNode(OVERLAY), MaterialNode(layers.add(HudLayer::new(ov.clone()))), full, GlobalZIndex(903)));
            let logos = if mode == Mode::Quotes { (0..10).map(|_| new_logo(root)).collect() } else { Vec::new() };
            *st = Some(Scroll {
                mode,
                y: root.1,
                height,
                chunks: n,
                timer: 0.04,
                speed: 0.04,
                paused: false,
                scale: s,
                scrolling: true,
                end_timer: None,
                egg: String::new(),
                egg_phrase: 0,
                egg_timer: 3.0,
                show_phrases: false,
                phrases: Vec::new(),
                logos,
                root,
                overlay: Some(ov),
                overlay_drawn: false,
            });
            OPEN.store(true, Ordering::Relaxed);
            crate::sound::set_music(Some(if mode == Mode::Credits { "Credits_Music.Credits_Music" } else { "Quotes_Music.Quotes_Music" }));
            if fonts.is_none() {
                *fonts = Some((
                    crate::classic_hud::load_font(assets, "FontLocation").map(Arc::new),
                    crate::classic_hud::load_font(assets, "FontSpinningDX").map(Arc::new),
                ));
            }
            println!("CREDITS {mode:?} {} units, {} chunks", height, n);
        }
    }
    let Some(sc) = st.as_mut() else { return };
    binds.capture = true;
    // StopRendering: the world isn't drawn under the credits (the mirrors stop by themselves).
    for mut c in &mut world_cams {
        if c.is_active {
            c.is_active = false;
        }
    }
    let dt = time.delta_secs().min(0.1);
    let k = &binds.keys;
    let shift_alt = k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight) || k.pressed(KeyCode::AltLeft) || k.pressed(KeyCode::AltRight);
    if !shift_alt {
        if k.just_pressed(KeyCode::ArrowDown) || k.just_pressed(KeyCode::Minus) {
            if sc.speed < 0.2 {
                sc.speed += 0.01;
            }
        }
        if k.just_pressed(KeyCode::ArrowUp) || k.just_pressed(KeyCode::Equal) {
            if sc.speed > 0.01 {
                sc.speed -= 0.01;
            }
        }
        if k.just_pressed(KeyCode::Space) {
            sc.paused = !sc.paused;
        }
    }
    // What ends this window: Escape closes it (root PopWindow); FinishedScrolling (a click, the end of the strip,
    // Escape on the quotes) may hand over to the quotes first.
    let mut close = false;
    let mut finished = binds.buttons.just_released(MouseButton::Left);
    if k.just_pressed(KeyCode::Escape) {
        if sc.mode == Mode::Quotes {
            finished = true;
        } else {
            close = true;
        }
    }
    if sc.mode == Mode::Quotes && !shift_alt {
        if k.just_pressed(KeyCode::Insert) {
            let l = new_logo(sc.root);
            // Lower(): new logos go under the others.
            sc.logos.insert(0, l);
        }
        if k.just_pressed(KeyCode::Delete) && !sc.logos.is_empty() {
            // The top-most logo goes.
            sc.logos.pop();
        }
        if k.just_pressed(KeyCode::End) {
            sc.logos.clear();
        }
    }
    // The easter egg (cheats only).
    let mut dance = false;
    if sc.mode == Mode::Credits && crate::settings::debug_mode() && !(k.pressed(KeyCode::AltLeft) || k.pressed(KeyCode::AltRight)) {
        let typed: Vec<char> = binds.typed.iter().filter_map(|(_, t)| t.as_ref()).flat_map(|t| t.chars()).filter(|c| c.is_ascii_alphanumeric()).collect();
        for ch in typed {
            sc.egg.push(ch.to_ascii_uppercase());
            let found = EASTER.iter().position(|e| *e == sc.egg);
            let partial = found.is_none() && EASTER.iter().any(|e| e.contains(sc.egg.as_str()));
            if let Some(i) = found {
                crate::sound::play_player(EGG_FOUND[rand(3) as usize], 1.0, 1024.0);
                sc.egg_phrase = i;
                sc.show_phrases = true;
                match EASTER[i] {
                    "QUOTES" => {
                        crate::settings::set("QuotesEnabled", 1.0);
                        crate::settings::save();
                    }
                    "DANCEPARTY" => dance = true,
                    "THEREISNOSPOON" => crate::eggs::toggle_matrix(),
                    "IAMWARREN" => crate::eggs::toggle_warren(),
                    _ => {}
                }
                sc.egg.clear();
            } else if partial {
                crate::sound::play_player(GOOD[rand(5) as usize], 1.0, 1024.0);
            } else {
                if sc.egg.len() > 1 {
                    if let Some(s) = BAD[rand(8) as usize] {
                        crate::sound::play_player(s, 1.0, 1024.0);
                    }
                }
                sc.egg.clear();
            }
        }
    }
    if !sc.paused {
        if sc.scrolling {
            sc.timer -= dt;
            let mut diff = 0;
            while sc.timer < 0.0 {
                sc.timer += sc.speed.max(0.005);
                diff += 1;
            }
            sc.y -= diff;
            if sc.y + sc.height < 0 {
                sc.scrolling = false;
                finished = true;
            }
        }
        // CreditsWindow.Tick: after the strip, the CreditsEnd sound's length, then the quotes.
        if let Some(t) = sc.end_timer.as_mut() {
            *t -= dt;
            if *t < 0.0 {
                sc.end_timer = None;
                SHOW_QUOTES.store(true, Ordering::Relaxed);
            }
        }
        if sc.show_phrases {
            sc.egg_timer -= dt;
            for _ in 0..rand(5) {
                let (w, h) = sc.root;
                sc.phrases.push(Phrase {
                    text: FOUND[sc.egg_phrase].to_string(),
                    x: (rand(w) - rand(100)) as f32,
                    y: rand(h) as f32,
                    move_x: 0.0,
                    move_y: 0.0,
                    fade_speed: frand() / 20.0,
                    fade_timer: 0.0,
                    hue: frand(),
                    lum: frand(),
                    col: [0, 0, 0],
                });
                let p = sc.phrases.last_mut().unwrap();
                p.fade_timer = p.fade_speed;
            }
            if sc.egg_timer < 0.0 {
                sc.show_phrases = false;
                sc.egg_timer = 3.0;
            }
        }
        sc.phrases.retain_mut(|p| tick_phrase(p, dt));
        let root = sc.root;
        for l in &mut sc.logos {
            tick_logo(l, dt, root);
        }
    }
    if finished && sc.end_timer.is_none() {
        // FinishedScrolling.
        match sc.mode {
            Mode::Credits if crate::settings::get("QuotesEnabled") > 0.5 => {
                // CreditsWindow: the sting, and the quotes once it has played. (The original only counts down once
                // the strip has scrolled away; a click just played the sting - we count down from the click too.)
                crate::sound::play_player("DeusExSounds.UserInterface.CreditsEnd", 1.0, 1024.0);
                sc.scrolling = false;
                sc.end_timer = Some(4.0);
            }
            _ => close = true,
        }
    }
    // The overlay: fading phrases and spinning logos.
    let busy = !sc.phrases.is_empty() || !sc.logos.is_empty();
    if busy || sc.overlay_drawn {
        if let (Some(h), Some((loc, spin))) = (&sc.overlay, fonts.as_ref()) {
            let (w, hh) = sc.root;
            let mut cv = Canvas::new(w, hh, 1);
            for l in &sc.logos {
                if let Some(f) = spin {
                    let ch = char::from(b'0' + l.frame as u8).to_string();
                    cv.text(f, l.x as i32, l.y as i32, 32, 32, HAlign::Left, VAlign::Top, l.col, &ch);
                }
            }
            for p in &sc.phrases {
                if let Some(f) = loc {
                    let tw = f.width(&p.text);
                    cv.text_styled(f, p.x as i32, p.y as i32, tw, f.height, HAlign::Left, VAlign::Top, p.col, &p.text, Style::Translucent);
                }
            }
            if let Some(mut img) = images.get_mut(h) {
                img.data = Some(cv.to_rgba8());
            }
        }
        sc.overlay_drawn = busy;
    }
    let kk = sc.scale as f32 / win.scale_factor();
    if std::env::var("DXVIEW_CREDITS").is_ok() {
        println!("CREDITS y={} t={:.1} dt={:.3}", sc.y, time.elapsed_secs(), time.delta_secs());
    }
    let screen_h = win.physical_height() as i32 / sc.scale.max(1);
    for (_, cn, mut node, mut vis) in &mut nodes {
        if cn.0 < sc.chunks {
            let top = sc.y + cn.0 as i32 * CHUNK;
            // Only the chunks on screen are drawn.
            let on = top < screen_h && top + CHUNK > 0;
            let want = if on { Visibility::Inherited } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
            if on {
                node.top = Val::Px((top as f32 * kk).round());
            }
        }
    }
    if close || dance {
        for (e, ..) in &nodes {
            commands.entity(e).despawn();
        }
        *st = None;
        OPEN.store(false, Ordering::Relaxed);
        for mut c in &mut world_cams {
            c.is_active = true;
        }
        binds.capture = false;
        crate::sound::set_music(None);
        if dance {
            // ConsoleCommand("OPEN 99_ENDGAME4").
            crate::loader::request("99_Endgame4", crate::loader::Goal::Travel(None));
        } else if LOAD_INTRO.load(Ordering::Relaxed) {
            crate::loader::request("DX", crate::loader::Goal::Title);
        }
    }
}
