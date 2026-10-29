//! moddiff <System> <mod DeusEx.u> [Class]: class defaults a mod's DeusEx.u changes (every DeusEx class, or one).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let sys = std::path::Path::new(&a[1]);
    let mut base = unreal_pkg::classes::ClassDb::open(sys);
    let mut m = unreal_pkg::classes::ClassDb::open_replacing(sys, &[("DeusEx", std::path::PathBuf::from(&a[2]))]);
    let classes = match a.get(3) {
        Some(c) => vec![c.clone()],
        None => m.classes_in("DeusEx"),
    };
    for c in classes {
        let b = base.base_defaults(&c);
        let d = m.base_defaults(&c);
        let mut lines = Vec::new();
        for p in &d {
            let old = b.iter().find(|q| q.name.eq_ignore_ascii_case(&p.name) && q.index == p.index);
            if old.map(|o| format!("{:?}", o.value)) != Some(format!("{:?}", p.value)) {
                lines.push(format!("  {}[{}] {:?} -> {:?}", p.name, p.index, old.map(|o| &o.value), p.value));
            }
        }
        for q in &b {
            if !d.iter().any(|p| p.name.eq_ignore_ascii_case(&q.name) && p.index == q.index) {
                lines.push(format!("  {}[{}] {:?} -> (gone)", q.name, q.index, q.value));
            }
        }
        if !lines.is_empty() {
            println!("{c}");
            for l in lines {
                println!("{l}");
            }
        }
    }
}
