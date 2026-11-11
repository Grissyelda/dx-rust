//! The Classic deathmatch's own HUD pieces, as the original patch draws them:
//! * AugmentationDisplayWindow.DrawMiscStatusMessages: the centred notices at a third of the screen in
//!   FontMenuTitle, fading over mpMsgDelay ("You hit your teammate!", "You have been poisoned!", "Only 4 more
//!   kills, and X will win the match!", "You're on Team Unatco!" ...). Words from the class's own defaults.
//! * MultiplayerMessageWin: the death screen ("You have been killed by X" + how, in FontMenuExtraLarge red; after
//!   3 s "Press <Fire> to restart." and "Press <key> for details."; the killer profile: the final blow, the killer's
//!   health, energy and streak, his weapon / skill / augmentations against yours).
//! * HUDMultiSkills: the skill list bought with the number keys (BuySkills toggles it), and the flashing "Skills
//!   available!" when the points cover a level.
//! * NintendoImmunityEffect: 6 s of protection after a (re)spawn.
//! The Classic scoreboard and the win screen (DeusExMPGame.ShowDMScoreboard / TeamDMGame.ShowTeamDMScoreboard /
//! PlayerWinScreen / TeamWinScreen) are `draw_board`, used by scoreboard.rs.

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::deathmatch::{self, Board, ScoreRow};
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::math::DVec3;
use bevy::prelude::*;
use std::sync::Mutex;

/// MPMSG_* notices.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    TeamUnatco,
    TeamNsf,
    TeamHit,
    FirstPoison,
    FirstBurn,
    LostLegs,
    KilledTeammate,
    /// MPMSG_CloseKills: kills to go, who will win ("Tied" in a team game).
    CloseKills(u32, String),
    /// MPMSG_TimeNearEnd: minutes, who leads.
    TimeNearEnd(u32, String),
}

/// DeusExPlayer.mpMsgDelay.
const MSG_DELAY: f32 = 5.0;
/// MultiplayerMessageWin: seconds before Fire restarts.
const DEATH_LOCKOUT: f32 = 3.0;
/// HUDMultiSkills.TimeDelay: seconds the "Skills available!" notice flashes.
const SKILL_NOTIFY: f32 = 10.0;

#[derive(Clone, Debug, Default)]
struct Profile {
    killer: String,
    weapon: String,
    augs: Vec<String>,
    health: u8,
    streak: u32,
    damage: i32,
    region: String,
    my_weapon: String,
    my_augs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct Death {
    killer: Option<String>,
    method: String,
    self_kill: bool,
    t: f32,
    profile: Option<Profile>,
}

#[derive(Default)]
struct State {
    msg: Option<(Msg, f32)>,
    /// mpMsgServerFlags: first poison / first burn said once a map.
    said: Vec<&'static str>,
    shield: f32,
    skills_open: bool,
    notify_points: i32,
    notify_left: f32,
    notified: bool,
    death: Option<Death>,
    show_profile: bool,
    last_hit: Option<(Option<u8>, String, f32, DVec3)>,
    team: Option<u8>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn on() -> bool {
    deathmatch::mp_body()
}

/// MultiplayerNotifyMsg / ServerConditionalNotifyMsg (the first poison and burn once a map).
pub fn notify(m: Msg) {
    if !on() {
        return;
    }
    with(|s| {
        let once = match m {
            Msg::FirstPoison => Some("poison"),
            Msg::FirstBurn => Some("burn"),
            _ => None,
        };
        if let Some(k) = once {
            if s.said.contains(&k) {
                return;
            }
            s.said.push(k);
        }
        s.msg = Some((m, MSG_DELAY));
    });
}

const NOTE: &str = "\u{1}mp:";

/// A notice the host sends everyone as a note: "close" (kills to go) or "time" (minutes left), and who leads.
pub fn note_code(kind: &str, n: u32, who: &str) -> String {
    format!("{NOTE}{kind}:{n}:{who}")
}

/// A note that is one of ours: shown as the notice (true), else the caller shows it as text.
pub fn from_note(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(NOTE) else { return false };
    let mut it = rest.splitn(3, ':');
    let (Some(kind), Some(n), Some(who)) = (it.next(), it.next(), it.next()) else { return true };
    let n = n.parse().unwrap_or(1);
    match kind {
        "close" => notify(Msg::CloseKills(n, who.to_string())),
        "time" => notify(Msg::TimeNearEnd(n, who.to_string())),
        _ => {}
    }
    true
}

/// NintendoImmunityEffect: hits do nothing for a while after a spawn.
pub fn shielded() -> bool {
    on() && with(|s| s.shield > 0.0)
}

pub fn start_shield(secs: f32) {
    with(|s| s.shield = secs);
}

/// A hit this player took (for the killer profile's final blow).
pub fn note_hit_taken(by: Option<u8>, weapon: &str, _kind: &str, dmg: f32, off: DVec3) {
    if !on() {
        return;
    }
    with(|s| s.last_hit = Some((by, weapon.to_string(), dmg, off)));
}

/// This player's shot hit a teammate (DeusExPlayer(instigatedBy).MultiplayerNotifyMsg(MPMSG_TeamHit)).
pub fn hit_teammate() {
    notify(Msg::TeamHit);
}

/// The skill list is open (the number keys buy skills instead of picking belt items).
pub fn skills_open() -> bool {
    on() && with(|s| s.skills_open)
}

/// The death screen is up (Fire restarts instead of shooting).
pub fn death_screen() -> bool {
    on() && with(|s| s.death.is_some())
}

/// This player died in a Classic match: the death screen. `killer`: (name, weapon, streak, health %, augs).
pub fn died(killer: Option<(String, String, u32, u8, Vec<String>)>, self_kill: bool, method: &str) {
    if !on() {
        return;
    }
    let my_weapon = crate::netplay::my_weapon();
    let my_augs: Vec<String> = crate::abilities::lock()
        .as_ref()
        .map(|a| a.augs.list.iter().filter(|x| x.active).map(|x| x.def.name.clone()).collect())
        .unwrap_or_default();
    with(|s| {
        let profile = killer.as_ref().filter(|_| !self_kill).map(|(name, weapon, streak, health, augs)| {
            let (damage, region) = match &s.last_hit {
                Some((_, _, d, off)) => (d.round() as i32, region_name(*off)),
                None => (0, "Torso".into()),
            };
            Profile {
                killer: name.clone(),
                weapon: weapon_name(weapon),
                augs: augs.iter().map(|a| word(a, "AugmentationName", a)).collect(),
                health: *health,
                streak: *streak,
                damage,
                region,
                my_weapon: weapon_name(&my_weapon),
                my_augs: my_augs.clone(),
            }
        });
        s.death = Some(Death { killer: killer.map(|k| k.0), method: method.to_string(), self_kill, t: 0.0, profile });
        s.show_profile = false;
        s.skills_open = false;
    });
}

fn region_name(off: DVec3) -> String {
    let h = 47.5;
    if off.z > h * 0.78 {
        "Head".into()
    } else if off.z < 0.0 {
        "Legs".into()
    } else {
        "Torso".into()
    }
}

fn weapon_name(class: &str) -> String {
    if class.is_empty() {
        return "None".into();
    }
    match crate::classicmp::class_value(class, "ItemName") {
        Some(unreal_pkg::props::Value::Str(s)) if !s.is_empty() => s,
        _ => class.to_string(),
    }
}

/// A new map: the notices said once a map can be said again.
pub fn reset_level() {
    with(|s| *s = State::default());
}

/// The words of a class's localized string property, else the given text.
fn word(class: &str, name: &str, dflt: &str) -> String {
    static CACHE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let key = format!("{class}.{name}");
    if let Some(v) = CACHE.lock().ok().and_then(|c| c.iter().find(|x| x.0 == key).map(|x| x.1.clone())) {
        return v;
    }
    let v = match crate::classicmp::class_value(class, name) {
        Some(unreal_pkg::props::Value::Str(s)) if !s.is_empty() => s,
        _ => dflt.to_string(),
    };
    if let Ok(mut c) = CACHE.lock() {
        c.push((key, v.clone()));
    }
    v
}

fn msg_text(m: &Msg) -> (String, [u8; 3]) {
    const ADW: &str = "AugmentationDisplayWindow";
    let green = [0, 255, 0];
    let red = [255, 0, 0];
    match m {
        Msg::TeamUnatco => (word(ADW, "msgTeamUnatco", "You're on Team Unatco!"), green),
        Msg::TeamNsf => (word(ADW, "msgTeamNsf", "You're on Team NSF!"), green),
        Msg::TeamHit => (word(ADW, "msgTeammateHit", "You hit your teammate!"), red),
        Msg::FirstPoison => (word(ADW, "YouArePoisonedString", "You have been poisoned!"), green),
        Msg::FirstBurn => (word(ADW, "YouAreBurnedString", "You are burning!"), green),
        Msg::LostLegs => (word(ADW, "LostLegsString", "You've lost your legs!"), green),
        Msg::KilledTeammate => (word(ADW, "YouKilledTeammateString", "You killed a teammate!"), red),
        Msg::CloseKills(n, who) => {
            let mut s = word(ADW, "OnlyString", "Only ") + &n.to_string();
            s += &if *n > 1 { word(ADW, "KillsToGoString", " more kills, and ") } else { word(ADW, "KillToGoString", " more kill, and ") };
            if who.eq_ignore_ascii_case("Tied") {
                s += &word(ADW, "TiedMatchString", "it's a tied match!");
            } else {
                s += who;
                s += &word(ADW, "WillWinMatchString", " will win the match!");
            }
            (s, green)
        }
        Msg::TimeNearEnd(n, who) => {
            let mut s = if *n > 1 {
                word(ADW, "LessThanXString1", "Less than ") + &n.to_string() + &word(ADW, "LessThanXString2", " minutes to go, and ")
            } else {
                word(ADW, "LessThanMinuteString", "Less than a minute to go, and ")
            };
            if who.eq_ignore_ascii_case("Tied") {
                s += &word(ADW, "TiedMatchString", "it's a tied match!");
            } else {
                s += who;
                s += &word(ADW, "LeadsMatchString", " leads the match!");
            }
            (s, green)
        }
    }
}

#[derive(Component, Default)]
pub struct MsgNode;
#[derive(Component, Default)]
pub struct DeathNode;
#[derive(Component, Default)]
pub struct SkillNode;

fn screen(win: &Window) -> (i32, f32, i32, i32) {
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    let w = (win.physical_width() as i32 / s).max(1);
    let h = (win.physical_height() as i32 / s).max(1);
    (s, k, w, h)
}

/// Timers, the keys (Fire after death, the skill menu and its number keys, the details key) and the three nodes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn mphud_system(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    ph: Res<crate::ai::PlayerHealth>,
    mut nodes: ParamSet<(
        Query<(Entity, &mut Node, &mut Visibility), With<MsgNode>>,
        Query<(Entity, &mut Node, &mut Visibility), With<DeathNode>>,
        Query<(Entity, &mut Node, &mut Visibility), With<SkillNode>>,
    )>,
    mut state: Local<[(Option<Handle<Image>>, u64); 3]>,
) {
    let Some(win) = windows.iter().next() else { return };
    let live = on() && !crate::loader::busy();
    let dt = time.delta_secs();
    let ui = crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed) || crate::menus::open() || crate::netplay::chat_open();
    // A match without a session (a test on one PC): netplay isn't there to notice the death.
    if live && ph.dead && !crate::netplay::active() && !death_screen() {
        died(None, false, "");
    }
    let team = if live { deathmatch::team_of(deathmatch::my_id()) } else { None };
    // Timers and keys.
    let (msg, death, skills) = with(|s| {
        if team != s.team {
            // MPMSG_TeamUnatco / MPMSG_TeamNsf on joining a team.
            s.team = team;
            if let Some(t) = team {
                s.msg = Some((if t == deathmatch::TEAM_NSF { Msg::TeamNsf } else { Msg::TeamUnatco }, MSG_DELAY));
            }
        }
        if !live {
            s.msg = None;
            s.death = None;
            s.skills_open = false;
            return (None, None, None);
        }
        s.shield = (s.shield - dt).max(0.0);
        if let Some((_, t)) = s.msg.as_mut() {
            *t -= dt;
        }
        if s.msg.as_ref().is_some_and(|m| m.1 <= 0.0) {
            s.msg = None;
        }
        if let Some(d) = s.death.as_mut() {
            d.t += dt;
            if !ph.dead {
                // Got up (the match restarted him another way).
                s.death = None;
            }
        }
        if !ui {
            if let Some(d) = s.death.as_ref() {
                if d.t >= DEATH_LOCKOUT {
                    if buttons.just_pressed(MouseButton::Left) || buttons.just_pressed(MouseButton::Right) {
                        s.death = None;
                        deathmatch::request_respawn();
                    } else if keys.just_pressed(KeyCode::KeyL) {
                        s.show_profile = !s.show_profile;
                    }
                }
            } else if keys.just_pressed(KeyCode::KeyB) && !ph.dead {
                // DeusExPlayer.BuySkills.
                s.skills_open = !s.skills_open;
                crate::sound::play_player("DeusExSounds.UserInterface.Menu_Focus", 0.75, 0.0);
            }
        }
        if ph.dead {
            s.skills_open = false;
        }
        // HUDMultiSkills: the number keys buy (OverrideBelt); a bought level closes the list.
        if s.skills_open && !ui {
            const DIGITS: [KeyCode; 10] = [
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
                KeyCode::Digit0,
            ];
            for (i, k) in DIGITS.iter().enumerate() {
                if keys.just_pressed(*k) && buy_skill(i) {
                    s.skills_open = false;
                }
            }
        }
        // SkillMessage: a level is affordable (not swimming): the notice flashes for 10 s.
        let points = crate::abilities::lock().as_ref().map_or(0, |a| a.skills.points_avail);
        if points != s.notify_points {
            s.notified = false;
        }
        let avail = crate::abilities::lock().as_ref().is_some_and(|a| a.skills.list.iter().any(|k| k.def.class != "SkillSwimming" && k.level < 3 && a.skills.points_avail >= k.def.cost[k.level]));
        if avail && !s.notified && !s.skills_open {
            s.notified = true;
            s.notify_points = points;
            s.notify_left = SKILL_NOTIFY;
            crate::sound::play_player("DeusExSounds.UserInterface.Menu_BuySkills", 0.75, 0.0);
        }
        s.notify_left = if avail { (s.notify_left - dt).max(0.0) } else { 0.0 };
        let flash = s.notify_left > 0.0 && (s.notify_left % 1.5) > 0.5;
        (s.msg.clone(), s.death.clone().map(|d| (d, s.show_profile)), Some((s.skills_open, flash)))
    });
    let (s, k, sw, sh) = screen(win);
    // The centred notice, faded in 8 steps.
    {
        let (img, key) = &mut state[0];
        match msg {
            Some((m, t)) => {
                let fade = ((t / MSG_DELAY).clamp(0.0, 1.0) * 8.0).ceil() / 8.0;
                let (text, col) = msg_text(&m);
                let h = crate::util::fnv1a(&text) ^ (fade * 64.0) as u64 ^ (sw as u64) << 32;
                if h != *key || nodes.p0().is_empty() {
                    *key = h;
                    if let Some(font) = hud.font(&mut assets, "FontMenuTitle") {
                        let w = (font.width(&text) + 8).min(sw).max(8);
                        let mut c = Canvas::new(w, font.height + 2, s);
                        let col = [(col[0] as f32 * fade) as u8, (col[1] as f32 * fade) as u8, (col[2] as f32 * fade) as u8];
                        c.text_styled(&font, 0, 0, w, font.height + 2, HAlign::Center, VAlign::Top, col, &text, Style::Translucent);
                        let pos = Vec2::new((win.width() - w as f32 * k) / 2.0, win.height() * 0.33);
                        crate::prompts::present::<MsgNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p0(), img, Some(&c), (w, font.height + 2), s, pos, k);
                    }
                }
            }
            None => {
                if *key != 0 {
                    *key = 0;
                    crate::prompts::present::<MsgNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p0(), img, None, (1, 1), 1, Vec2::ZERO, 1.0);
                }
            }
        }
    }
    // The death screen.
    {
        let (img, key) = &mut state[1];
        match death {
            Some((d, show_profile)) => {
                let ready = d.t >= DEATH_LOCKOUT;
                let h = crate::util::fnv1a(&format!("{:?}{}{}{}{}{}", d.killer, d.method, d.self_kill, ready, show_profile, sw));
                if h != *key || nodes.p1().is_empty() {
                    *key = h;
                    if let Some(c) = draw_death(&mut hud, &mut assets, &d, ready, show_profile, sw, sh, s) {
                        crate::prompts::present::<DeathNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p1(), img, Some(&c), (sw, sh), s, Vec2::ZERO, k);
                    }
                }
            }
            None => {
                if *key != 0 {
                    *key = 0;
                    crate::prompts::present::<DeathNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p1(), img, None, (1, 1), 1, Vec2::ZERO, 1.0);
                }
            }
        }
    }
    // The skill list / "Skills available!".
    {
        let (img, key) = &mut state[2];
        let show = skills.filter(|(open, flash)| *open || *flash);
        match show {
            Some((open, _)) => {
                let rows = skill_rows();
                let h = crate::util::fnv1a(&format!("{open}{rows:?}{sw}{sh}"));
                if h != *key || nodes.p2().is_empty() {
                    *key = h;
                    if let Some(c) = draw_skills(&mut hud, &mut assets, open, &rows, sw, sh, s) {
                        crate::prompts::present::<SkillNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p2(), img, Some(&c), (sw, sh), s, Vec2::ZERO, k);
                    }
                }
            }
            None => {
                if *key != 0 {
                    *key = 0;
                    crate::prompts::present::<SkillNode, _>(&mut commands, &mut images, &mut layers, &mut nodes.p2(), img, None, (1, 1), 1, Vec2::ZERO, 1.0);
                }
            }
        }
    }
}

/// The skills HUDMultiSkills lists (all but swimming): (name, level, cost of the next level or None at Master,
/// affordable).
fn skill_rows() -> Vec<(String, usize, Option<i32>, bool, i32)> {
    let g = crate::abilities::lock();
    let Some(a) = g.as_ref() else { return Vec::new() };
    let pts = a.skills.points_avail;
    a.skills
        .list
        .iter()
        .filter(|k| k.def.class != "SkillSwimming")
        .map(|k| {
            let cost = (k.level < 3).then(|| k.def.cost[k.level]);
            (k.def.name.clone(), k.level, cost, cost.is_some_and(|c| pts >= c), pts)
        })
        .collect()
}

/// AttemptBuySkill: the index-th listed skill (0 = key 1 ... 9 = key 0) up a level if the points cover it.
fn buy_skill(i: usize) -> bool {
    let class = {
        let g = crate::abilities::lock();
        let Some(a) = g.as_ref() else { return false };
        match a.skills.list.iter().filter(|k| k.def.class != "SkillSwimming").nth(i) {
            Some(k) => k.def.class.clone(),
            None => return false,
        }
    };
    let ok = crate::abilities::lock().as_mut().is_some_and(|a| a.skills.upgrade(&class));
    crate::sound::play_player(if ok { "DeusExSounds.UserInterface.Menu_OK" } else { "DeusExSounds.UserInterface.Menu_Cancel" }, 0.75, 0.0);
    ok
}

#[allow(clippy::too_many_arguments)]
fn draw_skills(hud: &mut ClassicHud, assets: &mut crate::weapons::RuntimeAssets, open: bool, rows: &[(String, usize, Option<i32>, bool, i32)], sw: i32, sh: i32, s: i32) -> Option<Canvas> {
    const MS: &str = "HUDMultiSkills";
    let font = hud.font(assets, "FontMenuSmall_DS")?;
    let mut c = Canvas::new(sw, sh, s);
    let fh = font.height;
    let x0 = (sw as f32 * 0.01) as i32;
    let costx = (sw as f32 * 0.25) as i32;
    let levelx = (sw as f32 * 0.19) as i32;
    let key = "B";
    if !open {
        // SkillMessage.
        let a = word(MS, "SkillsAvailableString", "Skills available!");
        let b = format!("{}{key}{}", word(MS, "PressString", "Press <"), word(MS, "PressEndString", ">"));
        let y = (sh as f32 * 0.7) as i32;
        let wa = font.width(&a);
        let wb = font.width(&b);
        let off = (-(x0 + (wa - wb) / 2)).max(0);
        c.text(&font, x0 + off, y, wa, fh, HAlign::Left, VAlign::Top, [0, 255, 0], &a);
        c.text(&font, x0 + (wa - wb) / 2 + off, y + fh, wb, fh, HAlign::Left, VAlign::Top, [0, 255, 0], &b);
        return Some(c);
    }
    let cost_s = word(MS, "CostString", "Cost");
    let bar = costx + (font.width(&cost_s) as f32 * 1.33) as i32 - x0;
    let white = [255, 255, 255];
    let mut y = (sh as f32 * 0.38) as i32;
    c.fill(x0, y, bar, 1, Style::Normal, white);
    y += fh / 4;
    let pts = rows.first().map_or(0, |r| r.4);
    c.text(&font, x0, y, bar, fh, HAlign::Left, VAlign::Top, white, &format!("{}{pts}", word(MS, "SkillPointsString", "Skill Points: ")));
    y += fh;
    c.fill(x0, y, bar, 1, Style::Normal, white);
    y += fh / 4;
    c.text(&font, x0, y, levelx - x0, fh, HAlign::Left, VAlign::Top, white, &word(MS, "SkillString", "Skill"));
    c.text(&font, levelx, y, costx - levelx, fh, HAlign::Left, VAlign::Top, white, &word(MS, "LevelString", "Level"));
    c.text(&font, costx, y, bar, fh, HAlign::Left, VAlign::Top, white, &cost_s);
    y += fh;
    c.fill(x0, y, bar, 1, Style::Normal, white);
    y += fh / 4;
    for (i, (name, level, cost, afford, _)) in rows.iter().enumerate() {
        let col = match (cost, afford, level) {
            (None, ..) => [0, 0, 255],
            (Some(_), true, 2) => [0, 255, 0],
            (Some(_), true, _) => [0, 128, 0],
            (Some(_), false, 2) => [255, 0, 0],
            _ => [128, 0, 0],
        };
        let n = if i == 9 { 0 } else { i + 1 };
        c.text(&font, x0, y, levelx - x0, fh, HAlign::Left, VAlign::Top, col, &format!("{n}. {name}"));
        // DrawLevel: a 5-unit box per level.
        let mut bx = levelx;
        for _ in 0..*level {
            c.fill(bx, y + 2, 5, 5, Style::Normal, col);
            bx += 7;
        }
        let cs = match cost {
            Some(v) => v.to_string(),
            None => word(MS, "NAString", "MAX"),
        };
        c.text(&font, costx, y, bar, fh, HAlign::Left, VAlign::Top, col, &cs);
        y += fh;
    }
    c.fill(x0, y, bar, 1, Style::Normal, white);
    y += fh / 4;
    let exit = format!("{}{key}{}", word(MS, "PressString", "Press <"), word(MS, "ToExitString", "> to exit."));
    c.text(&font, x0, y, bar, fh, HAlign::Center, VAlign::Top, white, &exit);
    y += fh;
    c.fill(x0, y, bar, 1, Style::Normal, white);
    Some(c)
}

#[allow(clippy::too_many_arguments)]
fn draw_death(hud: &mut ClassicHud, assets: &mut crate::weapons::RuntimeAssets, d: &Death, ready: bool, show_profile: bool, sw: i32, sh: i32, s: i32) -> Option<Canvas> {
    const MMW: &str = "MultiplayerMessageWin";
    let big = hud.font(assets, "FontMenuExtraLarge").or_else(|| hud.font(assets, "FontMenuTitle"))?;
    let title = hud.font(assets, "FontMenuTitle")?;
    let small = hud.font(assets, "FontMenuSmall_DS")?;
    let mut c = Canvas::new(sw, sh, s);
    let red = [128, 0, 0];
    let mut y = (sh as f32 * 0.25) as i32;
    if d.self_kill {
        c.text(&big, 0, y, sw, big.height, HAlign::Center, VAlign::Top, red, &word(MMW, "KilledYourselfString", "You killed yourself!"));
    } else if let Some(k) = &d.killer {
        c.text(&big, 0, y, sw, big.height, HAlign::Center, VAlign::Top, red, &format!("{}{k}", word(MMW, "KilledByString", "You have been killed by ")));
        y += big.height;
        c.text(&big, 0, y, sw, big.height, HAlign::Center, VAlign::Top, red, d.method.trim());
    } else {
        c.text(&big, 0, y, sw, big.height, HAlign::Center, VAlign::Top, red, &word(MMW, "WereKilledString", "You have been killed."));
    }
    if ready {
        let mut y = (sh as f32 * 0.88) as i32;
        c.text(&title, 0, y, sw, title.height, HAlign::Center, VAlign::Top, [255, 255, 255], &word(MMW, "FireToContinueMsg", "Press <Fire> to restart."));
        if !show_profile && d.profile.is_some() {
            y += title.height;
            let t = format!("{}L{}", word(MMW, "Detail1String", "Press <"), word(MMW, "Detail2String", "> for details."));
            c.text(&title, 0, y, sw, title.height, HAlign::Center, VAlign::Top, [255, 255, 255], &t);
        }
    }
    if let (true, Some(p)) = (show_profile, d.profile.as_ref()) {
        // ShowKillerProfile.
        let white = [255, 255, 255];
        let fh = small.height;
        let mut y = (sh as f32 * 0.4) as i32;
        let line = format!("{}{}{}{}.", word(MMW, "FinalBlowString", "The final blow of "), p.damage, word(MMW, "ToTheString", " points of damage was delivered to your "), p.region);
        c.text(&small, 0, y, sw, fh, HAlign::Center, VAlign::Top, white, &line);
        y += fh;
        let line = format!("{}{}{}{}%", word(MMW, "RemHealthString", "At the time of your death, "), p.killer, word(MMW, "RemHealth2String", " had health: "), p.health);
        c.text(&small, 0, y, sw, fh, HAlign::Center, VAlign::Top, white, &line);
        y += fh;
        let line = format!("{}{}{}.", p.killer, word(MMW, "StreakString", " now has a current streak of "), p.streak);
        c.text(&small, 0, y, sw, fh, HAlign::Center, VAlign::Top, white, &line);
        y += fh * 3;
        let top = y;
        let col = |c: &mut Canvas, x: i32, name: &str, weapon: &str, augs: &[String], tint: [u8; 3]| -> i32 {
            let mut y = top;
            c.text(&small, x, y, sw / 2, fh, HAlign::Left, VAlign::Top, tint, name);
            y += fh;
            c.fill(x, y, sw / 5, 1, Style::Normal, tint);
            y += fh / 4;
            c.text(&small, x, y, sw / 2, fh, HAlign::Left, VAlign::Top, tint, &format!("{}{weapon}", word(MMW, "WeaponString", "Current Weapon: ")));
            y += fh;
            c.text(&small, x, y, sw / 2, fh, HAlign::Left, VAlign::Top, tint, &format!("{}{}", word(MMW, "ActiveAugsString", "Active Augs: "), augs.len()));
            y += fh;
            for a in augs {
                c.text(&small, x + 12, y, sw / 2, fh, HAlign::Left, VAlign::Top, tint, a);
                y += fh;
            }
            y
        };
        let a = col(&mut c, (sw as f32 * 0.2) as i32, &p.killer, &p.weapon, &p.augs, [255, 0, 0]);
        let me = crate::travel::player_name();
        let b = col(&mut c, (sw as f32 * 0.6) as i32, &me, &p.my_weapon, &p.my_augs, [0, 255, 0]);
        let vy = top + ((a.max(b) - top) as f32 * 0.4) as i32;
        c.text(&title, 0, vy, sw, title.height, HAlign::Center, VAlign::Top, white, &word(MMW, "VSString", "VS"));
    }
    Some(c)
}

/// The Classic scoreboard (ShowDMScoreboard / ShowTeamDMScoreboard) and, when the match is over, the win screen
/// above it (PlayerWinScreen / TeamWinScreen) and the seconds to the next map (ContinueMsg): a full-screen canvas.
pub fn draw_board<'a>(hud: &mut ClassicHud, assets: &mut crate::weapons::RuntimeAssets, board: &'a Board, sw: i32, sh: i32, s: i32) -> Option<Canvas> {
    const MPG: &str = "DeusExMPGame";
    let small = hud.font(assets, "FontMenuSmall")?;
    let title = hud.font(assets, "FontMenuTitle")?;
    let big = hud.font(assets, "FontMenuExtraLarge").unwrap_or_else(|| title.clone());
    let r = deathmatch::rules()?;
    let team = r.mode == deathmatch::Mode::Team;
    let game_class = r.classic.game.class();
    let mut c = Canvas::new(sw, sh, s);
    let white = [255, 255, 255];
    let gold = [255, 255, 0];
    let green = [0, 255, 0];
    let red = [255, 0, 0];
    let me = deathmatch::my_id();
    let my_team = board.rows.iter().find(|x| x.id == me).map(|x| x.team);
    let fw = |s: &str| small.width(s);
    let ylen = small.height;
    let px = (sw as f32 * 0.17) as i32;
    let kx = (sw as f32 * 0.55) as i32;
    let dx = (sw as f32 * 0.65) as i32;
    let sx = (sw as f32 * 0.75) as i32;
    let kills_s = word(MPG, "KillsString", "Kills");
    let deaths_s = word(MPG, "DeathsString", "Deaths");
    let streak_s = word(MPG, "StreakString", "Current Streak");
    // The win screen's headline and last kill (PlayerWinScreen / TeamWinScreen).
    if let Some(e) = &board.ended {
        let mut y = (sh as f32 * 0.15) as i32;
        let won = word(MPG, "WonMatchString", " has won the match!");
        let (text, col) = match (e.winner_team, e.winner_id) {
            (Some(t), _) if team => {
                let name = if t == deathmatch::TEAM_NSF { word(MPG, "TeamNsfString", "Team NSF") } else { word(MPG, "TeamUnatcoString", "Team Unatco") };
                (format!("{name}{won}"), if my_team == Some(t) { green } else { [128, 0, 0] })
            }
            (_, Some(id)) => (format!("{}{won}", board.rows.iter().find(|x| x.id == id).map_or("", |x| x.name.as_str())), gold),
            _ => (word(MPG, "TeamDrawString", "The match is a draw!"), gold),
        };
        c.text(&big, 0, y, sw, big.height, HAlign::Center, VAlign::Top, col, &text);
        y += big.height;
        if !e.last_kill.is_empty() {
            c.text(&title, 0, y, sw, title.height, HAlign::Center, VAlign::Top, col, &e.last_kill);
        }
    }
    let ystart = (sh as f32 * 0.25) as i32;
    let mut y = ystart;
    // ShowVictoryConditions.
    let objective = match r.classic.victory {
        deathmatch::Victory::Frags => format!("{}{}{}", word(game_class, "VictoryConString1", if team { "Objective: First team that reaches " } else { "Objective: First player that reaches " }), r.classic.score_to_win, word(game_class, "VictoryConString2", " kills wins the match.")),
        deathmatch::Victory::Time => {
            let t = board.time_left.unwrap_or(0.0).max(0.0) as i32;
            format!("{}{}:{:02}{}", word(game_class, "TimeLimitString1", "Objective: Score the most kills before the clock ( "), t / 60, t % 60, word(game_class, "TimeLimitString2", " ) runs out!"))
        }
    };
    c.text(&small, 0, y, sw, ylen, HAlign::Center, VAlign::Top, white, &objective);
    y += ylen * 2;
    // DrawHeaders.
    c.text(&small, px, y, kx - px, ylen, HAlign::Left, VAlign::Top, white, &word(MPG, "PlayerString", "Player"));
    c.text(&small, kx, y, dx - kx, ylen, HAlign::Left, VAlign::Top, white, &kills_s);
    c.text(&small, dx, y, sx - dx, ylen, HAlign::Left, VAlign::Top, white, &deaths_s);
    c.text(&small, sx, y, sw - sx, ylen, HAlign::Left, VAlign::Top, white, &streak_s);
    let bar = sx + fw(&streak_s) - px;
    c.fill(px, y + ylen, bar, 1, Style::Normal, white);
    y += ylen + ylen / 2;
    // DrawNameAndScore: numbers right-aligned on the header's middle.
    let num = |c: &mut Canvas, x: i32, header: &str, v: i64, y: i32, col: [u8; 3]| {
        let cx = x + fw(header) / 2 + fw("00") / 2;
        let t = v.to_string();
        c.text(&small, cx - fw(&t), y, fw(&t) + 2, ylen, HAlign::Left, VAlign::Top, col, &t);
    };
    let row = |c: &mut Canvas, name: &str, score: i64, deaths: i64, streak: i64, y: i32, col: [u8; 3]| {
        c.set_clip(Some((px, y, kx - px - 4, ylen)));
        c.text(&small, px, y, kx - px, ylen, HAlign::Left, VAlign::Top, col, name);
        c.set_clip(None);
        num(c, kx, &kills_s, score, y, col);
        num(c, dx, &deaths_s, deaths, y, col);
        num(c, sx, &streak_s, streak, y, col);
    };
    let sorted = |rows: Vec<&'a ScoreRow>| -> Vec<&'a ScoreRow> {
        let mut v = rows;
        v.sort_by(|a, b| b.score.cmp(&a.score).then(a.deaths.cmp(&b.deaths)));
        v
    };
    if team {
        let mine = my_team.unwrap_or(deathmatch::TEAM_UNATCO);
        let tname = |t: u8| if t == deathmatch::TEAM_NSF { word(MPG, "TeamNsfString", "Team NSF") } else { word(MPG, "TeamUnatcoString", "Team Unatco") };
        for (k, (t, col)) in [(mine, green), (1 - mine, red)].into_iter().enumerate() {
            let rows = sorted(board.rows.iter().filter(|x| x.team == t).collect());
            let label = if k == 0 { word("TeamDMGame", "AlliesString", "Allies") } else { word("TeamDMGame", "EnemiesString", "Enemies") };
            let (ts, td, tk) = rows.iter().fold((0i64, 0i64, 0i64), |a, x| (a.0 + x.score as i64, a.1 + x.deaths as i64, a.2 + x.streak as i64));
            row(&mut c, &format!("{label} ({})", tname(t)), ts, td, tk, y, col);
            c.fill(px, y + ylen, bar, 1, Style::Normal, col);
            y += ylen / 4;
            for x in rows {
                y += ylen;
                let rc = if x.id == me { gold } else if k == 0 { green } else { red };
                row(&mut c, &x.name, x.score as i64, x.deaths as i64, x.streak as i64, y, rc);
            }
            y += ylen * 2;
        }
    } else {
        for x in sorted(board.rows.iter().collect()) {
            y += ylen;
            row(&mut c, &x.name, x.score as i64, x.deaths as i64, x.streak as i64, y, if x.id == me { gold } else { white });
        }
    }
    // ContinueMsg.
    if board.ended.is_some() {
        let t = format!("{}{}", board.next_in.max(0.0) as i32, word(MPG, "NewMapSecondsString", " seconds to new map."));
        c.text(&title, 0, (sh as f32 * 0.8) as i32, sw, title.height, HAlign::Center, VAlign::Top, white, &t);
    }
    Some(c)
}

/// The screen in UI units at the HUD's scale (for scoreboard.rs).
pub fn screen_units(win: &Window) -> (i32, f32, i32, i32) {
    screen(win)
}
