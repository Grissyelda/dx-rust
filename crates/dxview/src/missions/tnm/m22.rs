//! The Nameless Mod, Mission 22: the second day in Forum City. Written for DX RUST from how the mission behaves in
//! the player's own copy of the mod; the flag, tag and map names are the identifiers its maps and conversations
//! use, and every text comes from the script's own strings at run time.
//!
//! Covered: each map's set-up on arrival, the once-a-second checks (quest outcomes, the hostage crisis in the slums,
//! the kidnapping and the demolition squad, the air-traffic-control landing, the Goat and Llama cult jobs, Kevo's
//! rescue), the map's script functions (holograms, the torture room, the crash, the Citadel lava, Firewall Alpha's
//! four phases, the counter puzzle), the UC constructions (registration room items, the quad rocket launcher) and
//! who notices the player (PlayerDetected).
//!
//! Also: AJ's change of clothes (reskin.rs), Sol's bar burnt out (tnmmode's level fixup), the other ATM account
//! holders paid on the second day, the Citadel's random warp-zone exits and its falling rock and lava effects.

use super::*;

/// Mission 22's countdowns: (started flag, seconds-left flag).
const TIMERS: [(&str, &str); 13] = [
    ("TimerStarted_SpawnMidasChopper", "Timer_SpawnMidasChopper"),
    ("TimerStarted_RegistrationUCItem7", "Timer_RegistrationUCItem7"),
    ("TimerStarted_RegistrationUCItem14", "Timer_RegistrationUCItem14"),
    ("TimerStarted_RegistrationUCItem17", "Timer_RegistrationUCItem17"),
    ("TimerStarted_RegistrationUCItem22", "Timer_RegistrationUCItem22"),
    ("TimerStarted_RegistrationSecretUCItem7", "Timer_RegistrationSecretUCItem7"),
    ("TimerStarted_RegistrationSecretUCItem14", "Timer_RegistrationSecretUCItem14"),
    ("TimerStarted_RegistrationSecretUCItem17", "Timer_RegistrationSecretUCItem17"),
    ("TimerStarted_RegistrationSecretUCItem22", "Timer_RegistrationSecretUCItem22"),
    ("TimerStarted_Quad7", "Timer_Quad7"),
    ("TimerStarted_Quad14", "Timer_Quad14"),
    ("TimerStarted_Quad17", "Timer_Quad17"),
    ("TimerStarted_Quad22", "Timer_Quad22"),
];

/// The faction pawns of the corporate district: Wild Cards (pawn tags, robot tags) and PDX.
const WC_PAWNS: [&str; 8] = [
    "WCSecurity", "WCCommando", "WCThug", "WCIDAgent", "PDXHQAttackGroup", "OldHQAttackGroup", "Checkpoint1AttackGroup",
    "Checkpoint2AttackGroup",
];
const WC_BOTS: [&str; 5] = ["SecurityBot2WC", "WCHQPatrolBot", "MilitaryBotWC", "PDXHQAttackGroup", "SecurityBot3WC"];
const PDX_PAWNS: [&str; 2] = ["Firewall", "FireWall2"];
const PDX_BOTS: [&str; 3] = ["SecurityBot2PDX", "MilitaryBotPDX", "SecurityBot3PDX"];

const HOLOCOMS: [&str; 4] = ["KylieHolocom", "ZeroHolocom", "PhasmatisHolocom", "SlicerHolocom"];
const ATC_GUARDS: [&str; 5] = ["ATCGuard", "ATCGuardAlert1", "ATCGuardAlert2", "ATCGuardAlert3", "ATCGuardAlert4"];

/// Firewall Alpha's reinforcements (the script's spawnData): (spawn point tag, class, pawn tag) - two spydrones,
/// and Alpha himself when he's been taken down.
const ALPHA_SPAWNS: [(&str, &str, &str); 3] = [
    ("SpydroneSpawn1", "SpydronePawn", "SpydroneSpawnee1"),
    ("SpydroneSpawn2", "SpydronePawn", "SpydroneSpawnee2"),
    ("AlphaSpawnPoint", "FireWallAlpha", "FirewallAlpha"),
];

impl Ctx<'_> {
    fn hostile_all(&self, tags: &[&str]) {
        for t in tags {
            self.hostile(t);
        }
    }
    fn friendly_all(&self, tags: &[&str]) {
        for t in tags {
            self.ally(Sel::tag(t), 1.0, true);
        }
    }
    fn neutral_all(&self, tags: &[&str], permanent: bool) {
        for t in tags {
            self.ally(Sel::tag(t), 0.0, permanent);
        }
    }
    /// Everything with this Tag gone for good (pawns, decorations, bodies, items).
    fn remove_tag(&self, tag: &str) {
        self.destroy_actors(ActorSel::tag(tag));
        crate::ai::world_req(WorldReq::Destroy(Sel::tag(tag)));
    }
    /// Pawns with this Tag still around.
    fn n(&self, tag: &str) -> usize {
        // By Tag only: a class only spawned at run time may not be in the class tree's map list.
        self.pawns.iter().filter(|p| p.tag.eq_ignore_ascii_case(tag)).count()
    }
    /// Pawns with any of these Tags still around (robots in the Disabled state left out when `skip_disabled`).
    fn n_any(&self, tags: &[&str], skip_disabled: bool) -> usize {
        self.pawns
            .iter()
            .filter(|p| tags.iter().any(|t| p.tag.eq_ignore_ascii_case(t)))
            .filter(|p| !(skip_disabled && crate::classtree::is_a(&p.class, "Robot") && p.emp_hp <= 0.0))
            .count()
    }
    fn has_item(&self, class: &str) -> bool {
        self.inv_classes.iter().any(|k| k.eq_ignore_ascii_case(class))
    }
    /// Movers with this Tag moved to keyframe `key` over `secs`.
    fn mover_to(&mut self, tag: &str, key: u8, secs: f64) {
        for i in self.mover_ids(tag) {
            crate::movers::move_to(&self.lvl.movers[i], &mut self.movers.rt[i], key, secs);
        }
    }
    /// SetEnvironment: the bulletin that fits how the player has played Forum City so far.
    fn set_environment(&self) {
        if self.any(&["Killer_PDX", "Killer_FC"]) {
            self.show("KillerBulletin", true);
            self.leave_tag("GenericPawn");
        } else if self.b("ALLIED_WC") {
            self.show("WCBulletin", true);
        } else {
            self.show("PDXBulletin", true);
        }
    }
    fn escape_slums_goal(&mut self) {
        if !self.b("MS_EscapeSlums_AwardGiven") {
            self.add_goal("EscapeSlums", "strEscapeGoal", true, true);
            self.award(1000, "strSurvivalAward");
            self.set("MS_EscapeSlums_AwardGiven", true, 23);
        }
    }
    /// The player's ID was seen through: the air-traffic-control guards and robot turn on him and the cameras
    /// remember again.
    fn atc_void_id(&mut self) {
        self.hostile_all(&ATC_GUARDS);
        self.hostile("ATCBot");
        self.set("MS_ATCVoidID", true, 23);
    }
    /// A pawn appears at a spawn point with the UC flash (ExplosionLight, ellipse, sphere, arcs, spark).
    fn flash_at(&self, pos: DVec3) {
        self.fx(Fx::SpawnFlash(pos));
        crate::sound::play("DeusExSounds.Generic.Spark1", pos, 2.0, 2048.0, 2.0);
    }
    fn uc_sound(&self, at_tag: &str, which: &str) {
        for p in self.tag_points(at_tag) {
            crate::sound::play(&format!("TNMSounds.OneShot.{which}"), p, 1.0, 1600.0, 1.0);
        }
    }
    /// Every actor with this Tag: where it stands.
    fn tag_points(&self, tag: &str) -> Vec<DVec3> {
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
    /// A pawn put into the world by the script, attacking the player alongside the firewalls.
    fn spawn_firewall_side(&self, class: &str, tag: &str, pos: DVec3, yaw: i32, player: f32) {
        let orders = if player < 0.0 { "Attacking" } else { "Standing" };
        let props = vec![
            prop("Tag", Value::Name(tag.into())),
            prop("Orders", Value::Name(orders.into())),
            prop("bKeepWeaponDrawn", Value::Bool(true)),
            prop("bAlwaysPatrol", Value::Bool(true)),
            Property { name: "InitialAlliances".into(), index: 6, value: Value::Alliance("Player".into(), player, true) },
            Property { name: "InitialAlliances".into(), index: 7, value: Value::Alliance("FireWalls".into(), 1.0, true) },
        ];
        if let Ok(mut q) = PAWNS.lock() {
            q.push((class.to_string(), pos, yaw, props));
        }
    }
}

// ------------------------------------------------------------------------------------------------ FirstFrame

pub(super) fn first(c: &mut Ctx, map: &str) {
    // AJ keeps the clothes he changed into (the original's saved MultiSkins; ours are put back on arrival).
    if c.b("MS_AJChangedClothes") {
        crate::reskin::request_tag("AJ", script_skins("AJChangeClothes"));
    }
    if map.starts_with("22_FCSLUMS") {
        slums_first(c);
    }
    // uc: tnmMission22.CitadelRandomize
    // The Citadel (inside the apartment map) picks its maze's exits once before the lava rises and again after.
    if map == "22_PHASAPARTMENT" {
        let lava = c.b("Citadel_LavaRaised");
        let first = if lava { "Citadel_SecondRandomize" } else { "Citadel_FirstRandomize" };
        if !c.b(first) {
            citadel_randomize(c, lava);
            c.set(first, true, 24);
        }
    }
    match map {
        "22_FCDOWNTOWN" => {
            c.set_environment();
            if c.b("ALLIED_WC") {
                for t in ["SolsWitness", "SolsFireWall", "AlphaOperator", "SolsPlot", "SolBomber", "BomberVan"] {
                    c.remove_tag(t);
                }
                c.seal_door("SolsDoor");
                c.seal_door("SolsBackDoor");
                c.hostile_all(&["Firewall", "MilitaryBotPDX", "SecurityBot2PDX"]);
            }
            if c.b("QC_SUB_Bugging") && !c.down("PJ") {
                c.enter_tag("PimpJojo");
                c.pm("PM_PJ_Directions");
            }
            c.escape_slums_goal();
            let last = c.last_map();
            if last == "22_GOATCITY" && c.b("QC_CULT_KillAthena") && !c.b("KillCount_GoatCityDone") {
                c.set("KillCount_GoatCityDone", true, 23);
            }
            if last == "22_LLAMATEMPLE" && c.b("QC_CULT_KillBeefman") && !c.b("KillCount_LlamaTempleDone") {
                c.set("KillCount_LlamaTempleDone", true, 23);
            }
            if c.b("NVShackerPet_Abom") && !c.b("MS_NVShackerPetEntered") {
                c.enter_tag("NVShackerPet");
                c.set("MS_NVShackerPetEntered", true, 23);
            }
        }
        "22_FCCORPORATE" => {
            c.set_environment();
            if c.b("ALLIED_PDX") {
                c.hostile_all(&WC_PAWNS);
                c.hostile_all(&WC_BOTS);
                c.friendly_all(&PDX_PAWNS);
                c.friendly_all(&PDX_BOTS);
            } else if c.b("ALLIED_WC") {
                c.friendly_all(&WC_PAWNS);
                c.friendly_all(&WC_BOTS);
                c.hostile_all(&PDX_PAWNS);
                c.hostile_all(&PDX_BOTS);
                for d in ["PDXHQDoors", "PDXSideDoor1", "PDXSideDoor2"] {
                    c.seal_door(d);
                }
            }
            if c.b("StoleFromWeaponShop") && !(c.b("Raving_Dead") && c.b("Andreus_Dead")) {
                c.enter_tag("RavingThug");
            }
            c.escape_slums_goal();
            c.pm("PM_WolfySummons");
            if c.any(&["QR_MAIN_FreeKevo", "QR_MAIN_KidnapKevo"]) {
                if c.b("ALLIED_CULT_GOAT") && !c.any(&["QR_CULT_KillBeefman", "QC_CULT_KillBeefman"]) {
                    c.pm("PM_DT_QuestSummon");
                } else if c.b("ALLIED_CULT_LLAMA") && !c.any(&["QR_CULT_KillAthena", "QC_CULT_KillAthena"]) {
                    c.pm("PM_Abom_QuestSummon");
                }
            }
            if c.b("HelpRealZP_DeviceAttached") && !c.b("MS_RealZP_Transmitter") {
                c.frobbable("DatablockSwitch");
                c.set("MS_RealZP_Transmitter", true, 23);
            }
        }
        "22_FCCORPSEWERS" => {
            if c.b("ALLIED_WC") {
                c.seal_door("PDXDoor");
            }
            if c.b("ALLIED_PDX") {
                c.ally(Sel::isa("WCScubaDiver"), -1.0, true);
            }
            if c.b("WCHQ_CamerasOffline") {
                c.cameras_off();
            }
        }
        "22_WCFLOOR1" => {
            if c.b("QR_SUB_FindZP") {
                c.enter_tag("Kylie");
            }
            if c.b("ALLIED_PDX") {
                for i in c.mover_ids("ScaraElevatorSecretDoor") {
                    c.movers.rt[i].frobbable = Some(false);
                }
            }
        }
        "22_WCFLOOR4" => {
            if c.b("ALLIED_PDX") {
                c.lock("Scara_Office_Door");
                for i in c.mover_ids("Scara_Office_Door") {
                    c.movers.rt[i].pickable = false;
                }
                crate::devices::set_strength("Scara_Office_Keypad", 1.0);
            } else if !c.b("MS_ScaraSecretDoorOpened") {
                c.trigger("ScaraSecretDoor");
                c.set("MS_ScaraSecretDoorOpened", true, 23);
            }
        }
        "22_WCSUBLEVEL" => wc_sublevel_first(c),
        "22_GOATCITY" => {
            if c.b("QR_SUB_FindZP") && !c.b("QC_SUB_FindZP") {
                c.enter_tag("ZeroPresence");
            }
            if c.b("QC_CULT_DrainTheMelk") && !c.b("MS_GoatFountainsOff") {
                c.remove_tag("MelkValve1");
                c.remove_tag("MelkValve2");
                c.trigger("MelkFountain1");
                c.trigger("MelkFountain2");
                c.set("MS_GoatFountainsOff", true, 23);
            }
            if c.b("Athena_Fight_Dead") {
                c.set("Athena_Dead", true, 0);
            }
        }
        "22_GOATTEMPLAE" => {
            if c.b("QC_CULT_DestroyTheTablets") {
                c.enter_tag("DarkTemplar");
                c.enter_tag("Athena");
                c.leave_tag("Athena_Fight");
                c.friendly_all(&["InnerTemplaeGuards", "TemplaeSecurityRoomGuard"]);
                for cam in ["TemplaeSecRoomCam", "TemplaeStairsCam2", "TemplaeStairsCam1"] {
                    crate::security::event(&format!("*noalarm:{cam}"), true);
                }
            } else {
                c.enter_tag("DarkTemplar_Courtyard");
            }
        }
        "22_LLAMATEMPLE" => {
            if c.b("Hobbes_Unconscious") {
                c.hostile("Hobbes");
            }
            if !c.b("MS_Beefman_Unconscious") && c.b("Beefman_Unconscious") {
                c.pm("PM_Beefman_KnockoutRevenge");
                c.set("MS_Beefman_Unconscious", true, 23);
            }
        }
        "22_WEAPONSHOP" => {
            if c.b("StoleFromWeaponShop") {
                c.orders(Sel::tag("Andreus"), "WaitingFor", "");
                c.hostile("RavingNutter");
            }
        }
        "22_VOODOOSHOP" => {
            if c.b("Yardbomb_AdvisedToSell") {
                // A key nobody has: the portal door stays shut.
                c.lock("PortalDoor");
                for i in c.mover_ids("PortalDoor") {
                    c.movers.rt[i].pickable = false;
                }
            }
            if c.b("Yardbomb_Unconscious") {
                c.hostile("Yardbomb");
            }
        }
        "22_ATC" => {
            c.pm("PM_TG_EnterATC");
            c.hide(ActorSel::tag("Couch_LobbyBarricade2"), true);
            crate::scriptworld::push(crate::scriptworld::ActorReq::Hide(ActorSel::class_tag("BlockPlayer", "Couch_LobbyBarricade2"), true));
        }
        "22_PDXHQ01" => pdx1_first(c),
        "22_PDXHQ02" => {
            if c.b("MS_PDXMovePostBriefing1_Done") && !c.b("MS_PDXMovePostBriefing2_Done") {
                c.leave_tag("Evil");
                c.leave_tag("Phasmatis");
                c.set("MS_PDXMovePostBriefing2_Done", true, 23);
            }
        }
        "22_FCREGISTRATION" => {
            if !c.b("MS_RegistrationBreakableWalls") {
                for t in ["TurretGenerator1", "TurretGenerator2", "SpydroneSpawner1", "SpydroneSpawner2"] {
                    c.breakable(t, false);
                }
                c.set("MS_RegistrationBreakableWalls", true, 23);
            }
        }
        _ => {}
    }
}

/// Each Citadel warp zone (by ThisTag) leads to one of its lettered exits: rooms 1 and 5 share a pick of two,
/// rooms 2 and 3 a pick of four (a new one once the lava has risen), room 4 almost always its first.
fn citadel_randomize(c: &mut Ctx, lava: bool) {
    let letter = |n: u8| char::from(b'A' + n);
    let ab = letter((c.rand() * 2.0) as u8 % 2);
    for t in ["Citadel1", "Citadel5"] {
        crate::warpzone::relink(t, &format!("{t}{ab}"));
    }
    let mut k = 65 + (c.rand() * 4.0) as i32 % 4;
    if !lava {
        c.set_int("CitadelRandomization", k, 23);
    } else {
        let mut tries = 0;
        while k == c.int("CitadelRandomization") && tries < 64 {
            k = 65 + (c.rand() * 4.0) as i32 % 4;
            tries += 1;
        }
    }
    let ch = letter((k - 65) as u8);
    for t in ["Citadel2", "Citadel3"] {
        crate::warpzone::relink(t, &format!("{t}{ch}"));
    }
    let four = if c.rand() >= 0.95 { 'B' } else { 'A' };
    crate::warpzone::relink("Citadel4", &format!("Citadel4{four}"));
}

fn slums_first(c: &mut Ctx) {
    c.set_environment();
    let carrying = crate::carry::carried_bind();
    if !c.b("MS_SlumEnemiesSpawned") {
        if c.b("ALLIED_PDX") {
            c.show("WCCarcass", true);
            c.remove_tag("WCCube");
            c.remove_tag("WCSecurityComp");
            for t in ["WCThug", "WCCommando", "WCKeyHolder", "SecurityBot2WC", "MilitaryBotWC", "SecurityBot3WC"] {
                c.enter_tag(t);
            }
            if c.b("Zero_Dead") && !c.b("Zero_Dead_M20") && !c.b("Kylie_Dead") {
                c.enter_tag("KylieHolocom");
            } else if !c.b("Zero_Dead") && !carrying.eq_ignore_ascii_case("Zero") {
                c.enter_tag("ZeroHolocom");
            }
            if c.b("Zero_Dead") && !c.b("Zero_Dead_M20") {
                c.set("Zero_Dead_DXI", true, 0);
            }
        }
        if c.b("ALLIED_WC") {
            c.trigger("FactionFlipper");
            c.show("FireWallCarcass", true);
            for t in ["Firewall", "CheckpointFirewall", "CSISniper", "PDXKeyHolder", "SecurityBot2PDX", "MilitaryBotPDX", "SecurityBot3PDX"] {
                c.enter_tag(t);
            }
            c.remove_tag("PDXCube");
            c.remove_tag("PDXSecurityComp");
            if c.b("Slicer_Dead") && !c.b("Slicer_Dead_M20") {
                c.enter_tag("PhasmatisHolocom");
            } else if !c.b("Slicer_Dead") && !c.b("Slicer_Dead_M20") && !carrying.eq_ignore_ascii_case("Slicer") {
                c.enter_tag("SlicerHolocom");
            }
        }
        c.set("MS_SlumEnemiesSpawned", true, 23);
        // The other account holders have been paid since the first day: each shared ATM account gains up to the
        // amount FirstFrame gives it.
        let adds: Vec<(usize, i32)> = script_atm("FirstFrame").into_iter().map(|(i, n)| (i, (n * c.rand()) as i32)).collect();
        crate::computers::tnm_accounts_add(&adds);
    }
    if c.b("QC_SUB_Bugging") && !c.down("PJ") && !c.b("MS_BuggingUpdated") {
        c.add_goal("Bugging", "strBuggedGoal", false, false);
        c.pm("PM_PJ_Reward");
        c.set("MS_BuggingUpdated", true, 23);
    }
    if !c.b("MS_SlumEnemiesRemoved") && c.any(&["QR_MAIN_FreeKevo", "QR_MAIN_KidnapKevo"]) {
        if !c.b("Yardbomb_AdvisedToSell") && !c.b("Yardbomb_Dead") {
            c.unlock("VoodooShopDoor");
        }
        for t in HOLOCOMS {
            c.leave_tag(t);
        }
        c.enter_tag("AlleyBums");
        // Robots knocked out by EMP stay where they are.
        let still = |c: &Ctx, tag: &str| c.pawns.iter().any(|p| p.tag.eq_ignore_ascii_case(tag) && p.emp_hp > 0.0);
        if c.b("ALLIED_PDX") {
            c.show("WCCarcass", false);
            for t in ["WCThug", "WCCommando", "WCKeyHolder"] {
                c.leave_tag(t);
            }
            for t in ["SecurityBot2WC", "MilitaryBotWC", "SecurityBot3WC"] {
                if still(c, t) {
                    c.leave_tag(t);
                }
            }
        }
        if c.b("ALLIED_WC") {
            c.show("FireWallCarcass", false);
            for t in ["Firewall", "CSISniper", "PDXKeyHolder"] {
                c.leave_tag(t);
            }
            for t in ["SecurityBot2PDX", "SecurityBot3PDX"] {
                if still(c, t) {
                    c.leave_tag(t);
                }
            }
        }
        c.remove_tag("SubwayExplosives1");
        c.remove_tag("SubwayExplosives2");
        c.set("MS_SlumEnemiesRemoved", true, 23);
    }
    if c.b("Yardbomb_AdvisedToSell") {
        c.lock("VoodooShopDoor");
        c.open_door("VoodooShopWCSign");
        c.remove_tag("VoodooShopSignLight");
    }
    if c.b("QR_MAIN_KidnapKevo") {
        c.frobbable("TortureRoomDoors");
        c.enter_tag("Kevo");
    }
    if c.b("Kevo_Free") && !c.down("Kevo") && c.b("Kashue_MeetingDone") {
        crate::consys::bind_tag("ROFLCopter", "ROFLCopter", "Kevo-sama");
        crate::devices::set_strength("HelipadKeypad", 0.0);
    }
    if c.b("ALLIED_PDX") {
        for t in ["AJ", "AJThug", "AJCommando"] {
            c.leave_tag(t);
        }
    }
    if c.b("ALLIED_PDX") && c.down("Kevo") && !c.b("MS_DoneKevoDeath") {
        crate::devices::set_strength("HelipadKeypad", 0.0);
        crate::consys::bind_tag("ROFLCopter", "ROFLCopter", "ROFLCopter");
        c.set("MS_DoneKevoDeath", true, 23);
        c.add_goal("FreeThePilot", "strKevoDead", false, true);
    }
    if c.b("QR_MAIN_FreeKevo") {
        c.enter_tag("Vavrek");
    }
    if c.any(&["MS_MidasIncoming", "MS_MidasTimerStarted"]) {
        c.vehicle(ActorSel::tag("ROFLCopter"), false);
        c.leave_tag("Midashand");
        c.leave_tag("Midashand_Subway");
    }
    if c.b("MS_ChopperLockdownOff") && !c.b("Midashand_Dead") && !c.b("MS_MidasIncoming") && !c.b("MS_MidasTimerStarted") {
        c.leave_tag("Midashand_Subway");
        c.enter_tag("Midashand");
    }
}

fn wc_sublevel_first(c: &mut Ctx) {
    if c.b("NVShacker_WC") && !c.b("MS_NVS_Placed") {
        c.show("NVShackerCarcass", true);
        c.show("PrisonRegistry01", false);
        c.show("PrisonRegistry02", true);
        c.show("NVSGun", true);
        c.open_door("SublevelPrisonCellDoor02");
        if c.b("NVShackerPet_NVS") {
            c.show("NVShackerPetVictim", true);
            c.enter_tag("NVShackerPet");
        }
        c.set("MS_NVS_Placed", true, 23);
    }
    if c.b("MS_SublevelDestroyedWall") {
        c.show("WallExplosionCarcass", false);
        c.show("WallExplosionNoteBook", false);
        c.breakable("BreakableWall", true);
        c.blow_up("BreakableWall");
        c.remove_tag("CrateExplosiveMed");
        c.show("ThomasBarnes", false);
        c.show("ThomasBarnes_Lab", true);
        c.show("Datacube12", false);
        c.show("Datacube03", true);
    }
    if c.b("ALLIED_PDX") {
        c.show("ZPDataCube", false);
    }
    if c.b("QR_SUB_BlackICE") && !c.b("QC_SUB_BlackICE") {
        c.set("BlackICEVirusUploadable", true, 23);
    }
}

fn pdx1_first(c: &mut Ctx) {
    if c.b("NVShacker_PDX") && !c.b("MS_NVS_Placed") {
        c.enter_tag("NVShackerLobby");
        if c.b("NVShackerPet_NVS") {
            c.enter_tag("NVShackerPet");
        }
        c.set("MS_NVS_Placed", true, 23);
    }
    if c.b("Slicer_SetFree") {
        c.show("SlicerNewspaper_Old", false);
        c.show("SlicerNewspaper_New", true);
        if c.b("Slicer_GaveKnife") && !c.b("M22_SlicerKnifeSpawned") {
            c.spawn_item("WeaponTNMCombatKnife", "SlicerKnife");
            c.set("M22_SlicerKnifeSpawned", true, 23);
        }
    }
    // Evil's stock (an aug upgrade and three bio cells) reaches the player through his conversation's
    // TransferObject, which doesn't need the giver to hold it; only the flag is kept.
    if !c.b("Evil_Done") && !c.b("MS_Evil_Done_Done") {
        c.set("MS_Evil_Done_Done", true, 23);
    }
    if !c.b("MS_PDXMovePostBriefing1_Done") && c.b("QR_MAIN_FreeKevo") && c.last_map() != "22_PDXHQ02" {
        c.enter_tag("Evil");
        c.enter_tag("Phasmatis");
        if c.b("NVShacker_PDX") {
            c.enter_tag("NVShacker");
            c.leave_tag("NVShackerLobby");
        }
        c.spawn_item("WeaponFireStaff", "FirestaffSpawnPoint");
        c.pm("PM_Evil_ReturnStaff");
        c.set("MS_PDXMovePostBriefing1_Done", true, 23);
    }
}

// ------------------------------------------------------------------------------------------------ Timer

pub(super) fn timer(c: &mut Ctx, map: &str) {
    for (s, l) in TIMERS {
        c.timer_tick(s, l);
    }
    timers(c);
    detected(c, map);
    if c.b("Beefman_Dead") && c.b("QR_CULT_KillBeefman") && !c.b("QC_CULT_KillBeefman") {
        c.award(750, "strAssassinAward");
        c.add_goal_text("KillBeefman", "Beefman is dead. Return to the Goats for your reward.", false, false);
        c.set("QC_CULT_KillBeefman", true, 0);
    }
    if c.down("Beefman") && c.b("ALLIED_CULT_LLAMA") && !c.b("MS_AthenaGoalClose") {
        let key = match (c.b("QR_CULT_KillAthena"), c.b("Beefman_Dead")) {
            (true, true) => "strBeefmanDead1",
            (true, false) => "strBeefmanKnockedOut1",
            (false, true) => "strBeefmanDead2",
            (false, false) => "strBeefmanKnockedOut2",
        };
        c.add_goal("KillAthena", key, false, true);
        c.set("MS_AthenaGoalClose", true, 23);
    }
    if map.starts_with("22_FCSLUMS") {
        slums_timer(c);
    }
    match map {
        "22_FCDOWNTOWN" => downtown_timer(c),
        "22_SOLSBAR" => {
            if c.down("Winquman") && c.b("Winquman2_Played") && !c.b("MS_WinqumanGoalClose") {
                let key = if c.b("Winquman_Dead") { "strWinqumanDead" } else { "strWinqumanUnconscious" };
                c.add_goal("QuestionWitnesses", key, false, true);
                c.set("MS_WinqumanGoalClose", true, 23);
            }
            if c.b("AlphaOperator_Unconscious_M20") && c.b("Jackie_Unconscious_M20") && c.b("Winquman1_Played") && !c.b("MS_Winquman1_Played") {
                crate::tnmach::unlock("achSolsBombBoth");
                c.set("MS_Winquman1_Played", true, 23);
            }
        }
        "22_FCCORPORATE" => {
            if !c.b("Wolfy_GaveReward") && !c.b("Wolfy_KilledGuard") && c.down("Wolfy") && !c.b("MS_ClosedWolfyMission") {
                let key = if c.b("QR_SUB_DXEditing") { "strWolfyDead" } else { "strWolfyDeadEarly" };
                c.add_goal("DXEditing", key, false, true);
                c.set("MS_ClosedWolfyMission", true, 23);
            }
            if c.down("ZeroReal") && !c.b("MS_ZPGoalRemoved") {
                c.add_goal("HelpRealZP", "strRealZPQuest", false, true);
                c.set("MS_ZPGoalRemoved", true, 0);
            }
        }
        "22_WCFLOOR1" => {
            if c.b("ALLIED_WC") && c.b("Kylie_Dead") && !c.b("Kylie_Dead_M20") && !c.b("Scara_TOLD_KylieKill") {
                c.pm("PM_Scara_KilledKylie");
            }
        }
        "22_ATC" => atc_timer(c),
        "22_DXEDITING" => {
            if c.has_item("PosterTube") && !c.b("MS_SheepPoster_AwardGiven") && !c.b("Wolfy_KilledGuard") && !c.down("Wolfy") {
                c.award(250, "strPosterAward");
                c.set("MS_SheepPoster_AwardGiven", true, 23);
                crate::tnmach::unlock("achDXE");
            }
            if c.living("22_DXEDITING") < c.expected("22_DXEDITING") {
                c.pm("PM_Wolfy_KilledGuard");
            }
        }
        "22_GOATCITY" => {
            if !c.b("MS_FindZP_Closed") && !c.b("ALLIED_PDX") && c.down("Zero") {
                let post = c.b("MS_FindZP_AwardGiven");
                let key = match (c.b("Zero_Dead"), post) {
                    (true, true) => "strZPDeadPostAward",
                    (true, false) => "strZPDeadPreAward",
                    (false, true) => "strZPUnconPostAward",
                    (false, false) => "strZPUnconPreAward",
                };
                c.add_goal("FindZP", key, false, true);
                c.set("MS_FindZP_Closed", true, 23);
            }
            if c.b("QC_SUB_LureThePriest") && !c.b("MS_FindZP_AwardGiven") {
                c.award(300, "strMessengerAward");
                c.set("MS_FindZP_AwardGiven", true, 23);
            }
            if c.b("QC_SUB_FindZP") {
                c.pm("PM_Scara_FoundZP");
            }
        }
        "22_GOATTEMPLAE" => {
            if c.b("Athena_Fight_Dead") && c.b("QR_CULT_KillAthena") && c.b("MS_MelkShutOff") && !c.b("QC_CULT_KillAthena") {
                c.award(750, "strAssassinAward");
                c.add_goal("KillAthena", "strAthenaDead", false, false);
                c.set("QC_CULT_KillAthena", true, 0);
            }
            if c.down("DarkTemplar") && c.b("ALLIED_CULT_GOAT") && !c.b("MS_BeefmanGoalClose") {
                let key = match (c.b("QR_CULT_KillBeefman"), c.b("DarkTemplar_Dead")) {
                    (true, true) => "strTemplarDead1",
                    (true, false) => "strTemplarKnockedOut1",
                    (false, true) => "strTemplarDead2",
                    (false, false) => "strTemplarKnockedOut2",
                };
                c.add_goal("KillBeefman", key, false, true);
                c.set("MS_BeefmanGoalClose", true, 23);
            }
            if c.b("DTHologram_Played") && !c.b("MS_DTHologram_Played") {
                crate::tnmach::unlock("achAthenaRoom");
                c.set("MS_DTHologram_Played", true, 23);
            }
            // RespawnAthena with the melk shut off: she stays dead.
            if c.b("MS_MelkShutOff") && c.b("Athena_Fight_Dead") && c.n("Athena_Fight") == 0 {
                crate::tnmach::unlock("achAthena");
            }
            // Athena comes back from the melk pool until its source is shut off.
            if !c.b("MS_MelkShutOff") && c.b("Athena_Fight_Dead") && c.n("Athena_Fight") == 0 && !c.b("MS_AthenaRespawned") {
                c.set("MS_AthenaRespawned", true, 23);
            }
        }
        "22_DXO" => dxo_timer(c),
        "22_PHASAPARTMENT" => {
            if c.b("ALLIED_WC") {
                c.hostile_all(&["Firewall", "Cop1", "PhasecGuards", "PhasecBasementGuard"]);
            }
            // The giant greasel's crate broken: the stone greasel beside it comes to life.
            if !c.b("GiantGreaselCrateDone") && c.deco_exists("Decoration", Some("GiantGreaselCrate")) == 0 {
                c.set("GiantGreaselCrateDone", true, 24);
                for p in c.tag_points("StoneGreasel") {
                    let props = vec![prop("Tag", Value::Name("StoneGreasel".into()))];
                    if let Ok(mut q) = PAWNS.lock() {
                        q.push(("StoneGreasel".into(), p + DVec3::new(0.0, 0.0, 4.0), 0, props));
                    }
                }
                c.destroy_actors(ActorSel::class_tag("Decoration", "StoneGreasel"));
            }
        }
        "22_CITADEL" => {
            if c.b("Citadel_LavaRaised") {
                citadel_explosions(c);
            }
        }
        "22_FCREGISTRATION" => registration_timer(c),
        "22_WCSUBLEVEL" => {
            if c.b("Quad_PlansInserted") && c.b("Quad_CodesInserted") {
                c.set("Quad_ReadyToSpawn", true, 23);
            }
            if c.b("MS_SpawnQuadLauncher") {
                for f in ["HAS_QuadLauncherPlans", "HAS_ConstructionCodes", "Quad_PlansInserted", "Quad_CodesInserted", "Quad_ReadyToSpawn", "MS_SpawnQuadLauncher"] {
                    c.fl.delete(f);
                }
                start_uc(c, "Quad");
            }
        }
        _ => {}
    }
}

// uc: tnmMission22.CitadelExplosionEffects
/// The lava rising keeps shaking the Citadel: a third of the seconds a blast breaks loose segments of the walls
/// near the player, shakes and pushes him, swings what hangs, knocks explosive barrels near (but not in) their
/// blast range, and drops rock chips and lava bubbles from the ceiling round him.
fn citadel_explosions(c: &mut Ctx) {
    if c.rand() >= 0.33 {
        return;
    }
    let me = c.player_pos;
    for i in c.mover_ids("BreakableSegment") {
        let Some(m) = c.lvl.movers.get(i) else { continue };
        let p = DVec3::new(m.base_pos[0] as f64, m.base_pos[1] as f64, m.base_pos[2] as f64);
        if p.distance(me) <= 768.0 && c.rand() < 0.8 {
            c.movers.rt[i].breakable = Some(true);
            crate::movers::damage_mover(i, 100000.0, "Exploded");
        }
    }
    let size = c.rand() as f32;
    c.shake(0.5 + size, 512.0 + 1024.0 * size, 8.0 + 16.0 * size);
    c.sound(if size < 0.2 {
        "SmallExplosion1"
    } else if size < 0.4 {
        "MediumExplosion1"
    } else if size < 0.6 {
        "MediumExplosion2"
    } else if size < 0.8 {
        "LargeExplosion1"
    } else {
        "LargeExplosion2"
    });
    let v = c.vrand();
    let bobble = DVec3::new(300.0, 300.0, 200.0) + v * (500.0 * size as f64);
    // Only while walking and not reading something (the original checks the HUD's info window).
    if !crate::info::reading() {
        player_req(PlayerReq::Push(bobble));
    }
    crate::hanging::ship_shake(bobble, size as f64);
    for (actor, class, _, pos) in c.decos.clone() {
        if !crate::classtree::is_a(&class, "Barrel1") {
            continue;
        }
        let Some(a) = c.lvl.actors.get(actor) else { continue };
        if !matches!(unreal_pkg::props::find(&a.props, "bExplosive"), Some(Value::Bool(true))) {
            continue;
        }
        let r = match unreal_pkg::props::find(&a.props, "explosionRadius") {
            Some(Value::Float(f)) => *f as f64,
            _ => 160.0,
        };
        let d = pos.distance(me);
        if d > r && d < r * 2.0 {
            crate::destruct::hit_actor(actor, 2.0, "Shot");
        }
    }
    for _ in 0..(size * 20.0) as i32 {
        let v = c.vrand();
        let mut loc = me + v * 256.0;
        loc.z = me.z;
        let (hit, _) = c.ceiling(loc);
        let scale = (0.5 + 2.0 * c.rand()) as f32;
        c.fx(Fx::Debris(hit, "Rockchip", scale));
        if c.rand() < 0.25 {
            let dir = c.vrand();
            crate::missions::launch("LavaBubble", hit, dir);
        }
    }
}

fn downtown_timer(c: &mut Ctx) {
    if !c.b("MS_achFanFic2") && c.b("Chris_Menu_Priory_Played") && c.b("WaltonSimons_Greeting_Played") {
        crate::tnmach::unlock("achFanFic2");
        c.set("MS_achFanFic2", true, 24);
    }
    if !c.b("QC_SUB_KillBombers") && !c.b("ALLIED_WC") && c.n("SolBomber") == 0 {
        c.set("QC_SUB_KillBombers", true, 0);
        if !c.down("AlphaOperator") {
            let key = if c.b("QR_SUB_KillBombers") {
                "strBombersGoal"
            } else if c.b("SolBombers_Played") {
                "strMysteryGoal"
            } else {
                "strMysteryGoal2"
            };
            c.add_goal("SolBombing", key, false, false);
            c.award(500, "strAYBABTUAward");
        }
        crate::tnmach::unlock("achSolsBombers");
    }
    if c.down("PJ") && !c.b("PJ_GaveReward") {
        c.add_goal("Bugging", "strPJDead", false, true);
        c.set("PJ_GaveReward", true, 23);
    }
    if c.down("PJ") && !c.b("MSPJ_Dead") && !c.b("PJ_Dead_Early") {
        crate::tnmach::unlock("achPimpJojo");
        c.set("MSPJ_Dead", true, 23);
    }
    if c.down("AlphaOperator") && c.b("ALLIED_PDX") && !c.b("MS_AlphaGoalClose") {
        let dead = c.b("AlphaOperator_Dead");
        let key = match (c.b("QR_SUB_KillBombers"), c.b("QC_SUB_KillBombers"), dead) {
            (true, true, true) => "strAlphaDead",
            (true, true, false) => "strBombersDead",
            (true, false, true) => "strAlphaDead2",
            (true, false, false) => "strBombersDead2",
            (false, _, true) => "strAlphaDead3",
            (false, _, false) => "strAlphaUnconscious",
        };
        c.add_goal("SolBombing", key, false, true);
        c.set("MS_AlphaGoalClose", true, 23);
    }
}

fn slums_timer(c: &mut Ctx) {
    if c.b("Midas_TakingOff") {
        crate::tnmach::unlock("achATCNoLZ");
        c.leave_tag("Midashand");
    }
    // The hostages: alive, or knocked out on the floor.
    let tags = ["Hostage1", "Hostage2", "Hostage3"];
    let alive = c.n_any(&tags, false) + c.carcs.iter().filter(|b| b.unconscious && tags.iter().any(|t| b.tag.eq_ignore_ascii_case(t))).count();
    if alive == 0 {
        c.set("Slums_AllHostagesDead", true, 23);
    } else if alive < 3 {
        c.set("Slums_SomeHostagesDead", true, 23);
    }
    if c.down("Fender2k1") && c.down("Madmax") {
        c.pm("PM_KillDXO");
    }
    if c.b("PM_KillDXO_Played") && !c.b("MS_PM_KillDXO_Played") {
        crate::tnmach::unlock(if c.b("Slums_AllHostagesDead") { "achHostageVeryLethal" } else { "achHostageLethal" });
        c.set("MS_PM_KillDXO_Played", true, 23);
    }
    if c.b("PM_Fender_Leaving_Played") {
        crate::tnmach::unlock("achHostagePeaceful");
    }
    if c.b("QC_SUB_SubwayPower") && !c.b("MS_SubwayCleared") {
        if !c.down("Fender2k1") || !c.down("Madmax") {
            c.remove_tag("SubwayExplosives1");
        }
        c.leave_tag("Fender2k1");
        c.leave_tag("Madmax");
        if !c.b("Negotiations_Played") {
            crate::tnmach::unlock("achHostageNotMet");
        }
        c.set("MS_SubwayCleared", true, 23);
        c.pm("PM_SubwayPowerRestored");
    }
    if c.b("Kevo_PlacedOnSlab") && !c.b("MS_KevoInvincible") {
        // The body on the slab can't be destroyed (it's needed for the torture scene).
        c.set("MS_KevoInvincible", true, 23);
    }
    if c.b("AJ_Ready") {
        // The torture starts once nobody but AJ is in the room.
        let others = c
            .pawns
            .iter()
            .any(|p| p.in_world && !p.tag.eq_ignore_ascii_case("AJ") && zone_tag(c, p.pos).eq_ignore_ascii_case("TortureRoom"));
        if !others {
            c.seal_door("TortureRoomDoors");
            c.trigger("TortureSequenceDispatcher");
            c.setd("AJ_Ready", false);
        }
    }
    if c.b("QR_MAIN_KidnapKevo") && c.b("Kevo_Unconscious") {
        c.pm("PM_TG_KnockedOutKevo");
    }
    if c.b("QC_MAIN_KidnapKevo") && !c.b("MS_KidnapKevo_AwardGiven") {
        c.award(250, "strMarathonAward");
        c.set("MS_KidnapKevo_AwardGiven", true, 23);
    }
    let demolition = |c: &mut Ctx| {
        c.enter_tag("DemolitionMan");
        c.enter_tag("DemolitionSquad");
        c.breakable("HelipadGate", true);
        c.set("MS_PreparedDemolition", true, 23);
        crate::tnmach::unlock("achKevoKill");
    };
    if c.b("QR_MAIN_KidnapKevo") && !c.b("MS_PreparedDemolition") {
        if c.b("Kevo_Dead") && !c.b("AJ_TortureDone") {
            c.seal_door("TortureRoomDoors");
            c.pm("PM_TG_KilledKevo");
            demolition(c);
        } else if c.down("AJ") && c.b("AJ_TortureDone") {
            c.pm("PM_TG_KilledAJPostTorture");
        } else if c.down("AJ") {
            c.pm("PM_TG_KilledAJ");
            demolition(c);
        }
    }
    if c.b("MS_PreparedDemolition") && !c.b("MS_SpawnedDemoInterceptors") && c.movers_left("HelipadGate") < c.mover_ids("HelipadGate").len() {
        c.enter_tag("DemolitionInterceptors");
        c.add_goal("KidnapKevo", "strSurviveGoal", true, false);
        c.set("MS_SpawnedDemoInterceptors", true, 23);
    }
    let midas_busy = c.any(&["MS_MidasIncoming", "MS_ChopperLockdownOff", "MS_MidasTimerStarted"]);
    if c.b("MS_SpawnedDemoInterceptors") && !midas_busy && c.n("DemolitionInterceptors") == 0 {
        c.enter_tag("Midashand_Subway");
        c.pm("PM_TG_InterceptorsGone");
    }
    if c.b("QC_MAIN_KidnapKevo") && !c.any(&["MS_MidasIncoming", "MS_MidasTimerStarted"]) && !c.b("Midashand_Dead") {
        c.enter_tag("Midashand");
    }
    if c.b("DemolitionMan_Dead") && !c.b("MS_SpawnedDemoInterceptors") && !c.b("MS_DemoMan_Dead") {
        c.pm("PM_TG_KilledDemoMan");
        c.spawn_item("WeaponLAM", "LAMSpawnPoint");
        c.set("MS_DemoMan_Dead", true, 23);
        crate::tnmach::unlock("achSolo");
    }
    if c.b("Midashand_Dead") && !c.b("MS_DoneMidasDeath") {
        c.pm("PM_TG_KilledMidas");
        crate::consys::bind_tag("ROFLCopter", "ROFLCopter", "ROFLCopter");
        c.unlock("ATCSewers");
        c.set("MS_DoneMidasDeath", true, 23);
        crate::tnmach::unlock("achPilot");
    }
    if c.b("Vavrek_Dead") && !c.b("Vavrek_TOLD_Kidnapping") && !c.b("MS_VavrekDeadGoalUpdated") {
        c.add_goal("FreeThePilot", "strVavrekDead", false, false);
        c.set("MS_VavrekDeadGoalUpdated", true, 23);
    }
    if !c.b("Vavrek_TOLD_Kidnapping") && crate::consys::has_image("TNMDV_NoteVavrek") {
        c.set("Vavrek_TOLD_Kidnapping", true, 23);
        c.add_goal("FreeThePilot", "strVavrekRobbed", false, false);
    }
    if !c.b("Autosaved_Day2") {
        crate::save::autosave("mission");
        c.set("Autosaved_Day2", true, 23);
    }
}

/// The Tag of the zone a point is in.
fn zone_tag(c: &Ctx, p: DVec3) -> String {
    let z = c.lvl.collision.zone_at(p);
    c.lvl.zone_rules.get(z.max(0) as usize).map(|r| r.tag.clone()).unwrap_or_default()
}

fn atc_timer(c: &mut Ctx) {
    // The crash moved the alerted guards into the lobby behind the couch barricade.
    if c.b("MS_MoveLobbyGuards") && !c.b("ATC_AlertGuardsDead") && !c.b("MS_MovedLobbyGuards") {
        for i in 1..=4 {
            c.teleport(Sel::tag(&format!("ATCGuardAlert{i}")), &format!("LobbyPoint0{i}"), "Patrolling");
        }
        c.hide(ActorSel::tag("Couch_LobbyBarricade1"), true);
        c.hide(ActorSel::tag("Couch_LobbyBarricade2"), false);
        c.set("MS_MovedLobbyGuards", true, 23);
    }
    if c.n_any(&ATC_GUARDS[1..], false) == 0 {
        c.set("ATC_AlertGuardsDead", true, 23);
    }
    if c.b("HAS_ATCIDCard") && !c.b("MS_DoneID") && !c.b("MS_MoveLobbyGuards") {
        c.pm("PM_TG_GotID");
        c.neutral_all(&ATC_GUARDS, false);
        c.neutral_all(&["ATCBot"], false);
        c.set("MS_DoneID", true, 23);
    }
    let left = c.n_any(&ATC_GUARDS, false) + c.n_any(&["ATCBot"], true);
    if !c.b("Midashand_Dead") {
        if left == 0 && !c.b("MS_ChopperLockdownOff") {
            c.pm("PM_Midas_ClearedLZ");
        }
        if c.b("MS_ChopperLockdownOff") && !c.any(&["MS_MidasWaiting", "MS_MidasIncoming", "MS_MidasTimerStarted"]) {
            if left == 0 {
                c.award(750, "strAirControlAward");
                c.pm("PM_Midas_LockOff_LZClear");
                c.timer_add("TimerStarted_SpawnMidasChopper", "Timer_SpawnMidasChopper", 30);
                c.set("MS_MidasTimerStarted", true, 23);
            } else {
                c.pm("PM_Midas_LockOff_LZHot");
                c.set("MS_MidasWaiting", true, 23);
            }
        }
        if c.b("MS_ChopperLockdownOff") && c.b("MS_MidasWaiting") && !c.b("MS_MidasIncoming") && left == 0 {
            c.award(750, "strAirControlAward");
            c.pm("PM_Midas_TouchingDown");
            midas_chopper(c);
        }
    } else {
        if left == 0 {
            c.pm("PM_TG_ATCKilledEveryone");
        }
        if c.b("MS_ChopperLockdownOff") {
            c.pm("PM_TG_LockOff");
        }
    }
}

fn midas_chopper(c: &mut Ctx) {
    c.vehicle(ActorSel::tag("BlackHelicopter01"), true);
    c.trigger("LandingDispatcher");
    c.set("MS_MidasIncoming", true, 23);
}

fn dxo_timer(c: &mut Ctx) {
    let crew = ["TRM", "Compudoc", "Illuminati", "Mole"];
    if crew.iter().any(|t| c.b(&format!("{t}_Dead"))) {
        c.hostile_all(&crew);
    }
    if !c.b("ALLIED_WC") && c.b("TRM_Dead") && !c.b("Kevo_Free") && !c.b("MS_TRMDeathGoalUpdated") {
        c.add_goal("FreeThePilot", "strTRMDead", false, false);
        c.set("MS_TRMDeathGoalUpdated", true, 23);
    }
    if c.b("Kevo_Free") && !c.b("MS_FreeThePilot_AwardGiven") {
        c.award(1000, "strRescueAward");
        c.set("MS_FreeThePilot_AwardGiven", true, 23);
    }
    if !c.b("ALLIED_WC") && c.b("Kevo_Dead") || c.b("Kevo_Unconscious") {
        c.pm("PM_Evil_KillKevo");
        if c.b("Kevo_Dead") {
            crate::tnmach::unlock("achKevoKill");
        }
    }
    if !c.b("MS_AutopilotCodes_Done") && crate::consys::has_image("TNMDV_AutopilotCodes") {
        c.set("MS_AutopilotCodes_Done", true, 23);
        c.set("TrestkonFlying", true, 24);
        c.pm("PM_Evil_GotAutoPilotCodes");
        crate::tnmach::unlock("achPilot");
    }
}

/// PlayerDetected: someone turned on the player - the quiet-job flags are lost and some cameras start raising
/// alarms; at air traffic control a crime-scene investigator sees through the fake ID.
fn detected(c: &mut Ctx, map: &str) {
    let who: Vec<usize> = super::super::DETECTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for actor in who {
        let class = c.lvl.actors.get(actor).map(|a| a.class.clone()).unwrap_or_default();
        match map {
            "22_GOATCITY" | "22_GOATTEMPLAE" => {
                if c.b("QR_CULT_KillAthena") && !c.b("Beefman_LastQuestDone") {
                    c.set("NS_KillAthena", true, 23);
                }
            }
            "22_LLAMATEMPLE" => {
                if c.b("QR_CULT_KillBeefman") && !c.b("DT_LastQuestDone") {
                    c.set("NS_KillBeefman", true, 23);
                }
            }
            "22_DXO" => {
                for cam in ["HubRoomCam", "ArmouryCam", "BarrackCam"] {
                    crate::security::event(&format!("*noalarm:{cam}"), false);
                }
            }
            "22_ATC" => {
                if c.b("HAS_ATCIDCard")
                    && !c.b("MS_ATCVoidID")
                    && !c.b("MS_MoveLobbyGuards")
                    && crate::classtree::is_a(&class, "GenericCSI")
                {
                    c.pm("PM_TG_VoidID");
                    c.atc_void_id();
                }
            }
            "22_WCSUBLEVEL" => {
                if c.b("QR_SUB_BlackICE") && !c.b("Evil_Done") {
                    c.set("NS_BlackICE", true, 23);
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------ the UC

/// A UC construction: the satellites start, a warm-up sound, and four steps at 7 / 14 / 17 / 22 seconds.
fn start_uc(c: &mut Ctx, which: &str) {
    let (sats, at): (&[&str], &[&str]) = match which {
        "Quad" => (&["SatelliteGenerators"], &["QuadLauncherSpawnPoint"]),
        "RegistrationSecretUCItem" => (&["RegistrationSattGeneratorsA", "RegistrationSattGeneratorsB"], &["FirewallUCA", "FirewallUCB"]),
        _ => (&["RegistrationSattGenerators"], &["AlphaEMPGrenSpawn"]),
    };
    for s in sats {
        c.trigger(s);
    }
    for a in at {
        c.uc_sound(a, "UCWarmup");
    }
    for (secs, step) in [(7, "7"), (14, "14"), (17, "17"), (22, "22")] {
        c.timer_add(&format!("TimerStarted_{which}{step}"), &format!("Timer_{which}{step}"), secs);
    }
}

/// What the registration room's UC makes: a LAM when none is around, else a coffee. (The mod also makes ammo for a
/// weapon the player has run dry; not done here.)
fn registration_item(c: &Ctx) -> String {
    let lams = c.lvl.actors.iter().filter(|a| a.class.eq_ignore_ascii_case("WeaponTNMLAM")).count();
    if lams == 0 {
        return "WeaponTNMLAM".into();
    }
    "PlasticCoffee".into()
}

fn timers(c: &mut Ctx) {
    if c.timer_done("TimerStarted_SpawnMidasChopper", "Timer_SpawnMidasChopper") {
        c.timer_delete("TimerStarted_SpawnMidasChopper", "Timer_SpawnMidasChopper");
        if !c.b("MS_MidasIncoming") {
            midas_chopper(c);
        }
    }
    for which in ["RegistrationUCItem", "RegistrationSecretUCItem", "Quad"] {
        let step = |c: &Ctx, s: &str| c.timer_done(&format!("TimerStarted_{which}{s}"), &format!("Timer_{which}{s}"));
        let del = |c: &mut Ctx, s: &str| c.timer_delete(&format!("TimerStarted_{which}{s}"), &format!("Timer_{which}{s}"));
        let points: &[&str] = match which {
            "Quad" => &["QuadLauncherSpawnPoint"],
            "RegistrationSecretUCItem" => &["FirewallUCA", "FirewallUCB"],
            _ => &["AlphaEMPGrenSpawn"],
        };
        if step(c, "7") {
            del(c, "7");
            // The bang, then the item stands in the beam; it drops when the beam goes off at 22 s.
            for t in points {
                for p in c.tag_points(t) {
                    crate::dynlight::explosion(p, 8.0, 128, 96);
                    c.fx(Fx::Sphere(p, 4.0));
                }
                c.uc_sound(t, "UCConstruct");
            }
            let item = match which {
                "Quad" => "WeaponQdRocket".to_string(),
                "RegistrationSecretUCItem" => {
                    if c.b("NARC_Source") { "WeaponFirewallBeacon".into() } else { "WeaponFirewallBeaconAlt".into() }
                }
                _ => registration_item(c),
            };
            for t in points {
                c.spawn_item(&item, t);
            }
            if which == "Quad" {
                crate::tnmach::unlock("achQuadRocket");
            }
            c.trigger("UCElectricityEmitter");
            c.trigger("UCDamageTrigger");
        }
        if step(c, "14") {
            del(c, "14");
        }
        if step(c, "17") {
            del(c, "17");
            if which == "Quad" {
                c.trigger("SatelliteGenerators");
            }
        }
        if step(c, "22") {
            del(c, "22");
            c.untrigger("UCElectricityEmitter");
            c.trigger("UCDamageTrigger");
        }
    }
}

// ------------------------------------------------------------------------------------------------ Firewall Alpha

/// The registration office fight: reinforcements while a phase runs, phase changes when their spawners or
/// turrets are out.
fn registration_timer(c: &mut Ctx) {
    let ready = c.lvl.actors.iter().any(|a| tag_of(&a.props).eq_ignore_ascii_case("RegistrationUCspawnee"));
    if !ready && !c.b("TimerStarted_RegistrationUCItem7") && !c.b("MS_RegistrationUCStarted") {
        c.set("MS_RegistrationUCStarted", true, 23);
        start_uc(c, "RegistrationUCItem");
    }
    if c.b("Alpha_Phase1") && c.n("AlphaFirewallBoys1") + c.n("AlphaFirewallBoys2") == 0 {
        c.set("Alpha_Phase1", false, 23);
        alpha_break(c, 2);
    }
    if c.b("Alpha_Phase2") {
        reinforce(c);
        if c.b("DroneSpawner1_Destroyed") && c.b("DroneSpawner2_Destroyed") {
            c.set("Alpha_Phase2", false, 23);
            alpha_break(c, 3);
        }
    }
    if c.b("Alpha_Phase3") && c.b("Turret1Disable") && c.any(&["Turret2DisableEarly", "Turret2Disable"]) {
        c.set("Alpha_Phase3", false, 23);
        alpha_break(c, 4);
    }
    if c.any(&["Alpha_Phase3", "Alpha_Phase4"]) {
        reinforce(c);
    }
    if c.b("LoyalFirewall_Dead") {
        c.pm("PM_Firewall_CharlieDead");
    }
}

/// Reinforcements: each spawn slot left empty for 10 s gets a new one (not from a destroyed drone spawner).
fn reinforce(c: &mut Ctx) {
    if !c.vars.inited {
        c.vars.last_killed = [-1.0; 12];
        c.vars.inited = true;
    }
    // Drones knocked out by EMP blow up.
    for (_, class, tag) in ALPHA_SPAWNS {
        if class == "SpydronePawn" && c.pawns.iter().any(|p| p.tag.eq_ignore_ascii_case(tag) && p.emp_hp <= 0.0) {
            crate::ai::world_req(WorldReq::Kill(Sel::tag(tag)));
        }
    }
    for (i, (sp, class, tag)) in ALPHA_SPAWNS.iter().enumerate() {
        let n = c.n(tag);
        if n == 0 && c.vars.last_killed[i] == -1.0 {
            c.vars.last_killed[i] = c.now;
        }
        if n == 0 && c.now - c.vars.last_killed[i] > 10.0 {
            let spawner_gone = (i == 0 && c.b("DroneSpawner1_Destroyed")) || (i == 1 && c.b("DroneSpawner2_Destroyed"));
            if spawner_gone {
                continue;
            }
            let pts = c.points("SpawnPoint", sp);
            if pts.is_empty() {
                continue;
            }
            let pos = pts[(c.rand() * pts.len() as f64) as usize % pts.len()];
            c.flash_at(pos);
            c.spawn_firewall_side(class, tag, pos, 0, -1.0);
            c.vars.last_killed[i] = -1.0;
        }
    }
}

/// Alpha moves to the middle and stands down while the next phase is prepared.
fn alpha_break(c: &mut Ctx, next: i32) {
    if let Some((pos, yaw)) = c.point("SpawnPoint", "AlphaMasterSpawnPoint") {
        if c.n("FirewallAlpha") == 0 {
            // Taken down in the last phase: he's back, standing in the middle.
            c.spawn_firewall_side("FireWallAlpha", "FirewallAlpha", pos, yaw, 0.0);
            c.flash_at(pos);
        } else {
            crate::ai::world_req(WorldReq::Teleport(Sel::tag("FirewallAlpha"), pos, yaw));
            if next >= 3 {
                c.flash_at(pos);
            }
        }
    }
    c.ally(Sel::tag("FirewallAlpha"), 0.0, true);
    match next {
        3 => c.untrigger("AlarmLightSD"),
        4 => c.untrigger("AlarmLightTR"),
        _ => {}
    }
    c.pm(&format!("PM_Firewall_Phase{next}"));
}

/// A phase starts: Alpha jumps to one of his spawn points and attacks again.
fn alpha_start(c: &mut Ctx, phase: i32) {
    let pts = c.points("SpawnPoint", "AlphaSpawnPoint");
    if !pts.is_empty() {
        let pos = pts[(c.rand() * pts.len() as f64) as usize % pts.len()];
        let yaw = {
            let d = c.player_pos - pos;
            (d.y.atan2(d.x) * 32768.0 / std::f64::consts::PI) as i32
        };
        crate::ai::world_req(WorldReq::TeleportOrders(Sel::tag("FirewallAlpha"), pos, yaw, "Attacking".into()));
        c.flash_at(pos);
    }
    c.hostile("FirewallAlpha");
    match phase {
        2 => {
            c.breakable("SpydroneSpawner1", true);
            c.breakable("SpydroneSpawner2", true);
            c.trigger("SpydroneDoor1");
            c.trigger("SpydroneDoor2");
            c.trigger("AlarmLightSD");
            c.set("Alpha_Phase2", true, 23);
            // The first drones come two seconds in.
            if !c.vars.inited {
                c.vars.last_killed = [-1.0; 12];
                c.vars.inited = true;
            }
            for i in 0..2 {
                if c.vars.last_killed[i] == -1.0 {
                    c.vars.last_killed[i] = c.now - 8.0;
                }
            }
        }
        3 => {
            c.breakable("TurretGenerator1", true);
            c.breakable("TurretGenerator2", true);
            for t in ["MoveCover", "AlphaTurret1", "AlphaTurret1Turret", "RespawnGen1", "RespawnGen2", "RespawnGen3"] {
                c.trigger(t);
            }
            if !c.b("Turret2DisableEarly") {
                c.trigger("AlphaTurret2");
                c.trigger("AlphaTurret2Turret");
            }
            c.trigger("TurretGeneratorWall1");
            c.trigger("TurretGeneratorWall2");
            c.trigger("AlarmLightTR");
            c.set("Alpha_Phase3", true, 23);
        }
        _ => {
            for i in 1..=3 {
                c.trigger(&format!("VentingMovers{i}"));
                c.trigger(&format!("VentSteam{i}"));
                c.trigger(&format!("AlarmLight{i}"));
            }
            c.set("Alpha_Phase4", true, 23);
        }
    }
}

fn alpha_defeated(c: &mut Ctx) {
    c.trigger("FinishDoorUnlock");
    c.set("Alpha_Defeated", true, 23);
    c.add_goal("FirewallAlpha", "strAlphaDefeated", true, true);
    c.award(500, "strDebuggingAward");
    c.leave_tag("FriendlyFirewall");
    // Alpha's long death (his own state in the mod): he goes down where he stands.
    crate::ai::world_req(WorldReq::Kill(Sel::tag("FirewallAlpha")));
}

fn stop_generator(c: &mut Ctx, i: i32) {
    let flag = format!("Alpha_GeneratorDisabled{i}");
    if !c.b(&flag) {
        c.trigger(&format!("RespawnGen{i}"));
        c.untrigger(&format!("AlarmLight{i}"));
        c.trigger(&format!("VentingMovers{i}"));
        c.trigger(&format!("VentSteam{i}"));
    }
    c.set(&flag, true, 23);
    if (1..=3).all(|k| c.b(&format!("Alpha_GeneratorDisabled{k}"))) && !c.b("Alpha_Defeated") {
        c.set("Alpha_Phase3", false, 23);
        c.set("Alpha_Phase4", false, 23);
        c.pm("PM_Firewall_Death");
        alpha_defeated(c);
    }
}

// ------------------------------------------------------------------------------------------------ game events

/// Mission 22's script functions asked for by its maps.
pub(super) fn event(c: &mut Ctx, f: &str) -> bool {
    match f {
        "SlumHologramDisappear" => {
            for t in HOLOCOMS {
                c.leave_tag(t);
            }
        }
        // The knock on the torture room door: AJ talks differently when the player brings Kevo's body.
        "TortureKnockConvo" => {
            if !c.b("AJ_ComeIn_Played") && c.b("QR_MAIN_KidnapKevo") && !c.b("Kevo_Dead") {
                let body = crate::carry::carried_bind().eq_ignore_ascii_case("Kevo");
                c.trigger(if body { "TortureKnock_Body" } else { "TortureKnock_NoBody" });
            }
        }
        // AJ takes Kevo's body off the player and lays it on the slab.
        // uc: tnmMission22.AJChangeClothes
        "AJChangeClothes" => {
            crate::reskin::request_tag("AJ", script_skins("AJChangeClothes"));
            c.set("MS_AJChangedClothes", true, 0);
            if crate::carry::carried_bind().eq_ignore_ascii_case("Kevo") {
                crate::carry::discard_body();
            }
            c.hide(ActorSel::tag("KevoCarcass"), false);
            crate::tnmach::unlock("achKevoCapture");
        }
        "OpenTortureRoom" => c.open_door("TortureRoomDoors"),
        "EvacuateTemplae" => {
            if c.b("QR_SUB_LureThePriest") {
                for t in ["OiNutter", "FleeingGoatFemale", "OiBodyguard1", "OiBodyguard2"] {
                    c.enter_tag(t);
                }
            }
        }
        "ShootOiNutter" => {
            if !c.down("Zero") {
                crate::sound::play_player("DeusExSounds.Weapons.RifleFire", 1.5, 0.0);
                crate::ai::world_req(WorldReq::KillShot(Sel::tag("OiNutter")));
                if !c.b("PM_Zero_ShotOi_Played") {
                    c.award(200, "strAccompliceAward");
                }
                c.pm("PM_Zero_ShotOi");
            }
        }
        "LandedChopperAppear" => c.vehicle(ActorSel::tag("BlackChopper02"), true),
        "CarCrash" => {
            c.vehicle(ActorSel::tag("HovercarCrash"), true);
            c.set("MS_MoveLobbyGuards", true, 23);
            if c.b("HAS_ATCIDCard") && !c.b("MS_ATCVoidID") {
                c.atc_void_id();
            }
            crate::tnmach::unlock("achHovercar");
        }
        // Dark Templar leaves his rewards on the floor: an accuracy mod, then a silencer and an aug upgrade unless
        // the player had no room for those earlier.
        "DTDropItems" => {
            if let Some((p, _)) = c.point("PathNode", "DTDropItemPoint").or_else(|| c.tag_points("DTDropItemPoint").first().map(|p| (*p, 0))) {
                let mut items = vec![("WeaponModAccuracy", p)];
                if !c.b("DT_NoRoom_Accuracy") {
                    items.push(("WeaponModSilencer", p + DVec3::new(20.0, 0.0, 0.0)));
                    if !c.b("DT_NoRoom_Silencer") {
                        items.push(("AugmentationUpgradeCannister", p - DVec3::new(20.0, 0.0, 0.0)));
                    }
                }
                if let Ok(mut q) = ITEMS.lock() {
                    for (cl, at) in items {
                        q.push((cl.into(), at, Vec::new()));
                    }
                }
            }
        }
        "DTHologramAppear" => c.enter_tag("DTHologram"),
        "DTHologramDisappear" => c.leave_tag("DTHologram"),
        "KillShieldDoor" => c.mover_to("Specimen1Shield", 4, 0.0),
        "ParadronesEnter" => c.vehicle(ActorSel::class("ParadropDrone"), true),
        "HideMidasHand" => c.leave(Sel::isa("Midashand")),
        "OrderGuardsToApartment" => c.orders(Sel::tag("InnerTemplaeGuards"), "Wandering", ""),
        // The TV switch alternates between opening and closing the screen.
        "AthenaTVOpen" => crate::devices::set_event("TVSwitch", "TVClose"),
        "AthenaTVClose" => crate::devices::set_event("TVSwitch", "TVOpen"),
        "CitadelRaiseLava" => {
            c.mover_to("MainRoomRaise", 1, 0.0);
            c.mover_to("FirstEntranceRaise", 1, 0.0);
            c.breakable("FloatingRock", true);
            for i in c.mover_ids("FloatingRock") {
                crate::movers::damage_mover(i, 100.0, "Shot");
            }
            c.leave_tag("ConvoFanatic");
            c.leave_tag("CivFanatic");
            c.enter_tag("DXIFanaticBoss");
            crate::zones::event(c.lvl, "RaisedLavaZone", true);
            c.trigger("Firestarter");
            c.set("Citadel_LavaRaised", true, 23);
        }
        "AlphaPhase1Start" => {
            c.trigger("FireWallDoor1");
            c.trigger("FireWallDoor2");
            c.enter_tag("AlphaFirewallBoys1");
            c.enter_tag("AlphaFirewallBoys2");
            c.set("Alpha_Phase1", true, 23);
        }
        "AlphaPhase2Start" => alpha_start(c, 2),
        "AlphaPhase3Start" => alpha_start(c, 3),
        "AlphaPhase4Start" => alpha_start(c, 4),
        "AlphaStopGenerator1" => stop_generator(c, 1),
        "AlphaStopGenerator2" => stop_generator(c, 2),
        "AlphaStopGenerator3" => stop_generator(c, 3),
        "FirewallAlphaKillphrased" => {
            if !c.b("Alpha_Defeated") {
                alpha_defeated(c);
            }
        }
        // The levers must be pulled in order 1-2-3-4; a wrong one resets them all.
        "CounterPuzzle1" | "CounterPuzzle2" | "CounterPuzzle3" | "CounterPuzzle4" => {
            let i: i32 = f[f.len() - 1..].parse().unwrap_or(0);
            let next = if i == c.int("CounterPuzzle") + 1 { i } else { 0 };
            c.set_int("CounterPuzzle", next, 23);
            if next >= 4 {
                c.trigger("SecretMover");
            } else {
                for i in c.mover_ids("CounterLever") {
                    if c.movers.rt[i].key_num != 0 {
                        c.trigger("CounterLever");
                        break;
                    }
                }
            }
        }
        "OverloadFirewalls1" => crate::ai::world_req(WorldReq::Disable(Sel::tag("AlphaFirewallBoys1"))),
        "OverloadFirewalls2" => crate::ai::world_req(WorldReq::Disable(Sel::tag("AlphaFirewallBoys2"))),
        "FirewallBeacon" => start_uc(c, "RegistrationSecretUCItem"),
        _ => return false,
    }
    true
}
