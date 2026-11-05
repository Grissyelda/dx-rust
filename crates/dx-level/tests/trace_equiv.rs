//! The optimised BSP sweep must agree with the reference one, hit for hit. Needs the game: DX_GAME=<install dir>.
use glam::DVec3;

/// Outdoor and indoor maps of several sizes; any not in the install is skipped.
const MAPS: &[&str] = &[
    "01_NYC_UNATCOIsland.dx",
    "01_NYC_UNATCOHQ.dx",
    "02_NYC_Street.dx",
    "03_NYC_Airfield.dx",
    "05_NYC_UNATCOMJ12lab.dx",
    "06_HongKong_Helibase.dx",
    "00_Training.dx",
];

#[test]
fn trace_matches_reference() {
    let Ok(root) = std::env::var("DX_GAME") else { return };
    let root = std::path::Path::new(&root);
    for map in MAPS {
        if !root.join("Maps").join(map).exists() {
            continue;
        }
        let lvl = dx_level::load_level(root, map).expect("level");
        let c = &lvl.collision;
        // Deterministic pseudo-random sweeps among the map's actors.
        let mut s = 0x1234_5678_9abc_def0u64;
        let mut r = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let pts: Vec<DVec3> = lvl.actors.iter().map(|a| { let l = a.location(); DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64) }).collect();
        if pts.is_empty() {
            continue;
        }
        let pick = |u: f64| pts[((u * pts.len() as f64) as usize).min(pts.len() - 1)];
        let (mut n, mut hits) = (0, 0);
        let (mut t_new, mut t_old) = (0.0, 0.0);
        for i in 0..30000 {
            let (a, b, ext) = if i % 3 == 2 {
                // Eye to another actor, as the AI's on-screen and sight checks trace.
                let (p, q) = (pick(r()), pick(r()));
                (p + DVec3::new(0.0, 0.0, 40.0), q, DVec3::ZERO)
            } else {
                let a = pick(r()) + DVec3::new(r() - 0.5, r() - 0.5, r() - 0.5) * 200.0;
                let reach = if i % 2 == 0 { 3000.0 } else { 64.0 };
                let b = a + DVec3::new(r() - 0.5, r() - 0.5, r() - 0.5) * reach;
                let ext = match i % 4 {
                    0 => DVec3::ZERO,
                    1 => DVec3::splat(1.0),
                    _ => DVec3::new(20.0, 20.0, 48.0),
                };
                (a, b, ext)
            };
            let t0 = std::time::Instant::now();
            let x = c.trace(a, b, ext);
            let t1 = std::time::Instant::now();
            let y = c.trace_reference(a, b, ext);
            t_new += (t1 - t0).as_secs_f64();
            t_old += t1.elapsed().as_secs_f64();
            n += 1;
            hits += (y.fraction < 1.0) as usize;
            assert_eq!(x.fraction, y.fraction, "{map} sweep {i}: {a:?} -> {b:?} ext {ext:?}");
            assert_eq!(x.normal, y.normal, "{map} sweep {i} normal");
        }
        eprintln!("{map}: {n} sweeps, {hits} hits; trace {:.1} ms, reference {:.1} ms", t_new * 1e3, t_old * 1e3);
    }
}
