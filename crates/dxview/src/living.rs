//! The living level (MULTIPLAYER roadmap Phase 2): in a Modern deathmatch the level's own security - guards,
//! cameras, turrets and alarms - is a third side. It is neutral to every player until one of them gives it a
//! reason, and then it hunts that player only.
//!
//! * Heat (imsim_core::heat) per player, worked out on the host: unsilenced shots staff hear, being seen in a
//!   restricted room (a security room), a camera spotting a drawn weapon, staff seeing a lockpick or multitool at
//!   work, and hurting staff (wanted at once). Wanted players are hunted while staff keep seeing them; out of sight
//!   the heat drains. A new life starts clean.
//! * The staff's alliance tables follow the heat: each player is his own alliance to them ("Player" for the
//!   host's own, "MPPlayer<id>" for the others' bodies), hostile only while he is wanted.
//! * Becoming wanted raises the alarm: everyone's news, and a ping on everyone's screen where he was seen.
//! * Hacked security changes hands (security.rs): a security computer gives its cameras and turrets to whoever
//!   used it (and his team) for `hacked_secs`; somebody else can take them back the same way.
//! * Level activity (the host's Match Settings): Off = the security does nothing, Light = cameras and alarms,
//!   Full = cameras, alarms, turrets and guards (from the map's own patrol routes, added as the map is read).
//!
//! The tuning is data (data/deathmatch.ron `living`).

use crate::deathmatch::Activity;
use bevy::prelude::*;
use glam::DVec3;
use imsim_core::heat::{Heat, HeatCause, HeatLevel, HeatTuning};
use serde::Deserialize;
use std::sync::Mutex;

/// The killer id for "the level's security" (a guard or an unowned turret): no player gets the kill and the victim
/// loses nothing.
pub const LEVEL: u8 = 255;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LivingTuning {
    pub heat: HeatTuning,
    /// Seconds a security computer's cameras and turrets stay with whoever used it.
    pub hacked_secs: f32,
    /// Restricted rooms come from the map's data (`restricted`); on maps without any, security computers'
    /// rooms count when this is on (within `restricted_radius`, in sight of the computer).
    pub auto_restricted: bool,
    pub restricted_radius: f32,
    /// Seconds an alarm's ping stays on screen.
    pub ping_secs: f32,
    /// An owned camera re-pings a rival it keeps seeing this often.
    pub owned_ping_every: f32,
    /// Guards walk the map's patrol routes: the class for people, and for routes whose name says "bot".
    pub guard: String,
    pub bot: String,
    pub max_guards: usize,
    /// Guards don't see a player in the dark beyond this (light below `dark`).
    pub dark_reach: f32,
    pub dark: f32,
}

impl Default for LivingTuning {
    fn default() -> Self {
        LivingTuning {
            heat: HeatTuning::default(),
            hacked_secs: 120.0,
            auto_restricted: false,
            restricted_radius: 300.0,
            ping_secs: 8.0,
            owned_ping_every: 2.0,
            guard: "MJ12Troop".into(),
            bot: "SecurityBot3".into(),
            max_guards: 6,
            dark_reach: 320.0,
            dark: 0.06,
        }
    }
}

/// A map's own staff settings (data MapEntry `staff`): other classes, fewer guards, routes left unguarded.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct StaffSpec {
    pub guard: Option<String>,
    pub bot: Option<String>,
    pub max: Option<usize>,
    /// Patrol route names (their prefix, as `routes` gives them) that get no guard.
    pub skip: Vec<String>,
}

pub fn tuning() -> &'static LivingTuning {
    &crate::deathmatch::data().living
}

/// The level's activity in the match being played (Off outside a Modern deathmatch).
pub fn activity() -> Activity {
    if !crate::deathmatch::playing() {
        return Activity::Off;
    }
    crate::deathmatch::rules().filter(|r| !r.classic()).map_or(Activity::Off, |r| r.activity)
}

pub fn active() -> bool {
    activity() != Activity::Off
}

/// A Modern deathmatch is being played (whatever the level's activity).
pub fn modern_match() -> bool {
    crate::deathmatch::playing() && !crate::deathmatch::classic()
}

pub fn full() -> bool {
    activity() == Activity::Full
}

/// This machine runs the level's security (the host, or a game without a network).
fn runs_security() -> bool {
    !crate::netplay::is_client()
}

pub fn host_active() -> bool {
    active() && runs_security()
}

/// The alliance a player is to the level's staff: the host's own player is "Player", the others' bodies here are
/// "MPPlayer<id>" (netplay gives them that alliance in a Modern deathmatch).
pub fn alliance_for(id: u8) -> String {
    if id == crate::deathmatch::my_id() {
        "Player".into()
    } else {
        body_alliance(id)
    }
}

pub fn body_alliance(id: u8) -> String {
    format!("MPPlayer{id}")
}

/// The player behind an AI target, if it is one.
pub fn id_of(t: crate::ai::Target) -> Option<u8> {
    match t {
        crate::ai::Target::Player => Some(crate::deathmatch::my_id()),
        crate::ai::Target::Pawn(b) => crate::netplay::peer_of_blocker(b),
    }
}

/// An eye of the level's staff: a guard where he stands, looking his way.
#[derive(Clone, Copy, Debug)]
pub struct StaffEye {
    pub blocker: usize,
    pub eye: DVec3,
    pub yaw: f64,
    pub sight: f64,
}

/// Who sees a ping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    /// An alarm: everyone.
    All,
    /// A hacked camera's report in a team game: its owner's team.
    Team(u8),
    /// A hacked camera's report in a free-for-all: its owner.
    Player(u8),
}

impl Audience {
    /// On the wire (netplay Msg::Ping): None = all, a team, or 100 + a player.
    pub fn code(self) -> Option<u8> {
        match self {
            Audience::All => None,
            Audience::Team(t) => Some(t),
            Audience::Player(p) => Some(100u8.saturating_add(p)),
        }
    }
    pub fn from_code(c: Option<u8>) -> Audience {
        match c {
            None => Audience::All,
            Some(c) if c >= 100 => Audience::Player(c - 100),
            Some(t) => Audience::Team(t),
        }
    }
    fn includes(self, me: u8) -> bool {
        match self {
            Audience::All => true,
            Audience::Team(t) => crate::deathmatch::team_of(me) == Some(t),
            Audience::Player(p) => p == me,
        }
    }
}

/// A body of the level's staff (dead or knocked out), for framing and uniforms.
#[derive(Clone, Debug)]
pub struct StaffBody {
    pub blocker: usize,
    pub pos: DVec3,
    pub knocked_out: bool,
    /// Who took him down (None: the level, or nobody known).
    pub killer: Option<u8>,
    /// Carried, hidden or gone: nobody can find it.
    pub hidden: bool,
}

/// A restricted room: centre, BSP zone (-1: any), radius, seen from the centre only, name.
#[derive(Clone, Debug)]
struct Room {
    at: DVec3,
    zone: i32,
    r: f64,
    los: bool,
    name: String,
}

/// A hit on staff waiting to be judged: whether the victim stays up or anyone saw it decides the heat.
#[derive(Clone, Debug)]
struct PendingHit {
    who: u8,
    at: DVec3,
    victim: usize,
    left: f32,
}

#[derive(Clone, Debug)]
pub struct Ping {
    pub at: DVec3,
    pub who: u8,
    /// Seconds left on screen.
    pub left: f32,
    pub audience: Audience,
}

#[derive(Default)]
struct State {
    heat: Heat,
    /// Levels as this machine knows them (the host's own, or what he sent).
    levels: Vec<(u8, u8)>,
    /// (id, alliance, wanted) of every player in the match, for the staff's tables.
    players: Vec<(u8, String, bool)>,
    pings: Vec<Ping>,
    /// The restricted rooms of this map.
    restricted: Vec<Room>,
    map_gen: u64,
    /// Hits on staff to judge (silent takedowns raise nothing).
    pending: Vec<PendingHit>,
    sight_t: f32,
    send_t: f32,
    sent: Vec<(u8, u8)>,
    /// Pings to send to the joiners.
    out_pings: Vec<Ping>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { heat: Heat::new(tuning().heat.clone()), ..Default::default() }))
}

pub fn reset_level() {
    with(|s| {
        s.heat = Heat::new(tuning().heat.clone());
        s.levels.clear();
        s.players.clear();
        s.pings.clear();
        s.out_pings.clear();
        s.restricted.clear();
        s.pending.clear();
        s.sent.clear();
        s.map_gen = 0;
    });
}

/// A player's heat level as this machine knows it.
pub fn level_of(id: u8) -> HeatLevel {
    with(|s| s.levels.iter().find(|l| l.0 == id).map_or(HeatLevel::Calm, |l| HeatLevel::from_code(l.1)))
}

pub fn wanted(id: u8) -> bool {
    level_of(id) == HeatLevel::Wanted
}

fn name_of(id: u8) -> String {
    if id == crate::deathmatch::my_id() {
        crate::travel::player_name()
    } else {
        crate::netplay::players().into_iter().find(|p| p.0 == id).map_or_else(|| format!("Player {id}"), |p| p.1)
    }
}

/// Host: heat for `id`. Becoming wanted raises the alarm (`at`: where he was).
pub fn raise(id: u8, cause: HeatCause, secs: f32, at: Option<DVec3>) {
    if !host_active() || id == LEVEL {
        return;
    }
    // In a uniform (uniforms.rs) staff take him for one of their own: a weapon, a tool or a restricted room only
    // make them wonder (the cover's own heat, Disguise), and only where they can tell (observe / camera_sight).
    if disguised(id) && matches!(cause, HeatCause::WeaponSeen | HeatCause::Restricted | HeatCause::Tampering) {
        return;
    }
    let became = with(|s| s.heat.raise(id, cause, secs));
    if became {
        let why = match cause {
            HeatCause::AttackedStaff => "attacked security",
            HeatCause::ShotHeard => "was heard shooting",
            HeatCause::WeaponSeen => "was seen armed",
            HeatCause::Restricted => "was seen in a restricted area",
            HeatCause::Tampering => "was seen tampering",
            HeatCause::BodyFound => "is blamed for a body",
            HeatCause::WeaponFound => "left a weapon lying about",
            HeatCause::Dispatched => "was reported by Overwatch",
            HeatCause::Disguise => "was seen through his uniform",
        };
        // Wanted in a uniform: the cover is blown.
        if disguised(id) {
            crate::uniforms::host_blown(id);
        }
        let line = format!("ALARM: {} {why}.", name_of(id));
        crate::netplay::host_news(&line);
        if !crate::netplay::is_host() {
            crate::netplay::feed(&line, [255, 80, 64]);
        }
        if let Some(at) = at {
            ping_for(at, id, Audience::All);
        }
        crate::sound::play_player("DeusExSounds.Generic.Beep6", 0.6, 0.0);
        println!("LIVING {} wanted ({cause:?})", name_of(id));
    }
}

/// Host: staff hurt by `target` (a guard, a camera, a turret): he is wanted at once.
pub fn staff_hurt(target: crate::ai::Target, at: Option<DVec3>) {
    if let Some(id) = id_of(target) {
        raise(id, HeatCause::AttackedStaff, 0.0, at);
    }
}

/// A player is in a uniform (uniforms.rs): his own, or another's as his look says.
pub fn disguised(id: u8) -> bool {
    if id == crate::deathmatch::my_id() {
        crate::uniforms::worn_class().is_some()
    } else {
        crate::netplay::uniform_of(id).is_some()
    }
}

/// Host: the player taking this damage was hurt by the level's own security (the kill isn't anyone's).
pub fn hurt_by_staff() {
    if active() {
        crate::netplay::note_hurt_by(LEVEL);
    }
}

/// Host: a player died (or left): his next life starts clean.
pub fn died(id: u8) {
    with(|s| {
        s.heat.clear(id);
        s.levels.retain(|l| l.0 != id);
    });
}

/// A ping on screens: the host's own, and sent to the joiners.
pub fn ping_for(at: DVec3, who: u8, audience: Audience) {
    let p = Ping { at, who, left: tuning().ping_secs, audience };
    with(|s| {
        s.pings.retain(|q| !(q.who == who && q.audience == audience));
        s.out_pings.push(p.clone());
        s.pings.push(p);
    });
}

/// Host: player `from` (a Marksman) marked rival `who` at `at`: a ping for his team (or him alone in a
/// free-for-all).
pub fn spotted(from: u8, at: DVec3, who: u8) {
    let audience = match crate::deathmatch::team_of(from).filter(|_| crate::deathmatch::team_mode()) {
        Some(t) => Audience::Team(t),
        None => Audience::Player(from),
    };
    ping_for(at, who, audience);
}

/// Joiner: the host's heat levels and pings.
pub fn set_levels(v: Vec<(u8, u8)>) {
    if env_on!("DXVIEW_LIVINGLOG") {
        println!("LIVING levels from the host: {v:?}");
    }
    with(|s| s.levels = v);
}

pub fn add_ping(at: DVec3, who: u8, secs: f32, code: Option<u8>) {
    let audience = Audience::from_code(code);
    if env_on!("DXVIEW_LIVINGLOG") {
        println!("LIVING ping from the host: {who} at {at:.0?} for {audience:?}");
    }
    with(|s| {
        s.pings.retain(|q| !(q.who == who && q.audience == audience));
        s.pings.push(Ping { at, who, left: secs, audience });
    });
}

/// Host: what to send the joiners now (levels when they change or every 2 s, new pings).
pub fn take_out(dt: f32) -> (Option<Vec<(u8, u8)>>, Vec<Ping>) {
    with(|s| {
        s.send_t -= dt;
        let levels = if s.levels != s.sent || s.send_t <= 0.0 {
            s.send_t = 2.0;
            s.sent = s.levels.clone();
            Some(s.levels.clone())
        } else {
            None
        };
        (levels, std::mem::take(&mut s.out_pings))
    })
}

/// The map's restricted rooms, read once per map: the data's areas (in sight of their centre unless the data says
/// otherwise), or (auto_restricted) the rooms of its security computers.
fn refresh_rooms(lvl: &dx_level::Level) {
    let gen = crate::loader::generation();
    if with(|s| s.map_gen == gen) {
        return;
    }
    let t = tuning();
    let mut rooms: Vec<Room> = crate::deathmatch::map_entry(&lvl.name)
        .map(|m| {
            m.restricted
                .iter()
                .map(|a| Room { at: DVec3::new(a.at.0 as f64, a.at.1 as f64, a.at.2 as f64), zone: -1, r: a.radius as f64, los: a.in_sight, name: a.name.clone() })
                .collect()
        })
        .unwrap_or_default();
    if rooms.is_empty() && t.auto_restricted {
        rooms = lvl
            .actors
            .iter()
            .filter(|a| a.class.eq_ignore_ascii_case("ComputerSecurity"))
            .map(|a| {
                let l = a.location();
                let p = DVec3::new(l[0] as f64, l[1] as f64, l[2] as f64);
                Room { at: p, zone: lvl.collision.zone_at(p), r: t.restricted_radius as f64, los: true, name: "Security".into() }
            })
            .collect();
    }
    with(|s| {
        s.map_gen = gen;
        s.restricted = rooms;
    });
}

/// The restricted room a point is in (its name).
pub fn room_at(lvl: &dx_level::Level, at: DVec3) -> Option<String> {
    if !modern_match() {
        return None;
    }
    refresh_rooms(lvl);
    let rooms = with(|s| s.restricted.clone());
    let zone = if rooms.iter().any(|r| r.zone >= 0) { lvl.collision.zone_at(at) } else { -1 };
    // (From the centre a little up: the eye of whoever stands there.)
    rooms
        .iter()
        .find(|r| {
            r.at.distance(at) <= r.r
                && (r.zone < 0 || r.zone == zone)
                && (!r.los || lvl.collision.trace(r.at + DVec3::Z * 24.0, at, DVec3::ZERO).fraction >= 0.999)
        })
        .map(|r| r.name.clone())
}

/// A point is in a restricted room.
pub fn restricted(lvl: &dx_level::Level, at: DVec3) -> bool {
    room_at(lvl, at).is_some()
}

/// The class in this machine's player's hands (living_hud_system keeps it).
static MY_HELD: Mutex<String> = Mutex::new(String::new());

fn held_class(id: u8) -> String {
    if id == crate::deathmatch::my_id() {
        MY_HELD.lock().map(|w| w.clone()).unwrap_or_default()
    } else {
        crate::netplay::held_by(id)
    }
}

/// A weapon in his hands (not a tool or an item).
pub fn armed(id: u8) -> bool {
    let c = held_class(id);
    c.len() > 6 && c[..6].eq_ignore_ascii_case("Weapon")
}

/// A lockpick or multitool in his hands.
pub fn tampering(id: u8) -> bool {
    let c = held_class(id);
    c.eq_ignore_ascii_case("Lockpick") || c.eq_ignore_ascii_case("Multitool")
}

/// Host: a level camera sees player `id` at `at` for `secs`: heat for a drawn weapon, a restricted room or a tool.
/// Returns whether he is wanted (the camera then raises its own alarm).
pub fn camera_sight(lvl: &dx_level::Level, id: u8, at: DVec3, secs: f32) -> bool {
    if !host_active() {
        return false;
    }
    with(|s| s.heat.seen(id));
    if disguised(id) {
        // Staff carry weapons: only a restricted room or a tool at work looks wrong on camera.
        if restricted(lvl, at) || tampering(id) {
            raise(id, HeatCause::Disguise, secs, Some(at));
        }
    } else if restricted(lvl, at) {
        raise(id, HeatCause::Restricted, secs, Some(at));
    } else if tampering(id) {
        raise(id, HeatCause::Tampering, secs, Some(at));
    } else if armed(id) {
        raise(id, HeatCause::WeaponSeen, secs, Some(at));
    }
    with(|s| s.heat.wanted(id))
}

/// Does a guard of the level see this point (his view, his sight radius, light enough beyond dark_reach)?
pub fn eye_sees(lvl: &dx_level::Level, s: &StaffEye, at: DVec3, lit: &mut Option<bool>) -> bool {
    let to = at - s.eye;
    let d = to.length();
    if d > s.sight || d < 1.0 {
        return false;
    }
    // ScriptedPawn's 160 degree view.
    let facing = DVec3::new(s.yaw.cos(), s.yaw.sin(), 0.0);
    if to.truncate().normalize_or_zero().dot(facing.truncate()) < 0.17 {
        return false;
    }
    let t = tuning();
    if d > t.dark_reach as f64 && !*lit.get_or_insert_with(|| crate::stealth::light_at(lvl, at) >= t.dark) {
        return false;
    }
    lvl.collision.trace(s.eye, at, DVec3::ZERO).fraction >= 0.999
}

/// Any of the level's staff (but `except`) or its cameras sees this point.
pub fn witnessed(lvl: &dx_level::Level, staff: &[StaffEye], at: DVec3, except: Option<usize>) -> bool {
    let mut lit = None;
    staff.iter().filter(|s| Some(s.blocker) != except).any(|s| eye_sees(lvl, s, at, &mut lit)) || crate::security::level_camera_near(at, 640.0)
}

/// Host, from npc_system once a frame: shots the staff heard, staff the players hurt, what the guards see; the bodies
/// lying about go to framing.rs.
#[allow(clippy::too_many_arguments)]
pub fn observe(
    lvl: &dx_level::Level,
    dt: f32,
    heard: &[crate::stealth::Noise],
    hits: &std::collections::HashMap<usize, Vec<crate::damage::PawnHit>>,
    staff: &[StaffEye],
    players: &[(u8, DVec3, DVec3)],
    bodies: &[StaffBody],
) {
    if !host_active() {
        return;
    }
    refresh_rooms(lvl);
    // Unsilenced shots heard by a guard (within the noise's reach) or near a camera of the level.
    for n in heard.iter().filter(|n| n.kind == crate::stealth::NoiseKind::WeaponFire) {
        let Some(id) = n.maker.and_then(id_of) else { continue };
        let r = n.radius as f64;
        let near_staff = staff.iter().any(|s| s.eye.distance(n.pos) <= r) || crate::security::level_camera_near(n.pos, r * 0.75);
        if near_staff {
            raise(id, HeatCause::ShotHeard, 0.0, Some(n.pos));
        }
    }
    // Staff hurt by a player: judged a moment later. A guard who stays on his feet knows who hit him; one taken down
    // at once tells nobody, unless another guard or a camera saw it (a silent takedown).
    let wait = crate::mech::tuning().witness_secs;
    for s in staff {
        if let Some(list) = hits.get(&s.blocker) {
            for h in list.iter().filter(|h| !h.world) {
                let t = match h.attacker {
                    None => crate::ai::Target::Player,
                    Some(b) => crate::ai::Target::Pawn(b),
                };
                if let Some(who) = id_of(t) {
                    with(|st| {
                        if !st.pending.iter().any(|p| p.who == who && p.victim == s.blocker) {
                            st.pending.push(PendingHit { who, at: h.attacker_pos, victim: s.blocker, left: wait });
                        }
                    });
                }
            }
        }
    }
    let due: Vec<PendingHit> = with(|st| {
        for p in &mut st.pending {
            p.left -= dt;
        }
        let (due, keep): (Vec<PendingHit>, Vec<PendingHit>) = st.pending.drain(..).partition(|p| p.left <= 0.0);
        st.pending = keep;
        due
    });
    for p in due {
        let standing = staff.iter().any(|s| s.blocker == p.victim);
        if standing || witnessed(lvl, staff, p.at, Some(p.victim)) {
            raise(p.who, HeatCause::AttackedStaff, 0.0, Some(p.at));
        } else if env_on!("DXVIEW_LIVINGLOG") {
            println!("LIVING silent takedown by {}", name_of(p.who));
        }
    }
    // What the guards see, a few times a second.
    let sight = with(|s| {
        s.sight_t -= dt;
        if s.sight_t <= 0.0 {
            s.sight_t = 0.25;
            true
        } else {
            false
        }
    });
    if sight {
        let mt = crate::mech::tuning();
        for &(id, at, vel) in players {
            let mut lit: Option<bool> = None;
            let seeing: Vec<&StaffEye> = staff.iter().filter(|s| eye_sees(lvl, s, at, &mut lit)).collect();
            if seeing.is_empty() {
                continue;
            }
            with(|s| s.heat.seen(id));
            let room = restricted(lvl, at);
            if disguised(id) {
                // A guard among guards - until he acts unlike one where they can see.
                let running = vel.truncate().length() > mt.run_speed as f64;
                let armed_close = armed(id) && seeing.iter().any(|s| s.eye.distance(at) <= mt.armed_near as f64);
                if running || armed_close || room || tampering(id) {
                    raise(id, HeatCause::Disguise, 0.25, Some(at));
                }
            } else if room {
                raise(id, HeatCause::Restricted, 0.25, Some(at));
            } else if tampering(id) {
                raise(id, HeatCause::Tampering, 0.25, Some(at));
            }
        }
    }
    // Bodies and dropped weapons the staff come across (framing.rs).
    crate::framing::host_observe(lvl, dt, staff, bodies);
    // Time passes; the levels and the staff's view of each player follow.
    let me = crate::deathmatch::my_id();
    let mut ids: Vec<u8> = players.iter().map(|p| p.0).collect();
    if !ids.contains(&me) {
        ids.push(me);
    }
    with(|s| {
        s.heat.tick(dt);
        s.levels = s.heat.all().into_iter().map(|(id, l, _)| (id, l.code())).filter(|l| l.1 > 0).collect();
        s.players = ids.iter().map(|&id| (id, alliance_for(id), s.heat.wanted(id))).collect();
    });
}

/// Host, for each of the level's staff each frame: hostile to the wanted players, neutral to the rest.
pub fn staff_table(t: &mut imsim_core::alliance::AllianceTable) {
    with(|s| {
        for (_, name, wanted) in &s.players {
            if *wanted {
                if !t.hostile_to(name) || t.level(name) >= 0.0 {
                    t.change_ally(name, -1.0, true, false);
                }
            } else if t.attitude(name) != imsim_core::alliance::Attitude::Neutral || t.level(name) != 0.0 {
                t.forget(name);
            }
        }
        // Map-placed hostility to the player in general doesn't apply to a match.
        if !s.players.iter().any(|p| p.1 == "Player") {
            t.forget("Player");
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Guards: the map's own patrol routes, each walked by one guard (added to the map as it is read, so every machine
// numbers them alike and the joiners see them as the host's pawns).

/// The patrol routes of a map: (route name, the tag of its first point, where that is, its yaw), in a fixed order.
pub fn routes(patrols: &[dx_level::actors::PatrolInfo]) -> Vec<(String, String, [f32; 3], i32)> {
    let mut by: std::collections::BTreeMap<String, (String, [f32; 3], i32)> = Default::default();
    for p in patrols {
        let tag = &p.tag;
        if tag.is_empty() || tag.eq_ignore_ascii_case("PatrolPoint") {
            continue;
        }
        // The route: the tag without its trailing number ("tower_guard0" -> "tower_guard", "guard1_path2" ->
        // "guard1_path").
        let route = tag.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('_').to_ascii_lowercase();
        let e = by.entry(route).or_insert_with(|| (tag.clone(), p.location, p.yaw));
        if tag.to_ascii_lowercase() < e.0.to_ascii_lowercase() {
            *e = (tag.clone(), p.location, p.yaw);
        }
    }
    by.into_iter().map(|(k, (t, l, y))| (k, t, l, y)).collect()
}

/// The guards to add to `map` (dx-level LevelExtras): only in a Modern deathmatch room with the level on Full, on
/// one of its maps.
fn guard_extras(map: &str, _actors: &[dx_level::actors::ActorInfo], patrols: &[dx_level::actors::PatrolInfo]) -> Vec<(String, String, Vec<unreal_pkg::props::Property>)> {
    use unreal_pkg::props::{Property, Value};
    if env_on!("DXVIEW_LIVINGLOG") {
        println!("LIVING extras for '{map}': rules {:?}, {} routes", crate::deathmatch::rules().map(|r| (r.mode, r.family, r.activity)), routes(patrols).len());
    }
    let Some(r) = crate::deathmatch::rules() else { return Vec::new() };
    if !r.mode.versus() || r.classic() || r.activity != Activity::Full {
        return Vec::new();
    }
    let Some(entry) = crate::deathmatch::map_entry(map) else { return Vec::new() };
    let t = tuning();
    let spec = entry.staff.clone().unwrap_or_default();
    let max = spec.max.unwrap_or(t.max_guards);
    let mut out = Vec::new();
    // People first, then the bots (a map with more routes than guards keeps a mix).
    let mut list = routes(patrols);
    list.sort_by_key(|r| r.0.contains("bot"));
    for (route, tag, at, yaw) in list {
        if out.len() >= max {
            break;
        }
        if spec.skip.iter().any(|s| s.eq_ignore_ascii_case(&route)) {
            continue;
        }
        let class = if route.contains("bot") { spec.bot.clone().unwrap_or_else(|| t.bot.clone()) } else { spec.guard.clone().unwrap_or_else(|| t.guard.clone()) };
        let n = out.len();
        let props = vec![
            Property { name: "Location".into(), index: 0, value: Value::Vector([at[0], at[1], at[2] + 8.0]) },
            Property { name: "Rotation".into(), index: 0, value: Value::Rotator([0, yaw, 0]) },
            Property { name: "Tag".into(), index: 0, value: Value::Name(format!("LivingGuard{n}")) },
            Property { name: "Orders".into(), index: 0, value: Value::Name("Patrolling".into()) },
            Property { name: "OrderTag".into(), index: 0, value: Value::Name(tag.clone()) },
        ];
        println!("LIVING guard {class} on route {route} ({tag}) in {map}");
        out.push((class, format!("LivingGuard{n}"), props));
    }
    out
}

/// At start-up: the guards are added as maps are read.
pub fn install() {
    dx_level::actors::set_level_extras(Some(Box::new(|map: &str, actors: &[dx_level::actors::ActorInfo], patrols: &[dx_level::actors::PatrolInfo]| {
        let mut v = guard_extras(map, actors, patrols);
        // Killphrase datacubes (killphrase.rs).
        v.extend(crate::killphrase::cube_extras(map, actors));
        v
    })));
}

// ---------------------------------------------------------------------------------------------------------------
// HUD: the player's own heat (a small panel under the compass) and the pings.

#[derive(Component, Default)]
pub struct HeatNode;
#[derive(Component)]
pub struct PingMark;

#[derive(Default)]
pub struct HudState {
    /// The Marksman's spotting: (rival, seconds held) and the wait before the next mark.
    spot: (u8, f32),
    spot_cool: f32,
    img: Option<Handle<Image>>,
    key: u64,
    icon: Option<Handle<Image>>,
    marks: Vec<Entity>,
}

#[allow(clippy::too_many_arguments)]
pub fn living_hud_system(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    mut hud: ResMut<crate::classic_hud::ClassicHud>,
    mut assets: ResMut<crate::weapons::RuntimeAssets>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<Assets<crate::classic_hud::HudLayer>>,
    mut node: Query<(Entity, &mut Node, &mut Visibility), (With<HeatNode>, Without<PingMark>)>,
    mut marks: Query<(&mut Node, &mut Visibility), (With<PingMark>, Without<HeatNode>)>,
    cam: Query<(&Camera, &GlobalTransform), With<crate::FlyCam>>,
    weapons: Res<crate::weapons::Weapons>,
    (level, player): (Res<crate::LevelRes>, Res<crate::Player>),
    mut st: Local<HudState>,
) {
    let dt = time.delta_secs();
    if let Ok(mut w) = MY_HELD.lock() {
        let now = weapons.held.as_ref().map_or("", |h| h.stats.class.as_str());
        if w.as_str() != now {
            w.clear();
            w.push_str(now);
        }
    }
    let on = modern_match() && !crate::menus::open() && !crate::consys::third_person();
    let pings: Vec<Ping> = with(|s| {
        for p in &mut s.pings {
            p.left -= dt;
        }
        s.pings.retain(|p| p.left > 0.0);
        s.pings.clone()
    });
    // ---- the Marksman's Spotter: a rival held in the scope for a second is marked (for his team).
    let me = crate::deathmatch::my_id();
    st.spot_cool -= dt;
    let aimed = (crate::deathmatch::playing()
        && crate::dmchar::class_perk() == "marksman"
        && crate::weapons::ZOOMED.load(std::sync::atomic::Ordering::Relaxed))
    .then(crate::hud::aimed_pawn)
    .flatten()
    .and_then(crate::netplay::peer_of_blocker)
    .filter(|&id| !(crate::deathmatch::team_mode() && crate::deathmatch::allied(me, id)));
    match aimed {
        Some(id) => {
            st.spot = if st.spot.0 == id { (id, st.spot.1 + dt) } else { (id, dt) };
            if st.spot.1 >= 1.0 && st.spot_cool <= 0.0 {
                st.spot_cool = 3.0;
                if let Some((_, at)) = crate::netplay::remote_bodies().into_iter().find(|b| b.0 == id) {
                    if crate::netplay::is_client() {
                        crate::netplay::spot_out(at, id);
                    } else {
                        spotted(me, at, id);
                    }
                    crate::classic_hud::client_message("Rival marked.");
                }
            }
        }
        None => st.spot = (u8::MAX, 0.0),
    }

    let Some(win) = windows.iter().next() else { return };
    let s = crate::hud::ui_scale(win).max(1.0) as i32;
    let k = s as f32 / win.scale_factor();

    // ---- own heat, the restricted room he stands in, his uniform (a small panel under the compass)
    let lvl = if on { level_of(me) } else { HeatLevel::Calm };
    let mut lines: Vec<(String, [u8; 3])> = Vec::new();
    let blink = (time.elapsed_secs() * 2.0).fract() < 0.5;
    match lvl {
        HeatLevel::Wanted => lines.push(("SECURITY: WANTED".into(), if blink { [255, 64, 48] } else { [200, 40, 32] })),
        HeatLevel::Suspicious => lines.push(("SECURITY: SUSPICIOUS".into(), [255, 196, 64])),
        _ => {}
    }
    if on {
        if let Some(room) = room_at(&level.0, player.body.location) {
            lines.push((format!("RESTRICTED: {}", room.to_uppercase()), [255, 150, 60]));
        }
        if let Some((name, left)) = crate::uniforms::worn_status() {
            let m = left.max(0.0) as i32;
            lines.push((format!("UNIFORM: {} {}:{:02}", name.to_uppercase(), m / 60, m % 60), [90, 220, 200]));
        }
        lines.extend(crate::killphrase::hud_lines());
        lines.extend(crate::lockdown::hud_lines());
    }
    if lines.is_empty() {
        crate::prompts::present::<HeatNode, _>(&mut commands, &mut images, &mut layers, &mut node, &mut st.img, None, (1, 1), s, Vec2::ZERO, k);
        st.key = 0;
    } else if let Some(font) = hud.font(&mut assets, "FontMenuSmall_DS") {
        let w = lines.iter().map(|l| font.width(&l.0)).max().unwrap_or(0) + 12;
        let h = 15 * lines.len() as i32;
        // Under the compass (the HUD's top-left column), at the HUD's own scale.
        let (pos, s, k) = match crate::touch::hud_window("Compass") {
            Some((r, unit)) => (Vec2::new(r.min.x + 2.0 * unit, r.max.y + 3.0 * unit), (unit * win.scale_factor()).ceil().max(1.0) as i32, unit),
            None => (Vec2::new(14.0 * k, 104.0 * k), s, k),
        };
        let key = crate::util::fnv1a(&format!("{lines:?}{s}{pos:?}"));
        if key != st.key || node.is_empty() {
            st.key = key;
            let mut c = crate::uicanvas::Canvas::new(w, h, s);
            c.fill(0, 0, w, h, crate::uicanvas::Style::Normal, [8, 6, 6]);
            for (i, (text, col)) in lines.iter().enumerate() {
                let y = 15 * i as i32;
                c.fill(0, y, 2, 15, crate::uicanvas::Style::Normal, *col);
                c.text(&font, 6, y, w - 6, 15, crate::uicanvas::HAlign::Left, crate::uicanvas::VAlign::Center, *col, text);
            }
            crate::prompts::present::<HeatNode, _>(&mut commands, &mut images, &mut layers, &mut node, &mut st.img, Some(&c), (w, h), s, pos, k);
        }
    }

    // ---- pings: a red mark where an alarm saw someone (through walls), for a few seconds.
    let shown: Vec<&Ping> = if on {
        pings.iter().filter(|p| p.who != me && p.audience.includes(me)).collect()
    } else {
        Vec::new()
    };
    if st.icon.is_none() && !shown.is_empty() {
        st.icon = Some(images.add(crate::alertui::bang_icon([255, 60, 48])));
    }
    while st.marks.len() < shown.len() {
        let icon = st.icon.clone().unwrap_or_default();
        let e = commands
            .spawn((PingMark, ImageNode::new(icon), Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, GlobalZIndex(8), Pickable::IGNORE))
            .id();
        st.marks.push(e);
    }
    let cam = cam.single().ok();
    let size = 16.0 * k * 1.25;
    for (i, &m) in st.marks.iter().enumerate() {
        let Ok((mut n, mut vis)) = marks.get_mut(m) else { continue };
        let place = shown.get(i).and_then(|p| {
            let (camera, gt) = cam?;
            let at = p.at + DVec3::new(0.0, 0.0, 48.0);
            let v = camera.world_to_viewport(gt, Vec3::from(dx_level::ue_to_bevy(at.as_vec3().to_array()))).ok()?;
            Some((v, p.left))
        });
        match place {
            Some((v, left)) => {
                n.left = Val::Px((v.x - size * 0.5).round());
                n.top = Val::Px((v.y - size).round());
                n.width = Val::Px(size);
                n.height = Val::Px(size);
                // Blinking out over the last two seconds.
                let show = left > 2.0 || (left * 4.0).fract() < 0.6;
                vis.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
            }
            None => {
                vis.set_if_neq(Visibility::Hidden);
            }
        }
    }
}
