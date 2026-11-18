//! Warp zones (WarpZoneInfo.ActorEntered, Warp / UnWarp): walking into a warp zone's BSP zone carries the player out
//! of the zone it is linked to (OtherSideURL names that zone's ThisTag in the same map) - his place, velocity and
//! view turned through the two zones' WarpCoords. The arrival counts as a teleport (bJustTeleported), so the zone he
//! comes out in doesn't send him straight back; the next zone he walks into can. The Nameless Mod's warp mazes
//! (the voodoo shop, the corporate and downtown booths) are built from these.
//!
//! A mission script can relink a zone (SetupPortal / WarpZoneRandomize: a new OtherSideURL), see `relink`.
//! Not here: drawing what lies beyond a portal (the engine renders through it; here the portal face shows what is
//! behind it in this map).

use bevy::prelude::*;
use glam::DVec3;
use std::sync::Mutex;

/// Script overrides of OtherSideURL by ThisTag.
static RELINK: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

pub fn relink(this_tag: &str, other: &str) {
    if let Ok(mut r) = RELINK.lock() {
        r.retain(|x| !x.0.eq_ignore_ascii_case(this_tag));
        r.push((this_tag.to_string(), other.to_string()));
    }
}

/// The OtherSideURL a zone has now.
pub fn other_side(lvl: &dx_level::Level, this_tag: &str) -> Option<String> {
    if let Some(o) = RELINK.lock().ok().and_then(|r| r.iter().find(|x| x.0.eq_ignore_ascii_case(this_tag)).map(|x| x.1.clone())) {
        return Some(o);
    }
    lvl.zone_rules.iter().filter_map(|z| z.warp.as_ref()).find(|w| w.this_tag.eq_ignore_ascii_case(this_tag)).map(|w| w.other.clone())
}

/// Triggers waiting for the next frame: (Tag, only a zone still linked to itself - SetupPortal's test).
static TRIGGERS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

/// An event: WarpZoneInfo.Trigger on the warp zones with this Tag (OtherSideURL steps to the next Destination).
pub fn event(tag: &str) {
    if let Ok(mut q) = TRIGGERS.lock() {
        q.push((tag.to_string(), false));
    }
}

/// The Nameless Mod's SetupPortal: trigger the warp zones with this Tag that still lead to themselves (shut).
pub fn open_portal(tag: &str) {
    if let Ok(mut q) = TRIGGERS.lock() {
        q.push((tag.to_string(), true));
    }
}

/// Saved per map: the OtherSideURLs scripts and triggers set.
pub fn save() -> Vec<(String, String)> {
    RELINK.lock().map(|r| r.clone()).unwrap_or_default()
}

pub fn restore(v: &[(String, String)]) {
    if let Ok(mut r) = RELINK.lock() {
        *r = v.to_vec();
    }
}

pub fn reset_level() {
    if let Ok(mut r) = RELINK.lock() {
        r.clear();
    }
    if let Ok(mut q) = TRIGGERS.lock() {
        q.clear();
    }
}

#[derive(Default)]
pub struct WarpState {
    zone: i32,
    just_teleported: bool,
}

fn v(d: DVec3) -> [f64; 3] {
    [d.x, d.y, d.z]
}
fn d(a: [f64; 3]) -> DVec3 {
    DVec3::new(a[0], a[1], a[2])
}

pub fn warp_system(
    level: Res<crate::LevelRes>,
    mut player: ResMut<crate::Player>,
    mut cams: Query<&mut crate::FlyCam, Without<crate::SkyCam>>,
    mut st: Local<WarpState>,
    mut map_gen: Local<u64>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *st = WarpState { zone: -1, just_teleported: false };
    }
    let lvl = &level.0;
    if !lvl.zone_rules.iter().any(|z| z.warp.is_some()) {
        return;
    }
    for (tag, only_shut) in TRIGGERS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for w in lvl.zone_rules.iter().filter_map(|z| z.warp.as_ref()) {
            if !w.tag.eq_ignore_ascii_case(&tag) || w.destinations.first().is_none_or(|d| d.is_empty()) {
                continue;
            }
            let now = other_side(lvl, &w.this_tag).unwrap_or_default();
            if only_shut && !now.eq_ignore_ascii_case(&w.this_tag) {
                continue;
            }
            // Trigger: the destination after the current one (back to the first past the end or an empty one).
            let at = w.destinations.iter().position(|d| *d == now).map_or(0, |i| i + 1);
            let next = w.destinations.get(at).filter(|d| !d.is_empty()).unwrap_or(&w.destinations[0]).clone();
            println!("WARP zone {} now leads to {next}", w.this_tag);
            relink(&w.this_tag, &next);
        }
    }
    let loc = player.body.location;
    let zone = lvl.collision.zone_at(loc);
    if zone == st.zone {
        return;
    }
    let first = st.zone < 0;
    st.zone = zone;
    if first {
        return;
    }
    if st.just_teleported {
        st.just_teleported = false;
        return;
    }
    let Some(here) = lvl.zone_rules.get(zone as usize).and_then(|z| z.warp.as_ref()) else { return };
    let Some(url) = other_side(lvl, &here.this_tag) else { return };
    if url.is_empty() {
        return;
    }
    let Some((oz, there)) = lvl
        .zone_rules
        .iter()
        .enumerate()
        .find_map(|(i, z)| z.warp.as_ref().filter(|w| w.this_tag.eq_ignore_ascii_case(&url)).map(|w| (i, w)))
    else {
        return;
    };
    // UnWarp here, Warp there: place, velocity and view.
    let l = there.warp_point(here.unwarp_point(v(loc)));
    let vel = there.warp_vector(here.unwarp_vector(v(player.body.velocity)));
    let Ok(mut cam) = cams.single_mut() else { return };
    let ue_yaw = -(cam.yaw as f64) - std::f64::consts::FRAC_PI_2;
    let pitch = cam.pitch as f64;
    let fwd = DVec3::new(pitch.cos() * ue_yaw.cos(), pitch.cos() * ue_yaw.sin(), pitch.sin());
    let nf = d(there.warp_vector(here.unwarp_vector(v(fwd))));
    let new_yaw = nf.y.atan2(nf.x);
    let new_pitch = nf.z.atan2((nf.x * nf.x + nf.y * nf.y).sqrt());
    cam.yaw = (-new_yaw - std::f64::consts::FRAC_PI_2) as f32;
    cam.pitch = (new_pitch as f32).clamp(-1.55, 1.55);
    player.body.location = d(l);
    player.body.velocity = d(vel);
    st.just_teleported = true;
    st.zone = lvl.collision.zone_at(d(l));
    if st.zone != oz as i32 {
        // Came out somewhere that isn't the linked zone itself: the next zone change is a real one.
        st.just_teleported = false;
    }
    println!("WARP {} -> {} ({:.0},{:.0},{:.0})", here.this_tag, there.this_tag, l[0], l[1], l[2]);
}
