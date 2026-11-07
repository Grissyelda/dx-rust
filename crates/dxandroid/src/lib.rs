//! DX RUST for Android: the NativeActivity entry point. `bevy_main` makes `android_main`, which keeps the activity
//! for Bevy (bevy::android::ANDROID_APP) and runs the game (dxview::run) on it.
use bevy::prelude::bevy_main;

#[bevy_main]
fn main() {
    dxview::run();
}
