//! The player's doings that pawns react to by sight, gathered each frame for the AI (ScriptedPawn reaction callbacks;
//! the rules live in imsim_core::reactions and ai.rs):
//! - WeaponDrawn (DeusExWeapon.BringUp: AIStartEvent while a weapon is out) -> HandleWeapon (bHateWeapon, bFearWeapon),
//! - MegaFutz (a lock being picked or a device bypassed: AIStartEvent while it lasts) -> HandleHacking,
//! - Futz (frobbing or shooting something bOwned: AISendEvent once) -> HandleFutz (a futz bark).

use glam::DVec3;
use std::sync::Mutex;

#[derive(Default, Clone)]
pub struct Stimuli {
    /// A weapon is held up (not a tool or item).
    pub weapon_drawn: bool,
    /// A lockpick or multitool is being worked on something here.
    pub hacking: Option<DVec3>,
    /// Owned things messed with since the AI last looked (where).
    pub futz: Vec<DVec3>,
}

static STIM: Mutex<Option<Stimuli>> = Mutex::new(None);

fn with(f: impl FnOnce(&mut Stimuli)) {
    if let Ok(mut s) = STIM.lock() {
        f(s.get_or_insert_with(Stimuli::default));
    }
}

pub fn set_weapon_drawn(on: bool) {
    with(|s| s.weapon_drawn = on);
}

pub fn set_hacking(at: Option<DVec3>) {
    with(|s| s.hacking = at);
}

/// AISendEvent('Futz', EAITYPE_Visual): the player touched or shot something that belongs to someone.
pub fn futz(at: DVec3) {
    with(|s| s.futz.push(at));
}

/// The AI's view this frame (futz events are consumed).
pub fn take() -> Stimuli {
    let mut out = Stimuli::default();
    with(|s| {
        out = s.clone();
        s.futz.clear();
    });
    out
}

pub fn reset_level() {
    if let Ok(mut s) = STIM.lock() {
        *s = None;
    }
}
