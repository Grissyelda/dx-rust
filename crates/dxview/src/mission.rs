//! Mission scripts (MissionScript and Mission01): FirstFrame's flags, and Mission01's Timer, once a second:
//!
//! * M01PlayerAggressive once fewer than 23 terrorists are alive or merely knocked out (a lethal player kill
//!   doesn't count), for the later missions' reactions;
//! * the infolink when the NSF leader dies (DL_LeaderKilled, or DL_LeaderKilledInSpite after being told to let
//!   him live) and when he's talked to (DL_LeaderNotKilled);
//! * StatueMissionComplete: the UNATCO doors unlock (the main door opens), the custody and post-mission troops
//!   enter the world, the remaining terrorists drop, the thugs, the bot and Gunther go, and Paul waits at the
//!   PaulTeleport spawn point.

use crate::ai::{GameFlags, Sel, Takedowns, WorldReq};
use crate::LevelRes;
use bevy::prelude::*;
use glam::DVec3;
use imsim_core::flags::FlagValue;
use unreal_pkg::props::{find, Value};

#[derive(Default)]
pub struct MissionState {
    started: bool,
    first_done: bool,
    timer: f32,
    map: String,
    mission: i32,
    terrorists: usize,
    vars: crate::missions::Vars,
    seed: u64,
}

fn set(flags: &mut imsim_core::flags::FlagBase, name: &str, expires: i32) {
    flags.set(name, FlagValue::Bool(true), expires);
}

fn tag_of(a: &dx_level::actors::ActorInfo) -> String {
    match find(&a.props, "Tag") {
        Some(Value::Name(t)) => t.clone(),
        _ => String::new(),
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn mission_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut flags: ResMut<GameFlags>,
    takedowns: Res<Takedowns>,
    mut movers: ResMut<crate::movers::Movers>,
    (mut inv, mut weapons, decos, npcs, grenades): (
        ResMut<crate::pickups::PlayerInventory>,
        ResMut<crate::weapons::Weapons>,
        Query<&crate::actors::Deco>,
        Query<(&crate::ai::Npc, &crate::actors::Deco, Has<crate::ai::OutOfWorld>)>,
        Query<(&crate::grenades::Grenade, Option<&crate::actors::MeshActorRef>)>,
    ),
    (mut assets, player, ph): (
        ResMut<crate::weapons::RuntimeAssets>,
        Res<crate::Player>,
        Res<crate::ai::PlayerHealth>,
    ),
    mut map_gen: Local<u64>,
    mut st: Local<MissionState>,
) {
    // A new map (loader.rs): start this system's per-map state over.
    if crate::loader::new_map(&mut map_gen) {
        *st = Default::default();
    }
    let _prof = crate::prof::scope("mission::mission_system");
    let lvl = &level.0;
    let fl = &mut flags.0;
    let info = lvl.actors.iter().find(|a| a.class == "DeusExLevelInfo");
    if !st.started {
        st.started = true;
        st.map = info
            .and_then(|a| match find(&a.props, "MapName") {
                Some(Value::Str(s)) => Some(s.to_ascii_uppercase()),
                _ => None,
            })
            .unwrap_or_default();
        // missionNumber's default is 0 (the training maps don't store it).
        st.mission = info
            .and_then(|a| match find(&a.props, "missionNumber") {
                Some(Value::Int(n)) => Some(*n),
                _ => None,
            })
            .unwrap_or(0);
        st.terrorists = lvl.actors.iter().filter(|a| a.class == "Terrorist").count();
        // InitStateMachine: flags set from now on expire in the next mission unless told otherwise.
        fl.set_default_expiration(st.mission + 1);
        crate::classtree::register_level(&mut assets.classes, lvl);
        if st.mission == 99 && info.is_some_and(|a| find(&a.props, "Script").is_some()) {
            // MissionEndgame.InitStateMachine: destroy all flags; PlayerTraveling stays set.
            fl.delete_all();
            fl.set("PlayerTraveling", FlagValue::Bool(true), 0);
        }
        crate::missions::set_current(st.mission, &st.map);
        // Discord Rich Presence: where we are.
        {
            let loc = info
                .and_then(|a| match find(&a.props, "MissionLocation") {
                    Some(Value::Str(s)) if !s.trim().is_empty() => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| st.map.clone());
            if crate::menus::intro_map() || st.mission < 0 {
                crate::discord::set("In the main menu", "");
            } else {
                let diff = match crate::travel::combat_difficulty() {
                    d if d <= 1.0 => "Easy",
                    d if d <= 1.5 => "Medium",
                    d if d <= 2.0 => "Hard",
                    _ => "Realistic",
                };
                let m = if st.mission == 0 { "Training".to_string() } else { format!("Mission {}", st.mission) };
                crate::discord::set(&loc, &format!("{m} - {diff}"));
            }
        }
        // LDDP (JCDentonMale.TravelPostAccept): its flags on every arrival, never expiring.
        if crate::travel::lddp() {
            for (f, v) in crate::travel::lddp_flags() {
                fl.set(f, FlagValue::Bool(v), 0);
            }
        }
        st.seed = 0x9E37_79B9_7F4A_7C15 ^ (st.map.len() as u64);
    }
    if crate::save::restoring() || crate::menus::intro_map() {
        return;
    }
    // A map without a mission script (12_Vandenberg_Tunnels, 99_Endgame4) runs none of this.
    if info.is_none_or(|a| find(&a.props, "Script").is_none()) {
        return;
    }
    // MissionScript runs on a timer (checkTime 1 s); InitStateMachine + FirstFrame on its first tick, once the
    // player's carried state (goals, history) is in place.
    st.timer += time.delta_secs();
    if st.timer < 1.0 {
        return;
    }
    st.timer = 0.0;
    let mission = st.mission;
    if !st.first_done {
        if crate::consys::lock().is_none() {
            return;
        }
        st.first_done = true;
        // Not on a loaded game: the save already holds what FirstFrame would do (PlayerTraveling unset).
        if !crate::save::loaded_game() {
            fl.expire(mission);
            fl.delete("PlayerTraveling");
            // Important characters killed earlier in the game stay dead: <BindName>_Dead destroys them.
            for a in lvl.actors.iter() {
                if !matches!(find(&a.props, "bImportant"), Some(Value::Bool(true))) {
                    continue;
                }
                if let Some(Value::Str(b)) = find(&a.props, "BindName") {
                    if !b.is_empty() && fl.get_bool(&format!("{b}_Dead")) {
                        crate::ai::world_req(crate::ai::WorldReq::Destroy(crate::ai::Sel::Bind(b.clone())));
                    }
                }
            }
            let startup = format!("M{}_StartupText", st.map);
            if !fl.get_bool(&startup) {
                // The mission startup text, once per map (HUDMissionStartTextDisplay).
                let lines: Vec<String> = info
                    .map(|a| {
                        let mut v: Vec<(u32, String)> = a
                            .props
                            .iter()
                            .filter(|p| p.name.eq_ignore_ascii_case("startupMessage"))
                            .filter_map(|p| match &p.value {
                                Value::Str(s) => Some((p.index, s.clone())),
                                _ => None,
                            })
                            .collect();
                        v.sort_by_key(|x| x.0);
                        v.into_iter().map(|x| x.1).collect()
                    })
                    .unwrap_or_default();
                if lines.first().is_some_and(|l| !l.is_empty()) {
                    crate::classic_hud::start_text(&lines);
                    fl.set_bool(&startup, true);
                }
            }
            let start = format!("M{mission}MissionStart");
            if !fl.get_bool(&start) {
                // ResetGoals (completed primary and all secondary goals go) and ResetConversationHistory.
                if let Some(cs) = crate::consys::lock().as_mut() {
                    cs.goals.retain(|g| g.primary && !g.completed);
                    cs.history.clear();
                }
                fl.set_bool(&start, true);
            }
            lddp_first_frame(&st.map, fl);
            first_frame(lvl, &st.map, fl);
            if mission >= 2 {
                let map = st.map.clone();
                let st = &mut *st;
                let mut c = ctx(lvl, fl, &map, mission, &npcs, &decos, &mut movers, &mut assets, &player, &ph, &mut st.vars, &mut st.seed, time.elapsed_secs_f64());
                crate::missions::first_frame(&mut c);
            }
        }
    }
    if mission >= 2 {
        let map = st.map.clone();
        let st = &mut *st;
        let mut c = ctx(lvl, fl, &map, mission, &npcs, &decos, &mut movers, &mut assets, &player, &ph, &mut st.vars, &mut st.seed, time.elapsed_secs_f64());
        crate::missions::timer(&mut c);
        return;
    }
    match st.map.as_str() {
        "00_TRAINING" | "00_TRAININGCOMBAT" | "00_TRAININGFINAL" => {
            mission00_timer(lvl, &st.map, fl, &mut inv, &mut weapons, &decos, &npcs, &grenades);
            return;
        }
        "01_NYC_UNATCOHQ" => {
            // Unhide Paul for the briefing.
            if !fl.get_bool("MS_ReadyForBriefing") && fl.get_bool("M01ReadyForBriefing") {
                crate::ai::world_req(WorldReq::Enter(Sel::Class("PaulDenton".into())));
                set(fl, "MS_ReadyForBriefing", 2);
            }
            return;
        }
        "01_NYC_UNATCOISLAND" => {}
        _ => return,
    }
    // Terrorists still standing, or knocked out by JC, or killed by someone else.
    if !fl.get_bool("M01PlayerAggressive") {
        let killed_by_jc = takedowns
            .log
            .iter()
            .filter(|t| t.class == "Terrorist" && t.by_player && t.lethal)
            .count();
        let count = st.terrorists.saturating_sub(killed_by_jc);
        if count < 23 {
            set(fl, "M01PlayerAggressive", 6);
            println!("MISSION M01PlayerAggressive");
        }
    }
    if !fl.get_bool("MS_DL_Played") && fl.get_bool("TerroristCommander_Dead") {
        let name = if !fl.get_bool("DL_LeaderNotKilled_Played") {
            "DL_LeaderKilled"
        } else {
            "DL_LeaderKilledInSpite"
        };
        crate::consys::start_datalink(fl, name, None);
        set(fl, "MS_DL_Played", 2);
    }
    if !fl.get_bool("PlayerAttackedStatueTerrorist")
        && fl.get_bool("MeetTerrorist_Played")
        && !fl.get_bool("MS_DL2_Played")
    {
        crate::consys::start_datalink(fl, "DL_LeaderNotKilled", None);
        set(fl, "MS_DL2_Played", 2);
    }
    if !fl.get_bool("MS_MissionComplete") && fl.get_bool("StatueMissionComplete") {
        println!("MISSION statue mission complete");
        for (info, rt) in lvl.movers.iter().zip(movers.rt.iter_mut()) {
            let tag = info.tag.to_ascii_lowercase();
            if tag == "un_maindoor" {
                rt.locked = false;
                rt.lock_strength = 0.0;
                crate::movers::trigger(info, rt);
            } else if tag == "statueruindoors" || tag == "temp_celldoor" {
                rt.locked = false;
                rt.lock_strength = 0.0;
            }
        }
        crate::ai::world_req(WorldReq::Enter(Sel::Bind("custodytroop".into())));
        crate::ai::world_req(WorldReq::Enter(Sel::Bind("postmissiontroops".into())));
        crate::ai::world_req(WorldReq::Destroy(Sel::Class("ThugMale2".into())));
        crate::ai::world_req(WorldReq::Destroy(Sel::Class("SecurityBot3".into())));
        crate::ai::world_req(WorldReq::Kill(Sel::ClassBut(
            "Terrorist".into(),
            "TerroristCommander".into(),
        )));
        crate::ai::world_req(WorldReq::Destroy(Sel::Bind("GuntherHermann".into())));
        if let Some(sp) = lvl.actors.iter().find(|a| {
            a.class == "SpawnPoint" && matches!(find(&a.props, "Tag"), Some(Value::Name(t)) if t.eq_ignore_ascii_case("PaulTeleport"))
        }) {
            let l = sp.location();
            let yaw = match find(&sp.props, "Rotation") {
                Some(Value::Rotator(r)) => r[1],
                _ => 0,
            };
            crate::ai::world_req(WorldReq::Teleport(
                Sel::Bind("PaulDenton".into()),
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64),
                yaw,
            ));
        }
        // The NSF turrets and the statue laser trap go; the NSF cameras stop raising alarms.
        crate::security::statue_cleared();
        set(fl, "MS_MissionComplete", 2);
    }
}

/// Mission00 / Mission01 FirstFrame (after MissionScript.FirstFrame).
/// Lay D Denton's Mission01 / Mission06.FirstFrame: whoever doesn't fit this JC leaves - Shannon (her lines are
/// for the male JC) or the mod's Chet at UNATCO; Mercedes and Tessa (unless "Original Interactions") or the mod's
/// Russ in the Lucky Money.
fn lddp_first_frame(map: &str, fl: &imsim_core::flags::FlagBase) {
    if !crate::travel::lddp() {
        return;
    }
    let female = fl.get_bool("LDDPJCIsFemale");
    let destroy = |s: Sel| crate::ai::world_req(WorldReq::Destroy(s));
    match map {
        "01_NYC_UNATCOHQ" if female => destroy(Sel::isa_tag("Female2", "Female2")),
        "01_NYC_UNATCOHQ" => destroy(Sel::isa("LDDPChet")),
        "06_HONGKONG_WANCHAI_UNDERWORLD" => {
            if female && !fl.get_bool("LDDPMaleCont4FJC") {
                destroy(Sel::isa_tag("Hooker1", "ClubMercedes"));
                destroy(Sel::isa_tag("LowerClassFemale", "ClubTessa"));
            }
            if !female {
                destroy(Sel::isa("LDDPRuss"));
            }
        }
        _ => {}
    }
}

fn first_frame(lvl: &dx_level::Level, map: &str, fl: &mut imsim_core::flags::FlagBase) {
    match map {
        "00_TRAINING" | "00_TRAININGCOMBAT" | "00_TRAININGFINAL" => {
            // Zero the player's skill points.
            if let Some(a) = crate::abilities::lock().as_mut() {
                a.skills.points_avail = 0;
                a.skills.points_total = 0;
            }
            if map == "00_TRAINING" {
                // Knock the test subject out.
                crate::ai::world_req(WorldReq::KnockOut(Sel::ClassTag("UNATCOTroop".into(), "Test_Subject".into())));
            }
            // 00_TRAININGCOMBAT: the Bot1LAM is owned by the player, so pawns can set it off (grenades::attach_grenades).
        }
        "01_NYC_UNATCOISLAND" => {
            // Paul and company are gone after the final briefing.
            if fl.get_bool("M02Briefing_Played") {
                crate::ai::world_req(WorldReq::Destroy(Sel::Class("PaulDenton".into())));
                crate::ai::world_req(WorldReq::Destroy(Sel::ClassTag("UNATCOTroop".into(), "custodytroop".into())));
                crate::ai::world_req(WorldReq::Destroy(Sel::ClassTag(
                    "TerroristCommander".into(),
                    "TerroristCommander".into(),
                )));
            }
        }
        _ => {}
    }
    let _ = lvl;
}

/// Mission00.Timer: the training course's scripted reactions.
#[allow(clippy::too_many_arguments)]
fn mission00_timer(
    lvl: &dx_level::Level,
    map: &str,
    fl: &mut imsim_core::flags::FlagBase,
    inv: &mut crate::pickups::PlayerInventory,
    weapons: &mut crate::weapons::Weapons,
    decos: &Query<&crate::actors::Deco>,
    npcs: &Query<(&crate::ai::Npc, &crate::actors::Deco, Has<crate::ai::OutOfWorld>)>,
    grenades: &Query<(&crate::grenades::Grenade, Option<&crate::actors::MeshActorRef>)>,
) {
    let alive_npcs = |class: &str, tag: &str| {
        npcs.iter()
            .filter(|(_, _, oow)| !oow)
            .map(|(n, _, _)| n)
            .filter(|n| n.state != crate::ai::NpcState::Dead)
            .filter(|n| {
                let a = &lvl.actors[n.actor];
                a.class.eq_ignore_ascii_case(class) && tag_of(a).eq_ignore_ascii_case(tag)
            })
            .count()
    };
    if map == "00_TRAINING" || map == "00_TRAININGCOMBAT" {
        // Delete the player's inventory in case he tries to keep it (RemoveAllInventory: every displayable
        // item and all ammo).
        if fl.get_bool("MunitionsTrooperReady") && (!inv.0.items.is_empty() || !inv.0.ammo.is_empty()) {
            inv.0.items.clear();
            inv.0.belt = [None; 10];
            inv.0.ammo.clear();
            weapons.held = None;
            weapons.pending = None;
            weapons.pending_class = None;
        }
    }
    if map == "00_TRAINING" {
        // The player broke one of the lockpicking crates.
        if !fl.get_bool("MS_CratesBroken") {
            let count = decos
                .iter()
                .filter(|d| {
                    let Some(m) = lvl.mesh_actors.get(d.mesh_actor) else { return false };
                    let t = tag_of(&lvl.actors[m.actor]);
                    t.eq_ignore_ascii_case("Cratepick1") || t.eq_ignore_ascii_case("Cratepick2")
                })
                .count();
            if count < 2 {
                crate::consys::start_datalink(fl, "DL_Lockpick", None);
                fl.set_bool("MS_CratesBroken", true);
            }
        }
        // Hide holo-Bob.
        if fl.get_bool("M00MeetPage_Played") && !fl.get_bool("MS_PageHidden") {
            crate::ai::world_req(WorldReq::Leave(Sel::ClassTag("BobPage".into(), "holobob".into())));
            fl.set_bool("MS_PageHidden", true);
        }
    } else if map == "00_TRAININGCOMBAT" {
        if !fl.get_bool("MS_LAMBot1Destroyed") && alive_npcs("SecurityBot2", "LAMBot1") == 0 {
            crate::consys::start_datalink(fl, "DL_ReleaseBot2", None);
            fl.set_bool("MS_LAMBot1Destroyed", true);
        }
        if !fl.get_bool("MS_LAMBot2Destroyed") && alive_npcs("SecurityBot2", "LAMBot2") == 0 {
            crate::consys::start_datalink(fl, "DL_DemolitionLast", None);
            fl.set_bool("MS_LAMBot2Destroyed", true);
        }
        if !fl.get_bool("MS_LAMsDestroyed") {
            // foreach AllActors(class'LAM', lam, 'DefuseLAM'): the map's LAMs tagged DefuseLAM still in the world
            // (a disarmed one counts until it's picked up or goes off).
            let left = grenades
                .iter()
                .filter(|(_, r)| {
                    r.and_then(|r| lvl.mesh_actors.get(r.0)).is_some_and(|m| {
                        let a = &lvl.actors[m.actor];
                        a.class == "LAM" && tag_of(a).eq_ignore_ascii_case("DefuseLAM")
                    })
                })
                .count();
            if left == 0 {
                crate::consys::start_datalink(fl, "DL_DefuseFinish", None);
                fl.set_bool("MS_LAMsDestroyed", true);
            }
        }
        // Max out the rifle skill.
        if fl.get_bool("DL_RifleRangeShot1_Played") && !fl.get_bool("MS_SkillIncreased") {
            if let Some(a) = crate::abilities::lock().as_mut() {
                if let Some(s) = a.skills.list.iter_mut().find(|s| s.def.class.eq_ignore_ascii_case("SkillWeaponRifle")) {
                    s.level = 3;
                }
            }
            fl.set_bool("MS_SkillIncreased", true);
        }
    } else if map == "00_TRAININGFINAL" {
        if fl.get_bool("JaimeAppears") && !fl.get_bool("MS_ReyesUnhidden") {
            crate::ai::world_req(WorldReq::Enter(Sel::ClassTag("JaimeReyes".into(), "holojaime".into())));
            fl.set_bool("MS_ReyesUnhidden", true);
        }
        if fl.get_bool("JaimeBark_Played") && !fl.get_bool("MS_ReyesHidden") {
            crate::ai::world_req(WorldReq::Leave(Sel::ClassTag("JaimeReyes".into(), "holojaime".into())));
            fl.set_bool("MS_ReyesHidden", true);
        }
    }
}

/// A mission-script context for missions.rs: the map's pawns (alive, in the world or not), bodies and decorations.
#[allow(clippy::too_many_arguments)]
fn ctx<'a>(
    lvl: &'a dx_level::Level,
    fl: &'a mut imsim_core::flags::FlagBase,
    map: &str,
    mission: i32,
    npcs: &Query<(&crate::ai::Npc, &crate::actors::Deco, Has<crate::ai::OutOfWorld>)>,
    decos: &Query<&crate::actors::Deco>,
    movers: &'a mut crate::movers::Movers,
    assets: &mut crate::weapons::RuntimeAssets,
    player: &crate::Player,
    ph: &crate::ai::PlayerHealth,
    vars: &'a mut crate::missions::Vars,
    seed: &'a mut u64,
    now: f64,
) -> crate::missions::Ctx<'a> {
    let mut pawns = Vec::new();
    let mut carcs = Vec::new();
    let mut pawn_actors = std::collections::HashSet::new();
    for (n, d, oow) in npcs.iter() {
        let a = &lvl.actors[n.actor];
        pawn_actors.insert(n.actor);
        if n.state != crate::ai::NpcState::Dead {
            pawns.push(crate::missions::PawnInfo {
                actor: n.actor,
                class: a.class.clone(),
                tag: tag_of(a),
                bind: n.bind_name.clone(),
                in_world: !oow,
                standing: n.orders == crate::ai::Orders::Standing && n.state == crate::ai::NpcState::Idle,
                pos: d.body.location,
                emp_hp: n.emp_hp,
                robot: n.hp_body.robot,
            });
        } else if !n.carried && !n.hp_body.robot {
            // The body: a placed carcass is its own class, a pawn leaves its CarcassType.
            let class = if a.class.to_ascii_lowercase().ends_with("carcass") {
                a.class.clone()
            } else {
                let p = &a.props;
                let ct = match find(p, "CarcassType") {
                    Some(Value::Path(s)) => Some(s.clone()),
                    _ => match find(&assets.classes.defaults(&a.class), "CarcassType") {
                        Some(Value::Path(s)) => Some(s.clone()),
                        _ => None,
                    },
                };
                ct.map(|s| s.rsplit('.').next().unwrap_or("").to_string()).unwrap_or_else(|| "DeusExCarcass".into())
            };
            crate::classtree::register(&mut assets.classes, &class);
            carcs.push(crate::missions::CarcInfo {
                actor: n.actor,
                class,
                tag: tag_of(a),
                killer: n.killer_bind.clone(),
                unconscious: n.knocked_out,
            });
        }
    }
    let decos = decos
        .iter()
        .filter_map(|d| lvl.mesh_actors.get(d.mesh_actor).map(|m| (m.actor, d.body.location)))
        .filter(|(ai, _)| !pawn_actors.contains(ai))
        .map(|(ai, at)| (ai, lvl.actors[ai].class.clone(), tag_of(&lvl.actors[ai]), at))
        .collect();
    crate::missions::Ctx {
        lvl,
        fl,
        map: map.to_string(),
        mission,
        pawns,
        carcs,
        decos,
        movers,
        player_dead: ph.dead,
        player_pos: player.body.location,
        now,
        vars,
        seed,
    }
}
