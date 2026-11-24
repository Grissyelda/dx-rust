//! The Nameless Mod's TV screens (ScriptedTextureGSTVOn, a TNMScriptedTextureController): the controller draws onto
//! its ScriptedTexture every frame - the large text in FontMenuExtraLarge at (15, 10) in white and the small text in
//! FontMenuSmall_DS from (15, 40), one line per "/n" piece every 15 rows - over the texture's own picture. It moves
//! on to its next non-empty TextLarge / TextSmall pair every four seconds. On 95_ComputerRoom the mission script
//! writes the computer games' records into the first small text (TNMMissionScript.Timer).
//!
//! The texture's image comes from dx-level (TexLib::load_scripted: the SourceTexture, one mip); its handle is found
//! by name among the level's textures.

use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use unreal_pkg::props::Value;

use crate::uicanvas::{Canvas, HAlign, Style, UiTex, VAlign};

/// The level's texture images by name (lowercase), registered as the map is set up.
static LEVEL_TEX: Mutex<Option<HashMap<String, (Handle<Image>, u32, u32, Vec<u8>)>>> = Mutex::new(None);

/// A mission script's text for every TV's TextSmall[i] (tnmMission24's live census): (i, text).
static SMALL: Mutex<Option<(usize, String)>> = Mutex::new(None);

pub fn set_small(i: usize, text: String) {
    if let Ok(mut g) = SMALL.lock() {
        *g = Some((i, text));
    }
}

/// Every level texture's handle and first-mip RGBA (only ScriptedTextures are kept).
pub fn note(name: &str, handle: &Handle<Image>, t: &dx_level::TextureImage) {
    if t.bc1 || t.bc3 || t.mips.len() != 1 {
        return;
    }
    if let Ok(mut g) = LEVEL_TEX.lock() {
        g.get_or_insert_with(HashMap::new)
            .insert(name.to_ascii_lowercase(), (handle.clone(), t.width, t.height, t.mips[0].clone()));
    }
}

/// A ScriptedTexture drawn on this level: its image and size.
pub fn level_texture(name: &str) -> Option<(Handle<Image>, u32, u32)> {
    let g = LEVEL_TEX.lock().ok()?;
    let (h, w, ht, _) = g.as_ref()?.get(&name.to_ascii_lowercase())?;
    Some((h.clone(), *w, *ht))
}

pub fn reset_level() {
    if let Ok(mut g) = SMALL.lock() {
        *g = None;
    }
    if let Ok(mut g) = LEVEL_TEX.lock() {
        *g = None;
    }
}

struct Tv {
    handle: Handle<Image>,
    w: u32,
    h: u32,
    base: UiTex,
    large: Vec<String>,
    small: Vec<String>,
    /// currentText, and bReplaced (switched in this four-second window).
    current: usize,
    replaced: bool,
    last: Option<(String, String)>,
}

#[derive(Default)]
pub struct TvState {
    gen: u64,
    tvs: Vec<Tv>,
    fonts: Option<(Option<crate::uicanvas::UiFont>, Option<crate::uicanvas::UiFont>)>,
    computer_room: bool,
}

fn strings(props: &[unreal_pkg::props::Property], name: &str) -> Vec<String> {
    let mut v = vec![String::new(); 6];
    for p in props.iter().filter(|p| p.name.eq_ignore_ascii_case(name)) {
        if let (Some(slot), Value::Str(s)) = (v.get_mut(p.index as usize), &p.value) {
            *slot = s.clone();
        }
    }
    v
}

/// "~" @ str with %d (or %s) filled in, as TNMMissionScript's sprintf.
fn score_line(fmt: &str, n: i64) -> String {
    let f = fmt.replacen("%d", &n.to_string(), 1).replacen("%s", &n.to_string(), 1);
    format!("~ {f}")
}

pub fn tv_system(
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut st: Local<TvState>,
) {
    if !crate::tnmmode::active() {
        return;
    }
    let lvl = &level.0;
    if crate::loader::new_map(&mut st.gen) {
        st.tvs.clear();
        st.computer_room = lvl.name.trim_end_matches(".dx").eq_ignore_ascii_case("95_ComputerRoom");
        let tex = LEVEL_TEX.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
        for a in &lvl.actors {
            if !crate::classtree::is_a(&a.class, "ScriptedTextureGSTVOn") {
                continue;
            }
            let Some(Value::Path(p)) = unreal_pkg::props::find(&a.props, "ScriptedTexture") else { continue };
            let Some((handle, w, h, rgba)) = tex.get(&p.to_ascii_lowercase()).cloned() else {
                println!("TNM TV {}: texture {p} not on the level", a.name);
                continue;
            };
            let large = strings(&a.props, "TextLarge");
            // BeginPlay: the first non-empty large text.
            let start = large.iter().position(|t| !t.is_empty()).unwrap_or(0);
            st.tvs.push(Tv {
                handle,
                w,
                h,
                base: UiTex::from_rgba(w, h, &rgba),
                small: strings(&a.props, "TextSmall"),
                large,
                current: start,
                replaced: true,
                last: None,
            });
        }
    }
    if st.tvs.is_empty() {
        return;
    }
    if st.fonts.is_none() {
        st.fonts = Some((
            crate::classic_hud::load_font(&mut assets, "FontMenuExtraLarge"),
            crate::classic_hud::load_font(&mut assets, "FontMenuSmall_DS"),
        ));
    }
    // 95_ComputerRoom: the records in the first small text.
    let records = st.computer_room.then(|| {
        let d = assets.classes.defaults("TNMMissionScript");
        let s = |k: &str| match unreal_pkg::props::find(&d, k) {
            Some(Value::Str(t)) => t.clone(),
            _ => String::new(),
        };
        let mut t = String::from("/n");
        t += &(score_line(&s("strTetris"), crate::tnmgames::high("Tetris")) + "/n");
        t += &(score_line(&s("strPong"), crate::tnmgames::high("Pong")) + "/n");
        t += &(score_line(&s("strBreakout"), crate::tnmgames::high("Breakout")) + "/n");
        t += &score_line(&s("strLightsOut"), crate::tnmgames::high("LightsOut"));
        t
    });
    let nsec = (time.elapsed_secs() as i64) % 4;
    let st = &mut *st;
    let (Some(big), Some(small)) = st.fonts.as_ref().map(|f| (f.0.as_ref(), f.1.as_ref())).unwrap_or((None, None)) else { return };
    let script_small = SMALL.lock().ok().and_then(|g| g.clone());
    for tv in st.tvs.iter_mut() {
        if let Some(r) = &records {
            tv.small[0] = r.clone();
        }
        if let Some((i, t)) = &script_small {
            if tv.small.len() <= *i {
                tv.small.resize(*i + 1, String::new());
            }
            tv.small[*i] = t.clone();
        }
        // RenderTexture: once every four seconds, the next non-empty text.
        if nsec == 1 && !tv.replaced {
            tv.replaced = true;
            for _ in 0..tv.large.len() {
                tv.current = (tv.current + 1) % tv.large.len();
                if !tv.large[tv.current].is_empty() {
                    break;
                }
            }
        } else if nsec == 2 {
            tv.replaced = false;
        }
        let shown = (tv.large[tv.current].clone(), tv.small[tv.current].clone());
        if tv.last.as_ref() == Some(&shown) {
            continue;
        }
        let (w, h) = (tv.w as i32, tv.h as i32);
        let mut c = Canvas::new(w, h, 1);
        c.texture(0, 0, w, h, 0, 0, &tv.base, Style::Normal, [255, 255, 255]);
        c.text(big, 15, 10, w - 15, big.height, HAlign::Left, VAlign::Top, [255, 255, 255], &shown.0);
        // DrawText splits the small text at every "/n" (the piece after the last one is drawn too).
        for (i, line) in shown.1.split("/n").enumerate() {
            c.text(small, 15, 40 + 15 * i as i32, w - 15, small.height, HAlign::Left, VAlign::Top, [255, 255, 255], line);
        }
        // The level's images live in the render world only: a fresh one goes in under the same handle.
        let img = crate::make_image(&dx_level::TextureImage {
            name: String::new(),
            width: tv.w,
            height: tv.h,
            mips: vec![c.to_rgba8()],
            bc1: false,
            bc3: false,
        });
        let _ = images.insert(tv.handle.id(), img);
        tv.last = Some(shown);
    }
}
