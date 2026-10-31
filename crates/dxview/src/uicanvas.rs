//! A software canvas with the original UI's drawing rules (Extension GC: DrawTexture, DrawPattern, DrawBorders,
//! DrawText with a bitmap Font), so HUD windows can be ported line by line from their DrawWindow code.
//!
//! Coordinates are DX UI units; every unit is drawn as an s x s block of physical pixels (whole-number scaling
//! keeps the art crisp). Pixels are stored premultiplied, so the result composites over the 3D view with
//! premultiplied-alpha blending: DSTY_Translucent draws add colour and leave alpha alone (UE1's additive
//! blend), DSTY_Masked / DSTY_Normal draw opaque.

use std::sync::Arc;
use unreal_pkg::objects::Font;

/// An RGBA8 texture (straight alpha; masked textures have alpha 0 on palette index 0).
#[derive(Debug, Clone)]
pub struct UiTex {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[u8; 4]>,
}

impl UiTex {
    pub fn from_rgba(w: u32, h: u32, bytes: &[u8]) -> UiTex {
        UiTex {
            w,
            h,
            px: bytes
                .chunks_exact(4)
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect(),
        }
    }
    fn at(&self, u: i32, v: i32) -> [u8; 4] {
        let x = u.rem_euclid(self.w as i32) as u32;
        let y = v.rem_euclid(self.h as i32) as u32;
        self.px[(y * self.w + x) as usize]
    }
}

/// A bitmap font: one page texture (masked) and glyph rectangles by character code.
#[derive(Debug, Clone)]
pub struct UiFont {
    pub pages: Vec<Arc<UiTex>>,
    pub font: Font,
    pub height: i32,
}

impl UiFont {
    pub fn new(font: Font, pages: Vec<Arc<UiTex>>) -> UiFont {
        let height = font
            .pages
            .iter()
            .flat_map(|p| p.1.iter())
            .map(|g| g.h)
            .max()
            .unwrap_or(8);
        UiFont {
            pages,
            font,
            height,
        }
    }
    /// Width of a string in UI units (glyph cells already carry their spacing).
    pub fn width(&self, s: &str) -> i32 {
        s.chars()
            .filter_map(|c| self.font.glyph(c))
            .map(|g| g.1.w)
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Opaque.
    Normal,
    /// Opaque where the texel isn't the mask colour.
    Masked,
    /// Additive.
    Translucent,
    /// DSTY_Modulated: what's below times twice the texel (mid grey leaves it, black darkens it to nothing).
    Modulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

pub type Rgb = [u8; 3];

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub s: i32,
    /// Premultiplied linear-in-byte-space pixels, (w*s) x (h*s).
    pub px: Vec<[f32; 4]>,
    clip: (i32, i32, i32, i32),
}

impl Canvas {
    pub fn new(w: i32, h: i32, s: i32) -> Canvas {
        let s = s.max(1);
        Canvas {
            w,
            h,
            s,
            px: vec![[0.0; 4]; (w * s * h * s).max(0) as usize],
            clip: (0, 0, w, h),
        }
    }

    /// Limit drawing to a rectangle (a child window's area); `None` resets to the whole canvas.
    pub fn set_clip(&mut self, r: Option<(i32, i32, i32, i32)>) {
        self.clip = match r {
            Some((x, y, w, h)) => (x.max(0), y.max(0), (x + w).min(self.w), (y + h).min(self.h)),
            None => (0, 0, self.w, self.h),
        };
    }

    #[inline(always)]
    fn put_unclipped(&mut self, x: i32, y: i32, rgb: [f32; 3], style: Style) {
        let s = self.s as usize;
        let row = self.w as usize * s;
        for sy in 0..s {
            let base = (y as usize * s + sy) * row + x as usize * s;
            for p in &mut self.px[base..base + s] {
                blend(p, rgb, style);
            }
        }
    }

    /// GC.DrawTexture: the texture region starting at (u, v), unscaled (wrapping), into the destination box.
    /// (Clipped once per call and walked row by row: this is most of every UI screen's drawing time.)
    #[allow(clippy::too_many_arguments)]
    pub fn texture(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        u: i32,
        v: i32,
        tex: &UiTex,
        style: Style,
        col: Rgb,
    ) {
        let (tw, th) = (tex.w as i32, tex.h as i32);
        if tw <= 0 || th <= 0 {
            return;
        }
        let (x0, y0, x1, y1) = self.clip;
        let (dx_lo, dx_hi) = ((x0 - x).max(0), (x1 - x).min(w));
        let (dy_lo, dy_hi) = ((y0 - y).max(0), (y1 - y).min(h));
        if dx_lo >= dx_hi || dy_lo >= dy_hi {
            return;
        }
        let k = [col[0] as f32 / 255.0 / 255.0, col[1] as f32 / 255.0 / 255.0, col[2] as f32 / 255.0 / 255.0];
        let masked = style == Style::Masked;
        let tw_u = tw as usize;
        for dy in dy_lo..dy_hi {
            let ty = (v + dy).rem_euclid(th) as usize;
            let src = &tex.px[ty * tw_u..(ty + 1) * tw_u];
            let mut tx = (u + dx_lo).rem_euclid(tw) as usize;
            for dx in dx_lo..dx_hi {
                let t = src[tx];
                tx += 1;
                if tx == tw_u {
                    tx = 0;
                }
                if masked && t[3] == 0 {
                    continue;
                }
                let rgb = [t[0] as f32 * k[0], t[1] as f32 * k[1], t[2] as f32 * k[2]];
                self.put_unclipped(x + dx, y + dy, rgb, style);
            }
        }
    }

    /// A soft drop shadow of a masked texture's shape drawn at (x, y): its silhouette offset by `off` UI units,
    /// blurred over `blur` UI units (two box passes at full resolution) and laid down as black at `opacity`.
    pub fn drop_shadow(&mut self, x: i32, y: i32, tex: &UiTex, off: (f32, f32), blur: f32, opacity: f32) {
        let s = self.s as f32;
        let (w, h) = ((self.w * self.s) as usize, (self.h * self.s) as usize);
        if w == 0 || h == 0 {
            return;
        }
        let mut m = vec![0.0f32; w * h];
        for py in 0..h {
            for px in 0..w {
                let u = ((px as f32 + 0.5) / s - x as f32 - off.0).floor() as i32;
                let v = ((py as f32 + 0.5) / s - y as f32 - off.1).floor() as i32;
                if u >= 0 && v >= 0 && (u as u32) < tex.w && (v as u32) < tex.h && tex.at(u, v)[3] > 0 {
                    m[py * w + px] = 1.0;
                }
            }
        }
        let r = ((blur * s) / 2.0).round().max(1.0) as i32;
        let mut tmp = vec![0.0f32; w * h];
        for _ in 0..2 {
            for py in 0..h {
                for px in 0..w {
                    let (mut acc, mut n) = (0.0, 0.0);
                    for k in -r..=r {
                        let q = px as i32 + k;
                        if q >= 0 && (q as usize) < w {
                            acc += m[py * w + q as usize];
                        }
                        n += 1.0;
                    }
                    tmp[py * w + px] = acc / n;
                }
            }
            for py in 0..h {
                for px in 0..w {
                    let (mut acc, mut n) = (0.0, 0.0);
                    for k in -r..=r {
                        let q = py as i32 + k;
                        if q >= 0 && (q as usize) < h {
                            acc += tmp[q as usize * w + px];
                        }
                        n += 1.0;
                    }
                    m[py * w + px] = acc / n;
                }
            }
        }
        for (p, a) in self.px.iter_mut().zip(&m) {
            let a = a * opacity;
            if a <= 0.0 {
                continue;
            }
            p[0] *= 1.0 - a;
            p[1] *= 1.0 - a;
            p[2] *= 1.0 - a;
            p[3] = a + p[3] * (1.0 - a);
        }
    }

    /// Draws a layer that's the same every frame for a key (a screen's background art for one theme and scale)
    /// once and copies it in afterwards. Only for the first thing drawn on a blank canvas.
    pub fn cached(&mut self, key: impl std::hash::Hash, draw: impl FnOnce(&mut Canvas)) {
        use std::hash::{Hash, Hasher};
        static LAYERS: std::sync::Mutex<Vec<(u64, Vec<[f32; 4]>)>> = std::sync::Mutex::new(Vec::new());
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        (self.w, self.h, self.s).hash(&mut h);
        let k = h.finish();
        if let Ok(g) = LAYERS.lock() {
            if let Some((_, px)) = g.iter().find(|(kk, px)| *kk == k && px.len() == self.px.len()) {
                self.px.copy_from_slice(px);
                return;
            }
        }
        draw(self);
        if let Ok(mut g) = LAYERS.lock() {
            g.retain(|(kk, _)| *kk != k);
            if g.len() >= 12 {
                g.remove(0);
            }
            g.push((k, self.px.clone()));
        }
    }

    /// GC.DrawPattern with Texture'Solid': a flat box.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, style: Style, col: Rgb) {
        let rgb = [
            col[0] as f32 / 255.0,
            col[1] as f32 / 255.0,
            col[2] as f32 / 255.0,
        ];
        let (x0, y0, x1, y1) = self.clip;
        for py in (y.max(y0))..((y + h).min(y1)) {
            for px in (x.max(x0))..((x + w).min(x1)) {
                self.put_unclipped(px, py, rgb, style);
            }
        }
    }

    /// GC.DrawBorders: corners (TL, TR, BL, BR) at their sizes, edges (Left, Right, Top, Bottom) tiled between,
    /// centre tiled inside.
    pub fn borders(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        t: &[Option<Arc<UiTex>>; 9],
        style: Style,
        col: Rgb,
    ) {
        let dim = |i: usize| t[i].as_ref().map_or((0, 0), |t| (t.w as i32, t.h as i32));
        let (tlw, tlh) = dim(0);
        let (trw, trh) = dim(1);
        let (blw, blh) = dim(2);
        let (brw, brh) = dim(3);
        let (lw, _) = dim(4);
        let (rw, _) = dim(5);
        let (_, th) = dim(6);
        let (_, bh) = dim(7);
        let draw = |c: &mut Canvas, i: usize, dx: i32, dy: i32, dw: i32, dh: i32| {
            if let Some(tex) = &t[i] {
                if dw > 0 && dh > 0 {
                    c.texture(dx, dy, dw, dh, 0, 0, tex, style, col);
                }
            }
        };
        draw(self, 8, x + lw, y + th, w - lw - rw, h - th - bh);
        draw(self, 6, x + tlw, y, w - tlw - trw, th);
        draw(self, 7, x + blw, y + h - bh, w - blw - brw, bh);
        draw(self, 4, x, y + tlh, lw, h - tlh - blh);
        draw(self, 5, x + w - rw, y + trh, rw, h - trh - brh);
        draw(self, 0, x, y, tlw, tlh);
        draw(self, 1, x + w - trw, y, trw, trh);
        draw(self, 2, x, y + h - blh, blw, blh);
        draw(self, 3, x + w - brw, y + h - brh, brw, brh);
    }

    /// GC.DrawText (no word wrap): the string aligned in its box, glyphs drawn masked in the text colour.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        font: &UiFont,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        ha: HAlign,
        va: VAlign,
        col: Rgb,
        s: &str,
    ) {
        self.text_styled(font, x, y, w, h, ha, va, col, s, Style::Masked);
    }

    /// DrawText with a draw style (Canvas.Style 3 = translucent text, as the console's LOADING message).
    #[allow(clippy::too_many_arguments)]
    pub fn text_styled(
        &mut self,
        font: &UiFont,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        ha: HAlign,
        va: VAlign,
        col: Rgb,
        s: &str,
        style: Style,
    ) {
        let tw = font.width(s);
        let mut cx = match ha {
            HAlign::Left => x,
            HAlign::Center => x + (w - tw) / 2,
            HAlign::Right => x + w - tw,
        };
        let cy = match va {
            VAlign::Top => y,
            VAlign::Center => y + (h - font.height) / 2,
            VAlign::Bottom => y + h - font.height,
        };
        for c in s.chars() {
            let Some((page, g)) = font.font.glyph(c) else {
                continue;
            };
            if let Some(tex) = font.pages.get(page) {
                let tex = tex.clone();
                self.texture(cx, cy, g.w, g.h, g.u, g.v, &tex, style, col);
            }
            cx += g.w;
        }
    }

    /// The pixels as RGBA8 bytes (premultiplied).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.px.len() * 4];
        for (o, p) in out.chunks_exact_mut(4).zip(&self.px) {
            for c in 0..4 {
                // (`as u8` saturates: below 0 is 0, above 255 is 255.)
                o[c] = (p[c] * 255.0 + 0.5) as u8;
            }
        }
        out
    }
}

#[inline(always)]
fn blend(p: &mut [f32; 4], rgb: [f32; 3], style: Style) {
    match style {
        Style::Translucent => {
            p[0] = (p[0] + rgb[0]).min(1.0);
            p[1] = (p[1] + rgb[1]).min(1.0);
            p[2] = (p[2] + rgb[2]).min(1.0);
        }
        Style::Modulated => {
            p[0] = (p[0] * rgb[0] * 2.0).min(1.0);
            p[1] = (p[1] * rgb[1] * 2.0).min(1.0);
            p[2] = (p[2] * rgb[2] * 2.0).min(1.0);
        }
        Style::Normal | Style::Masked => *p = [rgb[0], rgb[1], rgb[2], 1.0],
    }
}

/// Word-wrap text to a width (TextWindow with SetWordWrap): '\n' and Unreal's "|n" break lines.
pub fn wrap(font: &UiFont, text: &str, width: i32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.replace("|n", "\n").split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let cand = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if !cur.is_empty() && font.width(&cand) > width {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = cand;
            }
        }
        out.push(cur);
    }
    out
}

/// Window.GetColorScaled: green above 80%, through yellow (40%) to red (10%), black at 0.
pub fn color_scaled(percent: f32) -> Rgb {
    if percent > 0.80 {
        [0, 255, 0]
    } else if percent > 0.40 {
        let m = (percent - 0.40) / 0.40;
        [(255.0 - 255.0 * m) as u8, 255, 0]
    } else if percent > 0.10 {
        let m = (percent - 0.10) / 0.30;
        [255, (255.0 * m) as u8, 0]
    } else if percent > 0.0 {
        [255, 0, 0]
    } else {
        [0, 0, 0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translucent_adds_and_masked_covers() {
        let tex = UiTex {
            w: 1,
            h: 1,
            px: vec![[255, 255, 255, 255]],
        };
        let mut c = Canvas::new(2, 1, 2);
        c.texture(0, 0, 2, 1, 0, 0, &tex, Style::Translucent, [100, 0, 0]);
        c.texture(0, 0, 1, 1, 0, 0, &tex, Style::Translucent, [100, 0, 0]);
        let b = c.to_rgba8();
        assert_eq!(&b[0..4], &[200, 0, 0, 0]);
        assert_eq!(&b[4 * 2..4 * 2 + 4], &[100, 0, 0, 0]);
        c.texture(1, 0, 1, 1, 0, 0, &tex, Style::Masked, [10, 20, 30]);
        let b = c.to_rgba8();
        assert_eq!(&b[4 * 2..4 * 2 + 4], &[10, 20, 30, 255]);
        // Each UI unit is a 2 x 2 block.
        assert_eq!(c.px.len(), 2 * 2 * 1 * 2);
    }

    #[test]
    fn scaled_colors_match_the_script() {
        assert_eq!(color_scaled(1.0), [0, 255, 0]);
        assert_eq!(color_scaled(0.6), [127, 255, 0]);
        assert_eq!(color_scaled(0.25), [255, 127, 0]);
        assert_eq!(color_scaled(0.05), [255, 0, 0]);
        assert_eq!(color_scaled(0.0), [0, 0, 0]);
    }
}
