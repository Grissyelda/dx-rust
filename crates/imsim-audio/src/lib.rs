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

/// Length of an MPEG-1/2/2.5 layer III payload in seconds, from its frame headers alone (no decoding). An ID3v2 tag
/// in front is skipped, and a Xing or Info frame at the start (a header, not audio) isn't counted. None when no valid
/// frame is found.
pub fn mp3_seconds(b: &[u8]) -> Option<f32> {
    let start = id3v2_end(b);
    let mut pos = start;
    let (mut frames, mut rate, mut per_frame) = (0u64, 0u32, 0u32);
    while pos + 4 <= b.len() {
        let Some(f) = mp3_frame(&b[pos..]) else {
            // Before the first frame, look a little way on (past junk); after it, anything else ends the audio.
            if rate == 0 && pos < start + 4096 {
                pos += 1;
                continue;
            }
            break;
        };
        if pos + f.len > b.len() {
            break;
        }
        if rate == 0 {
            rate = f.rate;
            per_frame = f.per_frame;
            let tag = b.get(pos + f.tag_at..).unwrap_or(&[]);
            if tag.len() >= 4 && (&tag[..4] == b"Xing" || &tag[..4] == b"Info") {
                pos += f.len;
                continue;
            }
        }
        frames += 1;
        pos += f.len;
    }
    (frames > 0 && rate > 0).then(|| frames as f32 * per_frame as f32 / rate as f32)
}

/// The byte after an ID3v2 tag at the start of an MP3 (0 when there isn't one).
fn id3v2_end(b: &[u8]) -> usize {
    if b.len() < 10 || &b[..3] != b"ID3" {
        return 0;
    }
    let size = b[6..10].iter().fold(0usize, |n, &x| (n << 7) | (x & 0x7f) as usize);
    10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 }
}

/// One MPEG-1/2/2.5 layer III frame header at the start of a slice.
struct Mp3Frame {
    /// Frame length in bytes, padding included.
    len: usize,
    rate: u32,
    per_frame: u32,
    /// Offset of a Xing or Info tag inside the frame.
    tag_at: usize,
}

fn mp3_frame(h: &[u8]) -> Option<Mp3Frame> {
    const MPEG1_KBPS: [u32; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const MPEG2_KBPS: [u32; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    if h.len() < 4 || h[0] != 0xff || h[1] & 0xe0 != 0xe0 || (h[1] >> 1) & 3 != 1 {
        return None; // no sync word, or not layer III
    }
    let version = (h[1] >> 3) & 3; // 3 MPEG-1, 2 MPEG-2, 0 MPEG-2.5
    let bitrate_idx = (h[2] >> 4) as usize;
    let rate_idx = ((h[2] >> 2) & 3) as usize;
    if bitrate_idx == 0 || bitrate_idx == 15 || rate_idx == 3 {
        return None; // free format, bad bitrate, reserved rate
    }
    let mpeg1 = version == 3;
    let rate = match version {
        3 => [44100, 48000, 32000][rate_idx],
        2 => [22050, 24000, 16000][rate_idx],
        0 => [11025, 12000, 8000][rate_idx],
        _ => return None,
    };
    let kbps = if mpeg1 { MPEG1_KBPS[bitrate_idx] } else { MPEG2_KBPS[bitrate_idx] };
    let padding = ((h[2] >> 1) & 1) as usize;
    let side = match (mpeg1, h[3] >> 6 == 3) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let crc = if h[1] & 1 == 0 { 2 } else { 0 }; // protection bit clear: a CRC follows the header
    Some(Mp3Frame {
        len: (if mpeg1 { 144 } else { 72 }) * kbps as usize * 1000 / rate as usize + padding,
        rate,
        per_frame: if mpeg1 { 1152 } else { 576 },
        tag_at: 4 + crc + side,
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
    /// Low-pass amount: 1 = open, toward 0 = muffled (a wall between). Smoothed.
    pub filter: f32,
    /// Send to the room reverb (0..1).
    pub reverb: f32,
}

impl Default for VoiceParams {
    fn default() -> Self {
        VoiceParams {
            volume: 1.0,
            pan: 0.0,
            pitch: 1.0,
            looping: false,
            filter: 1.0,
            reverb: 0.0,
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
    /// Low-pass state (left, right) and the smoothed coefficient.
    lp: [f32; 2],
    lpk: f32,
    started: bool,
    done: bool,
}

/// Freeverb (Jezar's, public domain design): 8 parallel combs and 4 series all-passes per side.
struct Reverb {
    combs: [[Comb; 8]; 2],
    alls: [[AllPass; 4]; 2],
    room: f32,
    damp: f32,
    wet: f32,
}

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}

struct AllPass {
    buf: Vec<f32>,
    i: usize,
}

impl Comb {
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * feedback;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

impl AllPass {
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        let y = -x + b;
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

impl Reverb {
    fn new(rate: u32) -> Reverb {
        let k = rate as f32 / 44100.0;
        let len = |n: usize, side: usize| (((n + side * 23) as f32) * k).max(1.0) as usize;
        let comb = |n: usize, side: usize| Comb { buf: vec![0.0; len(n, side)], i: 0, store: 0.0 };
        let ap = |n: usize, side: usize| AllPass { buf: vec![0.0; len(n, side)], i: 0 };
        let cl = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        let al = [556, 441, 341, 225];
        Reverb {
            combs: [0, 1].map(|s| cl.map(|n| comb(n, s))),
            alls: [0, 1].map(|s| al.map(|n| ap(n, s))),
            room: 0.5,
            damp: 0.5,
            wet: 0.0,
        }
    }

    fn process(&mut self, inp: &[f32], out: &mut [f32]) {
        let feedback = 0.7 + 0.28 * self.room;
        let damp = 0.4 * self.damp;
        let frames = inp.len() / 2;
        for f in 0..frames {
            let x = (inp[f * 2] + inp[f * 2 + 1]) * 0.015;
            for side in 0..2 {
                let mut acc = 0.0;
                for c in self.combs[side].iter_mut() {
                    acc += c.tick(x, feedback, damp);
                }
                for a in self.alls[side].iter_mut() {
                    acc = a.tick(acc);
                }
                out[f * 2 + side] += acc * self.wet;
            }
        }
    }
}

/// A zone's echo (Unreal's ZoneInfo reverb: bReverbZone with up to six Delay / Gain taps, MasterGain and CutoffHz,
/// played by the original's Galaxy driver): the sound sent to it comes back after each tap's delay at its gain,
/// low-passed at the cutoff, and the taps feed back a little so a room rings out instead of echoing once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EchoParams {
    /// Tap delays (milliseconds) and gains (0..1); a zero delay or gain is unused.
    pub delays_ms: [f32; 6],
    pub gains: [f32; 6],
    /// MasterGain (0..1) and the low-pass cutoff (Hz).
    pub master: f32,
    pub cutoff_hz: f32,
}

struct Echo {
    buf: Vec<[f32; 2]>,
    i: usize,
    lp: [f32; 2],
    /// The taps being played (eased toward `want`) and the level (0..1, for fading in / out).
    cur: EchoParams,
    level: f32,
}

impl Echo {
    fn new(rate: u32) -> Echo {
        Echo {
            buf: vec![[0.0; 2]; (rate as usize) / 2 + 8],
            i: 0,
            lp: [0.0; 2],
            cur: EchoParams { delays_ms: [0.0; 6], gains: [0.0; 6], master: 0.0, cutoff_hz: 6000.0 },
            level: 0.0,
        }
    }

    fn process(&mut self, rate: u32, inp: &[f32], out: &mut [f32], want: Option<EchoParams>) {
        let frames = inp.len() / 2;
        // Fade over ~0.3 s when the zone changes; switch the taps over once silent.
        let target = if want.is_some_and(|w| Some(w) == Some(self.cur)) { 1.0 } else { 0.0 };
        let step = frames as f32 / (0.3 * rate as f32);
        self.level = if target > self.level { (self.level + step).min(1.0) } else { (self.level - step).max(0.0) };
        if self.level <= 0.0 {
            if let Some(w) = want {
                if w != self.cur {
                    self.buf.iter_mut().for_each(|b| *b = [0.0; 2]);
                    self.lp = [0.0; 2];
                }
                self.cur = w;
            }
            if want.is_none() {
                return;
            }
        }
        let n = self.buf.len();
        let taps: Vec<(usize, f32)> = (0..6)
            .filter(|&k| self.cur.delays_ms[k] > 0.0 && self.cur.gains[k] > 0.0)
            .map(|k| (((self.cur.delays_ms[k] * 0.001 * rate as f32) as usize).clamp(1, n - 1), self.cur.gains[k]))
            .collect();
        let a = 1.0 - (-std::f32::consts::TAU * self.cur.cutoff_hz.clamp(200.0, 20000.0) / rate as f32).exp();
        let master = self.cur.master * self.level;
        for f in 0..frames {
            let mut e = [0.0f32; 2];
            for &(d, g) in &taps {
                let s = self.buf[(self.i + n - d) % n];
                e[0] += s[0] * g;
                e[1] += s[1] * g;
            }
            for c in 0..2 {
                self.lp[c] += (e[c] - self.lp[c]) * a;
            }
            // Input plus a quarter of the echo fed back (the room rings out).
            self.buf[self.i] = [inp[f * 2] + self.lp[0] * 0.25, inp[f * 2 + 1] + self.lp[1] * 0.25];
            self.i = (self.i + 1) % n;
            out[f * 2] += self.lp[0] * master;
            out[f * 2 + 1] += self.lp[1] * master;
        }
    }
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
    /// Better playback: 4-point interpolation and constant-power panning.
    hq: bool,
    reverb: Option<Reverb>,
    /// The reverb's targets (room size, damping, wet level); eased toward.
    reverb_want: (f32, f32, f32),
    /// The listener's zone echo (None: not a reverb zone).
    echo: Option<Echo>,
    echo_want: Option<EchoParams>,
}

/// 4-point, 3rd-order Hermite interpolation between y1 and y2 (x 0..1).
fn hermite(x: f32, y0: f32, y1: f32, y2: f32, y3: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * x + c2) * x + c1) * x + y1
}

impl State {
    fn mix(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let mut buf = vec![0.0f32; frames * 2];
        let echo_on = self.echo_want.is_some() || self.echo.as_ref().is_some_and(|e| e.level > 0.0);
        let reverb_on = self.reverb_want.2 > 0.0 || self.reverb.as_ref().is_some_and(|r| r.wet > 1e-4) || echo_on;
        let mut send = if reverb_on { vec![0.0f32; frames * 2] } else { Vec::new() };
        let rate = self.rate as f64;
        let smooth = 1.0 - (-1.0 / (0.004 * self.rate as f32)).exp();
        let hq = self.hq;
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
            let (tl, tr) = if hq {
                // Constant power: the centre isn't louder or quieter than the sides.
                let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                (vol * a.cos() * std::f32::consts::SQRT_2, vol * a.sin() * std::f32::consts::SQRT_2)
            } else {
                (vol * (1.0 - pan).min(1.0), vol * (1.0 + pan).min(1.0))
            };
            // One-pole low-pass: open at 1, about 600 Hz at 0.
            let fc = 600.0 * (40.0f32).powf(v.p.filter.clamp(0.0, 1.0));
            let k_want = if v.p.filter >= 0.999 { 1.0 } else { 1.0 - (-std::f32::consts::TAU * fc / self.rate as f32).exp() };
            if !v.started {
                v.started = true;
                v.gl = tl;
                v.gr = tr;
                v.lpk = k_want;
            }
            let rsend = v.p.reverb.clamp(0.0, 1.0);
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
                let (mut l, mut r) = if hq {
                    // The frames before i and after j, wrapping in a loop, held at the ends otherwise.
                    let prev = if i > ls { i - 1 } else if v.p.looping && le > ls { le - 1 } else { i };
                    let next = if j + 1 < le { j + 1 } else if v.p.looping { ls } else { j };
                    if ch == 1 {
                        let s = hermite(t, c.data[prev], c.data[i], c.data[j], c.data[next]);
                        (s, s)
                    } else {
                        let at = |k: usize, o: usize| c.data[k * ch + o];
                        (
                            hermite(t, at(prev, 0), at(i, 0), at(j, 0), at(next, 0)),
                            hermite(t, at(prev, 1), at(i, 1), at(j, 1), at(next, 1)),
                        )
                    }
                } else if ch == 1 {
                    let s = c.data[i] + (c.data[j] - c.data[i]) * t;
                    (s, s)
                } else {
                    let (a, b) = (i * ch, j * ch);
                    (
                        c.data[a] + (c.data[b] - c.data[a]) * t,
                        c.data[a + 1] + (c.data[b + 1] - c.data[a + 1]) * t,
                    )
                };
                v.lpk += (k_want - v.lpk) * smooth;
                if v.lpk < 0.999 {
                    v.lp[0] += (l - v.lp[0]) * v.lpk;
                    v.lp[1] += (r - v.lp[1]) * v.lpk;
                    l = v.lp[0];
                    r = v.lp[1];
                } else {
                    v.lp = [l, r];
                }
                v.gl += (tl - v.gl) * smooth;
                v.gr += (tr - v.gr) * smooth;
                buf[f * 2] += l * v.gl;
                buf[f * 2 + 1] += r * v.gr;
                if reverb_on && rsend > 0.0 {
                    send[f * 2] += l * v.gl * rsend;
                    send[f * 2 + 1] += r * v.gr * rsend;
                }
                v.pos += step;
            }
        }
        self.voices.retain(|v| !v.done);
        if echo_on {
            let rate = self.rate;
            let want = self.echo_want;
            self.echo.get_or_insert_with(|| Echo::new(rate)).process(rate, &send, &mut buf, want);
        }
        if reverb_on && (self.reverb_want.2 > 0.0 || self.reverb.as_ref().is_some_and(|r| r.wet > 1e-4)) {
            let rate = self.rate;
            let (room, damp, wet) = self.reverb_want;
            let rv = self.reverb.get_or_insert_with(|| Reverb::new(rate));
            // Ease the room over about half a second (per block).
            let k = (frames as f32 / (0.5 * rate as f32)).min(1.0);
            rv.room += (room - rv.room) * k;
            rv.damp += (damp - rv.damp) * k;
            rv.wet += (wet - rv.wet) * k;
            rv.process(&send, &mut buf);
        }
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
            hq: false,
            reverb: None,
            reverb_want: (0.5, 0.5, 0.0),
            echo: None,
            echo_want: None,
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
            lp: [0.0; 2],
            lpk: 1.0,
            started: false,
            done: false,
        });
        id
    }

    /// Better playback (4-point interpolation, constant-power pan) on or off.
    pub fn set_quality(&self, hq: bool) {
        self.state.lock().unwrap().hq = hq;
    }

    /// The room reverb: room size 0..1, damping 0..1, wet level (0 = off). Eased toward.
    pub fn set_reverb(&self, room: f32, damp: f32, wet: f32) {
        self.state.lock().unwrap().reverb_want = (room.clamp(0.0, 1.0), damp.clamp(0.0, 1.0), wet.max(0.0));
    }

    /// The listener's zone echo (Unreal's reverb zones), or None. Voices feed it through their reverb send.
    pub fn set_echo(&self, e: Option<EchoParams>) {
        self.state.lock().unwrap().echo_want = e;
    }

    /// Update many voices' filter and reverb send under one lock: (id, filter, reverb).
    pub fn set_fx_many(&self, list: &[(VoiceId, f32, f32)]) {
        let mut s = self.state.lock().unwrap();
        for &(id, filter, reverb) in list {
            if let Some(v) = s.voices.iter_mut().find(|v| v.id == id) {
                v.p.filter = filter;
                v.p.reverb = reverb;
            }
        }
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
            hq: false,
            reverb: None,
            reverb_want: (0.5, 0.5, 0.0),
            echo: None,
            echo_want: None,
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
            lp: [0.0; 2],
            lpk: 1.0,
            started: false,
            done: false,
        });
        let mut out = vec![0.0f32; 400];
        s.mix(&mut out, 2);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] - 0.5).abs() < 1e-3);
        assert_eq!(out[250], 0.0);
        assert!(s.voices.is_empty());
    }

    #[test]
    fn hermite_passes_through_points() {
        assert!((hermite(0.0, 0.0, 1.0, 2.0, 3.0) - 1.0).abs() < 1e-6);
        assert!((hermite(1.0, 0.0, 1.0, 2.0, 3.0) - 2.0).abs() < 1e-6);
        assert!((hermite(0.5, 0.0, 1.0, 2.0, 3.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn reverb_rings_on() {
        let mut r = Reverb::new(44100);
        r.wet = 1.0;
        r.room = 0.8;
        let mut inp = vec![0.0f32; 2 * 8000];
        inp[0] = 1.0;
        inp[1] = 1.0;
        let mut out = vec![0.0f32; 2 * 8000];
        r.process(&inp, &mut out);
        // Silent before the shortest comb, ringing after it.
        assert!(out[..200].iter().all(|x| x.abs() < 1e-9));
        assert!(out[2400..].iter().any(|x| x.abs() > 1e-5));
    }

    /// An MPEG-1 layer III frame: 128 kbps at 44.1 kHz, stereo, no CRC, no padding, 417 bytes.
    fn mp3_frame_bytes() -> Vec<u8> {
        let mut f = vec![0u8; 417];
        f[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        f
    }

    #[test]
    fn mp3_length_from_frame_headers() {
        let b: Vec<u8> = (0..100).flat_map(|_| mp3_frame_bytes()).collect();
        let secs = mp3_seconds(&b).expect("frames");
        assert!((secs - 100.0 * 1152.0 / 44100.0).abs() < 1e-4);
    }

    #[test]
    fn mp3_xing_frame_is_not_audio() {
        let mut first = mp3_frame_bytes();
        first[36..40].copy_from_slice(b"Xing");
        let b: Vec<u8> = std::iter::once(first).chain((0..100).map(|_| mp3_frame_bytes())).flatten().collect();
        let secs = mp3_seconds(&b).expect("frames");
        assert!((secs - 100.0 * 1152.0 / 44100.0).abs() < 1e-4);
    }

    /// Against the decoder, on a real file: DX_MP3_SAMPLE=<file.mp3> (skipped when unset).
    #[test]
    fn mp3_length_matches_the_decoder() {
        let Some(path) = std::env::var_os("DX_MP3_SAMPLE") else { return };
        let b = std::fs::read(path).expect("sample");
        let decoded = decode_mp3(&b).expect("decodes").duration();
        let walked = mp3_seconds(&b).expect("walks");
        assert!((decoded - walked).abs() < 0.05, "decoded {decoded} walked {walked}");
    }
}
