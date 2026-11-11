//! The multiplayer scoreboard (ours, after DeusExMPGame.ShowDMScoreboard / TeamDMGame.ShowTeamDMScoreboard): held
//! on the Classic / Modern key (Tab) in a session, and shown by itself on the results screen at the end of a match
//! (DeathMatchGame.PlayerWinScreen: who won, the last kill, the board, the seconds to the next map).
//!
//! Columns: player, kills (Score), deaths, heals given (medkits used on other players), streak, ping; co-op shows
//! pawns killed instead of the streak. Team Deathmatch puts the two teams side by side, each with its totals. The
//! local player's row is gold, as in the original. Drawn on one canvas in the HUD theme (prompts::present).

use crate::classic_hud::{ClassicHud, HudLayer};
use crate::deathmatch::{self, Board, ScoreRow};
use crate::uicanvas::{Canvas, HAlign, Style, VAlign};
use bevy::prelude::*;

#[derive(Component, Default)]
pub struct ScoreNode;

const GOLD: [u8; 3] = [255, 255, 0];
const RED: [u8; 3] = [255, 90, 90];
const BLUE: [u8; 3] = [120, 170, 255];

static HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tab in a multiplayer session shows the board (remaster.rs leaves the key alone then).
pub fn key_held(keys: &ButtonInput<KeyCode>) -> bool {
    crate::netplay::active()
        && keys.pressed(KeyCode::Tab)
        && !crate::netplay::chat_open()
        && !crate::keybinds::UI_MODE.load(std::sync::atomic::Ordering::Relaxed)
        && !crate::menus::open()
}

fn hash(b: &Board, extra: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for r in &b.rows {
        (r.id, &r.name, r.team, r.score, r.deaths, r.streak, r.heals, r.npc_kills).hash(&mut h);
    }
    b.time_left.map(|t| t.ceil() as i32).hash(&mut h);
    b.next_in.ceil().to_bits().hash(&mut h);
    b.ended.as_ref().map(|e| (&e.headline, &e.last_kill)).hash(&mut h);
    extra.hash(&mut h);
    h.finish()
}

fn fmt_time(secs: f32) -> String {
    let t = secs.max(0.0).ceil() as i32;
    format!("{}:{:02}", t / 60, t % 60)
}

/// Rows sorted as SortScores does (points, then fewer deaths).
fn sorted(rows: &[ScoreRow]) -> Vec<ScoreRow> {
    let mut v = rows.to_vec();
    v.sort_by(|a, b| b.score.cmp(&a.score).then(a.deaths.cmp(&b.deaths)).then(a.name.cmp(&b.name)));
    v
}

/// The local roster when the host hasn't sent a board (co-op before a death, or a joiner's first frames).
fn roster_board() -> Board {
    let mut rows = vec![ScoreRow { id: deathmatch::my_id(), name: crate::travel::player_name(), ..Default::default() }];
    for (id, name, _) in crate::netplay::players() {
        rows.push(ScoreRow { id, name, ..Default::default() });
    }
    Board { rows, ..Default::default() }
}

#[allow(clippy::too_many_arguments)]
pub fn scoreboard_system(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    mut hud: ResMut<ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<HudLayer>>,
    level: Res<crate::LevelRes>,
    mut node: Query<(Entity, &mut Node, &mut Visibility), With<ScoreNode>>,
    mut state: Local<(Option<Handle<Image>>, u64)>,
) {
    let held = key_held(&keys);
    HELD.store(held, std::sync::atomic::Ordering::Relaxed);
    let Some(win) = windows.iter().next() else { return };
    let (img, key) = &mut *state;
    let show = crate::netplay::active() && (held || deathmatch::match_over()) && !crate::loader::busy();
    if !show {
        if *key != 0 {
            crate::prompts::present::<ScoreNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, None, (1, 1), 1, Vec2::ZERO, 1.0);
            *key = 0;
        }
        return;
    }
    let board = deathmatch::board().unwrap_or_else(roster_board);
    let pings: Vec<(u8, f32)> = crate::netplay::players().into_iter().map(|p| (p.0, p.2)).collect();
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();
    let mode = deathmatch::mode();
    let extra = pings.iter().map(|p| (p.1 / 20.0) as u64).sum::<u64>() ^ (s as u64) << 32 ^ (win.width() as u64) << 40 ^ mode as u64;
    let h = hash(&board, extra);
    if deathmatch::classic() {
        // The original's own layout: full screen, no panel (DeusExMPGame.ShowDMScoreboard / the win screens).
        if h == *key && !node.is_empty() {
            return;
        }
        *key = h;
        let (s, k, sw, sh) = crate::mphud::screen_units(win);
        if let Some(c) = crate::mphud::draw_board(&mut hud, &mut assets, &board, sw, sh, s) {
            crate::prompts::present::<ScoreNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, Some(&c), (sw, sh), s, Vec2::ZERO, k);
        }
        return;
    }
    // Layout in UI units: a fixed-width panel, centred.
    let team = mode == deathmatch::Mode::Team;
    let w = if team { 600 } else { 420 };
    let rows_n = if team {
        let u = board.rows.iter().filter(|r| r.team == deathmatch::TEAM_UNATCO).count();
        u.max(board.rows.len() - u)
    } else {
        board.rows.len()
    };
    let ended = board.ended.clone();
    let head_h = if ended.is_some() { 78 } else { 40 };
    let hgt = head_h + 30 + rows_n.max(1) as i32 * 14 + if team { 22 } else { 6 } + if ended.is_some() { 20 } else { 0 };
    let pos = Vec2::new((win.width() - w as f32 * k) / 2.0, (win.height() * 0.18).max(8.0));
    if h == *key && !node.is_empty() {
        return;
    }
    *key = h;
    let (Some(small), Some(title)) = (hud.font(&mut assets, "FontMenuSmall_DS"), hud.font(&mut assets, "FontMenuTitle")) else { return };
    let big = hud.font(&mut assets, "FontMenuExtraLarge").unwrap_or_else(|| title.clone());
    let mut c = Canvas::new(w, hgt, s);
    c.fill(0, 0, w, hgt, Style::Normal, [4, 4, 6]);
    c.shade(0, 0, w, hgt, 0.25);
    let border = crate::menuui::h_col(1);
    for (x, y, bw, bh) in [(0, 0, w, 1), (0, hgt - 1, w, 1), (0, 0, 1, hgt), (w - 1, 0, 1, hgt)] {
        c.fill(x, y, bw, bh, Style::Normal, border);
    }
    let text = crate::menuui::h_col(8);
    let header = crate::menuui::h_col(7);
    let mut y = 6;
    if let Some(e) = &ended {
        c.text(&big, 0, y, w, big.height, HAlign::Center, VAlign::Top, GOLD, &e.headline);
        y += big.height + 2;
        if !e.last_kill.is_empty() {
            c.text(&small, 4, y, w - 8, small.height, HAlign::Center, VAlign::Top, text, &e.last_kill);
        }
        y += small.height + 4;
    }
    // ShowVictoryConditions + the map.
    let map = deathmatch::map_title(&level.0.name);
    let objective = match (mode, board.frag_limit, board.time_left) {
        (deathmatch::Mode::Coop, ..) => format!("Co-op: {}", level.0.name.trim_end_matches(".dx")),
        (m, f, Some(t)) if f > 0 => format!("{} on {map}: first to {f} kills, or the most when the clock ( {} ) runs out", m.name(), fmt_time(t)),
        (m, f, None) if f > 0 => format!("{} on {map}: first {} to {f} kills wins the match", m.name(), if m == deathmatch::Mode::Team { "team" } else { "player" }),
        (m, _, Some(t)) => format!("{} on {map}: score the most kills before the clock ( {} ) runs out", m.name(), fmt_time(t)),
        (m, ..) => format!("{} on {map}", m.name()),
    };
    c.text(&title, 0, y, w, title.height, HAlign::Center, VAlign::Top, header, &objective);
    y += title.height + 8;
    let me = deathmatch::my_id();
    let ping_of = |id: u8| if id == me { String::new() } else { pings.iter().find(|p| p.0 == id).map_or_else(String::new, |p| format!("{:.0}", p.1)) };
    let coop = mode == deathmatch::Mode::Coop;
    let fourth = if coop { "Pawns" } else { "Streak" };
    // One table: player column then five number columns.
    let table = |c: &mut Canvas, x0: i32, tw: i32, y0: i32, rows: &[ScoreRow], tint: Option<[u8; 3]>| -> i32 {
        let nw = 44;
        let px = x0 + 6;
        let pw = tw - 12 - nw * 5;
        let cols = ["Kills", "Deaths", "Heals", fourth, "Ping"];
        c.text(&small, px, y0, pw, small.height, HAlign::Left, VAlign::Top, tint.unwrap_or(header), "Player");
        for (i, t) in cols.iter().enumerate() {
            c.text(&small, px + pw + i as i32 * nw, y0, nw, small.height, HAlign::Right, VAlign::Top, header, t);
        }
        c.fill(x0 + 4, y0 + small.height + 1, tw - 8, 1, Style::Normal, border);
        let mut yy = y0 + small.height + 4;
        for r in sorted(rows) {
            let col = if r.id == me { GOLD } else { text };
            c.set_clip(Some((px, yy, pw - 4, 14)));
            c.text(&small, px, yy, pw - 4, 14, HAlign::Left, VAlign::Center, col, &r.name);
            c.set_clip(None);
            let nums = [r.score.to_string(), r.deaths.to_string(), r.heals.to_string(), if coop { r.npc_kills.to_string() } else { r.streak.to_string() }, ping_of(r.id)];
            for (i, n) in nums.iter().enumerate() {
                c.text(&small, px + pw + i as i32 * nw, yy, nw, 14, HAlign::Right, VAlign::Center, col, n);
            }
            yy += 14;
        }
        yy
    };
    if team {
        let half = (w - 12) / 2;
        let mut bottom = y;
        for (k, t) in [deathmatch::TEAM_UNATCO, deathmatch::TEAM_NSF].into_iter().enumerate() {
            let rows: Vec<ScoreRow> = board.rows.iter().filter(|r| r.team == t).cloned().collect();
            let tint = if t == deathmatch::TEAM_UNATCO { BLUE } else { RED };
            let x0 = 4 + k as i32 * (half + 4);
            let total: i32 = rows.iter().map(|r| r.score).sum();
            let deaths: u32 = rows.iter().map(|r| r.deaths).sum();
            c.text(&title, x0, y, half, title.height, HAlign::Center, VAlign::Top, tint, &format!("{}: {total}", deathmatch::team_name(t)));
            let yy = table(&mut c, x0, half, y + title.height + 2, &rows, Some(tint));
            c.text(&small, x0 + 6, yy + 2, half - 12, small.height, HAlign::Left, VAlign::Top, header, &format!("Team Score: {total}  Deaths: {deaths}  Players: {}", rows.len()));
            bottom = bottom.max(yy + small.height + 4);
        }
        y = bottom;
    } else {
        y = table(&mut c, 4, w - 8, y, &board.rows, None);
    }
    if let Some(e) = &ended {
        let after = match e.after {
            deathmatch::After::Room => "back to the room".to_string(),
            _ => deathmatch::map_title(&e.next_map),
        };
        c.text(&small, 0, y + 4, w, small.height, HAlign::Center, VAlign::Top, header, &format!("{} seconds to {after}.", board.next_in.max(0.0).ceil() as i32));
    }
    let _ = y;
    crate::prompts::present::<ScoreNode, _>(&mut commands, &mut images, &mut layers, &mut node, img, Some(&c), (w, hgt), s, pos, k);
}
