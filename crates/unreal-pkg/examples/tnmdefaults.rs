//! tnmdefaults <Deus Ex folder> <Nameless Mod folder> <Class>...: class defaults as The Nameless Mod sees them (its
//! TNM2 and root folders laid over the game, its code packages loaded).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (game, tnm) = (std::path::PathBuf::from(&a[1]), std::path::PathBuf::from(&a[2]));
    unreal_pkg::paths::set(&game, vec![tnm.join("TNM2"), tnm.clone()]);
    unreal_pkg::classes::set_extra_packages(unreal_pkg::tnm::PACKAGES);
    let mut db = unreal_pkg::classes::ClassDb::open(&game.join("System"));
    for c in &a[3..] {
        println!("== {c} ({:?}) {:?}", db.package_of(c), db.ancestry(c));
        for p in db.defaults(c) {
            println!("  {}[{}] = {:?}", p.name, p.index, p.value);
        }
    }
}
