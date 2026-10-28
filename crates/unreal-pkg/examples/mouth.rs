//! mouth <pkg> <mesh>: compare blend-anim frames (MouthA, Blink) with body frames.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    let e = pkg.exports.iter().find(|e| e.name.eq_ignore_ascii_case(&a[2])).unwrap();
    let m = unreal_pkg::mesh::read_lod_mesh(&pkg, e).unwrap();
    println!("verts/frame {} frames {}", m.frame_verts, m.anim_frames);
    for s in &m.anim_seqs { print!("{}({}+{}) ", s.name, s.start_frame, s.num_frames); }
    println!();
    let n = m.frame_verts - m.special_verts;
    let fr = |name: &str| m.seq(name).map(|s| s.start_frame as usize);
    for (x, y) in [("MouthA","Still"),("MouthA","MouthClosed"),("MouthClosed","Still"),("Blink","Still"),("MouthA","BreatheLight"),("MouthClosed","BreatheLight"),("HeadLeft","Still")] {
        let (Some(fx), Some(fy)) = (fr(x), fr(y)) else { println!("{x}/{y} missing"); continue };
        let mut diff = 0; let mut maxd: f32 = 0.0;
        for v in 0..n { let p = m.vertex(fx, v); let q = m.vertex(fy, v);
            let d = ((p[0]-q[0]).powi(2)+(p[1]-q[1]).powi(2)+(p[2]-q[2]).powi(2)).sqrt();
            if d > 0.01 { diff += 1; } maxd = maxd.max(d); }
        println!("{x} vs {y}: {diff}/{n} verts differ, max {maxd}");
    }
}
