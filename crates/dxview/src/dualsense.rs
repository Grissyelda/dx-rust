//! DualSense integration (ROADMAP Phase 0.3), straight over HID (the `hidapi` crate) beside gilrs:
//! * input: a DualSense on HID supplies the whole pad (sticks, triggers, buttons) - Windows' generic mapping of the
//!   pad is unreliable and never exposes the touchpad - plus the touchpad (two fingers, click) and the mic button;
//! * output: the light bar shows health on the HUD's own green-yellow-red scale (Window.GetColorScaled), pulsing
//!   when low; the five player LEDs are a bioelectric energy meter; the mic LED pulses while an infolink plays;
//!   adaptive triggers by weapon class (see `trigger_for`); rumble motors (classic rumble, `gamepad::rumble`).
//! * HD haptics and the pad speaker are USB audio (`haptics.rs`); here only the report bits that route the audio.
//!
//! Report layouts: USB input 0x01 (64 bytes), Bluetooth input 0x31 (78; the pad only sends it after feature report
//! 0x05 is read), USB output 0x02, Bluetooth output 0x31 with a CRC-32 over 0xA2 + the report. The 47-byte common
//! output block: 0 flags0 (rumble emulation, rumble-not-haptics, right / left trigger, headphone / speaker / mic
//! volume, audio control), 1 flags1 (mic LED, audio mute, light bar, release LEDs, player LEDs, ...), 2 / 3 right /
//! left motor, 4..6 volumes, 7 audio control (output path in bits 4-5), 8 mic LED mode, 10..20 right trigger,
//! 21..31 left trigger, 37 speaker pre-gain, 38 flags2, 41 light-bar setup, 42 LED brightness, 43 player LEDs,
//! 44..46 RGB. Trigger effects are the pad's own modes: 0x05 off, 0x21 feedback (zones from a position, force),
//! 0x25 weapon (a click between two zones), 0x26 vibration (zones, amplitude, frequency).

use crate::gamepad::PadBtn;
use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const VID: u16 = 0x054c;
const PIDS: [u16; 2] = [0x0ce6, 0x0df2]; // DualSense, DualSense Edge

/// What the pad last reported.
#[derive(Default, Clone)]
pub struct DsIn {
    pub connected: bool,
    pub usb: bool,
    /// Sticks (x right, y up, -1..1), triggers 0..1.
    pub ls: Vec2,
    pub rs: Vec2,
    pub lt: f32,
    pub rt: f32,
    pub held: Vec<PadBtn>,
    pub mic_btn: bool,
    pub battery: u8,
}

/// What the game wants the pad to show.
#[derive(Clone, PartialEq)]
pub struct DsOut {
    /// Right (small, fast) and left (big, slow) motors.
    pub motors: (u8, u8),
    /// Rumble through the motors (false while HD haptics play through the audio channels).
    pub motor_mode: bool,
    pub light: [u8; 3],
    pub player_leds: u8,
    /// 0 off, 1 on, 2 pulse.
    pub mic_led: u8,
    pub right: [u8; 11],
    pub left: [u8; 11],
    /// Route the audio's right channel to the pad speaker (USB audio).
    pub speaker: bool,
    pub speaker_volume: u8,
}

impl Default for DsOut {
    fn default() -> Self {
        DsOut {
            motors: (0, 0),
            motor_mode: true,
            light: [0, 0, 64],
            player_leds: 0x04,
            mic_led: 0,
            right: OFF,
            left: OFF,
            speaker: false,
            speaker_volume: 0x64,
        }
    }
}

const OFF: [u8; 11] = [0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Touchpad movement gathered between frames (pad units: 1920 x 1080 over the pad).
#[derive(Default, Clone)]
pub struct Touch {
    /// One finger dragging.
    pub drag: Vec2,
    /// Two fingers moving vertically (scroll).
    pub scroll: f32,
    /// A quick one-finger flick: -1 left, 1 right.
    pub swipe: i8,
    pub fingers: u8,
}

static IN: Mutex<Option<DsIn>> = Mutex::new(None);
static OUT: Mutex<Option<DsOut>> = Mutex::new(None);
static TOUCH: Mutex<Option<Touch>> = Mutex::new(None);
static TOUCH_CLICK: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// The touchpad is pressed in.
pub fn touch_click() -> bool {
    TOUCH_CLICK.load(Ordering::Relaxed)
}

/// The DualSense on HID, if one is connected.
pub fn input() -> Option<DsIn> {
    IN.lock().ok().and_then(|g| g.clone()).filter(|i| i.connected)
}

pub fn connected_usb() -> bool {
    input().is_some_and(|i| i.usb)
}

/// The touchpad movement since the last call.
pub fn take_touch() -> Touch {
    let mut g = TOUCH.lock().unwrap();
    let t = g.get_or_insert_with(Touch::default);
    let out = t.clone();
    t.drag = Vec2::ZERO;
    t.scroll = 0.0;
    t.swipe = 0;
    out
}

fn set_out(f: impl FnOnce(&mut DsOut)) {
    let mut g = OUT.lock().unwrap();
    f(g.get_or_insert_with(DsOut::default));
}

/// Startup: the HID thread (it waits for a pad and reconnects by itself).
pub fn setup() {
    if STARTED.swap(true, Ordering::Relaxed) || std::env::var("DXVIEW_NO_HID").is_ok() {
        return;
    }
    let _ = std::thread::Builder::new().name("dualsense".into()).spawn(hid_thread);
}

// ------------------------------------------------------------------------------------------------ HID thread

fn hid_thread() {
    let mut api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            warn!("DualSense: no HID access ({e})");
            return;
        }
    };
    loop {
        let _ = api.refresh_devices();
        let found = api
            .device_list()
            .find(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id()))
            .map(|d| (d.path().to_owned(), d.bus_type()));
        if let Some((path, bus)) = found {
            if let Ok(dev) = api.open_path(&path) {
                let usb = !matches!(bus, hidapi::BusType::Bluetooth);
                println!("PAD DualSense on HID ({})", if usb { "USB" } else { "Bluetooth" });
                run_pad(&dev, usb);
                println!("PAD DualSense disconnected");
                if let Ok(mut g) = IN.lock() {
                    *g = None;
                }
                TOUCH_CLICK.store(false, Ordering::Relaxed);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[derive(Default)]
struct Finger {
    down: bool,
    id: u8,
    pos: Vec2,
    start: Vec2,
    start_t: f64,
}

fn run_pad(dev: &hidapi::HidDevice, usb: bool) {
    if !usb {
        // Reading the calibration feature report switches the pad to its full 0x31 reports.
        let mut f = [0u8; 41];
        f[0] = 0x05;
        let _ = dev.get_feature_report(&mut f);
    }
    let clock = std::time::Instant::now();
    let mut buf = [0u8; 128];
    let mut sent: Option<DsOut> = None;
    let mut last_send = -10.0f64;
    let mut seq: u8 = 0;
    let mut first = true;
    let mut usb_len = 63usize;
    let mut fingers: [Finger; 2] = Default::default();
    loop {
        let n = match dev.read_timeout(&mut buf, 4) {
            Ok(n) => n,
            Err(_) => return,
        };
        let now = clock.elapsed().as_secs_f64();
        if n > 0 {
            let data = match buf[0] {
                0x01 if usb && n >= 64 => Some(&buf[1..n]),
                0x31 if !usb && n >= 64 => Some(&buf[2..n]),
                _ => None,
            };
            if let Some(d) = data.filter(|d| d.len() >= 53) {
                parse(d, usb, now, &mut fingers);
            }
        }
        // Output: when it changes, and twice a second anyway (a pad that missed one).
        let want = OUT.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
        if sent.as_ref() != Some(&want) || now - last_send > 0.5 {
            let common = common_block(&want, first);
            let ok = if usb {
                let mut r = vec![0u8; usb_len];
                r[0] = 0x02;
                r[1..48].copy_from_slice(&common);
                let mut ok = dev.write(&r).is_ok();
                if !ok && usb_len == 63 {
                    usb_len = 48;
                    r.truncate(48);
                    ok = dev.write(&r).is_ok();
                }
                ok
            } else {
                let mut r = [0u8; 78];
                r[0] = 0x31;
                r[1] = seq << 4;
                seq = (seq + 1) & 15;
                r[2] = 0x10;
                r[3..50].copy_from_slice(&common);
                let mut c = crc32(0xffff_ffff, &[0xa2]);
                c = crc32(c, &r[..74]) ^ 0xffff_ffff;
                r[74..78].copy_from_slice(&c.to_le_bytes());
                dev.write(&r).is_ok()
            };
            if !ok {
                return;
            }
            first = false;
            sent = Some(want);
            last_send = now;
        }
    }
}

fn parse(d: &[u8], usb: bool, now: f64, fingers: &mut [Finger; 2]) {
    let axis = |v: u8| (v as f32 - 128.0) / 127.0;
    let mut i = DsIn {
        connected: true,
        usb,
        ls: Vec2::new(axis(d[0]), -axis(d[1])).clamp(Vec2::splat(-1.0), Vec2::splat(1.0)),
        rs: Vec2::new(axis(d[2]), -axis(d[3])).clamp(Vec2::splat(-1.0), Vec2::splat(1.0)),
        lt: d[4] as f32 / 255.0,
        rt: d[5] as f32 / 255.0,
        held: Vec::new(),
        mic_btn: d[9] & 0x04 != 0,
        battery: d[52] & 0x0f,
    };
    let (b0, b1, b2) = (d[7], d[8], d[9]);
    // The D-pad is a hat: 0 up, clockwise, 8 = centred.
    let hat = b0 & 0x0f;
    if matches!(hat, 7 | 0 | 1) {
        i.held.push(PadBtn::Up);
    }
    if matches!(hat, 1..=3) {
        i.held.push(PadBtn::Right);
    }
    if matches!(hat, 3..=5) {
        i.held.push(PadBtn::Down);
    }
    if matches!(hat, 5..=7) {
        i.held.push(PadBtn::Left);
    }
    for (bit, b) in [(0x10, PadBtn::West), (0x20, PadBtn::South), (0x40, PadBtn::East), (0x80, PadBtn::North)] {
        if b0 & bit != 0 {
            i.held.push(b);
        }
    }
    for (bit, b) in [
        (0x01, PadBtn::LB),
        (0x02, PadBtn::RB),
        (0x10, PadBtn::Select),
        (0x20, PadBtn::Start),
        (0x40, PadBtn::L3),
        (0x80, PadBtn::R3),
    ] {
        if b1 & bit != 0 {
            i.held.push(b);
        }
    }
    TOUCH_CLICK.store(b2 & 0x02 != 0, Ordering::Relaxed);

    // Touch points: contact byte (bit 7 set = no finger, low bits an id), then 12-bit x and y.
    let mut t = TOUCH.lock().unwrap();
    let t = t.get_or_insert_with(Touch::default);
    let mut now_down = 0u8;
    let mut moves = [Vec2::ZERO; 2];
    for (k, f) in fingers.iter_mut().enumerate() {
        let p = &d[32 + k * 4..36 + k * 4];
        let down = p[0] & 0x80 == 0;
        let id = p[0] & 0x7f;
        let pos = Vec2::new((p[1] as u16 | ((p[2] as u16 & 0x0f) << 8)) as f32, ((p[2] >> 4) as u16 | ((p[3] as u16) << 4)) as f32);
        if down {
            now_down += 1;
            if f.down && f.id == id {
                moves[k] = pos - f.pos;
            } else {
                f.start = pos;
                f.start_t = now;
            }
        } else if f.down && k == 0 {
            // A quick, mostly sideways flick of one finger.
            let dv = f.pos - f.start;
            if now - f.start_t < 0.3 && dv.x.abs() > 450.0 && dv.x.abs() > 2.0 * dv.y.abs() {
                t.swipe = if dv.x > 0.0 { 1 } else { -1 };
            }
        }
        f.down = down;
        f.id = id;
        f.pos = pos;
    }
    match now_down {
        1 => t.drag += moves[0] + moves[1],
        2 => t.scroll += (moves[0].y + moves[1].y) * 0.5,
        _ => {}
    }
    t.fingers = now_down;
    if let Ok(mut g) = IN.lock() {
        *g = Some(i);
    }
}

fn common_block(o: &DsOut, first: bool) -> [u8; 47] {
    let mut c = [0u8; 47];
    // flags0: triggers always (an "off" effect is still an effect); motors only in rumble mode; the speaker's
    // volume and output path when routing.
    c[0] = 0x04 | 0x08;
    if o.motor_mode {
        c[0] |= 0x01 | 0x02;
        c[2] = o.motors.0;
        c[3] = o.motors.1;
    }
    c[0] |= 0x20 | 0x80;
    c[5] = if o.speaker { o.speaker_volume } else { 0 };
    // Output path: 0 = headphones L/R, 3 = the speaker from the right channel.
    c[7] = if o.speaker { 3 << 4 } else { 0 };
    // flags1: mic LED, light bar, player LEDs, speaker pre-gain (audio control 2).
    c[1] = 0x01 | 0x04 | 0x10 | 0x80;
    c[8] = o.mic_led;
    c[10..21].copy_from_slice(&o.right);
    c[21..32].copy_from_slice(&o.left);
    c[37] = if o.speaker { 4 } else { 0 };
    if first {
        // Release the pad's own start-up light animation so our colour shows.
        c[38] = 0x02;
        c[41] = 0x02;
    }
    c[42] = 0;
    c[43] = o.player_leds & 0x1f;
    c[44..47].copy_from_slice(&o.light);
    c
}

fn crc32(mut c: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    c
}

// ------------------------------------------------------------------------------------------------ trigger effects

/// Resistance from `pos` (0..9) to the end, `strength` 1..8.
fn feedback(pos: u8, strength: u8) -> [u8; 11] {
    let force = (strength.clamp(1, 8) - 1) as u32;
    let (mut zones, mut forces) = (0u16, 0u32);
    for z in pos.min(9)..10 {
        zones |= 1 << z;
        forces |= force << (3 * z as u32);
    }
    let mut e = [0u8; 11];
    e[0] = 0x21;
    e[1..3].copy_from_slice(&zones.to_le_bytes());
    e[3..7].copy_from_slice(&forces.to_le_bytes());
    e
}

/// A click: resistance from `start` (2..7) that gives way at `end` (start+1..8), `strength` 1..8.
fn weapon(start: u8, end: u8, strength: u8) -> [u8; 11] {
    let start = start.clamp(2, 7);
    let end = end.clamp(start + 1, 8);
    let zones: u16 = (1 << start) | (1 << end);
    let mut e = [0u8; 11];
    e[0] = 0x25;
    e[1..3].copy_from_slice(&zones.to_le_bytes());
    e[3] = strength.clamp(1, 8) - 1;
    e
}

/// Vibration from `pos` (0..9), amplitude 1..8, `freq` Hz.
fn vibration(pos: u8, amp: u8, freq: u8) -> [u8; 11] {
    let a = (amp.clamp(1, 8) - 1) as u32;
    let (mut zones, mut amps) = (0u16, 0u32);
    for z in pos.min(9)..10 {
        zones |= 1 << z;
        amps |= a << (3 * z as u32);
    }
    let mut e = [0u8; 11];
    e[0] = 0x26;
    e[1..3].copy_from_slice(&zones.to_le_bytes());
    e[3..7].copy_from_slice(&amps.to_le_bytes());
    e[9] = freq.max(1);
    e
}

/// The right trigger for the weapon in hand. Ours (no original behaviour): a pistol has a light click point, rifles
/// a heavier one, shotguns the strongest, launchers a heavy wall the whole way, tools and blades nothing.
fn trigger_for(class: &str, firing_auto: Option<f32>, strength: f32) -> [u8; 11] {
    let s = |v: f32| ((v * strength).round() as u8).clamp(1, 8);
    if let Some(rate) = firing_auto {
        // Automatic fire: the trigger buzzes at the gun's rate.
        return vibration(2, s(6.0), (rate * 3.0).clamp(10.0, 60.0) as u8);
    }
    match class {
        "WeaponPistol" => weapon(4, 6, s(5.0)),
        "WeaponStealthPistol" => weapon(4, 6, s(3.0)),
        "WeaponAssaultGun" => feedback(3, s(4.0)),
        "WeaponRifle" => weapon(5, 7, s(7.0)),
        "WeaponAssaultShotgun" => weapon(3, 6, s(6.0)),
        "WeaponSawedOffShotgun" => weapon(3, 6, s(8.0)),
        "WeaponGEPGun" | "WeaponLAW" => feedback(2, s(8.0)),
        "WeaponFlamethrower" => feedback(2, s(5.0)),
        "WeaponPlasmaRifle" => weapon(4, 7, s(6.0)),
        "WeaponMiniCrossbow" => weapon(5, 8, s(3.0)),
        "WeaponPepperGun" => feedback(5, s(2.0)),
        "WeaponProd" => feedback(6, s(3.0)),
        "WeaponCombatKnife" | "WeaponSword" | "WeaponNanoSword" | "WeaponCrowbar" | "WeaponBaton" => OFF,
        "WeaponEMPGrenade" | "WeaponGasGrenade" | "WeaponLAM" | "WeaponNanoVirusGrenade" | "WeaponShuriken" => feedback(1, s(2.0)),
        c if c.starts_with("Weapon") => weapon(4, 6, s(4.0)),
        _ => OFF,
    }
}

// ------------------------------------------------------------------------------------------------ game side

/// Update: what the pad shows, from the game state.
#[allow(clippy::too_many_arguments)]
pub fn dualsense_system(
    time: Res<Time<Real>>,
    ph: Option<Res<crate::ai::PlayerHealth>>,
    weapons: Option<Res<crate::weapons::Weapons>>,
    clips: Option<Res<crate::weapons::Clips>>,
    mut t: Local<f32>,
) {
    let Some(inp) = input() else { return };
    *t += time.delta_secs();
    let on = |k: &str| crate::settings::get(k) as i32;
    let menu_up = crate::menus::open();
    let mut o = OUT.lock().unwrap().clone().unwrap_or_default();

    // Light bar: health on GetColorScaled's scale, pulsing below 25%; black when dead; dim blue on the menus.
    o.light = match (on("PadLightbar"), &ph) {
        (0, _) => [0, 0, 0],
        (_, Some(ph)) if !menu_up || ph.dead => {
            let frac = (ph.hp / 100.0).clamp(0.0, 1.0);
            if ph.dead {
                [0, 0, 0]
            } else {
                let c = crate::uicanvas::color_scaled(frac);
                let k = if frac < 0.25 { 0.55 + 0.45 * (*t * 6.0).sin() } else { 1.0 };
                c.map(|v| (v as f32 * k) as u8)
            }
        }
        _ => [0, 24, 64],
    };
    // Player LEDs: the bioelectric energy meter (five steps), or the plain player-one light.
    o.player_leds = match (on("PadPlayerLeds"), &ph) {
        (0, Some(ph)) => {
            let n = ((ph.energy / 100.0).clamp(0.0, 1.0) * 5.0).ceil() as u32;
            // Fill from the outside in toward the centre reads oddly; left to right is the meter.
            [0x00, 0x01, 0x03, 0x07, 0x0f, 0x1f][n.min(5) as usize]
        }
        (1, _) => 0x04,
        _ => 0,
    };
    o.mic_led = if crate::consys::datalink_playing() { 2 } else { 0 };

    // Adaptive triggers.
    let strength = [0.0, 0.6, 1.0][on("PadTriggers").clamp(0, 2) as usize];
    (o.right, o.left) = if strength == 0.0 || menu_up || crate::consys::third_person() {
        (OFF, OFF)
    } else if let Some(h) = weapons.as_ref().and_then(|w| w.held.as_ref()) {
        use crate::weapons::WState;
        let class = h.stats.class.as_str();
        let empty = h.stats.ammo_class.is_some()
            && !h.stats.hand_to_hand
            && clips.as_ref().and_then(|c| c.0.get(class)).is_some_and(|n| *n == 0);
        let right = match h.state {
            // Out of ammo or reloading: a dead trigger.
            _ if empty => OFF,
            WState::Reloading | WState::Selecting | WState::Lowering => OFF,
            WState::Firing if h.stats.automatic => trigger_for(class, Some(1.0 / h.stats.shot_time.max(0.02)), strength),
            _ => trigger_for(class, None, strength),
        };
        // The left trigger (aim / scope): a light pull for anything that shoots.
        let left = if h.stats.hand_to_hand || class.starts_with("Weapon") == false { OFF } else { feedback(4, (2.0 * strength).round().max(1.0) as u8) };
        (right, left)
    } else {
        (OFF, OFF)
    };

    // Motors / HD haptics and the speaker.
    let hd = on("PadHaptics") == 1 && inp.usb && crate::haptics::available();
    o.motor_mode = !hd;
    let (lo, hi) = crate::gamepad::rumble_now();
    o.motors = if hd { (0, 0) } else { ((hi * 255.0) as u8, (lo * 255.0) as u8) };
    o.speaker = inp.usb && on("PadSpeaker") == 1 && crate::haptics::available();
    o.speaker_volume = [0x48, 0x56, 0x64][on("PadSpeakerVolume").clamp(0, 2) as usize];
    crate::haptics::set_bed(if hd { (lo, hi) } else { (0.0, 0.0) });
    set_out(|x| *x = o);
}

// ------------------------------------------------------------------------------------------------ events

/// Game events for the pad's haptics: HD haptics on USB when enabled, otherwise classic rumble.
pub enum Haptic {
    /// A footstep on a floor texture group (Footsteps by surface).
    Step(Option<String>),
    /// Landing from a fall, by impact speed.
    Land(f32),
    /// The player's weapon went off (0..1 strength).
    Shot(f32),
    /// Damage taken, and which side it came from (-1 left .. 1 right).
    Hurt(f32, f32),
    /// An explosion this strong (0..1) after distance.
    Boom(f32),
    /// A decoration (crate, bottle...) smashed nearby: strength 0..1.
    Smash(f32),
}

pub fn event(h: Haptic) {
    if !crate::gamepad::connected() {
        return;
    }
    let scale = [0.0, 0.5, 1.0, 1.5][(crate::settings::get("PadRumble") as usize).min(3)];
    if scale == 0.0 {
        return;
    }
    if matches!(h, Haptic::Step(_)) && crate::settings::get("PadHapticSteps") < 0.5 {
        return;
    }
    let hd = crate::settings::get("PadHaptics") as i32 == 1 && connected_usb() && crate::haptics::available();
    if hd {
        crate::haptics::play(&h, scale);
        return;
    }
    match h {
        Haptic::Step(_) => {}
        Haptic::Land(v) => crate::gamepad::rumble(0.3 * v * scale, 0.1 * scale, 0.12),
        Haptic::Shot(s) => crate::gamepad::rumble((0.2 + 0.6 * s) * scale, (0.4 + 0.4 * s) * scale, 0.06 + 0.1 * s),
        Haptic::Hurt(d, _) => crate::gamepad::rumble((0.3 + d).min(1.0) * scale, 0.5 * scale, 0.2),
        Haptic::Boom(s) => crate::gamepad::rumble(s * scale, 0.6 * s * scale, 0.3 + 0.6 * s),
        Haptic::Smash(s) => crate::gamepad::rumble(0.5 * s * scale, (0.3 + 0.5 * s) * scale, 0.12),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_ieee() {
        assert_eq!(crc32(0xffff_ffff, b"123456789") ^ 0xffff_ffff, 0xcbf4_3926);
    }

    #[test]
    fn trigger_effects() {
        // Feedback from zone 3 at strength 5: zones 3..9, force 4 in each.
        let f = feedback(3, 5);
        assert_eq!(f[0], 0x21);
        assert_eq!(u16::from_le_bytes([f[1], f[2]]), 0b11_1111_1000);
        let forces = u32::from_le_bytes([f[3], f[4], f[5], f[6]]);
        assert_eq!((forces >> 9) & 7, 4);
        assert_eq!(forces & 0x1ff, 0);
        let w = weapon(4, 6, 6);
        assert_eq!((w[0], u16::from_le_bytes([w[1], w[2]]), w[3]), (0x25, (1 << 4) | (1 << 6), 5));
        let v = vibration(2, 8, 30);
        assert_eq!((v[0], v[9]), (0x26, 30));
        assert_eq!(trigger_for("WeaponCombatKnife", None, 1.0), OFF);
        assert_eq!(trigger_for("WeaponMiniCrossbow", None, 1.0)[0], 0x25);
        assert_eq!(trigger_for("Nothing", None, 1.0), OFF);
    }

    #[test]
    fn parses_usb_report() {
        let mut r = [0u8; 63];
        r[0] = 255; // left stick right
        r[1] = 0; // up
        r[2] = 128;
        r[3] = 128;
        r[5] = 255; // R2
        r[7] = 0x08 | 0x20; // hat centred, cross
        r[8] = 0x01 | 0x20; // L1, options
        r[9] = 0x02; // touchpad click
        r[32] = 0x80; // no finger
        r[36] = 0x80;
        let mut fingers: [Finger; 2] = Default::default();
        parse(&r, true, 0.0, &mut fingers);
        let i = input().unwrap();
        assert!(i.ls.x > 0.99 && i.ls.y > 0.99 && i.rs.length() < 0.01 && i.rt == 1.0);
        assert!(i.held.contains(&PadBtn::South) && i.held.contains(&PadBtn::LB) && i.held.contains(&PadBtn::Start));
        assert!(!i.held.contains(&PadBtn::Up));
        assert!(touch_click());
        // A finger at (100, 200) then (160, 230): a drag of (60, 30).
        let put = |r: &mut [u8; 63], x: u16, y: u16| {
            r[32] = 1;
            r[33] = (x & 0xff) as u8;
            r[34] = ((x >> 8) as u8 & 0x0f) | (((y & 0x0f) as u8) << 4);
            r[35] = (y >> 4) as u8;
        };
        put(&mut r, 100, 200);
        parse(&r, true, 0.0, &mut fingers);
        let _ = take_touch();
        put(&mut r, 160, 230);
        parse(&r, true, 0.01, &mut fingers);
        assert_eq!(take_touch().drag, Vec2::new(60.0, 30.0));
    }
}
