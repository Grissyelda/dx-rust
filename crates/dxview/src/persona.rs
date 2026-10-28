//! The other persona screens, reached from the inventory screen's nav bar: Health (PersonaScreenHealth),
//! Augmentations (PersonaScreenAugmentations), Skills (PersonaScreenSkills) and Logs (PersonaScreenLogs).
//! Goals/Notes, Conversations and Images show their frames with what the game has recorded so far.
//!
//! Each screen is drawn on the same 640 x 480 canvas as the inventory screen, from the classes' own defaults
//! (client border and background textures, offsets) and the layout numbers in their CreateControls.

use crate::abilities;
use crate::ai::{PlayerHealth, P_ARM_L, P_ARM_R, P_HEAD, P_LEG_L, P_LEG_R, P_TORSO};
use crate::classic_hud::ClassicHud;
use crate::inventory_screen::{class_p_full, draw_button, inside, int_p, item_borders, label, Fonts, Layout, NAV_LABELS, SCREEN_H, SCREEN_W, WHITE};
use crate::pickups::PlayerInventory;
use crate::uicanvas::{color_scaled, wrap, Canvas, HAlign, Rgb, Style, VAlign};
use crate::weapons::RuntimeAssets;
use unreal_pkg::props::{Property, Value};

pub const SCREEN_INVENTORY: usize = 0;
pub const SCREEN_HEALTH: usize = 1;
pub const SCREEN_AUGS: usize = 2;
pub const SCREEN_SKILLS: usize = 3;
pub const SCREEN_GOALS: usize = 4;
pub const SCREEN_CONVERSATIONS: usize = 5;
pub const SCREEN_IMAGES: usize = 6;
pub const SCREEN_LOGS: usize = 7;
/// HUDRechargeWindow (repair bot): not a persona screen, a small window of its own.
pub const SCREEN_RECHARGE: usize = 100;

/// The medical / repair bot whose screen is up: whether it's a medical bot, seconds until it can serve again
/// (0 = ready), its refresh time and how much it gives (healAmount / chargeAmount).
#[derive(Clone, Debug)]
pub struct BotView {
    pub med: bool,
    pub remaining: f32,
    pub refresh: f32,
    pub amount: f32,
}

impl BotView {
    fn ready(&self) -> bool {
        self.remaining <= 0.0
    }
}

const CLASSES: [&str; 8] = [
    "PersonaScreenInventory",
    "PersonaScreenHealth",
    "PersonaScreenAugmentations",
    "PersonaScreenSkills",
    "PersonaScreenGoals",
    "PersonaScreenConversations",
    "PersonaScreenImages",
    "PersonaScreenLogs",
];

/// Things on a screen that can be pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PB {
    Nav(usize),
    Exit,
    Upgrade,
    Activate,
    UseCell,
    HealAll,
    Heal(usize),
    Part(usize),
    Skill(usize),
    Aug(usize),
    ClearLog,
    ShowCompleted,
    AddNote,
    DeleteNote,
    Note(usize),
    /// "Confirm Note Deletion" checkbox, and the Yes / No of the "Delete Note?" box.
    ConfirmNotes,
    DeleteYes,
    DeleteNo,
    ConRow(usize),
    ConSort(usize),
    ImageRow(usize),
    ShowImageNotes,
    /// A medical bot's canister list: (canister, which of its two augs).
    CanAug(usize, usize),
    Install,
    Recharge,
}

/// What the screen asks the frame to do.
pub enum Act {
    Exit,
    Nav(usize),
    /// Use an item through the normal pickup code (the bio cell).
    UseItem(String),
    /// MedBotHealPlayer / RepairBot.ChargePlayer.
    BotHeal,
    BotCharge,
    /// HUDMedBotAddAugsScreen.InstallAugmentation: canister index, augmentation class.
    Install(usize, String),
}

#[derive(Default)]
pub struct PersonaUi {
    skill_sel: Option<usize>,
    aug_sel: Option<usize>,
    health_sel: Option<usize>,
    pressed: Option<PB>,
    status: String,
    status_t: f32,
    scroll: i32,
    show_completed: bool,
    note_sel: Option<usize>,
    /// The note being typed into (PersonaNotesEditWindow with the focus): cursor and selection.
    edit: Option<NoteEdit>,
    /// A press on a note: (note, mouse) - the cursor goes where it was clicked once the layout is known.
    note_click: Option<(usize, (i32, i32))>,
    last_mouse: (i32, i32),
    /// bConfirmNoteDeletes is on by default; this is its inverse so Default keeps it on.
    no_confirm_notes: bool,
    /// The "Delete Note?" box is up.
    asking_delete: bool,
    con_sel: Option<usize>,
    /// Conversations list sort: column (0 contact, 1 location, 2 type), reversed.
    con_sort: Option<(usize, bool)>,
    image_sel: Option<usize>,
    hide_image_notes: bool,
    /// Images looked at (the "new" marker goes once the screen is left).
    viewed: std::collections::HashSet<String>,
    seen_images: std::collections::HashSet<String>,
    last_screen: usize,
    /// Medical bot: the canister augmentation picked, or the reason it can't be (title, text).
    can_sel: Option<(usize, usize)>,
    can_msg: Option<(String, String)>,
}

/// Editing state of the focused note: cursor and selection anchor as char indices, blink clock, and the column
/// kept while moving up and down.
#[derive(Clone, Copy, Default)]
struct NoteEdit {
    cursor: usize,
    anchor: Option<usize>,
    blink: f32,
    goal_x: Option<i32>,
}

impl PersonaUi {
    /// A note has the keyboard (keys other than Tab / Escape go to it, PersonaEditWindow.VirtualKeyPressed).
    pub fn editing(&self) -> bool {
        self.edit.is_some() || self.asking_delete
    }
    /// The screen closed: the next one opens without a focused note (a fresh PersonaScreenGoals).
    pub fn end_edit(&mut self) {
        self.edit = None;
        self.note_click = None;
        self.asking_delete = false;
    }
    /// A message box is up (Escape answers it instead of closing the screen).
    pub fn modal(&self) -> bool {
        self.asking_delete
    }
    pub(crate) fn set_status(&mut self, s: String) {
        self.status = s;
        self.status_t = 0.0;
    }
}

/// The message log history the Logs screen lists (DeusExPlayer's log, newest last).
static LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Remember a message for the Logs screen (called by the HUD log).
pub fn log_message(s: &str) {
    let mut l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    l.push(s.to_string());
    // DeusExPlayer.maxLogLines-ish bound so it can't grow forever.
    let n = l.len();
    if n > 200 {
        l.drain(0..n - 200);
    }
}

pub struct Ctx<'a> {
    pub hud: &'a mut ClassicHud,
    pub assets: &'a mut RuntimeAssets,
    pub fonts: &'a Fonts,
    pub lay: &'a Layout,
    pub inv: &'a mut PlayerInventory,
    pub ph: &'a mut PlayerHealth,
    pub bot: Option<BotView>,
}

pub struct Input {
    pub mouse: (i32, i32),
    pub down: bool,
    pub up: bool,
    pub wheel: f32,
    pub dt: f32,
    pub up_key: bool,
    pub down_key: bool,
    pub enter: bool,
    /// F3..F12 pressed this frame (augmentation screen hotkeys).
    pub fkey: Option<usize>,
    /// Key presses this frame including repeats, with the text they type (for the note editor).
    pub typed: Vec<(bevy::input::keyboard::KeyCode, Option<String>)>,
    pub ctrl: bool,
    pub shift: bool,
}

fn at<'a>(p: &'a [Property], name: &str, idx: u32) -> Option<&'a Value> {
    p.iter()
        .find(|x| x.index == idx && x.name.eq_ignore_ascii_case(name))
        .map(|x| &x.value)
}

/// A string default of a class (for callers without a Ctx).
pub fn s_default_pub(assets: &mut RuntimeAssets, class: &str, key: &str, d: &str) -> String {
    let p = assets.classes.defaults(class);
    match at(&p, key, 0) {
        Some(Value::Str(v)) if !v.is_empty() => v.clone(),
        _ => d.to_string(),
    }
}

fn s_default(ctx: &mut Ctx, class: &str, key: &str, d: &str) -> String {
    let p = ctx.assets.classes.defaults(class);
    match at(&p, key, 0) {
        Some(Value::Str(v)) if !v.is_empty() => v.clone(),
        _ => d.to_string(),
    }
}

/// The window class drawn for a screen (the medical bot's own versions of Health and Augmentations).
fn screen_class(ctx: &Ctx, screen: usize) -> &'static str {
    match (ctx.bot.as_ref().is_some_and(|b| b.med), screen) {
        (true, SCREEN_HEALTH) => "HUDMedBotHealthScreen",
        (true, SCREEN_AUGS) => "HUDMedBotAddAugsScreen",
        _ => CLASSES[screen.min(CLASSES.len() - 1)],
    }
}

fn med(ctx: &Ctx) -> bool {
    ctx.bot.as_ref().is_some_and(|b| b.med)
}

/// HUDMedBotNavBarWindow: Augmentations, Health (sized to their labels), then Exit.
fn med_nav(ctx: &mut Ctx) -> Vec<(usize, i32, i32, String)> {
    let d = ctx.assets.classes.defaults("HUDMedBotNavBarWindow");
    let s = |k: &str, dflt: &str| match at(&d, k, 0) {
        Some(Value::Str(v)) => v.clone(),
        _ => dflt.to_string(),
    };
    let labels = [
        (SCREEN_AUGS, s("AugsButtonLabel", "   |&Augmentations   ")),
        (SCREEN_HEALTH, s("HealthButtonLabel", " |&Health   ")),
    ];
    let mut x = 23;
    labels
        .into_iter()
        .map(|(sc, l)| {
            let w = (4 + ctx.fonts.headers.width(&label(&l)) + 8).max(20);
            let r = (sc, x, w, l);
            x += w;
            r
        })
        .collect()
}

/// Client origin on the screen (clientBorderOffset + clientOffset) and the client size.
fn client_rect(ctx: &mut Ctx, screen: usize) -> (i32, i32, i32, i32) {
    let d = ctx.assets.classes.defaults(screen_class(ctx, screen));
    let by = int_p(&d, "clientBorderOffsetY", 33);
    (
        int_p(&d, "clientOffsetX", 33),
        by + int_p(&d, "clientOffsetY", 10),
        int_p(&d, "ClientWidth", 585),
        int_p(&d, "ClientHeight", 361),
    )
}

/// PersonaScreenBaseWindow: client border (texture grid of 256 cells at y = clientBorderOffsetY), client background
/// (clipped to the client size), the nav bar with this screen's button insensitive, and the title.
fn draw_base(
    c: &mut Canvas,
    ctx: &mut Ctx,
    screen: usize,
    hover: Option<PB>,
    pressed: Option<PB>,
    title: &str,
) {
    let d = ctx.assets.classes.defaults(screen_class(ctx, screen));
    let by = int_p(&d, "clientBorderOffsetY", 33);
    let (cx, cy, cw, ch) = client_rect(ctx, screen);
    let grid = |key: &str, cols: i32, n: usize, d: &[Property]| -> Vec<(i32, i32, String)> {
        (0..n)
            .filter_map(|i| match at(d, key, i as u32) {
                Some(Value::Path(p)) if !p.ends_with(".None") => {
                    Some(((i as i32 % cols) * 256, (i as i32 / cols) * 256, p.clone()))
                }
                _ => None,
            })
            .collect()
    };
    let bcols = int_p(&d, "clientBorderTextureCols", 3).max(1);
    let ccols = int_p(&d, "clientTextureCols", 3).max(1);
    for (tx, ty, p) in grid("clientBorderTextures", bcols, 6, &d) {
        if let Some(t) = ctx.hud.tex(ctx.assets, &p) {
            c.texture(
                tx,
                by + ty,
                t.w as i32,
                t.h as i32,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(1),
            );
        }
    }
    c.set_clip(Some((cx, cy, cw, ch)));
    for (tx, ty, p) in grid("clientTextures", ccols, 6, &d) {
        if let Some(t) = ctx.hud.tex(ctx.assets, &p) {
            c.texture(
                cx + tx,
                cy + ty,
                t.w as i32,
                t.h as i32,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
    c.set_clip(None);
    // Nav bar.
    for (i, (x, w)) in [(17, 256), (17 + 256, 256), (17 + 512, 97)]
        .iter()
        .enumerate()
    {
        if let Some(t) = ctx
            .hud
            .ui(ctx.assets, &format!("PersonaNavBarBackground_{}", i + 1))
        {
            c.texture(*x, 6, *w, 21, 0, 0, &t, Style::Translucent, crate::menuui::h_col(0));
        }
    }
    for (i, (x, w)) in [(0, 256), (256, 256), (512, 175)].iter().enumerate() {
        if let Some(t) = ctx
            .hud
            .ui(ctx.assets, &format!("PersonaNavBarBorder_{}", i + 1))
        {
            c.texture(*x, 0, *w, 64, 0, 0, &t, Style::Translucent, crate::menuui::h_col(1));
        }
    }
    let state = |b: PB| -> usize {
        if pressed == Some(b) && hover == Some(b) {
            2
        } else if hover == Some(b) {
            1
        } else {
            0
        }
    };
    if med(ctx) {
        for (sc, x, w, l) in med_nav(ctx) {
            let s = if sc == screen { 3 } else { state(PB::Nav(sc)) };
            draw_button(c, ctx.hud, ctx.assets, &ctx.fonts.headers, x, 8, w, &l, s, true);
        }
        // CreateMedbotLabel: "MEDBOT INTERFACE" at the right of the title row.
        let t = s_default(ctx, screen_class(ctx, screen), "MedbotInterfaceText", "MEDBOT INTERFACE");
        c.text(
            &ctx.fonts.headers,
            305,
            9,
            250,
            16,
            HAlign::Right,
            VAlign::Center,
            crate::menuui::h_col(7),
            &t,
        );
    } else {
        let nav = ctx.lay.nav.clone();
        for (i, (x, w)) in nav.iter().enumerate() {
            let s = if i == screen { 3 } else { state(PB::Nav(i)) };
            draw_button(
                c,
                ctx.hud,
                ctx.assets,
                &ctx.fonts.headers,
                *x,
                8,
                *w,
                NAV_LABELS[i],
                s,
                true,
            );
        }
    }
    let exit = ctx.lay.exit;
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &ctx.fonts.headers,
        exit.0,
        8,
        exit.1,
        "E|&xit",
        state(PB::Exit),
        true,
    );
    c.text(
        &ctx.fonts.headers,
        cx + 9,
        cy + 5,
        300,
        ctx.fonts.headers.height,
        HAlign::Left,
        VAlign::Top,
        crate::menuui::h_col(2),
        title,
    );
}

fn nav_hits(lay: &Layout) -> Vec<(PB, (i32, i32, i32, i32), bool)> {
    let mut v: Vec<(PB, (i32, i32, i32, i32), bool)> = lay
        .nav
        .iter()
        .enumerate()
        .map(|(i, (x, w))| (PB::Nav(i), (*x, 8, *w, 16), true))
        .collect();
    v.push((PB::Exit, (lay.exit.0, 8, lay.exit.1, 16), true));
    v
}

/// ProgressBarWindow with a scaled colour (x modifier) over a background.
fn progress(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, pct: f32, modifier: f32, back: Rgb) {
    c.fill(x, y, w, h, Style::Normal, back);
    let col = color_scaled(pct).map(|v| (v as f32 * modifier) as u8);
    c.fill(
        x,
        y,
        (w as f32 * pct.clamp(0.0, 1.0)) as i32,
        h,
        Style::Normal,
        col,
    );
}

/// PersonaInfoWindow: header title, then word-wrapped text in the small font, scrolled.
fn draw_info(
    c: &mut Canvas,
    f: &Fonts,
    r: (i32, i32, i32, i32),
    title: &str,
    text: &str,
    scroll: i32,
) -> i32 {
    let (x0, y0, w, h) = r;
    c.text(
        &f.headers,
        x0 + 2,
        y0 + 1,
        w,
        f.headers.height,
        HAlign::Left,
        VAlign::Center,
        crate::menuui::h_col(7),
        title,
    );
    c.set_clip(Some((x0, y0 + 20, w, h - 20)));
    let lines = wrap(&f.small, text, w - 8);
    for (i, l) in lines.iter().enumerate() {
        c.text(
            &f.small,
            x0 + 4,
            y0 + 20 + i as i32 * f.small.height - scroll,
            w - 8,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            l,
        );
    }
    c.set_clip(None);
    (lines.len() as i32 * f.small.height - (h - 20)).max(0)
}

/// PersonaItemDetailWindow: a large icon with "Count: n" under it, and text beside.
#[allow(clippy::too_many_arguments)]
fn draw_detail(
    c: &mut Canvas,
    ctx: &mut Ctx,
    x: i32,
    y: i32,
    w: i32,
    class: &str,
    text: &str,
    count: i32,
) {
    let d = ctx.assets.classes.defaults(class);
    let (iw, ih) = (
        int_p(&d, "largeIconWidth", 42),
        int_p(&d, "largeIconHeight", 37),
    );
    if let Some(t) = class_p_full(&d, "largeIcon").and_then(|p| ctx.hud.tex(ctx.assets, &p)) {
        c.texture(
            55 / 2 - iw / 2 + x,
            55 / 2 - ih / 2 + y,
            iw,
            ih,
            0,
            0,
            &t,
            Style::Masked,
            WHITE,
        );
    }
    let f = ctx.fonts;
    let cs = format!("Count: {count}");
    c.text(
        &f.small_ds,
        x,
        y + 55 - f.small_ds.height,
        55,
        f.small_ds.height,
        HAlign::Center,
        VAlign::Top,
        crate::menuui::h_col(7),
        &cs,
    );
    let tw = w - 55 - 3 - 4;
    for (i, l) in wrap(&f.small, text, tw).iter().enumerate() {
        c.text(
            &f.small,
            x + 58,
            y + 2 + i as i32 * f.small.height,
            tw,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            l,
        );
    }
}

fn count_of(inv: &PlayerInventory, class: &str) -> i32 {
    inv.0
        .items
        .iter()
        .filter(|i| i.def.class.eq_ignore_ascii_case(class))
        .map(|i| i.count as i32)
        .sum()
}

/// Take one of `class` out of the inventory (UseOnce).
fn use_once(inv: &mut PlayerInventory, class: &str) {
    if let Some(i) = inv
        .0
        .items
        .iter()
        .position(|i| i.def.class.eq_ignore_ascii_case(class))
    {
        if inv.0.items[i].count > 1 {
            inv.0.items[i].count -= 1;
        } else {
            inv.0.remove(i);
        }
    }
}

/// One frame of a persona screen: input, then drawing.
pub fn page(
    screen: usize,
    ui: &mut PersonaUi,
    ctx: &mut Ctx,
    inp: &Input,
    s: i32,
) -> (Canvas, Option<Act>) {
    if ui.last_screen == SCREEN_IMAGES && screen != SCREEN_IMAGES {
        left_screen(ui);
    }
    ui.last_screen = screen;
    ui.last_mouse = inp.mouse;
    if let Some(ed) = ui.edit.as_mut() {
        ed.blink += inp.dt;
    }
    if screen != SCREEN_GOALS {
        ui.edit = None;
        ui.asking_delete = false;
    }
    ui.status_t += inp.dt;
    if ui.status_t > 5.0 {
        ui.status.clear();
    }
    if screen == SCREEN_RECHARGE {
        return recharge_page(ui, ctx, inp, s);
    }
    let mut hits = if med(ctx) {
        let mut v: Vec<(PB, (i32, i32, i32, i32), bool)> = med_nav(ctx)
            .into_iter()
            .map(|(sc, x, w, _)| (PB::Nav(sc), (x, 8, w, 16), true))
            .collect();
        v.push((PB::Exit, (ctx.lay.exit.0, 8, ctx.lay.exit.1, 16), true));
        v
    } else {
        nav_hits(ctx.lay)
    };
    let (cx, cy, _, _) = client_rect(ctx, screen);
    // Screen-specific widgets.
    match screen {
        SCREEN_SKILLS => skills_hits(ctx, ui, cx, cy, &mut hits),
        SCREEN_AUGS if med(ctx) => med_augs_hits(ctx, ui, cx, cy, &mut hits),
        SCREEN_AUGS => augs_hits(ctx, ui, cx, cy, &mut hits),
        SCREEN_HEALTH => health_hits(ctx, cx, cy, &mut hits),
        SCREEN_LOGS => hits.push((PB::ClearLog, (cx + 10, cy + 384, 90, 16), true)),
        SCREEN_GOALS => {
            goals_hits(ui, ctx.fonts, cx, cy, &mut hits);
            if ui.asking_delete {
                // The MenuUIMessageBoxWindow (menus.rs) has the input; only its answer matters here.
                hits.clear();
                if let Some(yes) = crate::menus::take_box_answer() {
                    screen_action(screen, ui, ctx, if yes { PB::DeleteYes } else { PB::DeleteNo });
                }
            }
        }
        SCREEN_CONVERSATIONS => cons_hits(ui, cx, cy, &mut hits),
        SCREEN_IMAGES => images_hits(ui, cx, cy, &mut hits),
        _ => {}
    }
    if let Some(m) = crate::snap::page_map() {
        crate::snap::offer(crate::snap::LAYER_PERSONA, m, hits.iter().filter(|h| h.2).map(|h| h.1));
    }
    let hover = hits
        .iter()
        .rev()
        .find(|h| inside(inp.mouse, h.1))
        .map(|h| h.0);
    let enabled = |b: PB| hits.iter().find(|h| h.0 == b).is_some_and(|h| h.2);
    let mut act = None;
    let mut pressed_now: Option<PB> = None;
    if inp.down {
        if let Some(b) = hover {
            ui.pressed = Some(b);
            // Selection buttons act on press (PersonaItemButton / skill buttons select at once).
            if matches!(b, PB::Skill(_) | PB::Aug(_) | PB::Part(_) | PB::CanAug(..) | PB::Note(_)) {
                pressed_now = Some(b);
            }
        }
    }
    if inp.up {
        if let Some(b) = ui.pressed.take() {
            if hover == Some(b)
                && !matches!(b, PB::Skill(_) | PB::Aug(_) | PB::Part(_) | PB::CanAug(..) | PB::Note(_))
                && enabled(b)
            {
                pressed_now = Some(b);
            }
        }
    }
    if let Some(b) = pressed_now {
        if !matches!(b, PB::Note(_)) {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
        }
        match b {
            PB::Exit => act = Some(Act::Exit),
            PB::Nav(i) if i != screen => act = Some(Act::Nav(i)),
            _ => act = screen_action(screen, ui, ctx, b),
        }
    }
    if inp.wheel != 0.0 {
        ui.scroll -= (inp.wheel * ctx.fonts.small.height as f32) as i32;
    }
    // Keys.
    match screen {
        SCREEN_GOALS => {
            if ui.asking_delete {
                // Keys go to the message box.
            } else if ui.edit.is_some() {
                edit_note_keys(ui, inp, ctx.fonts);
            }
        }
        SCREEN_SKILLS => {
            let n = abilities::lock()
                .as_ref()
                .map_or(0, |a| a.skills.list.len());
            if n > 0 {
                let cur = ui.skill_sel.unwrap_or(n - 1);
                if inp.up_key {
                    ui.skill_sel = Some((cur + n - 1) % n);
                    ui.scroll = 0;
                }
                if inp.down_key {
                    ui.skill_sel = Some((cur + 1) % n);
                    ui.scroll = 0;
                }
            }
        }
        SCREEN_AUGS if !med(ctx) => {
            if let Some(k) = inp.fkey {
                // SelectAugByKey: select the aug on that key and toggle it.
                let i = abilities::lock()
                    .as_ref()
                    .and_then(|a| a.augs.list.iter().position(|x| x.has && x.key() == Some(k)));
                if let Some(i) = i {
                    ui.aug_sel = Some(i);
                    act = act.or(screen_action(screen, ui, ctx, PB::Activate));
                }
            }
            if inp.enter {
                act = act.or(screen_action(screen, ui, ctx, PB::Activate));
            }
        }
        _ => {}
    }

    // Draw.
    let mut c = Canvas::new(SCREEN_W, SCREEN_H, s);
    let title = match screen {
        SCREEN_HEALTH => s_default(ctx, CLASSES[screen], "HealthTitleText", "Health"),
        SCREEN_AUGS => s_default(
            ctx,
            CLASSES[screen],
            "AugmentationsTitleText",
            "Augmentations",
        ),
        SCREEN_SKILLS => s_default(ctx, CLASSES[screen], "SkillsTitleText", "Skills"),
        SCREEN_GOALS => s_default(ctx, CLASSES[screen], "GoalsTitleText", "Goals"),
        SCREEN_CONVERSATIONS => s_default(
            ctx,
            CLASSES[screen],
            "ConversationsTitleText",
            "Conversations",
        ),
        SCREEN_IMAGES => s_default(ctx, CLASSES[screen], "ImagesTitleText", "Images"),
        _ => s_default(ctx, CLASSES[screen], "LogsTitleText", "Logs"),
    };
    draw_base(&mut c, ctx, screen, hover, ui.pressed, &title);
    let pressed_b = ui.pressed;
    let state = |b: PB| -> usize {
        if !enabled(b) {
            3
        } else if pressed_b == Some(b) && hover == Some(b) {
            2
        } else if hover == Some(b) {
            1
        } else {
            0
        }
    };
    let max_scroll = match screen {
        SCREEN_SKILLS => draw_skills(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_AUGS => draw_augs(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_HEALTH => draw_health(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_LOGS => draw_logs(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_GOALS => draw_goals(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_CONVERSATIONS => draw_cons(&mut c, ctx, ui, cx, cy, &state),
        SCREEN_IMAGES => draw_images(&mut c, ctx, ui, cx, cy, &state),
        _ => 0,
    };
    ui.scroll = ui.scroll.clamp(0, max_scroll.max(0));
    (c, act)
}

fn screen_action(screen: usize, ui: &mut PersonaUi, ctx: &mut Ctx, b: PB) -> Option<Act> {
    if med(ctx) {
        match (screen, b) {
            (SCREEN_HEALTH, PB::HealAll) => return Some(Act::BotHeal),
            (SCREEN_AUGS, PB::CanAug(i, k)) => {
                select_can_aug(ui, ctx, i, k);
                return None;
            }
            (SCREEN_AUGS, PB::Aug(i)) => {
                // An installed aug's button just shows its info.
                ui.aug_sel = Some(i);
                ui.can_sel = None;
                ui.can_msg = None;
                ui.scroll = 0;
                return None;
            }
            (SCREEN_AUGS, PB::Install) => {
                let (i, k) = ui.can_sel.take()?;
                let class = crate::augcan::list(ctx.inv).get(i)?.get(k)?.clone();
                ui.can_msg = None;
                return Some(Act::Install(i, class));
            }
            _ => {}
        }
    }
    match (screen, b) {
        (SCREEN_SKILLS, PB::Skill(i)) => {
            if ui.skill_sel != Some(i) {
                ui.scroll = 0;
            }
            ui.skill_sel = Some(i);
        }
        (SCREEN_SKILLS, PB::Upgrade) => {
            let n = abilities::lock()
                .as_ref()
                .map_or(0, |a| a.skills.list.len());
            let i = ui.skill_sel.unwrap_or(n.saturating_sub(1));
            let mut g = abilities::lock();
            if let Some(a) = g.as_mut() {
                if let Some(sk) = a.skills.list.get(i).map(|s| s.def.class.clone()) {
                    if a.skills.upgrade(&sk) {
                        let name = a.skills.list[i].def.name.clone();
                        drop(g);
                        let lab = s_default(
                            ctx,
                            "PersonaScreenSkills",
                            "SkillUpgradedLevelLabel",
                            "%s upgraded",
                        );
                        ui.set_status(lab.replacen("%s", &name, 1));
                    }
                }
            }
        }
        (SCREEN_AUGS, PB::Aug(i)) => {
            if ui.aug_sel != Some(i) {
                ui.scroll = 0;
            }
            ui.aug_sel = Some(i);
        }
        (SCREEN_AUGS, PB::Activate) => {
            let i = ui.aug_sel?;
            let energy = ctx.ph.energy;
            let mut g = abilities::lock();
            let a = g.as_mut()?;
            let t = if a.augs.list.get(i)?.active {
                a.augs.deactivate(i)
            } else {
                a.augs.activate(i, energy)
            };
            let aug = a.augs.list[i].clone();
            drop(g);
            match t {
                imsim_core::augs::Toggle::Activated(_)
                | imsim_core::augs::Toggle::Deactivated(_) => {
                    let on = aug.active;
                    let snd = if on {
                        &aug.def.activate_sound
                    } else {
                        &aug.def.deactivate_sound
                    };
                    crate::sound::play_player(snd, 1.0, 0.0);
                }
                _ => {}
            }
        }
        (SCREEN_AUGS, PB::Upgrade) => {
            let i = ui.aug_sel?;
            if count_of(ctx.inv, "AugmentationUpgradeCannister") > 0 {
                let mut g = abilities::lock();
                let a = g.as_mut()?;
                if a.augs.upgrade(i, true) {
                    drop(g);
                    use_once(ctx.inv, "AugmentationUpgradeCannister");
                }
            }
        }
        (SCREEN_AUGS, PB::UseCell) => return Some(Act::UseItem("BioelectricCell".into())),
        (SCREEN_HEALTH, PB::Part(i)) => ui.health_sel = Some(i),
        (SCREEN_HEALTH, PB::Heal(i)) => {
            let healed = heal_part(ctx, i, None, false);
            let lab = s_default(
                ctx,
                "PersonaScreenHealth",
                "PointsHealedLabel",
                "%d points healed",
            );
            ui.set_status(lab.replacen("%d", &healed.to_string(), 1));
        }
        (SCREEN_HEALTH, PB::HealAll) => {
            let healed = heal_all(ctx);
            let lab = s_default(
                ctx,
                "PersonaScreenHealth",
                "PointsHealedLabel",
                "%d points healed",
            );
            ui.set_status(lab.replacen("%d", &healed.to_string(), 1));
        }
        (SCREEN_LOGS, PB::ClearLog) => LOG.lock().unwrap_or_else(|e| e.into_inner()).clear(),
        (_, PB::ShowCompleted) => ui.show_completed = !ui.show_completed,
        (_, PB::Note(i)) => {
            // FocusEnteredDescendant: this note takes the keyboard; the cursor goes where it was clicked.
            ui.note_sel = Some(i);
            ui.edit = Some(NoteEdit::default());
            ui.note_click = Some((i, ui.last_mouse));
        }
        (_, PB::AddNote) => {
            // AddNote: player.AddNote(defaultNoteText, True) (the newest note goes first), its edit window gets
            // the focus with the whole default text selected, so typing replaces it.
            let t = s_default(ctx, "PersonaScreenGoals", "defaultNoteText", "New Note");
            let n = t.chars().count();
            if let Some(st) = crate::consys::lock().as_mut() {
                st.notes.insert(0, t);
            }
            ui.note_sel = Some(0);
            ui.edit = Some(NoteEdit {
                cursor: n,
                anchor: Some(0),
                ..Default::default()
            });
            ui.note_click = None;
        }
        (_, PB::DeleteNote) => {
            if ui.note_sel.is_some() {
                if ui.no_confirm_notes {
                    delete_selected_note(ui);
                } else {
                    // root.MessageBox(DeleteNoteTitle, DeleteNotePrompt, MB_YesNo)
                    ui.asking_delete = true;
                    let title = s_default(ctx, "PersonaScreenGoals", "DeleteNoteTitle", "Delete Note?");
                    let prompt = s_default(ctx, "PersonaScreenGoals", "DeleteNotePrompt", "Are you sure you wish to delete this note?");
                    let _ = crate::menus::take_box_answer();
                    crate::menus::open_yes_no(&title, &prompt);
                }
            }
        }
        (_, PB::DeleteYes) => {
            ui.asking_delete = false;
            delete_selected_note(ui);
        }
        (_, PB::DeleteNo) => ui.asking_delete = false,
        (_, PB::ConfirmNotes) => ui.no_confirm_notes = !ui.no_confirm_notes,
        (_, PB::ConRow(i)) => {
            ui.con_sel = Some(i);
            ui.scroll = 0;
        }
        (_, PB::ConSort(c)) => {
            ui.con_sort = match ui.con_sort {
                Some((k, r)) if k == c => Some((c, !r)),
                _ => Some((c, false)),
            };
            ui.con_sel = None;
        }
        (_, PB::ImageRow(i)) => {
            ui.image_sel = Some(i);
            if let Some(cl) = images().get(i) {
                ui.viewed.insert(cl.clone());
            }
        }
        (_, PB::ShowImageNotes) => ui.hide_image_notes = !ui.hide_image_notes,
        _ => {}
    }
    None
}

// ------------------------------------------------------------------------------------------------ skills

const SKILL_ROW_H: i32 = 27;

fn skills_hits(
    ctx: &mut Ctx,
    ui: &PersonaUi,
    cx: i32,
    cy: i32,
    hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>,
) {
    let g = abilities::lock();
    let Some(a) = g.as_ref() else { return };
    let n = a.skills.list.len();
    for i in 0..n {
        hits.push((
            PB::Skill(i),
            (cx + 12, cy + 39 + i as i32 * SKILL_ROW_H, 302, SKILL_ROW_H),
            true,
        ));
    }
    let sel = ui.skill_sel.unwrap_or(n.saturating_sub(1));
    let can = a
        .skills
        .list
        .get(sel)
        .is_some_and(|s| a.skills.can_upgrade(&s.def.class));
    drop(g);
    // The Upgrade bar (PersonaButtonBarWindow at (10, 338), 149 wide, not filling).
    let w = (4 + 6 + ctx.fonts.headers.width(&label("|&Upgrade")) + 8).max(20);
    hits.push((PB::Upgrade, (cx + 10, cy + 338, w, 16), can));
}

#[allow(clippy::too_many_arguments)]
fn draw_skills(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let not_avail = s_default(ctx, "PersonaSkillButtonWindow", "NotAvailableLabel", "N/A");
    let level_hdr = s_default(
        ctx,
        "PersonaScreenSkills",
        "SkillLevelHeaderText",
        "Skill Level",
    );
    let points_hdr = s_default(
        ctx,
        "PersonaScreenSkills",
        "PointsNeededHeaderText",
        "Points Needed",
    );
    let sp_hdr = s_default(
        ctx,
        "PersonaScreenSkills",
        "SkillPointsHeaderText",
        "Skill Points",
    );
    let up_label = s_default(
        ctx,
        "PersonaScreenSkills",
        "UpgradeButtonLabel",
        "|&Upgrade",
    );
    c.text(
        &f.small,
        cx + 177,
        cy + 24,
        80,
        f.small.height,
        HAlign::Left,
        VAlign::Top,
        crate::menuui::h_col(8),
        &level_hdr,
    );
    c.text(
        &f.small,
        cx + 247,
        cy + 24,
        80,
        f.small.height,
        HAlign::Left,
        VAlign::Top,
        crate::menuui::h_col(8),
        &points_hdr,
    );
    let snapshot: Vec<(String, String, String, usize, Option<i32>)>;
    let (avail, can_up, info) = {
        let g = abilities::lock();
        let Some(a) = g.as_ref() else { return 0 };
        snapshot = a
            .skills
            .list
            .iter()
            .map(|s| {
                (
                    s.def.icon.clone(),
                    s.def.name.clone(),
                    imsim_core::skills::LEVEL_NAMES[s.level].to_string(),
                    s.level,
                    s.def.cost.get(s.level).copied(),
                )
            })
            .collect();
        // CreateSkillsList selects skillButton, the LAST button made (a quirk kept from the original).
        let sel = ui
            .skill_sel
            .unwrap_or(a.skills.list.len().saturating_sub(1));
        ui.skill_sel = Some(sel);
        let sk = a.skills.list.get(sel);
        (
            a.skills.points_avail,
            sk.is_some_and(|s| a.skills.can_upgrade(&s.def.class)),
            sk.map(|s| (s.def.name.clone(), s.def.description.clone()))
                .unwrap_or_default(),
        )
    };
    let sel = ui.skill_sel.unwrap_or(0);
    let tex = |ctx: &mut Ctx, n: &str| ctx.hud.ui(ctx.assets, n);
    for (i, (icon, name, level, lv, cost)) in snapshot.iter().enumerate() {
        let (x, y) = (cx + 12, cy + 39 + i as i32 * SKILL_ROW_H);
        let selected = i == sel;
        // PersonaBorderButtonWindow: Left (4) / Center / Right (8) at 27 high; focus textures when selected.
        let set = if selected { "Focus" } else { "Normal" };
        if let Some(t) = tex(ctx, &format!("PersonaSkillsButton{set}_Left")) {
            c.texture(
                x,
                y,
                4,
                SKILL_ROW_H,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(3),
            );
        }
        if let Some(t) = tex(ctx, &format!("PersonaSkillsButton{set}_Center")) {
            c.texture(
                x + 4,
                y,
                302 - 12,
                SKILL_ROW_H,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(3),
            );
        }
        if let Some(t) = tex(ctx, &format!("PersonaSkillsButton{set}_Right")) {
            c.texture(
                x + 302 - 8,
                y,
                8,
                SKILL_ROW_H,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(3),
            );
        }
        if let Some(t) = ctx.hud.tex(ctx.assets, icon) {
            c.texture(x + 1, y + 1, 24, 24, 0, 0, &t, Style::Masked, WHITE);
        }
        let col = if selected {
            crate::menuui::h_btn(1)
        } else {
            crate::menuui::h_btn(0)
        };
        c.text(
            &f.headers,
            x + 28,
            y,
            138,
            27,
            HAlign::Left,
            VAlign::Center,
            col,
            name,
        );
        c.text(
            &f.small,
            x + 165,
            y,
            54,
            27,
            HAlign::Left,
            VAlign::Center,
            col,
            level,
        );
        // PersonaLevelIconWindow: one 5 x 5 chicklet per level reached (0..=level), 1 apart.
        if let Some(t) = tex(ctx, "PersonaSkillsChicklet") {
            for k in 0..=*lv as i32 {
                c.texture(x + 229 + k * 6, y + 11, 5, 5, 0, 0, &t, Style::Masked, col);
            }
        }
        let pts = if *lv == 3 {
            not_avail.clone()
        } else {
            cost.map_or(String::new(), |v| v.to_string())
        };
        c.text(
            &f.small,
            x + 264,
            y,
            30,
            27,
            HAlign::Right,
            VAlign::Center,
            col,
            &pts,
        );
    }
    // Upgrade button and its bar filler.
    let w = (4 + 6 + f.headers.width(&label(&up_label)) + 8).max(20);
    let st = if can_up { state(PB::Upgrade) } else { 3 };
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 10,
        cy + 338,
        w,
        &up_label,
        st,
        false,
    );
    if let Some(t) = ctx.hud.ui(ctx.assets, "PersonaButtonFiller") {
        c.texture(
            cx + 10 + w,
            cy + 338,
            149 - w,
            16,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(0),
        );
    }
    c.text(
        &f.headers,
        cx + 180,
        cy + 341,
        70,
        15,
        HAlign::Left,
        VAlign::Center,
        crate::menuui::h_col(7),
        &sp_hdr,
    );
    c.text(
        &f.headers,
        cx + 250,
        cy + 341,
        54,
        15,
        HAlign::Right,
        VAlign::Center,
        crate::menuui::h_col(7),
        &avail.to_string(),
    );
    let max = draw_info(
        c,
        f,
        (cx + 356, cy + 22, 238, 299),
        &info.0,
        &info.1,
        ui.scroll,
    );
    if !ui.status.is_empty() {
        c.text(
            &f.small,
            cx + 356 + 2,
            cy + 329 + 1,
            238,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(9),
            &ui.status,
        );
    }
    max
}

// ------------------------------------------------------------------------------------------------ augmentations

const AUG_BUTTON: (i32, i32) = (54, 59);

/// Where each installed aug's button goes (CreateAugmentationButtons): its location's spot, torso and subdermal
/// stacking down, default ones spreading right.
fn aug_buttons(ctx: &mut Ctx) -> Vec<(usize, i32, i32)> {
    let d = ctx.assets.classes.defaults("PersonaScreenAugmentations");
    let loc = |i: u32| -> (i32, i32) {
        match at(&d, "AugLocs", i) {
            Some(Value::Struct(_, raw)) if raw.len() >= 8 => (
                i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
                i32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
            ),
            _ => (0, 0),
        }
    };
    let sx = int_p(&d, "augSlotSpacingX", 53);
    let sy = int_p(&d, "augSlotSpacingY", 59);
    let g = abilities::lock();
    let Some(a) = g.as_ref() else {
        return Vec::new();
    };
    let (mut torso, mut skin, mut def) = (0, 0, 0);
    let mut out = Vec::new();
    for (i, aug) in a.augs.list.iter().enumerate() {
        if aug.def.name.is_empty() || !aug.has {
            continue;
        }
        let l = aug.def.location as u32;
        let (mut x, mut y) = loc(l);
        match l {
            2 => {
                y += torso * sy;
                torso += 1;
            }
            5 => {
                y += skin * sy;
                skin += 1;
            }
            6 => {
                x += def * sx;
                def += 1;
            }
            _ => {}
        }
        out.push((i, x, y));
    }
    out
}

fn augs_hits(
    ctx: &mut Ctx,
    ui: &PersonaUi,
    cx: i32,
    cy: i32,
    hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>,
) {
    for (i, x, y) in aug_buttons(ctx) {
        hits.push((
            PB::Aug(i),
            (cx + x, cy + y, AUG_BUTTON.0, AUG_BUTTON.1),
            true,
        ));
    }
    let f = ctx.fonts;
    let (up, act) = aug_bar(f);
    let (can_up, can_act) = {
        let g = abilities::lock();
        let sel = ui
            .aug_sel
            .and_then(|i| g.as_ref().and_then(|a| a.augs.list.get(i).cloned()));
        let cans = count_of(ctx.inv, "AugmentationUpgradeCannister") > 0;
        (
            sel.as_ref()
                .is_some_and(|a| cans && a.level < a.def.max_level),
            sel.as_ref()
                .is_some_and(|a| ctx.ph.energy > 0.0 && !a.def.always_active),
        )
    };
    hits.push((PB::Upgrade, (cx + 13, cy + 407, up, 16), can_up));
    hits.push((PB::Activate, (cx + 13 + up, cy + 407, act, 16), can_act));
    let cell_w = (4 + 6 + f.headers.width(&label("Us|&e Cell")) + 8).max(20);
    hits.push((
        PB::UseCell,
        (cx + 346, cy + 387, cell_w, 16),
        ctx.ph.energy < 100.0 && count_of(ctx.inv, "BioelectricCell") > 0,
    ));
}

fn aug_bar(f: &Fonts) -> (i32, i32) {
    let w = |l: &str| (4 + 6 + f.headers.width(&label(l)) + 8).max(20);
    (w("|&Upgrade"), w("Deac|&tivate").max(w("Acti|&vate")))
}

#[allow(clippy::too_many_arguments)]
fn draw_augs(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let d = ctx.assets.classes.defaults("PersonaScreenAugmentations");
    let path = |k: &str, i: u32| match at(&d, k, i) {
        Some(Value::Path(p)) => Some(p.clone()),
        _ => None,
    };
    // Body (masked), then the overlays (translucent background colour), lowered beneath everything else.
    for (k, (y, h)) in [(0, 256), (256, 103)].iter().enumerate() {
        if let Some(t) = ctx
            .hud
            .ui(ctx.assets, &format!("AugmentationsBody_{}", k + 1))
        {
            c.texture(
                cx + 72,
                cy + 28 + y,
                174,
                *h,
                0,
                0,
                &t,
                Style::Masked,
                WHITE,
            );
        }
    }
    for (k, (y, h)) in [(0, 256), (256, 305 - 256)].iter().enumerate() {
        if let Some(t) = ctx
            .hud
            .ui(ctx.assets, &format!("AugmentationsOverlays_{}", k + 1))
        {
            c.texture(
                cx + 72,
                cy + 28 + y,
                174,
                *h,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
    // Location labels.
    for (x, y, key, dflt) in [
        (57, 27, "AugLocationCranial", "Cranial"),
        (212, 27, "AugLocationEyes", "Eyes"),
        (19, 103, "AugLocationArms", "Arms"),
        (19, 187, "AugLocationSubdermal", "Subdermal"),
        (247, 109, "AugLocationTorso", "Torso"),
        (19, 330, "AugLocationDefault", "Default"),
        (247, 311, "AugLocationLegs", "Legs"),
    ] {
        let t = s_default(ctx, "PersonaScreenAugmentations", key, dflt);
        c.text(
            &f.small,
            cx + x + 2,
            cy + y + 1,
            52,
            11,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            &t,
        );
    }
    let buttons = aug_buttons(ctx);
    let augs: Vec<imsim_core::augs::Aug> = abilities::lock()
        .as_ref()
        .map(|a| a.augs.list.clone())
        .unwrap_or_default();
    // Highlights for the occupied locations (not Default).
    let rects = [
        (142, 45, 16, 19),
        (161, 63, 19, 12),
        (157, 108, 34, 48),
        (105, 110, 24, 43),
        (165, 222, 32, 94),
        (84, 160, 14, 36),
    ];
    for (k, r) in rects.iter().enumerate() {
        if buttons
            .iter()
            .any(|(i, _, _)| augs[*i].def.location as usize == k)
        {
            if let Some(t) =
                path("augHighlightTextures", k as u32).and_then(|p| ctx.hud.tex(ctx.assets, &p))
            {
                c.texture(cx + r.0, cy + r.1, r.2, r.3, 0, 0, &t, Style::Masked, WHITE);
            }
        }
    }
    let borders = item_borders(ctx.hud, ctx.assets);
    let chicklet = ctx.hud.ui(ctx.assets, "PersonaSkillsChicklet");
    for (i, x, y) in &buttons {
        let a = &augs[*i];
        let (bx, by) = (cx + x, cy + y);
        // PersonaItemButton: icon (translucent) green while active, yellow otherwise; selection border.
        if let Some(t) = ctx.hud.tex(ctx.assets, &a.def.icon) {
            let col = if a.active { [0, 255, 0] } else { [255, 255, 0] };
            c.texture(
                bx + 27 - 26,
                by + 27 - 26,
                52,
                52,
                0,
                0,
                &t,
                Style::Translucent,
                col,
            );
        }
        if ui.aug_sel == Some(*i) {
            c.borders(bx, by, 54, 54, &borders, Style::Masked, WHITE);
        }
        if !a.def.always_active && a.hotkey >= 3 {
            c.text(
                &f.small_ds,
                bx + 2,
                by + 52 - 9,
                50,
                10,
                HAlign::Left,
                VAlign::Top,
                crate::menuui::h_col(7),
                &format!("F{}", a.hotkey),
            );
        }
        if let Some(t) = &chicklet {
            for k in 0..=a.level as i32 {
                c.texture(
                    bx + 30 + k * 6,
                    by + 54,
                    5,
                    5,
                    0,
                    0,
                    t,
                    Style::Masked,
                    crate::menuui::h_btn(1),
                );
            }
        }
    }
    if med(ctx) {
        return draw_med_augs(c, ctx, ui, cx, cy, state, &augs);
    }
    // Buttons.
    let (up_w, act_w) = aug_bar(f);
    let sel = ui.aug_sel.and_then(|i| augs.get(i));
    let act_label = if sel.is_some_and(|a| a.active) {
        "Deac|&tivate"
    } else {
        "Acti|&vate"
    };
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 13,
        cy + 407,
        up_w,
        "|&Upgrade",
        state(PB::Upgrade),
        false,
    );
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 13 + up_w,
        cy + 407,
        act_w,
        act_label,
        state(PB::Activate),
        false,
    );
    if let Some(t) = ctx.hud.ui(ctx.assets, "PersonaButtonFiller") {
        let used = up_w + act_w;
        if used < 187 {
            c.texture(
                cx + 13 + used,
                cy + 407,
                187 - used,
                16,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
    let cell_w = (4 + 6 + f.headers.width(&label("Us|&e Cell")) + 8).max(20);
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 346,
        cy + 387,
        cell_w,
        "Us|&e Cell",
        state(PB::UseCell),
        false,
    );
    // Bioelectric energy bar.
    let pct = (ctx.ph.energy / 100.0).clamp(0.0, 1.0);
    progress(c, cx + 446, cy + 389, 140, 12, pct, 0.5, [0, 0, 0]);
    c.text(
        &f.small_ds,
        cx + 446,
        cy + 391,
        140,
        12,
        HAlign::Center,
        VAlign::Center,
        WHITE,
        &format!("{}%", (pct * 100.0) as i32),
    );
    // Detail windows.
    let can_text = s_default(ctx, "PersonaScreenAugmentations", "AugCanUseText", "");
    let cell_text = s_default(ctx, "PersonaScreenAugmentations", "BioCellUseText", "");
    let cans = count_of(ctx.inv, "AugmentationUpgradeCannister");
    let cells = count_of(ctx.inv, "BioelectricCell");
    draw_detail(
        c,
        ctx,
        cx + 346,
        cy + 274,
        242,
        "AugmentationUpgradeCannister",
        &can_text,
        cans,
    );
    draw_detail(
        c,
        ctx,
        cx + 346,
        cy + 332,
        242,
        "BioelectricCell",
        &cell_text,
        cells,
    );
    // Info: Augmentation.UpdateInfo.
    let (title, text) = match sel {
        Some(a) => {
            let mut t = a.def.description.clone();
            t.push_str(&format!(
                "\n\nEnergy Rate: {} Units/Minute",
                a.def.energy_rate as i32
            ));
            let mut lvl = format!("Current Level: {}", a.level + 1);
            if a.level < a.def.max_level && cans > 0 {
                lvl.push_str(" (Can Upgrade)");
            } else if a.level == a.def.max_level {
                lvl.push_str(" (Maximum)");
            }
            t.push_str(&format!("\n\n{lvl}"));
            if a.def.always_active {
                t.push_str("\n\n[Always Active]");
            }
            (a.def.name.clone(), t)
        }
        None => (String::new(), String::new()),
    };
    let max = draw_info(
        c,
        f,
        (cx + 348, cy + 14, 238, 218),
        &title,
        &text,
        ui.scroll,
    );
    if !ui.status.is_empty() {
        c.text(
            &f.small,
            cx + 350,
            cy + 241,
            238,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(9),
            &ui.status,
        );
    }
    max
}

// ------------------------------------------------------------------------------------------------ health

/// Region order on the screen (head, torso, right arm, left arm, right leg, left leg) -> PlayerHealth parts.
const PARTS: [usize; 6] = [P_HEAD, P_TORSO, P_ARM_R, P_ARM_L, P_LEG_R, P_LEG_L];
const PART_RECTS: [(i32, i32, i32, i32); 6] = [
    (141, 42, 40, 54),
    (133, 98, 56, 73),
    (90, 131, 28, 65),
    (204, 131, 28, 65),
    (119, 234, 41, 94),
    (162, 234, 41, 94),
];
const REGIONS: [(i32, i32); 6] = [
    (218, 29),
    (27, 43),
    (19, 237),
    (230, 237),
    (24, 347),
    (222, 347),
];
const PART_TITLES: [(&str, &str); 6] = [
    ("HealthLocationHead", "Head"),
    ("HealthLocationTorso", "Torso"),
    ("HealthLocationRightArm", "Right Arm"),
    ("HealthLocationLeftArm", "Left Arm"),
    ("HealthLocationRightLeg", "Right Leg"),
    ("HealthLocationLeftLeg", "Left Leg"),
];

fn health_hits(ctx: &mut Ctx, cx: i32, cy: i32, hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>) {
    let kits = count_of(ctx.inv, "MedKit") > 0;
    for (i, r) in PART_RECTS.iter().enumerate() {
        hits.push((PB::Part(i), (cx + r.0, cy + r.1, r.2, r.3), true));
    }
    for (i, (x, y)) in REGIONS.iter().enumerate() {
        // The region's own button (98 x 25) selects the part; its Heal button heals it.
        hits.push((PB::Part(i), (cx + x, cy + y, 98, 25), true));
        let damaged = ctx.ph.parts[PARTS[i]] < 100.0;
        hits.push((
            PB::Heal(i),
            (cx + x + 5, cy + y + 26, 66, 13),
            damaged && kits,
        ));
    }
    let damaged = PARTS.iter().any(|p| ctx.ph.parts[*p] < 100.0);
    if let Some(b) = ctx.bot.as_ref().filter(|b| b.med) {
        // HUDMedBotHealthScreen: Heal All (97 wide) works the bot, when it's ready and you're hurt.
        hits.push((PB::HealAll, (cx + 346, cy + 346, 97, 16), b.ready() && damaged));
        return;
    }
    let w = (4 + 6 + ctx.fonts.headers.width(&label("H|&eal All")) + 8).max(20);
    hits.push((PB::HealAll, (cx + 13, cy + 407, w, 16), damaged && kits));
}

fn medkit_points(_ctx: &Ctx) -> f32 {
    (30.0 * abilities::skill("SkillMedicine")).floor()
}

/// HealPart: one medkit's points (x Medicine) into one region, up to 100. Returns the points added.
fn heal_part(ctx: &mut Ctx, region: usize, points: Option<f32>, leave_kit: bool) -> i32 {
    if count_of(ctx.inv, "MedKit") == 0 {
        return 0;
    }
    let pts = points.unwrap_or_else(|| medkit_points(ctx));
    let p = PARTS[region];
    let before = ctx.ph.parts[p].max(0.0);
    let after = (before + pts).min(100.0);
    ctx.ph.parts[p] = after;
    ctx.ph.recompute();
    if !leave_kit {
        use_once(ctx.inv, "MedKit");
        crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
    }
    (after - before) as i32
}

/// HealAllParts: all the medkits' points handed out one at a time round the damaged parts; the kits used are
/// taken afterwards.
fn heal_all(ctx: &mut Ctx) -> i32 {
    let kits = count_of(ctx.inv, "MedKit");
    let per = medkit_points(ctx);
    let avail = (kits as f32 * per) as i32;
    let mut left = avail;
    let mut healed = 0;
    while left > 0 {
        let mut any = false;
        for r in 0..6 {
            if left > 0 && ctx.ph.parts[PARTS[r]] < 100.0 {
                healed += heal_part(ctx, r, Some(1.0), true);
                left -= 1;
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    // RemoveMedKits: a kit per medkit's worth of points used.
    let mut used = avail - left;
    while used > 0 && count_of(ctx.inv, "MedKit") > 0 {
        used -= per as i32;
        use_once(ctx.inv, "MedKit");
    }
    if healed > 0 {
        crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
    }
    healed
}

#[allow(clippy::too_many_arguments)]
fn draw_health(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    for (k, (y, h)) in [(0, 256), (256, 101)].iter().enumerate() {
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HealthBody_{}", k + 1)) {
            c.texture(
                cx + 24,
                cy + 36 + y,
                219,
                *h,
                0,
                0,
                &t,
                Style::Masked,
                WHITE,
            );
        }
    }
    for (k, (y, h)) in [(0, 256), (256, 357 - 256)].iter().enumerate() {
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HealthOverlays_{}", k + 1)) {
            c.texture(
                cx + 24,
                cy + 36 + y,
                219,
                *h,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
    let borders = item_borders(ctx.hud, ctx.assets);
    if let Some(sel) = ui.health_sel {
        let r = PART_RECTS[sel];
        c.borders(cx + r.0, cy + r.1, r.2, r.3, &borders, Style::Masked, WHITE);
    }
    let heal_label = s_default(
        ctx,
        "PersonaHealthActionButtonWindow",
        "HealButtonLabel",
        "Heal",
    );
    for (i, (x, y)) in REGIONS.iter().enumerate() {
        let (x, y) = (cx + x, cy + y);
        let title = s_default(
            ctx,
            "PersonaScreenHealth",
            PART_TITLES[i].0,
            PART_TITLES[i].1,
        );
        c.text(
            &f.small,
            x + 3,
            y + 2,
            60,
            11,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            &title,
        );
        let hp = ctx.ph.parts[PARTS[i]].max(0.0);
        progress(c, x + 4, y + 13, 66, 11, hp / 100.0, 0.5, [0, 0, 0]);
        c.text(
            &f.small_ds,
            x + 4,
            y + 14,
            66,
            11,
            HAlign::Center,
            VAlign::Center,
            WHITE,
            &format!("{} / 100", hp as i32),
        );
        // PersonaHealthActionButtonWindow (HealthButton textures, 66 x 13).
        let st = state(PB::Heal(i));
        let set = if st == 2 { "Pressed" } else { "Normal" };
        let (bx, by) = (x + 5, y + 26);
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HealthButton{set}_Left")) {
            c.texture(bx, by, 4, 13, 0, 0, &t, Style::Translucent, crate::menuui::h_col(3));
        }
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HealthButton{set}_Center")) {
            c.texture(
                bx + 4,
                by,
                66 - 12,
                13,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(3),
            );
        }
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HealthButton{set}_Right")) {
            c.texture(
                bx + 66 - 8,
                by,
                8,
                13,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(3),
            );
        }
        let off = if st == 2 { 1 } else { 0 };
        c.text(
            &f.headers,
            bx + 6 + off,
            by + off,
            60,
            13,
            HAlign::Left,
            VAlign::Center,
            crate::menuui::h_btn(st),
            &label(&heal_label),
        );
    }
    if med(ctx) {
        return draw_med_health(c, ctx, ui, cx, cy, state);
    }
    let w = (4 + 6 + f.headers.width(&label("H|&eal All")) + 8).max(20);
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 13,
        cy + 407,
        w,
        "H|&eal All",
        state(PB::HealAll),
        false,
    );
    if let Some(t) = ctx.hud.ui(ctx.assets, "PersonaButtonFiller") {
        if w < 92 {
            c.texture(
                cx + 13 + w,
                cy + 407,
                92 - w,
                16,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
    let kit_text = s_default(ctx, "PersonaScreenHealth", "MedKitUseText", "");
    let kits = count_of(ctx.inv, "MedKit");
    draw_detail(c, ctx, cx + 346, cy + 307, 242, "MedKit", &kit_text, kits);
    let (title, text) = match ui.health_sel {
        Some(i) => {
            let desc_idx = [0, 1, 2, 2, 3, 3][i];
            let d = ctx.assets.classes.defaults("PersonaScreenHealth");
            let desc = match at(&d, "HealthPartDesc", desc_idx) {
                Some(Value::Str(v)) => v.clone(),
                _ => String::new(),
            };
            (
                s_default(
                    ctx,
                    "PersonaScreenHealth",
                    PART_TITLES[i].0,
                    PART_TITLES[i].1,
                ),
                desc,
            )
        }
        None => (String::new(), String::new()),
    };
    let max = draw_info(
        c,
        f,
        (cx + 348, cy + 22, 238, 239),
        &title,
        &text,
        ui.scroll,
    );
    if !ui.status.is_empty() {
        c.text(
            &f.small,
            cx + 350,
            cy + 270,
            238,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(9),
            &ui.status,
        );
    }
    max
}

// ------------------------------------------------------------------------------------------------ medical bot

const CAN_LIST: (i32, i32, i32, i32) = (348, 34, 238, 116);
const CAN_ROW: i32 = 39;

/// The two augmentation buttons of each canister row (HUDMedBotAugCanWindow at 155 / 190, 34 x 34).
fn can_rows(inv: &PlayerInventory) -> Vec<[String; 2]> {
    crate::augcan::list(inv)
}

fn med_augs_hits(
    ctx: &mut Ctx,
    ui: &PersonaUi,
    cx: i32,
    cy: i32,
    hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>,
) {
    for (i, x, y) in aug_buttons(ctx) {
        hits.push((PB::Aug(i), (cx + x, cy + y, AUG_BUTTON.0, AUG_BUTTON.1), true));
    }
    let (lx, ly, _, lh) = CAN_LIST;
    for (i, can) in can_rows(ctx.inv).iter().enumerate() {
        let ry = ly + i as i32 * CAN_ROW - ui.scroll.min(0);
        if ry + 38 > ly + lh + 38 {
            break;
        }
        for k in 0..2 {
            if !can[k].is_empty() {
                hits.push((PB::CanAug(i, k), (cx + lx + 155 + 35 * k as i32, cy + ry + 2, 34, 34), true));
            }
        }
    }
    let w = 96;
    hits.push((PB::Install, (cx + 346, cy + 371, w, 16), ui.can_sel.is_some()));
}

/// HUDMedBotAddAugsScreen.SelectAugmentation: an aug already had, or whose location is full, only explains why.
fn select_can_aug(ui: &mut PersonaUi, ctx: &mut Ctx, i: usize, k: usize) {
    let Some(class) = can_rows(ctx.inv).get(i).and_then(|c| c.get(k).cloned()) else { return };
    let (name, has, full) = {
        let g = abilities::lock();
        let Some(a) = g.as_ref() else { return };
        let Some(idx) = a.augs.index(&class) else { return };
        (a.augs.list[idx].def.name.clone(), a.augs.list[idx].has, a.augs.slots_full(idx))
    };
    let cls = "HUDMedBotAddAugsScreen";
    let another = s_default(ctx, cls, "SelectAnotherText", "Please select another augmentation to install.");
    ui.aug_sel = None;
    ui.scroll = 0;
    if has {
        let t = s_default(ctx, cls, "AlreadyHasItText", "You already have this augmentation.");
        ui.can_msg = Some((name, format!("{t}\n\n{another}")));
        ui.can_sel = None;
    } else if full {
        let t = s_default(ctx, cls, "SlotFullText", "The slot that this augmentation occupies is already full.");
        ui.can_msg = Some((name, format!("{t}\n\n{another}")));
        ui.can_sel = None;
    } else {
        ui.can_msg = None;
        ui.can_sel = Some((i, k));
    }
}

/// Augmentation.UpdateInfo for an aug definition (UsingMedBot: no level / upgrade lines for one not installed).
fn aug_info(a: &imsim_core::augs::Aug, installed: bool) -> (String, String) {
    let mut t = a.def.description.clone();
    t.push_str(&format!("\n\nEnergy Rate: {} Units/Minute", a.def.energy_rate as i32));
    if installed {
        t.push_str(&format!("\n\nCurrent Level: {}", a.level + 1));
    }
    if a.def.always_active {
        t.push_str("\n\n[Always Active]");
    }
    (a.def.name.clone(), t)
}

#[allow(clippy::too_many_arguments)]
fn draw_med_augs(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
    augs: &[imsim_core::augs::Aug],
) -> i32 {
    let f = ctx.fonts;
    let cls = "HUDMedBotAddAugsScreen";
    // "Available Augmentations" header and the canister list.
    let head = s_default(ctx, cls, "AvailableAugsText", "Available Augmentations");
    c.text(&f.headers, cx + 349, cy + 15, 238, f.headers.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(7), &head);
    let (lx, ly, lw, lh) = CAN_LIST;
    let cans = can_rows(ctx.inv);
    let hl = item_borders(ctx.hud, ctx.assets);
    let border_col = [crate::menuui::h_col(0)[0] / 2, crate::menuui::h_col(0)[1] / 2, crate::menuui::h_col(0)[2] / 2];
    let item_col = [
        (crate::menuui::h_col(0)[0] as f32 * 0.75) as u8,
        (crate::menuui::h_col(0)[1] as f32 * 0.75) as u8,
        (crate::menuui::h_col(0)[2] as f32 * 0.75) as u8,
    ];
    let contains = {
        let d = ctx.assets.classes.defaults("HUDMedBotAugCanWindow");
        match at(&d, "AugContainsText", 0) {
            Some(Value::Str(v)) => v.replace("|n", ""),
            _ => "Contains:".into(),
        }
    };
    let can_icon = ctx.hud.ui(ctx.assets, "BeltIconAugmentationCannister").or_else(|| {
        ctx.hud.tex(ctx.assets, "DeusExUI.Icons.BeltIconAugmentationCannister")
    });
    c.set_clip(Some((cx + lx, cy + ly, lw, lh)));
    if cans.is_empty() {
        let t = s_default(ctx, cls, "NoCansAvailableText", "No Augmentation Cannisters Available!");
        c.text(&f.small, cx + lx + 4, cy + ly + 4, lw - 8, 30, HAlign::Left, VAlign::Center, crate::menuui::h_col(8), &t);
    }
    for (i, can) in cans.iter().enumerate() {
        let (rx, ry) = (cx + lx, cy + ly + i as i32 * CAN_ROW);
        c.borders(rx, ry, 226, 38, &hl, Style::Translucent, border_col);
        if let Some(t) = &can_icon {
            c.texture(rx - 7 + 7, ry + 1, 42 - 7, 37, 7, 0, t, Style::Masked, WHITE);
        }
        let name = |class: &str| {
            augs.iter().find(|a| a.def.class.eq_ignore_ascii_case(class)).map(|a| a.def.name.clone()).unwrap_or_default()
        };
        let lines = [contains.clone(), format!(" {}", name(&can[0])), format!(" {}", name(&can[1]))];
        for (k, l) in lines.iter().enumerate() {
            c.text(&f.small, rx + 29, ry + 2 + k as i32 * f.small.height, 126, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), l);
        }
        for k in 0..2 {
            let Some(a) = augs.iter().find(|a| a.def.class.eq_ignore_ascii_case(&can[k])) else { continue };
            let (bx, by) = (rx + 155 + 35 * k as i32, ry + 2);
            let full = {
                let g = abilities::lock();
                g.as_ref().and_then(|ab| ab.augs.index(&a.def.class).map(|i| ab.augs.slots_full(i))).unwrap_or(false)
            };
            let col = if a.has || full { [64, 64, 64] } else { [255, 255, 0] };
            if let Some(t) = ctx.hud.tex(ctx.assets, &a.def.small_icon) {
                c.texture(bx + 1, by + 1, 32, 32, 0, 0, &t, Style::Translucent, col);
            }
            if ui.can_sel == Some((i, k)) {
                c.borders(bx, by, 34, 34, &hl, Style::Masked, WHITE);
            } else {
                c.borders(bx, by, 34, 34, &hl, Style::Masked, item_col);
            }
        }
    }
    c.set_clip(None);
    // Install.
    let install = s_default(ctx, cls, "InstallButtonLabel", "|&Install");
    draw_button(c, ctx.hud, ctx.assets, &f.headers, cx + 346, cy + 371, 96, &install, state(PB::Install), false);
    // Info (348, 158, 238 x 210).
    let (title, text) = if let Some(m) = &ui.can_msg {
        m.clone()
    } else if let Some((i, k)) = ui.can_sel {
        cans.get(i)
            .and_then(|cn| augs.iter().find(|a| a.def.class.eq_ignore_ascii_case(&cn[k])))
            .map(|a| aug_info(a, false))
            .unwrap_or_default()
    } else if let Some(a) = ui.aug_sel.and_then(|i| augs.get(i)) {
        aug_info(a, true)
    } else {
        (String::new(), String::new())
    };
    let max = draw_info(c, f, (cx + 348, cy + 158, 238, 210), &title, &text, ui.scroll);
    if !ui.status.is_empty() {
        c.text(&f.small, cx + 15, cy + 410, 291, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(9), &ui.status);
    }
    max
}

/// "Recharging: %d second(s)" / "Ready!" and the bar fraction, for a bot.
fn bot_bar(ctx: &mut Ctx, cls: &str) -> (f32, String) {
    let Some(b) = ctx.bot.clone() else { return (0.0, String::new()) };
    if b.ready() {
        (1.0, s_default(ctx, cls, "ReadyLabel", "Ready!"))
    } else {
        let secs = b.remaining.ceil() as i32;
        let key = if secs == 1 { "SecondsSingularLabel" } else { "SecondsPluralLabel" };
        let t = s_default(ctx, cls, key, "Recharging: %d seconds").replacen("%d", &secs.to_string(), 1);
        ((1.0 - b.remaining / b.refresh.max(1.0)).clamp(0.0, 1.0), t)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_med_health(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let cls = "HUDMedBotHealthScreen";
    let heal = s_default(ctx, cls, "HealAllButtonLabel", "  H|&eal All  ");
    draw_button(c, ctx.hud, ctx.assets, &f.headers, cx + 346, cy + 346, 97, &heal, state(PB::HealAll), false);
    // The bot's charge (446, 348, 140 x 12) and what it says (348, 293, 238 x 50).
    let (pct, bar_text) = bot_bar(ctx, cls);
    progress(c, cx + 446, cy + 348, 140, 12, pct, 0.5, [0, 0, 0]);
    c.text(&f.small_ds, cx + 446, cy + 349, 140, 12, HAlign::Center, VAlign::Center, WHITE, &bar_text);
    let amount = ctx.bot.as_ref().map_or(300.0, |b| b.amount) as i32;
    let ready = ctx.bot.as_ref().is_some_and(|b| b.ready());
    let damaged = PARTS.iter().any(|p| ctx.ph.parts[*p] < 100.0);
    let mut info = s_default(ctx, cls, "HealthInfoTextLabel", "The MedBot will heal up to %d units.")
        .replacen("%d", &amount.to_string(), 1);
    info.push_str(&if !damaged {
        s_default(ctx, cls, "MedBotYouAreHealed", "|nYou are fully healed.")
    } else if ready {
        s_default(ctx, cls, "MedBotReadyLabel", "|nThe MedBot is Ready, you may now be Healed.")
    } else {
        s_default(ctx, cls, "MedBotRechargingLabel", "|nThe MedBot is currently Recharging.  Please Wait.")
    });
    for (k, l) in wrap(&f.small, &info, 238 - 4).iter().enumerate() {
        c.text(&f.small, cx + 350, cy + 293 + k as i32 * f.small.height, 234, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), l);
    }
    let (title, text) = match ui.health_sel {
        Some(i) => {
            let desc_idx = [0, 1, 2, 2, 3, 3][i];
            let d = ctx.assets.classes.defaults("PersonaScreenHealth");
            let desc = match at(&d, "HealthPartDesc", desc_idx) {
                Some(Value::Str(v)) => v.clone(),
                _ => String::new(),
            };
            (s_default(ctx, "PersonaScreenHealth", PART_TITLES[i].0, PART_TITLES[i].1), desc)
        }
        None => (String::new(), String::new()),
    };
    let max = draw_info(c, f, (cx + 348, cy + 22, 238, 243), &title, &text, ui.scroll);
    if !ui.status.is_empty() {
        c.text(&f.small, cx + 15, cy + 410, 291, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(9), &ui.status);
    }
    max
}

/// HUDRechargeWindow (265 x 153, centred): title, info, bio energy and the bot's charge, Close and Recharge.
fn recharge_page(ui: &mut PersonaUi, ctx: &mut Ctx, inp: &Input, s: i32) -> (Canvas, Option<Act>) {
    let cls = "HUDRechargeWindow";
    let f = ctx.fonts;
    let (wx, wy) = ((SCREEN_W - 265) / 2, (SCREEN_H - 153) / 2);
    let close = s_default(ctx, cls, "CloseButtonLabel", "  |&Close  ");
    let recharge = s_default(ctx, cls, "RechargeButtonLabel", "  |&Recharge  ");
    let bw = |l: &str| (4 + 6 + f.headers.width(&label(l)) + 8).max(20);
    let (cw, rw) = (bw(&close), bw(&recharge));
    let full = ctx.ph.energy >= 100.0;
    let ready = ctx.bot.as_ref().is_some_and(|b| b.ready());
    let hits = [
        (PB::Exit, (wx + 15, wy + 126, cw, 16), true),
        (PB::Recharge, (wx + 15 + cw, wy + 126, rw, 16), ready && !full),
    ];
    if let Some(m) = crate::snap::page_map() {
        crate::snap::offer(crate::snap::LAYER_PERSONA, m, hits.iter().filter(|h| h.2).map(|h| h.1));
    }
    let hover = hits.iter().find(|h| inside(inp.mouse, h.1)).map(|h| h.0);
    let enabled = |b: PB| hits.iter().find(|h| h.0 == b).is_some_and(|h| h.2);
    let mut act = None;
    if inp.down {
        ui.pressed = hover;
    }
    if inp.up {
        if let Some(b) = ui.pressed.take() {
            if hover == Some(b) && enabled(b) {
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
                act = Some(if b == PB::Exit { Act::Exit } else { Act::BotCharge });
            }
        }
    }
    if inp.enter && ready && !full {
        act = Some(Act::BotCharge);
    }
    let mut c = Canvas::new(SCREEN_W, SCREEN_H, s);
    for (k, (x, w)) in [(0, 256), (256, 9)].iter().enumerate() {
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HUDRepairbotBackground_{}", k + 1)) {
            c.texture(wx + x, wy, *w, 153, 0, 0, &t, Style::Translucent, crate::menuui::h_col(0));
        }
        if let Some(t) = ctx.hud.ui(ctx.assets, &format!("HUDRepairbotBorder_{}", k + 1)) {
            c.texture(wx + x, wy, *w, 153, 0, 0, &t, Style::Translucent, crate::menuui::h_col(1));
        }
    }
    let title = s_default(ctx, cls, "RechargeTitle", "REPAIRBOT INTERFACE");
    c.text(&f.headers, wx + 20, wy + 20, 233, 14, HAlign::Left, VAlign::Center, crate::menuui::h_col(7), &title);
    let (amount, refresh) = ctx.bot.as_ref().map_or((75.0, 60.0), |b| (b.amount, b.refresh));
    let mut info = s_default(ctx, cls, "RepairBotInfoText", "The RepairBot can restore up to %d points of Bio Electric Energy every %d seconds.")
        .replacen("%d", &(amount as i32).to_string(), 1)
        .replacen("%d", &(refresh as i32).to_string(), 1);
    info.push_str(&if full {
        s_default(ctx, cls, "RepairBotYouAreHealed", "|n|nYour Bio Electric Energy is full.")
    } else if ready {
        s_default(ctx, cls, "RepairBotReadyLabel", "|n|nThe RepairBot is Ready, you may now Recharge.")
    } else {
        s_default(ctx, cls, "RepairBotRechargingLabel", "|n|nThe RepairBot is currently Recharging.  Please Wait.")
    });
    let lines = wrap(&f.small, &info, 233);
    let top = wy + 39 + ((44 - lines.len() as i32 * f.small.height) / 2).max(0);
    for (k, l) in lines.iter().enumerate() {
        c.text(&f.small, wx + 20, top + k as i32 * f.small.height, 233, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), l);
    }
    // Bio energy and the bot's charge.
    let bio = s_default(ctx, cls, "BioStatusLabel", "Bio Energy:");
    c.text(&f.small, wx + 20, wy + 92, 90, 12, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), &bio);
    let e = (ctx.ph.energy / 100.0).clamp(0.0, 1.0);
    progress(&mut c, wx + 114, wy + 91, 140, 12, e, 0.5, [0, 0, 0]);
    c.text(&f.small_ds, wx + 114, wy + 93, 140, 12, HAlign::Center, VAlign::Center, WHITE, &format!("{}%", (e * 100.0) as i32));
    let st = s_default(ctx, cls, "RepairBotStatusLabel", "RepairBot Status:");
    c.text(&f.small, wx + 20, wy + 112, 90, 12, HAlign::Left, VAlign::Top, crate::menuui::h_col(8), &st);
    let (pct, bar) = bot_bar(ctx, cls);
    progress(&mut c, wx + 114, wy + 111, 140, 12, pct, 0.5, [0, 0, 0]);
    c.text(&f.small_ds, wx + 114, wy + 113, 140, 12, HAlign::Center, VAlign::Center, WHITE, &bar);
    let state = |b: PB| -> usize {
        if !enabled(b) {
            3
        } else if ui.pressed == Some(b) && hover == Some(b) {
            2
        } else if hover == Some(b) {
            1
        } else {
            0
        }
    };
    draw_button(&mut c, ctx.hud, ctx.assets, &f.headers, wx + 15, wy + 126, cw, &close, state(PB::Exit), false);
    draw_button(&mut c, ctx.hud, ctx.assets, &f.headers, wx + 15 + cw, wy + 126, rw, &recharge, state(PB::Recharge), false);
    (c, act)
}

// ------------------------------------------------------------------------------------------------ logs

fn draw_logs(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    // The log text window (a scrolling text area across the client).
    let (x, y, w, h) = (cx + 14, cy + 30, 400, 346);
    let lines: Vec<String> = {
        let l = LOG.lock().unwrap_or_else(|e| e.into_inner());
        l.iter().flat_map(|m| wrap(&f.small, m, w - 8)).collect()
    };
    c.set_clip(Some((x, y, w, h)));
    for (i, l) in lines.iter().enumerate() {
        c.text(
            &f.small,
            x + 4,
            y + i as i32 * f.small.height - ui.scroll,
            w - 8,
            f.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(8),
            l,
        );
    }
    c.set_clip(None);
    let lab = s_default(ctx, "PersonaScreenLogs", "ClearButtonLabel", "Cl|&ear Log");
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 10,
        cy + 384,
        90,
        &lab,
        state(PB::ClearLog),
        false,
    );
    (lines.len() as i32 * f.small.height - h).max(0)
}

// ------------------------------------------------------------------------------------------------ goals, notes

/// PersonaCheckboxWindow: the 12 x 12 box, 6 spacing, the label in FontMenuHeaders; right-aligned `right` from the
/// client's right edge. Returns its rectangle.
fn checkbox_rect(
    f: &Fonts,
    cx: i32,
    cw: i32,
    right: i32,
    y: i32,
    text: &str,
) -> (i32, i32, i32, i32) {
    let w = 12 + 6 + f.headers.width(&label(text));
    (cx + cw - right - w, y, w, 14)
}

#[allow(clippy::too_many_arguments)]
fn draw_checkbox(
    c: &mut Canvas,
    ctx: &mut Ctx,
    r: (i32, i32, i32, i32),
    text: &str,
    on: bool,
    enabled: bool,
) {
    let tex = if on {
        "PersonaCheckBox_On"
    } else {
        "PersonaCheckBox_Off"
    };
    let col = if enabled {
        crate::menuui::h_btn(0)
    } else {
        crate::menuui::h_btn(3)
    };
    if let Some(t) = ctx.hud.ui(ctx.assets, tex) {
        c.texture(
            r.0,
            r.1 + (r.3 - 12) / 2,
            12,
            12,
            0,
            0,
            &t,
            Style::Masked,
            col,
        );
    }
    c.text(
        &ctx.fonts.headers,
        r.0 + 18,
        r.1,
        r.2 - 18,
        r.3,
        HAlign::Left,
        VAlign::Center,
        col,
        &label(text),
    );
}

/// Goals and notes as the screen lays them out: (goal rows: header?, completed, bullet, lines), note boxes.
fn goal_rows(f: &Fonts, show_completed: bool) -> Vec<(bool, bool, bool, Vec<String>)> {
    let g = crate::consys::lock();
    let goals: Vec<crate::consys::Goal> = g.as_ref().map(|s| s.goals.clone()).unwrap_or_default();
    drop(g);
    let mut rows = Vec::new();
    for (primary, header) in [(true, "Primary Goals"), (false, "Secondary Goals")] {
        rows.push((true, false, false, vec![header.to_string()]));
        let mut found = false;
        for goal in goals.iter().filter(|x| x.primary == primary) {
            if goal.completed && !show_completed {
                continue;
            }
            found = true;
            let text = if goal.completed {
                format!("{} [Completed]", goal.text)
            } else {
                goal.text.clone()
            };
            rows.push((false, goal.completed, true, wrap(&f.small, &text, 574 - 16)));
        }
        if !found {
            rows.push((false, true, false, vec!["None".to_string()]));
        }
    }
    rows
}

fn notes_list() -> Vec<String> {
    crate::consys::lock()
        .as_ref()
        .map(|s| s.notes.clone())
        .unwrap_or_default()
}

const NOTES_AREA: (i32, i32, i32, i32) = (16, 226, 574, 182);

/// Note boxes: (top, height, chars, line spans); 5 px text margins top and bottom, 4 px between boxes.
fn note_boxes(f: &Fonts) -> Vec<(i32, i32, Vec<char>, Vec<(usize, usize)>)> {
    let mut y = 0;
    notes_list()
        .iter()
        .map(|n| {
            let chars: Vec<char> = n.chars().collect();
            let spans = note_spans(&f.small, &chars, NOTES_AREA.2 - 20);
            let h = spans.len() as i32 * f.small.height + 10;
            let r = (y, h, chars, spans);
            y += h + 4;
            r
        })
        .collect()
}

fn goals_hits(
    ui: &PersonaUi,
    f: &Fonts,
    cx: i32,
    cy: i32,
    hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>,
) {
    hits.push((
        PB::ConfirmNotes,
        checkbox_rect(f, cx, 604, 13, cy + 412, CONFIRM_NOTES_LABEL),
        true,
    ));
    // The fonts aren't at hand here; the note rows are hit-tested in draw order below, so register the whole
    // notes area per note using the cached layout from the last draw.
    hits.push((
        PB::ShowCompleted,
        (cx + 604 - 13 - 200, cy + 180, 200, 14),
        true,
    ));
    hits.push((PB::AddNote, (cx + 10, cy + 411, 89, 16), true));
    hits.push((
        PB::DeleteNote,
        (cx + 10 + 90, cy + 411, 89, 16),
        ui.note_sel.is_some(),
    ));
    let boxes = NOTE_HITS.lock().map(|v| v.clone()).unwrap_or_default();
    for (i, (y, h)) in boxes.into_iter().enumerate() {
        let top = (y - ui.scroll).max(0);
        let bottom = (y + h - ui.scroll).min(NOTES_AREA.3);
        if bottom > top {
            hits.push((
                PB::Note(i),
                (
                    cx + NOTES_AREA.0,
                    cy + NOTES_AREA.1 + top,
                    NOTES_AREA.2,
                    bottom - top,
                ),
                true,
            ));
        }
    }
}

static NOTE_HITS: std::sync::Mutex<Vec<(i32, i32)>> = std::sync::Mutex::new(Vec::new());

fn draw_goals(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let notes_title = s_default(ctx, "PersonaScreenGoals", "NotesTitleText", "Notes");
    c.text(
        &f.headers,
        cx + 9,
        cy + 209,
        300,
        f.headers.height,
        HAlign::Left,
        VAlign::Top,
        crate::menuui::h_col(2),
        &notes_title,
    );
    // Goals.
    let (gx, gy, gw, gh) = (cx + 16, cy + 21, 574, 154);
    c.set_clip(Some((gx, gy, gw, gh)));
    let mut y = gy;
    for (header, completed, bullet, lines) in goal_rows(f, ui.show_completed) {
        if header {
            c.text(
                &f.headers,
                gx,
                y,
                gw,
                f.headers.height + 2,
                HAlign::Left,
                VAlign::Center,
                crate::menuui::h_col(7),
                &lines[0],
            );
            y += f.headers.height + 2;
            continue;
        }
        let col = if completed {
            crate::menuui::h_col(8).map(|v| v / 2)
        } else {
            crate::menuui::h_col(8)
        };
        if bullet {
            c.text(
                &f.small,
                gx,
                y,
                12,
                f.small.height,
                HAlign::Left,
                VAlign::Top,
                col,
                "~",
            );
        }
        for l in &lines {
            c.text(
                &f.small,
                gx + 12,
                y,
                gw - 12,
                f.small.height,
                HAlign::Left,
                VAlign::Top,
                col,
                l,
            );
            y += f.small.height;
        }
    }
    c.set_clip(None);
    let lab = s_default(
        ctx,
        "PersonaScreenGoals",
        "DisplayCompletedGoals",
        "Display C|&ompleted Goals",
    );
    let r = checkbox_rect(f, cx, 604, 13, cy + 180, &lab);
    draw_checkbox(c, ctx, r, &lab, ui.show_completed, true);
    // Notes: each in its bracketed box; the selected one in the focus box.
    let boxes = note_boxes(f);
    *NOTE_HITS.lock().unwrap() = boxes.iter().map(|b| (b.0, b.1)).collect();
    let (nx, ny, nw, nh) = (
        cx + NOTES_AREA.0,
        cy + NOTES_AREA.1,
        NOTES_AREA.2,
        NOTES_AREA.3,
    );
    c.set_clip(Some((nx, ny, nw, nh)));
    let normal: [Option<std::sync::Arc<crate::uicanvas::UiTex>>; 9] = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ]
    .map(|n| ctx.hud.ui(ctx.assets, &format!("PersonaNoteNormal_{n}")));
    let focus: [Option<std::sync::Arc<crate::uicanvas::UiTex>>; 9] = [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ]
    .map(|n| ctx.hud.ui(ctx.assets, &format!("PersonaNoteFocus_{n}")));
    // A click on a note: put the cursor at the nearest character.
    if let Some((i, (mx, my))) = ui.note_click.take() {
        if let (Some((y, _, chars, spans)), Some(ed)) = (boxes.get(i), ui.edit.as_mut()) {
            let top = ny + y - ui.scroll + 5;
            let line = (((my - top).max(0)) / f.small.height.max(1)) as usize;
            let line = line.min(spans.len() - 1);
            ed.cursor = index_at_x(&f.small, chars, spans[line], mx - (nx + 10));
            ed.anchor = None;
            ed.goal_x = None;
            ed.blink = 0.0;
        }
    }
    let mut total = 0;
    for (i, (y, h, chars, spans)) in boxes.iter().enumerate() {
        let top = ny + y - ui.scroll;
        let t = if ui.note_sel == Some(i) {
            &focus
        } else {
            &normal
        };
        c.borders(nx, top, nw, *h, t, Style::Masked, crate::menuui::h_col(7));
        let ed = ui.edit.filter(|_| ui.note_sel == Some(i));
        let sel = ed
            .and_then(|e| e.anchor.filter(|a| *a != e.cursor).map(|a| (a.min(e.cursor), a.max(e.cursor))));
        for (k, &(s0, s1)) in spans.iter().enumerate() {
            let ly = top + 5 + k as i32 * f.small.height;
            let line: String = chars[s0..s1].iter().filter(|c| **c != '\n').collect();
            c.text(&f.small, nx + 10, ly, nw - 20, f.small.height, HAlign::Left, VAlign::Top, crate::menuui::h_col(9), &line);
            // Selected area: Solid in the text colour, the text over it in black.
            if let Some((a, b)) = sel {
                let (a, b) = (a.max(s0), b.min(s1));
                if a < b {
                    let x0 = nx + 10 + x_of(&f.small, chars, s0, a);
                    let part: String = chars[a..b].iter().filter(|c| **c != '\n').collect();
                    let pw = f.small.width(&part);
                    c.fill(x0, ly, pw, f.small.height, Style::Normal, crate::menuui::h_col(9));
                    c.text(&f.small, x0, ly, pw + 2, f.small.height, HAlign::Left, VAlign::Top, [0, 0, 0], &part);
                }
            }
        }
        // Insertion point: a thin bar in the cursor colour, blinking.
        if let Some(e) = ed {
            if (e.blink % 1.0) < 0.6 && !ui.asking_delete {
                let line = span_of(spans, e.cursor);
                let x = nx + 10 + x_of(&f.small, chars, spans[line].0, e.cursor);
                let ly = top + 5 + line as i32 * f.small.height;
                c.fill(x, ly, 1, f.small.height, Style::Normal, crate::menuui::h_col(13));
            }
        }
        total = y + h;
    }
    c.set_clip(None);
    let add = s_default(ctx, "PersonaScreenGoals", "AddButtonLabel", "Add |&Note");
    let del = s_default(
        ctx,
        "PersonaScreenGoals",
        "DeleteButtonLabel",
        "|&Delete Note",
    );
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 10,
        cy + 411,
        89,
        &add,
        state(PB::AddNote),
        true,
    );
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 100,
        cy + 411,
        89,
        &del,
        state(PB::DeleteNote),
        true,
    );
    let conf = s_default(
        ctx,
        "PersonaScreenGoals",
        "ConfirmNoteDeletionLabel",
        CONFIRM_NOTES_LABEL,
    );
    let r = checkbox_rect(f, cx, 604, 13, cy + 412, &conf);
    draw_checkbox(c, ctx, r, &conf, !ui.no_confirm_notes, true);
    (total - nh).max(0)
}

const CONFIRM_NOTES_LABEL: &str = "Confirm Note Deletion";

fn delete_selected_note(ui: &mut PersonaUi) {
    ui.edit = None;
    ui.note_click = None;
    if let Some(i) = ui.note_sel.take() {
        if let Some(st) = crate::consys::lock().as_mut() {
            if i < st.notes.len() {
                st.notes.remove(i);
            }
        }
    }
}

/// Lines of a note as char ranges [start, end): '\n' breaks a line, words wrap at spaces (a line keeps its
/// trailing space), a word longer than the line breaks where it must (EditWindow word wrap).
fn note_spans(font: &crate::uicanvas::UiFont, chars: &[char], width: i32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut w = 0;
    let mut last_break: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            out.push((start, i));
            start = i + 1;
            w = 0;
            last_break = None;
            i += 1;
            continue;
        }
        let cw = font.width(&c.to_string());
        if w + cw > width && i > start {
            let at = match last_break {
                Some(b) if b > start => b,
                _ => i,
            };
            out.push((start, at));
            start = at;
            w = chars[start..i].iter().map(|c| font.width(&c.to_string())).sum();
            last_break = None;
            continue;
        }
        w += cw;
        if c == ' ' {
            last_break = Some(i + 1);
        }
        i += 1;
    }
    out.push((start, chars.len()));
    out
}

/// The line holding a cursor position (at a wrap point, the start of the next line).
fn span_of(spans: &[(usize, usize)], cursor: usize) -> usize {
    let mut k = 0;
    for (i, &(s, _)) in spans.iter().enumerate() {
        if s <= cursor {
            k = i;
        }
    }
    k
}

fn x_of(font: &crate::uicanvas::UiFont, chars: &[char], from: usize, to: usize) -> i32 {
    chars[from..to.min(chars.len())]
        .iter()
        .map(|c| font.width(&c.to_string()))
        .sum()
}

/// The char boundary on a line nearest to x.
fn index_at_x(font: &crate::uicanvas::UiFont, chars: &[char], span: (usize, usize), x: i32) -> usize {
    let mut acc = 0;
    // A wrapped line's trailing space belongs before the next line's start.
    let end = if span.1 > span.0 && chars.get(span.1 - 1) == Some(&' ') && span.1 < chars.len() {
        span.1 - 1
    } else {
        span.1
    };
    for i in span.0..end {
        let cw = font.width(&chars[i].to_string());
        if x < acc + cw / 2 {
            return i;
        }
        acc += cw;
    }
    end
}

fn note_text(i: usize) -> Option<String> {
    crate::consys::lock().as_ref().and_then(|s| s.notes.get(i).cloned())
}

fn set_note_text(i: usize, t: String) {
    if let Some(st) = crate::consys::lock().as_mut() {
        if let Some(n) = st.notes.get_mut(i) {
            *n = t;
        }
    }
}

/// PersonaNotesEditWindow (EditWindow, multi-line, insert mode): typing, Backspace / Delete, arrows, Home / End,
/// Enter for a new line, Ctrl+A selects all; a selection is replaced by what's typed. Backslashes are filtered
/// (FilterChar). The text goes straight into the note (SaveNote).
fn edit_note_keys(ui: &mut PersonaUi, inp: &Input, f: &Fonts) {
    use bevy::input::keyboard::KeyCode as K;
    let (Some(i), Some(mut ed)) = (ui.note_sel, ui.edit) else {
        return;
    };
    let Some(text) = note_text(i) else {
        ui.edit = None;
        return;
    };
    let mut chars: Vec<char> = text.chars().collect();
    let mut changed = false;
    let width = NOTES_AREA.2 - 20;
    for (k, t) in &inp.typed {
        ed.blink = 0.0;
        let sel = ed.anchor.filter(|a| *a != ed.cursor).map(|a| (a.min(ed.cursor), a.max(ed.cursor)));
        let mut delete_sel = |chars: &mut Vec<char>, ed: &mut NoteEdit| -> bool {
            if let Some((a, b)) = sel {
                chars.drain(a..b);
                ed.cursor = a;
                ed.anchor = None;
                true
            } else {
                ed.anchor = None;
                false
            }
        };
        match k {
            K::Escape | K::Tab => {}
            K::Backspace => {
                if !delete_sel(&mut chars, &mut ed) && ed.cursor > 0 {
                    ed.cursor -= 1;
                    chars.remove(ed.cursor);
                }
                changed = true;
                ed.goal_x = None;
            }
            K::Delete => {
                if !delete_sel(&mut chars, &mut ed) && ed.cursor < chars.len() {
                    chars.remove(ed.cursor);
                }
                changed = true;
                ed.goal_x = None;
            }
            K::ArrowLeft | K::ArrowRight | K::Home | K::End | K::ArrowUp | K::ArrowDown => {
                let from = ed.cursor;
                let spans = note_spans(&f.small, &chars, width);
                let line = span_of(&spans, ed.cursor);
                let mut vertical = false;
                ed.cursor = match k {
                    K::ArrowLeft => match (sel, inp.shift) {
                        (Some((a, _)), false) => a,
                        _ => ed.cursor.saturating_sub(1),
                    },
                    K::ArrowRight => match (sel, inp.shift) {
                        (Some((_, b)), false) => b,
                        _ => (ed.cursor + 1).min(chars.len()),
                    },
                    K::Home if inp.ctrl => 0,
                    K::End if inp.ctrl => chars.len(),
                    K::Home => spans[line].0,
                    K::End => {
                        let (s, e) = spans[line];
                        if e > s && e < chars.len() && chars[e - 1] == ' ' && line + 1 < spans.len() && spans[line + 1].0 == e {
                            e - 1
                        } else {
                            e
                        }
                    }
                    _ => {
                        vertical = true;
                        let gx = ed.goal_x.unwrap_or_else(|| x_of(&f.small, &chars, spans[line].0, ed.cursor));
                        ed.goal_x = Some(gx);
                        let target = if *k == K::ArrowUp {
                            line.checked_sub(1)
                        } else {
                            (line + 1 < spans.len()).then_some(line + 1)
                        };
                        match target {
                            Some(l) => index_at_x(&f.small, &chars, spans[l], gx),
                            None if *k == K::ArrowUp => 0,
                            None => chars.len(),
                        }
                    }
                };
                if !vertical {
                    ed.goal_x = None;
                }
                if inp.shift {
                    ed.anchor = Some(ed.anchor.unwrap_or(from));
                } else {
                    ed.anchor = None;
                }
            }
            K::KeyA if inp.ctrl => {
                ed.anchor = Some(0);
                ed.cursor = chars.len();
            }
            K::Enter | K::NumpadEnter => {
                delete_sel(&mut chars, &mut ed);
                chars.insert(ed.cursor, '\n');
                ed.cursor += 1;
                changed = true;
                ed.goal_x = None;
            }
            _ => {
                if inp.ctrl {
                    continue;
                }
                let Some(t) = t else { continue };
                let add: Vec<char> = t.chars().filter(|c| !c.is_control() && *c != '\\').collect();
                if add.is_empty() {
                    continue;
                }
                delete_sel(&mut chars, &mut ed);
                for c in add {
                    chars.insert(ed.cursor, c);
                    ed.cursor += 1;
                }
                changed = true;
                ed.goal_x = None;
            }
        }
    }
    if changed {
        set_note_text(i, chars.into_iter().collect());
    }
    ui.edit = Some(ed);
}

// ------------------------------------------------------------------------------------------------ conversations

/// The history in list order (sorted by the chosen column).
fn con_rows(sort: Option<(usize, bool)>) -> Vec<crate::consys::History> {
    let mut v: Vec<crate::consys::History> = crate::consys::lock()
        .as_ref()
        .map(|s| s.history.clone())
        .unwrap_or_default();
    // Newest first until a column is chosen (the list isn't auto-sorted).
    let Some(sort) = sort else { return v };
    match sort.0 {
        0 => v.sort_by(|a, b| {
            a.owner
                .to_ascii_lowercase()
                .cmp(&b.owner.to_ascii_lowercase())
        }),
        1 => v.sort_by(|a, b| {
            a.location
                .to_ascii_lowercase()
                .cmp(&b.location.to_ascii_lowercase())
        }),
        _ => v.sort_by_key(|h| h.infolink),
    }
    if sort.1 {
        v.reverse();
    }
    v
}

const CON_LIST: (i32, i32, i32, i32) = (16, 38, 395, 133);
const CON_TEXT: (i32, i32, i32, i32) = (16, 190, 394, 204);
const CON_ROW_H: i32 = 12;

fn cons_hits(ui: &PersonaUi, cx: i32, cy: i32, hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>) {
    for (i, (x, w)) in [(14, 162), (176, 194), (370, 46)].into_iter().enumerate() {
        hits.push((PB::ConSort(i), (cx + x, cy + 19, w, 16), true));
    }
    let n = con_rows(ui.con_sort).len() as i32;
    for i in 0..n.min(CON_LIST.3 / CON_ROW_H) {
        hits.push((
            PB::ConRow(i as usize),
            (
                cx + CON_LIST.0,
                cy + CON_LIST.1 + i * CON_ROW_H,
                CON_LIST.2,
                CON_ROW_H,
            ),
            true,
        ));
    }
}

fn draw_cons(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let labels = [
        s_default(
            ctx,
            "PersonaScreenConversations",
            "ActorButtonLabel",
            "Contact",
        ),
        s_default(
            ctx,
            "PersonaScreenConversations",
            "LocationButtonLabel",
            "Location",
        ),
        s_default(ctx, "PersonaScreenConversations", "TypeButtonLabel", "Type"),
    ];
    for (i, (x, w)) in [(14, 162), (176, 194), (370, 46)].into_iter().enumerate() {
        draw_button(
            c,
            ctx.hud,
            ctx.assets,
            &f.headers,
            cx + x,
            cy + 19,
            w,
            &labels[i],
            state(PB::ConSort(i)),
            false,
        );
    }
    let rows = con_rows(ui.con_sort);
    let wing = ctx.hud.font(ctx.assets, "FontHUDWingDings");
    let (lx, ly, lw, lh) = (cx + CON_LIST.0, cy + CON_LIST.1, CON_LIST.2, CON_LIST.3);
    c.set_clip(Some((lx, ly, lw, lh)));
    for (i, h) in rows.iter().enumerate() {
        let y = ly + i as i32 * CON_ROW_H;
        let sel = ui.con_sel == Some(i);
        if sel {
            c.fill(lx, y, lw, CON_ROW_H, Style::Normal, [128, 128, 128]);
        }
        let col = if sel { WHITE } else { crate::menuui::h_col(9) };
        c.text(
            &f.small,
            lx + 2,
            y,
            159,
            CON_ROW_H,
            HAlign::Left,
            VAlign::Center,
            col,
            &h.owner,
        );
        c.text(
            &f.small,
            lx + 161,
            y,
            194,
            CON_ROW_H,
            HAlign::Left,
            VAlign::Center,
            col,
            &h.location,
        );
        if let Some(w) = &wing {
            c.text(
                w,
                lx + 355,
                y,
                40,
                CON_ROW_H,
                HAlign::Center,
                VAlign::Center,
                col,
                if h.infolink { "B" } else { "A" },
            );
        }
    }
    c.set_clip(None);
    // The chosen conversation: speaker in the header font, then the line.
    let Some(h) = ui.con_sel.and_then(|i| rows.get(i)) else {
        return 0;
    };
    let (tx, ty, tw, th) = (cx + CON_TEXT.0, cy + CON_TEXT.1, CON_TEXT.2, CON_TEXT.3);
    c.set_clip(Some((tx, ty, tw, th)));
    let mut y = ty - ui.scroll;
    for (k, (who, line)) in h.lines.iter().enumerate() {
        c.text(
            &f.headers,
            tx + 5,
            y + 2,
            tw - 10,
            f.headers.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(7),
            who,
        );
        y += f.headers.height + 4;
        for l in wrap(&f.small, line, tw - 20) {
            c.text(
                &f.small,
                tx + 10,
                y + 2,
                tw - 20,
                f.small.height,
                HAlign::Left,
                VAlign::Top,
                crate::menuui::h_col(8),
                &l,
            );
            y += f.small.height;
        }
        y += 4;
        if k + 1 < h.lines.len() {
            y += 5;
        }
    }
    c.set_clip(None);
    (y + ui.scroll - ty - th).max(0)
}

// ------------------------------------------------------------------------------------------------ images

fn images() -> Vec<String> {
    crate::consys::lock()
        .as_ref()
        .map(|s| s.images.clone())
        .unwrap_or_default()
}

const IMG_LIST: (i32, i32, i32, i32) = (417, 21, 184, 398);

fn images_hits(
    _ui: &PersonaUi,
    cx: i32,
    cy: i32,
    hits: &mut Vec<(PB, (i32, i32, i32, i32), bool)>,
) {
    let n = images().len() as i32;
    for i in 0..n.min(IMG_LIST.3 / CON_ROW_H) {
        hits.push((
            PB::ImageRow(i as usize),
            (
                cx + IMG_LIST.0,
                cy + IMG_LIST.1 + i * CON_ROW_H,
                IMG_LIST.2,
                CON_ROW_H,
            ),
            true,
        ));
    }
    hits.push((PB::AddNote, (cx + 10, cy + 422, 129, 16), false));
    hits.push((PB::DeleteNote, (cx + 140, cy + 422, 129, 16), false));
    hits.push((
        PB::ShowImageNotes,
        (cx + 617 - 203 - 100, cy + 424, 100, 14),
        true,
    ));
}

fn draw_images(
    c: &mut Canvas,
    ctx: &mut Ctx,
    ui: &mut PersonaUi,
    cx: i32,
    cy: i32,
    state: &dyn Fn(PB) -> usize,
) -> i32 {
    let f = ctx.fonts;
    let list = images();
    let desc = |ctx: &mut Ctx, cl: &str| {
        let d = ctx.assets.classes.defaults(cl);
        match at(&d, "imageDescription", 0) {
            Some(Value::Str(s)) => s.clone(),
            _ => cl.to_string(),
        }
    };
    // PersonaImageWindow (400 x 400 at 15, 20).
    let (ix, iy) = (cx + 15, cy + 20);
    match ui.image_sel.and_then(|i| list.get(i)).cloned() {
        Some(cl) => {
            let d = ctx.assets.classes.defaults(&cl);
            for (k, (x, y, w, h)) in [
                (0, 0, 256, 256),
                (256, 0, 144, 256),
                (0, 256, 256, 144),
                (256, 256, 144, 144),
            ]
            .into_iter()
            .enumerate()
            {
                if let Some(Value::Path(p)) = at(&d, "imageTextures", k as u32) {
                    if let Some(t) = ctx.hud.tex(ctx.assets, p) {
                        c.texture(ix + x, iy + y, w, h, 0, 0, &t, Style::Normal, WHITE);
                    }
                }
            }
            let title = desc(ctx, &cl);
            c.text(
                &f.headers,
                cx + 214,
                cy + 6,
                200,
                f.headers.height,
                HAlign::Right,
                VAlign::Center,
                crate::menuui::h_col(7),
                &title,
            );
        }
        None => {
            let s = s_default(
                ctx,
                "PersonaImageWindow",
                "strNoImages",
                "No Images Available",
            );
            c.text(
                &f.headers,
                ix,
                iy,
                400,
                400,
                HAlign::Center,
                VAlign::Center,
                WHITE,
                &s,
            );
        }
    }
    // The list: description, and the "new" mark until the screen is left after viewing it.
    let wing = ctx.hud.font(ctx.assets, "FontHUDWingDings");
    let (lx, ly, lw, lh) = (cx + IMG_LIST.0, cy + IMG_LIST.1, IMG_LIST.2, IMG_LIST.3);
    c.set_clip(Some((lx, ly, lw, lh)));
    for (i, cl) in list.iter().enumerate() {
        let y = ly + i as i32 * CON_ROW_H;
        let sel = ui.image_sel == Some(i);
        if sel {
            c.fill(lx, y, lw, CON_ROW_H, Style::Normal, [128, 128, 128]);
        }
        let col = if sel { WHITE } else { crate::menuui::h_col(9) };
        let d = desc(ctx, cl);
        c.text(
            &f.small,
            lx + 2,
            y,
            148,
            CON_ROW_H,
            HAlign::Left,
            VAlign::Center,
            col,
            &d,
        );
        if !ui.seen_images.contains(cl) {
            if let Some(w) = &wing {
                c.text(
                    w,
                    lx + 150,
                    y,
                    34,
                    CON_ROW_H,
                    HAlign::Center,
                    VAlign::Center,
                    col,
                    "C",
                );
            }
        }
    }
    c.set_clip(None);
    let add = s_default(
        ctx,
        "PersonaScreenImages",
        "AddNoteButtonLabel",
        "Add |&Note",
    );
    let del = s_default(
        ctx,
        "PersonaScreenImages",
        "DeleteNoteButtonLabel",
        "|&Delete Note",
    );
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 10,
        cy + 422,
        129,
        &add,
        state(PB::AddNote),
        true,
    );
    draw_button(
        c,
        ctx.hud,
        ctx.assets,
        &f.headers,
        cx + 140,
        cy + 422,
        129,
        &del,
        state(PB::DeleteNote),
        true,
    );
    let show = s_default(ctx, "PersonaScreenImages", "ShowNotesLabel", "Show N|&otes");
    let r = checkbox_rect(f, cx, 617, 203, cy + 424, &show);
    let has = ui.image_sel.is_some();
    draw_checkbox(c, ctx, r, &show, !ui.hide_image_notes, has);
    // PersonaImageNewLegendLabel: "C = New Image", right-aligned 13 from the edge.
    let leg = s_default(
        ctx,
        "PersonaImageNewLegendLabel",
        "NewLegendLabel",
        " = New Image",
    );
    if let Some(w) = &wing {
        let tw = f.headers.width(&leg);
        let ww = w.width("C");
        let x = cx + 617 - 13 - tw - ww;
        c.text(
            w,
            x,
            cy + 424,
            ww,
            14,
            HAlign::Left,
            VAlign::Center,
            crate::menuui::h_col(8),
            "C",
        );
        c.text(
            &f.headers,
            x + ww,
            cy + 424,
            tw,
            14,
            HAlign::Left,
            VAlign::Center,
            crate::menuui::h_col(8),
            &leg,
        );
    }
    0
}

/// Leaving the images screen: viewed images lose their "new" mark (ClearViewedImageFlags).
pub fn left_screen(ui: &mut PersonaUi) {
    let v: Vec<String> = ui.viewed.drain().collect();
    ui.seen_images.extend(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PERSONA_PREVIEW=<dir> HUD_GAME=<game root> cargo test -p dxview preview_persona: writes screen_N.rgba
    /// (640 x 480 at 2x = 1280 x 960) for the health, augmentations, skills and logs screens.
    #[test]
    fn preview_persona() {
        let (Ok(out), Ok(root)) = (std::env::var("PERSONA_PREVIEW"), std::env::var("HUD_GAME"))
        else {
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
        let built = abilities::build(&mut assets.classes);
        *abilities::lock() = Some(built);
        abilities::give_aug(&mut assets.classes, "AugSpeed", false);
        abilities::give_aug(&mut assets.classes, "AugHealing", false);
        abilities::give_aug(&mut assets.classes, "AugBallistic", false);
        let mut hud = ClassicHud::default();
        let mut inv = PlayerInventory::default();
        for (c, n) in [
            ("MedKit", 3),
            ("BioelectricCell", 2),
            ("AugmentationUpgradeCannister", 1),
        ] {
            let d = assets.classes.defaults(c);
            let def = crate::pickups::item_def(c, &d);
            inv.0.pick_up(&def, n);
        }
        let mut ph = PlayerHealth::default();
        ph.parts = [60.0, 85.0, 100.0, 40.0, 100.0, 70.0];
        ph.energy = 64.0;
        log_message("You picked up the Medkit");
        log_message("Light activated");
        let get = |hud: &mut ClassicHud, a: &mut RuntimeAssets, n: &str| hud.font(a, n).unwrap();
        let fonts = Fonts {
            headers: get(&mut hud, &mut assets, "FontMenuHeaders"),
            small: get(&mut hud, &mut assets, "FontMenuSmall"),
            small_ds: get(&mut hud, &mut assets, "FontMenuSmall_DS"),
            tiny: get(&mut hud, &mut assets, "FontTiny"),
        };
        let lay = crate::inventory_screen::layout(&fonts);
        use crate::consys::{Goal, History};
        let g = |n: &str, t: &str, p: bool, c: bool| Goal {
            name: n.into(),
            text: t.into(),
            primary: p,
            completed: c,
        };
        crate::consys::install_test_state(
            &mut assets.classes,
            vec![
                g("DefeatNSFCommandCenter", "The NSF seem to be directing the attack from the Statue.  Get inside and locate the threat.", true, false),
                g("RescueAgent", "One of UNATCO's top agents is being held inside the Statue.  Break him out, and he'll back you up against the NSF.", false, false),
                g("MeetFilben", "Meet UNATCO informant Harley Filben at the North Docks.  He has a key to the Statue doors.", false, true),
            ],
            vec!["The code to the security door is 1234.".into(), "Harley Filben responds to the code-phrase \"iron and copper.\"".into()],
            vec![
                History { owner: "Paul Denton".into(), location: "NYC - Liberty Island".into(), description: String::new(), infolink: false, lines: vec![("JC Denton".into(), "Paul!  I thought you were in Hong Kong.".into()), ("Paul Denton".into(), "Welcome to the Coalition, JC... I might as well start using your codename.  Think I'd miss my brother's first day?".into())] },
                History { owner: "Alex Jacobson".into(), location: "NYC - Liberty Island".into(), description: String::new(), infolink: true, lines: vec![("Alex Jacobson".into(), "Heads up, JC.  Your brother Paul is on his way to meet you at the dock.".into())] },
            ],
            vec!["Image01_LibertyIsland".into()],
        );
        {
            let d = assets.classes.defaults("AugmentationCannister");
            let def = crate::pickups::item_def("AugmentationCannister", &d);
            let _ = crate::augcan::add(&mut inv, &def, ["AugSpeed".into(), "AugStealth".into()]);
            let _ = crate::augcan::add(&mut inv, &def, ["AugAqualung".into(), "AugEnviro".into()]);
        }
        let medbot = |remaining: f32| BotView { med: true, remaining, refresh: 60.0, amount: 300.0 };
        for (screen, bot, name) in [
            (SCREEN_HEALTH, None, "health"),
            (SCREEN_AUGS, None, "augs"),
            (SCREEN_SKILLS, None, "skills"),
            (SCREEN_LOGS, None, "logs"),
            (SCREEN_GOALS, None, "goals"),
            (SCREEN_CONVERSATIONS, None, "cons"),
            (SCREEN_IMAGES, None, "images"),
            (SCREEN_HEALTH, Some(medbot(23.0)), "medbot_health"),
            (SCREEN_AUGS, Some(medbot(0.0)), "medbot_augs"),
            (SCREEN_RECHARGE, Some(BotView { med: false, remaining: 0.0, refresh: 60.0, amount: 75.0 }), "repairbot"),
            (SCREEN_GOALS, None, "goals_edit"),
            (SCREEN_GOALS, None, "goals_select"),
            (SCREEN_GOALS, None, "goals_delete"),
        ] {
            let mut ui = PersonaUi::default();
            ui.show_completed = true;
            ui.note_sel = Some(1);
            ui.con_sel = Some(0);
            ui.image_sel = Some(0);
            ui.aug_sel = Some(0);
            ui.health_sel = Some(3);
            let mut ctx = Ctx {
                hud: &mut hud,
                assets: &mut assets,
                fonts: &fonts,
                lay: &lay,
                inv: &mut inv,
                ph: &mut ph,
                bot,
            };
            if name == "medbot_augs" {
                select_can_aug(&mut ui, &mut ctx, 0, 0);
            }
            use bevy::input::keyboard::KeyCode as K;
            let mut typed: Vec<(K, Option<String>)> = Vec::new();
            match name {
                "goals_edit" | "goals_select" => {
                    // Add Note (the default text selected), then type over it.
                    screen_action(SCREEN_GOALS, &mut ui, &mut ctx, PB::AddNote);
                    if name == "goals_edit" {
                        for ch in "Gunther's cell: the laser grid switch is behind the crates. Remember to\\ grab the medkit".chars() {
                            typed.push((K::KeyA, Some(ch.to_string())));
                        }
                        typed.push((K::Enter, None));
                        for ch in "Code 0451".chars() {
                            typed.push((K::KeyA, Some(ch.to_string())));
                        }
                        typed.push((K::ArrowLeft, None));
                        typed.push((K::ArrowLeft, None));
                    }
                }
                "goals_delete" => {
                    ui.note_sel = Some(0);
                    screen_action(SCREEN_GOALS, &mut ui, &mut ctx, PB::DeleteNote);
                }
                _ => {}
            }
            let inp = Input {
                mouse: (-100, -100),
                down: false,
                up: false,
                wheel: 0.0,
                dt: 0.0,
                up_key: false,
                down_key: false,
                enter: false,
                fkey: None,
                typed,
                ctrl: false,
                shift: false,
            };
            let (c, _) = page(screen, &mut ui, &mut ctx, &inp, 2);
            let bytes: Vec<u8> =
                c.px.iter()
                    .flat_map(|p| {
                        let bg = [0.13f32, 0.15, 0.17];
                        let o: Vec<u8> = (0..3)
                            .map(|k| ((p[k] + bg[k] * (1.0 - p[3])).min(1.0) * 255.0) as u8)
                            .collect();
                        [o[0], o[1], o[2], 255]
                    })
                    .collect();
            std::fs::write(format!("{out}/screen_{name}.rgba"), bytes).unwrap();
        }
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    // (The message log travels with the player, as DeusExPlayer's log does.)
    if let Ok(mut v) = NOTE_HITS.lock() {
        v.clear();
    }
}
