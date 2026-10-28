//! Decode WAV files and print their format, length and loop points.
fn main() {
    for p in std::env::args().skip(1) {
        let b = std::fs::read(&p).unwrap();
        match imsim_audio::decode_wav(&b) {
            Some(c) => println!("{p}: {} ch {} Hz {:.2}s loop {:?} peak {:.2}", c.channels, c.rate, c.duration(), c.loop_range, c.data.iter().fold(0f32, |m, v| m.max(v.abs()))),
            None => println!("{p}: not decodable"),
        }
    }
}
