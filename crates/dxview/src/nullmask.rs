//! Mesh sections Deus Ex hides with a texture that draws nothing: GrayMaskTex on a modulated section (mid-grey
//! modulation leaves what's behind unchanged), BlackMaskTex on a translucent one (adding black adds nothing) and
//! PinkMaskTex on a masked one (every texel is the see-through colour). The original drew them anyway; here every
//! one was a see-through draw of its own, sorted with the rest (the NSF's masks and goggles slots alone were about
//! 30 of the ~110 sorted draws in a view of the Island's dock). They're hidden instead, and shown again if a skin
//! change gives them a real texture (an AlarmLight's lens lit, an outfit change).

use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gray,
    Black,
    Pink,
}

static IDS: Mutex<Option<HashMap<AssetId<Image>, Kind>>> = Mutex::new(None);

/// Remember an image made from a texture by name (level textures, runtime skins).
pub fn register(h: &Handle<Image>, name: &str) {
    let short = name.rsplit('.').next().unwrap_or(name).split('#').next().unwrap_or("");
    let kind = if short.eq_ignore_ascii_case("GrayMaskTex") {
        Kind::Gray
    } else if short.eq_ignore_ascii_case("BlackMaskTex") {
        Kind::Black
    } else if short.eq_ignore_ascii_case("PinkMaskTex") {
        Kind::Pink
    } else {
        return;
    };
    if let Ok(mut g) = IDS.lock() {
        g.get_or_insert_with(HashMap::new).insert(h.id(), kind);
    }
}

/// Whether a material with this texture and blend draws nothing.
fn draws_nothing(tex: Option<&Handle<Image>>, alpha: AlphaMode) -> bool {
    let Some(t) = tex else { return false };
    let Some(kind) = IDS.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&t.id()).copied())) else {
        return false;
    };
    matches!(
        (kind, alpha),
        (Kind::Gray, AlphaMode::Multiply) | (Kind::Black, AlphaMode::Add) | (Kind::Pink, AlphaMode::Mask(_))
    )
}

/// On a part this module hid.
#[derive(Component)]
pub struct MaskHidden;

type StdParts<'a> = (Entity, Ref<'a, MeshMaterial3d<StandardMaterial>>, &'a mut Visibility, Has<MaskHidden>);
type AnimParts<'a> = (Entity, Ref<'a, MeshMaterial3d<crate::gpuanim::AnimMaterial>>, &'a mut Visibility, Has<MaskHidden>);

/// Hides parts whose material just became a no-op mask, and shows again the ones whose material stopped being one.
/// (A material handle can arrive before its asset is in the store: those parts are looked at again next frame.)
#[allow(clippy::type_complexity)]
pub fn nullmask_system(
    mut commands: Commands,
    std_mats: Res<Assets<StandardMaterial>>,
    anim_mats: Res<Assets<crate::gpuanim::AnimMaterial>>,
    mut std_parts: Query<StdParts, (Without<crate::skeletal::HiddenPart>, Without<MeshMaterial3d<crate::gpuanim::AnimMaterial>>)>,
    mut anim_parts: Query<AnimParts, (Without<crate::skeletal::HiddenPart>, Without<MeshMaterial3d<StandardMaterial>>)>,
    mut pending: Local<Vec<Entity>>,
) {
    let _prof = crate::prof::scope("nullmask::nullmask_system");
    let apply = |commands: &mut Commands, e: Entity, nothing: bool, vis: &mut Mut<Visibility>, hidden: bool| {
        if nothing && !hidden {
            vis.set_if_neq(Visibility::Hidden);
            commands.entity(e).insert(MaskHidden);
        } else if !nothing && hidden {
            vis.set_if_neq(Visibility::Inherited);
            commands.entity(e).remove::<MaskHidden>();
        }
    };
    let retry = std::mem::take(&mut *pending);
    for (e, m, mut vis, hidden) in &mut std_parts {
        if !m.is_changed() && !retry.contains(&e) {
            continue;
        }
        match std_mats.get(&m.0) {
            Some(mat) => apply(&mut commands, e, draws_nothing(mat.base_color_texture.as_ref(), mat.alpha_mode), &mut vis, hidden),
            None => pending.push(e),
        }
    }
    for (e, m, mut vis, hidden) in &mut anim_parts {
        if !m.is_changed() && !retry.contains(&e) {
            continue;
        }
        match anim_mats.get(&m.0) {
            Some(mat) => apply(&mut commands, e, draws_nothing(mat.base.base_color_texture.as_ref(), mat.base.alpha_mode), &mut vis, hidden),
            None => pending.push(e),
        }
    }
}
