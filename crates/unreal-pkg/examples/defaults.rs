//! Print a class's effective default properties: defaults <System dir> <Class>...
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut db = unreal_pkg::classes::ClassDb::open(std::path::Path::new(&a[1]));
    for c in &a[2..] {
        println!("== {c}");
        for p in db.defaults(c) {
            println!("  {}[{}] = {:?}", p.name, p.index, p.value);
        }
    }
}
