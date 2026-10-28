//! Mission02 - Mission15 and MissionEndgame, ported from the DeusEx MissionScript subclasses (FirstFrame, Timer
//! once a second, PreTravel). Each runs against a `Ctx`: a snapshot of the map's pawns, bodies and decorations plus
//! the flag store, the movers and request queues for everything a script changes (pawns through ai::world_req /
//! triggers orders, other actors through scriptworld, the player through `PlayerReq`).
//!
//! Faithful to the scripts' quirks: counts are of actors that exist (pawns out of the world count, bodies don't),
//! `foreach AllActors(class'X')` takes subclasses, carcasses are found by their CarcassType class and Tag.

use crate::ai::{Sel, WorldReq};
use crate::scriptworld::{ActorReq, ActorSel};
use crate::triggers::PawnOrder;
use glam::DVec3;
use imsim_core::flags::{FlagBase, FlagValue};
use bevy::prelude::*;
use std::sync::Mutex;
use unreal_pkg::props::{find, Property, Value};

/// A pawn that exists (alive, maybe out of the world).
#[derive(Debug, Clone)]
pub struct PawnInfo {
    pub actor: usize,
    pub class: String,
    pub tag: String,
    pub bind: String,
    pub in_world: bool,
    /// IsInState('Standing').
    pub standing: bool,
    pub pos: DVec3,
    pub emp_hp: f32,
    pub robot: bool,
}

/// A body on the floor (DeusExCarcass).
#[derive(Debug, Clone)]
pub struct CarcInfo {
    pub actor: usize,
    pub class: String,
    pub tag: String,
    pub killer: String,
    pub unconscious: bool,
}

/// Script-local variables (Mission06 fireTime, Mission15 jockTimer / pageTimer / spawnData, MissionEndgame).
#[derive(Default)]
pub struct Vars {
    pub fire_time: f32,
    pub jock_timer: f64,
    pub page_timer: f64,
    /// Mission15 spawnData lastKilledTime (-1 = alive or respawned).
    pub last_killed: [f64; 12],
    pub endgame_timer: f32,
    pub quote_printed: bool,
    pub inited: bool,
}

pub struct Ctx<'a> {
    pub lvl: &'a dx_level::Level,
    pub fl: &'a mut FlagBase,
    pub map: String,
    pub mission: i32,
    pub pawns: Vec<PawnInfo>,
    pub carcs: Vec<CarcInfo>,
    /// Present non-pawn mesh actors: (actor, class, tag).
    pub decos: Vec<(usize, String, String, DVec3)>,
    pub movers: &'a mut crate::movers::Movers,
    pub player_dead: bool,
    pub player_pos: DVec3,
    /// Level.TimeSeconds.
    pub now: f64,
    pub vars: &'a mut Vars,
    pub seed: &'a mut u64,
}

/// Things scripts do to the player and the screen.
#[derive(Debug, Clone)]
pub enum PlayerReq {
    /// ShakeView(time, roll, vert).
    Shake(f32, f32, f32),
    /// Velocity += v.
    Push(DVec3),
    /// Mission05: every body part at least this.
    HealMin(f32),
    /// Mission05: drop the displayable inventory (not the key ring) one item per spot.
    DropInventory(Vec<DVec3>),
    /// Mission00 RemoveAllInventory.
    RemoveAll,
    /// Mission04: caught while dying - alive again (the lab's FirstFrame heals).
    Captured,
    /// MissionEndgame: the quote, shown centred for `secs`.
    Quote(Vec<String>, f32),
    /// MissionEndgame.FinishCinematic: ShowCredits.
    Credits,
    /// SetInstantSoundVolume (None = back to normal).
    SoundVolume(Option<f32>),
}

static PLAYER: Mutex<Vec<PlayerReq>> = Mutex::new(Vec::new());

pub fn player_req(r: PlayerReq) {
    if let Ok(mut q) = PLAYER.lock() {
        q.push(r);
    }
}


/// Effect-only set-piece pieces (no damage).
#[derive(Debug, Clone)]
pub enum Fx {
    /// Spawn(class'ExplosionSmall' / Medium / Large) with animSpeed + extra.
    Explosion(DVec3, &'static str),
    /// MetalFragment: CalcVelocity(speed, spread), DrawScale, Skin, bSmoking.
    Fragment { pos: DVec3, speed: f64, spread: f64, scale: f32, skin: Option<String>, smoking: bool },
    /// SphereEffect of this size.
    Sphere(DVec3, f32),
    /// Mission15 UC spawn flash: ExplosionLight, EllipseEffect, SphereEffect size 4 and four 1 s electricity arcs.
    SpawnFlash(DVec3),
}

static FX: Mutex<Vec<Fx>> = Mutex::new(Vec::new());


/// Rockets the helicopter fires (Mission06 FireMissilesAt): (from, direction, tracking target).
static ROCKETS: Mutex<Vec<(DVec3, DVec3, Option<DVec3>)>> = Mutex::new(Vec::new());

/// Pawns a script spawns (Mission15 UC): (class, tag, where, yaw, orders tag).
static SPAWNS: Mutex<Vec<(String, String, DVec3, i32, String)>> = Mutex::new(Vec::new());

/// Mission09: Player.HeadRegion.Zone.AmbientSound = Klaxon (volume and radius 255).
static ZONE_AMBIENT: Mutex<Option<String>> = Mutex::new(None);

/// A new map: drop what the old one's scripts queued.
pub fn reset_level() {
    if let Ok(mut z) = ZONE_AMBIENT.lock() {
        *z = None;
    }
    for q in [&PLAYER] {
        if let Ok(mut q) = q.lock() {
            q.clear();
        }
    }
    if let Ok(mut q) = FX.lock() {
        q.clear();
    }
    if let Ok(mut q) = ROCKETS.lock() {
        q.clear();
    }
    if let Ok(mut q) = SPAWNS.lock() {
        q.clear();
    }
}

fn tag_of(p: &[unreal_pkg::props::Property]) -> String {
    match find(p, "Tag") {
        Some(Value::Name(t)) => t.clone(),
        _ => String::new(),
    }
}

impl Ctx<'_> {
    fn b(&self, n: &str) -> bool {
        self.fl.get_bool(n)
    }
    /// flags.SetBool(name, v,, expiration).
    fn set(&mut self, n: &str, v: bool, exp: i32) {
        self.fl.set(n, FlagValue::Bool(v), exp);
    }
    /// flags.SetBool(name, v) - the default expiration.
    fn setd(&mut self, n: &str, v: bool) {
        if v {
            self.fl.set_bool(n, true);
        } else {
            self.fl.set_bool(n, false);
        }
    }
    fn rand(&mut self) -> f64 {
        *self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
    fn vrand(&mut self) -> DVec3 {
        loop {
            let v = DVec3::new(self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0);
            if v.length_squared() <= 1.0 && v.length_squared() > 1e-4 {
                return v.normalize();
            }
        }
    }
    /// foreach AllActors(class, P, tag) count++ over pawns.
    fn count(&self, class: &str, tag: Option<&str>) -> usize {
        self.pawns
            .iter()
            .filter(|p| crate::classtree::is_a(&p.class, class) && tag.is_none_or(|t| p.tag.eq_ignore_ascii_case(t)))
            .count()
    }
    fn pawns_of(&self, class: &str, tag: Option<&str>) -> Vec<PawnInfo> {
        self.pawns
            .iter()
            .filter(|p| crate::classtree::is_a(&p.class, class) && tag.is_none_or(|t| p.tag.eq_ignore_ascii_case(t)))
            .cloned()
            .collect()
    }
    fn carcs_of(&self, class: &str, tag: Option<&str>) -> Vec<CarcInfo> {
        self.carcs
            .iter()
            .filter(|c| crate::classtree::is_a(&c.class, class) && tag.is_none_or(|t| c.tag.eq_ignore_ascii_case(t)))
            .cloned()
            .collect()
    }
    fn datalink(&mut self, name: &str) {
        println!("MISSION datalink {name}");
        crate::consys::start_datalink(self.fl, name, None);
    }
    fn goal(&mut self, name: &str) {
        if let Some(cs) = crate::consys::lock().as_mut() {
            cs.goal_completed(name);
        }
    }
    fn conversation(&mut self, con: &str, bind: &str) {
        println!("MISSION conversation {con} with {bind}");
        crate::consys::start_named(self.fl, con, bind);
    }
    fn enter(&self, sel: Sel) {
        crate::ai::world_req(WorldReq::Enter(sel));
    }
    fn leave(&self, sel: Sel) {
        crate::ai::world_req(WorldReq::Leave(sel));
    }
    fn destroy(&self, sel: Sel) {
        crate::ai::world_req(WorldReq::Destroy(sel));
    }
    fn orders(&self, sel: Sel, orders: &str, tag: &str) {
        crate::triggers::push_order(PawnOrder::Script { sel, orders: orders.into(), orders_tag: tag.into() });
    }
    /// foreach AllActors(class'Actor', A, tag) A.Trigger(Self, Player).
    fn trigger(&self, tag: &str) {
        if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
            q.push((tag.to_string(), true));
        }
    }
    fn untrigger(&self, tag: &str) {
        if let Ok(mut q) = crate::movers::EVENTS_OUT.lock() {
            q.push((tag.to_string(), false));
        }
    }
    fn hide(&self, sel: ActorSel, hidden: bool) {
        crate::scriptworld::push(ActorReq::Hide(sel, hidden));
    }
    fn destroy_actors(&self, sel: ActorSel) {
        crate::scriptworld::push(ActorReq::Destroy(sel));
    }
    fn vehicle(&self, sel: ActorSel, enter: bool) {
        crate::scriptworld::push(ActorReq::Vehicle(sel, enter));
    }
    /// The first PatrolPoint / SpawnPoint (or any actor) with this Tag: (location, yaw).
    fn point(&self, class: &str, tag: &str) -> Option<(DVec3, i32)> {
        self.lvl
            .actors
            .iter()
            .find(|a| a.class.eq_ignore_ascii_case(class) && tag_of(&a.props).eq_ignore_ascii_case(tag))
            .map(|a| {
                let l = a.location();
                let yaw = match find(&a.props, "Rotation") {
                    Some(Value::Rotator(r)) => r[1],
                    _ => 0,
                };
                (DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64), yaw)
            })
    }
    fn points(&self, class: &str, tag: &str) -> Vec<DVec3> {
        self.lvl
            .actors
            .iter()
            .filter(|a| a.class.eq_ignore_ascii_case(class) && tag_of(&a.props).eq_ignore_ascii_case(tag))
            .map(|a| {
                let l = a.location();
                DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
            })
            .collect()
    }
    /// Mission06 TeleportPawn(pawn, patrolTag, orders).
    fn teleport(&self, sel: Sel, patrol: &str, orders: &str) {
        if let Some((pos, yaw)) = self.point("PatrolPoint", patrol) {
            crate::ai::world_req(WorldReq::TeleportOrders(sel, pos, yaw, orders.into()));
        }
    }
    fn mover_ids(&self, tag: &str) -> Vec<usize> {
        self.lvl.movers.iter().enumerate().filter(|(_, m)| m.tag.eq_ignore_ascii_case(tag)).map(|(i, _)| i).collect()
    }
    fn unlock(&mut self, tag: &str) {
        for i in self.mover_ids(tag) {
            self.movers.rt[i].locked = false;
            self.movers.rt[i].lock_strength = 0.0;
        }
    }
    fn breakable(&mut self, tag: &str, on: bool) {
        for i in self.mover_ids(tag) {
            self.movers.rt[i].breakable = Some(on);
        }
    }
    /// Movers with this tag not yet destroyed.
    fn movers_left(&self, tag: &str) -> usize {
        self.mover_ids(tag).into_iter().filter(|&i| !self.movers.rt[i].destroyed).count()
    }
    fn blow_up(&self, tag: &str) {
        for i in self.mover_ids(tag) {
            crate::movers::damage_mover(i, 100000.0, "Exploded");
        }
    }
    fn deco_exists(&self, class: &str, tag: Option<&str>) -> usize {
        self.decos
            .iter()
            .filter(|(_, c, t, _)| crate::classtree::is_a(c, class) && tag.is_none_or(|tg| t.eq_ignore_ascii_case(tg)))
            .count()
    }
    fn send_player(&self, map: &str) {
        println!("MISSION SendPlayer {map}");
        crate::loader::request(map, crate::loader::Goal::Travel(None));
    }
    fn shake(&self, time: f32, roll: f32, vert: f32) {
        player_req(PlayerReq::Shake(time, roll, vert));
    }
    fn sound(&self, s: &str) {
        crate::sound::play_player(&format!("DeusExSounds.Generic.{s}"), 1.0, 0.0);
    }
    fn fx(&self, f: Fx) {
        if let Ok(mut q) = FX.lock() {
            q.push(f);
        }
    }
    /// Trace(HitLocation, HitNormal, loc + 1024 up, loc): where a ceiling piece falls from.
    fn ceiling(&self, loc: DVec3) -> (DVec3, DVec3) {
        let end = loc + DVec3::new(0.0, 0.0, 1024.0);
        let h = self.lvl.collision.trace(loc, end, DVec3::ZERO);
        if h.fraction >= 1.0 {
            (end, DVec3::ZERO)
        } else {
            (loc + (end - loc) * h.fraction, h.normal)
        }
    }
    /// MetalFragment with CalcVelocity(vect(speed,0,0), spread), DrawScale 0.5 + 2 FRand, smoking by chance.
    fn falling_frag(&mut self, at: DVec3, smoke_chance: f64) {
        let scale = (0.5 + 2.0 * self.rand()) as f32;
        let smoking = self.rand() < smoke_chance;
        self.fx(Fx::Fragment { pos: at, speed: 20000.0, spread: 256.0, scale, skin: None, smoking });
    }
    /// A deco's live location (path riders move).
    fn deco_pos(&self, class: &str, tag: &str) -> Option<(DVec3, usize)> {
        self.decos
            .iter()
            .find(|(_, c, t, _)| crate::classtree::is_a(c, class) && t.eq_ignore_ascii_case(tag))
            .map(|d| (d.3, d.0))
    }
    /// The DataLink "Tagged One / Two / Three" checks of Mission03 (the ambrosia barrels).
    fn tagged(&mut self, checked: &str, own: &str, others: [&str; 2]) {
        if !self.b(checked) && self.b(own) {
            let mut count = 1;
            for o in others {
                if self.b(o) {
                    count += 1;
                }
            }
            match count {
                1 => self.datalink("DL_TaggedOne"),
                2 => self.datalink("DL_TaggedTwo"),
                _ => self.datalink("DL_TaggedThree"),
            }
            self.set(checked, true, 4);
        }
    }
}

// =================================================================================================== FirstFrame

pub fn first_frame(c: &mut Ctx) {
    let map = c.map.clone();
    match c.mission {
        2 => m02_first(c, &map),
        3 => m03_first(c, &map),
        4 => m04_first(c, &map),
        5 => m05_first(c, &map),
        6 => m06_first(c, &map),
        8 => m08_first(c, &map),
        9 => m09_first(c, &map),
        10 => m10_first(c, &map),
        12 => m12_first(c, &map),
        14 => m14_first(c, &map),
        15 => m15_first(c, &map),
        99 => endgame_first(c, &map),
        _ => {}
    }
}

pub fn timer(c: &mut Ctx) {
    let map = c.map.clone();
    match c.mission {
        2 => m02_timer(c, &map),
        3 => m03_timer(c, &map),
        4 => m04_timer(c, &map),
        5 => m05_timer(c, &map),
        6 => m06_timer(c, &map),
        8 => m08_timer(c, &map),
        9 => m09_timer(c, &map),
        10 => m10_timer(c, &map),
        11 => m11_timer(c, &map),
        12 => m12_timer(c, &map),
        14 => m14_timer(c, &map),
        15 => m15_timer(c, &map),
        99 => endgame_timer(c),
        _ => {}
    }
}

static CURRENT: Mutex<(i32, String)> = Mutex::new((0, String::new()));

/// The map the mission script runs on (set by mission::mission_system).
pub fn set_current(mission: i32, map: &str) {
    if let Ok(mut c) = CURRENT.lock() {
        *c = (mission, map.to_string());
    }
}

pub fn pre_travel_current(fl: &mut FlagBase, lvl: &dx_level::Level) {
    let (m, map) = CURRENT.lock().map(|c| c.clone()).unwrap_or_default();
    pre_travel(fl, m, &map);
    if m == 8 && map == "08_NYC_STREET" {
        // Mission08.PreTravel: make sure that damn helicopter is gone (kept gone if the player comes back).
        for (i, a) in lvl.actors.iter().enumerate() {
            if a.class == "BlackHelicopter" && tag_of(&a.props).eq_ignore_ascii_case("EntranceCopter") {
                crate::scriptworld::mark_hidden(i, true);
            }
        }
    }
}

/// PreTravel: flags set as the player leaves the map.
pub fn pre_travel(fl: &mut FlagBase, mission: i32, map: &str) {
    match (mission, map) {
        (2, "02_NYC_BATTERYPARK") => {
            if !fl.get_bool("MS_MapLeftEarly") && (!fl.get_bool("AmbrosiaTagged") || !fl.get_bool("SubTerroristsDead")) {
                fl.set("MS_MapLeftEarly", FlagValue::Bool(true), 3);
            }
        }
        (2, "02_NYC_UNDERGROUND") => {
            if fl.get_bool("MS_FordFollowing") && !fl.get_bool("FordSchick_Dead") {
                fl.set("FordSchickRescued", FlagValue::Bool(true), 9);
            }
        }
        (10, "10_PARIS_CLUB") => {
            if fl.get_bool("MeetNicolette_Played") && !fl.get_bool("NicoletteLeftClub") {
                fl.set("NicoletteLeftClub", FlagValue::Bool(true), 11);
            }
        }
        (99, _) => player_req(PlayerReq::SoundVolume(None)),
        _ => {}
    }
}

// =================================================================================================== Mission02

fn m02_first(c: &mut Ctx, map: &str) {
    match map {
        "02_NYC_STREET" => {
            c.set("M02StreetLoaded", true, 3);
            if !c.b("MS_GuardsWandering") && c.b("WarehouseDistrictEntered") {
                if !c.b("StreetOpened") || !c.b("ClinicCleared") {
                    c.orders(Sel::isa_tag("ScriptedPawn", "ClinicGuards"), "Wandering", "");
                    c.orders(Sel::isa_tag("ScriptedPawn", "HotelGuards"), "Wandering", "");
                }
                c.set("MS_GuardsWandering", true, 3);
            }
            if !c.b("M02ManderleyDisappointed") && !c.b("BatteryParkComplete") {
                c.set("M02ManderleyDisappointed", true, 3);
            }
            if c.b("MeetSandraRenton_Played") {
                c.destroy(Sel::isa("SandraRenton"));
            }
            if c.b("EscapeSuccessful") {
                c.enter(Sel::isa_tag("BumMale", "hostageMan"));
                c.enter(Sel::isa_tag("BumFemale", "hostageWoman"));
            }
        }
        "02_NYC_BAR" => {
            if c.b("MeetSandraRenton_Played") {
                c.enter(Sel::isa("SandraRenton"));
                if c.count("SandraRenton", None) > 0 {
                    c.set("MS_SandraInBar", true, 3);
                }
            }
        }
        "02_NYC_SMUG" => {
            if c.b("FordSchickRescued") {
                c.enter(Sel::isa("FordSchick"));
                c.setd("SchickThankedPlayer", true);
            }
        }
        _ => {}
    }
}

fn m02_timer(c: &mut Ctx, map: &str) {
    match map {
        "02_NYC_BATTERYPARK" => {
            if !c.b("BatteryParkSlaughter") {
                let count = c.count("Terrorist", Some("ClintonTerrorist"))
                    + c.carcs_of("TerroristCarcass", Some("ClintonTerrorist")).iter().filter(|k| k.unconscious).count();
                if count <= 3 {
                    c.orders(Sel::isa_tag("UNATCOTroop", "ClintonGuard"), "Wandering", "");
                    c.set("BatteryParkSlaughter", true, 6);
                }
            }
            if !c.b("SubTerroristsDead") && c.count("Terrorist", Some("SubTerrorist")) == 0 {
                c.orders(Sel::isa_tag("UNATCOTroop", "SubGuards"), "Wandering", "");
                c.goal("LiberateBatteryParkSubway");
                c.set("SubTerroristsDead", true, 6);
            }
            if !c.b("HostagesKilled") && c.b("SubHostageMale_Dead") && c.b("SubHostageFemale_Dead") {
                c.set("HostagesKilled", true, 3);
            }
            if !c.b("MS_MapLeftEarly") && !c.b("MS_DL_Played") {
                if !c.b("SubTerroristsDead") && c.b("EscapeSuccessful") && !c.b("HostagesKilled") {
                    c.datalink("DL_SubwayComplete3");
                    c.set("MS_DL_Played", true, 3);
                } else if c.b("HostagesKilled") {
                    c.datalink("DL_SubwayComplete2");
                    c.set("MS_DL_Played", true, 3);
                } else if c.b("SubTerroristsDead") || c.b("EscapeSuccessful") {
                    c.datalink("DL_SubwayComplete");
                    c.set("MS_DL_Played", true, 3);
                }
            }
            if !c.b("ShantyTownSecure") && c.count("Terrorist", Some("ShantyTerrorists")) == 0 {
                c.setd("ShantyTownSecure", true);
            }
        }
        "02_NYC_STREET" => {
            if !c.b("ClinicCleared") && c.count("Terrorist", Some("ClinicTerrorist")) == 0 {
                c.orders(Sel::isa_tag("UNATCOTroop", "ClinicGuards"), "Wandering", "");
                c.set("ClinicCleared", true, 6);
            }
            if !c.b("StreetOpened") && c.count("Terrorist", Some("StreetTerrorist")) == 0 {
                c.orders(Sel::isa_tag("UNATCOTroop", "HotelGuards"), "Wandering", "");
                c.set("StreetOpened", true, 6);
                let killed = c
                    .carcs_of("TerroristCarcass", Some("StreetTerrorist"))
                    .iter()
                    .filter(|k| k.killer == "JCDenton" && !k.unconscious)
                    .count();
                if killed >= 3 {
                    c.set("TenderloinSlaughter", true, 6);
                }
            }
            if !c.b("MS_ThugsDead") && c.count("ThugMale2", Some("AlleyThug")) == 0 {
                if c.count("BumMale", Some("AlleyBum")) > 0 {
                    c.set("AlleyBumRescued", true, 3);
                }
                c.set("MS_ThugsDead", true, 3);
            }
            if !c.b("SandrasPimpDone") && c.count("ThugMale", Some("Pimp")) == 0 {
                c.set("SandrasPimpDone", true, 3);
                c.goal("HelpJaneysFriend");
            }
            if !c.b("MS_SandraDead") && c.count("SandraRenton", None) == 0 {
                c.set("MS_SandraDead", true, 3);
                c.goal("HelpJaneysFriend");
            }
            if c.b("OverhearAlleyThug_Played") && !c.b("MS_ThugAttacks") {
                c.trigger("ThugAttacks");
                c.set("MS_ThugAttacks", true, 3);
            }
        }
        "02_NYC_WAREHOUSE" => {
            if !c.b("MS_GeneratorStuff") && !c.b("DL_Pickup_Played") && c.b("GeneratorBlown") {
                c.datalink("DL_Pickup");
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "Helicopter"), true);
                c.enter(Sel::isa("GuntherHermann"));
                c.set("MS_GeneratorStuff", true, 3);
            }
        }
        "02_NYC_FREECLINIC" => {
            if c.b("MS_BumLeaving") && !c.b("MS_BumRemoved") {
                for p in c.pawns_of("BumMale", Some("SickBum1")) {
                    if p.standing {
                        c.destroy(Sel::isa_tag("BumMale", "SickBum1"));
                    }
                }
                c.set("MS_BumRemoved", true, 3);
                c.fl.delete("MS_BumLeaving");
            }
            if c.b("Doctor2_Saved") && !c.b("MS_BumRemoved") && !c.b("MS_BumLeaving") {
                c.orders(Sel::isa_tag("BumMale", "SickBum1"), "GoingTo", "SickBumDestination");
                c.set("MS_BumLeaving", true, 3);
            }
            if c.b("SickBumInterrupted_Played") && !c.b("Doctor2_Saved") && !c.b("MS_BumTurned") {
                if let Some(doc) = c.pawns_of("Doctor", Some("Doctor2")).first() {
                    crate::ai::world_req(WorldReq::Face(Sel::isa_tag("BumMale", "SickBum1"), doc.pos));
                }
                c.set("MS_BumTurned", true, 3);
            }
        }
        "02_NYC_BAR" => {
            if !c.b("M02ViolenceInBar") && !c.carcs.is_empty() {
                c.set("M02ViolenceInBar", true, 4);
            }
        }
        "02_NYC_HOTEL" => {
            if !c.b("M02HostagesRescued") && c.count("Terrorist", Some("SecondFloorTerrorist")) == 0 {
                c.set("M02HostagesRescued", true, 3);
            }
        }
        "02_NYC_UNDERGROUND" => {
            if c.b("FordSchick_Dead") && !c.b("FordSchickRescueDone") {
                c.set("FordSchickRescueDone", true, 9);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission03

fn m03_first(c: &mut Ctx, map: &str) {
    let lebedev = c.b("MeetLebedev_Played") || c.b("JuanLebedev_Dead");
    if !lebedev {
        return;
    }
    match map {
        "03_NYC_AIRFIELDHELIBASE" | "03_NYC_AIRFIELD" | "03_NYC_HANGAR" | "03_NYC_747" => {
            c.destroy(Sel::isa("Terrorist"));
            c.destroy(Sel::isa("SecurityBot3"));
            if map != "03_NYC_747" {
                c.enter(Sel::isa_tag("UNATCOTroop", "UNATCOTroop"));
            }
            if map == "03_NYC_AIRFIELD" {
                c.vehicle(ActorSel::class("BlackHelicopter"), true);
                crate::security::untrigger_all();
                c.enter(Sel::isa("GuntherHermann"));
            }
            if map == "03_NYC_HANGAR" {
                c.destroy(Sel::isa("PaulDenton"));
            }
        }
        _ => {}
    }
}

fn m03_timer(c: &mut Ctx, map: &str) {
    match map {
        "03_NYC_UNATCOHQ" => {
            if !c.b("MS_SimonsWalking") && c.b("SimonsOverheard_Played") {
                c.orders(Sel::isa("WaltonSimons"), "GoingTo", "SimonsInterrogating");
                c.set("MS_SimonsWalking", true, 4);
            }
            if !c.b("SimonsInterrogating") && c.b("MS_SimonsWalking") && c.pawns_of("WaltonSimons", None).iter().any(|p| p.standing) {
                c.set("SimonsInterrogating", true, 4);
            }
            if !c.b("MS_MoverUnlocked") && c.b("UnlockDoor") {
                c.unlock("Manderley_Office");
                c.set("MS_MoverUnlocked", true, 4);
            }
            if c.b("AnnaAtDesk") && !c.b("MS_AnnaDeskHome") {
                crate::ai::world_req(WorldReq::HomeHere(Sel::isa("AnnaNavarre")));
                c.set("MS_AnnaDeskHome", true, 4);
            }
            if c.b("AnnaInOffice") && !c.b("MS_AnnaOfficeHome") {
                crate::ai::world_req(WorldReq::HomeHere(Sel::isa("AnnaNavarre")));
                c.set("MS_AnnaOfficeHome", true, 4);
            }
        }
        "03_NYC_AIRFIELDHELIBASE" => c.tagged("Barrel1Checked", "HelicopterBaseAmbrosia", ["BoatDocksAmbrosia", "747Ambrosia"]),
        "03_NYC_AIRFIELD" => {
            c.tagged("Barrel2Checked", "BoatDocksAmbrosia", ["HelicopterBaseAmbrosia", "747Ambrosia"]);
            if !c.b("MS_GuntherUnhidden") && (c.b("MeetLebedev_Played") || c.b("JuanLebedev_Dead")) {
                c.enter(Sel::isa("GuntherHermann"));
                c.set("MS_GuntherUnhidden", true, 4);
            }
        }
        "03_NYC_747" => {
            if c.b("JuanLebedev_Dead") && !c.b("MS_Anna747Unhidden") {
                c.enter(Sel::isa("AnnaNavarre"));
                c.set("MS_Anna747Unhidden", true, 4);
            }
            c.tagged("Barrel3Checked", "747Ambrosia", ["HelicopterBaseAmbrosia", "BoatDocksAmbrosia"]);
            if !c.b("MS_AnnaUnhidden") && (c.b("MeetLebedev_Played") || c.b("JuanLebedev_Dead")) {
                c.enter(Sel::isa("AnnaNavarre"));
                c.set("MS_AnnaUnhidden", true, 4);
            }
            if !c.b("PlayerKilledLebedev") && !c.b("AnnaKilledLebedev") {
                let carcs = c.carcs_of("JuanLebedevCarcass", None);
                for k in &carcs {
                    if k.killer == "JCDenton" || k.killer.is_empty() {
                        c.goal("AssassinateLebedev");
                        c.set("PlayerKilledLebedev", true, 6);
                    } else if k.killer == "AnnaNavarre" {
                        c.set("AnnaKilledLebedev", true, 6);
                    } else {
                        c.set("JuanLebedev_Dead", true, 0);
                    }
                }
                let juan = c.count("JuanLebedev", None) > 0;
                if carcs.is_empty() && !juan && c.b("JuanLebedev_Dead") {
                    c.set("PlayerKilledLebedev", true, 6);
                }
            }
        }
        "03_NYC_MOLEPEOPLE" => {
            if !c.b("MolePeopleSlaughtered") && c.count("ScriptedPawn", Some("MolePeople")) < 4 {
                c.set("MolePeopleSlaughtered", true, 4);
            }
            if !c.b("MoleTerroristsDefeated") && c.count("Terrorist", Some("MoleTerrorist")) < 3 {
                c.set("MoleTerroristsDefeated", true, 4);
            }
        }
        "03_NYC_BROOKLYNBRIDGESTATION" => {
            if !c.b("JugHeadGangDefeated") && c.count("ThugMale2", Some("ThugMale2")) == 0 {
                c.set("JugHeadGangDefeated", true, 4);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission04

fn m04_first(c: &mut Ctx, map: &str) {
    match map {
        "04_NYC_STREET" => {
            if c.b("TalkedToPaulAfterMessage_Played") {
                c.enter(Sel::isa("UNATCOTroop"));
                c.enter(Sel::isa("SecurityBot2"));
            }
        }
        "04_NYC_FREECLINIC" => {
            if c.b("TalkedToPaulAfterMessage_Played") {
                c.enter(Sel::isa("UNATCOTroop"));
            }
        }
        "04_NYC_HOTEL" => {
            if (c.b("SandraRenton_Dead") || c.b("GilbertRenton_Dead")) && !c.b("JoJoFine_Dead") {
                c.enter(Sel::isa_tag("ScriptedPawn", "JoJoInLobby"));
            }
        }
        _ => {}
    }
}

fn m04_timer(c: &mut Ctx, map: &str) {
    if !c.b("MS_PlayerCaptured") && c.b("TalkedToPaulAfterMessage_Played") && c.player_dead {
        c.set("MS_PlayerCaptured", true, 5);
        c.goal("EscapeToBatteryPark");
        player_req(PlayerReq::Captured);
        c.send_player("05_NYC_UNATCOMJ12Lab");
    }
    match map {
        "04_NYC_HOTEL" => {
            if !c.b("PlayerKilledRenton") {
                let mut count = c.count("SandraRenton", None) + c.count("GilbertRenton", None);
                if c.carcs_of("SandraRentonCarcass", None).iter().any(|k| k.killer == "JCDenton")
                    || c.carcs_of("GilbertRentonCarcass", None).iter().any(|k| k.killer == "JCDenton")
                {
                    count = 0;
                }
                if count < 2 {
                    c.set("PlayerKilledRenton", true, 5);
                    c.trigger("RentonsHatePlayer");
                }
            }
            if !c.b("M04RaidTeleportDone") && c.b("ApartmentEntered") && c.b("NSFSignalSent") {
                c.enter(Sel::isa("UNATCOTroop"));
                c.enter(Sel::isa("MIB"));
                c.leave(Sel::isa("SandraRenton"));
                c.leave(Sel::isa("GilbertRenton"));
                c.leave(Sel::isa("HarleyFilben"));
                if c.count("PaulDenton", None) > 0 {
                    // Lay D Denton: her own version.
                    let con = if c.b("LDDPJCIsFemale") { "FemJCTalkedToPaulAfterMessage" } else { "TalkedToPaulAfterMessage" };
                    c.conversation(con, "PaulDenton");
                }
                c.set("M04RaidTeleportDone", true, 5);
            }
            if !c.b("MS_MIBMortal") && c.b("TalkedToPaulAfterMessage_Played") {
                crate::ai::world_req(WorldReq::Invincible(Sel::isa("MIB"), false));
                c.set("MS_MIBMortal", true, 5);
            }
            let jojo_dead = c.b("JoJoFine_Dead");
            if !c.b("MS_JoJoUnhidden")
                && (c.b("SandraWaitingForJoJoBarks_Played") || c.b("GilbertWaitingForJoJoBarks_Played"))
                && !jojo_dead
            {
                c.enter(Sel::isa_tag("ScriptedPawn", "JoJoUpstairs"));
                c.set("MS_JoJoUnhidden", true, 5);
            }
            if !c.b("MS_JoJoUnhidden")
                && c.b("M03OverhearSquabble_Played")
                && !c.b("JoJoOverheard_Played")
                && c.b("JoJoEntrance")
                && !jojo_dead
            {
                c.enter(Sel::isa_tag("ScriptedPawn", "JoJoUpstairs"));
                c.set("MS_JoJoUnhidden", true, 5);
            }
            if c.b("JoJoOverheard_Played") && !c.b("MS_JoJo1Triggered") {
                if c.b("GaveRentonGun") {
                    c.trigger("GilbertAttacksJoJo");
                } else {
                    c.trigger("JoJoAttacksGilbert");
                }
                c.set("MS_JoJo1Triggered", true, 5);
            }
            if c.b("JoJoAndSandraOverheard_Played") && !c.b("MS_JoJo2Triggered") {
                c.trigger("SandraLeaves");
                c.set("MS_JoJo2Triggered", true, 5);
            }
            if c.b("JoJoAndGilbertOverheard_Played") && !c.b("MS_JoJo3Triggered") {
                c.trigger("JoJoAttacksGilbert");
                c.set("MS_JoJo3Triggered", true, 5);
            }
        }
        "04_NYC_NSFHQ" => {
            for (i, yaw) in [(1, 49152), (2, 0), (3, 16384)] {
                let rotated = format!("MS_Dish{i}Rotated");
                if !c.b(&rotated) && c.b(&format!("Dish{i}InPosition")) {
                    crate::scriptworld::push(ActorReq::Yaw(ActorSel::class_tag("SatelliteDish", &format!("Dish{i}")), yaw));
                    c.set(&rotated, true, 5);
                }
            }
            if !c.b("CanSendSignal") && c.b("Dish1InPosition") && c.b("Dish2InPosition") && c.b("Dish3InPosition") {
                c.set("CanSendSignal", true, 5);
            }
            if !c.b("MostWarehouseTroopsDead") && c.count("UNATCOTroop", None) <= 2 {
                c.setd("MostWarehouseTroopsDead", true);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission05

fn m05_first(c: &mut Ctx, map: &str) {
    match map {
        "05_NYC_UNATCOMJ12LAB" => {
            c.goal("EscapeToBatteryPark");
            if !c.b("PaulDenton_Dead") {
                c.destroy_actors(ActorSel::class("PaulDentonCarcass"));
            }
            if c.b("M05PaulDentonDone") || c.b("PlayerBailedOutWindow") {
                c.destroy(Sel::isa("PaulDenton"));
            }
            if c.b("MeetMiguel_Played") && !c.b("MiguelFollowing") {
                c.destroy(Sel::isa_bind("Terrorist", "Miguel"));
            }
            if !c.b("MS_InventoryRemoved") {
                player_req(PlayerReq::HealMin(50.0));
                let spots = c.points("SpawnPoint", "player_inv");
                player_req(PlayerReq::DropInventory(spots));
                c.set("MS_InventoryRemoved", true, 6);
            }
        }
        "05_NYC_UNATCOHQ" => {
            if c.b("MiguelFollowing") {
                c.enter(Sel::isa_bind("Terrorist", "Miguel"));
            }
            crate::ai::world_req(WorldReq::MinHealth(Sel::isa("AnnaNavarre"), 0.0));
        }
        "05_NYC_UNATCOISLAND" => {
            if c.b("MiguelFollowing") {
                c.enter(Sel::isa_bind("Terrorist", "Miguel"));
            }
        }
        _ => {}
    }
}

fn m05_timer(c: &mut Ctx, map: &str) {
    match map {
        "05_NYC_UNATCOHQ" => {
            if c.b("CarterUnlock") && !c.b("MS_DoorUnlocked") {
                c.unlock("supplydoor");
                c.set("MS_DoorUnlocked", true, 6);
            }
            if c.b("annadies") && !c.b("MS_AnnaKilled") {
                crate::ai::world_req(WorldReq::KillShot(Sel::isa("AnnaNavarre")));
                c.set("MS_AnnaKilled", true, 6);
            }
            if c.b("M05AnnaAtExit_Played") && !c.b("MS_AnnaAttacking") {
                c.orders(Sel::isa("AnnaNavarre"), "Attacking", "");
                c.set("MS_AnnaAttacking", true, 6);
            }
            if c.b("simonsappears") && !c.b("MS_SimonsUnhidden") {
                c.enter(Sel::isa("WaltonSimons"));
                c.set("MS_SimonsUnhidden", true, 6);
            }
            if (c.b("M05MeetManderley_Played") || c.b("M05SimonsAlone_Played")) && !c.b("MS_SimonsHidden") {
                c.leave(Sel::isa("WaltonSimons"));
                c.set("MS_SimonsHidden", true, 6);
            }
            if c.b("KnowsAnnasKillphrase1") && c.b("KnowsAnnasKillphrase2") && !c.b("MS_KillphraseGoalCleared") {
                c.goal("FindAnnasKillphrase");
                c.set("MS_KillphraseGoalCleared", true, 6);
            }
            if c.b("AnnaNavarre_Dead") && !c.b("MS_EliminateAnna") {
                c.goal("EliminateAnna");
                c.set("MS_EliminateAnna", true, 6);
            }
        }
        "05_NYC_UNATCOMJ12LAB" => {
            if !c.b("MS_DL_Played") && c.b("PaulInMedLab_Played") {
                c.datalink("DL_Paul");
                c.set("MS_DL_Played", true, 6);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission06

fn m06_first(c: &mut Ctx, map: &str) {
    match map {
        "06_HONGKONG_VERSALIFE" => {
            if c.b("M07Briefing_Played") {
                c.enter(Sel::isa("MJ12Commando"));
            }
        }
        "06_HONGKONG_WANCHAI_CANAL" => {
            if !c.b("Supervisor01_Dead") && c.b("HaveROM") {
                c.hide(ActorSel::class_tag("DeusExCarcass", "John_Smith_Body"), false);
            }
        }
        "06_HONGKONG_MJ12LAB" => {
            if c.b("M07Briefing_Played") {
                c.enter(Sel::isa("MJ12Commando"));
                c.enter(Sel::isa("SpiderBot"));
                c.destroy_actors(ActorSel::class_tag("Keypad3", "DummyKeypad_02"));
                c.hide(ActorSel::class_tag("Keypad3", "RealKeypad_02"), false);
            }
        }
        "06_HONGKONG_TONGBASE" => {
            if c.b("Versalife_Done") {
                c.enter(Sel::isa("PaulDenton"));
                c.enter(Sel::isa_tag("TriadRedArrow", "TriadRedArrow"));
            }
            if c.b("JaimeRecruited") && c.b("Versalife_Done") {
                c.enter(Sel::isa("JaimeReyes"));
            }
            if c.b("JacobsonRecruited") {
                c.enter(Sel::isa("AlexJacobson"));
            }
        }
        "06_HONGKONG_WANCHAI_UNDERWORLD" => {
            if c.b("QuickLetPlayerIn") {
                c.enter(Sel::isa_tag("MJ12Commando", "MJ12Commando"));
            }
            if c.b("TriadCeremony_Played") {
                c.set("DragonHeadsInLuckyMoney", true, 8);
                c.destroy_actors(ActorSel::class("DeusExCarcass"));
            }
            if c.b("DragonHeadsInLuckyMoney") && !c.b("MS_ChenTeleported") {
                c.enter(Sel::isa("GordonQuick"));
                c.teleport(Sel::isa("MaxChen"), "ChenAtBar", "Standing");
                c.set("MS_ChenTeleported", true, 8);
            }
        }
        "06_HONGKONG_WANCHAI_GARAGE" => {
            if c.b("M07Briefing_Played") {
                c.destroy(Sel::isa_tag("HKMilitary", "RumbleCops"));
                for k in ["HKMilitaryCarcass", "TriadLumPathCarcass", "TriadLumPath2Carcass"] {
                    c.destroy_actors(ActorSel::class(k));
                }
                c.destroy_actors(ActorSel::class_tag("Van", "Van01"));
                c.destroy_actors(ActorSel::class("CarWrecked"));
                c.destroy_actors(ActorSel::class("RoadBlock"));
            }
        }
        "06_HONGKONG_WANCHAI_MARKET" => {
            if c.b("Have_ROM") && c.b("MeetTracerTong_Played") && !c.b("TriadCeremony_Played") {
                c.teleport(Sel::isa("GordonQuick"), "QuickInTemple", "Standing");
                c.teleport(Sel::isa("MaxChen"), "ChenInTemple", "Standing");
                c.enter(Sel::tag("MarketMonk01"));
                c.destroy(Sel::tag("MarketKid"));
                c.set("CeremonyReadyToBegin", true, 8);
            }
            if c.b("MarketShopperOverheard_Played") {
                c.destroy(Sel::tag("MarketShopperFlowers"));
            }
            if c.b("CatererConvo_Played") {
                c.teleport(Sel::tag("TeaHouseRedArrow"), "TalkToCaterer", "Wandering");
            } else if c.b("TeaHouseDrama_Played") {
                c.teleport(Sel::tag("TeaHouseRedArrow"), "TalkToCaterer", "Standing");
                c.set("ReadyForCaterer", true, 8);
            }
            if c.b("TeaHouseDrama_Played") {
                c.destroy(Sel::tag("TeaHouseCustomer"));
                c.destroy(Sel::tag("TeaHouseWoman"));
            }
            if c.b("KidGetsMoney_Played") {
                c.teleport(Sel::tag("MarketKid"), "KidAtNewsStand", "Wandering");
            } else if c.b("KidSetsFire_Played") {
                c.teleport(Sel::tag("MarketKid"), "KidAtNewsStand", "Standing");
                c.set("MarketKidReadyForFifth", true, 8);
            } else if c.b("M06_Fire_Set") {
                c.teleport(Sel::tag("MarketKid"), "KidAtLumPath", "Standing");
                c.set("MarketKidReadyForFourth", true, 8);
            } else if c.b("KidAsksForHelp_Played") {
                c.teleport(Sel::tag("MarketKid"), "KidSettingFire", "Standing");
                c.set("MarketKidReadyForThird", true, 8);
            } else if c.b("KidAsksForWork_Played") {
                c.teleport(Sel::tag("MarketKid"), "KidAtLumPath", "Standing");
                c.set("MarketKidReadyForSecond", true, 8);
            }
            if c.b("M08Briefing_Played") {
                c.vehicle(ActorSel::class("BlackHelicopter"), true);
            }
            if c.b("M07Briefing_Played") {
                c.hide(ActorSel::class_tag("BookOpen", "TempleBook"), false);
            }
            if c.count("ScriptedPawn", Some("MarketGoth")) > 0 {
                let r = c.rand();
                let (spot, f) = if r < 0.33 {
                    ("GothAtFlower", [true, false, false])
                } else if r < 0.66 {
                    ("GothAtButcher", [false, true, false])
                } else {
                    ("GothAtVase", [false, false, true])
                };
                c.teleport(Sel::tag("MarketGoth"), spot, "Standing");
                c.set("GothAtFlower", f[0], 8);
                c.set("GothAtButcher", f[1], 8);
                c.set("GothAtVase", f[2], 8);
            }
        }
        _ => {}
    }
}

fn m06_timer(c: &mut Ctx, map: &str) {
    match map {
        "06_HONGKONG_WANCHAI_STREET" => {
            if c.b("WaltonAppears") && !c.b("MS_WaltonUnhidden") {
                c.enter(Sel::isa("WaltonSimons"));
                c.set("MS_WaltonUnhidden", true, 8);
            }
            if c.b("M06WaltonHolo_Played") && !c.b("MS_WaltonHidden") {
                c.leave(Sel::isa("WaltonSimons"));
                c.set("MS_WaltonHidden", true, 8);
            }
        }
        "06_HONGKONG_WANCHAI_UNDERWORLD" => {
            if c.b("MaxChenConvinced") && !c.b("MS_CommandosUnhidden") {
                c.enter(Sel::isa_tag("MJ12Commando", "RaidingCommando"));
                c.set("MS_CommandosUnhidden", true, 8);
            }
            if c.b("Raid_Underway") && c.count("MJ12Commando", Some("RaidingCommando")) == 0 {
                c.set("Raid_Underway", false, 8);
                c.set("M06AmbushDone", true, 8);
            }
            if c.b("ReadyForMercedes2") && !c.b("MS_HomeBaseSet") {
                if let Some((pos, yaw)) = c.point("PatrolPoint", "PartyGirlInClub02") {
                    crate::ai::world_req(WorldReq::HomeAt(Sel::isa_tag("LowerClassFemale", "ClubTessa"), pos, yaw));
                }
                c.set("MS_HomeBaseSet", true, 8);
            }
        }
        "06_HONGKONG_WANCHAI_GARAGE" => {
            for (cls, tag, flag) in [
                ("HKMilitary", "RumbleCops", "RumbleCops_Lost"),
                ("TriadRedArrow", "RumbleRedArrow", "RumbleRedArrow_Lost"),
                ("TriadLumPath", "RumbleLumPath", "RumbleLumPath_Lost"),
            ] {
                if !c.b(flag) && c.count(cls, Some(tag)) == 0 {
                    c.set(flag, true, 8);
                }
            }
        }
        "06_HONGKONG_WANCHAI_MARKET" => {
            if c.b("TeaHouseDrama_Played") && !c.b("MS_RatsReleased") {
                crate::spawn::trigger_generators("RatGenerator");
                c.set("MS_RatsReleased", true, 8);
            }
            if c.b("CatererConvo_Played") && !c.b("MS_ConvoTrigger1") {
                c.trigger("TeaHouseRedArrowWanders");
                c.set("MS_ConvoTrigger1", true, 8);
            }
            if c.b("TeaHouseDrama_Played") && !c.b("MS_ConvoTrigger2") {
                c.trigger("TeaHouseRedArrowPatrol");
                c.trigger("MarketWaiterWanders");
                c.set("MS_ConvoTrigger2", true, 8);
            }
            if c.b("FlowerShopperOverheard_Played") && !c.b("MS_ConvoTrigger3") {
                c.trigger("MarketShopperFlowersWanders");
                c.set("MS_ConvoTrigger3", true, 8);
            }
            if c.b("KidGetsMoney_Played") && !c.b("MS_ConvoTrigger4") {
                c.trigger("KidWanders");
                c.set("MS_ConvoTrigger4", true, 8);
            } else if c.b("KidSetsFire_Played") && !c.b("MS_ConvoTrigger5") {
                c.trigger("KidGoesToNewsStand");
                c.set("MS_ConvoTrigger5", true, 8);
            } else if c.b("KidStealsSomething_Played") && !c.b("MS_ConvoTrigger6") {
                c.trigger("KidGoesToLumPath");
                c.set("MS_ConvoTrigger6", true, 8);
            } else if c.b("KidAsksForHelp_Played") && !c.b("MS_ConvoTrigger7") {
                c.trigger("KidGoesToNewsStand");
                c.set("MS_ConvoTrigger7", true, 8);
            } else if c.b("KidAsksForWork_Played") && !c.b("MS_ConvoTrigger8") {
                c.trigger("KidGoesToLumPath");
                c.set("MS_ConvoTrigger8", true, 8);
            }
            if c.b("QuickLetPlayerIn") && !c.b("MS_KeypadsMoved") {
                c.destroy_actors(ActorSel::class_tag("Keypad1", "DummyKeypad01"));
                c.hide(ActorSel::class_tag("Keypad1", "GateKeypad"), false);
                c.hide(ActorSel::class_tag("Keypad1", "BasementKeypad"), false);
                c.set("MS_KeypadsMoved", true, 8);
            }
        }
        "06_HONGKONG_HELIBASE" => {
            if c.b("helicopter_armed") && !c.b("MS_JockInfoLink") {
                c.datalink("DL_Jock_02");
                c.set("MS_JockInfoLink", true, 8);
            }
            if c.b("MS_DoorsBlown") && !c.b("MS_RubbleBreakable") {
                c.breakable("DoorWreckage", true);
                c.set("MS_RubbleBreakable", true, 8);
            }
            if c.b("MS_HelicopterFired") && !c.b("MS_DoorsBlown") {
                c.vars.fire_time += 1.0;
                if c.movers_left("Blast_doors") == 0 {
                    c.datalink("DL_Jock_03");
                    c.trigger("AlarmUnit");
                    c.trigger("Go");
                    c.set("MS_DoorsBlown", true, 8);
                } else if c.vars.fire_time > 3.0 {
                    fire_missiles_at(c, "Blast_doors");
                    c.vars.fire_time = 0.0;
                }
            }
            if c.b("helicopter_fire") && !c.b("MS_HelicopterFired") {
                c.datalink("DL_Jock_Fired");
                c.breakable("Blast_doors", true);
                c.vars.fire_time = 0.0;
                fire_missiles_at(c, "Blast_doors");
                c.set("MS_HelicopterFired", true, 8);
            }
        }
        "06_HONGKONG_STORAGE" => {
            if c.b("ReadyToDestroyAUC") && !c.b("MS_ReadyAUC") {
                for t in ["Pod01", "Pod02", "Pod03", "Pod04", "AUC"] {
                    c.breakable(t, true);
                }
                c.set("MS_ReadyAUC", true, 8);
            }
        }
        "06_HONGKONG_WANCHAI_CANAL" => {
            if c.b("Overhear_Canal_Thug1_Played") && !c.b("MS_DrugDealersAttacking") {
                c.trigger("CanalDrugDealersAttack");
                c.set("MS_DrugDealersAttacking", true, 8);
            }
        }
        _ => {}
    }
}

/// Mission06.FireMissilesAt: the helicopter tagged 'chopper' fires two tracking LAW rockets at the target.
fn fire_missiles_at(c: &mut Ctx, target: &str) {
    // foreach AllActors(class'Actor', A, targetTag) Target = A: the last one.
    let tgt = c.mover_ids(target).last().map(|&i| c.movers.rt[i].location).or_else(|| {
        c.lvl.actors.iter().rev().find(|a| tag_of(&a.props).eq_ignore_ascii_case(target)).map(|a| {
            let l = a.location();
            DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64)
        })
    });
    let Some((at, ai)) = c.deco_pos("BlackHelicopter", "chopper") else { return };
    let ch = &c.lvl.actors[ai];
    let r = match find(&ch.props, "CollisionRadius") {
        Some(Value::Float(v)) => *v as f64,
        _ => 400.0,
    };
    let (pitch, yaw) = match find(&ch.props, "Rotation") {
        Some(Value::Rotator(r)) => (r[0] as f64 / 65536.0 * std::f64::consts::TAU, r[1] as f64 / 65536.0 * std::f64::consts::TAU),
        _ => (0.0, 0.0),
    };
    let fwd = DVec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin());
    let side = DVec3::new(-yaw.sin(), yaw.cos(), 0.0);
    for i in [-1.0, 1.0] {
        // (i * CollisionRadius * vect(0, 0.15, 0)) >> Rotation.
        let from = at + side * (i * r * 0.15);
        if let Ok(mut q) = ROCKETS.lock() {
            q.push((from, fwd, tgt));
        }
        crate::sound::play("DeusExSounds.Weapons.RocketIgnite", from, 2.0, 2048.0, 1.0);
    }
}

// =================================================================================================== Mission08

fn m08_first(c: &mut Ctx, map: &str) {
    if c.b("SandraWentToCalifornia") {
        c.destroy(Sel::isa("SandraRenton"));
    }
    if map == "08_NYC_SMUG" && c.b("FordSchickRescued") {
        c.enter(Sel::isa("FordSchick"));
    }
}

fn m08_timer(c: &mut Ctx, map: &str) {
    match map {
        "08_NYC_FREECLINIC" => {
            if c.b("JoeGreene_Dead") && !c.b("MS_GreeneGoalSet") {
                c.goal("KillGreene");
                c.set("MS_GreeneGoalSet", true, 9);
            }
        }
        "08_NYC_STREET" => {
            for i in 1..=6 {
                let flag = format!("MS_UnhideTroop{i}");
                if !c.b(&flag) && c.count("RiotCop", Some(&format!("Cop{i}"))) == 0 {
                    c.enter(Sel::isa_tag("UNATCOTroop", &format!("troop{i}")));
                    c.set(&flag, true, 9);
                }
            }
            if !c.b("MS_ThomasUnhidden") && c.b("HarleyFilben_Dead") {
                c.enter(Sel::isa_tag("Janitor", "ThomasDieter"));
                c.set("MS_ThomasUnhidden", true, 9);
            }
            if !c.b("MS_StantonUnhidden") && (c.b("M08MeetHarleyFilben_Played") || c.b("MeetThomasDieter_Played")) {
                c.enter(Sel::isa_tag("StantonDowd", "StantonDowd"));
                c.set("MS_StantonUnhidden", true, 9);
            }
            if !c.b("MS_ShadyGuyUnhidden")
                && c.b("MS_StantonUnhidden")
                && ((c.b("GreenKnowsAboutDowd") && !c.b("JoeGreen_Dead")) || c.b("SheaKnowsAboutDowd"))
            {
                c.enter(Sel::isa_tag("ThugMale", "ShadyGuy"));
                c.set("MS_ShadyGuyUnhidden", true, 9);
            }
            if !c.b("StantonAmbush") {
                let stanton = c.pawns_of("StantonDowd", Some("StantonDowd")).into_iter().last();
                let thug = c.pawns_of("ScriptedPawn", Some("ShadyGuy")).into_iter().last();
                if let (Some(s), Some(t)) = (stanton, thug) {
                    if crate::classtree::is_a(&t.class, "ThugMale") && t.in_world && t.pos.distance(s.pos) <= 128.0 {
                        c.enter(Sel::isa_tag("MJ12Troop", "MJ12AttackForce"));
                        c.set("StantonAmbush", true, 9);
                    }
                }
            }
            if !c.b("StantonAmbush") && c.b("MJ12Converging") {
                c.enter(Sel::isa_tag("MJ12Troop", "MJ12AttackForce"));
                c.set("StantonAmbush", true, 9);
            }
            if c.b("StantonAmbush") && !c.b("StantonAmbushDefeated") && c.count("MJ12Troop", Some("MJ12AttackForce")) == 0 {
                c.set("StantonAmbushDefeated", true, 9);
            }
            if c.b("StantonDowd_Played") && c.b("DL_Exit_Played") && !c.b("MS_Helicopter_Unhidden") {
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "CopterExit"), true);
                c.set("MS_Helicopter_Unhidden", true, 9);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission09

fn m09_first(c: &mut Ctx, map: &str) {
    match map {
        "09_NYC_SHIP" => {
            if c.b("ShipBreech") {
                for t in ["SewerGrate", "FrontDoor"] {
                    for i in c.mover_ids(t) {
                        let info = &c.lvl.movers[i];
                        let rt = &mut c.movers.rt[i];
                        if rt.key_num != 0 {
                            crate::movers::trigger(info, rt);
                        }
                        rt.breakable = Some(false);
                        rt.pickable = false;
                        rt.frobbable = Some(false);
                        rt.locked = true;
                    }
                }
            }
        }
        "09_NYC_DOCKYARD" => {
            if c.b("MS_ShipBreeched") {
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "BlackHelicopter"), true);
            }
        }
        _ => {}
    }
}

fn m09_timer(c: &mut Ctx, map: &str) {
    match map {
        "09_NYC_SHIP" => {
            if !c.b("MS_SimonsAppeared") && c.b("SummonSimons") {
                c.enter(Sel::isa("WaltonSimons"));
                c.set("MS_SimonsAppeared", true, 10);
            }
            if c.b("MS_SimonsAppeared") && c.b("M09SimonsDisappears") {
                c.leave(Sel::isa("WaltonSimons"));
                c.set("M09SimonsDisappears", false, 10);
                c.set("MS_SimonsAppeared", false, 10);
                c.set("SummonSimons", false, 10);
            }
            if c.b("MS_ShipBreeched") {
                ship_explosions(c, false);
            }
        }
        "09_NYC_SHIPBELOW" => {
            if !c.b("MS_ShipBreeched") && c.movers_left("ShipBreech") == 0 {
                if c.b("Bilge") {
                    c.datalink("DL_AllDone");
                    c.set("MS_ShipBreeched", true, 10);
                } else if !c.b("DL_AllPlaced_Played") {
                    c.datalink("DL_AllPlaced");
                }
            }
            if c.b("MS_ShipBreeched") {
                ship_explosions(c, true);
            }
        }
        "09_NYC_GRAVEYARD" => {
            if !c.b("MS_UnhideHelicopter") && c.b("deviceDestroyed") && c.b("M09MeetStantonDowd_Played") {
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "BlackHelicopter"), true);
                c.datalink("DL_ComingIn");
                c.enter(Sel::isa_tag("MJ12Troop", "TroopSupport"));
                c.set("MS_UnhideHelicopter", true, 10);
            }
            if !c.b("MS_TriggerOn") && c.b("M09MeetStantonDowd_Played") {
                crate::triggers::set_collision("TunnelTrigger", true);
                c.enter(Sel::isa_tag("MJ12Troop", "TroopInsertion"));
                c.set("MS_TriggerOn", true, 10);
            }
            if c.b("GreenKnowsAboutDowd") && c.b("suprisePoint") && !c.b("MS_UnhideCommandos") {
                c.enter(Sel::isa_tag("MJ12Commando", "paratroop"));
                c.set("MS_UnhideCommandos", true, 10);
            }
        }
        _ => {}
    }
}

/// Mission09.ShipExplosionEffects: random blasts shake the ship, push the player, swing hanging things, and below
/// decks throw debris; the klaxon sounds in the player's zone.
fn ship_explosions(c: &mut Ctx, fragments: bool) {
    let freq = if fragments { 0.33 } else { 0.1 };
    if c.rand() < freq {
        let size = c.rand() as f32;
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
        c.shake(0.5 + size, 512.0 + 1024.0 * size, 8.0 + 16.0 * size);
        let v = c.vrand();
        player_req(PlayerReq::Push(DVec3::new(300.0, 300.0, 200.0) + v * (500.0 * size as f64)));
        if fragments {
            for _ in 0..(size * 20.0) as i32 {
                let v = c.vrand();
                let mut loc = c.player_pos + v * 256.0;
                loc.z = c.player_pos.z;
                let (hit, _) = c.ceiling(loc);
                c.falling_frag(hit, 0.25);
            }
        }
    }
    // The player's zone gets the alarm as its ambient sound.
    if let Ok(mut z) = ZONE_AMBIENT.lock() {
        *z = Some("Ambient.Ambient.Klaxon".into());
    }
}

// =================================================================================================== Mission10/11

fn m10_first(c: &mut Ctx, map: &str) {
    match map {
        "10_PARIS_METRO" => {
            if c.b("NicoletteLeftClub") {
                c.enter(Sel::isa("NicoletteDuClare"));
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "BlackHelicopter"), true);
            }
            if !c.b("JaimeRecruited") {
                c.enter(Sel::isa("JaimeReyes"));
            }
        }
        "10_PARIS_CLUB" => {
            if c.b("NicoletteLeftClub") {
                c.destroy(Sel::isa("NicoletteDuClare"));
            }
        }
        "10_PARIS_CHATEAU" => c.set("ClubComplete", true, 11),
        _ => {}
    }
}

fn m10_timer(c: &mut Ctx, map: &str) {
    match map {
        "10_PARIS_CATACOMBS" => {
            if !c.b("SewerGreaselsDead") && c.count("Greasel", Some("SewerGreasel")) == 0 {
                c.set("SewerGreaselsDead", true, 11);
            }
            if !c.b("defoequestcomplete") && c.count("ScriptedPawn", Some("mj12metroguards")) == 0 {
                c.set("defoequestcomplete", true, 11);
            }
        }
        "10_PARIS_CATACOMBS_TUNNELS" => {
            if !c.b("SilhouetteRescueComplete") {
                let (mut not_safe, mut not_dead) = (0, 0);
                for p in &c.pawns {
                    if p.bind.eq_ignore_ascii_case("hostage") {
                        not_dead += 1;
                        if !c.fl.get_bool("CataMaleSafe") {
                            not_safe += 1;
                        }
                    } else if p.bind.eq_ignore_ascii_case("hostage_female") {
                        not_dead += 1;
                        if !c.fl.get_bool("CataFemaleSafe") {
                            not_safe += 1;
                        }
                    }
                }
                if not_safe == 0 || not_dead == 0 {
                    c.set("SilhouetteRescueComplete", true, 11);
                    c.goal("EscortHostages");
                    if not_dead == 0 {
                        c.set("SilhouetteHostagesDead", true, 11);
                    } else if not_dead < 2 {
                        c.set("SilhouetteHostagesSomeRescued", true, 11);
                    } else {
                        c.set("SilhouetteHostagesAllRescued", true, 11);
                    }
                }
            }
        }
        "10_PARIS_METRO" => {
            if !c.b("MS_GuntherUnhidden") && c.b("JockReady_Played") {
                c.enter(Sel::isa("GuntherHermann"));
                c.set("MS_GuntherUnhidden", true, 11);
            }
            if c.b("AlleyCopSeesPlayer_Played") && !c.b("MS_CopBarked") {
                c.trigger("AlleyCopAttacks");
                c.set("MS_CopBarked", true, 11);
            }
        }
        "10_PARIS_CHATEAU" => {
            if !c.b("MS_CommandosUnhidden") && c.b("everettsignal") {
                c.enter(Sel::isa("MJ12Commando"));
                c.set("MS_CommandosUnhidden", true, 11);
            }
        }
        _ => {}
    }
}

fn m11_timer(c: &mut Ctx, map: &str) {
    match map {
        "11_PARIS_UNDERGROUND" => {
            if c.b("templar_upload") && !c.b("MS_TobyUnhidden") {
                c.enter(Sel::isa("TobyAtanwe"));
                c.set("MS_TobyUnhidden", true, 12);
            }
            if c.b("MeetTobyAtanwe_Played") && !c.b("MS_PlayerTeleported") {
                c.set("MS_PlayerTeleported", true, 12);
                c.send_player("11_PARIS_EVERETT");
            }
        }
        "11_PARIS_EVERETT" => {
            if c.b("MeetEverett_Played") && !c.b("MS_ChopperUnhidden") {
                c.vehicle(ActorSel::class("BlackHelicopter"), true);
                c.set("MS_ChopperUnhidden", true, 12);
            }
            if c.b("AtanweAtEveretts_Played") && !c.b("MS_AlexUnhidden") {
                c.enter(Sel::isa("AlexJacobson"));
                c.set("MS_AlexUnhidden", true, 12);
            }
            if c.b("Ray_Dead") && !c.b("MS_RayDead") {
                c.goal("KillMechanic");
                c.set("MS_RayDead", true, 12);
            }
        }
        "11_PARIS_CATHEDRAL" => {
            if c.b("M11MeetGunther_Played") && c.b("KillGunther") && !c.b("MS_GuntherKilled") {
                if c.count("GuntherHermann", None) > 0 {
                    crate::ai::world_req(WorldReq::Killswitch(Sel::isa("GuntherHermann")));
                    c.set("GuntherHermann_Dead", true, 0);
                    c.set("MS_GuntherKilled", true, 12);
                }
            }
            if c.b("templar_upload") && c.b("M11NearWalt") && !c.b("MS_M11WaltAppeared") {
                c.enter(Sel::isa("WaltonSimons"));
                c.set("MS_M11WaltAppeared", true, 12);
            }
            if c.b("M11WaltonHolo_Played") && !c.b("MS_M11WaltRemoved") {
                c.leave(Sel::isa("WaltonSimons"));
                c.set("MS_M11WaltRemoved", true, 12);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission12/14

fn m12_first(c: &mut Ctx, map: &str) {
    match map {
        "12_VANDENBERG_CMD" => {
            if c.b("GaryHostageBriefing_Played") {
                c.vehicle(ActorSel::class("BlackHelicopter"), true);
                c.enter(Sel::isa("Jock"));
                c.enter(Sel::isa("TracerTong"));
            }
        }
        "12_VANDENBERG_GAS" => {
            c.set("RescueBegan", true, 14);
            if c.b("SandraWentToCalifornia") {
                c.enter(Sel::isa("SandraRenton"));
            }
        }
        _ => {}
    }
}

fn m12_timer(c: &mut Ctx, map: &str) {
    match map {
        "12_VANDENBERG_CMD" => {
            if !c.b("MS_DL_Played") {
                let count = c.pawns_of("Robot", Some("enemy_bot")).iter().filter(|p| p.emp_hp > 0.0).count();
                match count {
                    3 => c.datalink("DL_bots_left_3"),
                    2 => c.datalink("DL_bots_left_2"),
                    1 => c.datalink("DL_bots_left_1"),
                    0 => {
                        c.datalink("DL_command_bots_destroyed");
                        c.set("MS_DL_Played", true, 14);
                    }
                    _ => {}
                }
            }
            if !c.b("CapturedScientistRescued")
                && c.count("MJ12Troop", Some("MJ12_hazlab_troop1")) + c.count("MJ12Troop", Some("MJ12_hazlab_troop2")) == 0
            {
                c.set("CapturedScientistRescued", true, 14);
            }
        }
        "12_VANDENBERG_GAS" => {
            if !c.b("MS_ChopperGasUnhidden") && (c.b("MeetTiffanySavage_Played") || c.b("TiffanySavage_Dead")) {
                c.vehicle(ActorSel::class("BlackHelicopter"), true);
                c.blow_up("junkyard_doors");
                c.set("TiffanyRescued", true, 14);
                c.set("MS_ChopperGasUnhidden", true, 14);
            }
            if !c.b("MS_TiffanyDLPlayed") && c.b("TiffanySavage_Dead") {
                c.datalink("DL_JockTiffanyDead");
                c.set("MS_TiffanyDLPlayed", true, 14);
            }
        }
        "12_VANDENBERG_COMPUTER" => {
            if !c.b("MS_M12PageAppeared") && c.b("M12PageAppears") {
                c.hide(ActorSel::class("Earth"), true);
                c.enter(Sel::isa("BobPage"));
                c.set("MS_M12PageAppeared", true, 14);
            }
            if c.b("MS_M12PageAppeared") && !c.b("MS_BobPageHidden") && c.b("PageHostageBriefing_Played") {
                c.hide(ActorSel::class("Earth"), false);
                c.leave(Sel::isa("BobPage"));
                c.set("MS_BobPageHidden", true, 14);
            }
        }
        _ => {}
    }
}

fn m14_first(c: &mut Ctx, map: &str) {
    if map == "14_OCEANLAB_LAB" {
        c.goal("StealSub");
    }
}

fn m14_timer(c: &mut Ctx, map: &str) {
    match map {
        "14_VANDENBERG_SUB" => {
            if !c.b("MS_DestroySub") && c.b("DL_downloaded_Played") {
                c.destroy_actors(ActorSel::class_tag("MiniSub", "MiniSub2"));
                c.destroy_actors(ActorSel::tag("welding_stuff"));
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "BlackHelicopter"), true);
                c.enter(Sel::isa("GarySavage"));
                c.set("MS_DestroySub", true, 15);
            }
        }
        "14_OCEANLAB_LAB" => {
            if !c.b("MS_UnhideSub") && c.b("DL_downloaded_Played") {
                c.enter(Sel::isa("WaltonSimons"));
                c.vehicle(ActorSel::class_tag("MiniSub", "MiniSub2"), true);
                c.enter(Sel::isa_tag("ScubaDiver", "scubateam"));
                c.set("MS_UnhideSub", true, 15);
            }
        }
        "14_OCEANLAB_SILO" => {
            if !c.b("MS_UnhideHelicopter") && c.count("HowardStrong", None) == 0 {
                c.vehicle(ActorSel::class_tag("BlackHelicopter", "BlackHelicopter"), true);
                c.datalink("DL_Dead");
                c.set("MS_UnhideHelicopter", true, 15);
            }
        }
        "14_OCEANLAB_UC" => {
            if !c.b("MS_UnhideBobPage") && c.b("schematic_downloaded") {
                c.enter(Sel::isa("BobPage"));
                c.set("MS_UnhideBobPage", true, 15);
            }
            if !c.b("MS_HideBobPage") && c.b("PageTaunt_Played") {
                c.leave(Sel::isa("BobPage"));
                c.set("MS_HideBobPage", true, 15);
            }
        }
        _ => {}
    }
}

// =================================================================================================== Mission15

/// Mission15 spawnData (defaults): (spawn point tag, class, pawn tag, patrol order tag).
const UC_SPAWNS: [(&str, &str, &str, &str); 12] = [
    ("UC_spawn1", "SpiderBot2", "spbot1", "spiderbot1_0"),
    ("UC_spawn1", "SpiderBot2", "spbot2", "spiderbot2_0"),
    ("UC_spawn2", "Gray", "gray_1", "gray1_0"),
    ("UC_spawn2", "Gray", "gray_2", "gray2_0"),
    ("UC_spawn2", "Gray", "gray_3", "gray3_0"),
    ("UC_spawn3", "Karkian", "karkian_1", "karkian1_0"),
    ("UC_spawn3", "Karkian", "karkian_2", "karkian2_0"),
    ("UC_spawn3", "Karkian", "karkian_3", "karkian3_0"),
    ("UC_spawn3", "Greasel", "greasel_1", "greasel1_0"),
    ("UC_spawn3", "Greasel", "greasel_2", "greasel2_0"),
    ("UC_spawn3", "Greasel", "greasel_3", "greasel3_0"),
    ("UC_spawn3", "Greasel", "greasel_4", "greasel4_0"),
];

fn m15_first(c: &mut Ctx, map: &str) {
    if map == "15_AREA51_BUNKER" && !c.b("Ray_dead") {
        c.vehicle(ActorSel::class_tag("BlackHelicopter", "heli_sabotaged"), true);
        c.vehicle(ActorSel::class_tag("BlackHelicopter", "UN_BlackHeli"), false);
        c.trigger("InterpolateTrigger");
        c.vars.jock_timer = c.now;
        c.set("MS_BeginSabotage", true, 16);
    }
}

fn m15_timer(c: &mut Ctx, map: &str) {
    if !c.vars.inited {
        c.vars.inited = true;
        c.vars.last_killed = [-1.0; 12];
    }
    match map {
        "15_AREA51_BUNKER" => {
            if c.b("MS_BeginSabotage") && !c.b("MS_JockDead") {
                if !c.b("DL_JockDeath_Played") && c.now - c.vars.jock_timer >= 8.0 {
                    c.datalink("DL_JockDeath");
                }
                let heli = c.deco_pos("BlackHelicopter", "heli_sabotaged");
                let heli_h = heli.map_or(0.0, |(_, a)| match find(&c.lvl.actors[a].props, "CollisionHeight") {
                    Some(Value::Float(v)) => *v as f64,
                    _ => 112.0,
                });
                let heli_skin = heli.and_then(|(_, a)| match find(&c.lvl.actors[a].props, "Skin").or_else(|| find(&c.lvl.actors[a].props, "MultiSkins")) {
                    Some(Value::Path(p)) => Some(p.clone()),
                    _ => None,
                });
                if c.b("DL_JockDeath_Played") {
                    if let Some((at, _)) = heli {
                        // Large explosion sprites, metal fragments, a light sphere.
                        let mut loc = at;
                        for _ in 0..20 {
                            loc = at + c.vrand() * heli_h;
                            let class = if c.rand() < 0.25 { "ExplosionMedium" } else { "ExplosionLarge" };
                            c.fx(Fx::Explosion(loc, class));
                        }
                        for _ in 0..20 {
                            loc = at + c.vrand() * heli_h;
                            let scale = (5.0 + 2.0 * c.rand()) as f32;
                            let smoking = c.rand() < 0.5;
                            c.fx(Fx::Fragment { pos: loc, speed: 40000.0, spread: 512.0, scale, skin: heli_skin.clone(), smoking });
                        }
                        c.fx(Fx::Sphere(loc, 32.0));
                        c.sound("LargeExplosion2");
                        c.shake(1.0, 1024.0, 32.0);
                        c.destroy_actors(ActorSel::class_tag("BlackHelicopter", "heli_sabotaged"));
                    }
                    c.datalink("DL_JockDeathTongComment");
                    c.set("MS_JockDead", true, 16);
                } else if c.now - c.vars.jock_timer >= 11.0 {
                    if let Some((at, _)) = heli {
                        for _ in 0..6 {
                            let loc = at + c.vrand() * heli_h;
                            let class = if c.rand() < 0.25 { "ExplosionSmall" } else { "ExplosionMedium" };
                            c.fx(Fx::Explosion(loc, class));
                        }
                        c.sound("MediumExplosion1");
                    }
                }
            }
            if !c.b("MS_FanDestroyed") && c.deco_exists("Fan1", Some("Fan_vertical_shaft_1")) == 0 {
                c.trigger("fan");
                c.set("MS_FanDestroyed", true, 16);
            }
        }
        "15_AREA51_ENTRANCE" => {
            if !c.b("MS_EverettAppeared") && c.b("EverettAppears") {
                c.hide(ActorSel::class("Earth"), true);
                c.enter(Sel::isa("MorganEverett"));
                c.set("MS_EverettAppeared", true, 16);
            }
            if !c.b("MS_MorganEverettHidden") && c.b("M15MeetEverett_Played") {
                c.hide(ActorSel::class("Earth"), false);
                c.leave(Sel::isa("MorganEverett"));
                c.set("MS_MorganEverettHidden", true, 16);
            }
        }
        "15_AREA51_FINAL" => {
            if c.b("MeetHelios_Played") && !c.b("MS_CommandosUnhidden") {
                for t in ["commando1", "commando2", "commando3"] {
                    c.enter(Sel::isa_tag("MJ12Commando", t));
                }
                c.set("MS_CommandosUnhidden", true, 16);
            }
            if !c.b("MS_ButtonsHidden") && c.b("coolantcut") {
                c.hide(ActorSel::class_tag("Switch2", "gen_switch1_off"), true);
                c.hide(ActorSel::class_tag("Switch2", "gen_switch2_off"), true);
                c.hide(ActorSel::class_tag("Switch2", "gen_switch1_on"), false);
                c.hide(ActorSel::class_tag("Switch2", "gen_switch2_on"), false);
                c.set("MS_ButtonsHidden", true, 16);
            }
            if !c.b("MS_TongAppeared") && c.b("TongAppears") {
                c.hide(ActorSel::class("Earth"), true);
                c.enter(Sel::isa("TracerTong"));
                c.set("MS_TongAppeared", true, 16);
            }
            if !c.b("MS_TracerTongHidden") && c.b("M15MeetTong_Played") {
                c.hide(ActorSel::class("Earth"), false);
                c.leave(Sel::isa("TracerTong"));
                c.set("MS_TracerTongHidden", true, 16);
            }
            if !c.b("MS_PaulOrGaryAppeared") && c.b("PaulAppears") {
                if c.b("PaulDenton_Dead") {
                    c.enter(Sel::tag("GarySavage"));
                } else {
                    c.enter(Sel::tag("PaulDenton"));
                }
                c.set("MS_PaulOrGaryAppeared", true, 16);
            }
            if !c.b("MS_PaulOrGaryHidden") && (c.b("M15PaulHolo_Played") || c.b("M15GaryHolo_Played")) {
                c.leave(Sel::tag("PaulDenton"));
                c.leave(Sel::tag("GarySavage"));
                c.set("MS_PaulOrGaryHidden", true, 16);
            }
        }
        "15_AREA51_PAGE" => {
            // The UC respawns what's killed after 20 s, with a flash and sparks at the spawn point.
            for (i, (sp, class, tag, order)) in UC_SPAWNS.iter().enumerate() {
                let count = c.pawns.iter().filter(|p| p.class.eq_ignore_ascii_case(class) && p.tag.eq_ignore_ascii_case(tag)).count();
                if count == 0 && c.vars.last_killed[i] == -1.0 {
                    c.vars.last_killed[i] = c.now;
                }
                if count == 0 && c.now - c.vars.last_killed[i] > 20.0 {
                    if let Some((pos, yaw)) = c.point("SpawnPoint", sp) {
                        c.fx(Fx::SpawnFlash(pos));
                        crate::sound::play("DeusExSounds.Generic.Spark1", pos, 2.0, 2048.0, 2.0);
                        if let Ok(mut q) = SPAWNS.lock() {
                            q.push((class.to_string(), tag.to_string(), pos, yaw, order.to_string()));
                        }
                        c.vars.last_killed[i] = -1.0;
                    }
                }
            }
            if !c.b("MS_DL_Played") {
                let count = (1..=4).filter(|i| c.b(&format!("Node{i}_Frobbed"))).count();
                for (n, dl) in [(1, "DL_Blue1"), (2, "DL_Blue2"), (3, "DL_Blue3"), (4, "DL_Blue4")] {
                    if count == n && !c.b(&format!("{dl}_Played")) {
                        c.datalink(dl);
                        if n == 4 {
                            c.set("MS_DL_Played", true, 16);
                        }
                    }
                }
            }
            if c.b("killpage") && !c.b("MS_PageExploding") {
                c.vars.page_timer = c.now;
                c.set("MS_PageExploding", true, 16);
            }
            if c.b("MS_PageExploding") {
                let t = c.now - c.vars.page_timer;
                let page = c.decos.iter().find(|d| d.1.eq_ignore_ascii_case("BobPageAugmented")).map(|d| d.3);
                if t >= 3.0 {
                    // PageExplosionEffects.
                    let size = (0.5 * c.rand() + 0.5) as f32;
                    c.sound(if size < 0.75 {
                        "MediumExplosion2"
                    } else if size < 0.85 {
                        "LargeExplosion1"
                    } else {
                        "LargeExplosion2"
                    });
                    c.shake(0.5 + size, 512.0 + 1024.0 * size, 8.0 + 16.0 * size);
                    let v = c.vrand();
                    player_req(PlayerReq::Push(DVec3::new(300.0, 300.0, 200.0) + v * (500.0 * size as f64)));
                    for _ in 0..(size * 10.0) as i32 {
                        let v = c.vrand();
                        let base = page.unwrap_or(c.player_pos);
                        let mut loc = base + v * if page.is_some() { 64.0 } else { 256.0 };
                        loc.z = base.z;
                        let (hit, n) = c.ceiling(loc);
                        c.fx(Fx::Explosion(hit + n * 8.0, if size < 0.75 { "ExplosionMedium" } else { "ExplosionLarge" }));
                        c.falling_frag(hit, 0.5);
                    }
                }
                if t >= 6.0 && !c.b("MS_PageDestroyed") {
                    c.blow_up("platform_pieces");
                    c.destroy_actors(ActorSel::class("LifeSupportBase"));
                    c.destroy_actors(ActorSel::class("BobPageAugmented"));
                    c.set("MS_PageDestroyed", true, 16);
                }
                if t >= 9.0 {
                    c.trigger("start_endgame");
                }
            }
        }
        _ => {}
    }
}

// =================================================================================================== Endgame

const ENDGAME_DELAYS: [f32; 3] = [13.0, 13.5, 10.5];
const ENDGAME_QUOTES: [&str; 6] = [
    "YESTERDAY WE OBEYED KINGS AND BENT OUR NECKS BEFORE EMPERORS.  BUT TODAY WE KNEEL ONLY TO TRUTH...",
    "    -- KAHLIL GIBRAN",
    "IF THERE WERE NO GOD, IT WOULD BE NECESSARY TO INVENT HIM.",
    "    -- VOLTAIRE",
    "BETTER TO REIGN IN HELL, THAN SERVE IN HEAVEN.",
    "    -- PARADISE LOST, JOHN MILTON",
];

fn endgame_first(c: &mut Ctx, map: &str) {
    c.vars.endgame_timer = 0.0;
    let con = match map {
        "ENDGAME1" | "99_ENDGAME1" => Some("Endgame1"),
        "ENDGAME2" | "99_ENDGAME2" => Some("Endgame2"),
        "ENDGAME3" | "99_ENDGAME3" => Some("Endgame3"),
        _ => None,
    };
    if let Some(con) = con {
        // StartConversationByName(con, Player, False, True): forced, the player as the owner. Lay D Denton plays
        // FemJCEndgame1..3 for her (Human.bMadeFemale: the flags are gone by now).
        let con = if crate::travel::female() { format!("FemJC{con}") } else { con.to_string() };
        crate::consys::start_forced(c.fl, &con, "JCDenton");
    }
    player_req(PlayerReq::SoundVolume(Some(32.0)));
}

fn endgame_timer(c: &mut Ctx) {
    if c.b("EndgameExplosions") {
        if c.rand() < 0.8 {
            let size = c.rand() as f32;
            c.sound(if size < 0.5 { "LargeExplosion1" } else { "LargeExplosion2" });
            if c.rand() < 0.8 {
                for _ in 0..(size * 10.0) as i32 {
                    let v = c.vrand();
                    let mut loc = c.player_pos + v * 512.0;
                    loc.z = c.player_pos.z;
                    let (hit, n) = c.ceiling(loc);
                    c.fx(Fx::Explosion(hit + n * 8.0, if size < 0.5 { "ExplosionMedium" } else { "ExplosionLarge" }));
                    if c.rand() < 0.5 {
                        c.falling_frag(hit, 0.75);
                    }
                }
            }
        }
    }
    for (i, flag) in ["Endgame1_Played", "Endgame2_Played", "Endgame3_Played"].iter().enumerate() {
        if c.b(flag) {
            if !c.vars.quote_printed {
                c.vars.quote_printed = true;
                c.setd("EndgameExplosions", false);
                player_req(PlayerReq::Quote(vec![ENDGAME_QUOTES[2 * i].into(), ENDGAME_QUOTES[2 * i + 1].into()], ENDGAME_DELAYS[i]));
            }
            c.vars.endgame_timer += 1.0;
            if c.vars.endgame_timer > ENDGAME_DELAYS[i] {
                c.setd("EndgameExplosions", false);
                player_req(PlayerReq::Credits);
                c.vars.endgame_timer = -1.0e9;
            }
            break;
        }
    }
}

fn take<T>(q: &Mutex<Vec<T>>) -> Vec<T> {
    q.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// What the mission scripts asked of the player, the screen and the world this frame.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn mission_fx_system(
    mut commands: Commands,
    mut player: ResMut<crate::Player>,
    mut ph: ResMut<crate::ai::PlayerHealth>,
    (mut inv, mut weapons, mut throws, mut launches): (
        ResMut<crate::pickups::PlayerInventory>,
        ResMut<crate::weapons::Weapons>,
        ResMut<crate::destruct::ItemThrows>,
        ResMut<crate::projectiles::Launches>,
    ),
    (mut assets, mut images, mut meshes, mut materials): (
        ResMut<crate::weapons::RuntimeAssets>,
        ResMut<Assets<Image>>,
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
    ),
) {
    for r in take(&PLAYER) {
        match r {
            PlayerReq::Shake(t, roll, vert) => crate::shake::shake_view(t, roll, vert),
            PlayerReq::Push(v) => player.body.velocity += v,
            PlayerReq::HealMin(m) => {
                for p in ph.parts.iter_mut() {
                    *p = p.max(m);
                }
                ph.recompute();
            }
            PlayerReq::DropInventory(spots) => {
                // One displayable item per spot (ammo, credits and the key ring stay); a weapon goes back to its
                // default PickupAmmoCount.
                for at in spots {
                    if inv.0.items.is_empty() {
                        break;
                    }
                    let item = inv.0.remove(0);
                    let class = item.def.class.clone();
                    if weapons.held.as_ref().is_some_and(|h| h.stats.class == class) {
                        weapons.held = None;
                        weapons.pending = None;
                    }
                    if weapons.pending_class.as_deref() == Some(class.as_str()) {
                        weapons.pending_class = None;
                    }
                    let mut props = Vec::new();
                    if item.count > 1 {
                        props.push(Property { name: "NumCopies".into(), index: 0, value: Value::Int(item.count as i32) });
                    }
                    for (k, a) in item.data.iter().enumerate() {
                        props.push(Property { name: "AddAugs".into(), index: k as u32, value: Value::Name(a.clone()) });
                    }
                    println!("MISSION inventory: {class} left at {at:.0}");
                    throws.0.push((class, at, DVec3::ZERO, None, props));
                }
            }
            PlayerReq::RemoveAll => {
                inv.0.items.clear();
                inv.0.belt = [None; 10];
                inv.0.ammo.clear();
                weapons.held = None;
                weapons.pending = None;
                weapons.pending_class = None;
            }
            PlayerReq::Captured => {
                // Not dead after all: the lab's FirstFrame tops every part up to 50.
                ph.dead = false;
                for p in ph.parts.iter_mut() {
                    *p = p.max(1.0);
                }
                ph.recompute();
            }
            PlayerReq::Quote(lines, secs) => crate::classic_hud::quote_text(&lines, secs),
            PlayerReq::Credits => crate::credits::show(),
            PlayerReq::SoundVolume(v) => crate::sound::set_instant_sound_volume(v),
        }
    }
    if let Some(snd) = ZONE_AMBIENT.lock().ok().and_then(|z| z.clone()) {
        crate::sound::hold(0x5A4F_4E45_0000, &snd, None, 1.0, 0.0, 1.0);
    }
    for (from, dir, at) in take(&ROCKETS) {
        launches.0.push(crate::projectiles::Launch {
            class: "RocketLAW".into(),
            pos: from,
            dir,
            from: None,
            mult: 1.0,
            life: None,
            scale: None,
            still: false,
            track: None,
            track_at: at,
            speed: None,
            ownerless: true,
        });
    }
    for f in take(&FX) {
        match f {
            Fx::Explosion(at, class) => {
                crate::explosion::spawn_fireball(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, class, at);
            }
            Fx::Fragment { pos, speed, spread, scale, skin, smoking } => {
                let _ = smoking;
                crate::destruct::burst(crate::destruct::Burst {
                    class: "MetalFragment".into(),
                    pos,
                    count: 1,
                    spread: spread.min(speed / 40.0),
                    scale,
                    skin,
                });
            }
            Fx::Sphere(at, size) => {
                crate::explosion::spawn_mesh_fx(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, "SphereEffect", at, [0, 0, 0], size);
                crate::dynlight::explosion(at, 8.0, 16, 192);
            }
            Fx::SpawnFlash(at) => {
                crate::dynlight::explosion(at, 4.0, 16, 192);
                crate::explosion::spawn_mesh_fx(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, "EllipseEffect", at, [0, 0, 0], 1.0);
                crate::explosion::spawn_mesh_fx(&mut commands, &mut assets, &mut images, &mut meshes, &mut materials, "SphereEffect", at, [0, 0, 0], 4.0);
            }
        }
    }
    for (class, tag, pos, yaw, order) in take(&SPAWNS) {
        // Spawn + InitializePawn, SetOrders('Patrolling', orderTag), ChangeAlly('Player', -1, True),
        // ChangeAlly('MJ12', 0, True).
        crate::spawn::pawn_with(
            &class,
            pos,
            yaw,
            vec![
                Property { name: "Tag".into(), index: 0, value: Value::Name(tag) },
                Property { name: "Orders".into(), index: 0, value: Value::Name("Patrolling".into()) },
                Property { name: "OrderTag".into(), index: 0, value: Value::Name(order) },
                Property { name: "InitialAlliances".into(), index: 6, value: Value::Alliance("Player".into(), -1.0, true) },
                Property { name: "InitialAlliances".into(), index: 7, value: Value::Alliance("MJ12".into(), 0.0, true) },
            ],
        );
    }
}
