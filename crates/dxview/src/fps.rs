//! FPS Counter (Settings > Enhancements > Performance, off by default): frames a second and the average frame time
//! in the top-right corner, over each half second of real time. Ours.

use bevy::prelude::*;

#[derive(Component)]
pub struct FpsText;

/// Spawned once at start-up; it survives map changes.
pub fn spawn_fps_text(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.95, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(10.0),
            top: Val::Px(6.0),
            ..default()
        },
        GlobalZIndex(i32::MAX),
        Visibility::Hidden,
        FpsText,
        crate::loader::Persist,
    ));
}

/// Shows or hides the counter with the setting, and writes the text only when half a second has passed.
pub fn fps_system(
    time: Res<Time<Real>>,
    counter: Single<(&mut Text, &mut Visibility), With<FpsText>>,
    mut window: Local<(f32, u32)>,
) {
    let (mut text, mut vis) = counter.into_inner();
    let on = crate::perf::fps_counter();
    let want = if on { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if !on {
        *window = (0.0, 0);
        return;
    }
    window.0 += time.delta_secs();
    window.1 += 1;
    if window.0 >= 0.5 {
        let (secs, frames) = *window;
        text.0 = format!("{:.0} fps  {:.1} ms", frames as f32 / secs, 1000.0 * secs / frames as f32);
        *window = (0.0, 0);
    }
}
