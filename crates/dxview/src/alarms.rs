//! Alarm units (AlarmUnit): a pawn in the Alerting state runs to one and pushes it; while active (alarmTimeout
//! seconds) it sounds the klaxon, blinks its light, raises its Event and sends the 'Alarm' AI event (heard out
//! to 25 x (SoundRadius + 1) = 1625 units), which sends pawns that hate the alarm's instigator to search where
//! the alarm went off.

use bevy::prelude::*;
use glam::DVec3;
use unreal_pkg::props::{find, Value};

use crate::ai::Target;
use crate::LevelRes;

pub struct AlarmRt {
    pub actor: usize,
    pub pos: DVec3,
    /// Facing (radians): the pusher stands in front of it.
    pub yaw: f64,
    pub radius: f64,
    pub height: f64,
    pub alliance: String,
    pub tag: String,
    pub event: String,
    pub active: bool,
    pub t: f32,
    pub timeout: f32,
    pub instigator: Option<Target>,
    /// The pawn on its way to push it (associatedPawn), by blocker.
    pub associated: Option<usize>,
    /// Hacked: dead for good.
    pub disabled: bool,
    /// EMP'd (TakeDamage 'EMP': bConfused for alarmTimeout seconds): can't be set off meanwhile.
    pub confused: f32,
    light: (u8, u8, u8, f32),
    /// Not an AlarmUnit but another source of the 'Alarm' AI event (a security camera, a laser tripwire, a
    /// computer): nobody runs to push it, and its owner switches it on and off and sounds it.
    pub external: bool,
    /// Where pawns that hear it go looking (HandleAlarm's alarmLocation): the unit itself, a camera's last sight
    /// of the player, where a laser was broken.
    pub seek: DVec3,
}

impl AlarmRt {
    /// AI 'Alarm' event radius: 25 x (SoundRadius 64 + 1).
    pub const HEAR_RADIUS: f64 = 25.0 * 65.0;

    /// FindAlarmPosition: in front of the unit by both radii.
    pub fn push_spot(&self, pawn_radius: f64) -> DVec3 {
        self.pos + DVec3::new(self.yaw.cos(), self.yaw.sin(), 0.0) * (pawn_radius + self.radius)
    }

    /// IsAlarmInRange.
    pub fn in_range(&self, at: DVec3, pawn_radius: f64, pawn_height: f64) -> bool {
        DVec3::new(self.pos.x - at.x, self.pos.y - at.y, 0.0).length()
            < pawn_radius + self.radius + 24.0
            && (self.pos.z - at.z).abs() < pawn_height + self.height
    }

    /// IsAlarmReady for `who`.
    pub fn ready_for(&self, who: usize) -> bool {
        !self.active && !self.disabled && self.associated.map_or(true, |a| a == who)
    }
}

static HACKED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
static WARREN: std::sync::Mutex<Vec<(DVec3, f64)>> = std::sync::Mutex::new(Vec::new());

/// Warren's EMP field (eggs.rs): every alarm unit within `radius` that isn't confused takes 100 EMP.
pub fn warren(center: DVec3, radius: f64) {
    if let Ok(mut q) = WARREN.lock() {
        q.push((center, radius));
    }
}

/// A multitool bypass finished on the actor `actor`. AlarmUnit.HackAction: an alarm that is sounding goes quiet
/// and is disabled for good; one that isn't sounding is left alone (the script never sets one off this way).
pub fn hacked(actor: usize) {
    if let Ok(mut q) = HACKED.lock() {
        q.push(actor);
    }
}

#[derive(Resource, Default)]
pub struct Alarms {
    pub units: Vec<AlarmRt>,
    started: bool,
    /// Events to raise (drained into GameEvents).
    events: Vec<(String, bool)>,
}

impl Alarms {
    pub fn is_built(&self) -> bool {
        self.started
    }
    /// What a save keeps (save.rs): which placed alarm units were disabled (hacked). A sounding alarm isn't kept:
    /// it would have timed out by the time anyone reacts to it again.
    pub fn save(&self) -> Vec<usize> {
        self.units.iter().filter(|u| !u.external && u.disabled).map(|u| u.actor).collect()
    }
    pub fn restore(&mut self, disabled: &[usize]) {
        for u in self.units.iter_mut().filter(|u| !u.external) {
            u.disabled = disabled.contains(&u.actor);
        }
    }

    /// AlarmUnit.Trigger: switch on (and every unit sharing its Tag), raise Event.
    pub fn trigger(&mut self, i: usize, instigator: Option<Target>) {
        let Some(u) = self.units.get(i) else { return };
        if u.disabled || u.active || u.confused > 0.0 {
            return;
        }
        let tag = u.tag.clone();
        let mut fire = vec![i];
        if !tag.is_empty() {
            fire.extend(
                self.units
                    .iter()
                    .enumerate()
                    .filter(|(j, o)| *j != i && o.tag.eq_ignore_ascii_case(&tag))
                    .map(|(j, _)| j),
            );
        }
        for j in fire {
            let u = &mut self.units[j];
            if u.disabled || u.active || u.confused > 0.0 {
                continue;
            }
            u.active = true;
            u.t = 0.0;
            u.instigator = instigator;
            if !u.event.is_empty() {
                self.events.push((u.event.clone(), true));
            }
            println!("ALARM {} triggered", j);
        }
    }

    /// Register another source of the 'Alarm' AI event (SecurityCamera / LaserTrigger / Computers); returns its
    /// index for `set_external`.
    pub fn add_external(&mut self, actor: usize, pos: DVec3) -> usize {
        self.units.push(AlarmRt {
            actor,
            pos,
            yaw: 0.0,
            radius: 0.0,
            height: 0.0,
            alliance: String::new(),
            tag: String::new(),
            event: String::new(),
            active: false,
            t: 0.0,
            timeout: 0.0,
            instigator: None,
            associated: None,
            disabled: false,
            confused: 0.0,
            light: (0, 0, 0, 0.0),
            external: true,
            seek: pos,
        });
        self.units.len() - 1
    }

    /// AIStartEvent / AIEndEvent('Alarm') for an external source.
    pub fn set_external(&mut self, i: usize, active: bool, seek: DVec3, instigator: Option<Target>) {
        if let Some(u) = self.units.get_mut(i) {
            u.active = active;
            u.seek = seek;
            u.instigator = if active { instigator } else { None };
        }
    }

    /// Is any alarm going off within earshot (25 x 65 units) of `at`? (AlarmHeard on turrets.)
    pub fn heard_at(&self, at: DVec3) -> bool {
        self.units
            .iter()
            .any(|u| u.active && u.pos.distance(at) < AlarmRt::HEAR_RADIUS)
    }

    /// The nearest alarm a pawn could run to (Alerting.FindTarget): within 2400 (a squashed sphere), not
    /// belonging to an alliance it hates, not already going off or claimed by someone else.
    pub fn find_target(&self, me: usize, at: DVec3, hates: impl Fn(&str) -> bool) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, u) in self.units.iter().enumerate() {
            if u.external || hates(&u.alliance) {
                continue;
            }
            let d = (u.pos - at) * DVec3::new(1.0, 1.0, 2.0);
            if d.length() > 2400.0 {
                continue;
            }
            if best.map_or(true, |(_, bd)| d.length() < bd) {
                best = Some((i, d.length()));
            }
        }
        best.filter(|(i, _)| self.units[*i].ready_for(me))
            .map(|(i, _)| i)
    }
}

pub fn alarm_system(
    time: Res<Time>,
    level: Res<LevelRes>,
    mut alarms: ResMut<Alarms>,
    mut events: ResMut<crate::devices::GameEvents>,
) {
    let _prof = crate::prof::scope("alarms::alarm_system");
    let lvl = &level.0;
    if !alarms.started {
        alarms.started = true;
        for (i, a) in lvl.actors.iter().enumerate() {
            if a.class != "AlarmUnit" {
                continue;
            }
            let Some(Value::Vector(p)) = find(&a.props, "Location") else {
                continue;
            };
            let yaw = match find(&a.props, "Rotation") {
                Some(Value::Rotator(r)) => r[1] as f64 / 65536.0 * std::f64::consts::TAU,
                _ => 0.0,
            };
            let f = |n: &str, d: f32| match find(&a.props, n) {
                Some(Value::Float(v)) => *v,
                _ => d,
            };
            let b = |n: &str, d: u8| match find(&a.props, n) {
                Some(Value::Byte(v)) => *v,
                _ => d,
            };
            let name = |n: &str| crate::devices::name_prop(&a.props, n);
            let timeout = match find(&a.props, "alarmTimeout") {
                Some(Value::Int(v)) => *v as f32,
                _ => 30.0,
            };
            let pos = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
            alarms.units.push(AlarmRt {
                actor: i,
                pos,
                external: false,
                seek: pos,
                yaw,
                radius: f("CollisionRadius", 9.72) as f64,
                height: f("CollisionHeight", 9.72) as f64,
                alliance: name("Alliance"),
                tag: name("Tag"),
                event: name("Event"),
                active: false,
                t: 0.0,
                timeout,
                instigator: None,
                associated: None,
                disabled: false,
            confused: 0.0,
                light: (
                    b("LightHue", 0),
                    b("LightSaturation", 255),
                    b("LightBrightness", 255),
                    b("LightRadius", 1) as f32,
                ),
            });
        }
        println!("alarms: {}", alarms.units.len());
    }
    for actor in HACKED
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
    {
        for u in alarms.units.iter_mut().filter(|u| u.actor == actor) {
            if u.active {
                u.active = false;
                u.disabled = true;
                u.t = 0.0;
                u.instigator = None;
                u.associated = None;
                println!("ALARM {} disabled by hacking", lvl.actors[actor].name);
            }
        }
    }
    let dt = time.delta_secs();
    // AlarmUnit.TakeDamage 'EMP': confused for alarmTimeout (a sounding one goes quiet), EMPZap.
    for (c, r) in WARREN.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
        for u in alarms.units.iter_mut().filter(|u| !u.external && u.confused <= 0.0 && u.pos.distance(c) < r) {
            u.confused = u.timeout.max(1.0);
            u.active = false;
            u.t = 0.0;
            crate::sound::play("DeusExSounds.Weapons.EMPZap", u.pos, 1.0, 1280.0, 1.0);
            crate::eggs::device_sparks(u.pos);
        }
    }
    for u in alarms.units.iter_mut() {
        if u.confused > 0.0 {
            u.confused -= dt;
        }
    }
    let now = time.elapsed_secs_f64();
    for (i, u) in alarms.units.iter_mut().enumerate() {
        if !u.active || u.external {
            continue;
        }
        u.t += dt;
        if u.t >= u.timeout {
            // UnTrigger: back to the hum.
            u.active = false;
            u.t = 0.0;
            u.instigator = None;
            u.associated = None;
            continue;
        }
        crate::sound::hold(
            crate::sound::key_of(&format!("alarm-klaxon-{i}")),
            "Ambient.Ambient.Klaxon2",
            Some(u.pos),
            128.0 / 255.0,
            AlarmRt::HEAR_RADIUS as f32,
            1.0,
        );
        // The light blinks twice a second.
        if now % 0.5 > 0.25 {
            let (h, s, b, r) = u.light;
            crate::dynlight::hold(
                0xA_0000_0000 | i as u64,
                u.pos,
                crate::dynlight::Spec::new(h, s, b, r),
                0.0,
            );
        }
    }
    events.0.append(&mut alarms.events);
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = HACKED.lock() {
        v.clear();
    }
}
