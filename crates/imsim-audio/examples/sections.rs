//! Play each song section of an IT file for a few seconds (silently) and print where the player goes.
use xmrsplayer::prelude::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    for sec in 0..6usize {
        let Some(b) = imsim_audio::it_start_at(&bytes, sec) else { println!("sec {sec}: none"); continue };
        let m = xmrs::prelude::Module::load(&b).unwrap();
        let mut p = XmrsPlayer::new(&m, 22050, 0);
        p.set_max_loop_count(0);
        let mut trace = Vec::new();
        for _ in 0..40 {
            let s: Vec<i16> = p.by_ref().take(22050 * 2).collect();
            let rms = (s.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / s.len().max(1) as f64).sqrt();
            trace.push(format!("{}:{}/{:.0}", p.get_current_table_index(), p.get_current_pattern(), rms));
        }
        println!("sec {sec}: {}", trace.join(" "));
    }
}
