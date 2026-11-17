//! Controllers through SDL3 (RecompLauncher's recommendation, and the most controllers known by name): SDL3's
//! gamepad layer, loaded at run time from SDL3.dll (shipped beside dxview.exe in the Windows release) or the
//! system's libSDL3 (Linux, the Steam Deck, macOS with SDL3 installed). When there is no SDL3, or the setting
//! PadBackend says so, gilrs (gamepad.rs) reads the pads as before. A DualSense on USB / Bluetooth stays with our
//! own HID code (dualsense.rs: adaptive triggers, the speaker, HD haptics), so SDL is told to leave it alone.
//!
//! Only the gamepad subsystem is started: no SDL window, no SDL events beyond the pads. Background input stays off
//! (SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS = 0), as RecompLauncher asks, so its overlay keeps the pad when we're
//! not in front.

use std::ffi::{c_char, c_int, c_void, CStr};

type Bool = bool;

#[allow(non_snake_case)]
struct Api {
    _lib: libloading::Library,
    SDL_SetHint: unsafe extern "C" fn(*const c_char, *const c_char) -> Bool,
    SDL_Init: unsafe extern "C" fn(u32) -> Bool,
    SDL_GetError: unsafe extern "C" fn() -> *const c_char,
    SDL_UpdateGamepads: unsafe extern "C" fn(),
    SDL_GetGamepads: unsafe extern "C" fn(*mut c_int) -> *mut u32,
    SDL_free: unsafe extern "C" fn(*mut c_void),
    SDL_OpenGamepad: unsafe extern "C" fn(u32) -> *mut c_void,
    SDL_CloseGamepad: unsafe extern "C" fn(*mut c_void),
    SDL_GamepadConnected: unsafe extern "C" fn(*mut c_void) -> Bool,
    SDL_GetGamepadName: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    SDL_GetGamepadType: unsafe extern "C" fn(*mut c_void) -> c_int,
    SDL_GetGamepadAxis: unsafe extern "C" fn(*mut c_void, c_int) -> i16,
    SDL_GetGamepadButton: unsafe extern "C" fn(*mut c_void, c_int) -> Bool,
    SDL_RumbleGamepad: unsafe extern "C" fn(*mut c_void, u16, u16, u32) -> Bool,
}

const SDL_INIT_GAMEPAD: u32 = 0x0000_2000;

// SDL_GamepadButton / SDL_GamepadAxis / SDL_GamepadType (SDL3 headers).
const B_SOUTH: c_int = 0;
const B_EAST: c_int = 1;
const B_WEST: c_int = 2;
const B_NORTH: c_int = 3;
const B_BACK: c_int = 4;
const B_START: c_int = 6;
const B_LSTICK: c_int = 7;
const B_RSTICK: c_int = 8;
const B_LSHOULDER: c_int = 9;
const B_RSHOULDER: c_int = 10;
const B_UP: c_int = 11;
const B_DOWN: c_int = 12;
const B_LEFT: c_int = 13;
const B_RIGHT: c_int = 14;
const B_TOUCHPAD: c_int = 20;
const A_LX: c_int = 0;
const A_LY: c_int = 1;
const A_RX: c_int = 2;
const A_RY: c_int = 3;
const A_LT: c_int = 4;
const A_RT: c_int = 5;
const T_PS3: c_int = 4;
const T_PS4: c_int = 5;
const T_PS5: c_int = 6;

struct State {
    api: Api,
    pad: *mut c_void,
    name: String,
    kind: crate::gamepad::PadKind,
    rumble: (u16, u16),
}

// The SDL calls all happen on the thread holding the lock (Bevy's main-schedule systems take turns for it).
unsafe impl Send for State {}

static STATE: std::sync::Mutex<Option<State>> = std::sync::Mutex::new(None);
static TRIED: std::sync::Once = std::sync::Once::new();

fn lib_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["SDL3.dll"]
    } else if cfg!(target_os = "macos") {
        &["libSDL3.0.dylib", "libSDL3.dylib", "SDL3.framework/SDL3", "@rpath/SDL3.framework/SDL3"]
    } else {
        &["libSDL3.so.0", "libSDL3.so"]
    }
}

fn load() -> Option<Api> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()));
    for name in lib_names() {
        // Beside the exe first (the release ships it there), then the system's search path.
        let mut tries = Vec::new();
        if let Some(d) = &exe_dir {
            tries.push(d.join(name).into_os_string());
        }
        tries.push(std::ffi::OsString::from(name));
        for t in tries {
            let Ok(lib) = (unsafe { libloading::Library::new(&t) }) else { continue };
            macro_rules! sym {
                ($n:ident) => {
                    *unsafe { lib.get(concat!(stringify!($n), "\0").as_bytes()) }.ok()?
                };
            }
            let api = Api {
                SDL_SetHint: sym!(SDL_SetHint),
                SDL_Init: sym!(SDL_Init),
                SDL_GetError: sym!(SDL_GetError),
                SDL_UpdateGamepads: sym!(SDL_UpdateGamepads),
                SDL_GetGamepads: sym!(SDL_GetGamepads),
                SDL_free: sym!(SDL_free),
                SDL_OpenGamepad: sym!(SDL_OpenGamepad),
                SDL_CloseGamepad: sym!(SDL_CloseGamepad),
                SDL_GamepadConnected: sym!(SDL_GamepadConnected),
                SDL_GetGamepadName: sym!(SDL_GetGamepadName),
                SDL_GetGamepadType: sym!(SDL_GetGamepadType),
                SDL_GetGamepadAxis: sym!(SDL_GetGamepadAxis),
                SDL_GetGamepadButton: sym!(SDL_GetGamepadButton),
                SDL_RumbleGamepad: sym!(SDL_RumbleGamepad),
                _lib: lib,
            };
            println!("PAD SDL3 loaded from {}", t.to_string_lossy());
            return Some(api);
        }
    }
    None
}

/// Start SDL3's gamepad layer once (when allowed and found). True when SDL3 reads the pads.
pub fn init() -> bool {
    TRIED.call_once(|| {
        // PadBackend: 0 = SDL3 when it's there (default), 1 = gilrs only. DXVIEW_NO_SDL=1 for tests.
        if setting!("PadBackend") > 0.5 || std::env::var_os("DXVIEW_NO_SDL").is_some() || cfg!(target_os = "android") {
            return;
        }
        let Some(api) = load() else {
            println!("PAD SDL3 not found: gilrs reads the controllers");
            return;
        };
        unsafe {
            let hint = |k: &str, v: &str| {
                let k = std::ffi::CString::new(k).unwrap();
                let v = std::ffi::CString::new(v).unwrap();
                (api.SDL_SetHint)(k.as_ptr(), v.as_ptr());
            };
            hint("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "0");
            // Our own HID code drives the DualSense (triggers, speaker, haptics): SDL sees it as a plain pad at
            // most, never opens it over HID.
            hint("SDL_JOYSTICK_HIDAPI_PS5", "0");
            if !(api.SDL_Init)(SDL_INIT_GAMEPAD) {
                let e = CStr::from_ptr((api.SDL_GetError)()).to_string_lossy();
                println!("PAD SDL3 didn't start ({e}): gilrs reads the controllers");
                return;
            }
        }
        if let Ok(mut s) = STATE.lock() {
            *s = Some(State { api, pad: std::ptr::null_mut(), name: String::new(), kind: Default::default(), rumble: (0, 0) });
        }
    });
    active()
}

/// SDL3 is reading the controllers (gilrs then stays out).
pub fn active() -> bool {
    STATE.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// One pad's state for this frame (None: no pad on SDL).
pub struct Read {
    pub name: String,
    pub kind: crate::gamepad::PadKind,
    pub ls: (f32, f32),
    pub rs: (f32, f32),
    pub lt: f32,
    pub rt: f32,
    pub held: Vec<crate::gamepad::PadBtn>,
}

/// Update SDL's pads and read the first connected one.
pub fn read() -> Option<Read> {
    use crate::gamepad::{PadBtn, PadKind};
    let mut g = STATE.lock().ok()?;
    let s = g.as_mut()?;
    unsafe {
        (s.api.SDL_UpdateGamepads)();
        if !s.pad.is_null() && !(s.api.SDL_GamepadConnected)(s.pad) {
            println!("PAD {} disconnected", s.name);
            (s.api.SDL_CloseGamepad)(s.pad);
            s.pad = std::ptr::null_mut();
        }
        if s.pad.is_null() {
            let mut n: c_int = 0;
            let ids = (s.api.SDL_GetGamepads)(&mut n);
            if !ids.is_null() {
                if n > 0 {
                    let pad = (s.api.SDL_OpenGamepad)(*ids);
                    if !pad.is_null() {
                        s.pad = pad;
                        let nm = (s.api.SDL_GetGamepadName)(pad);
                        s.name = if nm.is_null() { "Controller".into() } else { CStr::from_ptr(nm).to_string_lossy().into_owned() };
                        s.kind = match (s.api.SDL_GetGamepadType)(pad) {
                            T_PS3 | T_PS4 | T_PS5 => PadKind::PlayStation,
                            _ => PadKind::Xbox,
                        };
                        s.rumble = (0, 0);
                        println!("PAD {} connected (SDL3)", s.name);
                    }
                }
                (s.api.SDL_free)(ids as *mut c_void);
            }
        }
        if s.pad.is_null() {
            return None;
        }
        let p = s.pad;
        let ax = |a: c_int| (s.api.SDL_GetGamepadAxis)(p, a) as f32 / 32767.0;
        let mut held = Vec::new();
        for (b, pb) in [
            (B_SOUTH, PadBtn::South),
            (B_EAST, PadBtn::East),
            (B_WEST, PadBtn::West),
            (B_NORTH, PadBtn::North),
            (B_LSHOULDER, PadBtn::LB),
            (B_RSHOULDER, PadBtn::RB),
            (B_BACK, PadBtn::Select),
            (B_START, PadBtn::Start),
            (B_LSTICK, PadBtn::L3),
            (B_RSTICK, PadBtn::R3),
            (B_UP, PadBtn::Up),
            (B_DOWN, PadBtn::Down),
            (B_LEFT, PadBtn::Left),
            (B_RIGHT, PadBtn::Right),
            (B_TOUCHPAD, PadBtn::Touch),
        ] {
            if (s.api.SDL_GetGamepadButton)(p, b) {
                held.push(pb);
            }
        }
        // SDL's Y axes grow downward; ours point up.
        Some(Read {
            name: s.name.clone(),
            kind: s.kind,
            ls: (ax(A_LX), -ax(A_LY)),
            rs: (ax(A_RX), -ax(A_RY)),
            lt: ax(A_LT).max(0.0),
            rt: ax(A_RT).max(0.0),
            held,
        })
    }
}

/// Rumble the SDL pad (both motors 0..1); sent only when it changes noticeably.
pub fn rumble(low: f32, high: f32) {
    let Ok(mut g) = STATE.lock() else { return };
    let Some(s) = g.as_mut() else { return };
    if s.pad.is_null() {
        return;
    }
    let want = ((low.clamp(0.0, 1.0) * 65535.0) as u16, (high.clamp(0.0, 1.0) * 65535.0) as u16);
    if want.0.abs_diff(s.rumble.0) < 2000 && want.1.abs_diff(s.rumble.1) < 2000 && (want != (0, 0) || s.rumble == (0, 0)) {
        return;
    }
    s.rumble = want;
    unsafe {
        (s.api.SDL_RumbleGamepad)(s.pad, want.0, want.1, if want == (0, 0) { 0 } else { 2000 });
    }
}
