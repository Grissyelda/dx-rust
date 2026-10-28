#[test]
fn anim_sizes() {
    let Ok(root) = std::env::var("DX_GAME") else { return };
    let lvl = dx_level::load_level(std::path::Path::new(&root), "01_NYC_UNATCOIsland.dx").expect("level");
    let mut seen = std::collections::HashSet::new();
    let (mut total, mut n) = (0usize, 0);
    for ma in &lvl.mesh_actors {
        if let Some(a) = &ma.anim {
            let p = std::sync::Arc::as_ptr(&a.mesh) as usize;
            if seen.insert(p) {
                n += 1;
                total += a.mesh.verts.len();
                eprintln!("anim mesh: {} verts x {} frames", a.mesh.frame_verts, a.mesh.frames);
            }
        }
    }
    eprintln!("{n} anim meshes, {total} vertex-frames = {:.1} MB at 8 B", total as f64 * 8.0 / 1e6);
}
