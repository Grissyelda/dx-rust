//! imsim-audio: a small software mixer for immersive sims.
//!
//! * Voices: decoded PCM clips played once or looped, each with a volume, a stereo pan and a pitch that the game
//!   can change every frame (positional sounds are attenuated and panned by the caller, see [`spatialize`]).
//! * Music: tracker modules (IT/S3M/XM/MOD, played by `xmrsplayer`), started at a given order position
//!   ("song section") and cross-faded when the section or song changes.
//!
//! Output goes to the default device through cpal on a thread of its own. Without a device every call still
//! works and simply produces no sound, so headless tests and machines without audio run the same code.

use std::sync::{Arc, Mutex};

pub use xmrs::prelude::Module;

/// Decoded PCM, stored as f32 frames (interleaved when stereo).
pub struct Clip {
    pub data: Vec<f32>,
    pub channels: u16,
    pub rate: u32,
    /// Loop points in frames from a WAV `smpl` chunk (start, end exclusive).
    pub loop_range: Option<(usize, usize)>,
}

impl Clip {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
    pub fn duration(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }
}

/// Decode a sound payload: RIFF WAVE, or MPEG audio (Deus Ex's conversation speech is MP3).
pub fn decode(b: &[u8]) -> Option<Clip> {
    if b.len() >= 4 && &b[0..4] == b"RIFF" {
        return decode_wav(b);
    }
    decode_mp3(b)
}

/// Decode MPEG-1/2 layer III to f32 PCM.
pub fn decode_mp3(b: &[u8]) -> Option<Clip> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let cursor = std::io::Cursor::new(b.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?.clone();
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let mut data = Vec::new();
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(1);
    let mut rate = track.codec_params.sample_rate.unwrap_or(22050);
    while let Ok(packet) = format.next_packet() {
        let Ok(buf) = dec.decode(&packet) else {
            continue;
        };
        let spec = *buf.spec();
        channels = spec.channels.count() as u16;
        rate = spec.rate;
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        data.extend_from_slice(sb.samples());
    }
    if data.is_empty() {
        return None;
    }
    Some(Clip {
        data,
        channels: channels.max(1),
        rate,
        loop_range: None,
    })
}

/// Decode a RIFF WAVE file holding 8- or 16-bit PCM (mono or stereo). Reads `smpl` loop points if present.
pub fn decode_wav(b: &[u8]) -> Option<Clip> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut pos = 12;
    let (mut fmt, mut channels, mut rate, mut bits) = (0u16, 1u16, 22050u32, 16u16);
    let mut data: Option<&[u8]> = None;
    let mut loop_range = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = (body + len).min(b.len());
        match id {
            b"fmt " if len >= 16 => {
                fmt = u16_at(body);
                channels = u16_at(body + 2).max(1);
                rate = u32_at(body + 4);
                bits = u16_at(body + 14);
            }
            b"data" => data = Some(&b[body..end]),
            b"smpl" if len >= 36 + 24 => {
                // SamplerChunk: 36 bytes of header, num loops at +28; each loop is 24 bytes (id, type, start, end, ...).
                let loops = u32_at(body + 28);
                if loops > 0 {
                    let l = body + 36;
                    let (s, e) = (u32_at(l + 8) as usize, u32_at(l + 12) as usize);
                    if e > s {
                        loop_range = Some((s, e + 1));
                    }
                }
            }
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    if fmt != 1 {
        return None;
    }
    let d = data?;
    let samples: Vec<f32> = match bits {
        8 => d.iter().map(|&v| (v as f32 - 128.0) / 128.0).collect(),
        16 => d
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        _ => return None,
    };
    let frames = samples.len() / channels as usize;
    let loop_range = loop_range
        .map(|(s, e): (usize, usize)| (s.min(frames), e.min(frames)))
        .filter(|(s, e)| e > s);
    Some(Clip {
        data: samples,
        channels,
        rate,
        loop_range,
    })
}

/// Unreal-style positional parameters for a sound at `src` heard from `listener` (any units, any handedness as
/// long as `right` is the listener's unit right vector in the same space). Linear roll-off clamped to `radius`
/// (OpenAL LINEAR_DISTANCE_CLAMPED with reference distance radius/10 and roll-off 1.1, as SurrealEngine sets it
/// up); pan is the sideways component of the direction to the source, softened so nothing goes fully one-eared.
/// Returns (gain 0..1, pan -1..1).
pub fn spatialize(listener: [f64; 3], right: [f64; 3], src: [f64; 3], radius: f64) -> (f32, f32) {
    let d = [
        src[0] - listener[0],
        src[1] - listener[1],
        src[2] - listener[2],
    ];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let r = radius.max(1.0);
    let rf = 0.1 * r;
    let g = 1.0 - 1.1 * (dist.clamp(rf, r) - rf) / (r - rf);
    let gain = g.clamp(0.0, 1.0) as f32;
    let pan = if dist < 1e-3 {
        0.0
    } else {
        ((d[0] * right[0] + d[1] * right[1] + d[2] * right[2]) / dist) as f32
    };
    // Close sources are wide: fade the pan in over the first 10% of the radius.
    let near = ((dist / rf) as f32).clamp(0.0, 1.0);
    (gain, pan * 0.8 * near)
}

#[derive(Clone, Copy, Debug)]
pub struct VoiceParams {
    pub volume: f32,
    pub pan: f32,
    pub pitch: f32,
    pub looping: bool,
}

impl Default for VoiceParams {
    fn default() -> Self {
        VoiceParams {
            volume: 1.0,
            pan: 0.0,
            pitch: 1.0,
            looping: false,
        }
    }
}

pub type VoiceId = u64;

struct Voice {
    id: VoiceId,
    clip: Arc<Clip>,
    pos: f64,
    p: VoiceParams,
    /// Smoothed left/right gains (move toward the targets over a few ms to avoid zipper noise).
    gl: f32,
    gr: f32,
    started: bool,
    done: bool,
}

/// Music fade lengths in seconds for Unreal's EMusicTransition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    Instant,
    Fade,
    FastFade,
    SlowFade,
}

impl Transition {
    pub fn seconds(self) -> f32 {
        match self {
            Transition::Instant => 0.0,
            Transition::Fade => 1.0,
            Transition::FastFade => 0.33,
            Transition::SlowFade => 5.0,
        }
    }
}

struct Track {
    player: xmrsplayer::prelude::XmrsPlayer<'static>,
    gain: f32,
    target: f32,
    /// Gain change per output frame.
    step: f32,
}

struct State {
    rate: u32,
    voices: Vec<Voice>,
    tracks: Vec<Track>,
    sfx_volume: f32,
    music_volume: f32,
    max_voices: usize,
}

impl State {
    fn mix(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let mut buf = vec![0.0f32; frames * 2];
        let rate = self.rate as f64;
        let smooth = 1.0 - (-1.0 / (0.004 * self.rate as f32)).exp();
        for v in self.voices.iter_mut() {
            if v.done {
                continue;
            }
            let c = &*v.clip;
            let n = c.frames();
            if n == 0 {
                v.done = true;
                continue;
            }
            let pan = v.p.pan.clamp(-1.0, 1.0);
            let vol = (v.p.volume * self.sfx_volume).clamp(0.0, 1.0);
            let (tl, tr) = (vol * (1.0 - pan).min(1.0), vol * (1.0 + pan).min(1.0));
            if !v.started {
                v.started = true;
                v.gl = tl;
                v.gr = tr;
            }
            let step = c.rate as f64 / rate * v.p.pitch.max(0.05) as f64;
            let (ls, le) = if v.p.looping {
                c.loop_range.unwrap_or((0, n))
            } else {
                (0, n)
            };
            let ch = c.channels as usize;
            for f in 0..frames {
                let mut i = v.pos as usize;
                if i >= le.min(n) {
                    if v.p.looping && le > ls {
                        v.pos = ls as f64 + (v.pos - le as f64) % (le - ls) as f64;
                        i = v.pos as usize;
                    } else {
                        v.done = true;
                        break;
                    }
                }
                let t = (v.pos - i as f64) as f32;
                let j = if i + 1 < le {
                    i + 1
                } else if v.p.looping {
                    ls
                } else {
                    i
                };
                let (l, r) = if ch == 1 {
                    let s = c.data[i] + (c.data[j] - c.data[i]) * t;
                    (s, s)
                } else {
                    let (a, b) = (i * ch, j * ch);
                    (
                        c.data[a] + (c.data[b] - c.data[a]) * t,
                        c.data[a + 1] + (c.data[b + 1] - c.data[a + 1]) * t,
                    )
                };
                v.gl += (tl - v.gl) * smooth;
                v.gr += (tr - v.gr) * smooth;
                buf[f * 2] += l * v.gl;
                buf[f * 2 + 1] += r * v.gr;
                v.pos += step;
            }
        }
        self.voices.retain(|v| !v.done);
        let mv = self.music_volume;
        for t in self.tracks.iter_mut() {
            for f in 0..frames {
                let (Some(l), Some(r)) = (t.player.next(), t.player.next()) else {
                    break;
                };
                if t.gain != t.target {
                    t.gain = if t.gain < t.target {
                        (t.gain + t.step).min(t.target)
                    } else {
                        (t.gain - t.step).max(t.target)
                    };
                }
                // Tracker output sits well below full scale; lift it toward the level of the sound effects.
                let g = t.gain * mv * 1.6 / 32768.0;
                buf[f * 2] += l as f32 * g;
                buf[f * 2 + 1] += r as f32 * g;
            }
        }
        self.tracks.retain(|t| !(t.target == 0.0 && t.gain <= 0.0));
        for f in 0..frames {
            let (l, r) = (soft_clip(buf[f * 2]), soft_clip(buf[f * 2 + 1]));
            let o = &mut out[f * channels..(f + 1) * channels];
            match channels {
                1 => o[0] = 0.5 * (l + r),
                _ => {
                    o[0] = l;
                    o[1] = r;
                    for x in o[2..].iter_mut() {
                        *x = 0.0;
                    }
                }
            }
        }
    }
}

/// Gentle limiter: linear up to 0.8, then eases toward 1.
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

/// Handle to the mixer. Cheap to share; all methods take `&self`.
pub struct Audio {
    state: Arc<Mutex<State>>,
    next_id: Mutex<VoiceId>,
    /// Whether an output device is running.
    pub live: bool,
}

impl Audio {
    /// Open the default output device. Falls back to a silent mixer if there is none.
    pub fn new() -> Audio {
        let state = Arc::new(Mutex::new(State {
            rate: 48000,
            voices: Vec::new(),
            tracks: Vec::new(),
            sfx_volume: 1.0,
            music_volume: 1.0,
            max_voices: 48,
        }));
        let (tx, rx) = std::sync::mpsc::channel::<Option<u32>>();
        let st = state.clone();
        let _ = std::thread::Builder::new()
            .name("imsim-audio".into())
            .spawn(move || match device::open(st) {
                Ok((stream, rate)) => {
                    let _ = tx.send(Some(rate));
                    let _keep = stream;
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    eprintln!("audio: no output ({e})");
                    let _ = tx.send(None);
                }
            });
        let live = match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Some(rate)) => {
                state.lock().unwrap().rate = rate;
                println!("audio: output at {rate} Hz");
                true
            }
            _ => false,
        };
        Audio {
            state,
            next_id: Mutex::new(1),
            live,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.state.lock().unwrap().rate
    }

    pub fn set_volumes(&self, sfx: f32, music: f32) {
        let mut s = self.state.lock().unwrap();
        s.sfx_volume = sfx;
        s.music_volume = music;
    }

    /// Start a voice. When all voices are busy the quietest non-looping one is dropped.
    pub fn play(&self, clip: Arc<Clip>, p: VoiceParams) -> VoiceId {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            *n
        };
        if !self.live {
            return id;
        }
        let mut s = self.state.lock().unwrap();
        if s.voices.len() >= s.max_voices {
            if let Some(k) = s
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.p.looping)
                .min_by(|a, b| a.1.p.volume.total_cmp(&b.1.p.volume))
                .map(|(k, _)| k)
            {
                s.voices.swap_remove(k);
            } else {
                return id;
            }
        }
        s.voices.push(Voice {
            id,
            clip,
            pos: 0.0,
            p,
            gl: 0.0,
            gr: 0.0,
            started: false,
            done: false,
        });
        id
    }

    pub fn set(&self, id: VoiceId, volume: f32, pan: f32, pitch: f32) {
        if let Some(v) = self
            .state
            .lock()
            .unwrap()
            .voices
            .iter_mut()
            .find(|v| v.id == id)
        {
            v.p.volume = volume;
            v.p.pan = pan;
            v.p.pitch = pitch;
        }
    }

    pub fn stop(&self, id: VoiceId) {
        self.state.lock().unwrap().voices.retain(|v| v.id != id);
    }

    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.live && self.state.lock().unwrap().voices.iter().any(|v| v.id == id)
    }

    /// Update many voices under one lock: (id, volume, pan, pitch).
    pub fn set_many(&self, list: &[(VoiceId, f32, f32, f32)]) {
        let mut s = self.state.lock().unwrap();
        for &(id, vol, pan, pitch) in list {
            if let Some(v) = s.voices.iter_mut().find(|v| v.id == id) {
                v.p.volume = vol;
                v.p.pan = pan;
                v.p.pitch = pitch;
            }
        }
    }

    /// Play `module` from its start, fading out whatever was playing. The module must live for the rest of the
    /// program (get it from [`Song::section`] or [`load_module`]).
    pub fn play_music(&self, module: &'static Module, transition: Transition) {
        if !self.live {
            return;
        }
        let rate = self.sample_rate();
        let mut player = xmrsplayer::prelude::XmrsPlayer::new(module, rate, 0);
        player.set_max_loop_count(0);
        let secs = transition.seconds();
        let step = if secs <= 0.0 {
            1.0
        } else {
            1.0 / (secs * rate as f32)
        };
        let mut s = self.state.lock().unwrap();
        for t in s.tracks.iter_mut() {
            t.target = 0.0;
            t.step = step;
        }
        let start = if secs <= 0.0 { 1.0 } else { 0.0 };
        s.tracks.push(Track {
            player,
            gain: start,
            target: 1.0,
            step,
        });
    }

    pub fn stop_music(&self, transition: Transition) {
        let secs = transition.seconds();
        let rate = self.sample_rate() as f32;
        let mut s = self.state.lock().unwrap();
        for t in s.tracks.iter_mut() {
            t.target = 0.0;
            t.step = if secs <= 0.0 {
                1.0
            } else {
                1.0 / (secs * rate)
            };
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Audio::new()
    }
}

/// An Impulse Tracker module rearranged to start at order position `section` (Unreal's song sections: a
/// level's music file holds ambient, combat, dying... parts at fixed order positions). Players walk a song from
/// order 0, and parts that are only ever reached by jumping into them are never on that walk, so the order list
/// is rotated to begin at `section` and every Bxx (jump to order) target is moved by the same amount. The edit is
/// in place: same file size, same offsets. Returns None if `bytes` isn't IT or the section is out of range.
pub fn it_start_at(bytes: &[u8], section: usize) -> Option<Vec<u8>> {
    if bytes.len() < 0xC0 || &bytes[0..4] != b"IMPM" {
        return None;
    }
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let ordn = u16_at(bytes, 0x20);
    let (insn, smpn, patn) = (
        u16_at(bytes, 0x22),
        u16_at(bytes, 0x24),
        u16_at(bytes, 0x26),
    );
    let orders = bytes.get(0xC0..0xC0 + ordn)?;
    // The song proper: everything before the first end marker.
    let len = orders.iter().position(|&o| o == 255).unwrap_or(ordn);
    if section == 0 {
        return Some(bytes.to_vec());
    }
    if section >= len {
        return None;
    }
    let mut out = bytes.to_vec();
    for i in 0..len {
        out[0xC0 + i] = orders[(i + section) % len];
    }
    let pat_table = 0xC0 + ordn + 4 * (insn + smpn);
    for p in 0..patn {
        let o = pat_table + 4 * p;
        let off = u32::from_le_bytes(out.get(o..o + 4)?.try_into().ok()?) as usize;
        if off == 0 || off + 8 > out.len() {
            continue;
        }
        let plen = u16_at(&out, off);
        let rows = u16_at(&out, off + 2);
        let mut i = off + 8;
        let end = (off + 8 + plen).min(out.len());
        let mut masks = [0u8; 64];
        let mut row = 0;
        while i < end && row < rows {
            let cv = out[i];
            i += 1;
            if cv == 0 {
                row += 1;
                continue;
            }
            let ch = ((cv - 1) & 63) as usize;
            if cv & 128 != 0 {
                masks[ch] = *out.get(i)?;
                i += 1;
            }
            let m = masks[ch];
            if m & 1 != 0 {
                i += 1;
            }
            if m & 2 != 0 {
                i += 1;
            }
            if m & 4 != 0 {
                i += 1;
            }
            if m & 8 != 0 {
                if i + 1 >= out.len() {
                    break;
                }
                // Command 2 = Bxx.
                if out[i] == 2 {
                    let t = out[i + 1] as usize;
                    if t < len {
                        out[i + 1] = ((t + len - section) % len) as u8;
                    }
                }
                i += 2;
            }
        }
    }
    Some(out)
}

/// A music file and the modules made from it, one per song section (see [`it_start_at`]), loaded on demand.
pub struct Song {
    bytes: Vec<u8>,
    sections: std::collections::HashMap<usize, Option<&'static Module>>,
}

impl Song {
    pub fn new(bytes: Vec<u8>) -> Song {
        Song {
            bytes,
            sections: std::collections::HashMap::new(),
        }
    }

    /// The module that starts at order position `n`. Formats other than IT play from the start.
    pub fn section(&mut self, n: usize) -> Option<&'static Module> {
        if let Some(m) = self.sections.get(&n) {
            return *m;
        }
        let m = match it_start_at(&self.bytes, n) {
            Some(b) => load_module(&b),
            None if n == 0 || &self.bytes[..4.min(self.bytes.len())] != b"IMPM" => {
                self.section_raw()
            }
            None => None,
        };
        self.sections.insert(n, m);
        m
    }

    fn section_raw(&mut self) -> Option<&'static Module> {
        load_module(&self.bytes)
    }
}

/// Parse a tracker module and keep it for the rest of the program.
pub fn load_module(bytes: &[u8]) -> Option<&'static Module> {
    match Module::load(bytes) {
        Ok(m) => Some(Box::leak(Box::new(m))),
        Err(e) => {
            eprintln!("audio: module failed to load: {e:?}");
            None
        }
    }
}

mod device {
    use super::State;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    pub fn open(state: Arc<Mutex<State>>) -> Result<(cpal::Stream, u32), String> {
        let host = cpal::default_host();
        let dev = host
            .default_output_device()
            .ok_or("no default output device")?;
        let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let sc: cpal::StreamConfig = cfg.config();
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                &sc,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut s) = state.lock() {
                        s.mix(out, channels);
                    } else {
                        out.fill(0.0);
                    }
                },
                err,
                None,
            ),
            fmt => build_converted(&dev, &sc, fmt, state, channels),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok((stream, rate))
    }

    fn build_converted(
        dev: &cpal::Device,
        sc: &cpal::StreamConfig,
        fmt: cpal::SampleFormat,
        state: Arc<Mutex<State>>,
        channels: usize,
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {
        fn go<T: cpal::SizedSample + cpal::FromSample<f32>>(
            dev: &cpal::Device,
            sc: &cpal::StreamConfig,
            state: Arc<Mutex<State>>,
            channels: usize,
        ) -> Result<cpal::Stream, cpal::BuildStreamError> {
            let mut tmp: Vec<f32> = Vec::new();
            dev.build_output_stream(
                sc,
                move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                    tmp.resize(out.len(), 0.0);
                    if let Ok(mut s) = state.lock() {
                        s.mix(&mut tmp, channels);
                    } else {
                        tmp.fill(0.0);
                    }
                    for (o, &x) in out.iter_mut().zip(tmp.iter()) {
                        *o = T::from_sample(x);
                    }
                },
                |e| eprintln!("audio stream error: {e}"),
                None,
            )
        }
        match fmt {
            cpal::SampleFormat::I16 => go::<i16>(dev, sc, state, channels),
            cpal::SampleFormat::U16 => go::<u16>(dev, sc, state, channels),
            cpal::SampleFormat::I32 => go::<i32>(dev, sc, state, channels),
            cpal::SampleFormat::F64 => go::<f64>(dev, sc, state, channels),
            _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let mut b = Vec::new();
        let data_len = samples.len() * 2;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    #[test]
    fn decodes_pcm16() {
        let c = decode_wav(&wav(&[0, 16384, -16384, 32767], 22050)).unwrap();
        assert_eq!(c.frames(), 4);
        assert_eq!(c.rate, 22050);
        assert!((c.data[1] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn attenuation_is_linear_and_clamped() {
        let (g0, _) = spatialize([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 1000.0);
        let (g1, _) = spatialize([0.0; 3], [0.0, 1.0, 0.0], [500.0, 0.0, 0.0], 1000.0);
        let (g2, _) = spatialize([0.0; 3], [0.0, 1.0, 0.0], [1000.0, 0.0, 0.0], 1000.0);
        assert_eq!(g0, 1.0);
        assert!(g1 > 0.3 && g1 < 0.6, "{g1}");
        assert_eq!(g2, 0.0);
        let (_, pan) = spatialize([0.0; 3], [0.0, 1.0, 0.0], [0.0, 500.0, 0.0], 1000.0);
        assert!(pan > 0.7);
    }

    #[test]
    fn voices_mix_and_finish() {
        let mut s = State {
            rate: 22050,
            voices: Vec::new(),
            tracks: Vec::new(),
            sfx_volume: 1.0,
            music_volume: 1.0,
            max_voices: 8,
        };
        let clip = Arc::new(Clip {
            data: vec![0.5; 100],
            channels: 1,
            rate: 22050,
            loop_range: None,
        });
        s.voices.push(Voice {
            id: 1,
            clip,
            pos: 0.0,
            p: VoiceParams::default(),
            gl: 0.0,
            gr: 0.0,
            started: false,
            done: false,
        });
        let mut out = vec![0.0f32; 400];
        s.mix(&mut out, 2);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] - 0.5).abs() < 1e-3);
        assert_eq!(out[250], 0.0);
        assert!(s.voices.is_empty());
    }
}
