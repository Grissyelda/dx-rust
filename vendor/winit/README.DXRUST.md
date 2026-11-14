# winit 0.30.13, modified for DX RUST

This is winit 0.30.13 (Apache-2.0 / MIT, see LICENSE) with these changes, for Android only:

- `src/platform/android.rs`: a `gamepad` module with `set_hook` and `PadEvent`.
- `src/platform_impl/android/mod.rs`: with a hook set, motion events from joystick-class sources and key events
  from gamepad / D-pad sources go to the hook instead of becoming touches and keyboard input (upstream turns
  controller sticks into touches and drops their axes).

- `src/platform_impl/android/mod.rs`: `set_cursor_grab` and `set_cursor_position` succeed as no-ops (upstream
  errors), so Bevy keeps the app's wanted grab mode and cursor position (DX RUST tells play from menus by the
  grab mode, and a finger moves its cursor in menus).

- `src/platform/android.rs`: a `lifecycle` module with `set_hook`; `src/platform_impl/android/mod.rs` passes the
  name of every activity lifecycle event (Start, Resume, InitWindow, Pause, Stop, Destroy...) to it, for the game's
  start-up log.

Every other platform builds exactly as upstream. Used through `[patch.crates-io]` in the workspace Cargo.toml.
