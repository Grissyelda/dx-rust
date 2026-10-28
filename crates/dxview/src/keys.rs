//! The NanoKey ring (NanoKeyRing / DeusExPlayer.KeyRing): the key codes the player has collected.
//!
//! * NanoKeys come from the world (a NanoKey pickup: KeyID + Description), from conversations (TransferObject
//!   "NanoKey" out of the giver's inventory) and from pawns that carry one (PickupDistributor.NanoKeyData gives
//!   pawns tagged ScriptedPawnTag a key at level start).
//! * With the ring in hand (belt slot 0), frobbing a door whose KeyIDNeeded is on the ring toggles its lock, and
//!   every mover sharing its Tag with it (DeusExMover.Frob).

use std::sync::Mutex;

/// (KeyID, Description), in the order collected.
static RING: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
/// Keys pawns carry: (pawn tag, KeyID, Description).
static CARRIED: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

pub fn has(id: &str) -> bool {
    RING.lock().is_ok_and(|r| r.iter().any(|(k, _)| k.eq_ignore_ascii_case(id)))
}

/// The keys on the ring (for the inventory screen).
pub fn list() -> Vec<(String, String)> {
    RING.lock().map(|r| r.clone()).unwrap_or_default()
}

/// The keys pawns still carry (save.rs).
pub fn carried() -> Vec<(String, String, String)> {
    CARRIED.lock().map(|c| c.clone()).unwrap_or_default()
}

/// Put the ring and the pawns' keys back as saved.
pub fn restore(ring: &[(String, String)], carried: &[(String, String, String)]) {
    if let Ok(mut r) = RING.lock() {
        *r = ring.to_vec();
    }
    if let Ok(mut c) = CARRIED.lock() {
        *c = carried.to_vec();
    }
}

/// KeyRing.GiveKey + DeusExPlayer.PickupNanoKey: the "%s added to Nano Key Ring" message.
pub fn give(id: &str, desc: &str) -> String {
    if let Ok(mut r) = RING.lock() {
        if !r.iter().any(|(k, _)| k.eq_ignore_ascii_case(id)) {
            r.push((id.to_string(), desc.to_string()));
        }
    }
    format!("{desc} added to Nano Key Ring")
}

/// PickupDistributor: what the level's pawns carry.
pub fn set_carried(list: &[(String, String, String)]) {
    if let Ok(mut c) = CARRIED.lock() {
        if c.is_empty() {
            *c = list.to_vec();
        }
    }
}

/// A pawn hands over (or drops) its nanokey: by its Tag or BindName. Returns the message, if it had one.
pub fn take_from(pawn: &str) -> Option<String> {
    let k = {
        let mut c = CARRIED.lock().ok()?;
        let i = c.iter().position(|(t, _, _)| t.eq_ignore_ascii_case(pawn))?;
        c.remove(i)
    };
    Some(give(&k.1, &k.2))
}

/// A new map is going in (loader.rs): this module's state of the map being left.
pub fn reset_level() {
    if let Ok(mut v) = RING.lock() {
        v.clear();
    }
    if let Ok(mut v) = CARRIED.lock() {
        v.clear();
    }
}
