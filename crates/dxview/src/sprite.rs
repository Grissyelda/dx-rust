//! Ours: shared materials and the one quad for the transient sprites (smoke, fire, rings, bubbles, muzzle
//! flashes, gas clouds). Every effect used to add its own StandardMaterial and its own quad mesh and fade by
//! writing the material every frame, which re-prepared it on the render thread each frame and kept every sprite
//! its own draw. Here a sprite's material is shared by every sprite with the same picture, and fading swaps to one
//! of 17 shared steps of that material (no asset ever changes after it is made).

use bevy::asset::AssetId;
use bevy::prelude::*;
use std::collections::HashMap;

/// Fade steps per picture (0 = black, STEPS = full).
pub const STEPS: u8 = 16;

#[derive(Default)]
pub struct Cache {
    /// (picture, blend, tint, step) -> material.
    mats: HashMap<(Option<AssetId<Image>>, u8, [u16; 3], u8), Handle<StandardMaterial>>,
    /// A material made here -> its key without the step.
    base: HashMap<AssetId<StandardMaterial>, (Option<AssetId<Image>>, u8, [u16; 3])>,
    quad: Option<Handle<Mesh>>,
}

/// How the sprite blends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Add,
    Blend,
    /// A decal (modulated picture, no depth bias).
    Decal,
    /// A bullet hole decal (depth bias 40 so it sits over the wall).
    Hole,
}

fn build(img: Option<Handle<Image>>, blend: Blend, color: Color) -> StandardMaterial {
    match blend {
        Blend::Decal | Blend::Hole => {
            let mut m = crate::impacts::decal_material(img.unwrap_or_default(), blend == Blend::Hole);
            m.base_color = color;
            m
        }
        _ => StandardMaterial {
            base_color_texture: img,
            base_color: color,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: if blend == Blend::Add { AlphaMode::Add } else { AlphaMode::Blend },
            ..default()
        },
    }
}

fn blend_of(b: u8) -> Blend {
    match b {
        0 => Blend::Add,
        1 => Blend::Blend,
        2 => Blend::Decal,
        _ => Blend::Hole,
    }
}

fn tint_key(c: [f32; 3]) -> [u16; 3] {
    [(c[0] * 1000.0) as u16, (c[1] * 1000.0) as u16, (c[2] * 1000.0) as u16]
}

impl Cache {
    /// The unit quad every sprite is drawn on.
    pub fn quad(&mut self, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        self.quad.get_or_insert_with(|| meshes.add(Rectangle::new(1.0, 1.0))).clone()
    }

    /// The full-brightness unlit, double-sided material for `img` (tint = linear colour multiplier).
    pub fn material_tinted(&mut self, materials: &mut Assets<StandardMaterial>, img: Option<Handle<Image>>, blend: Blend, tint: [f32; 3]) -> Handle<StandardMaterial> {
        self.step(materials, img, blend, tint, STEPS)
    }

    /// The full-brightness additive sprite material for `img`.
    pub fn material(&mut self, materials: &mut Assets<StandardMaterial>, img: Option<Handle<Image>>) -> Handle<StandardMaterial> {
        self.step(materials, img, Blend::Add, [1.0; 3], STEPS)
    }

    /// The shared decal material for a modulated picture (`hole` = bullet hole, raised off the wall).
    pub fn decal(&mut self, materials: &mut Assets<StandardMaterial>, img: Handle<Image>, hole: bool) -> Handle<StandardMaterial> {
        self.step(materials, Some(img), if hole { Blend::Hole } else { Blend::Decal }, [1.0; 3], STEPS)
    }

    fn step(&mut self, materials: &mut Assets<StandardMaterial>, img: Option<Handle<Image>>, blend: Blend, tint: [f32; 3], step: u8) -> Handle<StandardMaterial> {
        let key = (img.as_ref().map(|h| h.id()), blend as u8, tint_key(tint), step);
        if let Some(h) = self.mats.get(&key) {
            return h.clone();
        }
        let g = step as f32 / STEPS as f32;
        let h = materials.add(build(img, blend, Color::linear_rgb(tint[0] * g, tint[1] * g, tint[2] * g)));
        self.mats.insert(key, h.clone());
        self.base.insert(h.id(), (key.0, key.1, key.2));
        h
    }

    /// `base` (a material from here) at brightness `glow` (0..1, linear), quantised to the shared steps.
    /// Returns `base` itself for materials not made here.
    pub fn faded(&mut self, materials: &mut Assets<StandardMaterial>, base: &Handle<StandardMaterial>, glow: f32) -> Handle<StandardMaterial> {
        let Some(&(img, blend, tint)) = self.base.get(&base.id()) else { return base.clone() };
        let step = (glow.clamp(0.0, 1.0) * STEPS as f32).round() as u8;
        let key = (img, blend, tint, step);
        if let Some(h) = self.mats.get(&key) {
            return h.clone();
        }
        let g = step as f32 / STEPS as f32;
        let t = [tint[0] as f32 / 1000.0, tint[1] as f32 / 1000.0, tint[2] as f32 / 1000.0];
        let picture = self.mats.get(&(img, blend, tint, STEPS)).and_then(|h| materials.get(h)).and_then(|m| m.base_color_texture.clone());
        let h = materials.add(build(picture, blend_of(blend), Color::linear_rgb(t[0] * g, t[1] * g, t[2] * g)));
        self.mats.insert(key, h.clone());
        self.base.insert(h.id(), (img, blend, tint));
        h
    }
}

/// Puts `mat` on the mesh when it isn't already (no change is marked otherwise).
pub fn apply(slot: &mut Mut<MeshMaterial3d<StandardMaterial>>, mat: Handle<StandardMaterial>) {
    if slot.0 != mat {
        slot.0 = mat;
    }
}
