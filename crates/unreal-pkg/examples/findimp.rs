//! findimp <pkg> <needle>: imports/exports whose name contains needle, with their full paths.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(&a[1]).unwrap();
    // A number (e.g. -75) prints that object reference's path.
    if let Ok(i) = a[2].parse::<i32>() { println!("{i} -> {}", pkg.object_path(i)); return; }
    let n = a[2].to_ascii_lowercase();
    for i in 0..pkg.imports.len() {
        let r = -(i as i32) - 1;
        let p = pkg.object_path(r);
        if p.to_ascii_lowercase().contains(&n) { println!("import {p}"); }
    }
    for (i, e) in pkg.exports.iter().enumerate() {
        if e.name.to_ascii_lowercase().contains(&n) { println!("export {} ({})", pkg.object_path(i as i32 + 1), pkg.class_name(e.class_index)); }
    }
}
