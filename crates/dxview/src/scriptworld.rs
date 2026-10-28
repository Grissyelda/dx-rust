//! Mission-script changes to actors that aren't pawns' orders: bHidden on and off (a keypad, a book, a body, a switch,
//! the Earth hologram stand), Destroy, Vehicles.EnterWorld / LeaveWorld (helicopters, the mini-sub: hidden, out of
//! collision, silent), and DesiredRotation yaw turns (the NSF HQ satellite dishes). Selections are by class (IsA) and
//! Tag, like `foreach AllActors(class'X', A, 'Tag')`.

use crate::actors::{Blockers, Deco, MeshActorRef};
use crate::LevelRes;
use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::Mutex;
use unreal_pkg::props::{find, Value};

static HIDDEN: Mutex<Option<HashSet<usize>>> = Mutex::new(None);
static REQS: Mutex<Vec<ActorReq>> = Mutex::new(Vec::new());

/// Actors (by level actor index) drawn hidden right now: not frobbable, not searched.
pub fn is_hidden(actor: usize) -> bool {
    HIDDEN.lock().ok().is_some_and(|h| h.as_ref().is_some_and(|s| s.contains(&actor)))
}

pub fn mark_hidden(actor: usize, hidden: bool) {
    if let Ok(mut h) = HIDDEN.lock() {
        let s = h.get_or_insert_with(HashSet::new);
        if hidden {
            s.insert(actor);
        } else {
            s.remove(&actor);
        }
    }
}

pub fn reset_level() {
    if let Ok(mut h) = HIDDEN.lock() {
        *h = None;
    }
    if let Ok(mut q) = REQS.lock() {
        q.clear();
    }
}

/// What a save keeps: every hidden actor.
pub fn save() -> Vec<usize> {
    let mut v: Vec<usize> = HIDDEN.lock().ok().and_then(|h| h.as_ref().map(|s| s.iter().copied().collect())).unwrap_or_default();
    v.sort_unstable();
    v
}

/// Back from a save: show what isn't in the list, hide what is.
pub fn restore(list: &[usize]) {
    push(ActorReq::Restore(list.to_vec()));
}

/// foreach AllActors(class, A, tag): class by IsA (None = any), tag exact (None = any).
#[derive(Debug, Clone, Default)]
pub struct ActorSel {
    pub class: Option<String>,
    pub tag: Option<String>,
}

impl ActorSel {
    pub fn class(c: &str) -> ActorSel {
        ActorSel { class: Some(c.into()), tag: None }
    }
    pub fn tag(t: &str) -> ActorSel {
        ActorSel { class: None, tag: Some(t.into()) }
    }
    pub fn class_tag(c: &str, t: &str) -> ActorSel {
        ActorSel { class: Some(c.into()), tag: Some(t.into()) }
    }
    pub fn matches(&self, class: &str, tag: &str) -> bool {
        self.class.as_ref().is_none_or(|c| crate::classtree::is_a(class, c))
            && self.tag.as_ref().is_none_or(|t| t.eq_ignore_ascii_case(tag))
    }
}

#[derive(Debug, Clone)]
pub enum ActorReq {
    /// bHidden = true / false.
    Hide(ActorSel, bool),
    Destroy(ActorSel),
    /// Vehicles.PutInWorld(enter).
    Vehicle(ActorSel, bool),
    /// DesiredRotation.Yaw (turned at RotationRate).
    Yaw(ActorSel, i32),
    Restore(Vec<usize>),
}

pub fn push(r: ActorReq) {
    if let Ok(mut q) = REQS.lock() {
        q.push(r);
    }
}

fn tag_of(p: &[unreal_pkg::props::Property]) -> String {
    match find(p, "Tag") {
        Some(Value::Name(t)) => t.clone(),
        _ => String::new(),
    }
}

/// Turning decorations: (entity, goal yaw, rate per second).
#[derive(Default)]
pub struct Turning(Vec<(Entity, i32, f64)>);

#[allow(clippy::type_complexity)]
pub fn scriptworld_system(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<LevelRes>,
    mut blockers: ResMut<Blockers>,
    mut q: Query<(Entity, &MeshActorRef, &mut Visibility, Option<&mut Deco>)>,
    mut turning: Local<Turning>,
    mut map_gen: Local<u64>,
    mut started: Local<bool>,
) {
    if crate::loader::new_map(&mut map_gen) {
        *started = false;
        turning.0.clear();
    }
    let lvl = &level.0;
    let mut reqs: Vec<ActorReq> = REQS.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    if !*started {
        *started = true;
        // Vehicles.PostBeginPlay: bInWorld=False ones leave the world at once.
        let mut out = Vec::new();
        for (_, mr, _, _) in &q {
            let a = &lvl.actors[lvl.mesh_actors[mr.0].actor];
            if crate::classtree::is_a(&a.class, "Vehicles") && dx_level::actors::out_of_world(&a.props) {
                out.push(mr.0);
            }
        }
        for mi in out {
            let ai = lvl.mesh_actors[mi].actor;
            reqs.insert(0, ActorReq::Vehicle(ActorSel { class: Some(lvl.actors[ai].class.clone()), tag: Some(format!("#actor{ai}")) }, false));
        }
    }
    for r in reqs {
        if let ActorReq::Restore(list) = &r {
            let set: HashSet<usize> = list.iter().copied().collect();
            for (_, mr, mut vis, _) in &mut q {
                let ai = lvl.mesh_actors[mr.0].actor;
                let hide = set.contains(&ai);
                *vis = if hide { Visibility::Hidden } else { Visibility::Inherited };
                mark_hidden(ai, hide);
                if let Some(b) = blockers.list.iter_mut().find(|c| c.actor == ai) {
                    if hide {
                        b.radius = 0.0;
                    }
                }
            }
            continue;
        }
        for (e, mr, mut vis, deco) in &mut q {
            let ai = lvl.mesh_actors[mr.0].actor;
            let a = &lvl.actors[ai];
            let tag = tag_of(&a.props);
            let sel = match &r {
                ActorReq::Hide(s, _) | ActorReq::Destroy(s) | ActorReq::Vehicle(s, _) | ActorReq::Yaw(s, _) => s,
                ActorReq::Restore(_) => continue,
            };
            let hit = match &sel.tag {
                Some(t) if t.starts_with("#actor") => t[6..].parse::<usize>().ok() == Some(ai),
                _ => sel.matches(&a.class, &tag),
            };
            if !hit {
                continue;
            }
            let radius = match find(&a.props, "CollisionRadius") {
                Some(Value::Float(v)) => *v as f64,
                _ => 0.0,
            };
            let blocker = blockers.list.iter().position(|c| c.actor == ai);
            match &r {
                ActorReq::Hide(_, hide) => {
                    *vis = if *hide { Visibility::Hidden } else { Visibility::Inherited };
                    mark_hidden(ai, *hide);
                    if let Some(b) = blocker {
                        let blocks = matches!(find(&a.props, "bBlockPlayers"), Some(Value::Bool(true)));
                        blockers.list[b].radius = if *hide || !blocks { 0.0 } else { radius };
                        if let Some(d) = deco.as_ref() {
                            blockers.list[b].center = d.body.location;
                        }
                    }
                    println!("SCRIPT {} {} hidden={hide}", a.name, tag);
                }
                ActorReq::Vehicle(_, enter) => {
                    let hide = !*enter;
                    *vis = if hide { Visibility::Hidden } else { Visibility::Inherited };
                    mark_hidden(ai, hide);
                    if let Some(b) = blocker {
                        blockers.list[b].radius = if hide { 0.0 } else { radius };
                    }
                    if hide {
                        crate::sound::own_ambient(ai);
                    } else {
                        crate::sound::release_ambient(ai);
                    }
                    println!("SCRIPT vehicle {} {}", a.name, if *enter { "enters the world" } else { "leaves the world" });
                }
                ActorReq::Destroy(_) => {
                    if let Some(b) = blocker {
                        blockers.list[b].radius = 0.0;
                        blockers.list[b].height = 0.0;
                        blockers.deco.remove(&b);
                        blockers.pushable_mass.remove(&b);
                    }
                    crate::sound::own_ambient(ai);
                    // Kept gone in the level's save (a hidden actor is drawn and touched as absent).
                    mark_hidden(ai, true);
                    commands.entity(e).despawn();
                    println!("SCRIPT destroy {} {}", a.name, tag);
                }
                ActorReq::Yaw(_, yaw) => {
                    let rate = match find(&a.props, "RotationRate") {
                        Some(Value::Rotator(r)) => r[1].unsigned_abs().max(1) as f64,
                        _ => 1024.0,
                    };
                    turning.0.retain(|t| t.0 != e);
                    turning.0.push((e, *yaw, rate));
                }
                _ => {}
            }
        }
    }
    // DesiredRotation: turn toward the goal yaw at RotationRate (shortest way round).
    let dt = time.delta_secs_f64().min(0.1);
    turning.0.retain(|(e, goal, rate)| {
        let Ok((_, _, _, Some(mut d))) = q.get_mut(*e) else { return false };
        let cur = d.rotation[1];
        let diff = ((goal - cur) % 65536 + 65536 + 32768) % 65536 - 32768;
        let step = (rate * dt) as i32;
        if diff.abs() <= step.max(1) {
            d.rotation[1] = *goal;
            d.spun = true;
            return false;
        }
        d.rotation[1] = cur + step * diff.signum();
        d.spun = true;
        true
    });
}
