//! expclass <pkg> [substring]: every export's class and name (optionally filtered).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    let want = a.get(2).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    for e in &pkg.exports {
        let path = pkg.object_path(pkg.exports.iter().position(|x| std::ptr::eq(x, e)).unwrap() as i32 + 1);
        if path.to_ascii_lowercase().contains(&want) {
            println!("{} {}", pkg.class_name(e.class_index), path);
        }
    }
}
