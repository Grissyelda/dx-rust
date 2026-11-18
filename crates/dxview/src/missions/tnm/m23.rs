//! The Nameless Mod, Mission 23: the ABI complex in the snow. Written for DX RUST from how the mission behaves in
//! the player's own copy of the mod; flag, tag and map names are the identifiers its maps use, texts come from the
//! script's own strings at run time.
//!
//! Covered: arrival set-up for each map (the intro flight, the faction allies, the rescue side jobs, the archive
//! keypad's code, the boss fight in the ruins and being captured there, the labs' allies), the once-a-second checks
//! (side-job outcomes, the snow wolves, the mirror-Trestkon fight), the map's script functions and who noticed the
//! player (the "not seen" flags), the ABI game's item construction.
//!
//! Left out: the cinematic Cerberus guns and SAM site (their aiming movers and projectile points), the warp-zone
//! maze, the reversed music, snow following the intro camera.

use super::*;

const TIMERS: [(&str, &str); 4] = [
    ("TimerStarted_ABIGameItem7", "Timer_ABIGameItem7"),
    ("TimerStarted_ABIGameItem14", "Timer_ABIGameItem14"),
    ("TimerStarted_ABIGameItem17", "Timer_ABIGameItem17"),
    ("TimerStarted_ABIGameItem22", "Timer_ABIGameItem22"),
];

/// The archive keypad's possible codes (the map holds a placeholder; one of these is picked on arrival).
const ARCHIVE_CODES: [&str; 5] = ["3302", "7674", "5961", "8569", "1086"];

/// Wild Cards people the labs make friendly for a Wild Cards ally.
const LABS_WC: [&str; 12] = [
    "WCCommando", "WCCommando02", "OfficeHallWC", "CubiclesWC", "ResearchHallWC", "Commando01", "Commando02", "Commando03",
    "Commando04", "Commando05", "LoadingBayWC", "MIB",
];
const LABS_WC_BOTS: [&str; 4] = ["OfficeHallWC", "LoadingBayWC", "SecurityBot2WC", "SecurityBot3WC"];
/// ABI people who stand down once Ryan is on the player's side.
const LABS_ABI: [&str; 9] =
    ["ABIEliteGuard", "OfficeHallABI", "CubiclesABI", "LoadingBayABI", "ResearchHallABI", "SecurityBot2ABI", "MilitaryBotABI", "SpiderBot", "VentSpiderBot"];

/// The people who take the player alive in the ruins instead of killing him.
const CAPTORS: [&str; 4] = ["ZeroPresence", "ZPHenchman", "Slicer", "SlicerHenchman"];

pub(super) fn first(c: &mut Ctx, map: &str) {
    match map {
        "23_ABIAPPROACH" | "23_ABIEXTERIOR" => {
            if !c.b("MS_ABIIntro_Played") {
                if map == "23_ABIAPPROACH" {
                    c.trigger("Pilot1Scene1Disp");
                }
                c.trigger("ABIIntroCam");
                c.set("MS_ABIIntro_Played", true, 24);
                reprogrammed_pet(c, map == "23_ABIAPPROACH");
            }
            // For a Wild Cards ally the helicopter is Midashand's.
            if c.b("ALLIED_WC") && (map == "23_ABIAPPROACH" || !c.b("MS_ABIIntro_Done")) {
                crate::consys::bind_tag("ROFLCopter", "Midashand", "Midashand");
            }
            if map == "23_ABIEXTERIOR" {
                if c.b("ALLIED_WC") {
                    c.enter_tag("ricemanu");
                } else {
                    c.remove_tag23("RiceLaptop");
                }
                if c.b("QR_SUB_HelpRice") {
                    c.enter_tag("RiceAttackers");
                    c.show("RiceAttackers", true);
                    // Rice holds out until the player gets there.
                    c.heal_pawn("ricemanu", [100.0, 200.0, 125.0, 100.0, 250.0, 175.0]);
                    c.orders(Sel::tag("ricemanu"), "Standing", "");
                }
                if c.b("QC_SUB_FreeHao") {
                    c.leave_tag("Hao");
                }
            }
        }
        "23_ABIINTERIOR" => {
            let k = (c.rand() * ARCHIVE_CODES.len() as f64) as usize % ARCHIVE_CODES.len();
            crate::devices::set_code("Archives_Keypad", ARCHIVE_CODES[k]);
            c.pm("PM_EnterOffices");
            if !c.b("MS_EnterOffices_AwardGiven") {
                c.award(500, "strProgressAward");
                c.set("MS_EnterOffices_AwardGiven", true, 24);
            }
        }
        "23_ABIRUINS" => ruins_first(c),
        "23_ABILABS" => {
            c.pm("PM_EnterLabs");
            if !c.b("MS_FindLabs_AwardGiven") {
                c.award(750, "strProgressAwardTrio");
                c.set("MS_FindLabs_AwardGiven", true, 24);
            }
            if c.b("ALLIED_WC") {
                for t in LABS_WC.iter().chain(LABS_WC_BOTS.iter()) {
                    c.ally(Sel::tag(t), 1.0, false);
                }
                c.ally(Sel::isa("WCCommando"), 1.0, false);
                c.ally(Sel::isa("SecurityBot2WC"), 1.0, false);
                crate::security::event("*noalarm:LoadingBayCamera", true);
            }
        }
        "23_SLIPGATE" => c.hide(ActorSel::class_tag("DeusExPickup", "SurvivorSet"), true),
        _ => {}
    }
}

/// CerbFire / CerbFireBridge*: a Cerberus shell from every SpawnPoint with this tag, along its rotation (the bridge
/// volleys also sound the plasma shot once, heard everywhere).
fn cerb_fire(c: &mut Ctx, tag: &str, shot_sound: bool) {
    let mut first = true;
    for a in c.lvl.actors.iter() {
        if !crate::classtree::is_a(&a.class, "SpawnPoint") || !tag_of(&a.props).eq_ignore_ascii_case(tag) {
            continue;
        }
        let l = a.location();
        let at = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
        let rot = match unreal_pkg::props::find(&a.props, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => [0; 3],
        };
        if shot_sound && first {
            crate::sound::play("TNMSounds.OneShot.Plasma_Shot", at, 5.0, 65535.0, 1.0);
        }
        first = false;
        crate::missions::launch("CerberusProjectile", at, crate::staring::dir_of(rot));
    }
}

/// Mission Timer state kept between ticks on one map (cerbTimer: the bridge volley's cooldown; cerbCounter: how
/// long the idle guns have rested).
static CERB: std::sync::Mutex<(String, i32, i32)> = std::sync::Mutex::new((String::new(), 0, 0));

fn cerb_state(map: &str) -> (i32, i32) {
    let g = CERB.lock().unwrap_or_else(|e| e.into_inner());
    if g.0 == map {
        (g.1, g.2)
    } else {
        (0, 0)
    }
}

fn set_cerb_state(map: &str, timer: i32, counter: i32) {
    let mut g = CERB.lock().unwrap_or_else(|e| e.into_inner());
    *g = (map.to_string(), timer, counter);
}

/// The idle guns turn to a new random heading once they've rested three ticks (each that rests counts one).
/// `exterior`: the Cerberus guns (pneumatic sound while turning, a close sound on stopping, the barrel pitches);
/// else the approach's SAM launchers (yaw 23000..37000, quiet).
fn idle_turn(c: &mut Ctx, map: &str, tag: &str, exterior: bool) {
    let (timer, mut counter) = cerb_state(map);
    for (m, rot, rest) in crate::staring::with_tag(tag) {
        if rest {
            if counter == 0 {
                crate::staring::set_ambient(m, None, None);
                if exterior {
                    let at = c.lvl.movers[m].location;
                    crate::sound::play("MoverSFX.Pneumatic1Close", DVec3::new(at[0] as f64, at[1] as f64, at[2] as f64), 1.0, 1600.0, 1.0);
                }
            }
            counter += 1;
        }
        if counter >= 3 {
            let mut want = rot;
            if exterior {
                crate::staring::set_ambient(m, Some("MoverSFX.Pneumatic1Move"), None);
                want[1] = (c.rand() * 32767.0) as i32 + (c.rand() * 32767.0) as i32;
                let mut child = want;
                child[0] = 4096 - (c.rand() * 4096.0) as i32;
                crate::staring::turn_mover(m, want, Some(child));
            } else {
                want[1] = 23000 + (c.rand() * 14000.0) as i32;
                crate::staring::turn_mover(m, want, Some(want));
            }
            counter = 0;
        }
    }
    set_cerb_state(map, timer, counter);
}

/// NV Shacker's pet robot, knocked out on the second day, comes back reprogrammed on the player's side.
fn reprogrammed_pet(c: &mut Ctx, approach: bool) {
    let pet_down = c.b("NVShackerPet_Unconscious") && !(approach && c.b("NVShackerPet_Dead"));
    if c.b("ALLIED_WC") && pet_down {
        c.enter_tag("NVShackerPet_WC");
        c.set("NVShackerPet_ABI_WC", true, 24);
        c.set("NVShackerPet_Unconscious", false, 24);
        crate::tnmach::unlock("achA2Reprogram");
    }
    if c.b("ALLIED_PDX") && c.b("NVShacker_PDX") && pet_down {
        c.enter_tag("NVShackerPet_PDX");
        c.set("NVShackerPet_ABI_PDX", true, 24);
        c.set("NVShackerPet_Unconscious", false, 24);
        crate::tnmach::unlock("achA2Reprogram");
    }
}

fn ruins_first(c: &mut Ctx) {
    c.pm("PM_EnterRuins");
    if !c.b("PM_EnterOffices_Played") {
        crate::tnmach::unlock("achRuinsFast");
    }
    if !c.b("MS_ReachRuins_AwardGiven") {
        c.award(500, "strProgressAwardDuo");
        c.set("MS_ReachRuins_AwardGiven", true, 24);
    }
    // The boss of the ruins is the player's rival from the other faction.
    if c.b("ALLIED_PDX") && !c.b("Zero_Dead") {
        c.enter_tag("ZeroPresence");
        c.enter_tag("ZPHenchman");
        c.leave_tag("ChapelGuard");
        c.show("RicePlatoonCorpse", false);
        c.set("ABIBossFight", true, 24);
    } else if c.b("ALLIED_WC") && !c.b("Slicer_Dead") {
        c.enter_tag("Slicer");
        c.enter_tag("SlicerHenchman");
        c.leave_tag("ChapelGuard");
        c.set("ABIBossFight", true, 24);
    }
    if !c.b("fixedbots_Dead") && c.b("fixedbots_Disabled") {
        crate::ai::world_req(WorldReq::Disable(Sel::tag("fixedbot")));
    }
    if c.b("MS_PlayerCaptured") {
        // Woken up in a cell: a quarter of the health, everything taken and laid out in the storeroom, the dungeon
        // guarded by the captor's side.
        player_req(PlayerReq::Wounds([0.0; 6], [25.0; 6]));
        let spots = c.points("SpawnPoint", "player_inv");
        player_req(PlayerReq::DropInventory(spots));
        c.remove_tag23("DungeonStorageBoon");
        c.leave_tag("DungeonGuard_ABI");
        c.show("DungeonGuard_ABI", false);
        c.show("ABIDungeonGuardCorpse", true);
        let (ours, theirs, boss) = if c.b("ALLIED_PDX") {
            (["DungeonGuard_WC", "CellGuard_WC"], ["DungeonGuard_PDX", "CellGuard_PDX"], ["ZeroPresence", "ZPHenchman"])
        } else {
            (["DungeonGuard_PDX", "CellGuard_PDX"], ["DungeonGuard_WC", "CellGuard_WC"], ["Slicer", "SlicerHenchman"])
        };
        for t in ours {
            c.enter_tag(t);
        }
        for t in theirs {
            crate::ai::world_req(WorldReq::Destroy(Sel::tag(t)));
        }
        for t in boss {
            c.leave_tag(t);
        }
        c.show(boss[0], false);
        c.frobbable("CellDoor");
        c.pm("PM_PrisonCell");
    }
}

impl Ctx<'_> {
    fn remove_tag23(&self, tag: &str) {
        self.destroy_actors(ActorSel::tag(tag));
        crate::ai::world_req(WorldReq::Destroy(Sel::tag(tag)));
    }
    /// Each body part at least this much (head, torso, left leg, right leg, left arm, right arm).
    fn heal_pawn(&self, tag: &str, min: [f32; 6]) {
        crate::ai::world_req(WorldReq::MinParts(Sel::tag(tag), min));
    }
}

pub(super) fn timer(c: &mut Ctx, map: &str) {
    for (s, l) in TIMERS {
        c.timer_tick(s, l);
    }
    game_item_timers(c);
    let seen: Vec<usize> = super::super::DETECTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    if !seen.is_empty() {
        let flag = match map {
            "23_ABIEXTERIOR" => Some("NS_ABIExterior"),
            "23_ABIINTERIOR" => Some("NS_ABIInterior"),
            "23_ABIRUINS" => Some("NS_ABIRuins"),
            "23_ABILABS" => Some("NS_ABILabs"),
            _ => None,
        };
        if let Some(f) = flag {
            c.set(f, true, 24);
        }
    }
    match map {
        "23_ABIINTERIOR" => {
            // The archive burnt down (no fire starter left) for the Wild Cards; both files found for PDX.
            let starters = c.lvl.actors.iter().enumerate().any(|(i, a)| tag_of(&a.props).eq_ignore_ascii_case("FireStarter") && !crate::triggers::spent(i));
            if c.b("ALLIED_WC") && !c.b("PM_ScaraTorchArchive_Played") && !c.b("MS_GatherInfo_AwardGiven") && !starters {
                c.pm("PM_ScaraTorchArchive");
                c.award(500, "strPyromanicAward");
                c.set("MS_GatherInfo_AwardGiven", true, 24);
            }
            if c.b("ALLIED_PDX") && c.b("QC_SUB_FindABIBuildingPlans") && c.b("QC_SUB_FindABIEvidence") && !c.b("MS_GatherInfo_AwardGiven") {
                c.award(500, "strBlackmailAward");
                c.set("MS_GatherInfo_AwardGiven", true, 24);
            }
        }
        "23_ABIAPPROACH" => {
            if !c.b("SnowWolvesDead") && c.count("SnowWolf", Some("SnowWolfThreat")) == 0 {
                c.set("SnowWolvesDead", true, 24);
                if !c.b("MS_WolfGuy_GoalClosed") && c.b("QR_SUB_SnowWolves") {
                    c.add_goal("SnowWolves", "strWolfGuyMidway", false, false);
                }
            }
            if c.b("WolfGuy_Dead") && c.b("QR_SUB_SnowWolves") && !c.b("QC_SUB_SnowWolves") && !c.b("MS_WolfGuy_GoalClosed") {
                c.add_goal("SnowWolves", "strWolfGuyDead", false, true);
                c.set("MS_WolfGuy_GoalClosed", true, 24);
            }
            if c.b("QC_SUB_SnowWolves") && !c.b("MS_WolfGuy_VoucherAdded") {
                let n = c.int("ABI_VoucherUses");
                c.set_int("ABI_VoucherUses", n + 1, 25);
                c.add_goal("SnowWolves", "strWolfGuyComplete", false, true);
                c.set("MS_WolfGuy_VoucherAdded", true, 24);
            }
            // The Cerberus guns fire on a player crossing the bridge (unless their spotter is down), then rest ten
            // ticks.
            let (mut timer, counter) = cerb_state(map);
            let on = ["ABIOnBridge", "ABIOnBridge2", "ABIOnBridge3"];
            let detectable = !crate::cinematic::active();
            if timer == 0 && on.iter().any(|f| c.b(f)) && detectable && !c.down("ABIArtyGuy") {
                let tag = if c.b("ABIOnBridge") {
                    "CerbProjSpawnBridge"
                } else if c.b("ABIOnBridge2") {
                    "CerbProjSpawnBridge2"
                } else {
                    "CerbProjSpawnBridge3"
                };
                cerb_fire(c, tag, true);
                timer += 1;
            } else if timer != 0 {
                timer += 1;
                if timer >= 10 {
                    timer = 0;
                }
            }
            set_cerb_state(map, timer, counter);
            idle_turn(c, map, "IdleSAM", false);
            if !c.b("Autosaved_ABI") {
                crate::save::autosave("mission");
                c.set("Autosaved_ABI", true, 24);
            }
        }
        "23_ABIEXTERIOR" => {
            idle_turn(c, map, "Cerberus", true);
            if c.b("QC_SUB_FreeHao") && !c.b("MS_FreeHao_AwardGiven") {
                c.award(250, "strFriendAward");
                c.set("MS_FreeHao_AwardGiven", true, 24);
                crate::tnmach::unlock("achHao");
            }
            if c.b("Ricemanu_Dead") && c.b("QR_SUB_HelpRice") {
                c.pm("PM_RiceDies");
            }
            if c.b("Hao_Dead") && c.b("QR_SUB_FreeHao") && !c.b("QC_SUB_FreeHao") && !c.b("MS_Hao_GoalClosed") {
                c.add_goal("FreeHao", "strHaoDead", false, true);
                c.set("MS_Hao_GoalClosed", true, 24);
            }
            // What Hao's escape needs: the factory's lasers gone and its three cameras off.
            let lasers = crate::security::lasers_left("FactoryLasers");
            if lasers == 0 {
                c.set("Hao_LasersDisabled", true, 24);
            }
            let cams_on = ["FactoryMainDoorCam", "FactoryMainRoomCam", "FactoryTunnelCam"].iter().any(|t| crate::security::camera_active(t));
            c.set("Hao_CamerasDisabled", !cams_on, 24);
        }
        "23_ABIRUINS" => ruins_timer(c),
        "23_ABILABS" => {
            if !c.b("ALLIED_Ryan") && c.any(&["Ryan_Dead", "Hitman_Dead"]) {
                c.trigger("RyanGoHostile");
                c.trigger("HitmenGoHostile");
            }
            if c.b("ALLIED_Ryan") {
                for t in LABS_ABI {
                    c.ally(Sel::tag(t), 0.0, true);
                }
            }
            if c.b("ALLIED_WC") && !c.b("ClearedLoadingBay") {
                let left = c.pawns.iter().filter(|p| p.tag.eq_ignore_ascii_case("LoadingBayABI") && !(crate::classtree::is_a(&p.class, "Robot") && p.emp_hp <= 0.0)).count();
                if left == 0 {
                    c.pm("PM_ScaraClearLoadingBay");
                }
            }
            if c.b("MS_SpawnABIGameItem") {
                c.fl.delete("MS_SpawnABIGameItem");
                start_game_item(c);
            }
            if !c.b("MS_achSlicerGhost") && c.b("ALLIED_WC") && !c.b("Slicer_Dead") {
                crate::tnmach::unlock("achSlicerGhost");
                c.set("MS_achSlicerGhost", true, 24);
            } else if !c.b("MS_achZPGhost") && c.b("ALLIED_PDX") && !c.b("Zero_Dead") {
                crate::tnmach::unlock("achZPGhost");
                c.set("MS_achZPGhost", true, 24);
            }
            if !c.b("MS_AchKillWCID") && c.b("AgentBrown_Dead") && c.b("AgentJones_Dead") && c.b("WCIDAgent_Dead") {
                crate::tnmach::unlock("achWCID");
                c.set("MS_AchKillWCID", true, 24);
            }
            if !c.b("MS_AchABIStealth") && c.b("ABIShuttle_ReadyToLaunch") && !c.b("KilledABI") {
                crate::tnmach::unlock("achABIStealth");
                c.set("MS_AchABIStealth", true, 24);
            }
        }
        _ => {}
    }
}

fn ruins_timer(c: &mut Ctx) {
    // Killed by the boss's side after his taunt: the player wakes up in their cell instead.
    if c.player_dead && c.b("MS_ABIBossTauntSent") && !c.b("MS_PlayerCaptured") {
        let by = crate::ai::last_player_hitter().and_then(|i| c.lvl.actors.get(i)).map(|a| tag_of(&a.props)).unwrap_or_default();
        if CAPTORS.iter().any(|t| t.eq_ignore_ascii_case(&by)) {
            c.set("MS_PlayerCaptured", true, 0);
            crate::tnmach::unlock(if c.b("ALLIED_PDX") { "achZPCapture" } else { "achSlicerCapture" });
            player_req(PlayerReq::Wounds([0.0; 6], [25.0; 6]));
            player_req(PlayerReq::Revive);
            c.send_player("23_ABIRuins#PlayerCellSpawnPoint");
        }
    }
    let down = |c: &Ctx, n: &str| c.down(n);
    if down(c, "Treskton") && down(c, "TresktonMirror1") && down(c, "TresktonMirror2") && down(c, "TresktonMirror3") && !c.b("MS_MirrorTresksKilled") {
        c.remove_tag23("TrestkonDummy");
        c.destroy_actors(ActorSel::class("TrestkonDummy"));
        c.mover_to23("TresktShield", 1);
        c.enter_tag("TreskReal");
        c.add_goal("KillTresk", "strKillTresk", false, false);
        c.set("MS_MirrorTresksKilled", true, 24);
    }
    if c.b("MS_MirrorTresksKilled") && c.b("TresktonReal_Dead") && !c.b("MS_MirrorDummyKilled") {
        c.set("MS_MirrorDummyKilled", true, 24);
        crate::tnmach::unlock("achTreskton");
        c.award(250, "strTresktonAward");
        c.add_goal("KillTresk", "strKillTreskDone", false, true);
        c.trigger("TreskPortal");
        c.mover_to23("RealmLockShield", 0);
    }
}

impl Ctx<'_> {
    fn mover_to23(&mut self, tag: &str, key: u8) {
        for i in self.mover_ids(tag) {
            crate::movers::move_to(&self.lvl.movers[i], &mut self.movers.rt[i], key, 0.0);
        }
    }
}

/// The ABI game's prize is built by a UC: the satellites start, then at 7 s the item appears in the beam (it falls
/// when the beam goes off at 22 s).
fn start_game_item(c: &mut Ctx) {
    c.trigger("ABISattGenerators");
    for p in c.points_any("ABIGunSpawn") {
        crate::sound::play("TNMSounds.OneShot.UCWarmup", p, 1.0, 1600.0, 1.0);
    }
    for (secs, s) in [(7, "7"), (14, "14"), (17, "17"), (22, "22")] {
        c.timer_add(&format!("TimerStarted_ABIGameItem{s}"), &format!("Timer_ABIGameItem{s}"), secs);
    }
}

impl Ctx<'_> {
    fn points_any(&self, tag: &str) -> Vec<DVec3> {
        self.lvl
            .actors
            .iter()
            .filter(|a| tag_of(&a.props).eq_ignore_ascii_case(tag))
            .map(|a| {
                let l = a.location();
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            })
            .collect()
    }
}

fn game_item_timers(c: &mut Ctx) {
    let done = |c: &Ctx, s: &str| c.timer_done(&format!("TimerStarted_ABIGameItem{s}"), &format!("Timer_ABIGameItem{s}"));
    let del = |c: &mut Ctx, s: &str| c.timer_delete(&format!("TimerStarted_ABIGameItem{s}"), &format!("Timer_ABIGameItem{s}"));
    if done(c, "7") {
        del(c, "7");
        for p in c.points_any("ABIGunSpawn") {
            crate::dynlight::explosion(p, 8.0, 128, 96);
            c.fx(Fx::Sphere(p, 4.0));
            crate::sound::play("TNMSounds.OneShot.UCConstruct", p, 1.0, 1600.0, 1.0);
        }
        // The prize the player picked in the game (Trestkon's ABISpawnItem), else its class default.
        let item = c.int_text("ABISpawnItem").unwrap_or_else(|| "WeaponTNMRiotgun".to_string());
        c.spawn_item(&item, "ABIGunSpawn");
        c.trigger("UCElectricityEmitter");
        c.trigger("UCDamageTrigger");
    }
    if done(c, "14") {
        del(c, "14");
    }
    if done(c, "17") {
        del(c, "17");
        c.trigger("ABISattGenerators");
    }
    if done(c, "22") {
        del(c, "22");
        c.untrigger("UCElectricityEmitter");
        c.trigger("UCDamageTrigger");
    }
}

impl Ctx<'_> {
    /// A Name flag's value (the ABI game stores its prize as one).
    fn int_text(&self, n: &str) -> Option<String> {
        match self.fl.get(n) {
            Some(FlagValue::Name(s)) if !s.is_empty() => Some(s.rsplit('.').next().unwrap_or(s).to_string()),
            _ => None,
        }
    }
}

/// The player killed an ABI employee who wasn't stunned: the ABI-stealth outcome is lost.
pub(super) fn pawn_killed(c: &mut Ctx, class: &str, knocked_out: bool) {
    if !knocked_out && crate::classtree::is_a(class, "GenericABI") {
        c.set("KilledABI", true, 25);
    }
}

pub(super) fn event(c: &mut Ctx, f: &str) -> bool {
    match f {
        "LandedChopperAppear" => {
            c.vehicle(ActorSel::tag("ROFLCopterLanding"), false);
            c.vehicle(ActorSel::tag("IntroChopper1PilotScene3"), false);
            c.vehicle(ActorSel::tag("ROFLCopter"), true);
            c.pm("PM_InitialMessage");
            c.set("MS_ABIIntro_Done", true, 24);
            to_start(c);
        }
        "ChopperSplode" => {
            if let Some((pos, _)) = c.deco_pos("Vehicles", "ROFLCopterCrashing") {
                for _ in 0..20 {
                    let at = pos + c.vrand() * 120.0;
                    let size = if c.rand() < 0.25 { "ExplosionMedium" } else { "ExplosionLarge" };
                    c.fx(Fx::Explosion(at, size));
                }
                for _ in 0..20 {
                    let at = pos + c.vrand() * 120.0;
                    let scale = (5.0 + 2.0 * c.rand()) as f32;
                    let smoking = c.rand() < 0.5;
                    c.fx(Fx::Fragment { pos: at, speed: 40000.0, spread: 512.0, scale, skin: None, smoking });
                }
                c.fx(Fx::Sphere(pos, 32.0));
                c.sound("LargeExplosion2");
            }
            c.vehicle(ActorSel::tag("ROFLCopterCrashing"), false);
            chopper_wreck(c);
        }
        "SpawnChopperWreck" => chopper_wreck(c),
        "MidasDropStaff" => c.spawn_item("WeaponFireStaff", "FirestaffSpawnPoint"),
        "DockHatchDisable" => {
            for i in c.mover_ids("DockControlHatch") {
                c.movers.rt[i].frobbable = Some(false);
            }
        }
        // The pet's charge mode is a flag on the robot class in the mod; its normal AI covers it here.
        "NVSPet_Charge" | "NVSPet_DeCharge" => {}
        "MirrorImage" => {
            if !c.b("MS_SpawnedTreskton") {
                if let Some((pos, yaw)) = c.point("PathNode", "TresktonSpawn").or_else(|| c.points_any("TresktonSpawn").first().map(|p| (*p, 0))) {
                    let props = vec![prop("Tag", Value::Name("Treskton".into())), prop("Orders", Value::Name("Standing".into()))];
                    if let Ok(mut q) = PAWNS.lock() {
                        q.push(("Treskton".into(), pos, yaw, props));
                    }
                    c.fx(Fx::SpawnFlash(pos));
                }
                c.set("MS_SpawnedTreskton", true, 24);
            }
        }
        "CheckRiceStatus" => {
            if c.b("ALLIED_WC") && c.b("QR_SUB_HelpRice") && !c.b("QC_SUB_HelpRice") {
                crate::triggers::set_active("PMFoundPassword", false);
            }
        }
        // The intro's Cerberus gun turns to face the bridge, its barrel aims, the dispatcher fires it.
        "RotateCineCerb" => {
            for (m, _, _) in crate::staring::with_tag("CerberusCine") {
                crate::staring::turn_mover(m, [0, 0, 0], None);
                crate::staring::set_ambient(m, Some("MoverSFX.Pneumatic1Move"), Some(255.0));
            }
            for (m, _, _) in crate::staring::with_tag("CerbBarrel") {
                crate::staring::turn_mover(m, [0, 0, 0], None);
            }
        }
        "AimCineCerb" => {
            for (m, _, _) in crate::staring::with_tag("CerbBarrel") {
                crate::staring::turn_mover(m, [5000, 0, 0], None);
            }
            c.trigger("CerbFireDispatch");
        }
        // (The script means to quiet the gun here, but clears another actor's sound: it keeps turning noisily.)
        "FireCineCerb" => crate::staring::fire_tag("CineCerbSpawn"),
        "SAMFire" => {
            crate::staring::fire_tag("SAMRocketSpawn");
            crate::sound::play_player("DeusExSounds.Weapons.RocketIgnite", 5.0, 65535.0);
        }
        "CerbFire" => cerb_fire(c, "CerbProjSpawn", false),
        "CerbFire2" => cerb_fire(c, "CerbProjSpawn2", false),
        // Each RoomEntry warp zone leads to one of the three rooms or nowhere.
        "WarpZoneRandomize" => {
            let entries: Vec<String> = c
                .lvl
                .zone_rules
                .iter()
                .filter_map(|z| z.warp.as_ref())
                .filter(|w| w.tag.eq_ignore_ascii_case("RoomEntry"))
                .map(|w| w.this_tag.clone())
                .collect();
            for e in entries {
                let k = ((c.rand() * 4.0) as usize).min(3);
                crate::warpzone::relink(&e, ["Room1", "Room2", "Room3", "None"][k]);
            }
        }
        "ReverseMusic" | "UnReverseMusic" => {}
        "SpawnMirrorTresk1" => c.enter_tag("Tresk1"),
        "SpawnMirrorTresk2" => c.enter_tag("Tresk2"),
        "SpawnMirrorTresk3" => c.enter_tag("Tresk3"),
        "TreskAppear" => {
            c.enter_tag("TreskInitial");
            c.add_goal("KillTresk", "strKillTreskStart", false, false);
        }
        "RevealPowerups" => {
            for class in ["QuadDamage", "RingofShadows"] {
                c.hide(ActorSel::class(class), false);
                for (_, cl, _, p) in c.decos.clone() {
                    if crate::classtree::is_a(&cl, class) {
                        crate::dynlight::explosion(p, 8.0, 128, 96);
                    }
                }
            }
            c.enter_tag("SurvivorSet");
            c.show("SurvivorSet", true);
        }
        "GrayAmbush" => c.enter_tag("AmbushGrays"),
        _ => return false,
    }
    true
}

/// The crash: the wreck appears, the player is badly hurt and starts at the map's PlayerStart spawn point.
fn chopper_wreck(c: &mut Ctx) {
    c.vehicle(ActorSel::tag("ROFLCopterCrashing"), false);
    c.vehicle(ActorSel::tag("ROFLCopterCrashed"), true);
    c.sound("MediumExplosion1");
    player_req(PlayerReq::Wounds([0.25, 0.25, 0.25, 0.0, 0.0, 0.25], [10.0, 10.0, 0.0, 0.0, 0.0, 0.0]));
    c.add_goal("FlyTheChopper", "strGoalChopper", true, true);
    c.set("MS_ABIIntro_Done", true, 24);
    to_start(c);
}

fn to_start(c: &mut Ctx) {
    if let Some((pos, yaw)) = c.point("SpawnPoint", "PlayerStart") {
        let _ = yaw;
        player_req(PlayerReq::Teleport(pos));
    }
}
