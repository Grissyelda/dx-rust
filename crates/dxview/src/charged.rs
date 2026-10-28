//! ChargedPickup (hazmat suit, ballistic armor, rebreather, tech goggles, thermoptic camo). Using one from the
//! belt turns it on (they are all bOneUseOnly: it leaves the hand and the inventory), then it drains
//! 4 x skill value of Charge every 0.1 s (untrained Environmental Training = 1.0) until it is used up.
//! DXReduceDamage: an active hazmat suit takes gas/poison/radiation damage down to 0.75 x skill, ballistic armor
//! takes shots and explosions down to 0.5 x skill. Tech goggles turn on the light-amplification box in the middle
//! of the screen (AugmentationDisplayWindow, vision level 0: SolidGreen modulated twice, "LightAmp Active").

use crate::ai::PlayerHealth;
use crate::movers::HudMessage;
use crate::{FlyCam, SkyCam};
use bevy::{camera::visibility::NoFrustumCulling, prelude::*};

pub struct Charged {
    pub class: String,
    pub name: String,
    pub charge: f32,
    pub max: f32,
    /// Drain per 0.1 s.
    pub drain: f32,
    pub expire: String,
    pub loop_snd: Option<String>,
    pub deactivate_snd: Option<String>,
}

#[derive(Resource, Default)]
pub struct ActiveCharged(pub Vec<Charged>);

pub fn is_charged(class: &str) -> bool {
    matches!(
        class,
        "HazMatSuit" | "BallisticArmor" | "Rebreather" | "TechGoggles" | "AdaptiveArmor"
    )
}

impl ActiveCharged {
    /// Turn one on. Returns false if one of that kind is already running.
    pub fn activate(&mut self, class: &str, defaults: &[unreal_pkg::props::Property]) -> bool {
        use unreal_pkg::props::{find, Value};
        if self.0.iter().any(|c| c.class == class) {
            return false;
        }
        let charge = match find(defaults, "Charge") {
            Some(Value::Int(n)) => *n as f32,
            _ => 500.0,
        };
        // ChargedPickup: drain = 4 x the value of skillNeeded (Environmental Training).
        let skill = match find(defaults, "skillNeeded") {
            Some(Value::Path(p)) => crate::abilities::skill(p.rsplit('.').next().unwrap_or(p)),
            _ => 1.0,
        };
        let s = |n: &str| match find(defaults, n) {
            Some(Value::Str(v)) => v.clone(),
            _ => String::new(),
        };
        // ChargedPickup.BeginState (Activated): ActivateSound, then LoopSound as the AmbientSound.
        if let Some(snd) = crate::sound::prop(defaults, "ActivateSound") {
            crate::sound::play_player(&snd, 1.0, 0.0);
        }
        self.0.push(Charged {
            class: class.to_string(),
            name: s("ItemName"),
            charge,
            max: charge,
            drain: 4.0 * skill,
            expire: s("ExpireMessage"),
            loop_snd: crate::sound::prop(defaults, "LoopSound"),
            deactivate_snd: crate::sound::prop(defaults, "DeActivateSound"),
        });
        true
    }
    pub fn has(&self, class: &str) -> bool {
        self.0.iter().any(|c| c.class == class)
    }
}

#[derive(Component)]
pub struct GogglesQuad(bool);
#[derive(Component)]
pub struct GogglesFrame;

pub fn spawn_goggles(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    // Two quads in front of the eye: one multiplies the view by pure green, one adds a little green so dark areas
    // come up (the original modulates by SolidGreen twice, which brightens green fourfold).
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let mul = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 1.0, 0.15),
        unlit: true,
        alpha_mode: AlphaMode::Multiply,
        depth_bias: 1.0e6,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let add = materials.add(StandardMaterial {
        base_color: Color::srgb(0.0, 0.07, 0.0),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        depth_bias: 1.0e6,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(quad.clone()),
        MeshMaterial3d(mul),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        GogglesQuad(false),
    ));
    commands.spawn((
        Mesh3d(quad),
        MeshMaterial3d(add),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        GogglesQuad(true),
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(25.0),
                top: Val::Percent(25.0),
                width: Val::Percent(50.0),
                height: Val::Percent(50.0),
                border: UiRect::all(Val::Px(2.0)),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            Visibility::Hidden,
            Pickable::IGNORE,
            GogglesFrame,
        ));
    // ("LightAmp Active" is drawn in the original font by augdisplay::aug_text_system.)
}

#[allow(clippy::too_many_arguments)]
pub fn charged_system(
    time: Res<Time>,
    mut active: ResMut<ActiveCharged>,
    mut ph: ResMut<PlayerHealth>,
    mut hud: ResMut<HudMessage>,
    cam: Single<(&Transform, &Projection), (With<FlyCam>, Without<SkyCam>, Without<GogglesQuad>)>,
    mut quads: Query<(&GogglesQuad, &mut Transform, &mut Visibility), Without<FlyCam>>,
    mut frame: Single<&mut Visibility, (With<GogglesFrame>, Without<GogglesQuad>)>,
) {
    let _prof = crate::prof::scope("charged::charged_system");
    let dt = time.delta_secs();
    // Developer aid: DXVIEW_FX="drunk,goggles" forces those screen effects on (for screenshots).
    if let Ok(fx) = std::env::var("DXVIEW_FX") {
        if fx.contains("drunk") {
            ph.drug_timer = ph.drug_timer.max(10.0);
        }
        if fx.contains("goggles") && !active.has("TechGoggles") {
            active.0.push(Charged {
                class: "TechGoggles".into(),
                name: "Tech Goggles".into(),
                charge: 1.0e9,
                max: 1.0e9,
                drain: 0.0,
                expire: String::new(),
                loop_snd: None,
                deactivate_snd: None,
            });
        }
    }
    let mut expired = Vec::new();
    for c in active.0.iter_mut() {
        c.charge -= c.drain * 10.0 * dt;
        if c.charge <= 0.0 {
            expired.push(c.expire.clone());
            if let Some(snd) = &c.deactivate_snd {
                crate::sound::play_player(snd, 1.0, 0.0);
            }
        } else if let Some(snd) = &c.loop_snd {
            crate::sound::hold(crate::sound::key_of(&c.class), snd, None, 0.7, 825.0, 1.0);
        }
    }
    active.0.retain(|c| c.charge > 0.0);
    if let Some(m) = expired.into_iter().next() {
        hud.text = m;
        hud.time_left = 3.0;
    }
    if ph.dead {
        active.0.clear();
    }
    ph.hazmat = active.has("HazMatSuit");
    ph.armor = active.has("BallisticArmor");
    ph.camo = active.has("AdaptiveArmor");
    ph.charged_text = active
        .0
        .iter()
        .map(|c| format!("{} {:.0}%", c.name, (c.charge / c.max * 100.0).max(0.0)))
        .collect::<Vec<_>>()
        .join("\n");

    // Tech goggles: the light-amplification box covers the middle half of the screen.
    // AugVision (Vision Enhancement) draws the same light-amplification box (DrawVisionAugmentation, level 0).
    let goggles =
        (active.has("TechGoggles") || crate::abilities::aug_active("AugVision")) && !ph.dead;
    **frame = if goggles {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let (ctf, proj) = *cam;
    let (vfov, aspect) = match proj {
        Projection::Perspective(p) => (p.fov, p.aspect_ratio),
        _ => (1.2, 1.7),
    };
    let d = 0.1f32;
    let h = 2.0 * d * (vfov * 0.5).tan();
    for (q, mut tf, mut vis) in &mut quads {
        *vis = if goggles {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if goggles {
            tf.translation = ctf.translation + *ctf.forward() * (d + if q.0 { 0.0 } else { 0.001 });
            tf.rotation = ctf.rotation;
            tf.scale = Vec3::new(h * aspect * 0.5, h * 0.5, 1.0);
        }
    }
}
