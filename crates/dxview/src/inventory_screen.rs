//! The inventory screen (PersonaScreenInventory and the persona screen frame around it).
//!
//! Layout, art, fonts, colours and rules are ported from PersonaScreenBaseWindow, PersonaNavBarWindow,
//! PersonaScreenInventory, PersonaInventoryItemButton, PersonaInventoryInfoWindow / PersonaInfoWindow,
//! PersonaItemDetailWindow, PersonaActionButtonWindow / PersonaButtonBarWindow, PersonaStatusLineWindow,
//! PersonaInventoryObjectBelt and the Update(Ammo)Info functions of the item classes.
//!
//! The whole 640 x 480 screen is drawn on one `uicanvas::Canvas` (whole-number scale, centred) and the DX mouse
//! cursor on a second small one that follows the mouse. While it's open the game is paused (virtual time stops)
//! and gets no input (`Bindings::capture`); the HUD is hidden.
//!
//! Rules: the 5 x 6 grid (items take invSlotsX x invSlotsY cells), select by clicking, drag to move within the
//! grid, drag onto a belt slot to assign it (not slot 0, the key ring's), drag a belt slot onto another to swap or
//! off the belt to clear it, drag a weapon mod onto a weapon it fits (highlighted green) to fit it; Equip/Unequip,
//! Use, Drop and Change Ammo buttons; keys 1-9 put the selected item on the belt, Delete clears the selected belt
//! slot, Backspace drops, Enter uses; clicking the key ring or ammo icons lists keys or ammunition; Esc or the
//! inventory key (F1) closes.

use imsim_core::inventory::class_of;
use crate::classic_hud::{ClassicHud, HudLayer};
use crate::keybinds::Bindings;
use crate::pickups::PlayerInventory;
use crate::uicanvas::{wrap, Canvas, HAlign, Rgb, Style, UiFont, UiTex, VAlign};
use crate::weapons::{
    apply_mods, fit_mod, weapon_stats, Clips, ModState, RuntimeAssets, WState, WeaponStats, Weapons,
};
use crate::{FlyCam, Player};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::CursorOptions;
use glam::DVec3;
use imsim_core::inventory::{ItemKind, INV_COLS, INV_ROWS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use unreal_pkg::props::{Property, Value};

/// True while a persona screen is up (the HUD and crosshair hide).
pub static SCREEN_OPEN: AtomicBool = AtomicBool::new(false);

pub fn screen_open() -> bool {
    SCREEN_OPEN.load(Ordering::Relaxed) || crate::computers::open() || crate::menus::open() || crate::menus::intro_map() || crate::credits::open()
}

static OPEN_BOT: std::sync::Mutex<Option<(Entity, bool)>> = std::sync::Mutex::new(None);

/// MedicalBot / RepairBot.Frob: open its screen next frame.
pub fn open_bot(e: Entity, medic: bool) {
    if let Ok(mut o) = OPEN_BOT.lock() {
        *o = Some((e, medic));
    }
}

/// HUDMedBot*Screen / HUDRechargeWindow.DestroyWindow: the bot lowers its arm and goes back to its orders.
fn release_bot(npcs: &mut Query<&mut crate::ai::Npc>, e: Entity) {
    if let Ok(mut n) = npcs.get_mut(e) {
        n.service_pause = 0.001;
    }
}

// Default HUD colour theme.
pub(crate) const COL_FILL_SELECTED: Rgb = [63, 63, 63];
pub(crate) const COL_MOD_TRUE: Rgb = [32, 128, 32];
pub(crate) const COL_MOD_FALSE: Rgb = [128, 32, 32];
pub(crate) const WHITE: Rgb = [255, 255, 255];

pub(crate) const SCREEN_W: i32 = 640;
pub(crate) const SCREEN_H: i32 = 480;
/// winClient origin on the screen: clientBorderOffset (0, 33) + clientOffset (33, 10).
const CX: i32 = 33;
const CY: i32 = 43;
const INV_BUTTON: i32 = 53;
/// winItems, winInfo, the action button bar, the status line, the key ring and ammo detail windows (screen).
const ITEMS: (i32, i32) = (CX + 9, CY + 19);
const INFO: (i32, i32, i32, i32) = (CX + 337, CY + 17, 238, 218);
const INFO_TEXT_Y: i32 = 20;
const BUTTONS: (i32, i32, i32) = (CX + 9, CY + 339, 267);
const STATUS: (i32, i32, i32) = (CX + 337, CY + 243, 238);
const KEYRING: (i32, i32, i32) = (CX + 335, CY + 285, 121);
const AMMO: (i32, i32, i32) = (CX + 456, CY + 285, 120);
const DETAIL_H: i32 = 55;
/// The object belt copy (PersonaInventoryObjectBelt 631 x 69 at the bottom right, its belt at +90).
const BELT: (i32, i32) = (SCREEN_W - 631 + 90, SCREEN_H - 69);

pub(crate) const NAV_LABELS: [&str; 8] = [
    "|&Inventory",
    "|&Health",
    "|&Augs",
    "|&Skills",
    "|&Goals/Notes",
    "|&Conversations",
    "I|&mages",
    "|&Logs",
];

#[derive(Clone, Debug, PartialEq)]
enum Sel {
    Item(String),
    KeyRing,
    Ammo,
}

#[derive(Clone, Debug, PartialEq)]
enum DragSrc {
    Item(String),
    Belt(usize),
}

#[derive(Clone, Debug)]
struct Drag {
    src: DragSrc,
    start: (i32, i32),
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Btn {
    Equip,
    Use,
    Drop,
    ChangeAmmo,
    /// Ours: Enhancement "Inventory Auto-Sort".
    Sort,
    Exit,
    Nav(usize),
}

fn sort_on() -> bool {
    crate::settings::get("InventorySort") > 0.5
}

/// One line of the info panel.
#[derive(Clone, Debug)]
enum Row {
    Text(String),
    Line,
    /// PersonaInfoItemWindow: label (right-aligned, 70 wide), value; highlighted values are white headers.
    Item(String, String, bool),
    /// The ammo buttons (PersonaAmmoDetailButton): class, has it, rounds, loaded.
    Ammo(Vec<(String, bool, i32, bool)>),
    /// An ammunition entry (icon + text) in the ammo list.
    AmmoEntry(String, String),
    Checkbox(String, bool),
}

#[derive(Resource)]
pub struct InvScreen {
    pub open: bool,
    sel: Option<Sel>,
    belt_sel: Option<usize>,
    drag: Option<Drag>,
    status: String,
    status_t: f32,
    scroll: i32,
    show_ammo_desc: bool,
    pressed: Option<Btn>,
    canvas: Option<(Handle<Image>, Entity, i32)>,
    cursor: Option<(Handle<Image>, Entity, i32)>,
    last: Vec<u8>,
    last_cursor: Vec<u8>,
    built: bool,
    grabbed_before: bool,
    /// Which persona screen (nav bar index; 0 = inventory).
    pub screen: usize,
    persona: crate::persona::PersonaUi,
    /// The medical (true) or repair bot whose screen this is.
    bot: Option<(Entity, bool)>,
}

impl Default for InvScreen {
    fn default() -> Self {
        InvScreen {
            open: false,
            sel: None,
            belt_sel: None,
            drag: None,
            status: String::new(),
            status_t: 0.0,
            scroll: 0,
            show_ammo_desc: false,
            pressed: None,
            canvas: None,
            cursor: None,
            last: Vec::new(),
            last_cursor: Vec::new(),
            built: false,
            grabbed_before: false,
            screen: 0,
            persona: Default::default(),
            bot: None,
        }
    }
}

impl InvScreen {
    fn set_status(&mut self, s: String) {
        self.status = s;
        self.status_t = 0.0;
    }
}

pub(crate) fn find_ci<'a>(p: &'a [Property], n: &str) -> Option<&'a Value> {
    p.iter()
        .find(|x| x.index == 0 && x.name.eq_ignore_ascii_case(n))
        .map(|x| &x.value)
}
pub(crate) fn str_p(p: &[Property], n: &str) -> String {
    match find_ci(p, n) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
pub(crate) fn int_p(p: &[Property], n: &str, d: i32) -> i32 {
    match find_ci(p, n) {
        Some(Value::Int(v)) => *v,
        Some(Value::Byte(v)) => *v as i32,
        Some(Value::Float(v)) => *v as i32,
        _ => d,
    }
}
pub(crate) fn float_p(p: &[Property], n: &str, d: f32) -> f32 {
    match find_ci(p, n) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => d,
    }
}
fn bool_p(p: &[Property], n: &str) -> bool {
    matches!(find_ci(p, n), Some(Value::Bool(true)))
}
fn class_p(p: &[Property], n: &str) -> Option<String> {
    match find_ci(p, n) {
        Some(Value::Path(s)) if !s.ends_with(".None") && s != "None" => {
            Some(s.rsplit('.').next().unwrap_or("").to_string())
        }
        _ => None,
    }
}
/// Labels carry "|&" before the accelerator letter.
pub(crate) fn label(s: &str) -> String {
    s.replace("|&", "")
}

/// DeusExWeapon.FormatFloatString.
fn fmt_float(value: f32, precision: f32) -> String {
    let mut s = format!("{}", value as i32);
    if precision < 1.0 {
        let frac = value - (value as i32) as f32;
        s = format!(
            "{s}.{}",
            (0.5 * precision + frac * (1.0 / precision)) as i32
        );
    }
    s
}
/// DeusExWeapon.BuildPercentString.
fn pct(value: f32) -> String {
    let n = (value * 100.0).abs() as i32;
    format!("({}{}%)", if value < 0.0 { "-" } else { "+" }, n)
}

/// Everything the screen needs about the player's stuff, read once per frame.
struct View<'a> {
    inv: &'a imsim_core::inventory::Inventory,
    weapons: &'a Weapons,
    clips: &'a Clips,
    charged: &'a crate::charged::ActiveCharged,
}

impl View<'_> {
    /// An item by its key (its class, or class#uid when several of it are carried).
    fn item(&self, key: &str) -> Option<(usize, &imsim_core::inventory::Item)> {
        self.inv
            .items
            .iter()
            .enumerate()
            .find(|(_, i)| i.key() == key)
    }
    /// The ammo a weapon has loaded (AmmoName, or what CycleAmmo picked).
    fn loaded_ammo(&self, class: &str, d: &[Property]) -> Option<String> {
        self.weapons
            .loaded_ammo
            .get(class)
            .cloned()
            .or_else(|| class_p(d, "AmmoName").filter(|a| a != "AmmoNone"))
    }
    /// AmmoType.AmmoAmount: rounds in boxes plus the ones loaded in weapons using that ammo.
    fn ammo_amount(&self, ammo: &str, assets: &mut RuntimeAssets) -> i32 {
        let pool = self
            .inv
            .ammo
            .iter()
            .find(|x| x.0 == ammo)
            .map(|x| x.1 as i32)
            .unwrap_or(0);
        let mut loaded = 0;
        for it in &self.inv.items {
            if it.def.kind != ItemKind::Weapon {
                continue;
            }
            let d = assets.classes.defaults(&it.def.class);
            if int_p(&d, "ReloadCount", 0) > 0
                && self.loaded_ammo(&it.def.class, &d).as_deref() == Some(ammo)
            {
                loaded += *self.clips.0.get(&it.def.class).unwrap_or(&0) as i32;
            }
        }
        pool + loaded
    }
    fn in_hand(&self, class: &str) -> bool {
        self.weapons
            .held
            .as_ref()
            .is_some_and(|h| h.stats.class == class && h.state != WState::Lowering)
            || self.weapons.pending_class.as_deref() == Some(class)
    }
}

/// The ammo classes a weapon takes (AmmoNames[], or AmmoName alone).
fn ammo_names(d: &[Property]) -> Vec<String> {
    let mut v: Vec<(u32, String)> = d
        .iter()
        .filter(|x| x.name.eq_ignore_ascii_case("AmmoNames"))
        .filter_map(|x| match &x.value {
            Value::Path(s) if !s.ends_with(".None") => {
                Some((x.index, s.rsplit('.').next().unwrap_or("").to_string()))
            }
            _ => None,
        })
        .collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

/// WeaponMod.CanUpgradeWeapon (without changing anything).
fn mod_fits(mod_class: &str, w: &WeaponStats, m: Option<&ModState>) -> bool {
    let mut st = m.cloned().unwrap_or_default();
    fit_mod(mod_class, w, &mut st)
}

/// The info panel for a selected item (Inventory.UpdateInfo and its overrides).
fn item_info(key: &str, view: &View, assets: &mut RuntimeAssets) -> (String, Vec<Row>) {
    let class = class_of(key);
    let d = assets.classes.defaults(class);
    let name = str_p(&d, "ItemName");
    let desc = str_p(&d, "Description");
    let Some((_, it)) = view.item(key) else {
        return (name, vec![]);
    };
    let count = it.count;
    match &it.def.kind {
        ItemKind::Weapon => weapon_info(class, &d, view, assets),
        _ => {
            let mut rows = Vec::new();
            let count_label = str_p(&d, "CountLabel");
            let copies = int_p(&d, "maxCopies", 1) > 1 || bool_p(&d, "bCanHaveMultipleCopies");
            if class == "MedKit" {
                rows.push(Row::Text(format!(
                    "{desc}\n\n\n{}\n\n{count_label} {count}",
                    str_p(&d, "MustBeUsedOn")
                )));
            } else if class == "BioelectricCell" {
                rows.push(Row::Text(format!(
                    "{desc}\n\nRecharges {} Energy Units\n\n{count_label} {count}",
                    int_p(&d, "rechargeAmount", 25)
                )));
            } else if class.starts_with("WeaponMod") {
                rows.push(Row::Text(format!(
                    "{desc}\n\n{}",
                    str_p(&d, "DragToUpgrade")
                )));
            } else if class.starts_with("Augmentation") {
                rows.push(Row::Text(format!(
                    "{desc}\n\n{}",
                    str_p(&d, "MustBeUsedOn")
                )));
            } else if crate::charged::is_charged(class) {
                let charge = view
                    .charged
                    .0
                    .iter()
                    .find(|c| c.class == class)
                    .map(|c| c.charge / c.max.max(1.0) * 100.0)
                    .unwrap_or(100.0);
                rows.push(Row::Text(format!(
                    "{desc}\n\n{} {}%",
                    str_p(&d, "ChargeRemainingLabel"),
                    charge as i32
                )));
            } else if copies {
                rows.push(Row::Text(format!("{desc}\n\n{count_label} {count}")));
            } else {
                rows.push(Row::Text(desc));
            }
            (name, rows)
        }
    }
}

/// DeusExWeapon.UpdateInfo.
fn weapon_info(
    class: &str,
    d: &[Property],
    view: &View,
    assets: &mut RuntimeAssets,
) -> (String, Vec<Row>) {
    let name = str_p(d, "ItemName");
    let mut rows = vec![Row::Text("Weapon Stats:".into()), Row::Line];
    let names = ammo_names(d);
    let h2h = bool_p(d, "bHandToHand");
    let reload = int_p(d, "ReloadCount", 0);
    let ammo_name = class_p(d, "AmmoName").unwrap_or_else(|| "AmmoNone".into());
    let current = view
        .loaded_ammo(class, d)
        .unwrap_or_else(|| ammo_name.clone());
    let mut buttons = Vec::new();
    if !names.is_empty() {
        for a in &names {
            let n = view.ammo_amount(a, assets);
            buttons.push((a.clone(), n > 0, n, *a == current));
        }
    } else if ammo_name != "AmmoNone" && !h2h && reload != 0 {
        let n = view.ammo_amount(&ammo_name, assets);
        buttons.push((ammo_name.clone(), n > 0, n, true));
    }
    let has_buttons = !buttons.is_empty();
    if has_buttons {
        rows.push(Row::Ammo(buttons));
        rows.push(Row::Line);
    }
    let ammo_item =
        |a: &str, assets: &mut RuntimeAssets| str_p(&assets.classes.defaults(a), "ItemName");
    if ammo_name != "AmmoNone" && !h2h && reload != 0 {
        rows.push(Row::Item(
            "Ammo loaded:".into(),
            ammo_item(&current, assets),
            false,
        ));
    }
    let mut types = if ammo_name == "AmmoNone" || h2h || reload == 0 {
        "N/A".to_string()
    } else {
        ammo_item(&current, assets)
    };
    for a in names.iter().filter(|a| **a != current) {
        types = format!("{types}\n{}", ammo_item(a, assets));
    }
    rows.push(Row::Item("Ammo type(s):".into(), types, false));
    let base = weapon_stats(class, d);
    let modst = view.weapons.mods.get(class).cloned().unwrap_or_default();
    let mut w = base.clone();
    if let Some(w) = w.as_mut() {
        apply_mods(w, &modst);
    }
    let hit = int_p(d, "HitDamage", 0);
    let aoe = int_p(d, "AreaOfEffect", 0);
    let instant = bool_p(d, "bInstantHit");
    let dmg = if aoe == 1 {
        hit * if instant { 5 } else { 3 }
    } else {
        hit
    };
    rows.push(Row::Item("Base damage:".into(), dmg.to_string(), false));
    let clip_mod = modst.clip != 0.0;
    let mut s = if reload == 0 || h2h {
        "N/A".to_string()
    } else {
        format!("{reload} RDS")
    };
    if clip_mod {
        s = format!(
            "{s} {} = {} RDS",
            pct(modst.clip),
            w.as_ref().map_or(reload as u32, |w| w.reload_count)
        );
    }
    rows.push(Row::Item("Clip size:".into(), s, clip_mod));
    let shot = float_p(d, "ShotTime", 0.5);
    let s = if reload == 0 || h2h {
        "N/A".to_string()
    } else {
        format!(
            "{}, {} RDS/SEC",
            if bool_p(d, "bAutomatic") {
                "AUTO"
            } else {
                "SINGLE"
            },
            fmt_float(1.0 / shot.max(0.01), 0.1)
        )
    };
    rows.push(Row::Item("Rate of fire:".into(), s, false));
    let rt = float_p(d, "ReloadTime", 2.0);
    let rmod = modst.reload != 0.0;
    let mut s = if reload == 0 || h2h {
        "N/A".to_string()
    } else {
        format!("{} SEC", fmt_float(rt, 0.1))
    };
    if rmod {
        s = format!(
            "{s} {} = {} SEC",
            pct(modst.reload),
            fmt_float(w.as_ref().map_or(rt, |w| w.reload_time), 0.1)
        );
    }
    rows.push(Row::Item("Reload time:".into(), s, rmod));
    let rec = float_p(d, "recoilStrength", 0.0);
    let cmod = modst.recoil != 0.0;
    let mut s = fmt_float(rec, 0.01);
    if cmod {
        s = format!(
            "{s} {} = {}",
            pct(modst.recoil),
            fmt_float(w.as_ref().map_or(rec, |w| w.recoil), 0.01)
        );
    }
    rows.push(Row::Item("Recoil:".into(), s, cmod));
    let acc = float_p(d, "BaseAccuracy", 0.5);
    let cur_acc = w.as_ref().map_or(acc, |w| w.base_accuracy);
    let m = (acc - cur_acc) * 0.5;
    let mut s = format!("{}%", ((2.0 - acc) * 50.0) as i32);
    if m != 0.0 {
        s = format!(
            "{s} {} = {}%",
            pct(m),
            ((100.0 * m + (2.0 - acc) * 50.0) as i32).min(100)
        );
    }
    rows.push(Row::Item("Base Accuracy:".into(), s, m != 0.0));
    let ar = float_p(d, "AccurateRange", 0.0);
    let gmod = modst.range != 0.0;
    let mut s = if h2h {
        "N/A".to_string()
    } else {
        format!("{} FT", fmt_float(ar / 16.0, 1.0))
    };
    if gmod {
        s = format!(
            "{s} {} = {} FT",
            pct(modst.range),
            fmt_float(w.as_ref().map_or(ar, |w| w.accurate_range) / 16.0, 1.0)
        );
    }
    rows.push(Row::Item("Acc. range:".into(), s, gmod));
    let s = if h2h {
        "N/A".to_string()
    } else {
        format!("{} FT", fmt_float(float_p(d, "maxRange", 0.0) / 16.0, 1.0))
    };
    rows.push(Row::Item("Max. range:".into(), s, false));
    rows.push(Row::Item(
        "Mass:".into(),
        format!("{} LBS", fmt_float(float_p(d, "Mass", 0.0), 1.0)),
        false,
    ));
    for (lab, can, has_def, has_now) in [
        (
            "Laser sight:",
            bool_p(d, "bCanHaveLaser"),
            bool_p(d, "bHasLaser"),
            bool_p(d, "bHasLaser") || modst.laser,
        ),
        (
            "Scope:",
            bool_p(d, "bCanHaveScope"),
            bool_p(d, "bHasScope"),
            bool_p(d, "bHasScope") || modst.scope,
        ),
        (
            "Silencer:",
            bool_p(d, "bCanHaveSilencer"),
            bool_p(d, "bHasSilencer"),
            bool_p(d, "bHasSilencer") || modst.silencer,
        ),
    ] {
        let s = if can {
            if has_now {
                "YES"
            } else {
                "NO"
            }
        } else {
            "N/A"
        };
        rows.push(Row::Item(
            lab.into(),
            s.into(),
            can && has_now && has_def != has_now,
        ));
    }
    let skill = class_p(d, "GoverningSkill")
        .map(|s| str_p(&assets.classes.defaults(&s), "SkillName"))
        .unwrap_or_default();
    rows.push(Row::Item("Skill:".into(), skill, false));
    rows.push(Row::Line);
    rows.push(Row::Text(str_p(d, "Description")));
    if has_buttons {
        let ad = assets.classes.defaults(&current);
        rows.push(Row::Line);
        rows.push(Row::Text(format!(
            "{}\n{}",
            str_p(&ad, "ItemName"),
            str_p(&ad, "Description")
        )));
    }
    (name, rows)
}

/// PersonaScreenInventory.UpdateAmmoDisplay.
fn ammo_list(view: &View, show_desc: bool, assets: &mut RuntimeAssets) -> (String, Vec<Row>) {
    let mut rows = vec![
        Row::Checkbox("Show Ammo Descriptions".into(), show_desc),
        Row::Line,
    ];
    let mut classes: Vec<String> = view.inv.ammo.iter().map(|a| a.0.clone()).collect();
    for it in &view.inv.items {
        if it.def.kind == ItemKind::Weapon {
            let d = assets.classes.defaults(&it.def.class);
            if let Some(a) = view.loaded_ammo(&it.def.class, &d) {
                if !classes.contains(&a) {
                    classes.push(a);
                }
            }
        }
    }
    let mut n = 0;
    for a in classes {
        let d = assets.classes.defaults(&a);
        if d.is_empty() || matches!(find_ci(&d, "bShowInfo"), Some(Value::Bool(false))) {
            continue;
        }
        let amount = view.ammo_amount(&a, assets);
        let name = str_p(&d, "ItemName");
        let text = if show_desc {
            format!("{name} (Rounds: {amount})\n\n{}", str_p(&d, "Description"))
        } else {
            format!("{name}\n\nRounds: {amount}")
        };
        rows.push(Row::AmmoEntry(a.clone(), text));
        rows.push(Row::Line);
        n += 1;
    }
    if n == 0 {
        return (
            "Ammunition".into(),
            vec![Row::Text("No Ammo Available".into())],
        );
    }
    ("Ammunition".into(), rows)
}

/// NanoKeyRing.UpdateInfo: "Nano Keys Available:", a line, each key's description (newest first: GiveKey adds
/// to the front of KeyList), a line and the ring's Description; "No Nano Keys Available!" when it's empty.
fn keyring_info(assets: &mut RuntimeAssets) -> (String, Vec<Row>) {
    let d = assets.classes.defaults("NanoKeyRing");
    let keys = crate::keys::list();
    if keys.is_empty() {
        return (str_p(&d, "ItemName"), vec![Row::Text(str_p(&d, "NoKeys"))]);
    }
    let mut rows = vec![Row::Text(str_p(&d, "KeysAvailableLabel")), Row::Line];
    for (_, desc) in keys.iter().rev() {
        rows.push(Row::Text(format!("  {desc}")));
    }
    rows.push(Row::Line);
    rows.push(Row::Text(str_p(&d, "Description")));
    (str_p(&d, "ItemName"), rows)
}

pub(crate) struct Fonts {
    pub(crate) headers: Arc<UiFont>,
    pub(crate) small: Arc<UiFont>,
    pub(crate) small_ds: Arc<UiFont>,
    pub(crate) tiny: Arc<UiFont>,
}

/// An item button's rectangle on the screen.
fn item_rect(it: &imsim_core::inventory::Item) -> Option<(i32, i32, i32, i32)> {
    let (c, r) = it.pos?;
    Some((
        ITEMS.0 + c as i32 * INV_BUTTON,
        ITEMS.1 + r as i32 * INV_BUTTON,
        INV_BUTTON * it.def.inv_size.0 as i32 + 1,
        INV_BUTTON * it.def.inv_size.1 as i32 + 1,
    ))
}

pub(crate) fn inside(p: (i32, i32), r: (i32, i32, i32, i32)) -> bool {
    p.0 >= r.0 && p.1 >= r.1 && p.0 < r.0 + r.2 && p.1 < r.1 + r.3
}

/// Belt slot rectangles (HUDObjectBelt: 1..9 then 0).
fn belt_slot_rect(slot: usize) -> (i32, i32, i32, i32) {
    let (x, w) = if slot == 0 {
        (10 + 9 * 51, 44)
    } else {
        (10 + (slot as i32 - 1) * 51, 51)
    };
    (BELT.0 + x, BELT.1 + 6, w, 54)
}

/// PersonaButtonBarWindow layout: preferred widths plus an even share of the spare room (bFillAllSpace).
pub(crate) fn bar_layout(
    font: &UiFont,
    labels: &[&str],
    x: i32,
    width: i32,
    center: bool,
) -> Vec<(i32, i32)> {
    let pref: Vec<i32> = labels
        .iter()
        .map(|l| (4 + if center { 0 } else { 6 } + font.width(&label(l)) + 8).max(20))
        .collect();
    let used: i32 = pref.iter().sum();
    let pad = if labels.is_empty() {
        0
    } else {
        (width - used) / labels.len() as i32
    };
    let mut out = Vec::new();
    let mut cx = x;
    for p in pref {
        out.push((cx, p + pad));
        cx += p + pad;
    }
    out
}

/// What the mouse is over.
#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Item(String),
    Grid(i32, i32),
    Belt(usize),
    Button(Btn),
    KeyRing,
    Ammo,
    AmmoButton(String),
    Checkbox,
    Info,
    None,
}

pub(crate) struct Layout {
    pub(crate) nav: Vec<(i32, i32)>,
    pub(crate) exit: (i32, i32),
    actions: Vec<(Btn, i32, i32)>,
}

pub(crate) fn layout(f: &Fonts) -> Layout {
    let nav = bar_layout(&f.headers, &NAV_LABELS, 23, 534, true);
    let exit = bar_layout(&f.headers, &["E|&xit"], 573, 48, true)[0];
    let mut acts = vec![Btn::Equip, Btn::Use, Btn::Drop, Btn::ChangeAmmo];
    let mut labels = vec!["|&Equip", "|&Use", "|&Drop", "Change Amm|&o"];
    if sort_on() {
        acts.push(Btn::Sort);
        labels.push("|&Sort");
    }
    let bars = bar_layout(&f.headers, &labels, BUTTONS.0, BUTTONS.2, false);
    Layout {
        nav,
        exit,
        actions: acts
            .iter()
            .zip(bars)
            .map(|(b, (x, w))| (*b, x, w))
            .collect(),
    }
}

/// The inventory screen's widgets for the controller's snap-to (nav, exit, actions, items, belt, key ring, ammo).
fn snap_rects(view: &View, lay: &Layout) -> Vec<(i32, i32, i32, i32)> {
    let mut v: Vec<(i32, i32, i32, i32)> = lay.nav.iter().map(|(x, w)| (*x, 8, *w, 16)).collect();
    v.push((lay.exit.0, 8, lay.exit.1, 16));
    v.extend(lay.actions.iter().map(|(_, x, w)| (*x, BUTTONS.1, *w, 16)));
    v.extend(view.inv.items.iter().filter_map(item_rect));
    v.extend((0..10).map(belt_slot_rect));
    v.push((KEYRING.0, KEYRING.1, 55, 55));
    v.push((AMMO.0, AMMO.1, 55, 55));
    v
}

fn hit_test(
    p: (i32, i32),
    view: &View,
    lay: &Layout,
    info_rows: &[(i32, i32, Row)],
    scroll: i32,
) -> Hit {
    for (i, (x, w)) in lay.nav.iter().enumerate() {
        if inside(p, (*x, 8, *w, 16)) {
            return Hit::Button(Btn::Nav(i));
        }
    }
    if inside(p, (lay.exit.0, 8, lay.exit.1, 16)) {
        return Hit::Button(Btn::Exit);
    }
    for (b, x, w) in &lay.actions {
        if inside(p, (*x, BUTTONS.1, *w, 16)) {
            return Hit::Button(*b);
        }
    }
    for it in &view.inv.items {
        if let Some(r) = item_rect(it) {
            if inside(p, r) {
                return Hit::Item(it.key());
            }
        }
    }
    if inside(
        p,
        (
            ITEMS.0,
            ITEMS.1,
            INV_BUTTON * INV_COLS as i32,
            INV_BUTTON * INV_ROWS as i32,
        ),
    ) {
        return Hit::Grid((p.0 - ITEMS.0) / INV_BUTTON, (p.1 - ITEMS.1) / INV_BUTTON);
    }
    for slot in 0..10 {
        if inside(p, belt_slot_rect(slot)) {
            return Hit::Belt(slot);
        }
    }
    if inside(p, (KEYRING.0, KEYRING.1, 55, 55)) {
        return Hit::KeyRing;
    }
    if inside(p, (AMMO.0, AMMO.1, 55, 55)) {
        return Hit::Ammo;
    }
    if inside(
        p,
        (INFO.0, INFO.1 + INFO_TEXT_Y, INFO.2, INFO.3 - INFO_TEXT_Y),
    ) {
        for (y, h, row) in info_rows {
            let y = INFO.1 + INFO_TEXT_Y + y - scroll;
            if p.1 < y || p.1 >= y + h {
                continue;
            }
            match row {
                Row::Ammo(btns) => {
                    for (i, b) in btns.iter().enumerate() {
                        let bx = INFO.0 + 4 + 74 + i as i32 * 58;
                        if inside(p, (bx, y, 54, 54)) {
                            return Hit::AmmoButton(b.0.clone());
                        }
                    }
                }
                Row::Checkbox(..) => return Hit::Checkbox,
                _ => {}
            }
        }
        return Hit::Info;
    }
    Hit::None
}

/// Height of each info row (TileWindow ORDER_Down, margins 4/1): (y, height, row).
fn layout_rows(rows: &[Row], f: &Fonts) -> Vec<(i32, i32, Row)> {
    let width = INFO.2 - 8;
    let mut y = 1;
    let mut out = Vec::new();
    for r in rows {
        let h = match r {
            Row::Text(t) => wrap(&f.small, t, width).len() as i32 * f.small.height,
            Row::Line => 5,
            Row::Item(_, v, hl) => {
                let font = if *hl { &f.headers } else { &f.small };
                wrap(font, v, width - 80).len().max(1) as i32 * font.height
            }
            Row::Ammo(_) => 54,
            Row::AmmoEntry(_, t) => {
                (wrap(&f.small, t, width - 46).len() as i32 * f.small.height).max(37)
            }
            Row::Checkbox(..) => 14,
        };
        out.push((y, h, r.clone()));
        y += h;
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn inventory_screen_system(
    mut commands: Commands,
    real: Res<Time<Real>>,
    mut vtime: ResMut<Time<Virtual>>,
    window: Single<&Window>,
    mut cursor_opts: Single<&mut CursorOptions>,
    mut st: ResMut<InvScreen>,
    mut binds: ResMut<Bindings>,
    mut wheel: MessageReader<MouseWheel>,
    (mut hud, mut assets, mut images, mut layers): (
        ResMut<ClassicHud>,
        ResMut<RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<HudLayer>>,
    ),
    (mut inv, mut weapons, mut clips, charged): (
        ResMut<PlayerInventory>,
        ResMut<Weapons>,
        ResMut<Clips>,
        Res<crate::charged::ActiveCharged>,
    ),
    (player, cam, mut drops, mut msg, mut ph): (
        Res<Player>,
        Single<&Transform, With<FlyCam>>,
        ResMut<crate::destruct::ItemThrows>,
        ResMut<crate::movers::HudMessage>,
        ResMut<crate::ai::PlayerHealth>,
    ),
    mut nodes: Query<(&mut Node, &mut Visibility, &MaterialNode<HudLayer>)>,
    mut npcs: Query<&mut crate::ai::Npc>,
) {
    let _prof = crate::prof::scope("inventory_screen::inventory_screen_system");
    let dt = real.delta_secs();
    let st = &mut *st;
    if !st.built {
        st.built = true;
        for which in 0..2 {
            let img = images.add(blank(1, 1));
            let e = commands
                .spawn((
                    MaterialNode(layers.add(HudLayer::new(img.clone()))),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    Visibility::Hidden,
                    Pickable::IGNORE,
                    GlobalZIndex(40 + which),
                ))
                .id();
            if which == 0 {
                st.canvas = Some((img, e, 0));
            } else {
                st.cursor = Some((img, e, 0));
            }
        }
    }
    // F1 opens the inventory, F2 the goals screen (ShowInventoryWindow / ShowGoalsWindow); pressing the key of the
    // screen that's up, or Esc, closes it.
    // A medical or repair bot's screen (the game keeps running behind it, so its charge counts down).
    if let Some((e, medic)) = OPEN_BOT.lock().ok().and_then(|mut o| o.take()) {
        if st.open || ph.dead || binds.open || crate::computers::open() || crate::menus::open() || crate::menus::intro_map() {
            release_bot(&mut npcs, e);
        } else {
            st.open = true;
            st.bot = Some((e, medic));
            st.screen = if !medic {
                crate::persona::SCREEN_RECHARGE
            } else if !crate::augcan::list(&inv).is_empty() {
                crate::persona::SCREEN_AUGS
            } else {
                crate::persona::SCREEN_HEALTH
            };
            st.persona = Default::default();
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Activate", 0.25, 0.0);
            st.grabbed_before = cursor_opts.grab_mode != bevy::window::CursorGrabMode::None;
            cursor_opts.grab_mode = bevy::window::CursorGrabMode::None;
            cursor_opts.visible = false;
            st.drag = None;
            st.pressed = None;
            st.scroll = 0;
        }
    }
    // While a note is being typed into, its keys aren't screen hotkeys (PersonaEditWindow keeps all but Tab
    // and Escape); with a message box up, Escape answers it.
    let editing = st.open && st.persona.editing();
    let f1 = binds.action_just_pressed("Inventory screen") && st.bot.is_none() && !editing;
    let f2 = binds.action_just_pressed("Goals / notes screen") && st.bot.is_none() && !editing;
    let want = if f1 {
        Some(0)
    } else if f2 {
        Some(crate::persona::SCREEN_GOALS)
    } else {
        None
    };
    let mut toggle = (st.open && binds.keys.just_pressed(KeyCode::Escape) && !st.persona.modal())
        || want.is_some_and(|w| !st.open || w == st.screen);
    if let (Some(w), true) = (want, st.open) {
        if w != st.screen {
            st.screen = w;
            toggle = false;
        }
    } else if let (Some(w), false) = (want, st.open) {
        st.screen = w;
    }
    if toggle && !binds.open {
        if !st.open && (ph.dead || crate::computers::open()) {
            return;
        }
        st.open = !st.open;
        crate::sound::play_player(
            if st.open {
                "DeusExSounds.UserInterface.Menu_Activate"
            } else {
                "DeusExSounds.UserInterface.Menu_OK"
            },
            0.25,
            0.0,
        );
        if st.open {
            vtime.pause();
            st.grabbed_before = cursor_opts.grab_mode != bevy::window::CursorGrabMode::None;
            cursor_opts.grab_mode = bevy::window::CursorGrabMode::None;
            cursor_opts.visible = false;
            st.drag = None;
            st.pressed = None;
            st.scroll = 0;
        } else {
            vtime.unpause();
            cursor_opts.visible = !st.grabbed_before;
            if st.grabbed_before {
                cursor_opts.grab_mode = bevy::window::CursorGrabMode::Locked;
            }
        }
    }
    if !st.open {
        st.persona.end_edit();
        if let Some((e, _)) = st.bot.take() {
            release_bot(&mut npcs, e);
        }
    }
    binds.capture = st.open || crate::computers::open() || crate::menus::open() || crate::credits::open();
    SCREEN_OPEN.store(st.open, Ordering::Relaxed);
    let (canvas_img, canvas_e, _) = st.canvas.clone().unwrap();
    let (cursor_img, cursor_e, _) = st.cursor.clone().unwrap();
    if !st.open {
        for e in [canvas_e, cursor_e] {
            if let Ok((_, mut v, _)) = nodes.get_mut(e) {
                if *v != Visibility::Hidden {
                    *v = Visibility::Hidden;
                }
            }
        }
        wheel.clear();
        return;
    }
    st.status_t += dt;
    if st.status_t > 5.0 {
        st.status.clear();
    }

    let (pw, phh) = (
        window.physical_width() as i32,
        window.physical_height() as i32,
    );
    // The largest whole-number scale at which the 640 x 480 screen fits the window (crisp pixels).
    // (s: the canvas's whole-number scale; d: the scale shown, fractional with HUD Scale Large.)
    let (s, d) = crate::hud::screen_scales(&window, SCREEN_W, SCREEN_H);
    crate::hud::set_filter(&mut images, &canvas_img, s, d);
    let ox = (pw - (SCREEN_W as f32 * d) as i32) / 2;
    let oy = (phh - (SCREEN_H as f32 * d) as i32) / 2;
    let sf = window.scale_factor();
    let mouse = window
        .cursor_position()
        .map(|c| {
            (
                ((c.x * sf - ox as f32) / d).floor() as i32,
                ((c.y * sf - oy as f32) / d).floor() as i32,
            )
        })
        .unwrap_or((-100, -100));

    let fonts = {
        let get = |hud: &mut ClassicHud, assets: &mut RuntimeAssets, n: &str| hud.font(assets, n);
        let (Some(headers), Some(small), Some(small_ds), Some(tiny)) = (
            get(&mut hud, &mut assets, "FontMenuHeaders"),
            get(&mut hud, &mut assets, "FontMenuSmall"),
            get(&mut hud, &mut assets, "FontMenuSmall_DS"),
            get(&mut hud, &mut assets, "FontTiny"),
        ) else {
            return;
        };
        Fonts {
            headers,
            small,
            small_ds,
            tiny,
        }
    };
    let lay = layout(&fonts);

    // The other persona screens.
    if st.screen != 0 {
        let wheel_lines: f32 = wheel
            .read()
            .map(|w| {
                if w.unit == bevy::input::mouse::MouseScrollUnit::Line {
                    w.y * 3.0
                } else {
                    w.y / 4.0
                }
            })
            .sum();
        let fkey = crate::abilities::AUG_KEYS
            .iter()
            .position(|k| binds.keys.just_pressed(*k));
        let input = crate::persona::Input {
            mouse,
            down: binds.buttons.just_pressed(MouseButton::Left),
            up: binds.buttons.just_released(MouseButton::Left),
            wheel: wheel_lines,
            dt,
            up_key: binds.keys.just_pressed(KeyCode::ArrowUp),
            down_key: binds.keys.just_pressed(KeyCode::ArrowDown),
            enter: binds.keys.just_pressed(KeyCode::Enter),
            fkey,
            typed: binds.typed.clone(),
            ctrl: binds.keys.pressed(KeyCode::ControlLeft) || binds.keys.pressed(KeyCode::ControlRight),
            shift: binds.keys.pressed(KeyCode::ShiftLeft) || binds.keys.pressed(KeyCode::ShiftRight),
        };
        let screen = st.screen;
        let bot = st.bot.and_then(|(e, medic)| {
                let n = npcs.get(e).ok()?;
                let (cls, amt_key, ref_key, amt) = if medic {
                    ("MedicalBot", "healAmount", "healRefreshTime", 300.0)
                } else {
                    ("RepairBot", "chargeAmount", "chargeRefreshTime", 75.0)
                };
                let d = assets.classes.defaults(cls);
                Some(crate::persona::BotView {
                    med: medic,
                    remaining: n.service_cooldown,
                    refresh: int_p(&d, ref_key, 60) as f32,
                    amount: int_p(&d, amt_key, amt as i32) as f32,
                })
            });
        let (c, act) = {
            let mut ctx = crate::persona::Ctx {
                hud: &mut hud,
                assets: &mut assets,
                fonts: &fonts,
                lay: &lay,
                inv: &mut inv,
                ph: &mut ph,
                bot: bot.clone(),
            };
            crate::snap::set_page_map(Some(crate::snap::Map { s: d, ox, oy, sf }));
            let r = crate::persona::page(screen, &mut st.persona, &mut ctx, &input, s);
            crate::snap::set_page_map(None);
            r
        };
        match act {
            Some(crate::persona::Act::Exit) => {
                if let Some((e, _)) = st.bot.take() {
                    release_bot(&mut npcs, e);
                }
                st.open = false;
                vtime.unpause();
                binds.capture = false;
                SCREEN_OPEN.store(false, Ordering::Relaxed);
                cursor_opts.visible = !st.grabbed_before;
                if st.grabbed_before {
                    cursor_opts.grab_mode = bevy::window::CursorGrabMode::Locked;
                }
                return;
            }
            Some(crate::persona::Act::Nav(i)) => {
                st.screen = i;
                st.persona = Default::default();
            }
            Some(crate::persona::Act::UseItem(class)) => weapons.use_request = Some(class),
            Some(crate::persona::Act::BotHeal) | Some(crate::persona::Act::BotCharge) => {
                if let (Some((e, medic)), Some(b)) = (st.bot, bot.as_ref()) {
                    if let Ok(mut n) = npcs.get_mut(e) {
                        if n.service_cooldown <= 0.0 {
                            n.service_cooldown = b.refresh;
                            if medic {
                                // MedicalBot.HealPlayer -> DeusExPlayer.HealPlayer(healAmount).
                                let healed = ph.heal(b.amount);
                                crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
                                let lab = crate::persona::s_default_pub(&mut assets, "PersonaScreenHealth", "PointsHealedLabel", "%d points healed");
                                st.persona.set_status(lab.replacen("%d", &(healed as i32).to_string(), 1));
                            } else {
                                // RepairBot.ChargePlayer -> DeusExPlayer.ChargePlayer(chargeAmount).
                                ph.energy = (ph.energy + b.amount).min(100.0);
                            }
                        }
                    }
                }
            }
            Some(crate::persona::Act::Install(i, class)) => {
                // InstallAugmentation: GivePlayerAugmentation, the canister is used up.
                if let Some(m) = crate::abilities::give_aug(&mut assets.classes, &class, false) {
                    if !m.is_empty() {
                        msg.text = m.clone();
                        msg.time_left = 3.0;
                        st.persona.set_status(m);
                    }
                }
                crate::augcan::remove(&mut inv, i);
                crate::sound::play_player("DeusExSounds.Generic.MedicalHiss", 1.0, 256.0);
            }
            None => {}
        }
        present(
            &mut images,
            &mut layers,
            &mut nodes,
            st,
            &c,
            canvas_e,
            &canvas_img,
            ox,
            oy,
            d,
            sf,
        );
        let mut cc = Canvas::new(64, 64, s);
        cursor_with_shadow(&mut hud, &mut assets, &mut cc);
        if upload(&mut images, &cursor_img, &mut st.last_cursor, &cc) {
            if let Ok((_, _, m)) = nodes.get(cursor_e) {
                if let Some(m) = layers.get_mut(&m.0) {
                    m.into_inner();
                }
            }
        }
        if let Ok((mut n, mut v, _)) = nodes.get_mut(cursor_e) {
            *v = if window.cursor_position().is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            n.left = Val::Px((ox as f32 + (mouse.0 - 32) as f32 * d) / sf);
            n.top = Val::Px((oy as f32 + (mouse.1 - 32) as f32 * d) / sf);
            n.width = Val::Px(64.0 * d / sf);
            n.height = Val::Px(64.0 * d / sf);
        }
        return;
    }

    // Keep the selection pointing at something that still exists.
    if let Some(Sel::Item(c)) = &st.sel {
        if !inv.0.items.iter().any(|i| &i.key() == c) {
            st.sel = None;
        }
    }

    // ---------------------------------------------------------------- input
    let (info_title, info_rows) = {
        let view = View {
            inv: &inv.0,
            weapons: &weapons,
            clips: &clips,
            charged: &charged,
        };
        match &st.sel {
            Some(Sel::Item(c)) => item_info(c, &view, &mut assets),
            Some(Sel::Ammo) => ammo_list(&view, st.show_ammo_desc, &mut assets),
            Some(Sel::KeyRing) => keyring_info(&mut assets),
            None => (String::new(), Vec::new()),
        }
    };
    let rows = layout_rows(&info_rows, &fonts);
    let content_h = rows.last().map_or(0, |r| r.0 + r.1) + 1;
    let view_h = INFO.3 - INFO_TEXT_Y;
    for w in wheel.read() {
        let lines = if w.unit == bevy::input::mouse::MouseScrollUnit::Line {
            w.y * 3.0
        } else {
            w.y / 4.0
        };
        st.scroll -= (lines * fonts.small.height as f32) as i32;
    }
    st.scroll = st.scroll.clamp(0, (content_h - view_h).max(0));

    let hit = {
        let view = View {
            inv: &inv.0,
            weapons: &weapons,
            clips: &clips,
            charged: &charged,
        };
        {
            crate::snap::offer(crate::snap::LAYER_PERSONA, crate::snap::Map { s: d, ox, oy, sf }, snap_rects(&view, &lay));
            hit_test(mouse, &view, &lay, &rows, st.scroll)
        }
    };
    let left_down = binds.buttons.just_pressed(MouseButton::Left);
    let left_up = binds.buttons.just_released(MouseButton::Left);
    let mut action: Option<Btn> = None;
    if left_down {
        match &hit {
            Hit::Button(b) => st.pressed = Some(*b),
            Hit::Item(c) => {
                st.drag = Some(Drag {
                    src: DragSrc::Item(c.clone()),
                    start: mouse,
                    active: false,
                })
            }
            Hit::Belt(slot) if inv.0.belt[*slot].is_some() => {
                st.drag = Some(Drag {
                    src: DragSrc::Belt(*slot),
                    start: mouse,
                    active: false,
                })
            }
            _ => {}
        }
    }
    if let Some(d) = st.drag.as_mut() {
        if !d.active && ((mouse.0 - d.start.0).abs() > 2 || (mouse.1 - d.start.1).abs() > 2) {
            d.active = true;
        }
    }
    if left_up {
        if let Some(b) = st.pressed.take() {
            if hit == Hit::Button(b) {
                action = Some(b);
            }
        }
        match st.drag.take() {
            Some(d) if d.active => {
                finish_drag(st, &d, &hit, mouse, &mut inv, &mut weapons, &mut assets)
            }
            Some(Drag {
                src: DragSrc::Item(c),
                ..
            }) => select(st, Sel::Item(c), &inv),
            Some(Drag {
                src: DragSrc::Belt(slot),
                ..
            }) => {
                if let Some(i) = inv.0.belt[slot] {
                    let c = inv.0.items[i].key();
                    select(st, Sel::Item(c), &inv);
                }
            }
            None => match &hit {
                Hit::KeyRing => select(st, Sel::KeyRing, &inv),
                Hit::Ammo => select(st, Sel::Ammo, &inv),
                Hit::Checkbox => st.show_ammo_desc = !st.show_ammo_desc,
                Hit::AmmoButton(a) => {
                    if let Some(Sel::Item(c)) = st.sel.clone() {
                        load_ammo(
                            st,
                            class_of(&c),
                            Some(a.clone()),
                            &mut inv,
                            &mut weapons,
                            &mut clips,
                            &mut assets,
                        );
                    }
                }
                _ => {}
            },
        }
    }
    // Keys (VirtualKeyPressed).
    let sel_class = match &st.sel {
        Some(Sel::Item(c)) => Some(c.clone()),
        _ => None,
    };
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (i, k) in digits.iter().enumerate() {
        if binds.keys.just_pressed(*k) {
            if let Some(c) = &sel_class {
                if let Some(idx) = inv.0.items.iter().position(|it| &it.key() == c) {
                    inv.0.belt_assign(idx, i + 1);
                    st.belt_sel = Some(i + 1);
                }
            }
        }
    }
    if binds.keys.just_pressed(KeyCode::Backspace) {
        action = Some(Btn::Drop);
    }
    if binds.keys.just_pressed(KeyCode::Enter) {
        action = Some(Btn::Use);
    }
    if binds.keys.just_pressed(KeyCode::Delete) {
        if let Some(slot) = st.belt_sel.filter(|s| *s != 0) {
            inv.0.belt[slot] = None;
            st.belt_sel = None;
        }
    }

    // Button enabling (EnableButtons).
    let enabled = |b: Btn,
                   st: &InvScreen,
                   assets: &mut RuntimeAssets,
                   inv: &imsim_core::inventory::Inventory|
     -> bool {
        if b == Btn::Sort {
            return true;
        }
        let Some(Sel::Item(c)) = &st.sel else {
            return matches!(b, Btn::Exit | Btn::Nav(_));
        };
        let d = assets.classes.defaults(class_of(c));
        let weapon = inv
            .items
            .iter()
            .any(|i| &i.key() == c && i.def.kind == ItemKind::Weapon);
        match b {
            Btn::Exit | Btn::Nav(_) | Btn::Sort => true,
            Btn::Drop => true,
            Btn::Equip => {
                !c.starts_with("WeaponMod")
                    && !c.starts_with("Augmentation")
                    && (weapon || find_ci(&d, "PlayerViewMesh").is_some())
            }
            Btn::Use => !weapon && !c.starts_with("WeaponMod") && !c.starts_with("Augmentation"),
            Btn::ChangeAmmo => {
                weapon
                    && ammo_names(&d)
                        .iter()
                        .filter(|a| inv.ammo.iter().any(|x| &x.0 == *a && x.1 > 0))
                        .count()
                        >= 2
            }
        }
    };
    if let Some(b) = action {
        if enabled(b, st, &mut assets, &inv.0) {
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
            match b {
                Btn::Exit => {
                    st.open = false;
                    vtime.unpause();
                    binds.capture = false;
                    SCREEN_OPEN.store(false, Ordering::Relaxed);
                    cursor_opts.visible = !st.grabbed_before;
                    if st.grabbed_before {
                        cursor_opts.grab_mode = bevy::window::CursorGrabMode::Locked;
                    }
                    return;
                }
                Btn::Nav(i) => {
                    st.screen = i;
                    st.persona = Default::default();
                }
                Btn::Equip => {
                    if let Some(c) = sel_class.clone() {
                        equip(st, class_of(&c), &mut weapons);
                    }
                }
                Btn::Use => {
                    if let Some(c) = sel_class.clone() {
                        weapons.use_request = Some(class_of(&c).to_string());
                    }
                }
                Btn::Drop => {
                    if let Some(c) = sel_class.clone() {
                        drop_item(
                            st,
                            &c,
                            &mut inv,
                            &mut weapons,
                            &mut assets,
                            &player,
                            &cam,
                            &mut drops,
                            &mut msg,
                        );
                    }
                }
                Btn::Sort => inv.0.auto_sort(),
                Btn::ChangeAmmo => {
                    if let Some(c) = sel_class.clone() {
                        load_ammo(
                            st,
                            class_of(&c),
                            None,
                            &mut inv,
                            &mut weapons,
                            &mut clips,
                            &mut assets,
                        );
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- draw
    let en: Vec<(Btn, bool)> = [Btn::Equip, Btn::Use, Btn::Drop, Btn::ChangeAmmo, Btn::Sort]
        .iter()
        .map(|b| (*b, enabled(*b, st, &mut assets, &inv.0)))
        .collect();
    let view = View {
        inv: &inv.0,
        weapons: &weapons,
        clips: &clips,
        charged: &charged,
    };
    let unequip = view
        .inv
        .items
        .iter()
        .find(|i| view.in_hand(&i.def.class))
        .map(|i| i.def.class.clone());
    let mut c = Canvas::new(SCREEN_W, SCREEN_H, s);
    let frame = FrameInfo {
        credits: view.inv.credits,
        unequip,
        enabled: en,
    };
    draw_frame(
        &mut c,
        &mut hud,
        &mut assets,
        &fonts,
        &lay,
        st,
        &hit,
        &frame,
    );
    draw_items(
        &mut c,
        &mut hud,
        &mut assets,
        &fonts,
        st,
        &view,
        &hit,
        mouse,
    );
    draw_info(
        &mut c,
        &mut hud,
        &mut assets,
        &fonts,
        st,
        &info_title,
        &rows,
        &view,
    );
    draw_details(&mut c, &mut hud, &mut assets, &fonts, st);
    draw_belt(&mut c, &mut hud, &mut assets, &fonts, st, &view, &hit);
    // Status line.
    if !st.status.is_empty() {
        c.text(
            &fonts.small,
            STATUS.0 + 2,
            STATUS.1 + 1,
            STATUS.2,
            fonts.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(9),
            &st.status,
        );
    }
    if upload(&mut images, &canvas_img, &mut st.last, &c) {
        if let Ok((_, _, m)) = nodes.get(canvas_e) {
            if let Some(m) = layers.get_mut(&m.0) {
                m.into_inner();
            }
        }
    }
    if let Ok((mut n, mut v, _)) = nodes.get_mut(canvas_e) {
        *v = Visibility::Inherited;
        n.left = Val::Px(ox as f32 / sf);
        n.top = Val::Px(oy as f32 / sf);
        n.width = Val::Px(SCREEN_W as f32 * d / sf);
        n.height = Val::Px(SCREEN_H as f32 * d / sf);
    }

    // The cursor: DeusExCursor2 (hot spot 32, 32) with its shadow, or the dragged item's icon (dimmed where it
    // can't go).
    let mut cc = Canvas::new(64, 64, s);
    let mut hot = (32, 32);
    let dragging = st.drag.as_ref().filter(|d| d.active).cloned();
    let drag_icon = dragging.as_ref().and_then(|d| match &d.src {
        DragSrc::Item(c) => view.item(c).map(|(_, it)| {
            (
                it.def.large_icon.clone().or(it.def.icon.clone()),
                it.def.large_size,
            )
        }),
        DragSrc::Belt(slot) => view.inv.belt[*slot]
            .and_then(|i| view.inv.items.get(i))
            .map(|it| (it.def.icon.clone(), (42, 37))),
    });
    if let Some((Some(icon), size)) = drag_icon {
        let valid = drag_valid(dragging.as_ref().unwrap(), &hit, mouse, &view, &mut assets);
        if let Some(t) = hud.tex(&mut assets, &icon) {
            let (w, h) = (size.0.max(1) as i32, size.1.max(1) as i32);
            cc = Canvas::new(w, h, s);
            cc.texture(
                0,
                0,
                w,
                h,
                0,
                0,
                &t,
                Style::Masked,
                if valid { WHITE } else { [64, 64, 64] },
            );
            hot = (w / 2, h / 2);
        }
    } else {
        cursor_with_shadow(&mut hud, &mut assets, &mut cc);
    }
    if upload(&mut images, &cursor_img, &mut st.last_cursor, &cc) {
        if let Ok((_, _, m)) = nodes.get(cursor_e) {
            if let Some(m) = layers.get_mut(&m.0) {
                m.into_inner();
            }
        }
    }
    if let Ok((mut n, mut v, _)) = nodes.get_mut(cursor_e) {
        *v = if window.cursor_position().is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let mx = ox as f32 + (mouse.0 - hot.0) as f32 * d;
        let my = oy as f32 + (mouse.1 - hot.1) as f32 * d;
        n.left = Val::Px(mx / sf);
        n.top = Val::Px(my / sf);
        n.width = Val::Px(cc.w as f32 * d / sf);
        n.height = Val::Px(cc.h as f32 * d / sf);
    }
}

/// Upload the screen canvas and place its node (centred, whole-number scale).
#[allow(clippy::too_many_arguments)]
fn present(
    images: &mut Assets<Image>,
    layers: &mut Assets<HudLayer>,
    nodes: &mut Query<(&mut Node, &mut Visibility, &MaterialNode<HudLayer>)>,
    st: &mut InvScreen,
    c: &Canvas,
    canvas_e: Entity,
    canvas_img: &Handle<Image>,
    ox: i32,
    oy: i32,
    d: f32,
    sf: f32,
) {
    if upload(images, canvas_img, &mut st.last, c) {
        if let Ok((_, _, m)) = nodes.get(canvas_e) {
            if let Some(m) = layers.get_mut(&m.0) {
                m.into_inner();
            }
        }
    }
    if let Ok((mut n, mut v, _)) = nodes.get_mut(canvas_e) {
        *v = Visibility::Inherited;
        n.left = Val::Px(ox as f32 / sf);
        n.top = Val::Px(oy as f32 / sf);
        n.width = Val::Px(SCREEN_W as f32 * d / sf);
        n.height = Val::Px(SCREEN_H as f32 * d / sf);
    }
}

/// Per-frame facts the frame drawing needs.
struct FrameInfo {
    credits: u32,
    /// The item in hand (its Equip button reads Unequip).
    unequip: Option<String>,
    enabled: Vec<(Btn, bool)>,
}

pub(crate) fn blank(w: u32, h: u32) -> Image {
    use bevy::{
        asset::RenderAssetUsages,
        image::ImageSampler,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
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

/// Returns true when the image was resized (its material must then be marked changed so the bind group
/// picks up the new GPU texture).
pub(crate) fn upload(
    images: &mut Assets<Image>,
    handle: &Handle<Image>,
    last: &mut Vec<u8>,
    c: &Canvas,
) -> bool {
    let bytes = c.to_rgba8();
    if *last == bytes {
        return false;
    }
    let mut resized = false;
    if let Some(mut img) = images.get_mut(handle) {
        let (w, h) = ((c.w * c.s) as u32, (c.h * c.s) as u32);
        if img.width() != w || img.height() != h {
            // (Keeping its filtering: hud::set_filter's choice.)
            let sampler = img.sampler.clone();
            *img = blank(w, h);
            img.sampler = sampler;
            resized = true;
        }
        img.data = Some(bytes.clone());
    }
    *last = bytes;
    resized
}

fn select(st: &mut InvScreen, sel: Sel, inv: &PlayerInventory) {
    if st.sel.as_ref() != Some(&sel) {
        crate::sound::play_player("DeusExSounds.UserInterface.Menu_Press", 0.25, 0.0);
        st.scroll = 0;
    }
    st.belt_sel = match &sel {
        Sel::Item(c) => inv
            .0
            .items
            .iter()
            .position(|i| &i.key() == c)
            .and_then(|i| inv.0.belt_pos(i)),
        _ => None,
    };
    st.sel = Some(sel);
}

/// Whether the dragged thing can go where the mouse is (UpdateDragMouse).
fn drag_valid(
    d: &Drag,
    hit: &Hit,
    mouse: (i32, i32),
    view: &View,
    assets: &mut RuntimeAssets,
) -> bool {
    match &d.src {
        DragSrc::Item(c) => match hit {
            Hit::Belt(slot) => *slot != 0,
            Hit::Item(t) if t == c => grid_target(c, mouse, view).is_some(),
            Hit::Item(t) => c.starts_with("WeaponMod") && weapon_for_mod(c, t, view, assets),
            Hit::Grid(..) => grid_target(c, mouse, view).is_some(),
            _ => false,
        },
        DragSrc::Belt(_) => matches!(hit, Hit::Belt(s) if *s != 0),
    }
}

fn weapon_for_mod(mod_key: &str, weapon_key: &str, view: &View, assets: &mut RuntimeAssets) -> bool {
    let Some((_, it)) = view.item(weapon_key) else {
        return false;
    };
    if it.def.kind != ItemKind::Weapon {
        return false;
    }
    let (mod_class, weapon) = (class_of(mod_key), class_of(weapon_key));
    let d = assets.classes.defaults(weapon);
    weapon_stats(weapon, &d).is_some_and(|w| mod_fits(mod_class, &w, view.weapons.mods.get(weapon)))
}

/// CalculateItemPosition: the grid cell the dragged item would go to (centred on the icon), if empty.
fn grid_target(class: &str, mouse: (i32, i32), view: &View) -> Option<(u32, u32)> {
    let (idx, it) = view.item(class)?;
    let (iw, ih) = (it.def.large_size.0 as i32, it.def.large_size.1 as i32);
    let ax = if iw > INV_BUTTON {
        iw / 2 - INV_BUTTON / 2
    } else {
        0
    };
    let ay = if iw > INV_BUTTON {
        ih / 2 - INV_BUTTON / 2
    } else {
        0
    };
    let px = mouse.0 - ITEMS.0 - ax;
    let py = mouse.1 - ITEMS.1 - ay;
    if px > INV_BUTTON * INV_COLS as i32 || py > INV_BUTTON * INV_ROWS as i32 {
        return None;
    }
    let (sx, sy) = (
        (px / INV_BUTTON).max(0) as u32,
        (py / INV_BUTTON).max(0) as u32,
    );
    view.inv
        .fits(it.def.inv_size, sx, sy, Some(idx))
        .then_some((sx, sy))
}

#[allow(clippy::too_many_arguments)]
fn finish_drag(
    st: &mut InvScreen,
    d: &Drag,
    hit: &Hit,
    mouse: (i32, i32),
    inv: &mut PlayerInventory,
    weapons: &mut Weapons,
    assets: &mut RuntimeAssets,
) {
    match &d.src {
        DragSrc::Item(c) => {
            let Some(idx) = inv.0.items.iter().position(|i| &i.key() == c) else {
                return;
            };
            match hit {
                Hit::Belt(slot) if *slot != 0 => {
                    inv.0.belt_assign(idx, *slot);
                    st.belt_sel = Some(*slot);
                }
                Hit::Item(t) if t != c && c.starts_with("WeaponMod") => {
                    let tc = class_of(t);
                    let d = assets.classes.defaults(tc);
                    let Some(w) = weapon_stats(tc, &d) else { return };
                    let stt = weapons.mods.entry(tc.to_string()).or_default();
                    if fit_mod(class_of(c), &w, stt) {
                        // WeaponMod.ApplyMod, then the mod is gone.
                        inv.0.remove(idx);
                        st.set_status(format!("{} upgraded", w.name));
                        st.sel = Some(Sel::Item(t.clone()));
                        // A drawn weapon picks up its new stats next time it's raised.
                    }
                }
                Hit::Item(_) | Hit::Grid(..) => {
                    let view = View {
                        inv: &inv.0,
                        weapons,
                        clips: &Clips::default(),
                        charged: &crate::charged::ActiveCharged::default(),
                    };
                    if let Some((x, y)) = grid_target(c, mouse, &view) {
                        inv.0.move_item(idx, x, y);
                    }
                }
                _ => {}
            }
            st.sel = st.sel.clone().or(Some(Sel::Item(c.clone())));
        }
        DragSrc::Belt(from) => match hit {
            Hit::Belt(to) if *to != 0 => {
                inv.0.belt_swap(*from, *to);
                st.belt_sel = Some(*to);
            }
            Hit::Belt(_) => {}
            _ => {
                // Dragged off the belt: ClearSlotItem.
                if *from != 0 {
                    inv.0.belt[*from] = None;
                    st.belt_sel = None;
                }
            }
        },
    }
}

/// EquipSelectedItem: PutInHand, or put it away if it's already in hand.
fn equip(st: &mut InvScreen, class: &str, weapons: &mut Weapons) {
    let in_hand = weapons
        .held
        .as_ref()
        .is_some_and(|h| h.stats.class == class && h.state != WState::Lowering)
        || weapons.pending_class.as_deref() == Some(class);
    weapons.pending = None;
    if in_hand {
        weapons.pending_class = None;
    } else {
        weapons.pending_class = Some(class.to_string());
    }
    if let Some(h) = weapons.held.as_mut() {
        if h.state != WState::Lowering {
            h.state = WState::Lowering;
            h.clip = crate::weapons::Clip::named(&h.mesh, "Down", false).unwrap_or(h.clip.clone());
            h.clip.t = 0.0;
        }
    }
    let _ = st;
}

/// PersonaScreenInventory.DropSelectedItem / DeusExPlayer.DropItem(item, bDrop=True).
#[allow(clippy::too_many_arguments)]
fn drop_item(
    st: &mut InvScreen,
    class: &str,
    inv: &mut PlayerInventory,
    weapons: &mut Weapons,
    assets: &mut RuntimeAssets,
    player: &Player,
    cam: &Transform,
    throws: &mut crate::destruct::ItemThrows,
    msg: &mut crate::movers::HudMessage,
) {
    let key = class;
    let Some(idx) = inv.0.items.iter().position(|i| i.key() == key) else {
        return;
    };
    let class = class_of(key);
    let d = assets.classes.defaults(class);
    let name = str_p(&d, "ItemName");
    // A canister keeps what it holds when it leaves the inventory.
    let extra: Vec<Property> = crate::augcan::contents(inv, key)
        .map(|augs| {
            augs.iter()
                .enumerate()
                .map(|(k, a)| Property {
                    name: "AddAugs".into(),
                    index: k as u32,
                    value: Value::Name(if a.is_empty() { "None".into() } else { a.clone() }),
                })
                .collect()
        })
        .unwrap_or_default();
    // Weapons only drop when idle; active charged pickups can't be dropped.
    let held = weapons
        .held
        .as_ref()
        .is_some_and(|h| h.stats.class == class);
    if held
        && weapons.held.as_ref().is_some_and(|h| {
            !matches!(h.state, WState::Idle | WState::Lowering | WState::Reloading)
        })
    {
        st.set_status(format!("{name} cannot be dropped here"));
        return;
    }
    // Where it leaves the hand: in front of the eyes (Location + 0.8 x CollisionRadius along the view).
    let f = cam.forward().as_vec3();
    let fw = DVec3::new(f.x as f64, f.z as f64, 0.0).normalize_or_zero();
    let at = player.body.location
        + fw * (0.8 * player.params.radius)
        + DVec3::new(0.0, 0.0, player.eye_height);
    let grenade = matches!(
        class,
        "WeaponLAM" | "WeaponGasGrenade" | "WeaponEMPGrenade" | "WeaponNanoVirusGrenade"
    );
    let mut pickup_ammo = Some(0);
    if inv.0.items[idx].def.kind == ItemKind::Weapon {
        if grenade {
            // Grenades take all their ammo with them (the TestBox hack in Weapon.DropFrom).
            if let Some(a) = class_p(&d, "AmmoName") {
                if let Some(p) = inv.0.ammo.iter().position(|x| x.0 == a) {
                    pickup_ammo = Some(inv.0.ammo[p].1 as i32);
                    inv.0.ammo.remove(p);
                }
            }
        }
        if held {
            weapons.held = None;
            weapons.pending = None;
        }
        if weapons.pending_class.as_deref() == Some(class) {
            weapons.pending_class = None;
        }
    } else {
        pickup_ammo = None;
    }
    let left_before = inv.0.items[idx].count;
    if inv.0.drop_one(idx).is_none() {
        return;
    }
    if left_before <= 1 {
        st.sel = None;
        st.belt_sel = None;
        if held {
            weapons.held = None;
        }
    }
    let mut seed = (at.x * 13.0 + at.y * 7.0) as u64 | 1;
    let mut rnd = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
    };
    let vel = DVec3::new(rnd(), rnd(), rnd()).normalize_or_zero() * 30.0;
    throws.0.push((class.to_string(), at, vel, pickup_ammo, extra));
    crate::sound::play_player("DeusExSounds.Generic.Pickup", 0.0, 0.0);
    st.set_status(format!("{name} dropped"));
    let _ = msg;
}

/// WeaponChangeAmmo / LoadAmmo: `want` None cycles to the next ammo type we have rounds of.
fn load_ammo(
    st: &mut InvScreen,
    class: &str,
    want: Option<String>,
    inv: &mut PlayerInventory,
    weapons: &mut Weapons,
    clips: &mut Clips,
    assets: &mut RuntimeAssets,
) {
    let d = assets.classes.defaults(class);
    let names = ammo_names(&d);
    if names.len() < 2 {
        return;
    }
    let cur = weapons
        .loaded_ammo
        .get(class)
        .cloned()
        .or_else(|| class_p(&d, "AmmoName"))
        .unwrap_or_default();
    let next = match want {
        Some(a) => Some(a).filter(|a| *a != cur && inv.0.ammo.iter().any(|x| &x.0 == a && x.1 > 0)),
        None => {
            let i = names.iter().position(|n| *n == cur).unwrap_or(0);
            (1..names.len())
                .map(|k| names[(i + k) % names.len()].clone())
                .find(|n| inv.0.ammo.iter().any(|x| &x.0 == n && x.1 > 0))
        }
    };
    let Some(n) = next else { return };
    // The rounds in the clip go back into their box; the weapon reloads with the new type when next fired.
    let loaded = clips.0.get(class).copied().unwrap_or(0);
    if loaded > 0 {
        match inv.0.ammo.iter_mut().find(|x| x.0 == cur) {
            Some(e) => e.1 += loaded,
            None => inv.0.ammo.push((cur.clone(), loaded)),
        }
    }
    clips.0.insert(class.to_string(), 0);
    weapons.loaded_ammo.insert(class.to_string(), n.clone());
    if let Some(h) = weapons.held.as_mut().filter(|h| h.stats.class == class) {
        h.stats.ammo_class = Some(n.clone());
    }
    let an = str_p(&assets.classes.defaults(&n), "ItemName");
    st.set_status(format!("{an} loaded"));
}

// ---------------------------------------------------------------- drawing

/// A PersonaBorderButtonWindow: left cap (4), tiled centre, right cap (8), 16 high; text in FontMenuHeaders.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_button(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    font: &UiFont,
    x: i32,
    y: i32,
    w: i32,
    text: &str,
    state: usize,
    center: bool,
) {
    let pressed = state == 2;
    let set = if pressed { "Pressed" } else { "Normal" };
    let (l, ctr, r) = (
        hud.ui(assets, &format!("PersonaActionButton{set}_Left")),
        hud.ui(assets, &format!("PersonaActionButton{set}_Center")),
        hud.ui(assets, &format!("PersonaActionButton{set}_Right")),
    );
    if let Some(t) = l {
        c.texture(x, y, 4, 16, 0, 0, &t, Style::Translucent, crate::menuui::h_col(3));
    }
    if let Some(t) = ctr {
        c.texture(
            x + 4,
            y,
            w - 4 - 8,
            16,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(3),
        );
    }
    if let Some(t) = r {
        c.texture(
            x + w - 8,
            y,
            8,
            16,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(3),
        );
    }
    let off = if pressed { 1 } else { 0 };
    let col = crate::menuui::h_btn(state);
    if center {
        c.text(
            font,
            x + off,
            y + 1 + off,
            w,
            16,
            HAlign::Center,
            VAlign::Center,
            col,
            &label(text),
        );
    } else {
        c.text(
            font,
            x + 6 + off,
            y + 1 + off,
            w,
            16,
            HAlign::Left,
            VAlign::Center,
            col,
            &label(text),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    f: &Fonts,
    lay: &Layout,
    st: &InvScreen,
    hit: &Hit,
    fi: &FrameInfo,
) {
    let enabled = |b: Btn| fi.enabled.iter().find(|e| e.0 == b).map_or(true, |e| e.1);
    // Client border (640 x 450 at y 33) and client background (585 x 361), 3 x 2 textures of 256.
    for i in 0..6 {
        let (tx, ty) = ((i % 3) as i32 * 256, (i / 3) as i32 * 256);
        if let Some(t) = hud.ui(assets, &format!("InventoryBorder_{}", i + 1)) {
            c.texture(
                tx,
                33 + ty,
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
    c.set_clip(Some((CX, CY, 585, 361)));
    for i in 0..6 {
        let (tx, ty) = ((i % 3) as i32 * 256, (i / 3) as i32 * 256);
        if let Some(t) = hud.ui(assets, &format!("InventoryBackground_{}", i + 1)) {
            c.texture(
                CX + tx,
                CY + ty,
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
    // Nav bar: background strips at (17, 6), borders over the 640 x 64 bar.
    for (i, (x, w)) in [(17, 256), (17 + 256, 256), (17 + 512, 97)]
        .iter()
        .enumerate()
    {
        if let Some(t) = hud.ui(assets, &format!("PersonaNavBarBackground_{}", i + 1)) {
            c.texture(*x, 6, *w, 21, 0, 0, &t, Style::Translucent, crate::menuui::h_col(0));
        }
    }
    for (i, (x, w)) in [(0, 256), (256, 256), (512, 175)].iter().enumerate() {
        if let Some(t) = hud.ui(assets, &format!("PersonaNavBarBorder_{}", i + 1)) {
            c.texture(*x, 0, *w, 64, 0, 0, &t, Style::Translucent, crate::menuui::h_col(1));
        }
    }
    let state = |b: Btn, en: bool| -> usize {
        if !en {
            3
        } else if st.pressed == Some(b) && *hit == Hit::Button(b) {
            2
        } else if *hit == Hit::Button(b) {
            1
        } else {
            0
        }
    };
    for (i, (x, w)) in lay.nav.iter().enumerate() {
        // The current screen's button is insensitive (btnInventory.SetSensitivity(False)).
        let s = if i == 0 { 3 } else { state(Btn::Nav(i), true) };
        draw_button(
            c,
            hud,
            assets,
            &f.headers,
            *x,
            8,
            *w,
            NAV_LABELS[i],
            s,
            true,
        );
    }
    draw_button(
        c,
        hud,
        assets,
        &f.headers,
        lay.exit.0,
        8,
        lay.exit.1,
        "E|&xit",
        state(Btn::Exit, true),
        true,
    );
    // Title and credits.
    c.text(
        &f.headers,
        CX + 9,
        CY + 5,
        200,
        f.headers.height,
        HAlign::Left,
        VAlign::Top,
        crate::menuui::h_col(2),
        "Inventory",
    );
    let lw = f.headers.width("Credits");
    c.text(
        &f.headers,
        CX + 165,
        CY + 3,
        lw,
        15,
        HAlign::Left,
        VAlign::Center,
        crate::menuui::h_col(7),
        "Credits",
    );
    let credits = fi.credits;
    c.text(
        &f.headers,
        CX + 165 + lw + 5,
        CY + 3,
        63,
        15,
        HAlign::Right,
        VAlign::Center,
        crate::menuui::h_col(7),
        &credits.to_string(),
    );
    // Action buttons and the filler after them.
    let labels = [
        (Btn::Equip, "|&Equip"),
        (Btn::Use, "|&Use"),
        (Btn::Drop, "|&Drop"),
        (Btn::ChangeAmmo, "Change Amm|&o"),
        (Btn::Sort, "|&Sort"),
    ];
    let mut right = BUTTONS.0;
    for (b, x, w) in &lay.actions {
        let mut text = labels.iter().find(|l| l.0 == *b).map(|l| l.1).unwrap_or("");
        if *b == Btn::Equip {
            if let Some(Sel::Item(cl)) = &st.sel {
                if fi.unequip.as_deref() == Some(class_of(cl)) {
                    text = "Un|&equip";
                }
            }
        }
        let en = enabled(*b);
        draw_button(
            c,
            hud,
            assets,
            &f.headers,
            *x,
            BUTTONS.1,
            *w,
            text,
            state(*b, en),
            false,
        );
        right = right.max(x + w);
    }
    if right < BUTTONS.0 + BUTTONS.2 {
        if let Some(t) = hud.ui(assets, "PersonaButtonFiller") {
            c.texture(
                right,
                BUTTONS.1,
                BUTTONS.0 + BUTTONS.2 - right,
                16,
                0,
                0,
                &t,
                Style::Translucent,
                crate::menuui::h_col(0),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_items(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    f: &Fonts,
    st: &InvScreen,
    view: &View,
    hit: &Hit,
    mouse: (i32, i32),
) {
    let dragging = st.drag.as_ref().filter(|d| d.active);
    let drag_class = dragging.and_then(|d| match &d.src {
        DragSrc::Item(c) => Some(c.clone()),
        _ => None,
    });
    // A selected weapon mod lights up the weapons it fits (HighlightModWeapons).
    let sel_mod = match &st.sel {
        Some(Sel::Item(c)) if c.starts_with("WeaponMod") && drag_class.is_none() => Some(c.clone()),
        _ => None,
    };
    for (idx, it) in view.inv.items.iter().enumerate() {
        let Some(mut r) = item_rect(it) else { continue };
        let class = &it.def.class;
        let key = it.key();
        let is_drag = drag_class.as_deref() == Some(key.as_str());
        let mut fill: Option<Rgb> = None;
        if is_drag {
            // The button follows the mouse over valid empty cells (SetItemButtonPos), showing its drop fill.
            if let Some((x, y)) = grid_target(&key, mouse, view) {
                r.0 = ITEMS.0 + x as i32 * INV_BUTTON;
                r.1 = ITEMS.1 + y as i32 * INV_BUTTON;
                fill = Some(COL_MOD_TRUE);
            } else if matches!(hit, Hit::Belt(_)) {
                fill = None;
            } else {
                fill = Some(COL_MOD_FALSE);
            }
        } else if st.sel == Some(Sel::Item(key.clone())) {
            fill = Some(COL_FILL_SELECTED);
        }
        if let Some(m) = &sel_mod {
            if it.def.kind == ItemKind::Weapon && weapon_for_mod(m, &key, view, assets) {
                fill = Some(COL_MOD_TRUE);
            }
        }
        if let (Some(dc), Hit::Item(h)) = (&drag_class, hit) {
            if *h == key && *dc != key {
                let good = dc.starts_with("WeaponMod") && weapon_for_mod(dc, &key, view, assets);
                fill = Some(if good { COL_MOD_TRUE } else { COL_MOD_FALSE });
            }
        }
        let (x, y, w, h) = r;
        if let Some(col) = fill {
            c.fill(x + 1, y + 1, w - 2, h - 2, Style::Translucent, col);
        }
        if is_drag {
            continue;
        }
        let (icon, (iw, ih)) = match &it.def.large_icon {
            Some(p) => (
                Some(p.clone()),
                (it.def.large_size.0 as i32, it.def.large_size.1 as i32),
            ),
            None => (it.def.icon.clone(), (40, 35)),
        };
        if let Some(t) = icon.and_then(|p| hud.tex(assets, &p)) {
            c.texture(
                x + w / 2 - iw / 2,
                y + h / 2 - ih / 2,
                iw,
                ih,
                0,
                0,
                &t,
                Style::Masked,
                WHITE,
            );
        }
        if let Some(b) = view.inv.belt_pos(idx) {
            let s = b.to_string();
            let sw = f.small_ds.width(&s);
            c.text(
                &f.small_ds,
                x + w - sw - 3,
                y + 3,
                sw,
                f.small_ds.height,
                HAlign::Right,
                VAlign::Center,
                crate::menuui::h_col(7),
                &s,
            );
        }
        let d = assets.classes.defaults(class);
        let mut s = String::new();
        if it.def.kind == ItemKind::Weapon {
            let h2h = bool_p(&d, "bHandToHand");
            let ammo = view.loaded_ammo(class, &d);
            match (h2h, ammo) {
                (true, Some(a)) => {
                    let n = view.ammo_amount(&a, assets);
                    s = if n == 1 {
                        format!("{n} Rd")
                    } else {
                        format!("{n} Rds")
                    };
                }
                (false, Some(a)) => s = str_p(&assets.classes.defaults(&a), "beltDescription"),
                _ => {}
            }
        } else if matches!(it.def.kind, ItemKind::Pickup { .. }) && it.count > 1 {
            s = format!("Count: {}", it.count);
        }
        if !s.is_empty() {
            c.text(
                &f.small_ds,
                x,
                y + h - f.small_ds.height,
                w,
                f.small_ds.height,
                HAlign::Center,
                VAlign::Center,
                crate::menuui::h_col(7),
                &s,
            );
        }
        if st.sel == Some(Sel::Item(key.clone())) {
            let borders = item_borders(hud, assets);
            c.borders(x, y, w, h, &borders, Style::Masked, WHITE);
        }
    }
}

pub(crate) fn item_borders(
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
) -> [Option<Arc<UiTex>>; 9] {
    [
        "TL", "TR", "BL", "BR", "Left", "Right", "Top", "Bottom", "Center",
    ]
    .map(|n| hud.ui(assets, &format!("PersonaItemHighlight_{n}")))
}

#[allow(clippy::too_many_arguments)]
fn draw_info(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    f: &Fonts,
    st: &InvScreen,
    title: &str,
    rows: &[(i32, i32, Row)],
    view: &View,
) {
    let (x0, y0, w, h) = INFO;
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
    c.set_clip(Some((x0, y0 + INFO_TEXT_Y, w, h - INFO_TEXT_Y)));
    let x = x0 + 4;
    let width = w - 8;
    for (ry, rh, row) in rows {
        let y = y0 + INFO_TEXT_Y + ry - st.scroll;
        if y + rh < y0 + INFO_TEXT_Y || y > y0 + h {
            continue;
        }
        match row {
            Row::Text(t) => {
                for (i, l) in wrap(&f.small, t, width).iter().enumerate() {
                    c.text(
                        &f.small,
                        x,
                        y + i as i32 * f.small.height,
                        width,
                        f.small.height,
                        HAlign::Left,
                        VAlign::Top,
                        crate::menuui::h_col(8),
                        l,
                    );
                }
            }
            Row::Line => c.fill(x, y + 2, width, 1, Style::Normal, crate::menuui::h_col(8)),
            Row::Item(lab, v, hl) => {
                c.text(
                    &f.small,
                    x,
                    y,
                    70,
                    f.small.height,
                    HAlign::Right,
                    VAlign::Top,
                    crate::menuui::h_col(8),
                    lab,
                );
                let font = if *hl { &f.headers } else { &f.small };
                for (i, l) in wrap(font, v, width - 80).iter().enumerate() {
                    c.text(
                        font,
                        x + 80,
                        y + i as i32 * font.height,
                        width - 80,
                        font.height,
                        HAlign::Left,
                        VAlign::Top,
                        if *hl { crate::menuui::h_col(7) } else { crate::menuui::h_col(8) },
                        l,
                    );
                }
            }
            Row::Ammo(btns) => {
                c.text(
                    &f.small,
                    x,
                    y,
                    70,
                    54,
                    HAlign::Right,
                    VAlign::Center,
                    crate::menuui::h_col(8),
                    "Ammo:",
                );
                let borders = item_borders(hud, assets);
                for (i, (a, has, rounds, loaded)) in btns.iter().enumerate() {
                    let bx = x + 74 + i as i32 * 58;
                    if *loaded {
                        c.fill(bx + 1, y + 1, 52, 52, Style::Translucent, COL_FILL_SELECTED);
                    }
                    let d = assets.classes.defaults(a);
                    if let Some(t) = class_p_full(&d, "Icon").and_then(|p| hud.tex(assets, &p)) {
                        let col = if *has { WHITE } else { [85, 85, 85] };
                        c.texture(
                            bx + 27 - 21,
                            y + 27 - 18,
                            42,
                            37,
                            0,
                            0,
                            &t,
                            Style::Masked,
                            col,
                        );
                    }
                    c.borders(
                        bx,
                        y,
                        54,
                        54,
                        &borders,
                        Style::Masked,
                        if *loaded { WHITE } else { [128, 128, 128] },
                    );
                    let desc = str_p(&d, "beltDescription");
                    let tcol = if *has { crate::menuui::h_col(7) } else { [128, 128, 128] };
                    let th = f.tiny.height;
                    if *has && *rounds > 0 {
                        let rs = if *rounds == 1 {
                            format!("{rounds} Round")
                        } else {
                            format!("{rounds} Rounds")
                        };
                        c.text(
                            &f.tiny,
                            bx,
                            y + 54 - th - th + 2,
                            54,
                            th,
                            HAlign::Center,
                            VAlign::Top,
                            tcol,
                            &desc,
                        );
                        c.text(
                            &f.tiny,
                            bx,
                            y + 54 - th,
                            54,
                            th,
                            HAlign::Center,
                            VAlign::Top,
                            tcol,
                            &rs,
                        );
                    } else {
                        c.text(
                            &f.tiny,
                            bx,
                            y + 54 - th,
                            54,
                            th,
                            HAlign::Center,
                            VAlign::Top,
                            tcol,
                            &desc,
                        );
                    }
                }
            }
            Row::AmmoEntry(a, t) => {
                let d = assets.classes.defaults(a);
                if let Some(tx) = class_p_full(&d, "Icon").and_then(|p| hud.tex(assets, &p)) {
                    c.texture(x, y, 42, 37, 0, 0, &tx, Style::Masked, WHITE);
                }
                for (i, l) in wrap(&f.small, t, width - 46).iter().enumerate() {
                    c.text(
                        &f.small,
                        x + 46,
                        y + i as i32 * f.small.height,
                        width - 46,
                        f.small.height,
                        HAlign::Left,
                        VAlign::Top,
                        crate::menuui::h_col(8),
                        l,
                    );
                }
            }
            Row::Checkbox(text, on) => {
                if let Some(t) = hud.ui(
                    assets,
                    if *on {
                        "PersonaCheckBox_On"
                    } else {
                        "PersonaCheckBox_Off"
                    },
                ) {
                    c.texture(
                        x,
                        y + 1,
                        12,
                        12,
                        0,
                        0,
                        &t,
                        Style::Masked,
                        crate::menuui::h_btn(0),
                    );
                }
                c.text(
                    &f.small,
                    x + 18,
                    y,
                    width - 18,
                    14,
                    HAlign::Left,
                    VAlign::Center,
                    crate::menuui::h_btn(0),
                    text,
                );
            }
        }
    }
    c.set_clip(None);
    let _ = view;
}

pub(crate) fn class_p_full(p: &[Property], n: &str) -> Option<String> {
    match find_ci(p, n) {
        Some(Value::Path(s)) if !s.ends_with(".None") => Some(s.clone()),
        _ => None,
    }
}

/// The key ring and ammo PersonaItemDetailWindows.
fn draw_details(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    f: &Fonts,
    st: &InvScreen,
) {
    let borders = item_borders(hud, assets);
    for (x, y, w, class, text, count, selected) in [
        (
            KEYRING.0,
            KEYRING.1,
            KEYRING.2,
            "NanoKeyRing",
            "Click icon to see a list of Nano Keys.",
            Some(format!("Keys: {}", crate::keys::list().len())),
            st.sel == Some(Sel::KeyRing),
        ),
        (
            AMMO.0,
            AMMO.1,
            AMMO.2,
            "AmmoShell",
            "Click icon to see a list of Ammo.",
            None,
            st.sel == Some(Sel::Ammo),
        ),
    ] {
        let d = assets.classes.defaults(class);
        let (iw, ih) = (
            int_p(&d, "largeIconWidth", 42),
            int_p(&d, "largeIconHeight", 37),
        );
        if let Some(t) = class_p_full(&d, "largeIcon").and_then(|p| hud.tex(assets, &p)) {
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
        if selected {
            c.borders(x, y, 55, 55, &borders, Style::Masked, WHITE);
        }
        if let Some(cs) = count {
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
        }
        let tw = w - 55 - 3 - 4;
        let lines = wrap(&f.small, text, tw);
        let th = lines.len() as i32 * f.small.height;
        let ty = y + 2 + (DETAIL_H - 4 - th) / 2;
        for (i, l) in lines.iter().enumerate() {
            c.text(
                &f.small,
                x + 56 + 2,
                ty + i as i32 * f.small.height,
                tw,
                f.small.height,
                HAlign::Center,
                VAlign::Top,
                crate::menuui::h_col(8),
                l,
            );
        }
    }
}

/// The screen's own object belt (an interactive HUDObjectBelt).
fn draw_belt(
    c: &mut Canvas,
    hud: &mut ClassicHud,
    assets: &mut RuntimeAssets,
    f: &Fonts,
    st: &InvScreen,
    view: &View,
    hit: &Hit,
) {
    let (bx, by) = BELT;
    if let Some(t) = hud.ui(assets, "HUDObjectBeltBackground_Left") {
        c.texture(
            bx + 2,
            by + 6,
            9,
            54,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(0),
        );
    }
    if let Some(t) = hud.ui(assets, "HUDObjectBeltBackground_Right") {
        c.texture(
            bx + 514,
            by + 6,
            8,
            54,
            0,
            0,
            &t,
            Style::Translucent,
            crate::menuui::h_col(0),
        );
    }
    for (name, x, w) in [
        ("HUDObjectBeltBorder_1", 0, 256),
        ("HUDObjectBeltBorder_2", 256, 256),
        ("HUDObjectBeltBorder_3", 512, 29),
    ] {
        if let Some(t) = hud.ui(assets, name) {
            c.texture(bx + x, by, w, 69, 0, 0, &t, Style::Translucent, crate::menuui::h_col(1));
        }
    }
    let cell = hud.ui(assets, "HUDObjectBeltBackground_Cell");
    let borders = item_borders(hud, assets);
    let dragging = st.drag.as_ref().filter(|d| d.active);
    let tiny = &f.tiny;
    for slot in 0..10 {
        let (sx, sy, sw, sh) = belt_slot_rect(slot);
        if let Some(t) = &cell {
            c.texture(sx, sy, sw, sh, 0, 0, t, Style::Translucent, crate::menuui::h_col(0));
        }
        let item = view.inv.belt[slot].and_then(|i| view.inv.items.get(i));
        // Drop fill while dragging over it (SetDropFill), selected fill when toggled.
        let over = dragging.is_some() && *hit == Hit::Belt(slot);
        if over {
            c.fill(
                sx + 1,
                sy + 3,
                42,
                37,
                Style::Translucent,
                if slot != 0 {
                    COL_MOD_TRUE
                } else {
                    COL_MOD_FALSE
                },
            );
        } else if st.belt_sel == Some(slot) && item.is_some() {
            c.fill(
                sx + 1,
                sy + 3,
                42,
                37,
                Style::Translucent,
                COL_FILL_SELECTED,
            );
        }
        let is_drag_src = dragging.is_some_and(|d| d.src == DragSrc::Belt(slot));
        let (desc, icon, text) = match item {
            Some(it) => {
                let mut text = String::new();
                if matches!(it.def.kind, ItemKind::Pickup { .. }) && it.count > 1 {
                    let lab = str_p(&assets.classes.defaults(&it.def.class), "CountLabel");
                    text = format!(
                        "{} {}",
                        if lab.is_empty() { "COUNT:".into() } else { lab },
                        it.count
                    );
                } else if it.def.kind == ItemKind::Weapon {
                    let d = assets.classes.defaults(&it.def.class);
                    if !bool_p(&d, "bHandToHand") && int_p(&d, "ReloadCount", 0) != 0 {
                        if let Some(a) = view.loaded_ammo(&it.def.class, &d) {
                            text = str_p(&assets.classes.defaults(&a), "beltDescription");
                        }
                    }
                }
                (it.def.belt_name.clone(), it.def.icon.clone(), text)
            }
            None if slot == 0 => {
                let d = assets.classes.defaults("NanoKeyRing");
                (
                    str_p(&d, "beltDescription"),
                    class_p_full(&d, "Icon"),
                    String::new(),
                )
            }
            None => (String::new(), None, String::new()),
        };
        if !is_drag_src {
            if let Some(t) = icon.and_then(|p| hud.tex(assets, &p)) {
                c.texture(sx + 1, sy + 3, 42, 37, 0, 0, &t, Style::Masked, WHITE);
                c.text(
                    tiny,
                    sx + 1,
                    sy + 42,
                    42,
                    7,
                    HAlign::Center,
                    VAlign::Center,
                    crate::menuui::h_col(8),
                    &desc,
                );
                if !text.is_empty() {
                    c.text(
                        tiny,
                        sx + 1,
                        sy + 32,
                        42,
                        8,
                        HAlign::Center,
                        VAlign::Center,
                        crate::menuui::h_col(8),
                        &text,
                    );
                }
                if st.belt_sel == Some(slot) {
                    c.borders(sx, sy + 2, 44, 50, &borders, Style::Masked, WHITE);
                }
            }
        }
        c.text(
            tiny,
            sx + 37,
            sy + 3,
            6,
            7,
            HAlign::Right,
            VAlign::Center,
            crate::menuui::h_col(8),
            &slot.to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_format_matches_script() {
        assert_eq!(fmt_float(1.0 / 0.3, 0.1), "3.3");
        assert_eq!(fmt_float(2.0, 0.1), "2.0");
        assert_eq!(fmt_float(150.0, 1.0), "150");
        assert_eq!(pct(-0.1), "(-10%)");
    }

    /// INV_PREVIEW=<out.rgba> HUD_GAME=<root>: the inventory screen at 2x (1280 x 960 raw RGBA over grey).
    #[test]
    fn preview_inventory() {
        let (Ok(out), Ok(root)) = (std::env::var("INV_PREVIEW"), std::env::var("HUD_GAME")) else {
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
        let mut inv = PlayerInventory::default();
        for (c, n) in [
            ("WeaponPistol", 1),
            ("WeaponProd", 1),
            ("MedKit", 3),
            ("WeaponGEPGun", 1),
            ("Lockpick", 2),
            ("WeaponModAccuracy", 1),
            ("WeaponCombatKnife", 1),
        ] {
            let d = assets.classes.defaults(c);
            let def = crate::pickups::item_def(c, &d);
            inv.0.pick_up(&def, n);
        }
        inv.0.ammo.push(("Ammo10mm".into(), 14));
        inv.0.ammo.push(("AmmoRocket".into(), 4));
        inv.0.ammo.push(("AmmoBattery".into(), 4));
        inv.0.credits = 180;
        let weapons = Weapons::default();
        let clips = Clips::default();
        let charged = crate::charged::ActiveCharged::default();
        let get = |hud: &mut ClassicHud, a: &mut RuntimeAssets, n: &str| hud.font(a, n).unwrap();
        let fonts = Fonts {
            headers: get(&mut hud, &mut assets, "FontMenuHeaders"),
            small: get(&mut hud, &mut assets, "FontMenuSmall"),
            small_ds: get(&mut hud, &mut assets, "FontMenuSmall_DS"),
            tiny: get(&mut hud, &mut assets, "FontTiny"),
        };
        let lay = layout(&fonts);
        let mut st = InvScreen::default();
        st.sel = Some(Sel::Item("WeaponPistol".into()));
        st.belt_sel = Some(1);
        st.status = "Pistol dropped".into();
        let view = View {
            inv: &inv.0,
            weapons: &weapons,
            clips: &clips,
            charged: &charged,
        };
        let (title, info) = item_info("WeaponPistol", &view, &mut assets);
        let rows = layout_rows(&info, &fonts);
        let s = 2;
        let mut c = Canvas::new(SCREEN_W, SCREEN_H, s);
        let fi = FrameInfo {
            credits: 180,
            unequip: None,
            enabled: vec![
                (Btn::Equip, true),
                (Btn::Use, false),
                (Btn::Drop, true),
                (Btn::ChangeAmmo, false),
            ],
        };
        draw_frame(
            &mut c,
            &mut hud,
            &mut assets,
            &fonts,
            &lay,
            &st,
            &Hit::None,
            &fi,
        );
        draw_items(
            &mut c,
            &mut hud,
            &mut assets,
            &fonts,
            &st,
            &view,
            &Hit::None,
            (0, 0),
        );
        draw_info(
            &mut c,
            &mut hud,
            &mut assets,
            &fonts,
            &st,
            &title,
            &rows,
            &view,
        );
        draw_details(&mut c, &mut hud, &mut assets, &fonts, &st);
        draw_belt(
            &mut c,
            &mut hud,
            &mut assets,
            &fonts,
            &st,
            &view,
            &Hit::None,
        );
        c.text(
            &fonts.small,
            STATUS.0 + 2,
            STATUS.1 + 1,
            STATUS.2,
            fonts.small.height,
            HAlign::Left,
            VAlign::Top,
            crate::menuui::h_col(9),
            &st.status,
        );
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
        std::fs::write(out, bytes).unwrap();
    }
}

/// The menu cursor (DeusExCursor2, hot spot 32, 32) on a 64 x 64 canvas, over a soft shadow cast from its own shape
/// (down and right, blurred) instead of the flat DeusExCursor2_Shadow image.
pub fn cursor_with_shadow(hud: &mut ClassicHud, assets: &mut RuntimeAssets, cc: &mut Canvas) {
    if let Some(t) = hud.ui(assets, "DeusExCursor2") {
        cc.drop_shadow(0, 0, &t, (2.0, 2.5), 2.5, 0.6);
        cc.texture(0, 0, 64, 64, 0, 0, &t, Style::Masked, crate::menuui::h_col(13));
    }
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    SCREEN_OPEN.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = OPEN_BOT.lock() {
        *v = None;
    }
}
