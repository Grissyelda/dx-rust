//! Ours: small helpers for Bevy UI nodes.

use bevy::prelude::*;

/// Shows or hides a UI subtree. A hidden layer is taken out of layout too (`Display::None`), so Bevy's UI
/// layout, text shaping and visibility propagation skip it altogether, and nothing is written when the state
/// is already right (a write through `Mut` marks the component changed, which re-propagates the whole subtree).
pub fn show(mut vis: Mut<Visibility>, mut node: Mut<Node>, on: bool) {
    let want_vis = if on { Visibility::Inherited } else { Visibility::Hidden };
    let want_disp = if on { Display::Flex } else { Display::None };
    if *vis != want_vis {
        *vis = want_vis;
    }
    if node.display != want_disp {
        node.display = want_disp;
    }
}

/// Sets a node's left / top / width / height in pixels, only when one of them differs (a write marks the node
/// changed and re-runs the UI layout).
pub fn set_rect(node: &mut Mut<Node>, left: f32, top: f32, width: f32, height: f32) {
    let (l, t, w, h) = (Val::Px(left), Val::Px(top), Val::Px(width), Val::Px(height));
    if node.left != l || node.top != t || node.width != w || node.height != h {
        node.left = l;
        node.top = t;
        node.width = w;
        node.height = h;
    }
}

