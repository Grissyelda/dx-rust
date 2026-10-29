//! DeusExPlayer.DrugEffects (poison, alcohol): while drugEffectTimer runs, the view wanders (pitch and yaw drift by
//! 1024 x cos/sin(time x mult) x dt x mult, mult = timer / 10 up to 3), the field of view narrows by the timer's
//! seconds (with a one-degree flicker, never below 30), and the HUD background is the DrunkFX texture, modulated and
//! stretched over the screen. DrunkFX is a WetTexture (DrunkFXRef rippled as by water), redone here on the CPU.

use crate::ai::PlayerHealth;
use crate::FlyCam;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

#[derive(Component)]
pub struct DrugOverlay;

/// The DrunkFX source picture (grey levels 0..1) the ripples are applied to, and the image shown.
#[derive(Resource)]
pub struct DrunkFx {
    src: Vec<f32>,
    n: usize,
    image: Handle<Image>,
    next: f32,
}

/// The screen is modulated by DrunkFX (texture x 2). Only darkening matters (the picture tops out at about 90/255), so
/// it is drawn as black with alpha = 1 - 2 x grey, stretched over the whole screen like the HUD background.
pub fn spawn_drug_overlay(
    commands: &mut Commands,
    assets: &mut crate::weapons::RuntimeAssets,
    images: &mut Assets<Image>,
) {
    let n = 128usize;
    let src: Vec<f32> = match assets.lib.load("Effects.UserInterface.DrunkFXRef") {
        Some((t, mips))
            if t.mips
                .first()
                .map(|m| m.width as usize == n && m.height as usize == n)
                .unwrap_or(false) =>
        {
            mips[0]
                .chunks_exact(4)
                .map(|c| c[0] as f32 / 255.0)
                .collect()
        }
        _ => (0..n * n)
            .map(|i| 0.2 + 0.1 * (((i % n) as f32 * 0.2).sin() * ((i / n) as f32 * 0.15).cos()))
            .collect(),
    };
    let img = Image::new(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; n * n * 4],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    let image = images.add(img);
    commands.spawn((
        ImageNode::new(image.clone()).with_mode(bevy::ui::widget::NodeImageMode::Stretch),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        Visibility::Hidden,
        Pickable::IGNORE,
        GlobalZIndex(-10),
        DrugOverlay,
    ));
    commands.insert_resource(DrunkFx {
        src,
        n,
        image,
        next: 0.0,
    });
}

/// WetTexture stand-in: the source picture is displaced by travelling sine waves, so it ripples like water.
fn render_wet(fx: &DrunkFx, t: f32, out: &mut [u8]) {
    let n = fx.n;
    let tau = std::f32::consts::TAU;
    for y in 0..n {
        for x in 0..n {
            let (fx_, fy) = (x as f32 / n as f32, y as f32 / n as f32);
            let dx = 4.0 * (tau * (fy * 2.0) + t * 2.3).sin()
                + 2.0 * (tau * (fx_ + fy) * 3.0 - t * 1.7).sin();
            let dy = 4.0 * (tau * (fx_ * 2.0) + t * 1.9).cos()
                + 2.0 * (tau * (fx_ - fy) * 3.0 + t * 2.1).cos();
            let sx = ((x as f32 + dx).round() as i32).rem_euclid(n as i32) as usize;
            let sy = ((y as f32 + dy).round() as i32).rem_euclid(n as i32) as usize;
            let g = fx.src[sy * n + sx];
            let a = (1.0 - 2.0 * g).clamp(0.0, 1.0);
            let o = (y * n + x) * 4;
            out[o..o + 4].copy_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn drug_system(
    time: Res<Time>,
    ph: Res<PlayerHealth>,
    mut cam: Single<&mut FlyCam, Without<DrugOverlay>>,
    mut overlay: Single<&mut Visibility, With<DrugOverlay>>,
    mut fx: ResMut<DrunkFx>,
    mut images: ResMut<Assets<Image>>,
) {
    let _prof = crate::prof::scope("drug::drug_system");
    if ph.drug_timer <= 0.0 || ph.dead {
        **overlay = Visibility::Hidden;
        return;
    }
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let mult = (ph.drug_timer / 10.0).clamp(0.0, 3.0);
    let unit = std::f32::consts::TAU / 65536.0;
    let dp = (1024.0 * (t * mult).cos() * dt * mult).clamp(-4096.0, 4096.0) * unit;
    let dy = (1024.0 * (t * mult).sin() * dt * mult).clamp(-4096.0, 4096.0) * unit;
    cam.pitch = (cam.pitch + dp).clamp(-1.55, 1.55);
    cam.yaw -= dy;
    **overlay = Visibility::Inherited;
    // Ripple the picture about 25 times a second.
    fx.next -= dt;
    if fx.next <= 0.0 {
        fx.next = 0.04;
        let h = fx.image.clone();
        if let Some(mut img) = images.get_mut(&h) {
            if let Some(data) = img.data.as_mut() {
                render_wet(&fx, t, data);
                if std::env::var("DXVIEW_FX").map_or(false, |v| v.contains("dump")) {
                    let _ = std::fs::write("/tmp/drunk.raw", &data[..]);
                }
            }
        }
    }
}

/// Field of view while drugged: Default.DesiredFOV - timer + Rand(2), at least 30.
pub fn drug_fov(ph: &PlayerHealth, base: f32, n: &mut u32) -> f32 {
    if ph.drug_timer <= 0.0 || ph.dead {
        return base;
    }
    *n = n.wrapping_mul(1103515245).wrapping_add(12345);
    let r = ((*n >> 16) % 2) as f32;
    (base - ph.drug_timer + r).clamp(30.0, base)
}

/// DeusExWeapon.Tick while zoomed: every quarter second pick a new drift of currentAccuracy x +-2048 rotator units
/// per second for yaw and pitch, so a poor shot can't hold the scope still.
pub fn scope_sway_system(
    time: Res<Time>,
    aim: Res<crate::hud::Aim>,
    zoom: Res<crate::weapons::Zoom>,
    weapons: Res<crate::weapons::Weapons>,
    mut cam: Single<&mut FlyCam, Without<DrugOverlay>>,
    mut st: Local<(f32, f32, f32, u64)>,
) {
    let dt = time.delta_secs();
    let unit = std::f32::consts::TAU / 65536.0;
    if st.3 == 0 {
        st.3 = 0x5C0_9E;
    }
    // Recoil (DeusExWeapon.Tick): while the Shoot animation plays the view kicks up by (4096..8192) x recoil and
    // sideways by +-2048 x recoil per second; recoil = recoilStrength + skill x 2.
    if let Some(h) = weapons.held.as_ref() {
        let mut recoil = (h.stats.recoil + h.stats.skill * 2.0).max(0.0);
        // GMDX Marksman perk: half the recoil on rifles.
        if crate::perks::has("PerkMarksman") && h.stats.governing_skill.as_deref().is_some_and(|g| g.eq_ignore_ascii_case("SkillWeaponRifle")) {
            recoil *= 0.5;
        }
        if recoil > 0.0 && h.state == crate::weapons::WState::Firing && !h.clip.done() {
            let mut r = || {
                st.3 =
                    st.3.wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                ((st.3 >> 33) % 4096) as f32
            };
            let (y, p) = (r() - 2048.0, r() + 4096.0);
            cam.yaw -= dt * y * recoil * unit;
            cam.pitch = (cam.pitch + dt * p * recoil * unit).clamp(-1.55, 1.55);
        }
    }
    let scoped = zoom.0.is_some() && weapons.held.as_ref().map_or(false, |h| h.stats.has_scope);
    if !scoped {
        st.0 = 0.0;
        return;
    }
    if st.0 > 0.25 || (st.1 == 0.0 && st.2 == 0.0) {
        let mut r = || {
            st.3 =
                st.3.wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
            ((st.3 >> 33) % 4096) as f32 - 2048.0
        };
        let (y, p) = (r(), r());
        st.1 = aim.sway * y;
        st.2 = aim.sway * p;
        st.0 = (st.0 - 0.25).max(0.0);
    }
    st.0 += dt;
    cam.yaw -= st.1 * dt * unit;
    cam.pitch = (cam.pitch + st.2 * dt * unit).clamp(-1.55, 1.55);
}
