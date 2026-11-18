//! The Nameless Mod's ban gloves (TNMPawn.BanPawn + BanInfo + BanEffect): the banned pawn is killed (its _Dead flag
//! never expires), turns translucent with the IntroBan sound, a glowing copy of its body at half size inside it
//! (BanEffect: the mesh at half the owner's DrawScale, only the first skin slot drawn, with the BanEffect texture,
//! animated with its owner), and both fade - ScaleGlow down by 0.04 a tick, the effect at half - until the pawn is
//! gone: a Shadow and a ScorchMark on the floor under it, a small SmokeTrail, and its nano keys thrown out (the rest of
//! what it carried goes with it).

use crate::actors::Deco;
use crate::ai::Npc;
use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

/// Pawns (blockers) whose ban starts next frame.
static START: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// BanInfo.UpdateBanFX's ScaleGlow step, per tick; one tick taken as 1/60 s.
const STEP: f32 = 0.04 * 60.0;

static START_TAGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A mission script's BanPawn / BanScaraFX (BanInfo.StartBan) on every pawn with this Tag.
pub fn start_tag(tag: &str) {
    if let Ok(mut q) = START_TAGS.lock() {
        q.push(tag.to_string());
    }
}

/// A pawn was hit by the ban gloves (it dies as usual; the fade follows).
pub fn start(blocker: usize) {
    if let Ok(mut q) = START.lock() {
        if !q.contains(&blocker) {
            q.push(blocker);
        }
    }
}

/// A ban in progress on a pawn's root entity.
#[derive(Component)]
pub struct Banning {
    glow: f32,
    /// The pawn's own parts with their faded materials, and the BanEffect copies.
    own: Vec<(Entity, Handle<StandardMaterial>, Color)>,
    fx: Vec<(Entity, Handle<StandardMaterial>)>,
    anim_own: Vec<(Entity, Handle<crate::gpuanim::AnimMaterial>, Color)>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn ban_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<crate::LevelRes>,
    mut blockers: ResMut<crate::actors::Blockers>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut anim_mats: ResMut<Assets<crate::gpuanim::AnimMaterial>>,
    mut pawns: Query<(Entity, &Npc, &Deco, Option<&mut Banning>, &mut Visibility)>,
    children: Query<&Children>,
    parts: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<crate::gpuanim::AnimMaterial>>, Option<&Mesh3d>, &Transform), Without<Npc>>,
) {
    let lvl = &level.0;
    let mut starts: Vec<usize> = START.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for tag in START_TAGS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for (_, npc, ..) in &pawns {
            let alive = blockers.list.get(npc.blocker).is_some_and(|c| c.radius > 0.0);
            if alive && lvl.actors.get(npc.actor).is_some_and(|a| crate::ai::actor_tag(a).eq_ignore_ascii_case(&tag)) && !starts.contains(&npc.blocker) {
                starts.push(npc.blocker);
            }
        }
    }
    for b in starts {
        let Some((e, npc, deco, _, _)) = pawns.iter().find(|p| p.1.blocker == b) else { continue };
        let at = blockers.list.get(b).map_or(deco.body.location, |c| c.center);
        crate::sound::play("TNMSounds.OneShot.IntroBan", at, 2.0, 1600.0, 1.0);
        // The BanEffect texture, translucent (additive), ScaleGlow 0.5 of the owner's.
        let fx_img = crate::weapons::image_for(&mut assets, &mut images, "TNM_Core_Tex.Effects.BanEffect");
        let mut own = Vec::new();
        let mut anim_own = Vec::new();
        let mut fx = Vec::new();
        let ma = lvl.mesh_actors.get(deco.mesh_actor);
        for c in children.get(e).into_iter().flat_map(|c| c.iter()) {
            let Ok((std, anim, mesh, tf)) = parts.get(c) else { continue };
            if let Some(m) = std {
                if let Some(orig) = materials.get(&m.0).cloned() {
                    let color = orig.base_color;
                    let mut t = orig;
                    t.alpha_mode = AlphaMode::Add;
                    let h = materials.add(t);
                    commands.entity(c).insert(MeshMaterial3d(h.clone()));
                    own.push((c, h, color));
                }
            }
            if let Some(m) = anim {
                if let Some(orig) = anim_mats.get(&m.0).cloned() {
                    let color = orig.base.base_color;
                    let mut t = orig;
                    t.base.alpha_mode = AlphaMode::Add;
                    let h = anim_mats.add(t);
                    commands.entity(c).insert(MeshMaterial3d(h.clone()));
                    anim_own.push((c, h, color));
                }
            }
            // The BanEffect copy: the first skin slot only (the others are BlackMaskTex), at half scale.
            let first_slot = ma.and_then(|ma| ma.parts.iter().position(|p| p.slots.first().copied().unwrap_or(0) == 0)).is_some();
            if let (Some(mesh), Some(img), true) = (mesh, fx_img.clone(), first_slot) {
                if std.is_none() {
                    continue;
                }
                let mat = materials.add(StandardMaterial {
                    base_color_texture: Some(img),
                    base_color: Color::srgb(0.5, 0.5, 0.5),
                    unlit: true,
                    alpha_mode: AlphaMode::Add,
                    cull_mode: None,
                    ..default()
                });
                let mut t = *tf;
                t.scale *= 0.5;
                t.translation *= 0.5;
                let fe = commands.spawn((Mesh3d(mesh.0.clone()), MeshMaterial3d(mat.clone()), t)).id();
                commands.entity(e).add_child(fe);
                fx.push((fe, mat));
            }
        }
        commands.entity(e).insert(Banning { glow: 1.0, own, fx, anim_own });
        println!("TNM ban: {} banned", lvl.actors[npc.actor].name);
    }

    let dt = time.delta_secs();
    let lvl = &level.0;
    for (e, npc, deco, ban, mut vis) in &mut pawns {
        let Some(mut ban) = ban else { continue };
        ban.glow -= STEP * dt;
        let g = ban.glow.max(0.0);
        for (_, h, c) in &ban.own {
            if let Some(mut m) = materials.get_mut(h) {
                let l = c.to_linear();
                m.base_color = Color::linear_rgba(l.red * g, l.green * g, l.blue * g, l.alpha);
            }
        }
        for (_, h, c) in &ban.anim_own {
            if let Some(mut m) = anim_mats.get_mut(h) {
                let l = c.to_linear();
                m.base.base_color = Color::linear_rgba(l.red * g, l.green * g, l.blue * g, l.alpha);
            }
        }
        for (_, h) in &ban.fx {
            if let Some(mut m) = materials.get_mut(h) {
                let k = g * 0.5;
                m.base_color = Color::srgb(k, k, k);
            }
        }
        if ban.glow > 0.0 {
            continue;
        }
        // Gone: a shadow, a scorch mark and a little smoke where it stood; its keys thrown out.
        let (center, h) = blockers.list.get(npc.blocker).map_or((deco.body.location, 40.0), |c| (c.center, c.height));
        let feet = center - DVec3::new(0.0, 0.0, h);
        let floor = level.0.collision.trace(center, feet - DVec3::new(0.0, 0.0, 32.0), DVec3::ZERO);
        let at = center + (feet - DVec3::new(0.0, 0.0, 32.0) - center) * floor.fraction.clamp(0.0, 1.0);
        let n = if floor.fraction < 1.0 { floor.normal } else { DVec3::Z };
        crate::impacts::mark(at, n, "DeusExItems.Skins.FlatFXTex40", 0.4);
        crate::impacts::mark(at, n, "DeusExItems.Skins.FlatFXTex39", 0.3);
        crate::impacts::smoke(at, 0.5, 5.0);
        let tag = crate::ai::actor_tag(&lvl.actors[npc.actor]).to_string();
        for who in [npc.bind_name.as_str(), tag.as_str()] {
            while let Some((key, desc)) = (!who.is_empty()).then(|| crate::keys::take_carried(who)).flatten() {
                crate::keys::drop_world(&key, &desc, center);
            }
        }
        if let Some(c) = blockers.list.get_mut(npc.blocker) {
            c.radius = 0.0;
            c.height = 0.0;
        }
        for (fe, _) in &ban.fx {
            commands.entity(*fe).try_despawn();
        }
        *vis = Visibility::Hidden;
        crate::scriptworld::mark_hidden(npc.actor, true);
        commands.entity(e).remove::<Banning>();
        let _ = &mut assets;
    }
}
