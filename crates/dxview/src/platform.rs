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
pub fn lifecycle_system(
    mut ev: bevy::prelude::MessageReader<bevy::window::AppLifecycle>,
    mut vtime: bevy::prelude::ResMut<bevy::prelude::Time<bevy::prelude::Virtual>>,
    sound: Option<bevy::prelude::Res<crate::sound::SoundState>>,
) {
    for e in ev.read() {
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
