//! refs <pkg> <b0,b1> ...: decode compact-index object references (from struct bytes) into paths.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(&a[1]).unwrap();
    for s in &a[2..] {
        let b: Vec<u8> = s.split(',').map(|x| x.parse().unwrap()).collect();
        let mut r = unreal_pkg::Reader::new(&b, 0);
        let v = r.compact_index().unwrap();
        println!("{s} -> {v} {}", pkg.object_path(v));
    }
}
