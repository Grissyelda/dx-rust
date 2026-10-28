//! cidx <pkg> <b0,b1,...> <kinds>: decode a struct's leading compact indices; kinds is a string of n (name) / o (object).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    let b: Vec<u8> = a[2].split(',').map(|x| x.parse().unwrap()).collect();
    let mut r = unreal_pkg::Reader::new(&b, 0);
    let mut out = Vec::new();
    for k in a[3].chars() {
        let v = r.compact_index().unwrap();
        out.push(match k {
            'n' => pkg.names.get(v as usize).cloned().unwrap_or_default(),
            _ => pkg.object_path(v),
        });
    }
    println!("{}", out.join(" | "));
}
