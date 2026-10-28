//! text <DeusExText.u> <name>: print a text object raw; with "list <prefix>" list text names.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(&a[1]).unwrap();
    if a[2] == "list" {
        for e in &pkg.exports { if e.name.to_ascii_lowercase().starts_with(&a[3].to_ascii_lowercase()) { println!("{}", e.name); } }
        return;
    }
    println!("{}", pkg.text_named(&a[2]).unwrap_or_else(|| "(none)".into()));
}
