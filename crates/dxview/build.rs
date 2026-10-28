//! Windows: embed the DX RUST icon (assets/dxrust.ico, resource id 1) in the exe. A missing resource compiler only
//! warns, so the build never fails over the icon.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/dxrust.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("../../assets/dxrust.ico", "1");
    if let Err(e) = res.compile() {
        println!("cargo:warning=icon not embedded: {e}");
    }
}
