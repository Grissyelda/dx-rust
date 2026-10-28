//! meshtex <pkg> <mesh>: a mesh's textures and section flags.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    let e = pkg.exports.iter().find(|e| e.name.eq_ignore_ascii_case(&a[2])).unwrap();
    let m = unreal_pkg::mesh::read_lod_mesh(&pkg, e).unwrap();
    for (i, t) in m.textures.iter().enumerate() {
        println!("tex {i}: {} ({})", pkg.object_path(*t), if *t != 0 { pkg.class_name(if *t > 0 { pkg.exports[*t as usize - 1].class_index } else { 0 }).to_string() } else { "-".into() });
    }
    for mt in &m.materials { println!("mat flags {:#x} tex {}", mt.poly_flags, mt.texture_index); }
}
