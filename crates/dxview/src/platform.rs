//! What differs between the desktop and Android: the clipboard (arboard on the desktop, Android's ClipboardManager
//! through JNI there - android.rs).

/// The clipboard's text, if any.
pub fn clipboard_get() -> Option<String> {
    #[cfg(not(target_os = "android"))]
    {
        arboard::Clipboard::new().and_then(|mut c| c.get_text()).ok()
    }
    #[cfg(target_os = "android")]
    {
        crate::android::clipboard_get()
    }
}

/// Put text on the clipboard.
pub fn clipboard_set(text: &str) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        let mut c = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        c.set_text(text.to_string()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    {
        crate::android::clipboard_set(text)
    }
}

/// Running on Android (phones and tablets): Classic only, touch controls, the game folder found on the device.
pub const fn android() -> bool {
    cfg!(target_os = "android")
}

static BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The app is in the background (Android: switched away from, the screen off): sound is muted, the game paused.
pub fn backgrounded() -> bool {
    BACKGROUND.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bevy's lifecycle messages: suspended -> background (and the game paused behind the menu); resumed -> back.
/// Android: a line in startup.log (in the app's folder, Android/data/com.dxrust.game/files): what the app and its
/// window went through, for reports like GitHub issue #62 (the game starting in the background). Desktop: nothing.
pub fn note(msg: &str) {
    if !android() {
        return;
    }
    use std::io::Write;
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // A few hundred lines a run is plenty (the file starts over each run).
    let n = LINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n > 400 {
        return;
    }
    let t = T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
    let path = unreal_pkg::paths::data_dir().join("startup.log");
    let f = if n == 0 { std::fs::File::create(&path) } else { std::fs::OpenOptions::new().append(true).create(true).open(&path) };
    if let Ok(mut f) = f {
        let _ = writeln!(f, "{t:8.2} {msg}");
    }
}

/// Android: the window's focus / visibility / size changes into startup.log.
pub fn window_log_system(
    mut focus: bevy::prelude::MessageReader<bevy::window::WindowFocused>,
    mut occl: bevy::prelude::MessageReader<bevy::window::WindowOccluded>,
    mut size: bevy::prelude::MessageReader<bevy::window::WindowResized>,
    mut frames: bevy::prelude::Local<u32>,
) {
    for e in focus.read() {
        note(&format!("window focused {}", e.focused));
    }
    for e in occl.read() {
        note(&format!("window occluded {}", e.occluded));
    }
    for e in size.read() {
        note(&format!("window resized {}x{}", e.width, e.height));
    }
    *frames += 1;
    if matches!(*frames, 1 | 60 | 600) {
        note(&format!("frame {}", *frames));
    }
}

pub fn lifecycle_system(
    mut ev: bevy::prelude::MessageReader<bevy::window::AppLifecycle>,
    mut vtime: bevy::prelude::ResMut<bevy::prelude::Time<bevy::prelude::Virtual>>,
    sound: Option<bevy::prelude::Res<crate::sound::SoundState>>,
) {
    for e in ev.read() {
        note(&format!("lifecycle {e:?}"));
        match e {
            bevy::window::AppLifecycle::Suspended | bevy::window::AppLifecycle::WillSuspend => {
                if let Some(st) = &sound {
                    crate::sound::mute_now(st);
                }
                if !BACKGROUND.swap(true, std::sync::atomic::Ordering::Relaxed) && !vtime.is_paused() {
                    vtime.pause();
                    PAUSED_BY_US.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            bevy::window::AppLifecycle::WillResume | bevy::window::AppLifecycle::Running => {
                if BACKGROUND.swap(false, std::sync::atomic::Ordering::Relaxed) && PAUSED_BY_US.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    vtime.unpause();
                }
            }
            _ => {}
        }
    }
}

static PAUSED_BY_US: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
