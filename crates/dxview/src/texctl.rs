//! The Nameless Mod's texture controllers (tnmTextureController): an Info actor that owns a ScriptedTexture on the
//! level (SlaveTexture) and redraws it.
//!   tnmTextureSwapController: the primary picture, the secondary one once triggered (and back when bCanSwapBack).
//!   tnmTextureSwitchController: one of TextureSet[16]; a tnmTextureSwitcher that triggers it names the picture
//!     (TextureID), any other trigger follows DefaultTriggerBehavior (none / random / the next one).
//!   PongTextureController: two computer players at Pong, drawn every frame; P1ScoreEvent / P2ScoreEvent fire.
//! Numbers and pictures come from the actors and their class defaults in the player's own TNM.u.
// uc: tnmTextureController.RenderTexture, tnmTextureSwapController.Swap, tnmTextureSwitchController.Trigger,
// uc: tnmTextureSwitcher.Trigger, PongTextureController.Tick, PongTextureController.RenderTexture

use bevy::prelude::*;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

use crate::uicanvas::{Canvas, Style, UiTex};

static INBOX: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn event(name: &str) {
    if let Ok(mut q) = INBOX.lock() {
        if q.len() < 256 {
            q.push(name.to_string());
        }
    }
}

struct Pong {
    f: [f32; 9], // boardX, boardY, boardW, boardH, ballW, ballH, paddleW, paddleH, AIspeed
    ball: [f32; 5], // x, y, vx, vy, speed
    p1y: f32,
    p2y: f32,
    p2x: f32,
    dest: [f32; 2],
    search: [bool; 2],
    wait: f32,
    events: [String; 2],
    dx_style: bool,
    tex: [Option<UiTex>; 5], // pal, draw, blank, ball, paddle
    seed: u64,
}

enum Kind {
    Swap { primary: String, secondary: String, back: bool, swapped: bool },
    Switch { set: Vec<String>, behavior: u8, cur: usize },
    Pong(Box<Pong>),
}

struct Ctl {
    tag: String,
    handle: Handle<Image>,
    w: u32,
    h: u32,
    kind: Kind,
    /// ChangeTex: the picture to draw next frame.
    pending: Option<String>,
}

#[derive(Default)]
pub struct CtlState {
    gen: u64,
    ctls: Vec<Ctl>,
    /// tnmTextureSwitchers: (Tag, Event, TextureID).
    switchers: Vec<(String, String, i32)>,
    /// A switcher fired: (controller tag, TextureID) for the event on its way.
    named: Vec<(String, i32)>,
}

fn path(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Path(v)) => v.clone(),
        _ => String::new(),
    }
}

fn name(p: &[Property], n: &str) -> String {
    match find(p, n) {
        Some(Value::Name(v)) if v != "None" => v.clone(),
        _ => String::new(),
    }
}

fn rnd(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 40) as f32 / (1u64 << 24) as f32
}

fn load(assets: &mut crate::weapons::RuntimeAssets, p: &str) -> Option<UiTex> {
    if p.is_empty() {
        return None;
    }
    let (t, mips) = assets.lib.load(p)?;
    Some(UiTex::from_rgba(t.width(), t.height(), mips.first()?))
}

impl Pong {
    fn reset_ball(&mut self) {
        let f = self.f;
        let vx = rnd(&mut self.seed) * 0.5 + 0.5;
        let vy = (1.0 - vx * vx).max(0.0).sqrt();
        let sx = if rnd(&mut self.seed) < 0.5 { -1.0 } else { 1.0 };
        let sy = if rnd(&mut self.seed) < 0.5 { -1.0 } else { 1.0 };
        self.ball = [f[2] / 2.0 - f[4], f[3] / 2.0 - f[5], vx * sx, vy * sy, 1.5];
        self.search = [true, true];
        self.wait = 1.0;
    }

    /// Where the ball will cross a paddle's line (the original's bounce walk; it gives up after ten bounces).
    fn aim(&self, line_x: f32) -> Option<f32> {
        let [_, _, _, bh, _, ball_h, _, ph, _] = self.f;
        let [bx, by, vx, vy, _] = self.ball;
        if vx == 0.0 || vy == 0.0 {
            return Some(by);
        }
        let mut t1 = (line_x - bx) / vx;
        let mut dest = by + vy * t1;
        let (mut mult, mut pos, mut n) = (1.0f32, by, 0);
        while (dest < 0.0 || dest > bh - ph) && n < 10 {
            let t2;
            if mult * vy < 0.0 {
                t2 = -pos / vy * mult;
                pos = 0.0;
            } else {
                t2 = (bh - pos) / vy * mult;
                pos = bh - ball_h;
            }
            t1 -= t2;
            mult = -mult;
            dest = pos + vy * t1 * mult;
            n += 1;
        }
        if dest - ph / 2.0 > 0.0 {
            dest -= ph / 2.0;
        }
        (n < 10).then_some(dest)
    }

    fn tick(&mut self, dt: f32, out: &mut Vec<String>) {
        if self.wait > 0.0 {
            self.wait -= dt;
            return;
        }
        let dt = dt * 150.0;
        let [_, _, bw, bh, ball_w, ball_h, pw, ph, ai] = self.f;
        // Player one (left), then two.
        if self.search[0] {
            if self.ball[2] < 0.0 {
                if let Some(d) = self.aim(pw) {
                    self.dest[0] = d;
                    self.search[0] = false;
                }
            } else {
                self.search[0] = false;
            }
        } else if self.ball[2] > 0.0 {
            self.dest[0] = self.ball[1];
        }
        if self.search[1] {
            if self.ball[2] > 0.0 {
                if let Some(d) = self.aim(self.p2x) {
                    self.dest[1] = d;
                    self.search[1] = false;
                }
            } else {
                self.search[1] = false;
            }
        } else if self.ball[2] < 0.0 {
            self.dest[1] = self.ball[1];
        }
        for (y, dest) in [(&mut self.p1y, self.dest[0]), (&mut self.p2y, self.dest[1])] {
            let mut sp = 0.0;
            if (*y - dest).abs() > 3.0 {
                sp = if *y > dest { -ai } else { ai };
            }
            let ny = *y + sp * dt;
            if ny >= 0.0 && ny <= bh - ph {
                *y = ny;
            }
        }
        let mut update = true;
        let nx = self.ball[0] + self.ball[2] * dt * self.ball[4];
        let ny = self.ball[1] + self.ball[3] * dt * self.ball[4];
        if ny < 0.0 || ny + ball_w > bh {
            self.ball[3] = -self.ball[3];
            if self.ball[2] < 0.0 {
                self.search[0] = true;
            } else {
                self.search[1] = true;
            }
            update = false;
        }
        if nx <= pw && ny <= self.p1y + ph && ny + ball_h >= self.p1y {
            self.ball[0] = pw + 1.0;
            self.ball[2] = -self.ball[2];
            self.ball[4] *= 1.1;
            self.search[1] = true;
            update = false;
        }
        if nx + ball_w >= self.p2x && ny <= self.p2y + ph && ny + ball_h >= self.p2y {
            self.ball[0] = self.p2x - ball_w - 1.0;
            self.ball[2] = -self.ball[2];
            self.ball[4] *= 1.1;
            self.search[0] = true;
            update = false;
        }
        if update {
            self.ball[0] = nx;
            self.ball[1] = ny;
            if nx <= 0.0 {
                self.reset_ball();
                if !self.events[1].is_empty() {
                    out.push(self.events[1].clone());
                }
            } else if nx + ball_w >= bw {
                self.reset_ball();
                if !self.events[0].is_empty() {
                    out.push(self.events[0].clone());
                }
            }
        }
    }

    fn draw(&self, c: &mut Canvas) {
        let [bx, by, bw, _, ball_w, ball_h, pw, ph, _] = self.f;
        let t = |i: usize| self.tex[i].as_ref();
        if let Some(blank) = t(2) {
            c.texture_stretched(0, 0, 256, 256, (0, 0, 1, 1), blank, Style::Normal, [255, 255, 255]);
        }
        if let Some(pal) = t(0) {
            c.texture_stretched((bw / 2.0 - 2.0) as i32, 0, 4, 256, (0, 1, 1, 8), pal, Style::Normal, [255, 255, 255]);
        }
        let (pad, pr, ball, br, masked) = if self.dx_style {
            (t(4), (0, 0, 64, 128), t(3), (6, 0, 28, 36), Style::Masked)
        } else {
            (t(1), (0, 0, 8, 8), t(1), (0, 0, 8, 8), Style::Normal)
        };
        if let Some(p) = pad {
            c.texture_stretched(bx as i32, (self.p1y + by) as i32, pw as i32, ph as i32, pr, p, Style::Normal, [255, 255, 255]);
            c.texture_stretched((self.p2x + bx) as i32, (self.p2y + by) as i32, pw as i32, ph as i32, pr, p, Style::Normal, [255, 255, 255]);
        }
        if let Some(b) = ball {
            c.texture_stretched((self.ball[0] + bx) as i32, (self.ball[1] + by) as i32, ball_w as i32, ball_h as i32, br, b, masked, [255, 255, 255]);
        }
    }
}

fn put(images: &mut Assets<Image>, handle: &Handle<Image>, w: u32, h: u32, c: &Canvas) {
    let img = crate::make_image(&dx_level::TextureImage {
        name: String::new(),
        width: w,
        height: h,
        mips: vec![c.to_rgba8()],
        bc1: false,
        bc3: false,
    });
    let _ = images.insert(handle.id(), img);
}

pub fn texctl_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut events: ResMut<crate::devices::GameEvents>,
    mut st: Local<CtlState>,
) {
    let lvl = &level.0;
    if crate::loader::new_map(&mut st.gen) {
        st.ctls.clear();
        st.switchers.clear();
        st.named.clear();
        if let Ok(mut q) = INBOX.lock() {
            q.clear();
        }
        for (ai, a) in lvl.actors.iter().enumerate() {
            if a.class.eq_ignore_ascii_case("tnmTextureSwitcher") {
                let id = match find(&a.props, "TextureID") {
                    Some(Value::Int(v)) => *v,
                    _ => 0,
                };
                st.switchers.push((name(&a.props, "Tag"), name(&a.props, "Event"), id));
                continue;
            }
            if !crate::classtree::is_a(&a.class, "tnmTextureController") {
                continue;
            }
            let slave = path(&a.props, "SlaveTexture");
            let Some((handle, w, h)) = crate::scripted_tv::level_texture(&slave) else {
                println!("TNM texture controller {}: {slave} isn't drawn on this level", a.name);
                continue;
            };
            let p = &a.props;
            let class = a.class.to_ascii_lowercase();
            let (kind, pending) = match class.as_str() {
                "tnmtextureswapcontroller" => {
                    let primary = path(p, "primaryTexture");
                    (
                        Kind::Swap {
                            secondary: path(p, "secondaryTexture"),
                            back: matches!(find(p, "bCanSwapBack"), Some(Value::Bool(true))),
                            swapped: false,
                            primary: primary.clone(),
                        },
                        Some(primary),
                    )
                }
                "tnmtextureswitchcontroller" => {
                    let mut set = vec![String::new(); 16];
                    for x in p.iter().filter(|x| x.name.eq_ignore_ascii_case("TextureSet")) {
                        if let (Some(s), Value::Path(v)) = (set.get_mut(x.index as usize), &x.value) {
                            *s = v.clone();
                        }
                    }
                    let behavior = match find(p, "DefaultTriggerBehavior") {
                        Some(Value::Byte(b)) => *b,
                        _ => 0,
                    };
                    let first = set[0].clone();
                    (Kind::Switch { set, behavior, cur: 0 }, Some(first))
                }
                "pongtexturecontroller" => {
                    let fl = |n: &str| match find(p, n) {
                        Some(Value::Float(v)) => *v,
                        Some(Value::Int(v)) => *v as f32,
                        _ => 0.0,
                    };
                    let f = [
                        fl("boardX"), fl("boardY"), fl("boardW"), fl("boardH"), fl("ballW"), fl("ballH"), fl("paddleW"),
                        fl("paddleH"), fl("AIspeed"),
                    ];
                    let tex = ["palTexture", "drawTexture", "blankTexture", "ballTexture", "paddleTexture"]
                        .map(|n| load(&mut assets, &path(p, n)));
                    let mut g = Pong {
                        f,
                        ball: [0.0; 5],
                        p1y: f[3] / 2.0 - f[7] / 2.0,
                        p2y: f[3] / 2.0 - f[7] / 2.0,
                        p2x: f[2] - f[6],
                        dest: [f[3] / 2.0 - f[7] / 2.0; 2],
                        search: [true; 2],
                        wait: 1.0,
                        events: [name(p, "P1ScoreEvent"), name(p, "P2ScoreEvent")],
                        dx_style: matches!(find(p, "bDXStyle"), Some(Value::Bool(true))),
                        tex,
                        seed: 0x706F_6E67_0000_0001 ^ ((ai as u64 + 1) * 0x9E37_79B9_7F4A_7C15),
                    };
                    g.reset_ball();
                    (Kind::Pong(Box::new(g)), None)
                }
                _ => continue,
            };
            st.ctls.push(Ctl { tag: name(p, "Tag"), handle, w, h, kind, pending });
        }
        if !st.ctls.is_empty() {
            println!("TNM texture controllers: {}", st.ctls.len());
        }
    }
    if st.ctls.is_empty() {
        return;
    }
    let st = &mut *st;
    let dt = time.delta_secs().min(0.1);
    let inbox = INBOX.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for ev in &inbox {
        // A switcher passes its trigger on with its picture number.
        for (_, to, id) in st.switchers.iter().filter(|s| !s.0.is_empty() && s.0.eq_ignore_ascii_case(ev)) {
            if !to.is_empty() {
                st.named.push((to.clone(), *id));
                events.0.push((to.clone(), true));
            }
        }
        for c in st.ctls.iter_mut().filter(|c| !c.tag.is_empty() && c.tag.eq_ignore_ascii_case(ev)) {
            match &mut c.kind {
                Kind::Swap { primary, secondary, back, swapped } => {
                    if !*swapped {
                        c.pending = Some(secondary.clone());
                    } else if *back {
                        c.pending = Some(primary.clone());
                    } else {
                        continue;
                    }
                    *swapped = !*swapped;
                }
                Kind::Switch { set, behavior, cur } => {
                    let mut index = *cur;
                    if let Some(k) = st.named.iter().position(|n| n.0.eq_ignore_ascii_case(ev)) {
                        index = st.named.remove(k).1.clamp(0, 15) as usize;
                    } else if set.iter().any(|s| !s.is_empty()) {
                        match *behavior {
                            1 => {
                                let mut seed = (time.elapsed_secs_f64().to_bits() | 1) ^ 0x51ED_2701;
                                loop {
                                    index = (rnd(&mut seed) * 16.0) as usize % 16;
                                    if !set[index].is_empty() {
                                        break;
                                    }
                                }
                            }
                            2 => {
                                index = (index + 1) % 16;
                                if set[index].is_empty() {
                                    index = 0;
                                }
                            }
                            _ => {}
                        }
                    }
                    *cur = index;
                    c.pending = Some(set[index].clone());
                }
                Kind::Pong(_) => {}
            }
        }
    }
    let mut fired: Vec<String> = Vec::new();
    for c in st.ctls.iter_mut() {
        let (w, h) = (c.w as i32, c.h as i32);
        if let Kind::Pong(g) = &mut c.kind {
            g.tick(dt, &mut fired);
            let mut cv = Canvas::new(w, h, 1);
            g.draw(&mut cv);
            put(&mut images, &c.handle, c.w, c.h, &cv);
            continue;
        }
        let Some(p) = c.pending.take() else { continue };
        let Some(t) = load(&mut assets, &p) else { continue };
        let mut cv = Canvas::new(w, h, 1);
        cv.texture_stretched(0, 0, w, h, (0, 0, t.w as i32, t.h as i32), &t, Style::Normal, [255, 255, 255]);
        put(&mut images, &c.handle, c.w, c.h, &cv);
    }
    for e in fired {
        events.0.push((e, true));
    }
}
