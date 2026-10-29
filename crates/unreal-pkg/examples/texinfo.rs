//! texinfo <pkg>: every texture's class, format, size, mip count, bHasComp.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pkg = unreal_pkg::Package::open(std::path::Path::new(&a[1])).unwrap();
    let mut hist = std::collections::BTreeMap::new();
    for e in &pkg.exports {
        let c = pkg.class_name(e.class_index).to_string();
        if !c.ends_with("Texture") { continue; }
        match unreal_pkg::objects::read_texture(&pkg, e) {
            Ok(t) => {
                *hist.entry((t.format, t.class.clone())).or_insert(0) += 1;
                if a.len() > 2 { println!("{} {} fmt {} {}x{} mips {}", t.class, pkg.object_path((pkg.exports.iter().position(|x| std::ptr::eq(x, e)).unwrap() + 1) as i32), t.format, t.width(), t.height(), t.mips.len()); if a.len()>3 { for p in &t.props { println!("   {} = {:?}", p.name, p.value); } } }
            }
            Err(err) => println!("ERR {} {:?}", e.name, err),
        }
    }
    println!("{hist:?}");
}
