#[test]
fn find_actor() {
    let Ok(root) = std::env::var("DX_GAME") else { return };
    let Ok(pat) = std::env::var("FIND") else { return };
    let lvl = dx_level::load_level(std::path::Path::new(&root), "01_NYC_UNATCOIsland.dx").expect("level");
    for a in &lvl.actors {
        if a.class.to_lowercase().contains(&pat.to_lowercase()) {
            let l = a.location();
            eprintln!("{} {} UE {:?} bevy {:?}", a.class, a.name, l, dx_level::ue_to_bevy(l));
        }
    }
    if let Some(p) = &lvl.player_start { eprintln!("player start {:?}", p.position); }
}
