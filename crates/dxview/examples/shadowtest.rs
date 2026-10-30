//! Headless check of the shadow pair trick (shadowlight.rs): a lit floor, an unlit cube on the shadow layer,
//! a shadowed light on layer 8 + a negative unshadowed twin. MODE=plain|pair|pos.
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, shoot)
        .run();
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>) {
    let mode = std::env::var("MODE").unwrap_or("plain".into());
    let layers = mode != "plain";
    let mut plane: Mesh = Plane3d::default().mesh().size(20.0, 20.0).into();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = plane.attribute(Mesh::ATTRIBUTE_UV_0).cloned() {
        plane.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
    }
    let lm = images.add(Image::new_fill(
        bevy::render::render_resource::Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        &[60, 60, 60, 255],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ));
    let mut f = commands.spawn((Mesh3d(meshes.add(plane)), MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), lightmap_exposure: 1.0, double_sided: true, cull_mode: None, perceptual_roughness: 1.0, reflectance: 0.05, ..default() })), bevy::light::NotShadowCaster));
    if std::env::var_os("LM").is_some() {
        f.insert(bevy::pbr::Lightmap { image: lm, uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false });
    }
    let mut cube = commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.8, 0.2, 0.2), unlit: true, ..default() })),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
    if layers {
        cube.insert(RenderLayers::from_layers(&[0, 8]));
    }
    let i = 200000.0;
    let mut l = commands.spawn((PointLight { intensity: i, range: 20.0, shadow_maps_enabled: true, ..default() }, Transform::from_xyz(2.0, 3.0, 1.0)));
    if layers {
        l.insert(RenderLayers::layer(8));
    }
    if mode == "pair" {
        commands.spawn((PointLight { intensity: -i, range: 20.0, shadow_maps_enabled: false, ..default() }, Transform::from_xyz(2.0, 3.0, 1.0), RenderLayers::layer(8)));
    }
    commands.spawn((AmbientLight { brightness: 300.0, ..default() },));
    let mut c = commands.spawn((Camera3d::default(), Transform::from_xyz(-4.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y)));
    if layers {
        c.insert(RenderLayers::from_layers(&[0, 8]));
    }
    if std::env::var_os("CAM2").is_some() {
        commands.spawn((Camera3d::default(), Camera { order: -1, ..default() }, Transform::from_xyz(-4.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y), RenderLayers::layer(1), bevy::light::cluster::ClusterConfig::Single));
        commands.spawn((Camera3d::default(), Camera { order: 1, clear_color: ClearColorConfig::None, ..default() }, Transform::from_xyz(-4.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y), RenderLayers::layer(2)));
    }
}

fn shoot(mut commands: Commands, mut n: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *n += 1;
    if *n == 30 {
        let p = std::env::var("OUT").unwrap_or("/tmp/shadowtest.png".into());
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(p));
    }
    if *n == 40 {
        exit.write(AppExit::Success);
    }
}
