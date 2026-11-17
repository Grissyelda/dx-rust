//! tnmaudit <Deus Ex folder> <Nameless Mod folder> <map.dx>...: the classes placed in The Nameless Mod's maps that
//! come from its own packages, with how many there are, their nearest Deus Ex ancestor and how much script of their
//! own they (and their TNM ancestors) carry - what's left to port by hand.
use std::collections::BTreeMap;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (game, tnm) = (std::path::PathBuf::from(&a[1]), std::path::PathBuf::from(&a[2]));
    unreal_pkg::paths::set(&game, unreal_pkg::tnm::overlays(&tnm));
    unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
    unreal_pkg::tnm::set_active(Some(tnm.clone()));
    let mut db = unreal_pkg::classes::ClassDb::open(&game.join("System"));
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for m in &a[3..] {
        let Ok(pkg) = unreal_pkg::Package::open(m) else { eprintln!("can't open {m}"); continue };
        for e in &pkg.exports {
            if e.class_index == 0 || e.size == 0 {
                continue;
            }
            *count.entry(pkg.class_name(e.class_index).to_string()).or_default() += 1;
        }
    }
    let tnm_pkgs: Vec<String> = unreal_pkg::tnm::PACKAGES.iter().map(|p| p.to_ascii_lowercase()).collect();
    let mut rows = Vec::new();
    for (c, n) in count {
        let Some(p) = db.package_of(&c) else { continue };
        if !tnm_pkgs.contains(&p) {
            continue;
        }
        let chain = db.ancestry(&c);
        // Own script lines in the TNM part of the chain (defaultproperties left out).
        let mut lines = 0;
        let mut base = String::new();
        for k in &chain {
            let pk = db.package_of(k).unwrap_or_default();
            if !tnm_pkgs.contains(&pk) {
                base = k.clone();
                break;
            }
            if let Some(t) = db.script_text(k) {
                let body = t.split("defaultproperties").next().unwrap_or("");
                lines += body.lines().filter(|l| { let l = l.trim(); !l.is_empty() && !l.starts_with("//") }).count();
            }
        }
        rows.push((n, c, p, base, lines));
    }
    rows.sort_by(|a, b| b.4.cmp(&a.4).then(b.0.cmp(&a.0)));
    for (n, c, p, base, lines) in rows {
        println!("{n:5} {c:32} {p:14} base={base:24} script_lines={lines}");
    }
}
