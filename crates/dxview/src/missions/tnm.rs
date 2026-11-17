//! The Nameless Mod's mission scripts, written for DX RUST from how they behave in the player's own copy of the mod
//! (its TNM.u). Nothing of the mod is shipped: the flag, tag and map names below are the identifiers its maps and
//! conversations use, and every line of text a script shows (goal texts, award messages) is read at run time from
//! the mission script's own defaults in the player's files (`load_text`).
//!
//! Covered: the shared mission-script layer (count of the living per map, flag-held countdowns, the last map, the
//! killed-important-characters rule), and Mission 20 (the first half of the game, Forum City) - each map's set-up on
//! arrival, its once-a-second checks and what it records on leaving. Missions 21 to 25 run the shared layer only.

use super::*;
use std::collections::HashMap;

/// The mission script's own strings (its localized defaults), by property name.
static TEXT: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Read the strings of the map's mission script class (LevelInfo.Script) from the player's packages.
pub fn load_text(classes: &mut unreal_pkg::classes::ClassDb, script: &str) {
    let class = script.rsplit('.').next().unwrap_or(script);
    let mut m = HashMap::new();
    for p in classes.defaults(class) {
        match &p.value {
            Value::Str(s) => {
                m.entry(p.name.to_ascii_lowercase()).or_insert_with(|| s.clone());
            }
            Value::Int(n) => {
                m.entry(p.name.to_ascii_lowercase()).or_insert_with(|| n.to_string());
            }
            _ => {}
        }
    }
    if let Ok(mut t) = TEXT.lock() {
        *t = Some(m);
    }
}

fn text(key: &str) -> String {
    TEXT.lock()
        .ok()
        .and_then(|t| t.as_ref().and_then(|m| m.get(&key.to_ascii_lowercase()).cloned()))
        .unwrap_or_default()
}

/// A number from the script's defaults (0 when it has none).
fn num(key: &str) -> i32 {
    text(key).parse().unwrap_or(0)
}

// --------------------------------------------------------------------------------------------- shared helpers

/// Tags the living count leaves out (the Mission 20 script's exclusions).
const COUNT_EXCLUDED: [&str; 2] = ["ZeroPresence", "WCSpy"];

/// Items a script puts in the world: (class, location, property overrides).
pub(super) static ITEMS: Mutex<Vec<(String, DVec3, Vec<Property>)>> = Mutex::new(Vec::new());

fn prop(name: &str, value: Value) -> Property {
    Property { name: name.to_string(), index: 0, value }
}

impl Ctx<'_> {
    fn int(&self, n: &str) -> i32 {
        match self.fl.get(n) {
            Some(FlagValue::Int(v)) => *v,
            _ => 0,
        }
    }
    fn set_int(&mut self, n: &str, v: i32, exp: i32) {
        self.fl.set(n, FlagValue::Int(v), exp);
    }
    fn any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.b(n))
    }
    /// The character is out of the story: `<name>_Dead` or `<name>_Unconscious`.
    fn down(&self, name: &str) -> bool {
        self.b(&format!("{name}_Dead")) || self.b(&format!("{name}_Unconscious"))
    }

    // A countdown kept in the flags (it carries over saves and maps): `started` set once, `left` seconds.
    fn timer_add(&mut self, started: &str, left: &str, secs: i32) {
        if !self.b(started) {
            self.setd(started, true);
            let exp = self.fl_default_exp();
            self.set_int(left, secs, exp);
        }
    }
    fn timer_tick(&mut self, started: &str, left: &str) {
        if self.b(started) {
            let v = self.int(left);
            if v > 0 {
                let exp = self.fl_default_exp();
                self.set_int(left, v - 1, exp);
            }
        }
    }
    fn timer_done(&self, started: &str, left: &str) -> bool {
        self.b(started) && self.int(left) <= 0
    }
    fn timer_delete(&mut self, started: &str, left: &str) {
        self.fl.delete(started);
        self.fl.delete(left);
    }
    fn fl_default_exp(&self) -> i32 {
        self.mission + 1
    }

    /// The map the player came from (kept as a flag, so it survives saves).
    fn last_map(&self) -> String {
        match self.fl.get("TNMLastMap") {
            Some(FlagValue::Name(s)) => s.trim_end_matches("_2009").to_ascii_uppercase(),
            _ => String::new(),
        }
    }

    /// A private message from a contact: the mod's own infolinks.
    fn pm(&mut self, name: &str) {
        crate::consys::start_datalink(self.fl, name, None);
    }

    /// AddGoal(name, the script's string `text_key`, primary, completed).
    fn add_goal(&mut self, name: &str, text_key: &str, primary: bool, completed: bool) {
        let t = text(text_key);
        if let Some(cs) = crate::consys::lock().as_mut() {
            cs.add_goal(name, &t, primary);
            if completed {
                cs.goal_completed(name);
            }
        }
    }

    /// Skill points and the script's message for them.
    fn award(&mut self, points: i32, text_key: &str) {
        let _ = crate::abilities::award_points(points);
        let t = text(text_key);
        if !t.is_empty() {
            if let Some(cs) = crate::consys::lock().as_mut() {
                cs.message(&t);
            }
        }
    }

    /// Put an item (a class name, with or without its package) where the first actor with this Tag stands.
    fn spawn_item(&self, class: &str, at_tag: &str) {
        let class = class.rsplit('.').next().unwrap_or(class);
        let Some(a) = self.lvl.actors.iter().find(|a| tag_of(&a.props).eq_ignore_ascii_case(at_tag)) else { return };
        let l = a.location();
        if let Ok(mut q) = ITEMS.lock() {
            q.push((class.to_string(), DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), Vec::new()));
        }
    }

    /// ChangeAlly('Player', level, permanent) for the pawns picked.
    fn ally(&self, sel: Sel, level: f32, permanent: bool) {
        crate::triggers::push_order(PawnOrder::Ally { sel, alliance: "Player".into(), level, permanent });
    }
    fn hostile(&self, tag: &str) {
        self.ally(Sel::tag(tag), -1.0, true);
    }

    fn enter_tag(&self, tag: &str) {
        self.enter(Sel::tag(tag));
    }
    fn leave_tag(&self, tag: &str) {
        self.leave(Sel::tag(tag));
    }
    /// A body, datacube or decoration out of / into the world.
    fn show(&self, tag: &str, on: bool) {
        self.hide(ActorSel::tag(tag), !on);
    }

    fn lock(&mut self, tag: &str) {
        for i in self.mover_ids(tag) {
            self.movers.rt[i].locked = true;
        }
    }
    fn pickable(&mut self, tag: &str) {
        for i in self.mover_ids(tag) {
            self.movers.rt[i].pickable = true;
        }
    }
    fn frobbable(&mut self, tag: &str) {
        for i in self.mover_ids(tag) {
            self.movers.rt[i].frobbable = Some(true);
        }
    }
    /// Open a door for good: triggered open and unlocked.
    fn open_door(&mut self, tag: &str) {
        for i in self.mover_ids(tag) {
            if self.movers.rt[i].key_num == 0 {
                self.trigger(tag);
                break;
            }
        }
        self.unlock(tag);
    }
    /// Shut a door for good: closed if open, locked, not pickable, breakable or usable.
    fn seal_door(&mut self, tag: &str) {
        let ids = self.mover_ids(tag);
        if ids.iter().any(|&i| self.movers.rt[i].key_num != 0) {
            self.trigger(tag);
        }
        for i in ids {
            let m = &mut self.movers.rt[i];
            m.locked = true;
            m.pickable = false;
            m.breakable = Some(false);
            m.frobbable = Some(false);
        }
    }

    /// Every camera off (the Black ICE "cameras offline" option).
    fn cameras_off(&self) {
        crate::security::event("*cameras", false);
    }

    /// The living a map cares about: the mod's characters (not animals, not the excluded tags), plus bodies that
    /// still count - knocked out, or killed by someone other than the player.
    fn count_living(&self) -> i32 {
        let excluded = |t: &str| COUNT_EXCLUDED.iter().any(|e| e.eq_ignore_ascii_case(t));
        let pawns = self
            .pawns
            .iter()
            .filter(|p| crate::classtree::is_a(&p.class, "TNMPawn") && !crate::classtree::is_a(&p.class, "tnmAnimal"))
            .filter(|p| !excluded(&p.tag))
            .count();
        let bodies = self
            .carcs
            .iter()
            .filter(|c| !excluded(&c.tag))
            .filter(|c| c.unconscious || (!c.killer.is_empty() && !c.killer.eq_ignore_ascii_case("JCDenton")))
            .count();
        (pawns + bodies) as i32
    }
    /// `<map>_Count` for this map, refreshed; for another map, what was recorded there.
    fn living(&mut self, map: &str) -> i32 {
        let key = format!("{map}_Count");
        if self.map.eq_ignore_ascii_case(map) {
            let n = self.count_living();
            self.set_int(&key, n, 26);
        }
        self.int(&key)
    }
    fn expected(&self, map: &str) -> i32 {
        self.int(&format!("{map}_StartCount"))
    }
}

/// Every TNM mission script on its first tick (after Deus Ex's own MissionScript part in mission.rs).
pub fn first_frame(c: &mut Ctx) {
    // The first count on a map is the number later visits are measured against.
    let start = format!("{}_StartCount", c.map);
    if c.int(&start) == 0 {
        let n = c.living(&c.map.clone());
        c.set_int(&start, n, 26);
    }
    let map = c.map.clone();
    if c.mission == 20 || c.mission == 22 {
        m20and22_first(c, &map);
    }
    if c.mission == 20 {
        m20_first(c, &map);
    }
}

/// Once a second.
pub fn timer(c: &mut Ctx) {
    let map = c.map.clone();
    let _ = c.living(&map);
    if c.mission == 20 {
        for (s, l) in M20_TIMERS {
            c.timer_tick(s, l);
        }
        m20_countdowns(c, &map);
        m20_timer(c, &map);
    }
}

/// Leaving a map: what it records.
pub fn pre_travel(fl: &mut FlagBase, mission: i32, map: &str, movers: &crate::movers::Movers, lvl: &dx_level::Level) {
    fl.set("TNMLastMap", FlagValue::Name(map.to_string()), 0);
    let b = |fl: &FlagBase, n: &str| fl.get_bool(n);
    if mission == 20 {
        match map {
            "20_FCDOWNTOWN" if b(fl, "BreakAndEnter") => fl.set("MS_BreakAndEnter", FlagValue::Bool(true), 21),
            "20_SOLSBAR" if b(fl, "MS_SolsTrouble") => fl.set("MS_SolsTrouble_Post", FlagValue::Bool(true), 23),
            _ => {}
        }
        // The Wild Cards HQ's sub-level hatch keeps its state between the floors.
        if map.starts_with("20_WC") {
            for (i, m) in lvl.movers.iter().enumerate() {
                if m.tag.eq_ignore_ascii_case("SublevelKeypadHatch") {
                    let rt = &movers.rt[i];
                    fl.set("WC_ElevatorHatch_Locked", FlagValue::Bool(rt.locked), 21);
                    fl.set("WC_ElevatorHatch_Open", FlagValue::Bool(rt.key_num == 1), 21);
                    fl.set("WC_ElevatorHatch_Known", FlagValue::Bool(true), 21);
                }
            }
        }
    }
}

// =================================================================================================== Missions 20/22

fn m20and22_first(c: &mut Ctx, map: &str) {
    let m = map.trim_start_matches("20_").trim_start_matches("22_");
    if c.any(&["ALLIED_CULT_LLAMA", "DT_Pissed", "DarkTemplar_Dead", "Athena_Dead"]) {
        c.set("HOSTILE_CULT_GOAT", true, 25);
    }
    match m {
        "SOLSBAR" if map.starts_with("20_") && c.b("MS_CloseSols") => {
            // The bar is closed after a fight there: Jackie stands alert, the operator sits in his office,
            // everyone else is gone.
            if let Some((pos, yaw)) = c.point("PatrolPoint", "JackieAlertedPoint") {
                crate::ai::world_req(WorldReq::TeleportOrders(Sel::tag("Jackie"), pos, yaw, "Wandering".into()));
                c.hostile("Jackie");
            }
            if let Some((pos, yaw)) = c.point("PatrolPoint", "AOOfficePoint") {
                crate::ai::world_req(WorldReq::Teleport(Sel::tag("AlphaOperator"), pos, yaw));
                c.orders(Sel::tag("AlphaOperator"), "Sitting", "AOOfficeChair");
                c.hostile("AlphaOperator");
            }
            for p in c.pawns_of("TNMPawn", None) {
                if !p.tag.eq_ignore_ascii_case("Jackie") && !p.tag.eq_ignore_ascii_case("AlphaOperator") {
                    c.leave(Sel::isa_tag("TNMPawn", &p.tag));
                }
            }
            c.hide(ActorSel::class("TNMCarcass"), true);
        }
        "WCFLOOR1" | "WCFLOOR2" | "WCFLOOR3" | "WCFLOOR4" | "WCSUBLEVEL" => {
            if c.b("WCHQ_SublevelKeypad_Open") {
                crate::devices::set_strength("SublevelKeypad", 0.0);
            }
            if c.b("WCHQ_SublevelKeypadHatch_Open") {
                c.unlock("SublevelKeypadHatch");
            }
            if c.b("WCHQ_CamerasOffline") {
                c.cameras_off();
            }
            if m == "WCSUBLEVEL" && c.b("ALLIED_WC") {
                c.ally(Sel::tag("ThatGuy"), 1.0, false);
            }
            // The Wild Cards turn on a player who freed Silver Dragon, sided with PDX or killed Scara.
            if !c.b("ALLIED_WC") && (c.b("QC_MAIN_SaveSD") && !c.b("Silver_Dead") || c.any(&["ALLIED_PDX", "Scara_Dead"])) {
                for t in ["WCSecurity", "WCSecurity01", "WCCommando", "ScaraGuard", "ScaraKing"] {
                    c.hostile(t);
                }
            }
        }
        "PDXHQ01" if c.b("ALLIED_WC") => {
            for t in ["Krissy", "Evil", "Slicer", "Winquman", "DXAGuard", "DXAGuardLight", "FireWall2", "GPRentACop", "DXACop"] {
                c.hostile(t);
            }
        }
        "PDXHQ02" if c.b("ALLIED_WC") => {
            c.leave_tag("KingKashue");
            c.enter_tag("KingKashue_Patrolling");
            for t in ["Krissy", "Evil", "Slicer", "Winquman", "DXAGuard", "DXAGuardLight", "FireWall2", "Phasmatis", "SecurityBot2PDX"] {
                c.hostile(t);
            }
        }
        "FCDOWNTOWN" => {
            if c.b("ALLIED_CULT_GOAT") || c.b("Abom_Pissed") || c.down("Beefman") {
                for t in ["Abomnation", "LlamaGuard", "TunnelGuard", "Hobbes", "SD"] {
                    c.hostile(t);
                }
            }
            // The fan-fiction shop closes once its owners are hurt.
            if c.down("Chris") || c.down("Walton") {
                for t in ["Walton", "Chris", "jimmycracker", "FanFicCustomer"] {
                    c.leave_tag(t);
                }
                if c.b("QR_SUB_JimmyHeirloom") && !c.b("QC_SUB_JimmyHeirloom") {
                    c.add_goal("JimmyHeirloom", "strJimmyNoPay", false, true);
                    c.set("QC_SUB_JimmyHeirloom", true, 21);
                }
                c.seal_door("FanFicStoreDoor");
                c.show(if c.any(&["Chris_Dead", "Walton_Dead"]) { "FanFicNotice_Death" } else { "FanFicNotice_Knockout" }, true);
            } else {
                let groups: &[(&str, [&str; 3])] = &[
                    ("ALLIED_CULT_GOAT", ["AttackGroup_Llama01", "AttackGroup_Llama02", "AttackGroup_Llama03"]),
                    ("ALLIED_CULT_LLAMA", ["AttackGroup_goat01", "AttackGroup_goat02", "AttackGroup_goat03"]),
                ];
                for (flag, tags) in groups {
                    if c.b(flag) {
                        let here = c.pawns.iter().filter(|p| p.in_world && tags.iter().any(|t| p.tag.eq_ignore_ascii_case(t))).count();
                        if here < 2 {
                            for t in tags {
                                c.enter_tag(t);
                            }
                        }
                    }
                }
            }
        }
        "FCCORPORATE" => {
            if c.b("ALLIED_WC") {
                c.lock("PDXHQDoors");
            }
            let last = c.last_map();
            if c.b("Andreus_Dead") && c.b("Raving_Dead") && !last.ends_with("_WEAPONSHOP") {
                c.set("WeaponShop_Locked", true, 23);
                c.seal_door("WeaponShopDoor");
            }
        }
        "FCSLUMS" | "FCSLUMS_WC" | "FCSLUMS_PDX" if c.b("Yardbomb_Dead") => c.lock("VoodooShopDoor"),
        "WEAPONSHOP" => {
            if (c.down("Andreus") || c.b("Andreus_Unconscious_M20")) && !c.down("Raving") {
                c.leave_tag("RavingNutter");
                c.enter_tag("Raving_Angry");
                c.enter_tag("RavingThug");
                c.set("WeaponShop_Hostile", true, 21);
            } else if (c.b("Raving_Dead") || c.b("Andreus_Unconscious_M20")) && !c.down("Andreus") {
                c.leave_tag("Andreus");
                c.enter_tag("Andreus_Angry");
                c.set("WeaponShop_Hostile", true, 21);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission 20

/// Mission 20's countdowns: (started flag, seconds-left flag).
const M20_TIMERS: [(&str, &str); 2] = [("TimerStarted_MeetPhasPM", "Timer_MeetPhasPM"), ("TimerStarted_MeetDTPM", "Timer_MeetDTPM")];

fn m20_first(c: &mut Ctx, map: &str) {
    // The Wild Cards' floors: their bots and guards turn on a player who freed Silver Dragon, sided with PDX or
    // killed Scara; their cameras then raise alarms.
    if map.starts_with("20_WCFLOOR") && (c.b("SD_Escaped") && !c.b("Silver_Dead") || c.any(&["ALLIED_PDX", "Scara_Dead"])) {
        for p in c.pawns_of("TNMPawn", None) {
            if !crate::classtree::is_a(&p.class, "SD") {
                c.ally(Sel::isa_tag("TNMPawn", &p.tag), -1.0, true);
            }
        }
    }
    // A bar or club whose regulars were hurt closes for the rest of the mission.
    if map != "20_SOLSBAR" && map != "20_FCDOWNTOWN" && c.living("20_SOLSBAR") < c.expected("20_SOLSBAR") {
        c.set("MS_CloseSols", true, 21);
    }
    if map != "20_PARTYZONE" && map != "20_FCCORPORATE" && c.living("20_PARTYZONE") < c.expected("20_PARTYZONE") {
        c.set("MS_ClosePartyZone", true, 21);
    }
    let last = c.last_map();
    match map {
        "20_PHASAPARTMENT" => {
            // After the spyware job Phasmatis has moved to PDX's headquarters and locked up.
            if c.b("QC_MAIN_WCSpyware") && !c.b("MS_PhasDoorClosed") {
                c.seal_door_keep_usable("PhasAptDoor");
                c.leave_tag("Phasmatis");
                c.set("MS_PhasDoorClosed", true, 21);
            }
            // The gold coat is the reward for every one of the mod's achievements, which DX RUST doesn't track.
            c.destroy_actors(ActorSel::class("ABICoatBag"));
            if c.b("QR_MAIN_KillPhas") && !c.b("QC_MAIN_KillPhas") {
                c.ally(Sel::tag("Phasmatis"), 1.0, false);
            }
        }
        "20_LLAMATEMPLE" => {
            if !c.b("Silver_Dead") && !c.b("MS_SilverInTemple") {
                c.set("MS_SilverInTemple", true, 21);
            }
            let quest = c.b("QR_CULT_DestroyTheTablets") && !c.down("DarkTemplar");
            if (quest || c.b("ALLIED_CULT_LLAMA")) && !c.b("MS_SpawnedLlamaKey") {
                c.give_key_at("LlamaTunnelDoorKey", "TunnelKeySpawn", "strLlamaKey");
                c.pickable("LlamaTunnelDoor");
                c.set("MS_SpawnedLlamaKey", true, 21);
            }
        }
        "20_PDXHQ01" => {
            if c.b("QC_MAIN_WCSpyware") && !c.b("QR_MAIN_ShadowCode") {
                c.enter_tag("KingKashue_Evil");
            }
            if c.b("QC_MAIN_ShadowCode") && !c.b("QR_MAIN_Firestaff") {
                c.leave_tag("KingKashue_Evil");
            }
            if c.b("QC_MAIN_WCSpyware") && !c.b("MS_20_SpawnedWCSpywareReward") {
                c.set("MS_20_SpawnedWCSpywareReward", true, 21);
                for (item, at) in [("AugCannisterSS", "JonasAugSpawn"), ("Medkit", "WinqSpawn1"), ("Ammo45", "WinqSpawn2"), ("WeaponModRange", "WinqSpawn3")] {
                    c.spawn_item(item, at);
                }
            }
            if c.b("QC_MAIN_ShadowCode") && !c.b("MS_SpawnedShit") {
                c.set("MS_SpawnedShit", true, 21);
                for (item, at) in [("WeaponFatRifle", "FatRifleSpawn"), ("Lockpick", "WinqSpawn1"), ("BioelectricCell", "WinqSpawn2"), ("AmmoFatRifle", "WinqSpawn3")] {
                    c.spawn_item(item, at);
                }
            }
            if c.b("HAS_Newsscript") && !c.b("MS_SpawnedShit") {
                c.set("MS_SpawnedShit", true, 21);
                c.spawn_item("WeaponFatRifle", "FatRifleSpawn");
                c.spawn_item("AmmoFatRifle", "WinqSpawn3");
                c.pm("PM_TG_PHATRifle");
            }
            // The aug container's force field is shut on arrival.
            for i in c.mover_ids("AugContainerField") {
                if c.movers.rt[i].key_num == 1 {
                    c.untrigger("AugContainerField");
                    break;
                }
            }
        }
        "20_PDXHQ02" => {
            if c.b("HAS_Newsscript") {
                c.show("DespotOfficeDatacube", true);
                for t in ["DXAGuard_Mark1", "DXAGuard_Mark2", "DXAGuard_Mark3", "DXAGuard_Mark4"] {
                    c.enter_tag(t);
                }
            }
            if c.b("QC_MAIN_WCSpyware") {
                if !c.b("QC_MAIN_ShadowCode") {
                    c.leave_tag("KingKashue");
                }
                c.enter_tag("Phasmatis");
            }
            if c.b("QC_MAIN_ShadowCode") {
                crate::keys::give_pawn_key("KingKashue", "DXIYardGate", &text("strDXIKey"));
                if !c.b("QR_MAIN_Firestaff") {
                    c.enter_tag("KingKashue");
                }
            }
        }
        "20_DESPOT" => {
            if !c.b("KillCount_PDXDone") {
                if c.int("KillCount_PDXFloor1") + c.int("KillCount_PDXFloor2") > num("PDXFloor1AggressiveKills") {
                    c.set("Killer_PDX", true, 23);
                }
                c.set("KillCount_PDXDone", true, 23);
            }
        }
        "20_FCDOWNTOWN" => {
            if c.b("MS_CloseSols") {
                c.seal_door("SolsDoor");
                c.leave(Sel::isa_bind("TNMPawn", "DowntownFemale04"));
            }
            let from_east = ["20_FCCORPORATE", "20_FCSLUMS", "20_VOODOOSHOP"].contains(&last.as_str());
            if from_east && !c.b("MS_DTCleanup_Done") {
                downtown_cleanup(c);
                c.set("MS_DTCleanup_Done", true, 21);
            }
            if c.b("QR_MAIN_GetNewsscript") && !c.b("Kylie_TOLD_Despot") && !c.b("Kylie_Unconscious") {
                c.enter_tag("Kylie");
            }
            if c.b("Kylie_TOLD_DXO") && c.b("Kylie_Dislike") {
                c.leave_tag("Kylie");
            }
            if c.b("DT_TOLD_GoatCity") {
                c.frobbable("GoatPortalWall");
            }
            if last == "20_GOATCITY" && c.b("QC_CULT_DrainTheMelk") && !c.b("KillCount_GoatCityDone") {
                c.set("KillCount_GoatCityDone", true, 21);
            } else if last == "20_LLAMATEMPLE" && c.b("QC_CULT_DestroyTheTablets") && !c.b("KillCount_LlamaTempleDone") {
                c.set("KillCount_LlamaTempleDone", true, 21);
            }
        }
        "20_FCCORPORATE" => {
            if c.b("MS_ClosePartyZone") {
                c.seal_door("PartyZoneDoor");
                c.leave_tag("Shaina");
            }
            for (flag, pm) in [
                ("Scara_VisitedSlumsEarly", "PM_Scara_SlumsEarly"),
                ("QC_SUB_N00Bz", "PM_Phas_N00BzDebriefing"),
                ("QC_MAIN_WCSpyware", "PM_Scara_PostSpywareThreat"),
            ] {
                if c.b(flag) {
                    c.pm(pm);
                }
            }
            if c.b("HAS_Newsscript") && !c.b("QC_MAIN_GetNewsscript") && !c.down("Zero") {
                c.enter_tag("ZeroPresence");
            }
            if last == "20_DESPOT" {
                c.open_door("DespotMover");
                crate::triggers::set_active("DespotMoverTrigger", true);
                c.leave_tag("ZeroPresence");
            }
            if c.any(&["ALLIED_PDX", "SD_Escaped", "Scara_Dead"]) {
                c.ally(Sel::tag("FireWall2"), 1.0, true);
                c.ally(Sel::isa("WCSecurity"), -1.0, true);
                c.hostile("WCHQPatrolBot");
                for t in ["SecurityBot2PDX", "MilitaryBotPDX", "SecurityBot2PDXHQ"] {
                    c.ally(Sel::tag(t), 1.0, true);
                }
            }
            if c.b("ALLIED_WC") && c.b("Killed_20_PDXHQ01_and_20_PDXHQ02_0") {
                c.hostile("SecurityBot2PDXHQ");
            }
            if (last == "20_WCFLOOR1" || last == "20_WCFLOOR2") && c.b("QC_MAIN_WCSpyware") && !c.b("KillCount_WCHQDone") {
                c.set("KillCount_WCHQDone", true, 23);
            }
        }
        "20_FCSLUMS" => {
            if !c.b("QC_VisitFaction") && !c.b("QC_MAIN_SaveSD") {
                c.pm("PM_Phas_SlumsEarly");
            }
            if c.b("QR_MAIN_Firestaff") && !c.b("MS_DXIPrepared") {
                if c.b("ALLIED_PDX") {
                    c.enter_tag("PimpJojo");
                    c.enter_tag("ScaraHologram");
                }
                if c.b("ALLIED_WC") {
                    c.enter_tag("KKHologram");
                }
                c.leave_tag("SiloMechanic");
                c.show("SiloMechanicDatacube", true);
                c.open_door("SiloGate");
                c.set("MS_DXIPrepared", true, 21);
            }
        }
        "20_WEAPONSHOP" => {
            if c.b("HotelMissionComplete") && !c.b("MS_HooverSpawned") {
                c.enter_tag("HooverContact");
                c.leave_tag("TransientBot");
                c.set("MS_HooverSpawned", true, 21);
            }
            if c.b("PM_Hoover_TurnIn_Played") && !c.b("MS_Hoover_TurnIn") {
                c.destroy_actors(ActorSel::tag("HooverCredits"));
                c.set("MS_Hoover_TurnIn", true, 21);
            }
        }
        "20_SOLSBAR" => {
            if c.b("QR_MAIN_GetNewsscript") {
                if c.down("Kylie") {
                    crate::devices::set_strength("DXOKeypad", 0.0);
                    c.set("DXOSpy", true, 21);
                }
                c.leave_tag("Kylie");
                c.leave_tag("ZeroPresence");
            }
            if last == "20_DXO" && c.b("HAS_Newsscript") && !c.b("KillCount_DXODone") {
                if c.int("KillCount_DXO") >= num("DXOAggressiveKills") {
                    c.set("Killer_DXO", true, 23);
                }
                c.set("KillCount_DXODone", true, 21);
            }
            if c.b("HAS_SubwayPass") && !c.b("MS_SolsTrouble") && !c.b("MS_CloseSols") {
                c.enter_tag("Gambler");
            }
            if c.b("Kylie_Dislike") {
                c.leave_tag("Kylie");
            }
        }
        "20_VOODOOSHOP" => {
            if c.b("QR_CULT_FindTheGoats") && !c.b("DT_TOLD_GoatCity") {
                c.enter_tag("DarkTemplar_VoodooShop");
            }
            if c.b("DT_TOLD_GoatCity") {
                c.leave_tag("DarkTemplar_VoodooShop");
            }
            if c.b("YardbombOverheard_Played") {
                c.leave_tag("WCGuy");
            }
        }
        "20_GOATCITY" => {
            if !c.b("MS_GoatCityGoalsSet") {
                if c.b("QR_MAIN_StealPassword") {
                    c.add_goal("ServerComplex", "strGoalServerComplex", true, false);
                }
                if c.b("QR_MAIN_ShadowCode") {
                    c.add_goal("ShadowCode", "strGoalShadowCode", true, false);
                }
                c.set("MS_GoatCityGoalsSet", true, 21);
            }
            if c.b("QC_CULT_GetThePlans") {
                c.enter_tag("DarkTemplar");
                c.set("DT_IsInGoatCity", true, 21);
            }
            if c.any(&["QC_MAIN_ShadowCode", "QC_MAIN_StealPassword"]) {
                c.leave_tag("DarkTemplar");
                c.hide(ActorSel { class: Some("TNMCarcass".into()), tag: Some("DarkTemplar".into()) }, true);
            }
            if c.b("QR_CULT_DrainTheMelk") && !c.down("Beefman") {
                c.deco_frobbable("MelkValve1");
                c.deco_frobbable("MelkValve2");
            }
        }
        "20_GOATTEMPLAE" => {
            if c.any(&["QC_MAIN_ShadowCode", "QC_MAIN_StealPassword"]) && !c.b("DarkTemplar_Dead") {
                c.enter_tag("DarkTemplar");
                c.set("DarkTemplar_Unconscious", false, 0);
            }
        }
        "20_WCFLOOR1" | "20_WCFLOOR2" | "20_WCFLOOR3" => {
            if c.b("ALLIED_WC") {
                c.pm("PM_TG_Introduction");
            }
            if c.b("QR_MAIN_WCSpyware") {
                c.pm("PM_Evil_Spyware");
            }
        }
        "20_WCFLOOR4" => {
            if c.b("QC_MAIN_GetNewsscript") && !c.b("MS_20_SpawnedAugGetNewsscript") {
                c.set("MS_20_SpawnedAugGetNewsscript", true, 21);
                c.spawn_item("AugCannisterSS", "FireplaceAugSpawn");
            }
            // The fireplace safe, once open, keeps its secret passage open.
            if c.mover_ids("Fireplace_Safe").iter().any(|&i| c.movers.rt[i].key_num == 2) {
                c.trigger("Fireplace_Events");
            }
            if c.b("QC_MAIN_StealPassword") {
                crate::keys::give_pawn_key("ScaraKing", "DXIYardGate", &text("strDXIKey"));
            }
        }
        "20_WCSUBLEVEL" => {
            if c.b("QC_MAIN_KillPhas") {
                c.leave_tag("slicer");
                c.unlock("SlicerCellDoor");
                if !c.b("WCPrisonGuard_Dead") {
                    c.leave_tag("WCPrisonGuard");
                    c.show("WCPrisonGuardCorpse", true);
                }
            }
        }
        "20_NSC" => nsc_first(c, &last),
        "20_OSC01" => {
            if last == "20_NSC" {
                c.trigger("OSCMainElevator_Seq01");
            }
            if c.b("QC_MAIN_ShadowCode") {
                c.unlock("OSC04Door");
            }
            c.set("20_MineOwnerSetup", true, 21);
        }
        "20_DXO" => {
            if c.b("DXOSpy") {
                c.show("WCSpy", true);
                c.show("WCSpyConfession", true);
            }
            if c.b("QR_MAIN_GetNewsscript") {
                c.pm("PM_TG_DXO");
            }
        }
        "20_FCCORPSEWERS" if c.b("WCHQ_CamerasOffline") => c.cameras_off(),
        _ => {}
    }
    // The sub-level hatch as it was left on another floor.
    if map.starts_with("20_WC") && c.b("WC_ElevatorHatch_Known") {
        let locked = c.b("WC_ElevatorHatch_Locked");
        for i in c.mover_ids("SublevelKeypadHatch") {
            c.movers.rt[i].locked = locked;
            if !locked {
                c.movers.rt[i].lock_strength = 0.0;
            }
        }
        let want_open = c.b("WC_ElevatorHatch_Open");
        let open = c.mover_ids("SublevelKeypadHatch").iter().any(|&i| c.movers.rt[i].key_num == 1);
        if want_open != open {
            c.trigger("SublevelKeypadHatch");
        }
    }
}

/// Back in downtown from the east side for the first time: the street settles what happened while away.
fn downtown_cleanup(c: &mut Ctx) {
    if c.b("QC_SUB_N00Bz") {
        if c.b("Master_Kale_Dead") || c.b("N00Bz_Dead") {
            c.enter_tag("N00BFirewalls");
        }
        if !c.b("Master_Kale_Dead") {
            c.leave_tag("Master_Kale");
        }
        if !c.b("N00Bz_Dead") {
            c.leave_tag("N00B");
        }
    } else {
        if !c.b("N00Bz_Dead") {
            crate::ai::world_req(WorldReq::KillShot(Sel::tag("N00B")));
        }
        if !c.b("Master_Kale_Dead") {
            crate::ai::world_req(WorldReq::KillShot(Sel::tag("Master_Kale")));
        }
        c.enter_tag("N00BFirewalls");
        if c.b("QR_SUB_N00Bz") {
            c.add_goal("N00Bz", "strGoalN00Bz", false, true);
        }
    }
    c.hide(ActorSel { class: Some("TNMCarcass".into()), tag: Some("LlamaHostage".into()) }, true);
    if !c.b("QC_SUB_LlamaRescue") {
        c.set("LlamaHostage_Dead", true, 21);
        if c.b("QR_SUB_LlamaRescue") {
            c.add_goal("LlamaRescue", "strGoalAbom", false, true);
        }
    }
    c.leave_tag("GoatKidnapper");
    crate::triggers::set_active("LlamaHostageGoalUpdate", false);
}

fn nsc_first(c: &mut Ctx, last: &str) {
    let cults_down = (c.down("Abomination")) && c.down("DarkTemplar");
    let goats = c.b("QC_CULT_FindTheGoats");
    if c.b("DT_GaveFakePlans") {
        c.set("ALLIED_CULT_GOAT", false, 0);
        if goats {
            if !c.down("DarkTemplar") {
                c.set("DT_Pissed", true, 23);
                c.pm("PM_CULT_FakePlans_Leak");
            }
        } else {
            c.set("ALLIED_CULT_LLAMA", false, 0);
            if !cults_down {
                c.pm("PM_CULT_FakePlans_NoLeak");
            }
        }
    }
    if c.b("DT_GaveRealPlans") {
        c.set("ALLIED_CULT_LLAMA", false, 0);
        if goats {
            c.set("ALLIED_CULT_GOAT", false, 0);
            if !cults_down {
                c.pm("PM_CULT_RealPlans_Leak");
            }
        } else if !c.down("Abomination") {
            c.set("Abom_Pissed", true, 23);
            c.pm("PM_CULT_RealPlans_NoLeak");
        }
    }
    if c.b("ALLIED_PDX") {
        if !c.b("QC_MAIN_ShadowCode") {
            c.enter_tag("MiB_Lobby");
            c.leave_tag("UpperLobbyPatrollers");
            for t in ["LobbyDoors02", "ServerRoomsABDoors", "ServerRoomCDoors1"] {
                if c.mover_ids(t).iter().any(|&i| c.movers.rt[i].key_num != 0) {
                    c.trigger(t);
                }
                c.lock(t);
            }
        } else {
            c.leave_tag("MiB_Lobby");
        }
    }
    if c.b("ALLIED_WC") {
        c.enter_tag("MiB_Mainframe");
        for p in c.pawns_of("TNMPawn", None) {
            if !["Jim", "Eric", "ThomasGood"].iter().any(|t| p.tag.eq_ignore_ascii_case(t)) {
                c.ally(Sel::isa_tag("TNMPawn", &p.tag), -1.0, true);
            }
        }
        c.hostile("SecurityBot3PDX");
    }
    if last == "20_OSC01" {
        c.trigger("OSCMainElevator_Seq00");
    }
}

/// Mission 20's countdowns running out.
fn m20_countdowns(c: &mut Ctx, map: &str) {
    if c.timer_done("TimerStarted_MeetPhasPM", "Timer_MeetPhasPM") && map == "20_FCDOWNTOWN" && c.credits < 1000 {
        c.pm("PM_Phas_Subway");
    }
    let streets = ["20_FCDOWNTOWN", "20_FCCORPORATE", "20_FCSLUMS"].contains(&map);
    if c.timer_done("TimerStarted_MeetDTPM", "Timer_MeetDTPM") && streets && !c.b("DT_TOLD_GoatCity") {
        if c.any(&["QR_MAIN_ShadowCode", "QR_MAIN_StealPassword"]) && c.b("Abomination_Dead") {
            c.pm(if c.b("QR_CULT_GetThePlans") { "PM_DT_AbortMission" } else { "PM_DT_AbomDead" });
        } else {
            c.pm("PM_DT_MeetMe");
        }
    }
}

fn m20_timer(c: &mut Ctx, map: &str) {
    let streets = map == "20_FCDOWNTOWN" || map == "20_SOLSBAR";
    // Saving up the subway money finishes that job wherever the player is in downtown.
    if c.credits >= 1000 && !c.b("QC_MAIN_SubwayMoney") && streets {
        c.pm(if c.b("QR_MAIN_SubwayMoney") { "PM_Phas_TestCompleted" } else { "PM_Phas_SubwayEarly" });
        c.award(1000, "strPiggyBank");
        c.set("QC_MAIN_SubwayMoney", true, 0);
    }
    if map == "20_WCHOTEL" && c.down("Bogie") && !c.b("MS_BogieGoal") {
        c.trigger("BogieLabDispatcher");
        c.set("MS_BogieGoal", true, 0);
    }
    if streets {
        if c.credits >= 1500 && !c.b("HAS_SubwayPass") {
            c.pm("PM_Scara_SubwayMoneyAlt");
        }
        if c.b("ALLIED_WC") && c.b("Kylie_Dead") && !c.b("Scara_TOLD_KylieKill") {
            c.pm("PM_Scara_KilledKylie");
            c.set("Kylie_Dead_M20", true, 23);
        }
    }
    match map {
        "20_FCDOWNTOWN" => downtown_timer(c),
        "20_FCCORPORATE" | "20_FCSLUMS" => {
            if c.b("ZPDev_Played") && !c.b("MS_PostZPDev") {
                c.leave(Sel::isa_bind("ScriptedPawn", "ZPDev"));
                c.leave(Sel::isa_bind("ScriptedPawn", "Hawk"));
                c.set("MS_PostZPDev", true, 21);
            }
        }
        "20_LLAMATEMPLE" => {
            if c.down("Beefman") && c.b("QR_CULT_DrainTheMelk") && !c.b("QC_CULT_DrainTheMelk") && !c.b("MS_DrainTheMelkFailed") {
                c.add_goal("DrainTheMelk", "strBeefmanDead", false, true);
                c.set("MS_DrainTheMelkFailed", true, 21);
            }
        }
        "20_VOODOOSHOP" => voodoo_timer(c),
        "20_GOATCITY" | "20_GOATTEMPLAE" => {
            if c.down("DarkTemplar") && c.b("QR_CULT_DestroyTheTablets") && !c.b("MS_DestroyTheTablets_Failed") {
                c.add_goal("DestroyTheTablets", "strDarkTemplarDead", false, true);
                c.set("MS_DestroyTheTablets_Failed", true, 21);
            }
        }
        "20_WCFLOOR1" => {
            if c.b("Silver_Dead") && !c.b("QC_MAIN_SaveSD") && !c.b("SD_KilledAnyway") && !c.b("QC_MAIN_KillSD") {
                c.pm("PM_KilledSD");
                c.award(750, "strDragonSlayerAward");
                c.set("QC_MAIN_KillSD", true, 0);
            }
            if c.b("QC_MAIN_SaveSD") && !c.b("MS_SavedSD_AwardGiven") {
                c.pm("PM_SavedSD");
                c.award(750, "strRescueSilver");
                c.set("MS_SavedSD_AwardGiven", true, 21);
            }
            if c.b("QC_MAIN_SaveSD") && c.b("Silver_Dead") && !c.b("MS_SilverInTemple") {
                c.pm("PM_SavedThenKilledSD");
            }
        }
        "20_WCFLOOR2" | "20_WCFLOOR3" => {
            if c.b("WCHQ_SpywarePlaced") && c.b("WCHQ_ESFDisabled") {
                c.pm("PM_Evil_SpywareQC");
            }
            if c.b("WCHQ_CamerasOffline") {
                c.cameras_off();
            }
            // The spyware box needs a multitool: one is put by the stairs for a player who has none and can't buy one.
            if !c.b("WCHQ_SpywarePlaced") && !c.has_multitool && c.credits < 250 && !c.b("MS_SoftlockMultitool") {
                c.spawn_item("Multitool", "MultitoolSpawn");
                c.set("MS_SoftlockMultitool", true, 21);
            }
        }
        "20_WCFLOOR4" if c.b("QR_SUB_PDXVirus") => c.pm("PM_TG_PDXVirus"),
        "20_WCSUBLEVEL" => {
            if c.b("Quad_PlansInserted") && c.b("Quad_CodesInserted") {
                c.set("Quad_ReadyToSpawn", true, 21);
            }
        }
        _ => {}
    }
}

fn downtown_timer(c: &mut Ctx) {
    if c.b("Abom1_Played") || (c.b("Abomination_Dead") && c.b("Abom_SDSituationResolved")) {
        c.unlock("LlamaTempleDoors");
    }
    c.timer_add("TimerStarted_MeetPhasPM", "Timer_MeetPhasPM", 10);
    if c.b("QR_CULT_FindTheGoats") || c.down("Beefman") {
        c.timer_add("TimerStarted_MeetDTPM", "Timer_MeetDTPM", 10);
    }
    // The N00Bz gang: gone without anyone left standing counts as dead.
    if !c.b("N00Bz_Dead") {
        let alive = c.count("TNMPawn", Some("N00B")) > 0 || c.carcs_of("TNMCarcass", Some("N00B")).iter().any(|b| b.unconscious);
        if !alive {
            c.set("N00Bz_Dead", true, 21);
            if !c.b("Master_Kale_Dead") && !c.b("Kale_InCustody") {
                c.pm("PM_Firewall_KilledGang");
            }
        }
    }
    if c.b("N00Bz_Dead") {
        if c.b("Master_Kale_Dead") {
            c.pm("PM_Firewall_KilledEveryone");
        } else if c.b("Kale_InCustody") {
            c.pm("PM_Firewall_ArrestedKilled");
        }
    }
    if c.b("Master_Kale_Unconscious") {
        c.pm("PM_Firewall_KaleKnockout");
    }
    if c.carcs_of("TNMCarcass", Some("N00B")).iter().any(|b| b.unconscious) {
        c.pm("PM_Firewall_N00BKnockout");
    }
    // Side jobs that end when the person behind them is taken out.
    let sym = c.any(&["QR_SUB_SymDisc01", "QR_SUB_SymDisc02"]) && !c.any(&["QC_SUB_SymDisc00", "QC_SUB_SymDisc01", "QC_SUB_SymDisc02"]);
    let jobs: [(bool, &str, &str, &str, &str, &str); 4] = [
        (sym, "Kunio", "FindTheFloppy", "strKunioDead", "strKunioUnconscious", "QC_SUB_SymDisc00"),
        (c.b("QR_SUB_N00Bz") && !c.b("QC_SUB_N00Bz"), "DowntownFirewall", "N00Bz", "strFirewallDead", "strFirewallUnconscious", "QC_SUB_N00Bz"),
        (c.b("QR_SUB_JimmyHeirloom") && !c.b("QC_SUB_JimmyHeirloom"), "jimmycracker", "JimmyHeirloom", "strJimmyDead", "strJimmyUnconscious", "QC_SUB_JimmyHeirloom"),
        (c.b("QR_SUB_LlamaRescue") && !c.b("QC_SUB_LlamaRescue"), "Abomination", "LlamaRescue", "strAbomDead", "strAbomUnconscious", "QC_SUB_LlamaRescue"),
    ];
    for (open, who, goal, dead, ko, done) in jobs {
        if !open {
            continue;
        }
        if c.b(&format!("{who}_Dead")) {
            c.add_goal(goal, dead, false, true);
            c.set(done, true, 21);
        } else if c.b(&format!("{who}_Unconscious")) {
            c.add_goal(goal, ko, false, true);
            c.set(done, true, 21);
        }
    }
    if c.b("QR_CULT_FindTheGoats") && !c.b("QC_CULT_FindTheGoats") && !c.b("MS_ClosedGoal_GoatCity") {
        if c.b("Abomination_Dead") {
            c.add_goal("FindTheGoats", "strAbomDead2", false, true);
            if c.b("QR_CULT_GetThePlans") && !c.b("QC_CULT_GetThePlans") && !c.b("DarkTemplar_Dead") {
                c.add_goal("GetThePlans", "strDarkTemplar", false, false);
            }
            c.set("MS_ClosedGoal_GoatCity", true, 21);
        } else if c.b("Abomination_Unconscious") {
            c.add_goal("FindTheGoats", "strAbomUnconscious2", false, true);
            c.set("MS_ClosedGoal_GoatCity", true, 21);
        }
    }
    // The subway-pass hacker job.
    let used = c.b("NVShacker_DatacubeUsed");
    let pass = c.b("HAS_SubwayPass");
    if used && !c.b("HAS_NVShackerProgram") && !pass && c.down("AlphaOperator") {
        c.pm("PM_NVS_MissedFloppy");
    }
    if used && c.b("HAS_NVShackerProgram") && !c.b("MS_NVShackerEvaluated") && !pass {
        c.pm("PM_NVS_Evaluation");
    }
    if used && pass {
        c.pm("PM_NVS_GotPass");
    }
    if c.b("QR_SUB_SubwayMoneyAlt") && pass && !c.b("MS_NVShackerDone") {
        c.set("NVShacker_DatacubeErased", true, 21);
        let key = if c.b("QC_SUB_SubwayMoneyAlt") { "strSubPassNVS" } else { "strSubPass" };
        c.add_goal("SubwayMoneyAlt", key, false, true);
        c.set("MS_NVShackerDone", true, 21);
    }
}

fn voodoo_timer(c: &mut Ctx) {
    let yard_down = c.down("Yardbomb");
    if yard_down && c.b("QR_SUB_VoodooStuff") && !c.b("QC_SUB_VoodooStuff") && !c.b("MS_VoodooStuffFailed") {
        c.add_goal("VoodooStuff", "strYardbombDead", false, true);
        c.set("MS_VoodooStuffFailed", true, 21);
    }
    if yard_down && !c.b("QC_SUB_VoodooStuff") && !c.b("MS_VoodooStuffFailed2") {
        crate::cinematic::set_teleporter_enabled("VoodooTele", false);
        c.set("MS_VoodooStuffFailed2", true, 21);
    }
    if c.b("QC_SUB_VoodooStuff") && !c.b("MS_VoodooStuff_AwardGiven") {
        c.award(200, "strVoodooAward");
        c.set("MS_VoodooStuff_AwardGiven", true, 21);
    }
    if c.b("DT_TOLD_GoatCity") && !c.b("MS_GoatCity_AwardGiven") {
        c.award(250, "strInvestigationAward");
        c.set("MS_GoatCity_AwardGiven", true, 21);
    }
    if c.down("DarkTemplar") && !c.b("MS_ClosedDTGoal") {
        if c.b("QR_CULT_GetThePlans") {
            c.add_goal("GetThePlans", "strDarkTemplarDead", false, true);
            c.set("MS_ClosedDTGoal", true, 21);
        } else if !c.b("DT_AbomDead_Played") {
            c.add_goal("MeetDT", "strDarkTemplarUnconscious", false, true);
            c.set("MS_ClosedDTGoal", true, 21);
        }
    }
    // The goat portal picture in the DataVault tells the player where Goat City is.
    if !c.b("DT_TOLD_GoatCity") && crate::consys::has_image("TNMDV_GoatPortal") {
        c.set("DT_TOLD_GoatCity", true, 21);
        if !c.down("Abomination") {
            c.add_goal("FindTheGoats", "strDarkTemplarRobbed", false, false);
        }
        if c.b("QR_MAIN_ShadowCode") {
            c.add_goal("ShadowCode", "strShadowCode", true, false);
        }
    }
}

impl Ctx<'_> {
    /// The apartment door: shut and locked (it stays a door the key opens).
    fn seal_door_keep_usable(&mut self, tag: &str) {
        let ids = self.mover_ids(tag);
        if ids.iter().any(|&i| self.movers.rt[i].key_num != 0) {
            self.trigger(tag);
        }
        self.lock(tag);
    }
    /// A NanoKey placed where an actor with `at_tag` stands (its description from the script's strings).
    fn give_key_at(&self, key_id: &str, at_tag: &str, text_key: &str) {
        let Some(a) = self.lvl.actors.iter().find(|a| tag_of(&a.props).eq_ignore_ascii_case(at_tag)) else { return };
        let l = a.location();
        let props = vec![prop("KeyID", Value::Name(key_id.to_string())), prop("Description", Value::Str(text(text_key)))];
        if let Ok(mut q) = ITEMS.lock() {
            q.push(("NanoKey".into(), DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), props));
        }
    }
    /// A decoration made usable (and highlighted) by the script.
    fn deco_frobbable(&self, tag: &str) {
        crate::frobui::force_highlight(tag);
    }
    /// UnTrigger the actors with this Tag.
    fn untrigger(&self, tag: &str) {
        if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
            q.push((tag.to_string(), false));
        }
    }
}

/// A mission-script function the map asks for (MSCallbackTrigger's ScriptFunction).
pub fn game_event(c: &mut Ctx, f: &str) {
    println!("MISSION game event {f}");
    match f {
        "AbomOneliner02" => c.conversation("AbomKilledBG", "Abomination"),
        "AbomCutsceneEscape" => c.leave_tag("AbomBG"),
        "GiveNSCKey" => {
            let desc = text("strNSCKey");
            let msg = crate::keys::give("NSCKey", &desc);
            if let Some(cs) = crate::consys::lock().as_mut() {
                cs.message(&msg);
            }
        }
        "GhandCatchFire" => crate::ai::world_req(WorldReq::Ignite(Sel::tag("Ghand"))),
        "PhasLockDoor" => {
            c.seal_door("PhasAptDoor");
            c.leave_tag("Phasmatis");
        }
        // The elevator between the two server complexes goes on a dispatcher's delay, from its resting end only.
        "OSCElevator_To_OSC" | "OSCElevator_To_NSC" => {
            let want = if f.ends_with("OSC") { 0 } else { 1 };
            let ok = f.ends_with("OSC") || c.b("QC_MAIN_ShadowCode");
            if ok && c.mover_ids("OSCMainElevator").iter().any(|&i| c.movers.rt[i].key_num == want) {
                c.trigger("OSCMainElevator_Dispatcher");
            }
        }
        "DXIHologramDisappear" => {
            c.leave_tag("ScaraHologram");
            c.leave_tag("KKHologram");
        }
        "SummonDevs" => {
            c.enter(Sel::isa_bind("ScriptedPawn", "ZPDev"));
            c.enter(Sel::isa_bind("ScriptedPawn", "Hawk"));
        }
        "WrenchAward" => c.award(25, "strWrenchAward"),
        "TurnMelkValve1" | "TurnMelkValve2" => crate::destruct::make_vulnerable(&f["Turn".len()..]),
        "DestroyedValve" => {
            let n = c.int("MS_ValvesDestroyed");
            if n == 0 {
                c.pm("PM_Abom_FirstValve");
                c.add_goal("DrainTheMelk", "strGoalDrainTheMelk1", false, false);
            } else if n == 1 {
                c.pm("PM_Abom_SecondValve");
                c.add_goal("DrainTheMelk", "strGoalDrainTheMelk2", false, false);
                c.award(500, "strSabotageAward");
                c.set("GoatValvesDestroyed", true, 23);
            }
            c.set_int("MS_ValvesDestroyed", n + 1, 21);
        }
        _ => println!("MISSION game event {f}: not handled yet"),
    }
}
