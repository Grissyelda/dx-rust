//! moddefaults <System dir> <Pkg=path,...> <Class>...: class defaults with some code packages read from other files.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let repl: Vec<(String, std::path::PathBuf)> = a[2]
        .split(',')
        .filter_map(|s| s.split_once('=').map(|(n, p)| (n.to_string(), std::path::PathBuf::from(p))))
        .collect();
    let r: Vec<(&str, std::path::PathBuf)> = repl.iter().map(|(n, p)| (n.as_str(), p.clone())).collect();
    let mut db = unreal_pkg::classes::ClassDb::open_replacing(std::path::Path::new(&a[1]), &r);
    for c in &a[3..] {
        println!("== {c}");
        for p in db.defaults(c) {
            println!("  {}[{}] = {:?}", p.name, p.index, p.value);
        }
    }
}
