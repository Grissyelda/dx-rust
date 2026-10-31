//! Achievements: the fifty of Deus Ex Remastered (names and goals as that release lists them), our own icons, a
//! persona screen (from the Logs screen's Achievements button) and an unlock notice in the HUD's manner.
//!
//! Unlocks are kept for the player, not per save (Save/achievements.txt beside the saves: `id=unix time` lines and
//! `progress.id=n` for the counted ones). Each one is detected from the same things the game itself tracks:
//! mission arrivals, conversation flags and choices, events (MadeBasket, SecretDoor, alexcloset), takedowns,
//! augmentations, skills, weapon mods, and a few watched situations (sneaking past guards, a cloaked walk through
//! someone's view, a body near a Karkian, the Underworld's pool table).

use crate::ai::{Npc, NpcState, Target};
use crate::uicanvas::{Canvas, HAlign, Style, UiTex, VAlign};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// (id, name, goal).
pub const LIST: [(&str, &str, &str); 50] = [
    ("majestic", "Majestic Revelations", "Unlock every other achievement."),
    ("lady", "Rescue the Lady", "Stop the NSF terrorists on Liberty Island."),
    ("town", "On the Town", "Free the hostages, discover the NSF generator, and take down the NSF's EMP field."),
    ("lebedev", "The Lebedev Sanction", "Locate the airfield where the NSF moved the Ambrosia, and hear some shocking details from Paul."),
    ("choices", "Inevitable Choices", "Question your alliances."),
    ("breakout", "Breakout", "Escape the Majestic 12 facility."),
    ("hongkong", "Hong Kong", "Find Tracer Tong, obtain the ROM encoding for the Dragon's Tooth, and ally the Triads."),
    ("tanker1", "The Supertanker Part 1", "Meet Stanton Dowd and discuss the supertanker."),
    ("tanker2", "The Supertanker Part 2", "Scuttle the supertanker and disable the EM disruption field before escaping the shipyard."),
    ("paree", "Grey Paree", "Meet with Nicolette DuClare and make contact with the Illuminati."),
    ("templars", "Treasures of the Templars", "Access the computer so that Morgan Everett can complete his work on the Gray Death cure."),
    ("vandenberg", "Vandenberg AFB", "Uplink Daedalus and gain Morgan Everett access to the Universal Constructor."),
    ("sea", "By the Sea", "Steal a submarine and retrieve the schematic for the Universal Constructor."),
    ("area51", "Area 51", "Decide the fate of mankind."),
    ("helios", "Second Coming", "Complete the game by merging with Helios."),
    ("darkage", "Cut the Cord", "Complete the game by enacting a new Dark Age."),
    ("illuminati", "Shaking the Invisible Hand", "Complete the game by joining the Illuminati."),
    ("biomachina", "Biomachina", "Install at least one augmentation into each body part."),
    ("slugger", "Slugger", "Take down 3 enemies with a melee attack while the Combat Strength augmentation is active."),
    ("sneak", "Solid Sneak", "Sneak past 3 enemies with the Run Silent augmentation active."),
    ("section9", "Section 9 Methods", "Travel through a humanoid enemy's line of sight while the Cloak augmentation is active."),
    ("automoto", "Automotonophobia", "Travel through the line of sight of a machine with the Radar Transparency augmentation active."),
    ("wardenclyffe", "Wardenclyffe Man", "Activate 3 other augmentations while the Power Recirculator augmentation is active."),
    ("atomheart", "Atom Heart Son", "Increase the level of 3 other augmentations by using the Synthetic Heart augmentation."),
    ("bestoffense", "The Best Offense Is...", "Kill an enemy with their own projectile via the Aggressive Defense System."),
    ("specialeyes", "Special Eyes", "Alert an enemy seen through a surface with the Vision Enhancement augmentation active."),
    ("servant", "Public Servant", "Grant the sick bum's request in Battery Park."),
    ("dunkin", "JC Dunkin'", "Score a basket in Hell's Kitchen."),
    ("escapa", "El Escapa", "Successfully break Miguel out of the MJ12 detention facility."),
    ("sawnothing", "You Saw Nothing", "Listen to Walton Simons' interrogation of the NSF soldiers."),
    ("imin", "I'm In", "Enter the secret compartment in Alex Jacobson's office."),
    ("pinball", "Pinball Wizard", "Reveal the secret room in the NSF Helibase."),
    ("wildlife", "Don't Feed the Wildlife", "Feed a body to a Karkian or Greasel."),
    ("dibbuk", "Dibbuk Defusal", "Something's odd about that odd mechanic. Maybe Everett would know about him."),
    ("sure", "Sure", "Pay Mercedes and Tessa's entry fee."),
    ("renton", "Renton, Far From Home", "Save Sandra from JoJo, mourn Mr. Renton's untimely death, and meet Sandra in California."),
    ("hero", "American Hero", "Despite everything, keep your allies and friends alive."),
    ("tail", "Lose the Tail", "If you mention Stanton Dowd to Joe Greene... watch your back."),
    ("meanjoe", "No Thanks, Mean Joe", "Ah, yes. A journalist. You can trust him..."),
    ("based", "Based", "Talk with Leo Gold further after detaining him."),
    ("nicegun", "Nice Gun, Mind If I Have It?", "Obtain an assault rifle on your first mission on Liberty Island."),
    ("nerves", "Nerves of Steel", "Stop Anna Navarre from killing Lebedev."),
    ("tonjc", "Thanks a 'Ton, JC", "Don't be afraid, I'm with you, Paul."),
    ("howdid", "How Did You...", "Two passwords for your life."),
    ("notmachine", "I Am Not a Machi...", "Having a man on the inside at UNATCO pays off."),
    ("shame", "What a Shame", "Comment on Mr. Renton's untimely passing."),
    ("pool", "Pool Shark", "Use the cue ball to sink all other balls on the Underworld's pool table."),
    ("veteran", "Veteran Agent", "Have two Master level skills at the same time."),
    ("lockload", "Lock and Load", "Install one of each compatible upgrade type onto a single weapon."),
    ("upgrades", "Upgrades, people. Upgrades", "Fully upgrade an augmentation."),
];

/// Counted achievements: (id, target).
const COUNTED: [(&str, u32); 4] = [("slugger", 3), ("sneak", 3), ("wardenclyffe", 3), ("atomheart", 3)];

#[derive(Default)]
struct Store {
    loaded: bool,
    unlocked: HashMap<String, u64>,
    progress: HashMap<String, u32>,
    /// Unlocked this session, waiting to be shown.
    popups: Vec<String>,
}

static STORE: std::sync::LazyLock<Mutex<Store>> = std::sync::LazyLock::new(|| Mutex::new(Store::default()));

fn path() -> std::path::PathBuf {
    crate::save::dir().join("achievements.txt")
}

fn load(s: &mut Store) {
    if s.loaded {
        return;
    }
    s.loaded = true;
    let Ok(text) = std::fs::read_to_string(path()) else { return };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v: u64 = v.trim().parse().unwrap_or(0);
        if let Some(id) = k.strip_prefix("progress.") {
            s.progress.insert(id.to_string(), v as u32);
        } else {
            s.unlocked.insert(k.trim().to_string(), v);
        }
    }
}

fn save(s: &Store) {
    let mut out = String::new();
    let mut ids: Vec<_> = s.unlocked.iter().collect();
    ids.sort();
    for (k, v) in ids {
        out.push_str(&format!("{k}={v}\n"));
    }
    let mut pr: Vec<_> = s.progress.iter().collect();
    pr.sort();
    for (k, v) in pr {
        out.push_str(&format!("progress.{k}={v}\n"));
    }
    let _ = std::fs::create_dir_all(crate::save::dir());
    let _ = std::fs::write(path(), out);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Unlocks an achievement (once).
pub fn unlock(id: &str) {
    let Ok(mut s) = STORE.lock() else { return };
    load(&mut s);
    if s.unlocked.contains_key(id) || !LIST.iter().any(|a| a.0 == id) {
        return;
    }
    s.unlocked.insert(id.to_string(), now_secs());
    s.popups.push(id.to_string());
    println!("ACHIEVEMENT {id}");
    // Majestic Revelations: everything else.
    if id != "majestic" && LIST.iter().filter(|a| a.0 != "majestic").all(|a| s.unlocked.contains_key(a.0)) {
        s.unlocked.insert("majestic".into(), now_secs());
        s.popups.push("majestic".into());
    }
    save(&s);
}

/// Adds to a counted achievement's progress; unlocks it at the target.
pub fn add_progress(id: &str, n: u32) {
    let target = COUNTED.iter().find(|c| c.0 == id).map_or(1, |c| c.1);
    let reached = {
        let Ok(mut s) = STORE.lock() else { return };
        load(&mut s);
        if s.unlocked.contains_key(id) {
            return;
        }
        let p = s.progress.entry(id.to_string()).or_insert(0);
        *p = (*p + n).min(target);
        let r = *p >= target;
        save(&s);
        r
    };
    if reached {
        unlock(id);
    }
}

/// (unlocked unix time, progress) for the persona screen.
pub fn state(id: &str) -> (Option<u64>, u32) {
    let Ok(mut s) = STORE.lock() else { return (None, 0) };
    load(&mut s);
    (s.unlocked.get(id).copied(), s.progress.get(id).copied().unwrap_or(0))
}

pub fn target(id: &str) -> u32 {
    COUNTED.iter().find(|c| c.0 == id).map_or(0, |c| c.1)
}

pub fn unlocked_count() -> usize {
    let Ok(mut s) = STORE.lock() else { return 0 };
    load(&mut s);
    s.unlocked.len()
}

// ------------------------------------------------------------------------------------------------ hooks

/// An event went out (devices.rs dispatch).
pub fn event(map: &str, name: &str) {
    let map = map.to_ascii_lowercase();
    match name.to_ascii_lowercase().as_str() {
        "madebasket" => unlock("dunkin"),
        "secretdoor" if map.contains("airfieldhelibase") => unlock("pinball"),
        "alexcloset" => unlock("imin"),
        _ => {}
    }
}

/// A conversation choice was made (consys.rs).
pub fn choice(con: &str, text: &str) {
    if con.eq_ignore_ascii_case("ClubMercedesConvo1") && text.trim().eq_ignore_ascii_case("Sure.") {
        unlock("sure");
    }
    if con.eq_ignore_ascii_case("M11MeetGunther") && text.to_ascii_lowercase().contains("laputan machine") {
        unlock("notmachine");
    }
}

/// A pawn was taken down (ai.rs).
pub fn takedown(by_player: bool, class: &str, bind: &str, blocker: usize, explosive: bool) {
    if !by_player {
        // Aggressive Defense: a pawn killed by the blast of its own projectile, which the ADS set off.
        if explosive && ads_owner_recent(blocker) {
            unlock("bestoffense");
        }
        return;
    }
    if explosive && ads_owner_recent(blocker) {
        unlock("bestoffense");
    }
    if bind.eq_ignore_ascii_case("SickMan") {
        unlock("servant");
    }
    let _ = class;
    if crate::perks::in_hand_melee() && crate::abilities::aug_active("AugCombat") {
        add_progress("slugger", 1);
    }
}

/// Aggressive Defense set off a projectile fired by this pawn (blocker).
static ADS_OWNERS: Mutex<Vec<(usize, std::time::Instant)>> = Mutex::new(Vec::new());

pub fn ads_detonated(owner: usize) {
    if let Ok(mut v) = ADS_OWNERS.lock() {
        v.retain(|(_, t)| t.elapsed().as_secs_f32() < 2.0);
        v.push((owner, std::time::Instant::now()));
    }
}

fn ads_owner_recent(blocker: usize) -> bool {
    ADS_OWNERS.lock().ok().is_some_and(|v| v.iter().any(|(b, t)| *b == blocker && t.elapsed().as_secs_f32() < 2.0))
}

/// A weapon mod went on (weapons::fit_mod): every upgrade the weapon can take is on it.
pub fn mods_fitted(w: &crate::weapons::WeaponStats, m: &crate::weapons::ModState) {
    let c = &w.can_mod;
    let ok = (!c[0] || m.accuracy > 0.0)
        && (!c[1] || m.clip > 0.0)
        && (!c[2] || m.range > 0.0)
        && (!c[3] || m.reload < 0.0)
        && (!c[4] || m.recoil < 0.0)
        && (!c[5] || w.has_scope || m.scope)
        && (!c[6] || w.silenced || m.silencer)
        && (!c[7] || w.has_laser || m.laser);
    if ok && c.iter().any(|x| *x) {
        unlock("lockload");
    }
}

/// While the cloak / radar transparency hides the player from a pawn that's looking right at him (ai.rs).
pub fn hidden_in_view(robot: bool) {
    if robot {
        if crate::abilities::aug_active("AugRadarTrans") {
            unlock("automoto");
        }
    } else if crate::abilities::aug_active("AugCloak") {
        unlock("section9");
    }
}

// ------------------------------------------------------------------------------------------------ the watcher

#[derive(Default)]
pub struct Watch {
    gen: u64,
    mission: i32,
    map: String,
    tick: f32,
    /// Augs active last tick, and the count activated during this Power Recirculator run.
    active: HashSet<String>,
    power_count: u32,
    /// Pawns already counted as snuck past.
    passed: HashSet<Entity>,
    /// States last tick (Special Eyes: a pawn going on alert).
    states: HashMap<Entity, bool>,
    /// Bodies the player has carried (Don't Feed the Wildlife).
    carried: HashSet<Entity>,
    /// The pool balls' starting heights (by mesh actor), cue ball apart.
    pool: Option<Vec<(usize, f64, bool)>>,
    popup: Option<(String, f32)>,
    layer: Option<crate::menuui::Layer>,
}

fn mission_of(lvl: &dx_level::Level) -> i32 {
    lvl.actors
        .iter()
        .find(|a| a.class == "DeusExLevelInfo")
        .and_then(|a| match unreal_pkg::props::find(&a.props, "MissionNumber") {
            Some(unreal_pkg::props::Value::Int(n)) => Some(*n),
            Some(unreal_pkg::props::Value::Byte(n)) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn achievements_system(
    mut commands: Commands,
    time: Res<Time<Real>>,
    level: Res<LevelRes>,
    flags: Res<crate::ai::GameFlags>,
    player: Res<crate::Player>,
    inv: Res<crate::pickups::PlayerInventory>,
    weapons: Res<crate::weapons::Weapons>,
    npcs: Query<(Entity, &Npc)>,
    blockers: Res<crate::actors::Blockers>,
    decos: Query<(&crate::actors::Deco, &crate::actors::MeshActorRef)>,
    window: Single<&Window>,
    mut st: Local<Watch>,
    (mut hud, mut assets, mut images, mut layers): (
        ResMut<crate::classic_hud::ClassicHud>,
        ResMut<crate::weapons::RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<crate::classic_hud::HudLayer>>,
    ),
    mut nodes: Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<crate::classic_hud::HudLayer>)>,
) {
    let _prof = crate::prof::scope("achievements::achievements_system");
    let lvl = &level.0;
    let dt = time.delta_secs().min(0.1);
    if crate::loader::new_map(&mut st.gen) {
        st.mission = mission_of(lvl);
        st.map = lvl.name.to_ascii_lowercase();
        st.passed.clear();
        st.states.clear();
        st.carried.clear();
        st.pool = None;
        on_arrival(st.mission, &st.map, &flags.0);
    }
    // ---- the unlock notice
    draw_popup(&mut commands, &mut st, dt, &window, &mut hud, &mut assets, &mut images, &mut layers, &mut nodes);
    if crate::menus::intro_map() || crate::credits::open() {
        return;
    }
    // Carried bodies (POVCorpse) are remembered for a while after they're dropped.
    if let Some(e) = weapons.corpse {
        st.carried.insert(e);
    }
    st.tick += dt;
    if st.tick < 0.25 {
        return;
    }
    let period = std::mem::take(&mut st.tick);
    let f = &flags.0;
    let fl = |n: &str| f.get_bool(n);
    // ---- flags
    if fl("TerroristFollowup2_Played") {
        unlock("based");
    }
    if fl("SimonsInterrogating3_Played") {
        unlock("sawnothing");
    }
    if fl("TalkAboutRay_Played") {
        unlock("dibbuk");
    }
    if fl("M12MeetSandraRenton_Played") {
        unlock("renton");
    }
    if fl("RentonLateComment_Played") {
        unlock("shame");
    }
    if fl("LebedevAnnaDead_Played") {
        unlock("nerves");
    }
    if fl("AnnaExitKillphrase_Played") {
        unlock("howdid");
    }
    if fl("GreenKnowsAboutDowd") && (fl("MS_ShadyGuyUnhidden") || fl("MS_UnhideCommandos")) {
        unlock("tail");
    }
    if fl("JoeGreen_Dead") {
        unlock("meanjoe");
    }
    if fl("Endgame1_Played") {
        unlock("darkage");
        unlock("area51");
    }
    if fl("Endgame2_Played") {
        unlock("helios");
        unlock("area51");
    }
    if fl("Endgame3_Played") {
        unlock("illuminati");
        unlock("area51");
    }
    if st.mission == 1 && inv.0.items.iter().any(|i| i.def.class == "WeaponAssaultGun") {
        unlock("nicegun");
    }
    // ---- skills and augmentations
    {
        let ab = crate::abilities::lock();
        if let Some(a) = ab.as_ref() {
            if a.skills.list.iter().filter(|s| s.level >= 3).count() >= 2 {
                unlock("veteran");
            }
            let mut locs = [false; 6];
            for g in a.augs.list.iter().filter(|g| g.has) {
                if (g.def.location as usize) < 6 {
                    locs[g.def.location as usize] = true;
                }
                if g.level >= g.def.max_level && !g.boosted && g.def.max_level > 0 {
                    unlock("upgrades");
                }
            }
            if locs.iter().all(|x| *x) {
                unlock("biomachina");
            }
            if a.augs.list.iter().filter(|g| g.boosted).count() >= 3 {
                add_progress("atomheart", 3);
            }
            let now: HashSet<String> = a.augs.list.iter().filter(|g| g.active).map(|g| g.def.class.clone()).collect();
            if now.contains("AugPower") {
                let newly = now.iter().filter(|c| c.as_str() != "AugPower" && !st.active.contains(*c)).count() as u32;
                // (The ones already on when it came on don't count; it's activations while it runs.)
                if st.active.contains("AugPower") {
                    st.power_count += newly;
                }
                if st.power_count >= 3 {
                    add_progress("wardenclyffe", 3);
                }
            } else {
                st.power_count = 0;
            }
            st.active = now;
        }
    }
    // ---- pawns: sneaking past, going on alert behind a wall, eating bodies
    let me = player.body.location;
    let eye = me + DVec3::new(0.0, 0.0, player.params.height * 0.8);
    let run_silent = crate::abilities::aug_active("AugStealth");
    let vision = crate::abilities::aug_active("AugVision") && crate::abilities::aug_level("AugVision").unwrap_or(0) >= 2;
    let animals: Vec<DVec3> = npcs
        .iter()
        .filter(|(_, n)| n.state != NpcState::Dead && ["Karkian", "Greasel"].iter().any(|c| crate::classtree::is_a(&lvl.actors[n.actor].class, c) || lvl.actors[n.actor].class.starts_with(c)))
        .filter_map(|(_, n)| blockers.list.get(n.blocker).map(|c| c.center))
        .collect();
    for (e, n) in &npcs {
        let alerted = matches!(n.state, NpcState::Seeking | NpcState::Attacking | NpcState::Surprised(_)) && (n.target == Some(Target::Player) || n.state == NpcState::Seeking);
        let was = st.states.insert(e, alerted).unwrap_or(alerted);
        if n.state == NpcState::Dead {
            // A body the player carried, dropped by a Karkian or Greasel.
            if st.carried.contains(&e) && weapons.corpse != Some(e) {
                if let Some(at) = n.body.as_ref().map(|b| b.location) {
                    if animals.iter().any(|a| a.distance(at) < 220.0) {
                        unlock("wildlife");
                    }
                }
            }
            continue;
        }
        let Some(c) = blockers.list.get(n.blocker).copied() else { continue };
        if c.radius <= 0.0 || !n.hates("Player") {
            continue;
        }
        if vision && alerted && !was && lvl.collision.trace(eye, c.center, DVec3::ZERO).fraction < 0.999 {
            unlock("specialeyes");
        }
        if run_silent && !st.passed.contains(&e) && n.state == NpcState::Idle && n.readiness < 0.3 && c.center.distance(me) < 220.0 {
            st.passed.insert(e);
            add_progress("sneak", 1);
        }
    }
    // ---- the Underworld's pool table (the NYC bar)
    if st.map.contains("nyc_bar") {
        if st.pool.is_none() {
            let mut v = Vec::new();
            for (i, ma) in lvl.mesh_actors.iter().enumerate() {
                let a = &lvl.actors[ma.actor];
                if a.class.eq_ignore_ascii_case("Poolball") {
                    let cue = matches!(unreal_pkg::props::find(&a.props, "SkinColor"), Some(unreal_pkg::props::Value::Byte(15)));
                    v.push((i, a.location()[2] as f64, cue));
                }
            }
            st.pool = Some(v);
        }
        let balls = st.pool.as_ref().unwrap();
        if balls.iter().filter(|b| !b.2).count() >= 2 {
            let sunk = |i: usize, z0: f64| decos.iter().find(|(_, r)| r.0 == i).is_none_or(|(d, _)| d.body.location.z < z0 - 12.0);
            let all = balls.iter().filter(|b| !b.2).all(|b| sunk(b.0, b.1));
            let cue_up = balls.iter().filter(|b| b.2).all(|b| !sunk(b.0, b.1));
            if all && cue_up {
                unlock("pool");
            }
        }
    }
    let _ = period;
}

/// Mission arrivals: the mission before this one is done; and the story checks made on the way in.
fn on_arrival(mission: i32, map: &str, f: &imsim_core::flags::FlagBase) {
    let done = [(2, "lady"), (3, "town"), (4, "lebedev"), (5, "choices"), (6, "breakout"), (8, "hongkong"), (9, "tanker1"), (10, "tanker2"), (11, "paree"), (12, "templars"), (14, "vandenberg"), (15, "sea")];
    for (m, id) in done {
        if mission >= m {
            unlock(id);
        }
    }
    if mission == 5 && !f.get_bool("PaulDenton_Dead") {
        unlock("tonjc");
    }
    if map.starts_with("05_nyc_unatcohq") && f.get_bool("MiguelFollowing") && !f.get_bool("Miguel_Dead") {
        unlock("escapa");
    }
    if mission >= 15 && !f.get_bool("PaulDenton_Dead") && !f.get_bool("DL_JockDeath_Played") && !f.get_bool("SandraRenton_Dead") && f.get_bool("MeetSandraRenton_Played") {
        unlock("hero");
    }
}

// ------------------------------------------------------------------------------------------------ the notice

#[allow(clippy::too_many_arguments)]
fn draw_popup(
    commands: &mut Commands,
    st: &mut Watch,
    dt: f32,
    window: &Window,
    hud: &mut crate::classic_hud::ClassicHud,
    assets: &mut crate::weapons::RuntimeAssets,
    images: &mut Assets<Image>,
    layers: &mut Assets<crate::classic_hud::HudLayer>,
    nodes: &mut Query<(&mut bevy::ui::Node, &mut Visibility, &MaterialNode<crate::classic_hud::HudLayer>)>,
) {
    if st.popup.is_none() {
        let next = STORE.lock().ok().and_then(|mut s| (!s.popups.is_empty()).then(|| s.popups.remove(0)));
        if let Some(id) = next {
            crate::sound::play_player("DeusExSounds.UserInterface.LogGoalAdded", 1.0, 0.0);
            st.popup = Some((id, 0.0));
        }
    }
    let layer = st.layer.get_or_insert_with(|| crate::menuui::layer(commands, images, layers, 30));
    let Some((id, t)) = st.popup.as_mut() else {
        if let Ok((_, mut v, _)) = nodes.get_mut(layer.e) {
            *v = Visibility::Hidden;
        }
        return;
    };
    *t += dt;
    const LIFE: f32 = 5.0;
    if *t > LIFE {
        st.popup = None;
        if let Ok((_, mut v, _)) = nodes.get_mut(layer.e) {
            *v = Visibility::Hidden;
        }
        return;
    }
    let s = crate::hud::ui_scale(window) as i32;
    let (w, h) = (280, 46);
    let mut c = Canvas::new(w, h, s.max(1));
    let Some((name, _)) = LIST.iter().find(|a| a.0 == id.as_str()).map(|a| (a.1, a.2)) else { return };
    let (Some(head), Some(small)) = (hud.font(assets, "FontMenuHeaders"), hud.font(assets, "FontMenuSmall")) else { return };
    let col = crate::menuui::h_col(9);
    frame_box(&mut c, 0, 0, w, h, col);
    let icon = icon(LIST.iter().position(|a| a.0 == id.as_str()).unwrap_or(0), true, col);
    c.texture(5, 5, 36, 36, 0, 0, &icon, Style::Masked, [255, 255, 255]);
    c.text(&small, 48, 6, w - 54, small.height, HAlign::Left, VAlign::Top, col, "ACHIEVEMENT UNLOCKED");
    c.text(&head, 48, 22, w - 54, head.height, HAlign::Left, VAlign::Top, [255, 255, 255], name);
    // Slides down from the top, back up at the end.
    let k = (*t / 0.3).min(1.0).min(((LIFE - *t) / 0.3).max(0.0));
    let sf = window.scale_factor();
    let d = s as f32;
    let pw = window.physical_width() as f32 / d;
    let y = -(h as f32) + (h as f32 + 12.0) * k * (2.0 - k);
    crate::menuui::place(layer, &c, (((pw - w as f32) / 2.0) as i32, y as i32), d, sf, images, layers, nodes);
}

/// A window frame in the HUD's manner: dark glass, a light bevel top-left, shadow bottom-right.
fn frame_box(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, col: [u8; 3]) {
    c.fill(x, y, w, h, Style::Normal, [14, 16, 19]);
    let lite = col.map(|v| (v as u16 * 3 / 4) as u8);
    let dark = col.map(|v| v / 4);
    c.fill(x, y, w, 1, Style::Normal, lite);
    c.fill(x, y, 1, h, Style::Normal, lite);
    c.fill(x, y + h - 1, w, 1, Style::Normal, dark);
    c.fill(x + w - 1, y, 1, h, Style::Normal, dark);
    c.fill(x + 2, y + 2, w - 4, 1, Style::Normal, dark);
}

// ------------------------------------------------------------------------------------------------ icons

/// Shapes in a 0..1 square (y down).
#[derive(Clone, Copy)]
enum S {
    /// Disc (cx, cy, r).
    C(f32, f32, f32),
    /// Ring (cx, cy, r, width).
    O(f32, f32, f32, f32),
    /// Box (x0, y0, x1, y1).
    R(f32, f32, f32, f32),
    /// Thick segment (x0, y0, x1, y1, width).
    L(f32, f32, f32, f32, f32),
    /// Triangle.
    T(f32, f32, f32, f32, f32, f32),
    /// Cut a disc out of what's drawn so far.
    XC(f32, f32, f32),
    /// Cut a box.
    XR(f32, f32, f32, f32),
}

fn inside(s: S, x: f32, y: f32) -> bool {
    match s {
        S::C(cx, cy, r) | S::XC(cx, cy, r) => (x - cx).powi(2) + (y - cy).powi(2) <= r * r,
        S::O(cx, cy, r, w) => {
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            d >= r - w * 0.5 && d <= r + w * 0.5
        }
        S::R(x0, y0, x1, y1) | S::XR(x0, y0, x1, y1) => x >= x0 && x <= x1 && y >= y0 && y <= y1,
        S::L(x0, y0, x1, y1, w) => {
            let (dx, dy) = (x1 - x0, y1 - y0);
            let t = (((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
            let (px, py) = (x0 + dx * t, y0 + dy * t);
            (x - px).powi(2) + (y - py).powi(2) <= (w * 0.5).powi(2)
        }
        S::T(ax, ay, bx, by, cx, cy) => {
            let d = |px: f32, py: f32, qx: f32, qy: f32| (x - qx) * (py - qy) - (px - qx) * (y - qy);
            let (d1, d2, d3) = (d(ax, ay, bx, by), d(bx, by, cx, cy), d(cx, cy, ax, ay));
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            !(neg && pos)
        }
    }
}

/// Each achievement's picture: a few shapes.
fn glyph(i: usize) -> Vec<S> {
    use S::*;
    match LIST[i].0 {
        // A laurel of stars round a pyramid.
        "majestic" => vec![T(0.5, 0.2, 0.2, 0.75, 0.8, 0.75), XR(0.2, 0.42, 0.8, 0.46), O(0.5, 0.5, 0.42, 0.05)],
        // The Statue's torch.
        "lady" => vec![R(0.44, 0.45, 0.56, 0.85), T(0.36, 0.45, 0.64, 0.45, 0.5, 0.58), C(0.5, 0.33, 0.1), T(0.42, 0.3, 0.58, 0.3, 0.5, 0.12)],
        // A generator's bolt.
        "town" => vec![T(0.55, 0.12, 0.3, 0.55, 0.52, 0.52), T(0.48, 0.48, 0.7, 0.45, 0.45, 0.88)],
        // A jet.
        "lebedev" => vec![L(0.5, 0.12, 0.5, 0.88, 0.1), T(0.5, 0.4, 0.12, 0.62, 0.88, 0.62), T(0.5, 0.75, 0.35, 0.88, 0.65, 0.88)],
        // A fork in the road.
        "choices" => vec![L(0.5, 0.88, 0.5, 0.5, 0.1), L(0.5, 0.52, 0.22, 0.15, 0.1), L(0.5, 0.52, 0.78, 0.15, 0.1)],
        // A broken chain.
        "breakout" => vec![O(0.3, 0.6, 0.15, 0.07), O(0.7, 0.4, 0.15, 0.07), XR(0.42, 0.3, 0.58, 0.7)],
        // The Dragon's Tooth.
        "hongkong" => vec![T(0.5, 0.08, 0.42, 0.62, 0.58, 0.62), R(0.3, 0.62, 0.7, 0.68), R(0.46, 0.68, 0.54, 0.9)],
        // A tanker.
        "tanker1" => vec![T(0.08, 0.55, 0.92, 0.55, 0.8, 0.72), R(0.2, 0.55, 0.8, 0.72), R(0.62, 0.35, 0.78, 0.55)],
        // A tanker going down.
        "tanker2" => vec![T(0.15, 0.4, 0.85, 0.62, 0.75, 0.75), L(0.08, 0.82, 0.92, 0.82, 0.06), L(0.18, 0.92, 0.82, 0.92, 0.04)],
        // The Eiffel tower.
        "paree" => vec![T(0.5, 0.08, 0.25, 0.9, 0.75, 0.9), XR(0.38, 0.62, 0.62, 0.92), XR(0.46, 0.25, 0.54, 0.6), R(0.3, 0.58, 0.7, 0.62)],
        // A cross pattee.
        "templars" => vec![T(0.5, 0.5, 0.32, 0.1, 0.68, 0.1), T(0.5, 0.5, 0.32, 0.9, 0.68, 0.9), T(0.5, 0.5, 0.1, 0.32, 0.1, 0.68), T(0.5, 0.5, 0.9, 0.32, 0.9, 0.68)],
        // A dish.
        "vandenberg" => vec![O(0.45, 0.45, 0.3, 0.08), XR(0.45, 0.0, 1.0, 1.0), L(0.45, 0.45, 0.75, 0.2, 0.06), L(0.4, 0.6, 0.6, 0.9, 0.1), R(0.3, 0.86, 0.75, 0.92)],
        // A submarine.
        "sea" => vec![C(0.3, 0.6, 0.15), C(0.7, 0.6, 0.15), R(0.3, 0.45, 0.7, 0.75), R(0.45, 0.3, 0.6, 0.45), L(0.55, 0.3, 0.55, 0.18, 0.04)],
        // The globe.
        "area51" => vec![O(0.5, 0.5, 0.36, 0.07), L(0.5, 0.14, 0.5, 0.86, 0.05), L(0.14, 0.5, 0.86, 0.5, 0.05), O(0.5, 0.5, 0.18, 0.05)],
        // Helios: a sun.
        "helios" => vec![C(0.5, 0.5, 0.2), L(0.5, 0.06, 0.5, 0.22, 0.06), L(0.5, 0.78, 0.5, 0.94, 0.06), L(0.06, 0.5, 0.22, 0.5, 0.06), L(0.78, 0.5, 0.94, 0.5, 0.06), L(0.19, 0.19, 0.3, 0.3, 0.06), L(0.7, 0.7, 0.81, 0.81, 0.06), L(0.81, 0.19, 0.7, 0.3, 0.06), L(0.3, 0.7, 0.19, 0.81, 0.06)],
        // A cut cable.
        "darkage" => vec![L(0.1, 0.5, 0.42, 0.5, 0.1), L(0.58, 0.5, 0.9, 0.5, 0.1), T(0.42, 0.38, 0.5, 0.5, 0.42, 0.62), L(0.58, 0.4, 0.66, 0.32, 0.04), L(0.6, 0.6, 0.68, 0.68, 0.04)],
        // The eye in the pyramid.
        "illuminati" => vec![T(0.5, 0.12, 0.12, 0.85, 0.88, 0.85), XC(0.5, 0.55, 0.16), C(0.5, 0.55, 0.07)],
        // A body with its augmentation points.
        "biomachina" => vec![C(0.5, 0.18, 0.09), L(0.5, 0.3, 0.5, 0.62, 0.12), L(0.5, 0.36, 0.25, 0.55, 0.07), L(0.5, 0.36, 0.75, 0.55, 0.07), L(0.5, 0.62, 0.35, 0.9, 0.08), L(0.5, 0.62, 0.65, 0.9, 0.08), XC(0.5, 0.45, 0.04)],
        // A fist.
        "slugger" => vec![R(0.28, 0.35, 0.72, 0.72), XR(0.38, 0.35, 0.39, 0.5), XR(0.5, 0.35, 0.51, 0.5), XR(0.61, 0.35, 0.62, 0.5), C(0.28, 0.6, 0.1), R(0.36, 0.72, 0.64, 0.88)],
        // Footprints.
        "sneak" => vec![C(0.35, 0.65, 0.1), C(0.35, 0.82, 0.06), C(0.65, 0.3, 0.1), C(0.65, 0.47, 0.06)],
        // A figure fading out.
        "section9" => vec![O(0.5, 0.22, 0.1, 0.04), L(0.5, 0.34, 0.5, 0.62, 0.04), L(0.5, 0.62, 0.35, 0.9, 0.04), L(0.5, 0.62, 0.65, 0.9, 0.04), L(0.5, 0.42, 0.28, 0.55, 0.04), L(0.5, 0.42, 0.72, 0.55, 0.04)],
        // A robot's head.
        "automoto" => vec![R(0.25, 0.3, 0.75, 0.75), XC(0.38, 0.48, 0.07), XC(0.62, 0.48, 0.07), XR(0.36, 0.62, 0.64, 0.66), L(0.5, 0.3, 0.5, 0.14, 0.04), C(0.5, 0.12, 0.05)],
        // Two coils and a spark.
        "wardenclyffe" => vec![C(0.2, 0.3, 0.1), C(0.8, 0.3, 0.1), R(0.17, 0.4, 0.23, 0.9), R(0.77, 0.4, 0.83, 0.9), L(0.3, 0.3, 0.45, 0.4, 0.04), L(0.45, 0.4, 0.55, 0.22, 0.04), L(0.55, 0.22, 0.7, 0.3, 0.04)],
        // A heart in an orbit.
        "atomheart" => vec![C(0.4, 0.42, 0.13), C(0.6, 0.42, 0.13), T(0.27, 0.47, 0.73, 0.47, 0.5, 0.78), O(0.5, 0.5, 0.42, 0.04)],
        // A rocket turned back.
        "bestoffense" => vec![L(0.75, 0.25, 0.35, 0.65, 0.12), T(0.25, 0.75, 0.25, 0.5, 0.5, 0.75), L(0.6, 0.2, 0.85, 0.2, 0.04), L(0.85, 0.2, 0.85, 0.45, 0.04)],
        // An eye through a wall.
        "specialeyes" => vec![R(0.45, 0.1, 0.55, 0.9), O(0.5, 0.5, 0.3, 0.06), C(0.5, 0.5, 0.11)],
        // A single round.
        "servant" => vec![R(0.42, 0.35, 0.58, 0.85), C(0.5, 0.35, 0.08), XR(0.42, 0.7, 0.58, 0.72)],
        // A basketball.
        "dunkin" => vec![C(0.5, 0.5, 0.36), XR(0.48, 0.14, 0.52, 0.86), XR(0.14, 0.48, 0.86, 0.52), XC(0.5, 0.5, 0.02)],
        // An open door.
        "escapa" => vec![R(0.25, 0.12, 0.75, 0.9), XR(0.32, 0.18, 0.68, 0.9), T(0.32, 0.18, 0.55, 0.28, 0.32, 0.9), T(0.55, 0.28, 0.55, 0.82, 0.32, 0.9)],
        // A closed eye.
        "sawnothing" => vec![O(0.5, 0.3, 0.36, 0.06), XR(0.0, 0.0, 1.0, 0.5), L(0.3, 0.68, 0.25, 0.78, 0.04), L(0.5, 0.72, 0.5, 0.84, 0.04), L(0.7, 0.68, 0.75, 0.78, 0.04)],
        // A key.
        "imin" => vec![O(0.3, 0.5, 0.14, 0.07), L(0.44, 0.5, 0.88, 0.5, 0.08), L(0.78, 0.5, 0.78, 0.65, 0.06), L(0.68, 0.5, 0.68, 0.62, 0.06)],
        // A ball and a flipper.
        "pinball" => vec![C(0.62, 0.3, 0.12), L(0.15, 0.78, 0.55, 0.65, 0.09), C(0.15, 0.78, 0.07)],
        // A paw.
        "wildlife" => vec![C(0.5, 0.62, 0.16), C(0.28, 0.4, 0.07), C(0.42, 0.28, 0.07), C(0.58, 0.28, 0.07), C(0.72, 0.4, 0.07)],
        // A ghost.
        "dibbuk" => vec![C(0.5, 0.4, 0.25), R(0.25, 0.4, 0.75, 0.82), XC(0.35, 0.85, 0.08), XC(0.65, 0.85, 0.08), XC(0.42, 0.38, 0.05), XC(0.58, 0.38, 0.05)],
        // A credit chit.
        "sure" => vec![R(0.18, 0.3, 0.82, 0.7), XR(0.24, 0.36, 0.76, 0.64), R(0.3, 0.45, 0.7, 0.48), R(0.3, 0.52, 0.55, 0.55)],
        // A suitcase.
        "renton" => vec![R(0.2, 0.38, 0.8, 0.8), O(0.5, 0.36, 0.1, 0.05), XR(0.0, 0.36, 1.0, 1.0), R(0.2, 0.38, 0.8, 0.8), XR(0.48, 0.5, 0.52, 0.68)],
        // A shield and star.
        "hero" => vec![R(0.25, 0.15, 0.75, 0.55), T(0.25, 0.55, 0.75, 0.55, 0.5, 0.9), XC(0.5, 0.45, 0.12)],
        // A magnifier.
        "tail" => vec![O(0.42, 0.42, 0.2, 0.07), L(0.56, 0.56, 0.85, 0.85, 0.09)],
        // A newspaper.
        "meanjoe" => vec![R(0.18, 0.2, 0.82, 0.8), XR(0.24, 0.26, 0.76, 0.74), R(0.28, 0.3, 0.72, 0.4), R(0.28, 0.46, 0.48, 0.7), R(0.52, 0.46, 0.72, 0.5), R(0.52, 0.56, 0.72, 0.6), R(0.52, 0.66, 0.72, 0.7)],
        // A speech balloon.
        "based" => vec![C(0.5, 0.42, 0.3), T(0.3, 0.6, 0.45, 0.68, 0.2, 0.88), XR(0.3, 0.38, 0.7, 0.42), XR(0.3, 0.48, 0.6, 0.52)],
        // A rifle.
        "nicegun" => vec![R(0.1, 0.42, 0.78, 0.52), R(0.78, 0.44, 0.92, 0.5), R(0.38, 0.52, 0.48, 0.7), T(0.1, 0.42, 0.1, 0.62, 0.25, 0.52), R(0.55, 0.36, 0.7, 0.42)],
        // Crosshairs.
        "nerves" => vec![O(0.5, 0.5, 0.3, 0.06), L(0.5, 0.08, 0.5, 0.36, 0.05), L(0.5, 0.64, 0.5, 0.92, 0.05), L(0.08, 0.5, 0.36, 0.5, 0.05), L(0.64, 0.5, 0.92, 0.5, 0.05)],
        // Two figures side by side.
        "tonjc" => vec![C(0.33, 0.3, 0.1), R(0.23, 0.42, 0.43, 0.85), C(0.67, 0.3, 0.1), R(0.57, 0.42, 0.77, 0.85)],
        // Two asterisks.
        "howdid" => vec![L(0.3, 0.3, 0.3, 0.7, 0.07), L(0.13, 0.4, 0.47, 0.6, 0.07), L(0.13, 0.6, 0.47, 0.4, 0.07), L(0.7, 0.3, 0.7, 0.7, 0.07), L(0.53, 0.4, 0.87, 0.6, 0.07), L(0.53, 0.6, 0.87, 0.4, 0.07)],
        // A broken gear.
        "notmachine" => vec![C(0.5, 0.5, 0.28), R(0.44, 0.1, 0.56, 0.9), R(0.1, 0.44, 0.9, 0.56), XC(0.5, 0.5, 0.12), XR(0.48, 0.5, 1.0, 0.54)],
        // A falling tear.
        "shame" => vec![C(0.5, 0.62, 0.2), T(0.5, 0.15, 0.32, 0.55, 0.68, 0.55)],
        // The eight ball.
        "pool" => vec![C(0.5, 0.5, 0.36), XC(0.5, 0.42, 0.14), C(0.5, 0.36, 0.045), C(0.5, 0.48, 0.055)],
        // Two chevrons.
        "veteran" => vec![L(0.2, 0.42, 0.5, 0.22, 0.09), L(0.5, 0.22, 0.8, 0.42, 0.09), L(0.2, 0.68, 0.5, 0.48, 0.09), L(0.5, 0.48, 0.8, 0.68, 0.09)],
        // A gun and a plus.
        "lockload" => vec![R(0.08, 0.55, 0.62, 0.64), R(0.25, 0.64, 0.33, 0.8), R(0.68, 0.18, 0.76, 0.48), R(0.57, 0.29, 0.87, 0.37)],
        // Three rising arrows.
        "upgrades" => vec![T(0.5, 0.1, 0.25, 0.32, 0.75, 0.32), T(0.5, 0.36, 0.25, 0.58, 0.75, 0.58), T(0.5, 0.62, 0.25, 0.84, 0.75, 0.84), XC(0.5, 0.36, 0.0)],
        _ => vec![C(0.5, 0.5, 0.3)],
    }
}

/// An achievement's icon, 36 x 36: the picture lit in the theme's colour on dark glass in a bevelled frame
/// (unlocked), or grey with a padlock (locked).
pub fn icon(i: usize, unlocked: bool, col: [u8; 3]) -> UiTex {
    const N: usize = 36;
    const SS: usize = 4;
    let shapes = glyph(i.min(LIST.len() - 1));
    let mut cov = vec![0f32; N * N];
    for py in 0..N {
        for px in 0..N {
            let mut hit = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - 4.0) / (N as f32 - 8.0);
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32 - 4.0) / (N as f32 - 8.0);
                    let mut on = false;
                    for s in &shapes {
                        match s {
                            S::XC(..) | S::XR(..) => {
                                if inside(*s, x, y) {
                                    on = false;
                                }
                            }
                            _ => {
                                if inside(*s, x, y) {
                                    on = true;
                                }
                            }
                        }
                    }
                    hit += on as u32;
                }
            }
            cov[py * N + px] = hit as f32 / (SS * SS) as f32;
        }
    }
    let base = if unlocked { col } else { [96, 100, 104] };
    let hi = if unlocked { [255, 255, 255] } else { [140, 144, 148] };
    let mut px = vec![[0u8; 4]; N * N];
    for y in 0..N {
        for x in 0..N {
            // Glass: dark, a little lighter at the top.
            let g = 18.0 + 14.0 * (1.0 - y as f32 / N as f32);
            let mut c = [g, g + 2.0, g + 5.0];
            // Frame: light top / left, dark bottom / right.
            let edge = x == 0 || y == 0 || x == N - 1 || y == N - 1;
            if edge {
                let v = if x == 0 || y == 0 { base.map(|b| b as f32 * 0.8) } else { base.map(|b| b as f32 * 0.3) };
                c = v;
            } else if x == 1 || y == 1 || x == N - 2 || y == N - 2 {
                c = [8.0, 9.0, 11.0];
            }
            // The picture: theme colour shading to white at the top, with a soft dark outline under it.
            let k = cov[y * N + x];
            let below = if y > 0 && x > 0 { cov[(y - 1) * N + (x - 1)] } else { 0.0 };
            if !edge {
                let shade = below * (1.0 - k) * 0.6;
                c = c.map(|v| v * (1.0 - shade));
                let t = 1.0 - y as f32 / N as f32;
                let fg = [0, 1, 2].map(|j| base[j] as f32 + (hi[j] as f32 - base[j] as f32) * t * 0.55);
                c = [0, 1, 2].map(|j| c[j] + (fg[j] - c[j]) * k);
            }
            px[y * N + x] = [c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8, 255];
        }
    }
    if !unlocked {
        // A small padlock in the corner.
        let lock = [S::O(0.84, 0.74, 0.07, 0.04), S::R(0.74, 0.76, 0.94, 0.94)];
        for y in 0..N {
            for x in 0..N {
                let (fx, fy) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
                if lock.iter().any(|s| inside(*s, fx, fy)) {
                    px[y * N + x] = [200, 170, 60, 255];
                }
            }
        }
    }
    UiTex { w: N as u32, h: N as u32, px }
}

/// The persona Achievements screen's list (inside the Logs screen's frame): returns the scroll range.
#[allow(clippy::too_many_arguments)]
pub fn draw_list(c: &mut Canvas, head: &crate::uicanvas::UiFont, small: &crate::uicanvas::UiFont, x: i32, y: i32, w: i32, h: i32, scroll: i32) -> i32 {
    const ROW: i32 = 46;
    let col = crate::menuui::h_col(9);
    c.set_clip(Some((x, y, w, h)));
    for (i, (id, name, goal)) in LIST.iter().enumerate() {
        let ry = y + i as i32 * ROW - scroll;
        if ry + ROW < y || ry > y + h {
            continue;
        }
        let (when, prog) = state(id);
        let on = when.is_some();
        let ic = icon(i, on, col);
        c.texture(x + 4, ry + 5, 36, 36, 0, 0, &ic, Style::Masked, [255, 255, 255]);
        let title_col = if on { [255, 255, 255] } else { [150, 154, 158] };
        c.text(head, x + 46, ry + 4, w - 140, head.height, HAlign::Left, VAlign::Top, title_col, name);
        let status = match (when, target(id)) {
            (Some(t), _) => date(t),
            (None, n) if n > 0 => format!("{prog} / {n}"),
            _ => "Locked".to_string(),
        };
        c.text(small, x + w - 96, ry + 4, 90, small.height, HAlign::Right, VAlign::Top, if on { col } else { [120, 124, 128] }, &status);
        let lines = crate::uicanvas::wrap(small, goal, w - 54);
        for (k, l) in lines.iter().take(2).enumerate() {
            c.text(small, x + 46, ry + 4 + head.height + 2 + k as i32 * small.height, w - 54, small.height, HAlign::Left, VAlign::Top, if on { col } else { [128, 132, 136] }, l);
        }
        c.fill(x + 2, ry + ROW - 1, w - 4, 1, Style::Normal, [40, 44, 48]);
    }
    c.set_clip(None);
    (LIST.len() as i32 * ROW - h).max(0)
}

/// A unix time as a calendar date (UTC), MM/DD/YYYY.
fn date(t: u64) -> String {
    let days = (t / 86400) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{m:02}/{d:02}/{y}")
}

#[cfg(test)]
mod tests {
    /// ACH_SHEET=out.rgba cargo test -p dxview icon_sheet: every icon, 10 x 5 (unlocked; locked in the second half).
    #[test]
    fn icon_sheet() {
        let Ok(out) = std::env::var("ACH_SHEET") else { return };
        let (cols, n) = (10usize, super::LIST.len());
        let rows = n.div_ceil(cols) * 2;
        let (w, h) = (cols * 40, rows * 40);
        let mut px = vec![0u8; w * h * 4];
        for i in 0..n * 2 {
            let t = super::icon(i % n, i < n, [200, 210, 225]);
            let (gx, gy) = ((i % n) % cols * 40 + 2, ((i % n) / cols + if i < n { 0 } else { rows / 2 }) * 40 + 2);
            for y in 0..36 {
                for x in 0..36 {
                    let p = t.px[y * 36 + x];
                    let o = ((gy + y) * w + gx + x) * 4;
                    px[o..o + 4].copy_from_slice(&p);
                }
            }
        }
        std::fs::write(&out, &px).unwrap();
        println!("{w}x{h}");
    }

    #[test]
    fn dates_and_icons() {
        assert_eq!(super::date(0), "01/01/1970");
        assert_eq!(super::date(1_793_404_800), "10/31/2026");
        for i in 0..super::LIST.len() {
            let t = super::icon(i, true, [200, 210, 220]);
            assert_eq!(t.px.len(), 36 * 36);
        }
        let ids: std::collections::HashSet<_> = super::LIST.iter().map(|a| a.0).collect();
        assert_eq!(ids.len(), 50);
    }
}
