//! The Nameless Mod's cheat menu commands (TNMMenuCheats runs each row's console command; Trestkon's and
//! DeusExPlayer's exec functions do the work). The rows, their descriptions and which variable shows a row's state
//! come from the player's TNMGUI.u; the item lists, skill levels, account numbers and texture names a command uses
//! are read from the commands' own script text in the player's TNM.u. Only the rules are written here.
//!
//! Not in this port (they open developer windows of the original): ISeeIn2D's 2D test terminal, Phasrox's summon
//! window, EditFlags, InvokeConWindow, LoadMap, Legend's map list, ShowHoverbot's actor display, and the mesh swaps
//! of IAmJonas / IAmBogie. RearViewMirror's extra view and Screenshots' camera flag are kept as switches only.
// uc: TNMMenuCheats.BuildItemString, TNMMenuCheats.SwitchVariable

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::weapons::RuntimeAssets;

macro_rules! flag {
    ($name:ident) => {
        static $name: AtomicBool = AtomicBool::new(false);
    };
}
// Trestkon's cheat variables (and PlayerPawn's), as TNMMenuCheats reads them.
flag!(SCREENSHOTS); // bbeeblequixview
flag!(INSTAGIB_FISTS); // binstagibfists
flag!(REAR_VIEW); // bRearView
flag!(AUSTRALIAN); // bAustralian
flag!(DESPOT); // bdespot
flag!(PLAYERS_ONLY); // Level.bPlayersOnly
flag!(BRUH); // bruhSound
flag!(MINIATURE); // MiniatureWorld ran (drawscale x 0.1, once a map)

/// Fly / Ghost / Walk: the noclip state wanted (lib.rs toggle_noclip takes it), and the state shown.
static NOCLIP_REQ: Mutex<Option<bool>> = Mutex::new(None);
flag!(FLYING);

/// Requests that need the world (the level's actors): taken by `cheat_system`.
static WORLD: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub fn instagib_fists() -> bool {
    INSTAGIB_FISTS.load(Ordering::Relaxed)
}
pub fn australian() -> bool {
    AUSTRALIAN.load(Ordering::Relaxed)
}
/// IAmDespot: Trestkon.CalculatePlayerVisibility returns 0 (nobody sees the player).
pub fn despot() -> bool {
    DESPOT.load(Ordering::Relaxed)
}
/// PlayersOnly: everything but the player stands still.
pub fn players_only() -> bool {
    PLAYERS_ONLY.load(Ordering::Relaxed)
}
/// LookAtThisDood: pawns' pain and footsteps and weapons' impacts sound TNM.BruhSound.
pub fn bruh() -> bool {
    BRUH.load(Ordering::Relaxed)
}
pub fn take_noclip() -> Option<bool> {
    NOCLIP_REQ.lock().ok().and_then(|mut g| g.take())
}
pub fn note_noclip(on: bool) {
    FLYING.store(on, Ordering::Relaxed);
}

/// A new map: Level.bPlayersOnly and the shrunk world belong to the level.
pub fn reset_level() {
    PLAYERS_ONLY.store(false, Ordering::Relaxed);
    MINIATURE.store(false, Ordering::Relaxed);
    if let Ok(mut q) = WORLD.lock() {
        q.clear();
    }
}

/// The value a row shows (None = "N/A"): `var` the variable it names, `invert` its varInvert, `custom` its
/// varCustomCondition (rows 31 PlayersOnly, 32 Fly, 33 Ghost, 35 God, 45 Walk).
pub fn value(row: usize, var: &str, invert: bool, custom: bool) -> Option<bool> {
    if custom {
        let flying = FLYING.load(Ordering::Relaxed);
        return Some(match row {
            31 => players_only(),
            // Our noclip has no collision either way: Fly and Ghost show the same state.
            32 | 33 => flying,
            35 => crate::ai::GOD_MODE.load(Ordering::Relaxed),
            45 => !flying,
            _ => false,
        });
    }
    if var.is_empty() {
        return None;
    }
    let v = match var.to_ascii_lowercase().as_str() {
        // PlayerPawn.bDetectable (the Invisible row inverts it).
        "bdetectable" => !crate::abilities::CHEAT_INVISIBLE.load(Ordering::Relaxed),
        "bbeeblequixview" => SCREENSHOTS.load(Ordering::Relaxed),
        "binstagibfists" => instagib_fists(),
        "brearview" => REAR_VIEW.load(Ordering::Relaxed),
        "baustralian" => australian(),
        "bdespot" => despot(),
        "bbehindview" => crate::thirdperson::active(),
        _ => false,
    };
    Some(if invert { !v } else { v })
}

/// The text of one of the player's Trestkon functions (exec or not), from its name to the next function.
fn body(assets: &mut RuntimeAssets, func: &str) -> String {
    let Some(src) = assets.classes.script_text("Trestkon") else { return String::new() };
    let lower = src.to_ascii_lowercase();
    let want = format!("function {}(", func.to_ascii_lowercase());
    let Some(at) = lower.find(&want) else { return String::new() };
    let rest = &src[at + want.len()..];
    let end = rest.to_ascii_lowercase().find("\nfunction ").or_else(|| rest.to_ascii_lowercase().find("\nexec function ")).unwrap_or(rest.len());
    rest[..end].to_string()
}

/// The string literals passed to `call(` in a function's text, in order.
fn literals(text: &str, call: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let pat = format!("{}(", call.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find(&pat) {
        let s = from + i + pat.len();
        let rest = &text[s..];
        if let Some(q) = rest.find('"') {
            let close = rest[..rest.find(')').unwrap_or(rest.len())].len();
            if q < close {
                if let Some(e) = rest[q + 1..].find('"') {
                    out.push(rest[q + 1..q + 1 + e].to_string());
                }
            }
        }
        from = s;
    }
    out
}

/// Run a row's console command. `state` is the row's value before (for the commands that take the opposite).
pub fn run(assets: &mut RuntimeAssets, cmd: &str, state: Option<bool>) {
    use crate::missions::{player_req, PlayerReq};
    crate::rlach::casual("cheats");
    let flip = |f: &AtomicBool| {
        f.fetch_xor(true, Ordering::Relaxed);
    };
    match cmd.to_ascii_lowercase().as_str() {
        // PlayerPawn.Invisible(B): the row passes !bDetectable's opposite.
        "invisible" => {
            let on = !state.unwrap_or(false);
            crate::abilities::CHEAT_INVISIBLE.store(on, Ordering::Relaxed);
        }
        "screenshots" => flip(&SCREENSHOTS),
        "brucelee" => flip(&INSTAGIB_FISTS),
        "rearviewmirror" => flip(&REAR_VIEW),
        "australianoutback" => flip(&AUSTRALIAN),
        "iamdespot" => DESPOT.store(true, Ordering::Relaxed),
        "playersonly" => flip(&PLAYERS_ONLY),
        "lookatthisdood" => BRUH.store(true, Ordering::Relaxed),
        "god" | "iamgwog" => {
            crate::ai::GOD_MODE.fetch_xor(true, Ordering::Relaxed);
        }
        "fly" | "ghost" | "iamspyguy" => {
            // In flight already: the menu sends "Walk".
            let on = !(FLYING.load(Ordering::Relaxed) && cmd.eq_ignore_ascii_case("fly") | cmd.eq_ignore_ascii_case("ghost"));
            if let Ok(mut g) = NOCLIP_REQ.lock() {
                *g = Some(on);
            }
        }
        "walk" => {
            if let Ok(mut g) = NOCLIP_REQ.lock() {
                *g = Some(false);
            }
        }
        "behindview" => crate::thirdperson::cheat_toggle(),
        "iamscarab" | "allcredits" => player_req(PlayerReq::Cheat("AllCredits")),
        "allhealth" => player_req(PlayerReq::Cheat("AllHealth")),
        "allenergy" => player_req(PlayerReq::Cheat("AllEnergy")),
        "allskills" => player_req(PlayerReq::Cheat("AllSkills")),
        "allweapons" => player_req(PlayerReq::Cheat("AllWeapons")),
        "allammo" => player_req(PlayerReq::Cheat("AllAmmo")),
        "iamwarren" => crate::eggs::toggle_warren(),
        "showquoteswindow" => crate::credits::show_quotes(),
        "playmusicwindow" => crate::jukebox::open(false),
        "resetatms" => crate::computers::tnm_accounts_cheat_reset(assets),
        // TheVoicesAreTellingMeThings: HealthHead = 0, TakeDamage(0, 'Insane').
        "thevoicesaretellingmethings" => player_req(PlayerReq::Kill("Insane")),
        // ChemMan: CatchFire(Self), and its message.
        "chemman" => {
            player_req(PlayerReq::CatchFire);
            for m in literals(&body(assets, "chemman"), "ClientMessage") {
                crate::classic_hud::client_message(&m);
            }
        }
        // IAmTrestkon: its line (a FadeTextWindow in the original; here the message log).
        "iamtrestkon" => {
            if let Some(t) = literals(&body(assets, "iamtrestkon"), "SetText").into_iter().next() {
                crate::classic_hud::client_message(&t);
            }
        }
        // EquipMe: the give() list of the function, then its skill levels.
        "equipme" => {
            let b = body(assets, "equipme");
            player_req(PlayerReq::Give(literals(&b, "give")));
            let mut skills = Vec::new();
            let mut last: Option<String> = None;
            for stmt in b.split(';') {
                let st = stmt.trim();
                if let Some(i) = st.find("class'") {
                    let rest = &st[i + 6..];
                    last = rest.find('\'').map(|e| rest[..e].to_string());
                } else if let Some(i) = st.to_ascii_lowercase().find("currentlevel") {
                    if let (Some(cl), Some(v)) = (last.clone(), st[i..].split('=').nth(1)) {
                        if let Ok(n) = v.trim().parse::<u8>() {
                            skills.push((cl, n));
                        }
                    }
                }
            }
            player_req(PlayerReq::Skills(skills));
        }
        // StudiedTheBlade(""): a katana.
        "studiedtheblade" => player_req(PlayerReq::Give(vec!["WeaponKatana".into()])),
        "allkeys" | "allpasswords" | "allitems" | "allimages" | "hateme" | "loveme" | "massban" | "trestwoke" | "installwindows" | "miniatureworld" | "bang"
        | "burnbabyburn" | "tantalus" | "opensesame" | "fonkey" => {
            let c: &'static str = match cmd.to_ascii_lowercase().as_str() {
                "allkeys" | "allpasswords" => "allkeys",
                "allitems" => "allitems",
                "allimages" => "allimages",
                "hateme" => "hateme",
                "loveme" => "loveme",
                "massban" => "massban",
                "trestwoke" => "trestwoke",
                "installwindows" => "installwindows",
                "miniatureworld" => "miniatureworld",
                "bang" => "bang",
                "burnbabyburn" => "burnbabyburn",
                "tantalus" => "tantalus",
                "opensesame" => "opensesame",
                _ => "fonkey",
            };
            if let Ok(mut q) = WORLD.lock() {
                q.push(c);
            }
        }
        other => crate::classic_hud::client_message(&format!("{other}: not in this port")),
    }
}

/// The commands that work on the level's actors.
#[allow(clippy::too_many_arguments)]
pub fn cheat_system(
    level: bevy::prelude::Res<crate::LevelRes>,
    player: bevy::prelude::Res<crate::Player>,
    mut assets: bevy::prelude::ResMut<RuntimeAssets>,
    mut hits: bevy::prelude::ResMut<crate::damage::PawnHits>,
    mut throws: bevy::prelude::ResMut<crate::destruct::ItemThrows>,
    highlight: bevy::prelude::Res<crate::frobui::FrobHighlight>,
    cam: bevy::prelude::Query<&crate::FlyCam>,
    mut roots: bevy::prelude::Query<&mut bevy::prelude::Transform, bevy::prelude::Or<(bevy::prelude::With<crate::actors::Deco>, bevy::prelude::With<crate::actors::Animated>)>>,
) {
    use crate::ai::{world_req, Sel, WorldReq};
    let reqs: Vec<&'static str> = match WORLD.lock() {
        Ok(mut q) if !q.is_empty() => std::mem::take(&mut *q),
        _ => return,
    };
    let lvl = &level.0;
    let at = player.body.location;
    for r in reqs {
        match r {
            // AllKeys: every key a door asks for (KeyIDNeeded), named by its id.
            "allkeys" => {
                for a in &lvl.actors {
                    if let Some(unreal_pkg::props::Value::Name(k)) = unreal_pkg::props::find(&a.props, "KeyIDNeeded") {
                        let k = k.clone();
                        if !k.is_empty() && !k.eq_ignore_ascii_case("None") {
                            crate::keys::give(&k, &k);
                        }
                    }
                }
            }
            // AllItems: one of each class the function spawns, two collision radii in front of the player.
            "allitems" => {
                for path in literals(&body(&mut assets, "allitems"), "DynamicLoadObject") {
                    let class = path.rsplit('.').next().unwrap_or(&path).to_string();
                    throws.0.push((class, at + bevy::math::DVec3::Z * 10.0, bevy::math::DVec3::ZERO, None, Vec::new()));
                }
            }
            // AllImages: each DataVault image the function spawns and frobs goes in the DataVault.
            "allimages" => {
                for path in literals(&body(&mut assets, "allimages"), "DynamicLoadObject") {
                    crate::consys::add_image(path.rsplit('.').next().unwrap_or(&path));
                }
            }
            // HateMe / LoveMe with no tag: every pawn, ChangeAlly('Player', -1 / 1, true).
            "hateme" | "loveme" => crate::triggers::push_order(crate::triggers::PawnOrder::Ally {
                sel: Sel::Pick { class: None, tag: None, bind: None },
                alliance: "Player".into(),
                level: if r == "hateme" { -1.0 } else { 1.0 },
                permanent: true,
            }),
            // MassBan: every TNMPawn and TNMRobot, HealthTorso = 0, TakeDamage(1, 'Shot').
            "massban" => {
                world_req(WorldReq::KillShot(Sel::isa("TNMPawn")));
                world_req(WorldReq::KillShot(Sel::isa("TNMRobot")));
            }
            // TrestWoke: every pawn and robot a firewall (bFirewall).
            "trestwoke" => crate::tnmpawn::set_all_firewall(),
            // InstallWindows: each TNMMonitor_FS's screen (slot 2) and each cash register's (3, 4) one of the two
            // pictures the function names, at random.
            "installwindows" => {
                let pics: Vec<String> = literals(&body(&mut assets, "installwindows"), "DynamicLoadObject");
                if pics.is_empty() {
                    continue;
                }
                let mut seed = (at.x.to_bits() ^ at.y.to_bits()) | 1;
                for (i, a) in lvl.actors.iter().enumerate() {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let p = pics[(seed % pics.len() as u64) as usize].clone();
                    if crate::classtree::is_a(&a.class, "TNMMonitor_FS") {
                        crate::reskin::request_actor(i, vec![(2, p)]);
                    } else if crate::classtree::is_a(&a.class, "CashRegister") {
                        crate::reskin::request_actor(i, vec![(3, p.clone()), (4, p)]);
                    }
                }
            }
            // MiniatureWorld: every actor drawn a tenth its size (collision unchanged, as DrawScale leaves it).
            "miniatureworld" => {
                for mut t in &mut roots {
                    t.scale *= 0.1;
                }
                MINIATURE.store(true, Ordering::Relaxed);
            }
            // Bang: an ExplosionLight (size 8, hue 128) and a SphereEffect of size 24 at the player.
            "bang" => {
                crate::missions::fx(crate::missions::Fx::Sphere(at, 768.0 / 32.0));
                crate::dynlight::explosion(at, 8.0, 128, 96);
            }
            // BurnBabyBurn: a TNMFire on what the player looks at (a pawn catches fire); Tantalus: it dies.
            "burnbabyburn" | "tantalus" => {
                if let Some(b) = crate::hud::aimed_pawn() {
                    let (damage, damage_type) = if r == "tantalus" { (100000.0, "Shot") } else { (1.0, "Flamed") };
                    hits.0.push(crate::damage::PawnHit { blocker: b, damage, damage_type, hit_loc: at, attacker: None, attacker_pos: at, world: false });
                }
            }
            // OpenSesame: the door the player looks at opens, locked or not.
            "opensesame" => {
                if let Some(i) = highlight.target.as_ref().and_then(|h| h.mover()) {
                    crate::movers::pawn_door(crate::movers::PawnDoor::Open(i));
                }
            }
            // Fonkey: a summoned ghand in front of the player (CollisionRadius + its own + 30 ahead, 15 up).
            "fonkey" => {
                // FlyCam.yaw = -(Unreal yaw) - 90 degrees.
                let uy = cam.iter().next().map_or(0.0, |c| (-(c.yaw.to_degrees() as f64) - 90.0).to_radians());
                let dir = bevy::math::DVec3::new(uy.cos(), uy.sin(), 0.0);
                let ahead = at + dir * (player.params.radius + 60.0) + bevy::math::DVec3::Z * 15.0;
                let yaw = (uy.to_degrees() / 360.0 * 65536.0) as i32;
                crate::missions::queue_pawn("ghandsummoned", ahead, yaw, Vec::new());
            }
            _ => {}
        }
    }
}
