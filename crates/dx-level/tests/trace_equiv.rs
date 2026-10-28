//! The optimised BSP sweep must agree with the reference one. Needs the game: DX_GAME=<install dir>.
use glam::DVec3;

#[test]
fn trace_matches_reference() {
    let Ok(root) = std::env::var("DX_GAME") else { return };
    let lvl = dx_level::load_level(std::path::Path::new(&root), "01_NYC_UNATCOIsland.dx").expect("level");
    let c = &lvl.collision;
    // Deterministic pseudo-random sweeps around the island, mixing line traces and box sweeps.
    let mut s = 0x1234_5678_9abc_def0u64;
    let mut r = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let pts: Vec<DVec3> = lvl.actors.iter().map(|a| { let l = a.location(); DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64) }).collect();
    let (mut n, mut hits) = (0, 0);
    let (mut t_new, mut t_old) = (0.0, 0.0);
    for i in 0..20000 {
        let a = pts[(r() * pts.len() as f64) as usize] + DVec3::new(r() - 0.5, r() - 0.5, r() - 0.5) * 200.0;
        let b = if i % 2 == 0 {
            a + DVec3::new(r() - 0.5, r() - 0.5, r() - 0.5) * 3000.0
        } else {
            a + DVec3::new(r() - 0.5, r() - 0.5, r() - 0.5) * 64.0
        };
        let ext = match i % 3 {
            0 => DVec3::ZERO,
            1 => DVec3::splat(1.0),
            _ => DVec3::new(20.0, 20.0, 48.0),
        };
        let t0 = std::time::Instant::now();
        let x = c.trace(a, b, ext);
        let t1 = std::time::Instant::now();
        let y = c.trace_reference(a, b, ext);
        t_new += (t1 - t0).as_secs_f64();
        t_old += t1.elapsed().as_secs_f64();
        n += 1;
        hits += (y.fraction < 1.0) as usize;
        assert_eq!(x.fraction, y.fraction, "sweep {i}: {a:?} -> {b:?} ext {ext:?}");
        assert_eq!(x.normal, y.normal, "sweep {i} normal");
    }
    eprintln!("{n} sweeps, {hits} hits; new {:.1} ms, reference {:.1} ms", t_new * 1e3, t_old * 1e3);
}
