//! DualSense USB audio (ROADMAP Phase 0.3, USB only by the user's choice): the pad shows up as a 4-channel 48 kHz
//! output device. Channels 3 and 4 drive the left / right haptic actuators (HD haptics: a waveform, not just a motor
//! speed); channel 2 feeds the pad's speaker when dualsense.rs routes the output path there (infolinks play
//! through it with Infolinks Through Controller Speaker on). Ours, no original behaviour:
//! * footsteps by floor texture group (metal rings high and short, carpet and earth low and soft, water a wash),
//!   alternating feet left / right;
//! * shots (a sub thump plus a crack), landings, explosions (long and deep), damage from the side it came from;
//! * classic rumble requests (`gamepad::rumble`) as a low and a high tone bed, so everything still rumbles.
//! The device is looked for while a DualSense is connected over USB; the stream lives on its own thread (a cpal
//! stream isn't Send everywhere).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const RATE: f32 = 48_000.0;

struct Grain {
    freq: f32,
    amp: f32,
    /// Envelope time constant (s) and cut-off.
    decay: f32,
    len: f32,
    t: f32,
    phase: f32,
    /// -1 left .. 1 right.
    pan: f32,
    /// Share of noise mixed in (texture for cracks, washes, blasts).
    noise: f32,
}

#[derive(Default)]
struct Synth {
    grains: Vec<Grain>,
    bed: (f32, f32),
    bed_now: (f32, f32),
    bed_phase: (f32, f32),
    speech: Option<(Arc<imsim_audio::Clip>, f64, f32)>,
    seed: u32,
}

static SYNTH: Mutex<Option<Synth>> = Mutex::new(None);
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static FOOT: AtomicBool = AtomicBool::new(false);

/// The pad's audio device is open with its four channels.
pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

fn with(f: impl FnOnce(&mut Synth)) {
    if let Ok(mut g) = SYNTH.lock() {
        f(g.get_or_insert_with(Synth::default));
    }
}

pub fn setup() {
    if STARTED.swap(true, Ordering::Relaxed) || std::env::var("DXVIEW_NO_HID").is_ok() {
        return;
    }
    let _ = std::thread::Builder::new().name("dualsense-audio".into()).spawn(audio_thread);
}

fn find_device() -> Option<cpal::Device> {
    let host = cpal::default_host();
    host.output_devices().ok()?.find(|d| {
        d.name().is_ok_and(|n| {
            let n = n.to_ascii_lowercase();
            n.contains("dualsense") || n.contains("wireless controller")
        })
    })
}

fn audio_thread() {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if !crate::dualsense::connected_usb() {
            continue;
        }
        let Some(dev) = find_device() else { continue };
        let Some(cfg) = dev.supported_output_configs().ok().and_then(|mut c| {
            c.find(|c| c.channels() >= 4 && c.min_sample_rate().0 <= 48_000 && c.max_sample_rate().0 >= 48_000)
        }) else {
            continue;
        };
        let cfg = cfg.with_sample_rate(cpal::SampleRate(48_000));
        let fmt = cfg.sample_format();
        let sc: cpal::StreamConfig = cfg.config();
        let ch = sc.channels as usize;
        let err = |e| eprintln!("DualSense audio: {e}");
        let stream = match fmt {
            cpal::SampleFormat::F32 => dev.build_output_stream(&sc, move |o: &mut [f32], _: &_| fill(o, ch, |v| v), err, None),
            cpal::SampleFormat::I16 => {
                dev.build_output_stream(&sc, move |o: &mut [i16], _: &_| fill(o, ch, |v| (v * 32767.0) as i16), err, None)
            }
            cpal::SampleFormat::I32 => {
                dev.build_output_stream(&sc, move |o: &mut [i32], _: &_| fill(o, ch, |v| (v as f64 * 2147483647.0) as i32), err, None)
            }
            _ => continue,
        };
        let Ok(stream) = stream else { continue };
        if stream.play().is_err() {
            continue;
        }
        println!("PAD DualSense audio open ({ch} channels, {fmt:?})");
        AVAILABLE.store(true, Ordering::Relaxed);
        while crate::dualsense::connected_usb() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        AVAILABLE.store(false, Ordering::Relaxed);
        drop(stream);
        with(|s| *s = Synth::default());
    }
}

fn fill<T: Copy>(out: &mut [T], ch: usize, conv: impl Fn(f32) -> T) {
    let mut g = match SYNTH.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let s = g.get_or_insert_with(Synth::default);
    let dt = 1.0 / RATE;
    for frame in out.chunks_mut(ch) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        // The rumble bed: 45 Hz for the big motor, 150 Hz for the small one, eased so steps don't click.
        s.bed_now.0 += (s.bed.0 - s.bed_now.0) * 0.002;
        s.bed_now.1 += (s.bed.1 - s.bed_now.1) * 0.002;
        s.bed_phase.0 = (s.bed_phase.0 + 45.0 * dt).fract();
        s.bed_phase.1 = (s.bed_phase.1 + 150.0 * dt).fract();
        let bed = (s.bed_phase.0 * std::f32::consts::TAU).sin() * s.bed_now.0 * 0.8
            + (s.bed_phase.1 * std::f32::consts::TAU).sin() * s.bed_now.1 * 0.5;
        l += bed;
        r += bed;
        for gr in s.grains.iter_mut() {
            gr.phase = (gr.phase + gr.freq * dt).fract();
            let env = (gr.t / 0.003).min(1.0) * (-gr.t / gr.decay).exp();
            s.seed = s.seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (s.seed >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0;
            let v = ((gr.phase * std::f32::consts::TAU).sin() * (1.0 - gr.noise) + n * gr.noise) * gr.amp * env;
            l += v * (1.0 - gr.pan).min(1.0);
            r += v * (1.0 + gr.pan).min(1.0);
            gr.t += dt;
        }
        s.grains.retain(|g| g.t < g.len);
        // The speaker: the speech clip resampled to 48 kHz on the right channel.
        let mut sp = 0.0f32;
        if let Some((clip, pos, vol)) = s.speech.as_mut() {
            let chans = clip.channels.max(1) as usize;
            let i = *pos as usize;
            if i + 1 < clip.frames() {
                let f = (*pos - i as f64) as f32;
                let a = clip.data[i * chans];
                let b = clip.data[(i + 1) * chans];
                sp = (a + (b - a) * f) * *vol;
                *pos += clip.rate as f64 / RATE as f64;
            } else {
                s.speech = None;
            }
        }
        let soft = |v: f32| v.clamp(-1.5, 1.5) * (1.0 - v.clamp(-1.5, 1.5).powi(2) / 6.75);
        for (c, o) in frame.iter_mut().enumerate() {
            *o = conv(match c {
                1 => soft(sp),
                2 => soft(l),
                3 => soft(r),
                _ => 0.0,
            });
        }
    }
}

/// The classic rumble as tones (low motor, high motor), set every frame by dualsense.rs.
pub fn set_bed(v: (f32, f32)) {
    if available() {
        with(|s| s.bed = v);
    }
}

fn grain(s: &mut Synth, freq: f32, amp: f32, decay: f32, pan: f32, noise: f32) {
    if s.grains.len() < 24 {
        s.grains.push(Grain { freq, amp, decay, len: decay * 5.0, t: 0.0, phase: 0.0, pan, noise });
    }
}

/// An HD haptic event (dualsense::event decides between this and rumble).
pub fn play(h: &crate::dualsense::Haptic, scale: f32) {
    use crate::dualsense::Haptic;
    with(|s| match h {
        Haptic::Step(group) => {
            let foot = FOOT.fetch_xor(true, Ordering::Relaxed);
            let pan = if foot { 0.6 } else { -0.6 };
            let g = group.as_deref().unwrap_or("").to_ascii_lowercase();
            let (f, a, d, n) = match g.as_str() {
                "metal" | "ladder" => (170.0, 0.5, 0.008, 0.1),
                "wood" => (105.0, 0.4, 0.009, 0.15),
                "glass" | "ceramic" | "tiles" => (220.0, 0.35, 0.005, 0.2),
                "textile" | "paper" | "foliage" | "earth" => (60.0, 0.25, 0.012, 0.5),
                "water" => (70.0, 0.3, 0.025, 0.7),
                _ => (85.0, 0.4, 0.008, 0.25),
            };
            grain(s, f, a * scale, d, pan, n);
        }
        Haptic::Land(v) => grain(s, 55.0, (0.3 + 0.6 * v.min(1.0)) * scale, 0.03, 0.0, 0.3),
        Haptic::Shot(st) => {
            grain(s, 60.0, (0.4 + 0.5 * st) * scale, 0.02 + 0.02 * st, 0.3, 0.0);
            grain(s, 140.0, 0.5 * st.max(0.3) * scale, 0.008, 0.3, 0.6);
        }
        Haptic::Hurt(d, side) => grain(s, 90.0, (0.4 + 0.5 * d.min(1.0)) * scale, 0.035, side.clamp(-1.0, 1.0), 0.2),
        Haptic::Smash(st) => {
            // A crack and a rattle of pieces.
            grain(s, 120.0, (0.35 + 0.5 * st.min(1.0)) * scale, 0.012, 0.0, 0.6);
            grain(s, 65.0, 0.4 * st.min(1.0) * scale, 0.03, 0.0, 0.4);
        }
        Haptic::Boom(st) => {
            grain(s, 38.0, st.min(1.0) * scale, 0.08 + 0.16 * st, 0.0, 0.2);
            grain(s, 70.0, 0.6 * st.min(1.0) * scale, 0.04, 0.0, 0.7);
        }
    });
}

/// Infolinks go to the pad speaker (Infolinks Through Controller Speaker, USB).
pub fn speaker_on() -> bool {
    available() && crate::dualsense::connected_usb() && crate::settings::get("PadSpeaker") as i32 == 1
}

/// Play a speech sound on the pad speaker. False when it can't (not decoded yet: the caller plays it normally).
pub fn speak(path: &str) -> bool {
    if !speaker_on() {
        return false;
    }
    let Some(Some(clip)) = crate::sound::cached(path) else { return false };
    let vol = crate::settings::speech_volume().clamp(0.0, 1.0) * 1.6;
    with(|s| s.speech = Some((clip, 0.0, vol)));
    true
}

pub fn stop_speech() {
    with(|s| s.speech = None);
}
