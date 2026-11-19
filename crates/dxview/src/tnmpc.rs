//! The Nameless Mod's computer games (tnmComputerScreenGames' list): Pong, Breakout and Tetris, written for DX RUST
//! from how the mod's PongWindow / BreakoutBoard / TetrisWindow play. Their sizes, speeds, colours, pictures, fonts,
//! sounds and words are read from the player's TNMGUI.u (class defaults); Breakout's levels (block grids, names and
//! passcodes) and Tetris's pieces exist in the mod only as code, so they are read from its own script text at start
//! through the mods-only UnrealScript data loader (uscript.rs). Nothing of the mod is stored here.
//!
//! The games keep the mod's records (tnmgames.rs) and achievements (achPong, achBreakout, achTetris).

use bevy::input::ButtonInput;
use bevy::prelude::KeyCode;
use std::sync::Arc;
use unreal_pkg::props::{find, Property, Value};

use crate::menuui::{KitWid, Ui};
use crate::uicanvas::{HAlign, Style, UiFont, UiTex, VAlign};
use crate::weapons::RuntimeAssets;

type Rgb = [u8; 3];

fn num(d: &[Property], k: &str, def: f32) -> f32 {
    match find(d, k) {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f32,
        Some(Value::Byte(v)) => *v as f32,
        _ => def,
    }
}
fn flag(d: &[Property], k: &str) -> bool {
    matches!(find(d, k), Some(Value::Bool(true)))
}
fn path(d: &[Property], k: &str) -> String {
    match find(d, k) {
        Some(Value::Path(p)) if !p.ends_with(".None") => p.clone(),
        _ => String::new(),
    }
}
fn text(d: &[Property], k: &str) -> String {
    match find(d, k) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
fn color_at(d: &[Property], k: &str, index: u32) -> Option<Rgb> {
    d.iter().find(|p| p.name.eq_ignore_ascii_case(k) && p.index == index).and_then(|p| match &p.value {
        Value::Struct(_, b) if b.len() >= 3 => Some([b[0], b[1], b[2]]),
        _ => None,
    })
}
/// Two ints of a struct (Coord2D / Size2D).
fn pair(d: &[Property], k: &str, def: (i32, i32)) -> (i32, i32) {
    match find(d, k) {
        Some(Value::Struct(_, b)) if b.len() >= 8 => (
            i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        ),
        _ => def,
    }
}

/// sprintf with one %d / %s.
fn fmt1(f: &str, v: impl std::fmt::Display) -> String {
    let s = v.to_string();
    for k in ["%d", "%i", "%s"] {
        if f.contains(k) {
            return f.replacen(k, &s, 1);
        }
    }
    f.to_string()
}

/// A small random generator (FRand / Rand).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn frand(&mut self) -> f32 {
        (self.next() % 1_000_000) as f32 / 1_000_000.0
    }
    fn rand(&mut self, n: i32) -> i32 {
        if n <= 0 {
            0
        } else {
            (self.next() % n as u64) as i32
        }
    }
}

fn sound(s: &str) {
    if !s.is_empty() {
        crate::sound::play_player(s, 1.0, 0.0);
    }
}

fn font(assets: &mut RuntimeAssets, p: &str) -> Option<Arc<UiFont>> {
    if p.is_empty() {
        return None;
    }
    crate::classic_hud::load_font(assets, p).map(Arc::new)
}

/// Draws a texture (a full path) stretched over a box, tinted (gc.DrawStretchedTexture).
fn stretch<W: KitWid>(u: &mut Ui<W>, tex: &str, (x, y, w, h): (f32, f32, f32, f32), src: Option<(i32, i32, i32, i32)>, style: Style, col: Rgb) {
    let (x, y, w, h) = (x as i32, y as i32, w as i32, h as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let t: Option<Arc<UiTex>> = if tex.is_empty() { None } else { u.hud.tex(u.assets, tex) };
    match t {
        Some(t) => {
            let r = src.unwrap_or((0, 0, t.w as i32, t.h as i32));
            u.c.texture_stretched(x, y, w, h, r, &t, style, col);
        }
        None => u.c.fill(x, y, w, h, style, col),
    }
}

// ------------------------------------------------------------------------------------------------ Pong

/// PongWindow: the computer on the left, the player's paddle on the right (Up / W, Down / S), T or the Start button
/// starts (and restarts) a game; every point the player wins counts toward the record (achPong).
pub struct Pong {
    started: bool,
    board: (f32, f32, f32, f32),
    ball_w: f32,
    ball_h: f32,
    paddle_w: f32,
    paddle_h: f32,
    ball: (f32, f32),
    vel: (f32, f32),
    speed: f32,
    p1: (f32, f32),
    p2: (f32, f32),
    score: (i32, i32),
    ai_dest: f32,
    ai_search: bool,
    ai_speed: f32,
    draw_tex: String,
    font: Option<Arc<UiFont>>,
    rng: Rng,
    client: (i32, i32),
}

impl Pong {
    fn new(assets: &mut RuntimeAssets, seed: u64) -> Pong {
        let d = assets.classes.defaults("PongWindow");
        let mut p = Pong {
            started: false,
            board: (num(&d, "boardX", 16.0), num(&d, "boardY", 40.0), num(&d, "boardW", 600.0), num(&d, "boardH", 400.0)),
            ball_w: num(&d, "ballW", 8.0),
            ball_h: num(&d, "ballH", 8.0),
            paddle_w: num(&d, "paddleW", 8.0),
            paddle_h: num(&d, "paddleH", 48.0),
            ball: (0.0, 0.0),
            vel: (0.0, 0.0),
            speed: 1.5,
            p1: (0.0, 0.0),
            p2: (0.0, 0.0),
            score: (0, 0),
            ai_dest: 0.0,
            ai_search: true,
            ai_speed: num(&d, "AIspeed", 2.0),
            draw_tex: path(&d, "drawTexture"),
            font: font(assets, "TNMGUI.TestTrueTypeFont").or_else(|| font(assets, "FontMenuTitle")),
            rng: Rng(seed | 1),
            client: (num(&d, "ClientWidth", 640.0) as i32, num(&d, "ClientHeight", 480.0) as i32),
        };
        p.setup();
        p
    }
    /// SetupGame.
    fn setup(&mut self) {
        let (_, _, bw, bh) = self.board;
        self.p1 = (0.0, bh / 2.0 - self.paddle_h / 2.0);
        self.p2 = (bw - self.paddle_w, self.p1.1);
        self.score = (0, 0);
        self.ai_dest = self.p1.1;
        self.reset_ball();
    }
    /// ResetBall.
    fn reset_ball(&mut self) {
        let (_, _, bw, bh) = self.board;
        self.ball = (bw / 2.0 - self.ball_w, bh / 2.0 - self.ball_h);
        let mut vx = self.rng.frand() * 0.5 + 0.5;
        let mut vy = (1.0 - vx * vx).max(0.0).sqrt();
        if self.rng.frand() < 0.5 {
            vx = -vx;
        }
        if self.rng.frand() < 0.5 {
            vy = -vy;
        }
        self.vel = (vx, vy);
        self.speed = 1.5;
        self.ai_search = true;
    }
    fn start(&mut self) {
        self.started = true;
        self.setup();
    }
    fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>) {
        if !self.started {
            return;
        }
        crate::tnmgames::played("PONG", dt);
        let dt = dt * 150.0;
        let (_, _, bw, bh) = self.board;
        let (bx, by) = self.ball;
        let (vx, vy) = self.vel;
        // The computer works out where the ball will meet its side, bouncing off top and bottom.
        if self.ai_search {
            let mut fubar = 0;
            if vx < 0.0 {
                let mut t1 = (self.paddle_w - bx) / vx;
                self.ai_dest = by + vy * t1;
                let mut mult = 1.0;
                let mut pos = by;
                while (self.ai_dest < 0.0 || self.ai_dest > bh - self.paddle_h) && fubar < 10 {
                    let t2 = if mult * vy < 0.0 {
                        let t = -pos / vy * mult;
                        pos = 0.0;
                        t
                    } else {
                        let t = (bh - pos) / vy * mult;
                        pos = bh - self.ball_h;
                        t
                    };
                    t1 -= t2;
                    mult = -mult;
                    self.ai_dest = pos + vy * t1 * mult;
                    fubar += 1;
                }
                if self.ai_dest - self.paddle_h / 2.0 > 0.0 {
                    self.ai_dest -= self.paddle_h / 2.0;
                }
            }
            if fubar < 10 {
                self.ai_search = false;
            }
        } else if vx > 0.0 {
            self.ai_dest = by;
        }
        let mut sp = 0.0;
        if (self.p1.1 - self.ai_dest).abs() > 3.0 {
            sp = if self.p1.1 > self.ai_dest { -self.ai_speed } else { self.ai_speed };
        }
        let ny = self.p1.1 + sp * dt;
        if ny >= 0.0 && ny <= bh - self.paddle_h {
            self.p1.1 = ny;
        }
        let up = keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW);
        let down = keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS);
        let sp2 = if up { -3.0 } else if down { 3.0 } else { 0.0 };
        let ny = self.p2.1 + sp2 * dt;
        if ny >= 0.0 && ny <= bh - self.paddle_h {
            self.p2.1 = ny;
        }
        let mut update = true;
        let nx = bx + vx * dt * self.speed;
        let ny = by + vy * dt * self.speed;
        if ny < 0.0 || ny + self.ball_w > bh {
            self.vel.1 = -self.vel.1;
            if self.vel.0 >= 0.0 {
                // (the computer only watches its own side)
            } else {
                self.ai_search = true;
            }
            update = false;
        }
        if nx <= self.paddle_w && ny <= self.p1.1 + self.paddle_h && ny + self.ball_h >= self.p1.1 {
            self.ball.0 = self.paddle_w + 1.0;
            self.vel.0 = -self.vel.0;
            self.speed = (self.speed * 1.1).min(10.0);
            update = false;
        }
        if nx + self.ball_w >= self.p2.0 && ny <= self.p2.1 + self.paddle_h && ny + self.ball_h >= self.p2.1 {
            self.ball.0 = self.p2.0 - self.ball_w - 1.0;
            self.vel.0 = -self.vel.0;
            self.speed = (self.speed * 1.1).min(10.0);
            self.ai_search = true;
            update = false;
        }
        if update {
            self.ball = (nx, ny);
            if self.ball.0 <= 0.0 {
                self.score.1 += 1;
                crate::tnmgames::offer("Pong", self.score.1 as i64);
                crate::tnmach::unlock("achPong");
                self.reset_ball();
            } else if self.ball.0 + self.ball_w >= bw {
                self.score.0 += 1;
                self.reset_ball();
            }
        }
    }
    fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let (bx, by, bw, bh) = self.board;
        let (ox, oy) = (x as f32 + bx, y as f32 + by);
        let wt = 3.0;
        let white = [255, 255, 255];
        let t = self.draw_tex.clone();
        // The border lines.
        stretch(u, &t, (ox - wt * 2.0, oy - wt * 2.0, wt, bh + wt * 3.0), Some((0, 0, 8, 8)), Style::Normal, white);
        stretch(u, &t, (ox + bw + wt, oy - wt * 2.0, wt, bh + wt * 3.0), Some((0, 0, 8, 8)), Style::Normal, white);
        stretch(u, &t, (ox - wt * 2.0, oy - wt * 2.0, bw + wt * 3.0, wt), Some((0, 0, 8, 8)), Style::Normal, white);
        stretch(u, &t, (ox - wt * 2.0, oy + bh + wt, bw + wt * 4.0, wt), Some((0, 0, 8, 8)), Style::Normal, white);
        // Paddles and ball.
        stretch(u, &t, (ox + self.p1.0, oy + self.p1.1, self.paddle_w, self.paddle_h), Some((0, 0, 8, 8)), Style::Masked, white);
        stretch(u, &t, (ox + self.p2.0, oy + self.p2.1, self.paddle_w, self.paddle_h), Some((0, 0, 8, 8)), Style::Masked, white);
        stretch(u, &t, (ox + self.ball.0, oy + self.ball.1, self.ball_w, self.ball_h), Some((0, 0, 8, 8)), Style::Masked, white);
        if let Some(f) = self.font.clone() {
            let p1 = self.score.0.to_string();
            let p2 = self.score.1.to_string();
            u.text(&f, x + 40, y + 8, 80, f.height, HAlign::Left, VAlign::Top, white, &p1);
            let px = 595 - 16 * (p2.len() as i32 - 1);
            u.text(&f, x + px, y + 8, 80, f.height, HAlign::Left, VAlign::Top, white, &p2);
        }
    }
}

// ------------------------------------------------------------------------------------------------ Breakout

#[derive(Clone, Default)]
struct LevelBlock {
    used: bool,
    unbreakable: bool,
    col: Rgb,
    strength: i32,
}

#[derive(Clone, Default)]
struct BLevel {
    blocks: Vec<LevelBlock>,
    name: String,
    passcode: String,
}

/// The board's levels as the mod's BreakoutBoardTNM.InitLevels writes them (read through uscript.rs).
#[derive(Default)]
struct Levels {
    levels: Vec<BLevel>,
    order: Vec<usize>,
    count: usize,
}

struct LevelHost<'a>(&'a mut Levels);

impl crate::uscript::Host for LevelHost<'_> {
    fn call(&mut self, name: &str, _args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        match name {
            "cr" => Some(crate::uscript::V::Str("\n".into())),
            _ => None,
        }
    }
    fn get(&mut self, _name: &str) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn set(&mut self, path: &str, v: &crate::uscript::V) {
        // level_count, level_order[i], levels[l].name / passcode / blocks[b].busd / bunbreakable / str / col.r|g|b
        let idx = |s: &str| -> Option<(String, usize)> {
            let (n, rest) = s.split_once('[')?;
            Some((n.to_string(), rest.trim_end_matches(']').parse().ok()?))
        };
        let parts: Vec<&str> = path.split('.').collect();
        let l = &mut *self.0;
        match parts.as_slice() {
            ["level_count"] => l.count = v.int().max(0) as usize,
            [p] => {
                if let Some((n, i)) = idx(p) {
                    if n == "level_order" && i < 100 {
                        if l.order.len() <= i {
                            l.order.resize(i + 1, 0);
                        }
                        l.order[i] = v.int().max(0) as usize;
                    }
                }
            }
            [lv, rest @ ..] => {
                let Some((n, li)) = idx(lv) else { return };
                if n != "levels" || li >= 100 {
                    return;
                }
                if l.levels.len() <= li {
                    l.levels.resize(li + 1, BLevel::default());
                }
                let level = &mut l.levels[li];
                match rest {
                    ["name"] => level.name = v.str(),
                    ["passcode"] => level.passcode = v.str(),
                    [b, field @ ..] => {
                        let Some((bn, bi)) = idx(b) else { return };
                        if bn != "blocks" || bi >= 280 {
                            return;
                        }
                        if level.blocks.len() < 280 {
                            level.blocks.resize(280, LevelBlock::default());
                        }
                        let blk = &mut level.blocks[bi];
                        match field {
                            ["bused"] => blk.used = v.truthy(),
                            ["bunbreakable"] => blk.unbreakable = v.truthy(),
                            ["str"] => blk.strength = v.int() as i32,
                            ["col", c] => {
                                let x = v.int().clamp(0, 255) as u8;
                                match *c {
                                    "r" => blk.col[0] = x,
                                    "g" => blk.col[1] = x,
                                    "b" => blk.col[2] = x,
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// BreakoutBoardTNM's levels, run once from the player's TNMGUI.u (InitLevels calls InitLevel1..n itself when
/// the script has them; else each InitLevelN is run).
fn load_levels(classes: &mut unreal_pkg::classes::ClassDb) -> Levels {
    let mut out = Levels::default();
    let Some(src) = classes.script_text("BreakoutBoardTNM") else { return out };
    let mut script = crate::uscript::Script::default();
    script.add_source(&src);
    let mut host = LevelHost(&mut out);
    script.call(&mut host, "initlevels", &[]);
    let lower = src.to_ascii_lowercase();
    let calls_itself = lower.contains("initlevel1()") && lower.matches("initlevel1()").count() > 1;
    if !calls_itself {
        for i in 1..=100 {
            if !lower.contains(&format!("function initlevel{i}()")) {
                break;
            }
            script.call(&mut host, &format!("initlevel{i}"), &[]);
        }
    }
    if out.order.is_empty() {
        out.order = (0..out.levels.len()).collect();
    }
    if out.count == 0 {
        out.count = out.order.len().min(out.levels.len());
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ball,
    Block,
    Paddle,
    Powerup,
    Wall(WallSide),
}

#[derive(Clone, Copy, PartialEq)]
enum WallSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// Per-kind defaults (BreakoutObject subclasses).
#[derive(Clone)]
struct ObjDef {
    w: f32,
    h: f32,
    vs: f32,
    tex: String,
    col: Rgb,
    style: Style,
    remove_amt: f32,
    moving: bool,
    stat: bool,
    collidable: bool,
    visible: bool,
    override_draw: bool,
}

fn obj_def(assets: &mut RuntimeAssets, class: &str) -> ObjDef {
    let d = assets.classes.defaults(class);
    ObjDef {
        w: num(&d, "W", 8.0),
        h: num(&d, "H", 8.0),
        vs: num(&d, "VS", 0.0),
        tex: path(&d, "drawTexture"),
        col: color_at(&d, "drawColor", 0).unwrap_or([255, 255, 255]),
        style: match find(&d, "drawStyle") {
            Some(Value::Byte(2)) => Style::Masked,
            Some(Value::Byte(3)) => Style::Translucent,
            _ => Style::Normal,
        },
        remove_amt: num(&d, "removeTimerAmt", 0.0),
        moving: flag(&d, "bMoving"),
        stat: flag(&d, "bStatic"),
        collidable: flag(&d, "bCollidable"),
        visible: flag(&d, "bVisible"),
        override_draw: flag(&d, "bOverrideTextureDraw"),
    }
}

#[derive(Clone)]
struct Obj {
    kind: Kind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
    vs: f32,
    tex: String,
    col: Rgb,
    style: Style,
    visible: bool,
    stat: bool,
    moving: bool,
    collidable: bool,
    override_draw: bool,
    remove: bool,
    remove_timer: f32,
    remove_amt: f32,
    // ball
    slow: bool,
    fast: bool,
    killer: bool,
    superkiller: bool,
    t_slow: f32,
    t_fast: f32,
    t_killer: f32,
    t_super: f32,
    // block
    str_max: i32,
    str_cur: i32,
    full_col: Rgb,
    old: (f32, f32, f32, f32),
    score_mult: f32,
    broken: bool,
    unbreakable: bool,
    // paddle
    ball: Option<usize>,
    ball_off: (f32, f32),
    has_ball: bool,
    base_w: f32,
    invert: bool,
    magnet: bool,
    t_invert: f32,
    // powerup
    power: &'static str,
    letter: &'static str,
    letter_col: Rgb,
}

impl Obj {
    fn new(kind: Kind, d: &ObjDef) -> Obj {
        Obj {
            kind,
            x: 0.0,
            y: 0.0,
            w: d.w,
            h: d.h,
            px: 0.0,
            py: 0.0,
            vx: 0.0,
            vy: 0.0,
            vs: d.vs,
            tex: d.tex.clone(),
            col: d.col,
            style: d.style,
            visible: d.visible,
            stat: d.stat,
            moving: d.moving,
            collidable: d.collidable,
            override_draw: d.override_draw,
            remove: false,
            remove_timer: 0.0,
            remove_amt: d.remove_amt,
            slow: false,
            fast: false,
            killer: false,
            superkiller: false,
            t_slow: 0.0,
            t_fast: 0.0,
            t_killer: 0.0,
            t_super: 0.0,
            str_max: 1,
            str_cur: 1,
            full_col: d.col,
            old: (0.0, 0.0, 0.0, 0.0),
            score_mult: 1.0,
            broken: false,
            unbreakable: false,
            ball: None,
            ball_off: (0.0, 0.0),
            has_ball: false,
            base_w: d.w,
            invert: false,
            magnet: false,
            t_invert: 0.0,
            power: "",
            letter: "",
            letter_col: [255, 255, 255],
        }
    }
    /// IsColliding (the # operator truncates).
    fn colliding(&self, b: &Obj) -> bool {
        if let Kind::Wall(s) = self.kind {
            return match s {
                WallSide::Top => b.y <= self.y,
                WallSide::Bottom => b.y + b.h >= self.y,
                WallSide::Left => b.x <= self.x,
                WallSide::Right => b.x + b.w >= self.x,
            };
        }
        let t = |v: f32| v.trunc();
        let (x, y, w, h) = (t(self.x), t(self.y), t(self.w), t(self.h));
        let (bx, by, bw, bh) = (t(b.x), t(b.y), t(b.w), t(b.h));
        let ox = (x >= bx && x <= bx + bw) || (bx >= x && bx <= x + w) || (x + w >= bx && x + w <= bx + bw) || (bx + bw >= x && bx + bw <= x + w);
        let oy = (y >= by && y <= by + bh) || (by >= y && by <= y + h) || (y + h >= by && y + h <= by + bh) || (by + bh >= y && by + bh <= y + h);
        ox && oy
    }
    /// CollisionSide: where B came from.
    fn side(&self, b: &Obj) -> &'static str {
        let t = |v: f32| v.trunc();
        if !self.colliding(b) {
            "NONE"
        } else if t(b.x) + t(b.w) >= t(self.x) && t(b.px) + t(b.w) < t(self.x) {
            "LEFT"
        } else if t(b.x) <= t(self.x) + t(self.w) && t(b.px) > t(self.x) + t(self.w) {
            "RIGHT"
        } else if t(b.y) + t(b.h) >= t(self.y) && t(b.py) + t(b.h) < t(self.y) {
            "TOP"
        } else if t(b.y) <= t(self.y) + t(self.h) && t(b.py) > t(self.y) + t(self.h) {
            "BOTTOM"
        } else {
            ""
        }
    }
    fn ball_mult(&self, balls: i32) -> f32 {
        let mut m = 1.0;
        if self.fast {
            m *= 2.0;
        }
        if self.slow {
            m *= 0.75;
        }
        if self.killer {
            m *= 0.5;
        }
        if self.superkiller {
            m *= 0.25;
        }
        if balls > 1 {
            m *= 0.75;
        }
        m
    }
    fn paddle_mult(&self) -> f32 {
        let mut m = 1.0;
        if self.magnet {
            m *= 0.75;
        }
        if self.invert {
            m *= 1.5;
        }
        if self.w <= self.base_w * 0.5 {
            m *= 1.5;
        }
        if self.w <= self.base_w * 0.75 {
            m *= 1.5;
        }
        if self.w >= self.base_w * 1.5 {
            m *= 0.75;
        }
        if self.w >= self.base_w * 2.0 {
            m *= 0.75;
        }
        m
    }
}

struct Defs {
    ball: ObjDef,
    block: ObjDef,
    paddle: ObjDef,
    powerup: ObjDef,
    wall: ObjDef,
    ball_speed: f32,
    broken_tex: String,
    unbreak_tex: String,
    powerup_chance: f32,
    letter_font: Option<Arc<UiFont>>,
    sounds: [String; 9],
}

/// BreakoutBoard (BreakoutBoardTNM): Space launches, Left / Right or A / D move, P pauses (or, stopped, asks for a
/// passcode), Enter starts / tries the passcode, End drops the ball. Breaking every breakable block moves on to the
/// next level (a life back every second level, achBreakout); powerups fall out of broken blocks.
pub struct Breakout {
    w: f32,
    h: f32,
    levels: Levels,
    defs: Defs,
    objs: Vec<Obj>,
    paddle: usize,
    running: bool,
    paused: bool,
    game_over: bool,
    passcode: bool,
    passcode_str: String,
    block_count: i32,
    ball_count: i32,
    lives: i32,
    score: i32,
    level: usize,
    lives_start: i32,
    level_start: usize,
    block_start: (f32, f32),
    block_space: (f32, f32),
    pending_ball: Option<f32>,
    created: bool,
    rng: Rng,
    // The scoreboard.
    sb: (f32, f32, f32, f32),
    sb_fonts: (Option<Arc<UiFont>>, Option<Arc<UiFont>>, Option<Arc<UiFont>>),
    overlay_font: Option<Arc<UiFont>>,
    strs: [String; 10],
    bg: Vec<String>,
    bg_cols: usize,
    bg_rows: usize,
}

impl Breakout {
    /// The board (BreakoutWindow: 495 - 16 wide, 480 high) and the scoreboard to its right.
    pub const BOARD: (f32, f32) = (479.0, 480.0);

    fn new(assets: &mut RuntimeAssets, seed: u64) -> Breakout {
        let d = assets.classes.defaults("BreakoutBoardTNM");
        let bd = assets.classes.defaults("BreakoutBlock");
        let wd = assets.classes.defaults("BreakoutWindow");
        let bg: Vec<String> = (0..16)
            .map_while(|i| match wd.iter().find(|p| p.name.eq_ignore_ascii_case("clientTextures") && p.index == i).map(|p| &p.value) {
                Some(Value::Path(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let snd = |n: &str| format!("TNMGUI.{n}");
        let mut b = Breakout {
            w: Self::BOARD.0,
            h: Self::BOARD.1,
            levels: load_levels(&mut assets.classes),
            defs: Defs {
                ball: obj_def(assets, "BreakoutBall"),
                block: obj_def(assets, "BreakoutBlock"),
                paddle: obj_def(assets, "BreakoutPaddle"),
                powerup: obj_def(assets, "BreakoutPowerup"),
                wall: obj_def(assets, "BreakoutWall"),
                ball_speed: num(&assets.classes.defaults("BreakoutBall"), "speedInit", 3.0),
                broken_tex: path(&bd, "BrokenTexture"),
                unbreak_tex: path(&bd, "UnbreakableTexture"),
                powerup_chance: num(&bd, "powerupChance", 0.1),
                letter_font: {
                    let lf = path(&assets.classes.defaults("BreakoutPowerup"), "letterFont");
                    font(assets, &lf)
                },
                sounds: [
                    snd("BreakoutBallHit"),
                    snd("BreakoutBallFell"),
                    snd("BreakoutUnbreakable"),
                    snd("BreakoutBlockBreak"),
                    snd("BreakoutPowerUpGood"),
                    snd("BreakoutPowerUpBad"),
                    snd("BreakoutGrowPaddle"),
                    snd("BreakoutShrinkPaddle"),
                    snd("BreakoutExtraLife"),
                ],
            },
            objs: Vec::new(),
            paddle: 0,
            running: false,
            paused: false,
            game_over: false,
            passcode: false,
            passcode_str: String::new(),
            block_count: 0,
            ball_count: 0,
            lives: 0,
            score: 0,
            level: 0,
            lives_start: num(&d, "lives_start", 2.0) as i32,
            level_start: num(&d, "level_start", 0.0) as usize,
            block_start: (num(&d, "blockStartX", 0.0), num(&d, "blockStartY", 0.0)),
            block_space: (num(&d, "blockSpaceX", 0.0), num(&d, "blockSpaceY", 0.0)),
            pending_ball: None,
            created: false,
            rng: Rng(seed | 1),
            sb: (num(&d, "scoreboardX", 16.0), num(&d, "scoreboardY", 0.0), num(&d, "scoreboardW", 160.0), num(&d, "scoreboardSpacer", 4.0)),
            sb_fonts: (
                font(assets, &path(&d, "scoreboardHeaderFont")),
                font(assets, &path(&d, "scoreboardFont")),
                font(assets, &path(&d, "scoreboardPasscodeFont")).or_else(|| font(assets, "TNMGUI.PasscodeFont")),
            ),
            overlay_font: font(assets, &path(&d, "gameOverlayFont")),
            strs: [
                text(&d, "scoreboardLivesStr"),
                text(&d, "scoreboardLevelStr"),
                text(&d, "scoreboardScoreStr"),
                text(&d, "scoreboardPasscodeStr"),
                text(&d, "strPaused1"),
                text(&d, "strPaused2"),
                text(&d, "strGameOver1"),
                text(&d, "strGameOver2"),
                text(&d, "strPasscode"),
                text(&d, "strRestart"),
            ],
            bg,
            bg_cols: num(&wd, "textureCols", 2.0) as usize,
            bg_rows: num(&wd, "textureRows", 2.0) as usize,
        };
        b.init_game();
        b
    }

    fn snd(&self, i: usize) {
        sound(&self.defs.sounds[i]);
    }

    fn init_game(&mut self) {
        self.objs.clear();
        self.level = self.level_start;
        self.lives = self.lives_start;
        self.created = false;
    }

    fn level_idx(&self, l: usize) -> Option<usize> {
        if self.levels.count == 0 {
            return None;
        }
        self.levels.order.get(l % self.levels.count).copied()
    }

    /// CreateLevel.
    fn create_level(&mut self, l: usize) {
        self.objs.retain(|o| o.kind != Kind::Block);
        self.fix_paddle();
        self.block_count = 0;
        let Some(li) = self.level_idx(l) else { return };
        let Some(level) = self.levels.levels.get(li).cloned() else { return };
        for row in 0..20 {
            for col in 0..14 {
                let Some(lb) = level.blocks.get(row * 14 + col) else { continue };
                if !lb.used {
                    continue;
                }
                let mut o = Obj::new(Kind::Block, &self.defs.block);
                o.col = lb.col;
                o.str_max = lb.strength.max(1);
                o.x = self.block_start.0 + (o.w + self.block_space.0) * col as f32;
                o.y = self.block_start.1 + (o.h + self.block_space.1) * row as f32;
                o.unbreakable = lb.unbreakable;
                if !lb.unbreakable {
                    self.block_count += 1;
                }
                // Init.
                o.str_cur = o.str_max;
                o.full_col = o.col;
                o.remove_amt = 7.5;
                if o.unbreakable {
                    o.tex = self.defs.unbreak_tex.clone();
                }
                self.objs.push(o);
            }
        }
    }

    fn fix_paddle(&mut self) {
        if let Some(i) = self.objs.iter().position(|o| o.kind == Kind::Paddle) {
            self.paddle = i;
        }
    }

    /// CreatePaddleBall.
    fn create_paddle_ball(&mut self) {
        self.fix_paddle();
        let (px, py, pw) = {
            let p = &self.objs[self.paddle];
            (p.x, p.y, p.w)
        };
        let mut ball = Obj::new(Kind::Ball, &self.defs.ball);
        ball.x = px + pw / 2.0 - ball.w / 2.0;
        ball.y = py - ball.h - 1.0;
        self.objs.push(ball);
        let bi = self.objs.len() - 1;
        self.get_ball(bi);
        self.ball_count += 1;
    }

    /// BreakoutPaddle.GetBall.
    fn get_ball(&mut self, bi: usize) {
        let p = self.paddle;
        let vs = self.objs[p].vs;
        let (px, py) = (self.objs[p].x, self.objs[p].y);
        let b = &mut self.objs[bi];
        b.vx = 0.0;
        b.vy = 0.0;
        b.collidable = false;
        b.vs = vs;
        let off = (b.x - px, b.y - py);
        let p = &mut self.objs[self.paddle];
        p.ball = Some(bi);
        p.ball_off = off;
        p.has_ball = true;
    }

    /// CreateGame.
    fn create_game(&mut self) {
        self.objs.clear();
        let mut p = Obj::new(Kind::Paddle, &self.defs.paddle);
        p.x = self.w / 2.0 - p.w / 2.0;
        p.y = self.h - p.h - 3.0;
        p.base_w = p.w;
        self.objs.push(p);
        self.paddle = 0;
        for (s, x, y, w, h) in [
            (WallSide::Top, 0.0, 0.0, self.w, 1.0),
            (WallSide::Bottom, 0.0, self.h - 1.0, self.w, 1.0),
            (WallSide::Left, 0.0, 0.0, 1.0, self.h - 1.0),
            (WallSide::Right, self.w - 1.0, 0.0, 1.0, self.h - 1.0),
        ] {
            let mut o = Obj::new(Kind::Wall(s), &self.defs.wall);
            o.x = x;
            o.y = y;
            o.w = w;
            o.h = h;
            self.objs.push(o);
        }
        self.ball_count = 0;
        self.create_paddle_ball();
        self.create_level(self.level);
        self.ball_count = 1;
        self.created = true;
    }

    fn block_destroyed(&mut self, mult: f32) {
        self.fix_paddle();
        let pm = self.objs.get(self.paddle).map_or(1.0, |p| p.paddle_mult());
        self.score += (10.0 * (mult * pm).clamp(0.1, 4.0)) as i32;
        crate::tnmgames::offer("Breakout", self.score as i64);
        self.block_count -= 1;
        if self.block_count == 0 {
            if self.level % 2 == 1 {
                self.lives += 1;
            }
            crate::tnmach::unlock("achBreakout");
            self.next_level();
        }
    }

    fn next_level(&mut self) {
        self.block_count = 0;
        self.objs.retain(|o| o.kind != Kind::Ball);
        self.fix_paddle();
        self.level += 1;
        let l = self.level;
        self.create_level(l);
        self.ball_count = 0;
        self.create_paddle_ball();
        self.paddle_ball_fell();
        self.ball_count = 1;
    }

    fn paddle_ball_fell(&mut self) {
        self.fix_paddle();
        if let Some(p) = self.objs.get_mut(self.paddle) {
            p.w = p.base_w;
            p.invert = false;
            p.magnet = false;
        }
    }

    fn ball_fell(&mut self) {
        self.ball_count -= 1;
        if self.ball_count == 0 {
            self.lives -= 1;
            if self.lives < 0 {
                self.running = false;
                self.game_over = true;
                return;
            }
            self.paddle_ball_fell();
            self.snd(1);
            self.pending_ball = Some(1.0);
        }
    }

    fn kill(&mut self, i: usize) {
        if self.objs[i].remove {
            return;
        }
        let o = &mut self.objs[i];
        o.remove_timer = o.remove_amt;
        o.remove = true;
        if o.kind == Kind::Block {
            // PostKill.
            o.style = Style::Translucent;
            o.col = o.full_col;
            o.stat = false;
            o.collidable = false;
            o.old = (o.x, o.y, o.w, o.h);
            let unbreak = o.unbreakable;
            let mult = o.score_mult;
            let (x, y, w, h) = (o.x, o.y, o.w, o.h);
            self.snd(3);
            if !unbreak {
                self.block_destroyed(mult);
            }
            if self.rng.frand() < self.defs.powerup_chance {
                self.make_powerup(x + w / 2.0, y + h / 2.0);
            }
        }
    }

    /// BreakoutBlock.MakePowerup: one of eleven kinds, its letter and colours.
    fn make_powerup(&mut self, cx: f32, cy: f32) {
        let mut p = Obj::new(Kind::Powerup, &self.defs.powerup);
        p.x = cx - p.w / 2.0;
        p.y = cy - p.h / 2.0;
        let (power, letter, lc, dc): (&'static str, &'static str, Rgb, Rgb) = match self.rng.rand(11) {
            0 => ("SHRINK", "S", [0, 0, 128], [255, 0, 255]),
            1 => ("GROW", "G", [0, 255, 0], [255, 255, 0]),
            2 => ("INVERT", " I", [255, 0, 0], [255, 255, 255]),
            3 => ("FAST", "F", [255, 128, 0], [64, 255, 64]),
            4 => ("SLOW", "S", [255, 255, 255], [0, 255, 0]),
            5 => ("KILLER", "K", [255, 255, 255], [128, 0, 0]),
            6 => ("SUPERKILLER", "K", [255, 0, 0], [64, 64, 64]),
            7 => ("SPLIT", "+", [255, 0, 255], [64, 64, 64]),
            8 => ("MAGNET", "M", [255, 255, 255], [0, 0, 128]),
            9 => ("SKIP", "L", [255, 255, 0], [255, 255, 255]),
            _ => ("1UP", "E", [0, 255, 0], [0, 128, 0]),
        };
        p.power = power;
        p.letter = letter;
        p.letter_col = lc;
        p.col = dc;
        self.objs.push(p);
    }

    /// BreakoutPaddle.PowerUp.
    fn power_up(&mut self, power: &str) {
        self.fix_paddle();
        let bw = self.w;
        match power {
            "SPLIT" | "FAST" | "SLOW" | "KILLER" | "SUPERKILLER" => self.power_balls(power),
            "INVERT" => {
                let p = &mut self.objs[self.paddle];
                p.invert = true;
                p.t_invert = 10.0;
            }
            "GROW" => {
                let p = &mut self.objs[self.paddle];
                if p.w < p.base_w * 2.0 {
                    if p.w < p.base_w {
                        p.x = (p.x - (p.base_w - p.w) / 2.0).max(1.0);
                        p.w = p.base_w;
                    } else {
                        p.x = (p.x - p.base_w / 4.0).max(1.0);
                        p.w += p.base_w / 2.0;
                    }
                    if p.x + p.w > bw - 2.0 {
                        p.x = bw - 2.0 - p.w;
                    }
                }
            }
            "SHRINK" => {
                let p = &mut self.objs[self.paddle];
                if p.w > p.base_w / 2.0 {
                    if p.w > p.base_w {
                        p.x += (p.w - p.base_w) / 2.0;
                        p.w = p.base_w;
                    } else {
                        p.x += p.w / 8.0;
                        p.w -= p.w / 4.0;
                    }
                }
            }
            "MAGNET" => self.objs[self.paddle].magnet = true,
            "SKIP" => self.next_level(),
            "1UP" => self.lives += 1,
            _ => {}
        }
        match power {
            "SPLIT" | "SLOW" | "KILLER" | "SUPERKILLER" | "MAGNET" | "SKIP" => self.snd(4),
            "INVERT" | "FAST" => self.snd(5),
            "GROW" => self.snd(6),
            "SHRINK" => self.snd(7),
            "1UP" => self.snd(8),
            _ => {}
        }
    }

    /// PowerUpBalls (a split only splits the first ball).
    fn power_balls(&mut self, pwr: &str) {
        let speed = self.defs.ball_speed;
        for i in 0..self.objs.len() {
            if self.objs[i].kind != Kind::Ball {
                continue;
            }
            match pwr {
                "SPLIT" => {
                    let (x, y) = (self.objs[i].x, self.objs[i].y);
                    for _ in 0..2 {
                        let mut b = Obj::new(Kind::Ball, &self.defs.ball);
                        b.x = x;
                        b.y = y;
                        b.vs = speed;
                        b.vx = self.rng.frand();
                        if self.rng.frand() < 0.5 {
                            b.vx = -b.vx;
                        }
                        b.vy = (1.0 - b.vx * b.vx).max(0.0).sqrt();
                        if self.rng.frand() < 0.5 {
                            b.vy = -b.vy;
                        }
                        self.objs.push(b);
                    }
                    self.ball_count += 2;
                    return;
                }
                "FAST" => {
                    let b = &mut self.objs[i];
                    if b.slow {
                        b.slow = false;
                        b.vs = speed;
                    } else {
                        b.fast = true;
                        b.t_fast = 10.0;
                        b.vs *= 1.5;
                    }
                }
                "SLOW" => {
                    let b = &mut self.objs[i];
                    if b.fast {
                        b.fast = false;
                        b.vs = speed;
                    } else {
                        b.slow = true;
                        b.t_slow = 30.0;
                        b.vs /= 1.5;
                    }
                }
                "KILLER" => {
                    let b = &mut self.objs[i];
                    b.superkiller = false;
                    b.killer = true;
                    b.t_killer = 10.0;
                    b.col = [255, 0, 0];
                }
                "SUPERKILLER" => {
                    let b = &mut self.objs[i];
                    b.killer = false;
                    b.superkiller = true;
                    b.t_super = 6.0;
                    b.col = [255, 0, 0];
                }
                _ => {}
            }
        }
    }

    /// The paddle's SetBallVelocityNormal: the further from the middle, the flatter.
    fn paddle_bounce(p: &Obj, b: &mut Obj) {
        let dst = p.x + p.w / 2.0 - b.x - b.w / 2.0;
        let mut t = (dst.abs() / (p.w / 2.0)).clamp(0.15, 0.75);
        if dst > 0.0 {
            t = -t;
        }
        b.vx = t;
        b.vy = -(1.0 - t * t).sqrt();
    }

    /// The block's SetBallVelocityNormal by the side the ball came in from.
    fn block_bounce(k: &Obj, b: &mut Obj) {
        let side = k.side(b);
        let t = match side {
            "TOP" | "BOTTOM" => {
                let dst = k.x + k.w / 2.0 - b.x - b.w / 2.0;
                let t = (dst.abs() / (k.w / 2.0)).clamp(0.15, 0.75);
                if b.vx < 0.0 { -t } else { t }
            }
            "LEFT" | "RIGHT" => {
                let dst = k.y + k.h / 2.0 - b.y - b.h / 2.0;
                let t = (dst.abs() / (k.h / 2.0)).clamp(0.35, 0.9);
                if b.vy < 0.0 { -t } else { t }
            }
            _ => return,
        };
        let s = (1.0 - t * t).max(0.0).sqrt();
        match side {
            "TOP" => {
                b.vx = t;
                b.vy = -s;
            }
            "BOTTOM" => {
                b.vx = t;
                b.vy = s;
            }
            "LEFT" => {
                b.vx = -s;
                b.vy = t;
            }
            _ => {
                b.vx = s;
                b.vy = t;
            }
        }
    }

    /// One object's Collide against another.
    fn collide(&mut self, a: usize, b: usize, keys: &ButtonInput<KeyCode>) {
        let ka = self.objs[a].kind;
        let kb = self.objs[b].kind;
        match ka {
            Kind::Ball => {
                if matches!(kb, Kind::Wall(_) | Kind::Paddle) {
                    self.snd(0);
                }
                if let Kind::Wall(s) = kb {
                    match s {
                        WallSide::Bottom => {
                            self.ball_fell();
                            self.kill(a);
                        }
                        WallSide::Top => self.objs[a].vy = -self.objs[a].vy,
                        _ => self.objs[a].vx = -self.objs[a].vx,
                    }
                }
            }
            Kind::Block => {
                if kb != Kind::Ball {
                    return;
                }
                let balls = self.ball_count;
                let ball = self.objs[b].clone();
                {
                    let k = &mut self.objs[a];
                    k.score_mult = ball.ball_mult(balls);
                    if k.unbreakable {
                        k.score_mult = 2.0;
                    } else {
                        k.score_mult *= 1.0 + 0.5 * (k.str_max - 1) as f32;
                    }
                }
                let unbreak = self.objs[a].unbreakable;
                if ((ball.killer || keys.pressed(KeyCode::KeyZ)) && !unbreak) || ball.superkiller {
                    self.kill(a);
                    return;
                }
                let k = self.objs[a].clone();
                Self::block_bounce(&k, &mut self.objs[b]);
                if unbreak {
                    self.snd(2);
                    return;
                }
                self.objs[a].str_cur -= 1;
                if self.objs[a].str_cur == 0 {
                    self.kill(a);
                    return;
                }
                let broken_tex = self.defs.broken_tex.clone();
                let k = &mut self.objs[a];
                if !k.broken {
                    k.tex = broken_tex;
                    k.broken = true;
                }
                let f = (128.0 + 127.0 * k.str_cur as f32 / k.str_max as f32) / 255.0;
                k.col = k.full_col.map(|c| (c as f32 * f) as u8);
                self.snd(0);
            }
            Kind::Paddle => {
                if kb != Kind::Ball {
                    return;
                }
                let py = self.objs[a].y;
                let bh = self.objs[b].h;
                self.objs[b].y = self.objs[b].y.min(py - bh - 2.0);
                if self.objs[a].magnet && !self.objs[a].has_ball {
                    self.paddle = a;
                    self.get_ball(b);
                } else {
                    let p = self.objs[a].clone();
                    Self::paddle_bounce(&p, &mut self.objs[b]);
                }
            }
            Kind::Powerup => {
                if kb == Kind::Paddle {
                    let power = self.objs[a].power;
                    self.power_up(power);
                    self.kill(a);
                } else if matches!(kb, Kind::Wall(_)) {
                    self.kill(a);
                }
            }
            Kind::Wall(_) => {}
        }
    }

    /// UpdateChild: move, then collide with everything it now touches (and step back).
    fn update_child(&mut self, i: usize, nx: f32, ny: f32, keys: &ButtonInput<KeyCode>) {
        {
            let o = &mut self.objs[i];
            o.px = o.x;
            o.py = o.y;
            o.x = nx;
            o.y = ny;
            if !o.collidable {
                return;
            }
        }
        let mut j = 0;
        while j < self.objs.len() && i < self.objs.len() {
            if j != i {
                let (a, b) = (&self.objs[i], &self.objs[j]);
                let skip = b.kind == Kind::Powerup
                    || (a.kind == Kind::Powerup && b.kind == Kind::Block)
                    || (a.kind == Kind::Ball && b.kind == Kind::Ball);
                if !skip && b.collidable && a.colliding(b) {
                    self.collide(i, j, keys);
                    if i < self.objs.len() && j < self.objs.len() {
                        self.collide(j, i, keys);
                    }
                    if i < self.objs.len() {
                        let o = &mut self.objs[i];
                        o.x = o.px;
                        o.y = o.py;
                    }
                }
            }
            j += 1;
        }
    }

    fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>) {
        if !self.created && self.running {
            self.create_game();
        }
        if !self.running {
            return;
        }
        crate::tnmgames::played("BREAKOUT", dt);
        if let Some(t) = self.pending_ball.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.pending_ball = None;
                self.create_paddle_ball();
            }
        }
        let d = dt * 100.0;
        let mut i = 0;
        while i < self.objs.len() {
            if self.objs[i].remove && self.objs[i].remove_timer <= 0.0 {
                self.objs.remove(i);
                for o in self.objs.iter_mut() {
                    if let Some(b) = o.ball {
                        o.ball = if b == i { None } else if b > i { Some(b - 1) } else { Some(b) };
                    }
                }
                self.fix_paddle();
                continue;
            }
            self.update_obj(i, d, keys);
            i += 1;
        }
    }

    /// BreakoutObject.Update and each kind's PostUpdate.
    fn update_obj(&mut self, i: usize, d: f32, keys: &ButtonInput<KeyCode>) {
        if self.objs[i].remove {
            self.objs[i].remove_timer -= d;
        }
        if self.objs[i].stat {
            return;
        }
        let (dx, dy) = {
            let o = &self.objs[i];
            if o.moving { (o.vx * o.vs * d, o.vy * o.vs * d) } else { (0.0, 0.0) }
        };
        let (nx, ny) = (self.objs[i].x + dx, self.objs[i].y + dy);
        self.update_child(i, nx, ny, keys);
        if i >= self.objs.len() {
            return;
        }
        let secs = d / 100.0;
        let speed = self.defs.ball_speed;
        match self.objs[i].kind {
            Kind::Ball => {
                let b = &mut self.objs[i];
                if b.fast {
                    b.t_fast -= secs;
                    if b.t_fast <= 0.0 {
                        b.fast = false;
                        b.vs = speed;
                    }
                }
                if b.slow {
                    b.t_slow -= secs;
                    if b.t_slow <= 0.0 {
                        b.slow = false;
                        b.vs = speed;
                    }
                }
                if b.killer {
                    b.t_killer -= secs;
                    if b.t_killer <= 0.0 {
                        b.killer = false;
                        b.col = [255, 255, 255];
                    }
                }
                if b.superkiller {
                    b.t_super -= secs;
                    if b.t_super <= 0.0 {
                        b.superkiller = false;
                        b.col = [255, 255, 255];
                    }
                }
            }
            Kind::Block => {
                let b = &mut self.objs[i];
                if b.remove && b.remove_amt > 0.0 {
                    let k = (b.remove_amt - b.remove_timer) / b.remove_amt;
                    b.x = b.old.0 + b.old.2 / 4.0 * k;
                    b.y = b.old.1 + b.old.3 / 4.0 * k;
                    b.w = b.old.2 * (1.0 - 0.5 * k);
                    b.h = b.old.3 * (1.0 - 0.5 * k);
                }
            }
            Kind::Paddle => {
                let left = keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA);
                let right = keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD);
                let space = keys.pressed(KeyCode::Space);
                let p = &mut self.objs[i];
                p.vx = (right as i32 - left as i32) as f32;
                if p.invert {
                    p.t_invert -= secs;
                    p.vx = -p.vx;
                    if p.t_invert <= 0.0 {
                        p.invert = false;
                    }
                }
                if p.has_ball {
                    if let Some(bi) = p.ball {
                        let (px, py, off) = (p.x, p.y, p.ball_off);
                        if bi < self.objs.len() {
                            self.objs[bi].x = off.0 + px;
                            self.objs[bi].y = off.1 + py;
                            if space {
                                let p = self.objs[i].clone();
                                Self::paddle_bounce(&p, &mut self.objs[bi]);
                                self.objs[bi].collidable = true;
                                self.objs[bi].vs = speed;
                                self.objs[i].has_ball = false;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// VirtualKeyPressed / KeyPressed.
    fn keys(&mut self, keys: &ButtonInput<KeyCode>, typed: &[char]) {
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
            if !self.running && !self.paused && !self.passcode {
                self.running = true;
                if self.game_over || !self.created {
                    self.game_over = false;
                    self.init_game();
                    self.create_game();
                }
            } else if !self.running && self.passcode {
                if !self.passcode_str.is_empty() {
                    let code = self.passcode_str.clone();
                    let hit = (0..self.levels.count).find(|&i| self.levels.order.get(i).and_then(|&li| self.levels.levels.get(li)).is_some_and(|l| l.passcode == code));
                    if let Some(l) = hit {
                        self.init_game();
                        self.level = l;
                        self.create_game();
                        self.running = true;
                        self.passcode = false;
                    } else {
                        self.passcode_str.clear();
                    }
                } else {
                    self.passcode = false;
                }
            }
        }
        if keys.just_pressed(KeyCode::KeyP) && !self.passcode {
            if self.running != self.paused {
                self.running = !self.running;
                self.paused = !self.paused;
            } else if !self.running {
                self.game_over = false;
                self.passcode = true;
                self.passcode_str.clear();
            }
        } else if self.passcode && !self.running {
            if keys.just_pressed(KeyCode::Backspace) {
                self.passcode_str.pop();
            }
            for &c in typed {
                let c = if c == '-' || c == ' ' { '_' } else { c };
                if c.is_ascii_alphanumeric() || c == '_' {
                    self.passcode_str.push(c.to_ascii_uppercase());
                }
            }
        }
        if keys.just_pressed(KeyCode::End) && self.running {
            let balls: Vec<usize> = (0..self.objs.len()).filter(|&i| self.objs[i].kind == Kind::Ball).collect();
            for b in balls {
                self.ball_fell();
                self.kill(b);
            }
        }
    }

    fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let (ox, oy) = (x as f32, y as f32);
        // BreakoutWindow's client pictures, cols x rows, each at its own size.
        let mut ty = y;
        for r in 0..self.bg_rows.max(1) {
            let mut tx = x;
            let mut row_h = 0;
            for c in 0..self.bg_cols.max(1) {
                let Some(p) = self.bg.get(r * self.bg_cols.max(1) + c).cloned() else { continue };
                if let Some(t) = u.hud.tex(u.assets, &p) {
                    let w = (t.w as i32).min(x + self.w as i32 - tx);
                    let h = (t.h as i32).min(y + self.h as i32 - ty);
                    if w > 0 && h > 0 {
                        u.c.texture(tx, ty, w, h, 0, 0, &t, Style::Normal, [255, 255, 255]);
                    }
                    tx += t.w as i32;
                    row_h = row_h.max(t.h as i32);
                }
            }
            ty += row_h;
        }
        for o in &self.objs {
            if !o.visible {
                continue;
            }
            let mut col = o.col;
            if let Kind::Wall(_) = o.kind {
                col = if self.running { [255, 255, 255] } else { [64, 64, 64] };
            }
            let style = o.style;
            let r = (ox + o.x.trunc(), oy + o.y.trunc(), o.w.trunc(), o.h.trunc());
            match o.kind {
                Kind::Ball if o.override_draw => stretch(u, &o.tex, r, None, style, col),
                Kind::Paddle if o.override_draw => stretch(u, &o.tex, (r.0 - 1.0, r.1, r.2 + 2.0, r.3), None, style, col),
                _ => stretch(u, &o.tex, r, None, style, col),
            }
            if o.kind == Kind::Powerup {
                if let Some(f) = self.defs.letter_font.clone() {
                    u.text(&f, (ox + o.w / 3.0 + o.x) as i32, (oy + o.h / 5.0 + o.y) as i32, o.w as i32, o.h as i32, HAlign::Left, VAlign::Top, o.letter_col, o.letter);
                }
            }
        }
        if !self.running {
            // A blue wash over the board and the message.
            u.c.fill(x, y, self.w as i32, self.h as i32, Style::Translucent, [0, 0, 96]);
            let s = &self.strs;
            let msg = if self.paused {
                format!("{}\n{}", s[4], s[5])
            } else if self.game_over {
                format!("{}\n{}", s[6], s[7])
            } else if self.passcode {
                format!("{}\n{}", s[8], self.passcode_str)
            } else {
                s[9].clone()
            };
            if let Some(f) = self.overlay_font.clone() {
                u.wrapped(&f, x, y, self.w as i32, self.h as i32, HAlign::Center, VAlign::Center, [255, 255, 255], &msg);
            }
            return;
        }
        // The scoreboard right of the board.
        let (sx, sy0, sw, sp) = self.sb;
        let sx = (ox + self.w + sx) as i32;
        let mut sy = oy + sy0;
        let name = self.level_idx(self.level).and_then(|i| self.levels.levels.get(i)).map(|l| (l.name.clone(), l.passcode.clone())).unwrap_or_default();
        if let Some(f) = self.sb_fonts.0.clone() {
            let lines = name.0.lines().count().max(1) as f32;
            u.wrapped(&f, sx, sy as i32, sw as i32, (f.height as f32 * lines) as i32 + 2, HAlign::Left, VAlign::Top, [255, 0, 0], &name.0);
            sy += f.height as f32 * lines * 1.25 + sp;
        }
        if let Some(f) = self.sb_fonts.1.clone() {
            for line in [fmt1(&self.strs[0], self.lives), fmt1(&self.strs[1], self.level + 1), fmt1(&self.strs[2], self.score)] {
                u.text(&f, sx, sy as i32, sw as i32, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], &line);
                sy += f.height as f32 + sp;
            }
            sy += f.height as f32 + sp;
            if !name.1.is_empty() {
                u.text(&f, sx, sy as i32, sw as i32, f.height, HAlign::Left, VAlign::Top, [255, 255, 0], &self.strs[3]);
                sy += f.height as f32 + sp;
                let pf = self.sb_fonts.2.clone().unwrap_or(f);
                u.text(&pf, sx, sy as i32, sw as i32, pf.height, HAlign::Left, VAlign::Top, [255, 255, 0], &format!("  {}", name.1));
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ Tetris

/// The mod's pieces (TetrisWindow.DefaultShapes, run through uscript.rs: each AddPosition(piece, x, y)).
struct ShapeHost(Vec<Vec<(i32, i32)>>);

/// The pieces with their widths and heights in cells.
fn load_shapes(classes: &mut unreal_pkg::classes::ClassDb) -> Vec<(Vec<(i32, i32)>, i32, i32)> {
    let mut shapes = ShapeHost(Vec::new());
    if let Some(src) = classes.script_text("TetrisWindow") {
        let mut s = crate::uscript::Script::default();
        s.add_source(&src);
        s.call(&mut shapes, "defaultshapes", &[]);
    }
    shapes
        .0
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let w = p.iter().map(|q| q.0 + 1).max().unwrap_or(1);
            let h = p.iter().map(|q| q.1 + 1).max().unwrap_or(1);
            (p, w, h)
        })
        .collect()
}

impl crate::uscript::Host for ShapeHost {
    fn call(&mut self, name: &str, args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        if name == "addposition" && args.len() >= 3 {
            let (p, x, y) = (args[0].int().clamp(0, 9) as usize, args[1].int() as i32, args[2].int() as i32);
            if self.0.len() <= p {
                self.0.resize(p + 1, Vec::new());
            }
            if self.0[p].len() < 10 {
                self.0[p].push((x, y));
            }
            return Some(crate::uscript::V::None);
        }
        None
    }
    fn get(&mut self, _name: &str) -> crate::uscript::V {
        crate::uscript::V::None
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::None
    }
}

#[derive(Clone, Copy, Default)]
struct PBlock {
    loc: (i32, i32),
    rel: (i32, i32),
    col: Rgb,
    id: usize,
}

impl PBlock {
    fn pos(&self) -> (i32, i32) {
        (self.loc.0 + self.rel.0, self.loc.1 + self.rel.1)
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    occupied: bool,
    col: Rgb,
    id: usize,
}

/// TetrisWindow: a 10 x 18 well; Left / Right or A / D move, Up / W turns, Down / S drops faster, Ctrl drops the
/// piece; T or Start starts (and pauses). Every cleared line scores (lines left x 10 x level, achTetris); a level
/// every LinesPerLevel lines, faster, with new shades.
pub struct Tetris {
    started: bool,
    shapes: Vec<(Vec<(i32, i32)>, i32, i32)>,
    last_played: Vec<i32>,
    colors: Vec<Rgb>,
    base_colors: Vec<Rgb>,
    cells: [Cell; 180],
    player: Vec<PBlock>,
    pending: Vec<PBlock>,
    pending_shape: Option<usize>,
    level: i32,
    lines_left: i32,
    speed: f32,
    time_left: f32,
    key_time: f32,
    rotate_up: bool,
    lines_per_level: i32,
    speed_base: f32,
    speed_level: f32,
    speed_key: f32,
    score: i32,
    current: i32,
    fast_piece: i32,
    block_tex: String,
    block_size: (i32, i32),
    board_pos: (i32, i32),
    sb: (f32, f32, f32, f32),
    fonts: (Option<Arc<UiFont>>, Option<Arc<UiFont>>),
    strs: [String; 3],
    rng: Rng,
    sounds: [String; 3],
    client: (i32, i32),
}

impl Tetris {
    fn new(assets: &mut RuntimeAssets, seed: u64) -> Tetris {
        let d = assets.classes.defaults("TetrisWindow");
        let shapes = load_shapes(&mut assets.classes);
        let mut colors = Vec::new();
        for i in 0..20 {
            match color_at(&d, "ColorList", i) {
                Some(c) => colors.push(c),
                None => break,
            }
        }
        let n_colors = num(&d, "ColorListLength", colors.len() as f32) as usize;
        colors.truncate(n_colors.max(1).min(colors.len().max(1)));
        if colors.is_empty() {
            colors.push([255, 255, 255]);
        }
        let n = shapes.len();
        Tetris {
            started: false,
            last_played: vec![0; n],
            shapes,
            base_colors: colors.clone(),
            colors,
            cells: [Cell::default(); 180],
            player: Vec::new(),
            pending: Vec::new(),
            pending_shape: None,
            level: 1,
            lines_left: num(&d, "LinesPerLevel", 10.0) as i32,
            speed: 0.0,
            time_left: 1.0,
            key_time: 0.0,
            rotate_up: true,
            lines_per_level: num(&d, "LinesPerLevel", 10.0) as i32,
            speed_base: num(&d, "SpeedBase", 1.0),
            speed_level: num(&d, "SpeedLevelIncrease", 0.5),
            speed_key: num(&d, "SpeedKeyMultiplier", 10.0),
            score: 0,
            current: 0,
            fast_piece: -1,
            block_tex: path(&d, "BlockTexture"),
            block_size: pair(&d, "BlockSize", (24, 24)),
            board_pos: pair(&d, "BoardPos", (11, 0)),
            sb: (num(&d, "scoreboardX", 16.0), num(&d, "scoreboardY", 0.0), num(&d, "scoreboardW", 160.0), num(&d, "scoreboardSpacer", 4.0)),
            fonts: (font(assets, &path(&d, "scoreboardHeaderFont")), font(assets, &path(&d, "scoreboardFont"))),
            strs: [text(&d, "scoreboardHeaderStr"), text(&d, "scoreboardLevelStr"), text(&d, "scoreboardScoreStr")],
            rng: Rng(seed | 1),
            sounds: ["TNMGUI.Tetris_Rotate".into(), "TNMGUI.Tetris_Land".into(), "TNMGUI.Tetris_RemoveLine".into()],
            client: (num(&d, "ClientWidth", 287.0) as i32, num(&d, "ClientHeight", 480.0) as i32),
        }
    }

    pub fn title_suffix(&self) -> String {
        format!(" - {}/{}", self.score, crate::tnmgames::high("Tetris"))
    }

    fn idx(row: i32, col: i32) -> usize {
        (row * 10 + col).clamp(0, 179) as usize
    }
    fn occupied(&self, row: i32, col: i32) -> bool {
        (0..18).contains(&row) && (0..10).contains(&col) && self.cells[Self::idx(row, col)].occupied
    }

    /// GameOver.
    fn game_over(&mut self) {
        self.cells = [Cell::default(); 180];
        self.player.clear();
        self.pending.clear();
        self.speed = 0.0;
        self.started = false;
        self.score = 0;
        self.level = 1;
        self.pending_shape = None;
        self.last_played.iter_mut().for_each(|v| *v = 0);
    }

    fn piece(&self, shape: usize, x0: i32) -> Vec<PBlock> {
        let Some((pos, w, h)) = self.shapes.get(shape) else { return Vec::new() };
        let col = self.colors.get(shape).copied().unwrap_or([255, 255, 255]);
        pos.iter().map(|&rel| PBlock { rel, loc: (x0 - w / 2, -h), col, id: shape }).collect()
    }

    fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>) {
        if !self.started || self.shapes.is_empty() {
            return;
        }
        crate::tnmgames::played("TETRIS", dt);
        let speed_down = keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS);
        let mut rotate = keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW);
        self.rotate_up = self.rotate_up || !rotate;
        rotate = rotate && self.rotate_up;
        let left = keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA);
        let right = keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD);
        let drop = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        if drop && self.fast_piece == -1 {
            self.fast_piece = self.current;
        } else if !drop {
            self.fast_piece = -1;
        }
        self.key_time -= dt;
        if self.key_time <= 0.0 {
            if rotate {
                self.rotate_up = false;
            }
            let entered = self.player.first().is_some_and(|b| b.loc.1 >= 0);
            if rotate && !self.player.is_empty() {
                let off = self.player.iter().map(|b| b.rel.1).max().unwrap_or(0).max(0);
                let ok = self.player.iter().all(|b| {
                    let r = (off - b.rel.1, b.rel.0);
                    let (x, y) = (b.loc.0 + r.0, b.loc.1 + r.1);
                    (0..10).contains(&x) && (!entered || ((0..18).contains(&y) && !self.occupied(y, x)))
                });
                if ok {
                    for b in self.player.iter_mut() {
                        b.rel = (off - b.rel.1, b.rel.0);
                    }
                    sound(&self.sounds[0]);
                }
            }
            let minx = self.player.iter().map(|b| b.pos().0).min().unwrap_or(10).min(10);
            let mut move_left = left && minx > 0;
            let maxx = self.player.iter().map(|b| b.pos().0).max().unwrap_or(0).max(0);
            let mut move_right = right && maxx < 9;
            if entered {
                for b in &self.player {
                    let (x, y) = b.pos();
                    if move_left && self.occupied(y, x - 1) {
                        move_left = false;
                    }
                    if move_right && self.occupied(y, x + 1) {
                        move_right = false;
                    }
                }
            }
            for b in self.player.iter_mut() {
                if move_left {
                    b.loc.0 -= 1;
                }
                if move_right {
                    b.loc.0 += 1;
                }
            }
            self.key_time = 0.08;
        }
        let mut mult = self.speed_base + self.speed;
        if speed_down && mult < self.speed_key {
            mult = self.speed_key;
        }
        self.time_left -= dt * mult;
        let fast = drop && self.fast_piece == self.current && ((dt * 100.0) % 3.0 > 0.0);
        if self.time_left > 0.0 && !fast {
            return;
        }
        let mut landed = self.player.is_empty();
        for b in &self.player {
            let (x, y) = b.pos();
            if !landed && y + 1 >= 0 && (y == 17 || self.occupied(y + 1, x)) {
                landed = true;
            }
        }
        let mut sound_land = false;
        let mut sound_line = false;
        if landed {
            for b in self.player.clone() {
                let (x, y) = b.pos();
                if y < 0 {
                    self.game_over();
                    return;
                }
                self.cells[Self::idx(y, x)] = Cell { occupied: true, col: b.col, id: b.id };
            }
            // The next piece: the waiting one (a shape not seen for twice the number of shapes comes next).
            let shape = self.pending_shape.unwrap_or_else(|| self.rng.rand(self.shapes.len() as i32) as usize);
            self.player = self.piece(shape, 5);
            self.last_played.iter_mut().for_each(|v| *v += 1);
            let n = self.shapes.len() as i32;
            let mut next = self.last_played.iter().position(|&v| v > n * 2);
            if next.is_none() {
                next = Some(self.rng.rand(n) as usize);
            }
            let next = next.unwrap();
            self.last_played[next] = 0;
            self.pending_shape = Some(next);
            self.pending = self.piece(next, 0);
            sound_land = true;
            self.current += 1;
        } else {
            for b in self.player.iter_mut() {
                b.loc.1 += 1;
            }
        }
        // Full lines, from the bottom up (a line is checked again after the rows above fall into it).
        let mut cleared = 0;
        let mut i = 1;
        while i <= 18 {
            let row = 18 - i;
            if (0..10).all(|c| self.cells[Self::idx(row, c)].occupied) {
                self.score += self.lines_left * 10 * self.level;
                crate::tnmgames::offer("Tetris", self.score as i64);
                crate::tnmach::unlock("achTetris");
                cleared += 1;
                sound_line = true;
                for r in (0..=row).rev() {
                    for c in 0..10 {
                        self.cells[Self::idx(r, c)] = if r == 0 { Cell::default() } else { self.cells[Self::idx(r - 1, c)] };
                    }
                }
                continue;
            }
            i += 1;
        }
        if cleared >= self.lines_left {
            let over = cleared - self.lines_left;
            self.lines_left = self.lines_per_level - over;
            self.speed += self.speed_level;
            self.level += 1;
            for (k, c) in self.colors.iter_mut().enumerate() {
                let base = self.base_colors[k];
                for ch in 0..3 {
                    if base[ch] != 0 {
                        c[ch] = (92.0 + self.rng.rand(164) as f32 * (base[ch] as f32 / 255.0)) as u8;
                    }
                }
            }
            for cell in self.cells.iter_mut() {
                cell.col = self.colors.get(cell.id).copied().unwrap_or(cell.col);
            }
            for b in self.player.iter_mut() {
                b.col = self.colors.get(b.id).copied().unwrap_or(b.col);
            }
        } else {
            self.lines_left -= cleared;
        }
        self.time_left = 1.0;
        if sound_line {
            sound(&self.sounds[2]);
        } else if sound_land {
            sound(&self.sounds[1]);
        }
    }

    fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let (bw, bh) = self.block_size;
        let tex = self.block_tex.clone();
        let cell_at = |px: i32, py: i32| (x + px, y + py);
        let draw_block = |u: &mut Ui<W>, (px, py): (i32, i32), col: Rgb, style: Style| {
            stretch(u, &tex, (px as f32, py as f32, bw as f32, bh as f32), Some((0, 0, 32, 32)), style, col);
        };
        let to_coord = |p: (i32, i32)| ((p.0 + 1) * bw + self.board_pos.0, (p.1 + 1) * bh + self.board_pos.1);
        u.c.fill(x, y, self.client.0, self.client.1, Style::Normal, [0, 0, 0]);
        // The grey frame.
        for i in 0..=10 {
            draw_block(u, cell_at(11 + i * 24, 24), [48, 48, 48], Style::Normal);
            draw_block(u, cell_at(11 + i * 24, 480), [48, 48, 48], Style::Normal);
        }
        for i in 0..19 {
            draw_block(u, cell_at(11, 24 + i * 24), [48, 48, 48], Style::Normal);
            draw_block(u, cell_at(275, 24 + i * 24), [48, 48, 48], Style::Normal);
        }
        draw_block(u, cell_at(275, 24 + 19 * 24), [48, 48, 48], Style::Normal);
        for r in 0..18 {
            for c in 0..10 {
                let cell = self.cells[Self::idx(r, c)];
                if cell.occupied {
                    let (px, py) = to_coord((c, r));
                    draw_block(u, cell_at(px, py), cell.col, Style::Normal);
                }
            }
        }
        // The ghost: where the piece would land.
        if !self.player.is_empty() {
            let mut drop = 0;
            'down: for j in 1..20 {
                for b in &self.player {
                    let (bx, by) = b.pos();
                    if by + j >= 0 && (by >= 17 || by + j > 17 || self.occupied(by + j, bx)) {
                        break 'down;
                    }
                }
                drop = j;
            }
            for b in &self.player {
                let (bx, by) = b.pos();
                if b.loc.0 >= 0 && b.loc.0 < 10 && b.loc.1 + drop >= 0 && b.loc.1 + drop < 18 {
                    let col = [b.col[0].max(96 * 2) / 2, b.col[1].max(96 * 2) / 2, b.col[2].max(96 * 2) / 2];
                    let col = [col[0].max(96), col[1].max(96), col[2].max(96)];
                    let (px, py) = to_coord((bx, by + drop));
                    draw_block(u, cell_at(px, py), col, Style::Masked);
                }
            }
            for b in &self.player {
                if b.loc.0 >= 0 && b.loc.0 < 10 && b.loc.1 >= 0 && b.loc.1 < 18 {
                    let (px, py) = to_coord(b.pos());
                    draw_block(u, cell_at(px, py), b.col, Style::Translucent);
                }
            }
        }
        // The scoreboard, then the next piece under it.
        let (sx, sy0, sw, sp) = self.sb;
        let sx = x + self.client.0 + sx as i32;
        let mut sy = y as f32 + sy0;
        if let Some(f) = self.fonts.0.clone() {
            u.text(&f, sx, sy as i32, sw as i32, f.height, HAlign::Left, VAlign::Top, [0, 0, 255], &self.strs[0]);
            sy += f.height as f32 * 1.25 + sp;
        }
        if let Some(f) = self.fonts.1.clone() {
            for line in [fmt1(&self.strs[1], self.level), fmt1(&self.strs[2], self.score)] {
                u.text(&f, sx, sy as i32, sw as i32, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], &line);
                sy += f.height as f32 + sp;
            }
        }
        for b in self.pending.clone() {
            let (px, py) = to_coord(b.pos());
            draw_block(u, (sx + 16 + px, y + sy0 as i32 + 112 + py), b.col, Style::Normal);
        }
    }


}


// ------------------------------------------------------------------------------------------------ the packet puzzle

/// The mouse over a game: its place in the game's client area, and the left button.
#[derive(Clone, Copy, Default)]
pub struct Pointer {
    pub pos: Option<(i32, i32)>,
    pub down: bool,
    pub pressed: bool,
}

#[derive(Clone)]
struct Piece {
    class: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    horiz: bool,
    vert: bool,
    tex: String,
    size: (i32, i32),
    key: bool,
}

/// The layout calls (AIPuzzleGrid.InitPieces: AddPiece(type, x, y) for the puzzle number).
struct LayoutHost {
    number: i64,
    kinds: Vec<String>,
    out: Vec<(usize, i32, i32)>,
}

impl crate::uscript::Host for LayoutHost {
    fn call(&mut self, name: &str, args: &[crate::uscript::V]) -> Option<crate::uscript::V> {
        if name == "addpiece" && args.len() >= 3 {
            self.out.push((args[0].int().max(0) as usize, args[1].int() as i32, args[2].int() as i32));
            return Some(crate::uscript::V::None);
        }
        None
    }
    fn get(&mut self, name: &str) -> crate::uscript::V {
        if name == "puzzlenumber" {
            return crate::uscript::V::Int(self.number);
        }
        match self.kinds.iter().position(|k| k.eq_ignore_ascii_case(name)) {
            Some(i) => crate::uscript::V::Int(i as i64),
            None => crate::uscript::V::None,
        }
    }
    fn get_index(&mut self, _name: &str, _index: i64, _field: Option<&str>) -> crate::uscript::V {
        crate::uscript::V::None
    }
}

/// The names of an enum in script text, in order.
fn enum_names(src: &str, name: &str) -> Vec<String> {
    let lower = src.to_ascii_lowercase();
    let Some(at) = lower.find(&format!("enum {}", name.to_ascii_lowercase())) else { return Vec::new() };
    let Some(open) = src[at..].find('{') else { return Vec::new() };
    let Some(close) = src[at + open..].find('}') else { return Vec::new() };
    src[at + open + 1..at + open + close]
        .split(',')
        .map(|w| w.split("//").next().unwrap_or("").trim().to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// AIPuzzleWindow (the Applications list opens its debug version, with Switch): slide the packets, each only along
/// its own axis, until the key packet reaches the exit on the right.
pub struct Packets {
    number: i64,
    count: i64,
    pieces: Vec<Piece>,
    bg: Vec<String>,
    bg_cols: usize,
    drag: Option<(usize, (i32, i32), (i32, i32), bool)>,
    done: bool,
    win_t: f32,
    win_len: f32,
    wait_t: f32,
    wait_len: f32,
    col: (Rgb, Rgb),
    finished: bool,
    client: (i32, i32),
    defs: Vec<(String, Piece)>,
    kinds: Vec<String>,
    src: String,
}

impl Packets {
    fn new(assets: &mut RuntimeAssets, seed: u64) -> Packets {
        let wd = assets.classes.defaults("AIPuzzleWindow");
        let gd = assets.classes.defaults("AIPuzzleGrid");
        let kd = assets.classes.defaults("AIPuzzlePieceKey");
        let src = assets.classes.script_text("AIPuzzleGrid").unwrap_or_default();
        let kinds = enum_names(&src, "PieceType");
        let mut defs = Vec::new();
        for k in &kinds {
            let class = format!("AIPuzzlePiece{}", k.trim_start_matches(['p', 'P']));
            let d = assets.classes.defaults(&class);
            defs.push((
                class.clone(),
                Piece {
                    class,
                    x: 0,
                    y: 0,
                    w: num(&d, "gridW", 1.0) as i32,
                    h: num(&d, "gridH", 1.0) as i32,
                    horiz: flag(&d, "bMoveHorizontal"),
                    vert: flag(&d, "bMoveVertical"),
                    tex: path(&d, "displayTexture"),
                    size: (num(&d, "textureWidth", 52.0) as i32, num(&d, "textureHeight", 50.0) as i32),
                    key: k.eq_ignore_ascii_case("pKey"),
                },
            ));
        }
        let bg: Vec<String> = (0..8)
            .map_while(|i| match gd.iter().find(|p| p.name.eq_ignore_ascii_case("backgroundTextures") && p.index == i).map(|p| &p.value) {
                Some(Value::Path(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        // How many layouts the script has: the highest `case n:` of InitPieces' switch, plus one.
        let count = src
            .to_ascii_lowercase()
            .split("case ")
            .skip(1)
            .filter_map(|c| c.split(':').next().and_then(|n| n.trim().parse::<i64>().ok()))
            .max()
            .map_or(13, |m| m + 1);
        let mut p = Packets {
            number: (seed % count.max(1) as u64) as i64,
            count,
            pieces: Vec::new(),
            bg,
            bg_cols: num(&gd, "backgroundTextureCols", 2.0) as usize,
            drag: None,
            done: false,
            win_t: 0.0,
            win_len: num(&kd, "winTime", 1.0),
            wait_t: 0.0,
            wait_len: num(&kd, "wintimewait", 0.6),
            col: (color_at(&kd, "col", 0).unwrap_or([255, 255, 255]), color_at(&kd, "colwin", 0).unwrap_or([255, 0, 255])),
            finished: false,
            client: (num(&wd, "ClientWidth", 468.0) as i32, num(&wd, "ClientHeight", 394.0) as i32),
            defs,
            kinds,
            src,
        };
        p.reset();
        p
    }

    /// ResetPuzzle / InitPieces.
    fn reset(&mut self) {
        let mut host = LayoutHost { number: self.number, kinds: self.kinds.clone(), out: Vec::new() };
        let mut script = crate::uscript::Script::default();
        script.add_source(&self.src);
        script.call(&mut host, "initpieces", &[]);
        self.pieces = host
            .out
            .into_iter()
            .filter_map(|(k, x, y)| {
                let mut p = self.defs.get(k)?.1.clone();
                p.x = x;
                p.y = y;
                Some(p)
            })
            .take(32)
            .collect();
        self.drag = None;
        self.done = false;
        self.finished = false;
        self.win_t = 0.0;
        self.wait_t = self.wait_len;
    }

    fn switch(&mut self) {
        self.number = (self.number + 1) % self.count.max(1);
        self.reset();
    }

    /// GridCoordinates.
    fn coords(x: i32, y: i32) -> (i32, i32) {
        (12 + x * 66, 12 + y * 64)
    }

    /// FindGridSquare (70-unit cells).
    fn cell(p: (i32, i32)) -> (i32, i32) {
        let f = |v: i32| (1..6).find(|i| v < 70 * i).unwrap_or(6) - 1;
        (f(p.0), f(p.1))
    }

    /// AttemptMove: one cell at a time, inside the 6 x 6 grid, onto free cells only.
    fn attempt(&mut self, i: usize, x: i32, y: i32) -> bool {
        let p = &self.pieces[i];
        if self.done || x < 0 || x + p.w > 6 || y < 0 || y + p.h > 6 || (x - p.x).abs() + (y - p.y).abs() > 1 {
            return false;
        }
        let mut cells = [0u8; 36];
        for (j, q) in self.pieces.iter().enumerate() {
            if j != i {
                for c in 0..q.h {
                    for r in 0..q.w {
                        if let Some(v) = cells.get_mut(((q.y + c) * 6 + q.x + r) as usize) {
                            *v += 1;
                        }
                    }
                }
            }
        }
        for c in 0..p.h {
            for r in 0..p.w {
                if cells.get(((y + c) * 6 + x + r) as usize).is_none_or(|v| *v > 0) {
                    return false;
                }
            }
        }
        let p = &mut self.pieces[i];
        p.x = x;
        p.y = y;
        true
    }

    fn tick(&mut self, dt: f32, ptr: Pointer) -> bool {
        crate::tnmgames::played("RUSHHOUR", dt);
        if self.done {
            // The key packet fades to its win colour, a pause, then the win.
            if self.win_t < 1.0 {
                self.win_t = (self.win_t + dt / self.win_len.max(0.01)).min(1.0);
            } else if self.wait_t > 0.0 {
                self.wait_t -= dt;
            } else if !self.finished {
                self.finished = true;
                sound("TNMGUI.AIPuzzle_WinGame");
                crate::tnmach::unlock("achAIPuzzle");
                return true;
            }
            return false;
        }
        let Some(m) = ptr.pos else { return false };
        if ptr.pressed && self.drag.is_none() {
            let hit = self.pieces.iter().position(|p| {
                let (px, py) = Self::coords(p.x, p.y);
                m.0 >= px && m.1 >= py && m.0 < px + p.size.0 && m.1 < py + p.size.1
            });
            if let Some(i) = hit {
                let c = Self::cell(m);
                let p = &self.pieces[i];
                self.drag = Some((i, (c.0 - p.x, c.1 - p.y), (p.x, p.y), false));
                sound("TNMGUI.AIPuzzle_GrabPacket");
            }
        }
        if let Some((i, off, og, mut moved)) = self.drag {
            if !ptr.down {
                self.drag = None;
                if moved {
                    sound("TNMGUI.AIPuzzle_PlacePacket");
                    if self.pieces[i].key && self.pieces[i].x == 4 {
                        self.done = true;
                        self.win_t = 0.0;
                    }
                }
                return false;
            }
            let c = Self::cell(m);
            let (horiz, vert) = (self.pieces[i].horiz, self.pieces[i].vert);
            if horiz {
                let nx = (c.0 - off.0).max(0);
                let (px, py) = (self.pieces[i].x, self.pieces[i].y);
                let step = px + (nx - px).signum();
                if nx != px && self.attempt(i, step, py) {
                    moved = true;
                }
            }
            if vert {
                let ny = (c.1 - off.1).max(0);
                let (px, py) = (self.pieces[i].x, self.pieces[i].y);
                let step = py + (ny - py).signum();
                if ny != py && self.attempt(i, px, step) {
                    moved = true;
                }
            }
            if (self.pieces[i].x, self.pieces[i].y) == og {
                moved = false;
            }
            self.drag = Some((i, off, og, moved));
        }
        false
    }

    fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        let mut tx = x;
        let mut ty = y;
        let mut row_h = 0;
        for (k, b) in self.bg.clone().iter().enumerate() {
            if k > 0 && k % self.bg_cols.max(1) == 0 {
                tx = x;
                ty += row_h;
                row_h = 0;
            }
            if let Some(t) = u.hud.tex(u.assets, b) {
                u.c.texture(tx, ty, t.w as i32, t.h as i32, 0, 0, &t, Style::Normal, [255, 255, 255]);
                tx += t.w as i32;
                row_h = row_h.max(t.h as i32);
            }
        }
        for p in self.pieces.clone() {
            let (px, py) = Self::coords(p.x, p.y);
            let col = if p.key && self.done {
                let k = self.win_t;
                let (a, b) = self.col;
                [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - k) + b[i] as f32 * k) as u8)
            } else if p.key {
                self.col.0
            } else {
                [255, 255, 255]
            };
            stretch(u, &p.tex, ((x + px) as f32, (y + py) as f32, p.size.0 as f32, p.size.1 as f32), None, Style::Masked, col);
        }
    }
}


// ------------------------------------------------------------------------------------------------ the text adventure

/// sprintf with several %s / %d / %i, in order.
fn fmtn(f: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = f;
    let mut next = args.iter();
    while let Some(i) = rest.find('%') {
        let code = rest[i + 1..].chars().next();
        out.push_str(&rest[..i]);
        match code {
            Some('s' | 'd' | 'i') => {
                out.push_str(next.next().copied().unwrap_or(""));
                rest = &rest[i + 2..];
            }
            _ => {
                out.push('%');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The command line's words: plain words, or "quoted text" as one (TextConsoleInterface.ParseRaw).
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The text adventure's words (TextBasedGameInterface / TextConsoleInterface defaults).
struct Words {
    d: Vec<Property>,
}

impl Words {
    fn s(&self, k: &str) -> String {
        text(&self.d, k)
    }
    fn at(&self, k: &str, i: u32) -> String {
        self.d
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(k) && p.index == i)
            .and_then(|p| match &p.value {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }
    fn list(&self, k: &str) -> Vec<String> {
        (0..256).map_while(|i| Some(self.at(k, i)).filter(|s| !s.is_empty())).collect()
    }
}

/// tnmTextGameWindow: The Nameless Text Adventure - a console (log above, command line below) running
/// TextBasedGameInterface: name, surname and age, then a dark place, a handful of moves, and the grue.
pub struct TextGame {
    w: Words,
    commands: Vec<String>,
    help: Vec<String>,
    lines: Vec<String>,
    input: String,
    history: Vec<String>,
    hist_at: usize,
    stage: Stage,
    first: String,
    last: String,
    age: i64,
    moves_left: i32,
    done: bool,
    font: Option<Arc<UiFont>>,
    bg: Vec<String>,
    bg_cols: usize,
    client: (i32, i32),
    rng: Rng,
    blink: f32,
}

#[derive(PartialEq, Clone, Copy)]
enum Stage {
    First,
    Last,
    Age,
    Confirm,
    NewGame,
    Playing,
}

impl TextGame {
    fn new(assets: &mut RuntimeAssets, seed: u64) -> TextGame {
        let d = assets.classes.defaults("TextBasedGameInterface");
        let wd = assets.classes.defaults("tnmTextGameWindow");
        let cd = assets.classes.defaults("TextConsole");
        let w = Words { d };
        let font = font(assets, &path(&cd, "consoleFont")).or_else(|| font(assets, "FontMenuSmall"));
        let bg: Vec<String> = (0..16)
            .map_while(|i| match wd.iter().find(|p| p.name.eq_ignore_ascii_case("clientTextures") && p.index == i).map(|p| &p.value) {
                Some(Value::Path(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let mut g = TextGame {
            commands: w.list("command_list"),
            help: (0..256).map(|i| w.at("command_help", i)).take_while(|_| true).take(64).collect(),
            w,
            lines: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            hist_at: 0,
            stage: Stage::First,
            first: String::new(),
            last: String::new(),
            age: 0,
            moves_left: 0,
            done: false,
            font,
            bg,
            bg_cols: num(&wd, "textureCols", 3.0) as usize,
            client: (num(&wd, "ClientWidth", 600.0) as i32, num(&wd, "ClientHeight", 400.0) as i32),
            rng: Rng(seed | 1),
            blink: 0.0,
        };
        // Init: strInit, then the first name.
        let init = g.w.s("strInit");
        g.say(&init);
        g.prompt_first();
        g
    }

    fn say(&mut self, s: &str) {
        self.lines.push(s.to_string());
        if self.lines.len() > 2000 {
            self.lines.drain(..500);
        }
    }
    fn prompt_first(&mut self) {
        let s = self.w.s("strNameFirst");
        self.say(&s);
        self.stage = Stage::First;
    }
    fn prompt_new_game(&mut self) {
        let (a, b) = (self.w.at("strNewGame", 0), self.w.at("strNewGame", 1));
        self.say(&a);
        self.say(&b);
        self.stage = Stage::NewGame;
    }
    fn death(&mut self) {
        let s = fmtn(&self.w.s("strDeath"), &[&self.first]);
        self.say(&s);
        crate::tnmach::unlock("achTheNamelessAdventure");
    }
    fn one_of(&mut self, k: &str, n: u32) -> String {
        let i = self.rng.rand(n as i32) as u32;
        self.w.at(k, i)
    }
    fn command_bad(&mut self, cmd: &str) {
        // CommandBad: one of ten replies (the seventh is two lines).
        match self.rng.rand(10) {
            6 => {
                let (a, b) = (self.w.at("strCommandBad", 6), self.w.at("strCommandBad", 7));
                self.say(&a);
                self.say(&b);
            }
            r => {
                let idx = [0, 1, 2, 3, 4, 5, 6, 8, 9, 10][r as usize];
                let s = fmtn(&self.w.at("strCommandBad", idx), &[cmd]);
                self.say(&s);
            }
        }
    }
    fn command_not_found(&mut self, cmd: &str) {
        let r = self.rng.rand(10) as u32;
        let s = fmtn(&self.w.at("strCommandUnknown", r), &[cmd, cmd]);
        self.say(&s);
    }
    fn yes_no(&self, raw: &str) -> Option<bool> {
        let up = raw.to_uppercase();
        if up.eq_ignore_ascii_case(&self.w.s("strYes")) {
            Some(true)
        } else if up.eq_ignore_ascii_case(&self.w.s("strNo")) {
            Some(false)
        } else {
            None
        }
    }

    /// One line typed (EditActivated -> ParseRaw -> Parse).
    fn enter(&mut self, raw: &str) {
        if self.done {
            return;
        }
        let ws = words(raw);
        let cmd = ws.first().cloned().unwrap_or_default();
        let args: Vec<String> = ws.iter().skip(1).cloned().collect();
        self.say(&format!(">{raw}"));
        match self.stage {
            Stage::First => {
                self.first = raw.to_string();
                let s = self.w.s("strNameLast");
                self.say(&s);
                self.stage = Stage::Last;
                return;
            }
            Stage::Last => {
                self.last = raw.to_string();
                let s = self.w.s("strAge");
                self.say(&s);
                self.stage = Stage::Age;
                return;
            }
            Stage::Age => {
                self.age = raw.trim().parse::<i64>().unwrap_or(0);
                if self.age < 0 {
                    let s = self.w.s("strAgeNegative");
                    self.say(&s);
                    return;
                }
                let a = fmtn(&self.w.at("strConfirm", 0), &[&self.first, &self.last, &self.age.to_string()]);
                let b = self.w.at("strConfirm", 1);
                self.say(&a);
                self.say(&b);
                self.stage = Stage::Confirm;
                return;
            }
            Stage::Confirm => {
                match self.yes_no(raw) {
                    Some(true) => {
                        self.moves_left = self.rng.rand(6) + 3;
                        self.stage = Stage::Playing;
                        let s = self.w.s("strDark");
                        self.say(&s);
                        let early = if self.first.eq_ignore_ascii_case("HELEN") && self.last.eq_ignore_ascii_case("KELLER") {
                            Some("strHelen")
                        } else if self.age < 12 {
                            Some("strAgeYoung")
                        } else if self.age > 60 {
                            Some("strAgeOld")
                        } else {
                            None
                        };
                        match early {
                            Some(k) => {
                                let s = self.w.s(k);
                                self.say(&s);
                                self.death();
                                self.prompt_new_game();
                            }
                            None => {
                                let s = self.w.s("strHelpComs");
                                self.say(&s);
                            }
                        }
                    }
                    Some(false) => self.prompt_first(),
                    None => {
                        let s = self.w.s("strYesNoBad");
                        self.say(&s);
                    }
                }
                return;
            }
            Stage::NewGame => {
                match self.yes_no(raw) {
                    Some(true) => self.prompt_first(),
                    Some(false) => {
                        let s = self.w.s("strEnd");
                        self.say(&s);
                        self.done = true;
                    }
                    None => {
                        let s = self.w.s("strYesNoBad");
                        self.say(&s);
                    }
                }
                return;
            }
            Stage::Playing => {}
        }
        let up = raw.to_uppercase();
        let mut cmd_up = cmd.to_uppercase();
        if (0..5).any(|i| up.eq_ignore_ascii_case(&self.w.at("strInsults", i))) {
            cmd_up.clear();
            self.moves_left = 0;
            let s = self.w.s("strInsultReply");
            self.say(&s);
        }
        let c = |i: usize| self.commands.get(i).map(|s| s.to_uppercase()).unwrap_or_default();
        let a0 = args.first().cloned().unwrap_or_default();
        let mut handled = true;
        if cmd_up.is_empty() {
        } else if cmd_up == c(0) {
            if args.is_empty() {
                let s = self.w.s("strMoveReply");
                self.say(&s);
            }
            self.attempt_move(&a0);
        } else if cmd_up == c(1) {
            let s = if args.len() == 1 && a0.eq_ignore_ascii_case(&self.w.s("strGrue")) { self.w.at("strInfoReply", 0) } else { self.w.at("strInfoReply", 1) };
            self.say(&s);
        } else if cmd_up == c(2) {
            let s = self.w.s("strAttackReply");
            self.say(&s);
            self.moves_left -= 1;
        } else if cmd_up == c(3) {
            if args.len() == 1 && a0.eq_ignore_ascii_case(&self.w.s("strClothes")) {
                let s = self.w.at("strInventoryReply", 0);
                self.say(&s);
            } else {
                let (a, b) = (self.w.at("strInventoryReply", 1), self.w.at("strInventoryReply", 2));
                self.say(&a);
                self.say(&b);
            }
        } else if cmd_up == c(4) {
            let s = self.w.s("strCryReply");
            self.say(&s);
            self.moves_left -= 1;
        } else if cmd_up == c(5) {
            let body = ["strSelf", "strMe", "strLimb", "strHand"].iter().any(|k| a0.eq_ignore_ascii_case(&self.w.s(k)));
            if args.len() == 1 && body {
                let s = self.w.at("strEatReply", 0);
                self.say(&s);
                self.moves_left -= 1;
            } else if !args.is_empty() {
                self.command_bad(&cmd_up);
            } else {
                let s = self.w.at("strEatReply", 1);
                self.say(&s);
            }
        } else if cmd_up == c(6) {
            let s = self.w.s("strSleepReply");
            self.say(&s);
            self.moves_left = 0;
        } else if (0..4).any(|i| cmd_up == self.w.at("strProfanity", i)) {
            let s = self.one_of("strProfanityReply", 2);
            self.say(&s);
            self.moves_left -= 1;
        } else if cmd_up == self.w.s("strLook") || cmd_up == self.w.s("strListen") {
            let s = self.w.s("strLookListenReply");
            self.say(&s);
            self.moves_left -= 1;
        } else if cmd_up == self.w.s("strExamine") || cmd_up == self.w.s("strSearch") {
            if args.is_empty() {
                self.command_bad(&cmd_up);
            } else if ["strClothes", "strShirt", "strPants"].iter().any(|k| a0.eq_ignore_ascii_case(&self.w.s(k))) {
                let s = self.w.at("strSearchExamineReply", 0);
                self.say(&s);
                self.moves_left -= 1;
            } else {
                let s = fmtn(&self.w.at("strSearchExamineReply", 1), &[&a0]);
                self.say(&s);
                self.moves_left -= 1;
            }
        } else {
            handled = false;
        }
        if self.moves_left == 0 {
            let s = self.w.s("strEaten");
            self.say(&s);
            self.death();
            self.prompt_new_game();
            handled = true;
        }
        // TextConsoleInterface.Parse: help.
        if !handled && cmd_up == self.w.s("strHelpCom") {
            handled = true;
            if let Some(a) = args.first() {
                if a.to_uppercase() == self.w.s("strHelpCom") {
                    let s = self.w.s("strHelpDef");
                    self.say(&s);
                } else if let Some(i) = self.commands.iter().position(|c| c.eq_ignore_ascii_case(a)).filter(|&i| !self.help.get(i).is_none_or(|h| h.is_empty())) {
                    let s = format!("{} - {}", a.to_uppercase(), self.help[i]);
                    self.say(&s);
                } else {
                    let s = fmtn(&self.w.at("strError", 2), &[a]);
                    self.say(&s);
                }
            } else {
                let s = self.w.s("strCommandList");
                self.say(&s);
                for c in self.commands.clone() {
                    self.say(&c.to_uppercase());
                }
            }
        }
        if !handled {
            self.command_not_found(&cmd);
        }
    }

    /// AttemptMove: the eight directions (short or long), "light", or a bad move.
    fn attempt_move(&mut self, dir: &str) {
        let up = dir.to_uppercase();
        let hit = (0..16).position(|i| up == self.w.at("strCardinalDirections", i));
        if let Some(i) = hit {
            let full = self.w.at("strCardinalDirectionsFull", (i / 2) as u32);
            let s = fmtn(&self.w.s("strMoveValid"), &[&full, &full]);
            self.say(&s);
            self.moves_left -= 1;
        } else if up.eq_ignore_ascii_case(&self.w.s("strLight")) {
            let s = self.w.s("strMoveLight");
            self.say(&s);
        } else {
            self.command_bad("MOVE");
        }
    }

    fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>, typed: &[char]) {
        crate::tnmgames::played("TEXT", dt);
        self.blink = (self.blink + dt) % 1.0;
        for &c in typed {
            if !c.is_control() && self.input.chars().count() < 256 {
                self.input.push(c);
            }
        }
        if keys.just_pressed(KeyCode::Backspace) {
            self.input.pop();
        }
        if keys.just_pressed(KeyCode::ArrowUp) && self.hist_at > 0 {
            self.hist_at -= 1;
            self.input = self.history[self.hist_at].clone();
        }
        if keys.just_pressed(KeyCode::ArrowDown) && self.hist_at < self.history.len() {
            self.hist_at += 1;
            self.input = self.history.get(self.hist_at).cloned().unwrap_or_default();
        }
        if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
            let line = std::mem::take(&mut self.input);
            if !line.is_empty() {
                self.history.push(line.clone());
                self.hist_at = self.history.len();
                self.enter(&line);
            }
        }
    }

    fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        // The client pictures, cols x rows.
        let (mut tx, mut ty, mut row_h) = (x, y, 0);
        for (k, b) in self.bg.clone().iter().enumerate() {
            if k > 0 && k % self.bg_cols.max(1) == 0 {
                tx = x;
                ty += row_h;
                row_h = 0;
            }
            if let Some(t) = u.hud.tex(u.assets, b) {
                u.c.texture(tx, ty, t.w as i32, t.h as i32, 0, 0, &t, Style::Normal, [255, 255, 255]);
                tx += t.w as i32;
                row_h = row_h.max(t.h as i32);
            }
        }
        let Some(f) = self.font.clone() else { return };
        let (cw, ch) = self.client;
        // The log, newest at the bottom, word wrapped (TextConsoleOutput).
        let (lx, lw) = (x + 8, cw - 16);
        let mut ly = y + ch - 23 - 4;
        'up: for l in self.lines.iter().rev() {
            let wrapped = crate::uicanvas::wrap(&f, l, lw);
            for wl in wrapped.iter().rev() {
                ly -= f.height;
                if ly < y + 7 {
                    break 'up;
                }
                u.c.text(&f, lx, ly, lw, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], wl);
            }
        }
        // The command line (an info button's well).
        u.c.fill(x + 4, y + ch - 23, cw - 8, 21, Style::Translucent, [24, 24, 24]);
        let caret = if self.blink < 0.5 { "_" } else { "" };
        let shown = format!("{}{caret}", self.input);
        u.c.text(&f, x + 8, y + ch - 23 + 4, cw - 16, f.height, HAlign::Left, VAlign::Top, [255, 255, 255], &shown);
    }
}

// ------------------------------------------------------------------------------------------------ the screen

pub enum Game {
    Pong(Pong),
    Breakout(Box<Breakout>),
    Tetris(Box<Tetris>),
    Packets(Box<Packets>),
    Text(Box<TextGame>),
}

/// What a game's tick asks of its terminal.
#[derive(PartialEq, Eq)]
pub enum Outcome {
    Stay,
    /// The packet puzzle was solved (AIPuzzleWindow.PuzzleFinished).
    Finished,
}

impl Game {
    /// The game a tnmComputerScreenGames action opens.
    pub fn new(action: &str, assets: &mut RuntimeAssets, seed: u64) -> Option<Game> {
        match action {
            "PONG" => Some(Game::Pong(Pong::new(assets, seed))),
            "BREAKOUT" => Some(Game::Breakout(Box::new(Breakout::new(assets, seed)))),
            "TETRIS" => Some(Game::Tetris(Box::new(Tetris::new(assets, seed)))),
            "RUSHHOUR" => Some(Game::Packets(Box::new(Packets::new(assets, seed)))),
            "TEXTGAME" => Some(Game::Text(Box::new(TextGame::new(assets, seed)))),
            _ => None,
        }
    }
    pub fn class(&self) -> &'static str {
        match self {
            Game::Pong(_) => "PongWindow",
            Game::Breakout(_) => "BreakoutWindow",
            Game::Tetris(_) => "TetrisWindow",
            Game::Packets(_) => "AIPuzzleWindowDebug",
            Game::Text(_) => "tnmTextGameWindow",
        }
    }
    /// The client area (board plus scoreboard).
    pub fn client_size(&self) -> (i32, i32) {
        match self {
            Game::Pong(p) => p.client,
            Game::Breakout(b) => ((b.w + b.sb.0 + b.sb.2) as i32, b.h as i32),
            Game::Tetris(t) => (t.client.0 + t.sb.0 as i32 + t.sb.2 as i32, t.client.1),
            Game::Packets(p) => p.client,
            Game::Text(t) => t.client,
        }
    }
    /// Has a Start button (T presses it).
    pub fn has_start(&self) -> bool {
        matches!(self, Game::Pong(_) | Game::Tetris(_))
    }
    /// The packet puzzle's buttons (Reset Packets, Switch) instead of Return.
    pub fn is_packets(&self) -> bool {
        matches!(self, Game::Packets(_))
    }
    pub fn packets_solved(&self) -> bool {
        matches!(self, Game::Packets(p) if p.done)
    }
    pub fn reset(&mut self) {
        if let Game::Packets(p) = self {
            p.reset();
        }
    }
    pub fn switch(&mut self) {
        if let Game::Packets(p) = self {
            p.switch();
        }
    }
    pub fn start(&mut self) {
        match self {
            Game::Pong(p) => p.start(),
            Game::Tetris(t) => t.started = !t.started,
            Game::Breakout(_) | Game::Packets(_) | Game::Text(_) => {}
        }
    }
    pub fn title_suffix(&self) -> String {
        match self {
            Game::Tetris(t) => t.title_suffix(),
            _ => String::new(),
        }
    }
    /// The game takes typed keys (the terminal's own keys - Tab, Escape's Return - stay the terminal's).
    pub fn types(&self) -> bool {
        matches!(self, Game::Text(_) | Game::Breakout(_))
    }
    pub fn tick(&mut self, dt: f32, keys: &ButtonInput<KeyCode>, typed: &[char], ptr: Pointer) -> Outcome {
        if self.has_start() && keys.just_pressed(KeyCode::KeyT) && !keys.pressed(KeyCode::AltLeft) {
            self.start();
        }
        match self {
            Game::Pong(p) => p.tick(dt, keys),
            Game::Breakout(b) => {
                b.keys(keys, typed);
                b.tick(dt, keys);
            }
            Game::Tetris(t) => t.tick(dt, keys),
            Game::Packets(p) => {
                if !p.done && keys.just_pressed(KeyCode::KeyR) {
                    p.reset();
                }
                if p.tick(dt, ptr) {
                    return Outcome::Finished;
                }
            }
            Game::Text(t) => t.tick(dt, keys, typed),
        }
        Outcome::Stay
    }
    pub fn draw<W: KitWid>(&mut self, u: &mut Ui<W>, x: i32, y: i32) {
        match self {
            Game::Pong(p) => p.draw(u, x, y),
            Game::Breakout(b) => b.draw(u, x, y),
            Game::Tetris(t) => t.draw(u, x, y),
            Game::Packets(p) => p.draw(u, x, y),
            Game::Text(t) => t.draw(u, x, y),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Run against the player's own copy of the mod: TNM_SYS=<folder with TNM.u and TNMGUI.u>.
    use super::*;

    fn db() -> Option<unreal_pkg::classes::ClassDb> {
        let dir = std::env::var("TNM_SYS").ok()?;
        unreal_pkg::classes::set_extra_packages(&["TNMGUI", "TNM"]);
        Some(unreal_pkg::classes::ClassDb::open(std::path::Path::new(&dir)))
    }

    #[test]
    fn mod_data_loads() {
        let Some(mut db) = db() else { return };
        let lv = load_levels(&mut db);
        println!("breakout: {} levels, count {}, order {:?}", lv.levels.len(), lv.count, &lv.order[..lv.order.len().min(8)]);
        assert!(lv.count > 0 && !lv.levels.is_empty());
        let first = &lv.levels[lv.order[0]];
        let used = first.blocks.iter().filter(|b| b.used).count();
        println!("first level: {} blocks, passcode len {}", used, first.passcode.len());
        assert!(used > 0);
        let shapes = load_shapes(&mut db);
        println!("tetris: {} pieces, sizes {:?}", shapes.len(), shapes.iter().map(|s| (s.0.len(), s.1, s.2)).collect::<Vec<_>>());
        assert!(shapes.len() >= 7);
        let src = db.script_text("AIPuzzleGrid").unwrap_or_default();
        let kinds = enum_names(&src, "PieceType");
        println!("packet kinds {kinds:?}");
        assert!(kinds.len() >= 9);
        for n in 0..13 {
            let mut host = LayoutHost { number: n, kinds: kinds.clone(), out: Vec::new() };
            let mut script = crate::uscript::Script::default();
            script.add_source(&src);
            script.call(&mut host, "initpieces", &[]);
            let keys = host.out.iter().filter(|p| p.0 == 0).count();
            println!("layout {n}: {} packets", host.out.len());
            assert!(!host.out.is_empty() && keys == 1, "layout {n}");
        }
    }
}
