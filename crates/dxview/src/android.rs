//! Android: what the phone does for the game. Startup (the app's own data folder, the screen kept on, the controller
//! hook), finding the Deus Ex folder (All Files Access, then a scan of the phone and its SD cards), the first-run
//! setup screen, and the activity's services through JNI: clipboard, the on-screen keyboard and restarting the app.
//!
//! The game itself is Classic only here (no mods, no Modern features): see `classic_only`.
use bevy::android::{android_activity::AndroidApp, ANDROID_APP};
use bevy::prelude::*;
use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use std::path::PathBuf;
use std::sync::Mutex;

fn app() -> Option<&'static AndroidApp> {
    ANDROID_APP.get()
}

/// Run `f` with a JNI environment on this thread and the activity. Java exceptions are printed and cleared.
fn with_env<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let r = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("android: JNI call failed: {e}");
            None
        }
    }
}

fn sdk() -> i32 {
    AndroidApp::sdk_version()
}

// ---------------------------------------------------------------------------------------------------------------
// Startup.

/// Before anything reads files: the data folder, the window flags and the controller hook. Then the game folder:
/// launcher.txt's GamePath when it still holds the game, else a search; with neither, the setup screen runs (it
/// restarts the app once the game is found, so this never returns without a game folder).
pub fn startup() -> String {
    if let Some(a) = app() {
        // Settings, saves and logs in the app's folder on shared storage (Android/data/com.dxrust.game/files), so
        // they can be copied off over USB; the internal folder when there's no shared storage.
        if let Some(dir) = a.external_data_path().or_else(|| a.internal_data_path()) {
            unreal_pkg::paths::set_data_dir(dir);
        }
        use bevy::android::android_activity::WindowManagerFlags as F;
        a.set_window_flags(F::KEEP_SCREEN_ON | F::FULLSCREEN, F::empty());
    }
    crate::platform::note(&format!("DX RUST {} starting (Android SDK {})", env!("CARGO_PKG_VERSION"), sdk()));
    // A crash from here on leaves crash.log in the app's folder (the release build aborts on a panic, so the hook
    // is the only trace).
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("dxview crashed: {info}\n{}\n", std::backtrace::Backtrace::force_capture());
        crate::platform::note(&format!("PANIC {info}"));
        let _ = std::fs::write(unreal_pkg::paths::data_dir().join("crash.log"), &msg);
    }));
    winit::platform::android::gamepad::set_hook(pad_event);
    if let Some(p) = saved_game_path().filter(|p| can_read(p)) {
        crate::platform::note(&format!("game folder {}", p.display()));
        return p.display().to_string();
    }
    if has_storage_access() {
        if let Some(p) = find_game() {
            save_game_path(&p);
            return p.display().to_string();
        }
    }
    crate::platform::note("no game folder yet: setup screen");
    run_setup_app();
    // The setup screen restarts the app; if that ever fails, quit rather than load nothing.
    std::process::exit(0);
}

fn launcher_txt() -> PathBuf {
    unreal_pkg::paths::data_dir().join("launcher.txt")
}

fn saved_game_path() -> Option<PathBuf> {
    std::fs::read_to_string(launcher_txt())
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("GamePath=").map(|v| PathBuf::from(v.trim())))
}

fn can_read(p: &std::path::Path) -> bool {
    unreal_pkg::gamedir::is_game_dir(p) && unreal_pkg::paths::find_ci(&p.join("System"), "DeusEx.u").is_some_and(|f| std::fs::File::open(f).is_ok())
}

fn save_game_path(p: &std::path::Path) {
    let path = launcher_txt();
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with("GamePath="))
        .map(str::to_owned)
        .collect();
    lines.insert(0, format!("GamePath={}", p.display()));
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

/// The Deus Ex folders on the phone and its SD cards, nearest first (5 folders down at most).
pub fn find_game() -> Option<PathBuf> {
    unreal_pkg::gamedir::scan(&unreal_pkg::gamedir::android_roots(), 5).into_iter().find(|p| can_read(p))
}

/// The app may read the whole shared storage: "All files access" from Android 11, the storage permission before.
pub fn has_storage_access() -> bool {
    with_env(|env, act| {
        if sdk() >= 30 {
            env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])?.z()
        } else {
            let perm = env.new_string("android.permission.READ_EXTERNAL_STORAGE")?;
            let r = env.call_method(act, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&perm)])?.i()?;
            Ok(r == 0)
        }
    })
    .unwrap_or(false)
}

/// Ask for it: Android 11+ opens the "All files access" page for this app, older ones the permission dialog.
pub fn request_storage_access() {
    with_env(|env, act| {
        if sdk() >= 30 {
            let pkg = env.call_method(act, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
            let scheme = env.new_string("package")?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "fromParts",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&scheme), JValue::Object(&pkg), JValue::Object(&JObject::null())],
                )?
                .l()?;
            let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?;
            env.call_method(act, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        } else {
            let cls = env.find_class("java/lang/String")?;
            let read = env.new_string("android.permission.READ_EXTERNAL_STORAGE")?;
            let arr = env.new_object_array(2, cls, &read)?;
            let write = env.new_string("android.permission.WRITE_EXTERNAL_STORAGE")?;
            env.set_object_array_element(&arr, 1, &write)?;
            env.call_method(act, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&arr), JValue::Int(1)])?;
        }
        Ok(())
    });
}

/// Start the app again from scratch (after the setup screen found the game): the launch intent in a fresh task,
/// then this process ends and Android starts a new one for it.
pub fn relaunch() -> ! {
    crate::platform::note("relaunching after setup");
    with_env(|env, act| {
        let pm = env.call_method(act, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?.l()?;
        let pkg = env.call_method(act, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
        let intent = env
            .call_method(&pm, "getLaunchIntentForPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&pkg)])?
            .l()?;
        // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK.
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000 | 0x0000_8000)])?;
        env.call_method(act, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::process::exit(0);
}

// ---------------------------------------------------------------------------------------------------------------
// Services.

pub fn clipboard_get() -> Option<String> {
    with_env(|env, act| {
        let name = env.new_string("clipboard")?;
        let cm = env.call_method(act, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()?;
        let clip = env.call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)])?.l()?;
        let text = env.call_method(&item, "getText", "()Ljava/lang/CharSequence;", &[])?.l()?;
        if text.is_null() {
            return Ok(None);
        }
        let s = env.call_method(&text, "toString", "()Ljava/lang/String;", &[])?.l()?;
        let s: String = env.get_string(&s.into())?.into();
        Ok(Some(s))
    })
    .flatten()
}

pub fn clipboard_set(text: &str) -> Result<(), String> {
    with_env(|env, act| {
        let name = env.new_string("clipboard")?;
        let cm = env.call_method(act, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()?;
        let label = env.new_string("DX RUST")?;
        let t = env.new_string(text)?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&t)],
            )?
            .l()?;
        env.call_method(&cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
        Ok(())
    })
    .ok_or_else(|| "the clipboard isn't available".to_string())
}

static KEYBOARD_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Show or hide the on-screen keyboard (typing a name, a login, a note). Typed keys arrive as keyboard input.
pub fn keyboard(show: bool) {
    if KEYBOARD_UP.swap(show, std::sync::atomic::Ordering::Relaxed) == show {
        return;
    }
    if let Some(a) = app() {
        if show {
            a.show_soft_input(true);
        } else {
            a.hide_soft_input(false);
        }
    }
}

/// A short buzz (taking damage, a hit on a touch button when the setting is on).
pub fn vibrate(ms: i64) {
    with_env(|env, act| {
        let name = env.new_string("vibrator")?;
        let v = env.call_method(act, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()?;
        if !v.is_null() {
            env.call_method(&v, "vibrate", "(J)V", &[JValue::Long(ms)])?;
        }
        Ok(())
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Controllers: winit's hook (vendor/winit) hands us Android's controller events; gamepad.rs reads the state.

use winit::platform::android::gamepad::PadEvent;

/// One controller's state, as Android last reported it.
#[derive(Default, Clone)]
pub struct AndroidPad {
    pub device: i32,
    pub name: String,
    /// Left stick, right stick (x right, y up), triggers 0..1.
    pub ls: Vec2,
    pub rs: Vec2,
    pub lt: f32,
    pub rt: f32,
    /// The D-pad as a hat (some pads only send this).
    pub hat: Vec2,
    /// Android key codes held.
    pub held: Vec<u32>,
    pub seen: bool,
}

static PAD: Mutex<Option<AndroidPad>> = Mutex::new(None);

fn pad_event(e: PadEvent) {
    let Ok(mut g) = PAD.lock() else { return };
    let dev = match e {
        PadEvent::Axes { device, .. } | PadEvent::Button { device, .. } => device,
    };
    let p = g.get_or_insert_with(AndroidPad::default);
    if p.device != dev || !p.seen {
        p.device = dev;
        p.seen = true;
        p.name.clear();
    }
    match e {
        PadEvent::Axes { x, y, z, rz, ltrigger, rtrigger, brake, gas, hat_x, hat_y, .. } => {
            p.ls = Vec2::new(x, -y);
            p.rs = Vec2::new(z, -rz);
            p.lt = ltrigger.max(brake).clamp(0.0, 1.0);
            p.rt = rtrigger.max(gas).clamp(0.0, 1.0);
            p.hat = Vec2::new(hat_x, -hat_y);
        }
        PadEvent::Button { keycode, down, .. } => {
            p.held.retain(|k| *k != keycode);
            if down {
                p.held.push(keycode);
            }
        }
    }
}

/// The controller in use, with its name looked up once (InputDevice.getDevice(id).getName()).
pub fn pad() -> Option<AndroidPad> {
    let mut p = PAD.lock().ok()?.clone()?;
    if p.name.is_empty() {
        let dev = p.device;
        let name = with_env(|env, _| {
            let d = env
                .call_static_method("android/view/InputDevice", "getDevice", "(I)Landroid/view/InputDevice;", &[JValue::Int(dev)])?
                .l()?;
            if d.is_null() {
                return Ok(String::from("Controller"));
            }
            let n = env.call_method(&d, "getName", "()Ljava/lang/String;", &[])?.l()?;
            Ok(env.get_string(&n.into())?.into())
        })
        .unwrap_or_else(|| "Controller".into());
        if let Ok(mut g) = PAD.lock() {
            if let Some(q) = g.as_mut() {
                q.name = name.clone();
            }
        }
        p.name = name;
    }
    Some(p)
}

// ---------------------------------------------------------------------------------------------------------------
// The first-run setup screen: a small Bevy app of its own (the game can't start without its files), with Bevy's
// built-in font. It asks for All Files Access, searches the phone and SD cards, and restarts the app into the game
// once it finds the folder.

#[derive(Resource, Default)]
struct Setup {
    access: bool,
    searching: bool,
    tried: bool,
    found: Option<PathBuf>,
    found_at: f32,
    last_check: f32,
    result: Option<Mutex<std::sync::mpsc::Receiver<Option<PathBuf>>>>,
}

#[derive(Component)]
struct SetupText;

#[derive(Component)]
struct SetupButton;

fn run_setup_app() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "DX RUST".into(), ..default() }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::BLACK))
        .init_resource::<Setup>()
        .add_systems(Startup, setup_ui)
        .add_systems(Update, setup_step)
        .run();
}

fn setup_ui(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(28.0),
            padding: UiRect::all(Val::Px(40.0)),
            ..default()
        })
        .with_children(|p| {
            p.spawn((Text::new("DX RUST"), TextFont { font_size: bevy::text::FontSize::Px(54.0), ..default() }, TextColor(Color::srgb(0.95, 0.75, 0.3))));
            p.spawn((
                Text::new(""),
                TextFont { font_size: bevy::text::FontSize::Px(24.0), ..default() },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                TextLayout::justify(Justify::Center),
                Node { max_width: Val::Px(1100.0), ..default() },
                SetupText,
            ));
            p.spawn((
                Button,
                SetupButton,
                Node { padding: UiRect::axes(Val::Px(36.0), Val::Px(18.0)), border: UiRect::all(Val::Px(2.0)), ..default() },
                BorderColor::all(Color::srgb(0.95, 0.75, 0.3)),
                BackgroundColor(Color::srgb(0.12, 0.1, 0.06)),
            ))
            .with_children(|b| {
                b.spawn((Text::new("Allow Access"), TextFont { font_size: bevy::text::FontSize::Px(30.0), ..default() }, TextColor(Color::WHITE)));
            });
        });
}

fn setup_step(
    mut st: ResMut<Setup>,
    time: Res<Time>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut text: Query<&mut Text, (With<SetupText>, Without<SetupButton>)>,
    button: Query<&Children, With<SetupButton>>,
    mut labels: Query<&mut Text, Without<SetupText>>,
) {
    let now = time.elapsed_secs();
    // Back from the settings page: check again twice a second.
    if now - st.last_check > 0.5 {
        st.last_check = now;
        st.access = has_storage_access();
    }
    // A search in the background (it can take a few seconds on a full phone).
    let got = st.result.as_ref().and_then(|rx| rx.lock().ok()?.try_recv().ok());
    {
        if let Some(r) = got {
            st.searching = false;
            st.tried = true;
            st.result = None;
            if let Some(p) = r {
                save_game_path(&p);
                st.found = Some(p);
                st.found_at = now;
            }
        }
    }
    let tapped = touches.any_just_released() || mouse.just_released(MouseButton::Left);
    let start_search = |st: &mut Setup| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(find_game());
        });
        st.result = Some(Mutex::new(rx));
        st.searching = true;
    };
    if st.access && !st.searching && st.found.is_none() && (!st.tried || tapped) {
        start_search(&mut *st);
    } else if !st.access && tapped {
        request_storage_access();
    }
    if let Some(p) = &st.found {
        if now - st.found_at > 1.5 {
            relaunch();
        }
        let _ = p;
    }
    let (msg, label) = if let Some(p) = &st.found {
        (format!("Found Deus Ex in\n{}\n\nStarting...", p.display()), "")
    } else if !st.access {
        (
            "DX RUST plays Deus Ex from your own copy of the game.\n\nCopy your whole Deus Ex folder (the one with System, Maps, \
             Textures and Sounds inside) anywhere on this phone or its SD card - Download is fine.\n\nThen tap below and turn on \
             \"Allow access to manage all files\" for DX RUST, so it can find the folder."
                .to_string(),
            "Allow Access",
        )
    } else if st.searching {
        ("Looking for Deus Ex on this phone and its SD cards...".to_string(), "Searching...")
    } else {
        (
            "Deus Ex wasn't found.\n\nCopy your whole Deus Ex folder (with System, Maps, Textures and Sounds inside) anywhere on \
             this phone or its SD card - for example Download/Deus Ex - then tap Search Again.\n\n(GOG and Steam copies both work: \
             copy the folder from your PC.)"
                .to_string(),
            "Search Again",
        )
    };
    if let Ok(mut t) = text.single_mut() {
        if t.0 != msg {
            t.0 = msg;
        }
    }
    if let Ok(kids) = button.single() {
        for k in kids.iter() {
            if let Ok(mut t) = labels.get_mut(k) {
                if t.0 != label {
                    t.0 = label.to_string();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Gyro aim (setting GyroAim, off by default): the gyroscope on a thread of its own with its own looper; the turn
// since the last frame is taken by touch::look_delta.

static GYRO: Mutex<Vec2> = Mutex::new(Vec2::ZERO);
static GYRO_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ROTATION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

/// Turn the sensor on or off (touch.rs, each frame: on while the setting is on and the game is in play).
pub fn gyro_enable(on: bool) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    GYRO_ON.store(on, std::sync::atomic::Ordering::Relaxed);
    if on {
        STARTED.call_once(|| {
            ROTATION.store(display_rotation(), std::sync::atomic::Ordering::Relaxed);
            let _ = std::thread::Builder::new().name("gyro".into()).spawn(gyro_thread);
        });
    } else if let Ok(mut g) = GYRO.lock() {
        *g = Vec2::ZERO;
    }
}

/// The view turn (yaw right, pitch up; radians) the phone made since the last call.
pub fn gyro_take() -> Vec2 {
    GYRO.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default()
}

/// The screen's rotation (Surface.ROTATION_0..270 = 0..3), so tilts map to the landscape the player holds.
fn display_rotation() -> i32 {
    with_env(|env, act| {
        let wm = env.call_method(act, "getWindowManager", "()Landroid/view/WindowManager;", &[])?.l()?;
        let d = env.call_method(&wm, "getDefaultDisplay", "()Landroid/view/Display;", &[])?.l()?;
        env.call_method(&d, "getRotation", "()I", &[])?.i()
    })
    .unwrap_or(1)
}

fn gyro_thread() {
    use ndk_sys as s;
    unsafe {
        let mgr = s::ASensorManager_getInstanceForPackage(c"com.dxrust.game".as_ptr());
        if mgr.is_null() {
            return;
        }
        let sensor = s::ASensorManager_getDefaultSensor(mgr, s::ASENSOR_TYPE_GYROSCOPE as i32);
        if sensor.is_null() {
            eprintln!("android: no gyroscope");
            return;
        }
        let looper = s::ALooper_prepare(s::ALOOPER_PREPARE_ALLOW_NON_CALLBACKS as i32);
        let queue = s::ASensorManager_createEventQueue(mgr, looper, 1, None, std::ptr::null_mut());
        if queue.is_null() {
            return;
        }
        let mut enabled = false;
        let mut last_ts: i64 = 0;
        let mut checked = std::time::Instant::now();
        let mut buf: [s::ASensorEvent; 16] = std::mem::zeroed();
        loop {
            let want = GYRO_ON.load(std::sync::atomic::Ordering::Relaxed);
            if want != enabled {
                if want {
                    s::ASensorEventQueue_enableSensor(queue, sensor);
                    s::ASensorEventQueue_setEventRate(queue, sensor, 10_000);
                    last_ts = 0;
                } else {
                    s::ASensorEventQueue_disableSensor(queue, sensor);
                }
                enabled = want;
            }
            if !enabled {
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            // The landscape side can flip while playing (sensorLandscape): look again every few seconds.
            if checked.elapsed().as_secs() >= 3 {
                checked = std::time::Instant::now();
                ROTATION.store(display_rotation(), std::sync::atomic::Ordering::Relaxed);
            }
            s::ALooper_pollOnce(100, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            let n = s::ASensorEventQueue_getEvents(queue, buf.as_mut_ptr(), buf.len());
            if n <= 0 {
                continue;
            }
            let rot = ROTATION.load(std::sync::atomic::Ordering::Relaxed);
            // The screen's up and right axes in the device's frame (x right, y up in portrait).
            let (up, right) = match rot {
                0 => (Vec3::Y, Vec3::X),
                1 => (Vec3::X, -Vec3::Y),
                2 => (-Vec3::Y, -Vec3::X),
                _ => (-Vec3::X, Vec3::Y),
            };
            let mut turn = Vec2::ZERO;
            for e in &buf[..n as usize] {
                let d = e.__bindgen_anon_1.__bindgen_anon_1.data;
                let w = Vec3::new(d[0], d[1], d[2]);
                if last_ts != 0 && e.timestamp > last_ts {
                    let dt = ((e.timestamp - last_ts) as f32 * 1e-9).min(0.05);
                    // Turning the phone left about the screen's up axis turns the view left; tipping its top
                    // toward you looks up.
                    turn += Vec2::new(-w.dot(up), -w.dot(right)) * dt;
                }
                last_ts = e.timestamp;
            }
            if let Ok(mut g) = GYRO.lock() {
                *g += turn;
            }
        }
    }
}
