//! The Nameless Mod's fader (TNMMissionScript.fade, NarcFadeInInfo): a pawn drawn translucent while its ScaleGlow
//! runs 0 -> 1 (fade in) or 1 -> 0 (fade out) over a few seconds; a faded-in one goes back to its own style, a
//! faded-out one stays in the world unseen. The endings use it: the aliens appearing, the circle of pawns one by one,
//! Trestkon's double leaving, Gwog taking Beeblequix's and Pattie's place.
//!
//! Pawns drawn through the skeletal bodies can't be made translucent here: they appear / vanish at the end.

use bevy::prelude::*;
use std::sync::Mutex;

use crate::actors::Deco;
use crate::ai::Npc;

/// Who to fade: by Tag, or by class (and subclasses).
#[derive(Clone, Debug)]
pub enum Who {
    Tag(String),
    Class(String),
    /// One placed pawn (actor index).
    Actor(usize),
}

/// (who, seconds, fade in, start after this many seconds).
static REQ: Mutex<Vec<(Who, f32, bool, f32)>> = Mutex::new(Vec::new());

pub fn request(who: Who, secs: f32, fade_in: bool) {
    request_after(who, secs, fade_in, 0.0);
}

pub fn request_after(who: Who, secs: f32, fade_in: bool, delay: f32) {
    if let Ok(mut q) = REQ.lock() {
        q.push((who, secs, fade_in, delay));
    }
}

pub fn reset_level() {
    if let Ok(mut q) = REQ.lock() {
        q.clear();
    }
}

#[derive(Component)]
pub struct Fading {
    t: f32,
    secs: f32,
    fade_in: bool,
    std: Vec<(Entity, Handle<StandardMaterial>, Handle<StandardMaterial>, Color)>,
    anim: Vec<(Entity, Handle<crate::gpuanim::AnimMaterial>, Handle<crate::gpuanim::AnimMaterial>, Color)>,
}

/// A pawn a fade has left invisible (faded out): kept hidden.
#[derive(Component)]
pub struct FadedOut;

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn fade_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<crate::gpuanim::AnimMaterial>>,
    mut pawns: Query<(Entity, &Npc, &Deco, Option<&mut Fading>, &mut Visibility, Option<&FadedOut>, Option<&crate::ai::OutOfWorld>)>,
    children: Query<&Children>,
    parts: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>), Without<Npc>>,
    mut pending: Local<Vec<(Who, f32, bool, f32)>>,
) {
    let lvl = &level.0;
    let dt = time.delta_secs();
    pending.extend(REQ.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default());
    let mut keep = Vec::new();
    for (who, secs, fade_in, delay) in std::mem::take(&mut *pending) {
        if delay > 0.0 {
            // A pawn waiting for its turn to fade in isn't seen before it.
            if let (Who::Actor(i), true) = (&who, fade_in) {
                for (_, npc, _, _, mut vis, _, _) in &mut pawns {
                    if npc.actor == *i {
                        vis.set_if_neq(Visibility::Hidden);
                    }
                }
            }
            keep.push((who, secs, fade_in, delay - dt));
            continue;
        }
        let mut found = false;
        for (e, npc, _, fading, mut vis, faded, oow) in &mut pawns {
            let a = &lvl.actors[npc.actor];
            let hit = match &who {
                Who::Tag(t) => crate::ai::actor_tag(a).eq_ignore_ascii_case(t),
                Who::Class(c) => crate::classtree::is_a(&a.class, c),
                Who::Actor(i) => npc.actor == *i,
            };
            if !hit {
                continue;
            }
            // A pawn just put in the world isn't shown yet: wait for it.
            if oow.is_some() {
                continue;
            }
            found = true;
            if faded.is_some() && fade_in {
                commands.entity(e).remove::<FadedOut>();
            }
            if fading.is_some() {
                // Already fading (its materials are the faded copies): this fade starts over on those.
                commands.entity(e).remove::<Fading>();
            }
            let mut std = Vec::new();
            let mut anim = Vec::new();
            for c in children.get(e).into_iter().flat_map(|c| c.iter()) {
                let Ok((sm, am)) = parts.get(c) else { continue };
                if let Some(m) = sm {
                    if let Some(orig) = materials.get(&m.0).cloned() {
                        let col = orig.base_color;
                        let mut t = orig;
                        t.alpha_mode = AlphaMode::Add;
                        let h = materials.add(t);
                        commands.entity(c).insert(MeshMaterial3d(h.clone()));
                        std.push((c, m.0.clone(), h, col));
                    }
                }
                if let Some(m) = am {
                    if let Some(orig) = anim_mats.get(&m.0).cloned() {
                        let col = orig.base.base_color;
                        let mut t = orig;
                        t.base.alpha_mode = AlphaMode::Add;
                        let h = anim_mats.add(t);
                        commands.entity(c).insert(MeshMaterial3d(h.clone()));
                        anim.push((c, m.0.clone(), h, col));
                    }
                }
            }
            vis.set_if_neq(Visibility::Inherited);
            commands.entity(e).insert(Fading { t: 0.0, secs: secs.max(0.01), fade_in, std, anim });
        }
        if !found {
            // Not there yet (EnterWorld this frame): try again for a few seconds.
            if delay > -3.0 {
                keep.push((who, secs, fade_in, delay - dt));
            }
        }
    }
    *pending = keep;
    for (e, _, _, fading, mut vis, faded, _) in &mut pawns {
        if faded.is_some() {
            vis.set_if_neq(Visibility::Hidden);
        }
        let Some(mut f) = fading else { continue };
        f.t += dt;
        let k = (f.t / f.secs).clamp(0.0, 1.0);
        let g = if f.fade_in { k } else { 1.0 - k };
        for (_, _, h, c) in &f.std {
            if let Some(mut m) = materials.get_mut(h) {
                let l = c.to_linear();
                m.base_color = Color::linear_rgba(l.red * g, l.green * g, l.blue * g, l.alpha);
            }
        }
        for (_, _, h, c) in &f.anim {
            if let Some(mut m) = anim_mats.get_mut(h) {
                let l = c.to_linear();
                m.base.base_color = Color::linear_rgba(l.red * g, l.green * g, l.blue * g, l.alpha);
            }
        }
        if f.t < f.secs {
            continue;
        }
        // finish(): a faded-in pawn back to its own look; a faded-out one stays unseen.
        if f.fade_in {
            for (c, orig, _, _) in &f.std {
                commands.entity(*c).insert(MeshMaterial3d(orig.clone()));
            }
            for (c, orig, _, _) in &f.anim {
                commands.entity(*c).insert(MeshMaterial3d(orig.clone()));
            }
        } else {
            vis.set_if_neq(Visibility::Hidden);
            commands.entity(e).insert(FadedOut);
        }
        commands.entity(e).remove::<Fading>();
    }
}
