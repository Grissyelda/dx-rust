//! Skins changed while a map is up (a script setting `MultiSkins[i]` on a pawn or decoration that's already drawn):
//! every part of its mesh that draws one of those slots gets a copy of its material with the new texture. Skeletal bodies
//! copy their textures from those parts when built, so they're rebuilt after the swap.
//! The texture names come from the mod's own script (missions/tnm.rs); nothing is drawn from our side.

use bevy::prelude::*;
use std::sync::Mutex;

/// Who gets the skins: pawns with a Tag, or the mesh actor (pawn or decoration) placed as this actor.
enum Target {
    Tag(String),
    Actor(usize),
}

/// (who, [(MultiSkins slot, texture path)]).
static REQS: Mutex<Vec<(Target, Vec<(u32, String)>)>> = Mutex::new(Vec::new());

/// Give every pawn with this Tag these MultiSkins.
pub fn request_tag(tag: &str, skins: Vec<(u32, String)>) {
    if let Ok(mut q) = REQS.lock() {
        q.push((Target::Tag(tag.to_string()), skins));
    }
}

/// Give the pawn or decoration placed as this actor these MultiSkins.
pub fn request_actor(actor: usize, skins: Vec<(u32, String)>) {
    if let Ok(mut q) = REQS.lock() {
        q.push((Target::Actor(actor), skins));
    }
}

pub fn reset_level() {
    if let Ok(mut q) = REQS.lock() {
        q.clear();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn reskin_system(
    mut commands: Commands,
    level: Res<crate::LevelRes>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    pawns: Query<(Entity, &crate::actors::Animated, &Children)>,
    decos: Query<(Entity, &crate::actors::Deco, &Children), Without<crate::actors::Animated>>,
    parts: Query<(
        &Mesh3d,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>,
    )>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<crate::gpuanim::AnimMaterial>>,
) {
    let reqs = match REQS.lock() {
        Ok(mut q) if !q.is_empty() => std::mem::take(&mut *q),
        _ => return,
    };
    let lvl = &level.0;
    for (target, skins) in reqs {
        let tex: Vec<(u32, Option<Handle<Image>>)> =
            skins.iter().map(|(s, p)| (*s, crate::weapons::image_for(&mut assets, &mut images, p))).collect();
        let wanted = |actor: usize| match &target {
            Target::Actor(a) => *a == actor,
            Target::Tag(t) => lvl.actors.get(actor).is_some_and(|x| crate::ai::actor_tag(x).eq_ignore_ascii_case(t)),
        };
        let list: Vec<(Entity, usize, &[(Handle<Mesh>, usize)], &Children, bool)> = pawns
            .iter()
            .map(|(e, a, k)| (e, a.mesh_actor, a.parts.as_slice(), k, true))
            .chain(decos.iter().map(|(e, d, k)| (e, d.mesh_actor, d.parts.as_slice(), k, false)))
            .collect();
        for (e, mesh_actor, handles, kids, pawn) in list {
            let Some(ma) = lvl.mesh_actors.get(mesh_actor) else { continue };
            if !wanted(ma.actor) {
                continue;
            }
            let mut changed = false;
            for c in kids.iter() {
                let Ok((m3, sm, am)) = parts.get(c) else { continue };
                let Some(pi) = handles.iter().find(|(h, _)| *h == m3.0).map(|(_, i)| *i) else { continue };
                let Some(part) = ma.parts.get(pi) else { continue };
                // The part's first slot decides (a part draws one texture).
                let Some(img) = part.slots.iter().find_map(|s| tex.iter().find(|(t, _)| t == s)).and_then(|(_, h)| h.clone())
                else {
                    continue;
                };
                if let Some(m) = sm.and_then(|h| std_mats.get(&h.0)).cloned() {
                    let mut m = m;
                    m.base_color_texture = Some(img);
                    commands.entity(c).try_insert(MeshMaterial3d(std_mats.add(m)));
                    changed = true;
                } else if let Some(m) = am.and_then(|h| anim_mats.get(&h.0)).cloned() {
                    let mut m = m;
                    m.base.base_color_texture = Some(img);
                    commands.entity(c).try_insert(MeshMaterial3d(anim_mats.add(m)));
                    changed = true;
                }
            }
            if changed && pawn {
                crate::skeletal::rebuild(e);
            }
        }
    }
}
