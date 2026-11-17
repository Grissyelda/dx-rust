//! The Nameless Mod's mission scripts, written for DX RUST from how they behave in the player's own copy of the mod
//! (its TNM.u). Nothing of the mod is shipped: the flag, tag and map names below are the identifiers its maps and
//! conversations use, and every line of text a script shows (goal texts, award messages) is read at run time from
//! the mission script's own defaults in the player's files (`load_text`).
//!
//! Covered: the shared mission-script layer (count of the living per map, flag-held countdowns, the last map, the
//! killed-important-characters rule), Mission 20 (the first day in Forum City) and 21 (DXI) here - each map's set-up
//! on arrival, its once-a-second checks and what it records on leaving - and missions 22 to 25 in their own modules.

use super::*;
use std::collections::HashMap;

mod m22;
mod m23;
mod m24;
mod m25;

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

/// Pawns a script spawns: (class, where, yaw, property overrides).
pub(super) static PAWNS: Mutex<Vec<(String, DVec3, i32, Vec<Property>)>> = Mutex::new(Vec::new());

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

    /// AddGoal with a text of our own (the script's own text for it isn't one of its localized strings).
    fn add_goal_text(&mut self, name: &str, text: &str, primary: bool, completed: bool) {
        if let Some(cs) = crate::consys::lock().as_mut() {
            cs.add_goal(name, text, primary);
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

/// Mission 25's script also runs the intro, ending and epilogue maps (TNMIntro, TNMEndgame*, TNMDenouement).
fn is_m25(c: &Ctx) -> bool {
    c.mission == 25 || c.map.starts_with("TNM")
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
    if c.mission == 21 {
        m21_first(c, &map);
    }
    if c.mission == 22 {
        m22::first(c, &map);
    }
    if c.mission == 23 {
        m23::first(c, &map);
    }
    if c.mission == 24 {
        m24::first(c, &map);
    }
    if is_m25(c) {
        m25::first(c, &map);
    }
}

/// The player took someone out (TNMMissionScript.PawnKilled): in PDX's headquarters, killing anyone but its
/// animals, robots and invaders, unless allied with the Wild Cards, turns PDX on him.
pub fn pawn_killed(c: &mut Ctx, class: &str, tag: &str, knocked_out: bool) {
    let _ = tag;
    if c.mission == 23 {
        m23::pawn_killed(c, class, knocked_out);
    }
    if c.mission == 24 && !knocked_out {
        m24::pawn_killed(c, class);
    }
    if !(c.mission == 20 || c.mission == 22) {
        return;
    }
    let pdx = c.map.ends_with("_PDXHQ01") || c.map.ends_with("_PDXHQ02");
    if !pdx || c.b("ALLIED_WC") || c.b("MS_PDX_Murderer") {
        return;
    }
    let is = |k: &str| crate::classtree::is_a(class, k);
    if is("NBDRat") || is("TNMCat") || is("Invader") || is("TNMRobot") || !is("TNMPawn") {
        return;
    }
    if !knocked_out || !is("Jonas") {
        c.set("PDX_Dead", !knocked_out, 21);
        pdx_murder(c);
    } else {
        c.pm("PM_Evil_JonasKnockout");
    }
}

/// PDXHQMurder: everyone in the building turns on the player, Kashue comes for him and every way out is shut.
fn pdx_murder(c: &mut Ctx) {
    if c.b("MS_KashueKilling") {
        return;
    }
    c.set("MS_KashueKilling", true, 23);
    let kashue_here = c.pawns.iter().any(|p| p.in_world && (p.tag.eq_ignore_ascii_case("KingKashue_Evil") || p.tag.eq_ignore_ascii_case("KingKashue")));
    c.leave_tag("KingKashue_Evil");
    c.leave_tag("KingKashue");
    if !kashue_here {
        c.pm("PM_Kashue_KilledFriend");
    }
    for t in ["Krissy", "Evil", "Jonas", "Winquman"] {
        c.hostile(t);
    }
    c.enter_tag("KingKashue_Attack");
    for d in ["PDXMainDoor", "PDXSewerGrate", "StaircaseDoor"] {
        c.seal_door(d);
    }
    crate::cinematic::redirect_exits(None);
    crate::triggers::set_active("Exit1Trigger", false);
    crate::triggers::set_active("Exit2Trigger", false);
}

/// Missions 20 and 22's shared checks, once a second.
fn m20and22_timer(c: &mut Ctx, map: &str) {
    if c.b("Scara_Dead") {
        crate::cinematic::redirect_exits(Some(("20_WCFloor4#Narcissus", "Narcissus")));
    }
    if !c.b("CheckedSubwayPass") && c.inv_classes.iter().any(|k| k.eq_ignore_ascii_case("SubwayPass")) {
        c.add_goal("SubwayMoney", "strHasSubPass", true, true);
        c.set("CheckedSubwayPass", true, 23);
    }
    if map == "20_FCSLUMS" || map == "22_FCDOWNTOWN" {
        if map == "20_FCSLUMS" && c.down("PJ") {
            c.set("PJ_Dead_Early", true, 23);
        }
        if c.b("QR_SUB_Bugging") && !c.b("QC_SUB_Bugging") {
            if c.b("PJ_Dead") {
                c.add_goal("Bugging", "strPJDead", false, true);
                c.set("QC_SUB_Bugging", true, 23);
            } else if c.b("PJ_Unconscious") {
                c.add_goal("Bugging", "strPJUnconscious", false, true);
                c.set("QC_SUB_Bugging", true, 23);
            }
        }
    }
    let exp = if c.mission == 20 { 21 } else { 23 };
    if map.ends_with("_LLAMATEMPLE") {
        // Beefman turned hostile: his turrets drop and his force field goes; once they're destroyed, the field
        // goes down again and Beefman fights in person.
        if !c.b("CheckedBeefmanTurretsDrop") && c.pawns.iter().any(|p| p.tag.eq_ignore_ascii_case("Beefman") && p.hostile) {
            c.trigger("BeefmanTurretHatches");
            c.trigger("BeefmanForceField");
            c.set("CheckedBeefmanTurretsDrop", true, exp);
            c.set("CheckBeefmanTurretsDie", true, exp);
        }
        if c.b("CheckBeefmanTurretsDie") && c.deco_exists("DeusExDecoration", Some("BeefmanTurret")) == 0 {
            c.hostile("Beefman");
            c.trigger("BeefmanForceField");
            c.set("CheckBeefmanTurretsDie", false, exp);
        }
    }
    if map.ends_with("_PDXHQ01") {
        if c.b("ALLIED_WC") && map != "22_PDXHQ01" && c.b("QR_SUB_PDXVirus") && c.b("Evil_Dead") {
            c.pm("PM_Scara_EvilDead");
        }
        if !c.b("ALLIED_WC") && !c.b("MS_PDX_Murderer") && c.b("Jonas_Dead") {
            c.set("PDX_Dead", true, 21);
            pdx_murder(c);
        }
    }
}

/// Once a second.
pub fn timer(c: &mut Ctx) {
    let map = c.map.clone();
    let _ = c.living(&map);
    if c.mission == 20 || c.mission == 22 {
        m20and22_timer(c, &map);
        flushed(c);
    }
    if c.mission == 20 {
        for (s, l) in M20_TIMERS {
            c.timer_tick(s, l);
        }
        m20_countdowns(c, &map);
        m20_timer(c, &map);
    }
    if c.mission == 21 {
        m21_timer(c, &map);
    }
    if c.mission == 22 {
        m22::timer(c, &map);
    } else if c.mission == 23 {
        m23::timer(c, &map);
    } else if c.mission == 24 {
        m24::timer(c, &map);
    } else if is_m25(c) {
        m25::timer(c, &map);
        if let Ok(mut q) = super::DETECTED.lock() {
            q.clear();
        }
    } else if let Ok(mut q) = super::DETECTED.lock() {
        q.clear();
    }
}

/// Leaving a map: what it records.
pub fn pre_travel(
    fl: &mut FlagBase,
    mission: i32,
    map: &str,
    movers: &crate::movers::Movers,
    lvl: &dx_level::Level,
    present: &dyn Fn(usize) -> bool,
) {
    fl.set("TNMLastMap", FlagValue::Name(map.to_string()), 0);
    if mission == 24 {
        m24::pre_travel(fl);
    }
    // Forum City is visited twice (missions 20 and 22): what was taken or used up the first time stays gone.
    if mission == 20 || mission == 22 {
        if let Some((flag, tags)) = persistent(map, false) {
            let mut bits = 0i32;
            for (i, tag) in tags.iter().enumerate() {
                let mut found = false;
                for (ai, a) in lvl.actors.iter().enumerate() {
                    if tag_of(&a.props).eq_ignore_ascii_case(tag) {
                        found = present(ai);
                        break;
                    }
                }
                if !found {
                    bits |= 1 << i;
                }
            }
            fl.set(flag, FlagValue::Int(bits), 23);
        }
    }
    let b = |fl: &FlagBase, n: &str| fl.get_bool(n);
    if mission == 20 {
        match map {
            "20_FCDOWNTOWN" if b(fl, "BreakAndEnter") => fl.set("MS_BreakAndEnter", FlagValue::Bool(true), 21),
            "20_SOLSBAR" if b(fl, "MS_SolsTrouble") => fl.set("MS_SolsTrouble_Post", FlagValue::Bool(true), 23),
            _ => {}
        }
    }
    if mission == 20 || mission == 22 {
        // The Wild Cards HQ's sub-level hatch keeps its state between the floors.
        let exp = if mission == 20 { 21 } else { 23 };
        if map.starts_with("20_WC") || map.starts_with("22_WC") {
            for (i, m) in lvl.movers.iter().enumerate() {
                if m.tag.eq_ignore_ascii_case("SublevelKeypadHatch") {
                    let rt = &movers.rt[i];
                    fl.set("WC_ElevatorHatch_Locked", FlagValue::Bool(rt.locked), exp);
                    fl.set("WC_ElevatorHatch_Open", FlagValue::Bool(rt.key_num == 1), exp);
                    fl.set("WC_ElevatorHatch_Known", FlagValue::Bool(true), exp);
                }
            }
        }
    }
}

// =================================================================================================== Missions 20/22

/// Things placed in both visits to a Forum City map that only exist once: (map name after "20_" / "22_", the flag
/// that remembers them, their Tags). Saved on leaving the mission-20 map, removed on arriving at the mission-22 one.
const PERSISTENT: &[(&str, &str, &[&str])] = &[
    ("GOATTEMPLAE", "PersistentLevel_GoatTemplae_1", &["TemplaeSkillAwardAOM", "TemplaeSkillAwardCave", "TemplaeAug", "TemplaeRebreatherCrate"]),
    ("WCSUBLEVEL", "PersistentLevel_WCSublevel_1", &[
        "SublevelAug", "SublevelAugUpgrade", "SublevelModRange", "SublevelModRecoil", "SublevelJumpShotgun", "SublevelJumpShell1",
        "SublevelJumpShell2", "SublevelJumpSA", "SublevelExplosionSA", "SublevelUCSA", "SublevelPrisonKey", "SublevelGasroomMedkit",
        "SublevelGasroomCrate", "KylieCredits", "ZPPistol", "SublevelLAMCrate", "FistPickupStun", "SublevelAugUpgradeZP",
    ]),
    ("WCFLOOR4", "PersistentLevel_WCFloor4_1", &["WC4Upgrade"]),
    ("FCDOWNTOWN", "PersistentLevel_FCDowntown_1", &[
        "DowntownApollo_SkillAward", "DowntownCanals_SkillAward", "DowntownCanals_Lockpick1", "DowntownCanals_Lockpick2",
        "DowntownCanals_ReloadMod", "DowntownCanals_Liquor", "DowntownAlley_MedKit", "DowntownAlley_RecoilMod", "DowntownAlley_Sabot",
        "DowntownShip_Shotgun", "DowntownShip_LAM1", "DowntownShip_LAM2",
    ]),
    ("WEAPONSHOP", "PersistentLevel_WeaponShop_1", &[
        "FistPickupLethal", "Locker_Aug", "Locker_Credits", "Locker_EMPGrenade", "Locker_Ammo45", "Locker_BECell1", "Locker_BECell2",
        "WeaponDoubleGuns", "Locker_Pistol", "Locker_10mm1", "Locker_10mm2", "Locker_GasGren1", "Locker_GasGren2", "Locker_GasGren3",
        "Locker_Blowdart1", "Locker_Blowdart2", "Locker_20mm1", "Locker_20mm2", "Locker_EMPGrenade2", "Locker_Shell", "Locker_Scramble",
        "Locker_3006", "Locker_Mine", "Locker_Plasma", "Locker_Prod", "Locker_ProdBattery", "Locker_Pepper1", "Locker_Pepper2",
        "Locker_Darts", "Locker_Taser",
    ]),
    ("FCSLUMS", "PersistentLevel_Slums_1", &[
        "YardbombAptAward", "FistPickupShock", "ElevatorLiquorBottle", "ElevatorVialCrack1", "ElevatorVialCrack2", "SewerCrate1",
        "SewerCrate2", "SewerFlare1", "SewerFlare2",
    ]),
    ("DXO", "PersistentLevel_DXO_1", &[]),
    ("FCCORPORATE", "PersistentLevel_FCCorporate_1", &[]),
    ("PDXHQ01", "PersistentLevel_PDXHQ01_1", &[]),
    ("PDXHQ02", "PersistentLevel_PDXHQ02_1", &["FistPickupBan"]),
    ("GOATCITY", "PersistentLevel_GoatCity_1", &["SkillAward_Rock", "SkillAward_Cellar", "SkillAward_SpyRoom", "GoatCityTrapdoorKey"]),
    ("FCCORPSEWERS", "PersistentLevel_FCCorpSewers_1", &[
        "ItemCrate01", "ItemCrate02", "ItemCrate03", "ItemCrate04", "ItemCrate05", "ItemCrate06", "ItemCrate07", "ItemCrate08",
        "ItemCrate09", "Molotov01", "Molotov02", "LAM01", "SniperRifle01", "SniperAmmo01", "Multitool01",
    ]),
    ("VOODOOSHOP", "PersistentLevel_VoodooShop_1", &["WeaponKatanaRonin", "WeaponBlowPipe"]),
];

/// The persistent entry of a map: leaving the mission-20 one (`load` false) or arriving at the mission-22 one.
fn persistent(map: &str, load: bool) -> Option<(&'static str, &'static [&'static str])> {
    let rest = map.strip_prefix(if load { "22_" } else { "20_" })?;
    PERSISTENT.iter().find(|(m, ..)| m.eq_ignore_ascii_case(rest)).map(|(_, f, t)| (*f, *t))
}

fn m20and22_first(c: &mut Ctx, map: &str) {
    let m = map.trim_start_matches("20_").trim_start_matches("22_");
    if let Some((flag, tags)) = persistent(map, true) {
        let gone = c.int(flag);
        for (i, tag) in tags.iter().enumerate() {
            if gone & (1 << i) != 0 {
                c.destroy_actors(ActorSel::tag(tag));
                crate::ai::world_req(WorldReq::Destroy(Sel::tag(tag)));
                crate::triggers::set_active(tag, false);
            }
        }
    }
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
    restore_hatch(c, map);
}

/// The Wild Cards' sub-level hatch as it was left on another floor (both visits).
fn restore_hatch(c: &mut Ctx, map: &str) {
    if (map.starts_with("20_WC") || map.starts_with("22_WC")) && c.b("WC_ElevatorHatch_Known") {
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
    if c.mission == 21 && m21_event(c, f) {
        return;
    }
    if c.mission == 22 && m22::event(c, f) {
        return;
    }
    if c.mission == 23 && m23::event(c, f) {
        return;
    }
    if c.mission == 24 && m24::event(c, f) {
        return;
    }
    if is_m25(c) && m25::event(c, f) {
        return;
    }
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
        // Silver Dragon runs for the nearest of his escape points.
        "SDFindPatrol" => {
            if let Some(sd) = c.pawns.iter().find(|p| crate::classtree::is_a(&p.class, "SD")).cloned() {
                let best = (0..=5)
                    .map(|i| format!("SDRunPoint{i}"))
                    .filter_map(|t| c.point("PatrolPoint", &t).map(|(p, _)| ((p - sd.pos).length(), t)))
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, tag)) = best {
                    c.orders(Sel::isa("SD"), "RunningTo", &tag);
                }
            }
        }
        // Zero's sniper marks on PDX's guards: a guard standing at his mark is shot through the window.
        "PDXMark1" | "PDXMark2" | "PDXMark3" | "PDXMark4" => {
            if !c.down("Zero") && !c.b("QC_MAIN_GetNewsscript") {
                pdx_mark(c, f.as_bytes()[7] - b'0');
            }
        }
        // The IRC channel isn't simulated.
        "IRCKick" | "PlayOutroTrack" | "OSC02Fan" => {}
        "BackOutTunnel" => {
            if !c.b("Shawn_PissedOffGuards") {
                c.ally(Sel::tag("PhasecGuards"), 0.0, true);
                c.ally(Sel::tag("Cop1"), 0.0, true);
            }
        }
        "BullseyeCheck" => {
            let p = c.player_pos;
            if p.x > 5880.0 && p.x < 6855.0 && p.y > 11192.0 && p.y < 11911.0 {
                c.trigger("BullseyeAward");
            }
        }
        // Phasmatis has everything the player carries laid out at the item dump (keys stay).
        "PhasTakeInventory" => {
            if let Some((at, _)) = c.point("Actor", "itemdump").or_else(|| c.lvl.actors.iter().find(|a| tag_of(&a.props).eq_ignore_ascii_case("itemdump")).map(|a| {
                let l = a.location();
                (DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), 0)
            })) {
                let spots: Vec<DVec3> = (0..80)
                    .map(|_| {
                        let v = c.vrand();
                        at + DVec3::new(v.x, v.y, 0.0) * 64.0
                    })
                    .collect();
                player_req(PlayerReq::DropInventory(spots));
            }
        }
        "PlayNVSMessage" => {
            crate::consys::start_forced_con(c.fl, "LaptopBriefing");
        }
        "LlamaGuardInvul" => {
            if crate::carry::carried_bind().eq_ignore_ascii_case("LlamaHostage") {
                crate::carry::drop_body();
            }
        }
        "KaleDropBody" => {
            if crate::carry::carried_bind().eq_ignore_ascii_case("Master_Kale") {
                crate::carry::drop_body();
            }
        }
        // The sewer flush: the current runs while the water is let out; what lay in the sewer's pipe or its
        // deposit basin ends up in the treatment plant's lost property (counted by class).
        "FlushStart" => {
            crate::zones::set_velocity("SuccZone", Some(DVec3::new(0.0, -1000.0, 1.0)));
            crate::sound::play_player("DeusExSounds.Weapons.WPApproach", 1.0, 0.0);
        }
        "FlushStop" => {
            crate::zones::set_velocity("SuccZone", None);
            crate::pickups::flush_zones(&["SuccZone", "DepositZone"]);
            c.set("MS_FlushPending", true, 23);
        }
        "DDHologramAppear" => c.enter_tag("DDHologram"),
        "DDHologramDisappear" => c.leave_tag("DDHologram"),
        // Narcissus warns about the weapon cases while their keypads still stand.
        "VoodooCalloutBlowpipe" | "VoodooCalloutKatana" => c.pm("PM_Narc_Yardbomb"),
        "KillTrestkon" => kill_trestkon(c),
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

/// PDXMark: the guard at his mark is shot through his window; two marks (3 and 4) go together.
fn pdx_mark(c: &mut Ctx, n: u8) {
    let at_mark = |c: &Ctx, guard: &str, nav: &str| -> Option<bool> {
        let g = c.pawns.iter().find(|p| p.tag.eq_ignore_ascii_case(guard))?;
        let mark = c.lvl.actors.iter().find(|a| tag_of(&a.props).eq_ignore_ascii_case(nav))?;
        let l = mark.location();
        let d = DVec3::new(g.pos.x - l[0] as f64, g.pos.y - l[1] as f64, 0.0);
        Some(d.length() < 128.0)
    };
    let shoot = |c: &mut Ctx, i: u8| {
        crate::ai::world_req(WorldReq::KillShot(Sel::tag(&format!("DXAGuard_Mark{i}"))));
        c.blow_up(&format!("PDXWindow_Mark{i}"));
    };
    match n {
        1 | 2 => match at_mark(c, &format!("DXAGuard_Mark{n}"), &format!("GP_Mark{n}")) {
            Some(true) => {
                shoot(c, n);
                c.pm(&format!("PM_Zero_PDXMark{n}"));
            }
            Some(false) => c.pm(&format!("PM_Zero_PDXMark{n}_Failed")),
            None => c.pm(&format!("PM_Zero_PDXMark{n}_Dead")),
        },
        3 => match (at_mark(c, "DXAGuard_Mark3", "GP_Mark3"), at_mark(c, "DXAGuard_Mark4", "GP_Mark4")) {
            (Some(true), Some(true)) => {
                shoot(c, 3);
                c.trigger("MSMark4Delay");
            }
            (Some(_), Some(_)) => c.pm("PM_Zero_PDXMark3and4_Failed"),
            _ => c.pm("PM_Zero_PDXMark3and4_Dead"),
        },
        _ => {
            if c.pawns.iter().any(|p| p.tag.eq_ignore_ascii_case("DXAGuard_Mark4")) {
                shoot(c, 4);
                c.pm("PM_Zero_PDXMark3and4");
            }
        }
    }
}

/// The Narcissus loop's end: Trestkon dies one of several ways (the arena's spawn points add the strange ones).
fn kill_trestkon(c: &mut Ctx) {
    let points = c.points("SpawnPoint", "NarcSpawnPoints");
    let n = if points.is_empty() { 3 } else { 8 };
    let pick = (c.rand() * n as f64) as usize % n;
    let hunters = ["TNMGreasel", "MelkyGreasel", "RedGreasel", "Abomnation", "Ghand"];
    match pick {
        0 => player_req(PlayerReq::Kill("Shot")),
        1 => player_req(PlayerReq::Kill("Flamed")),
        2 => player_req(PlayerReq::Kill("Exploded")),
        _ => {
            // Something deathless comes out of every spawn point for him.
            let class = hunters[(pick - 3) % hunters.len()];
            for p in points {
                let d = c.player_pos - p;
                let yaw = (d.y.atan2(d.x) * 32768.0 / std::f64::consts::PI) as i32;
                let props = vec![
                    prop("bInvincible", Value::Bool(true)),
                    prop("Alliance", Value::Name("Narcissus".into())),
                    prop("Orders", Value::Name("Attacking".into())),
                    Property { name: "InitialAlliances".into(), index: 6, value: Value::Alliance("Player".into(), -1.0, true) },
                ];
                if let Ok(mut q) = PAWNS.lock() {
                    q.push((class.to_string(), p, yaw, props));
                }
                c.fx(Fx::SpawnFlash(p));
            }
        }
    }
}

/// What the sewer flush took: counted per class into SewerFlush_<Class> flags for the lost-property clerk.
fn flushed(c: &mut Ctx) {
    if !c.b("MS_FlushPending") {
        return;
    }
    let Some(items) = crate::pickups::take_flushed() else { return };
    c.fl.delete("MS_FlushPending");
    if items.is_empty() {
        return;
    }
    for (class, n) in &items {
        let key = format!("SewerFlush_{class}");
        let v = c.int(&key);
        c.set_int(&key, v + n, 23);
    }
    c.pm("PM_Hoover_FlushedItems");
    if !c.b("MS_FlushedItem") {
        c.add_goal("SewerFlushRetrieval", "strGoalSewerRetrieval", false, false);
    }
    c.set("FlushedItem", true, 23);
    c.set("MS_FlushedItem", true, 23);
}

// =================================================================================================== Mission 21

/// The people Mission 20 knocked out: on to Mission 21 they're remembered as "_M20" (some die in the bombing).
const M20_KNOCKOUTS: [(&str, bool, i32); 8] = [
    // (name, killed in the bombing unless allied with the Wild Cards, flag expiration)
    ("AlphaOperator", true, 23),
    ("Jackie", true, 23),
    ("Jonas", false, 23),
    ("Raving", false, 23),
    ("Andreus", false, 23),
    ("Kylie", false, 24),
    ("NVShacker", false, 23),
    ("NVShackerPet", false, 23),
];

fn m21_first(c: &mut Ctx, map: &str) {
    remove_collapsed_bodies(c);
    match map {
        "21_DXI_FRONTEND" => {
            if !c.b("Mission20Cleanup_Done") {
                c.trigger("StartMissionCinematic");
                if !c.b("KillCount_Mission20Done")
                    && c.int("KillCount_FCDowntown") + c.int("KillCount_FCCorporate") + c.int("KillCount_FCSlums") >= num("FCAggressiveKills")
                {
                    c.set("Killer_FC", true, 23);
                }
                if c.b("QR_SUB_Bugging") {
                    c.add_goal("Bugging", "strGoalBug", false, false);
                }
                for (who, bombed, exp) in M20_KNOCKOUTS {
                    let ko = format!("{who}_Unconscious");
                    if c.b(&ko) {
                        c.set(&format!("{who}_Unconscious_M20"), true, exp);
                        if bombed && !c.b("ALLIED_WC") {
                            c.set(&format!("{who}_Dead"), true, 0);
                        }
                        c.fl.delete(&ko);
                    }
                }
                for who in ["Abomination", "Slicer", "Zero"] {
                    if c.b(&format!("{who}_Dead")) {
                        c.set(&format!("{who}_Dead_M20"), true, 23);
                    }
                }
                if c.b("NVShackerPet_Dead") {
                    c.set("NVShackerPet_Dead_M20", true, 23);
                    c.fl.delete("NVShackerPet_Dead");
                }
                if !c.b("NVShacker_Dead") && !c.b("NVShackerPet_Betrayed") {
                    let wc = c.b("QC_SUB_SubwayMoneyAlt") || c.b("NVShacker_Unhappy");
                    c.set(if wc { "NVShacker_WC" } else { "NVShacker_PDX" }, true, 24);
                }
                if !c.b("NVShackerPet_Dead_M20") {
                    if c.b("NVShackerPet_Unconscious_M20") && (c.b("NVShacker_Dead") || c.b("NVShackerPet_Betrayed")) {
                        c.set("NVShackerPet_Dead_M20", true, 0);
                    } else if !c.b("MET_NVShacker") || c.b("NVShackerPet_Talked") || c.b("Abomination_Dead") {
                        c.set("NVShackerPet_NVS", true, 24);
                    } else {
                        c.set("NVShackerPet_Abom", true, 24);
                    }
                }
                c.set("Mission20Cleanup_Done", true, 22);
            }
            if c.b("DXI_ElevatorActive") {
                c.trigger("LiftActiveFX");
            }
        }
        "21_DXI_MCR" => {
            if c.b("ALLIED_PDX") {
                c.enter_tag("ZeroPresence");
            }
            if c.b("ALLIED_WC") {
                c.enter_tag("Slicer");
            }
            if c.down("Beeblequix") && !c.b("Beeble_Greeting_Played") {
                c.show("SoftlockDatacube", true);
            }
        }
        _ => {}
    }
}

/// After the tunnel collapses, the knocked-out Zero or Slicer left behind there are gone.
fn remove_collapsed_bodies(c: &mut Ctx) {
    if c.b("MS_TunnelCollapsed") {
        for who in ["Zero", "Slicer"] {
            if c.carcs.iter().any(|b| b.unconscious && b.tag.eq_ignore_ascii_case(who)) {
                c.hide(ActorSel { class: Some("TNMCarcass".into()), tag: Some(who.into()) }, true);
            }
        }
    }
}

fn m21_timer(c: &mut Ctx, map: &str) {
    if !c.b("Autosaved_DXI") {
        crate::save::autosave("mission");
        c.set("Autosaved_DXI", true, 21);
    }
    for (flag, class) in [("Jonas_SecretConvo_Played", "Jonas"), ("Phasmatis_SecretConvo_Played", "Phasmatis"), ("Winquman_SecretConvo_Played", "Winquman")] {
        if c.b(flag) {
            c.leave(Sel::isa(class));
        }
    }
    match map {
        "21_DXI_MCR" => {
            if c.down("Zero") && !c.b("Zero_Dead_M20") {
                c.pm("PM_Kylie_ZP");
            }
            if c.any(&["Zero_PreBossFight_Played", "Slicer_PreBossFight_Played"]) && !c.b("MS_DXI_HolosLeftWorld") {
                c.leave_tag("ZeroPresenceHolo");
                c.leave_tag("SlicerHolo");
                c.set("MS_DXI_HolosLeftWorld", true, 22);
            }
            if !c.b("MS_GotStaff") && c.inv_classes.iter().any(|k| k.eq_ignore_ascii_case("WeaponFireStaff")) {
                c.award(2000, "strEpicAward");
                c.pm("PM_MAIN_GotStaff");
                c.set("MS_GotStaff", true, 22);
            }
        }
        "21_DXI_RUINS" => {
            let guards = ["ThreadProtector01", "ThreadProtector02", "ThreadProtector03"];
            if !c.b("DXI_ThreadsClosed") && guards.iter().all(|g| c.down(g)) {
                c.blow_up("ThreadsFacilityFissure");
                c.set("DXI_ThreadsClosed", true, 22);
                c.add_goal("DestroyThreads", "strGoalThreadsClosed", true, true);
                c.pm("PM_MAIN_ThreadsClosed");
                for g in guards {
                    c.fl.delete(&format!("{g}_Dead"));
                    c.fl.delete(&format!("{g}_Unconscious"));
                }
            }
        }
        _ => {}
    }
}

/// Mission 21's script functions asked for by its map.
fn m21_event(c: &mut Ctx, f: &str) -> bool {
    match f {
        "BuzzAppear" => c.enter_tag("Buzz"),
        "BuzzDisappear" => c.leave_tag("Buzz"),
        "RoninAppear" => c.enter_tag("Ronin"),
        "RoninDisappear" => c.leave_tag("Ronin"),
        "GwogDisableCollision" => c.setd("Gwog_Spooked", true),
        // The HUD static of Beeblequix's codes isn't drawn.
        "StaticBurst" | "BeebleStatic" => {}
        "ToggleVents" => {
            let venting = !c.b("DXIHQ_VentingEnabled");
            c.set("DXIHQ_VentingEnabled", venting, 22);
            if c.b("DXIHQ_GasEnabled") {
                crate::projgen::set_spewing("CompRoomGas", !venting);
            }
        }
        "CeilingExplode" => {
            c.breakable("CeilingExplodeMover", true);
            c.blow_up("CeilingExplodeMover");
        }
        "PostCollapse" => {
            crate::scriptworld::push(crate::scriptworld::ActorReq::Hide(ActorSel::class_tag("BlockPlayer", "TunnelBlocker"), false));
            c.breakable("ThreadConsole", true);
            c.set("MS_TunnelCollapsed", true, 22);
            remove_collapsed_bodies(c);
        }
        "ThreadsHacked" => {
            for g in ["ThreadProtector01", "ThreadProtector02", "ThreadProtector03"] {
                crate::ai::world_req(WorldReq::Destroy(Sel::tag(g)));
            }
            c.set("DXI_ThreadsClosed", true, 22);
            c.trigger("ThreadsSelfDestruct");
            c.blow_up("ThreadsFacilityFissure");
            c.pm("PM_MAIN_ThreadsClosed");
        }
        "BeebleOneLiner" | "BeebleRant" => {
            if c.b("BEEBLE_GotHUDCodes") {
                let (stem, n) = if f == "BeebleRant" { ("PM_Beeble_Rant", 10) } else { ("PM_Beeble_OneLiner", 5) };
                for i in 1..=n {
                    c.pm(&format!("{stem}{i:02}"));
                }
            }
        }
        "DXIBSP1" | "DXIBSP2" | "DXIBSP3" | "DXIBSP4" => {
            let tag = &f[3..];
            let flag = format!("MS_{f}");
            if !c.b(&flag) {
                crate::cinematic::set_teleporter_enabled(tag, false);
                c.hide(ActorSel::class_tag("ScaledSprite", tag), true);
                c.set(&flag, true, 22);
            }
        }
        _ => return false,
    }
    true
}
