//! The Nameless Mod, Mission 24: the GameSpy space station. Written for DX RUST from how the mission behaves in the
//! player's own copy of the mod; flag, tag and map names are the identifiers its maps use, texts come from the
//! script's own strings at run time.
//!
//! Covered: arrival set-up for the four station maps (holograms of the faction leader, Kylie's arrival, Deus
//! Diablo, the airlocks, the faction's prize), the respawning GameSpy firewalls and their switched loyalties, the
//! station shaking itself apart once the turbines stop, quest awards, the airlocks, schematics, the robots that die
//! together, the "not seen" flags.
//!
//! Left out: oxygen zones (no air to run out of), the Narcissus window and New Game Plus, the station TV's live
//! census text, the turning solar panels and sky box, Trestkon catching fire on cue.

use super::*;

/// The GameSpy firewalls that come back 20 s after they're taken down: (spawn point tag, pawn tag, patrol start).
const FIREWALLS: [(&str, &str, &str); 3] = [
    ("GSFirewallS1", "GSFirewallSpawnee1", "GSFPPA1"),
    ("GSFirewallS2", "GSFirewallSpawnee2", "GSFPPB1"),
    ("GSFirewallS3", "GSFirewallSpawnee3", "GSFPPC1"),
];

/// The flags that open the Narcissus ending: three of them are enough.
const NARC: [&str; 9] =
    ["NARC_Source", "NARC_Tresk", "NARC_Kashue", "NARC_Phas", "NARC_IRC", "NARC_Testers", "NARC_NoCult", "NARC_Sewers", "NARC_Despots"];

pub(super) fn first(c: &mut Ctx, map: &str) {
    if map != "24_SPACESTATION02" && c.b("MS_DDSafe") {
        c.set("MS_DDSafe", false, 0);
        c.pm("PM_Kashue_DDSafe");
    }
    match map {
        "24_SPACESTATION01" => {
            let n = NARC.iter().filter(|f| c.b(f)).count();
            c.set("MS_NarcissusEndingAvailable", n >= 3, 25);
            if !c.b("MS_SSIntro_Played") {
                c.trigger("ShuttleIntroSequence");
                c.set("MS_SSIntro_Played", true, 25);
            }
            if c.b("ALLIED_WC") && !c.b("Scara_Briefing_Played") {
                c.enter_tag("Scara_Hologram");
            } else if c.b("ALLIED_PDX") && !c.b("Kashue_Briefing_Played") {
                c.enter_tag("KK_Hologram1");
            }
            kylie_arrives(c);
            if c.b("QC_MAIN_DestroyLEETs") && c.b("DD_Safe") {
                crate::consys::bind_tag("ABISpaceShip", "", "Deus Diablo");
            }
            if c.b("MS_TurbinesStopped") && !c.any(&["DeusDiablo_PDX_Played", "DeusDiablo_WC_Played"]) {
                c.enter_tag("DD");
            }
        }
        "24_SPACESTATION02" => {
            if c.b("ALLIED_WC") || c.b("MS_TurbinesStopped") {
                c.leave_tag("DD");
            }
            if c.b("ALLIED_PDX") {
                c.trigger("DDLivingRoomTrigger");
            }
        }
        "24_SPACESTATION03" => {
            if !c.b("MS_SSAirlockDoorsSetup") {
                for t in [
                    "EngineeringAirlock1Door2", "EngineeringAirlock2Door1", "EngineeringAirlock3Door1", "EngineeringAirlock4Door1",
                    "EngineeringAirlock5Door1", "EngineeringAirlock6Door1",
                ] {
                    c.trigger(t);
                }
                c.set("MS_SSAirlockDoorsSetup", true, 25);
            }
            let can = c.b("QR_MAIN_DestroyLEETs") && !c.b("Ryan_Banned");
            c.set("CanDestroyLEETs", can, 25);
            if !c.b("MS_SpawnedDD") && c.b("QR_MAIN_ActivateAPGs") {
                c.leave_tag("APGAlien");
                c.leave_tag("APGSecBot");
                c.enter_tag("DD");
                crate::security::event("APGCamera", false);
                c.set("MS_SpawnedDD", true, 25);
            }
            // Each faction's prize: the other one's upgrade canister is gone.
            if c.b("ALLIED_PDX") {
                c.remove_tag24("APGComputer");
                c.remove_tag24("AugUpgrade_WC");
            } else {
                c.remove_tag24("AugUpgrade_PDX");
            }
        }
        "24_SPACESTATION04" => {
            if !c.b("Ryan_Unbannable") {
                let v = c.b("Ryan_Dead") || c.b("MS_TurbinesStopped");
                c.set("Ryan_Unbannable", v, 25);
            }
            if c.b("QR_MAIN_TNM") && !c.b("MS_DebuggerRoom_Open") {
                c.trigger("DebuggerRoomHider");
                c.set("MS_DebuggerRoom_Open", true, 0);
            }
            if c.b("ALLIED_PDX") && !c.b("MS_RemoveROFLConsole") {
                c.remove_tag24("ROFLConsole");
                c.set("MS_RemoveROFLConsole", true, 0);
            }
        }
        _ => {}
    }
}

/// Kylie comes to the station once the player's side has done its part: to the interface room after the plants
/// are off, to the docks after the LEETs are gone.
fn kylie_arrives(c: &mut Ctx) {
    if c.b("MS_SpawnedKylie") || c.b("Kylie_Deleted") {
        return;
    }
    let dead = c.b("Kylie_Dead");
    if c.b("QC_MAIN_DisableAnniePlants") && !c.b("QC_MAIN_DestroyLEETs") {
        c.vehicle(ActorSel::tag("KylieShuttle"), true);
        c.leave_tag("InterfaceRoomAliens");
        c.leave_tag("InterfaceRoomBots");
        c.show("InterfaceRoomCorpses", true);
        c.enter_tag(if dead { "Kylie_Dead" } else { "Kylie" });
        crate::security::event("AIModuleCam03", false);
        c.set("MS_SpawnedKylie", true, 25);
    } else if c.b("QC_MAIN_DestroyLEETs") {
        c.vehicle(ActorSel::tag("KylieShuttle"), true);
        if c.mover_ids("DockDoor1").iter().any(|&i| c.movers.rt[i].key_num != 1) {
            c.trigger("DockDoor1Dispatcher");
        }
        if c.mover_ids("DockDoor2").iter().any(|&i| c.movers.rt[i].key_num != 0) {
            c.trigger("DockDoor2Dispatcher");
        }
        for t in ["DockingAlien", "HangarGuard01", "Fargo", "DockingBots"] {
            c.leave_tag(t);
        }
        c.show("DockingCorpses", true);
        c.enter_tag(if dead { "Kylie_Dock_Dead" } else { "Kylie_Dock" });
        c.set("MS_SpawnedKylie", true, 25);
    }
}

impl Ctx<'_> {
    fn remove_tag24(&self, tag: &str) {
        self.destroy_actors(ActorSel::tag(tag));
        crate::ai::world_req(WorldReq::Destroy(Sel::tag(tag)));
    }
}

/// Leaving a station map with Deus Diablo in tow: he waits safely behind.
pub(super) fn pre_travel(fl: &mut FlagBase) {
    if (fl.get_bool("DD_Following") || fl.get_bool("DD_Moving")) && !fl.get_bool("QC_MAIN_FindDD") {
        fl.set("MS_DDSafe", FlagValue::Bool(true), 0);
    }
}

pub(super) fn timer(c: &mut Ctx, map: &str) {
    let seen: Vec<usize> = super::super::DETECTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if !seen.is_empty() {
        if let Some(n) = map.strip_prefix("24_SPACESTATION") {
            c.set(&format!("NS_SpaceStation{n}"), true, 25);
        }
    }
    match map {
        "24_SPACESTATION01" => {
            let lead_done = (c.b("ALLIED_PDX") && c.b("QC_MAIN_DisableAILocks")) || (c.b("ALLIED_WC") && c.b("QC_MAIN_NarcissusLoop"));
            if !c.b("MS_SpawnedKKHologram") && lead_done {
                c.hide(ActorSel::tag("GSLogo2"), true);
                c.enter_tag("KK_Hologram2");
                c.set("MS_SpawnedKKHologram", true, 25);
            }
            if c.any(&["QC_MAIN_DisableAILocks", "QC_MAIN_NarcissusLoop"]) {
                c.remove_tag24("NarcissusLogo");
            }
            if !c.b("Autosaved_GameSpy") {
                crate::save::autosave("mission");
                c.set("Autosaved_GameSpy", true, 25);
            }
            firewalls(c, false);
        }
        "24_SPACESTATION02" => {
            if c.b("DeusDiablo_PDX_Played") && !c.b("QC_MAIN_FindDD") && !c.b("DeusDiablo_Dead") && c.count("TNMPawn", Some("DD")) == 0 {
                c.set("QC_MAIN_FindDD", true, 0);
                c.award(500, "strDeusAward");
                c.pm("PM_Kashue_DDSafe");
            }
            if c.b("DeusDiablo_Dead") && c.b("QR_MAIN_FindDD") && !c.b("QC_MAIN_FindDD") && !c.b("MS_FindDD_AwardGiven") {
                c.award(500, "strEffortAward");
                c.pm("PM_Kashue_DDDead");
                c.set("MS_FindDD_AwardGiven", true, 25);
            }
            if c.b("QC_Main_KillDD") && !c.b("MS_KillDD_AwardGiven") {
                c.award(500, "strEffortAward");
                c.set("MS_KillDD_AwardGiven", true, 25);
            }
        }
        "24_SPACESTATION03" => {
            let show_scara = (c.b("ALLIED_WC") && c.any(&["QC_MAIN_ActivateAPGs", "QC_MAIN_DestroyLEETs"]))
                || (c.b("ALLIED_PDX") && c.b("QC_MAIN_DisableAnniePlants") && !c.b("QC_MAIN_DestroyLEETs"));
            if !c.b("MS_SpawnedScaraHologram") && show_scara {
                c.hide(ActorSel::tag("GSLogo3"), true);
                c.enter_tag("Scara_Hologram");
                c.set("MS_SpawnedScaraHologram", true, 25);
            }
            if c.b("ALLIED_PDX") && c.b("QC_MAIN_DestroyLEETs") && !c.b("Kashue_RyanEnding_Played") {
                c.hide(ActorSel::tag("GSLogo3"), true);
                c.leave_tag("Scara_Hologram");
                c.enter_tag("KK_Hologram");
            }
            if c.b("MS_TurbinesStopped") {
                c.leave_tag("DD");
            }
            firewalls(c, true);
        }
        _ => {}
    }
    // The turbines are off: the station shakes itself apart.
    if c.b("MS_TurbinesStopped") {
        super::super::ship_explosions(c, true);
    }
}

/// The GameSpy firewalls come back 20 s after they're taken down, patrolling from their first point. On the
/// third floor a hacked IFF turns new ones to the player's side.
fn firewalls(c: &mut Ctx, iff: bool) {
    if !c.vars.inited {
        c.vars.last_killed = [-1.0; 12];
        c.vars.inited = true;
    }
    for (i, (sp, tag, patrol)) in FIREWALLS.iter().enumerate() {
        let n = c.pawns.iter().filter(|p| p.tag.eq_ignore_ascii_case(tag)).count();
        if n == 0 && c.vars.last_killed[i] == -1.0 {
            c.vars.last_killed[i] = c.now;
        }
        if n == 0 && c.now - c.vars.last_killed[i] > 20.0 {
            let Some((pos, yaw)) = c.point("SpawnPoint", sp) else { continue };
            let friendly = iff && ((i == 2 && c.b("M24_03_InvertIFF")) || (i < 2 && c.b("M24_03_InvertIFFLeet")));
            let (player, gamespy) = if friendly { (1.0, -1.0) } else { (-1.0, 1.0) };
            let props = vec![
                prop("Tag", Value::Name((*tag).into())),
                prop("Orders", Value::Name("Patrolling".into())),
                prop("OrderTag", Value::Name((*patrol).into())),
                prop("bKeepWeaponDrawn", Value::Bool(true)),
                prop("bAlwaysPatrol", Value::Bool(true)),
                Property { name: "InitialAlliances".into(), index: 6, value: Value::Alliance("Player".into(), player, true) },
                Property { name: "InitialAlliances".into(), index: 7, value: Value::Alliance("GameSpy".into(), gamespy, true) },
            ];
            if let Ok(mut q) = PAWNS.lock() {
                q.push(("FirewallGS".into(), pos, yaw, props));
            }
            c.fx(Fx::SpawnFlash(pos));
            crate::sound::play("DeusExSounds.Generic.Spark1", pos, 2.0, 2048.0, 2.0);
            c.vars.last_killed[i] = -1.0;
        }
    }
}

pub(super) fn pawn_killed(c: &mut Ctx, class: &str) {
    if crate::classtree::is_a(class, "Kylie") {
        c.set("Kylie_Dead_M24", true, 0);
    }
}

/// An airlock with one inner door: which dispatcher runs depends on whether that door is open.
fn airlock(c: &mut Ctx, n: u32) {
    let door = format!("EngineeringAirlock{n}Door1");
    let open = c.mover_ids(&door).iter().any(|&i| c.movers.rt[i].key_num != 0);
    let which = if open { 1 } else { 2 };
    c.trigger(&format!("EngineeringAirlock{n}Dispatcher{which}"));
}

pub(super) fn event(c: &mut Ctx, f: &str) -> bool {
    if let Some(n) = f.strip_prefix("EngineeringAirlock").and_then(|r| r.parse::<u32>().ok()) {
        airlock(c, n);
        return true;
    }
    // The newer airlocks: entering opens one door and closes the other; cycling swaps them through a dispatcher.
    if let Some(rest) = f.strip_prefix("EngineeringAirlock") {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let what = &rest[n.len()..];
        let (d1, d2) = (format!("NewEngineeringAirlock{n}Door1"), format!("NewEngineeringAirlock{n}Door2"));
        match what {
            "Entry" | "Exit" => {
                let (open, close) = if what == "Entry" { (&d1, &d2) } else { (&d2, &d1) };
                if c.mover_ids(open).iter().any(|&i| c.movers.rt[i].key_num == 0) {
                    c.trigger(open);
                }
                for i in c.mover_ids(close) {
                    crate::movers::move_to(&c.lvl.movers[i], &mut c.movers.rt[i], 0, 0.25);
                }
                return true;
            }
            "Cycle" => {
                if !c.b("M24_InAirlock") {
                    c.set("M24_InAirlock", true, 25);
                    let first_open = c.mover_ids(&d1).iter().any(|&i| c.movers.rt[i].key_num != 0);
                    if first_open {
                        c.trigger(&d1);
                        c.trigger(&format!("EngineeringAirlock{n}CycleIn"));
                    } else {
                        if c.mover_ids(&d2).iter().any(|&i| c.movers.rt[i].key_num != 0) {
                            c.trigger(&d2);
                        }
                        c.trigger(&format!("EngineeringAirlock{n}CycleOut"));
                    }
                }
                return true;
            }
            _ => {}
        }
    }
    match f {
        "SpawnDockedShuttle" => {
            c.vehicle(ActorSel::tag("ABISpaceShip"), true);
            c.vehicle(ActorSel::tag("ABIShuttleDocking"), false);
            if c.mover_ids("DockDoor2Outer").iter().any(|&i| c.movers.rt[i].key_num != 0) {
                for i in c.mover_ids("DockDoor2Outer") {
                    crate::movers::move_to(&c.lvl.movers[i], &mut c.movers.rt[i], 0, 0.1);
                }
            }
            c.trigger("FirstEntryDispatcher");
        }
        "DownloadSchematics" => {
            for img in ["TNMDV_SS03", "TNMDV_SS02", "TNMDV_SS01", "TNMDV_SS04"] {
                crate::consys::add_image(img);
            }
            c.setd("QC_MAIN_DownloadSchematics", true);
            c.pm(if c.b("ALLIED_WC") { "PM_Scara_GotSchematics" } else { "PM_Kashue_GotSchematics" });
        }
        "EnableAirPumps" => c.set("MS_SSAirPumpsEnabled", true, 25),
        "LEETRobotsDie" => crate::ai::world_req(WorldReq::Disable(Sel::tag("LEETRobot"))),
        "AnnieSpiderbotsDie" => crate::ai::world_req(WorldReq::Disable(Sel::tag("AnnieFarmSpiderBots"))),
        "APGSecBotDie" => crate::ai::world_req(WorldReq::Disable(Sel::tag("APGSecBot"))),
        "HoloDisappear" => {
            c.leave_tag("Scara_Hologram");
            c.leave_tag("KK_Hologram1");
            c.hide(ActorSel::tag("GSLogo1"), false);
        }
        "KKDisappear" => {
            c.leave_tag("KK_Hologram2");
            c.leave_tag("KK_Hologram");
            c.hide(ActorSel::tag("GSLogo2"), false);
            c.hide(ActorSel::tag("GSLogo3"), false);
        }
        "ScaraDisappear" => {
            c.leave_tag("Scara_Hologram");
            c.hide(ActorSel::tag("GSLogo3"), false);
        }
        "M24_FirewallIFF" => {
            c.ally(Sel::tag("GSFirewallSpawnee3"), 1.0, true);
            ally_with(c, "GSFirewallSpawnee3", "GameSpy", -1.0);
        }
        "M24_FirewallIFFLeet" => {
            for t in ["GSFirewallSpawnee1", "GSFirewallSpawnee2"] {
                c.ally(Sel::tag(t), 1.0, true);
                ally_with(c, t, "GameSpy", -1.0);
            }
        }
        "BreakTheGame" | "TrestAblaze" | "TrestPutOut" | "PlayGSSound" => {}
        _ => return false,
    }
    true
}

fn ally_with(_c: &Ctx, tag: &str, alliance: &str, level: f32) {
    crate::triggers::push_order(PawnOrder::Ally { sel: Sel::tag(tag), alliance: alliance.into(), level, permanent: true });
}
