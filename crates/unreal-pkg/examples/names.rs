//! names <pkg> <index>...: print name table entries.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    for i in &a[2..] {
        let i: usize = i.parse().unwrap();
        println!("{i}: {:?}", pkg.names.get(i));
    }
}
