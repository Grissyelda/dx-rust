//! Writes a grown plant as OBJ: treeobj <park|tall|bush> <seed> <out.obj>
use imsim_plants::tree::{grow, Species};
use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let s = match a.get(1).map(String::as_str) {
        Some("tall") => Species::tall_tree(),
        Some("bush") => Species::bush(),
        _ => Species::park_tree(),
    };
    let p = grow(&s, a.get(2).and_then(|v| v.parse().ok()).unwrap_or(1));
    let mut f = std::fs::File::create(a.get(3).map(String::as_str).unwrap_or("tree.obj")).unwrap();
    let mut base = 1;
    for (name, m) in [("bark", &p.bark), ("leaves", &p.leaves)] {
        writeln!(f, "o {name}").unwrap();
        for v in &m.positions {
            writeln!(f, "v {} {} {}", v[0], v[1], v[2]).unwrap();
        }
        for t in m.indices.chunks(3) {
            writeln!(f, "f {} {} {}", t[0] + base, t[1] + base, t[2] + base).unwrap();
        }
        base += m.positions.len() as u32;
    }
    println!("bark {} tris, leaves {} tris, bounds {:?}", p.bark.indices.len() / 3, p.leaves.indices.len() / 3, p.bounds);
}
